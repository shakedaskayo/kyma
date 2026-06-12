//! Slack data source — channels, messages, users, and reply threads as a graph.
//!
//! Authenticates with a stored OAuth2 token. Slack's Web API returns HTTP 200
//! with `{"ok": false, ...}` on logical errors, so every call is checked via
//! [`slack_ok`]. Incremental per channel via the `oldest` message `ts` cursor.
//! Ids are prefixed `slack:`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::catalog::CatalogEntry;
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

const API: &str = "https://slack.com/api";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackConfig {
    #[serde(default)]
    credential_id: Option<Uuid>,
    /// Optional: restrict to these channel ids. Empty = public channels the
    /// token can list.
    #[serde(default)]
    channels: Vec<String>,
    /// Max channels processed per tick (one history page each).
    #[serde(default = "default_max_channels")]
    max_channels_per_tick: usize,
}
fn default_max_channels() -> usize {
    20
}

pub struct SlackDataSource;

#[async_trait]
impl DataSource for SlackDataSource {
    fn type_id(&self) -> &'static str {
        "slack"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "slack".into(),
            label: "Slack".into(),
            category: "knowledge".into(),
            description: "Channels, messages, threads, and the people graph from a Slack \
                          workspace."
                .into(),
            brand: "slack".into(),
            auth_mode: "oauth".into(),
            status: "available".into(),
            drive_model: "periodic".into(),
            default_schedule_ms: 10 * 60_000,
            fields: Vec::new(),
            resource: None,
            default_target_table: Some("slack_nodes".into()),
            config_defaults: None,
            graph_name: Some("slack".into()),
            accepted_credential_kinds: vec!["oauth2".into()],
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let parsed: SlackConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        if parsed.credential_id.is_none() {
            return Err(ConfigError("a Slack OAuth credential is required".into()));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &Value,
        cursor: Option<&Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let config: SlackConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;
        let cid = config
            .credential_id
            .ok_or_else(|| DataSourceError::Config("credential_id required".into()))?;
        let token = crate::oauth::valid_access_token(ctx, cid).await?;

        let mut nodes: Vec<Value> = Vec::new();
        let mut edges: Vec<Value> = Vec::new();
        let mut user_seen: HashSet<String> = HashSet::new();
        // Per-channel latest ts (incremental floor) from the cursor.
        let mut ts_cursor: HashMap<String, String> = cursor
            .and_then(|c| c.get("ts"))
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        // 1. Users directory (one page).
        let users = slack_get(&ctx.http, &token, "users.list", &[("limit", "200")]).await?;
        for u in users.get("members").and_then(Value::as_array).into_iter().flatten() {
            if let Some(uid) = u.get("id").and_then(Value::as_str) {
                emit_user(uid, u, &mut nodes, &mut user_seen);
            }
        }

        // 2. Channels — configured set or list public+private.
        let channel_ids: Vec<String> = if config.channels.is_empty() {
            let resp = slack_get(
                &ctx.http,
                &token,
                "conversations.list",
                &[("types", "public_channel,private_channel"), ("limit", "200")],
            )
            .await?;
            resp.get("channels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    let id = c.get("id").and_then(Value::as_str)?.to_string();
                    emit_channel(c, &mut nodes);
                    Some(id)
                })
                .collect()
        } else {
            config.channels.clone()
        };

        // 3. History per channel (bounded), incremental from the stored ts.
        for chan in channel_ids.iter().take(config.max_channels_per_tick) {
            let oldest = ts_cursor.get(chan).cloned().unwrap_or_default();
            let resp = slack_get(
                &ctx.http,
                &token,
                "conversations.history",
                &[("channel", chan), ("limit", "100"), ("oldest", &oldest)],
            )
            .await?;
            let mut newest = oldest.clone();
            for msg in resp.get("messages").and_then(Value::as_array).into_iter().flatten() {
                let ts = msg.get("ts").and_then(Value::as_str).unwrap_or("");
                if ts > newest.as_str() {
                    newest = ts.to_string();
                }
                emit_message(chan, msg, &mut nodes, &mut edges);
            }
            if !newest.is_empty() {
                ts_cursor.insert(chan.clone(), newest);
            }
        }

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: Some(json!({ "ts": ts_cursor })),
            tables: vec![
                TableRows { table: "slack_nodes".into(), rows: nodes },
                TableRows { table: "slack_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "slack".into(),
                node_table: "slack_nodes".into(),
                edge_table: "slack_edges".into(),
            }),
        })
    }
}

