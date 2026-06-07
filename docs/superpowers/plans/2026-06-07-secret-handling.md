# End-to-End Secret Handling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Secrets (provider keys, connector credentials, bearer tokens, foreign tokens in user data) never reach model context, persisted transcripts, logs, or the web UI — masked as `[REDACTED:<kind>]` with audit events.

**Architecture:** A new dependency-light `kyma-redact` crate provides a process-global `SecretGuard` (known-value Aho-Corasick matcher + curated regex patterns), a `StreamScrubber` for delta streams, and a `ScrubWriter` for logs. Choke points: tool results (via a `RedactingTool` decorator wrapped around every agent/MCP tool), the `Emitter` trace/UI stream, `persist_run`/`persist_turn`, memory writes, the tracing writer, and the Claude CLI subprocess env. Defense-in-depth: web UI display masking + a one-time DB backfill.

**Tech Stack:** Rust (axum, adk-rust 0.6, sqlx, tokio), `regex` + `aho-corasick` crates, React/TypeScript web UI, PostgreSQL.

**Spec:** `docs/superpowers/specs/2026-06-07-secret-handling-design.md`

**Branch:** `feat/secret-handling` (already created)

**Verification baseline:** `cargo build --workspace` and `cargo test --workspace` must pass before Task 1 (record any pre-existing failures so they aren't attributed to this work).

---

### Task 1: `kyma-redact` crate — pattern detection + `redact_text`

**Files:**
- Modify: `Cargo.toml` (workspace root — members + dependencies)
- Create: `crates/kyma-redact/Cargo.toml`
- Create: `crates/kyma-redact/src/lib.rs`

- [ ] **Step 1: Add workspace member and dependencies**

In root `Cargo.toml`, add `"crates/kyma-redact",` to `members` (alphabetical position, near `"crates/kyma-queue"`). In `[workspace.dependencies]` add (near the `chumsky` "Parser" block):

```toml
# Secret detection / redaction
regex = "1"
aho-corasick = "1"
```

And in the workspace-deps section where sibling crates are declared (search for `kyma-queue = { path =` to find the block), add:

```toml
kyma-redact = { path = "crates/kyma-redact" }
```

- [ ] **Step 2: Create the crate manifest**

`crates/kyma-redact/Cargo.toml`:

```toml
[package]
name = "kyma-redact"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "Secret detection + redaction: known-value matching, pattern matching, stream scrubbing."

[lints]
workspace = true

[dependencies]
aho-corasick.workspace = true
regex.workspace = true
serde_json.workspace = true
tracing.workspace = true
```

(If sibling crates don't use `version.workspace = true`, copy the exact `[package]` style from `crates/kyma-kql/Cargo.toml`.)

- [ ] **Step 3: Write failing tests for pattern redaction**

`crates/kyma-redact/src/lib.rs` — start with the test module at the bottom; the implementation in Step 5 goes above it:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> SecretGuard {
        SecretGuard::new()
    }

    #[test]
    fn redacts_github_pat() {
        let (out, f) = guard().redact_text(
            "GH_PAT=\"github_pat_11A22PAOI0QD7qilOFoYIK_9beaDlLnkO4GA0eKKSGdG7YDYM6vGgs\"",
        );
        assert_eq!(out, "GH_PAT=\"[REDACTED:github-pat]\"");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "github-pat");
    }

    #[test]
    fn redacts_classic_github_token() {
        let (out, _) = guard().redact_text("token ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef1234 end");
        assert_eq!(out, "token [REDACTED:github-token] end");
    }

    #[test]
    fn redacts_anthropic_key_before_generic_sk() {
        let (out, f) = guard().redact_text("sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWx");
        assert_eq!(out, "[REDACTED:anthropic-key]");
        assert_eq!(f[0].kind, "anthropic-key");
    }

    #[test]
    fn redacts_generic_sk_key() {
        let (out, _) = guard().redact_text("key=sk-AbCdEfGhIjKlMnOpQrStUvWxYz123456");
        assert_eq!(out, "key=[REDACTED:api-key]");
    }

    #[test]
    fn redacts_aws_key_id() {
        let (out, _) = guard().redact_text("AKIAIOSFODNN7EXAMPLE");
        assert_eq!(out, "[REDACTED:aws-key-id]");
    }

    #[test]
    fn redacts_jwt() {
        let (out, _) = guard().redact_text(
            "auth eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9P",
        );
        assert_eq!(out, "auth [REDACTED:jwt]");
    }

    #[test]
    fn redacts_url_password_only() {
        let (out, f) = guard().redact_text("postgres://kyma:s3cretPW@db.host:5432/kyma");
        assert_eq!(out, "postgres://kyma:[REDACTED:url-credential]@db.host:5432/kyma");
        assert_eq!(f[0].kind, "url-credential");
    }

    #[test]
    fn redacts_bearer_token_value_only() {
        let (out, _) = guard().redact_text("Authorization: Bearer abcdefghijklmnopqrstuvwxyz123456");
        assert_eq!(out, "Authorization: Bearer [REDACTED:bearer-token]");
    }

    #[test]
    fn redacts_slack_token() {
        let (out, _) = guard().redact_text("xoxb-123456789012-abcdefghij");
        assert_eq!(out, "[REDACTED:slack-token]");
    }

    #[test]
    fn redacts_pem_private_key() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEow...base64...\n-----END RSA PRIVATE KEY-----";
        let (out, _) = guard().redact_text(pem);
        assert_eq!(out, "[REDACTED:private-key]");
    }

    #[test]
    fn idempotent_on_redacted_text() {
        let g = guard();
        let (once, _) = g.redact_text("postgres://u:pw12345678@h/db and Bearer abcdefghijklmnop1234");
        let (twice, findings) = g.redact_text(&once);
        assert_eq!(once, twice);
        assert!(findings.is_empty(), "re-scan of redacted text must find nothing");
    }

    #[test]
    fn clean_text_passes_through_unchanged() {
        let g = guard();
        let input = "SELECT count(*) FROM logs WHERE level = 'error'";
        let (out, f) = g.redact_text(input);
        assert_eq!(out, input);
        assert!(f.is_empty());
    }
}
```

- [ ] **Step 4: Run tests to verify they fail to compile (no implementation yet)**

Run: `cargo test -p kyma-redact`
Expected: compile error — `SecretGuard` not found.

- [ ] **Step 5: Implement `SecretGuard` pattern matching**

Above the test module in `crates/kyma-redact/src/lib.rs`:

```rust
//! Secret detection + redaction.
//!
//! Two detector classes feed a single [`SecretGuard`]:
//!
//! - **Known values** — exact strings registered at runtime (decrypted
//!   credentials, env API keys, bearer tokens). Matched with Aho-Corasick;
//!   zero false positives.
//! - **Patterns** — a curated regex set for *foreign* secrets that show up in
//!   scanned data (GitHub PATs, `sk-` keys, JWTs, connection-string
//!   passwords, …).
//!
//! Replacement is always `[REDACTED:<kind>]` and is idempotent: redacted
//! text never re-matches.

use regex::Regex;
use std::sync::{Arc, LazyLock, RwLock};

/// One detected secret. Carries the kind only — never the value.
#[derive(Debug, Clone)]
pub struct Finding {
    pub kind: String,
}

struct Pattern {
    kind: &'static str,
    re: Regex,
    /// Capture group to replace; 0 = the whole match.
    group: usize,
}

fn patterns() -> Vec<Pattern> {
    let p = |kind, re: &str, group| Pattern {
        kind,
        re: Regex::new(re).expect("static redaction regex must compile"),
        group,
    };
    vec![
        p("github-pat", r"\bgithub_pat_[A-Za-z0-9_]{20,}", 0),
        p("github-token", r"\bgh[pousr]_[A-Za-z0-9]{20,}\b", 0),
        // anthropic before generic sk- so the more specific kind wins.
        p("anthropic-key", r"\bsk-ant-[A-Za-z0-9_-]{16,}\b", 0),
        p("api-key", r"\bsk-[A-Za-z0-9_-]{16,}\b", 0),
        p("aws-key-id", r"\bAKIA[0-9A-Z]{16}\b", 0),
        p("slack-token", r"\bxox[bpars]-[A-Za-z0-9-]{10,}\b", 0),
        p(
            "jwt",
            r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b",
            0,
        ),
        // `[` and `]` excluded from the password class so `[REDACTED:…]`
        // never re-matches (idempotency).
        p(
            "url-credential",
            r#"://[^/\s:@"']{1,128}:([^@\s/"'\[\]]{1,256})@"#,
            1,
        ),
        p(
            "bearer-token",
            r"(?i)\bbearer\s+([A-Za-z0-9._~+/-]{16,}=*)",
            1,
        ),
        p(
            "private-key",
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.{0,16384}?-----END [A-Z ]*PRIVATE KEY-----",
            0,
        ),
    ]
}

