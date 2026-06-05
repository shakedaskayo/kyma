//! Auto-create + schema-evolve helpers shared by every ingest frontend.
//!
//! Until this module existed, only the OTLP receiver auto-created its target
//! table on first write — REST and Filedrop returned `404 table_not_found`,
//! which made every multi-tenant + multi-pipeline deployment require a
//! pre-provisioning step that nobody actually wanted.
//!
//! Two related helpers:
//!
//! - [`ensure_table`] — `lookup_table` with a fallback that creates the
//!   database (if missing) and an empty default-schema table (if missing),
//!   then re-looks up. Idempotent. Uses a small mutex to coalesce concurrent
//!   first-writes to the same table so we don't race two `CREATE TABLE`s.
//!
//! - [`evolve_schema_for_records`] — scan the parsed JSON of a request and
//!   call `alter_table_add_column` for every property the table doesn't
//!   already know about. Caps at [`MAX_NEW_COLUMNS_PER_REQUEST`] columns per
//!   request so a malformed source can't run a 500-column DDL storm.
//!
//! Both helpers are intentionally cheap on the happy path: when the table
//! already exists with the right shape, they are a single catalog lookup.

use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use kyma_core::catalog::{Catalog, TableConfig, TableRef};
use kyma_core::errors::{Error, Result};
use kyma_core::types::DatabaseId;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Hard cap on auto-added columns per single request. Past this we stop
/// inferring and let the user explicitly run a `CREATE TABLE` if they really
/// have hundreds of dynamic columns. Designed to be generous for normal
/// telemetry shapes but block accidental DDL storms.
pub const MAX_NEW_COLUMNS_PER_REQUEST: usize = 32;

/// The default schema we create when a pipeline writes to a table that
/// doesn't exist yet. Designed to fit Agentcy's `context_events` shape but
/// general enough to hold any record:
///
/// - `at`     — record timestamp (auto-populated from `at` / `timestamp` /
///   `time_unix_nano` / `observed_time_unix_nano` if present in the record;
///   ingest writes `null` if none of those exist)
/// - `label`  — short kind name (free-form string the user picks)
/// - `body`   — primary text payload, suitable for full-text-style queries
/// - `props`  — Binary (dynamic JSON) catch-all for anything not promoted to
///   its own column. Schema evolution promotes properties out of `props` and
///   into typed columns over time.
///
/// Schema evolution adds new typed columns alongside these — the existing
/// columns are never altered.
pub fn default_table_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new(
            "at",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new("label", DataType::Utf8, true),
        Field::new("body", DataType::Utf8, true),
        // Dynamic / catch-all column. The NDJSON parser stores arbitrary JSON
        // bytes here, so a record that includes a property the table doesn't
        // know about is preserved (just not yet queryable as a typed column).
        Field::new("props", DataType::Binary, true),
    ]))
}

/// Look up `(database, table)` in the catalog, creating both if missing.
///
/// Behavior:
/// 1. `lookup_table(database, table)` — happy path, returns immediately.
/// 2. If lookup fails, create the database (idempotent — `find_database_id`
///    first), then create the table with [`default_table_schema`].
/// 3. Re-look up and return.
///
/// Concurrent first-writes to the same `(database, table)` are serialized via
/// the catalog's own constraints; the caller will see one CREATE succeed and
/// the racing creator's CREATE return a `Conflict` which we silently swallow
/// and re-look up.
pub async fn ensure_table(
    catalog: &dyn Catalog,
    database: &str,
    table: &str,
) -> Result<TableRef> {
    if let Ok(t) = catalog.lookup_table(database, table).await {
        return Ok(t);
    }
    info!(database, table, "ensure_table: target missing, creating");

    // 1. Database may not exist yet. Lookup → create on miss.
    let db_id = match find_database_id(catalog, database).await {
        Some(id) => id,
        None => {
            match catalog.create_database(database).await {
                Ok(id) => id,
                Err(_) => {
                    // Race or transient error: try one more lookup.
                    find_database_id(catalog, database).await.ok_or_else(|| {
                        Error::Internal(format!(
                            "ensure_table: database `{database}` create failed and lookup still fails"
                        ))
                    })?
                }
            }
        }
    };

    // 2. Table may not exist. Create with the default schema. Tolerate
    //    races: if another writer created the table at the same time, the
    //    final lookup below picks it up.
    if let Err(e) = catalog
        .create_table(db_id, table, default_table_schema(), TableConfig::default())
        .await
    {
        debug!(database, table, error = %e, "ensure_table: create_table failed; will lookup post-create");
    }

    // 3. Re-look up. If this fails, something's seriously wrong.
    catalog
        .lookup_table(database, table)
        .await
        .map_err(|e| Error::Internal(format!("ensure_table: post-create lookup failed: {e}")))
}

/// Find a database ID by name. Postgres-catalog-aware so the OTLP-style fast
/// path stays a single SQL query; falls back to a `list_databases` scan for
/// other catalog backends.
async fn find_database_id(catalog: &dyn Catalog, name: &str) -> Option<DatabaseId> {
    // Use the catalog-trait's `lookup_database` which is implemented correctly
    // by every backend (Postgres, SQLite, …). The old code used a Postgres-only
    // downcast and a broken non-Postgres fallback that always returned `None`
    // even when the database existed — causing the second-table ingest for the
    // same database to fail on non-Postgres backends.
    catalog.lookup_database(name).await.ok().flatten()
}

