//! Property-based roundtrip tests for the telemetry format.
//!
//! For each generated case:
//!   1. Build an Arrow `RecordBatch` with randomized values and null masks.
//!   2. Run it through `TelemetryFormat::start_extent(...).append(batch).finish()`
//!      against an in-memory `ObjectStore`.
//!   3. Re-open the extent via `open_extent(...)` and read every block.
//!   4. Assert the decoded batch is equal to the original (value + null bitmap).
//!
//! These tests catch format-layer bugs in minutes rather than production.
//! They run on every `cargo test` — no Docker, no network.

use arrow_array::builder::{
    BooleanBuilder, Float64Builder, Int32Builder, Int64Builder, StringBuilder,
    TimestampNanosecondBuilder,
};
use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use kyma_core::segment_format::{ColumnId, OpenExtentInput, SegmentFormat};
use kyma_core::types::TableId;
use kyma_format_tlm::TelemetryFormat;
use object_store::memory::InMemory;
use proptest::prelude::*;
use std::sync::Arc;

// ------------------------------------------------------------------
// Schema under test
// ------------------------------------------------------------------

fn standard_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new("int_col", DataType::Int32, true),
        Field::new("long_col", DataType::Int64, true),
        Field::new("real_col", DataType::Float64, true),
        Field::new("bool_col", DataType::Boolean, true),
        Field::new("string_col", DataType::Utf8, true),
    ]))
}

// ------------------------------------------------------------------
// Row strategy → RecordBatch
// ------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Row {
    timestamp: Option<i64>,
    int_col: Option<i32>,
    long_col: Option<i64>,
    real_col: Option<f64>,
    bool_col: Option<bool>,
    string_col: Option<String>,
}

fn row_strategy() -> impl Strategy<Value = Row> {
    (
        prop::option::weighted(0.95, 0i64..2_000_000_000_000_000_000i64),
        prop::option::weighted(0.95, any::<i32>()),
        prop::option::weighted(0.95, any::<i64>()),
        prop::option::weighted(0.95, prop::num::f64::NORMAL),
        prop::option::weighted(0.95, any::<bool>()),
        prop::option::weighted(0.95, "[ -~]{0,64}"),
    )
        .prop_map(|(t, i, l, r, b, s)| Row {
            timestamp: t,
            int_col: i,
            long_col: l,
            real_col: r,
            bool_col: b,
            string_col: s,
        })
}

fn build_batch(rows: &[Row]) -> RecordBatch {
    let mut ts = TimestampNanosecondBuilder::with_capacity(rows.len());
    let mut ic = Int32Builder::with_capacity(rows.len());
    let mut lc = Int64Builder::with_capacity(rows.len());
    let mut rc = Float64Builder::with_capacity(rows.len());
    let mut bc = BooleanBuilder::with_capacity(rows.len());
    let mut sc = StringBuilder::with_capacity(rows.len(), rows.len() * 16);
    for r in rows {
        match r.timestamp {
            Some(v) => ts.append_value(v),
            None => ts.append_null(),
        }
        match r.int_col {
            Some(v) => ic.append_value(v),
            None => ic.append_null(),
        }
        match r.long_col {
            Some(v) => lc.append_value(v),
            None => lc.append_null(),
        }
        match r.real_col {
            Some(v) => rc.append_value(v),
            None => rc.append_null(),
        }
        match r.bool_col {
            Some(v) => bc.append_value(v),
            None => bc.append_null(),
        }
        match &r.string_col {
            Some(v) => sc.append_value(v),
            None => sc.append_null(),
        }
    }
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(ts.finish()),
        Arc::new(ic.finish()),
        Arc::new(lc.finish()),
        Arc::new(rc.finish()),
        Arc::new(bc.finish()),
        Arc::new(sc.finish()),
    ];
    RecordBatch::try_new(standard_schema(), arrays).expect("record batch construction")
}

// ------------------------------------------------------------------
// Roundtrip helper
// ------------------------------------------------------------------

async fn roundtrip(batches: Vec<RecordBatch>) -> Result<(), String> {
    let store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
    let format = TelemetryFormat::new(store.clone(), "test-prefix");
    let schema = standard_schema();

    let mut writer = format
        .start_extent(schema.clone(), 0)
        .await
        .map_err(|e| format!("start_extent: {e}"))?;
    let expected: Vec<RecordBatch> = batches.clone();
    let expected_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    for b in batches {
        writer
            .append(b)
            .await
            .map_err(|e| format!("append: {e}"))?;
    }
    let result = writer.finish().await.map_err(|e| format!("finish: {e}"))?;

    assert_eq!(
        result.row_count as usize, expected_rows,
        "writer reported row count differs from input"
    );

    let reader = format
        .open_extent(OpenExtentInput {
            extent_id: result.extent_id,
            table_id: TableId::new(),
            schema: schema.clone(),
            object_path: result.object_path.clone(),
            byte_size: result.byte_size,
        })
        .await
        .map_err(|e| format!("open_extent: {e}"))?;

    let blocks = reader
        .pruned_blocks(&kyma_core::segment_format::BlockPredicate::All)
        .await
        .map_err(|e| format!("pruned_blocks: {e}"))?;

    assert_eq!(
        blocks.len(),
        expected.len(),
        "block count should match append count"
    );

    let projection: Vec<ColumnId> = (0..schema.fields().len() as u32).map(ColumnId).collect();
    for (i, bid) in blocks.into_iter().enumerate() {
        let got = reader
            .read_block(bid, &projection)
            .await
            .map_err(|e| format!("read_block[{i}]: {e}"))?;
        let want = &expected[i];
        if got.num_rows() != want.num_rows() {
            return Err(format!(
                "block {i} row count mismatch: want {}, got {}",
                want.num_rows(),
                got.num_rows()
            ));
        }
        if got.num_columns() != want.num_columns() {
            return Err(format!(
                "block {i} column count mismatch: want {}, got {}",
                want.num_columns(),
                got.num_columns()
            ));
        }
        for col in 0..want.num_columns() {
            if !arrays_equal(got.column(col), want.column(col)) {
                return Err(format!(
                    "block {i} column {col} ({}) differs after roundtrip",
                    want.schema().field(col).name()
                ));
            }
        }
    }

    Ok(())
}

