//! Microsoft Fabric connector — **metadata sync only** (federated source).
//!
//! Unlike row-producing connectors, this one never ingests data. Each tick it
//! introspects a Fabric SQL analytics endpoint (`INFORMATION_SCHEMA` over TDS)
//! and reconciles the target kyma database to mirror the remote tables as
//! **federated** (live-proxied) tables: schema in the catalog, rows fetched at
//! query time by `kyma-federation`. See `kyma_core::catalog::FederatedTableSpec`.
//!
//! Reconciliation rules per tick:
//! - new remote table          → create federated table;
//! - remote schema/spec drift  → drop + recreate (federated tables hold no
//!   extents, so this is cheap and atomic enough);
//! - remote table disappeared  → drop the federated table — but only if its
//!   spec points at *this* endpoint+database, so two msfabric connectors can
//!   share a target database;
//! - name collides with a non-federated table → skip with a warning, never
//!   clobber ingested data.

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::{info, warn};

use serde_json::json;

use crate::catalog::{CatalogEntry, CatalogField};
use crate::types::{
    ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun, GraphHint, TableRows,
};
use kyma_core::catalog::{FederatedTableSpec, TableConfig};
use kyma_core::credentials::CredentialValue;
use kyma_federation::msfabric::{introspect, RemoteTableMeta, PLATFORM_ID};

/// Wall-clock budget for one introspection pass (connect + walk
/// INFORMATION_SCHEMA). Generous: metadata sync is a background tick.
const INTROSPECT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
struct MsFabricConfig {
    /// SQL analytics endpoint host, e.g. `<id>.datawarehouse.fabric.microsoft.com`.
    endpoint: String,
    /// Remote database on the endpoint — the Lakehouse/Warehouse item name.
    database: String,
    /// Stored credential of kind `service_principal`.
    credential_id: uuid::Uuid,
    /// Opt this source out of the cross-database `x-database: *` union.
    #[serde(default)]
    exclude_from_wildcard: bool,
}

pub struct MsFabricConnector;

#[async_trait]
impl Connector for MsFabricConnector {
    fn type_id(&self) -> &'static str {
        "msfabric"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "msfabric".into(),
            label: "Microsoft Fabric".into(),
            category: "data".into(),
            description: "Query Lakehouse and Warehouse tables live over the Fabric SQL \
                          analytics endpoint — schemas are cataloged, data stays in Fabric \
                          and is fetched at query time (no ingestion)."
                .into(),
            brand: "microsoft".into(),
            auth_mode: "service_principal".into(),
            status: "available".into(),
            // Metadata-only sync; 30 min keeps schemas fresh without hammering
            // the endpoint.
            default_schedule_ms: 30 * 60_000,
            fields: vec![
                CatalogField::text(
                    "endpoint",
                    "SQL analytics endpoint",
                    "xxxxxxxx.datawarehouse.fabric.microsoft.com",
                ),
                CatalogField::text("database", "Lakehouse / Warehouse name", "my_lakehouse"),
                CatalogField::checkbox(
                    "exclude_from_wildcard",
                    "Exclude from \"All databases\" queries",
                    "Keep this source out of x-database:* fan-outs; explicit queries still reach it.",
                ),
            ],
            resource: None,
            default_target_table: None,
            config_defaults: None,
            graph_name: None,
            accepted_credential_kinds: vec!["service_principal".into()],
        }
    }

    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError> {
        let parsed: MsFabricConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        if parsed.endpoint.trim().is_empty() {
            return Err(ConfigError("`endpoint` must not be empty".into()));
        }
        if parsed.endpoint.contains("://") {
            return Err(ConfigError(
                "`endpoint` is a bare host (no scheme), e.g. xxxx.datawarehouse.fabric.microsoft.com".into(),
            ));
        }
        if parsed.database.trim().is_empty() {
            return Err(ConfigError("`database` must not be empty".into()));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &ConnectorCtx,
        cfg: &serde_json::Value,
        _cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        let parsed: MsFabricConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| ConnectorError::Permanent(format!("bad config: {e}")))?;
        let catalog = ctx.catalog.as_ref().ok_or_else(|| {
            ConnectorError::Config(
                "msfabric requires catalog access; this runner wasn't wired with one".into(),
            )
        })?;

        let cred = ctx
            .credentials
            .get(ctx.tenant, parsed.credential_id)
            .await
            .map_err(|e| {
                ConnectorError::Permanent(format!(
                    "resolve credential {}: {e}",
                    parsed.credential_id
                ))
            })?;
        if !matches!(cred.value, CredentialValue::ServicePrincipal { .. }) {
            return Err(ConnectorError::Config(format!(
                "credential {} has kind={}; msfabric requires `service_principal`",
                parsed.credential_id,
                cred.value.kind()
            )));
        }

        // 1. Live introspection of the remote endpoint.
        let remote = introspect(
            &parsed.endpoint,
            &parsed.database,
            &cred.value,
            INTROSPECT_TIMEOUT,
        )
        .await
        .map_err(|e| ConnectorError::Transient(format!("fabric introspection: {e}")))?;

        // 2. Ensure the target kyma database exists.
        let db_name = ctx.target_database.clone();
        let db_id = match catalog
            .lookup_database_in_tenant(ctx.tenant, &db_name)
            .await
            .map_err(|e| ConnectorError::Transient(format!("lookup database: {e}")))?
        {
            Some(id) => id,
            None => catalog
                .create_database_in_tenant(ctx.tenant, &db_name)
                .await
                .map_err(|e| ConnectorError::Transient(format!("create database: {e}")))?,
        };

        // 3. Desired state: kyma table name → (remote meta, federated spec).
        let desired: BTreeMap<String, (&RemoteTableMeta, FederatedTableSpec)> = remote
            .iter()
            .map(|t| {
                let name = kyma_table_name(t);
                let spec = FederatedTableSpec {
                    platform: PLATFORM_ID.into(),
                    endpoint: parsed.endpoint.clone(),
                    remote_database: parsed.database.clone(),
                    remote_schema: t.remote_schema.clone(),
                    remote_table: t.remote_table.clone(),
                    credential_id: parsed.credential_id,
                    exclude_from_wildcard: parsed.exclude_from_wildcard,
                };
                (name, (t, spec))
            })
            .collect();

        // 4. Reconcile against what's currently cataloged.
        let existing = catalog
            .list_tables_in_database_in_tenant(ctx.tenant, &db_name)
            .await
            .map_err(|e| ConnectorError::Transient(format!("list tables: {e}")))?;

        let mut created = 0usize;
        let mut recreated = 0usize;
        let mut dropped = 0usize;

        for table in &existing {
            let Some(fed) = table.config.federated.as_ref() else {
                if desired.contains_key(&table.name) {
                    warn!(
                        table = %table.name,
                        database = %db_name,
                        "msfabric: remote table name collides with a non-federated kyma table; skipping"
                    );
                }
                continue;
            };
            // Only manage tables that point at this connector's source.
            if fed.platform != PLATFORM_ID
                || fed.endpoint != parsed.endpoint
                || fed.remote_database != parsed.database
            {
                continue;
            }
            match desired.get(&table.name) {
                Some((meta, spec)) if *fed == *spec && *table.schema == *meta.schema => {
                    // Unchanged — leave in place.
                }
                Some(_) | None => {
                    catalog
                        .drop_table_in_tenant(ctx.tenant, &db_name, &table.name)
                        .await
                        .map_err(|e| {
                            ConnectorError::Transient(format!("drop {}: {e}", table.name))
                        })?;
                    if desired.contains_key(&table.name) {
                        recreated += 1;
                    } else {
                        dropped += 1;
                    }
                }
            }
        }

        // Re-list after drops so "create what's missing" sees fresh state.
        let existing_names: std::collections::BTreeSet<String> = catalog
            .list_tables_in_database_in_tenant(ctx.tenant, &db_name)
            .await
            .map_err(|e| ConnectorError::Transient(format!("list tables: {e}")))?
            .into_iter()
            .map(|t| t.name)
            .collect();

        for (name, (meta, spec)) in &desired {
            if existing_names.contains(name) {
                continue;
            }
            let config = TableConfig {
                federated: Some(spec.clone()),
                ..TableConfig::default()
            };
            catalog
                .create_table_in_tenant(ctx.tenant, db_id, name, meta.schema.clone(), config)
                .await
                .map_err(|e| ConnectorError::Transient(format!("create {name}: {e}")))?;
            created += 1;
        }

        info!(
            connector_id = %ctx.connector_id,
            database = %db_name,
            remote_tables = desired.len(),
            created, recreated, dropped,
            "msfabric metadata sync complete"
        );

        // Context-graph entities: endpoint → database item → tables. These DO
        // ingest (normal rows into <db>.msfabric_nodes/_edges) — the graph is
        // kyma metadata about the remote source, not remote data.
        let (nodes, edges) = graph_rows(&parsed, &desired);

        Ok(ConnectorRun {
            rows: Vec::new(),
            new_cursor: None,
            tables: vec![
                TableRows { table: "msfabric_nodes".into(), rows: nodes },
                TableRows { table: "msfabric_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "msfabric".into(),
                node_table: "msfabric_nodes".into(),
                edge_table: "msfabric_edges".into(),
            }),
        })
    }
}