/// Known-value registry: parallel `kinds`/`values` vectors + a rebuilt
/// Aho-Corasick automaton. Values shorter than 8 chars are ignored (too
/// collision-prone to mask globally).
#[derive(Default)]
struct KnownValues {
    kinds: Vec<String>,
    values: Vec<String>,
    ac: Option<aho_corasick::AhoCorasick>,
}

pub struct SecretGuard {
    patterns: Vec<Pattern>,
    known: RwLock<KnownValues>,
}

impl Default for SecretGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretGuard {
    pub fn new() -> Self {
        Self {
            patterns: patterns(),
            known: RwLock::new(KnownValues::default()),
        }
    }

    /// Register a live secret value for exact matching. No-op for values
    /// under 8 chars or already registered.
    pub fn register_value(&self, kind: &str, value: &str) {
        if value.len() < 8 {
            return;
        }
        let mut k = self.known.write().expect("SecretGuard lock poisoned");
        if k.values.iter().any(|v| v == value) {
            return;
        }
        k.kinds.push(kind.to_string());
        k.values.push(value.to_string());
        k.ac = aho_corasick::AhoCorasick::new(&k.values).ok();
    }

    /// Redact known values + pattern matches. Returns the (possibly
    /// unchanged) text and one `Finding` per replacement.
    pub fn redact_text(&self, text: &str) -> (String, Vec<Finding>) {
        let mut findings = Vec::new();
        let mut out = self.redact_known(text, &mut findings);
        for p in &self.patterns {
            if !p.re.is_match(&out) {
                continue;
            }
            out = p
                .re
                .replace_all(&out, |caps: &regex::Captures| {
                    findings.push(Finding {
                        kind: p.kind.to_string(),
                    });
                    let marker = format!("[REDACTED:{}]", p.kind);
                    if p.group == 0 {
                        marker
                    } else {
                        let whole = caps.get(0).expect("group 0 always present");
                        let g = caps.get(p.group).expect("declared group must match");
                        let s = whole.as_str();
                        let start = g.start() - whole.start();
                        let end = g.end() - whole.start();
                        format!("{}{}{}", &s[..start], marker, &s[end..])
                    }
                })
                .into_owned();
        }
        (out, findings)
    }

    /// Scan without rewriting.
    pub fn scan(&self, text: &str) -> Vec<Finding> {
        self.redact_text(text).1
    }

    fn redact_known(&self, text: &str, findings: &mut Vec<Finding>) -> String {
        let k = self.known.read().expect("SecretGuard lock poisoned");
        let Some(ac) = &k.ac else {
            return text.to_string();
        };
        let mut out = String::with_capacity(text.len());
        let mut last = 0usize;
        for m in ac.find_iter(text) {
            out.push_str(&text[last..m.start()]);
            let kind = &k.kinds[m.pattern().as_usize()];
            out.push_str(&format!("[REDACTED:{kind}]"));
            findings.push(Finding { kind: kind.clone() });
            last = m.end();
        }
        out.push_str(&text[last..]);
        out
    }
}

/// Process-global guard. All production choke points share this instance so
/// a credential registered anywhere is masked everywhere (tools, traces,
/// memory, logs). Unit tests use private `SecretGuard::new()` instances.
static GLOBAL: LazyLock<Arc<SecretGuard>> = LazyLock::new(|| Arc::new(SecretGuard::new()));

pub fn global() -> Arc<SecretGuard> {
    GLOBAL.clone()
}

