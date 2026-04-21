//! The `QueryFrontend` trait — language parsers that produce logical plans.
//!
//! # Why `Arc<dyn Any>` for the returned plan
//!
//! `kyma-core` cannot depend on `kyma-plan` — that would invert the crate
//! dependency graph (every crate depends on core). But a `QueryFrontend` has
//! to return _something_ plan-shaped. Solution: return the plan as
//! `Arc<dyn Any + Send + Sync>`; `kyma-plan::query_frontend_registry`
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
    /// The payload is typed `Arc<dyn Any + Send + Sync>` so `kyma-core`
    /// avoids depending on `kyma-plan`. Callers downcast via the plan-crate
    /// registry.
    async fn parse(&self, source: &str, ctx: &QueryContext) -> Result<Arc<dyn Any + Send + Sync>>;
}
