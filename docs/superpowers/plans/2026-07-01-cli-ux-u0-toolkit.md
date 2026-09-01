# CLI UX U0 — Shared `ux` Toolkit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the shared `crates/pensieve-cli/src/ux/` toolkit (color theme, tables, spinners, unified error rendering, relative-time/truncation helpers) and wire color detection + unified error printing + styled `--help` into `main()`, without changing any individual command's success-path output yet.

**Architecture:** One new module tree, `ux::{theme, format, table, spinner, error}`, each independently unit-tested. `theme` owns the single color-enabled decision (flag + `NO_COLOR` + TTY detection) via a `OnceLock<bool>` set once in `main()`; every other submodule takes an explicit `enabled`/`interactive` bool in its internally-testable core so tests never depend on process-global state or the real terminal. `main()` is split into a thin `main()` (parses args, inits theme, prints errors) and `run()` (the entire existing command dispatch, unchanged).

**Tech Stack:** Rust, clap 4 (derive), `console` 0.15 (color/TTY detection), `comfy-table` 7 (tables), `indicatif` 0.17 (spinners), `insta` 1 (snapshot tests, dev-only), `anyhow` (errors, already in use).

## Global Constraints

- Full spec: `docs/superpowers/specs/2026-07-01-cli-ux-overhaul-design.md`. This plan implements **U0 only** — the toolkit foundations. U1 (recall/remember/entity/distill/status), U2 (datasource/ingest), U3 (everything else), and U4 (ratatui live views) are separate follow-up plans; do not wire `table`/`format`/`spinner` into any command's output in this plan.
- `pensieve-cli` is a **binary crate** — run unit tests with `cargo test -p pensieve-cli --bins` (not `--lib`, which doesn't exist for this crate).
- Match the crate's existing visibility convention: internal items are `pub(crate)`, never bare `pub` (see `plugin.rs`/`datasource.rs` — this is a binary with no external consumers, so `pub` items would trip the workspace's `unreachable_pub = "warn"` lint).
- `--json` and any command's existing output are untouched by this plan — U0 changes only `--help` styling and error-path output (via `main()`'s new top-level handler), which apply automatically to every command including ones this plan never touches.
- No new dependency beyond `console`, `comfy-table`, `indicatif` (already resolved in `Cargo.lock` as transitive deps — this plan makes them direct deps) and `insta` (dev-only, also already resolved, used elsewhere in the workspace by `pensieve-embed`).
- It is expected that `format.rs`, `table.rs`, and `spinner.rs` have no callers yet after this plan — `cargo build -p pensieve-cli` may show `dead_code` warnings (not errors) for their public functions until U1 wires them in. `cargo test -p pensieve-cli --bins` will not warn, since each function is exercised by its own test module.

---

### Task 1: `ux` module scaffolding + `theme.rs` (color detection & semantic styles)

**Files:**
- Modify: `crates/pensieve-cli/Cargo.toml`
- Create: `crates/pensieve-cli/src/ux/mod.rs`
- Create: `crates/pensieve-cli/src/ux/theme.rs`
- Modify: `crates/pensieve-cli/src/main.rs:23-29` (add `mod ux;`)

**Interfaces:**
- Produces: `ux::theme::init(no_color_flag: bool)` — call once from `main()` before anything else. `ux::theme::color_enabled() -> bool`. `ux::theme::{success, error, warn, info, muted, accent}(text: &str) -> String`. `ux::theme::{CHECK, CROSS, ARROW, BULLET}: &str` symbol constants. `pub(crate) fn apply(text: &str, style: console::Style, enabled: bool) -> String` (crate-visible, used by `error.rs` in Task 5 for deterministic rendering).

- [ ] **Step 1: Add dependencies to `crates/pensieve-cli/Cargo.toml`**

In the `[dependencies]` section, right after the `clap` line (`clap = { version = "4", features = ["derive", "env"] }`), add:

```toml
# Terminal UX toolkit (crates/pensieve-cli/src/ux/): color/TTY detection, tables, spinners.
console = "0.15"
comfy-table = "7"
indicatif = "0.17"
```

In `[dev-dependencies]` (currently just `wiremock = "0.6"`), add:

