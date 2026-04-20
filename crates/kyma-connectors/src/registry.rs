//! Compile-time connector-type registry.

use crate::types::Connector;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct ConnectorRegistry {
    by_type: HashMap<&'static str, Arc<dyn Connector>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, c: Arc<dyn Connector>) {
        let id = c.type_id();
        assert!(
            !self.by_type.contains_key(id),
            "connector type {id:?} already registered",
        );
        self.by_type.insert(id, c);
    }

    pub fn lookup(&self, type_id: &str) -> Option<Arc<dyn Connector>> {
        self.by_type.get(type_id).cloned()
    }

    pub fn types(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_type.keys().copied()
    }
}
