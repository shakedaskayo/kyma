//! Object-storage construction + (future) local block cache.
//!
//! Thin factory over `object_store`. Picks the right backend (S3/MinIO,
//! local filesystem, in-memory for tests) from config. The engine consumes
//! the resulting `Arc<dyn ObjectStore>` directly — no custom trait wraps
//! it, so `object_store` ecosystem improvements flow through without
//! adaptation.
//!
//! # Config shape
//!
//! A `StorageConfig` is typically loaded from env vars or a TOML file and
//! passed to [`build_object_store`].

#![forbid(unsafe_code)]

use kyma_core::errors::{Result, StorageError};
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::ObjectStore;
use std::sync::Arc;

/// Storage backend configuration.
#[derive(Debug, Clone)]
pub enum StorageConfig {
    /// S3-compatible store (real AWS or MinIO). MinIO needs `endpoint`
    /// set and path-style addressing.
    S3Compatible {
        endpoint: Option<String>,
        region: String,
        bucket: String,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        /// Path-style addressing (MinIO default) vs virtual-host (AWS default).
        path_style: bool,
        /// Skip TLS verification (dev-only; never in prod).
        allow_http: bool,
    },
    /// Local filesystem — useful for tests without any docker.
    Local { root: String },
    /// Pure in-memory, for unit tests.
    Memory,
}

/// Construct an `ObjectStore` from configuration.
pub fn build_object_store(config: &StorageConfig) -> Result<Arc<dyn ObjectStore>> {
    match config {
        StorageConfig::S3Compatible {
            endpoint,
            region,
            bucket,
            access_key_id,
            secret_access_key,
            path_style,
            allow_http,
        } => {
            // No static keys → start from the environment so the standard AWS
            // credential chain applies (ECS/Fargate task role via
            // `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI`, web identity, `AWS_*`
            // vars, IMDS). `AmazonS3Builder::new()` would silently skip the
            // task-role provider, breaking keyless deployments.
            let base = if access_key_id.is_none() && secret_access_key.is_none() {
                AmazonS3Builder::from_env()
            } else {
                AmazonS3Builder::new()
            };
            let mut builder = base
                .with_bucket_name(bucket)
                .with_region(region)
                .with_virtual_hosted_style_request(!*path_style)
                .with_allow_http(*allow_http);
            if let Some(ep) = endpoint {
                builder = builder.with_endpoint(ep);
            }
            if let Some(ak) = access_key_id {
                builder = builder.with_access_key_id(ak);
            }
            if let Some(sk) = secret_access_key {
                builder = builder.with_secret_access_key(sk);
            }
            let store = builder
                .build()
                .map_err(|e| StorageError::ObjectStore(e.to_string()))?;
            Ok(Arc::new(store))
        }
        StorageConfig::Local { root } => {
            let fs = LocalFileSystem::new_with_prefix(root)
                .map_err(|e| StorageError::ObjectStore(e.to_string()))?;
            Ok(Arc::new(fs))
        }
        StorageConfig::Memory => Ok(Arc::new(InMemory::new())),
    }
}

/// Resolve the local data root for single-binary mode. Order: `KYMA_LOCAL_DATA`
/// env, else `$HOME/.kyma/data`, else `./.kyma/data`.
pub fn local_data_root() -> String {
    if let Ok(p) = std::env::var("KYMA_LOCAL_DATA") {
        return p;
    }
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{base}/.kyma/data")
}

/// True if any `KYMA_S3_*` variable is set — i.e. an object store is configured
/// (the server/docker path). When none are set we assume local single-binary
/// mode and fall back to a local filesystem store.
fn any_s3_env_set() -> bool {
    [
        "KYMA_S3_ENDPOINT",
        "KYMA_S3_BUCKET",
        "KYMA_S3_REGION",
        "KYMA_S3_ACCESS_KEY_ID",
        "KYMA_S3_SECRET_ACCESS_KEY",
    ]
    .iter()
    .any(|k| std::env::var(k).is_ok())
}

