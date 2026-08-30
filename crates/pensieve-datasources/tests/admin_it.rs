use async_trait::async_trait;
use axum::http::StatusCode;
use axum::{Extension, Router};
use pensieve_catalog::PostgresCatalog;
use pensieve_datasources::admin::{router, AdminState};
use pensieve_datasources::catalog_trait::{DataSourceCatalog, PgDataSourceCatalog};
use pensieve_datasources::registry::DataSourceRegistry;
use pensieve_datasources::{ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun};
use pensieve_core::tenant::DEFAULT_TENANT;
use serde_json::json;
use sqlx::PgPool;  // used for WatcherRegistry::register
use std::sync::Arc;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

struct StubConn;

#[async_trait]
impl DataSource for StubConn {
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

/// A stub whose catalog declares `drive_model = "continuous"` (like obsidian) —
/// create must persist the registry's drive model, not hardcode periodic.
struct WatcherStub;

#[async_trait]
impl DataSource for WatcherStub {
    fn type_id(&self) -> &'static str {
        "wstub"
    }
    fn catalog(&self) -> pensieve_datasources::CatalogEntry {
        let mut c = pensieve_datasources::CatalogEntry::minimal("wstub");
        c.drive_model = "continuous".into();
        c
    }
    fn validate_config(&self, _cfg: &serde_json::Value) -> Result<(), ConfigError> {
        Ok(())
    }
    async fn run_once(
        &self,
        _: &DataSourceCtx,
        _: &serde_json::Value,
        _: Option<&serde_json::Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        Err(DataSourceError::Permanent("watcher-driven".into()))
    }
}

async fn state() -> (testcontainers::ContainerAsync<Postgres>, AdminState, PgPool) {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pg_catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());
    let pool = pg_catalog.pool().clone();
    let catalog: Arc<dyn DataSourceCatalog> =
        Arc::new(PgDataSourceCatalog::from_pg_catalog(&pg_catalog));
    let mut reg = DataSourceRegistry::new();
    reg.register(Arc::new(StubConn));
    reg.register(Arc::new(WatcherStub));
    let s = AdminState {
        catalog,
        registry: Arc::new(reg),
    };
    (pg, s, pool)
}

fn app(s: AdminState) -> Router {
    // The handlers expect a `TenantId` request extension (normally injected
    // by the auth middleware). Tests skip auth, so attach an Extension layer
    // that pins the default tenant.
    router(s).layer(Extension(DEFAULT_TENANT))
}

#[tokio::test]
async fn create_list_get_delete() {
    let (_pg, s, _pool) = state().await;
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
                .uri("/v1/data-sources")
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
                .uri("/v1/data-sources")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn watchers_list_empty_then_rows() {
    let (_pg, s, pool) = state().await;
    let app = app(s);

    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/data-sources/watchers")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["items"], json!([]));

    pensieve_datasources::watchers::WatcherRegistry::register(
        &pool, "filedrop", "h", "n", "u", json!({}),
    )
    .await
    .unwrap();

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/data-sources/watchers")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "filedrop");
}

#[tokio::test]
async fn catalog_includes_installed_and_drive_model() {
    let (_pg, s, _pool) = state().await;
    let app = app(s);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/data-sources/catalog")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let items = v["items"].as_array().unwrap();

    for item in items {
        let dm = item["drive_model"].as_str().unwrap_or("");
        assert!(!dm.is_empty(), "every entry carries drive_model: {item}");
    }
    let claude = items
        .iter()
        .find(|i| i["type_id"] == "claude_code")
        .expect("claude_code present");
    assert_eq!(claude["status"], "installed");
    assert_eq!(claude["drive_model"], "continuous");
    assert_eq!(claude["brand"], "claude");
    let wstub = items.iter().find(|i| i["type_id"] == "wstub").unwrap();
    assert_eq!(wstub["drive_model"], "continuous");
    assert_eq!(wstub["status"], "available");
}

#[tokio::test]
async fn create_uses_registry_drive_model() {
    let (_pg, s, pool) = state().await;
    let app = app(s);

    let body = json!({
        "name": "w1",
        "type": "wstub",
        "target_database": "db",
        "target_table": "",
        "schedule_ms": 60000,
        "config": {}
    });
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/data-sources")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let dm: String =
        sqlx::query_scalar("SELECT drive_model FROM data_sources WHERE name = 'w1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(dm, "continuous");
}

#[tokio::test]
async fn rejects_invalid_config() {
    let (_pg, s, _pool) = state().await;
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
                .uri("/v1/data-sources")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
