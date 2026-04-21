# arrow-arith 53.4.1 — local patch

Vendored copy of `arrow-arith` 53.4.1 with two minimal changes so that the
crate compiles against `chrono >= 0.4.44` (the version adk-core pulls in
transitively).

## Why

`adk-rust 0.6` brings `chrono 0.4.44`, which ships a default
`Datelike::quarter` method. Upstream arrow-arith 53.x defines its own
`ChronoDateExt::quarter`, and the bare call `d.quarter()` in
`get_date_time_part_extract_fn` resolves ambiguously under the new chrono,
breaking `cargo build`.

DataFusion 44 is the only DF release line that tracks arrow 53.x, and there
is no arrow-arith 53.x release with the fix — arrow-rs addressed it in
54.x by removing the shadowing entry. We therefore carry a tiny patch
instead of pinning chrono back (adk-core requires `>= 0.4.44`).

## Changes vs upstream 53.4.1

1. `src/temporal.rs`: `d.quarter()` → `Datelike::quarter(&d)`.
2. `Cargo.toml`: chrono upper bound widened from `< 0.4.40` to `< 0.5`.

Nothing else was edited. Upstream source was pulled from the registry
tarball (`~/.cargo/registry/src/index.crates.io-.../arrow-arith-53.4.1/`).

Wired in via `[patch.crates-io] arrow-arith = { path = ... }` at the
workspace root. Remove this patch once DataFusion publishes a release
compatible with `chrono >= 0.4.44`.