/// Convenience: load storage config from standard env vars.
///
/// **Local single-binary mode** is auto-selected when `KYMA_LOCAL_MODE` is
/// truthy *or* no `KYMA_S3_*` variable is set: the store becomes a local
/// filesystem rooted at [`local_data_root`] (created if missing) — zero infra.
///
/// Otherwise an **S3-compatible** store is built from (matching our
/// docker-compose defaults):
/// - `KYMA_S3_ENDPOINT` (e.g. `http://localhost:9000`)
/// - `KYMA_S3_BUCKET` (default: `kyma`)
/// - `KYMA_S3_REGION` (default: `us-east-1`)
/// - `KYMA_S3_ACCESS_KEY_ID`
/// - `KYMA_S3_SECRET_ACCESS_KEY`
/// - `KYMA_S3_PATH_STYLE` (default: `true`)
///
/// When both `KYMA_S3_ACCESS_KEY_ID` and `KYMA_S3_SECRET_ACCESS_KEY` are
/// unset, credentials come from the standard AWS provider chain (ECS/Fargate
/// task role, web identity, `AWS_*` env vars, IMDS) — set only
/// `KYMA_S3_BUCKET`/`KYMA_S3_REGION` for keyless IAM-role deployments.
pub fn config_from_env() -> StorageConfig {
    let local_mode = std::env::var("KYMA_LOCAL_MODE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    if local_mode || !any_s3_env_set() {
        let root = local_data_root();
        // Best-effort create; `build_object_store` surfaces a real error if it
        // still can't open the directory.
        let _ = std::fs::create_dir_all(&root);
        return StorageConfig::Local { root };
    }

    StorageConfig::S3Compatible {
        endpoint: std::env::var("KYMA_S3_ENDPOINT").ok(),
        region: std::env::var("KYMA_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        bucket: std::env::var("KYMA_S3_BUCKET").unwrap_or_else(|_| "kyma".to_string()),
        access_key_id: std::env::var("KYMA_S3_ACCESS_KEY_ID").ok(),
        secret_access_key: std::env::var("KYMA_S3_SECRET_ACCESS_KEY").ok(),
        path_style: std::env::var("KYMA_S3_PATH_STYLE")
            .map(|v| v != "false" && v != "0")
            .unwrap_or(true),
        allow_http: std::env::var("KYMA_S3_ALLOW_HTTP")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Artifact helpers
//
// Full-file blobs (CI job logs, agent-contributed files, filesystem-watch
// snapshots) live on the object store as standalone artifacts — distinct from
// the columnar extents. These helpers add the semantics those call sites share:
// `Option`-on-NotFound reads, an if-absent write for content-addressed dedup,
// range reads so agents fetch only the window they need, and a stable content
// hash. Redaction is the CALLER's responsibility — bytes are stored verbatim.
// ─────────────────────────────────────────────────────────────────────────────

use bytes::Bytes;
use object_store::path::Path as ObjPath;

/// Lowercase hex SHA-256 of `bytes`. Used to content-address artifacts so a
/// re-read of unchanged content is a no-op write.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Store `bytes` at `key`, overwriting any existing object. Use for
/// deterministic keys where the latest write wins (e.g. a CI job-log path that
/// the connector re-captures on each tick).
pub async fn put_artifact(store: &Arc<dyn ObjectStore>, key: &ObjPath, bytes: Bytes) -> Result<()> {
    store
        .put(key, bytes.into())
        .await
        .map_err(|e| StorageError::ObjectStore(e.to_string()))?;
    Ok(())
}

/// Store `bytes` at `key` only if no object already exists there. Returns
/// `true` when newly written, `false` when an object was already present.
/// Intended for content-addressed keys, where an existing object holds the
/// same bytes by construction, so the skipped write loses nothing.
pub async fn put_artifact_if_absent(
    store: &Arc<dyn ObjectStore>,
    key: &ObjPath,
    bytes: Bytes,
) -> Result<bool> {
    match store.head(key).await {
        Ok(_) => Ok(false),
        Err(object_store::Error::NotFound { .. }) => {
            put_artifact(store, key, bytes).await?;
            Ok(true)
        }
        Err(e) => Err(StorageError::ObjectStore(e.to_string()).into()),
    }
}

/// Fetch the full object at `key`. Returns `None` if it does not exist.
pub async fn get_artifact(store: &Arc<dyn ObjectStore>, key: &ObjPath) -> Result<Option<Bytes>> {
    match store.get(key).await {
        Ok(res) => {
            let bytes = res
                .bytes()
                .await
                .map_err(|e| StorageError::ObjectStore(e.to_string()))?;
            Ok(Some(bytes))
        }
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(StorageError::ObjectStore(e.to_string()).into()),
    }
}

/// Fetch the byte window `[offset, offset+len)` of the object at `key`.
///
/// Returns `None` if the object does not exist, and an empty `Bytes` if
/// `offset` is at or past the end. The window is clamped to the object size, so
/// the returned slice may be shorter than `len`. On S3 this issues an HTTP
/// Range request — only the requested bytes leave the store, which is what lets
/// agents page large logs without pulling the whole file.
pub async fn get_artifact_range(
    store: &Arc<dyn ObjectStore>,
    key: &ObjPath,
    offset: usize,
    len: usize,
) -> Result<Option<Bytes>> {
    let meta = match store.head(key).await {
        Ok(m) => m,
        Err(object_store::Error::NotFound { .. }) => return Ok(None),
        Err(e) => return Err(StorageError::ObjectStore(e.to_string()).into()),
    };
    let size = meta.size;
    if offset >= size {
        return Ok(Some(Bytes::new()));
    }
    let end = offset.saturating_add(len).min(size);
    let bytes = store
        .get_range(key, offset..end)
        .await
        .map_err(|e| StorageError::ObjectStore(e.to_string()))?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod artifact_tests {
    use super::*;

    fn mem_store() -> Arc<dyn ObjectStore> {
        build_object_store(&StorageConfig::Memory).unwrap()
    }

    #[tokio::test]
    async fn put_and_get_roundtrips_and_missing_is_none() {
        let store = mem_store();
        let key = ObjPath::from("artifacts/test/a.blob");

        assert!(get_artifact(&store, &key).await.unwrap().is_none());

        put_artifact(&store, &key, Bytes::from_static(b"hello world"))
            .await
            .unwrap();

        let got = get_artifact(&store, &key).await.unwrap();
        assert_eq!(got.as_deref(), Some(&b"hello world"[..]));
    }

    #[tokio::test]
    async fn get_range_returns_clamped_window() {
        let store = mem_store();
        let key = ObjPath::from("artifacts/test/range.blob");
        put_artifact(&store, &key, Bytes::from_static(b"0123456789"))
            .await
            .unwrap();

        let mid = get_artifact_range(&store, &key, 2, 3).await.unwrap();
        assert_eq!(mid.as_deref(), Some(&b"234"[..]));

        // Past-end request clamps to the object size rather than erroring.
        let tail = get_artifact_range(&store, &key, 8, 100).await.unwrap();
        assert_eq!(tail.as_deref(), Some(&b"89"[..]));

        // Offset at/after end yields an empty slice, not None.
        let empty = get_artifact_range(&store, &key, 10, 5).await.unwrap();
        assert_eq!(empty.as_deref(), Some(&b""[..]));

        // Missing object is None.
        let missing = ObjPath::from("artifacts/test/nope.blob");
        assert!(get_artifact_range(&store, &missing, 0, 4).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn put_if_absent_dedups_and_does_not_overwrite() {
        let store = mem_store();
        let key = ObjPath::from("artifacts/test/dedup.blob");

        let first = put_artifact_if_absent(&store, &key, Bytes::from_static(b"original"))
            .await
            .unwrap();
        assert!(first, "first write should report newly-stored");

        let second = put_artifact_if_absent(&store, &key, Bytes::from_static(b"different"))
            .await
            .unwrap();
        assert!(!second, "second write should report already-present");

        // The original bytes must survive — if-absent never overwrites.
        let got = get_artifact(&store, &key).await.unwrap();
        assert_eq!(got.as_deref(), Some(&b"original"[..]));
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        // NIST FIPS-180 test vector for "abc".
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

#[cfg(test)]
mod s3_credential_chain_tests {
    use super::*;
    use std::io::{Read, Write};

    /// With no static keys in the config, credentials must come from the
    /// standard AWS provider chain (task role / web identity / `AWS_*` env) —
    /// the Fargate task-role path. Observable: the S3 request arriving at the
    /// endpoint is signed with the `AWS_ACCESS_KEY_ID` from the environment.
    #[test]
    fn s3_without_static_keys_uses_aws_env_credential_chain() {
        std::env::set_var("AWS_ACCESS_KEY_ID", "env-chain-key");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "env-chain-secret");

        // One-shot HTTP server capturing the raw request.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let n = sock.read(&mut buf).unwrap_or(0);
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
            }
        });

        let cfg = StorageConfig::S3Compatible {
            endpoint: Some(format!("http://{addr}")),
            region: "us-east-1".to_string(),
            bucket: "kyma-test".to_string(),
            access_key_id: None,
            secret_access_key: None,
            path_style: true,
            allow_http: true,
        };
        let store = build_object_store(&cfg).expect("S3 store builds without static keys");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // The GET result is irrelevant (empty 200 won't parse as an object);
        // we only care which credentials signed the request. Bound the wait so
        // a misrouted credential lookup (e.g. IMDS) can't hang the test.
        let _ = rt.block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                store.get(&object_store::path::Path::from("probe")),
            )
            .await
        });

        let req = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or_default();
        assert!(
            req.contains("env-chain-key"),
            "request should be signed with env-chain credentials, got:\n{req}"
        );

        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
    }
}

#[cfg(test)]
mod local_mode_tests {
    use super::*;

    #[test]
    fn local_mode_auto_selects_filesystem_store() {
        let dir = std::env::temp_dir().join(format!("kyma-local-{}", std::process::id()));
        let root = dir.to_str().unwrap().to_string();
        std::env::set_var("KYMA_LOCAL_MODE", "1");
        std::env::set_var("KYMA_LOCAL_DATA", &root);

        let cfg = config_from_env();
        match &cfg {
            StorageConfig::Local { root: r } => assert_eq!(r, &root),
            other => panic!("expected Local, got {other:?}"),
        }
        // config_from_env created the directory and the store opens against it.
        assert!(dir.exists(), "data root created");
        assert!(build_object_store(&cfg).is_ok(), "local FS store builds");

        std::env::remove_var("KYMA_LOCAL_MODE");
        std::env::remove_var("KYMA_LOCAL_DATA");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
