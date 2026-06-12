//! Disk LRU cache for immutable index-sidecar objects.
//!
//! Sidecars (ANN / FTS blobs built per extent — see
//! `kyma_core::index_sidecar`) are immutable: a given
//! `(extent, column, kind, model)` key always names the same bytes, so the
//! cache never needs invalidation — only byte-capped LRU eviction.
//!
//! Layout: one file per sidecar under
//! `${KYMA_CACHE_DIR:-~/.kyma/cache}/indexes`, named from the cache key
//! (extent id + a short hash of column/kind/model). Writes go through a
//! temp file + atomic rename, so concurrent fetchers of the same key are
//! safe (last rename wins; both see complete bytes). LRU recency is the
//! file's mtime, refreshed on every cache hit.
//!
//! Capacity: `KYMA_SIDECAR_CACHE_MAX_BYTES` (default 2 GiB). Eviction may
//! remove a file another reader still holds a `PathBuf` to — on Unix an
//! already-open fd keeps working; a reader re-opening by path should treat
//! `NotFound` as a cache miss and call [`SidecarCache::get_or_fetch`] again.

use crate::sha256_hex;
use kyma_core::errors::{Result, StorageError};
use kyma_core::index_sidecar::SidecarKind;
use kyma_core::types::ExtentId;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Default capacity: 2 GiB.
const DEFAULT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Identity of one cached sidecar. Mirrors the `extent_indexes` upsert key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SidecarCacheKey {
    pub extent_id: ExtentId,
    pub column: String,
    pub kind: SidecarKind,
    /// Embedding model for ANN sidecars; `None` for model-free kinds.
    pub model_id: Option<String>,
}

impl SidecarCacheKey {
    /// Stable on-disk file name: readable extent id prefix + a short hash of
    /// the full key (so arbitrary column names can never escape the dir).
    fn file_name(&self) -> String {
        let canonical = format!(
            "{}\u{0}{}\u{0}{}\u{0}{}",
            self.extent_id,
            self.column,
            self.kind.as_str(),
            self.model_id.as_deref().unwrap_or("")
        );
        let hash = sha256_hex(canonical.as_bytes());
        format!("{}-{}.{}", self.extent_id, &hash[..16], self.kind.as_str())
    }
}

/// Byte-capped disk LRU cache for sidecar objects. Cheap to clone/share —
/// all state lives on disk.
#[derive(Debug, Clone)]
pub struct SidecarCache {
    root: PathBuf,
    max_bytes: u64,
}

