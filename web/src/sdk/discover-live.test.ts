import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LiveSession, type LiveStatus } from "./discover-live";
import type { Frame, Scope } from "./discover";

// ---------------------------------------------------------------------------
// FakeWebSocket — captures sent messages, lets tests simulate WS events
// ---------------------------------------------------------------------------

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];

  url: string;
  sent: string[] = [];
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  readyState: number = 0; // CONNECTING

  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close(code?: number, reason?: string) {
    this.readyState = 3; // CLOSED
    if (this.onclose) {
      this.onclose(new CloseEvent("close", { code: code ?? 1000, reason, wasClean: code === undefined || code === 1000 }));
    }
  }

  // Test helpers to simulate server-side events
  simulateOpen() {
    this.readyState = 1; // OPEN
    if (this.onopen) this.onopen(new Event("open"));
  }

  simulateMessage(data: unknown) {
    if (this.onmessage) {
      this.onmessage(new MessageEvent("message", { data: JSON.stringify(data) }));
    }
  }

  simulateMalformedMessage(raw: string) {
    if (this.onmessage) {
      this.onmessage(new MessageEvent("message", { data: raw }));
    }
  }

  simulateClose(code: number, reason = "") {
    this.readyState = 3; // CLOSED
    if (this.onclose) {
      this.onclose(new CloseEvent("close", { code, reason, wasClean: code === 1000 }));
    }
  }

  static reset() {
    FakeWebSocket.instances = [];
  }

  static latest(): FakeWebSocket {
    return FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
  }
}

function makeFactory() {
  return (url: string): WebSocket => new FakeWebSocket(url) as unknown as WebSocket;
}

