import { create } from "zustand";

type State = { online: boolean; setOnline: (v: boolean) => void; start: (endpoint: string) => () => void };

export const useHealth = create<State>((set) => ({
  online: true,
  setOnline: (online) => set({ online }),
  start: (endpoint) => {
    const tick = async () => {
      try {
        const res = await fetch(`${endpoint.replace(/\/$/, "")}/health`, { cache: "no-store" });
        set({ online: res.ok });
      } catch {
        set({ online: false });
      }
    };
    tick();
    const id = window.setInterval(tick, 30_000);
    return () => { clearInterval(id); };
  },
}));
