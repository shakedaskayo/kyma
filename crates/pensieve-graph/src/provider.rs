//! The provider abstraction every graph kind implements. G1a ships only
//! `SchemaGraphProvider`; later phases add `StoredGraphProvider` behind the
//! same trait.

use async_trait::async_trait;

use crate::types::{
    Direction, EdgeExpansion, GraphNode, GraphPayload, GraphSchema, GraphStats, SearchHits,
};

#[async_trait]
pub trait GraphProvider: Send + Sync {
    /// Capped sample of the whole graph: stats + nodes + edges.
    async fn overview(&self, realm: Option<&str>, limit: usize) -> anyhow::Result<GraphPayload>;
    /// A single node by id, or `None` if absent.
    async fn node(&self, id: &str) -> anyhow::Result<Option<GraphNode>>;
    /// Edges touching `ids` (respecting `dir`), plus the node ids newly reached.
    async fn neighbors(
        &self,
        ids: &[String],
        dir: Direction,
        only_internal: bool,
        limit: usize,
    ) -> anyhow::Result<EdgeExpansion>;
    /// BFS from `id` up to `depth` hops (both directions); returns the subgraph.
    async fn subgraph(&self, id: &str, depth: usize) -> anyhow::Result<GraphPayload>;
    /// Nodes whose name matches `text` (case-insensitive), optionally filtered
    /// by `labels` / `realm`.
    async fn search(
        &self,
        text: &str,
        labels: &[String],
        realm: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<SearchHits>;
    /// Node and relationship counts, optionally scoped to a single realm.
    async fn stats(&self, realm: Option<&str>) -> anyhow::Result<GraphStats>;
    /// Structural summary: known node kinds, edge types, and property keys per kind.
    async fn schema(&self) -> anyhow::Result<GraphSchema>;
}