```toml
insta = "1"
```

- [ ] **Step 2: Run `cargo build -p pensieve-cli` to confirm the new deps resolve**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly (these versions are already resolved in `Cargo.lock` as transitive deps, so this should not change the lockfile's resolved versions — if it does, stop and check for a version conflict before continuing).

- [ ] **Step 3: Create `crates/pensieve-cli/src/ux/mod.rs`**

```rust
//! Shared terminal-output toolkit for the pensieve CLI. Commands use these
//! helpers instead of hand-rolled `println!` formatting, so color, tables,
//! spinners, and error rendering stay consistent across every subcommand.

pub(crate) mod theme;
```

- [ ] **Step 4: Write `crates/pensieve-cli/src/ux/theme.rs` with failing tests**

```rust
//! Semantic terminal styling — the single place that decides whether color
//! is on, and what each semantic style/symbol looks like. Every other `ux`
//! submodule that needs color goes through here.

use console::{Style, Term};
use std::sync::OnceLock;

static COLOR_ENABLED: OnceLock<bool> = OnceLock::new();

/// Must be called once, early in `main()`, before any other `ux` function
/// runs. `no_color_flag` is the CLI's `--no-color` flag value.
pub(crate) fn init(no_color_flag: bool) {
    let enabled = !no_color_flag
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
        && Term::stdout().features().colors_supported();
    let _ = COLOR_ENABLED.set(enabled);
}

/// Whether styled output should be emitted. Defaults to `true` if `init`
/// was never called (e.g. in unit tests, where no output is ever printed
/// for real).
pub(crate) fn color_enabled() -> bool {
    *COLOR_ENABLED.get().unwrap_or(&true)
}

/// Applies `style` to `text` when `enabled`, otherwise returns `text`
/// unchanged. Crate-visible so `error.rs` can render deterministically in
/// tests without depending on the global flag above.
pub(crate) fn apply(text: &str, style: Style, enabled: bool) -> String {
    todo!("apply {text} with {style:?} enabled={enabled}")
}

pub(crate) fn success(text: &str) -> String {
    apply(text, Style::new().green(), color_enabled())
}

pub(crate) fn error(text: &str) -> String {
    apply(text, Style::new().red().bold(), color_enabled())
}

pub(crate) fn warn(text: &str) -> String {
    apply(text, Style::new().yellow(), color_enabled())
}

pub(crate) fn info(text: &str) -> String {
    apply(text, Style::new().cyan(), color_enabled())
}

pub(crate) fn muted(text: &str) -> String {
    apply(text, Style::new().dim(), color_enabled())
}

pub(crate) fn accent(text: &str) -> String {
    apply(text, Style::new().magenta().bold(), color_enabled())
}

pub(crate) const CHECK: &str = "✓";
pub(crate) const CROSS: &str = "✗";
pub(crate) const ARROW: &str = "→";
pub(crate) const BULLET: &str = "•";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_returns_plain_text_when_disabled() {
        assert_eq!(apply("hello", Style::new().green(), false), "hello");
    }

    #[test]
    fn apply_returns_styled_text_when_enabled() {
        let styled = apply("hello", Style::new().green(), true);
        assert_ne!(styled, "hello");
        assert!(styled.contains("hello"));
    }

    #[test]
    fn color_enabled_defaults_true_when_uninitialized() {
        // theme::init() is never called anywhere in this test binary, so
        // this exercises the OnceLock's default branch.
        assert!(color_enabled());
    }
}
```

- [ ] **Step 5: Run tests to verify the two `apply` tests fail**

Run: `cargo test -p pensieve-cli --bins ux::theme::tests`
Expected: `apply_returns_plain_text_when_disabled` and `apply_returns_styled_text_when_enabled` FAIL (panic: `not yet implemented: apply ...`). `color_enabled_defaults_true_when_uninitialized` PASSES already.

- [ ] **Step 6: Implement `apply`**

Replace the `todo!(...)` body from Step 4 with:

```rust
pub(crate) fn apply(text: &str, style: Style, enabled: bool) -> String {
    style.force_styling(enabled).apply_to(text).to_string()
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins ux::theme::tests`
Expected: all 3 tests PASS.

- [ ] **Step 8: Wire the module into `main.rs`**

In `crates/pensieve-cli/src/main.rs`, the module declarations at the top of the file currently read:

```rust
mod client;
mod datasource;
mod deploy;
mod plugin;
mod scrape;
mod update;
mod users;
```

Change to:

```rust
mod client;
mod datasource;
mod deploy;
mod plugin;
mod scrape;
mod update;
mod users;
mod ux;
```

- [ ] **Step 9: Confirm the whole crate still builds**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly (a `dead_code` warning on `ux::theme`'s unused public functions is expected and fine — nothing calls them yet).

- [ ] **Step 10: Commit**

```bash
git add crates/pensieve-cli/Cargo.toml crates/pensieve-cli/src/ux/mod.rs crates/pensieve-cli/src/ux/theme.rs crates/pensieve-cli/src/main.rs
git commit -m "feat(cli): add ux::theme — color detection + semantic styles"
```

---

### Task 2: `format.rs` (relative time, truncation, score coloring)

**Files:**
- Modify: `crates/pensieve-cli/src/ux/mod.rs`
- Create: `crates/pensieve-cli/src/ux/format.rs`

**Interfaces:**
- Consumes: `super::theme::{success, warn, muted}` (Task 1).
- Produces: `ux::format::relative_time(ts: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>) -> String`. `ux::format::truncate(text: &str, max_chars: usize) -> String`. `ux::format::score_style(score: f32, text: &str) -> String`.

- [ ] **Step 1: Add the module declaration**

In `crates/pensieve-cli/src/ux/mod.rs`, add below `pub(crate) mod theme;`:

```rust
pub(crate) mod format;
```

- [ ] **Step 2: Write `crates/pensieve-cli/src/ux/format.rs` with failing tests**

```rust
//! Formatting helpers shared across commands: relative timestamps,
//! word-boundary-aware truncation, and score-to-color mapping.

use chrono::{DateTime, Utc};

/// Human relative time, e.g. `"2m ago"`, `"3h ago"`, `"5d ago"`, or
/// `"just now"` for anything under 5 seconds.
pub(crate) fn relative_time(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    todo!("relative_time({ts}, {now})")
}

/// Truncates `text` to at most `max_chars`, breaking on the last word
/// boundary before the limit and appending an ellipsis. Returns `text`
/// unchanged if it already fits.
pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    todo!("truncate({text}, {max_chars})")
}

/// Maps a similarity/relevance score in `[0.0, 1.0]` to a semantic color:
/// green at `>= 0.8`, yellow at `>= 0.5`, gray otherwise.
pub(crate) fn score_style(score: f32, text: &str) -> String {
    if score >= 0.8 {
        super::theme::success(text)
    } else if score >= 0.5 {
        super::theme::warn(text)
    } else {
        super::theme::muted(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts_secs_ago(secs: i64) -> (DateTime<Utc>, DateTime<Utc>) {
        let now = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
        (now - chrono::Duration::seconds(secs), now)
    }

    #[test]
    fn relative_time_just_now() {
        let (t, now) = ts_secs_ago(2);
        assert_eq!(relative_time(t, now), "just now");
    }

    #[test]
    fn relative_time_minutes() {
        let (t, now) = ts_secs_ago(90);
        assert_eq!(relative_time(t, now), "1m ago");
    }

    #[test]
    fn relative_time_hours() {
        let (t, now) = ts_secs_ago(7200);
        assert_eq!(relative_time(t, now), "2h ago");
    }

    #[test]
    fn relative_time_days() {
        let (t, now) = ts_secs_ago(172_800);
        assert_eq!(relative_time(t, now), "2d ago");
    }

    #[test]
    fn truncate_leaves_short_text_alone() {
        assert_eq!(truncate("hello world", 20), "hello world");
    }

    #[test]
    fn truncate_breaks_on_word_boundary() {
        assert_eq!(truncate("hello there world", 12), "hello there…");
    }

    #[test]
    fn score_style_buckets() {
        assert_eq!(score_style(0.9, "x"), super::super::theme::success("x"));
        assert_eq!(score_style(0.6, "x"), super::super::theme::warn("x"));
        assert_eq!(score_style(0.2, "x"), super::super::theme::muted("x"));
    }
}
```

- [ ] **Step 3: Run tests to verify `relative_time`/`truncate` tests fail**

Run: `cargo test -p pensieve-cli --bins ux::format::tests`
Expected: `relative_time_*` and `truncate_*` tests FAIL (panic: `not yet implemented`). `score_style_buckets` PASSES already (it's implemented in Step 2).

- [ ] **Step 4: Implement `relative_time` and `truncate`**

Replace the two `todo!(...)` bodies:

```rust
pub(crate) fn relative_time(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = now.signed_duration_since(ts).num_seconds();
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    let cut = truncated.rfind(' ').unwrap_or(truncated.len());
    format!("{}…", &truncated[..cut])
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins ux::format::tests`
Expected: all 7 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/pensieve-cli/src/ux/mod.rs crates/pensieve-cli/src/ux/format.rs
git commit -m "feat(cli): add ux::format — relative time, truncation, score coloring"
```

---

### Task 3: `table.rs` (consistent table preset + status cell)

**Files:**
- Modify: `crates/pensieve-cli/src/ux/mod.rs`
- Create: `crates/pensieve-cli/src/ux/table.rs`

**Interfaces:**
- Consumes: `super::theme::{CHECK, CROSS}` (Task 1).
- Produces: `ux::table::table(headers: Vec<&str>) -> comfy_table::Table`. `ux::table::status_cell(status: &str) -> comfy_table::Cell`.

- [ ] **Step 1: Add the module declaration**

In `crates/pensieve-cli/src/ux/mod.rs`, add below `pub(crate) mod format;`:

```rust
pub(crate) mod table;
```

- [ ] **Step 2: Write `crates/pensieve-cli/src/ux/table.rs` with failing tests**

```rust
//! One consistent table preset used by every list-style command.

use comfy_table::{Cell, Color, ContentArrangement, Table};

/// Returns a table pre-configured with pensieve's standard look: rounded
/// UTF-8 borders (comfy-table falls back to ASCII automatically on
/// terminals that report no UTF-8 support), dynamic content arrangement,
/// and the given header row.
pub(crate) fn table(headers: Vec<&str>) -> Table {
    todo!("table({headers:?})")
}

/// A colored status glyph cell: green ✓ for healthy/ok/active/success,
/// yellow `~` for pending/degraded/paused, red ✗ for error/failed, gray
/// `?` for anything unrecognized.
pub(crate) fn status_cell(status: &str) -> Cell {
    let (glyph, color) = match status.to_ascii_lowercase().as_str() {
        "ok" | "healthy" | "active" | "success" => (super::theme::CHECK, Color::Green),
        "pending" | "degraded" | "paused" => ("~", Color::Yellow),
        "error" | "failed" | "unhealthy" => (super::theme::CROSS, Color::Red),
        _ => ("?", Color::DarkGrey),
    };
    Cell::new(format!("{glyph} {status}")).fg(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_renders_given_headers() {
        let t = table(vec!["NAME", "STATUS"]);
        let rendered = t.to_string();
        assert!(rendered.contains("NAME"));
        assert!(rendered.contains("STATUS"));
    }

    #[test]
    fn status_cell_known_statuses() {
        assert_eq!(status_cell("ok").content(), "✓ ok");
        assert_eq!(status_cell("error").content(), "✗ error");
        assert_eq!(status_cell("pending").content(), "~ pending");
    }

    #[test]
    fn status_cell_unknown_status() {
        assert_eq!(status_cell("weird").content(), "? weird");
    }
}
```

- [ ] **Step 3: Run tests to verify `table_renders_given_headers` fails**

Run: `cargo test -p pensieve-cli --bins ux::table::tests`
Expected: `table_renders_given_headers` FAILS (panic: `not yet implemented`). The two `status_cell_*` tests PASS already.

- [ ] **Step 4: Implement `table`**

Replace the `todo!(...)` body:

```rust
pub(crate) fn table(headers: Vec<&str>) -> Table {
    let mut t = Table::new();
    t.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers);
    t
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins ux::table::tests`
Expected: all 3 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/pensieve-cli/src/ux/mod.rs crates/pensieve-cli/src/ux/table.rs
git commit -m "feat(cli): add ux::table — consistent table preset + status cell"
```

---

### Task 4: `spinner.rs` (spinner/progress wrapper, TTY-aware fallback)

**Files:**
- Modify: `crates/pensieve-cli/src/ux/mod.rs`
- Create: `crates/pensieve-cli/src/ux/spinner.rs`

**Interfaces:**
- Consumes: `super::theme::{success, error, CHECK, CROSS}` (Task 1).
- Produces: `ux::spinner::spinner(msg: impl Into<String>) -> Spinner`. `Spinner::finish_success(&self, msg: &str)`. `Spinner::finish_error(&self, msg: &str)`.

- [ ] **Step 1: Add the module declaration**

In `crates/pensieve-cli/src/ux/mod.rs`, add below `pub(crate) mod table;`:

```rust
pub(crate) mod spinner;
```

- [ ] **Step 2: Write `crates/pensieve-cli/src/ux/spinner.rs` with failing tests**

```rust
//! Spinner/progress-bar wrapper with one consistent style. Ticks on
//! stderr (stdout stays clean for piped command output); auto-falls-back
//! to a single plain message line when stderr isn't a terminal, so CI
//! logs and piped output don't fill up with spinner frames.

use console::Term;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

use super::theme;

pub(crate) struct Spinner {
    bar: Option<ProgressBar>,
}

/// Starts a spinner showing `msg`.
pub(crate) fn spinner(msg: impl Into<String>) -> Spinner {
    build_spinner(msg.into(), Term::stderr().is_term())
}

fn build_spinner(msg: String, interactive: bool) -> Spinner {
    todo!("build_spinner({msg}, interactive={interactive})")
}

impl Spinner {
    /// Stops the spinner and prints a final success line.
    pub(crate) fn finish_success(&self, msg: &str) {
        let line = theme::success(&format!("{} {msg}", theme::CHECK));
        match &self.bar {
            Some(bar) => bar.finish_with_message(line),
            None => eprintln!("{line}"),
        }
    }

    /// Stops the spinner and prints a final failure line.
    pub(crate) fn finish_error(&self, msg: &str) {
        let line = theme::error(&format!("{} {msg}", theme::CROSS));
        match &self.bar {
            Some(bar) => bar.finish_with_message(line),
            None => eprintln!("{line}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_interactive_has_no_bar() {
        let s = build_spinner("working".to_string(), false);
        assert!(s.bar.is_none());
    }

    #[test]
    fn interactive_has_a_bar() {
        let s = build_spinner("working".to_string(), true);
        assert!(s.bar.is_some());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pensieve-cli --bins ux::spinner::tests`
Expected: both tests FAIL (panic: `not yet implemented`).

- [ ] **Step 4: Implement `build_spinner`**

Replace the `todo!(...)` body:

```rust
fn build_spinner(msg: String, interactive: bool) -> Spinner {
    if !interactive {
        eprintln!("{msg}...");
        return Spinner { bar: None };
    }
    let bar = ProgressBar::new_spinner();
    bar.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .expect("static spinner template is valid"),
    );
    bar.set_message(msg);
    bar.enable_steady_tick(Duration::from_millis(100));
    Spinner { bar: Some(bar) }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins ux::spinner::tests`
Expected: both tests PASS. (Note: `cargo test` runs with stderr captured/non-tty, so `spinner()` itself — as opposed to `build_spinner` — would always take the non-interactive branch in this environment; that's expected and is exactly why the tests call `build_spinner` directly with an explicit bool instead of `spinner`.)

- [ ] **Step 6: Commit**

```bash
git add crates/pensieve-cli/src/ux/mod.rs crates/pensieve-cli/src/ux/spinner.rs
git commit -m "feat(cli): add ux::spinner — TTY-aware spinner/progress wrapper"
```

---

### Task 5: `error.rs` (unified error rendering + hints)

**Files:**
- Modify: `crates/pensieve-cli/src/ux/mod.rs`
- Create: `crates/pensieve-cli/src/ux/error.rs`

**Interfaces:**
- Consumes: `super::theme::{apply, CROSS}` (Task 1, `apply` is `pub(crate)` specifically so this module can render deterministically without touching the global flag).
- Produces: `ux::error::print_error(err: &anyhow::Error)` — called once from `main()` in Task 6.

- [ ] **Step 1: Add the module declaration**

In `crates/pensieve-cli/src/ux/mod.rs`, add below `pub(crate) mod spinner;`:

```rust
pub(crate) mod error;
```

- [ ] **Step 2: Write `crates/pensieve-cli/src/ux/error.rs` with failing tests**

```rust
//! Unified error presentation. `print_error` is called exactly once, from
//! `main()`'s top-level error path, so every command gets consistent
//! error formatting — a readable cause chain instead of an `anyhow`
//! `{:?}` debug dump — without any per-command changes.

use console::Style;

use super::theme;

/// Prints `err`'s full cause chain to stderr as human-readable text, with
/// an actionable hint appended for a handful of well-known failure
/// signatures.
pub(crate) fn print_error(err: &anyhow::Error) {
    eprint!("{}", render_error(err, theme::color_enabled()));
}

fn render_error(err: &anyhow::Error, color: bool) -> String {
    todo!("render_error({err}, color={color})")
}

/// Not exhaustive by design — extend as new failure signatures are
/// noticed in real usage.
fn hint_for(err: &anyhow::Error) -> Option<&'static str> {
    let text = err
        .chain()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if text.contains("connection refused") {
        Some("is `pensieve serve` running? check the URL from `pensieve status`")
    } else if text.contains("401") || text.contains("unauthorized") {
        Some("run `pensieve connect` to re-authenticate")
    } else if text.contains("403") || text.contains("forbidden") {
        Some("your token may lack permission for this operation")
    } else if text.contains("404") || text.contains("not found") {
        Some("double-check the id/name — it may not exist")
    } else if text.contains("timed out") || text.contains("timeout") {
        Some("the server took too long to respond — try again or check its logs")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn hint_for_connection_refused() {
        let err = anyhow!("Connection refused (os error 61)");
        assert_eq!(
            hint_for(&err),
            Some("is `pensieve serve` running? check the URL from `pensieve status`")
        );
    }

    #[test]
    fn hint_for_unauthorized() {
        let err = anyhow!("request failed: 401 Unauthorized");
        assert_eq!(hint_for(&err), Some("run `pensieve connect` to re-authenticate"));
    }

    #[test]
    fn hint_for_unknown_returns_none() {
        let err = anyhow!("something entirely novel broke");
        assert_eq!(hint_for(&err), None);
    }

    #[test]
    fn render_error_no_color_snapshot() {
        let err = anyhow!("connection refused").context("failed to reach pensieve server");
        insta::assert_snapshot!(render_error(&err, false));
    }

    #[test]
    fn render_error_includes_full_chain() {
        let err = anyhow!("root cause").context("middle layer").context("top layer");
        let rendered = render_error(&err, false);
        assert!(rendered.contains("top layer"));
        assert!(rendered.contains("middle layer"));
        assert!(rendered.contains("root cause"));
    }
}
```

- [ ] **Step 3: Run tests to verify `render_error_*` tests fail**

Run: `cargo test -p pensieve-cli --bins ux::error::tests`
Expected: `render_error_no_color_snapshot` and `render_error_includes_full_chain` FAIL (panic: `not yet implemented`). The three `hint_for_*` tests PASS already.

- [ ] **Step 4: Implement `render_error`**

Replace the `todo!(...)` body:

```rust
fn render_error(err: &anyhow::Error, color: bool) -> String {
    let mut out = String::new();
    let header = theme::apply(
        &format!("{} Error:", theme::CROSS),
        Style::new().red().bold(),
        color,
    );
    out.push_str(&header);
    out.push('\n');
    for (i, cause) in err.chain().enumerate() {
        if i == 0 {
            out.push_str(&format!("  {cause}\n"));
        } else {
            out.push_str(&format!("  caused by: {cause}\n"));
        }
    }
    if let Some(hint) = hint_for(err) {
        let hint_line = theme::apply(&format!("  hint: {hint}"), Style::new().dim(), color);
        out.push_str(&hint_line);
        out.push('\n');
    }
    out
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins ux::error::tests`
Expected: `render_error_includes_full_chain` and the three `hint_for_*` tests PASS. `render_error_no_color_snapshot` FAILS with an insta message like `snapshot assertion for 'render_error_no_color_snapshot' failed` and creates a pending file at `crates/pensieve-cli/src/ux/snapshots/pensieve_cli__ux__error__tests__render_error_no_color_snapshot.snap.new` — this is expected; insta always fails on the first run since no accepted snapshot exists yet.

- [ ] **Step 6: Review and accept the snapshot**

Run: `cat crates/pensieve-cli/src/ux/snapshots/pensieve_cli__ux__error__tests__render_error_no_color_snapshot.snap.new`
Expected output body (the `---` frontmatter above it is insta metadata). Note the hint line: the
test's error text contains "connection refused", so `hint_for` matches that branch:
```
✗ Error:
  failed to reach pensieve server
  caused by: connection refused
  hint: is `pensieve serve` running? check the URL from `pensieve status`
```
If it matches, accept it: `cargo insta accept` (or manually `mv crates/pensieve-cli/src/ux/snapshots/pensieve_cli__ux__error__tests__render_error_no_color_snapshot.snap.new crates/pensieve-cli/src/ux/snapshots/pensieve_cli__ux__error__tests__render_error_no_color_snapshot.snap`, then remove the insta metadata's `.new` suffix from the retained filename if `cargo insta` isn't installed as a cargo subcommand).

- [ ] **Step 7: Run tests to verify they all pass**

Run: `cargo test -p pensieve-cli --bins ux::error::tests`
Expected: all 5 tests PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/pensieve-cli/src/ux/mod.rs crates/pensieve-cli/src/ux/error.rs crates/pensieve-cli/src/ux/snapshots/
git commit -m "feat(cli): add ux::error — unified cause-chain rendering + hints"
```

---

### Task 6: Wire `ux` into `main()` — global `--no-color`, styled `--help`, unified error path

**Files:**
- Modify: `crates/pensieve-cli/src/main.rs:47-60` (Cli struct)
- Modify: `crates/pensieve-cli/src/main.rs:494-518` (entry point)

**Interfaces:**
- Consumes: `ux::theme::init` (Task 1), `ux::error::print_error` (Task 5).
- Produces: every command now goes through unified error rendering + gets colored `--help`; `--no-color` flag available globally (`pensieve --no-color <cmd>` or `pensieve <cmd> --no-color`).

- [ ] **Step 1: Add `--no-color` flag + clap `Styles` to the `Cli` struct**

The current `Cli` struct (`crates/pensieve-cli/src/main.rs:47-60`):

```rust
#[derive(Debug, Parser)]
#[command(name = "pensieve", about = "Pensieve CLI — client queries + admin operations")]
struct Cli {
    /// Postgres connection URL (admin subcommands only).
    #[arg(
        long,
        env = "PENSIEVE_CATALOG_URL",
        default_value = "postgres://pensieve:pensieve_dev@localhost:5433/pensieve"
    )]
    catalog_url: String,

    #[command(subcommand)]
    command: Command,
}
```

Replace with:

```rust
#[derive(Debug, Parser)]
#[command(
    name = "pensieve",
    about = "Pensieve CLI — client queries + admin operations",
    styles = clap::builder::Styles::styled()
)]
struct Cli {
    /// Postgres connection URL (admin subcommands only).
    #[arg(
        long,
        env = "PENSIEVE_CATALOG_URL",
        default_value = "postgres://pensieve:pensieve_dev@localhost:5433/pensieve"
    )]
    catalog_url: String,

    /// Disable colored/styled output (also respects NO_COLOR and non-TTY stdout).
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Command,
}
```

- [ ] **Step 2: Split `main()` into a thin entry point + `run()`**

The current entry point (`crates/pensieve-cli/src/main.rs:494-518`):

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // `pensieve serve` sets up a richer subscriber that includes the OTel self-trace
    // layer; all other subcommands use a plain fmt subscriber.
    let self_trace_handle = if matches!(cli.command, Command::Serve { .. }) {
        Some(pensieve_local::setup_serve_tracing())
    } else {
        // Logs go to STDERR so command output (and the `pensieve mcp` stdio
        // protocol channel) stays clean on stdout.
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new("info,sqlx=warn,hyper=warn")
                }),
            )
            .with_target(false)
            .try_init()
            .ok();
        None
    };

    match cli.command {
```

