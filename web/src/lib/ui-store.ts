import { create } from "zustand";
import { persist } from "zustand/middleware";

/**
 * Small global UI store: sidebar collapse (persisted) and the app-wide command
 * palette open state (transient). Kept separate from feature stores so the
 * shell chrome has one tiny source of truth.
 */
type UIStore = {
  sidebarCollapsed: boolean;
  toggleSidebar(): void;
  setSidebarCollapsed(v: boolean): void;
  /** Command palette visibility — driven by ⌘K and the top-bar search button. */
  cmdOpen: boolean;
  setCmdOpen(v: boolean): void;
  toggleCmd(): void;
};

export const useUI = create<UIStore>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
      setSidebarCollapsed: (v) => set({ sidebarCollapsed: v }),
      cmdOpen: false,
      setCmdOpen: (v) => set({ cmdOpen: v }),
      toggleCmd: () => set((s) => ({ cmdOpen: !s.cmdOpen })),
    }),
    {
      name: "pensieve:ui",
      // Only the durable preference is persisted; palette state stays transient.
      partialize: (s) => ({ sidebarCollapsed: s.sidebarCollapsed }),
    },
  ),
);
