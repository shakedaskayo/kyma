/**
 * ThemePlayground — lets designers tweak every PensieveTheme token live.
 *
 * Each token is exposed as a Storybook control (colour strings use a text
 * control because Storybook's colour picker does not handle HSL triplets
 * like "213 26% 7%" natively). The story renders swatches of all --pensieve-*
 * colour tokens plus a live PensieveQueryEditor so the result is visible
 * immediately in context.
 *
 * The PensieveProvider decorator in preview.tsx is bypassed for this story
 * (we render our own provider so args feed directly into it). We do this
 * by wrapping in a plain div — the outer decorator adds an extra provider
 * but since CSS custom properties cascade, the inner one wins.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { PensieveProvider } from "./PensieveProvider";
import { PensieveQueryEditor } from "../query/PensieveQueryEditor";
import { pensieveDark } from "../theme/presets";
import type { PensieveTheme } from "../theme/tokens";

// ── Swatch grid ──────────────────────────────────────────────────────────────

const COLOR_TOKENS: Array<keyof PensieveTheme> = [
  "background", "surface", "foreground",
  "border", "borderStrong", "input", "ring",
  "primary", "primaryForeground",
  "secondary", "secondaryForeground",
  "muted", "mutedForeground",
  "accent", "accentForeground",
  "card", "cardForeground",
  "popover", "popoverForeground",
  "destructive", "destructiveForeground",
  "brandFrom", "brandTo",
];

function Swatch({ token, value }: { token: string; value: string }) {
  const cssVar = `--pensieve-${token.replace(/([A-Z])/g, (c) => `-${c.toLowerCase()}`)}`;
  return (
    <div className="pv-flex pv-flex-col pv-gap-1">
      <div
        className="pv-h-10 pv-w-full pv-rounded-md pv-border pv-border-border"
        style={{ background: `hsl(${value})` }}
      />
      <p className="pv-text-2xs pv-font-mono pv-text-muted-foreground">{cssVar}</p>
      <p className="pv-text-2xs pv-text-muted-foreground">{value}</p>
    </div>
  );
}

// ── Story component ──────────────────────────────────────────────────────────

type ThemePlaygroundArgs = PensieveTheme;

function ThemePlaygroundStory(args: ThemePlaygroundArgs) {
  const theme: Partial<PensieveTheme> = { ...args };

  return (
    <PensieveProvider endpoint="https://pensieve.demo" auth={{ token: "storybook" }} theme={theme}>
      <div className="pv-flex pv-h-screen pv-flex-col pv-gap-0 pv-overflow-hidden pv-bg-background pv-text-foreground">
        {/* Token swatches */}
        <div className="pv-shrink-0 pv-overflow-x-auto pv-border-b pv-p-4">
          <h2 className="pv-mb-3 pv-text-sm pv-font-semibold pv-text-foreground">
            Color tokens — all --pensieve-* variables
          </h2>
          <div className="pv-grid pv-grid-cols-6 pv-gap-3" style={{ minWidth: 720 }}>
            {COLOR_TOKENS.map((token) => (
              <Swatch
                key={token}
                token={token}
                value={String(args[token])}
              />
            ))}
          </div>
          <div className="pv-mt-4 pv-flex pv-items-center pv-gap-6 pv-text-xs pv-text-muted-foreground">
            <span>
              radius: <code className="pv-font-mono">{args.radius}</code>
            </span>
            <span>
              brand gradient:{" "}
              <span
                className="pv-inline-block pv-h-4 pv-w-24 pv-rounded"
                style={{
                  background: `linear-gradient(90deg, hsl(${args.brandFrom}), hsl(${args.brandTo}))`,
                }}
              />
            </span>
          </div>
        </div>

        {/* Live query editor */}
        <div className="pv-min-h-0 pv-flex-1">
          <PensieveQueryEditor
            defaultQuery="events | take 10"
            style={{ height: "100%" }}
          />
        </div>
      </div>
    </PensieveProvider>
  );
}

// ── Meta ─────────────────────────────────────────────────────────────────────

function tokenControl(description: string) {
  return { control: "text" as const, description };
}

const meta: Meta<ThemePlaygroundArgs> = {
  title: "Theme/Playground",
  render: (args) => <ThemePlaygroundStory {...args} />,
  argTypes: {
    background:           tokenControl("Background HSL triplet"),
    surface:              tokenControl("Surface HSL triplet"),
    foreground:           tokenControl("Foreground HSL triplet"),
    border:               tokenControl("Border HSL triplet"),
    borderStrong:         tokenControl("Strong border HSL triplet"),
    input:                tokenControl("Input background HSL triplet"),
    ring:                 tokenControl("Focus ring HSL triplet"),
    primary:              tokenControl("Primary colour HSL triplet"),
    primaryForeground:    tokenControl("Primary foreground HSL triplet"),
    secondary:            tokenControl("Secondary colour HSL triplet"),
    secondaryForeground:  tokenControl("Secondary foreground HSL triplet"),
    muted:                tokenControl("Muted colour HSL triplet"),
    mutedForeground:      tokenControl("Muted foreground HSL triplet"),
    accent:               tokenControl("Accent colour HSL triplet"),
    accentForeground:     tokenControl("Accent foreground HSL triplet"),
    card:                 tokenControl("Card background HSL triplet"),
    cardForeground:       tokenControl("Card foreground HSL triplet"),
    popover:              tokenControl("Popover background HSL triplet"),
    popoverForeground:    tokenControl("Popover foreground HSL triplet"),
    destructive:          tokenControl("Destructive colour HSL triplet"),
    destructiveForeground: tokenControl("Destructive foreground HSL triplet"),
    brandFrom:            tokenControl("Brand gradient start HSL triplet"),
    brandTo:              tokenControl("Brand gradient end HSL triplet"),
    radius:               tokenControl("Border radius CSS length (e.g. 0.625rem)"),
    fontSans:             tokenControl("Sans-serif font stack"),
    fontMono:             tokenControl("Monospace font stack"),
  },
  args: { ...pensieveDark },
};

export default meta;
type Story = StoryObj<ThemePlaygroundArgs>;

/** Dark preset — default Pensieve dark theme with all tokens editable. */
export const Dark: Story = {
  args: { ...pensieveDark },
};

/** Light preset — all tokens editable. */
export const Light: Story = {
  args: {
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
    radius: "0.625rem",
    fontSans: "ui-sans-serif, system-ui, sans-serif",
    fontMono: "ui-monospace, SFMono-Regular, Menlo, monospace",
  },
};
