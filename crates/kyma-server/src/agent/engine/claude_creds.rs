//! Read the Claude Code creds file at `~/.claude/.credentials.json` and
//! return an Anthropic API key if one is present.
//!
//! The file shape can vary between Claude Code versions, so we use a
//! permissive serde derive and a small set of well-known keys. If the file
//! doesn't exist, or doesn't contain a recognisable key, return `None` —
//! callers fall back to env vars and the catalog credential store.

use serde::Deserialize;
use std::path::PathBuf;

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
/// Returns `None` quietly if the file doesn't exist or doesn't yield a key —
/// not finding a key is an expected case, not an error.
pub fn discover_anthropic_key() -> Option<String> {
    let path = default_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
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

    #[test]
    fn missing_file_is_none() {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", "/nonexistent/kyma-test-home");
        assert_eq!(discover_anthropic_key(), None);
        if let Some(v) = prev {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn parses_top_level_api_key() {
        let raw = r#"{"apiKey":"sk-ant-test"}"#;
        let parsed: ClaudeCreds = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.api_key.as_deref(), Some("sk-ant-test"));
    }

    #[test]
    fn parses_active_subscription_key() {
        let raw = r#"{"subscriptions":[{"active":false,"apiKey":"old"},{"active":true,"apiKey":"new"}]}"#;
        let parsed: ClaudeCreds = serde_json::from_str(raw).unwrap();
        let active = parsed.subscriptions.into_iter().find(|s| s.active).unwrap();
        assert_eq!(active.api_key.as_deref(), Some("new"));
    }
}