impl SidecarCache {
    /// Cache at `root` holding at most `max_bytes` (≥ 1).
    pub fn new(root: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            root: root.into(),
            max_bytes: max_bytes.max(1),
        }
    }

    /// Build from the environment: `${KYMA_CACHE_DIR:-~/.kyma/cache}/indexes`,
    /// capped by `KYMA_SIDECAR_CACHE_MAX_BYTES` (default 2 GiB).
    pub fn from_env() -> Self {
        let base = std::env::var("KYMA_CACHE_DIR").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            format!("{home}/.kyma/cache")
        });
        let max_bytes = std::env::var("KYMA_SIDECAR_CACHE_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_BYTES);
        Self::new(PathBuf::from(base).join("indexes"), max_bytes)
    }

    /// The cache directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the local path of the cached sidecar for `key`, fetching
    /// `object_path` from `store` on a miss. Refreshes LRU recency on hits
    /// and evicts least-recently-used files when the cache exceeds its cap.
    pub async fn get_or_fetch(
        &self,
        store: &Arc<dyn ObjectStore>,
        object_path: &str,
        key: &SidecarCacheKey,
    ) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.root)
            .map_err(|e| StorageError::Cache(format!("create cache dir: {e}")))?;
        let dest = self.root.join(key.file_name());

        // Hit: bump recency (best-effort) and return.
        if dest.exists() {
            if let Ok(f) = std::fs::File::options().write(true).open(&dest) {
                let _ = f.set_modified(SystemTime::now());
            }
            return Ok(dest);
        }

        // Miss: fetch, write to a temp file, atomic-rename into place.
        let bytes = store
            .get(&ObjPath::from(object_path))
            .await
            .map_err(|e| StorageError::Cache(format!("fetch sidecar {object_path}: {e}")))?
            .bytes()
            .await
            .map_err(|e| StorageError::Cache(format!("read sidecar {object_path}: {e}")))?;

        let tmp = self
            .root
            .join(format!(".tmp-{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, &bytes)
            .map_err(|e| StorageError::Cache(format!("write cache temp: {e}")))?;
        std::fs::rename(&tmp, &dest).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            StorageError::Cache(format!("install cache file: {e}"))
        })?;

        self.evict_lru(&dest);
        Ok(dest)
    }

    /// Evict least-recently-used files (by mtime) until the cache fits the
    /// byte cap. `keep` (the file just installed) is never evicted, so a
    /// single oversized sidecar still makes progress. Best-effort: I/O
    /// errors are swallowed — eviction can always run again.
    fn evict_lru(&self, keep: &Path) {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        let mut total: u64 = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            // Skip in-flight temp files; their writers will rename them.
            if name.to_string_lossy().starts_with(".tmp-") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            total += meta.len();
            files.push((path, meta.len(), mtime));
        }
        if total <= self.max_bytes {
            return;
        }
        files.sort_by_key(|(_, _, mtime)| *mtime); // oldest first
        for (path, len, _) in files {
            if total <= self.max_bytes {
                break;
            }
            if path == keep {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_object_store, StorageConfig};
    use bytes::Bytes;
    use object_store::path::Path as ObjPath;

    fn mem_store() -> Arc<dyn ObjectStore> {
        build_object_store(&StorageConfig::Memory).unwrap()
    }

    fn key(extent: ExtentId, column: &str) -> SidecarCacheKey {
        SidecarCacheKey {
            extent_id: extent,
            column: column.to_string(),
            kind: SidecarKind::IvfRabitq,
            model_id: None,
        }
    }

    async fn put(store: &Arc<dyn ObjectStore>, path: &str, payload: &[u8]) {
        store
            .put(&ObjPath::from(path), Bytes::copy_from_slice(payload).into())
            .await
            .unwrap();
    }

    fn temp_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "kyma-sidecar-cache-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[tokio::test]
    async fn hit_miss_round_trip_and_distinct_keys() {
        let root = temp_root("roundtrip");
        let cache = SidecarCache::new(&root, 1024 * 1024);
        let store = mem_store();
        let e = ExtentId::new();
        put(&store, "t/indexes/e/a.ivf_rabitq", b"index-bytes-a").await;
        put(&store, "t/indexes/e/b.ivf_rabitq", b"index-bytes-b").await;

        let p1 = cache
            .get_or_fetch(&store, "t/indexes/e/a.ivf_rabitq", &key(e, "a"))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&p1).unwrap(), b"index-bytes-a");

        // Hit returns the same path with the same content; no store needed
        // (delete the object to prove the hit never refetches).
        store
            .delete(&ObjPath::from("t/indexes/e/a.ivf_rabitq"))
            .await
            .unwrap();
        let p1_again = cache
            .get_or_fetch(&store, "t/indexes/e/a.ivf_rabitq", &key(e, "a"))
            .await
            .unwrap();
        assert_eq!(p1_again, p1);
        assert_eq!(std::fs::read(&p1_again).unwrap(), b"index-bytes-a");

        // A different column is a different cache entry.
        let p2 = cache
            .get_or_fetch(&store, "t/indexes/e/b.ivf_rabitq", &key(e, "b"))
            .await
            .unwrap();
        assert_ne!(p2, p1);
        assert_eq!(std::fs::read(&p2).unwrap(), b"index-bytes-b");

        // Model id participates in the key.
        let mut with_model = key(e, "a");
        with_model.model_id = Some("m1".into());
        assert_ne!(with_model.file_name(), key(e, "a").file_name());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn lru_eviction_removes_least_recently_used() {
        let root = temp_root("lru");
        // Cap fits two 100-byte sidecars, not three.
        let cache = SidecarCache::new(&root, 250);
        let store = mem_store();
        let e = ExtentId::new();
        put(&store, "s/a", &[b'a'; 100]).await;
        put(&store, "s/b", &[b'b'; 100]).await;
        put(&store, "s/c", &[b'c'; 100]).await;

        let pa = cache.get_or_fetch(&store, "s/a", &key(e, "a")).await.unwrap();
        let pb = cache.get_or_fetch(&store, "s/b", &key(e, "b")).await.unwrap();

        // Make recency unambiguous regardless of fs mtime granularity:
        // a is old, b is fresh.
        #[allow(clippy::duration_suboptimal_units)] // from_hours is unstable on MSRV
        let old = SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&pa)
            .unwrap()
            .set_modified(old)
            .unwrap();

        // Touch b via a cache hit (refreshes its mtime).
        cache.get_or_fetch(&store, "s/b", &key(e, "b")).await.unwrap();

        // Inserting c (total 300 > 250) must evict a — the LRU — and keep b + c.
        let pc = cache.get_or_fetch(&store, "s/c", &key(e, "c")).await.unwrap();
        assert!(!pa.exists(), "least-recently-used entry should be evicted");
        assert!(pb.exists(), "recently-touched entry should survive");
        assert!(pc.exists(), "just-inserted entry is never evicted");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_fetchers_of_one_key_all_get_full_bytes() {
        let root = temp_root("concurrent");
        let cache = SidecarCache::new(&root, 10 * 1024 * 1024);
        let store = mem_store();
        let e = ExtentId::new();
        let payload: Vec<u8> = (0u32..64 * 1024).map(|i| u8::try_from(i % 251).unwrap()).collect();
        put(&store, "s/big", &payload).await;

        let mut handles = Vec::new();
        for _ in 0..16 {
            let cache = cache.clone();
            let store = store.clone();
            let payload = payload.clone();
            let k = key(e, "vec");
            handles.push(tokio::spawn(async move {
                let p = cache.get_or_fetch(&store, "s/big", &k).await.unwrap();
                let got = std::fs::read(&p).unwrap();
                assert_eq!(got, payload, "every concurrent reader sees full bytes");
                p
            }));
        }
        let mut paths = Vec::new();
        for h in handles {
            paths.push(h.await.unwrap());
        }
        // All callers converge on the same cache file.
        assert!(paths.windows(2).all(|w| w[0] == w[1]));
        // No temp files leak.
        let leftovers: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files must not leak: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
