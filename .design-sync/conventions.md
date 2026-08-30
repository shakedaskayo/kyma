# Building with @pensieve-ai/react

## Always wrap in PensieveProvider

Every Pensieve component throws `Pensieve components must be rendered inside <PensieveProvider>` when mounted outside the provider. It supplies the API client, React Query cache, and the theme (it injects every `--pensieve-*` CSS variable at runtime — none are defined in the stylesheets).

```jsx
const { PensieveProvider, PensieveQueryEditor, pensieveDark } = window.PensieveAiReact;

<PensieveProvider
  endpoint="https://pensieve.example.com"   // Pensieve server URL
  auth={{ token: "demo" }}              // or { getToken: async () => "…" }
  theme={pensieveDark}                      // pensieveDark (default) | pensieveLight | partial override | "inherit"
  database="production"                 // optional default database
>
  <div style={{ height: 600 }}>
    <PensieveQueryEditor defaultQuery="events | take 10" />
  </div>
</PensieveProvider>
```

Components are full-bleed views — give them an explicitly sized container (`height: 100%` chain or a fixed px height); in an unsized div they collapse.

**Data**: these are live data-driven views. With a real Pensieve endpoint they populate themselves; without one they render their full chrome with loading/empty/error states — fine for layout work, but don't expect rows/nodes in a static design.

## Styling idiom — theme tokens, not CSS classes

Internal styles are compiled Tailwind utilities under the `ky-` prefix — never write `ky-*` classes yourself, and don't try Tailwind class overrides. Restyle through the theme:

- `theme` accepts a partial `PensieveTheme`: keys `background, surface, foreground, border, borderStrong, input, ring, primary, primaryForeground, secondary, secondaryForeground, destructive, destructiveForeground, muted, mutedForeground, accent, accentForeground, card, cardForeground, popover, popoverForeground, brandFrom, brandTo, radius, fontSans, fontMono`.
- Color values are **HSL triplet strings without `hsl()`** — e.g. `"213 26% 7%"`, composed as `hsl(var(--pensieve-*))`. `radius` is a CSS length (`"0.625rem"`); `fontSans`/`fontMono` are font stacks.
- Example accent swap: `theme={{ ...pensieveDark, accent: "185 80% 40%", primary: "185 80% 40%" }}`.
- `theme="inherit"` maps the host app's existing `--pensieve-*` variables instead of injecting presets.
- For your own layout glue around the components, match the look by reading the same vars: `background: hsl(var(--pensieve-surface))`, `border: 1px solid hsl(var(--pensieve-border))`, `border-radius: var(--pensieve-radius)`.
- Every view component also takes `className`/`style` on its root for sizing and placement.

## The components

- `PensieveQueryEditor` — KQL/SQL editor + schema browser + results (`language`, `defaultQuery`, `showSchemaBrowser`, `showResults`, `readOnly`).
- `PensieveDiscover` — log/event search over all sources (`defaultQuery`, `scope`, `timeRange`).
- `PensieveGraph` — knowledge-graph canvas (`layout`, `focusQuery`, `graphs`, `sidebar`, `toolbar`).
- `PensieveDashboard` — panel grid with charts (`dashboardId` required, `editable`, `timeRange`).
- `PensieveAgentChat` — natural-language agent chat (`database`, `placeholder`).
- Headless hooks (`usePensieveQuery`, `usePensieveGraph`, `usePensieveDiscover`, `usePensieveDashboards`, `usePensieveAgent`, `usePensieveCapabilities`) exist on the same global for custom UI.

Per-component props and verified example JSX: `components/<group>/<Name>/<Name>.prompt.md` (the `.d.ts` beside it is the exact contract). `styles.css` → `_ds_bundle.css` is the compiled truth for every `ky-*` rule and `--pensieve-*` usage.
