use async_trait::async_trait;
use kyma_datasources::registry::DataSourceRegistry;
use kyma_datasources::{ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun};
use std::sync::Arc;

struct FakeConn;

#[async_trait]
impl DataSource for FakeConn {
    fn type_id(&self) -> &'static str {
        "fake"
    }
    fn validate_config(&self, _: &serde_json::Value) -> Result<(), ConfigError> {
        Ok(())
    }
    async fn run_once(
        &self,
        _: &DataSourceCtx,
        _: &serde_json::Value,
        _: Option<&serde_json::Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        Ok(DataSourceRun {
            rows: vec![],
            new_cursor: None,
            tables: vec![],
            graph: None,
        })
    }
}

#[test]
fn register_and_lookup() {
    let mut reg = DataSourceRegistry::new();
    reg.register(Arc::new(FakeConn));
    let c = reg.lookup("fake").expect("found");
    assert_eq!(c.type_id(), "fake");
    assert!(reg.lookup("missing").is_none());
}

#[test]
#[should_panic(expected = "already registered")]
fn double_register_panics() {
    let mut reg = DataSourceRegistry::new();
    reg.register(Arc::new(FakeConn));
    reg.register(Arc::new(FakeConn));
}

#[test]
fn types_list() {
    let mut reg = DataSourceRegistry::new();
    reg.register(Arc::new(FakeConn));
    let mut types: Vec<_> = reg.types().collect();
    types.sort();
    assert_eq!(types, vec!["fake"]);
}
