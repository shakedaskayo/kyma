//! Typed credentials — the system-wide secret model.
//!
//! A [`Credential`] is an encrypted, labelled, typed secret value referenced by
//! id. Subsystems that need authenticated access to something (connectors,
//! MCP servers, agent providers, …) store a `credential_id` instead of inline
//! secrets, so a single rotation propagates.
//!
//! `CredentialValue` carries the actual secret material and is serialized as a
//! tagged JSON object inside the encrypted `enc_value` blob. Adding a new
//! credential kind is one variant + (optionally) a new auth method on the
//! catalog entries that accept it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The kinds of secret material a credential can hold. Each variant maps to a
/// recognisable real-world auth shape so callers can pattern-match precisely.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialValue {
    /// A single bearer token (Personal Access Token, API key, project token, …).
    Pat { token: String },
    /// HTTP Basic authentication.
    Basic { username: String, password: String },
    /// OAuth2 token pair — access + optional refresh + expiry.
    Oauth2 {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_at: Option<DateTime<Utc>>,
    },
    /// A connection URL that carries its own credentials (Postgres, MySQL, …).
    Url { connection_string: String },
    /// AWS-style static access keys.
    AwsCreds {
        access_key_id: String,
        secret_access_key: String,
        #[serde(default)]
        session_token: Option<String>,
    },
    /// Generic API key with an optional custom header name (default: bearer).
    ApiKey {
        value: String,
        #[serde(default)]
        header_name: Option<String>,
    },
    /// GitHub App authentication (app id + installation id + PEM private key).
    GithubApp {
        app_id: String,
        installation_id: String,
        private_key_pem: String,
    },
}

impl CredentialValue {
    /// Stable string identifier for the kind (matches the serde tag).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Pat { .. } => "pat",
            Self::Basic { .. } => "basic",
            Self::Oauth2 { .. } => "oauth2",
            Self::Url { .. } => "url",
            Self::AwsCreds { .. } => "aws_creds",
            Self::ApiKey { .. } => "api_key",
            Self::GithubApp { .. } => "github_app",
        }
    }

    /// Short, non-secret preview suitable for showing in lists (e.g.
    /// `ghp_…1f2a` or `kyma_admin / ····`). Never leaks more than the last
    /// 4 chars of any actual secret material.
    pub fn preview(&self) -> String {
        fn tail(s: &str) -> String {
            let n = s.chars().count();
            if n <= 4 {
                "·".repeat(n.max(1))
            } else {
                format!("{}{}", "·".repeat(n.saturating_sub(4).min(8)), &s[s.len().saturating_sub(4)..])
            }
        }
        match self {
            Self::Pat { token } => tail(token),
            Self::Basic { username, .. } => format!("{username} / ····"),
            Self::Oauth2 { access_token, .. } => tail(access_token),
            Self::Url { connection_string } => {
                // Show scheme + host, hide creds.
                if let Some(scheme_end) = connection_string.find("://") {
                    let scheme = &connection_string[..scheme_end];
                    let rest = &connection_string[scheme_end + 3..];
                    let host = rest.rsplit('@').next().unwrap_or(rest);
                    format!("{scheme}://···@{}", host.split('/').next().unwrap_or(host))
                } else {
                    "····".into()
                }
            }
            Self::AwsCreds { access_key_id, .. } => tail(access_key_id),
            Self::ApiKey { value, .. } => tail(value),
            Self::GithubApp { app_id, .. } => format!("app {app_id}"),
        }
    }
}

/// A credential as stored — `value` is the decrypted secret material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub id: Uuid,
    pub label: String,
    pub kind: String,
    /// Decrypted secret material — present only when the caller has explicit
    /// access to the plaintext (e.g. inside the connector framework). The
    /// REST API returns [`CredentialSummary`] instead.
    pub value: CredentialValue,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Public, non-secret view of a credential. This is what `GET /v1/credentials`
/// returns and what the UI displays — no `value`, just a short preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialSummary {
    pub id: Uuid,
    pub label: String,
    pub kind: String,
    pub preview: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Credential> for CredentialSummary {
    fn from(c: Credential) -> Self {
        let preview = c.value.preview();
        Self {
            id: c.id,
            label: c.label,
            kind: c.kind,
            preview,
            metadata: c.metadata,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

/// Abstract access to the credentials store. Implemented by [`kyma_catalog`]
/// for real persistence; trivial in-memory mocks make testing connectors easy
/// without standing up Postgres.
#[async_trait::async_trait]
pub trait CredentialStore: Send + Sync {
    async fn get(&self, tenant: crate::tenant::TenantId, id: Uuid) -> anyhow::Result<Credential>;
}
