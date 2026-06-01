//! Token-endpoint exchanges: authorization-code → tokens, and refresh-token →
//! tokens. Handles both standard JSON token responses and Slack's
//! `{ "ok": true, … }` envelope.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use serde_json::Value;

use super::client::ClientCreds;
use super::provider::{ClientAuthStyle, OAuthProvider};

/// The token material returned by an exchange or refresh.
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Exchange an authorization `code` for tokens.
pub async fn exchange_code(
    http: &Client,
    provider: &OAuthProvider,
    client: &ClientCreds,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
) -> Result<TokenSet> {
    let mut form: Vec<(String, String)> = vec![
        ("grant_type".into(), "authorization_code".into()),
        ("code".into(), code.into()),
        ("redirect_uri".into(), redirect_uri.into()),
    ];
    if let Some(v) = code_verifier {
        form.push(("code_verifier".into(), v.into()));
    }
    post_token(http, provider, client, form).await
}

/// Exchange a `refresh_token` for a fresh access token.
pub async fn refresh_token(
    http: &Client,
    provider: &OAuthProvider,
    client: &ClientCreds,
    refresh_token: &str,
) -> Result<TokenSet> {
    let form: Vec<(String, String)> = vec![
        ("grant_type".into(), "refresh_token".into()),
        ("refresh_token".into(), refresh_token.into()),
    ];
    post_token(http, provider, client, form).await
}

async fn post_token(
    http: &Client,
    provider: &OAuthProvider,
    client: &ClientCreds,
    mut form: Vec<(String, String)>,
) -> Result<TokenSet> {
    let mut req = http.post(provider.token_url).header("Accept", "application/json");
    match provider.client_auth {
        ClientAuthStyle::BodyPost => {
            form.push(("client_id".into(), client.client_id.clone()));
            form.push(("client_secret".into(), client.client_secret.clone()));
        }
        ClientAuthStyle::BasicHeader => {
            req = req.basic_auth(&client.client_id, Some(&client.client_secret));
        }
    }
    let resp = req
        .form(&form)
        .send()
        .await
        .map_err(|e| anyhow!("token request: {e}"))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("token response decode: {e}"))?;

    if provider.envelope_ok {
        // Slack: HTTP 200 even on failure; the real signal is `ok`.
        if !body.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let err = body.get("error").and_then(Value::as_str).unwrap_or("unknown");
            return Err(anyhow!("provider error: {err}"));
        }
    } else if !status.is_success() {
        let err = body
            .get("error_description")
            .or_else(|| body.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(anyhow!("token endpoint {status}: {err}"));
    }
    parse_token_set(&body)
}

fn parse_token_set(body: &Value) -> Result<TokenSet> {
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing access_token in token response"))?
        .to_string();
    let refresh_token = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(String::from);
    let expires_at = body
        .get("expires_in")
        .and_then(Value::as_i64)
        .map(|secs| Utc::now() + Duration::seconds(secs));
    Ok(TokenSet {
        access_token,
        refresh_token,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_token_response() {
        let body = serde_json::json!({
            "access_token": "at-123",
            "refresh_token": "rt-456",
            "expires_in": 3600
        });
        let t = parse_token_set(&body).unwrap();
        assert_eq!(t.access_token, "at-123");
        assert_eq!(t.refresh_token.as_deref(), Some("rt-456"));
        assert!(t.expires_at.is_some());
    }

    #[test]
    fn parses_token_without_refresh_or_expiry() {
        let body = serde_json::json!({ "access_token": "xoxb-789" });
        let t = parse_token_set(&body).unwrap();
        assert_eq!(t.access_token, "xoxb-789");
        assert!(t.refresh_token.is_none());
        assert!(t.expires_at.is_none());
    }
}
