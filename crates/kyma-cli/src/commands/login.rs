//! `kyma login` — browser-flow OAuth.
//!
//! The CLI binds a localhost port, opens the dashboard's `/login?cli=1&redirect=...`
//! URL in the browser, and waits for the dashboard to redirect back to
//! `http://127.0.0.1:<port>/cb?token=<workspace-mcp-token>`. The token is then
//! validated by listing workspaces and the first workspace is saved as default.

use crate::api_client::ApiClient;
use crate::profile::{load, save};
use anyhow::{anyhow, Context, Result};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub async fn run(api_url: &str) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}/cb");

    let auth_url = format!(
        "{api_url}/login?cli=1&redirect={}",
        urlencoding::encode(&redirect)
    );
    println!("Opening {auth_url}\nIf the browser does not open, paste it manually.");
    let _ = webbrowser::open(&auth_url);

    let (tx, rx) = oneshot::channel::<String>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => break,
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let tx = tx.clone();
                    async move {
                        let q = req.uri().query().unwrap_or("");
                        let token = q.split('&').find_map(|kv| kv.strip_prefix("token="));
                        if let Some(t) = token {
                            if let Some(s) = tx.lock().await.take() {
                                let _ = s.send(t.to_string());
                            }
                            Ok::<_, hyper::Error>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .body(Full::new(Bytes::from(
                                        "kyma CLI authenticated. You can close this tab.",
                                    )))
                                    .unwrap(),
                            )
                        } else {
                            Ok(Response::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(Full::new(Bytes::from("missing ?token=")))
                                .unwrap())
                        }
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });

    let token = tokio::time::timeout(std::time::Duration::from_secs(180), rx)
        .await
        .map_err(|_| anyhow!("login timed out after 3 minutes"))?
        .context("login channel closed")?;

    // Validate by listing workspaces.
    let api = ApiClient::new(api_url.to_string(), Some(token.clone()));
    let workspaces = api.list_workspaces().await?;
    let first = workspaces
        .first()
        .ok_or_else(|| anyhow!("no workspaces; create one in the dashboard"))?;

    let mut creds = load().unwrap_or_default();
    creds.api_url = Some(api_url.to_string());
    creds.token = Some(token);
    creds.workspace_slug = Some(first.slug.clone());
    creds.workspace_id = Some(first.id.clone());
    creds.mcp_endpoint = Some(first.mcp_endpoint.clone());
    save(&creds)?;
    println!(
        "logged in. default workspace: {} ({})",
        first.name, first.slug
    );
    Ok(())
}
