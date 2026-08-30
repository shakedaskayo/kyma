import { create } from "zustand";

export type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "pensieve:theme";

function systemPrefersDark(): boolean {
  if (typeof window === "undefined") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

/**
 * Apply the theme by toggling the `.dark` class on `<html>`. shadcn-ui's
 * theme tokens (declared in globals.css) switch based on that class — every
 * component built on `bg-background`, `text-foreground`, `border-border`
 * etc. picks up the new palette without any additional wiring.
 */
function applyTheme(theme: Theme) {
  if (typeof document === "undefined") return;
  const isDark = theme === "dark" || (theme === "system" && systemPrefersDark());
  document.documentElement.classList.toggle("dark", isDark);
  document.documentElement.style.colorScheme = isDark ? "dark" : "light";
}

function readStoredTheme(): Theme {
  if (typeof window === "undefined") return "system";
  const v = window.localStorage?.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

interface ThemeStore {
  theme: Theme;
  /** Resolved theme — what's actually applied. "system" collapses to light/dark. */
  resolved: "light" | "dark";
  setTheme: (theme: Theme) => void;
}

export const useTheme = create<ThemeStore>((set) => {
  const initial = readStoredTheme();
  applyTheme(initial);
  return {
    theme: initial,
    resolved:
      initial === "dark" || (initial === "system" && systemPrefersDark())
        ? "dark"
        : "light",
    setTheme: (theme) => {
      window.localStorage?.setItem(STORAGE_KEY, theme);
      applyTheme(theme);
      set({
        theme,
        resolved:
          theme === "dark" || (theme === "system" && systemPrefersDark())
            ? "dark"
            : "light",
      });
    },
  };
});

// Keep `resolved` in sync when the OS preference changes and the user is on
// "system". Subscribers (graph canvas reading background colour, ReactFlow
// minimap, etc.) need that signal so they re-render with the new palette.
if (typeof window !== "undefined" && window.matchMedia) {
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  mql.addEventListener?.("change", () => {
    const { theme, setTheme } = useTheme.getState();
    if (theme === "system") setTheme("system");
  });
}
