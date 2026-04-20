//! Object-store file-drop ingest.
//!
//! Polls a configured bucket/prefix on an interval. For each object found:
//!   1. Route by path convention `{prefix}/{database}/{table}/*.ndjson` to
//!      the target (database, table).
//!   2. Compute a SHA256 of the object contents; use `filedrop:{sha}` as
//!      the idempotency key in the existing `ingest_ledger`. Re-processing
//!      the same content is a no-op.
//!   3. Parse the content per extension (NDJSON for MVP; CSV + Parquet in
//!      follow-on work).
//!   4. Route through the same `WritePath` the REST ingest uses, so
//!      staging / commit-coordinator / metrics all fire unchanged.
//!
//! We do **not** delete processed files by default — the idempotency ledger
//! makes re-polling a no-op and keeping the raw files preserves a
//! replayable audit trail. Configurable via `FiledropConfig::delete_after_ingest`.

#![forbid(unsafe_code)]

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use futures::StreamExt;
use kyma_core::catalog::Catalog;
use kyma_core::errors::{Error, Result};
use kyma_ingest_core::WritePath;
use object_store::path::Path;
use object_store::ObjectStore;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, instrument, warn};

#[derive(Debug, Clone)]
pub struct FiledropConfig {
    /// Object-store prefix under which user files land (before the
    /// database/table path segments). Default `"ingest"`, so the full
    /// convention is `ingest/{database}/{table}/<anything>.ndjson`.
    pub prefix: String,
    /// How often to scan for new files.
    pub poll_interval: Duration,
    /// If true, delete object after successful ingest. If false (default),
    /// leave in place — the idempotency ledger handles re-scans.
    pub delete_after_ingest: bool,
}

impl Default for FiledropConfig {
    fn default() -> Self {
        Self {
            prefix: "ingest".to_string(),
            poll_interval: Duration::from_secs(5),
            delete_after_ingest: false,
        }
    }
}

impl FiledropConfig {
    pub fn from_env() -> Self {
        let mut d = Self::default();
        if let Ok(v) = std::env::var("KYMA_FILEDROP_PREFIX") {
            if !v.is_empty() {
                d.prefix = v;
            }
        }
        if let Ok(v) = std::env::var("KYMA_FILEDROP_POLL_SECS")
            .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
        {
            d.poll_interval = Duration::from_secs(v);
        }
        if let Ok(v) = std::env::var("KYMA_FILEDROP_DELETE_AFTER_INGEST") {
            d.delete_after_ingest = v == "1" || v.eq_ignore_ascii_case("true");
        }
        d
    }
}

/// The file-drop watcher. Cloneable; `run` consumes self.
#[derive(Clone)]
pub struct FiledropWatcher {
    catalog: Arc<dyn Catalog>,
    store: Arc<dyn ObjectStore>,
    write_path: WritePath,
    config: FiledropConfig,
}

