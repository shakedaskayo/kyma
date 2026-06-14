//! In-process read cache for the object store (S2.4 read scale-out).
//!
//! Wraps any [`ObjectStore`] and memoizes read results (full-object gets,
//! bounded range gets, and `head`) in a byte-bounded LRU. Repeated queries hit
//! the same parquet footers, page indexes, ANN/FTS sidecar partitions, and hot
//! extents; caching them collapses object-store round-trips — the S2 gate's
//! ">80% cache hit on repeated queries" target — and helps local mode too (a
//! filesystem store still pays a syscall + decode per read).
//!
//! **Correctness rests on Kyma's write-once path discipline.** Extents and
//! sidecars are content-addressed (`{extent_id}` in the path) and never
//! rewritten, so a path→bytes memo needs no invalidation for them. The one
//! mutable case is deterministic-key artifacts ([`crate::put_artifact`], e.g. a
//! CI job log re-captured each tick): writes and deletes therefore evict every
//! cached range for the destination path, so a re-read after an overwrite is
//! always fresh. Conditional gets (if-match / if-modified-since / version) and
//! offset/suffix ranges bypass the cache entirely.
//!
//! Off by default: with `KYMA_CACHE_MEM_MB` unset or `0` the store is returned
//! unwrapped, so existing deployments and the fresh-install are byte-identical.
//! `KYMA_CACHE_MEM_MB` sets the total budget; `KYMA_CACHE_MAX_OBJECT_KB`
//! (default 4096) caps which single objects are admitted — a few large extents
//! must not evict the whole footer working set.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use object_store::path::Path;
use object_store::{
    Attributes, GetOptions, GetRange, GetResult, GetResultPayload, ListResult, MultipartUpload,
    ObjectMeta, ObjectStore, PutMultipartOpts, PutOptions, PutPayload, PutResult, Result,
};

/// Minimum byte cost charged per cached entry, so that many tiny entries
/// (notably `head` results, which carry no body) are still bounded by the
/// budget instead of growing without limit.
const ENTRY_OVERHEAD: usize = 256;

/// Which read of a path an entry caches.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum RangeKey {
    /// Whole-object get (no range).
    Full,
    /// `GetRange::Bounded(start..end)`.
    Bounded(usize, usize),
    /// `head` metadata (no body bytes).
    Head,
}

/// A cached read, reconstructable into a fresh [`GetResult`].
#[derive(Clone)]
struct Entry {
    bytes: Bytes,
    meta: ObjectMeta,
    attributes: Attributes,
    range: Range<usize>,
    /// LRU recency tick; also the key into `by_tick`.
    tick: u64,
    /// Accounted size (>= `ENTRY_OVERHEAD`).
    nbytes: usize,
}

/// Byte-bounded LRU over `(path, range)` reads.
struct Lru {
    map: HashMap<(Path, RangeKey), Entry>,
    /// Recency index: tick → key. Smallest tick is the least-recently-used.
    by_tick: BTreeMap<u64, (Path, RangeKey)>,
    /// Reverse index for O(ranges-per-path) eviction on write/delete.
    by_path: HashMap<Path, Vec<RangeKey>>,
    total_bytes: usize,
    max_total: usize,
    tick: u64,
    hits: u64,
    misses: u64,
}

impl Lru {
    fn new(max_total: usize) -> Self {
        Self {
            map: HashMap::new(),
            by_tick: BTreeMap::new(),
            by_path: HashMap::new(),
            total_bytes: 0,
            max_total,
            tick: 0,
            hits: 0,
            misses: 0,
        }
    }

    fn get(&mut self, path: &Path, rk: &RangeKey) -> Option<Entry> {
        let key = (path.clone(), rk.clone());
        let old_tick = match self.map.get(&key) {
            Some(e) => e.tick,
            None => {
                self.misses += 1;
                return None;
            }
        };
        // Promote to most-recently-used.
        self.by_tick.remove(&old_tick);
        self.tick += 1;
        let nt = self.tick;
        self.by_tick.insert(nt, key.clone());
        let e = self.map.get_mut(&key).expect("present above");
        e.tick = nt;
        self.hits += 1;
        Some(e.clone())
    }

