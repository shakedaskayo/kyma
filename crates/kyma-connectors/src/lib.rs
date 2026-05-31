#![forbid(unsafe_code)]
//! Ingestion connectors: pull-side integrations to third-party sources.
//!
//! See `docs/superpowers/specs/2026-04-20-connectors-design.md`.

pub mod admin;
pub mod arrow_coerce;
pub mod bitbucket;
pub mod catalog;
pub mod catalog_sql;
pub mod metrics;
pub mod gitlab;
pub mod graph_row;
pub mod postgres;
pub mod prometheus;
pub mod registry;
pub mod s3;
pub mod runner;
pub mod scheduler;
pub mod secrets;
pub mod types;
#[cfg(feature = "github")]
pub mod github;

pub use catalog::{CatalogEntry, CatalogField, CatalogResource};
pub use types::{
    ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun, DriveModel, GraphHint,
    TableRows,
};
