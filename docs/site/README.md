# kyma docs site

Build: `pnpm install && pnpm dev` (from this directory).

Production deploy: a separate Railway service named `kyma-docs` on the same
account that runs the `kyma` engine, configured to track `main`. The Dockerfile
+ railway.toml live in this directory; Railway auto-detects them. Custom domain
`docs.kyma.<domain>` is finalized at D0 review (spec §12 question 1).

## Editing

- Markdown lives in this directory under section folders.
- `architecture.md` and `benchmarks.md` are sourced from one level up
  (`docs/architecture.md`, `docs/benchmarks.md`); the build copies them.
- SVG diagrams: drop into `public/diagrams/` and reference via
  `<Diagram name="..." caption="..." />`. The build validates references.
- Mermaid: fenced ` ```mermaid ` blocks render automatically.
