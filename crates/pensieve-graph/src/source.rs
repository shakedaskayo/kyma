//! Narrow read interface the schema-graph needs from a catalog. Keeping this
//! tiny (3 methods) lets `SchemaGraphProvider` be unit-tested with a fake.

use async_trait::async_trait;

/// A column as the schema-graph sees it (decoupled from `kyma_core::ColumnInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    /// kyma type token: `string`, `int`, `long`, `real`, `datetime`, `bool`, `dynamic`.
    pub type_: String,
    pub nullable: bool,
}

#[async_trait]
pub trait SchemaSource: Send + Sync {
    async fn databases(&self) -> anyhow::Result<Vec<String>>;
    async fn tables(&self, database: &str) -> anyhow::Result<Vec<String>>;
    async fn columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnDef>>;
}
