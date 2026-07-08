# Building with @kyma-ai/react

## Always wrap in KymaProvider

Every Kyma component throws `Kyma components must be rendered inside <KymaProvider>` when mounted outside the provider. It supplies the API client, React Query cache, and the theme (it injects every `--kyma-*` CSS variable at runtime — none are defined in the stylesheets).

```jsx
const { KymaProvider, KymaQueryEditor, kymaDark } = window.KymaAiReact;

<KymaProvider
  endpoint="https://kyma.example.com"   // Kyma server URL
  auth={{ token: "demo" }}              // or { getToken: async () => "…" }
  theme={kymaDark}                      // kymaDark (default) | kymaLight | partial override | "inherit"
  database="production"                 // optional default database
>
  <div style={{ height: 600 }}>
    <KymaQueryEditor defaultQuery="events | take 10" />
  </div>
</KymaProvider>
```

Components are full-bleed views — give them an explicitly sized container (`height: 100%` chain or a fixed px height); in an unsized div they collapse.

**Data**: these are live data-driven views. With a real Kyma endpoint they populate themselves; without one they render their full chrome with loading/empty/error states — fine for layout work, but don't expect rows/nodes in a static design.

## Styling idiom — theme tokens, not CSS classes

Internal styles are compiled Tailwind utilities under the `ky-` prefix — never write `ky-*` classes yourself, and don't try Tailwind class overrides. Restyle through the theme:

- `theme` accepts a partial `KymaTheme`: keys `background, surface, foreground, border, borderStrong, input, ring, primary, primaryForeground, secondary, secondaryForeground, destructive, destructiveForeground, muted, mutedForeground, accent, accentForeground, card, cardForeground, popover, popoverForeground, brandFrom, brandTo, radius, fontSans, fontMono`.
- Color values are **HSL triplet strings without `hsl()`** — e.g. `"213 26% 7%"`, composed as `hsl(var(--kyma-*))`. `radius` is a CSS length (`"0.625rem"`); `fontSans`/`fontMono` are font stacks.
- Example accent swap: `theme={{ ...kymaDark, accent: "185 80% 40%", primary: "185 80% 40%" }}`.
- `theme="inherit"` maps the host app's existing `--kyma-*` variables instead of injecting presets.
- For your own layout glue around the components, match the look by reading the same vars: `background: hsl(var(--kyma-surface))`, `border: 1px solid hsl(var(--kyma-border))`, `border-radius: var(--kyma-radius)`.
- Every view component also takes `className`/`style` on its root for sizing and placement.

## The components

- `KymaQueryEditor` — KQL/SQL editor + schema browser + results (`language`, `defaultQuery`, `showSchemaBrowser`, `showResults`, `readOnly`).
- `KymaDiscover` — log/event search over all sources (`defaultQuery`, `scope`, `timeRange`).
- `KymaGraph` — knowledge-graph canvas (`layout`, `focusQuery`, `graphs`, `sidebar`, `toolbar`).
- `KymaDashboard` — panel grid with charts (`dashboardId` required, `editable`, `timeRange`).
- `KymaAgentChat` — natural-language agent chat (`database`, `placeholder`).
- Headless hooks (`useKymaQuery`, `useKymaGraph`, `useKymaDiscover`, `useKymaDashboards`, `useKymaAgent`, `useKymaCapabilities`) exist on the same global for custom UI.

Per-component props and verified example JSX: `components/<group>/<Name>/<Name>.prompt.md` (the `.d.ts` beside it is the exact contract). `styles.css` → `_ds_bundle.css` is the compiled truth for every `ky-*` rule and `--kyma-*` usage.