fn arrays_equal(a: &dyn Array, b: &dyn Array) -> bool {
    a.data_type() == b.data_type() && a.len() == b.len() && {
        let a_data = a.to_data();
        let b_data = b.to_data();
        a_data == b_data
    }
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[tokio::test]
async fn empty_extent_roundtrips() {
    roundtrip(vec![build_batch(&[])]).await.unwrap();
}

#[tokio::test]
async fn single_row_roundtrips() {
    let rows = vec![Row {
        timestamp: Some(1_700_000_000_000_000_000),
        int_col: Some(42),
        long_col: Some(9_000_000_000_000_000_000),
        real_col: Some(std::f64::consts::PI),
        bool_col: Some(true),
        string_col: Some("hello".into()),
    }];
    roundtrip(vec![build_batch(&rows)]).await.unwrap();
}

#[tokio::test]
async fn all_nulls_roundtrip() {
    let rows = vec![
        Row {
            timestamp: None,
            int_col: None,
            long_col: None,
            real_col: None,
            bool_col: None,
            string_col: None,
        };
        10
    ];
    roundtrip(vec![build_batch(&rows)]).await.unwrap();
}

#[tokio::test]
async fn multi_block_roundtrip() {
    let a = build_batch(&(0..32)
        .map(|i| Row {
            timestamp: Some(1_700_000_000_000_000_000 + (i as i64) * 1_000_000_000),
            int_col: Some(i),
            long_col: Some(i as i64 * 10),
            real_col: Some(i as f64 * 0.5),
            bool_col: Some(i % 2 == 0),
            string_col: Some(format!("a-{i}")),
        })
        .collect::<Vec<_>>());
    let b = build_batch(&(0..17)
        .map(|i| Row {
            timestamp: Some(1_800_000_000_000_000_000 + (i as i64) * 1_000_000_000),
            int_col: Some(i + 1000),
            long_col: None,
            real_col: Some(-(i as f64)),
            bool_col: Some(false),
            string_col: Some(format!("b-{i}")),
        })
        .collect::<Vec<_>>());
    roundtrip(vec![a, b]).await.unwrap();
}

#[tokio::test]
async fn string_boundary_values_roundtrip() {
    let rows = vec![
        Row {
            timestamp: Some(0),
            int_col: Some(0),
            long_col: Some(0),
            real_col: Some(0.0),
            bool_col: Some(false),
            string_col: Some(String::new()),
        },
        Row {
            timestamp: Some(i64::MAX / 2),
            int_col: Some(i32::MAX),
            long_col: Some(i64::MAX),
            real_col: Some(f64::MAX),
            bool_col: Some(true),
            string_col: Some("a".repeat(10_000)),
        },
        Row {
            timestamp: Some(-1),
            int_col: Some(i32::MIN),
            long_col: Some(i64::MIN),
            real_col: Some(f64::MIN),
            bool_col: Some(false),
            // Full printable-ASCII range.
            string_col: Some((32u8..127u8).map(|c| c as char).collect()),
        },
    ];
    roundtrip(vec![build_batch(&rows)]).await.unwrap();
}

// Property-based: random row counts, nullability, values.
proptest! {
    #![proptest_config(ProptestConfig { cases: 48, max_shrink_iters: 1024, .. ProptestConfig::default() })]

    #[test]
    fn prop_single_block_roundtrip(rows in prop::collection::vec(row_strategy(), 0..128)) {
        let batch = build_batch(&rows);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                roundtrip(vec![batch]).await
            })
            .unwrap_or_else(|e| panic!("roundtrip failed: {e}"));
    }

    #[test]
    fn prop_multi_block_roundtrip(
        blocks in prop::collection::vec(prop::collection::vec(row_strategy(), 0..48), 1..6)
    ) {
        let batches: Vec<RecordBatch> = blocks.iter().map(|rows| build_batch(rows)).collect();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                roundtrip(batches).await
            })
            .unwrap_or_else(|e| panic!("multi-block roundtrip failed: {e}"));
    }
}
