//! Environment configuration: one place that owns the `PENSIEVE_` prefix and
//! the `~/.pensieve` home directory.
//!
//! Before this module, both were open-coded at the point of use — 236 distinct
//! `std::env::var("PENSIEVE_…")` call sites, and the home directory
//! independently re-derived in more than a dozen places, each repeating the
//! same "`PENSIEVE_HOME`, else `$HOME/.pensieve`" fallback slightly
//! differently. That duplication is a latent bug source (one site drifting from
//! the rest is invisible until someone's data lands in two places) and it is
//! what made the Kyma → Pensieve rename a 236-site edit instead of a one-line
//! one.
//!
//! ```
//! use pensieve_core::config;
//!
//! let addr = config::var_or("HTTP_ADDR", "127.0.0.1:8080");
//! let verbose = config::flag("INGEST_VERBOSE", false);
//! let workers: usize = config::parse("FABRIC_WORKERS").unwrap_or(4);
//! ```
//!
//! Suffixes are passed without the prefix: `var("HTTP_ADDR")` reads
//! `PENSIEVE_HTTP_ADDR`. Passing a full name by mistake is caught in debug
//! builds rather than silently reading `PENSIEVE_PENSIEVE_HTTP_ADDR`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::str::FromStr;

/// The prefix on every environment variable this project reads.
pub const ENV_PREFIX: &str = "PENSIEVE_";

/// The directory name under `$HOME` when `PENSIEVE_HOME` is unset.
pub const HOME_DIR_NAME: &str = ".pensieve";

/// Build the full variable name for a suffix.
///
/// Debug-asserts that the caller passed a suffix rather than a full name;
/// `key("PENSIEVE_HOME")` is always a mistake and would otherwise read
/// `PENSIEVE_PENSIEVE_HOME` and quietly fall back to the default.
pub fn key(suffix: &str) -> String {
    debug_assert!(
        !suffix.starts_with(ENV_PREFIX),
        "config::key() takes a suffix, not a full variable name: got {suffix:?} — \
         pass {:?} instead",
        suffix.trim_start_matches(ENV_PREFIX)
    );
    format!("{ENV_PREFIX}{suffix}")
}

/// Read `PENSIEVE_<suffix>`, treating empty as unset.
///
/// Empty-is-unset matters in CI and container land, where a variable is often
/// present but blank (`FOO=` in a compose file, an unset GitHub Actions
/// secret). Callers almost always want the default in that case.
pub fn var(suffix: &str) -> Option<String> {
    match std::env::var(key(suffix)) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

/// Read `PENSIEVE_<suffix>` as an `OsString`, for values that are paths and
/// may not be valid UTF-8.
pub fn var_os(suffix: &str) -> Option<OsString> {
    std::env::var_os(key(suffix)).filter(|v| !v.is_empty())
}

/// Read `PENSIEVE_<suffix>`, or fall back to `default`.
pub fn var_or(suffix: &str, default: &str) -> String {
    var(suffix).unwrap_or_else(|| default.to_string())
}

/// Parse `PENSIEVE_<suffix>` into `T`. Unset *and* unparseable both yield
/// `None`, so a typo in a tuning knob degrades to the default rather than
/// panicking a server on boot.
pub fn parse<T: FromStr>(suffix: &str) -> Option<T> {
    var(suffix)?.parse().ok()
}

/// Read `PENSIEVE_<suffix>` as a boolean flag.
///
/// Accepts `1/true/yes/on` and `0/false/no/off`, case-insensitively. Anything
/// else falls back to `default`.
pub fn flag(suffix: &str, default: bool) -> bool {
    match var(suffix) {
        Some(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        None => default,
    }
}

/// The pensieve home directory: `PENSIEVE_HOME`, else `$HOME/.pensieve`.
///
/// Returns `None` only when neither is available, which in practice means a
/// process with no `HOME` — callers that cannot proceed without a home should
/// surface that rather than inventing a relative path.
pub fn home() -> Option<PathBuf> {
    if let Some(explicit) = var_os("HOME") {
        return Some(PathBuf::from(explicit));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(HOME_DIR_NAME))
}

/// [`home`], or the given fallback when there is no home directory at all.
pub fn home_or(fallback: impl Into<PathBuf>) -> PathBuf {
    home().unwrap_or_else(|| fallback.into())
}

/// A path inside the pensieve home, e.g. `home_path("logs/server.log")`.
pub fn home_path(rel: impl AsRef<std::path::Path>) -> Option<PathBuf> {
    home().map(|h| h.join(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    // These mutate process-global environment, so they run under one lock and
    // one #[test] rather than racing each other across threads.
    #[test]
    fn reads_prefixed_vars_with_empty_as_unset() {
        std::env::set_var("PENSIEVE_TEST_PLAIN", "hello");
        std::env::set_var("PENSIEVE_TEST_BLANK", "   ");
        std::env::remove_var("PENSIEVE_TEST_ABSENT");

        assert_eq!(var("TEST_PLAIN").as_deref(), Some("hello"));
        assert_eq!(var("TEST_BLANK"), None, "blank should read as unset");
        assert_eq!(var("TEST_ABSENT"), None);
        assert_eq!(var_or("TEST_ABSENT", "fallback"), "fallback");

        std::env::set_var("PENSIEVE_TEST_NUM", "42");
        std::env::set_var("PENSIEVE_TEST_JUNK", "not-a-number");
        assert_eq!(parse::<u32>("TEST_NUM"), Some(42));
        assert_eq!(
            parse::<u32>("TEST_JUNK"),
            None,
            "unparseable must degrade to the default, not panic"
        );

        for (raw, expected) in [
            ("1", true), ("true", true), ("TRUE", true), ("yes", true), ("on", true),
            ("0", false), ("false", false), ("No", false), ("off", false),
        ] {
            std::env::set_var("PENSIEVE_TEST_FLAG", raw);
            assert_eq!(flag("TEST_FLAG", !expected), expected, "flag({raw:?})");
        }
        std::env::set_var("PENSIEVE_TEST_FLAG", "banana");
        assert!(flag("TEST_FLAG", true), "unrecognised keeps the default");
        assert!(!flag("TEST_FLAG", false));

        for k in ["TEST_PLAIN", "TEST_BLANK", "TEST_NUM", "TEST_JUNK", "TEST_FLAG"] {
            std::env::remove_var(key(k));
        }
    }

    #[test]
    fn key_builds_the_prefixed_name() {
        assert_eq!(key("HTTP_ADDR"), "PENSIEVE_HTTP_ADDR");
    }

    #[test]
    #[should_panic(expected = "takes a suffix")]
    #[cfg(debug_assertions)]
    fn key_rejects_an_already_prefixed_name() {
        // Silently reading PENSIEVE_PENSIEVE_HOME is worse than failing loudly.
        let _ = key("PENSIEVE_HOME");
    }

    #[test]
    fn home_prefers_the_explicit_override() {
        std::env::set_var("PENSIEVE_HOME", "/tmp/explicit-pensieve-home");
        assert_eq!(home(), Some(PathBuf::from("/tmp/explicit-pensieve-home")));
        assert_eq!(
            home_path("logs/server.log"),
            Some(PathBuf::from("/tmp/explicit-pensieve-home/logs/server.log"))
        );
        std::env::remove_var("PENSIEVE_HOME");
    }
}
