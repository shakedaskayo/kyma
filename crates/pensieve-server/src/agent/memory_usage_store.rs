//! Persistence for per-memory usage/reinforcement counters (M8.1).
//!
//! Two backends behind one enum, mirroring [`super::memory_queue_store`]:
//!   - **Pg** — the `memory_usage_stats` table (server / worker-fabric mode),
//!     tenant-scoped on every query.
//!   - **Local** — a JSON file next to the local memory-settings file
//!     (`pensieve serve` single-binary mode, no Postgres).
//!
//! Counters live here, not on `memory_nodes` rows, because every mutation to
//! `memory_nodes` re-embeds and appends a whole new versioned row (see
//! [`pensieve_memory::MemoryWriter::save_as`]) — bumping a counter that way on
//! every recall hit would mean paying a full embedding rewrite per hit. This
//! store is dumb (record/fetch); [`pensieve_memory::reinforcement::decayed_salience`]
//! turns the counters into a score.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::Utc;
use pensieve_memory::reinforcement::UsageStats;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;

use pensieve_core::tenant::TenantId;

use super::state::AgentState;

/// One memory surfaced in a recall result — the unit [`UsageStore::record_surfaced`] bumps.
#[derive(Debug, Clone)]
pub struct UsageHit {
    pub memory_id: String,
    pub realm: String,
}

/// The explicit `reinforce_memory` verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Helpful,
    NotHelpful,
}

