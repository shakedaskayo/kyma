//! Kyma's own spans → its own `otel_traces` table, in-process.
//!
//! The exporter is installed into the tracing stack at process start but
//! only begins writing once wired (`handle().set(SelfTraceCtx{…})`) to a
//! catalog + WritePath (they are built later in startup). Until then
//! batches are dropped — never buffered, never blocking.
//!
//! Recursion guard lives one level up: the tracing-opentelemetry layer is
//! filtered to `target = "kyma_telemetry"` spans only, and nothing in the
//! ingest/storage path uses that target.

use crate::traces::{otel_traces_schema, OTEL_TRACES_TABLE};
use arrow_array::builder::{Int64Builder, StringBuilder, TimestampNanosecondBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use futures::future::BoxFuture;
use kyma_core::catalog::Catalog;
use kyma_ingest_core::WritePath;
use opentelemetry::trace::{SpanId, SpanKind, Status as OtelStatus};
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

/// Storage wiring, set once when the server's write path is ready.
pub struct SelfTraceCtx {
    pub catalog: Arc<dyn Catalog>,
    pub write_path: WritePath,
    pub database: String,
}

/// A [`SpanExporter`] that writes Kyma's own spans straight onto the shared
/// `otel_traces` schema through the in-process [`WritePath`] — no loopback
/// gRPC hop.
#[derive(Clone)]
pub struct SelfTraceExporter {
    ctx: Arc<OnceLock<SelfTraceCtx>>,
    service_name: String,
}

impl std::fmt::Debug for SelfTraceExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelfTraceExporter")
            .field("service_name", &self.service_name)
            .field("wired", &self.ctx.get().is_some())
            .finish()
    }
}

impl SelfTraceExporter {
    /// Create an exporter with no storage attached; pair with [`Self::handle`].
    pub fn unwired() -> Self {
        Self {
            ctx: Arc::new(OnceLock::new()),
            service_name: "kyma-server".to_string(),
        }
    }

    /// The shared slot to wire later: `handle.set(SelfTraceCtx{…})`.
    pub fn handle(&self) -> Arc<OnceLock<SelfTraceCtx>> {
        self.ctx.clone()
    }
}

fn ns(t: std::time::SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn kind_label(k: &SpanKind) -> &'static str {
    match k {
        SpanKind::Internal => "INTERNAL",
        SpanKind::Server => "SERVER",
        SpanKind::Client => "CLIENT",
        SpanKind::Producer => "PRODUCER",
        SpanKind::Consumer => "CONSUMER",
    }
}

/// Pure mapping: a batch of SDK spans → one RecordBatch on the shared schema.
pub fn spans_to_batch(batch: &[SpanData], service_name: &str) -> anyhow::Result<RecordBatch> {
    let cap = batch.len();

    let mut start_b = TimestampNanosecondBuilder::with_capacity(cap);
    let mut end_b = TimestampNanosecondBuilder::with_capacity(cap);
    let mut dur_b = Int64Builder::with_capacity(cap);
    let mut trace_id_b = StringBuilder::with_capacity(cap, cap * 32);
    let mut span_id_b = StringBuilder::with_capacity(cap, cap * 16);
    let mut parent_span_id_b = StringBuilder::with_capacity(cap, cap * 16);
    let mut name_b = StringBuilder::with_capacity(cap, cap * 32);
    let mut kind_b = StringBuilder::with_capacity(cap, cap * 8);
    let mut status_code_b = StringBuilder::with_capacity(cap, cap * 8);
    let mut status_msg_b = StringBuilder::with_capacity(cap, cap * 32);
    let mut service_b = StringBuilder::with_capacity(cap, cap * 16);
    let mut subject_b = StringBuilder::with_capacity(cap, cap * 16);
    let mut tenant_b = StringBuilder::with_capacity(cap, cap * 16);
    let mut attrs_b = StringBuilder::with_capacity(cap, cap * 64);
    let mut resource_b = StringBuilder::with_capacity(cap, cap * 8);

    for sp in batch {
        // start / end / duration
        let start = ns(sp.start_time);
        let end = ns(sp.end_time);
        start_b.append_value(start);
        end_b.append_value(end);
        dur_b.append_value((end - start).max(0));

        // trace_id / span_id — Display is lowercase hex.
        trace_id_b.append_value(sp.span_context.trace_id().to_string());
        span_id_b.append_value(sp.span_context.span_id().to_string());
        // parent_span_id: null for root spans
        if sp.parent_span_id == SpanId::INVALID {
            parent_span_id_b.append_null();
        } else {
            parent_span_id_b.append_value(sp.parent_span_id.to_string());
        }

        // name
        if sp.name.is_empty() {
            name_b.append_null();
        } else {
            name_b.append_value(sp.name.as_ref());
        }

        // kind
        kind_b.append_value(kind_label(&sp.span_kind));

        // status
        match &sp.status {
            OtelStatus::Ok => {
                status_code_b.append_value("OK");
                status_msg_b.append_null();
            }
            OtelStatus::Error { description } => {
                status_code_b.append_value("ERROR");
                if description.is_empty() {
                    status_msg_b.append_null();
                } else {
                    status_msg_b.append_value(description.as_ref());
                }
            }
            OtelStatus::Unset => {
                status_code_b.append_value("UNSET");
                status_msg_b.append_null();
            }
        }

        // service_name: same for every row of a self-export batch
        service_b.append_value(service_name);

        // kyma.subject / kyma.tenant promoted; the rest → attributes_json
        let mut subject: Option<String> = None;
        let mut tenant: Option<String> = None;
        let mut rest = serde_json::Map::new();
        for kv in &sp.attributes {
            let val = kv.value.to_string();
            match kv.key.as_str() {
                "kyma.subject" => subject = Some(val),
                "kyma.tenant" => tenant = Some(val),
                key => {
                    rest.insert(key.to_string(), serde_json::Value::String(val));
                }
            }
        }
        match subject {
            Some(s) => subject_b.append_value(&s),
            None => subject_b.append_null(),
        }
        match tenant {
            Some(t) => tenant_b.append_value(&t),
            None => tenant_b.append_null(),
        }
        attrs_b.append_value(
            serde_json::to_string(&serde_json::Value::Object(rest))
                .unwrap_or_else(|_| "{}".into()),
        );
        resource_b.append_value("{}");
    }

    let arrays: Vec<ArrayRef> = vec![
        Arc::new(start_b.finish()),
        Arc::new(end_b.finish()),
        Arc::new(dur_b.finish()),
        Arc::new(trace_id_b.finish()),
        Arc::new(span_id_b.finish()),
        Arc::new(parent_span_id_b.finish()),
        Arc::new(name_b.finish()),
        Arc::new(kind_b.finish()),
        Arc::new(status_code_b.finish()),
        Arc::new(status_msg_b.finish()),
        Arc::new(service_b.finish()),
        Arc::new(subject_b.finish()),
        Arc::new(tenant_b.finish()),
        Arc::new(attrs_b.finish()),
        Arc::new(resource_b.finish()),
    ];

    RecordBatch::try_new(otel_traces_schema(), arrays).map_err(Into::into)
}

