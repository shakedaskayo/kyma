import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { usePensieveHealth } from "./usePensieveHealth";

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

describe("usePensieveHealth server-version tracking", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    usePensieveHealth.setState({ online: true, version: null, updated: false });
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

    const stop = usePensieveHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0); // initial tick
    expect(usePensieveHealth.getState().version).toBe("0.0.1");
    expect(usePensieveHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // same version → no flag
    expect(usePensieveHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // server restarted on 0.0.2
    expect(usePensieveHealth.getState().version).toBe("0.0.2");
    expect(usePensieveHealth.getState().updated).toBe(true);
    stop();
  });

  it("keeps the last seen version through failed probes and stays un-flagged", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(healthResponse("0.0.1"))
      .mockRejectedValueOnce(new Error("connection refused")) // server restarting
      .mockResolvedValue(healthResponse("0.0.1"));
    vi.stubGlobal("fetch", fetchMock);

    const stop = usePensieveHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0);
    expect(usePensieveHealth.getState().version).toBe("0.0.1");

    await vi.advanceTimersByTimeAsync(30_000); // offline blip
    expect(usePensieveHealth.getState().online).toBe(false);
    expect(usePensieveHealth.getState().version).toBe("0.0.1");
    expect(usePensieveHealth.getState().updated).toBe(false);

    await vi.advanceTimersByTimeAsync(30_000); // back, same version
    expect(usePensieveHealth.getState().online).toBe(true);
    expect(usePensieveHealth.getState().updated).toBe(false);
    stop();
  });

  it("ignores health bodies without a version field", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(healthResponse("0.0.1"))
      .mockResolvedValue(healthResponse(null));
    vi.stubGlobal("fetch", fetchMock);

    const stop = usePensieveHealth.getState().start("http://localhost:8080");
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(usePensieveHealth.getState().version).toBe("0.0.1"); // sticky
    expect(usePensieveHealth.getState().updated).toBe(false);
    stop();
  });
});