    fn insert(
        &mut self,
        path: &Path,
        rk: RangeKey,
        bytes: Bytes,
        meta: ObjectMeta,
        attributes: Attributes,
        range: Range<usize>,
    ) {
        let nbytes = bytes.len().max(ENTRY_OVERHEAD);
        // An object larger than the whole budget is never worth caching (it
        // would immediately evict everything, including itself).
        if nbytes > self.max_total {
            return;
        }
        let key = (path.clone(), rk.clone());
        self.remove_key(&key);
        self.tick += 1;
        let tick = self.tick;
        self.map.insert(
            key.clone(),
            Entry {
                bytes,
                meta,
                attributes,
                range,
                tick,
                nbytes,
            },
        );
        self.by_tick.insert(tick, key.clone());
        self.by_path.entry(path.clone()).or_default().push(rk);
        self.total_bytes += nbytes;
        // Evict least-recently-used until back under budget. The just-inserted
        // entry has the largest tick, so it is never the eviction victim.
        while self.total_bytes > self.max_total {
            let Some((&oldest, _)) = self.by_tick.iter().next() else {
                break;
            };
            let victim = self.by_tick[&oldest].clone();
            self.remove_key(&victim);
        }
    }

    fn remove_key(&mut self, key: &(Path, RangeKey)) {
        if let Some(e) = self.map.remove(key) {
            self.total_bytes = self.total_bytes.saturating_sub(e.nbytes);
            self.by_tick.remove(&e.tick);
            if let Some(v) = self.by_path.get_mut(&key.0) {
                v.retain(|r| r != &key.1);
                if v.is_empty() {
                    self.by_path.remove(&key.0);
                }
            }
        }
    }

    /// Drop every cached read for `path` — called on write/delete so an
    /// overwrite of a deterministic-key artifact is never served stale.
    fn evict_path(&mut self, path: &Path) {
        if let Some(rks) = self.by_path.remove(path) {
            for rk in rks {
                let key = (path.clone(), rk);
                if let Some(e) = self.map.remove(&key) {
                    self.total_bytes = self.total_bytes.saturating_sub(e.nbytes);
                    self.by_tick.remove(&e.tick);
                }
            }
        }
    }
}

/// Caching wrapper over an inner [`ObjectStore`].
pub struct CachingObjectStore {
    inner: Arc<dyn ObjectStore>,
    lru: Mutex<Lru>,
    /// Largest single object (bytes) admitted to the cache.
    max_object: usize,
}

impl fmt::Debug for CachingObjectStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (hits, misses, bytes) = self.stats();
        f.debug_struct("CachingObjectStore")
            .field("inner", &self.inner)
            .field("hits", &hits)
            .field("misses", &misses)
            .field("cached_bytes", &bytes)
            .finish()
    }
}

impl fmt::Display for CachingObjectStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CachingObjectStore({})", self.inner)
    }
}

impl CachingObjectStore {
    /// Wrap `inner` with a byte-bounded read cache. `max_total` is the overall
    /// budget; `max_object` caps the size of any single admitted object (clamped
    /// to `max_total`).
    pub fn new(inner: Arc<dyn ObjectStore>, max_total: usize, max_object: usize) -> Self {
        Self {
            inner,
            lru: Mutex::new(Lru::new(max_total.max(1))),
            max_object: max_object.min(max_total.max(1)),
        }
    }

    /// `(hits, misses, cached_bytes)` — for tests and `Debug`.
    pub fn stats(&self) -> (u64, u64, usize) {
        match self.lru.lock() {
            Ok(g) => (g.hits, g.misses, g.total_bytes),
            Err(_) => (0, 0, 0),
        }
    }

    /// Map a request's range to a cache key, or `None` if it must not be cached
    /// (offset/suffix ranges have no fixed key until the object size is known).
    fn range_key(range: &Option<GetRange>) -> Option<RangeKey> {
        match range {
            None => Some(RangeKey::Full),
            Some(GetRange::Bounded(r)) => Some(RangeKey::Bounded(r.start, r.end)),
            Some(GetRange::Offset(_)) | Some(GetRange::Suffix(_)) => None,
        }
    }

    /// A plain read with no conditional headers / version / head flag is safe to
    /// serve from or store in the cache.
    fn cacheable(opts: &GetOptions) -> bool {
        opts.if_match.is_none()
            && opts.if_none_match.is_none()
            && opts.if_modified_since.is_none()
            && opts.if_unmodified_since.is_none()
            && opts.version.is_none()
            && !opts.head
    }
}

