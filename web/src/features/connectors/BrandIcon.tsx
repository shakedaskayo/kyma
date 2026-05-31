import type { ComponentType } from "react";
import {
  SiGithub,
  SiGitlab,
  SiBitbucket,
  SiNotion,
  SiConfluence,
  SiGoogledrive,
  SiJira,
  SiLinear,
  SiAsana,
  SiPostgresql,
  SiPrometheus,
} from "@icons-pack/react-simple-icons";
import { Database, MessageSquare, Plug } from "lucide-react";
import { useTheme } from "@/lib/theme";

// Brands whose simple-icons "default" color is too dark to read on a dark
// background (e.g. GitHub's #181717, Notion's near-black). In dark mode we
// swap them for the brand's official light-mode mark colour so the icon
// stays recognisable but readable. Light mode keeps the canonical brand
// colour from simple-icons. Brands not listed here use `color="default"` in
// both themes — they're already mid-tone enough to read either way.
const DARK_BRAND_COLOR: Record<string, string> = {
  github: "#f0f6fc",
  notion: "#ffffff",
  bitbucket: "#2684ff",
};

// Brand slug (from the engine catalog's `brand` field) → an icon component.
// `any` props: simple-icons and lucide have different (but compatible-at-runtime)
// prop shapes; we call each with the props it accepts in `BrandIcon` below.
// Adding a connector in the engine needs no change here unless it introduces a
// brand not yet mapped — in which case it falls back to a generic glyph.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const BRAND: Record<string, ComponentType<any>> = {
  github: SiGithub,
  gitlab: SiGitlab,
  bitbucket: SiBitbucket,
  notion: SiNotion,
  confluence: SiConfluence,
  googledrive: SiGoogledrive,
  jira: SiJira,
  linear: SiLinear,
  asana: SiAsana,
  postgresql: SiPostgresql,
  prometheus: SiPrometheus,
};

// Vendors without a simple-icons mark (Slack and Amazon had their icons removed
// from simple-icons) map to a sensible lucide glyph instead.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const FALLBACK: Record<string, ComponentType<any>> = {
  slack: MessageSquare,
  amazons3: Database,
};

/**
 * Renders a vendor's brand mark from its catalog `brand` slug. Brand-color by
 * default (the recognisable look); pass `monochrome` to inherit currentColor.
 */
export function BrandIcon({
  brand,
  size = 18,
  monochrome = false,
  className,
}: {
  brand: string;
  size?: number;
  monochrome?: boolean;
  className?: string;
}) {
  const isDark = useTheme((s) => s.resolved === "dark");
  const Si = BRAND[brand];
  if (Si) {
    const color = monochrome
      ? "currentColor"
      : isDark && DARK_BRAND_COLOR[brand]
        ? DARK_BRAND_COLOR[brand]
        : "default";
    return <Si size={size} color={color} className={className} />;
  }
  const Fb = FALLBACK[brand] ?? Plug;
  return <Fb size={size} className={className} />;
}
