//! Stable extension API for third-party kyma data sources.
//!
//! Customers add a new data source in **four steps**:
//!
//! 1. Create a Rust crate (`my-data-source`) depending on this one.
//! 2. Implement the [`DataSource`] trait. Override [`DataSource::catalog`] so it
//!    self-describes (label, category, brand, auth, config fields, accepted
//!    credential kinds).
//! 3. Register it on `kyma-bin` startup:
//!    ```ignore
//!    conn_reg.register(std::sync::Arc::new(my_data_source::MyDataSource));
//!    ```
//!    The catalog endpoint promotes it from "coming soon" to "available" and
//!    the UI wizard renders the config form straight from `catalog()`.
//! 4. (Optional) Accept a `credential_id` in `config` and resolve it via
//!    `ctx.credentials.get(ctx.tenant, id)` — typed secrets are stored once
//!    in `/v1/credentials` and shared across every data source instance.
//!
//! Nothing in `kyma-datasources`' internals — the runner, the scheduler, the
//! Postgres-backed task queue — is re-exported here. The customer surface
//! stays small (and so the burden of backward-compat stays bounded).
#![forbid(unsafe_code)]

// ── Trait + lifecycle ────────────────────────────────────────────────────────
pub use kyma_datasources::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, DriveModel, GraphHint,
    TableRows,
};

// ── Self-describing catalog ──────────────────────────────────────────────────
pub use kyma_datasources::catalog::{CatalogEntry, CatalogField, CatalogResource};

// ── Graph-row helpers ────────────────────────────────────────────────────────
pub use kyma_datasources::graph_row::{normalize_edge, normalize_node};

// ── Credentials (system-wide; resolved via DataSourceCtx::credentials) ────────
pub use kyma_core::credentials::{Credential, CredentialStore, CredentialSummary, CredentialValue};
pub use kyma_core::tenant::TenantId;
