# CLI UX U2 — Data Source & Ingest Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the `ux` toolkit (built in U0, extended slightly here, wired into memory commands in U1 — all already merged to `main`) into pensieve's data-source and ingest CLI commands in `crates/pensieve-cli/src/datasource.rs`: `datasource list/add/pause/resume/trigger/remove` and `ingest status/tail`. This is the toolkit's first production use of `ux::table` and `ux::table::status_cell` (built in U0, unused ever since).

**Architecture:** One small addition to the toolkit itself (`ux::theme::ok`/`bad` — a glyph+color convenience the U1 final review recommended, since the hand-rolled `theme::success(&format!("{} ...", CHECK, ...))` shape was already repeating), then five tasks editing `crates/pensieve-cli/src/datasource.rs` command-by-command. Row-building logic that's pure JSON-in/String-out gets extracted into small private helpers and unit-tested directly (matching the pattern from U1's `remember_success_line`/`entity_success_line`); the two list-style commands additionally move from manual fixed-width `println!` columns to `ux::table`.

**Tech Stack:** Rust. Consumes `crate::ux::{theme, table}` — `theme` gets one small addition in Task 1, `table` is consumed as-is from U0. Also uses `comfy_table::{Cell, Color}` directly (already a dependency since U0; not previously imported in `datasource.rs`). No `Cargo.toml` change.

## Global Constraints

- Full spec: `docs/superpowers/specs/2026-07-01-cli-ux-overhaul-design.md`. This plan implements **U2 only** — `datasource list/add/pause/resume/trigger/remove` and `ingest status/tail`. U3 (everything else) and U4 (ratatui) are separate future plans.
- **`datasource show` is explicitly OUT OF SCOPE.** It prints `serde_json::to_string_pretty` — raw, tool-friendly JSON meant for scripting/debugging. Coloring or restructuring it would work against that use case, the same reasoning that keeps `recall --json` untouched in U1.
- **`ingest push` is explicitly OUT OF SCOPE.** Its implementation lives in `crates/pensieve-cli/src/plugin.rs::ingest_push` (not `datasource.rs`), and — like `distill` before U1 restyled it — it's silent by default (`"Quiet on success unless asked for detail (hooks pipe this to /dev/null)"`, per its own comment) because Claude Code hooks invoke it. Unlike `distill`, it's a single fast POST, not a multi-second LLM call, so there's no dead-air problem a spinner would fix. Leave it alone.
- **`poll_status`** (called by `cmd_add` when `--start` is passed) is a separate function in `datasource.rs` whose source this plan's author did not inspect — its internal output is out of scope here. Do not modify it as part of any task below.
- `pensieve-cli` is a **binary crate** — internal visibility is `pub(crate)` for crate-wide items, plain (private) for file-local helpers.
- Run tests with `cargo test -p pensieve-cli --bins <filter>` (no `--lib` target exists for this crate).
- `crates/pensieve-cli/src/datasource.rs` has **no existing `#[cfg(test)] mod tests` block** and **no existing `use crate::ux;` import** — Task 2 creates both; Tasks 3-6 append to the same test module and reuse the same import.
- No new dependency, no `Cargo.toml` change — `comfy-table` is already a direct dependency (added in U0).

---

### Task 1: `ux::theme` — add `ok`/`bad` glyph+semantic convenience helpers

**Files:**
- Modify: `crates/pensieve-cli/src/ux/theme.rs`

**Interfaces:**
- Consumes: existing `success`, `error`, `CHECK`, `CROSS` in the same file.
- Produces: `pub(crate) fn ok(text: &str) -> String`, `pub(crate) fn bad(text: &str) -> String` — used by every task in this plan (2 through 6).

- [ ] **Step 1: Write the failing tests**

At the end of the existing `#[cfg(test)] mod tests` block in `crates/pensieve-cli/src/ux/theme.rs` (after the last test, `stderr_color_enabled_defaults_true_when_uninitialized`), add:

```rust

    #[test]
    fn ok_prefixes_check_glyph_and_matches_success_styling() {
        let line = ok("done");
        assert!(line.contains("done"));
        assert!(line.contains(CHECK));
        assert_eq!(line, success(&format!("{CHECK} done")));
    }

    #[test]
    fn bad_prefixes_cross_glyph_and_matches_error_styling() {
        let line = bad("failed");
        assert!(line.contains("failed"));
        assert!(line.contains(CROSS));
        assert_eq!(line, error(&format!("{CROSS} failed")));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pensieve-cli --bins ux::theme::tests`
