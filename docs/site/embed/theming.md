---
title: Theming — @pensieve-ai/react
description: Design tokens, built-in presets, partial overrides, and host-CSS inherit mode for embedded Pensieve components.
---

# Theming

`PensieveProvider` accepts a `theme` prop that controls the visual appearance of
all embedded components. The SDK expresses every visual primitive as a flat
record of 26 design tokens (`PensieveTheme`), which are emitted as CSS custom
properties (`--pensieve-*`) on the provider's root element.

## Token table

All colour values are HSL triplet strings **without** the `hsl()` wrapper
(e.g. `"213 26% 7%"`), compatible with Tailwind's `hsl(var(--pensieve-*))` pattern.
`radius` is a CSS length. `fontSans` / `fontMono` are CSS font-stack strings.

| Token | CSS variable | Description |
|---|---|---|
| `background` | `--pensieve-background` | Page / panel background |
| `surface` | `--pensieve-surface` | Slightly elevated surface (toolbar, sidebar) |
| `foreground` | `--pensieve-foreground` | Default text colour |
| `border` | `--pensieve-border` | Subtle dividers and outlines |
| `borderStrong` | `--pensieve-border-strong` | Stronger borders (e.g. input focus ring adjacent) |
| `input` | `--pensieve-input` | Input field border |
| `ring` | `--pensieve-ring` | Focus ring colour |
| `primary` | `--pensieve-primary` | Brand accent (buttons, active states) |
| `primaryForeground` | `--pensieve-primary-foreground` | Text on primary backgrounds |
| `secondary` | `--pensieve-secondary` | Secondary / ghost buttons background |
| `secondaryForeground` | `--pensieve-secondary-foreground` | Text on secondary backgrounds |
| `destructive` | `--pensieve-destructive` | Danger states (delete, error) |
| `destructiveForeground` | `--pensieve-destructive-foreground` | Text on destructive backgrounds |
| `muted` | `--pensieve-muted` | Muted backgrounds (empty states, badges) |
| `mutedForeground` | `--pensieve-muted-foreground` | Subdued text (captions, labels) |
| `accent` | `--pensieve-accent` | Hover highlights, selection backgrounds |
| `accentForeground` | `--pensieve-accent-foreground` | Text on accent backgrounds |
| `card` | `--pensieve-card` | Card surfaces |
| `cardForeground` | `--pensieve-card-foreground` | Text on card surfaces |
| `popover` | `--pensieve-popover` | Popover / dropdown backgrounds |
| `popoverForeground` | `--pensieve-popover-foreground` | Text in popovers |
| `brandFrom` | `--pensieve-brand-from` | Gradient start (logo, brand accents) |
| `brandTo` | `--pensieve-brand-to` | Gradient end |
| `radius` | `--pensieve-radius` | Base border radius (CSS length, e.g. `"0.625rem"`) |
| `fontSans` | `--pensieve-font-sans` | Sans-serif font stack |
| `fontMono` | `--pensieve-font-mono` | Monospace font stack (editor, code) |

## Built-in presets

```ts
import { pensieveDark, pensieveLight } from "@pensieve-ai/react";
```

| Preset | When to use |
|---|---|
| `pensieveDark` | Default. Dark background (`background: "213 26% 7%"`). Used when `theme` is omitted or `undefined`. |
| `pensieveLight` | Light background (`background: "0 0% 100%"`). |

Both presets use system font stacks (`ui-sans-serif`, `ui-monospace`) so they
do not force a specific brand font onto the host application.

## Applying a preset

```tsx
import { pensieveLight } from "@pensieve-ai/react";

<PensieveProvider theme={pensieveLight} ...>
  {children}
</PensieveProvider>
```

## Partial override

Pass any subset of `PensieveTheme`. The SDK merges the overrides over `pensieveDark`:

```tsx
<PensieveProvider
  theme={{
    primary: "262 83% 58%",          // purple brand
    primaryForeground: "0 0% 100%",
    ring: "262 83% 65%",
    radius: "0.375rem",              // tighter corners
  }}
  ...
>
  {children}
</PensieveProvider>
```

Only the tokens you provide are overridden; the rest inherit from `pensieveDark`.

## `"inherit"` mode

When your host application already defines `--pensieve-*` custom properties in its
own CSS, set `theme="inherit"` to prevent the SDK from emitting any inline
token values:

```tsx
<PensieveProvider theme="inherit" ...>
  {children}
</PensieveProvider>
```

In `inherit` mode:

- No `--pensieve-*` vars are set as inline styles on the provider element.
- Your host CSS variables are respected at their natural cascade specificity.
- The provider still resolves `pensieveDark` internally for logic that needs token
  values (e.g. `isDark` context flag for Monaco's theme); it uses those as
  fallback values if your vars are not set.

## Portal container

Pensieve uses Radix UI primitives for dropdowns, dialogs, and tooltips. These
portals render into a `<div class="pensieve-root">` element appended by
`PensieveProvider` — outside the component subtree. The provider sets the same
`--pensieve-*` inline style on that container so portalled content is always
themed correctly, even when it escapes the DOM subtree.

If your application restricts portal rendering (e.g. to a shadow DOM host),
you can pass a custom React Query client via `queryClient` but there is
currently no prop to override the portal container. File an issue if you
need this.

## Reacting to dark/light dynamically

```tsx
import { usePensieveContext, pensieveDark, pensieveLight } from "@pensieve-ai/react";

function ThemeToggle() {
  const { isDark } = usePensieveContext();
  // isDark is true when the background token is perceptually dark
  return <span>{isDark ? "Dark mode" : "Light mode"}</span>;
}
```

`isDark` is derived from the `background` token's lightness component. You
can use this to drive external UI (icon variants, charts outside the provider)
that need to know the current theme mode.

## Using `themeToCssVars`

If you want to drive your own CSS variables from a `PensieveTheme` record:

```ts
import { themeToCssVars, pensieveLight } from "@pensieve-ai/react";

const vars = themeToCssVars(pensieveLight);
// { "--pensieve-background": "0 0% 100%", "--pensieve-foreground": "214 32% 14%", ... }
Object.assign(document.documentElement.style, vars);
```

Only tokens present in the partial theme are emitted; passing `{}` produces an
empty object.
