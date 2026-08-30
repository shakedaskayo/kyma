//! In-RAM per-tenant quota cache (S2.6 ops hardening).
//!
//! The admission limiter (`crate::concurrency`) is intentionally synchronous and
//! env-driven — no DB hit on the hot path. To let an operator configure
//! DIFFERENT limits per tenant (the `tenant_quotas` catalog table) without a
//! per-request catalog lookup, this module holds a process-global snapshot of
//! the quota rows, refreshed periodically from the catalog. The limiter reads
//! the snapshot synchronously and uses a tenant's override when present, falling
//! back to the env-global default otherwise.
//!
//! Empty by default: with no `tenant_quotas` rows the cache is empty and every
//! lookup returns `None` ⇒ the env-global limits apply everywhere ⇒ behaviour is
//! byte-identical to the pre-quota engine and to the fresh-install.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use pensieve_core::catalog::{Catalog, TenantQuota};
use pensieve_core::tenant::TenantId;

fn cache() -> &'static RwLock<HashMap<TenantId, TenantQuota>> {
    static C: OnceLock<RwLock<HashMap<TenantId, TenantQuota>>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Replace the cache with the catalog's current quota rows. Called on startup
/// and by the periodic refresh task. Errors are swallowed: a stale cache is
/// safe because quotas only shape admission (429s), never correctness.
pub async fn refresh(catalog: &Arc<dyn Catalog>) {
    if let Ok(rows) = catalog.list_tenant_quotas().await {
        let map: HashMap<TenantId, TenantQuota> =
            rows.into_iter().map(|q| (q.tenant, q)).collect();
        if let Ok(mut g) = cache().write() {
            *g = map;
        }
    }
}

/// Per-tenant query-concurrency override, or `None` when this tenant has no
/// configured quota (the limiter then uses the env-global default).
pub fn query_limit_override(tenant: TenantId) -> Option<u32> {
    cache().read().ok()?.get(&tenant)?.max_query_concurrent
}

/// Per-tenant agent-run-concurrency override, or `None` when unconfigured.
pub fn agent_limit_override(tenant: TenantId) -> Option<u32> {
    cache().read().ok()?.get(&tenant)?.max_agent_concurrent
}

/// Spawn the periodic refresh loop. `PENSIEVE_QUOTA_REFRESH_SECS` (default 30, min 1)
/// sets the interval. Returns the join handle so the caller can abort on
/// shutdown. Cheap when the table is empty (one catalog list per interval).
pub fn spawn_refresh(catalog: Arc<dyn Catalog>) -> tokio::task::JoinHandle<()> {
    let secs = std::env::var("PENSIEVE_QUOTA_REFRESH_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30)
        .max(1);
    tokio::spawn(async move {
        loop {
            refresh(&catalog).await;
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    })
}

#[cfg(any(test, feature = "test-support"))]
/// Test seam: set one tenant's quota directly without a catalog round-trip.
pub fn set_for_test(quota: TenantQuota) {
    if let Ok(mut g) = cache().write() {
        g.insert(quota.tenant, quota);
    }
}

#[cfg(any(test, feature = "test-support"))]
/// Test seam: clear the cache so a test starts from the default (empty) state.
pub fn clear_for_test() {
    if let Ok(mut g) = cache().write() {
        g.clear();
    }
}
