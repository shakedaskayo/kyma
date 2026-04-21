//! DataFusion UDFs for vector distance.
//!
//! Three scalar functions operating on `FixedSizeList<Float32, N>` (or
//! `List<Float32>`) arguments:
//!   - `cosine_distance(a, b)` — 1 - cos(a, b); range [0, 2].
//!   - `l2_distance(a, b)` — Euclidean distance.
//!   - `inner_product(a, b)` — dot product.
//!
//! Output is always `Float64` so downstream arithmetic doesn't lose
//! precision. Mismatched-length pairs return NULL (not an error) so a
//! malformed row in one extent doesn't crash an entire query.

use arrow_array::{Array, FixedSizeListArray, Float64Array, ListArray};
use arrow_schema::DataType;
use datafusion::common::cast::as_float32_array;
use datafusion::error::{DataFusionError, Result as DfResult};
use datafusion::logical_expr::{
    ColumnarValue, ScalarUDF, Signature, SimpleScalarUDF, Volatility,
};
use datafusion::prelude::SessionContext;
use datafusion::scalar::ScalarValue;
use std::sync::Arc;

/// Register `cosine_distance`, `l2_distance`, `inner_product` on `ctx`.
/// Idempotent — re-registration just replaces the previous UDF.
pub fn register_vector_udfs(ctx: &SessionContext) {
    ctx.register_udf(build("cosine_distance", cosine));
    ctx.register_udf(build("l2_distance", l2));
    ctx.register_udf(build("inner_product", inner));
}

fn build(name: &str, f: fn(&[f32], &[f32]) -> Option<f64>) -> ScalarUDF {
    // Signature accepts ANY two list-like columns; per-call code path inspects
    // the runtime ColumnarValue types and decodes either FixedSizeList or List
    // of Float32. Using Signature::any(2, ...) avoids enumerating concrete
    // list types (which change with inner element nullability).
    let sig = Signature::any(2, Volatility::Immutable);
    let fun = move |args: &[ColumnarValue]| -> DfResult<ColumnarValue> {
        let (a_rows, b_rows) = (to_f32_rows(&args[0])?, to_f32_rows(&args[1])?);
        let n = a_rows.len().max(b_rows.len());
        let mut out: Vec<Option<f64>> = Vec::with_capacity(n);
        for i in 0..n {
            let a = if a_rows.len() == 1 { &a_rows[0] } else { &a_rows[i] };
            let b = if b_rows.len() == 1 { &b_rows[0] } else { &b_rows[i] };
            out.push(f(a, b));
        }
        Ok(ColumnarValue::Array(Arc::new(Float64Array::from(out))))
    };
    ScalarUDF::from(SimpleScalarUDF::new_with_signature(
        name,
        sig,
        DataType::Float64,
        Arc::new(fun),
    ))
}

fn to_f32_rows(col: &ColumnarValue) -> DfResult<Vec<Vec<f32>>> {
    match col {
        ColumnarValue::Array(arr) => {
            if let Some(fsl) = arr.as_any().downcast_ref::<FixedSizeListArray>() {
                let mut out = Vec::with_capacity(fsl.len());
                for i in 0..fsl.len() {
                    if fsl.is_null(i) {
                        out.push(Vec::new());
                        continue;
                    }
                    let row = fsl.value(i);
                    out.push(extract_f32s(row.as_ref())?);
                }
                return Ok(out);
            }
            if let Some(l) = arr.as_any().downcast_ref::<ListArray>() {
                let mut out = Vec::with_capacity(l.len());
                for i in 0..l.len() {
                    if l.is_null(i) {
                        out.push(Vec::new());
                        continue;
                    }
                    let row = l.value(i);
                    out.push(extract_f32s(row.as_ref())?);
                }
                return Ok(out);
            }
            Err(DataFusionError::Execution(format!(
                "vector UDF expected FixedSizeList<Float32> or List<Float32>; got {:?}",
                arr.data_type()
            )))
        }
        ColumnarValue::Scalar(s) => {
            // Scalar literals materialize through `make_array(...)` in user
            // queries (`SELECT cosine_distance(col, make_array(1.0, ...))`).
            // DataFusion wraps that as `ScalarValue::List(ArrayRef)` or
            // `ScalarValue::FixedSizeList(ArrayRef)` where the inner ArrayRef
            // is a 1-element list of floats. Unwrap to a single-row Vec<f32>
            // so the per-row loop treats it as a broadcastable singleton.
            let inner: Arc<dyn Array> = match s {
                ScalarValue::List(arr) => arr.clone() as Arc<dyn Array>,
                ScalarValue::LargeList(arr) => arr.clone() as Arc<dyn Array>,
                ScalarValue::FixedSizeList(arr) => arr.clone() as Arc<dyn Array>,
                other => {
                    return Err(DataFusionError::Execution(format!(
                        "vector UDF expected list-typed scalar literal; got {other:?}"
                    )));
                }
            };
            // inner is a 1-element ListArray/FixedSizeListArray; extract its
            // single row.
            let floats = if let Some(fsl) = inner.as_any().downcast_ref::<FixedSizeListArray>() {
                let row = fsl.value(0);
                extract_f32s(row.as_ref())?
            } else if let Some(l) = inner.as_any().downcast_ref::<ListArray>() {
                let row = l.value(0);
                extract_f32s(row.as_ref())?
            } else {
                return Err(DataFusionError::Execution(format!(
                    "vector UDF: scalar literal inner type not recognised: {:?}",
                    inner.data_type()
                )));
            };
            Ok(vec![floats])
        }
    }
}

fn extract_f32s(a: &dyn Array) -> DfResult<Vec<f32>> {
    // Accept Float32 directly.
    if let Ok(f) = as_float32_array(a) {
        return Ok(f.values().to_vec());
    }
    // Cast Float64 literals down to Float32 (make_array with f64 literals
    // produces Float64 inner; users shouldn't need an explicit CAST).
    if let Some(f) = a.as_any().downcast_ref::<arrow_array::Float64Array>() {
        return Ok(f.values().iter().map(|v| *v as f32).collect());
    }
    Err(DataFusionError::Execution(format!(
        "vector UDF inner element type must be Float32 or Float64; got {:?}",
        a.data_type()
    )))
}

fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for i in 0..a.len() {
        let (x, y) = (a[i] as f64, b[i] as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return None;
    }
    Some(1.0 - dot / (na.sqrt() * nb.sqrt()))
}

fn l2(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() {
        return None;
    }
    let mut s = 0f64;
    for i in 0..a.len() {
        let d = (a[i] as f64) - (b[i] as f64);
        s += d * d;
    }
    Some(s.sqrt())
}

fn inner(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() {
        return None;
    }
    let mut s = 0f64;
    for i in 0..a.len() {
        s += (a[i] as f64) * (b[i] as f64);
    }
    Some(s)
}
