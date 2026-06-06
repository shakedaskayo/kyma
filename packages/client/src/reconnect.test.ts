import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useHealth } from "./reconnect";

function healthResponse(version: string | null, ok = true): Response {
  const body = version === null ? { status: "ok" } : { status: "ok", version };
  return {
    ok,
    clone() {
      return this;
    },
    async json() {
      return body;
    },
  } as unknown as Response;
}

describe("useHealth server-version tracking", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useHealth.setState({ online: true, version: null, updated: false });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("flags `updated` when /health starts reporting a new version mid-session", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(healthResponse("0.0.1"))
      .mockResolvedValueOnce(healthResponse("0.0.1"))
      .mockResolvedValue(healthResponse("0.0.2"));
    vi.stubGlobal("fetch", fetchMock);

    const stop = useHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0); // initial tick
    expect(useHealth.getState().version).toBe("0.0.1");
    expect(useHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // same version → no flag
    expect(useHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // server restarted on 0.0.2
    expect(useHealth.getState().version).toBe("0.0.2");
    expect(useHealth.getState().updated).toBe(true);
    stop();
  });

  it("keeps the last seen version through failed probes and stays un-flagged", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(healthResponse("0.0.1"))
      .mockRejectedValueOnce(new Error("connection refused")) // server restarting
      .mockResolvedValue(healthResponse("0.0.1"));
    vi.stubGlobal("fetch", fetchMock);

    const stop = useHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0);
    expect(useHealth.getState().version).toBe("0.0.1");

    await vi.advanceTimersByTimeAsync(30_000); // offline blip
    expect(useHealth.getState().online).toBe(false);
    expect(useHealth.getState().version).toBe("0.0.1");
    expect(useHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // back, same version
    expect(useHealth.getState().online).toBe(true);
    expect(useHealth.getState().updated).toBe(false);
    stop();
  });

  it("ignores health bodies without a version field", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(healthResponse("0.0.1"))
      .mockResolvedValue(healthResponse(null));
    vi.stubGlobal("fetch", fetchMock);

    const stop = useHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(useHealth.getState().version).toBe("0.0.1"); // sticky
    expect(useHealth.getState().updated).toBe(false);
    stop();
  });
});