impl SpanExporter for SelfTraceExporter {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let ctx = self.ctx.clone();
        let service_name = self.service_name.clone();
        Box::pin(async move {
            let Some(ctx) = ctx.get() else {
                return Ok(()); // unwired — drop
            };
            let rb = match spans_to_batch(&batch, &service_name) {
                Ok(rb) if rb.num_rows() > 0 => rb,
                _ => return Ok(()),
            };
            let table = match crate::ensure_otel_table(
                &ctx.catalog,
                &ctx.database,
                OTEL_TRACES_TABLE,
                otel_traces_schema(),
            )
            .await
            {
                Ok(t) => t,
                Err(_) => {
                    ::metrics::counter!("kyma_self_trace_dropped_total")
                        .increment(batch.len() as u64);
                    return Ok(());
                }
            };
            if ctx
                .write_path
                .ingest(&ctx.database, &table, vec![rb])
                .await
                .is_err()
            {
                ::metrics::counter!("kyma_self_trace_dropped_total").increment(batch.len() as u64);
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Array, Int64Array, StringArray};
    use opentelemetry::trace::{
        SpanContext, SpanId, SpanKind, Status as OtelStatus, TraceFlags, TraceId, TraceState,
    };
    use opentelemetry::KeyValue as OtelKv;
    use opentelemetry_sdk::export::trace::SpanData;
    use std::borrow::Cow;
    use std::time::{Duration, UNIX_EPOCH};

    fn sample_span() -> SpanData {
        SpanData {
            span_context: SpanContext::new(
                TraceId::from_bytes(0xabad1dea_u128.to_be_bytes()),
                SpanId::from_bytes(0xbeef_u64.to_be_bytes()),
                TraceFlags::SAMPLED,
                false,
                TraceState::default(),
            ),
            parent_span_id: SpanId::INVALID,
            span_kind: SpanKind::Server,
            name: Cow::Borrowed("memory.recall"),
            start_time: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            end_time: UNIX_EPOCH + Duration::from_secs(1_700_000_000) + Duration::from_millis(250),
            attributes: vec![
                OtelKv::new("kyma.subject", "ws-mbp-shaked"),
                OtelKv::new("kyma.tenant", "default"),
                OtelKv::new("memory.results", 7_i64),
            ],
            dropped_attributes_count: 0,
            events: Default::default(),
            links: Default::default(),
            status: OtelStatus::Ok,
            instrumentation_scope: Default::default(),
        }
    }

    #[test]
    fn span_data_maps_to_row() {
        let batch = spans_to_batch(&[sample_span()], "kyma-server").expect("batch");
        assert_eq!(batch.num_rows(), 1);
        let schema = batch.schema();
        let col = |n: &str| schema.index_of(n).unwrap();
        let s = |n: &str| {
            batch
                .column(col(n))
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0)
                .to_string()
        };
        assert_eq!(s("name"), "memory.recall");
        assert_eq!(s("service_name"), "kyma-server");
        assert_eq!(s("subject"), "ws-mbp-shaked");
        assert_eq!(s("status_code"), "OK");
        assert_eq!(s("kind"), "SERVER");
        let parents = batch
            .column(col("parent_span_id"))
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert!(parents.is_null(0));
        let dur = batch
            .column(col("duration_ns"))
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(dur.value(0), 250_000_000);
        assert!(s("attributes_json").contains("memory.results"));
        assert!(!s("attributes_json").contains("kyma.subject"));
    }

    #[test]
    fn unwired_exporter_drops_without_error() {
        let exporter = SelfTraceExporter::unwired();
        futures::executor::block_on(async {
            let mut e = exporter;
            use opentelemetry_sdk::export::trace::SpanExporter as _;
            e.export(vec![sample_span()]).await.expect("drop ok");
        });
    }
}
