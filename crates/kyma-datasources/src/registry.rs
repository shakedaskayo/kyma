//! Compile-time data-source-type registry.

use crate::catalog::CatalogEntry;
use crate::types::DataSource;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct DataSourceRegistry {
    by_type: HashMap<&'static str, Arc<dyn DataSource>>,
}

impl DataSourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, c: Arc<dyn DataSource>) {
        let id = c.type_id();
        assert!(
            !self.by_type.contains_key(id),
            "connector type {id:?} already registered",
        );
        self.by_type.insert(id, c);
    }

    pub fn lookup(&self, type_id: &str) -> Option<Arc<dyn DataSource>> {
        self.by_type.get(type_id).cloned()
    }

    pub fn types(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_type.keys().copied()
    }

    /// Catalog metadata for every registered data source (status `"available"`).
    pub fn catalog(&self) -> Vec<CatalogEntry> {
        self.by_type.values().map(|c| c.catalog()).collect()
    }
}