/// Build a one-shot `GetResult` whose payload streams the cached bytes once.
fn get_result_from(
    bytes: Bytes,
    meta: ObjectMeta,
    range: Range<usize>,
    attrs: Attributes,
) -> GetResult {
    let payload: BoxStream<'static, Result<Bytes>> = stream::once(async move { Ok(bytes) }).boxed();
    GetResult {
        payload: GetResultPayload::Stream(payload),
        meta,
        range,
        attributes: attrs,
    }
}

#[async_trait]
impl ObjectStore for CachingObjectStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> Result<PutResult> {
        let r = self.inner.put_opts(location, payload, opts).await?;
        // The path's bytes may have changed (overwrite-on-write artifacts).
        if let Ok(mut g) = self.lru.lock() {
            g.evict_path(location);
        }
        Ok(r)
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOpts,
    ) -> Result<Box<dyn MultipartUpload>> {
        // Multipart targets are new, unique extent paths; nothing to evict until
        // completion, and those paths are never previously cached.
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
        let cacheable = Self::cacheable(&options);
        let rkey = if cacheable {
            Self::range_key(&options.range)
        } else {
            None
        };

        if let Some(rk) = rkey.clone() {
            let hit = self.lru.lock().ok().and_then(|mut g| g.get(location, &rk));
            if let Some(e) = hit {
                return Ok(get_result_from(e.bytes, e.meta, e.range, e.attributes));
            }
        }

        let res = self.inner.get_opts(location, options).await?;

        // Admit only when we have a stable key and the payload fits the cap.
        let Some(rk) = rkey else {
            return Ok(res);
        };
        let len = res.range.end.saturating_sub(res.range.start);
        if len > self.max_object {
            return Ok(res);
        }
        let meta = res.meta.clone();
        let attrs = res.attributes.clone();
        let range = res.range.clone();
        // Buffer the body so it can be replayed on hits.
        let bytes = res.bytes().await?;
        if let Ok(mut g) = self.lru.lock() {
            g.insert(
                location,
                rk,
                bytes.clone(),
                meta.clone(),
                attrs.clone(),
                range.clone(),
            );
        }
        Ok(get_result_from(bytes, meta, range, attrs))
    }

    async fn head(&self, location: &Path) -> Result<ObjectMeta> {
        if let Ok(mut g) = self.lru.lock() {
            if let Some(e) = g.get(location, &RangeKey::Head) {
                return Ok(e.meta);
            }
        }
        let meta = self.inner.head(location).await?;
        if let Ok(mut g) = self.lru.lock() {
            g.insert(
                location,
                RangeKey::Head,
                Bytes::new(),
                meta.clone(),
                Attributes::default(),
                0..0,
            );
        }
        Ok(meta)
    }

    async fn delete(&self, location: &Path) -> Result<()> {
        let r = self.inner.delete(location).await;
        if let Ok(mut g) = self.lru.lock() {
            g.evict_path(location);
        }
        r
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'_, Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let r = self.inner.copy(from, to).await?;
        if let Ok(mut g) = self.lru.lock() {
            g.evict_path(to);
        }
        Ok(r)
    }

    async fn copy_if_not_exists(&self, from: &Path, to: &Path) -> Result<()> {
        let r = self.inner.copy_if_not_exists(from, to).await?;
        if let Ok(mut g) = self.lru.lock() {
            g.evict_path(to);
        }
        Ok(r)
    }
}

