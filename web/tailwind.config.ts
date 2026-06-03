import type { Config } from "tailwindcss";
import animate from "tailwindcss-animate";

export default {
  darkMode: ["class"],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    container: { center: true, padding: "1.5rem" },
    extend: {
      colors: {
        border: "hsl(var(--border))",
        "border-strong": "hsl(var(--border-strong))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        surface: "hsl(var(--surface))",
        foreground: "hsl(var(--foreground))",
        primary:   { DEFAULT: "hsl(var(--primary))",   foreground: "hsl(var(--primary-foreground))" },
        secondary: { DEFAULT: "hsl(var(--secondary))", foreground: "hsl(var(--secondary-foreground))" },
        destructive: { DEFAULT: "hsl(var(--destructive))", foreground: "hsl(var(--destructive-foreground))" },
        muted:     { DEFAULT: "hsl(var(--muted))",     foreground: "hsl(var(--muted-foreground))" },
        accent:    { DEFAULT: "hsl(var(--accent))",    foreground: "hsl(var(--accent-foreground))" },
        card:      { DEFAULT: "hsl(var(--card))",      foreground: "hsl(var(--card-foreground))" },
        popover:   { DEFAULT: "hsl(var(--popover))",   foreground: "hsl(var(--popover-foreground))" },
        brand:     { from: "hsl(var(--brand-from))",   to: "hsl(var(--brand-to))" },
      },
      borderRadius: {
        xl: "calc(var(--radius) + 4px)",
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
      },
      fontFamily: {
        sans: ["IBM Plex Sans","ui-sans-serif","system-ui","-apple-system","Segoe UI","Roboto","sans-serif"],
        mono: ["JetBrains Mono","ui-monospace","SFMono-Regular","Menlo","monospace"],
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
        glow: "0 0 0 1px hsl(var(--ring) / 0.5), 0 0 18px hsl(var(--ring) / 0.35)",
      },
      backgroundImage: {
        "brand-gradient": "linear-gradient(100deg, hsl(var(--brand-from)), hsl(var(--brand-to)))",
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
