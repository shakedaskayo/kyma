import { create } from "zustand";
import { persist, createJSONStorage, type StateStorage } from "zustand/middleware";

export type SessionState = {
  endpoint: string;
  token: string;
  database: string;
  set: (p: Partial<Pick<SessionState, "endpoint" | "token" | "database">>) => void;
  reset: () => void;
  isConfigured: () => boolean;
};

const isTauri = typeof window !== "undefined" && "__TAURI__" in window;

const tauriStorage: StateStorage = {
  async getItem(name) {
    const { Store } = await import("@tauri-apps/plugin-store");
    const s = await Store.load("kyma.store");
    return (await s.get<string>(name)) ?? null;
  },
  async setItem(name, val) {
    const { Store } = await import("@tauri-apps/plugin-store");
    const s = await Store.load("kyma.store");
    await s.set(name, val);
    await s.save();
  },
  async removeItem(name) {
    const { Store } = await import("@tauri-apps/plugin-store");
    const s = await Store.load("kyma.store");
    await s.delete(name);
    await s.save();
  },
};

export const useSession = create<SessionState>()(
  persist(
    (set, get) => ({
      endpoint: "",
      token: "",
      database: "obs",
      set: (p) => set(p),
      reset: () => set({ endpoint: "", token: "", database: "obs" }),
      isConfigured: () => {
        const { endpoint, token } = get();
        return endpoint.length > 0 && token.length > 0;
      },
    }),
    {
      name: "kyma.session",
      storage: isTauri ? createJSONStorage(() => tauriStorage) : createJSONStorage(() => localStorage),
    },
  ),
);
