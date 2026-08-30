//! Parquet SegmentFormat: round-trip, identical column_stats vs TLM, and the
//! compression win — all over an `InMemory` store, no Docker.
//!
//! 1. Write the same batches through `ParquetFormat` and `TelemetryFormat`.
//! 2. Assert the two `column_stats` payloads are byte-identical (the shared
//!    accumulator → catalog pruning + the ANN vec-stats gate behave the same
//!    regardless of format).
//! 3. Read every block back through `open_extent` + `read_block` and assert the
//!    rows survive the round trip.
//! 4. Assert the Parquet object is materially smaller than the Arrow-IPC TLM
//!    object (ZSTD columnar encoding) on compressible data.

use std::sync::Arc;

use arrow_array::builder::{FixedSizeListBuilder, Float32Builder};
use arrow_array::{Int64Array, RecordBatch, StringArray, TimestampNanosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use pensieve_core::segment_format::FormatRegistry;
use pensieve_core::segment_format::{BlockPredicate, ColumnId, OpenExtentInput, SegmentFormat};
use pensieve_core::types::SchemaRef;
use pensieve_format_parquet::ParquetFormat;
use pensieve_format_tlm::TelemetryFormat;
use object_store::memory::InMemory;
use object_store::ObjectStore;

const DIM: usize = 8;

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new("id", DataType::Int64, true),
        Field::new("msg", DataType::Utf8, true),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                DIM as i32,
            ),
            true,
        ),
    ]))
}

fn make_batch(base: i64, n: usize) -> RecordBatch {
    let ts = TimestampNanosecondArray::from((0..n).map(|i| base + i as i64).collect::<Vec<_>>());
    let ids = Int64Array::from((0..n).map(|i| base + i as i64).collect::<Vec<_>>());
    // A few repeating service names → compressible + small distinct set.
    let msgs: Vec<String> = (0..n)
        .map(|i| format!("service-{} connection pool event", i % 5))
        .collect();
    let msg = StringArray::from(msgs.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    let vb = Float32Builder::new();
    let mut b = FixedSizeListBuilder::new(vb, DIM as i32);
    for i in 0..n {
        for j in 0..DIM {
            b.values().append_value(((i + j) % 7) as f32 * 0.1);
        }
        b.append(true);
    }
    RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(ts),
            Arc::new(ids),
            Arc::new(msg),
            Arc::new(b.finish()),
        ],
    )
    .unwrap()
}

async fn write_all(
    format: &dyn SegmentFormat,
    batches: &[RecordBatch],
) -> (String, u64, serde_json::Value, u32, u64) {
    let mut w = format
        .start_extent(schema(), 64 * 1024 * 1024)
        .await
        .unwrap();
    for b in batches {
        w.append(b.clone()).await.unwrap();
    }
    let res = w.finish().await.unwrap();
    (
        res.object_path,
        res.byte_size,
        res.column_stats,
        res.block_count,
        res.row_count,
    )
}

