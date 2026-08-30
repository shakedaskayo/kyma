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
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, RwLock};

/// One detected secret. Carries the kind only — never the value.
#[derive(Debug, Clone)]
pub struct Finding {
    /// The kind of secret detected (e.g. `"github-pat"`, `"api-key"`).
    pub kind: String,
}

struct Pattern {
    kind: &'static str,
    re: Regex,
    /// Capture group to replace; 0 = the whole match.
    group: usize,
}

fn build_re(re: &str) -> Regex {
    Regex::new(re).expect("static redaction regex must compile")
}

/// Build a regex with an increased NFA size limit (needed for the
/// private-key pattern whose `(?s).{0,16384}` body exceeds the default).
fn build_re_large(re: &str) -> Regex {
    regex::RegexBuilder::new(re)
        .size_limit(64 * 1024 * 1024) // 64 MiB — enough for the PEM body
        .build()
        .expect("static redaction regex must compile")
}

fn patterns() -> Vec<Pattern> {
    let p = |kind, re: &str, group| Pattern {
        kind,
        re: build_re(re),
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
        // Private key uses a large NFA; build with an elevated size limit.
        Pattern {
            kind: "private-key",
            re: build_re_large(
                r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.{0,16384}?-----END [A-Z ]*PRIVATE KEY-----",
            ),
            group: 0,
        },
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

/// Central secret guard: combines known-value exact matching with pattern
/// matching to redact secrets from text and JSON values.
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
    /// Create a new `SecretGuard` with the built-in pattern set.
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

    /// Byte ranges of every detection hit (known values + patterns) in
    /// `text`, unmerged and in no particular order. Used by
    /// [`StreamScrubber`] to avoid cutting the emission boundary through a
    /// match.
    pub fn find_spans(&self, text: &str) -> Vec<std::ops::Range<usize>> {
        let mut spans = Vec::new();
        {
            let k = self.known.read().expect("SecretGuard lock poisoned");
            if let Some(ac) = &k.ac {
                for m in ac.find_iter(text) {
                    spans.push(m.start()..m.end());
                }
            }
        }
        for p in &self.patterns {
            for m in p.re.find_iter(text) {
                spans.push(m.start()..m.end());
            }
        }
        spans
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
            let _ = write!(out, "[REDACTED:{kind}]");
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

/// Return a clone of the process-global [`SecretGuard`].
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
    /// Create a new `StreamScrubber` backed by the given guard.
    pub fn new(guard: Arc<SecretGuard>) -> Self {
        Self {
            guard,
            buf: String::new(),
            findings: Vec::new(),
        }
    }

    /// Append a delta; returns redacted text that is safe to emit now
    /// (may be empty while the holdback window fills).
    ///
    /// Raw (un-redacted) input is accumulated in the buffer. When the buffer
    /// exceeds [`STREAM_HOLDBACK_BYTES`], the leading portion is redacted and
    /// emitted; only the holdback tail remains buffered raw. This ensures
    /// that a secret split across several small deltas is still caught: the
    /// full token assembles in the holdback window before it is ever scanned.
    ///
    /// The cut is **match-aware**: before draining, the full raw buffer is
    /// scanned with [`SecretGuard::find_spans`]. If any detected span crosses
    /// the provisional cut point, the cut is pulled back to the start of that
    /// span so no secret is ever bisected at the drain boundary. If pulling
    /// the cut back reaches zero (the entire buffer is one big match, or a
    /// match starts at byte 0), an empty string is returned and the whole
    /// buffer is held until either the match resolves or [`finish`](Self::finish)
    /// is called.
    pub fn push(&mut self, delta: &str) -> String {
        self.buf.push_str(delta);
        if self.buf.len() <= STREAM_HOLDBACK_BYTES {
            return String::new();
        }
        // Provisional cut: advance to a char boundary going forward.
        let mut cut = self.buf.len() - STREAM_HOLDBACK_BYTES;
        while !self.buf.is_char_boundary(cut) {
            cut += 1;
        }
        // Pull cut back past any span that would be bisected.
        let spans = self.guard.find_spans(&self.buf);
        for span in &spans {
            if span.start < cut && span.end > cut {
                // This match crosses the cut — pull the cut back to span.start.
                if span.start < cut {
                    cut = span.start;
                }
            }
        }
        // Snap the (possibly retracted) cut back to a valid char boundary
        // going downward.
        while cut > 0 && !self.buf.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut == 0 {
            // The whole buffer must be held (a match starts at byte 0 or
            // covers the entire holdback window).
            return String::new();
        }
        let prefix: String = self.buf.drain(..cut).collect();
        // Redact the emitted prefix.
        // The tail (self.buf) stays raw so a cross-delta secret assembles.
        let (red, mut f) = self.guard.redact_text(&prefix);
        self.findings.append(&mut f);
        red
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
        let (out, f) = guard().redact_text("postgres://pensieve:s3cretPW@db.host:5432/pensieve");
        assert_eq!(out, "postgres://pensieve:[REDACTED:url-credential]@db.host:5432/pensieve");
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

    #[test]
    fn stream_scrubber_no_leak_when_text_follows_secret() {
        let g = Arc::new(guard());
        let mut s = StreamScrubber::new(g);
        let pat = "github_pat_11A22PAOI0QD7qilOFoYIK_9beaDlLnkO4GA0eKKSGdG7YDYM6vGgs";
        let mut out = String::new();
        out.push_str(&s.push(&format!("answer start {pat} then ")));
        for _ in 0..20 {
            out.push_str(&s.push("more clean text follows here. "));
        }
        out.push_str(&s.finish());
        assert!(!out.contains("github_pat_11A22"), "leaked head: {out}");
        assert!(!out.contains("9beaDlLnkO4GA0e"), "leaked tail: {out}");
        assert!(out.contains("[REDACTED:github-pat]"));
        assert!(out.ends_with("more clean text follows here. "));
        assert_eq!(s.take_findings().len(), 1);
    }
}
