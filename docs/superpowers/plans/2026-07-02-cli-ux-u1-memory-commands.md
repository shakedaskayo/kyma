# CLI UX U1 — Memory Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the `ux` toolkit (built in U0, already merged to `main`) into `kyma recall`, `kyma remember`, `kyma entity`, `kyma distill`, and `kyma status` — the memory commands the user named explicitly — replacing their plain `println!` output with colored, truncation-aware, spinner-backed output.

**Architecture:** No new modules. Every task edits an existing function in `crates/kyma-cli/src/plugin.rs` (recall/remember/entity/distill) or `crates/kyma-cli/src/main.rs` (status) to call `ux::theme`/`ux::format`/`ux::spinner` functions that already exist and are already tested from U0. Where a task's changed logic is a pure function (string formatting from already-fetched data), it gets extracted into a small private helper and unit-tested directly. Where a task's changed logic is inseparable from a live network call (all of `status`, most of `distill`), verification is manual (build + run against controlled/unreachable endpoints), matching how U0's own `main.rs` wiring task was verified.

**Tech Stack:** Rust. Consumes `crate::ux::{theme, format, spinner}` from U0 (`crates/kyma-cli/src/ux/`) — no new dependency, no `Cargo.toml` change.

## Global Constraints

- Full spec: `docs/superpowers/specs/2026-07-01-cli-ux-overhaul-design.md`. This plan implements **U1 only** — `recall`, `remember`, `entity`, `distill`, `status`. U2 (datasource/ingest), U3 (everything else), U4 (ratatui) are separate future plans.
- `kyma-cli` is a **binary crate** — internal visibility is `pub(crate)` for anything crate-wide, plain (private) for anything file-local. The new helper functions this plan adds (`remember_success_line`, `entity_success_line`) are file-local — no visibility modifier, matching the existing private `render_memory_line`/`mcp_call` in the same file.
- Run tests with `cargo test -p kyma-cli --bins <filter>` (no `--lib` target exists for this crate).
- `--json` output paths (`recall --json`) are untouched by this plan — only the human-facing fallback rendering changes.
- `ux::theme::{success, error, warn, info, muted, accent}` are the **stdout-oriented** convenience wrappers (they read `color_enabled()`) — correct for every command in this plan, since `recall`/`remember`/`entity`/`status` all write to stdout via `println!`. Only `ux::spinner` (used in the `distill` task) writes to stderr, and it already handles its own stderr-appropriate color detection internally — nothing in this plan needs to call `ux::theme::stderr_color_enabled()` directly.
- `distill`'s non-verbose ("quiet") path currently prints nothing — used by Claude Code hooks. This plan adds a spinner to that path (user-approved trade-off: hook-piped runs will see one new plain `"distilling memories..."` line on stderr from `ux::spinner`'s non-interactive fallback, where today there is none). Do not add any *other* new output to the quiet path.

---

### Task 1: `recall` — word-boundary truncation + score-colored ranked list

**Files:**
- Modify: `crates/kyma-cli/src/plugin.rs:1-19` (imports)
- Modify: `crates/kyma-cli/src/plugin.rs:213-237` (`render_memory_line`)
- Create (in the same file): a new `#[cfg(test)] mod tests` block at the end of `crates/kyma-cli/src/plugin.rs`

**Interfaces:**
- Consumes: `crate::ux::format::{truncate, score_style}`, `crate::ux::theme::{muted, BULLET}` (all from U0, already merged).
- Produces: `render_memory_line(row: &serde_json::Value) -> String` — same signature as before, only its internals and output styling change. No other task depends on this.

- [ ] **Step 1: Add the `ux` import**

The current imports at the top of `crates/kyma-cli/src/plugin.rs` (lines 13-19):

```rust
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::client::{self, ClientConfig};
```

Add one line:

```rust
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::client::{self, ClientConfig};
use crate::ux;
```

- [ ] **Step 2: Write the failing tests**

