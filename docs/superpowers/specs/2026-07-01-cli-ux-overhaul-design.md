# kyma CLI — UX overhaul (shared output toolkit + phased command rollout)

**Date:** 2026-07-01
**Status:** Approved (brainstorm with user; all decisions locked)

## Summary

The kyma CLI (`crates/kyma-cli`, ~30 subcommands across `main.rs`, `plugin.rs`, `datasource.rs`)
currently formats every command's output by hand with raw `println!()`. There is no color, no
tables, no progress indication, error messages are often Rust `anyhow` debug dumps, and every
command invents its own layout (fixed-width columns in `datasource list`, tab-separated fields in
`sessions list`, bullet points in `recall`). This spec covers a shared terminal-UX toolkit and a
phased rollout across every subcommand, ending with live (ratatui) views for the three streaming
commands.

Decisions made with the user:

1. **Modern CLI aesthetic** (gh/cargo-style: color, glyphs, aligned tables, spinners) — not a
   minimalist plain-text polish, not a full TUI by default.
2. **Every subcommand is in scope**, not just `recall`. Rolled out in five phases (U0–U4) so it
   ships incrementally rather than as one large PR.
3. **A shared `ux` module**, not per-command inline styling — the current inconsistency is a
   direct result of every command formatting independently.
4. **`--json` is preserved and extended** as the scripting escape hatch — when passed, `ux` is
   bypassed entirely and raw `serde_json` is emitted. Pretty output is human-only.
5. **Errors are unified too** — a single error printer replaces per-command `anyhow` debug dumps,
   wired once into `main()`'s top-level handler so it applies to every command including ones not
   yet touched by later phases.
6. **Ratatui live views are in scope**, but sequenced last (U4) as the highest-risk, most novel
   piece — only for the three commands that stream (`ingest tail`, `query`, `watch`). Everything
   else stays line-based with spinners/tables.

## 1. Architecture — the `ux` module

New module `crates/kyma-cli/src/ux/`. Every command imports from it instead of hand-rolling
output. It wraps three libraries already resolved in `Cargo.lock` as transitive deps but not
currently used directly by `kyma-cli`: `console`, `comfy-table`, `indicatif` — no new dependency
for U0–U3 (U4 adds `ratatui`).

- **`theme.rs`** — semantic style functions (`success`, `error`, `warn`, `info`, `muted`,
  `accent`) built on `console::style()`; status symbols (`✓ ✗ → ▸ •`); `color_enabled()` resolves
  once at startup from `NO_COLOR`, `TERM=dumb`, non-TTY stdout (via `console::Term`), and a new
  global `--no-color` flag — every other module reads this instead of re-detecting.
- **`table.rs`** — `ux::table()` returns a `comfy_table::Table` preconfigured with one preset
  (rounded UTF-8 borders, ASCII fallback when the terminal doesn't support UTF-8, header row
  styled via `theme`). Column-level helpers for common patterns (status columns render as
  colored glyphs, not raw strings).
- **`spinner.rs`** — `ux::spinner(msg)` / `ux::progress_bar(len, msg)` wrap `indicatif` with one
  style (braille spinner + elapsed time). Auto-disabled to a plain `"{msg}..."` line followed by
  a result line when stdout isn't a TTY (CI logs, piped output).
- **`error.rs`** — `ux::print_error(&anyhow::Error)`, called once from `main()`'s top-level error
  return path. Walks the `anyhow` cause chain and prints a `✗ Error:` header (red) with each cause
  indented underneath, instead of `{:?}`-style debug output. Appends a `hint:` line for a small,
  growing table of known cases: connection refused → "is `kyma serve` running?"; 401/403 → "run
  `kyma connect` to re-auth"; 404 → resource-not-found phrasing; timeout; validation errors. Not
  exhaustive on day one — designed to grow as new cases are noticed.
