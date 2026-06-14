//! Short-TTL result cache for the unified data-search arm (S1.7).
//!
//! Under fan-out (`many clients, repeated queries`) the same `(tenant, query,
//! scope, page)` is asked over and over — dashboards polling, retries, identical
//! agent calls. Recomputing the full multi-source RRF (+ optional rerank) each
//! time is wasteful. This caches the fused rows for a short TTL so a repeat is a
//! map lookup.
//!
//! Correctness is bounded by the TTL, not by snapshot tracking: a fresh ingest
//! becomes visible within `KYMA_SEARCH_CACHE_TTL_MS` (default 3000; `0` disables
//! the cache entirely). Keep it small — this trades a few seconds of staleness
//! for a large drop in repeated-query cost. Per-tenant key isolation means one
//! tenant never sees another's cached rows.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Cached rows for one query page: the fused `(source, score, row)` tuples plus
/// the number of sources searched, stamped with insertion time for TTL checks.
struct Entry {
    at: Instant,
    rows: Vec<(String, f64, Value)>,
    sources_searched: usize,
}

/// Max distinct query pages cached at once. On overflow the whole map is cleared
/// (simple + cheap; the TTL is short so churn is fine).
const MAX_ENTRIES: usize = 4096;

fn store() -> &'static Mutex<HashMap<String, Entry>> {
    static S: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// TTL from `KYMA_SEARCH_CACHE_TTL_MS` (default 3000ms). `0` disables the cache.
pub fn ttl() -> Duration {
    let ms = std::env::var("KYMA_SEARCH_CACHE_TTL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3000);
    Duration::from_millis(ms)
}

/// Stable cache key for a data-search page. Pure — same inputs → same key,
/// different tenant/query/scope/page → different key.
pub fn key(
    tenant: &str,
    query: &str,
    scope_repr: &str,
    limit: usize,
    offset: usize,
    time_range_repr: &str,
) -> String {
    // A field-separated string hashed compactly. `\u{1}` can't appear in the
    // textual inputs, so it's an unambiguous separator.
    let raw = format!(
        "{tenant}\u{1}{query}\u{1}{scope_repr}\u{1}{limit}\u{1}{offset}\u{1}{time_range_repr}"
    );
    kyma_core::crypto::content_hash_hex(raw.as_bytes())
}

/// Fetch a fresh cached page, or `None` on miss / expiry / disabled. A returned
/// hit is cloned so the caller owns it.
pub fn get(key: &str) -> Option<(Vec<(String, f64, Value)>, usize)> {
    let ttl = ttl();
    if ttl.is_zero() {
        return None;
    }
    let guard = store().lock().ok()?;
    let e = guard.get(key)?;
    if e.at.elapsed() > ttl {
        return None; // expired (lazily replaced on next put)
    }
    Some((e.rows.clone(), e.sources_searched))
}

/// Insert a page. No-op when the cache is disabled. Clears the map first if it
/// would exceed [`MAX_ENTRIES`].
pub fn put(key: String, rows: Vec<(String, f64, Value)>, sources_searched: usize) {
    if ttl().is_zero() {
        return;
    }
    if let Ok(mut guard) = store().lock() {
        if guard.len() >= MAX_ENTRIES {
            guard.clear();
        }
        guard.insert(
            key,
            Entry {
                at: Instant::now(),
                rows,
                sources_searched,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_stable_and_discriminating() {
        let a = key("t1", "q", "All", 10, 0, "none");
        assert_eq!(a, key("t1", "q", "All", 10, 0, "none"), "stable");
        assert_ne!(a, key("t2", "q", "All", 10, 0, "none"), "tenant differs");
        assert_ne!(a, key("t1", "q2", "All", 10, 0, "none"), "query differs");
        assert_ne!(a, key("t1", "q", "All", 10, 10, "none"), "offset differs");
        assert_ne!(a, key("t1", "q", "Sources", 10, 0, "none"), "scope differs");
    }

    #[test]
    fn get_put_roundtrip_and_disable() {
        std::env::set_var("KYMA_SEARCH_CACHE_TTL_MS", "3000");
        let k = key("tenantX", "hello", "All", 5, 0, "none");
        assert!(get(&k).is_none(), "cold miss");
        let rows = vec![("src".to_string(), 1.0, serde_json::json!({"id": "a"}))];
        put(k.clone(), rows.clone(), 2);
        let (got, srcs) = get(&k).expect("hit");
        assert_eq!(got.len(), 1);
        assert_eq!(srcs, 2);

        // TTL=0 disables both get and put.
        std::env::set_var("KYMA_SEARCH_CACHE_TTL_MS", "0");
        assert!(get(&k).is_none(), "disabled → miss");
        put(k.clone(), rows, 2);
        std::env::set_var("KYMA_SEARCH_CACHE_TTL_MS", "3000");
        // (no assertion on the disabled put beyond it not panicking)
        std::env::remove_var("KYMA_SEARCH_CACHE_TTL_MS");
    }
}
