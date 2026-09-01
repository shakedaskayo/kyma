# Contributing to pensieve

Thanks for your interest in pensieve. The project is pre-alpha — the design is
stable, the surface is not. Contributions are very welcome; please read this
short guide before opening a PR.

## Ground rules

- Be kind. This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
- Discuss non-trivial changes in an issue first. Small fixes (typos, obvious
  bugs, tightening tests) can go straight to a PR.
- Security issues go to [SECURITY.md](SECURITY.md), not the public issue tracker.

## Development setup

```bash
# bring up dev dependencies (Postgres + MinIO + Redpanda)
docker-compose up -d

# build + run the engine
cargo run --release -p pensieve-bin

# web UI (optional)
pnpm install
pnpm -C web dev
```

The end-to-end scripts in `scripts/` (`e2e-test.sh`, `test-kql.sh`,
`test-flight.sh`, `load-test.sh`, ...) are the fastest way to exercise the
running engine and to verify a change end-to-end.

## Workflow

1. Fork the repo, create a topic branch from `main`.
2. Make your change. Keep commits focused; squash trivia before pushing.
3. Run the relevant tests:
   - `cargo test --workspace` for Rust changes.
   - `pnpm -C web test && pnpm -C web typecheck && pnpm -C web lint` for web.
   - Any of the `scripts/test-*.sh` suites relevant to the change.
4. Open a PR against `main`. Describe the change, the motivation, and how you
   tested it.

## Commit style

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(server): add Flight SQL DoGet handler
fix(catalog): bind tenant_id in alter_table_add_column INSERT
docs(architecture): clarify pruning cascade
ci(image): bump base image to debian:12-slim
test(kql): cover summarize-by + having
```

The scope (in parens) is usually the crate name (`pensieve-server`, `pensieve-catalog`,
...) or a subsystem (`web`, `docs`, `ci`).

## Architectural invariants

pensieve has five hard invariants (see [`docs/architecture.md`](docs/architecture.md)):

1. Object storage is the only durable source of truth.
2. Compute is stateless.
3. The catalog is externalized (Postgres today).
4. The on-disk segment format is pluggable behind `SegmentFormat`.
5. The query parser is pluggable behind the planner IR.

Architectural tests enforce these. If your change requires bending an
invariant, please flag it explicitly in the PR description — there is almost
always another way.

## Stability and deprecation policy

From `v1.0.0` onward, pensieve maintains a written stability contract: [`docs/stability.md`](docs/stability.md). It lists every surface pensieve promises not to break across the v1.x series — HTTP REST API, Flight gRPC, KQL dialect, SQL dialect, MCP surface, catalog schema, config keys, extent format, metrics naming.

If your change touches any frozen surface, your PR must either:

- Stay within the contract (additive, non-breaking) — preferred. The CI workflow `.github/workflows/backcompat.yml` enforces this on every PR by replaying a fixed query set against every committed version fixture under `scripts/fixtures/backcompat/` (one per tag, growing as `v1.0.0-pre.N` tags are cut).
- Or follow the deprecation policy in `docs/stability.md` section 10 (RFC, replacement-first, 6-month warning window).

Pre-`v1.0.0` builds (`v0.x`, `v1.0.0-pre.N`, `v1.0.0-rc.N`) are not under the contract.

## License

By contributing, you agree that your contributions will be licensed under the
MIT License, the same license as the project.
