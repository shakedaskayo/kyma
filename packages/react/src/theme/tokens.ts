/**
 * KymaTheme — flat design-token record.
 *
 * All colour values are HSL triplet strings without the `hsl()` wrapper
 * (e.g. "213 26% 7%") so they compose cleanly with Tailwind's
 * `hsl(var(--kyma-*))` pattern.
 *
 * `radius` is a CSS length (e.g. "0.625rem").
 * `fontSans` / `fontMono` are CSS font-stack strings.
 */
export interface KymaTheme {
  // ── Colour tokens ────────────────────────────────────────────────────────
  background: string;
  surface: string;
  foreground: string;
  border: string;
  borderStrong: string;
  input: string;
  ring: string;
  primary: string;
  primaryForeground: string;
  secondary: string;
  secondaryForeground: string;
  destructive: string;
  destructiveForeground: string;
  muted: string;
  mutedForeground: string;
  accent: string;
  accentForeground: string;
  card: string;
  cardForeground: string;
  popover: string;
  popoverForeground: string;
  brandFrom: string;
  brandTo: string;
  // ── Shape / typography ───────────────────────────────────────────────────
  radius: string;
  fontSans: string;
  fontMono: string;
}

/**
 * Convert camelCase token names to --kyma-kebab-case CSS custom properties.
 * Only keys present in the partial theme are emitted — callers can spread a
 * partial override without clobbering unrelated host vars.
 */
export function themeToCssVars(theme: Partial<KymaTheme>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(theme) as [string, string][]) {
    if (value == null) continue;
    // camelCase → kebab-case, then prefix with --kyma-
    const kebab = key.replace(/([A-Z])/g, (ch) => `-${ch.toLowerCase()}`);
    out[`--kyma-${kebab}`] = value;
  }
  return out;
}
