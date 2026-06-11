use kyma_datasources::prometheus::PromConnector;
use kyma_datasources::Connector;
use serde_json::json;

fn c() -> PromConnector {
    PromConnector::default()
}

#[test]
fn type_id_is_prometheus() {
    assert_eq!(c().type_id(), "prometheus");
}

#[test]
fn accepts_minimal_config() {
    let cfg = json!({ "endpoint": "http://127.0.0.1:9090/metrics" });
    c().validate_config(&cfg).expect("ok");
}

#[test]
fn accepts_https_and_http_only() {
    for scheme in &["http", "https"] {
        let cfg = json!({ "endpoint": format!("{scheme}://x/metrics") });
        c().validate_config(&cfg)
            .unwrap_or_else(|e| panic!("{scheme}: {e:?}"));
    }
    let cfg = json!({ "endpoint": "ftp://x/metrics" });
    c().validate_config(&cfg).expect_err("ftp rejected");
}

#[test]
fn rejects_missing_endpoint() {
    let cfg = json!({});
    let err = c().validate_config(&cfg).unwrap_err();
    assert!(err.0.contains("endpoint"), "error: {err:?}");
}

#[test]
fn auth_bearer_requires_token_ref() {
    let cfg = json!({
        "endpoint": "http://x/metrics",
        "auth": { "type": "bearer" }
    });
    let err = c().validate_config(&cfg).unwrap_err();
    assert!(err.0.contains("token_ref"), "error: {err:?}");
}

#[test]
fn auth_basic_requires_username_and_password() {
    let cfg = json!({
        "endpoint": "http://x/metrics",
        "auth": { "type": "basic", "username": "u" }
    });
    let err = c().validate_config(&cfg).unwrap_err();
    assert!(err.0.contains("password_ref"), "error: {err:?}");
}