Replace with:

```rust
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    ux::theme::init(cli.no_color);
    if let Err(err) = run(cli).await {
        ux::error::print_error(&err);
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    // `pensieve serve` sets up a richer subscriber that includes the OTel self-trace
    // layer; all other subcommands use a plain fmt subscriber.
    let self_trace_handle = if matches!(cli.command, Command::Serve { .. }) {
        Some(pensieve_local::setup_serve_tracing())
    } else {
        // Logs go to STDERR so command output (and the `pensieve mcp` stdio
        // protocol channel) stays clean on stdout.
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new("info,sqlx=warn,hyper=warn")
                }),
            )
            .with_target(false)
            .try_init()
            .ok();
        None
    };

    match cli.command {
```

**Everything from `match cli.command {` to the end of the file is unchanged** — the big match expression and its closing `}}` (one closes the match, one closes what is now `run` instead of `main`) stay exactly as they are. Do not touch anything below this point.

- [ ] **Step 3: Build**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly. If it complains about `cli.catalog_url` or any other field being used after a partial move inside `run`, that's pre-existing behavior unchanged from before this refactor — the body of `run` is byte-for-byte the old body of `main`, just under a new name and taking `cli` as a parameter instead of calling `Cli::parse()` itself.

- [ ] **Step 4: Manual verification — styled help**

