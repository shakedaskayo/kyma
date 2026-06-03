//! Icon gallery — the server-side source of truth mapping entity *kinds* and
//! *vendors* to icon names, so entities carry an explicit `icon` the web graph
//! (and any other client) can render, and the agent can pick one when minting
//! entities.
//!
//! Icon names are stable string keys shared with the web client's icon registry
//! (`web/src/features/graph/graph-icons.tsx`): kind keys like `service`, `repo`,
//! `database`, `person`, `infra`, …, and brand keys like `github`, `datadog`,
//! `kubernetes`, `slack`, `aws`, `postgresql`, ….
//!
//! Built-in defaults cover the common set; deployments extend/override them with
//! a JSON file pointed to by `KYMA_ICON_GALLERY`, shaped:
//! ```json
//! { "kinds": { "pipeline": "function" }, "vendors": { "clickhouse": "clickhouse" } }
//! ```

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IconGallery {
    /// entity kind → icon name (e.g. "service" → "service").
    #[serde(default)]
    pub kinds: BTreeMap<String, String>,
    /// vendor / source slug → brand icon name (e.g. "k8s" → "kubernetes").
    #[serde(default)]
    pub vendors: BTreeMap<String, String>,
    /// resource slug → kind icon name, for classification types
    /// (`provider::resource`, e.g. the "pod" in "kubernetes::pod").
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

impl IconGallery {
    /// Built-in defaults. Values are icon names known to the web icon registry.
    pub fn builtin() -> Self {
        let kinds = [
            ("service", "service"),
            ("server", "service"),
            ("repo", "repo"),
            ("repository", "repo"),
            ("table", "table"),
            ("column", "column"),
            ("database", "database"),
            ("schema", "schema"),
            ("person", "person"),
            ("user", "person"),
            ("team", "person"),
            ("file", "file"),
            ("directory", "directory"),
            ("config", "config"),
            ("secret", "secret"),
            ("credential", "secret"),
            ("concept", "concept"),
            ("namespace", "namespace"),
            ("deployment", "deployment"),
            ("pod", "pod"),
            ("infra", "infra"),
            ("infracomponent", "infra"),
            ("endpoint", "endpoint"),
            ("api", "api"),
            ("function", "function"),
            ("class", "class"),
            ("module", "module"),
            ("memory", "memory"),
            ("fact", "memory"),
            ("decision", "concept"),
            ("issue", "issue"),
            ("pullrequest", "pullrequest"),
            ("organization", "organization"),
        ];
        let vendors = [
            ("github", "github"),
            ("gitlab", "gitlab"),
            ("bitbucket", "bitbucket"),
            ("slack", "slack"),
            ("datadog", "datadog"),
            ("kubernetes", "kubernetes"),
            ("k8s", "kubernetes"),
            ("docker", "docker"),
            ("grafana", "grafana"),
            ("pagerduty", "pagerduty"),
            ("gcp", "gcp"),
            ("googlecloud", "gcp"),
            ("aws", "aws"),
            ("amazon", "aws"),
            ("s3", "s3"),
            ("postgres", "postgresql"),
            ("postgresql", "postgresql"),
            ("prometheus", "prometheus"),
            ("sentry", "sentry"),
            ("opentelemetry", "opentelemetry"),
            ("otel", "opentelemetry"),
            ("redis", "redis"),
            ("mongodb", "mongodb"),
            ("snowflake", "snowflake"),
            ("elastic", "elastic"),
            ("elasticsearch", "elastic"),
            ("terraform", "terraform"),
            ("jira", "jira"),
            ("linear", "linear"),
            ("asana", "asana"),
            ("notion", "notion"),
            ("confluence", "confluence"),
            ("googledrive", "googledrive"),
            ("gmail", "gmail"),
        ];
        // Cloud/infra resource → kind icon, for `provider::resource` types.
        let resources = [
            ("pod", "pod"),
            ("container", "pod"),
            ("deployment", "deployment"),
            ("replicaset", "deployment"),
            ("statefulset", "deployment"),
            ("daemonset", "deployment"),
            ("service", "service"),
            ("svc", "service"),
            ("ingress", "endpoint"),
            ("node", "infra"),
            ("cluster", "infra"),
            ("namespace", "namespace"),
            ("configmap", "config"),
            ("secret", "secret"),
            ("instance", "service"),
            ("vm", "service"),
            ("ec2", "service"),
            ("lambda", "function"),
            ("function", "function"),
            ("bucket", "database"),
            ("s3", "database"),
            ("volume", "database"),
            ("disk", "database"),
            ("database", "database"),
            ("db", "database"),
            ("rds", "database"),
            ("table", "table"),
            ("queue", "module"),
            ("topic", "module"),
            ("stream", "module"),
            ("repo", "repo"),
            ("repository", "repo"),
            ("monitor", "service"),
            ("dashboard", "concept"),
            ("alert", "issue"),
        ];
        Self {
            kinds: kinds.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            vendors: vendors.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            resources: resources.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    /// Built-ins, then merge any overrides from `KYMA_ICON_GALLERY` (a JSON file).
    pub fn load() -> Self {
        let mut g = Self::builtin();
        if let Ok(path) = std::env::var("KYMA_ICON_GALLERY") {
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<IconGallery>(&s) {
                    Ok(extra) => {
                        for (k, v) in extra.kinds {
                            g.kinds.insert(norm(&k), v);
                        }
                        for (k, v) in extra.vendors {
                            g.vendors.insert(norm(&k), v);
                        }
                        for (k, v) in extra.resources {
                            g.resources.insert(norm(&k), v);
                        }
                    }
                    Err(e) => tracing::warn!("KYMA_ICON_GALLERY parse failed: {e}"),
                },
                Err(e) => tracing::warn!("KYMA_ICON_GALLERY read failed ({path}): {e}"),
            }
        }
        g
    }

    /// Process-global gallery (loaded once).
    pub fn global() -> &'static IconGallery {
        static G: OnceLock<IconGallery> = OnceLock::new();
        G.get_or_init(IconGallery::load)
    }

    /// Resolve an icon name from a kind and/or vendor. Vendor wins (a GitHub
    /// repo shows the GitHub mark, not a generic repo glyph).
    pub fn resolve(&self, kind: Option<&str>, vendor: Option<&str>) -> Option<String> {
        if let Some(v) = vendor.map(norm).filter(|s| !s.is_empty()) {
            if let Some(icon) = self.vendors.get(&v) {
                return Some(icon.clone());
            }
        }
        if let Some(k) = kind.map(norm).filter(|s| !s.is_empty()) {
            if let Some(icon) = self.kinds.get(&k) {
                return Some(icon.clone());
            }
        }
        None
    }

    /// Resolve an icon from a classification type `provider::resource`
    /// (also accepts `:` or `/` separators). The provider drives the brand mark;
    /// failing that the resource drives a kind glyph.
    pub fn resolve_type(&self, type_str: &str) -> Option<String> {
        let parts: Vec<String> = type_str
            .split(|c| c == ':' || c == '/')
            .map(norm)
            .filter(|s| !s.is_empty())
            .collect();
        if let Some(provider) = parts.first() {
            if let Some(icon) = self.vendors.get(provider) {
                return Some(icon.clone());
            }
        }
        if let Some(resource) = parts.last() {
            if let Some(icon) = self.resources.get(resource) {
                return Some(icon.clone());
            }
            if let Some(icon) = self.kinds.get(resource) {
                return Some(icon.clone());
            }
        }
        None
    }

    /// A compact, de-duplicated hint of available icon names for the agent prompt.
    pub fn catalog_hint(&self) -> String {
        let mut kinds: Vec<&str> = self.kinds.values().map(String::as_str).collect();
        kinds.sort_unstable();
        kinds.dedup();
        let mut brands: Vec<&str> = self.vendors.values().map(String::as_str).collect();
        brands.sort_unstable();
        brands.dedup();
        format!("kinds: {} · brands: {}", kinds.join(", "), brands.join(", "))
    }
}