Expected: `ok_prefixes_check_glyph_and_matches_success_styling` and `bad_prefixes_cross_glyph_and_matches_error_styling` FAIL to compile (`ok`/`bad` don't exist yet — a compile-error FAIL, same as this skill's own "FAIL with 'function not defined'" example). The existing tests are unaffected.

- [ ] **Step 3: Implement `ok` and `bad`**

In `crates/pensieve-cli/src/ux/theme.rs`, immediately after the existing `pub(crate) fn accent(text: &str) -> String { ... }` function and before the `pub(crate) const CHECK: &str = "✓";` block, add:

```rust
pub(crate) fn ok(text: &str) -> String {
    success(&format!("{CHECK} {text}"))
}

pub(crate) fn bad(text: &str) -> String {
    error(&format!("{CROSS} {text}"))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins ux::theme::tests`
Expected: all tests in `ux::theme::tests` PASS (the 2 new ones plus all pre-existing ones).

- [ ] **Step 5: Build check**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly. `ok`/`bad` will show as unused (`dead_code`) until Task 2 consumes them — expected and fine.

- [ ] **Step 6: Commit**

```bash
git add crates/pensieve-cli/src/ux/theme.rs
git commit -m "feat(cli): add ux::theme::ok/bad glyph+semantic convenience helpers"
```

---

### Task 2: `datasource list` — colored table via `ux::table`

**Files:**
- Modify: `crates/pensieve-cli/src/datasource.rs:19-22` (imports)
- Modify: `crates/pensieve-cli/src/datasource.rs:295-321` (`cmd_list`)
- Create (in the same file): a new `#[cfg(test)] mod tests` block at the end of `crates/pensieve-cli/src/datasource.rs`

**Interfaces:**
- Consumes: `crate::ux::table::{table, status_cell}` (from U0), `comfy_table::Cell` (external crate, already a dependency).
- Produces: `fn data_source_row(c: &serde_json::Value) -> [String; 4]` — a private, pure helper. No other task in this plan depends on it (each list-style task gets its own row helper), but it establishes the pattern Task 5 follows for `ingest status`.

- [ ] **Step 1: Add imports**

The current imports at the top of `crates/pensieve-cli/src/datasource.rs` (lines 19-22):

```rust
use crate::client::{self, ClientConfig};
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::{json, Value};
```

Add two lines:

```rust
use crate::client::{self, ClientConfig};
use crate::ux;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use comfy_table::Cell;
use serde_json::{json, Value};
```

- [ ] **Step 2: Write the failing tests**

At the very end of `crates/pensieve-cli/src/datasource.rs` (after the last line, currently the closing `}` of `http_delete`), add:

```rust

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_source_row_enabled_with_last_success() {
        let c = json!({
            "type": "github",
            "name": "pensieve",
            "enabled": true,
            "last_success_at": "2026-07-01T00:00:00Z",
        });
        assert_eq!(
            data_source_row(&c),
            [
                "github".to_string(),
                "pensieve".to_string(),
                "enabled".to_string(),
                "2026-07-01T00:00:00Z".to_string(),
            ]
        );
    }

    #[test]
    fn data_source_row_paused_falls_back_to_last_run() {
        let c = json!({
            "type": "gitlab",
            "name": "proj",
            "enabled": false,
            "last_run_at": "2026-06-01T00:00:00Z",
        });
        assert_eq!(
            data_source_row(&c),
            [
                "gitlab".to_string(),
                "proj".to_string(),
                "paused".to_string(),
                "2026-06-01T00:00:00Z".to_string(),
            ]
        );
    }

    #[test]
    fn data_source_row_missing_fields_use_placeholders() {
        let c = json!({});
        assert_eq!(
            data_source_row(&c),
            [
                "?".to_string(),
                "?".to_string(),
                "enabled".to_string(),
                "never".to_string(),
            ]
        );
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: all 3 tests FAIL to compile (`data_source_row` doesn't exist yet).

- [ ] **Step 4: Implement `data_source_row` and rewrite `cmd_list`**

The current `cmd_list` (lines 295-321):

```rust
async fn cmd_list(cfg: &ClientConfig) -> Result<()> {
    let items = list_data_sources(cfg).await?;
    if items.is_empty() {
        println!("(no data sources registered)");
        return Ok(());
    }
    println!(
        "{:<14}  {:<22}  {:<10}  {}",
        "TYPE", "NAME", "STATUS", "LAST"
    );
    for c in items {
        let kind = c.get("type").and_then(Value::as_str).unwrap_or("?");
        let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
        let status = if c.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
            "enabled"
        } else {
            "paused"
        };
        let last = c
            .get("last_success_at")
            .and_then(Value::as_str)
            .or_else(|| c.get("last_run_at").and_then(Value::as_str))
            .unwrap_or("never");
        println!("{kind:<14}  {name:<22}  {status:<10}  {last}");
    }
    Ok(())
}
```

Replace with:

```rust
async fn cmd_list(cfg: &ClientConfig) -> Result<()> {
    let items = list_data_sources(cfg).await?;
    if items.is_empty() {
        println!("(no data sources registered)");
        return Ok(());
    }
    let mut t = ux::table::table(vec!["TYPE", "NAME", "STATUS", "LAST"]);
    for c in &items {
        let [kind, name, status, last] = data_source_row(c);
        t.add_row(vec![
            Cell::new(kind),
            Cell::new(name),
            ux::table::status_cell(&status),
            Cell::new(last),
        ]);
    }
    println!("{t}");
    Ok(())
}

