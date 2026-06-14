//! Query concurrency admission control (S2.6 ops hardening).
//!
//! Bounds the number of in-flight `/v1/query` + `/v1/search` requests a node
//! will execute at once. Each query can pin a DataFusion task pool and a query
//! memory budget; without a ceiling, a burst of concurrent heavy queries drives
//! the node into memory pressure and pathological context-switching instead of
//! shedding load. A bounded semaphore turns that overload into a fast,
//! retryable `429 Too Many Requests` with `Retry-After` — backpressure the
//! client can honor — rather than a slow collapse that hurts every request.
//!
//! Admission is **non-blocking**: a request either gets a permit immediately or
//! is rejected. Queuing would convert a saturated node's overload into unbounded
//! latency; rejecting lets a load balancer route elsewhere.
//!
//! Off by default: `KYMA_QUERY_MAX_CONCURRENT` unset or `0` ⇒ unlimited (the
//! permit is a no-op), so existing deployments and the fresh-install are
//! unchanged. `KYMA_QUERY_RETRY_AFTER_SECS` (default 1) sets the advertised
//! retry hint.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kyma_core::tenant::TenantId;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Process-global query semaphore, or `None` when admission control is
/// disabled. Resolved once from the environment on first use.
fn limiter() -> Option<&'static Arc<Semaphore>> {
    static S: OnceLock<Option<Arc<Semaphore>>> = OnceLock::new();
    S.get_or_init(|| {
        let n = std::env::var("KYMA_QUERY_MAX_CONCURRENT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        if n == 0 {
            None
        } else {
            Some(Arc::new(Semaphore::new(n)))
        }
    })
    .as_ref()
}

