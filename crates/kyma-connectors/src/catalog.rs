//! Connector catalog — the vendor-agnostic, self-describing metadata that
//! powers the connectors UI. Each connector returns a [`CatalogEntry`] from
//! [`crate::types::Connector::catalog`]; the registry collects them and the
//! admin API serves them at `GET /v1/connectors/catalog` (merged with the
//! [`coming_soon`] list of not-yet-implemented vendors).
//!
//! Adding a new connector therefore needs **no UI change**: implement the
//! trait (including `catalog()`) and register it — it appears in the catalog
//! grid automatically, fully described (icon, category, auth, config fields).

use serde::Serialize;
use serde_json::Value;

/// A single config input the wizard renders for a connector.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogField {
    /// Key under the connector's `config` object.
    pub key: String,
    pub label: String,
    /// `"text"` | `"secret"`. Secrets render as password inputs and are scrubbed.
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl CatalogField {
    pub fn secret(key: &str, label: &str, placeholder: &str, help: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind: "secret".into(),
            required: true,
            placeholder: Some(placeholder.into()),
            help: Some(help.into()),
        }
    }
    pub fn text(key: &str, label: &str, placeholder: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind: "text".into(),
            required: true,
            placeholder: Some(placeholder.into()),
            help: None,
        }
    }
}

/// Describes an optional resource-selection step (e.g. picking repositories).
#[derive(Debug, Clone, Serialize)]
pub struct CatalogResource {
    /// Human label for the step + the selected items (e.g. `"Repositories"`).
    pub label: String,
    /// The `config` key the selected items are written under (e.g. `"repos"`).
    pub config_key: String,
    /// The secret field whose value unlocks fetching the list (e.g. `"token"`).
    pub token_field: String,
}

/// Self-describing metadata for one connector type.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub type_id: String,
    pub label: String,
    /// `"code"` | `"knowledge"` | `"project"` | `"data"`.
    pub category: String,
    pub description: String,
    /// simple-icons slug used by the UI to render a brand mark (e.g. `"github"`).
    pub brand: String,
    /// `"pat"` | `"oauth"` | `"url"` | `"none"`.
    pub auth_mode: String,
    /// `"available"` (registered + implemented) | `"coming_soon"`.
    pub status: String,
    pub default_schedule_ms: i64,
    pub fields: Vec<CatalogField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<CatalogResource>,
    /// Default target table base (e.g. `"github_nodes"`); UI uses it when the
    /// operator doesn't override the advanced target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_target_table: Option<String>,
    /// Config fragment the UI merges into what it sends (e.g. github modules).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_defaults: Option<Value>,
    /// The graph name this connector registers, if it produces a graph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_name: Option<String>,
    /// Credential kinds this connector accepts via `credential_id` in its
    /// config (e.g. `["url"]` for Postgres, `["pat","github_app"]` for
    /// GitHub once both are wired). The UI filters the credential picker by
    /// these kinds. Empty means the connector takes inline secrets only —
    /// every connector should opt in as it grows credential support.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_credential_kinds: Vec<String>,
}

impl CatalogEntry {
    /// Minimal fallback so a connector that doesn't override `catalog()` still
    /// shows up sensibly (rather than being invisible in the UI).
    pub fn minimal(type_id: &str) -> Self {
        Self {
            type_id: type_id.into(),
            label: type_id.into(),
            category: "data".into(),
            description: String::new(),
            brand: String::new(),
            auth_mode: "none".into(),
            status: "available".into(),
            default_schedule_ms: 5 * 60_000,
            fields: Vec::new(),
            resource: None,
            default_target_table: None,
            config_defaults: None,
            graph_name: None,
            accepted_credential_kinds: Vec::new(),
        }
    }
}

/// A "coming soon" catalog entry — presentation-only, no `Connector` impl yet.
fn soon(type_id: &str, label: &str, category: &str, brand: &str, auth: &str, desc: &str) -> CatalogEntry {
    CatalogEntry {
        type_id: type_id.into(),
        label: label.into(),
        category: category.into(),
        description: desc.into(),
        brand: brand.into(),
        auth_mode: auth.into(),
        status: "coming_soon".into(),
        default_schedule_ms: 5 * 60_000,
        fields: Vec::new(),
        resource: None,
        default_target_table: None,
        config_defaults: None,
        graph_name: None,
        accepted_credential_kinds: Vec::new(),
    }
}

/// Vendors we present in the catalog but haven't implemented yet. These are
/// merged with the registered connectors (which win on `type_id` collision) so
/// the catalog reads like a real product surface, not a single-vendor tool.
pub fn coming_soon() -> Vec<CatalogEntry> {
    vec![
        // Code & repositories
        soon("github", "GitHub", "code", "github", "pat",
             "Repositories, branches, pull requests, issues, contributors, and a deep code graph from GitHub."),
        soon("gitlab", "GitLab", "code", "gitlab", "pat",
             "Projects, merge requests, issues, and the code graph from GitLab."),
        soon("bitbucket", "Bitbucket", "code", "bitbucket", "pat",
             "Repositories, pull requests, and pipelines from Bitbucket Cloud."),
        // Knowledge & docs
        soon("notion", "Notion", "knowledge", "notion", "oauth",
             "Pages, databases, and their relations from a Notion workspace."),
        soon("confluence", "Confluence", "knowledge", "confluence", "oauth",
             "Spaces, pages, and links from Atlassian Confluence."),
        soon("googledrive", "Google Drive", "knowledge", "googledrive", "oauth",
             "Documents and folder structure from Google Drive."),
        soon("slack", "Slack", "knowledge", "slack", "oauth",
             "Channels, threads, and shared knowledge from Slack."),
        // Project & issues
        soon("jira", "Jira", "project", "jira", "oauth",
             "Projects, epics, issues, and their links from Jira."),
        soon("linear", "Linear", "project", "linear", "oauth",
             "Teams, issues, projects, and cycles from Linear."),
        soon("asana", "Asana", "project", "asana", "oauth",
             "Projects, tasks, and dependencies from Asana."),
        // Data & object stores
        soon("postgres", "Postgres", "data", "postgresql", "url",
             "Tables, columns, and foreign-key relationships from a Postgres database."),
        soon("s3", "Amazon S3", "data", "amazons3", "none",
             "Objects and prefixes from an S3 bucket as a navigable tree."),
    ]
}
