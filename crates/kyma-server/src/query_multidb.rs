//! Cross-database query support for `POST /v1/query`.
//!
//! When a request is *not* scoped to a concrete database — the `x-database`
//! header is `*` — an unqualified table reference (`from t` / `FROM t`)
//! resolves to a UNION-BY-NAME of that table across every accessible
//! non-internal database, with a synthetic `__database` provenance column.
//! Database-qualified references (`db.table`, SQL only) still target a single
//! database, because each database is also registered as its own DataFusion
//! schema. A concrete `x-database: <db>` keeps the original single-database
//! path untouched.
//!
//! Mechanism:
//!   1. Register one `MemorySchemaProvider` per database under the default
//!      catalog, each holding that database's `KymaTable`s. This makes
//!      `db.table` resolve natively via DataFusion's `TableReference`.
//!   2. For every distinct bare table name, register a `public` view that is a
//!      `UNION ALL` (reconciled by name) across the databases that have it,
//!      each branch tagged with its `__database`. Unqualified names bind here.
//!
//! The union/null-fill string construction mirrors what `kyma_kql`'s `union`
//! operator already emits (`crates/kyma-kql/src/parser.rs`), reusing
//! DataFusion's own UNION-ALL planner + type coercion for the heavy lifting.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_schema::DataType;
use datafusion::catalog_common::MemorySchemaProvider;
use datafusion::prelude::SessionContext;
use datafusion::sql::TableReference;
use kyma_core::catalog::{Catalog, TableRef};
use kyma_core::segment_format::SegmentFormat;
use kyma_core::tenant::TenantId;
use kyma_core::types::NodeId;
use kyma_exec::KymaTable;

/// Databases holding kyma-internal state (agent memory). Excluded from the
/// unqualified cross-database union — mirrors `discover::scope::INTERNAL_DATABASES`
/// so "all databases" means the user's data, not the engine's.
pub const INTERNAL_DATABASES: &[&str] = &[kyma_memory::DEFAULT_DATABASE];

/// Synthetic provenance column added to unified results so each row carries the
/// database it originated from.
pub const PROVENANCE_COLUMN: &str = "__database";

/// One concrete `(database, table)` pair to include in the cross-database context.
pub struct DbTable {
    pub db: String,
    pub table: TableRef,
}

/// A column of a per-database table, for union-view construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnionField {
    pub name: String,
    pub data_type: DataType,
}

/// True when `x-database` selects the cross-database union. Only the explicit
/// `*` triggers it: an absent/empty header keeps the legacy default-database
/// behavior so existing API callers that omit the header are unaffected.
pub fn is_all_databases(header: Option<&str>) -> bool {
    matches!(header.map(str::trim), Some("*"))
}

/// Walk the catalog for every `(db, table)` the caller may query, honoring
/// `allowed_databases` (None = unrestricted) and excluding internal databases.
/// Per-database list errors are swallowed (treated as RBAC drops), mirroring
/// `discover::scope::resolve`.
pub async fn resolve_all_db_tables(
    catalog: &Arc<dyn Catalog>,
    tenant: TenantId,
    allowed_databases: Option<&[String]>,
) -> Result<Vec<DbTable>, String> {
    let dbs = catalog
        .list_databases_in_tenant(tenant)
        .await
        .map_err(|e| e.to_string())?;

    let mut out: Vec<DbTable> = Vec::new();
    for db in dbs {
        if INTERNAL_DATABASES.contains(&db.as_str()) {
            continue;
        }
        if let Some(allow) = allowed_databases {
            if !allow.iter().any(|a| a == &db) {
                continue;
            }
        }
        match catalog.list_tables_in_database_in_tenant(tenant, &db).await {
            Ok(tables) => {
                for t in tables {
                    out.push(DbTable {
                        db: db.clone(),
                        table: t,
                    });
                }
            }
            Err(_) => continue, // RBAC drop / per-DB error: skip silently
        }
    }
    Ok(out)
}

