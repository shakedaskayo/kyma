use async_trait::async_trait;
use kyma_connectors::registry::ConnectorRegistry;
use kyma_connectors::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun};
use std::sync::Arc;

struct FakeConn;

#[async_trait]
impl Connector for FakeConn {
    fn type_id(&self) -> &'static str {
        "fake"
    }
    fn validate_config(&self, _: &serde_json::Value) -> Result<(), ConfigError> {
        Ok(())
    }
    async fn run_once(
        &self,
        _: &ConnectorCtx,
        _: &serde_json::Value,
        _: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        Ok(ConnectorRun {
            rows: vec![],
            new_cursor: None,
        })
    }
}

#[test]
fn register_and_lookup() {
    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(FakeConn));
    let c = reg.lookup("fake").expect("found");
    assert_eq!(c.type_id(), "fake");
    assert!(reg.lookup("missing").is_none());
}

#[test]
#[should_panic(expected = "already registered")]
fn double_register_panics() {
    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(FakeConn));
    reg.register(Arc::new(FakeConn));
}

#[test]
fn types_list() {
    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(FakeConn));
    let mut types: Vec<_> = reg.types().collect();
    types.sort();
    assert_eq!(types, vec!["fake"]);
}
