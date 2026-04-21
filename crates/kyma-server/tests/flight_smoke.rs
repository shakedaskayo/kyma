//! Arrow Flight smoke test — runs against a live `kyma` binary.
//!
//! Expects:
//!   - docker-compose stack up (Postgres + MinIO)
//!   - `target/debug/kyma` present
//!   - target table `default.flight_t` created + populated (see the
//!     companion shell script `scripts/test-flight.sh` for fixture setup)
//!
//! This test uses the `arrow-flight` Rust client directly, avoiding the
//! pyarrow dependency and Python version compatibility issues.
//!
//! Ignored by default — run with `cargo test -p kyma-server --test flight_smoke -- --ignored`.

use arrow_array::RecordBatch;
use arrow_flight::decode::FlightRecordBatchStream;
use arrow_flight::flight_service_client::FlightServiceClient;
use arrow_flight::Ticket;
use futures::StreamExt;

async fn query(
    endpoint: &str,
    database: &str,
    query: &str,
    language: &str,
) -> Result<Vec<RecordBatch>, String> {
    let mut client = FlightServiceClient::connect(endpoint.to_string())
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let ticket = serde_json::json!({
        "database": database,
        "query": query,
        "language": language,
    });
    let resp = client
        .do_get(Ticket {
            ticket: serde_json::to_vec(&ticket).unwrap().into(),
        })
        .await
        .map_err(|e| format!("do_get rpc: {e}"))?;
    let stream = resp
        .into_inner()
        .map(|r| r.map_err(arrow_flight::error::FlightError::Tonic));
    let mut batches = Vec::new();
    let mut reader = FlightRecordBatchStream::new_from_flight_data(stream);
    while let Some(res) = reader.next().await {
        let batch = res.map_err(|e| format!("decode: {e}"))?;
        batches.push(batch);
    }
    Ok(batches)
}

#[tokio::test]
#[ignore = "requires a live kyma server on 127.0.0.1:9090"]
async fn flight_sql_roundtrip() {
    let batches = query(
        "http://127.0.0.1:9090",
        "default",
        "SELECT COUNT(*) AS n FROM flight_t",
        "sql",
    )
    .await
    .expect("flight query");
    assert_eq!(batches.len(), 1, "expected exactly one batch");
    let b = &batches[0];
    assert_eq!(b.num_rows(), 1);
    let col = b
        .column_by_name("n")
        .expect("column n")
        .as_any()
        .downcast_ref::<arrow_array::Int64Array>()
        .expect("int64");
    assert_eq!(col.value(0), 100);
}

#[tokio::test]
#[ignore = "requires a live kyma server on 127.0.0.1:9090"]
async fn flight_kql_roundtrip() {
    let batches = query(
        "http://127.0.0.1:9090",
        "default",
        "flight_t | where n >= 50 | count",
        "kql",
    )
    .await
    .expect("kql via flight");
    assert!(!batches.is_empty());
    let b = &batches[0];
    let col = b
        .column_by_name("Count")
        .expect("Count column")
        .as_any()
        .downcast_ref::<arrow_array::Int64Array>()
        .expect("int64");
    assert_eq!(col.value(0), 50);
}
