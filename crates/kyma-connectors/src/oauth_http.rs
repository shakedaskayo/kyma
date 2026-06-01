//! Shared HTTP helpers for OAuth2 data connectors.
//!
//! Bearer-authenticated JSON `GET`/`POST` with consistent error classification:
//! 429, 5xx, network timeouts, and Google's `403 rateLimitExceeded` map to
//! [`ConnectorError::Transient`] (retried next tick); other 4xx map to
//! [`ConnectorError::Permanent`]. `Retry-After` is honored before a retry.
//!
//! Access tokens are resolved + refreshed via [`crate::oauth::valid_access_token`];
//! connectors pass the resulting bearer string here.

use std::time::Duration;

use reqwest::{Client, Method};
use serde_json::Value;

use crate::types::ConnectorError;

const MAX_RETRIES: u32 = 3;
const TIMEOUT: Duration = Duration::from_secs(30);

/// Bearer-authenticated JSON `GET`.
pub async fn get_json(
    http: &Client,
    url: &str,
    bearer: &str,
    extra_headers: &[(&str, &str)],
) -> Result<Value, ConnectorError> {
    request_json(http, Method::GET, url, bearer, extra_headers, None).await
}

/// Bearer-authenticated JSON `POST` with a JSON body.
pub async fn post_json(
    http: &Client,
    url: &str,
    bearer: &str,
    body: &Value,
    extra_headers: &[(&str, &str)],
) -> Result<Value, ConnectorError> {
    request_json(http, Method::POST, url, bearer, extra_headers, Some(body)).await
}

async fn request_json(
    http: &Client,
    method: Method,
    url: &str,
    bearer: &str,
    extra_headers: &[(&str, &str)],
    body: Option<&Value>,
) -> Result<Value, ConnectorError> {
    let mut attempt: u32 = 0;
    loop {
        let mut req = http
            .request(method.clone(), url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Accept", "application/json")
            .timeout(TIMEOUT);
        for (k, v) in extra_headers {
            req = req.header(*k, *v);
        }
        if let Some(b) = body {
            req = req.json(b);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let retryable = e.is_timeout() || e.is_connect();
                if retryable && attempt < MAX_RETRIES {
                    attempt += 1;
                    backoff(attempt).await;
                    continue;
                }
                return Err(if retryable {
                    ConnectorError::Transient(format!("{method} {url}: {e}"))
                } else {
                    ConnectorError::Permanent(format!("{method} {url}: {e}"))
                });
            }
        };

        let status = resp.status();
        let retry_after = parse_retry_after(resp.headers());
        if status.is_success() {
            return resp
                .json::<Value>()
                .await
                .map_err(|e| ConnectorError::Transient(format!("parse {url}: {e}")));
        }

        let code = status.as_u16();
        let text = resp.text().await.unwrap_or_default();
        // Google/Drive overload 403 for both rate-limit and real permission
        // errors — inspect the reason to avoid disabling a connector on a
        // transient quota blip.
        let rate_limited_403 = code == 403
            && (text.contains("rateLimitExceeded")
                || text.contains("userRateLimitExceeded")
                || text.contains("quotaExceeded"));
        let transient = code == 429 || status.is_server_error() || rate_limited_403;

        if transient && attempt < MAX_RETRIES {
            attempt += 1;
            match retry_after {
                Some(secs) => tokio::time::sleep(Duration::from_secs(secs.min(60))).await,
                None => backoff(attempt).await,
            }
            continue;
        }
        if transient {
            return Err(ConnectorError::Transient(format!("{method} {url} → {status}")));
        }
        return Err(ConnectorError::Permanent(format!(
            "{method} {url} → {status}: {}",
            truncate(&text, 200)
        )));
    }
}

/// Resolve the Atlassian Cloud id for the token's first accessible site (shared
/// by the Jira + Confluence connectors). Returns `Permanent` if the token has no
/// accessible Atlassian resources.
pub async fn resolve_atlassian_cloud_id(http: &Client, bearer: &str) -> Result<String, ConnectorError> {
    let resources = get_json(
        http,
        "https://api.atlassian.com/oauth/token/accessible-resources",
        bearer,
        &[],
    )
    .await?;
    resources
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|r| r.get("id"))
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| {
            ConnectorError::Permanent(
                "token has no accessible Atlassian sites — re-authorize and grant access".into(),
            )
        })
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

async fn backoff(attempt: u32) {
    let base = 250u64 * (1u64 << (attempt.saturating_sub(1)).min(5));
    let jitter = fastrand::u64(..base / 3 + 1);
    tokio::time::sleep(Duration::from_millis(base.saturating_add(jitter))).await;
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}
