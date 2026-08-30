//! The seam that lets the stored-graph provider run SQL over node/edge tables
//! without `kyma-graph` depending on the engine. `kyma-server` implements this
//! over its DataFusion query path.

use async_trait::async_trait;

/// One result row: column name -> JSON value.
pub type JsonRow = serde_json::Map<String, serde_json::Value>;

#[async_trait]
pub trait GraphQueryExecutor: Send + Sync {
    /// Run `sql` against `database`'s tables, returning rows as JSON objects.
    async fn query(&self, database: &str, sql: String) -> anyhow::Result<Vec<JsonRow>>;
}

/// Column roles + table names for a registered graph (decoupled mirror of the
/// catalog's `GraphRegistration`). The server maps registration → this.
#[derive(Debug, Clone)]
pub struct StoredGraphConfig {
    pub database: String,
    pub node_table: String,
    pub edge_table: String,
    pub id_col: String,
    pub label_col: String,
    pub src_col: String,
    pub dst_col: String,
    pub type_col: String,
    pub realm_col: Option<String>,
}
