//! Pure usage-based reinforcement scoring — no I/O (M8.1).
//!
//! Usage counters live in a small Postgres side table (`memory_usage_stats`,
//! see `pensieve-server::agent::memory_usage_store`), not on `memory_nodes` rows:
//! every `memory_nodes` mutation re-embeds and appends a whole new versioned
//! row (see [`crate::MemoryWriter::save_as`]), so bumping a counter on every
//! recall hit there would mean paying a full embedding rewrite per hit. This
//! module only turns whatever counters the caller already fetched into a
//! decayed salience score — the same shape as `memory_retrieve`'s existing
//! `recency_decay`, just anchored on usage instead of `created_at`.

use serde::{Deserialize, Serialize};

/// Usage counters for one memory, as stored in `memory_usage_stats`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageStats {
    pub hit_count: i64,
    pub reinforced_count: i64,
    pub miss_count: i64,
    /// RFC3339, set the first time a recall ever returns this memory.
    pub last_surfaced_at: Option<String>,
    /// RFC3339, set on the most recent explicit `reinforce_memory` verdict
    /// (helpful or not — either way it's the freshest signal about standing).
    pub last_reinforced_at: Option<String>,
}

/// Ebbinghaus-style decayed salience in `[0, 1]`: recently-and-often-confirmed
/// memories decay slowly; disputed or long-unconfirmed ones decay fast.
/// Anchored on whichever is more recent of `last_reinforced_at`/
/// `last_surfaced_at`, falling back to `created_at` for a never-surfaced
/// memory — no usage signal yet, so it starts fully "fresh" rather than being
/// punished by a blend term nobody has had a chance to reinforce.
pub fn decayed_salience(
    stats: &UsageStats,
    created_at: &str,
    now: chrono::DateTime<chrono::Utc>,
    half_life_days: f64,
    hit_weight: f64,
    miss_penalty: f64,
) -> f64 {
    let anchor = stats
        .last_reinforced_at
        .as_deref()
        .or(stats.last_surfaced_at.as_deref())
        .unwrap_or(created_at);
    let Ok(anchor_dt) = chrono::DateTime::parse_from_rfc3339(anchor) else {
        return 1.0;
    };
    let age_days = (now - anchor_dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86_400.0;
    if age_days <= 0.0 {
        return 1.0;
    }
    let hl = if half_life_days > 0.0 {
        half_life_days
    } else {
        30.0
    };
    // Reinforcement narrows/widens the effective half-life: more confirmed
    // hits ⇒ slower decay (the memory survives longer at the top of recall);
    // more misses ⇒ faster decay. Clamped so neither signal can invert decay
    // entirely or freeze it.
    let strength = ((1.0 + stats.reinforced_count as f64).ln()) * hit_weight
        - ((1.0 + stats.miss_count as f64).ln()) * miss_penalty;
    let effective_hl = (hl * (1.0 + strength)).clamp(hl * 0.25, hl * 4.0);
    (-std::f64::consts::LN_2 * age_days / effective_hl)
        .exp()
        .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn stats(reinforced: i64, miss: i64, anchor: Option<String>) -> UsageStats {
        UsageStats {
            hit_count: 0,
            reinforced_count: reinforced,
            miss_count: miss,
            last_surfaced_at: anchor,
            last_reinforced_at: None,
        }
    }

    #[test]
    fn never_surfaced_falls_back_to_created_at_fresh() {
        let now = Utc::now();
        let created = now.to_rfc3339();
        let s = stats(0, 0, None);
        assert_eq!(decayed_salience(&s, &created, now, 30.0, 0.3, 0.3), 1.0);
    }

    #[test]
    fn reinforced_memory_decays_slower_than_baseline() {
        let now = Utc::now();
        let old = (now - Duration::days(30)).to_rfc3339();
        let created = (now - Duration::days(60)).to_rfc3339();
        let baseline = stats(0, 0, Some(old.clone()));
        let reinforced = stats(5, 0, Some(old));
        let b = decayed_salience(&baseline, &created, now, 30.0, 0.3, 0.3);
        let r = decayed_salience(&reinforced, &created, now, 30.0, 0.3, 0.3);
        assert!(
            r > b,
            "reinforced ({r}) should decay slower than baseline ({b})"
        );
    }

    #[test]
    fn missed_memory_decays_faster_than_baseline() {
        let now = Utc::now();
        let old = (now - Duration::days(30)).to_rfc3339();
        let created = (now - Duration::days(60)).to_rfc3339();
        let baseline = stats(0, 0, Some(old.clone()));
        let missed = stats(0, 5, Some(old));
        let b = decayed_salience(&baseline, &created, now, 30.0, 0.3, 0.3);
        let m = decayed_salience(&missed, &created, now, 30.0, 0.3, 0.3);
        assert!(
            m < b,
            "missed ({m}) should decay faster than baseline ({b})"
        );
    }

    #[test]
    fn unparseable_anchor_defaults_fresh() {
        let now = Utc::now();
        let s = stats(0, 0, Some("not-a-date".into()));
        assert_eq!(decayed_salience(&s, "also-bad", now, 30.0, 0.3, 0.3), 1.0);
    }

    #[test]
    fn recent_reinforcement_anchor_beats_stale_surfaced_anchor() {
        // last_reinforced_at is fresher than last_surfaced_at ⇒ decay uses it.
        let now = Utc::now();
        let stale_surface = (now - Duration::days(90)).to_rfc3339();
        let fresh_reinforce = (now - Duration::days(1)).to_rfc3339();
        let s = UsageStats {
            hit_count: 3,
            reinforced_count: 1,
            miss_count: 0,
            last_surfaced_at: Some(stale_surface),
            last_reinforced_at: Some(fresh_reinforce),
        };
        let score = decayed_salience(&s, "2020-01-01T00:00:00Z", now, 30.0, 0.3, 0.3);
        assert!(
            score > 0.9,
            "fresh reinforcement anchor should barely decay, got {score}"
        );
    }
}