/// Extracts the four display fields for one `datasource list` row. Pure and
/// tested directly — `cmd_list` just formats what this returns.
fn data_source_row(c: &Value) -> [String; 4] {
    let kind = c
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let name = c
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let status = if c.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
        "enabled"
    } else {
        "paused"
    }
    .to_string();
    let last = c
        .get("last_success_at")
        .and_then(Value::as_str)
        .or_else(|| c.get("last_run_at").and_then(Value::as_str))
        .unwrap_or("never")
        .to_string();
    [kind, name, status, last]
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: all 3 tests PASS.

- [ ] **Step 6: Build check**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly. `dead_code` warnings for `ux::table::{table, status_cell}` should now be GONE (first real callers).

- [ ] **Step 7: Commit**

```bash
git add crates/pensieve-cli/src/datasource.rs
git commit -m "feat(cli): restyle datasource list — colored table via ux::table"
```

---

### Task 3: `datasource add` — colored success line

**Files:**
- Modify: `crates/pensieve-cli/src/datasource.rs` (end of `cmd_add`, currently around lines 470-484 — re-locate by searching for the exact text below, since Task 2 shifted line numbers earlier in the file)
- Modify: the `#[cfg(test)] mod tests` block created in Task 2 (append tests)

**Interfaces:**
- Consumes: `crate::ux::theme::ok` (from Task 1).
- Produces: `fn add_success_line(name: &str, kind: &str, id: &str) -> String` — private, pure, tested. No other task depends on it.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block in `crates/pensieve-cli/src/datasource.rs` (after `data_source_row_missing_fields_use_placeholders`'s closing `}`, still inside `mod tests { ... }`):

```rust

    #[test]
    fn add_success_line_includes_name_kind_and_id() {
        let line = add_success_line("pensieve", "github", "abc-123");
        assert!(line.contains("Created data source pensieve (github) → id=abc-123"));
        assert!(line.contains(ux::theme::CHECK));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: `add_success_line_includes_name_kind_and_id` FAILS to compile (`add_success_line` doesn't exist yet). The other 3 tests still PASS.

- [ ] **Step 3: Implement `add_success_line` and wire it in**

Find this exact block near the end of `cmd_add` (the `println!` calls right after `http_post(cfg, "/v1/data-sources", &create_body).await?;` resolves the new `id`):

```rust
    println!("Created data source {name} ({kind}) → id={id}");
    println!("  database:      {db}");
    println!("  credential:    {credential_id}");
    println!("  schedule:      every {}ms", schedule_ms);
```

Replace the first line only (leave the three `database:`/`credential:`/`schedule:` detail lines exactly as they are — plain text, matching how `pensieve status`'s `Endpoint:` line also stayed plain in U1):

```rust
    println!("{}", add_success_line(&name, kind, &id));
    println!("  database:      {db}");
    println!("  credential:    {credential_id}");
    println!("  schedule:      every {}ms", schedule_ms);
```

Then add the helper function immediately after `cmd_add`'s closing `}` (before whatever function currently follows it):

```rust

