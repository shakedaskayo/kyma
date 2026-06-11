//! Exercises PromConnector::run_once against an in-process HTTP server.

use kyma_datasources::metrics::ConnectorMetrics;
use kyma_datasources::prometheus::PromConnector;
use kyma_datasources::runner::NoopCredentialStore;
use kyma_datasources::secrets::EnvSecretStore;
use kyma_datasources::{Connector, ConnectorCtx};
use serde_json::json;
use std::sync::Arc;

async fn spawn_mock(body: &'static str, status: u16) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let body = body;
            let status = status;
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 8192];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 {status} x\r\nContent-Length: {len}\r\n\r\n{body}",
                    len = body.len(),
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/metrics")
}

fn ctx() -> ConnectorCtx {
    ConnectorCtx {
        connector_id: uuid::Uuid::new_v4(),
        tenant: kyma_core::tenant::DEFAULT_TENANT,
        http: reqwest::Client::new(),
        secrets: Arc::new(EnvSecretStore),
        credentials: Arc::new(NoopCredentialStore),
        oauth: None,
        scheduled_for: chrono::Utc::now(),
        metrics: ConnectorMetrics {
            connector_id: uuid::Uuid::new_v4(),
            type_id: "prometheus",
        },
        artifacts: None,
        catalog: None,
        target_database: "default".into(),
    }
}

#[tokio::test]
async fn happy_path_emits_rows() {
    let endpoint = spawn_mock("a_metric{x=\"y\"} 1\nb_metric 2\n", 200).await;
    let cfg = json!({ "endpoint": endpoint });
    let run = PromConnector::default()
        .run_once(&ctx(), &cfg, None)
        .await
        .expect("ok");
    assert_eq!(run.rows.len(), 2);
    assert!(run.new_cursor.is_none());
    let r0 = &run.rows[0];
    assert_eq!(r0["name"], "a_metric");
    assert_eq!(r0["labels"]["x"], "y");
    assert_eq!(r0["value"], 1.0);
    assert!(r0["timestamp"].is_string());
}

#[tokio::test]
async fn http_5xx_is_transient() {
    let endpoint = spawn_mock("", 503).await;
    let cfg = json!({ "endpoint": endpoint });
    let err = PromConnector::default()
        .run_once(&ctx(), &cfg, None)
        .await
        .unwrap_err();
    match err {
        kyma_datasources::ConnectorError::Transient(_) => {}
        other => panic!("expected Transient, got {other:?}"),
    }
}

#[tokio::test]
async fn http_4xx_is_permanent() {
    let endpoint = spawn_mock("", 404).await;
    let cfg = json!({ "endpoint": endpoint });
    let err = PromConnector::default()
        .run_once(&ctx(), &cfg, None)
        .await
        .unwrap_err();
    match err {
        kyma_datasources::ConnectorError::Permanent(_) => {}
        other => panic!("expected Permanent, got {other:?}"),
    }
}
