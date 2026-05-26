import { create } from "zustand";
import { persist, createJSONStorage, type StateStorage } from "zustand/middleware";
import type { AuthUser } from "./auth";

export type SessionState = {
  endpoint: string;
  /** The short-lived access token sent as the API bearer. */
  token: string;
  /** The long-lived refresh token, exchanged for a new pair on expiry. */
  refreshToken: string;
  /** ISO timestamp when the access token expires (informational). */
  accessExpiresAt: string;
  database: string;
  user: AuthUser | null;
  set: (
    p: Partial<
      Pick<
        SessionState,
        "endpoint" | "token" | "refreshToken" | "accessExpiresAt" | "database" | "user"
      >
    >,
  ) => void;
  reset: () => void;
  // Configured once we have a server endpoint (used by the Settings form).
  isConfigured: () => boolean;
  // Authenticated once we hold an access token. Guests are not allowed — the
  // only ways to obtain a token are logging in or pasting a static API token.
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
      refreshToken: "",
      accessExpiresAt: "",
      database: "obs",
      user: null,
      set: (p) => set(p),
      reset: () =>
        set({
          endpoint: "",
          token: "",
          refreshToken: "",
          accessExpiresAt: "",
          database: "obs",
          user: null,
        }),
      isConfigured: () => get().endpoint.length > 0,
      isAuthenticated: () => Boolean(get().token),
    }),
    {
      name: "kyma.session",
      storage: isTauri ? createJSONStorage(() => tauriStorage) : createJSONStorage(() => localStorage),
    },
  ),
);
