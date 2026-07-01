//! Formatting helpers shared across commands: relative timestamps,
//! word-boundary-aware truncation, and score-to-color mapping.

use chrono::{DateTime, Utc};

/// Human relative time, e.g. `"2m ago"`, `"3h ago"`, `"5d ago"`, or
/// `"just now"` for anything under 5 seconds.
pub(crate) fn relative_time(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = now.signed_duration_since(ts).num_seconds();
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Truncates `text` to at most `max_chars`, breaking on the last word
/// boundary before the limit and appending an ellipsis. Returns `text`
/// unchanged if it already fits.
pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    let cut = truncated.rfind(' ').unwrap_or(truncated.len());
    format!("{}…", &truncated[..cut])
}

/// Maps a similarity/relevance score in `[0.0, 1.0]` to a semantic color:
/// green at `>= 0.8`, yellow at `>= 0.5`, gray otherwise.
pub(crate) fn score_style(score: f32, text: &str) -> String {
    if score >= 0.8 {
        super::theme::success(text)
    } else if score >= 0.5 {
        super::theme::warn(text)
    } else {
        super::theme::muted(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts_secs_ago(secs: i64) -> (DateTime<Utc>, DateTime<Utc>) {
        let now = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
        (now - chrono::Duration::seconds(secs), now)
    }

    #[test]
    fn relative_time_just_now() {
        let (t, now) = ts_secs_ago(2);
        assert_eq!(relative_time(t, now), "just now");
    }

    #[test]
    fn relative_time_minutes() {
        let (t, now) = ts_secs_ago(90);
        assert_eq!(relative_time(t, now), "1m ago");
    }

    #[test]
    fn relative_time_hours() {
        let (t, now) = ts_secs_ago(7200);
        assert_eq!(relative_time(t, now), "2h ago");
    }

    #[test]
    fn relative_time_days() {
        let (t, now) = ts_secs_ago(172_800);
        assert_eq!(relative_time(t, now), "2d ago");
    }

    #[test]
    fn truncate_leaves_short_text_alone() {
        assert_eq!(truncate("hello world", 20), "hello world");
    }

    #[test]
    fn truncate_breaks_on_word_boundary() {
        assert_eq!(truncate("hello there world", 12), "hello there…");
    }

    #[test]
    fn score_style_buckets() {
        assert_eq!(score_style(0.9, "x"), super::super::theme::success("x"));
        assert_eq!(score_style(0.6, "x"), super::super::theme::warn("x"));
        assert_eq!(score_style(0.2, "x"), super::super::theme::muted("x"));
    }
}
