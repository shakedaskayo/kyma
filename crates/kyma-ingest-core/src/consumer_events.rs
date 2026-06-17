//! In-process "consumer activity" notifications. Published by the memory read
//! (recall/search) and write (remember) paths whenever a client touches
//! memories; consumed by the live consumers WebSocket (`GET /v1/consumers/live`)
//! that drives the graph explorer's realtime overlay.
//!
//! Lossy by design (bounded broadcast), exactly like [`super::events::IngestEvents`]:
//! no subscribers is fine, and a slow subscriber drops events rather than
//! blocking the hot path. The UI re-renders from its own ring buffer, so a
//! dropped tick is cosmetic.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// What a consumer did to memory. Agent-agnostic. `recall`/`search`/`remember`
/// are the v1 emit set; `graph_query`/`ask` are reserved so the wire enum stays
/// forward-stable as more emit points are added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerAction {
    Recall,
    Search,
    Remember,
    GraphQuery,
    Ask,
}

/// A single read/write of memory by a consumer (coding agent, MCP client, the
/// Ask-Kyma agent, …). Serialized verbatim into the live WS frames the frontend
/// renders, so field names are the wire contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerActivity {
    /// Stable per-consumer key for grouping/dedup in the UI — `"{kind}:{subject}"`
    /// (or `"{kind}:anon"` when the subject is unknown). NOT a session id, which
    /// isn't available at every emit point.
    pub consumer_id: String,
    /// Agent kind / source: the MCP client name when known (`"claude-code"`,
    /// `"cursor"`, `"windsurf"`, `"codex"`, …), else the transport source
    /// (`"local-serve"`, `"server"`, `"mcp-stdio"`, `"kyma"`). The frontend maps
    /// this to an icon + colour with a sensible default for unknown kinds.
    pub kind: String,
    /// Human display label (best-effort: the subject, else the client/kind).
    pub label: String,
    /// Auth subject when in scope (`Some` at the HTTP recall handler; `None` on
    /// the tool write path, which runs below the auth layer).
    pub subject: Option<String>,
    /// Tenant id as a string — used to scope the live stream per principal.
    pub tenant: String,
    pub action: ConsumerAction,
    /// Composite-free node ids touched: `memory:<uuid>` for recall results and
    /// the written node, plus linked-resource node ids on recall. Empty when none
    /// (e.g. a recall that returned nothing, or a backfilled row).
    pub node_ids: Vec<String>,
    /// Namespaces / realms touched, deduped. May be empty.
    pub namespaces: Vec<String>,
    /// Truncated query text for recall/search (already capped upstream). `None`
    /// for writes.
    pub query_preview: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub ts: i64,
}

/// In-process broadcast bus for [`ConsumerActivity`]. Cloneable handle around a
/// [`tokio::sync::broadcast`] sender; mirrors [`super::events::IngestEvents`].
#[derive(Debug, Clone)]
pub struct ConsumerEvents {
    tx: broadcast::Sender<ConsumerActivity>,
}

impl ConsumerEvents {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }
    /// Fire-and-forget: no subscribers is fine; lagging subscribers drop.
    pub fn publish(&self, ev: ConsumerActivity) {
        let _ = self.tx.send(ev);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<ConsumerActivity> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(action: ConsumerAction) -> ConsumerActivity {
        ConsumerActivity {
            consumer_id: "claude-code:shaked".into(),
            kind: "claude-code".into(),
            label: "shaked".into(),
            subject: Some("shaked".into()),
            tenant: "default".into(),
            action,
            node_ids: vec!["memory:abc".into()],
            namespaces: vec!["kyma".into()],
            query_preview: Some("auth flow".into()),
            ts: 1_700_000_000_000,
        }
    }

    #[tokio::test]
    async fn publish_is_lossy_nonblocking_and_received() {
        let bus = ConsumerEvents::new(8);
        let mut sub = bus.subscribe();
        bus.publish(sample(ConsumerAction::Recall));
        let ev = sub.try_recv().expect("event delivered");
        assert_eq!(ev.action, ConsumerAction::Recall);
        assert_eq!(ev.node_ids, vec!["memory:abc".to_string()]);
    }

    #[tokio::test]
    async fn publish_without_subscribers_does_not_error() {
        let bus = ConsumerEvents::new(8);
        bus.publish(sample(ConsumerAction::Remember)); // must not panic
    }

    #[test]
    fn action_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConsumerAction::Remember).unwrap(),
            "\"remember\""
        );
        assert_eq!(
            serde_json::to_string(&ConsumerAction::GraphQuery).unwrap(),
            "\"graph_query\""
        );
    }

    #[test]
    fn activity_round_trips() {
        let ev = sample(ConsumerAction::Search);
        let json = serde_json::to_string(&ev).unwrap();
        let back: ConsumerActivity = serde_json::from_str(&json).unwrap();
        assert_eq!(back.action, ConsumerAction::Search);
        assert_eq!(back.consumer_id, ev.consumer_id);
        assert_eq!(back.ts, ev.ts);
    }
}