The current `render_memory_line` (lines 213-237):

```rust
/// Render one recalled memory row as a single compact line. Tolerant of which
/// columns the recall SQL projected.
fn render_memory_line(row: &Value) -> String {
    let mtype = row
        .get("memory_type")
        .and_then(Value::as_str)
        .unwrap_or("memory");
    let score = row
        .get("score")
        .and_then(Value::as_f64)
        .or_else(|| row.get("distance").and_then(Value::as_f64).map(|d| 1.0 - d));
    let body = row
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| row.get("content_preview").and_then(Value::as_str))
        .or_else(|| row.get("content").and_then(Value::as_str))
        .unwrap_or("")
        .trim();
    let body: String = body.chars().take(280).collect();
    match score {
        Some(s) => format!("- [{mtype} {:.2}] {body}", s),
        None => format!("- [{mtype}] {body}"),
    }
}
```

Leave it unchanged for now. At the very end of `crates/kyma-cli/src/plugin.rs` (after the last line, currently the closing `}` of `set_private`), add:

```rust

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_memory_line_includes_type_and_body() {
        let row = json!({
            "memory_type": "decision",
            "score": 0.92,
            "content": "use worktrees for all branch work",
        });
        let line = render_memory_line(&row);
        assert!(line.contains("decision"));
        assert!(line.contains("0.92"));
        assert!(line.contains("use worktrees for all branch work"));
    }

    #[test]
    fn render_memory_line_falls_back_to_title_then_preview_then_content() {
        let row = json!({
            "memory_type": "fact",
            "title": "preferred title",
            "content_preview": "should not appear",
            "content": "should not appear either",
        });
        let line = render_memory_line(&row);
        assert!(line.contains("preferred title"));
        assert!(!line.contains("should not appear"));
    }

    #[test]
    fn render_memory_line_handles_missing_score() {
        let row = json!({ "memory_type": "learning", "content": "no score here" });
        let line = render_memory_line(&row);
        assert!(line.contains("learning"));
        assert!(line.contains("no score here"));
        assert!(!line.contains("0."));
    }

    #[test]
    fn render_memory_line_truncates_long_body_with_ellipsis() {
        let long_body = "word ".repeat(100); // 500 chars, trims to 499
        let row = json!({ "memory_type": "fact", "content": long_body });
        let line = render_memory_line(&row);
        assert!(line.contains('…'));
    }
}
```

- [ ] **Step 3: Run tests to verify the truncation test fails**

Run: `cargo test -p kyma-cli --bins plugin::tests`
Expected: `render_memory_line_truncates_long_body_with_ellipsis` FAILS (the current implementation hard-cuts at 280 chars with no ellipsis, so `line.contains('…')` is false). The other 3 tests PASS already — they describe behavior this change doesn't alter (type/body inclusion, fallback ordering, missing-score handling).

- [ ] **Step 4: Implement the new `render_memory_line`**

Replace the function body (lines 213-237) with:

```rust
/// Render one recalled memory row as a single compact line. Tolerant of which
/// columns the recall SQL projected.
fn render_memory_line(row: &Value) -> String {
    let mtype = row
        .get("memory_type")
        .and_then(Value::as_str)
        .unwrap_or("memory");
    let score = row
        .get("score")
        .and_then(Value::as_f64)
        .or_else(|| row.get("distance").and_then(Value::as_f64).map(|d| 1.0 - d));
    let body = row
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| row.get("content_preview").and_then(Value::as_str))
        .or_else(|| row.get("content").and_then(Value::as_str))
        .unwrap_or("")
        .trim();
    let body = ux::format::truncate(body, 280);
    let prefix = match score {
        Some(s) => ux::format::score_style(s as f32, &format!("[{mtype} {s:.2}]")),
        None => ux::theme::muted(&format!("[{mtype}]")),
    };
    format!("{} {prefix} {body}", ux::theme::BULLET)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kyma-cli --bins plugin::tests`
