import { create } from "zustand";
import { persist } from "zustand/middleware";

export type SessionState = {
  endpoint: string;
  token: string;
  database: string;
  set: (p: Partial<Pick<SessionState, "endpoint" | "token" | "database">>) => void;
  reset: () => void;
  isConfigured: () => boolean;
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
    { name: "kyma.session" },
  ),
);
