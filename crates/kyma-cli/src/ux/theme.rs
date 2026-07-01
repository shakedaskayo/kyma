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
    style.force_styling(enabled).apply_to(text).to_string()
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
