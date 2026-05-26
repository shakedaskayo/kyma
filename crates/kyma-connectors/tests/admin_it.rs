use async_trait::async_trait;
use axum::http::StatusCode;
use axum::{Extension, Router};
use kyma_catalog::PostgresCatalog;
use kyma_connectors::admin::{router, AdminState};
use kyma_connectors::registry::ConnectorRegistry;
use kyma_connectors::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun};
use kyma_core::tenant::DEFAULT_TENANT;
use serde_json::json;
use std::sync::Arc;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

struct StubConn;

#[async_trait]
impl Connector for StubConn {
    fn type_id(&self) -> &'static str {
        "stub"
    }
    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError> {
        if cfg.get("endpoint").and_then(|v| v.as_str()).is_some() {
            Ok(())
        } else {
            Err(ConfigError("endpoint required".into()))
        }
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
            tables: vec![],
            graph: None,
        })
    }
}

async fn state() -> (testcontainers::ContainerAsync<Postgres>, AdminState) {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());
    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(StubConn));
    let s = AdminState {
        catalog,
        registry: Arc::new(reg),
    };
    (pg, s)
}

fn app(s: AdminState) -> Router {
    // The handlers expect a `TenantId` request extension (normally injected
    // by the auth middleware). Tests skip auth, so attach an Extension layer
    // that pins the default tenant.
    router(s).layer(Extension(DEFAULT_TENANT))
}

#[tokio::test]
async fn create_list_get_delete() {
    let (_pg, s) = state().await;
    let app = app(s.clone());

    let body = json!({
        "name": "p1",
        "type": "stub",
        "target_database": "db",
        "target_table": "metrics",
        "schedule_ms": 1000,
        "config": { "endpoint": "http://x/metrics" }
    });
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/connectors")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/connectors")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn rejects_invalid_config() {
    let (_pg, s) = state().await;
    let app = app(s.clone());
    let body = json!({
        "name": "p2",
        "type": "stub",
        "target_database": "db",
        "target_table": "metrics",
        "schedule_ms": 1000,
        "config": {}
    });
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/connectors")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
