//! Watcher registry — file watchers (filedrop, cc-sync) register themselves
//! with node + identity provenance and heartbeat each poll cycle. Read-side
//! the UI shows who is feeding the graph from where. Best-effort by design:
//! a registry failure must never break ingestion — `heartbeat` logs and
//! continues instead of returning an error.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Stats from a single watcher scan cycle, stored as `last_scan_jsonb`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanStats {
    pub seen: u64,
    pub processed: u64,
    pub errors: u64,
    pub duration_ms: u64,
    /// RFC 3339 timestamp of the scan.
    pub at: String,
    /// Watcher-specific extras (cc-sync packs per-realm report rollups here).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

/// Read-side row for the API (Task 10): one registered watcher with
/// provenance and a SQL-computed staleness flag.
#[derive(Debug, Serialize)]
pub struct WatcherRow {
    pub id: Uuid,
    pub kind: String,
    pub node_host: String,
    pub node_id: String,
    pub identity: String,
    pub config: serde_json::Value,
    pub started_at: String,
    pub last_heartbeat_at: String,
    pub last_scan: Option<serde_json::Value>,
    /// Heartbeat older than 3x the poll interval (config `poll_secs`,
    /// default 30).
    pub stale: bool,
}

/// Handle a watcher holds after registration; clones share the same row.
#[derive(Clone)]
pub struct WatcherRegistry {
    pool: PgPool,
    id: Uuid,
}

/// `(hostname, node_id, os user)` for registration. `KYMA_NODE_ID` overrides
/// the node id (defaults to the hostname).
pub fn node_identity() -> (String, String, String) {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());
    let node_id = std::env::var("KYMA_NODE_ID").unwrap_or_else(|_| host.clone());
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    (host, node_id, user)
}

fn config_hash(config: &serde_json::Value) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let digest = Sha256::digest(config.to_string().as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for &b in &digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

impl WatcherRegistry {
    /// The registry row id for this watcher.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Register (or re-register) a watcher. Identical `(kind, node_id,
    /// config)` upserts onto the same row, refreshing `started_at` and the
    /// heartbeat — a watcher restart reclaims its row instead of leaving a
    /// stale duplicate behind.
    pub async fn register(
        pool: PgPool,
        kind: &str,
        node_host: &str,
        node_id: &str,
        identity: &str,
        config: serde_json::Value,
    ) -> Result<Self, sqlx::Error> {
        let hash = config_hash(&config);
        let (id,) = sqlx::query_as::<_, (Uuid,)>(
            "INSERT INTO data_source_watchers
                 (kind, node_host, node_id, identity, config_jsonb, config_hash)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (kind, node_id, config_hash) DO UPDATE SET
                 node_host = EXCLUDED.node_host,
                 identity = EXCLUDED.identity,
                 config_jsonb = EXCLUDED.config_jsonb,
                 started_at = now(),
                 last_heartbeat_at = now()
             RETURNING id",
        )
        .bind(kind)
        .bind(node_host)
        .bind(node_id)
        .bind(identity)
        .bind(&config)
        .bind(&hash)
        .fetch_one(&pool)
        .await?;
        Ok(Self { pool, id })
    }

    /// Record a heartbeat (and optionally the latest scan stats).
    /// Best-effort: registry failures must never break ingestion, so errors
    /// are logged and swallowed.
    pub async fn heartbeat(&self, scan: Option<&ScanStats>) {
        let scan_json = scan.and_then(|s| match serde_json::to_value(s) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "watcher registry: failed to serialize scan stats");
                None
            }
        });
        let res = sqlx::query(
            "UPDATE data_source_watchers
             SET last_heartbeat_at = now(),
                 last_scan_jsonb = COALESCE($2, last_scan_jsonb)
             WHERE id = $1",
        )
        .bind(self.id)
        .bind(scan_json)
        .execute(&self.pool)
        .await;
        if let Err(e) = res {
            tracing::warn!(watcher_id = %self.id, error = %e, "watcher registry: heartbeat failed");
        }
    }

    /// List registered watchers, pruning rows silent for more than 24 hours.
    /// `stale` is computed in SQL: heartbeat older than 3x the configured
    /// poll interval (`poll_secs`, default 30s).
    pub async fn list(pool: &PgPool) -> Result<Vec<WatcherRow>, sqlx::Error> {
        sqlx::query(
            "DELETE FROM data_source_watchers
             WHERE last_heartbeat_at < now() - interval '24 hours'",
        )
        .execute(pool)
        .await?;

        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                String,
                String,
                String,
                serde_json::Value,
                DateTime<Utc>,
                DateTime<Utc>,
                Option<serde_json::Value>,
                bool,
            ),
        >(
            "SELECT id, kind, node_host, node_id, identity, config_jsonb,
                    started_at, last_heartbeat_at, last_scan_jsonb,
                    last_heartbeat_at < now() - make_interval(secs =>
                        3 * COALESCE((config_jsonb->>'poll_secs')::float8, 30.0)) AS stale
             FROM data_source_watchers
             ORDER BY kind, node_host, started_at",
        )
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    kind,
                    node_host,
                    node_id,
                    identity,
                    config,
                    started_at,
                    last_heartbeat_at,
                    last_scan,
                    stale,
                )| WatcherRow {
                    id,
                    kind,
                    node_host,
                    node_id,
                    identity,
                    config,
                    started_at: started_at.to_rfc3339(),
                    last_heartbeat_at: last_heartbeat_at.to_rfc3339(),
                    last_scan,
                    stale,
                },
            )
            .collect())
    }
}