Expected: all 4 tests PASS.

- [ ] **Step 6: Build check**

Run: `cargo build -p kyma-cli`
Expected: builds cleanly.

- [ ] **Step 7: Commit**

```bash
git add crates/kyma-cli/src/plugin.rs
git commit -m "feat(cli): restyle recall — word-boundary truncation + score-colored ranked list"
```

---

### Task 2: `remember` + `entity` — colored one-line success messages

**Files:**
- Modify: `crates/kyma-cli/src/plugin.rs:121-146` (`remember`)
- Modify: `crates/kyma-cli/src/plugin.rs:152-211` (`entity`)
- Modify: the `#[cfg(test)] mod tests` block created in Task 1 (append tests)

**Interfaces:**
- Consumes: `crate::ux::theme::{success, CHECK}` (from U0).
- Produces: `remember_success_line(verb: &str, id: &str) -> String`, `entity_success_line(verb: &str, id: &str, n: u64) -> String` — private helpers, used only within `plugin.rs`.

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block at the end of `crates/kyma-cli/src/plugin.rs` (after `render_memory_line_truncates_long_body_with_ellipsis`'s closing `}`, still inside `mod tests { ... }`):

```rust

    #[test]
    fn remember_success_line_includes_verb_and_id() {
        let line = remember_success_line("saved", "abc-123");
        assert!(line.contains("saved memory abc-123"));
        assert!(line.contains(ux::theme::CHECK));
    }

    #[test]
    fn entity_success_line_includes_verb_id_and_link_count() {
        let line = entity_success_line("created", "xyz-789", 3);
        assert!(line.contains("created entity xyz-789 (3 links)"));
        assert!(line.contains(ux::theme::CHECK));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kyma-cli --bins plugin::tests`
Expected: `remember_success_line_includes_verb_and_id` and `entity_success_line_includes_verb_id_and_link_count` FAIL to compile (`remember_success_line`/`entity_success_line` don't exist yet) — this is a compile-error FAIL, same as the writing-plans template's own "FAIL with 'function not defined'" example.

- [ ] **Step 3: Implement the two helpers and wire them in**

The current end of `remember` (lines 138-146):

```rust
    let id = result.get("id").and_then(Value::as_str).unwrap_or("?");
    let verb = if result.get("upserted").and_then(Value::as_bool).unwrap_or(false) {
        "updated"
    } else {
        "saved"
    };
    println!("{verb} memory {id}");
    Ok(())
}
```

Replace with:

```rust
    let id = result.get("id").and_then(Value::as_str).unwrap_or("?");
    let verb = if result.get("upserted").and_then(Value::as_bool).unwrap_or(false) {
        "updated"
    } else {
        "saved"
    };
    println!("{}", remember_success_line(verb, id));
    Ok(())
}

/// Builds the one-line success message for `remember`: a green check mark
/// followed by plain-colored text.
fn remember_success_line(verb: &str, id: &str) -> String {
    format!("{} {verb} memory {id}", ux::theme::success(ux::theme::CHECK))
}
```

The current end of `entity` (lines 202-211):

```rust
    let id = result.get("id").and_then(Value::as_str).unwrap_or("?");
    let n = result.get("links").and_then(Value::as_u64).unwrap_or(0);
    let verb = if result.get("upserted").and_then(Value::as_bool).unwrap_or(false) {
        "updated"
    } else {
        "created"
    };
    println!("{verb} entity {id} ({n} links)");
    Ok(())
}
```

Replace with:

```rust
    let id = result.get("id").and_then(Value::as_str).unwrap_or("?");
    let n = result.get("links").and_then(Value::as_u64).unwrap_or(0);
    let verb = if result.get("upserted").and_then(Value::as_bool).unwrap_or(false) {
        "updated"
    } else {
        "created"
    };
    println!("{}", entity_success_line(verb, id, n));
    Ok(())
}

/// Builds the one-line success message for `entity`: a green check mark
/// followed by plain-colored text.
fn entity_success_line(verb: &str, id: &str, n: u64) -> String {
    format!(
        "{} {verb} entity {id} ({n} links)",
        ux::theme::success(ux::theme::CHECK)
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kyma-cli --bins plugin::tests`
Expected: all 6 tests in `plugin::tests` PASS (4 from Task 1 + 2 new).

- [ ] **Step 5: Build check**

Run: `cargo build -p kyma-cli`
Expected: builds cleanly.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-cli/src/plugin.rs
git commit -m "feat(cli): restyle remember/entity — colored success messages"
```

---

### Task 3: `distill` — progress spinner on the quiet path

**Files:**
- Modify: `crates/kyma-cli/src/plugin.rs:278-321` (`distill`)

**Interfaces:**
- Consumes: `crate::ux::spinner::spinner` (from U0), returning a `crate::ux::spinner::Spinner` with `.finish_success(&self, msg: &str)` / `.finish_error(&self, msg: &str)`.
- Produces: no new public interface — `distill`'s signature is unchanged.

- [ ] **Step 1: Modify `distill`**

The current function (lines 278-321):

```rust
/// Read a session transcript from stdin and hand it to the kyma agent (which
/// owns `save_memory`) with an extraction instruction. The agent persists the
/// durable memories; we stay quiet unless `KYMA_DISTILL_VERBOSE` is set.
pub(crate) async fn distill(_session: Option<String>, realm: Option<String>) -> Result<()> {
    let cfg = client::effective_config()?;
    let mut transcript = String::new();
    std::io::stdin()
        .read_to_string(&mut transcript)
        .context("reading transcript from stdin")?;
    if transcript.trim().is_empty() {
        return Ok(());
    }
    // Keep the tail if it's large (most recent context matters most).
    if transcript.len() > DISTILL_MAX_CHARS {
        let start = transcript.len() - DISTILL_MAX_CHARS;
        transcript = transcript[start..].to_string();
    }
    let realm = realm.unwrap_or_else(|| "default".to_string());
    let instruction = format!(
        "You are curating durable memory from a Claude Code coding session. Below is the \
session transcript (JSONL or text). Extract the SMALL set of genuinely durable, reusable \
memories: decisions (with rationale), preferences, conventions, non-obvious facts, \
learnings, and open threads worth resuming. For each, call save_memory with realm \
\"{realm}\", an appropriate memory_type (fact/decision/preference/learning/summary) and \
importance 0.3-0.9. Recall first to avoid duplicates; skip transient details and anything \
secret. Quality over quantity (typically 0-6). Then reply with a one-line summary of what \
you saved.\n\n--- SESSION TRANSCRIPT ---\n{transcript}"
    );

    let verbose = std::env::var_os("KYMA_DISTILL_VERBOSE").is_some();
    client::stream_agent_ask(&cfg, &instruction, None, |event, data| {
        if !verbose {
            return;
        }
        if event == "answer_delta" || event == "answer_final" {
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(t) = v.get("text").and_then(Value::as_str) {
                    print!("{t}");
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
            }
        }
    })
    .await?;
    Ok(())
}
```

Replace the doc comment and the last two statements (from `let verbose = ...` through the final `Ok(())`) — everything above `let verbose` stays exactly the same:

```rust
/// Read a session transcript from stdin and hand it to the kyma agent (which
/// owns `save_memory`) with an extraction instruction. The agent persists the
/// durable memories. Shows a progress spinner while waiting (silent plain
/// fallback when stderr isn't a terminal — e.g. when a hook pipes it to a
/// log file); stays quiet about the *extracted memories themselves* unless
/// `KYMA_DISTILL_VERBOSE` is set.
pub(crate) async fn distill(_session: Option<String>, realm: Option<String>) -> Result<()> {
    let cfg = client::effective_config()?;
    let mut transcript = String::new();
    std::io::stdin()
        .read_to_string(&mut transcript)
        .context("reading transcript from stdin")?;
    if transcript.trim().is_empty() {
        return Ok(());
    }
    // Keep the tail if it's large (most recent context matters most).
    if transcript.len() > DISTILL_MAX_CHARS {
        let start = transcript.len() - DISTILL_MAX_CHARS;
        transcript = transcript[start..].to_string();
    }
    let realm = realm.unwrap_or_else(|| "default".to_string());
    let instruction = format!(
        "You are curating durable memory from a Claude Code coding session. Below is the \
session transcript (JSONL or text). Extract the SMALL set of genuinely durable, reusable \
memories: decisions (with rationale), preferences, conventions, non-obvious facts, \
learnings, and open threads worth resuming. For each, call save_memory with realm \
\"{realm}\", an appropriate memory_type (fact/decision/preference/learning/summary) and \
importance 0.3-0.9. Recall first to avoid duplicates; skip transient details and anything \
secret. Quality over quantity (typically 0-6). Then reply with a one-line summary of what \
you saved.\n\n--- SESSION TRANSCRIPT ---\n{transcript}"
    );

    let verbose = std::env::var_os("KYMA_DISTILL_VERBOSE").is_some();
    let spinner = (!verbose).then(|| ux::spinner::spinner("distilling memories"));
    let result = client::stream_agent_ask(&cfg, &instruction, None, |event, data| {
        if !verbose {
            return;
        }
        if event == "answer_delta" || event == "answer_final" {
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(t) = v.get("text").and_then(Value::as_str) {
                    print!("{t}");
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
            }
        }
    })
    .await;
    if let Some(s) = &spinner {
        match &result {
            Ok(()) => s.finish_success("distilled memories"),
            // Deliberately short — the full cause chain + hint is printed by
            // main()'s unified error handler once this Err propagates; a
            // second copy of the same text here would just be noise.
            Err(_) => s.finish_error("distill failed"),
        }
    }
    result
}
```

Note: `ux::spinner::spinner` was never invoked anywhere before this task, so this is the toolkit's first production use — sanity-check the import. `plugin.rs` already has `use crate::ux;` from Task 1, so `ux::spinner::spinner(...)` resolves without any new `use` line.

- [ ] **Step 2: Build check**

Run: `cargo build -p kyma-cli`
Expected: builds cleanly. `dead_code` warnings on `ux::spinner`'s public functions should now be GONE (this is the first real caller) — if they're still present, something didn't wire up correctly; investigate before proceeding.

- [ ] **Step 3: Run the full test suite (regression check)**

Run: `cargo test -p kyma-cli --bins`
Expected: all tests pass, same count as before this task plus Task 1/2's new tests (no `distill`-specific automated tests exist — this task has no pure-function surface to unit test; verification is manual, below).

- [ ] **Step 4: Manual verification — quiet path still ships zero *content* output, spinner is the only addition**

```bash
TMP_HOME=$(mktemp -d)
echo "test transcript, nothing durable here" | KYMA_SERVER_URL=http://127.0.0.1:59999 HOME="$TMP_HOME" ./target/debug/kyma-cli distill 2>&1 1>/dev/null; echo "exit: $?"
```

Expected: stderr shows two lines: `distilling memories...` (the non-interactive spinner fallback — stderr isn't a tty when captured this way) followed by `✗ distill failed` (the spinner's short finish line), then (from `main()`'s unified error handler) the full `✗ Error: ... caused by: connection refused ... hint: is \`kyma serve\` running?...` block. `exit: 1`. stdout (redirected to `/dev/null` above) is empty either way — confirms the quiet path's only new *stdout* behavior is none; the new output is entirely on stderr.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-cli/src/plugin.rs
git commit -m "feat(cli): add progress spinner to distill's quiet path"
```

---

### Task 4: `status` — colored health/auth/capture indicators

**Files:**
- Modify: `crates/kyma-cli/src/main.rs:985-1032` (`cmd_status`)

**Interfaces:**
- Consumes: `crate::ux::theme::{success, error, warn, muted, CHECK, CROSS}` (from U0). `main.rs` is the crate root, so these resolve as `ux::theme::...` directly (no `use crate::ux;` needed — `main.rs` already has `mod ux;` and uses `ux::theme::init`/`ux::error::print_error` this way since U0).
- Produces: no new interface — `cmd_status`'s signature and control flow (which branch runs when) are unchanged; only the text passed to each `println!` changes.

- [ ] **Step 1: Modify `cmd_status`**

The current function (`crates/kyma-cli/src/main.rs:985-1032`):

```rust
async fn cmd_status() -> Result<()> {
    match load_config() {
        Ok(cfg) => {
            println!("Endpoint:  {}", cfg.endpoint);
            println!(
                "Token:     {}",
                if cfg.token.is_some() {
                    "configured"
                } else {
                    "not set"
                }
            );
            // Use effective_config for probes so KYMA_SERVER_URL/KYMA_TOKEN env
            // overrides are honoured; fall back to the on-disk config if it fails.
            let probe_cfg = effective_config().unwrap_or_else(|_| cfg.clone());
            match probe_health(&probe_cfg).await {
                Ok(body) => println!("Health:    {}", body.trim()),
                Err(e) => println!("Health:    error — {e}"),
            }
            match probe_auth(&probe_cfg).await {
                Ok(true) => println!("Auth:      ok (token accepted)"),
                Ok(false) => println!(
                    "Auth:      TOKEN REJECTED — the server does not accept the configured token.\n           Fix: re-run the installer, or `kyma service install --addr <addr> --token <tok>`,\n           or `kyma connect {} --token <tok>` with the server's real token.",
                    probe_cfg.endpoint
                ),
                Err(e) => println!("Auth:      probe error — {e}"),
            }
            // Hook-side capture health (written by the kyma-memory plugin hooks).
            if let Ok(dir) = client::config_dir() {
                let p = dir.join("capture-health.json");
                if let Ok(raw) = std::fs::read_to_string(&p) {
                    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
                    println!(
                        "Capture:   LAST INGEST FAILED at {} — {}",
                        v["ts"].as_str().unwrap_or("?"),
                        v["detail"].as_str().unwrap_or("unknown error"),
                    );
                } else {
                    println!("Capture:   ok (no recorded hook failures)");
                }
            }
        }
        Err(_) => {
            println!("No config found. Run `kyma connect <url>` first.");
        }
    }
    Ok(())
}
```

Replace with:

```rust
async fn cmd_status() -> Result<()> {
    match load_config() {
        Ok(cfg) => {
            println!("Endpoint:  {}", cfg.endpoint);
            let token_line = if cfg.token.is_some() {
                ux::theme::success(&format!("{} configured", ux::theme::CHECK))
            } else {
                ux::theme::muted(&format!("{} not set", ux::theme::CROSS))
            };
            println!("Token:     {token_line}");
            // Use effective_config for probes so KYMA_SERVER_URL/KYMA_TOKEN env
            // overrides are honoured; fall back to the on-disk config if it fails.
            let probe_cfg = effective_config().unwrap_or_else(|_| cfg.clone());
            match probe_health(&probe_cfg).await {
                Ok(body) => println!(
                    "Health:    {}",
                    ux::theme::success(&format!("{} {}", ux::theme::CHECK, body.trim()))
                ),
                Err(e) => println!(
                    "Health:    {}",
                    ux::theme::error(&format!("{} error — {e}", ux::theme::CROSS))
                ),
            }
            match probe_auth(&probe_cfg).await {
                Ok(true) => println!(
                    "Auth:      {}",
                    ux::theme::success(&format!("{} ok (token accepted)", ux::theme::CHECK))
                ),
                Ok(false) => println!(
                    "Auth:      {}",
                    ux::theme::warn(&format!(
                        "{} TOKEN REJECTED — the server does not accept the configured token.\n           Fix: re-run the installer, or `kyma service install --addr <addr> --token <tok>`,\n           or `kyma connect {} --token <tok>` with the server's real token.",
                        ux::theme::CROSS,
                        probe_cfg.endpoint
                    ))
                ),
                Err(e) => println!(
                    "Auth:      {}",
                    ux::theme::error(&format!("{} probe error — {e}", ux::theme::CROSS))
                ),
            }
            // Hook-side capture health (written by the kyma-memory plugin hooks).
            if let Ok(dir) = client::config_dir() {
                let p = dir.join("capture-health.json");
                if let Ok(raw) = std::fs::read_to_string(&p) {
                    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
                    println!(
                        "Capture:   {}",
                        ux::theme::error(&format!(
                            "{} LAST INGEST FAILED at {} — {}",
                            ux::theme::CROSS,
                            v["ts"].as_str().unwrap_or("?"),
                            v["detail"].as_str().unwrap_or("unknown error"),
                        ))
                    );
                } else {
                    println!(
                        "Capture:   {}",
                        ux::theme::success(&format!(
                            "{} ok (no recorded hook failures)",
                            ux::theme::CHECK
                        ))
                    );
                }
            }
        }
        Err(_) => {
            println!(
                "{}",
                ux::theme::muted("No config found. Run `kyma connect <url>` first.")
            );
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build check**

Run: `cargo build -p kyma-cli`
Expected: builds cleanly.

- [ ] **Step 3: Run the full test suite (regression check)**

Run: `cargo test -p kyma-cli --bins`
Expected: all tests pass (no `status`-specific automated tests exist — `cmd_status` is a thin orchestration function over live network probes with no pure-function surface to extract without over-engineering; verification is manual, below).

- [ ] **Step 4: Manual verification — "no config" branch**

```bash
TMP_HOME=$(mktemp -d)
env -i HOME="$TMP_HOME" PATH="$PATH" ./target/debug/kyma-cli status
```

Expected: a single muted/dim line: `No config found. Run \`kyma connect <url>\` first.` (piped/non-tty here, so no visible ANSI either way — the point is confirming the branch still runs and prints the right text).

- [ ] **Step 5: Manual verification — health/auth error branches**

```bash
TMP_HOME=$(mktemp -d)
mkdir -p "$TMP_HOME/.kyma"
echo '{"endpoint":"http://127.0.0.1:59999"}' > "$TMP_HOME/.kyma/config.json"
env -i HOME="$TMP_HOME" PATH="$PATH" ./target/debug/kyma-cli status
```

Expected:
```
Endpoint:  http://127.0.0.1:59999
Token:     ✗ not set
Health:    ✗ error — ...connection refused...
Auth:      ✗ probe error — ...connection refused...
Capture:   ✓ ok (no recorded hook failures)
```
(exact error text after "error — " varies by OS; the important thing is the `✗`/`✓` glyphs appear and the command doesn't crash.) This never touches the real `~/.kyma/config.json`.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-cli/src/main.rs
git commit -m "feat(cli): restyle status — colored health/auth/capture indicators"
```

---

## Post-plan note

After this plan, `cargo build -p kyma-cli` should show fewer `dead_code` warnings than after U0 — `ux::theme::success/error/warn/muted/CHECK/CROSS/BULLET`, `ux::format::truncate/score_style`, and `ux::spinner::{spinner, Spinner}` all gain their first real callers. Expected *remaining* warnings after this plan: `ux::table::*` (unused until U2), `ux::format::relative_time` (unused until a later phase touches a timestamp-listing command like `sessions list`), and `ux::theme::{info, accent, ARROW}` (reserved, no consumer yet in any phase's current scope) — none of these are regressions, just phases not yet reached.