/// Build the KQL [`kyma_kql::SchemaMap`] for the cross-database context: each
/// distinct bare table name maps to its merged column superset (with the
/// `__database` provenance column prepended), matching the registered union view.
pub fn build_multidb_schema_map(db_tables: &[DbTable]) -> kyma_kql::SchemaMap {
    group_by_table(db_tables)
        .into_iter()
        .map(|(name, per_db)| {
            let fields: Vec<(String, Vec<UnionField>)> = per_db
                .iter()
                .map(|(db, table)| (db.clone(), fields_of(table)))
                .collect();
            let mut cols = vec![PROVENANCE_COLUMN.to_string()];
            cols.extend(superset_columns(&fields).into_iter().map(|f| f.name));
            (name, cols)
        })
        .collect()
}

/// Register the cross-database context into `ctx`: one schema per database (so
/// `db.table` resolves), then one `public` union view per distinct bare name.
pub async fn register_multidb_context(
    ctx: &SessionContext,
    db_tables: &[DbTable],
    catalog: &Arc<dyn Catalog>,
    format: &Arc<dyn SegmentFormat>,
    node_id: Option<NodeId>,
) -> Result<(), String> {
    let cat = ctx
        .catalog("datafusion")
        .ok_or_else(|| "default DataFusion catalog missing".to_string())?;

    // 1. Register a schema per database (idempotent: register only once).
    let mut seen_dbs: BTreeMap<&str, ()> = BTreeMap::new();
    for dt in db_tables {
        if seen_dbs.insert(dt.db.as_str(), ()).is_none() {
            cat.register_schema(&dt.db, Arc::new(MemorySchemaProvider::new()))
                .map_err(|e| format!("register schema {}: {e}", dt.db))?;
        }
    }

    // 2. Register each KymaTable under its `db.table` schema-qualified name.
    for dt in db_tables {
        let kt: Arc<KymaTable> = match node_id {
            Some(nid) => Arc::new(KymaTable::with_node_id(
                dt.table.clone(),
                catalog.clone(),
                format.clone(),
                nid,
                dt.db.clone(),
            )),
            None => Arc::new(KymaTable::new(
                dt.table.clone(),
                catalog.clone(),
                format.clone(),
            )),
        };
        let reference = TableReference::partial(dt.db.clone(), dt.table.name.clone());
        ctx.register_table(reference, kt)
            .map_err(|e| format!("register {}.{}: {e}", dt.db, dt.table.name))?;
    }

    // 3. One union view per distinct bare table name, in the default schema.
    for (name, per_db) in group_by_table(db_tables) {
        let fields: Vec<(String, Vec<UnionField>)> = per_db
            .iter()
            .map(|(db, table)| (db.clone(), fields_of(table)))
            .collect();
        let sql = build_union_view_sql(&name, &fields)?;
        ctx.sql(&sql)
            .await
            .map_err(|e| format!("create union view {name}: {e}"))?;
    }
    Ok(())
}