- **`format.rs`** — relative timestamps (`"2m ago"`), word-boundary-aware truncation with ellipsis
  (replacing `recall`'s hardcoded 280-char hard cut), and a score→color mapping for similarity
  scores (e.g. green ≥0.8, yellow ≥0.5, gray below).

**Scripting is unaffected.** `--json` (already present on `recall`, extended in later phases to
`status`, `sessions list`, `datasource list/show`, `ingest status` where a machine-readable shape
makes sense) bypasses `ux` entirely — raw `serde_json::to_writer_pretty` to stdout, no color, no
tables, no spinners. Anything piped or run with `--json` sees plain deterministic output.

**`--help` styling.** `clap::builder::Styles` wired into the top-level `Command` in `main.rs` —
colors headers/literals in `--help` output across all ~30 subcommands for free, no per-command
changes needed.

## 2. Phased rollout

Five phases, each independently mergeable as its own branch/PR:

| Phase | Scope | Commands |
|---|---|---|
| **U0** | Toolkit foundations | `ux` module, clap `Styles`, global `--no-color` flag, `error.rs` wired into `main()`'s top-level handler |
| **U1** | Memory commands (named explicitly by the user; highest visibility) | `recall`, `remember`, `entity`, `distill`, `status` |
| **U2** | Data sources & ingest | `datasource` (`add`/`show`/`list`/`pause`/`resume`/`remove`/`trigger`), `ingest` (`push`/`status`/`tail` — plain-line polish only, no live view yet) |
| **U3** | Everything else | `sessions`, `connect`, `query` (non-streaming parts), `watch`, `compact`, `scrape`, `user`, `install-skill`, `install-plugin`, `mcp`, `serve`, `setup`, `sync`, `worker`, `service`, admin (`create-database`/`create-table`/`list-tables`/`alter-table`/`configure-embed`/`create-graph`/`list-graphs`/`drop-graph`), `deploy`, `update`, `version` |
| **U4** | Ratatui live views | `ingest tail`, `query` (streaming SSE), `watch` get a live-updating view (scrolling output pane + status header) replacing plain streamed lines. Adds `ratatui` as a new dependency — the one genuinely new/risky piece, sequenced last so it doesn't block the rest |

U0 must land first (everything else depends on it). U1–U3 are otherwise independent of each other
and can proceed in any order once U0 is merged, but U1 → U2 → U3 is the priority order per the
user. U4 depends on U2 (ingest tail) and U3 (query, watch) having already been migrated to the
`ux` module for their non-streaming pieces (headers, error paths).

## 3. Error handling

Covered by `error.rs` in U0. Because it hooks `main()`'s top-level error return (the `Result<(),
anyhow::Error>` that every subcommand handler already returns into), it applies retroactively to
every command from U0 onward — including commands not yet touched by U1–U4's output-formatting
work. The hint table starts small (~5 cases) and is not meant to be exhaustive; add cases as
they're noticed in real usage rather than trying to enumerate every failure mode up front.

## 4. Testing

- **Unit tests** for pure logic: truncation/ellipsis (`format.rs`), relative-time formatting,
  error-hint lookup. Deterministic, no TTY/color involved.
- **Snapshot tests** (`insta`, already resolved in `Cargo.lock`) for table and error rendering,
  with `NO_COLOR=1` forced so snapshots capture plain text structure, not ANSI escape codes.
- **Manual verification per phase** — each phase's PR includes a terminal screenshot/paste of the
  retrofitted commands, since colors/spinners/live views can only really be confirmed by looking
  at a real terminal (matches the project's existing `/verify` workflow).
- **`--json` regression check** — for every command where `--json` exists or is added, confirm
  output is unchanged plain JSON regardless of `ux` changes (run once with and without a TTY).

## 5. Out of scope

- Interactive command discovery / fuzzy `--help` search.
- Ratatui (or any TUI) for commands outside the three named streaming commands.
- Rewriting command *behavior* or flags — this is presentation-only; no subcommand's semantics,
  inputs, or JSON shapes change because of this program (aside from `--json` being added to a few
  list/status commands that don't have it yet).
- A standalone crate for the `ux` module — it's a module inside `kyma-cli` since no other crate
  consumes it; promote to a crate later only if that changes.
