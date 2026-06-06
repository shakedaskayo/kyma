import type { Config } from "tailwindcss";
import animate from "tailwindcss-animate";

// Embeddable build: every utility is prefixed (`ky-flex`) and every rule is
// scoped under `.kyma-root` so host-app CSS and Kyma CSS cannot collide in
// either direction. Tokens are `--kyma-*`, set inline by <KymaProvider>.
export default {
  prefix: "ky-",
  important: ".kyma-root",
  corePlugins: { preflight: false },
  content: ["./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        border: "hsl(var(--kyma-border))",
        "border-strong": "hsl(var(--kyma-border-strong))",
        input: "hsl(var(--kyma-input))",
        ring: "hsl(var(--kyma-ring))",
        background: "hsl(var(--kyma-background))",
        surface: "hsl(var(--kyma-surface))",
        foreground: "hsl(var(--kyma-foreground))",
        primary:   { DEFAULT: "hsl(var(--kyma-primary))",   foreground: "hsl(var(--kyma-primary-foreground))" },
        secondary: { DEFAULT: "hsl(var(--kyma-secondary))", foreground: "hsl(var(--kyma-secondary-foreground))" },
        destructive: { DEFAULT: "hsl(var(--kyma-destructive))", foreground: "hsl(var(--kyma-destructive-foreground))" },
        muted:     { DEFAULT: "hsl(var(--kyma-muted))",     foreground: "hsl(var(--kyma-muted-foreground))" },
        accent:    { DEFAULT: "hsl(var(--kyma-accent))",    foreground: "hsl(var(--kyma-accent-foreground))" },
        card:      { DEFAULT: "hsl(var(--kyma-card))",      foreground: "hsl(var(--kyma-card-foreground))" },
        popover:   { DEFAULT: "hsl(var(--kyma-popover))",   foreground: "hsl(var(--kyma-popover-foreground))" },
        brand:     { from: "hsl(var(--kyma-brand-from))",   to: "hsl(var(--kyma-brand-to))" },
      },
      borderRadius: {
        xl: "calc(var(--kyma-radius) + 4px)",
        lg: "var(--kyma-radius)",
        md: "calc(var(--kyma-radius) - 2px)",
        sm: "calc(var(--kyma-radius) - 4px)",
      },
      fontFamily: {
        sans: ["var(--kyma-font-sans)"],
        mono: ["var(--kyma-font-mono)"],
      },
      fontSize: {
        "2xs": ["0.6875rem", { lineHeight: "1.4" }],            // 11px
        xs:    ["0.75rem",   { lineHeight: "1.5" }],            // 12px
        sm:    ["0.8125rem", { lineHeight: "1.5" }],            // 13px
        base:  ["0.875rem",  { lineHeight: "1.55" }],           // 14px
        lg:    ["1rem",      { lineHeight: "1.5" }],            // 16px
        xl:    ["1.125rem",  { lineHeight: "1.45" }],           // 18px
        "2xl": ["1.375rem",  { lineHeight: "1.25", letterSpacing: "-0.01em" }],  // 22px
        "3xl": ["1.75rem",   { lineHeight: "1.2",  letterSpacing: "-0.02em" }],  // 28px
        "4xl": ["2.25rem",   { lineHeight: "1.15", letterSpacing: "-0.02em" }],  // 36px
      },
      boxShadow: {
        "elev-1": "0 1px 2px hsl(213 40% 3% / 0.40)",
        "elev-2": "0 2px 6px hsl(213 40% 3% / 0.30), 0 4px 16px hsl(213 40% 3% / 0.22)",
        "elev-3": "0 10px 32px hsl(213 50% 2% / 0.50), 0 2px 8px hsl(213 50% 2% / 0.35)",
        glow: "0 0 0 1px hsl(var(--kyma-ring) / 0.5), 0 0 18px hsl(var(--kyma-ring) / 0.35)",
      },
      backgroundImage: {
        "brand-gradient": "linear-gradient(100deg, hsl(var(--kyma-brand-from)), hsl(var(--kyma-brand-to)))",
      },
      keyframes: {
        shimmer: {
          "100%": { transform: "translateX(100%)" },
        },
        "fade-up": {
          "0%": { opacity: "0", transform: "translateY(6px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        "fade-in": {
          "0%": { opacity: "0" },
          "100%": { opacity: "1" },
        },
      },
      animation: {
        shimmer: "shimmer 1.6s infinite",
        "fade-up": "fade-up 0.25s ease-out both",
        "fade-in": "fade-in 0.2s ease-out both",
      },
    },
  },
  plugins: [animate],
} satisfies Config;
