//! S3 data source — walks a bucket and emits a `Bucket → Prefix(es) → Object`
//! graph. Works against AWS S3 and any S3-compatible API (MinIO, Cloudflare R2,
//! Backblaze B2, …) — path-style and a custom endpoint are exposed in config.
//!
//! Read-only at runtime: only `list_with_delimiter` is used. Bounded per tick
//! (`max_objects`) so a huge bucket doesn't tie up a runner slot.

use async_trait::async_trait;
use futures::StreamExt;
use object_store::aws::AmazonS3Builder;
use object_store::{ObjectStore, path::Path as ObjPath};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;

use crate::catalog::{CatalogEntry, CatalogField};
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

#[derive(Debug, Deserialize)]
struct S3Config {
    bucket: String,
    /// Inline access key id. Optional when `credential_id` is set.
    #[serde(default)]
    access_key_id: String,
    /// Inline secret access key. Optional when `credential_id` is set.
    #[serde(default)]
    secret_access_key: String,
    /// Reference to a stored credential of kind `aws_creds`. Preferred over
    /// inline keys; rotation propagates and the secret is encrypted at rest.
    #[serde(default)]
    credential_id: Option<uuid::Uuid>,
    #[serde(default)]
    region: Option<String>,
    /// Custom endpoint for non-AWS S3-compatible stores (e.g. http://minio:9000).
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    path_style: bool,
    /// Optional prefix to limit the walk (e.g. "logs/2026/").
    #[serde(default)]
    prefix: Option<String>,
    /// Safety cap on objects ingested per tick.
    #[serde(default = "default_max_objects")]
    max_objects: usize,
}
fn default_max_objects() -> usize {
    2000
}

pub struct S3DataSource;