impl Outcome {
    pub fn parse(s: &str) -> Option<Outcome> {
        match s {
            "helpful" => Some(Outcome::Helpful),
            "not_helpful" => Some(Outcome::NotHelpful),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LocalRecord {
    realm: String,
    stats: UsageStats,
}

/// The two backends. Cheap to construct per request via [`super::tools::SharedToolCtx::usage_store`].
pub enum UsageStore {
    Pg { pool: PgPool, tenant: TenantId },
    Local { path: PathBuf },
}

/// One process-wide lock guarding local-file read-modify-write, mirroring
/// `memory_queue_store`'s `LOCAL_LOCK` — there is only ever one local
/// deployment (single tenant, one file).
static LOCAL_LOCK: Mutex<()> = Mutex::const_new(());

impl UsageStore {
    /// Pick the backend for this server, mirroring `QueueStore::from_state`.
    pub fn from_state(state: &AgentState) -> Option<UsageStore> {
        if let Some(pool) = state.pool.as_ref() {
            return Some(UsageStore::Pg {
                pool: pool.clone(),
                tenant: state.tenant,
            });
        }
        let p = state.memory_settings_path.as_ref()?;
        let path = p
            .parent()
            .map(|d| d.join("memory-usage-stats.json"))
            .unwrap_or_else(|| PathBuf::from("memory-usage-stats.json"));
        Some(UsageStore::Local { path })
    }

    /// Passive signal: bump `hit_count`/`last_surfaced_at` for every memory a
    /// recall returned. Best-effort — callers run this detached.
    pub async fn record_surfaced(&self, hits: &[UsageHit]) -> anyhow::Result<()> {
        if hits.is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        match self {
            UsageStore::Pg { pool, tenant } => {
                for h in hits {
                    sqlx::query(
                        "INSERT INTO memory_usage_stats \
                           (memory_id, tenant_id, realm, hit_count, last_surfaced_at) \
                         VALUES ($1, $2, $3, 1, $4::timestamptz) \
                         ON CONFLICT (tenant_id, memory_id) DO UPDATE SET \
                           hit_count = memory_usage_stats.hit_count + 1, \
                           last_surfaced_at = $4::timestamptz, \
                           updated_at = now()",
                    )
                    .bind(&h.memory_id)
                    .bind(tenant.as_uuid())
                    .bind(&h.realm)
                    .bind(&now)
                    .execute(pool)
                    .await?;
                }
                Ok(())
            }
            UsageStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                let mut map = read_local(path).await?;
                for h in hits {
                    let rec = map
                        .entry(h.memory_id.clone())
                        .or_insert_with(|| LocalRecord {
                            realm: h.realm.clone(),
                            stats: UsageStats::default(),
                        });
                    rec.stats.hit_count += 1;
                    rec.stats.last_surfaced_at = Some(now.clone());
                }
                write_local(path, &map).await
            }
        }
    }

    /// Explicit signal: record a `helpful`/`not_helpful` verdict for one
    /// memory. Returns the updated counters.
    pub async fn record_feedback(
        &self,
        memory_id: &str,
        realm: &str,
        outcome: Outcome,
    ) -> anyhow::Result<UsageStats> {
        let now = Utc::now().to_rfc3339();
        let (reinforced_delta, miss_delta): (i64, i64) = match outcome {
            Outcome::Helpful => (1, 0),
            Outcome::NotHelpful => (0, 1),
        };
        match self {
            UsageStore::Pg { pool, tenant } => {
                let row = sqlx::query(
                    "INSERT INTO memory_usage_stats \
                       (memory_id, tenant_id, realm, reinforced_count, miss_count, last_reinforced_at) \
                     VALUES ($1, $2, $3, $4, $5, $6::timestamptz) \
                     ON CONFLICT (tenant_id, memory_id) DO UPDATE SET \
                       reinforced_count = memory_usage_stats.reinforced_count + $4, \
                       miss_count = memory_usage_stats.miss_count + $5, \
                       last_reinforced_at = $6::timestamptz, \
                       updated_at = now() \
                     RETURNING hit_count, reinforced_count, miss_count, last_surfaced_at, last_reinforced_at",
                )
                .bind(memory_id)
                .bind(tenant.as_uuid())
                .bind(realm)
                .bind(reinforced_delta)
                .bind(miss_delta)
                .bind(&now)
                .fetch_one(pool)
                .await?;
                Ok(row_to_stats(&row))
            }
            UsageStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                let mut map = read_local(path).await?;
                let rec = map
                    .entry(memory_id.to_string())
                    .or_insert_with(|| LocalRecord {
                        realm: realm.to_string(),
                        stats: UsageStats::default(),
                    });
                rec.stats.reinforced_count += reinforced_delta;
                rec.stats.miss_count += miss_delta;
                rec.stats.last_reinforced_at = Some(now);
                let out = rec.stats.clone();
                write_local(path, &map).await?;
                Ok(out)
            }
        }
    }

    /// Batch-fetch counters for a candidate set (recall's blend term).
    pub async fn get_many(
        &self,
        memory_ids: &[String],
    ) -> anyhow::Result<HashMap<String, UsageStats>> {
        if memory_ids.is_empty() {
            return Ok(HashMap::new());
        }
        match self {
            UsageStore::Pg { pool, tenant } => {
                let rows = sqlx::query(
                    "SELECT memory_id, hit_count, reinforced_count, miss_count, \
                            last_surfaced_at, last_reinforced_at \
                     FROM memory_usage_stats WHERE tenant_id = $1 AND memory_id = ANY($2)",
                )
                .bind(tenant.as_uuid())
                .bind(memory_ids)
                .fetch_all(pool)
                .await?;
                Ok(rows
                    .into_iter()
                    .map(|r| (r.get::<String, _>("memory_id"), row_to_stats(&r)))
                    .collect())
            }
            UsageStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                let map = read_local(path).await?;
                Ok(memory_ids
                    .iter()
                    .filter_map(|id| map.get(id).map(|r| (id.clone(), r.stats.clone())))
                    .collect())
            }
        }
    }

    /// Memories that have been surfaced but never explicitly judged — the
    /// dreaming backstop's worklist (`list_memory_usage` MCP tool).
    pub async fn list_unreinforced(
        &self,
        realm: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, UsageStats)>> {
        let limit = limit.clamp(1, 500) as i64;
        match self {
            UsageStore::Pg { pool, tenant } => {
                let rows = sqlx::query(
                    "SELECT memory_id, hit_count, reinforced_count, miss_count, \
                            last_surfaced_at, last_reinforced_at \
                     FROM memory_usage_stats \
                     WHERE tenant_id = $1 AND hit_count > 0 \
                       AND reinforced_count = 0 AND miss_count = 0 \
                       AND ($2::text IS NULL OR realm = $2) \
                     ORDER BY hit_count DESC, last_surfaced_at DESC LIMIT $3",
                )
                .bind(tenant.as_uuid())
                .bind(realm)
                .bind(limit)
                .fetch_all(pool)
                .await?;
                Ok(rows
                    .into_iter()
                    .map(|r| (r.get::<String, _>("memory_id"), row_to_stats(&r)))
                    .collect())
            }
            UsageStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                let map = read_local(path).await?;
                let mut out: Vec<(String, UsageStats)> = map
                    .into_iter()
                    .filter(|(_, r)| {
                        r.stats.hit_count > 0
                            && r.stats.reinforced_count == 0
                            && r.stats.miss_count == 0
                            && realm.map_or(true, |rl| r.realm == rl)
                    })
                    .map(|(id, r)| (id, r.stats))
                    .collect();
                out.sort_by(|a, b| b.1.hit_count.cmp(&a.1.hit_count));
                out.truncate(limit as usize);
                Ok(out)
            }
        }
    }
}

