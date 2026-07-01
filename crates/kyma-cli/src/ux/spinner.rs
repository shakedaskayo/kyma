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
