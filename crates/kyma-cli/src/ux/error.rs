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
    eprint!("{}", render_error(err, theme::stderr_color_enabled()));
}

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
        Some("is `kyma serve` running? check the URL from `kyma status`")
    } else if text.contains("401") || text.contains("unauthorized") {
        Some("run `kyma connect` to re-authenticate")
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
            Some("is `kyma serve` running? check the URL from `kyma status`")
        );
    }

    #[test]
    fn hint_for_unauthorized() {
        let err = anyhow!("request failed: 401 Unauthorized");
        assert_eq!(
            hint_for(&err),
            Some("run `kyma connect` to re-authenticate")
        );
    }

    #[test]
    fn hint_for_unknown_returns_none() {
        let err = anyhow!("something entirely novel broke");
        assert_eq!(hint_for(&err), None);
    }

    #[test]
    fn render_error_no_color_snapshot() {
        let err = anyhow!("connection refused").context("failed to reach kyma server");
        insta::assert_snapshot!(render_error(&err, false));
    }

    #[test]
    fn render_error_includes_full_chain() {
        let err = anyhow!("root cause")
            .context("middle layer")
            .context("top layer");
        let rendered = render_error(&err, false);
        assert!(rendered.contains("top layer"));
        assert!(rendered.contains("middle layer"));
        assert!(rendered.contains("root cause"));
    }
}
