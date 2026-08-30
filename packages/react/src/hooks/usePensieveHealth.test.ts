import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useKymaHealth } from "./useKymaHealth";

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

describe("useKymaHealth server-version tracking", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useKymaHealth.setState({ online: true, version: null, updated: false });
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

    const stop = useKymaHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0); // initial tick
    expect(useKymaHealth.getState().version).toBe("0.0.1");
    expect(useKymaHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // same version → no flag
    expect(useKymaHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // server restarted on 0.0.2
    expect(useKymaHealth.getState().version).toBe("0.0.2");
    expect(useKymaHealth.getState().updated).toBe(true);
    stop();
  });

  it("keeps the last seen version through failed probes and stays un-flagged", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(healthResponse("0.0.1"))
      .mockRejectedValueOnce(new Error("connection refused")) // server restarting
      .mockResolvedValue(healthResponse("0.0.1"));
    vi.stubGlobal("fetch", fetchMock);

    const stop = useKymaHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0);
    expect(useKymaHealth.getState().version).toBe("0.0.1");

    await vi.advanceTimersByTimeAsync(30_000); // offline blip
    expect(useKymaHealth.getState().online).toBe(false);
    expect(useKymaHealth.getState().version).toBe("0.0.1");
    expect(useKymaHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // back, same version
    expect(useKymaHealth.getState().online).toBe(true);
    expect(useKymaHealth.getState().updated).toBe(false);
    stop();
  });

  it("ignores health bodies without a version field", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(healthResponse("0.0.1"))
      .mockResolvedValue(healthResponse(null));
    vi.stubGlobal("fetch", fetchMock);

    const stop = useKymaHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(useKymaHealth.getState().version).toBe("0.0.1"); // sticky
    expect(useKymaHealth.getState().updated).toBe(false);
    stop();
  });
});
