//! Notion data source — pages, databases, users, and their relations as a
//! property graph.
//!
//! Authenticates with a stored OAuth2 credential (the connect flow mints it).
//! Walks the workspace via `POST /v1/search` (sorted by `last_edited_time`
//! descending) so ticks are incremental: a stored `last_edited` floor stops
//! pagination once we reach already-seen items. Emits graph rows under the
//! `"notion"` graph; ids are prefixed `notion:` so they never collide.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

use crate::catalog::CatalogEntry;
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

const API: &str = "https://api.notion.com/v1";
const NOTION_VERSION: &str = "2022-06-28";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotionConfig {
    /// Stored OAuth2 credential (the only Notion auth path).
    #[serde(default)]
    credential_id: Option<Uuid>,
    /// Optional: restrict ingested pages to these database ids. Empty = all
    /// pages/databases the integration can see.
    #[serde(default)]
    databases: Vec<String>,
    /// Max `search` pages fetched per tick (each ≤ 100 items).
    #[serde(default = "default_max_pages")]
    max_pages_per_tick: usize,
}
fn default_max_pages() -> usize {
    10
}

pub struct NotionDataSource;

#[async_trait]
impl DataSource for NotionDataSource {
    fn type_id(&self) -> &'static str {
        "notion"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "notion".into(),
            label: "Notion".into(),
            category: "knowledge".into(),
            description: "Pages, databases, their relations, and authors from a Notion \
                          workspace — a navigable knowledge graph."
                .into(),
            brand: "notion".into(),
            auth_mode: "oauth".into(),
            status: "available".into(),
            drive_model: "periodic".into(),
            default_schedule_ms: 10 * 60_000,
            fields: Vec::new(),
            resource: None,
            default_target_table: Some("notion_nodes".into()),
            config_defaults: None,
            graph_name: Some("notion".into()),
            accepted_credential_kinds: vec!["oauth2".into()],
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let parsed: NotionConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        if parsed.credential_id.is_none() {
            return Err(ConfigError("a Notion OAuth credential is required".into()));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &Value,
        cursor: Option<&Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let config: NotionConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;
        let cid = config
            .credential_id
            .ok_or_else(|| DataSourceError::Config("credential_id required".into()))?;
        let token = crate::oauth::valid_access_token(ctx, cid).await?;

        let db_filter: HashSet<&str> = config.databases.iter().map(String::as_str).collect();
        let floor: Option<String> = cursor
            .and_then(|c| c.get("last_edited").and_then(Value::as_str))
            .map(String::from);
        let mut newest: Option<String> = floor.clone();

        let mut nodes: Vec<Value> = Vec::new();
        let mut edges: Vec<Value> = Vec::new();
        let mut user_seen: HashSet<String> = HashSet::new();

        let headers = [("Notion-Version", NOTION_VERSION)];
        let mut start_cursor: Option<String> = None;
        let mut pages_fetched = 0usize;

        'outer: loop {
            if pages_fetched >= config.max_pages_per_tick {
                break;
            }
            let mut body = json!({
                "sort": { "timestamp": "last_edited_time", "direction": "descending" },
                "page_size": 100,
            });
            if let Some(sc) = &start_cursor {
                body["start_cursor"] = json!(sc);
            }
            let resp =
                crate::oauth_http::post_json(&ctx.http, &format!("{API}/search"), &token, &body, &headers)
                    .await?;
            pages_fetched += 1;

            let results = resp
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in &results {
                let last_edited = item
                    .get("last_edited_time")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                // Sorted descending: once we cross the stored floor, everything
                // older has already been ingested — stop.
                if let Some(f) = &floor {
                    if !last_edited.is_empty() && last_edited <= f.as_str() {
                        break 'outer;
                    }
                }
                if newest.as_deref().map_or(true, |n| last_edited > n) {
                    newest = Some(last_edited.to_string());
                }
                emit_item(item, &db_filter, &mut nodes, &mut edges, &mut user_seen);
            }

            if resp.get("has_more").and_then(Value::as_bool) == Some(true) {
                start_cursor = resp.get("next_cursor").and_then(Value::as_str).map(String::from);
                if start_cursor.is_none() {
                    break;
                }
            } else {
                break;
            }
        }

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: Some(json!({ "last_edited": newest })),
            tables: vec![
                TableRows { table: "notion_nodes".into(), rows: nodes },
                TableRows { table: "notion_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "notion".into(),
                node_table: "notion_nodes".into(),
                edge_table: "notion_edges".into(),
            }),
        })
    }
}

