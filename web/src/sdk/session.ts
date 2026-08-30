import { create } from "zustand";
import { persist, createJSONStorage, type StateStorage } from "zustand/middleware";
import type { AuthUser } from "./auth";

export type AuthProvider = "password" | "supabase";

export type SessionState = {
  endpoint: string;
  /** The short-lived access token sent as the API bearer. */
  token: string;
  /** The long-lived refresh token, exchanged for a new pair on expiry. */
  refreshToken: string;
  /** ISO timestamp when the access token expires (informational). */
  accessExpiresAt: string;
  /** Which login flow produced this session (drives 401 recovery). */
  provider: AuthProvider;
  /** Supabase project URL + anon key (supabase provider only) — persisted so
   *  the client can be recreated after a page reload. */
  supabaseUrl: string;
  supabaseAnonKey: string;
  /** The active concrete database for single-database pages (Data Sources,
   *  Dashboards, Agent). Empty until resolved from the catalog on first load.
   *  Never the "*" all-databases sentinel — those pages need a real database. */
  database: string;
  /** Scope for the cross-database-capable Query Editor: either "*" (unify across
   *  every database — the default) or a concrete database name to filter to.
   *  Sent verbatim as the `x-database` header; "*" makes the engine span all DBs. */
  queryScope: string;
  user: AuthUser | null;
  set: (
    p: Partial<
      Pick<
        SessionState,
        | "endpoint"
        | "token"
        | "refreshToken"
        | "accessExpiresAt"
        | "provider"
        | "supabaseUrl"
        | "supabaseAnonKey"
        | "database"
        | "queryScope"
        | "user"
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
    const s = await Store.load("pensieve.store");
    return (await s.get<string>(name)) ?? null;
  },
  async setItem(name, val) {
    const { Store } = await import("@tauri-apps/plugin-store");
    const s = await Store.load("pensieve.store");
    await s.set(name, val);
    await s.save();
  },
  async removeItem(name) {
    const { Store } = await import("@tauri-apps/plugin-store");
    const s = await Store.load("pensieve.store");
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
      provider: "password" as AuthProvider,
      supabaseUrl: "",
      supabaseAnonKey: "",
      database: "",
      queryScope: "*",
      user: null,
      set: (p) => set(p),
      reset: () =>
        set({
          endpoint: "",
          token: "",
          refreshToken: "",
          accessExpiresAt: "",
          provider: "password",
          supabaseUrl: "",
          supabaseAnonKey: "",
          database: "",
          queryScope: "*",
          user: null,
        }),
      isConfigured: () => get().endpoint.length > 0,
      isAuthenticated: () => Boolean(get().token),
    }),
    {
      name: "pensieve.session",
      storage: isTauri ? createJSONStorage(() => tauriStorage) : createJSONStorage(() => localStorage),
    },
  ),
);
