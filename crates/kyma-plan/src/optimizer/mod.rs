//! Optimizer rules for the unified logical plan.
//!
//! TODO(M5): implement telemetry-specific rewrites:
//!   - Intersect consecutive inverted-index lookups (`has "x" | where has "y"`).
//!   - Push count-thresholds into block-skip metadata.
//!   - Collapse `summarize by bin(ts, N)` expressions into time-series scans.
