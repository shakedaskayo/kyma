//! Self-heal `~/.pensieve/config.json` on `pensieve serve` startup.
//!
//! The CLI, the loopback MCP server, and every coding-agent capture hook
//! authenticate with the token stored in `~/.pensieve/config.json`. Web-UI
//! session tokens expire within the hour, so a `pensieve connect` run with a
//! browser token silently 401s all of them an hour later (the 2026-06-12
//! capture outage). On startup the local server validates the stored token
//! against its own auth backend and mints a durable `api`-kind admin token
//! to replace it when it is missing, expired, revoked, or unknown — so a
//! fresh install, an upgrade, or a clobbered config always converges to a
//! working CLI connection without user action.
//!
//! A config that points at a *different* server (a remote control plane) is
//! never touched.

use anyhow::{Context, Result};
use pensieve_core::catalog::Catalog;
use pensieve_server::auth::{hash_token, AuthBackend};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HealOutcome {
    /// Stored token authenticates and the endpoint is set — nothing to do.
    TokenValid,
    /// Config points at another server — left alone.
    ForeignEndpoint,
    /// Token was valid but the endpoint was missing; endpoint written.
    EndpointRepaired,
    /// Minted + persisted a fresh durable token (and the endpoint).
    TokenMinted,
}

/// The loopback URL the CLI should dial for this serve.
fn serve_endpoint(addr: SocketAddr) -> String {
    if addr.ip().is_unspecified() {
        format!("http://127.0.0.1:{}", addr.port())
    } else {
        format!("http://{addr}")
    }
}

/// Does `endpoint` (as stored in config.json) point at this serve?
fn is_this_serve(endpoint: &str, addr: SocketAddr) -> bool {
    let trimmed = endpoint.trim().trim_end_matches('/');
    let Some(rest) = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
    else {
        return false;
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()),
        None => (rest, Some(80)),
    };
    if port != Some(addr.port()) {
        return false;
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0") || host == addr.ip().to_string()
}

