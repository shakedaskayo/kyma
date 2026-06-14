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

use std::sync::{Arc, OnceLock};
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
