//! Discriminated frame types emitted by `POST /v1/explore/search`.
//! Wire format: one JSON object per line, NDJSON.
//!
//! See spec section 4.1.

use serde::Serialize;

use super::compile::DroppedClause;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    Plan {
        sources: Vec<PlanSource>,
    },
    SourceProgress {
        source: String,
        state: ProgressState,
    },
    Rows {
        source: String,
        rows: Vec<serde_json::Value>,
    },
    Histogram {
        source: String,
        buckets: Vec<Bucket>,
    },
    SourceDone {
        source: String,
        total: usize,
        capped: bool,
        dropped_clauses: Vec<DroppedClause>,
    },
    Error {
        source: Option<String>,
        code: String,
        message: String,
    },
    Done {
        elapsed_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanSource {
    pub source: String, // "db.table"
    pub has_timestamp: bool,
    /// Name of the timestamp-typed column used for time filtering/sorting,
    /// when one exists. Additive — older clients ignore it.
    pub timestamp_column: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProgressState {
    Running,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub t: String, // ISO-8601 UTC
    pub n: u64,
}

pub fn frame_to_line(f: &Frame) -> String {
    let mut s = serde_json::to_string(f).expect("frame serialization is infallible");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_frame_round_trips() {
        let f = Frame::Plan {
            sources: vec![
                PlanSource {
                    source: "prod.otel_logs".into(),
                    has_timestamp: true,
                    timestamp_column: Some("timestamp".into()),
                },
                PlanSource {
                    source: "prod.metrics".into(),
                    has_timestamp: false,
                    timestamp_column: None,
                },
            ],
        };
        let line = frame_to_line(&f);
        assert!(line.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["type"], "plan");
        assert_eq!(v["sources"][0]["source"], "prod.otel_logs");
        assert_eq!(v["sources"][0]["has_timestamp"], true);
        assert_eq!(v["sources"][0]["timestamp_column"], "timestamp");
        assert!(v["sources"][1]["timestamp_column"].is_null());
    }

    #[test]
    fn done_frame_serializes() {
        let f = Frame::Done { elapsed_ms: 842 };
        let line = frame_to_line(&f);
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["type"], "done");
        assert_eq!(v["elapsed_ms"], 842);
    }

    #[test]
    fn error_frame_includes_source_when_present() {
        let f = Frame::Error {
            source: Some("prod.http_reqs".into()),
            code: "timeout".into(),
            message: "per-source timeout".into(),
        };
        let v: serde_json::Value = serde_json::from_str(frame_to_line(&f).trim()).unwrap();
        assert_eq!(v["source"], "prod.http_reqs");
        assert_eq!(v["code"], "timeout");
    }

    #[test]
    fn source_done_includes_dropped_clauses() {
        use crate::discover::compile::{DropReason, DroppedClause};
        use crate::discover::grammar::Clause;

        let f = Frame::SourceDone {
            source: "prod.otel_logs".into(),
            total: 1238,
            capped: true,
            dropped_clauses: vec![DroppedClause {
                reason: DropReason::UnknownField,
                clause: Clause::Eq {
                    field: "foo".into(),
                    value: "bar".into(),
                },
            }],
        };
        let v: serde_json::Value = serde_json::from_str(frame_to_line(&f).trim()).unwrap();
        assert_eq!(v["type"], "source_done");
        assert_eq!(v["capped"], true);
        assert_eq!(v["dropped_clauses"][0]["reason"], "unknown_field");
    }
}