fn row_to_stats(r: &sqlx::postgres::PgRow) -> UsageStats {
    UsageStats {
        hit_count: r.get("hit_count"),
        reinforced_count: r.get("reinforced_count"),
        miss_count: r.get("miss_count"),
        last_surfaced_at: r
            .get::<Option<chrono::DateTime<Utc>>, _>("last_surfaced_at")
            .map(|t| t.to_rfc3339()),
        last_reinforced_at: r
            .get::<Option<chrono::DateTime<Utc>>, _>("last_reinforced_at")
            .map(|t| t.to_rfc3339()),
    }
}

async fn read_local(path: &PathBuf) -> anyhow::Result<HashMap<String, LocalRecord>> {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) if !raw.trim().is_empty() => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        _ => Ok(HashMap::new()),
    }
}

async fn write_local(path: &PathBuf, map: &HashMap<String, LocalRecord>) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(path, serde_json::to_string_pretty(map)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &std::path::Path) -> UsageStore {
        UsageStore::Local {
            path: dir.join("nested").join("usage.json"),
        }
    }

    #[tokio::test]
    async fn record_surfaced_then_feedback_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());

        s.record_surfaced(&[UsageHit {
            memory_id: "memory:a".into(),
            realm: "r1".into(),
        }])
        .await
        .unwrap();
        s.record_surfaced(&[UsageHit {
            memory_id: "memory:a".into(),
            realm: "r1".into(),
        }])
        .await
        .unwrap();

        let stats = s
            .record_feedback("memory:a", "r1", Outcome::Helpful)
            .await
            .unwrap();
        assert_eq!(stats.hit_count, 2);
        assert_eq!(stats.reinforced_count, 1);
        assert_eq!(stats.miss_count, 0);
        assert!(stats.last_reinforced_at.is_some());

        let fetched = s.get_many(&["memory:a".to_string()]).await.unwrap();
        assert_eq!(fetched["memory:a"].hit_count, 2);
        assert_eq!(fetched["memory:a"].reinforced_count, 1);
    }

    #[tokio::test]
    async fn not_helpful_bumps_miss_count() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let stats = s
            .record_feedback("memory:a", "r1", Outcome::NotHelpful)
            .await
            .unwrap();
        assert_eq!(stats.miss_count, 1);
        assert_eq!(stats.reinforced_count, 0);
    }

    #[tokio::test]
    async fn unreinforced_list_excludes_judged_memories() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.record_surfaced(&[
            UsageHit {
                memory_id: "memory:a".into(),
                realm: "r1".into(),
            },
            UsageHit {
                memory_id: "memory:b".into(),
                realm: "r1".into(),
            },
        ])
        .await
        .unwrap();
        s.record_feedback("memory:b", "r1", Outcome::Helpful)
            .await
            .unwrap();

        let list = s.list_unreinforced(Some("r1"), 10).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "memory:a");
    }

    #[tokio::test]
    async fn get_many_empty_ids_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        assert!(s.get_many(&[]).await.unwrap().is_empty());
    }

    #[test]
    fn outcome_parse_rejects_unknown_strings() {
        assert_eq!(Outcome::parse("helpful"), Some(Outcome::Helpful));
        assert_eq!(Outcome::parse("not_helpful"), Some(Outcome::NotHelpful));
        assert_eq!(Outcome::parse("maybe"), None);
    }
}