/// Build the `CREATE VIEW "<table>" AS …` SQL that unions `<table>` across all
/// databases that have it, reconciled by name with a `__database` provenance
/// column. Pure + unit-tested.
///
/// Reconciliation per superset column:
///   - all present types equal      → reference the column as-is;
///   - all present types numeric     → `CAST(col AS DOUBLE)`;
///   - otherwise (incompatible)      → `CAST(col AS VARCHAR)`.
/// Missing columns are emitted as a bare `NULL` (DataFusion's UNION coercion
/// types it to the other branches).
pub fn build_union_view_sql(
    table: &str,
    per_db: &[(String, Vec<UnionField>)],
) -> Result<String, String> {
    if per_db.is_empty() {
        return Err(format!("no databases provide table {table}"));
    }

    // Reject a real column colliding with the provenance column.
    for (db, fields) in per_db {
        if fields.iter().any(|f| f.name == PROVENANCE_COLUMN) {
            return Err(format!(
                "table {db}.{table} already has a `{PROVENANCE_COLUMN}` column, which collides with the cross-database provenance column",
            ));
        }
    }

    let superset = superset_columns(per_db);

    // Reconciled comparison/cast decision per column.
    let plans: Vec<(String, ColPlan)> = superset
        .iter()
        .map(|f| (f.name.clone(), reconcile(&f.name, per_db)))
        .collect();

    let mut branches: Vec<String> = Vec::with_capacity(per_db.len());
    for (db, fields) in per_db {
        let mut selects: Vec<String> = Vec::with_capacity(plans.len() + 1);
        selects.push(format!(
            "{} AS {}",
            sql_str_lit(db),
            quote_ident(PROVENANCE_COLUMN)
        ));
        for (col, plan) in &plans {
            let present = fields.iter().any(|f| &f.name == col);
            let qcol = quote_ident(col);
            let expr = if !present {
                format!("NULL AS {qcol}")
            } else {
                match plan {
                    ColPlan::Keep => qcol.clone(),
                    ColPlan::CastDouble => format!("CAST({qcol} AS DOUBLE) AS {qcol}"),
                    ColPlan::CastVarchar => format!("CAST({qcol} AS VARCHAR) AS {qcol}"),
                }
            };
            selects.push(expr);
        }
        branches.push(format!(
            "SELECT {} FROM {}.{}",
            selects.join(", "),
            quote_ident(db),
            quote_ident(table),
        ));
    }

    Ok(format!(
        "CREATE VIEW {} AS {}",
        quote_ident(table),
        branches.join(" UNION ALL ")
    ))
}

// ── internals ───────────────────────────────────────────────────────────────

enum ColPlan {
    Keep,
    CastDouble,
    CastVarchar,
}

/// Decide how a superset column is projected across branches that have it.
fn reconcile(col: &str, per_db: &[(String, Vec<UnionField>)]) -> ColPlan {
    let types: Vec<&DataType> = per_db
        .iter()
        .filter_map(|(_, fields)| fields.iter().find(|f| f.name == col).map(|f| &f.data_type))
        .collect();
    if types.is_empty() {
        return ColPlan::Keep;
    }
    let first = types[0];
    if types.iter().all(|t| *t == first) {
        return ColPlan::Keep;
    }
    if types.iter().all(|t| is_numeric(t)) {
        return ColPlan::CastDouble;
    }
    ColPlan::CastVarchar
}

fn is_numeric(t: &DataType) -> bool {
    matches!(
        t,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Decimal128(_, _)
            | DataType::Decimal256(_, _)
    )
}

/// Group `(db, table)` pairs by bare table name, databases sorted for
/// deterministic view schemas.
fn group_by_table(db_tables: &[DbTable]) -> Vec<(String, Vec<(String, TableRef)>)> {
    let mut map: BTreeMap<String, Vec<(String, TableRef)>> = BTreeMap::new();
    for dt in db_tables {
        map.entry(dt.table.name.clone())
            .or_default()
            .push((dt.db.clone(), dt.table.clone()));
    }
    for v in map.values_mut() {
        v.sort_by(|a, b| a.0.cmp(&b.0));
    }
    map.into_iter().collect()
}

fn fields_of(table: &TableRef) -> Vec<UnionField> {
    table
        .schema
        .fields()
        .iter()
        .map(|f| UnionField {
            name: f.name().clone(),
            data_type: f.data_type().clone(),
        })
        .collect()
}

/// First-seen column superset across the per-database field lists.
fn superset_columns(per_db: &[(String, Vec<UnionField>)]) -> Vec<UnionField> {
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    let mut out: Vec<UnionField> = Vec::new();
    for (_, fields) in per_db {
        for f in fields {
            if seen.insert(f.name.clone(), ()).is_none() {
                out.push(f.clone());
            }
        }
    }
    out
}