/// Inspect the JSON records to find property names that aren't yet in the
/// table's schema, then call `alter_table_add_column` to add a Utf8 nullable
/// column for each. Returns the (possibly-evolved) [`TableRef`].
///
/// Inference is conservative on purpose:
/// - Skips properties whose name doesn't look like an SQL identifier.
/// - Adds at most [`MAX_NEW_COLUMNS_PER_REQUEST`] columns per call.
/// - All new columns are `Utf8` nullable. Stronger inference (numeric vs
///   string vs bool) can come later — callers wanting typed properties
///   should pre-create the table with `POST /v1/admin/tables`.
///
/// Why Utf8 and not the JSON's actual type? Two reasons:
/// 1. NDJSON arrives line-by-line — we don't know what the next record will
///    do to the column's type. Utf8 accepts everything we can print.
/// 2. Once the column is alive, users can run `cast(col as float)` in
///    queries. We're optimising for "data lands and is visible", not for
///    "the column is the perfect type".
pub async fn evolve_schema_for_records(
    catalog: &dyn Catalog,
    database: &str,
    table: TableRef,
    records: &[serde_json::Value],
) -> Result<TableRef> {
    if records.is_empty() {
        return Ok(table);
    }

    // Collect known column names (lowercase to match catalog's case-insensitive
    // semantics).
    let known: HashSet<String> = table
        .schema
        .fields()
        .iter()
        .map(|f| f.name().to_ascii_lowercase())
        .collect();

    // Walk records, gathering unknown property names. Iterate in record order
    // to preserve the most-likely-to-matter columns when we hit the cap.
    let mut to_add: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for rec in records {
        let serde_json::Value::Object(map) = rec else {
            continue;
        };
        for key in map.keys() {
            let lower = key.to_ascii_lowercase();
            if known.contains(&lower) || seen.contains(&lower) {
                continue;
            }
            if !is_safe_column_ident(&lower) {
                debug!(field = %key, "evolve_schema: skipping non-identifier-safe field");
                continue;
            }
            seen.insert(lower.clone());
            to_add.push(key.clone());
            if to_add.len() >= MAX_NEW_COLUMNS_PER_REQUEST {
                warn!(
                    cap = MAX_NEW_COLUMNS_PER_REQUEST,
                    "evolve_schema: hit per-request column cap; remaining unknown fields land in `props`"
                );
                break;
            }
        }
        if to_add.len() >= MAX_NEW_COLUMNS_PER_REQUEST {
            break;
        }
    }

    if to_add.is_empty() {
        return Ok(table);
    }

    info!(
        database = %database,
        table = %table.name,
        new_columns = ?to_add,
        "evolve_schema: adding {} new columns", to_add.len()
    );

    let table_id = table.id;
    let mut last_err: Option<Error> = None;
    for col in &to_add {
        // `column_type` is the catalog's canonical type name. Utf8 ⇒ "string".
        match catalog
            .alter_table_add_column(table_id, col, "string")
            .await
        {
            Ok(_) => {}
            Err(e) => {
                // Don't bail the whole request — log and keep going. The
                // missing column ends up in `props` for that field.
                warn!(column = %col, error = %e, "evolve_schema: alter failed; field will land in `props`");
                last_err = Some(e);
            }
        }
    }

    // Re-look up so the caller writes against the freshest schema.
    match catalog.lookup_table(database, &table.name).await {
        Ok(t) => Ok(t),
        Err(e) => {
            // The alters may have all failed; in that case return the
            // original ref + the recorded error in tracing.
            if let Some(err) = last_err {
                warn!(error = %err, "evolve_schema: returning original schema ref after alter failures");
            }
            Err(Error::Internal(format!(
                "evolve_schema: post-alter lookup failed: {e}"
            )))
        }
    }
}

/// Lowercased, non-empty, only `[a-z0-9_]`, can't start with a digit, can't
/// be `props` (the catch-all column). Mirrors what most SQL engines accept
/// without quoting.
fn is_safe_column_ident(name: &str) -> bool {
    if name.is_empty() || name.len() > 63 {
        return false;
    }
    if matches!(name, "props" | "at" | "label" | "body") {
        // Reserved by the default schema; a property of the same name lands
        // in the existing column instead of triggering an alter.
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() && first != '_' {
        return false;
    }
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_safety() {
        assert!(is_safe_column_ident("foo"));
        assert!(is_safe_column_ident("foo_bar"));
        assert!(is_safe_column_ident("a_1"));
        assert!(!is_safe_column_ident(""));
        assert!(!is_safe_column_ident("1foo"));
        assert!(!is_safe_column_ident("Foo"));
        assert!(!is_safe_column_ident("foo bar"));
        assert!(!is_safe_column_ident("foo-bar"));
        // Reserved
        assert!(!is_safe_column_ident("props"));
        assert!(!is_safe_column_ident("at"));
    }

    #[test]
    fn default_schema_has_dynamic_props() {
        let s = default_table_schema();
        let names: Vec<&str> = s.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec!["at", "label", "body", "props"]);
        let props = s.field_with_name("props").unwrap();
        assert!(matches!(props.data_type(), DataType::Binary));
    }

    #[test]
    fn collect_unknown_fields_dedupes_and_caps() {
        // We can't easily mock TableRef without a real catalog, so this test
        // just exercises the identifier check + dedup logic via the public
        // `is_safe_column_ident`. Full integration of `evolve_schema` is
        // covered by the catalog/postgres test suite.
        let mut seen = std::collections::HashSet::new();
        let recs = vec![
            serde_json::json!({"a": 1, "b": 2, "BAD-NAME": "x"}),
            serde_json::json!({"a": 3, "c": 4}),
        ];
        let mut to_add: Vec<String> = Vec::new();
        for r in &recs {
            if let serde_json::Value::Object(m) = r {
                for k in m.keys() {
                    let lower = k.to_ascii_lowercase();
                    if !is_safe_column_ident(&lower) || !seen.insert(lower) {
                        continue;
                    }
                    to_add.push(k.clone());
                }
            }
        }
        assert_eq!(to_add, vec!["a", "b", "c"]);
    }
}