impl FiledropWatcher {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        store: Arc<dyn ObjectStore>,
        write_path: WritePath,
        config: FiledropConfig,
    ) -> Self {
        Self {
            catalog,
            store,
            write_path,
            config,
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(
            prefix = %self.config.prefix,
            poll_secs = self.config.poll_interval.as_secs(),
            "filedrop watcher starting"
        );
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    info!("filedrop watcher shutting down");
                    return;
                }
                _ = tokio::time::sleep(self.config.poll_interval) => {
                    if let Err(e) = self.tick().await {
                        warn!(error = %e, "filedrop tick failed");
                    }
                }
            }
        }
    }

    #[instrument(skip(self))]
    async fn tick(&self) -> Result<()> {
        let prefix_path = Path::from(self.config.prefix.clone());
        let mut stream = self.store.list(Some(&prefix_path));
        let mut seen = 0usize;
        let mut processed = 0usize;
        while let Some(obj) = stream.next().await {
            let obj = obj.map_err(|e| {
                Error::Internal(format!("list {}: {e}", self.config.prefix))
            })?;
            seen += 1;
            match self.process_one(&obj.location).await {
                Ok(true) => processed += 1,
                Ok(false) => {} // replayed from ledger; not counted as new work
                Err(e) => {
                    warn!(path = %obj.location, error = %e, "filedrop: failed to process file");
                }
            }
        }
        if seen > 0 {
            debug!(seen, processed, "filedrop scan complete");
        }
        Ok(())
    }

    /// Returns `Ok(true)` if the file was newly ingested, `Ok(false)` if it
    /// was an idempotency-ledger replay.
    async fn process_one(&self, path: &Path) -> Result<bool> {
        // 1. Parse database + table from the path. Convention:
        //    `{prefix}/{database}/{table}/{filename}`
        let (database, table, filename) = match split_path(&self.config.prefix, path) {
            Some(x) => x,
            None => {
                debug!(path = %path, "filedrop: skipping (does not match prefix/database/table/file)");
                return Ok(false);
            }
        };

        // 2. Download bytes + SHA256.
        let get = self.store.get(path).await.map_err(|e| {
            Error::Internal(format!("get {path}: {e}"))
        })?;
        let bytes = get.bytes().await.map_err(|e| {
            Error::Internal(format!("read {path}: {e}"))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let sha_hex = hex_encode(&digest);
        let idem_key = format!("filedrop:{sha_hex}");

        // 3. Lookup the target table.
        let table_ref = self.catalog.lookup_table(&database, &table).await?;

        // 4. Parse by extension.
        let ext = filename
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let batches: Vec<RecordBatch> = match ext.as_str() {
            "ndjson" | "jsonl" | "json" => parse_ndjson(&bytes, &table_ref.schema)?,
            other => {
                return Err(Error::Internal(format!(
                    "filedrop: unsupported extension `.{other}` (MVP supports ndjson/jsonl/json)"
                )));
            }
        };
        if batches.is_empty() {
            // Still record the ledger entry so we don't re-download an
            // empty file on every poll.
            let _ = self
                .catalog
                .record_idempotency(
                    &idem_key,
                    kyma_core::catalog::IngestLedgerEntry {
                        table_id: table_ref.id,
                        snapshot_id: table_ref.current_snapshot_id,
                        rows_ingested: 0,
                        bytes_written: 0,
                        applied_at: chrono::Utc::now(),
                    },
                    chrono::Duration::hours(24 * 30),
                )
                .await?;
            return Ok(false);
        }

        // 5. Ingest via WritePath with the SHA-keyed idempotency header.
        //    If the ledger already has this SHA, write_path returns
        //    `replayed: true` without writing another extent.
        let ack = self
            .write_path
            .ingest_with_idempotency(&table_ref, batches, Some(&idem_key))
            .await?;

        ::metrics::counter!("kyma_filedrop_objects_processed_total",
            "replayed" => ack.replayed.to_string()).increment(1);
        ::metrics::counter!("kyma_filedrop_rows_total", "table" => table.clone())
            .increment(ack.rows_ingested);

        if self.config.delete_after_ingest && !ack.replayed {
            if let Err(e) = self.store.delete(path).await {
                warn!(path = %path, error = %e, "filedrop: delete after ingest failed");
            }
        }

        info!(
            path = %path,
            database = %database,
            table = %table,
            rows = ack.rows_ingested,
            replayed = ack.replayed,
            "filedrop: ingest complete"
        );
        Ok(!ack.replayed)
    }
}

/// `{prefix}/{database}/{table}/{filename}` → `(database, table, filename)`.
fn split_path(prefix: &str, path: &Path) -> Option<(String, String, String)> {
    let s = path.as_ref();
    let prefix = prefix.trim_end_matches('/');
    let s = s.strip_prefix(prefix)?.trim_start_matches('/');
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() < 3 {
        return None;
    }
    let database = parts[0].to_owned();
    let table = parts[1].to_owned();
    let filename = parts[2..].join("/"); // allow nested subdirs, preserve
    if database.is_empty() || table.is_empty() || filename.is_empty() {
        return None;
    }
    Some((database, table, filename))
}

/// Parse an NDJSON body into `RecordBatch`es according to the table's schema.
/// Uses `arrow::json::ReaderBuilder`, the same path REST ingest uses.
fn parse_ndjson(bytes: &[u8], schema: &Arc<Schema>) -> Result<Vec<RecordBatch>> {
    use arrow::json::ReaderBuilder;
    let cursor = Cursor::new(bytes.to_vec());
    let reader = ReaderBuilder::new(schema.clone())
        .build(cursor)
        .map_err(|e| Error::Internal(format!("ndjson reader: {e}")))?;
    let mut batches = Vec::new();
    for r in reader {
        let b = r.map_err(|e| Error::Internal(format!("ndjson decode: {e}")))?;
        batches.push(b);
    }
    Ok(batches)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}
