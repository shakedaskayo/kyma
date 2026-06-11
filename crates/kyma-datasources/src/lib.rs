#![forbid(unsafe_code)]
//! Ingestion data sources: pull-side integrations to third-party sources.

pub mod admin;
pub mod artifacts;
pub mod arrow_coerce;
pub mod bitbucket;
pub mod catalog;
pub mod catalog_sql;
pub mod confluence;
pub mod metrics;
pub mod gitlab;
pub mod gmail;
pub mod msfabric;
pub mod googledrive;
pub mod graph_row;
pub mod jira;
pub mod notion;
pub mod oauth;
pub mod oauth_http;
pub mod postgres;
pub mod prometheus;
pub mod slack;
pub mod registry;
pub mod s3;
pub mod runner;
pub mod scheduler;
pub mod secrets;
pub mod types;
pub mod watchers;
// The GitHub data source compiles unconditionally so it always appears in the
// catalog and ingests repo metadata. The deep tree-sitter code graph (the heavy
// C grammar crates) stays behind the `github` feature — see `github::transform`.
pub mod github;

pub use catalog::{CatalogEntry, CatalogField, CatalogResource};
pub use types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, DriveModel, GraphHint,
    TableRows,
};
