---
title: Embedding Pensieve — overview
description: Drop Pensieve's Graph, Query, Discover, Dashboards, and Agent views into any React app with @pensieve-ai/react.
---

# Embedding Pensieve in React

`@pensieve-ai/react` is an Apache-2.0 licensed embeddable SDK that drops five
production-quality Pensieve views — **Graph**, **Query Editor**, **Discover**,
**Dashboards**, and **Agent Chat** — into any React 18+ host application.
Each view is a fully self-contained component backed by the same UI rendered
in the Pensieve web app itself.

## In this section

| Page | What it covers |
|---|---|
| [Quickstart](/embed/quickstart) | Install, wire the provider, render a view in under 20 lines |
| [Authentication](/embed/authentication) | Static tokens, server-side minting, full OIDC setup |
| [CORS](/embed/cors) | `PENSIEVE_CORS_ALLOWED_ORIGINS` — permissive dev vs. production lock-down |
| [Theming](/embed/theming) | All 26 design tokens, presets, partial overrides, `inherit` mode |
| [Components](/embed/components) | Per-component prop tables and bundle notes |
| [Headless hooks](/embed/hooks) | Raw data layer without the bundled UI |
| [Server compatibility](/embed/server-compat) | Feature detection, capability gating, known limitations |
| [Versioning and release](/embed/versioning) | Changesets flow, semver policy |

## Reference demo

The `examples/embed-demo` package in the repository is a working reference host
app that exercises every component and hook. Read
`examples/embed-demo/README.md` to run it locally.