Run: `./target/debug/pensieve-cli --help`
Expected: section headers (e.g. "Commands:", "Options:") and option names render bold/underlined in a real terminal (colors won't show if this command's output is piped/captured, which is fine — that's the point of the styling being conditional).

- [ ] **Step 5: Manual verification — unified error path**

`PENSIEVE_SERVER_URL` overrides any saved `~/.pensieve/config.json` (see `effective_config()` in
`crates/pensieve-cli/src/client.rs:63-80`), so pointing it at a port nothing listens on deterministically
reproduces a connection failure without touching real saved config:

```bash
PENSIEVE_SERVER_URL=http://127.0.0.1:59999 ./target/debug/pensieve-cli recall "test" 2>&1 || true
```

Expected: stderr shows something like:

```
✗ Error:
  <underlying reqwest connection-refused error text>
  caused by: ...
  hint: is `pensieve serve` running? check the URL from `pensieve status`
```

instead of a raw `Error: <big Rust Debug dump with backtraces>`. Exit code is `1`.

- [ ] **Step 6: Manual verification — `--no-color` and `NO_COLOR`**

Run:

```bash
PENSIEVE_SERVER_URL=http://127.0.0.1:59999 ./target/debug/pensieve-cli --no-color recall "test" 2>&1 | cat
PENSIEVE_SERVER_URL=http://127.0.0.1:59999 NO_COLOR=1 ./target/debug/pensieve-cli recall "test" 2>&1 | cat
```

Expected: both produce plain, uncolored error text (no ANSI escape codes) — pipe through `cat -v` if unsure and confirm no `^[` sequences appear.

- [ ] **Step 7: Run the full test suite for the crate**

Run: `cargo test -p pensieve-cli --bins`
Expected: all `ux::*` tests plus every pre-existing `pensieve-cli` test PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/pensieve-cli/src/main.rs
git commit -m "feat(cli): wire ux — styled --help, global --no-color, unified error path"
```

---

## Post-plan note

`ux::table`, `ux::format`, and `ux::spinner` are built and tested but have no callers yet — that's intentional (see Global Constraints). The next plan (U1) wires them into `recall`, `remember`, `entity`, `distill`, and `status` in `crates/pensieve-cli/src/plugin.rs` and `crates/pensieve-cli/src/main.rs`.
