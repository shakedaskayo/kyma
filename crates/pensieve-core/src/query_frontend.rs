//! The `QueryFrontend` trait — language parsers that produce logical plans.
//!
//! # Why `Arc<dyn Any>` for the returned plan
//!
//! `pensieve-core` cannot depend on `pensieve-plan` — that would invert the crate
//! dependency graph (every crate depends on core). But a `QueryFrontend` has
//! to return _something_ plan-shaped. Solution: return the plan as
//! `Arc<dyn Any + Send + Sync>`; `pensieve-plan::query_frontend_registry`
//! downcasts it to the concrete `LogicalPlan`.
//!
//! This is the same pattern DataFusion uses for `ExecutionPlan::as_any()` —
//! one downcast at parse time buys a clean acyclic crate graph. Frontend
//! authors implement [`QueryFrontend`] and return `Arc::new(plan)` of their
//! concrete `LogicalPlan`.

use crate::errors::Result;
use async_trait::async_trait;
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

/// Context accompanying a single query.
#[derive(Debug, Clone)]
pub struct QueryContext {
    pub user: Option<String>,
    pub database: String,
    pub budget: QueryBudget,
    pub trace_id: Option<String>,
}

/// Resource budget for one query. Exceeding any limit cancels the query.
#[derive(Debug, Clone)]
pub struct QueryBudget {
    pub max_object_store_bytes: u64,
    pub max_memory_bytes: u64,
    pub max_wall_clock: Duration,
}

impl Default for QueryBudget {
    fn default() -> Self {
        Self {
            max_object_store_bytes: 10 * 1024 * 1024 * 1024, // 10 GiB
            max_memory_bytes: 4 * 1024 * 1024 * 1024,        // 4 GiB
            max_wall_clock: Duration::from_secs(300),        // 5 min
        }
    }
}

impl QueryBudget {
    /// Deployment-wide defaults: `Default` overridden by env vars
    /// `PENSIEVE_QUERY_MEMORY_BYTES`, `PENSIEVE_QUERY_WALL_MS`, and
    /// `PENSIEVE_QUERY_OBJECT_STORE_BYTES`. Read once and cached; per-request
    /// header overrides still apply on top in the server.
    pub fn from_env() -> Self {
        static CACHED: std::sync::OnceLock<QueryBudget> = std::sync::OnceLock::new();
        CACHED
            .get_or_init(|| {
                fn parse(key: &str) -> Option<u64> {
                    std::env::var(key).ok().and_then(|v| v.parse().ok())
                }
                let mut b = QueryBudget::default();
                if let Some(n) = parse("PENSIEVE_QUERY_MEMORY_BYTES") {
                    b.max_memory_bytes = n.max(1024 * 1024);
                }
                if let Some(ms) = parse("PENSIEVE_QUERY_WALL_MS") {
                    b.max_wall_clock = Duration::from_millis(ms.max(10));
                }
                if let Some(n) = parse("PENSIEVE_QUERY_OBJECT_STORE_BYTES") {
                    b.max_object_store_bytes = n.max(1024 * 1024);
                }
                b
            })
            .clone()
    }
}

/// A query-language frontend. One implementation per language.
#[async_trait]
pub trait QueryFrontend: Send + Sync {
    /// Short identifier used in tracing / logging / registry lookup:
    /// `"kql"`, `"sql"`, `"promql"`.
    fn name(&self) -> &'static str;

    /// MIME types that route incoming queries to this frontend.
    fn content_types(&self) -> &'static [&'static str];

    /// Parse a source string into an opaque logical-plan payload.
    ///
    /// The payload is typed `Arc<dyn Any + Send + Sync>` so `pensieve-core`
    /// avoids depending on `pensieve-plan`. Callers downcast via the plan-crate
    /// registry.
    async fn parse(&self, source: &str, ctx: &QueryContext) -> Result<Arc<dyn Any + Send + Sync>>;
}
