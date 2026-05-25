import { create } from "zustand";
import { persist, createJSONStorage, type StateStorage } from "zustand/middleware";
import type { AuthUser } from "./auth";

export type SessionState = {
  endpoint: string;
  token: string;
  database: string;
  user: AuthUser | null;
  set: (p: Partial<Pick<SessionState, "endpoint" | "token" | "database" | "user">>) => void;
  reset: () => void;
  // Configured once we have a server endpoint. The token is optional — a
  // kyma engine with auth disabled (empty KYMA_AUTH_TOKENS) accepts any
  // bearer, so requiring a token here would trap unauthenticated dev on
  // the settings page after Save + Connect.
  isConfigured: () => boolean;
  // True when a session token is present (i.e. the user went through login).
  isAuthenticated: () => boolean;
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
      user: null,
      set: (p) => set(p),
      reset: () => set({ endpoint: "", token: "", database: "obs", user: null }),
      isConfigured: () => get().endpoint.length > 0,
      isAuthenticated: () => Boolean(get().token),
    }),
    {
      name: "kyma.session",
      storage: isTauri ? createJSONStorage(() => tauriStorage) : createJSONStorage(() => localStorage),
    },
  ),
);
