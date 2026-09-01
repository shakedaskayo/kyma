import { create } from "zustand";

type HealthState = {
  online: boolean;
  /** Server version reported by /health; null until the first successful probe. */
  version: string | null;
  /** Flips true when /health starts reporting a different version mid-session —
   * the server was updated/restarted under us, so the assets this tab loaded
   * (the UI is embedded in the server binary) are stale until a reload. */
  updated: boolean;
  setOnline: (v: boolean) => void;
  start: (endpoint: string) => () => void;
};

/**
 * Zustand store that polls /health every 30 seconds and tracks:
 * - `online`: whether the last probe succeeded
 * - `version`: server version reported by /health
 * - `updated`: whether the version changed mid-session (server restarted)
 *
 * Call `start(endpoint)` to begin polling; the returned callback stops it.
 */
export const usePensieveHealth = create<HealthState>((set, get) => ({
  online: true,
  version: null,
  updated: false,
  setOnline: (online) => set({ online }),
  start: (endpoint) => {
    const tick = async () => {
      try {
        const res = await fetch(`${endpoint.replace(/\/$/, "")}/health`, { cache: "no-store" });
        let version: string | null = null;
        try {
          const body: unknown = await res.clone().json();
          const v = (body as { version?: unknown } | null)?.version;
          if (typeof v === "string") version = v;
        } catch {
          /* non-JSON health body — version stays unknown */
        }
        const prev = get().version;
        set({
          online: res.ok,
          version: version ?? prev,
          updated: get().updated || (prev !== null && version !== null && version !== prev),
        });
      } catch {
        set({ online: false });
      }
    };
    tick();
    const id = globalThis.setInterval(tick, 30_000);
    return () => { clearInterval(id); };
  },
}));
