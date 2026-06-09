---
title: Theming — @kyma-ai/react
description: Design tokens, built-in presets, partial overrides, and host-CSS inherit mode for embedded Kyma components.
---

# Theming

`KymaProvider` accepts a `theme` prop that controls the visual appearance of
all embedded components. The SDK expresses every visual primitive as a flat
record of 26 design tokens (`KymaTheme`), which are emitted as CSS custom
properties (`--kyma-*`) on the provider's root element.

## Token table

All colour values are HSL triplet strings **without** the `hsl()` wrapper
(e.g. `"213 26% 7%"`), compatible with Tailwind's `hsl(var(--kyma-*))` pattern.
`radius` is a CSS length. `fontSans` / `fontMono` are CSS font-stack strings.

| Token | CSS variable | Description |
|---|---|---|
| `background` | `--kyma-background` | Page / panel background |
| `surface` | `--kyma-surface` | Slightly elevated surface (toolbar, sidebar) |
| `foreground` | `--kyma-foreground` | Default text colour |
| `border` | `--kyma-border` | Subtle dividers and outlines |
| `borderStrong` | `--kyma-border-strong` | Stronger borders (e.g. input focus ring adjacent) |
| `input` | `--kyma-input` | Input field border |
| `ring` | `--kyma-ring` | Focus ring colour |
| `primary` | `--kyma-primary` | Brand accent (buttons, active states) |
| `primaryForeground` | `--kyma-primary-foreground` | Text on primary backgrounds |
| `secondary` | `--kyma-secondary` | Secondary / ghost buttons background |
| `secondaryForeground` | `--kyma-secondary-foreground` | Text on secondary backgrounds |
| `destructive` | `--kyma-destructive` | Danger states (delete, error) |
| `destructiveForeground` | `--kyma-destructive-foreground` | Text on destructive backgrounds |
| `muted` | `--kyma-muted` | Muted backgrounds (empty states, badges) |
| `mutedForeground` | `--kyma-muted-foreground` | Subdued text (captions, labels) |
| `accent` | `--kyma-accent` | Hover highlights, selection backgrounds |
| `accentForeground` | `--kyma-accent-foreground` | Text on accent backgrounds |
| `card` | `--kyma-card` | Card surfaces |
| `cardForeground` | `--kyma-card-foreground` | Text on card surfaces |
| `popover` | `--kyma-popover` | Popover / dropdown backgrounds |
| `popoverForeground` | `--kyma-popover-foreground` | Text in popovers |
| `brandFrom` | `--kyma-brand-from` | Gradient start (logo, brand accents) |
| `brandTo` | `--kyma-brand-to` | Gradient end |
| `radius` | `--kyma-radius` | Base border radius (CSS length, e.g. `"0.625rem"`) |
| `fontSans` | `--kyma-font-sans` | Sans-serif font stack |
| `fontMono` | `--kyma-font-mono` | Monospace font stack (editor, code) |

## Built-in presets

```ts
import { kymaDark, kymaLight } from "@kyma-ai/react";
```

| Preset | When to use |
|---|---|
| `kymaDark` | Default. Dark background (`background: "213 26% 7%"`). Used when `theme` is omitted or `undefined`. |
| `kymaLight` | Light background (`background: "0 0% 100%"`). |

Both presets use system font stacks (`ui-sans-serif`, `ui-monospace`) so they
do not force a specific brand font onto the host application.

## Applying a preset

```tsx
import { kymaLight } from "@kyma-ai/react";

<KymaProvider theme={kymaLight} ...>
  {children}
</KymaProvider>
```

## Partial override

Pass any subset of `KymaTheme`. The SDK merges the overrides over `kymaDark`:

```tsx
<KymaProvider
  theme={{
    primary: "262 83% 58%",          // purple brand
    primaryForeground: "0 0% 100%",
    ring: "262 83% 65%",
    radius: "0.375rem",              // tighter corners
  }}
  ...
>
  {children}
</KymaProvider>
```

Only the tokens you provide are overridden; the rest inherit from `kymaDark`.

## `"inherit"` mode

When your host application already defines `--kyma-*` custom properties in its
own CSS, set `theme="inherit"` to prevent the SDK from emitting any inline
token values:

```tsx
<KymaProvider theme="inherit" ...>
  {children}
</KymaProvider>
```

In `inherit` mode:

- No `--kyma-*` vars are set as inline styles on the provider element.
- Your host CSS variables are respected at their natural cascade specificity.
- The provider still resolves `kymaDark` internally for logic that needs token
  values (e.g. `isDark` context flag for Monaco's theme); it uses those as
  fallback values if your vars are not set.

## Portal container

Kyma uses Radix UI primitives for dropdowns, dialogs, and tooltips. These
portals render into a `<div class="kyma-root">` element appended by
`KymaProvider` — outside the component subtree. The provider sets the same
`--kyma-*` inline style on that container so portalled content is always
themed correctly, even when it escapes the DOM subtree.

If your application restricts portal rendering (e.g. to a shadow DOM host),
you can pass a custom React Query client via `queryClient` but there is
currently no prop to override the portal container. File an issue if you
need this.

## Reacting to dark/light dynamically

```tsx
import { useKymaContext, kymaDark, kymaLight } from "@kyma-ai/react";

function ThemeToggle() {
  const { isDark } = useKymaContext();
  // isDark is true when the background token is perceptually dark
  return <span>{isDark ? "Dark mode" : "Light mode"}</span>;
}
```

`isDark` is derived from the `background` token's lightness component. You
can use this to drive external UI (icon variants, charts outside the provider)
that need to know the current theme mode.

## Using `themeToCssVars`

If you want to drive your own CSS variables from a `KymaTheme` record:

```ts
import { themeToCssVars, kymaLight } from "@kyma-ai/react";

const vars = themeToCssVars(kymaLight);
// { "--kyma-background": "0 0% 100%", "--kyma-foreground": "214 32% 14%", ... }
Object.assign(document.documentElement.style, vars);
```

Only tokens present in the partial theme are emitted; passing `{}` produces an
empty object.
