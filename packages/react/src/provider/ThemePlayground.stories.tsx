/**
 * ThemePlayground — lets designers tweak every KymaTheme token live.
 *
 * Each token is exposed as a Storybook control (colour strings use a text
 * control because Storybook's colour picker does not handle HSL triplets
 * like "213 26% 7%" natively). The story renders swatches of all --kyma-*
 * colour tokens plus a live KymaQueryEditor so the result is visible
 * immediately in context.
 *
 * The KymaProvider decorator in preview.tsx is bypassed for this story
 * (we render our own provider so args feed directly into it). We do this
 * by wrapping in a plain div — the outer decorator adds an extra provider
 * but since CSS custom properties cascade, the inner one wins.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { KymaProvider } from "./KymaProvider";
import { KymaQueryEditor } from "../query/KymaQueryEditor";
import { kymaDark } from "../theme/presets";
import type { KymaTheme } from "../theme/tokens";

// ── Swatch grid ──────────────────────────────────────────────────────────────

const COLOR_TOKENS: Array<keyof KymaTheme> = [
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
  const cssVar = `--kyma-${token.replace(/([A-Z])/g, (c) => `-${c.toLowerCase()}`)}`;
  return (
    <div className="ky-flex ky-flex-col ky-gap-1">
      <div
        className="ky-h-10 ky-w-full ky-rounded-md ky-border ky-border-border"
        style={{ background: `hsl(${value})` }}
      />
      <p className="ky-text-2xs ky-font-mono ky-text-muted-foreground">{cssVar}</p>
      <p className="ky-text-2xs ky-text-muted-foreground">{value}</p>
    </div>
  );
}

// ── Story component ──────────────────────────────────────────────────────────

type ThemePlaygroundArgs = KymaTheme;

function ThemePlaygroundStory(args: ThemePlaygroundArgs) {
  const theme: Partial<KymaTheme> = { ...args };

  return (
    <KymaProvider endpoint="https://kyma.demo" auth={{ token: "storybook" }} theme={theme}>
      <div className="ky-flex ky-h-screen ky-flex-col ky-gap-0 ky-overflow-hidden ky-bg-background ky-text-foreground">
        {/* Token swatches */}
        <div className="ky-shrink-0 ky-overflow-x-auto ky-border-b ky-p-4">
          <h2 className="ky-mb-3 ky-text-sm ky-font-semibold ky-text-foreground">
            Color tokens — all --kyma-* variables
          </h2>
          <div className="ky-grid ky-grid-cols-6 ky-gap-3" style={{ minWidth: 720 }}>
            {COLOR_TOKENS.map((token) => (
              <Swatch
                key={token}
                token={token}
                value={String(args[token])}
              />
            ))}
          </div>
          <div className="ky-mt-4 ky-flex ky-items-center ky-gap-6 ky-text-xs ky-text-muted-foreground">
            <span>
              radius: <code className="ky-font-mono">{args.radius}</code>
            </span>
            <span>
              brand gradient:{" "}
              <span
                className="ky-inline-block ky-h-4 ky-w-24 ky-rounded"
                style={{
                  background: `linear-gradient(90deg, hsl(${args.brandFrom}), hsl(${args.brandTo}))`,
                }}
              />
            </span>
          </div>
        </div>

        {/* Live query editor */}
        <div className="ky-min-h-0 ky-flex-1">
          <KymaQueryEditor
            defaultQuery="events | take 10"
            style={{ height: "100%" }}
          />
        </div>
      </div>
    </KymaProvider>
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
  args: { ...kymaDark },
};

export default meta;
type Story = StoryObj<ThemePlaygroundArgs>;

/** Dark preset — default Kyma dark theme with all tokens editable. */
export const Dark: Story = {
  args: { ...kymaDark },
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
