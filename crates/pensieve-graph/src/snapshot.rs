//! Persistent graph-topology snapshots (S3.2): store a built [`CsrGraph`] as an
//! object-store sidecar and load it back.
//!
//! A snapshot is the checksummed `KYCS` serialization of a graph's CSR topology
//! ([`CsrGraph::to_bytes`]) written to a deterministic per-graph key. It lets a
//! deep traversal reuse a prebuilt topology across process restarts and across
//! query nodes (the in-process topo cache only helps within one process), and
//! is the substrate the `graph_snapshot` fabric job will refresh. Building the
//! CSR from a live graph's edges and a catalog registry for versioned snapshots
//! are the activation layers above this; this module is the storage primitive.

use std::sync::Arc;

use pensieve_graph_topo::CsrGraph;
use object_store::path::Path;
use object_store::ObjectStore;

/// Deterministic object key for a graph's latest topology snapshot:
/// `graphs/<database>/<graph>/topo.csr`. Rebuilds overwrite in place.
pub fn snapshot_path(database: &str, graph: &str) -> String {
    format!("graphs/{database}/{graph}/topo.csr")
}

/// Object key for a snapshot's freshness marker, sitting beside the snapshot:
/// `graphs/<database>/<graph>/topo.csr.meta`. It records the *source* version
/// the snapshot was built from (the edge table's `current_snapshot_id`), so a
/// scheduler can debounce rebuilds: if the live source version still matches the
/// marker, the snapshot is current and no rebuild is needed.
pub fn meta_path(database: &str, graph: &str) -> String {
    format!("{}.meta", snapshot_path(database, graph))
}

/// Record `source_version` (the edge table's snapshot id at build time) at
/// `path`, overwriting any prior marker. Stored as a raw UTF-8 string.
pub async fn store_snapshot_meta(
    store: &Arc<dyn ObjectStore>,
    path: &str,
    source_version: &str,
) -> anyhow::Result<()> {
    store
        .put(&Path::from(path), source_version.as_bytes().to_vec().into())
        .await?;
    Ok(())
}

/// Load the source-version marker at `path`, or `None` when no marker exists
/// yet (no snapshot built, or one built before markers were written → treated
/// as stale, so the next sweep rebuilds and writes the marker).
pub async fn load_snapshot_meta(
    store: &Arc<dyn ObjectStore>,
    path: &str,
) -> anyhow::Result<Option<String>> {
    match store.get(&Path::from(path)).await {
        Ok(res) => {
            let bytes = res.bytes().await?;
            Ok(Some(String::from_utf8_lossy(&bytes).trim().to_string()))
        }
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Serialize `csr` and write it to `path` in `store` (overwriting any prior
/// snapshot at that key).
pub async fn store_snapshot(
    store: &Arc<dyn ObjectStore>,
    path: &str,
    csr: &CsrGraph,
) -> anyhow::Result<()> {
    let bytes = csr.to_bytes();
    store.put(&Path::from(path), bytes.into()).await?;
    Ok(())
}

/// Load and decode the snapshot at `path`, or `None` when no snapshot exists
/// there yet. A corrupt/incompatible payload is a hard error (the caller should
/// rebuild) rather than a silent miss.
pub async fn load_snapshot(
    store: &Arc<dyn ObjectStore>,
    path: &str,
) -> anyhow::Result<Option<CsrGraph>> {
    match store.get(&Path::from(path)).await {
        Ok(res) => {
            let bytes = res.bytes().await?;
            let csr = CsrGraph::from_bytes(&bytes)
                .map_err(|e| anyhow::anyhow!("decode graph snapshot {path}: {e}"))?;
            Ok(Some(csr))
        }
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pensieve_graph_topo::Direction;
    use object_store::memory::InMemory;

    fn sample() -> CsrGraph {
        CsrGraph::build(
            ["a", "b", "c", "d"],
            [
                ("a", "b", "KNOWS"),
                ("b", "c", "KNOWS"),
                ("c", "d", "CALLS"),
            ],
        )
    }

    #[tokio::test]
    async fn round_trips_through_object_store() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let path = snapshot_path("default", "memory");
        let csr = sample();

        // Missing snapshot → None.
        assert!(load_snapshot(&store, &path).await.unwrap().is_none());

        store_snapshot(&store, &path, &csr).await.unwrap();
        let loaded = load_snapshot(&store, &path)
            .await
            .unwrap()
            .expect("present");

        // Identical topology, and traversal matches the original end-to-end.
        assert_eq!(loaded, csr);
        assert_eq!(
            loaded.k_hop(&["a"], 3, Direction::Fwd, None),
            csr.k_hop(&["a"], 3, Direction::Fwd, None)
        );
        assert_eq!(
            loaded.shortest_path_len("a", "d", Direction::Fwd, None),
            Some(3)
        );

        // Overwrite with a different topology → the new one loads.
        let csr2 = CsrGraph::build(["x", "y"], [("x", "y", "E")]);
        store_snapshot(&store, &path, &csr2).await.unwrap();
        assert_eq!(load_snapshot(&store, &path).await.unwrap().unwrap(), csr2);
    }

    #[tokio::test]
    async fn corrupt_payload_is_an_error_not_a_silent_miss() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let path = "graphs/db/g/topo.csr";
        store
            .put(&Path::from(path), b"not a KYCS blob".to_vec().into())
            .await
            .unwrap();
        assert!(load_snapshot(&store, path).await.is_err());
    }

    #[test]
    fn path_is_deterministic_and_namespaced() {
        assert_eq!(snapshot_path("db1", "memory"), "graphs/db1/memory/topo.csr");
        assert_eq!(
            meta_path("db1", "memory"),
            "graphs/db1/memory/topo.csr.meta"
        );
    }

    #[tokio::test]
    async fn meta_marker_round_trips_and_missing_is_none() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let path = meta_path("default", "memory");

        // No marker yet → None (treated as stale by the scheduler).
        assert!(load_snapshot_meta(&store, &path).await.unwrap().is_none());

        store_snapshot_meta(&store, &path, "snap-42").await.unwrap();
        assert_eq!(
            load_snapshot_meta(&store, &path).await.unwrap().as_deref(),
            Some("snap-42")
        );

        // Overwrite with a newer source version.
        store_snapshot_meta(&store, &path, "snap-43").await.unwrap();
        assert_eq!(
            load_snapshot_meta(&store, &path).await.unwrap().as_deref(),
            Some("snap-43")
        );
    }
}