/// Slack Web API GET with `ok`-envelope checking. `ratelimited` → Transient.
async fn slack_get(
    http: &reqwest::Client,
    token: &str,
    method: &str,
    params: &[(&str, &str)],
) -> Result<Value, DataSourceError> {
    let mut url = reqwest::Url::parse(&format!("{API}/{method}"))
        .map_err(|e| DataSourceError::Permanent(format!("url: {e}")))?;
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    let resp = crate::oauth_http::get_json(http, url.as_str(), token, &[]).await?;
    if resp.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(resp);
    }
    let err = resp.get("error").and_then(Value::as_str).unwrap_or("unknown");
    if err == "ratelimited" || err == "rate_limited" {
        Err(DataSourceError::Transient(format!("slack {method}: {err}")))
    } else {
        Err(DataSourceError::Permanent(format!("slack {method}: {err}")))
    }
}

fn emit_user(uid: &str, u: &Value, nodes: &mut Vec<Value>, seen: &mut HashSet<String>) {
    let user_id = format!("slack:user:{uid}");
    if seen.insert(user_id.clone()) {
        let name = u
            .get("real_name")
            .and_then(Value::as_str)
            .or_else(|| u.get("name").and_then(Value::as_str))
            .unwrap_or(uid);
        nodes.push(json!({
            "id": user_id, "labels": ["User"], "name": name, "vendor": "slack",
        }));
    }
}

fn emit_channel(c: &Value, nodes: &mut Vec<Value>) {
    if let Some(id) = c.get("id").and_then(Value::as_str) {
        nodes.push(json!({
            "id": format!("slack:chan:{id}"),
            "labels": ["Channel"],
            "name": c.get("name").and_then(Value::as_str).unwrap_or(id),
            "is_private": c.get("is_private").cloned().unwrap_or(Value::Null),
            "topic": c.get("topic").and_then(|t| t.get("value")).cloned().unwrap_or(Value::Null),
            "vendor": "slack",
        }));
    }
}

fn emit_message(chan: &str, msg: &Value, nodes: &mut Vec<Value>, edges: &mut Vec<Value>) {
    let ts = msg.get("ts").and_then(Value::as_str).unwrap_or("");
    if ts.is_empty() {
        return;
    }
    let chan_id = format!("slack:chan:{chan}");
    let msg_id = format!("slack:msg:{chan}:{ts}");
    let text = msg.get("text").and_then(Value::as_str).unwrap_or("");
    let name: String = text.chars().take(80).collect();
    nodes.push(json!({
        "id": msg_id, "labels": ["Message"], "name": name,
        "ts": ts, "reply_count": msg.get("reply_count").cloned().unwrap_or(Value::Null),
        "vendor": "slack",
    }));
    edges.push(json!({
        "id": format!("e:posted_in:{msg_id}::{chan_id}"),
        "src": msg_id, "dst": chan_id, "type": "POSTED_IN",
    }));
    if let Some(u) = msg.get("user").and_then(Value::as_str) {
        let user_id = format!("slack:user:{u}");
        edges.push(json!({
            "id": format!("e:sent_by:{msg_id}::{user_id}"),
            "src": msg_id, "dst": user_id, "type": "SENT_BY",
        }));
    }
    // thread reply
    if let Some(parent) = msg.get("thread_ts").and_then(Value::as_str) {
        if parent != ts {
            let parent_id = format!("slack:msg:{chan}:{parent}");
            edges.push(json!({
                "id": format!("e:reply_to:{msg_id}::{parent_id}"),
                "src": msg_id, "dst": parent_id, "type": "REPLY_TO",
            }));
        }
    }
    // mentions: <@U0123>
    for uid in mentions(text) {
        let user_id = format!("slack:user:{uid}");
        edges.push(json!({
            "id": format!("e:mentions:{msg_id}::{user_id}"),
            "src": msg_id, "dst": user_id, "type": "MENTIONS",
        }));
    }
}

/// Extract user ids from Slack `<@U…>` mention tokens.
fn mentions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'>' && bytes[j] != b'|' {
                j += 1;
            }
            if j > start {
                out.push(text[start..j].to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mentions() {
        assert_eq!(mentions("hi <@U123> and <@U456|bob>"), vec!["U123", "U456"]);
        assert!(mentions("no mentions here").is_empty());
    }

    #[test]
    fn emits_message_edges() {
        let msg = json!({ "ts": "1.2", "text": "ping <@U9>", "user": "U1", "thread_ts": "0.5" });
        let (mut n, mut e) = (vec![], vec![]);
        emit_message("C1", &msg, &mut n, &mut e);
        let types: Vec<&str> = e.iter().filter_map(|x| x["type"].as_str()).collect();
        assert!(types.contains(&"POSTED_IN"));
        assert!(types.contains(&"SENT_BY"));
        assert!(types.contains(&"REPLY_TO"));
        assert!(types.contains(&"MENTIONS"));
    }
}
