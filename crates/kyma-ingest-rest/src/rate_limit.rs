//! Token-bucket ingest rate limiting (S2.6 ops hardening).
//!
//! Protects the node from ingest overload ("many clients/workers" — the core
//! production-readiness concern) by metering requests per key with a token
//! bucket: a steady refill of `rps` tokens/sec up to a `burst` cap; each request
//! costs one token; an empty bucket yields `429 Too Many Requests` with a
//! `Retry-After`. Keyed by database (the available isolation unit on the ingest
//! path today; a per-tenant key lands when multi-tenant ingest does).
//!
//! Off by default: `KYMA_INGEST_RATE_RPS` unset or `0` ⇒ unlimited (every
//! request allowed), so existing deployments + the fresh-install are unchanged.
//! Set it to enable; `KYMA_INGEST_RATE_BURST` overrides the burst cap (default
//! `2×rps`, min 1).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Configured limits, or `None` when rate limiting is disabled.
struct Limits {
    rps: f64,
    burst: f64,
}

fn limits() -> Option<Limits> {
    let rps = std::env::var("KYMA_INGEST_RATE_RPS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    if rps <= 0.0 {
        return None; // disabled
    }
    let burst = std::env::var("KYMA_INGEST_RATE_BURST")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(rps * 2.0)
        .max(1.0);
    Some(Limits { rps, burst })
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

fn store() -> &'static Mutex<HashMap<String, Bucket>> {
    static S: OnceLock<Mutex<HashMap<String, Bucket>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Refill a bucket over `elapsed_secs` and try to take one token. Pure (no
/// clock) so it is unit-testable. Returns `(new_tokens, Ok|Err(retry_after_secs))`.
fn refill_and_take(tokens: f64, elapsed_secs: f64, rps: f64, burst: f64) -> (f64, Result<(), f64>) {
    let refilled = (tokens + elapsed_secs.max(0.0) * rps).min(burst);
    if refilled >= 1.0 {
        (refilled - 1.0, Ok(()))
    } else {
        // Seconds until one token accrues.
        let retry = if rps > 0.0 {
            (1.0 - refilled) / rps
        } else {
            1.0
        };
        (refilled, Err(retry.max(0.001)))
    }
}

/// Meter one ingest request for `key`. `Ok(())` when allowed (or limiting is
/// disabled); `Err(retry_after_secs)` when the bucket is empty.
pub fn check(key: &str) -> Result<(), f64> {
    let Some(lim) = limits() else {
        return Ok(());
    };
    let now = Instant::now();
    let mut g = match store().lock() {
        Ok(g) => g,
        Err(_) => return Ok(()), // never block ingest on a poisoned lock
    };
    let b = g.entry(key.to_string()).or_insert(Bucket {
        tokens: lim.burst,
        last: now,
    });
    let elapsed = now.duration_since(b.last).as_secs_f64();
    let (new_tokens, res) = refill_and_take(b.tokens, elapsed, lim.rps, lim.burst);
    b.tokens = new_tokens;
    b.last = now;
    res
}

#[cfg(test)]
mod tests {
    use super::refill_and_take;

    #[test]
    fn full_bucket_allows_then_drains() {
        // burst 3, rps 1; no time passes between calls.
        let (t1, r1) = refill_and_take(3.0, 0.0, 1.0, 3.0);
        assert!(r1.is_ok());
        assert!((t1 - 2.0).abs() < 1e-9);
        let (t2, r2) = refill_and_take(t1, 0.0, 1.0, 3.0);
        assert!(r2.is_ok());
        let (t3, r3) = refill_and_take(t2, 0.0, 1.0, 3.0);
        assert!(r3.is_ok());
        // 4th with empty bucket → denied with a retry-after ≈ 1/rps.
        let (_t4, r4) = refill_and_take(t3, 0.0, 1.0, 3.0);
        let retry = r4.unwrap_err();
        assert!((retry - 1.0).abs() < 1e-6, "retry≈1s at rps=1, got {retry}");
    }

    #[test]
    fn refill_caps_at_burst_and_recovers() {
        // Empty bucket, 10s elapsed at rps 2 → would be 20 but caps at burst 5.
        let (t, r) = refill_and_take(0.0, 10.0, 2.0, 5.0);
        assert!(r.is_ok());
        assert!(
            (t - 4.0).abs() < 1e-9,
            "capped at burst then -1 = 4, got {t}"
        );
    }

    #[test]
    fn partial_refill_still_denies_when_under_one() {
        // 0 tokens, 0.25s at rps 1 → 0.25 tokens < 1 → denied, retry ≈ 0.75s.
        let (t, r) = refill_and_take(0.0, 0.25, 1.0, 5.0);
        assert!((t - 0.25).abs() < 1e-9);
        assert!((r.unwrap_err() - 0.75).abs() < 1e-6);
    }
}
