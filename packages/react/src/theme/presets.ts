/**
 * Built-in theme presets — values mirror web/src/styles/globals.css exactly.
 *
 * Light mode  → :root block
 * Dark mode   → .dark block
 *
 * fontSans / fontMono use system-font stacks so the SDK does NOT force
 * IBM Plex (or any other brand font) onto the host application.
 */
import type { PensieveTheme } from "./tokens";

export const pensieveLight: PensieveTheme = {
  // ── Colours — :root in globals.css ────────────────────────────────────
  background: "0 0% 100%",
  foreground: "214 32% 14%",
  surface: "210 30% 98%",
  card: "0 0% 100%",
  cardForeground: "214 32% 14%",
  popover: "0 0% 100%",
  popoverForeground: "214 32% 14%",
  primary: "183 74% 38%",
  primaryForeground: "0 0% 100%",
  secondary: "210 40% 96.1%",
  secondaryForeground: "214 40% 16%",
  muted: "210 40% 96.1%",
  mutedForeground: "215 16% 42%",
  accent: "185 46% 94%",
  accentForeground: "214 40% 16%",
  destructive: "0 84.2% 60.2%",
  destructiveForeground: "0 0% 100%",
  border: "214 28% 90%",
  borderStrong: "214 24% 82%",
  input: "214 28% 88%",
  ring: "199 74% 46%",
  brandFrom: "182 72% 42%",
  brandTo: "199 82% 48%",
  // ── Shape / typography ─────────────────────────────────────────────────
  radius: "0.625rem",
  fontSans: "ui-sans-serif, system-ui, sans-serif",
  fontMono: "ui-monospace, SFMono-Regular, Menlo, monospace",
};

export const pensieveDark: PensieveTheme = {
  // ── Colours — .dark in globals.css ────────────────────────────────────
  background: "213 26% 7%",
  foreground: "200 24% 96%",
  surface: "213 24% 10%",
  card: "213 22% 12%",
  cardForeground: "200 24% 96%",
  popover: "213 22% 13%",
  popoverForeground: "200 24% 96%",
  primary: "180 72% 45%",
  primaryForeground: "200 45% 8%",
  secondary: "213 20% 16%",
  secondaryForeground: "200 24% 96%",
  muted: "213 18% 17%",
  mutedForeground: "214 16% 64%",
  accent: "213 22% 16%",
  accentForeground: "200 30% 96%",
  destructive: "0 72% 51%",
  destructiveForeground: "0 0% 100%",
  border: "213 18% 17%",
  borderStrong: "213 16% 27%",
  input: "213 18% 17%",
  ring: "199 74% 53%",
  brandFrom: "180 80% 52%",
  brandTo: "199 85% 60%",
  // ── Shape / typography ─────────────────────────────────────────────────
  // globals.css only sets --radius in :root; dark inherits it unchanged.
  radius: "0.625rem",
  fontSans: "ui-sans-serif, system-ui, sans-serif",
  fontMono: "ui-monospace, SFMono-Regular, Menlo, monospace",
};
