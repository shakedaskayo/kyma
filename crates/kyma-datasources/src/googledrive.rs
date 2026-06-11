//! Google Drive data source — files, folders, owners, and the folder tree as a
//! property graph.
//!
//! Authenticates with a stored OAuth2 credential (scope
//! `drive.metadata.readonly`). Walks `files.list` page-by-page, carrying the
//! `pageToken` in the cursor so a large drive is covered across ticks and then
//! re-walked (picking up changes). Emits graph rows under the `"googledrive"`
//! graph; ids are prefixed `gdrive:`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

use crate::catalog::CatalogEntry;
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const FIELDS: &str = "nextPageToken,files(id,name,mimeType,parents,modifiedTime,\
owners(displayName,emailAddress,permissionId),webViewLink,driveId,trashed,size)";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GdriveConfig {
    #[serde(default)]
    credential_id: Option<Uuid>,
    /// Optional: restrict to direct children of these folder ids. Empty = the
    /// whole drive the token can see.
    #[serde(default)]
    folders: Vec<String>,
    #[serde(default)]
    include_shared_drives: bool,
    /// Max `files.list` pages fetched per tick (each ≤ 100 files).
    #[serde(default = "default_max_pages")]
    max_pages_per_tick: usize,
}
fn default_max_pages() -> usize {
    10
}

pub struct GdriveDataSource;

#[async_trait]
impl DataSource for GdriveDataSource {
    fn type_id(&self) -> &'static str {
        "googledrive"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "googledrive".into(),
            label: "Google Drive".into(),
            category: "knowledge".into(),
            description: "Files, folders, owners, and the folder structure from Google \
                          Drive — a navigable document graph (metadata only)."
                .into(),
            brand: "googledrive".into(),
            auth_mode: "oauth".into(),
            status: "available".into(),
            default_schedule_ms: 10 * 60_000,
            fields: Vec::new(),
            resource: None,
            default_target_table: Some("googledrive_nodes".into()),
            config_defaults: None,
            graph_name: Some("googledrive".into()),
            accepted_credential_kinds: vec!["oauth2".into()],
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let parsed: GdriveConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        if parsed.credential_id.is_none() {
            return Err(ConfigError("a Google OAuth credential is required".into()));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &Value,
        cursor: Option<&Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let config: GdriveConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;
        let cid = config
            .credential_id
            .ok_or_else(|| DataSourceError::Config("credential_id required".into()))?;
        let token = crate::oauth::valid_access_token(ctx, cid).await?;

        let q = build_query(&config.folders);
        let shared = if config.include_shared_drives { "true" } else { "false" };

        let mut nodes: Vec<Value> = Vec::new();
        let mut edges: Vec<Value> = Vec::new();
        let mut user_seen: HashSet<String> = HashSet::new();

        // Resume from the stored pageToken; walk a bounded number of pages this
        // tick; persist where we stopped (None ⇒ start over next tick).
        let mut page_token: Option<String> = cursor
            .and_then(|c| c.get("page_token").and_then(Value::as_str))
            .map(String::from);
        let mut pages = 0usize;

        let next_cursor_token: Option<String> = loop {
            if pages >= config.max_pages_per_tick {
                // Hit the per-tick page cap — resume here next tick.
                break page_token.clone();
            }
            let mut url = reqwest::Url::parse(FILES_URL)
                .map_err(|e| DataSourceError::Permanent(format!("url: {e}")))?;
            {
                let mut p = url.query_pairs_mut();
                p.append_pair("q", &q);
                p.append_pair("fields", FIELDS);
                p.append_pair("pageSize", "100");
                p.append_pair("supportsAllDrives", "true");
                p.append_pair("includeItemsFromAllDrives", shared);
                p.append_pair("corpora", if config.include_shared_drives { "allDrives" } else { "user" });
                if let Some(pt) = &page_token {
                    p.append_pair("pageToken", pt);
                }
            }
            let resp = crate::oauth_http::get_json(&ctx.http, url.as_str(), &token, &[]).await?;
            pages += 1;

            for file in resp.get("files").and_then(Value::as_array).into_iter().flatten() {
                emit_file(file, &mut nodes, &mut edges, &mut user_seen);
            }

            match resp.get("nextPageToken").and_then(Value::as_str) {
                Some(tok) => page_token = Some(tok.to_string()),
                // Reached the end — restart the walk next tick.
                None => break None,
            }
        };

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: Some(json!({ "page_token": next_cursor_token })),
            tables: vec![
                TableRows { table: "googledrive_nodes".into(), rows: nodes },
                TableRows { table: "googledrive_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "googledrive".into(),
                node_table: "googledrive_nodes".into(),
                edge_table: "googledrive_edges".into(),
            }),
        })
    }
}

