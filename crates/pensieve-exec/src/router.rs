//! Read-router for multi-node scale-out.
//!
//! Given a list of extents to scan + the current node id + a list of live
//! peer nodes, assigns each extent to exactly one node via rendezvous
//! hashing. Extents that land on the current node are read locally;
//! extents that land on a peer are fetched over Arrow Flight via a
//! `{kind: "extent", ...}` ticket.
//!
//! Rendezvous hashing (highest-random-weight) keeps cache locality stable
//! across node additions/removals: only `1/N` extents remap when a node
//! joins or leaves. That matters because we count on each node caching
//! the extents it's responsible for.

use arrow_array::RecordBatch;
use arrow_flight::decode::FlightRecordBatchStream;
use arrow_flight::flight_service_client::FlightServiceClient;
use arrow_flight::Ticket;
use datafusion::error::{DataFusionError, Result as DfResult};
use futures::StreamExt;
use kyma_core::catalog::{ExtentManifest, LiveNode};
use kyma_core::types::{ExtentId, NodeId};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::Hasher;

/// Where a given extent should be read from.
#[derive(Debug, Clone)]
pub enum Target {
    /// Read locally via the current format.
    Local,
    /// Read from a peer via Arrow Flight (`kind: "extent"` ticket).
    Remote { node_id: NodeId, endpoint: String },
}

/// A simple rendezvous-hash router across `self + live_peers`.
pub struct ReadRouter {
    self_id: NodeId,
    /// (node_id, endpoint). `endpoint` is None for the self entry.
    nodes: Vec<(NodeId, Option<String>)>,
}

impl ReadRouter {
    pub fn new(self_id: NodeId, live: &[LiveNode]) -> Self {
        let mut nodes: Vec<(NodeId, Option<String>)> = Vec::with_capacity(live.len() + 1);
        let mut saw_self = false;
        for n in live {
            if n.node_id == self_id {
                saw_self = true;
                nodes.push((n.node_id, None));
            } else {
                nodes.push((n.node_id, Some(endpoint_to_url(&n.endpoint))));
            }
        }
        if !saw_self {
            // Self's heartbeat row may not be visible yet; include ourselves
            // unconditionally so the router never returns zero targets.
            nodes.push((self_id, None));
        }
        // Stable order for determinism across runs (sort by uuid bytes,
        // since NodeId doesn't implement Ord).
        nodes.sort_by(|a, b| a.0.as_uuid().as_bytes().cmp(b.0.as_uuid().as_bytes()));
        Self { self_id, nodes }
    }

    /// Pick the best target for `extent_id` via rendezvous hashing.
    pub fn assign(&self, extent_id: ExtentId) -> Target {
        // In a single-node environment nodes has length 1 (self). Short-
        // circuit to avoid paying the hash cost.
        if self.nodes.len() == 1 {
            return Target::Local;
        }
        let mut best: Option<(u64, usize)> = None;
        let ext_bytes = extent_id.as_uuid().as_bytes();
        for (i, (node_id, _)) in self.nodes.iter().enumerate() {
            let mut h = DefaultHasher::new();
            h.write(ext_bytes);
            h.write(node_id.as_uuid().as_bytes());
            let score = h.finish();
            if best.map_or(true, |(b, _)| score > b) {
                best = Some((score, i));
            }
        }
        let (_, idx) = best.expect("router has at least one node");
        let (node_id, endpoint) = &self.nodes[idx];
        if *node_id == self.self_id {
            Target::Local
        } else {
            Target::Remote {
                node_id: *node_id,
                endpoint: endpoint.clone().expect("peer entry has endpoint"),
            }
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// Partition a manifest list into (local, remote-by-node).
pub fn partition(
    router: &ReadRouter,
    manifests: Vec<ExtentManifest>,
) -> (Vec<ExtentManifest>, HashMap<NodeId, (String, Vec<ExtentManifest>)>) {
    let mut local: Vec<ExtentManifest> = Vec::new();
    let mut remote: HashMap<NodeId, (String, Vec<ExtentManifest>)> = HashMap::new();
    for m in manifests {
        match router.assign(m.id) {
            Target::Local => local.push(m),
            Target::Remote { node_id, endpoint } => {
                remote
                    .entry(node_id)
                    .or_insert_with(|| (endpoint, Vec::new()))
                    .1
                    .push(m);
            }
        }
    }
    (local, remote)
}

/// Fetch a list of extents from a peer node via Arrow Flight. Falls back
/// to `Err` on transport failure — the caller is expected to retry
/// locally (to preserve correctness at the cost of cache locality).
pub async fn fetch_remote_extents(
    endpoint: String,
    database: String,
    table_name: String,
    manifests: Vec<ExtentManifest>,
) -> DfResult<Vec<RecordBatch>> {
    let mut client = FlightServiceClient::connect(endpoint.clone())
        .await
        .map_err(|e| DataFusionError::External(Box::new(std::io::Error::other(format!(
            "flight connect {endpoint}: {e}"
        )))))?;

    let mut batches: Vec<RecordBatch> = Vec::new();
    for m in manifests {
        let ticket = serde_json::json!({
            "kind": "extent",
            "database": database,
            "table": table_name,
            "object_path": m.object_path,
            "byte_size": m.byte_size,
        });
        let resp = client
            .do_get(Ticket {
                ticket: serde_json::to_vec(&ticket).unwrap().into(),
            })
            .await
            .map_err(|e| DataFusionError::External(Box::new(std::io::Error::other(format!(
                "flight do_get: {e}"
            )))))?;
        let stream = resp
            .into_inner()
            .map(|r| r.map_err(arrow_flight::error::FlightError::Tonic));
        let mut reader = FlightRecordBatchStream::new_from_flight_data(stream);
        while let Some(res) = reader.next().await {
            let batch = res.map_err(|e| {
                DataFusionError::External(Box::new(std::io::Error::other(format!(
                    "flight decode: {e}"
                ))))
            })?;
            batches.push(batch);
        }
        ::metrics::counter!(
            "kyma_scan_extents_remote_total",
            "peer" => endpoint.clone()
        )
        .increment(1);
    }
    Ok(batches)
}

/// Normalize an endpoint string (`host:port`) into a gRPC URL.
fn endpoint_to_url(endpoint: &str) -> String {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("http://{endpoint}")
    }
}