/// Build the `endpoint → database item → table` entity rows for the context
/// graph, in the canonical normalized node/edge shape.
fn graph_rows(
    cfg: &MsFabricConfig,
    desired: &BTreeMap<String, (&RemoteTableMeta, FederatedTableSpec)>,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let endpoint_id = format!("msfabric:{}", cfg.endpoint);
    let db_id = format!("{endpoint_id}/{}", cfg.database);

    let mut nodes = vec![
        json!({
            "id": endpoint_id,
            "labels": ["FabricEndpoint"],
            "name": cfg.endpoint,
            "vendor": "msfabric",
        }),
        json!({
            "id": db_id,
            "labels": ["FabricItem"],
            "name": cfg.database,
            "vendor": "msfabric",
            "endpoint": cfg.endpoint,
        }),
    ];
    let mut edges = vec![json!({
        "id": format!("e:contains:{endpoint_id}::{db_id}"),
        "src": endpoint_id,
        "dst": db_id,
        "type": "CONTAINS",
    })];

    for (kyma_name, (meta, _)) in desired {
        let table_id = format!("{db_id}/{}.{}", meta.remote_schema, meta.remote_table);
        nodes.push(json!({
            "id": table_id,
            "labels": ["FabricTable"],
            "name": kyma_name,
            "vendor": "msfabric",
            "remote_schema": meta.remote_schema,
            "remote_table": meta.remote_table,
            "columns": meta.schema.fields().len(),
        }));
        edges.push(json!({
            "id": format!("e:contains:{db_id}::{table_id}"),
            "src": db_id,
            "dst": table_id,
            "type": "CONTAINS",
        }));
    }

    let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
    let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();
    (nodes, edges)
}

/// Kyma-side table name for a remote table: bare name for the default `dbo`
/// schema, `schema__table` otherwise (lakehouse multi-schema support).
fn kyma_table_name(t: &RemoteTableMeta) -> String {
    if t.remote_schema.eq_ignore_ascii_case("dbo") {
        t.remote_table.clone()
    } else {
        format!("{}__{}", t.remote_schema, t.remote_table)
    }
}
