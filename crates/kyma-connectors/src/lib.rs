#![forbid(unsafe_code)]
//! Ingestion connectors: pull-side integrations to third-party sources.
//!
//! See `docs/superpowers/specs/2026-04-20-connectors-design.md`.

pub mod metrics;
pub mod secrets;
pub mod types;

pub use types::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun, DriveModel};
