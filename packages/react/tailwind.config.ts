import type { Config } from "tailwindcss";
import animate from "tailwindcss-animate";

// Embeddable build: every utility is prefixed (`pv-flex`) and every rule is
// scoped under `.pensieve-root` so host-app CSS and Pensieve CSS cannot collide in
// either direction. Tokens are `--pensieve-*`, set inline by <PensieveProvider>.
export default {
  prefix: "pv-",
  important: ".pensieve-root",
  corePlugins: { preflight: false },
  content: ["./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        border: "hsl(var(--pensieve-border))",
        "border-strong": "hsl(var(--pensieve-border-strong))",
        input: "hsl(var(--pensieve-input))",
        ring: "hsl(var(--pensieve-ring))",
        background: "hsl(var(--pensieve-background))",
        surface: "hsl(var(--pensieve-surface))",
        foreground: "hsl(var(--pensieve-foreground))",
        primary:   { DEFAULT: "hsl(var(--pensieve-primary))",   foreground: "hsl(var(--pensieve-primary-foreground))" },
        secondary: { DEFAULT: "hsl(var(--pensieve-secondary))", foreground: "hsl(var(--pensieve-secondary-foreground))" },
        destructive: { DEFAULT: "hsl(var(--pensieve-destructive))", foreground: "hsl(var(--pensieve-destructive-foreground))" },
        muted:     { DEFAULT: "hsl(var(--pensieve-muted))",     foreground: "hsl(var(--pensieve-muted-foreground))" },
        accent:    { DEFAULT: "hsl(var(--pensieve-accent))",    foreground: "hsl(var(--pensieve-accent-foreground))" },
        card:      { DEFAULT: "hsl(var(--pensieve-card))",      foreground: "hsl(var(--pensieve-card-foreground))" },
        popover:   { DEFAULT: "hsl(var(--pensieve-popover))",   foreground: "hsl(var(--pensieve-popover-foreground))" },
        brand:     { from: "hsl(var(--pensieve-brand-from))",   to: "hsl(var(--pensieve-brand-to))" },
      },
      borderRadius: {
        xl: "calc(var(--pensieve-radius) + 4px)",
        lg: "var(--pensieve-radius)",
        md: "calc(var(--pensieve-radius) - 2px)",
        sm: "calc(var(--pensieve-radius) - 4px)",
      },
      fontFamily: {
        sans: ["var(--pensieve-font-sans)"],
        mono: ["var(--pensieve-font-mono)"],
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
        glow: "0 0 0 1px hsl(var(--pensieve-ring) / 0.5), 0 0 18px hsl(var(--pensieve-ring) / 0.35)",
      },
      backgroundImage: {
        "brand-gradient": "linear-gradient(100deg, hsl(var(--pensieve-brand-from)), hsl(var(--pensieve-brand-to)))",
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
