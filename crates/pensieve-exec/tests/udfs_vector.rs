//! Unit-test the three vector UDFs against FixedSizeList<Float32, N> columns
//! using DataFusion's in-memory MemTable.

use arrow_array::{FixedSizeListArray, Float32Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

async fn setup() -> SessionContext {
    let ctx = SessionContext::new();
    pensieve_exec::register_vector_udfs(&ctx);

    // Table: id STRING, v FixedSizeList<Float32, 3>
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new(
            "v",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, false)), 3),
            false,
        ),
    ]));
    let ids = StringArray::from(vec!["a", "b", "c"]);
    let values = Float32Array::from(vec![
        1.0, 0.0, 0.0, // a = [1, 0, 0]
        0.0, 1.0, 0.0, // b = [0, 1, 0]
        1.0, 1.0, 0.0, // c = [1, 1, 0]
    ]);
    let list = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, false)),
        3,
        Arc::new(values),
        None,
    );
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(list)]).unwrap();
    let mem = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
    ctx.register_table("vecs", Arc::new(mem)).unwrap();
    ctx
}

#[tokio::test]
async fn cosine_distance_against_literal() {
    let ctx = setup().await;
    // cosine_distance(a, a) = 0; cosine_distance(a, b) = 1 (orthogonal).
    // Use a subquery to construct the literal FixedSizeList: DataFusion
    // doesn't have direct FixedSizeList literal syntax, so we compute
    // distance between two columns by self-joining via id.
    let df = ctx
        .sql(
            "SELECT x.id, cosine_distance(x.v, y.v) AS d
         FROM vecs x, vecs y
         WHERE x.id = 'a' AND y.id = 'a'",
        )
        .await
        .unwrap();
    let batches = df.collect().await.unwrap();
    let d: f64 = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .unwrap()
        .value(0);
    assert!((d - 0.0).abs() < 1e-9, "cosine(a,a) should be 0; got {d}");
}

#[tokio::test]
async fn l2_distance_triangle() {
    let ctx = setup().await;
    let df = ctx
        .sql(
            "SELECT x.id, y.id AS y_id, l2_distance(x.v, y.v) AS d
         FROM vecs x, vecs y
         WHERE x.id = 'a' AND (y.id = 'b' OR y.id = 'c')
         ORDER BY y.id",
        )
        .await
        .unwrap();
    let batches = df.collect().await.unwrap();
    let ds = batches[0]
        .column(2)
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .unwrap();
    // l2(a=[1,0,0], b=[0,1,0]) = sqrt(2)
    assert!((ds.value(0) - 2.0_f64.sqrt()).abs() < 1e-9);
    // l2(a=[1,0,0], c=[1,1,0]) = 1
    assert!((ds.value(1) - 1.0).abs() < 1e-9);
}

#[tokio::test]
async fn inner_product_sanity() {
    let ctx = setup().await;
    let df = ctx
        .sql(
            "SELECT inner_product(x.v, y.v) AS d
         FROM vecs x, vecs y
         WHERE x.id = 'a' AND y.id = 'c'",
        )
        .await
        .unwrap();
    let batches = df.collect().await.unwrap();
    let d: f64 = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .unwrap()
        .value(0);
    // inner([1,0,0], [1,1,0]) = 1
    assert!((d - 1.0).abs() < 1e-9, "got {d}");
}
