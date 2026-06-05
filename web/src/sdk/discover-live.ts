import type { Frame, Scope } from "./discover";

export type LiveStatus = "connecting" | "backfilling" | "live" | "closed" | "error";

export type LiveBody = {
  query: string;
  scope: Scope;
  time_range: { from: string; to: string } | null;
  per_source_limit?: number;
};

type LiveSessionOpts = {
  endpoint: string;
  token: string;
  body: LiveBody;
  onFrame: (f: Frame) => void;
  onStatus: (s: LiveStatus) => void;
  wsFactory?: (url: string) => WebSocket;
};

const BACKOFF_INITIAL_MS = 1_000;
const BACKOFF_MAX_MS = 30_000;
const NO_RECONNECT_CODES = new Set([1008]); // policy violation — bad token

export class LiveSession {
  private readonly opts: LiveSessionOpts;
  private readonly wsUrl: string;

  private ws: WebSocket | null = null;
  private closed = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private backoffMs = BACKOFF_INITIAL_MS;

  /** Wall-clock ms at the time of the last received rows frame. */
  private lastEventAt: number | null = null;

  /** Current body (may be updated via update()). */
  private currentBody: LiveBody;

  constructor(opts: LiveSessionOpts) {
    this.opts = opts;
    this.currentBody = { ...opts.body };
    this.wsUrl = opts.endpoint.replace(/\/$/, "").replace(/^http/, "ws") + "/v1/explore/live";
    this.emitStatus("connecting");
    this.connect();
  }

  private defaultFactory(url: string): WebSocket {
    return new WebSocket(url);
  }

  private openWs(): WebSocket {
    const factory = this.opts.wsFactory ?? ((url: string) => this.defaultFactory(url));
    return factory(this.wsUrl);
  }

  private connect() {
    if (this.closed) return;
    const ws = this.openWs();
    // Assign this.ws BEFORE attaching handlers so that stale-socket guards
    // (ws !== this.ws) inside handlers are reliable from the first event.
    this.ws = ws;

    ws.onopen = () => {
      if (this.closed || ws !== this.ws) {
        ws.close();
        return;
      }
      // Send auth first, then subscribe
      ws.send(JSON.stringify({ type: "auth", token: this.opts.token }));
      ws.send(JSON.stringify({ type: "subscribe", ...this.currentBody }));
      this.emitStatus("backfilling");
    };

    ws.onmessage = (ev: MessageEvent) => {
      if (this.closed) return;
      let frame: Frame;
      try {
        frame = JSON.parse(ev.data as string) as Frame;
      } catch {
        // Malformed JSON — ignore without killing the session
        return;
      }

      // Track last event time for reconnect overlap
      if (frame.type === "rows") {
        this.lastEventAt = Date.now();
      }

      // Status transitions
      if (frame.type === "live") {
        this.backoffMs = BACKOFF_INITIAL_MS; // reset backoff on successful live
        this.emitStatus("live");
      }

      this.opts.onFrame(frame);
    };

    ws.onclose = (ev: CloseEvent) => {
      if (this.closed || ws !== this.ws) return;

      // Auth failure — do not reconnect
      if (NO_RECONNECT_CODES.has(ev.code)) {
        this.emitStatus("error");
        return;
      }

      // Do NOT special-case wasClean/1000 here: client-initiated closes are
      // already caught by `this.closed` above; a server-initiated clean 1000
      // is still an unexpected disconnect and should trigger a reconnect.
      this.scheduleReconnect();
    };

    ws.onerror = (_ev: Event) => {
      // onclose will fire after onerror; handle reconnect there
    };
  }

  private scheduleReconnect() {
    if (this.closed) return;
    // Guard against duplicate scheduling (e.g. onerror + onclose both firing)
    if (this.reconnectTimer !== null) return;
    const delay = this.backoffMs;
    // Double for next time, capped at max
    this.backoffMs = Math.min(this.backoffMs * 2, BACKOFF_MAX_MS);

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (this.closed) return;

      // Adjust time_range for reconnect: if we saw events, use overlap window
      if (this.lastEventAt !== null) {
        const overlapFrom = new Date(this.lastEventAt - 5_000).toISOString();
        const overlapTo = new Date(Date.now()).toISOString();
        this.currentBody = {
          ...this.currentBody,
          time_range: { from: overlapFrom, to: overlapTo },
        };
      }
      // If no events seen, reuse original time_range (already set in currentBody)

      this.emitStatus("connecting");
      this.connect();
    }, delay);
  }

  /** Send an update (re-subscribe in place) with a new body. */
  update(body: LiveBody): void {
    this.currentBody = { ...body };
    if (this.ws && this.ws.readyState === 1 /* OPEN */) {
      this.ws.send(JSON.stringify({ type: "update", ...body }));
    }
  }

  /** Close the session permanently — no reconnect. */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      this.ws.close(1000, "session closed");
      this.ws = null;
    }
    this.emitStatus("closed");
  }

  private emitStatus(s: LiveStatus) {
    this.opts.onStatus(s);
  }
}