/// Wrap `inner` with a read cache when `KYMA_CACHE_MEM_MB` is set and positive;
/// otherwise return it unchanged. Centralizes the env contract so
/// [`crate::build_object_store`] stays a plain factory.
pub fn maybe_wrap(inner: Arc<dyn ObjectStore>) -> Arc<dyn ObjectStore> {
    let mb = std::env::var("KYMA_CACHE_MEM_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if mb == 0 {
        return inner;
    }
    let max_total = mb.saturating_mul(1024 * 1024).max(1);
    let max_object = std::env::var("KYMA_CACHE_MAX_OBJECT_KB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4096)
        .saturating_mul(1024)
        .max(1);
    Arc::new(CachingObjectStore::new(inner, max_total, max_object))
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;

    fn store_with(
        max_total: usize,
        max_object: usize,
    ) -> (Arc<CachingObjectStore>, Arc<dyn ObjectStore>) {
        let inner: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let cache = Arc::new(CachingObjectStore::new(
            inner.clone(),
            max_total,
            max_object,
        ));
        (cache, inner)
    }

    async fn put(store: &Arc<dyn ObjectStore>, path: &str, body: &[u8]) {
        store
            .put(&Path::from(path), PutPayload::from(body.to_vec()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn second_read_is_a_hit_and_returns_same_bytes() {
        let (cache, inner) = store_with(1 << 20, 1 << 20);
        put(&inner, "a", b"hello world").await;

        let b1 = cache
            .get(&Path::from("a"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let b2 = cache
            .get(&Path::from("a"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(&b1[..], b"hello world");
        assert_eq!(b1, b2);
        let (hits, misses, _) = cache.stats();
        assert_eq!((hits, misses), (1, 1), "one miss then one hit");
    }

    #[tokio::test]
    async fn range_reads_are_cached_independently() {
        let (cache, inner) = store_with(1 << 20, 1 << 20);
        put(&inner, "a", b"0123456789").await;

        let r1 = cache.get_range(&Path::from("a"), 2..5).await.unwrap();
        assert_eq!(&r1[..], b"234");
        let r2 = cache.get_range(&Path::from("a"), 2..5).await.unwrap();
        assert_eq!(r1, r2);
        let (hits, misses, _) = cache.stats();
        assert_eq!((hits, misses), (1, 1));
    }

    #[tokio::test]
    async fn overwrite_evicts_so_reads_are_fresh() {
        let (cache, inner) = store_with(1 << 20, 1 << 20);
        put(&inner, "k", b"v1").await;
        let first = cache
            .get(&Path::from("k"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(&first[..], b"v1");

        // Overwrite through the caching store → eviction → next read is fresh.
        cache
            .put(&Path::from("k"), PutPayload::from(b"v2-longer".to_vec()))
            .await
            .unwrap();
        let second = cache
            .get(&Path::from("k"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(&second[..], b"v2-longer", "must not serve stale v1");
    }

    #[tokio::test]
    async fn delete_evicts_entry() {
        let (cache, inner) = store_with(1 << 20, 1 << 20);
        put(&inner, "d", b"bytes").await;
        let _ = cache
            .get(&Path::from("d"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let (_, _, bytes_before) = cache.stats();
        assert!(bytes_before > 0);

        cache.delete(&Path::from("d")).await.unwrap();
        let (_, _, bytes_after) = cache.stats();
        assert_eq!(bytes_after, 0, "delete drops the cached entry");
    }

    #[tokio::test]
    async fn oversized_object_is_not_admitted() {
        // max_object 4 bytes: a 10-byte object is served but never cached.
        let (cache, inner) = store_with(1 << 20, 4);
        put(&inner, "big", b"0123456789").await;
        let _ = cache
            .get(&Path::from("big"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let _ = cache
            .get(&Path::from("big"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let (hits, misses, bytes) = cache.stats();
        assert_eq!(bytes, 0, "oversized object not cached");
        assert_eq!((hits, misses), (0, 2), "both reads miss");
    }

    #[tokio::test]
    async fn lru_evicts_under_budget_pressure() {
        // Bodies (1000 B) dominate the per-entry overhead, so the budget admits
        // exactly two before the third forces an eviction.
        let (cache, inner) = store_with(2200, 1 << 20);
        let body = vec![b'x'; 1000];
        put(&inner, "a", &body).await;
        put(&inner, "b", &body).await;
        put(&inner, "c", &body).await;

        // Read a, then b → both cached. Read a again → a is now MRU.
        let _ = cache
            .get(&Path::from("a"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let _ = cache
            .get(&Path::from("b"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let _ = cache
            .get(&Path::from("a"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        // Read c → inserting c must evict b (the LRU), not a.
        let _ = cache
            .get(&Path::from("c"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();

        // a is still cached (hit), b was evicted (miss).
        let (h0, m0, _) = cache.stats();
        let _ = cache
            .get(&Path::from("a"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let _ = cache
            .get(&Path::from("b"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let (h1, m1, _) = cache.stats();
        assert_eq!(h1 - h0, 1, "a was retained → hit");
        assert_eq!(m1 - m0, 1, "b was evicted → miss");
    }
}
