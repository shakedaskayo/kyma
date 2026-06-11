//! Gmail data source — messages, threads, and the correspondent graph (metadata
//! only; never message bodies).
//!
//! Authenticates with a stored OAuth2 token (scope `gmail.metadata`). Lists
//! message ids page-by-page (carrying the `pageToken` in the cursor across
//! ticks), then fetches header-only metadata for each. Ids are prefixed
//! `gmail:`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

use crate::catalog::CatalogEntry;
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GmailConfig {
    #[serde(default)]
    credential_id: Option<Uuid>,
    /// Gmail search query that scopes which messages are ingested.
    #[serde(default = "default_query")]
    query: String,
    /// Hard cap on messages fetched per tick (one metadata GET each).
    #[serde(default = "default_max_messages")]
    max_messages_per_tick: usize,
}
fn default_query() -> String {
    "newer_than:1y".into()
}
fn default_max_messages() -> usize {
    100
}

pub struct GmailConnector;

#[async_trait]
impl DataSource for GmailConnector {
    fn type_id(&self) -> &'static str {
        "gmail"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "gmail".into(),
            label: "Gmail".into(),
            category: "knowledge".into(),
            description: "Threads, messages, and the people you correspond with — a metadata \
                          graph (headers only, never message bodies)."
                .into(),
            brand: "gmail".into(),
            auth_mode: "oauth".into(),
            status: "available".into(),
            default_schedule_ms: 15 * 60_000,
            fields: Vec::new(),
            resource: None,
            default_target_table: Some("gmail_nodes".into()),
            config_defaults: None,
            graph_name: Some("gmail".into()),
            accepted_credential_kinds: vec!["oauth2".into()],
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let parsed: GmailConfig =
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
        let config: GmailConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;
        let cid = config
            .credential_id
            .ok_or_else(|| DataSourceError::Config("credential_id required".into()))?;
        let token = crate::oauth::valid_access_token(ctx, cid).await?;

        let mut nodes: Vec<Value> = Vec::new();
        let mut edges: Vec<Value> = Vec::new();
        let mut user_seen: HashSet<String> = HashSet::new();

        // List message ids (resume from stored pageToken).
        let mut list_url = reqwest::Url::parse(&format!("{BASE}/messages"))
            .map_err(|e| DataSourceError::Permanent(format!("url: {e}")))?;
        {
            let mut p = list_url.query_pairs_mut();
            p.append_pair("q", &config.query);
            p.append_pair("maxResults", &config.max_messages_per_tick.min(500).to_string());
            if let Some(tok) = cursor.and_then(|c| c.get("page_token")).and_then(Value::as_str) {
                p.append_pair("pageToken", tok);
            }
        }
        let list = crate::oauth_http::get_json(&ctx.http, list_url.as_str(), &token, &[]).await?;
        let next_token = list.get("nextPageToken").and_then(Value::as_str).map(String::from);

        let ids: Vec<String> = list
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| m.get("id").and_then(Value::as_str).map(String::from))
            .take(config.max_messages_per_tick)
            .collect();

        for id in &ids {
            let mut url = reqwest::Url::parse(&format!("{BASE}/messages/{id}"))
                .map_err(|e| DataSourceError::Permanent(format!("url: {e}")))?;
            {
                let mut p = url.query_pairs_mut();
                p.append_pair("format", "metadata");
                for h in ["From", "To", "Cc", "Subject", "Date"] {
                    p.append_pair("metadataHeaders", h);
                }
            }
            let msg = crate::oauth_http::get_json(&ctx.http, url.as_str(), &token, &[]).await?;
            emit_message(&msg, &mut nodes, &mut edges, &mut user_seen);
        }

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: Some(json!({ "page_token": next_token })),
            tables: vec![
                TableRows { table: "gmail_nodes".into(), rows: nodes },
                TableRows { table: "gmail_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "gmail".into(),
                node_table: "gmail_nodes".into(),
                edge_table: "gmail_edges".into(),
            }),
        })
    }
}

