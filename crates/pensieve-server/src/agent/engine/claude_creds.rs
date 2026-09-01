//! Read the Claude Code creds file at `~/.claude/.credentials.json` and
//! return an Anthropic API key if one is present.
//!
//! The file shape can vary between Claude Code versions, so we use a
//! permissive serde derive and a small set of well-known keys. If the file
//! doesn't exist, or doesn't contain a recognisable key, return `None` —
//! callers fall back to env vars and the catalog credential store.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default)]
struct ClaudeCreds {
    #[serde(default, rename = "apiKey")]
    api_key: Option<String>,
    #[serde(default, rename = "ANTHROPIC_API_KEY")]
    anthropic_api_key: Option<String>,
    #[serde(default)]
    subscriptions: Vec<ClaudeSubscription>,
}

#[derive(Debug, Deserialize)]
struct ClaudeSubscription {
    #[serde(default)]
    active: bool,
    #[serde(default, rename = "apiKey")]
    api_key: Option<String>,
}

fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(".claude");
    p.push(".credentials.json");
    Some(p)
}

/// Try to discover an Anthropic API key from the Claude Code config dir.
///
/// Resolves the default path (`$HOME/.claude/.credentials.json`) and reads
/// it. Returns `None` quietly if `$HOME` isn't set, the file doesn't exist,
/// can't be parsed, or doesn't contain a recognisable key — not finding one
/// is an expected case, not an error.
pub fn discover_anthropic_key() -> Option<String> {
    let path = default_path()?;
    discover_anthropic_key_at(&path)
}

/// Same as [`discover_anthropic_key`] but reads from an explicit path. Kept
/// `pub(crate)` so tests can exercise the parser without mutating `$HOME` —
/// `set_var` would race against parallel tests.
pub(crate) fn discover_anthropic_key_at(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: ClaudeCreds = serde_json::from_str(&raw).ok()?;
    if let Some(k) = parsed.api_key.or(parsed.anthropic_api_key) {
        if !k.is_empty() {
            return Some(k);
        }
    }
    parsed
        .subscriptions
        .into_iter()
        .find(|s| s.active)
        .and_then(|s| s.api_key)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn missing_file_returns_none() {
        let path = PathBuf::from("/nonexistent/pensieve-test/.credentials.json");
        assert_eq!(discover_anthropic_key_at(&path), None);
    }

    #[test]
    fn parses_top_level_api_key_from_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), r#"{"apiKey":"sk-ant-test"}"#).unwrap();
        assert_eq!(
            discover_anthropic_key_at(tmp.path()),
            Some("sk-ant-test".into()),
        );
    }

    #[test]
    fn parses_active_subscription_key_from_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"subscriptions":[{"active":false,"apiKey":"old"},{"active":true,"apiKey":"new"}]}"#,
        )
        .unwrap();
        assert_eq!(
            discover_anthropic_key_at(tmp.path()),
            Some("new".into()),
        );
    }

    #[test]
    fn ignores_empty_keys() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), r#"{"apiKey":""}"#).unwrap();
        assert_eq!(discover_anthropic_key_at(tmp.path()), None);
    }
}