/// Validate the stored CLI token against `backend`; mint + persist a durable
/// replacement when it doesn't authenticate. Preserves unknown config fields
/// (e.g. `last_session_id`) and 0600s the file. Never clobbers a config that
/// points at a different server.
pub(crate) async fn heal_cli_config(
    config_path: &Path,
    addr: SocketAddr,
    backend: &Arc<dyn AuthBackend>,
    catalog: &Arc<dyn Catalog>,
) -> Result<HealOutcome> {
    let mut cfg: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();

    let endpoint_set = match cfg.get("endpoint").and_then(|v| v.as_str()) {
        Some(ep) if !ep.is_empty() => {
            if !is_this_serve(ep, addr) {
                return Ok(HealOutcome::ForeignEndpoint);
            }
            true
        }
        _ => false,
    };

    let token_valid = match cfg.get("token").and_then(|v| v.as_str()) {
        Some(tok) if !tok.is_empty() => backend.authenticate(tok).await.is_ok(),
        _ => false,
    };
    if token_valid && endpoint_set {
        return Ok(HealOutcome::TokenValid);
    }

    if !token_valid {
        // Durable admin token (`kind='api'`, no expiry). Two UUIDv4s give
        // 244 bits of CSPRNG entropy; safe to store as unsalted SHA-256.
        let raw = format!(
            "kyl_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        catalog
            .insert_api_token(&hash_token(&raw), "admin", Some("local-cli"), "api", None)
            .await
            .context("persisting minted CLI token")?;
        cfg.insert("token".into(), serde_json::Value::String(raw));
    }
    cfg.insert(
        "endpoint".into(),
        serde_json::Value::String(serve_endpoint(addr)),
    );

    if let Some(dir) = config_path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(cfg))?;
    std::fs::write(config_path, json)
        .with_context(|| format!("write {}", config_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(config_path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(if token_valid {
        HealOutcome::EndpointRepaired
    } else {
        HealOutcome::TokenMinted
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pensieve_catalog_sqlite::SqliteCatalog;
    use pensieve_server::auth::{EnvAuthBackend, SessionAuthBackend};

    const ADDR: &str = "127.0.0.1:7777";

    async fn fixture(dir: &Path) -> (Arc<dyn Catalog>, Arc<dyn AuthBackend>) {
        let catalog: Arc<dyn Catalog> = Arc::new(
            SqliteCatalog::connect(dir.join("catalog.db").to_str().unwrap())
                .await
                .expect("sqlite catalog"),
        );
        let backend: Arc<dyn AuthBackend> = Arc::new(SessionAuthBackend::new(
            catalog.clone(),
            EnvAuthBackend::default(),
            true,
        ));
        (catalog, backend)
    }

    fn read_cfg(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("config exists"))
            .expect("valid json")
    }

    #[tokio::test]
    async fn missing_config_mints_working_token() {
        let dir = tempfile::tempdir().unwrap();
        let (catalog, backend) = fixture(dir.path()).await;
        let cfg_path = dir.path().join("config.json");

        let out = heal_cli_config(&cfg_path, ADDR.parse().unwrap(), &backend, &catalog)
            .await
            .unwrap();
        assert_eq!(out, HealOutcome::TokenMinted);

        let cfg = read_cfg(&cfg_path);
        assert_eq!(cfg["endpoint"], "http://127.0.0.1:7777");
        let tok = cfg["token"].as_str().unwrap();
        assert!(tok.starts_with("kyl_"));
        backend
            .authenticate(tok)
            .await
            .expect("minted token authenticates");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&cfg_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn valid_token_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let (catalog, backend) = fixture(dir.path()).await;
        let cfg_path = dir.path().join("config.json");

        let first = heal_cli_config(&cfg_path, ADDR.parse().unwrap(), &backend, &catalog)
            .await
            .unwrap();
        assert_eq!(first, HealOutcome::TokenMinted);
        let tok_before = read_cfg(&cfg_path)["token"].as_str().unwrap().to_string();

        let second = heal_cli_config(&cfg_path, ADDR.parse().unwrap(), &backend, &catalog)
            .await
            .unwrap();
        assert_eq!(second, HealOutcome::TokenValid);
        assert_eq!(read_cfg(&cfg_path)["token"], tok_before.as_str());
    }

    #[tokio::test]
    async fn stale_token_replaced_preserving_other_fields() {
        let dir = tempfile::tempdir().unwrap();
        let (catalog, backend) = fixture(dir.path()).await;
        let cfg_path = dir.path().join("config.json");
        std::fs::write(
            &cfg_path,
            r#"{"endpoint":"http://127.0.0.1:7777","token":"expired-session-token","last_session_id":"s-42"}"#,
        )
        .unwrap();

        let out = heal_cli_config(&cfg_path, ADDR.parse().unwrap(), &backend, &catalog)
            .await
            .unwrap();
        assert_eq!(out, HealOutcome::TokenMinted);

        let cfg = read_cfg(&cfg_path);
        assert_eq!(cfg["last_session_id"], "s-42", "unknown fields preserved");
        let tok = cfg["token"].as_str().unwrap();
        assert_ne!(tok, "expired-session-token");
        backend
            .authenticate(tok)
            .await
            .expect("replacement authenticates");
    }

    #[tokio::test]
    async fn foreign_endpoint_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let (catalog, backend) = fixture(dir.path()).await;
        let cfg_path = dir.path().join("config.json");
        let original = r#"{"endpoint":"https://cloud.getpensieve.dev","token":"remote-token"}"#;
        std::fs::write(&cfg_path, original).unwrap();

        let out = heal_cli_config(&cfg_path, ADDR.parse().unwrap(), &backend, &catalog)
            .await
            .unwrap();
        assert_eq!(out, HealOutcome::ForeignEndpoint);
        assert_eq!(std::fs::read_to_string(&cfg_path).unwrap(), original);
    }

    #[tokio::test]
    async fn unspecified_bind_matches_loopback_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let (catalog, backend) = fixture(dir.path()).await;
        let cfg_path = dir.path().join("config.json");
        std::fs::write(
            &cfg_path,
            r#"{"endpoint":"http://localhost:9999","token":"stale"}"#,
        )
        .unwrap();

        // Serve bound to 0.0.0.0:9999 — localhost:9999 is "this serve".
        let out = heal_cli_config(
            &cfg_path,
            "0.0.0.0:9999".parse().unwrap(),
            &backend,
            &catalog,
        )
        .await
        .unwrap();
        assert_eq!(out, HealOutcome::TokenMinted);
        assert_eq!(read_cfg(&cfg_path)["endpoint"], "http://127.0.0.1:9999");
    }

    #[test]
    fn is_this_serve_cases() {
        let addr: SocketAddr = "127.0.0.1:7777".parse().unwrap();
        assert!(is_this_serve("http://127.0.0.1:7777", addr));
        assert!(is_this_serve("http://localhost:7777/", addr));
        assert!(!is_this_serve("http://localhost:8080", addr));
        assert!(!is_this_serve("https://cloud.getpensieve.dev", addr));
        assert!(!is_this_serve("not a url", addr));
    }
}