#[async_trait]
impl DataSource for S3DataSource {
    fn type_id(&self) -> &'static str {
        "s3"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "s3".into(),
            label: "Amazon S3".into(),
            category: "data".into(),
            description: "Walk an S3 bucket (or any S3-compatible store) into a graph of \
                          prefixes and objects."
                .into(),
            brand: "amazons3".into(),
            auth_mode: "url".into(),
            status: "available".into(),
            drive_model: "periodic".into(),
            default_schedule_ms: 30 * 60_000,
            fields: vec![
                CatalogField {
                    key: "bucket".into(), label: "Bucket".into(), kind: "text".into(),
                    required: true, placeholder: Some("my-bucket".into()), help: None,
                },
                CatalogField {
                    key: "access_key_id".into(), label: "Access key ID".into(), kind: "text".into(),
                    required: true, placeholder: Some("AKIA…".into()), help: None,
                },
                CatalogField::secret(
                    "secret_access_key",
                    "Secret access key",
                    "··········",
                    "Stored encrypted; only used to list objects in the bucket.",
                ),
                CatalogField {
                    key: "region".into(), label: "Region".into(), kind: "text".into(),
                    required: false, placeholder: Some("us-east-1".into()), help: None,
                },
                CatalogField {
                    key: "endpoint".into(), label: "Endpoint (S3-compatible stores only)".into(),
                    kind: "text".into(), required: false,
                    placeholder: Some("https://s3.example.com".into()),
                    help: Some("Leave blank for AWS. Set for MinIO, R2, B2, etc.".into()),
                },
                CatalogField {
                    key: "prefix".into(), label: "Prefix (optional)".into(), kind: "text".into(),
                    required: false, placeholder: Some("logs/2026/".into()), help: None,
                },
            ],
            resource: None,
            default_target_table: Some("s3_nodes".into()),
            config_defaults: Some(json!({ "path_style": false, "max_objects": 2000 })),
            graph_name: Some("s3".into()),
            accepted_credential_kinds: vec!["aws_creds".into()],
        }
    }

    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError> {
        let parsed: S3Config =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        if parsed.bucket.is_empty() {
            return Err(ConfigError("bucket is required".into()));
        }
        // Either a credential or both inline keys must be present.
        if parsed.credential_id.is_none()
            && (parsed.access_key_id.is_empty() || parsed.secret_access_key.is_empty())
        {
            return Err(ConfigError(
                "provide either `credential_id` (preferred) or both inline `access_key_id` + `secret_access_key`"
                    .into(),
            ));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &serde_json::Value,
        _cursor: Option<&serde_json::Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let parsed: S3Config = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;

        // Resolve access keys: credential_id (preferred) → inline. Both yield
        // a triple (access_key_id, secret_access_key, session_token?).
        let (access_key_id, secret_access_key, session_token): (String, String, Option<String>) =
            if let Some(cid) = parsed.credential_id {
                use pensieve_core::credentials::CredentialValue;
                let cred = ctx
                    .credentials
                    .get(ctx.tenant, cid)
                    .await
                    .map_err(|e| DataSourceError::Permanent(format!("resolve credential {cid}: {e}")))?;
                match cred.value {
                    CredentialValue::AwsCreds {
                        access_key_id,
                        secret_access_key,
                        session_token,
                    } => (access_key_id, secret_access_key, session_token),
                    other => {
                        return Err(DataSourceError::Permanent(format!(
                            "credential {cid} has kind={}; s3 data source requires `aws_creds`",
                            other.kind()
                        )));
                    }
                }
            } else {
                (parsed.access_key_id.clone(), parsed.secret_access_key.clone(), None)
            };

        let mut b = AmazonS3Builder::new()
            .with_bucket_name(&parsed.bucket)
            .with_access_key_id(&access_key_id)
            .with_secret_access_key(&secret_access_key)
            .with_virtual_hosted_style_request(!parsed.path_style);
        if let Some(t) = session_token.as_deref() {
            b = b.with_token(t);
        }
        if let Some(r) = parsed.region.as_deref() {
            b = b.with_region(r);
        } else {
            b = b.with_region("us-east-1");
        }
        if let Some(e) = parsed.endpoint.as_deref() {
            b = b.with_endpoint(e);
            if e.starts_with("http://") {
                b = b.with_allow_http(true);
            }
        }

        let store = b
            .build()
            .map_err(|e| DataSourceError::Permanent(format!("build s3 client: {e}")))?;

        let walk_prefix: Option<ObjPath> = parsed
            .prefix
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| ObjPath::from(p.trim_end_matches('/')));

        // List up to max_objects keys, then stop. Bounded by-design: bigger
        // buckets are picked up across ticks (next iteration will see new keys).
        let mut stream = store.list(walk_prefix.as_ref());
        let mut keys: Vec<(String, i64)> = Vec::new();
        while let Some(item) = stream.next().await {
            let meta = item.map_err(|e| DataSourceError::Transient(format!("list: {e}")))?;
            let key = meta.location.to_string();
            keys.push((key, meta.size as i64));
            if keys.len() >= parsed.max_objects {
                break;
            }
        }

        // ── transform → graph ─────────────────────────────────────────────────
        let mut nodes = Vec::<serde_json::Value>::new();
        let mut edges = Vec::<serde_json::Value>::new();
        let mut prefix_seen: HashSet<String> = HashSet::new();

        let bucket_id = format!("s3b::{}", parsed.bucket);
        nodes.push(json!({
            "id": bucket_id,
            "labels": ["S3Bucket"],
            "name": parsed.bucket,
            "endpoint": parsed.endpoint.clone().unwrap_or_default(),
        }));

        let prefix_id = |bkt: &str, p: &str| format!("s3p::{bkt}::{p}");
        let object_id = |bkt: &str, k: &str| format!("s3o::{bkt}::{k}");

        for (key, size) in &keys {
            // Walk the key's path components, emitting a Prefix node + CONTAINS
            // edge for each unseen ancestor, then the Object node and the leaf
            // CONTAINS edge from the immediate parent.
            let parts: Vec<&str> = key.split('/').collect();
            let mut parent_id = bucket_id.clone();
            for i in 0..parts.len() - 1 {
                let p = parts[..=i].join("/");
                let pid = prefix_id(&parsed.bucket, &p);
                if prefix_seen.insert(p.clone()) {
                    nodes.push(json!({
                        "id": pid,
                        "labels": ["S3Prefix"],
                        "name": parts[i],
                        "path": p,
                        "bucket": parsed.bucket,
                    }));
                    edges.push(json!({
                        "id": format!("e:contains:{parent_id}::{pid}"),
                        "src": parent_id,
                        "dst": pid,
                        "type": "CONTAINS",
                    }));
                }
                parent_id = pid;
            }
            let oid = object_id(&parsed.bucket, key);
            let leaf = parts.last().copied().unwrap_or("");
            nodes.push(json!({
                "id": oid,
                "labels": ["S3Object"],
                "name": leaf,
                "key": key,
                "size": size,
                "bucket": parsed.bucket,
            }));
            edges.push(json!({
                "id": format!("e:contains:{parent_id}::{oid}"),
                "src": parent_id,
                "dst": oid,
                "type": "CONTAINS",
            }));
        }

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: None,
            tables: vec![
                TableRows { table: "s3_nodes".into(), rows: nodes },
                TableRows { table: "s3_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "s3".into(),
                node_table: "s3_nodes".into(),
                edge_table: "s3_edges".into(),
            }),
        })
    }
}