/// Register every env var that looks secret-bearing (name suffix heuristic)
/// with the global guard. Call once at process start, before serving.
pub fn register_env_secrets() {
    const SUFFIXES: [&str; 5] = ["_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "_PASSWD"];
    for (name, value) in std::env::vars() {
        let upper = name.to_ascii_uppercase();
        if value.len() >= 12 && SUFFIXES.iter().any(|s| upper.ends_with(s)) {
            global().register_value("env", &value);
        }
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p kyma-redact`
Expected: all tests PASS.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/kyma-redact
git commit -m "feat(redact): kyma-redact crate — pattern-based secret detection + redaction"
```

---

### Task 2: Known-value matching tests + `redact_json` deep walk

**Files:**
- Modify: `crates/kyma-redact/src/lib.rs`

- [ ] **Step 1: Write failing tests (append inside `mod tests`)**

```rust
    #[test]
    fn known_value_exact_match() {
        let g = guard();
        g.register_value("credential", "qf81nn-NOT-A-PATTERN-x29a");
        let (out, f) = g.redact_text("conn uses qf81nn-NOT-A-PATTERN-x29a here");
        assert_eq!(out, "conn uses [REDACTED:credential] here");
        assert_eq!(f[0].kind, "credential");
    }

    #[test]
    fn short_values_are_not_registered() {
        let g = guard();
        g.register_value("credential", "abc");
        let (out, f) = g.redact_text("abc everywhere abc");
        assert_eq!(out, "abc everywhere abc");
        assert!(f.is_empty());
    }

    #[test]
    fn redact_json_walks_values_and_keys() {
        let g = guard();
        g.register_value("credential", "deepSecretValue42");
        let mut v = serde_json::json!({
            "rows": [{"token": "deepSecretValue42", "ok": true}],
            "deepSecretValue42": "key position",
            "nested": {"url": "postgres://u:pw1234567@h/db"},
            "n": 7
        });
        let findings = g.redact_json(&mut v);
        assert_eq!(v["rows"][0]["token"], "[REDACTED:credential]");
        assert!(v.get("deepSecretValue42").is_none());
        assert_eq!(v["[REDACTED:credential]"], "key position");
        assert_eq!(v["nested"]["url"], "postgres://u:[REDACTED:url-credential]@h/db");
        assert_eq!(v["n"], 7);
        assert_eq!(findings.len(), 3);
    }
```

- [ ] **Step 2: Run to verify the json test fails**

Run: `cargo test -p kyma-redact`
Expected: FAIL — `redact_json` not found (known-value tests pass already).

- [ ] **Step 3: Implement `redact_json` (add to `impl SecretGuard`)**

```rust
    /// Deep-walk a JSON value, redacting every string value and object key.
    pub fn redact_json(&self, v: &mut serde_json::Value) -> Vec<Finding> {
        let mut findings = Vec::new();
        self.walk_json(v, &mut findings);
        findings
    }

    fn walk_json(&self, v: &mut serde_json::Value, findings: &mut Vec<Finding>) {
        match v {
            serde_json::Value::String(s) => {
                let (red, mut f) = self.redact_text(s);
                if !f.is_empty() {
                    *s = red;
                    findings.append(&mut f);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    self.walk_json(item, findings);
                }
            }
            serde_json::Value::Object(map) => {
                let dirty_keys: Vec<String> = map
                    .keys()
                    .filter(|k| !self.scan(k).is_empty())
                    .cloned()
                    .collect();
                for key in dirty_keys {
                    if let Some(val) = map.remove(&key) {
                        let (red, mut f) = self.redact_text(&key);
                        findings.append(&mut f);
                        map.insert(red, val);
                    }
                }
                for (_, val) in map.iter_mut() {
                    self.walk_json(val, findings);
                }
            }
            _ => {}
        }
    }
```

Note: key scanning calls `scan` then `redact_text` (two passes) — keys are short and rare; clarity over micro-optimization.

- [ ] **Step 4: Run tests**

Run: `cargo test -p kyma-redact`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-redact/src/lib.rs
git commit -m "feat(redact): known-value registry + JSON deep-walk redaction"
```

---

### Task 3: `StreamScrubber` for delta streams

**Files:**
- Modify: `crates/kyma-redact/src/lib.rs`

- [ ] **Step 1: Write failing tests (append inside `mod tests`)**

```rust
    #[test]
    fn stream_scrubber_catches_secret_split_across_deltas() {
        let g = Arc::new(guard());
        let mut s = StreamScrubber::new(g);
        let mut out = String::new();
        // A GitHub PAT split mid-token across three deltas.
        out.push_str(&s.push("prefix github_pat_11A22PAOI0"));
        out.push_str(&s.push("QD7qilOFoYIK_9beaDlLnk"));
        out.push_str(&s.push("O4GA0eKKSGdG7YDYM6vGgs suffix"));
        out.push_str(&s.finish());
        assert_eq!(out, "prefix [REDACTED:github-pat] suffix");
        assert_eq!(s.take_findings().len(), 1);
    }

    #[test]
    fn stream_scrubber_emits_long_clean_text_incrementally() {
        let g = Arc::new(guard());
        let mut s = StreamScrubber::new(g);
        let chunk = "All work and no play makes Jack a dull boy. ".repeat(20); // ~900 bytes
        let first = s.push(&chunk);
        // Everything except the holdback tail must stream out immediately.
        assert!(first.len() >= chunk.len() - 256);
        let rest = s.finish();
        assert_eq!(format!("{first}{rest}"), chunk);
        assert!(s.take_findings().is_empty());
    }

    #[test]
    fn stream_scrubber_holdback_respects_char_boundaries() {
        let g = Arc::new(guard());
        let mut s = StreamScrubber::new(g);
        let text = "é".repeat(400); // 2-byte chars; 800 bytes total
        let first = s.push(&text);
        let rest = s.finish();
        assert_eq!(format!("{first}{rest}"), text);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kyma-redact`
Expected: compile FAIL — `StreamScrubber` not found.

- [ ] **Step 3: Implement `StreamScrubber` (top-level, after `SecretGuard` impl)**

```rust
/// Bytes held back from emission so a secret arriving split across stream
/// deltas can still be matched. Anything longer than this can straddle the
/// boundary and reach the live UI partially — the persistence-time scrub
/// still catches it (accepted trade-off in the design spec).
const STREAM_HOLDBACK_BYTES: usize = 256;

/// Incremental scrubber for streaming text deltas (answer/thinking blocks).
/// Feed deltas with [`push`](Self::push), emit its return value, and flush
/// the tail with [`finish`](Self::finish) when the block closes.
pub struct StreamScrubber {
    guard: Arc<SecretGuard>,
    buf: String,
    findings: Vec<Finding>,
}

impl StreamScrubber {
    pub fn new(guard: Arc<SecretGuard>) -> Self {
        Self {
            guard,
            buf: String::new(),
            findings: Vec::new(),
        }
    }

    /// Append a delta; returns redacted text that is safe to emit now
    /// (may be empty while the holdback window fills).
    pub fn push(&mut self, delta: &str) -> String {
        self.buf.push_str(delta);
        let (red, mut f) = self.guard.redact_text(&self.buf);
        self.findings.append(&mut f);
        self.buf = red;
        if self.buf.len() <= STREAM_HOLDBACK_BYTES {
            return String::new();
        }
        let mut cut = self.buf.len() - STREAM_HOLDBACK_BYTES;
        while !self.buf.is_char_boundary(cut) {
            cut += 1;
        }
        self.buf.drain(..cut).collect()
    }

    /// Flush and return the redacted holdback tail.
    pub fn finish(&mut self) -> String {
        let (red, mut f) = self.guard.redact_text(&self.buf);
        self.findings.append(&mut f);
        self.buf.clear();
        red
    }

    /// Drain accumulated findings (for audit events).
    pub fn take_findings(&mut self) -> Vec<Finding> {
        std::mem::take(&mut self.findings)
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kyma-redact`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-redact/src/lib.rs
git commit -m "feat(redact): StreamScrubber — holdback-window scrubbing for delta streams"
```

---

### Task 4: `ScrubWriter` + scrubbed logging in both binaries

**Files:**
- Modify: `crates/kyma-redact/src/lib.rs`
- Modify: `crates/kyma-bin/src/main.rs:80-93`
- Modify: `crates/kyma-bin/Cargo.toml`
- Modify: `crates/kyma-cli/src/main.rs:385-395`
- Modify: `crates/kyma-cli/Cargo.toml`

- [ ] **Step 1: Write failing test (append inside `mod tests`)**

```rust
    #[test]
    fn scrub_writer_masks_log_lines() {
        // Uses the process-global guard (what production wiring uses).
        global().register_value("test-secret", "GLOBALscrubwriterSECRET99");
        let mut sink: Vec<u8> = Vec::new();
        {
            use std::io::Write;
            let mut w = ScrubWriter::new(&mut sink);
            w.write_all(b"error connecting with GLOBALscrubwriterSECRET99 to db")
                .unwrap();
        }
        let line = String::from_utf8(sink).unwrap();
        assert_eq!(line, "error connecting with [REDACTED:test-secret] to db");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kyma-redact`
Expected: compile FAIL — `ScrubWriter` not found.

- [ ] **Step 3: Implement `ScrubWriter`**

```rust
/// An `io::Write` wrapper that redacts every chunk through the global guard.
/// Wrap the tracing fmt writer with this so secrets never reach log output:
/// `fmt().with_writer(|| ScrubWriter::new(std::io::stderr()))`.
pub struct ScrubWriter<W: std::io::Write> {
    inner: W,
}

impl<W: std::io::Write> ScrubWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: std::io::Write> std::io::Write for ScrubWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match std::str::from_utf8(buf) {
            Ok(s) => {
                let (red, _) = global().redact_text(s);
                self.inner.write_all(red.as_bytes())?;
                // Claim the original length: the caller tracks its own buffer.
                Ok(buf.len())
            }
            // Non-UTF-8 (shouldn't happen for tracing fmt): pass through.
            Err(_) => self.inner.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kyma-redact`
Expected: all PASS.

- [ ] **Step 5: Wire into `kyma-bin`**

Add `kyma-redact.workspace = true` to `crates/kyma-bin/Cargo.toml` `[dependencies]`.

In `crates/kyma-bin/src/main.rs` (current lines 82-89), change:

```rust
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,sqlx=warn,hyper=warn,h2=warn")
            }),
        )
        .with_target(true)
        .init();
```

to:

```rust
    // Register secret-bearing env values BEFORE the first log line, then
    // route all log output through the scrubbing writer.
    kyma_redact::register_env_secrets();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,sqlx=warn,hyper=warn,h2=warn")
            }),
        )
        .with_target(true)
        .with_writer(|| kyma_redact::ScrubWriter::new(std::io::stdout()))
        .init();
```

- [ ] **Step 6: Wire into `kyma-cli`**

Add `kyma-redact.workspace = true` to `crates/kyma-cli/Cargo.toml` `[dependencies]`.

In `crates/kyma-cli/src/main.rs` (current lines ~387-394), the fmt builder uses `.with_writer(std::io::stderr)`. Insert `kyma_redact::register_env_secrets();` immediately before the `tracing_subscriber::fmt()` statement and replace the writer line:

```rust
        .with_writer(|| kyma_redact::ScrubWriter::new(std::io::stderr()))
```

- [ ] **Step 7: Build both binaries**

Run: `cargo build -p kyma-bin -p kyma-cli`
Expected: clean build.

- [ ] **Step 8: Commit**

```bash
git add crates/kyma-redact crates/kyma-bin crates/kyma-cli
git commit -m "feat(redact): ScrubWriter — secret-scrubbed log output in kyma-bin and kyma-cli"
```

---

### Task 5: `RedactingTool` decorator + credential registration decorator

**Files:**
- Create: `crates/kyma-server/src/agent/redacting.rs`
- Modify: `crates/kyma-server/src/agent/mod.rs` (module + re-exports)
- Modify: `crates/kyma-server/Cargo.toml` (add `kyma-redact.workspace = true`)

- [ ] **Step 1: Write the failing test (bottom of the new `redacting.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use adk_rust::tool::{FunctionTool, SimpleToolContext};
    use serde_json::json;

    #[tokio::test]
    async fn redacting_tool_masks_secrets_in_results() {
        let inner: Arc<dyn Tool> = Arc::new(FunctionTool::new(
            "leaky",
            "returns a secret",
            |_tc: Arc<dyn ToolContext>, _args: serde_json::Value| async move {
                Ok(json!({"rows": [{"token": "github_pat_11A22PAOI0QD7qilOFoYIK_9beaDlLnkO4GA0e"}]}))
            },
        ));
        let wrapped = redacted(inner);
        assert_eq!(wrapped.name(), "leaky");
        let ctx = Arc::new(SimpleToolContext::new("test"));
        let out = wrapped.execute(ctx, json!({})).await.unwrap();
        assert_eq!(out["rows"][0]["token"], "[REDACTED:github-pat]");
    }

    #[test]
    fn register_credential_covers_every_variant() {
        use kyma_core::credentials::CredentialValue;
        let g = kyma_redact::SecretGuard::new();
        register_credential(
            &g,
            &CredentialValue::Basic {
                username: "u".into(),
                password: "basicPW-longenough".into(),
            },
        );
        register_credential(
            &g,
            &CredentialValue::GithubApp {
                app_id: "1".into(),
                installation_id: "2".into(),
                private_key_pem: "pem-material-longenough".into(),
            },
        );
        let (out, _) = g.redact_text("basicPW-longenough pem-material-longenough");
        assert_eq!(out, "[REDACTED:credential] [REDACTED:credential]");
    }
}
```

(If `SimpleToolContext::new` has a different signature, copy the construction from `crates/kyma-mcp/src/tools.rs:106`.)

- [ ] **Step 2: Implement the decorators (top of `redacting.rs`)**

```rust
//! Redaction decorators: wrap tools so results are scrubbed before they
//! reach model context, and wrap the credential store so every decrypted
//! value is registered with the global [`kyma_redact::SecretGuard`].

use adk_rust::{Tool, ToolContext};
use async_trait::async_trait;
use kyma_core::credentials::{Credential, CredentialStore, CredentialValue};
use kyma_core::tenant::TenantId;
use kyma_redact::SecretGuard;
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

/// Wrap a tool so its result JSON is redacted before returning to the
/// runner (ADK path) or MCP client (Claude CLI path) — i.e. before the
/// model ever sees it.
pub fn redacted(inner: Arc<dyn Tool>) -> Arc<dyn Tool> {
    Arc::new(RedactingTool {
        inner,
        guard: kyma_redact::global(),
    })
}

struct RedactingTool {
    inner: Arc<dyn Tool>,
    guard: Arc<SecretGuard>,
}

#[async_trait]
impl Tool for RedactingTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn declaration(&self) -> Value {
        self.inner.declaration()
    }
    fn enhanced_description(&self) -> String {
        self.inner.enhanced_description()
    }
    fn is_long_running(&self) -> bool {
        self.inner.is_long_running()
    }
    fn is_builtin(&self) -> bool {
        self.inner.is_builtin()
    }
    fn parameters_schema(&self) -> Option<Value> {
        self.inner.parameters_schema()
    }
    fn response_schema(&self) -> Option<Value> {
        self.inner.response_schema()
    }
    fn required_scopes(&self) -> &[&str] {
        self.inner.required_scopes()
    }
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }
    fn is_concurrency_safe(&self) -> bool {
        self.inner.is_concurrency_safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        let mut out = self.inner.execute(ctx, args).await?;
        let findings = self.guard.redact_json(&mut out);
        if !findings.is_empty() {
            warn!(
                tool = self.inner.name(),
                count = findings.len(),
                "redacted secret values from tool result"
            );
        }
        Ok(out)
    }
}

/// Wrap a credential store so every decrypted value is registered with the
/// global guard at fetch time — once fetched, the value is masked in every
/// downstream surface (tool results, traces, memory, logs).
pub fn registering(inner: Arc<dyn CredentialStore>) -> Arc<dyn CredentialStore> {
    Arc::new(RegisteringCredentialStore {
        inner,
        guard: kyma_redact::global(),
    })
}

struct RegisteringCredentialStore {
    inner: Arc<dyn CredentialStore>,
    guard: Arc<SecretGuard>,
}

#[async_trait]
impl CredentialStore for RegisteringCredentialStore {
    async fn get(&self, tenant: TenantId, id: Uuid) -> anyhow::Result<Credential> {
        let c = self.inner.get(tenant, id).await?;
        register_credential(&self.guard, &c.value);
        Ok(c)
    }

    async fn update_value(
        &self,
        tenant: TenantId,
        id: Uuid,
        value: &CredentialValue,
    ) -> anyhow::Result<()> {
        register_credential(&self.guard, value);
        self.inner.update_value(tenant, id, value).await
    }
}

/// Register every secret field of a credential value. Exhaustive match so a
/// new variant is a compile error here, not a silent leak.
fn register_credential(guard: &SecretGuard, v: &CredentialValue) {
    match v {
        CredentialValue::Pat { token } => guard.register_value("credential", token),
        CredentialValue::Basic { password, .. } => guard.register_value("credential", password),
        CredentialValue::Oauth2 {
            access_token,
            refresh_token,
            ..
        } => {
            guard.register_value("credential", access_token);
            if let Some(r) = refresh_token {
                guard.register_value("credential", r);
            }
        }
        CredentialValue::Url { connection_string } => {
            guard.register_value("credential", connection_string)
        }
        CredentialValue::AwsCreds {
            secret_access_key,
            session_token,
            ..
        } => {
            guard.register_value("credential", secret_access_key);
            if let Some(t) = session_token {
                guard.register_value("credential", t);
            }
        }
        CredentialValue::ApiKey { value, .. } => guard.register_value("credential", value),
        CredentialValue::GithubApp {
            private_key_pem, ..
        } => guard.register_value("credential", private_key_pem),
    }
}
```

(Verify the `CredentialStore` trait import path: `state.rs:6` uses `kyma_core::credentials::CredentialStore`. If `adk_rust::Result` is not the trait's return type, match whatever `crates/kyma-mcp/src/tools.rs` sees from `tool.execute(...)`.)

- [ ] **Step 3: Register the module and re-export**

In `crates/kyma-server/src/agent/mod.rs`, add alongside the other module declarations:

```rust
pub mod redacting;
```

and to the re-export block (near `pub use state::AgentState;` at line 40):

```rust
pub use redacting::{redacted, registering};
```

Add `kyma-redact.workspace = true` to `crates/kyma-server/Cargo.toml` `[dependencies]`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p kyma-server redacting`
Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server
git commit -m "feat(agent): RedactingTool + credential-registering store decorators"
```

---

### Task 6: Wrap every tool registration + credential store construction

**Files:**
- Modify: `crates/kyma-server/src/agent/runner.rs:155-175` (the `.tool(...)` chain)
- Modify: `crates/kyma-mcp/src/tools.rs:25-79` (`ToolDispatch::new`)
- Modify: `crates/kyma-bin/src/main.rs` (cred store construction + AgentState at :381)
- Modify: `crates/kyma-local/src/lib.rs` (AgentState at :264, ToolDispatch at :333)

- [ ] **Step 1: Wrap ADK runner tools**

In `crates/kyma-server/src/agent/runner.rs`, add the import:

```rust
use super::redacting::redacted;
```

Then wrap every `.tool(tool_x(shared.clone()))` call in the builder chain (lines ~155-175) as `.tool(redacted(tool_x(shared.clone())))`. Example for the first three (apply the same to ALL of them — there are ~20):

```rust
        .tool(redacted(tool_list_databases(shared.clone())))
        .tool(redacted(tool_explore_schema(shared.clone())))
        .tool(redacted(tool_describe_table(shared.clone())))
```

- [ ] **Step 2: Wrap MCP dispatch tools**

In `crates/kyma-mcp/src/tools.rs`, extend the existing `use kyma_server::agent::{...}` import list with `redacted`. Then in `ToolDispatch::new`, wrap every inserted tool, e.g.:

```rust
        map.insert("list_databases", redacted(tool_list_databases(shared.clone())));
        map.insert("describe_table", redacted(tool_describe_table(shared.clone())));
        map.insert("run_sql", redacted(tool_run_sql(shared.clone())));
```

Apply to ALL `map.insert` lines in the function.

- [ ] **Step 3: Wrap credential stores at construction**

In `crates/kyma-bin/src/main.rs`, find the `let cred_store =` construction (search `cred_store`); wrap the constructed value:

```rust
    let cred_store = kyma_server::agent::registering(cred_store);
```

placed immediately after the original construction so connectors AND the agent both go through the registering wrapper. (If the variable is used as a concrete type somewhere, instead wrap only at the `AgentState { credentials: ... }` field: `credentials: kyma_server::agent::registering(cred_store.clone()),`.)

In `crates/kyma-local/src/lib.rs:264`, apply the same wrap to the `credentials:` field of its `AgentState`, and add `kyma_redact::register_env_secrets();` near the start of the serve entry point (the function containing line 264). Add `kyma-redact.workspace = true` to `crates/kyma-local/Cargo.toml` `[dependencies]`.

- [ ] **Step 4: Build + run existing agent tests**

Run: `cargo build --workspace && cargo test -p kyma-server -p kyma-mcp -p kyma-local`
Expected: clean build; pre-existing tests still pass (the decorator is behavior-preserving for secret-free results).

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server crates/kyma-mcp crates/kyma-bin crates/kyma-local
git commit -m "feat(agent): scrub all tool results before model context; register decrypted credentials"
```

---

### Task 7: Emitter integration + `persist_run`/`persist_turn` scrub (ADK path)

**Files:**
- Modify: `crates/kyma-server/src/agent/routes.rs` (Emitter at :358-523, `persist_run` at :879, callers unchanged)
- Modify: `crates/kyma-server/src/agent/sessions.rs` (`persist_turn` at :114)

- [ ] **Step 1: Write the failing test (append to `routes.rs` test module, or create `mod emitter_redact_tests` near the Emitter)**

```rust
#[cfg(test)]
mod emitter_redact_tests {
    use super::*;

    #[test]
    fn emitter_scrubs_tool_results_and_records_security_event() {
        let (ui, _rx) = ui_stream::channel();
        let mut em = Emitter::new(ui, "msg-1");
        em.tool_result(
            "run_sql",
            json!({"rows": [{"pat": "github_pat_11A22PAOI0QD7qilOFoYIK_9beaDlLnkO4GA0e"}]}),
        );
        let trace = serde_json::to_value(&em.trace).unwrap();
        let s = trace.to_string();
        assert!(!s.contains("github_pat_11A22"), "raw secret in trace: {s}");
        assert!(s.contains("[REDACTED:github-pat]"));
        assert!(s.contains("security_event"));
    }

    #[test]
    fn emitter_scrubs_streamed_answer_deltas() {
        let (ui, _rx) = ui_stream::channel();
        let mut em = Emitter::new(ui, "msg-2");
        em.answer_delta("the key is github_pat_11A22PAOI0");
        em.answer_delta("QD7qilOFoYIK_9beaDlLnkO4GA0e done");
        em.answer_final("ignored", None, None);
        let s = serde_json::to_value(&em.trace).unwrap().to_string();
        assert!(!s.contains("github_pat_11A22"), "raw secret in trace: {s}");
        assert!(s.contains("[REDACTED:github-pat]"));
    }
}
```

(If `em.trace` is private to the module this test lives outside of, place the test module inside `routes.rs` where `Emitter` is defined — it is module-private there already, so same-file tests can read it. Check `TraceFrame` derives `Serialize`; it must, since it's serialized into `trace_json`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kyma-server emitter_redact`
Expected: FAIL — raw secret present / `security_event` missing.

- [ ] **Step 3: Extend the `Emitter` struct and constructor (routes.rs:358-387)**

Add fields:

```rust
struct Emitter {
    // ... existing fields unchanged ...
    /// Shared secret guard — every recorded/streamed payload passes through it.
    guard: std::sync::Arc<kyma_redact::SecretGuard>,
    /// Incremental scrubbers for the open answer/reasoning blocks.
    answer_scrub: Option<kyma_redact::StreamScrubber>,
    reasoning_scrub: Option<kyma_redact::StreamScrubber>,
}
```

In `Emitter::new`, initialize:

```rust
            guard: kyma_redact::global(),
            answer_scrub: None,
            reasoning_scrub: None,
```

- [ ] **Step 4: Add the audit helper (inside `impl Emitter`)**

```rust
    /// Record + surface a redaction audit event. Never carries the value.
    fn security(&mut self, surface: &'static str, findings: &[kyma_redact::Finding]) {
        if findings.is_empty() {
            return;
        }
        let kinds: Vec<&str> = findings.iter().map(|f| f.kind.as_str()).collect();
        warn!(surface, count = findings.len(), ?kinds, "redacted secrets from agent stream");
        self.record(
            "security_event",
            json!({ "surface": surface, "count": findings.len(), "kinds": kinds }),
        );
        self.ui.data(
            "security",
            json!({ "surface": surface, "count": findings.len(), "kinds": kinds }),
        );
    }
```

- [ ] **Step 5: Scrub tool calls/results (replace bodies at :454-478)**

```rust
    fn tool_call(&mut self, tool: &str, mut args: Value, call_index: u32) {
        let findings = self.guard.redact_json(&mut args);
        self.record(
            "tool_call",
            json!({ "tool": tool, "args": args, "call_index": call_index }),
        );
        self.security("tool_call", &findings);
        self.close_text();
        self.close_reasoning();
        let id = format!("call-{}", self.tool_seq);
        self.tool_seq += 1;
        self.tool_ids
            .entry(tool.to_string())
            .or_default()
            .push_back(id.clone());
        self.ui.tool_input_available(&id, tool, args);
    }

    fn tool_result(&mut self, tool: &str, mut result: Value) {
        let findings = self.guard.redact_json(&mut result);
        self.record("tool_result", json!({ "tool": tool, "result": result }));
        self.security("tool_result", &findings);
        let id = self
            .tool_ids
            .get_mut(tool)
            .and_then(|q| q.pop_front())
            .unwrap_or_else(|| format!("call-{tool}"));
        self.ui.tool_output_available(&id, result);
    }
```

Note: `args`/`result` params become `mut`; the `json!` now embeds the redacted value, and the same redacted value goes to the UI stream.

- [ ] **Step 6: Route deltas through scrubbers (replace `answer_delta`/`thinking_delta` at :424-452 and `close_text`/`close_reasoning` at :399-408)**

```rust
    fn answer_delta(&mut self, text: &str) {
        self.close_reasoning();
        let guard = self.guard.clone();
        let safe = self
            .answer_scrub
            .get_or_insert_with(|| kyma_redact::StreamScrubber::new(guard))
            .push(text);
        if safe.is_empty() {
            return;
        }
        self.record("answer_delta", json!({ "text": safe }));
        let id = match &self.text_id {
            Some(id) => id.clone(),
            None => {
                let id = self.next_block();
                self.ui.text_start(&id);
                self.text_id = Some(id.clone());
                id
            }
        };
        self.ui.text_delta(&id, &safe);
    }

    fn thinking_delta(&mut self, text: &str) {
        self.close_text();
        let guard = self.guard.clone();
        let safe = self
            .reasoning_scrub
            .get_or_insert_with(|| kyma_redact::StreamScrubber::new(guard))
            .push(text);
        if safe.is_empty() {
            return;
        }
        self.record("thinking_delta", json!({ "text": safe }));
        let id = match &self.reasoning_id {
            Some(id) => id.clone(),
            None => {
                let id = self.next_block();
                self.ui.reasoning_start(&id);
                self.reasoning_id = Some(id.clone());
                id
            }
        };
        self.ui.reasoning_delta(&id, &safe);
    }

    fn close_text(&mut self) {
        if let Some(mut scrub) = self.answer_scrub.take() {
            let tail = scrub.finish();
            let findings = scrub.take_findings();
            if !tail.is_empty() {
                self.record("answer_delta", json!({ "text": tail }));
                let id = match &self.text_id {
                    Some(id) => id.clone(),
                    None => {
                        let id = self.next_block();
                        self.ui.text_start(&id);
                        self.text_id = Some(id.clone());
                        id
                    }
                };
                self.ui.text_delta(&id, &tail);
            }
            self.security("answer", &findings);
        }
        if let Some(id) = self.text_id.take() {
            self.ui.text_end(&id);
        }
    }

    fn close_reasoning(&mut self) {
        if let Some(mut scrub) = self.reasoning_scrub.take() {
            let tail = scrub.finish();
            let findings = scrub.take_findings();
            if !tail.is_empty() {
                self.record("thinking_delta", json!({ "text": tail }));
                let id = match &self.reasoning_id {
                    Some(id) => id.clone(),
                    None => {
                        let id = self.next_block();
                        self.ui.reasoning_start(&id);
                        self.reasoning_id = Some(id.clone());
                        id
                    }
                };
                self.ui.reasoning_delta(&id, &tail);
            }
            self.security("thinking", &findings);
        }
        if let Some(id) = self.reasoning_id.take() {
            self.ui.reasoning_end(&id);
        }
    }
```

CAUTION — recursion check: `close_text` calls `security` which calls `record` and `ui.data` only; no recursion. `answer_delta` calls `close_reasoning` (flushes reasoning scrub) — intended: switching block types finalizes the previous block.

- [ ] **Step 7: Redact `run_started` question and `answer_final` (at :416-422 and :483+)**

```rust
    fn run_started(&mut self, run_id: &str, model: &str, question: &str) {
        // The live question still reaches the model (the user chose to send
        // it); the trace copy is scrubbed.
        let (q, findings) = self.guard.redact_text(question);
        self.record(
            "run_started",
            json!({ "run_id": run_id, "model": model, "question": q }),
        );
        self.security("question", &findings);
        self.ui.data("model", json!({ "model": model }));
    }
```

In `answer_final`, FIRST flush blocks, THEN record redacted values. Replace the start of the function (the `record` + the two `close_*` calls) with:

```rust
    fn answer_final(&mut self, text: &str, sql_used: Option<&str>, kql_used: Option<&str>) {
        self.close_text();
        self.close_reasoning();
        let (t, findings) = self.guard.redact_text(text);
        let sql_used = sql_used.map(|s| self.guard.redact_text(s).0);
        let kql_used = kql_used.map(|s| self.guard.redact_text(s).0);
        self.record(
            "answer_final",
            json!({ "text": t, "kql_used": kql_used, "sql_used": sql_used }),
        );
        self.security("answer_final", &findings);
        // ... keep the rest of the existing function body, but use the
        // redacted `sql_used`/`kql_used` Options (now Option<String>) for the
        // ui.data("sql"/"kql", ...) parts that follow.
```

Read the remainder of the existing function (after line 489) and substitute the redacted variables where `sql_used`/`kql_used` are emitted as data parts (adjust `&str` vs `String` with `.as_deref()` as needed).

- [ ] **Step 8: Belt-and-braces in `persist_run` (at :879) and `persist_turn` (sessions.rs:114)**

In `persist_run`, after the `let Some(pool) = pool else ...` line, insert:

```rust
    // Belt-and-braces: nothing secret-bearing is persisted even if an
    // upstream scrub was missed (e.g. error paths that build traces by hand).
    let guard = kyma_redact::global();
    let (question, _) = guard.redact_text(question);
    let mut trace_json = trace_json.clone();
    let _ = guard.redact_json(&mut trace_json);
    let question = question.as_str();
    let trace_json = &trace_json;
```

(Shadowing keeps the `.bind(question)` / `.bind(SqlxJson(trace_json.clone()))` lines below unchanged.)

In `sessions.rs` `persist_turn`, after the `let Some(pool) = pool else ...` line, insert:

```rust
    let (text, _) = kyma_redact::global().redact_text(text);
    let text = text.as_str();
```

- [ ] **Step 9: Run tests**

Run: `cargo test -p kyma-server`
Expected: new emitter tests PASS; existing tests PASS. The most likely breakage: tests asserting exact trace shapes for streamed deltas (deltas are now coalesced by the holdback window). Fix such tests by asserting on concatenated delta text rather than per-delta equality.

- [ ] **Step 10: Commit**

```bash
git add crates/kyma-server
git commit -m "feat(agent): scrub Emitter trace/UI stream + persisted runs and turns"
```

---

### Task 8: Scrub the Claude CLI route (`ask_via_claude_cli`)

**Files:**
- Modify: `crates/kyma-server/src/agent/routes.rs:1446-1608`

- [ ] **Step 1: Add a security data-part helper near `ask_via_claude_cli`**

```rust
/// Surface a redaction audit event on the CLI-path UI stream.
fn emit_security(ui: &ui_stream::UiStream, surface: &str, findings: &[kyma_redact::Finding]) {
    if findings.is_empty() {
        return;
    }
    let kinds: Vec<&str> = findings.iter().map(|f| f.kind.as_str()).collect();
    warn!(surface, count = findings.len(), ?kinds, "redacted secrets from claude_cli stream");
    ui.data(
        "security",
        json!({ "surface": surface, "count": findings.len(), "kinds": kinds }),
    );
}
```

- [ ] **Step 2: Scrub the event loop**

Inside the `tokio::spawn` in `ask_via_claude_cli` (after `let mut claude_session = ...`), add:

```rust
        let guard = kyma_redact::global();
        // Per-block scrubbers: text/thinking blocks stream independently.
        let mut scrubs: HashMap<String, kyma_redact::StreamScrubber> = HashMap::new();
```

Replace the matching event arms (current lines 1518-1548):

```rust
                claude_cli::ClaudeEvent::TextStart { block_id } => ui.text_start(&block_id),
                claude_cli::ClaudeEvent::TextDelta { block_id, text } => {
                    let safe = scrubs
                        .entry(block_id.clone())
                        .or_insert_with(|| kyma_redact::StreamScrubber::new(guard.clone()))
                        .push(&text);
                    if !safe.is_empty() {
                        answer.push_str(&safe);
                        ui.text_delta(&block_id, &safe);
                    }
                }
                claude_cli::ClaudeEvent::TextEnd { block_id } => {
                    if let Some(mut s) = scrubs.remove(&block_id) {
                        let tail = s.finish();
                        if !tail.is_empty() {
                            answer.push_str(&tail);
                            ui.text_delta(&block_id, &tail);
                        }
                        emit_security(&ui, "answer", &s.take_findings());
                    }
                    ui.text_end(&block_id)
                }
                claude_cli::ClaudeEvent::ThinkingStart { block_id } => {
                    ui.reasoning_start(&block_id)
                }
                claude_cli::ClaudeEvent::ThinkingDelta { block_id, text } => {
                    let safe = scrubs
                        .entry(block_id.clone())
                        .or_insert_with(|| kyma_redact::StreamScrubber::new(guard.clone()))
                        .push(&text);
                    if !safe.is_empty() {
                        ui.reasoning_delta(&block_id, &safe);
                    }
                }
                claude_cli::ClaudeEvent::ThinkingEnd { block_id } => {
                    if let Some(mut s) = scrubs.remove(&block_id) {
                        let tail = s.finish();
                        if !tail.is_empty() {
                            ui.reasoning_delta(&block_id, &tail);
                        }
                        emit_security(&ui, "thinking", &s.take_findings());
                    }
                    ui.reasoning_end(&block_id)
                }
                claude_cli::ClaudeEvent::ToolUse { id, name, mut input } => {
                    let findings = guard.redact_json(&mut input);
                    emit_security(&ui, "tool_call", &findings);
                    ui.tool_input_available(&id, &name, input)
                }
                claude_cli::ClaudeEvent::ToolResult {
                    id,
                    mut output,
                    is_error,
                } => {
                    let findings = guard.redact_json(&mut output);
                    emit_security(&ui, "tool_result", &findings);
                    if is_error {
                        let txt = output
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| output.to_string());
                        ui.tool_output_error(&id, &txt);
                    } else {
                        ui.tool_output_available(&id, output);
                    }
                }
```

(Ensure `use std::collections::HashMap;` is already imported in routes.rs — it is, for `Emitter::tool_ids`.)

The final `persist_run` call in this function is already covered by Task 7's internal scrub. The `answer` string accumulates only scrubbed text now.

- [ ] **Step 3: Register the per-request bearer token (spec §2 registration table)**

In `ask_handler` (routes.rs:549-552), immediately after `auth_header` is extracted, register the raw token value so it is masked everywhere if it ever surfaces in data:

```rust
            if let Some(h) = &auth_header {
                let token = h.trim_start_matches("Bearer ").trim();
                kyma_redact::global().register_value("bearer", token);
            }
```

(`register_value` dedups identical values, so repeated requests from the same principal don't grow the registry.)

- [ ] **Step 4: Build + test**

Run: `cargo build -p kyma-server && cargo test -p kyma-server`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server/src/agent/routes.rs
git commit -m "feat(agent): scrub claude_cli stream — deltas, tool I/O, audit data parts"
```

---

### Task 9: Claude CLI subprocess containment (env allowlist + stdin question)

**Files:**
- Modify: `crates/kyma-server/src/agent/engine/claude_cli.rs:134-196` (+ tests at bottom)

- [ ] **Step 1: Write the failing env-filter test (append to the existing `mod tests`)**

```rust
    #[test]
    fn child_env_filter_blocks_server_secrets() {
        let vars = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/Users/x".to_string()),
            ("ANTHROPIC_API_KEY".to_string(), "sk-ant-xyz".to_string()),
            ("CLAUDE_CODE_FOO".to_string(), "1".to_string()),
            ("LC_ALL".to_string(), "en_US.UTF-8".to_string()),
            ("KYMA_SECRET_KEY".to_string(), "supersecret".to_string()),
            ("OPENAI_API_KEY".to_string(), "sk-openai".to_string()),
            ("KYMA_SUPABASE_JWT_SECRET".to_string(), "jwt".to_string()),
        ];
        let kept = filter_child_env(vars.into_iter());
        let names: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"PATH"));
        assert!(names.contains(&"HOME"));
        assert!(names.contains(&"ANTHROPIC_API_KEY"), "the CLI's own auth must pass");
        assert!(names.contains(&"CLAUDE_CODE_FOO"));
        assert!(names.contains(&"LC_ALL"));
        assert!(!names.contains(&"KYMA_SECRET_KEY"));
        assert!(!names.contains(&"OPENAI_API_KEY"));
        assert!(!names.contains(&"KYMA_SUPABASE_JWT_SECRET"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kyma-server claude_cli`
Expected: compile FAIL — `filter_child_env` not found.

- [ ] **Step 3: Implement env filtering (above `run_stream`)**

```rust
/// Env vars the spawned `claude` subprocess is allowed to inherit. Everything
/// else — `KYMA_SECRET_KEY`, provider keys, Supabase secrets — is withheld so
/// a CLI-side tool can never `echo` them into the transcript.
const CHILD_ENV_ALLOWLIST: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TERM", "LANG", "TMPDIR",
    // The CLI's own Anthropic auth (when not using the keychain).
    "ANTHROPIC_API_KEY",
];

fn filter_child_env(
    vars: impl Iterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    vars.filter(|(k, _)| {
        CHILD_ENV_ALLOWLIST.contains(&k.as_str())
            || k.starts_with("LC_")
            || k.starts_with("CLAUDE_")
    })
    .collect()
}
```

In `run_stream`, immediately after `let mut cmd = Command::new(&binary);` add:

```rust
    // Containment: start from an empty env and pass only the allowlist.
    cmd.env_clear();
    cmd.envs(filter_child_env(std::env::vars()));
```

- [ ] **Step 4: Pass the question via stdin instead of argv**

Replace `cmd.arg(question);` (line ~183) and `cmd.stdin(Stdio::null());` (line ~187) with:

```rust
    // The question goes via stdin, not argv — argv is world-readable in `ps`.
    cmd.stdin(Stdio::piped());
```

After `let mut child = cmd.spawn()...` and the stdout/stderr takes, add:

```rust
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdin on claude child"))?;
    let q = question.to_string();
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(q.as_bytes()).await;
        let _ = stdin.shutdown().await; // EOF signals end-of-prompt
    });
```

- [ ] **Step 5: Run tests + manual smoke (if a Claude CLI is installed)**

Run: `cargo test -p kyma-server claude_cli`
Expected: all PASS (existing stream-parsing tests don't spawn the binary).

Smoke (optional, requires `claude` on PATH and a configured engine): start the server, `POST /v1/agent/ask` with the ClaudeCli engine selected, confirm a normal answer streams back (stdin prompt path works).

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-server/src/agent/engine/claude_cli.rs
git commit -m "feat(agent): claude_cli containment — env allowlist + stdin prompt"
```

---

### Task 10: Memory writer auto-redaction

**Files:**
- Modify: `crates/kyma-memory/Cargo.toml` (add `kyma-redact.workspace = true`)
- Modify: `crates/kyma-memory/src/writer.rs:315-322` (`redact_create`)

- [ ] **Step 1: Write the failing test (append to the existing `mod redact_tests` in writer.rs)**

```rust
    #[test]
    fn redact_create_auto_masks_pattern_secrets() {
        let m = crate::types::CreateMemory {
            content: "deploy key is github_pat_11A22PAOI0QD7qilOFoYIK_9beaDlLnkO4GA0e ok".into(),
            title: Some("Bearer abcdefghijklmnopqrstuv12 note".into()),
            ..Default::default()
        };
        let red = super::redact_create(&m);
        assert_eq!(
            red.content,
            "deploy key is [REDACTED:github-pat] ok"
        );
        assert_eq!(red.title.as_deref(), Some("Bearer [REDACTED:bearer-token] note"));
    }
```

(If `CreateMemory` doesn't implement `Default`, construct it the way the existing tests in `crates/kyma-memory` do — copy a construction from `ingest.rs` tests.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kyma-memory redact`
Expected: FAIL — content unchanged.

- [ ] **Step 3: Extend `redact_create`**

```rust
/// Return a copy of `m` with `<private>…</private>` spans redacted AND any
/// detected secret values/patterns masked via the global guard. Cheap clone
/// on the (low-volume) save path.
///
/// Public so the async enqueue path can redact *before* a memory is persisted
/// to the durable job store — secrets must never reach any store, including
/// the queue.
pub fn redact_create(m: &CreateMemory) -> CreateMemory {
    let mut out = m.clone();
    out.content = redact_secrets(&redact_private(&m.content));
    out.title = m
        .title
        .as_deref()
        .map(|t| redact_secrets(&redact_private(t)));
    out
}

/// Auto-detect secrets (known values + patterns) in memory text.
fn redact_secrets(s: &str) -> String {
    let (red, findings) = kyma_redact::global().redact_text(s);
    if !findings.is_empty() {
        tracing::warn!(
            count = findings.len(),
            "memory content contained secrets; redacted before save"
        );
    }
    red
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kyma-memory`
Expected: all PASS (existing `<private>` tests unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-memory
git commit -m "feat(memory): auto-redact detected secrets on memory save"
```

---

### Task 11: One-time trace backfill

**Files:**
- Create: `crates/kyma-catalog/migrations/022_maintenance_flags.sql`
- Create: `crates/kyma-server/src/agent/backfill.rs`
- Modify: `crates/kyma-server/src/agent/mod.rs` (add `pub mod backfill;`)
- Modify: `crates/kyma-bin/src/main.rs` (spawn after AgentState at :381)

- [ ] **Step 1: Migration**

`crates/kyma-catalog/migrations/022_maintenance_flags.sql`:

```sql
-- One-shot maintenance task markers (e.g. the redaction backfill). A row's
-- presence means the named task ran (or is running); INSERT ... ON CONFLICT
-- DO NOTHING is the claim.
CREATE TABLE maintenance_flags (
    name    TEXT PRIMARY KEY,
    done_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

(Confirm `022` is the next free number — `021_external_auth.sql` is currently last.)

- [ ] **Step 2: Backfill module**

`crates/kyma-server/src/agent/backfill.rs`:

```rust
//! One-time redaction backfill: re-scrub `agent_runs.trace_json` /
//! `.question` and `agent_session_turns.content_json` written before the
//! redaction layer existed. Claimed via `maintenance_flags` so it runs once
//! per database, ever. Volumes are small (one row per ask), so a full-table
//! pass is fine.

use serde_json::Value;
use sqlx::types::Json as SqlxJson;
use sqlx::{PgPool, Row};
use tracing::{info, warn};

const FLAG: &str = "redact_backfill_v1";

/// Fire-and-forget: claims the flag and scrubs historic rows.
pub fn spawn(pool: PgPool) {
    tokio::spawn(async move {
        if let Err(e) = run(&pool).await {
            warn!(error = %e, "redaction backfill failed (will not retry this boot)");
        }
    });
}

async fn run(pool: &PgPool) -> anyhow::Result<()> {
    let claimed = sqlx::query(
        "INSERT INTO maintenance_flags (name) VALUES ($1) ON CONFLICT (name) DO NOTHING",
    )
    .bind(FLAG)
    .execute(pool)
    .await?
    .rows_affected()
        == 1;
    if !claimed {
        return Ok(());
    }

    let guard = kyma_redact::global();
    let mut updated = 0u64;

    let rows = sqlx::query("SELECT run_id, question, trace_json FROM agent_runs")
        .fetch_all(pool)
        .await?;
    for row in rows {
        let run_id: uuid::Uuid = row.get("run_id");
        let question: String = row.get("question");
        let SqlxJson(trace): SqlxJson<Value> = row.get("trace_json");
        let (q2, f1) = guard.redact_text(&question);
        let mut t2 = trace;
        let f2 = guard.redact_json(&mut t2);
        if f1.is_empty() && f2.is_empty() {
            continue;
        }
        sqlx::query("UPDATE agent_runs SET question = $2, trace_json = $3 WHERE run_id = $1")
            .bind(run_id)
            .bind(&q2)
            .bind(SqlxJson(t2))
            .execute(pool)
            .await?;
        updated += 1;
    }

    let rows =
        sqlx::query("SELECT session_id, turn_index, content_json FROM agent_session_turns")
            .fetch_all(pool)
            .await?;
    for row in rows {
        let sid: uuid::Uuid = row.get("session_id");
        let idx: i32 = row.get("turn_index");
        let SqlxJson(content): SqlxJson<Value> = row.get("content_json");
        let mut c2 = content;
        let f = guard.redact_json(&mut c2);
        if f.is_empty() {
            continue;
        }
        sqlx::query(
            "UPDATE agent_session_turns SET content_json = $3 \
             WHERE session_id = $1 AND turn_index = $2",
        )
        .bind(sid)
        .bind(idx)
        .bind(SqlxJson(c2))
        .execute(pool)
        .await?;
        updated += 1;
    }

    info!(rows = updated, "redaction backfill complete");
    Ok(())
}
```

Add `pub mod backfill;` to `crates/kyma-server/src/agent/mod.rs`.

- [ ] **Step 3: Spawn at server start**

In `crates/kyma-bin/src/main.rs`, right after the `agent_state` construction (line ~391):

```rust
    // One-time scrub of traces written before the redaction layer existed.
    kyma_server::agent::backfill::spawn(pg_pool.clone());
```

(`kyma-local` has no Postgres pool — nothing to backfill there.)

- [ ] **Step 4: Build + migration smoke**

Run: `cargo build -p kyma-server -p kyma-bin`
Expected: clean. If a local dev database is configured, boot the server once and confirm the log line `redaction backfill complete`; a second boot must log nothing (flag claimed). If no dev DB is available, rely on `cargo build` + the migration being picked up by the existing sqlx migration runner.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-catalog/migrations/022_maintenance_flags.sql crates/kyma-server crates/kyma-bin
git commit -m "feat(agent): one-time redaction backfill for historic traces"
```

---

### Task 12: Web UI display masking + shield badge

**Files:**
- Create: `web/src/lib/redact.ts`
- Create: `web/src/lib/redact.test.ts`
- Modify: `web/src/components/ai-elements/tool.tsx:124-163`
- Modify: `web/src/features/agent/AgentConsole.tsx` (MessageView at :201-290)

- [ ] **Step 1: Write the failing test**

`web/src/lib/redact.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { redactDisplay } from "./redact";

describe("redactDisplay", () => {
  it("masks a GitHub PAT", () => {
    expect(
      redactDisplay('GH_PAT="github_pat_11A22PAOI0QD7qilOFoYIK_9beaDlLnkO4GA0e"'),
    ).toBe('GH_PAT="[REDACTED:github-pat]"');
  });

  it("masks only the password in a connection URL", () => {
    expect(redactDisplay("postgres://kyma:s3cretPW@db:5432/k")).toBe(
      "postgres://kyma:[REDACTED:url-credential]@db:5432/k",
    );
  });

  it("masks bearer token values", () => {
    expect(redactDisplay("Bearer abcdefghijklmnopqrstuvwxyz123456")).toBe(
      "Bearer [REDACTED:bearer-token]",
    );
  });

  it("is idempotent", () => {
    const once = redactDisplay("sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWx");
    expect(redactDisplay(once)).toBe(once);
  });

  it("leaves clean text alone", () => {
    const s = "SELECT count(*) FROM logs";
    expect(redactDisplay(s)).toBe(s);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd web && npx vitest run src/lib/redact.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `web/src/lib/redact.ts`**

```ts
/**
 * Client-side display masking — last line of defense for sessions persisted
 * before server-side redaction existed (the server scrubs at the source for
 * new runs). Mirrors the pattern list in crates/kyma-redact/src/lib.rs;
 * keep the two in sync when adding kinds.
 */
type Pattern = { kind: string; re: RegExp; keepGroup1?: boolean };

const PATTERNS: Pattern[] = [
  { kind: "github-pat", re: /\bgithub_pat_[A-Za-z0-9_]{20,}/g },
  { kind: "github-token", re: /\bgh[pousr]_[A-Za-z0-9]{20,}\b/g },
  { kind: "anthropic-key", re: /\bsk-ant-[A-Za-z0-9_-]{16,}\b/g },
  { kind: "api-key", re: /\bsk-[A-Za-z0-9_-]{16,}\b/g },
  { kind: "aws-key-id", re: /\bAKIA[0-9A-Z]{16}\b/g },
  { kind: "slack-token", re: /\bxox[bpars]-[A-Za-z0-9-]{10,}\b/g },
  {
    kind: "jwt",
    re: /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b/g,
  },
  {
    kind: "url-credential",
    re: /(:\/\/[^/\s:@"']{1,128}:)[^@\s/"'[\]]{1,256}(?=@)/g,
    keepGroup1: true,
  },
  {
    kind: "bearer-token",
    re: /\b(bearer\s+)[A-Za-z0-9._~+/-]{16,}=*/gi,
    keepGroup1: true,
  },
];

export function redactDisplay(text: string): string {
  let out = text;
  for (const p of PATTERNS) {
    out = p.keepGroup1
      ? out.replace(p.re, (_m, g1: string) => `${g1}[REDACTED:${p.kind}]`)
      : out.replace(p.re, `[REDACTED:${p.kind}]`);
  }
  return out;
}
```

- [ ] **Step 4: Run the test**

Run: `cd web && npx vitest run src/lib/redact.test.ts`
Expected: PASS.

- [ ] **Step 5: Apply in the tool renderer**

In `web/src/components/ai-elements/tool.tsx`, add the import and route both render paths through it:

```tsx
import { redactDisplay } from "@/lib/redact";
```

`JsonBlock` (line ~147):

```tsx
function JsonBlock({ value }: { value: unknown }) {
  const text = redactDisplay(
    typeof value === "string" ? value : safeStringify(value),
  );
  // ... unchanged pre ...
```

`ToolOutput` errorText branch (line ~130): change `{errorText}` to `{redactDisplay(errorText)}`.

- [ ] **Step 6: Apply in `MessageView` + shield badge**

In `web/src/features/agent/AgentConsole.tsx`:

```tsx
import { redactDisplay } from "@/lib/redact";
import { ShieldAlert } from "lucide-react"; // merge into the existing lucide import
```

- User bubble (line ~210): `{redactDisplay(text)}` instead of `{text}`.
- Reasoning part (line ~235): `<ReasoningContent>{redactDisplay(text)}</ReasoningContent>`.
- Text part (line ~242): `<Response key={i}>{redactDisplay(text)}</Response>`.
- After the `model` data-part lookup (line ~222), add:

```tsx
  const security = findDataPart<{ count: number; kinds?: string[] }>(
    message,
    "data-security",
  );
```

- In the footer area (next to the usage/model row, line ~279), add:

```tsx
        {security && (
          <div className="flex items-center gap-1.5 pt-1 text-[11px] text-amber-600 dark:text-amber-500">
            <ShieldAlert className="h-3.5 w-3.5" />
            <span>
              secret value{(security.count ?? 1) === 1 ? "" : "s"} redacted
              {security.kinds?.length ? ` (${security.kinds.join(", ")})` : ""}
            </span>
          </div>
        )}
```

- [ ] **Step 7: Typecheck + test the web app**

Run: `cd web && npm run typecheck && npx vitest run`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add web/src/lib/redact.ts web/src/lib/redact.test.ts web/src/components/ai-elements/tool.tsx web/src/features/agent/AgentConsole.tsx
git commit -m "feat(web): client-side secret display masking + redaction shield badge"
```

---

### Task 13: Repo hygiene — PAT file + gitignore

**Files:**
- Delete: `read_only_token`
- Modify: `.gitignore`

- [ ] **Step 1: Verify the PAT was never committed**

Run: `git log --all --oneline -- read_only_token`
Expected: empty output (file is untracked). If NOT empty, STOP and tell the user — the PAT is in git history and needs rotation + history scrubbing, which is their call.

- [ ] **Step 2: Delete the file and extend .gitignore**

```bash
rm /Users/shakedaskayo/shaked/projects/kyma/read_only_token
```

Append to `.gitignore` (after the `.env.local` line):

```
.env.*
# Never commit token/credential files
*_token
*.token
```

- [ ] **Step 3: Verify ignore works**

Run: `touch read_only_token && git status --porcelain read_only_token; rm read_only_token`
Expected: no output from git status (ignored).

- [ ] **Step 4: Commit + remind the user**

```bash
git add .gitignore
git commit -m "chore: gitignore token files; remove local plaintext PAT"
```

Report to the user: the local `read_only_token` file is deleted — **rotate that GitHub PAT now** (it appeared on disk in plaintext; treat it as burned).

---

### Task 14: Final verification sweep

**Files:** none (verification only)

- [ ] **Step 1: Full workspace build + tests**

Run: `cargo build --workspace && cargo test --workspace`
Expected: clean (compare against the pre-Task-1 baseline for any pre-existing failures).

- [ ] **Step 2: Lints**

Run: `cargo clippy --workspace -- -D warnings`
Expected: clean (the workspace has `[lints] workspace = true`; fix anything the new code introduced).

- [ ] **Step 3: Web build**

Run: `cd web && npm run build`
Expected: clean.

- [ ] **Step 4: End-to-end behavioral check (requires dev server + DB)**

1. Boot the server; confirm the `redaction backfill complete` log line on first boot.
2. Ask the agent a question whose SQL result includes a seeded fake token (e.g. insert a row containing `github_pat_TESTTESTTESTTESTTESTTESTTESTTEST12` into any test table first).
3. Confirm: the streamed tool output shows `[REDACTED:github-pat]`; the shield badge appears; `SELECT trace_json FROM agent_runs ORDER BY started_at DESC LIMIT 1` contains no raw token; server logs contain no raw token.

- [ ] **Step 5: Commit any verification fixes, then report**

Summarize results to the user, including the PAT-rotation reminder from Task 13.