fn build_query(folders: &[String]) -> String {
    if folders.is_empty() {
        return "trashed = false".to_string();
    }
    let ors: Vec<String> = folders
        .iter()
        .map(|f| format!("'{}' in parents", f.replace('\'', "\\'")))
        .collect();
    format!("({}) and trashed = false", ors.join(" or "))
}

/// Emit a node (File or Folder) + CONTAINS (parent→child) and OWNED_BY edges.
fn emit_file(
    file: &Value,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    user_seen: &mut HashSet<String>,
) {
    let id = file.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return;
    }
    let node_id = format!("gdrive:{id}");
    let is_folder = file.get("mimeType").and_then(Value::as_str) == Some(FOLDER_MIME);
    let label = if is_folder { "Folder" } else { "File" };
    nodes.push(json!({
        "id": node_id,
        "labels": [label],
        "name": file.get("name").and_then(Value::as_str).unwrap_or("untitled"),
        "mimeType": file.get("mimeType").cloned().unwrap_or(Value::Null),
        "modifiedTime": file.get("modifiedTime").cloned().unwrap_or(Value::Null),
        "size": file.get("size").cloned().unwrap_or(Value::Null),
        "webViewLink": file.get("webViewLink").cloned().unwrap_or(Value::Null),
        "trashed": file.get("trashed").cloned().unwrap_or(Value::Null),
        "vendor": "googledrive",
    }));

    // Folder structure: a CONTAINS edge from each parent.
    for parent in file.get("parents").and_then(Value::as_array).into_iter().flatten() {
        if let Some(pid) = parent.as_str() {
            let parent_id = format!("gdrive:{pid}");
            edges.push(json!({
                "id": format!("e:contains:{parent_id}::{node_id}"),
                "src": parent_id, "dst": node_id, "type": "CONTAINS",
            }));
        }
    }

    // Ownership.
    for owner in file.get("owners").and_then(Value::as_array).into_iter().flatten() {
        let key = owner
            .get("emailAddress")
            .and_then(Value::as_str)
            .or_else(|| owner.get("permissionId").and_then(Value::as_str));
        if let Some(key) = key {
            let user_id = format!("gdrive:user:{key}");
            if user_seen.insert(user_id.clone()) {
                nodes.push(json!({
                    "id": user_id,
                    "labels": ["User"],
                    "name": owner.get("displayName").and_then(Value::as_str).unwrap_or(key),
                    "email": owner.get("emailAddress").cloned().unwrap_or(Value::Null),
                    "vendor": "googledrive",
                }));
            }
            edges.push(json!({
                "id": format!("e:owned_by:{node_id}::{user_id}"),
                "src": node_id, "dst": user_id, "type": "OWNED_BY",
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_scopes_to_folders() {
        assert_eq!(build_query(&[]), "trashed = false");
        let q = build_query(&["abc".into(), "def".into()]);
        assert!(q.contains("'abc' in parents"));
        assert!(q.contains("'def' in parents"));
        assert!(q.contains("trashed = false"));
    }

    #[test]
    fn emit_folder_and_file_with_edges() {
        let folder = json!({ "id": "F", "name": "Docs", "mimeType": FOLDER_MIME, "parents": ["root"] });
        let file = json!({
            "id": "X", "name": "report.pdf", "mimeType": "application/pdf",
            "parents": ["F"], "owners": [{ "displayName": "Ada", "emailAddress": "ada@x.io" }]
        });
        let (mut nodes, mut edges, mut seen) = (vec![], vec![], HashSet::new());
        emit_file(&folder, &mut nodes, &mut edges, &mut seen);
        emit_file(&file, &mut nodes, &mut edges, &mut seen);
        assert!(nodes.iter().any(|n| n["id"] == "gdrive:F" && n["labels"][0] == "Folder"));
        assert!(nodes.iter().any(|n| n["id"] == "gdrive:X" && n["labels"][0] == "File"));
        let types: Vec<&str> = edges.iter().filter_map(|e| e["type"].as_str()).collect();
        assert!(types.contains(&"CONTAINS"));
        assert!(types.contains(&"OWNED_BY"));
    }
}