/// Emit graph rows for one `search` result (a page or a database).
fn emit_item(
    item: &Value,
    db_filter: &HashSet<&str>,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    user_seen: &mut HashSet<String>,
) {
    let object = item.get("object").and_then(Value::as_str).unwrap_or("");
    let id = item.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return;
    }
    match object {
        "database" => {
            let node_id = format!("notion:db:{id}");
            nodes.push(json!({
                "id": node_id,
                "labels": ["Database"],
                "name": rich_text_plain(item.get("title")),
                "url": item.get("url").cloned().unwrap_or(Value::Null),
                "last_edited_time": item.get("last_edited_time").cloned().unwrap_or(Value::Null),
                "vendor": "notion",
            }));
            link_author(item, &node_id, nodes, edges, user_seen);
        }
        "page" => {
            // Optional scoping: skip pages outside the configured databases.
            let parent = item.get("parent");
            let parent_db = parent
                .and_then(|p| p.get("database_id"))
                .and_then(Value::as_str);
            if !db_filter.is_empty() {
                match parent_db {
                    Some(db) if db_filter.contains(db) => {}
                    _ => return,
                }
            }
            let node_id = format!("notion:page:{id}");
            nodes.push(json!({
                "id": node_id,
                "labels": ["Page"],
                "name": page_title(item),
                "url": item.get("url").cloned().unwrap_or(Value::Null),
                "archived": item.get("archived").cloned().unwrap_or(Value::Null),
                "last_edited_time": item.get("last_edited_time").cloned().unwrap_or(Value::Null),
                "vendor": "notion",
            }));

            // Parent containment.
            if let Some(db) = parent_db {
                let db_id = format!("notion:db:{db}");
                edges.push(json!({
                    "id": format!("e:contains:{db_id}::{node_id}"),
                    "src": db_id, "dst": node_id, "type": "CONTAINS",
                }));
            } else if let Some(pp) = parent.and_then(|p| p.get("page_id")).and_then(Value::as_str) {
                let parent_id = format!("notion:page:{pp}");
                edges.push(json!({
                    "id": format!("e:child_of:{node_id}::{parent_id}"),
                    "src": node_id, "dst": parent_id, "type": "CHILD_OF",
                }));
            }

            link_author(item, &node_id, nodes, edges, user_seen);
            emit_relations(item, &node_id, edges);
        }
        _ => {}
    }
}

/// Emit CREATED_BY / LAST_EDITED_BY edges (+ minimal User nodes).
fn link_author(
    item: &Value,
    node_id: &str,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    user_seen: &mut HashSet<String>,
) {
    for (field, rel) in [("created_by", "CREATED_BY"), ("last_edited_by", "LAST_EDITED_BY")] {
        if let Some(uid) = item.get(field).and_then(|u| u.get("id")).and_then(Value::as_str) {
            let user_id = format!("notion:user:{uid}");
            if user_seen.insert(user_id.clone()) {
                nodes.push(json!({
                    "id": user_id, "labels": ["User"], "name": uid, "vendor": "notion",
                }));
            }
            edges.push(json!({
                "id": format!("e:{}:{node_id}::{user_id}", rel.to_lowercase()),
                "src": node_id, "dst": user_id, "type": rel,
            }));
        }
    }
}

/// Emit RELATES_TO edges for every Notion `relation` property value — the
/// cross-page graph that makes a Notion workspace navigable.
fn emit_relations(item: &Value, node_id: &str, edges: &mut Vec<Value>) {
    let Some(props) = item.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (prop_name, prop) in props {
        if prop.get("type").and_then(Value::as_str) != Some("relation") {
            continue;
        }
        let Some(rels) = prop.get("relation").and_then(Value::as_array) else {
            continue;
        };
        for r in rels {
            if let Some(target) = r.get("id").and_then(Value::as_str) {
                let dst = format!("notion:page:{target}");
                edges.push(json!({
                    "id": format!("e:relates_to:{node_id}::{dst}::{prop_name}"),
                    "src": node_id, "dst": dst, "type": "RELATES_TO", "via": prop_name,
                }));
            }
        }
    }
}

/// Extract a page's title from its `properties` (the property of type `title`).
fn page_title(page: &Value) -> String {
    if let Some(props) = page.get("properties").and_then(Value::as_object) {
        for prop in props.values() {
            if prop.get("type").and_then(Value::as_str) == Some("title") {
                return rich_text_plain(prop.get("title"));
            }
        }
    }
    "Untitled".to_string()
}

/// Concatenate the `plain_text` of a Notion rich-text array into a string.
fn rich_text_plain(arr: Option<&Value>) -> String {
    let Some(arr) = arr.and_then(Value::as_array) else {
        return String::new();
    };
    let s: String = arr
        .iter()
        .filter_map(|rt| rt.get("plain_text").and_then(Value::as_str))
        .collect();
    if s.is_empty() {
        "Untitled".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_title_from_properties() {
        let page = json!({
            "object": "page",
            "properties": {
                "Name": { "type": "title", "title": [{ "plain_text": "Hello " }, { "plain_text": "World" }] }
            }
        });
        assert_eq!(page_title(&page), "Hello World");
    }

    #[test]
    fn emit_page_with_db_parent_and_relation() {
        let item = json!({
            "object": "page",
            "id": "p1",
            "last_edited_time": "2026-05-01T00:00:00.000Z",
            "url": "https://notion.so/p1",
            "parent": { "type": "database_id", "database_id": "db1" },
            "created_by": { "object": "user", "id": "u1" },
            "properties": {
                "Title": { "type": "title", "title": [{ "plain_text": "Task" }] },
                "Blocks": { "type": "relation", "relation": [{ "id": "p2" }] }
            }
        });
        let (mut nodes, mut edges, mut seen) = (vec![], vec![], HashSet::new());
        emit_item(&item, &HashSet::new(), &mut nodes, &mut edges, &mut seen);
        let types: Vec<&str> = edges.iter().filter_map(|e| e["type"].as_str()).collect();
        assert!(types.contains(&"CONTAINS"));
        assert!(types.contains(&"CREATED_BY"));
        assert!(types.contains(&"RELATES_TO"));
        assert!(nodes.iter().any(|n| n["id"] == "notion:page:p1"));
    }

    #[test]
    fn db_filter_skips_foreign_pages() {
        let item = json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-01-01T00:00:00Z",
            "parent": { "type": "database_id", "database_id": "other" },
            "properties": {}
        });
        let mut filter = HashSet::new();
        filter.insert("db1");
        let (mut nodes, mut edges, mut seen) = (vec![], vec![], HashSet::new());
        emit_item(&item, &filter, &mut nodes, &mut edges, &mut seen);
        assert!(nodes.is_empty(), "page outside the database filter must be skipped");
    }
}