/// Builds the headline success message for `datasource add`. Pure and
/// tested directly.
fn add_success_line(name: &str, kind: &str, id: &str) -> String {
    ux::theme::ok(&format!("Created data source {name} ({kind}) → id={id}"))
}
```

Note: `name` in `cmd_add` at this point is a `String` (bound via the big `match source { ... }` destructuring earlier in the function) — pass it as `&name`. `kind` is already a `&'static str` literal (`"github"`/`"gitlab"`/`"bitbucket"`) — pass it directly. `id` is a `String` — pass as `&id`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: all 4 tests PASS.

- [ ] **Step 5: Build check**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly.

- [ ] **Step 6: Commit**

```bash
git add crates/pensieve-cli/src/datasource.rs
git commit -m "feat(cli): restyle datasource add — colored success line"
```

---

### Task 4: `datasource pause/resume/trigger` + `datasource remove` — colored messages

**Files:**
- Modify: `crates/pensieve-cli/src/datasource.rs` (`cmd_simple_op` and `cmd_remove` — locate by exact text, line numbers have shifted from Tasks 2-3)
- Modify: the `#[cfg(test)] mod tests` block (append tests)

**Interfaces:**
- Consumes: `crate::ux::theme::{ok, muted}` (from Task 1 and U0 respectively).
- Produces: `fn simple_op_success_line(op: &str, id: &str) -> String` — private, pure, tested.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust

    #[test]
    fn simple_op_success_line_pause() {
        let line = simple_op_success_line("pause", "abc");
        assert!(line.contains("paused data source abc"));
        assert!(line.contains(ux::theme::CHECK));
    }

    #[test]
    fn simple_op_success_line_resume() {
        let line = simple_op_success_line("resume", "abc");
        assert!(line.contains("resumed data source abc"));
    }

    #[test]
    fn simple_op_success_line_trigger() {
        let line = simple_op_success_line("trigger", "abc");
        assert!(line.contains("triggered a run for data source abc"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: the 3 new tests FAIL to compile (`simple_op_success_line` doesn't exist yet). The other 4 tests still PASS.

- [ ] **Step 3: Implement `simple_op_success_line` and update both functions**

The current `cmd_simple_op`:

```rust
async fn cmd_simple_op(cfg: &ClientConfig, name_or_id: &str, op: &str) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    http_post(cfg, &format!("/v1/data-sources/{id}/{op}"), &json!({})).await?;
    // Past-tense per op — "{op}d" only reads right for pause/resume.
    let past = match op {
        "trigger" => "triggered a run for",
        "pause" => "paused",
        "resume" => "resumed",
        other => other,
    };
    println!("{past} data source {id}");
    Ok(())
}
```

Replace with:

```rust
async fn cmd_simple_op(cfg: &ClientConfig, name_or_id: &str, op: &str) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    http_post(cfg, &format!("/v1/data-sources/{id}/{op}"), &json!({})).await?;
    println!("{}", simple_op_success_line(op, &id));
    Ok(())
}

/// Builds the success message for pause/resume/trigger. Pure and tested
/// directly.
fn simple_op_success_line(op: &str, id: &str) -> String {
    // Past-tense per op — "{op}d" only reads right for pause/resume.
    let past = match op {
        "trigger" => "triggered a run for",
        "pause" => "paused",
        "resume" => "resumed",
        other => other,
    };
    ux::theme::ok(&format!("{past} data source {id}"))
}
```

The current `cmd_remove`:

```rust
async fn cmd_remove(cfg: &ClientConfig, name_or_id: &str, yes: bool) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    if !yes {
        use std::io::{stdin, stdout, Write};
        print!("Delete data source {id}? [y/N] ");
        stdout().flush().ok();
        let mut s = String::new();
        stdin().read_line(&mut s).ok();
        if !matches!(s.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("aborted");
            return Ok(());
        }
    }
    http_delete(cfg, &format!("/v1/data-sources/{id}")).await?;
    println!("removed data source {id}");
    Ok(())
}
```

Replace the two `println!` calls (leave the confirmation `print!`/`stdin`/`matches!` logic exactly as-is):

```rust
async fn cmd_remove(cfg: &ClientConfig, name_or_id: &str, yes: bool) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    if !yes {
        use std::io::{stdin, stdout, Write};
        print!("Delete data source {id}? [y/N] ");
        stdout().flush().ok();
        let mut s = String::new();
        stdin().read_line(&mut s).ok();
        if !matches!(s.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("{}", ux::theme::muted("aborted"));
            return Ok(());
        }
    }
    http_delete(cfg, &format!("/v1/data-sources/{id}")).await?;
    println!("{}", ux::theme::ok(&format!("removed data source {id}")));
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: all 7 tests PASS.

- [ ] **Step 5: Build check**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly.

- [ ] **Step 6: Commit**

```bash
git add crates/pensieve-cli/src/datasource.rs
git commit -m "feat(cli): restyle datasource pause/resume/trigger/remove — colored messages"
```

---

### Task 5: `ingest status` — colored table via `ux::table`

**Files:**
- Modify: `crates/pensieve-cli/src/datasource.rs:19-24` (imports — extend the `comfy_table` import with `Color`)
- Modify: `crates/pensieve-cli/src/datasource.rs` (`cmd_ingest_status` — locate by exact text)
- Modify: the `#[cfg(test)] mod tests` block (append tests)

**Interfaces:**
- Consumes: `crate::ux::table::table` (from U0), `comfy_table::{Cell, Color}`.
- Produces: `fn ingest_status_row(c: &serde_json::Value) -> [String; 5]` — private, pure, tested.

- [ ] **Step 1: Extend the `comfy_table` import**

Current (after Task 2's edit):

```rust
use comfy_table::Cell;
```

Change to:

```rust
use comfy_table::{Cell, Color};
```

- [ ] **Step 2: Write the failing tests**

Append to the `mod tests` block:

```rust

    #[test]
    fn ingest_status_row_extracts_all_fields() {
        let c = json!({
            "name": "pensieve",
            "type": "github",
            "last_run_at": "t1",
            "last_success_at": "t2",
            "last_error": "boom",
        });
        assert_eq!(
            ingest_status_row(&c),
            [
                "pensieve".to_string(),
                "github".to_string(),
                "t1".to_string(),
                "t2".to_string(),
                "boom".to_string(),
            ]
        );
    }

    #[test]
    fn ingest_status_row_missing_fields_use_placeholders() {
        let c = json!({});
        assert_eq!(
            ingest_status_row(&c),
            [
                "?".to_string(),
                "?".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
            ]
        );
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: the 2 new tests FAIL to compile (`ingest_status_row` doesn't exist yet). The other 7 tests still PASS.

- [ ] **Step 4: Implement `ingest_status_row` and rewrite `cmd_ingest_status`**

The current tail of `cmd_ingest_status` (from the `println!` header through the end of the loop — the earlier part of the function that builds `items` is unchanged):

```rust
    println!(
        "{:<22}  {:<14}  {:<30}  {:<30}  {}",
        "NAME", "TYPE", "LAST_RUN", "LAST_SUCCESS", "LAST_ERROR"
    );
    for c in items {
        let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
        let kind = c.get("type").and_then(Value::as_str).unwrap_or("?");
        let lr = c.get("last_run_at").and_then(Value::as_str).unwrap_or("-");
        let ls = c
            .get("last_success_at")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let le = c.get("last_error").and_then(Value::as_str).unwrap_or("-");
        println!("{name:<22}  {kind:<14}  {lr:<30}  {ls:<30}  {le}");
    }
    Ok(())
}
```

Replace with:

```rust
    let mut t = ux::table::table(vec!["NAME", "TYPE", "LAST_RUN", "LAST_SUCCESS", "LAST_ERROR"]);
    for c in &items {
        let [name, kind, lr, ls, le] = ingest_status_row(c);
        let le_cell = if le == "-" {
            Cell::new(le)
        } else {
            Cell::new(le).fg(Color::Red)
        };
        t.add_row(vec![
            Cell::new(name),
            Cell::new(kind),
            Cell::new(lr),
            Cell::new(ls),
            le_cell,
        ]);
    }
    println!("{t}");
    Ok(())
}

/// Extracts the five display fields for one `ingest status` row. Pure and
/// tested directly.
fn ingest_status_row(c: &Value) -> [String; 5] {
    let name = c
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let kind = c
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let lr = c
        .get("last_run_at")
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string();
    let ls = c
        .get("last_success_at")
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string();
    let le = c
        .get("last_error")
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string();
    [name, kind, lr, ls, le]
}
```

Note: this task changes `for c in items` to `for c in &items` (iterate by reference) since `ingest_status_row` takes `&Value` — `items` isn't used after the loop, so this is a safe, mechanical change.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: all 9 tests PASS.

- [ ] **Step 6: Build check**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly.

- [ ] **Step 7: Commit**

```bash
git add crates/pensieve-cli/src/datasource.rs
git commit -m "feat(cli): restyle ingest status — colored table with red LAST_ERROR"
```

---

### Task 6: `ingest tail` — colored streaming lines

**Files:**
- Modify: `crates/pensieve-cli/src/datasource.rs` (`cmd_ingest_tail` — locate by exact text)
- Modify: the `#[cfg(test)] mod tests` block (append tests)

**Interfaces:**
- Consumes: `crate::ux::theme::{ok, bad}` (from Task 1).
- Produces: `fn tail_line(lr: &str, name: &str, err: &str) -> String` — private, pure, tested. This is the last task in this plan.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust

    #[test]
    fn tail_line_ok_when_no_error() {
        let line = tail_line("2026-07-01T00:00:00Z", "pensieve", "");
        assert!(line.contains("[2026-07-01T00:00:00Z] pensieve: ok"));
        assert!(line.contains(ux::theme::CHECK));
    }

    #[test]
    fn tail_line_error_includes_message() {
        let line = tail_line("2026-07-01T00:00:00Z", "pensieve", "connection refused");
        assert!(line.contains("[2026-07-01T00:00:00Z] pensieve: ERROR connection refused"));
        assert!(line.contains(ux::theme::CROSS));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: the 2 new tests FAIL to compile (`tail_line` doesn't exist yet). The other 9 tests still PASS.

- [ ] **Step 3: Implement `tail_line` and wire it in**

Find this exact block inside `cmd_ingest_tail`'s loop:

```rust
            let prev = last_seen.get(&id).cloned();
            if prev.as_ref() != Some(&(lr.clone(), le.clone())) && !lr.is_empty() {
                last_seen.insert(id.clone(), (lr.clone(), le.clone()));
                if le.is_empty() {
                    println!("[{lr}] {name}: ok");
                } else {
                    println!("[{lr}] {name}: ERROR {le}");
                }
            }
```

Replace with:

```rust
            let prev = last_seen.get(&id).cloned();
            if prev.as_ref() != Some(&(lr.clone(), le.clone())) && !lr.is_empty() {
                last_seen.insert(id.clone(), (lr.clone(), le.clone()));
                println!("{}", tail_line(&lr, name, &le));
            }
```

Then add the helper function immediately after `cmd_ingest_tail`'s closing `}` (before whatever function currently follows it):

```rust

/// Builds one `ingest tail` line: green "ok" or red "ERROR {msg}". Pure and
/// tested directly.
fn tail_line(lr: &str, name: &str, err: &str) -> String {
    if err.is_empty() {
        ux::theme::ok(&format!("[{lr}] {name}: ok"))
    } else {
        ux::theme::bad(&format!("[{lr}] {name}: ERROR {err}"))
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pensieve-cli --bins datasource::tests`
Expected: all 11 tests PASS.

- [ ] **Step 5: Build check**

Run: `cargo build -p pensieve-cli`
Expected: builds cleanly.

- [ ] **Step 6: Run the full test suite (regression check)**

Run: `cargo test -p pensieve-cli --bins`
Expected: 65 passed, 0 failed (the 52 pre-existing tests from U0/U1 + 2 new `ux::theme::tests` from Task 1 + 11 new `datasource::tests` from Tasks 2-6). If the count differs, something regressed or a test was silently dropped — investigate before committing.

- [ ] **Step 7: Commit**

```bash
git add crates/pensieve-cli/src/datasource.rs
git commit -m "feat(cli): restyle ingest tail — colored streaming lines"
```

---

## Post-plan note

After this plan, `ux::table` and `ux::table::status_cell` gain their first real callers (dead_code warnings for them should disappear from `cargo build -p pensieve-cli`'s output). Expected *remaining* dead_code warnings: `ux::format::relative_time` (still no consumer — no timestamp-listing command has been touched yet; `sessions list` in U3 is a candidate), `ux::theme::{info, accent, ARROW}` (reserved, no consumer in any phase's current scope). `datasource show`'s pretty-JSON output and `ingest push`'s hook-silent output are unchanged by design (see Global Constraints) — do not treat their continued plainness as a gap in a future review.