fn emit_message(
    msg: &Value,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    user_seen: &mut HashSet<String>,
) {
    let id = msg.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return;
    }
    let thread = msg.get("threadId").and_then(Value::as_str).unwrap_or(id);
    let headers = header_map(msg);
    let subject = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("subject"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();

    let msg_id = format!("gmail:msg:{id}");
    let thread_id = format!("gmail:thread:{thread}");
    nodes.push(json!({
        "id": msg_id, "labels": ["Message"], "name": subject,
        "snippet": msg.get("snippet").cloned().unwrap_or(Value::Null),
        "labelIds": msg.get("labelIds").cloned().unwrap_or(Value::Null),
        "vendor": "gmail",
    }));
    nodes.push(json!({
        "id": thread_id, "labels": ["Thread"], "name": subject, "vendor": "gmail",
    }));
    edges.push(json!({
        "id": format!("e:in_thread:{msg_id}::{thread_id}"),
        "src": msg_id, "dst": thread_id, "type": "IN_THREAD",
    }));

    for (key, val) in &headers {
        match key.to_ascii_lowercase().as_str() {
            "from" => {
                for (name, email) in parse_addresses(val) {
                    let uid = upsert_user(&email, &name, nodes, user_seen);
                    edges.push(json!({
                        "id": format!("e:sent:{uid}::{msg_id}"),
                        "src": uid, "dst": msg_id, "type": "SENT",
                    }));
                }
            }
            "to" | "cc" => {
                for (name, email) in parse_addresses(val) {
                    let uid = upsert_user(&email, &name, nodes, user_seen);
                    edges.push(json!({
                        "id": format!("e:received:{msg_id}::{uid}"),
                        "src": msg_id, "dst": uid, "type": "RECEIVED",
                    }));
                }
            }
            _ => {}
        }
    }
}

fn upsert_user(email: &str, name: &str, nodes: &mut Vec<Value>, seen: &mut HashSet<String>) -> String {
    let uid = format!("gmail:user:{}", email.to_ascii_lowercase());
    if seen.insert(uid.clone()) {
        nodes.push(json!({
            "id": uid, "labels": ["User"],
            "name": if name.is_empty() { email } else { name },
            "email": email, "vendor": "gmail",
        }));
    }
    uid
}

fn header_map(msg: &Value) -> Vec<(String, String)> {
    msg.get("payload")
        .and_then(|p| p.get("headers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let name = h.get("name").and_then(Value::as_str)?.to_string();
            let value = h.get("value").and_then(Value::as_str)?.to_string();
            Some((name, value))
        })
        .collect()
}

/// Parse an address-list header into `(display_name, email)` pairs.
fn parse_addresses(header: &str) -> Vec<(String, String)> {
    header
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            if let (Some(lt), Some(gt)) = (part.find('<'), part.find('>')) {
                if gt > lt {
                    let email = part[lt + 1..gt].trim().to_string();
                    let name = part[..lt].trim().trim_matches('"').to_string();
                    return Some((name, email));
                }
            }
            Some((String::new(), part.to_string()))
        })
        .filter(|(_, e)| e.contains('@'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_address_lists() {
        let got = parse_addresses("Ada Lovelace <ada@x.io>, bob@y.io");
        assert_eq!(got, vec![("Ada Lovelace".into(), "ada@x.io".into()), (String::new(), "bob@y.io".into())]);
    }

    #[test]
    fn emits_thread_and_correspondents() {
        let msg = json!({
            "id": "m1", "threadId": "t1", "snippet": "hi",
            "payload": { "headers": [
                { "name": "From", "value": "Ada <ada@x.io>" },
                { "name": "To", "value": "bob@y.io" },
                { "name": "Subject", "value": "Hello" }
            ]}
        });
        let (mut n, mut e, mut seen) = (vec![], vec![], HashSet::new());
        emit_message(&msg, &mut n, &mut e, &mut seen);
        let types: Vec<&str> = e.iter().filter_map(|x| x["type"].as_str()).collect();
        assert!(types.contains(&"IN_THREAD"));
        assert!(types.contains(&"SENT"));
        assert!(types.contains(&"RECEIVED"));
        assert!(n.iter().any(|x| x["id"] == "gmail:thread:t1"));
    }
}
