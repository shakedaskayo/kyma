//! Failure-signature extraction for CI job logs (E4a).
//!
//! Runs on the **redacted** job log to characterise a failure: a coarse `kind`,
//! a normalised `signature` that groups recurring failures (line numbers / paths
//! / hashes templated out), an error-line `error_count`, and a representative
//! `sample` line. The dreaming correlation pipeline (E4b) reads these off the
//! `github_job_logs` rows to find recurring problems and tie them to commits,
//! services, and memories.

/// A characterised CI failure. `failed == false` ⇒ the other fields are empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureSignature {
    pub failed: bool,
    /// `traceback` | `test_failure` | `build_error` | `exit_nonzero` | `generic` | `none`.
    pub kind: String,
    /// Normalised template of the salient error line — stable across runs whose
    /// only difference is line numbers / paths / hashes, so recurring failures
    /// group together.
    pub signature: String,
    /// Number of error-ish lines detected.
    pub error_count: usize,
    /// A representative raw (already-redacted) error line, truncated.
    pub sample: String,
}

/// A conclusion that means the job failed (vs. success / skipped / neutral).
fn conclusion_is_failure(conclusion: &str) -> bool {
    matches!(
        conclusion,
        "failure" | "timed_out" | "startup_failure" | "action_required"
    )
}

/// Markers that flag an error line. Order is irrelevant; classification
/// precedence is handled separately.
fn line_is_error(lower: &str) -> bool {
    const MARKERS: [&str; 14] = [
        "##[error]",
        "traceback (most recent call last)",
        "panicked at",
        "error[",
        "error:",
        " error ",
        "assertionerror",
        "exception",
        "fatal:",
        "segmentation fault",
        "process completed with exit code",
        "exited with code",
        "failed",
        "✗",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

/// Classify the dominant failure kind from the collected error lines.
fn classify(error_lines: &[&str]) -> &'static str {
    let any = |needle: &str| {
        error_lines
            .iter()
            .any(|l| l.to_ascii_lowercase().contains(needle))
    };
    if any("traceback (most recent call last)") || any("panicked at") {
        "traceback"
    } else if any("assertionerror")
        || any("✗")
        || any("test result: failed")
        || (any("failed") && any("test"))
    {
        "test_failure"
    } else if any("error[") || any("error:") {
        "build_error"
    } else if any("exit code") || any("exited with code") {
        "exit_nonzero"
    } else {
        "generic"
    }
}

/// Strip a leading GitHub-Actions timestamp (`2026-06-08T10:00:00.0000000Z `).
fn strip_ts(line: &str) -> &str {
    // Cheap heuristic: a leading token containing 'T' and ending 'Z' followed by
    // a space.
    if let Some((head, rest)) = line.split_once(' ') {
        if head.len() >= 20 && head.contains('T') && head.ends_with('Z') {
            return rest;
        }
    }
    line
}

/// Normalise an error line into a grouping template: drop the timestamp + the
/// `##[error]` prefix, template out digits and 0x-hashes, collapse whitespace,
/// and truncate.
fn normalise(line: &str) -> String {
    let line = strip_ts(line).trim();
    let line = line.strip_prefix("##[error]").unwrap_or(line);
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut prev_space = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            // Collapse a run of digits to a single '#'.
            out.push('#');
            while matches!(chars.peek(), Some(d) if d.is_ascii_digit()) {
                chars.next();
            }
            prev_space = false;
        } else if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    let out = out.trim();
    out.chars().take(200).collect()
}

/// Extract a [`FailureSignature`] from a redacted job log + its conclusion.
pub fn extract_failure_signature(log: &str, conclusion: &str) -> FailureSignature {
    if !conclusion_is_failure(conclusion) {
        return FailureSignature {
            failed: false,
            kind: "none".into(),
            ..Default::default()
        };
    }

    let error_lines: Vec<&str> = log
        .lines()
        .filter(|l| line_is_error(&l.to_ascii_lowercase()))
        .collect();

    let kind = classify(&error_lines).to_string();

    // The salient line: for a traceback, the LAST error line (the exception);
    // otherwise the FIRST.
    let salient = if kind == "traceback" {
        error_lines.last().copied()
    } else {
        error_lines.first().copied()
    };

    let signature = salient.map(normalise).unwrap_or_default();
    let sample = salient
        .map(|l| strip_ts(l).trim().chars().take(300).collect::<String>())
        .unwrap_or_default();

    FailureSignature {
        failed: true,
        kind,
        signature,
        error_count: error_lines.len(),
        sample,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_conclusion_is_not_a_failure() {
        let fs = extract_failure_signature("error: something noisy\n", "success");
        assert!(!fs.failed);
        assert_eq!(fs.kind, "none");
        assert!(fs.signature.is_empty());
    }

    #[test]
    fn python_traceback_classified_and_normalised() {
        let log = "\
2026-06-08T10:00:00.0000000Z Traceback (most recent call last):
2026-06-08T10:00:00.1000000Z   File \"app.py\", line 42, in main
2026-06-08T10:00:00.2000000Z ValueError: bad config at offset 1234
";
        let fs = extract_failure_signature(log, "failure");
        assert!(fs.failed);
        assert_eq!(fs.kind, "traceback");
        // Salient = the final exception line; digits templated to '#'.
        assert_eq!(fs.signature, "ValueError: bad config at offset #");
        assert!(fs.sample.contains("ValueError"));
        assert!(fs.error_count >= 1);
    }

    #[test]
    fn github_error_annotation_with_exit_code() {
        let log = "\
2026-06-08T10:00:00.0Z ##[error]Process completed with exit code 1.
2026-06-08T10:00:00.0Z some other line
";
        let fs = extract_failure_signature(log, "failure");
        assert!(fs.failed);
        assert_eq!(fs.kind, "exit_nonzero");
        // timestamp + ##[error] stripped, exit code templated.
        assert_eq!(fs.signature, "Process completed with exit code #.");
    }

    #[test]
    fn rust_build_error_classified() {
        let log = "2026-06-08T10:00:00.0Z error[E0382]: borrow of moved value: `x`\n";
        let fs = extract_failure_signature(log, "failure");
        assert!(fs.failed);
        assert_eq!(fs.kind, "build_error");
        assert_eq!(fs.signature, "error[E#]: borrow of moved value: `x`");
    }

    #[test]
    fn recurring_failures_share_a_signature() {
        let a = extract_failure_signature(
            "2026-06-08T10:00:00.0Z ##[error]Test failed at line 10\n",
            "failure",
        );
        let b = extract_failure_signature(
            "2026-06-09T11:11:11.0Z ##[error]Test failed at line 873\n",
            "failure",
        );
        // Different line numbers, same normalised signature → groups together.
        assert_eq!(a.signature, b.signature);
        assert_eq!(a.signature, "Test failed at line #");
    }
}
