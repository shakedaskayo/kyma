//! One consistent table preset used by every list-style command.

use comfy_table::{Cell, Color, ContentArrangement, Table};

/// Returns a table pre-configured with kyma's standard look: rounded
/// UTF-8 borders, dynamic content arrangement, and the given header row.
pub(crate) fn table(headers: Vec<&str>) -> Table {
    let mut t = Table::new();
    t.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers);
    t
}

/// A colored status glyph cell: green ✓ for healthy/ok/active/success,
/// yellow `~` for pending/degraded/paused, red ✗ for error/failed, gray
/// `?` for anything unrecognized.
pub(crate) fn status_cell(status: &str) -> Cell {
    let (glyph, color) = match status.to_ascii_lowercase().as_str() {
        "ok" | "healthy" | "active" | "success" => (super::theme::CHECK, Color::Green),
        "pending" | "degraded" | "paused" => ("~", Color::Yellow),
        "error" | "failed" | "unhealthy" => (super::theme::CROSS, Color::Red),
        _ => ("?", Color::DarkGrey),
    };
    Cell::new(format!("{glyph} {status}")).fg(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_renders_given_headers() {
        let t = table(vec!["NAME", "STATUS"]);
        let rendered = t.to_string();
        assert!(rendered.contains("NAME"));
        assert!(rendered.contains("STATUS"));
    }

    #[test]
    fn status_cell_known_statuses() {
        assert_eq!(status_cell("ok").content(), "✓ ok");
        assert_eq!(status_cell("error").content(), "✗ error");
        assert_eq!(status_cell("pending").content(), "~ pending");
    }

    #[test]
    fn status_cell_unknown_status() {
        assert_eq!(status_cell("weird").content(), "? weird");
    }
}