/// Double-quote a SQL identifier, escaping embedded quotes.
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Single-quote a SQL string literal, escaping embedded quotes.
fn sql_str_lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, dt: DataType) -> UnionField {
        UnionField {
            name: name.to_string(),
            data_type: dt,
        }
    }

    #[test]
    fn single_db_view_has_provenance_and_columns() {
        let per_db = vec![(
            "db1".to_string(),
            vec![f("ts", DataType::Utf8), f("msg", DataType::Utf8)],
        )];
        let sql = build_union_view_sql("events", &per_db).unwrap();
        assert_eq!(
            sql,
            "CREATE VIEW \"events\" AS SELECT 'db1' AS \"__database\", \"ts\", \"msg\" FROM \"db1\".\"events\""
        );
    }

    #[test]
    fn two_dbs_disjoint_columns_null_fill_by_name() {
        let per_db = vec![
            (
                "db1".to_string(),
                vec![f("ts", DataType::Utf8), f("msg", DataType::Utf8)],
            ),
            (
                "db2".to_string(),
                vec![f("ts", DataType::Utf8), f("code", DataType::Int64)],
            ),
        ];
        let sql = build_union_view_sql("events", &per_db).unwrap();
        // Superset (first-seen): ts, msg, code. db1 null-fills code; db2 null-fills msg.
        assert_eq!(
            sql,
            "CREATE VIEW \"events\" AS \
SELECT 'db1' AS \"__database\", \"ts\", \"msg\", NULL AS \"code\" FROM \"db1\".\"events\" \
UNION ALL \
SELECT 'db2' AS \"__database\", \"ts\", NULL AS \"msg\", \"code\" FROM \"db2\".\"events\""
        );
    }

    #[test]
    fn incompatible_types_reconcile_to_varchar() {
        let per_db = vec![
            ("db1".to_string(), vec![f("v", DataType::Int64)]),
            ("db2".to_string(), vec![f("v", DataType::Utf8)]),
        ];
        let sql = build_union_view_sql("t", &per_db).unwrap();
        assert!(sql.contains("CAST(\"v\" AS VARCHAR) AS \"v\""));
        assert_eq!(sql.matches("CAST(\"v\" AS VARCHAR)").count(), 2);
    }

    #[test]
    fn numeric_widening_casts_to_double() {
        let per_db = vec![
            ("db1".to_string(), vec![f("n", DataType::Int64)]),
            ("db2".to_string(), vec![f("n", DataType::Float64)]),
        ];
        let sql = build_union_view_sql("t", &per_db).unwrap();
        assert!(sql.contains("CAST(\"n\" AS DOUBLE) AS \"n\""));
    }

    #[test]
    fn provenance_collision_is_rejected() {
        let per_db = vec![(
            "db1".to_string(),
            vec![f("__database", DataType::Utf8), f("ts", DataType::Utf8)],
        )];
        let err = build_union_view_sql("events", &per_db).unwrap_err();
        assert!(err.contains("__database"));
    }

    #[test]
    fn schema_map_prepends_provenance() {
        // group_by_table + superset are exercised via the schema-map builder.
        use kyma_core::catalog::{TableConfig, TableRef};
        use kyma_core::types::{DatabaseId, SchemaSnapshotId, SnapshotId, TableId};
        use arrow_schema::{Field, Schema as ArrowSchema};

        let mk = |name: &str, cols: &[&str]| TableRef {
            id: TableId::new(),
            database_id: DatabaseId::new(),
            name: name.to_string(),
            current_snapshot_id: SnapshotId::new(),
            schema_snapshot_id: SchemaSnapshotId::new(),
            schema: Arc::new(ArrowSchema::new(
                cols.iter()
                    .map(|c| Field::new(*c, DataType::Utf8, true))
                    .collect::<Vec<_>>(),
            )),
            config: TableConfig::default(),
        };
        let db_tables = vec![
            DbTable { db: "db1".into(), table: mk("events", &["ts", "msg"]) },
            DbTable { db: "db2".into(), table: mk("events", &["ts", "code"]) },
        ];
        let map = build_multidb_schema_map(&db_tables);
        assert_eq!(
            map.get("events").unwrap(),
            &vec![
                "__database".to_string(),
                "ts".to_string(),
                "msg".to_string(),
                "code".to_string()
            ]
        );
    }
}