fn retry_after_secs() -> u64 {
    std::env::var("KYMA_QUERY_RETRY_AFTER_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1)
        .max(1)
}

/// Held for the lifetime of a query; releasing it returns the slot to the pool.
/// `None` when admission control is disabled (the permit is inert).
pub struct QueryPermit(#[allow(dead_code)] Option<OwnedSemaphorePermit>);

/// Try to admit one query. `Ok(permit)` to proceed (hold it until the response
/// is produced); `Err(retry_after_secs)` when the node is at capacity.
pub fn acquire() -> Result<QueryPermit, u64> {
    match limiter() {
        None => Ok(QueryPermit(None)),
        Some(sem) => match Arc::clone(sem).try_acquire_owned() {
            Ok(p) => Ok(QueryPermit(Some(p))),
            Err(_) => Err(retry_after_secs()),
        },
    }
}

/// Process-global cap on concurrent **agent runs** (S2.6). Each run is far
/// heavier than a query — an LLM tool loop with model calls, memory recall, and
/// data-source reads — so a separate, smaller ceiling protects the node from
/// agent-run overload. `None` when disabled.
fn agent_limiter() -> Option<&'static Arc<Semaphore>> {
    static S: OnceLock<Option<Arc<Semaphore>>> = OnceLock::new();
    S.get_or_init(|| {
        let n = std::env::var("KYMA_AGENT_MAX_CONCURRENT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        if n == 0 {
            None
        } else {
            Some(Arc::new(Semaphore::new(n)))
        }
    })
    .as_ref()
}

fn agent_retry_after_secs() -> u64 {
    std::env::var("KYMA_AGENT_RETRY_AFTER_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5)
        .max(1)
}

/// Held for the lifetime of an agent run (move it into the run's spawned task so
/// the slot frees when the run finishes). `None` when the cap is disabled.
pub struct AgentRunPermit(#[allow(dead_code)] Option<OwnedSemaphorePermit>);

/// Try to admit one agent run. `Ok(permit)` to proceed (hold it for the whole
/// run); `Err(retry_after_secs)` when the node is at its agent-run capacity.
/// Off by default (`KYMA_AGENT_MAX_CONCURRENT` unset/0 ⇒ unlimited).
pub fn acquire_agent_run() -> Result<AgentRunPermit, u64> {
    match agent_limiter() {
        None => Ok(AgentRunPermit(None)),
        Some(sem) => match Arc::clone(sem).try_acquire_owned() {
            Ok(p) => Ok(AgentRunPermit(Some(p))),
            Err(_) => Err(agent_retry_after_secs()),
        },
    }
}

/// Per-tenant query concurrency limit, or `None` when disabled. Each tenant
/// gets its OWN semaphore of this size, so one tenant saturating its budget
/// cannot starve another (the S2 "two-tenant quota isolation" goal). Distinct
/// from the process-global [`acquire`] node-wide cap; both can apply.
fn per_tenant_query_limit() -> Option<usize> {
    static N: OnceLock<Option<usize>> = OnceLock::new();
    *N.get_or_init(|| {
        let n = std::env::var("KYMA_QUERY_MAX_CONCURRENT_PER_TENANT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        if n == 0 {
            None
        } else {
            Some(n)
        }
    })
}

fn tenant_query_semaphores() -> &'static Mutex<HashMap<TenantId, Arc<Semaphore>>> {
    static M: OnceLock<Mutex<HashMap<TenantId, Arc<Semaphore>>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Admit one query for `tenant` against that tenant's own concurrency budget.
/// `Ok(permit)` to proceed (hold it for the request); `Err(retry_after_secs)`
/// when this tenant is at capacity. Off by default
/// (`KYMA_QUERY_MAX_CONCURRENT_PER_TENANT` unset/0 ⇒ unlimited).
pub fn acquire_for_tenant(tenant: TenantId) -> Result<QueryPermit, u64> {
    let Some(n) = per_tenant_query_limit() else {
        return Ok(QueryPermit(None));
    };
    let sem = {
        let Ok(mut g) = tenant_query_semaphores().lock() else {
            return Ok(QueryPermit(None)); // never block a query on a poisoned lock
        };
        Arc::clone(
            g.entry(tenant)
                .or_insert_with(|| Arc::new(Semaphore::new(n))),
        )
    };
    match sem.try_acquire_owned() {
        Ok(p) => Ok(QueryPermit(Some(p))),
        Err(_) => Err(retry_after_secs()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_when_unset_grants_inert_permits() {
        // No env set in this test ⇒ disabled ⇒ always Ok, never exhausts.
        for _ in 0..1000 {
            assert!(acquire().is_ok());
        }
    }

    #[test]
    fn agent_run_disabled_when_unset_grants_inert_permits() {
        // KYMA_AGENT_MAX_CONCURRENT unset ⇒ unlimited ⇒ never rejects.
        for _ in 0..1000 {
            assert!(acquire_agent_run().is_ok());
        }
    }

    #[test]
    fn per_tenant_disabled_when_unset_grants_inert_permits() {
        // KYMA_QUERY_MAX_CONCURRENT_PER_TENANT unset ⇒ unlimited per tenant.
        let t = kyma_core::tenant::DEFAULT_TENANT;
        for _ in 0..1000 {
            assert!(acquire_for_tenant(t).is_ok());
        }
    }

    #[tokio::test]
    async fn per_tenant_semaphores_isolate_tenants() {
        // Independent per-tenant semaphores: saturating tenant A's budget leaves
        // tenant B's untouched (the env-global limit is unsafe to mutate in
        // parallel tests, so exercise the isolation mechanics directly).
        let a = Arc::new(Semaphore::new(1));
        let b = Arc::new(Semaphore::new(1));
        let a1 = Arc::clone(&a).try_acquire_owned();
        assert!(a1.is_ok(), "tenant A admitted");
        assert!(
            Arc::clone(&a).try_acquire_owned().is_err(),
            "tenant A now saturated"
        );
        assert!(
            Arc::clone(&b).try_acquire_owned().is_ok(),
            "tenant B unaffected by A's saturation"
        );
    }

    #[tokio::test]
    async fn semaphore_admits_up_to_capacity_then_rejects() {
        // Exercise the Semaphore mechanics directly (the global limiter() reads
        // a process-wide env var, which is unsafe to mutate in parallel tests).
        let sem = Arc::new(Semaphore::new(2));
        let p1 = Arc::clone(&sem).try_acquire_owned();
        let p2 = Arc::clone(&sem).try_acquire_owned();
        assert!(p1.is_ok() && p2.is_ok(), "first two admitted");
        assert!(
            Arc::clone(&sem).try_acquire_owned().is_err(),
            "third rejected at capacity"
        );
        drop(p1);
        assert!(
            Arc::clone(&sem).try_acquire_owned().is_ok(),
            "slot freed after a permit drops"
        );
    }
}
