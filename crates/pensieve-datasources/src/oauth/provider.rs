//! Static registry of supported OAuth2 providers.
//!
//! Each [`OAuthProvider`] is vendor-agnostic, plain data — the authorize/token
//! endpoints, default scopes, PKCE support, how client creds are presented, and
//! the env-var suffix for operator-configured apps. Several data sources can map
//! onto one provider (Gmail + Drive → `google`; Jira + Confluence →
//! `atlassian`); see [`provider_for_data_source`].

use serde::Serialize;

/// How a provider's client credentials are presented at the token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuthStyle {
    /// `client_id` + `client_secret` in the form body (Google, Atlassian, Slack).
    BodyPost,
    /// HTTP Basic `Authorization` header (Notion).
    BasicHeader,
}

/// Self-describing metadata for one OAuth2 provider.
#[derive(Debug, Clone)]
pub struct OAuthProvider {
    /// Provider slug used in the route path and as the `oauth_clients` key.
    pub slug: &'static str,
    /// Human label (e.g. `"Google"`).
    pub display: &'static str,
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    /// Default scopes requested when the caller doesn't override them.
    pub default_scopes: &'static [&'static str],
    /// Separator used to join scopes in the `scope` param (space for most,
    /// comma for Slack).
    pub scope_sep: &'static str,
    pub use_pkce: bool,
    pub client_auth: ClientAuthStyle,
    /// Extra static query params appended to the authorize URL (e.g. Google's
    /// `access_type=offline` + `prompt=consent` to force a refresh token).
    pub extra_authorize_params: &'static [(&'static str, &'static str)],
    /// Suffix for operator env vars: `PENSIEVE_OAUTH_<ENV_KEY>_CLIENT_ID` / `_SECRET`.
    pub env_key: &'static str,
    /// Slack returns the token JSON wrapped in `{ "ok": true, … }` with HTTP 200
    /// even on logical errors — the token parser special-cases this.
    pub envelope_ok: bool,
}

static PROVIDERS: &[OAuthProvider] = &[
    OAuthProvider {
        slug: "google",
        display: "Google",
        authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        default_scopes: &["openid", "email"],
        scope_sep: " ",
        use_pkce: true,
        client_auth: ClientAuthStyle::BodyPost,
        // access_type=offline + prompt=consent are required to receive a
        // refresh_token from Google (otherwise only an access token comes back).
        extra_authorize_params: &[("access_type", "offline"), ("prompt", "consent")],
        env_key: "GOOGLE",
        envelope_ok: false,
    },
    OAuthProvider {
        slug: "notion",
        display: "Notion",
        authorize_url: "https://api.notion.com/v1/oauth/authorize",
        token_url: "https://api.notion.com/v1/oauth/token",
        // Notion is capability-based (set on the integration), not scope-based.
        default_scopes: &[],
        scope_sep: " ",
        use_pkce: false,
        client_auth: ClientAuthStyle::BasicHeader,
        extra_authorize_params: &[("owner", "user")],
        env_key: "NOTION",
        envelope_ok: false,
    },
    OAuthProvider {
        slug: "atlassian",
        display: "Atlassian",
        authorize_url: "https://auth.atlassian.com/authorize",
        token_url: "https://auth.atlassian.com/oauth/token",
        // offline_access yields a refresh token; read:me resolves accessible
        // resources (cloud id). Data sources append their own read scopes.
        default_scopes: &["offline_access", "read:me"],
        scope_sep: " ",
        use_pkce: true,
        client_auth: ClientAuthStyle::BodyPost,
        extra_authorize_params: &[("audience", "api.atlassian.com"), ("prompt", "consent")],
        env_key: "ATLASSIAN",
        envelope_ok: false,
    },
    OAuthProvider {
        slug: "slack",
        display: "Slack",
        authorize_url: "https://slack.com/oauth/v2/authorize",
        token_url: "https://slack.com/api/oauth.v2.access",
        default_scopes: &["channels:read", "channels:history", "users:read"],
        scope_sep: ",",
        use_pkce: false,
        client_auth: ClientAuthStyle::BodyPost,
        extra_authorize_params: &[],
        env_key: "SLACK",
        envelope_ok: true,
    },
];

/// Look up a provider by its slug (e.g. `"google"`).
pub fn lookup(slug: &str) -> Option<&'static OAuthProvider> {
    PROVIDERS.iter().find(|p| p.slug == slug)
}

/// All registered providers.
pub fn all() -> &'static [OAuthProvider] {
    PROVIDERS
}

/// The OAuth scopes a data source needs — the full set the UI requests at
/// `/start` (already including `offline_access` / `read:me` where a provider
/// needs them for refresh + cloud-id resolution). Empty ⇒ use provider defaults.
pub fn scopes_for_data_source(type_id: &str) -> Vec<String> {
    let scopes: &[&str] = match type_id {
        "googledrive" => &["https://www.googleapis.com/auth/drive.metadata.readonly"],
        "gmail" => &["https://www.googleapis.com/auth/gmail.metadata"],
        "slack" => &[
            "channels:read",
            "channels:history",
            "groups:read",
            "groups:history",
            "users:read",
        ],
        "jira" => &["read:jira-work", "read:jira-user", "read:me", "offline_access"],
        "confluence" => &[
            "read:confluence-content.all",
            "read:confluence-space.summary",
            "read:confluence-user",
            "read:me",
            "offline_access",
        ],
        // Notion is capability-based (no scope strings).
        _ => &[],
    };
    scopes.iter().map(|s| s.to_string()).collect()
}

/// Map a data source `type_id` to its OAuth provider slug. Several data sources can
/// share one provider/app (Gmail + Drive → Google; Jira + Confluence →
/// Atlassian); everything else maps to a same-named provider.
pub fn provider_for_data_source(type_id: &str) -> &str {
    match type_id {
        "googledrive" | "gmail" => "google",
        "jira" | "confluence" => "atlassian",
        "notion" => "notion",
        "slack" => "slack",
        // Unknown data sources map to a same-named provider — borrow the input
        // rather than claiming a `'static` lifetime we don't have.
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_resolves() {
        for p in all() {
            assert!(lookup(p.slug).is_some());
            assert!(p.authorize_url.starts_with("https://"));
            assert!(p.token_url.starts_with("https://"));
        }
    }

    #[test]
    fn data_source_provider_mapping() {
        assert_eq!(provider_for_data_source("gmail"), "google");
        assert_eq!(provider_for_data_source("googledrive"), "google");
        assert_eq!(provider_for_data_source("jira"), "atlassian");
        assert_eq!(provider_for_data_source("confluence"), "atlassian");
        assert_eq!(provider_for_data_source("notion"), "notion");
        assert_eq!(provider_for_data_source("slack"), "slack");
    }
}