#[tokio::test]
async fn parquet_roundtrip_matches_tlm_stats_and_compresses() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let pq: Arc<dyn SegmentFormat> = Arc::new(ParquetFormat::new(store.clone(), "pensieve"));
    let tlm: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store.clone(), "tlm"));

    // 3 blocks of 1000 rows.
    let batches: Vec<RecordBatch> = (0..3).map(|c| make_batch(c * 1000, 1000)).collect();

    let (pq_path, pq_bytes, pq_stats, pq_blocks, pq_rows) = write_all(pq.as_ref(), &batches).await;
    let (_tlm_path, tlm_bytes, tlm_stats, _tlm_blocks, _tlm_rows) =
        write_all(tlm.as_ref(), &batches).await;

    // (1) Identical column_stats → identical catalog pruning + ANN vec-gate.
    assert_eq!(
        pq_stats, tlm_stats,
        "Parquet column_stats must equal TLM's (shared accumulator)"
    );
    // The embedding column must carry a vec stat (the scheduler's gate signal).
    assert!(pq_stats["embedding"]["vec"]["count"].as_u64().unwrap() == 3000);

    assert_eq!(pq_rows, 3000);
    assert_eq!(pq_blocks, 3, "one row group per appended batch");

    // (2) Round-trip: read every block back and count rows + check ids.
    let reader = pq
        .open_extent(OpenExtentInput {
            extent_id: pensieve_core::types::ExtentId::new(),
            table_id: pensieve_core::types::TableId::new(),
            schema: schema(),
            object_path: pq_path,
            byte_size: pq_bytes,
        })
        .await
        .unwrap();
    assert_eq!(reader.metadata().row_count, 3000);
    assert_eq!(reader.metadata().block_count, 3);

    let blocks = reader.pruned_blocks(&BlockPredicate::All).await.unwrap();
    assert_eq!(blocks.len(), 3);
    let mut total = 0usize;
    for bid in blocks {
        // Project just the id column (ColumnId 1).
        let batch = reader.read_block(bid, &[ColumnId(1)]).await.unwrap();
        total += batch.num_rows();
        assert_eq!(batch.num_columns(), 1, "projection kept only id");
    }
    assert_eq!(total, 3000, "all rows survive the round trip");

    // Full read (no projection) preserves all 4 columns.
    let full = reader.read_block(blocks_first(), &[]).await.unwrap();
    assert_eq!(full.num_columns(), 4);
    assert_eq!(full.num_rows(), 1000);

    // (3) Compression win: ZSTD columnar Parquet is materially smaller than the
    // Arrow-IPC TLM frame on this compressible data.
    eprintln!("parquet={pq_bytes} bytes, tlm={tlm_bytes} bytes");
    assert!(
        pq_bytes < tlm_bytes,
        "parquet ({pq_bytes}) should be smaller than TLM ({tlm_bytes})"
    );
}

fn blocks_first() -> pensieve_core::segment_format::BlockId {
    pensieve_core::segment_format::BlockId(0)
}

#[tokio::test]
async fn format_registry_dispatches_by_magic() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let tlm: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store.clone(), "pensieve"));
    let pq: Arc<dyn SegmentFormat> = Arc::new(ParquetFormat::new(store.clone(), "pensieve"));

    // One extent in each format, same store.
    let batches = vec![make_batch(0, 500)];
    let (tlm_path, tlm_bytes, _, _, _) = write_all(tlm.as_ref(), &batches).await;
    let (pq_path, pq_bytes, _, _, _) = write_all(pq.as_ref(), &batches).await;

    // A registry whose WRITE format is Parquet but which also reads TLM.
    let registry = FormatRegistry::new(pq.clone(), vec![tlm.clone()]);

    // The TLM (PENSIEVE…) object is dispatched to the TLM reader…
    let r_tlm = registry
        .open_extent(OpenExtentInput {
            extent_id: pensieve_core::types::ExtentId::new(),
            table_id: pensieve_core::types::TableId::new(),
            schema: schema(),
            object_path: tlm_path,
            byte_size: tlm_bytes,
        })
        .await
        .expect("registry reads TLM by magic");
    assert_eq!(r_tlm.metadata().row_count, 500);

    // …and the Parquet (PAR1) object to the Parquet reader, through one registry.
    let r_pq = registry
        .open_extent(OpenExtentInput {
            extent_id: pensieve_core::types::ExtentId::new(),
            table_id: pensieve_core::types::TableId::new(),
            schema: schema(),
            object_path: pq_path,
            byte_size: pq_bytes,
        })
        .await
        .expect("registry reads Parquet by magic");
    assert_eq!(r_pq.metadata().row_count, 500);
    assert_eq!(
        r_pq.metadata().format_version,
        pensieve_format_parquet::CURRENT_VERSION
    );

    // start_extent uses the write format (Parquet) → new object begins PAR1.
    let (new_path, _, _, _, _) = write_all(&registry, &batches).await;
    let head = store
        .get_range(&object_store::path::Path::from(new_path.as_str()), 0..4)
        .await
        .unwrap();
    assert_eq!(&head[..], b"PAR1", "registry writes the configured format");
}
