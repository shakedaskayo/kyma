//! Postgres data source — introspects a target Postgres instance's schema and
//! emits a property graph (schemas → tables → columns, with FK edges).
//!
//! Authentication: a single `url` field (`postgres://user:pass@host/db`),
//! stored as a secret. Read-only at runtime — only `information_schema` is
//! queried. The emitted graph is registered under the static name `"postgres"`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use std::time::Duration;

use crate::catalog::{CatalogEntry, CatalogField};
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

#[derive(Debug, Deserialize, Default)]
struct PgConfig {
    /// Inline connection URL (backward-compat). One of `url` OR `credential_id`
    /// must be present.
    #[serde(default)]
    url: String,
    /// Reference to a stored credential of kind `url`. Preferred over inline
    /// `url` — a single rotation propagates and the secret is encrypted at
    /// rest in the credentials store.
    #[serde(default)]
    credential_id: Option<uuid::Uuid>,
}

pub struct PgIntrospectDataSource;

#[async_trait]
impl DataSource for PgIntrospectDataSource {
    fn type_id(&self) -> &'static str {
        "postgres"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "postgres".into(),
            label: "Postgres".into(),
            category: "data".into(),
            description: "Introspect a Postgres database into a graph of schemas, tables, \
                          columns, and foreign-key relationships."
                .into(),
            brand: "postgresql".into(),
            auth_mode: "url".into(),
            status: "available".into(),
            drive_model: "periodic".into(),
            default_schedule_ms: 15 * 60_000,
            fields: vec![CatalogField {
                key: "url".into(),
                label: "Connection URL".into(),
                kind: "secret".into(),
                required: true,
                placeholder: Some("postgres://user:pass@host:5432/db".into()),
                help: Some(
                    "Read-only connection string. Only information_schema is queried; stored \
                     encrypted and never shown again."
                        .into(),
                ),
            }],
            resource: None,
            default_target_table: Some("postgres_nodes".into()),
            config_defaults: None,
            graph_name: Some("postgres".into()),
            accepted_credential_kinds: vec!["url".into()],
        }
    }

    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError> {
        let parsed: PgConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        // Exactly one of credential_id / inline url. We can't fully validate
        // the credential at create time (no credential-store access here) —
        // just check that *something* was supplied; runtime resolution does
        // the rest.
        if parsed.credential_id.is_none() {
            if parsed.url.is_empty() {
                return Err(ConfigError(
                    "provide either `credential_id` (preferred) or inline `url`".into(),
                ));
            }
            if !(parsed.url.starts_with("postgres://") || parsed.url.starts_with("postgresql://")) {
                return Err(ConfigError("url must start with postgres:// or postgresql://".into()));
            }
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &serde_json::Value,
        _cursor: Option<&serde_json::Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let parsed: PgConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;

        // Resolve the connection URL: credential first (preferred), then inline.
        let url: String = if let Some(cid) = parsed.credential_id {
            use pensieve_core::credentials::CredentialValue;
            let cred = ctx
                .credentials
                .get(ctx.tenant, cid)
                .await
                .map_err(|e| DataSourceError::Permanent(format!("resolve credential {cid}: {e}")))?;
            match cred.value {
                CredentialValue::Url { connection_string } => connection_string,
                other => {
                    return Err(DataSourceError::Permanent(format!(
                        "credential {cid} has kind={}; postgres data source requires `url`",
                        other.kind()
                    )));
                }
            }
        } else {
            parsed.url.clone()
        };

        // Bounded connect — operator typos shouldn't tie up a runner slot.
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .map_err(|e| DataSourceError::Transient(format!("connect: {e}")))?;

        // The database name from current_database() gives stable ids that survive
        // host/port changes across deploys (the URL host might be a proxy).
        let db: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&pool)
            .await
            .map_err(|e| DataSourceError::Transient(format!("current_database: {e}")))?;

        // ── schemas ────────────────────────────────────────────────────────────
        let schemas: Vec<String> = sqlx::query_scalar(
            "SELECT schema_name FROM information_schema.schemata
             WHERE schema_name NOT IN ('pg_catalog','information_schema','pg_toast')
               AND schema_name NOT LIKE 'pg_temp%'
             ORDER BY schema_name",
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| DataSourceError::Transient(format!("schemas: {e}")))?;

        // ── tables ─────────────────────────────────────────────────────────────
        let table_rows = sqlx::query(
            "SELECT table_schema, table_name, table_type
             FROM information_schema.tables
             WHERE table_schema NOT IN ('pg_catalog','information_schema','pg_toast')
               AND table_schema NOT LIKE 'pg_temp%'
             ORDER BY table_schema, table_name",
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| DataSourceError::Transient(format!("tables: {e}")))?;

        // ── columns ────────────────────────────────────────────────────────────
        let column_rows = sqlx::query(
            "SELECT table_schema, table_name, column_name, data_type, is_nullable, ordinal_position
             FROM information_schema.columns
             WHERE table_schema NOT IN ('pg_catalog','information_schema','pg_toast')
               AND table_schema NOT LIKE 'pg_temp%'
             ORDER BY table_schema, table_name, ordinal_position",
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| DataSourceError::Transient(format!("columns: {e}")))?;

        // ── foreign keys (src.column → dst.column) ────────────────────────────
        let fk_rows = sqlx::query(
            "SELECT
                tc.table_schema AS src_schema, tc.table_name AS src_table, kcu.column_name AS src_column,
                ccu.table_schema AS dst_schema, ccu.table_name AS dst_table, ccu.column_name AS dst_column
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
                  ON  tc.constraint_catalog = kcu.constraint_catalog
                  AND tc.constraint_schema  = kcu.constraint_schema
                  AND tc.constraint_name    = kcu.constraint_name
             JOIN information_schema.constraint_column_usage ccu
                  ON  tc.constraint_catalog = ccu.constraint_catalog
                  AND tc.constraint_schema  = ccu.constraint_schema
                  AND tc.constraint_name    = ccu.constraint_name
             WHERE tc.constraint_type = 'FOREIGN KEY'
               AND tc.table_schema NOT IN ('pg_catalog','information_schema','pg_toast')",
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| DataSourceError::Transient(format!("foreign_keys: {e}")))?;

        // ── transform → nodes/edges ───────────────────────────────────────────
        let mut nodes = Vec::<serde_json::Value>::new();
        let mut edges = Vec::<serde_json::Value>::new();

        let schema_id = |s: &str| format!("pgs::{db}::{s}");
        let table_id = |s: &str, t: &str| format!("pgt::{db}::{s}.{t}");
        let column_id = |s: &str, t: &str, c: &str| format!("pgc::{db}::{s}.{t}.{c}");

        for schema in &schemas {
            nodes.push(json!({
                "id": schema_id(schema),
                "labels": ["PgSchema"],
                "name": schema,
                "database": db,
            }));
        }

        for r in &table_rows {
            let s: String = r.get("table_schema");
            let t: String = r.get("table_name");
            let kind: String = r.get("table_type");
            let id = table_id(&s, &t);
            nodes.push(json!({
                "id": id,
                "labels": ["PgTable"],
                "name": t,
                "schema": s,
                "kind": kind,
                "database": db,
            }));
            edges.push(json!({
                "id": format!("e:has_table:{}::{}", schema_id(&s), id),
                "src": schema_id(&s),
                "dst": id,
                "type": "HAS_TABLE",
            }));
        }

        for r in &column_rows {
            let s: String = r.get("table_schema");
            let t: String = r.get("table_name");
            let c: String = r.get("column_name");
            let dt: String = r.get("data_type");
            let nullable: String = r.get("is_nullable");
            let id = column_id(&s, &t, &c);
            nodes.push(json!({
                "id": id,
                "labels": ["PgColumn"],
                "name": c,
                "data_type": dt,
                "nullable": nullable == "YES",
                "table": t,
                "schema": s,
                "database": db,
            }));
            edges.push(json!({
                "id": format!("e:has_column:{}::{}", table_id(&s, &t), id),
                "src": table_id(&s, &t),
                "dst": id,
                "type": "HAS_COLUMN",
            }));
        }

        for r in &fk_rows {
            let ss: String = r.get("src_schema");
            let st: String = r.get("src_table");
            let sc: String = r.get("src_column");
            let ds: String = r.get("dst_schema");
            let dt: String = r.get("dst_table");
            let dc: String = r.get("dst_column");
            let src = column_id(&ss, &st, &sc);
            let dst = column_id(&ds, &dt, &dc);
            edges.push(json!({
                "id": format!("e:references:{src}::{dst}"),
                "src": src,
                "dst": dst,
                "type": "REFERENCES",
            }));
        }

        pool.close().await;

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: None,
            tables: vec![
                TableRows { table: "postgres_nodes".into(), rows: nodes },
                TableRows { table: "postgres_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "postgres".into(),
                node_table: "postgres_nodes".into(),
                edge_table: "postgres_edges".into(),
            }),
        })
    }
}