const defaultScope: Scope = { kind: "all" };
const defaultBody = {
  query: "error",
  scope: defaultScope,
  time_range: { from: "2024-01-01T00:00:00Z", to: "2024-01-02T00:00:00Z" } as { from: string; to: string } | null,
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("LiveSession", () => {
  beforeEach(() => {
    FakeWebSocket.reset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // (a) Sends auth as FIRST message then subscribe after open
  it("(a) sends auth as first message then subscribe after open", () => {
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok-123",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: makeFactory(),
    });

    const ws = FakeWebSocket.latest();
    expect(ws.url).toBe("ws://localhost:8080/v1/explore/live");
    // Before open: nothing sent yet
    expect(ws.sent).toHaveLength(0);

    ws.simulateOpen();

    // After open: auth first, then subscribe
    expect(ws.sent).toHaveLength(2);
    expect(JSON.parse(ws.sent[0])).toEqual({ type: "auth", token: "tok-123" });
    const sub = JSON.parse(ws.sent[1]);
    expect(sub.type).toBe("subscribe");
    expect(sub.query).toBe("error");
    expect(sub.scope).toEqual({ kind: "all" });
    expect(sub.time_range).toEqual({ from: "2024-01-01T00:00:00Z", to: "2024-01-02T00:00:00Z" });

    session.close();
  });

  // (b) Emits parsed frames via onFrame
  it("(b) emits parsed frames via onFrame", () => {
    const frames: Frame[] = [];
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: (f) => frames.push(f),
      onStatus: () => {},
      wsFactory: makeFactory(),
    });

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();
    ws.simulateMessage({ type: "plan", sources: [{ source: "db.t", has_timestamp: true }] });
    ws.simulateMessage({ type: "rows", source: "db.t", rows: [{ msg: "hello" }] });
    ws.simulateMessage({ type: "live" });
    ws.simulateMessage({ type: "heartbeat" });

    expect(frames).toHaveLength(4);
    expect(frames[0].type).toBe("plan");
    expect(frames[1].type).toBe("rows");
    expect(frames[2].type).toBe("live");
    expect(frames[3].type).toBe("heartbeat");

    session.close();
  });

  // (c) update() sends an update message
  it("(c) update() sends an update message", () => {
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: makeFactory(),
    });

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();
    ws.sent = []; // clear auth + subscribe

    const newBody = { query: "warn", scope: defaultScope, time_range: null };
    session.update(newBody);

    expect(ws.sent).toHaveLength(1);
    const msg = JSON.parse(ws.sent[0]);
    expect(msg.type).toBe("update");
    expect(msg.query).toBe("warn");
    expect(msg.scope).toEqual({ kind: "all" });
    expect(msg.time_range).toBeNull();

    session.close();
  });

  // (d) Abnormal close schedules reconnect with backoff and resubscribes with overlap
  it("(d) on abnormal close schedules reconnect with backoff and resubscribes with from-overlap", async () => {
    vi.useFakeTimers();
    const statuses: LiveStatus[] = [];
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: (s) => statuses.push(s),
      wsFactory: makeFactory(),
    });

    const ws1 = FakeWebSocket.latest();
    ws1.simulateOpen();
    // Simulate a rows frame arriving at a known wall-clock time
    const knownTime = new Date("2024-01-01T12:00:00Z").getTime();
    vi.setSystemTime(knownTime);
    ws1.simulateMessage({ type: "rows", source: "db.t", rows: [{ msg: "hi" }] });
    // Advance time a bit
    vi.setSystemTime(knownTime + 10_000);

    // Abnormal close (code 1006)
    ws1.simulateClose(1006, "network error");

    // Should NOT reconnect immediately — should schedule with backoff
    expect(FakeWebSocket.instances).toHaveLength(1);

    // Advance fake timers by 1s (first backoff)
    await vi.advanceTimersByTimeAsync(1000);

    expect(FakeWebSocket.instances).toHaveLength(2);
    const ws2 = FakeWebSocket.latest();
    ws2.simulateOpen();

    // Reconnect subscribe should use time_range.from = lastEventAt - 5000
    expect(ws2.sent).toHaveLength(2);
    expect(JSON.parse(ws2.sent[0])).toMatchObject({ type: "auth", token: "tok" });
    const resubMsg = JSON.parse(ws2.sent[1]);
    expect(resubMsg.type).toBe("subscribe");
    // from = knownTime - 5000ms → "2024-01-01T11:59:55.000Z"
    const expectedFrom = new Date(knownTime - 5000).toISOString();
    expect(resubMsg.time_range.from).toBe(expectedFrom);
    // to should be current time at reconnect — knownTime + 10_000 (set before close) + 1_000 (backoff timer)
    expect(resubMsg.time_range.to).toBe(new Date(knownTime + 10_000 + 1_000).toISOString());

    session.close();
  });

  // (e) close() prevents reconnect
  it("(e) close() prevents reconnect", async () => {
    vi.useFakeTimers();
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: makeFactory(),
    });

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();

    // close the session first
    session.close();

    // Simulate an abnormal close AFTER we called close()
    // (the ws.close() from session.close() triggers onclose with 1000)
    // Try an abnormal one too
    ws.simulateClose(1006, "late close");

    // Advance timers — no new WS should be created
    await vi.advanceTimersByTimeAsync(5000);
    expect(FakeWebSocket.instances).toHaveLength(1);
  });

  // (f) Status transitions: "connecting" → "backfilling" after subscribe → "live" on live frame
  it("(f) emits status transitions: connecting → backfilling → live", () => {
    const statuses: LiveStatus[] = [];
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: (s) => statuses.push(s),
      wsFactory: makeFactory(),
    });

    expect(statuses).toContain("connecting");

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();
    // After sending auth+subscribe, status becomes "backfilling"
    expect(statuses).toContain("backfilling");

    ws.simulateMessage({ type: "live" });
    expect(statuses[statuses.length - 1]).toBe("live");

    session.close();
    expect(statuses[statuses.length - 1]).toBe("closed");
  });

  // (g) Malformed JSON frame is ignored without killing the session
  it("(g) malformed JSON is ignored without killing the session", () => {
    const frames: Frame[] = [];
    const errors: unknown[] = [];
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: (f) => frames.push(f),
      onStatus: () => {},
      wsFactory: makeFactory(),
    });

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();
    ws.simulateMalformedMessage("{not valid json");
    // Session should still be alive — send a valid message after
    ws.simulateMessage({ type: "heartbeat" });

    expect(frames).toHaveLength(1);
    expect(frames[0].type).toBe("heartbeat");
    expect(errors).toHaveLength(0);

    session.close();
  });

  // backoff cap: after several reconnects backoff should not exceed 30s
  it("backoff increases exponentially and caps at 30s", async () => {
    vi.useFakeTimers();
    let connectCount = 0;

    const factory = (url: string): WebSocket => {
      const ws = new FakeWebSocket(url) as unknown as WebSocket;
      connectCount++;
      return ws;
    };

    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: factory,
    });

    // Open and immediately close abnormally each time
    const openAndClose = async (delay: number) => {
      const ws = FakeWebSocket.latest();
      ws.simulateOpen();
      ws.simulateClose(1006);
      await vi.advanceTimersByTimeAsync(delay);
    };

    // 1st connection opens, closes → backoff 1s
    await openAndClose(1000); // advances 1s → 2nd connects
    expect(connectCount).toBe(2);
    await openAndClose(2000); // backoff 2s → 3rd
    expect(connectCount).toBe(3);
    await openAndClose(4000); // backoff 4s → 4th
    expect(connectCount).toBe(4);
    await openAndClose(8000); // backoff 8s → 5th
    expect(connectCount).toBe(5);
    await openAndClose(16000); // backoff 16s → 6th
    expect(connectCount).toBe(6);
    await openAndClose(30000); // backoff capped at 30s → 7th
    expect(connectCount).toBe(7);

    session.close();
  });

  // on 1008 (auth failure) — no reconnect, status → error
  it("on close code 1008 sets status error and does not reconnect", async () => {
    vi.useFakeTimers();
    const statuses: LiveStatus[] = [];
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "bad-token",
      body: defaultBody,
      onFrame: () => {},
      onStatus: (s) => statuses.push(s),
      wsFactory: makeFactory(),
    });

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();
    ws.simulateClose(1008, "policy violation");

    await vi.advanceTimersByTimeAsync(5000);
    // No new WS created
    expect(FakeWebSocket.instances).toHaveLength(1);
    expect(statuses).toContain("error");

    session.close();
  });

  // if no rows frame received yet, reconnect uses original time_range
  it("reconnect with no lastEventAt reuses original time_range", async () => {
    vi.useFakeTimers();
    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: makeFactory(),
    });

    const ws1 = FakeWebSocket.latest();
    ws1.simulateOpen();
    // No rows frame — close abnormally
    ws1.simulateClose(1006);

    await vi.advanceTimersByTimeAsync(1000);
    const ws2 = FakeWebSocket.latest();
    ws2.simulateOpen();

    const resubMsg = JSON.parse(ws2.sent[1]);
    expect(resubMsg.time_range).toEqual(defaultBody.time_range);

    session.close();
  });

  // Fix-6a: duplicate onclose (called twice with 1006) → only ONE new socket
  it("(6a) duplicate onclose fires only one reconnect", async () => {
    vi.useFakeTimers();
    let factoryCallCount = 0;
    const factory = (url: string): WebSocket => {
      factoryCallCount++;
      return new FakeWebSocket(url) as unknown as WebSocket;
    };

    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: factory,
    });

    expect(factoryCallCount).toBe(1); // initial connect

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();
    // Simulate the close event firing twice (e.g. onerror then onclose)
    ws.simulateClose(1006, "network error");
    ws.simulateClose(1006, "network error");

    // Timer not yet fired — still only the first socket
    expect(factoryCallCount).toBe(1);

    // Advance past the 1s backoff — only ONE new socket should be created
    await vi.advanceTimersByTimeAsync(1000);
    expect(factoryCallCount).toBe(2);

    session.close();
  });

  // Fix-6b: close() while reconnect timer pending → timer fires → NO new socket
  it("(6b) close() while reconnect timer pending prevents new socket", async () => {
    vi.useFakeTimers();
    let factoryCallCount = 0;
    const factory = (url: string): WebSocket => {
      factoryCallCount++;
      return new FakeWebSocket(url) as unknown as WebSocket;
    };

    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: factory,
    });

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();
    ws.simulateClose(1006, "network error");

    // Reconnect timer is now pending; close the session before it fires
    session.close();

    // Advance past the 1s backoff — session is closed, no new socket
    await vi.advanceTimersByTimeAsync(1000);
    expect(factoryCallCount).toBe(1);
  });

  // Fix-6c: server-initiated clean 1000 → reconnect IS scheduled (Fix 3)
  it("(6c) server-initiated clean 1000 close triggers a reconnect", async () => {
    vi.useFakeTimers();
    let factoryCallCount = 0;
    const factory = (url: string): WebSocket => {
      factoryCallCount++;
      return new FakeWebSocket(url) as unknown as WebSocket;
    };

    const session = new LiveSession({
      endpoint: "http://localhost:8080",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: factory,
    });

    const ws = FakeWebSocket.latest();
    ws.simulateOpen();
    // Simulate a server-initiated clean close (code 1000, wasClean=true)
    ws.simulateClose(1000, "server shutdown");

    // Advance timers — a reconnect SHOULD have been scheduled
    await vi.advanceTimersByTimeAsync(1000);
    expect(factoryCallCount).toBe(2);

    session.close();
  });

  // Fix-6d: trailing-slash endpoint produces a single-slash-free URL
  it("(6d) trailing-slash endpoint produces correct WS URL", () => {
    let capturedUrl = "";
    const factory = (url: string): WebSocket => {
      capturedUrl = url;
      return new FakeWebSocket(url) as unknown as WebSocket;
    };

    const session = new LiveSession({
      endpoint: "http://localhost:8080/",
      token: "tok",
      body: defaultBody,
      onFrame: () => {},
      onStatus: () => {},
      wsFactory: factory,
    });

    expect(capturedUrl).toBe("ws://localhost:8080/v1/explore/live");
    session.close();
  });
});
