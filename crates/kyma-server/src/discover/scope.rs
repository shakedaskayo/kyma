//! Scope resolver for cross-source Discover queries.
//!
//! Resolves a `Scope` (all / explicit source patterns / saved view) into a
//! concrete list of `(database, TableRef)` pairs the engine should fan out
//! to. Patterns are `db.table` globs where `*` is a single-segment wildcard.
//!
//! The resolver is tenant-aware: it uses `_in_tenant` catalog variants so
//! RBAC filters apply. Per-DB list errors are swallowed (treated as RBAC
//! drops) so a single inaccessible database does not poison the whole
//! request.

use std::sync::Arc;

use kyma_core::catalog::{Catalog, TableRef};
use kyma_core::tenant::TenantId;
use serde::Deserialize;

/// A user-supplied scope for a Discover request.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Scope {
    /// Every table the caller can see.
    All,
    /// Explicit list of `db.table` patterns (with `*` wildcards).
    Sources { sources: Vec<String> },
    /// A saved-view reference; sources are loaded via `SavedViewLookup`.
    View { view_id: String },
}

/// One concrete source the engine will fan out to.
#[derive(Clone, Debug)]
pub struct ResolvedSource {
    pub db: String,
    pub table: TableRef,
}

#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    #[error("scope resolves to {0} sources, exceeds max_sources_per_request={1}")]
    ScopeTooLarge(usize, usize),
    #[error("view {0} not found")]
    ViewNotFound(String),
    #[error("catalog error: {0}")]
    Catalog(String),
}

/// Pluggable lookup for saved-view → sources mappings.
#[async_trait::async_trait]
pub trait SavedViewLookup: Send + Sync {
    async fn load_sources(&self, view_id: &str) -> Result<Option<Vec<String>>, String>;
}

/// Match a `db.table` pattern against a concrete `(db, table)` pair.
///
/// `*` is a single-segment wildcard. The pattern must contain exactly one
/// `.` separator; patterns without one never match.
pub fn matches_pattern(pattern: &str, db: &str, table: &str) -> bool {
    let Some((p_db, p_tbl)) = pattern.split_once('.') else {
        return false;
    };
    seg_match(p_db, db) && seg_match(p_tbl, table)
}

fn seg_match(pat: &str, value: &str) -> bool {
    pat == "*" || pat == value
}

/// Resolve a scope into a concrete list of sources for the calling tenant.
///
/// Behavior:
/// - `Scope::All` is treated as the single pattern `*.*`.
/// - `Scope::View` requires `saved_view_lookup`; missing lookup or missing
///   view both yield `ViewNotFound`.
/// - Per-DB `list_tables_in_database_in_tenant` errors are swallowed
///   silently (RBAC drops).
/// - Fails fast with `ScopeTooLarge` once the result exceeds the cap.
pub async fn resolve(
    scope: &Scope,
    tenant: TenantId,
    catalog: Arc<dyn Catalog>,
    saved_view_lookup: Option<&(dyn SavedViewLookup + Send + Sync)>,
    max_sources_per_request: usize,
) -> Result<Vec<ResolvedSource>, ScopeError> {
    let patterns: Vec<String> = match scope {
        Scope::All => vec!["*.*".to_string()],
        Scope::Sources { sources } => sources.clone(),
        Scope::View { view_id } => match saved_view_lookup {
            None => return Err(ScopeError::ViewNotFound(view_id.clone())),
            Some(l) => match l
                .load_sources(view_id)
                .await
                .map_err(ScopeError::Catalog)?
            {
                None => return Err(ScopeError::ViewNotFound(view_id.clone())),
                Some(srcs) => srcs,
            },
        },
    };

    let dbs = catalog
        .list_databases_in_tenant(tenant)
        .await
        .map_err(|e| ScopeError::Catalog(e.to_string()))?;

    let mut out: Vec<ResolvedSource> = Vec::new();
    for db in dbs {
        let tables = match catalog
            .list_tables_in_database_in_tenant(tenant, &db)
            .await
        {
            Ok(t) => t,
            Err(_) => continue, // RBAC drop / per-DB error: skip silently
        };
        for t in tables {
            let matched = patterns
                .iter()
                .any(|p| matches_pattern(p, &db, &t.name));
            if !matched {
                continue;
            }
            out.push(ResolvedSource {
                db: db.clone(),
                table: t,
            });
            if out.len() > max_sources_per_request {
                return Err(ScopeError::ScopeTooLarge(
                    out.len(),
                    max_sources_per_request,
                ));
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_exact() {
        assert!(matches_pattern("prod.otel_logs", "prod", "otel_logs"));
        assert!(!matches_pattern("prod.otel_logs", "prod", "http_reqs"));
        assert!(!matches_pattern("prod.otel_logs", "stg", "otel_logs"));
    }

    #[test]
    fn pattern_glob_table() {
        assert!(matches_pattern("prod.*", "prod", "anything"));
        assert!(matches_pattern("prod.*", "prod", "otel_logs"));
        assert!(!matches_pattern("prod.*", "stg", "anything"));
    }

    #[test]
    fn pattern_glob_db() {
        assert!(matches_pattern("*.otel_logs", "prod", "otel_logs"));
        assert!(matches_pattern("*.otel_logs", "stg", "otel_logs"));
        assert!(!matches_pattern("*.otel_logs", "prod", "http_reqs"));
    }

    #[test]
    fn pattern_full_wildcard() {
        assert!(matches_pattern("*.*", "any", "thing"));
        assert!(matches_pattern("*.*", "x", "y"));
    }
}
