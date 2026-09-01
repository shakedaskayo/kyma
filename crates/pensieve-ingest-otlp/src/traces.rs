//! OTLP traces ingest — `ExportTraceServiceRequest` → `otel_traces` rows.
//!
//! Same shape as the logs path in `lib.rs`: one RecordBatch per export,
//! built column-at-a-time, written through the shared [`WritePath`].
//! `pensieve.subject` / `pensieve.tenant` span attributes are promoted to real
//! columns — the Traces page filters on them constantly.

use arrow_array::builder::{Int64Builder, StringBuilder, TimestampNanosecondBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::KeyValue;
use opentelemetry_proto::tonic::trace::v1::span::SpanKind;
use std::sync::Arc;
use tonic::Status;

use crate::{any_value_to_string, hex_encode, keyvalue_to_json, split_service_and_json};

pub const OTEL_TRACES_TABLE: &str = "otel_traces";

pub fn otel_traces_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "start_time",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new(
            "end_time",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new("duration_ns", DataType::Int64, true),
        Field::new("trace_id", DataType::Utf8, true),
        Field::new("span_id", DataType::Utf8, true),
        Field::new("parent_span_id", DataType::Utf8, true),
        Field::new("name", DataType::Utf8, true),
        Field::new("kind", DataType::Utf8, true),
        Field::new("status_code", DataType::Utf8, true),
        Field::new("status_message", DataType::Utf8, true),
        Field::new("service_name", DataType::Utf8, true),
        Field::new("subject", DataType::Utf8, true),
        Field::new("tenant", DataType::Utf8, true),
        Field::new("attributes_json", DataType::Utf8, true),
        Field::new("resource_json", DataType::Utf8, true),
    ]))
}

fn kind_label(kind: i32) -> &'static str {
    match SpanKind::try_from(kind) {
        Ok(SpanKind::Internal) => "INTERNAL",
        Ok(SpanKind::Server) => "SERVER",
        Ok(SpanKind::Client) => "CLIENT",
        Ok(SpanKind::Producer) => "PRODUCER",
        Ok(SpanKind::Consumer) => "CONSUMER",
        _ => "UNSPECIFIED",
    }
}

fn status_label(code: i32) -> &'static str {
    match code {
        1 => "OK",
        2 => "ERROR",
        _ => "UNSET",
    }
}

/// Split span attributes into (subject, tenant, remaining-as-json).
fn split_pensieve_attrs(attrs: &[KeyValue]) -> (Option<String>, Option<String>, String) {
    let mut subject = None;
    let mut tenant = None;
    let mut rest: Vec<KeyValue> = Vec::with_capacity(attrs.len());
    for kv in attrs {
        let val = kv.value.as_ref().and_then(any_value_to_string);
        match kv.key.as_str() {
            "pensieve.subject" => subject = val,
            "pensieve.tenant" => tenant = val,
            _ => rest.push(kv.clone()),
        }
    }
    let json = serde_json::to_string(&keyvalue_to_json(&rest)).unwrap_or_else(|_| "{}".into());
    (subject, tenant, json)
}

/// Pure mapping: the whole export request → one RecordBatch.
pub fn request_to_batch(req: &ExportTraceServiceRequest) -> Result<RecordBatch, Status> {
    let total_spans: usize = req
        .resource_spans
        .iter()
        .flat_map(|rs| rs.scope_spans.iter())
        .map(|ss| ss.spans.len())
        .sum();

    let cap = total_spans;
    let str_cap = cap * 32;

    let mut start_b = TimestampNanosecondBuilder::with_capacity(cap);
    let mut end_b = TimestampNanosecondBuilder::with_capacity(cap);
    let mut dur_b = Int64Builder::with_capacity(cap);
    let mut trace_id_b = StringBuilder::with_capacity(cap, str_cap);
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
    let mut resource_b = StringBuilder::with_capacity(cap, cap * 64);

    for rs in &req.resource_spans {
        let (service_name, resource_json) = match &rs.resource {
            Some(r) => split_service_and_json(&r.attributes),
            None => (None, "{}".to_string()),
        };
        for ss in &rs.scope_spans {
            for span in &ss.spans {
                // start / end / duration
                let start = span.start_time_unix_nano as i64;
                let end = span.end_time_unix_nano as i64;
                let dur = (span.end_time_unix_nano.saturating_sub(span.start_time_unix_nano))
                    as i64;

                if span.start_time_unix_nano > 0 {
                    start_b.append_value(start);
                } else {
                    start_b.append_null();
                }
                if span.end_time_unix_nano > 0 {
                    end_b.append_value(end);
                } else {
                    end_b.append_null();
                }
                dur_b.append_value(dur);

                // trace_id / span_id
                if span.trace_id.is_empty() {
                    trace_id_b.append_null();
                } else {
                    trace_id_b.append_value(&hex_encode(&span.trace_id));
                }
                if span.span_id.is_empty() {
                    span_id_b.append_null();
                } else {
                    span_id_b.append_value(&hex_encode(&span.span_id));
                }
                // parent_span_id: null for root spans
                if span.parent_span_id.is_empty() {
                    parent_span_id_b.append_null();
                } else {
                    parent_span_id_b.append_value(&hex_encode(&span.parent_span_id));
                }

                // name
                if span.name.is_empty() {
                    name_b.append_null();
                } else {
                    name_b.append_value(&span.name);
                }

                // kind
                kind_b.append_value(kind_label(span.kind));

                // status
                match &span.status {
                    Some(s) => {
                        status_code_b.append_value(status_label(s.code));
                        if s.message.is_empty() {
                            status_msg_b.append_null();
                        } else {
                            status_msg_b.append_value(&s.message);
                        }
                    }
                    None => {
                        status_code_b.append_value("UNSET");
                        status_msg_b.append_null();
                    }
                }

                // service_name from resource
                match &service_name {
                    Some(s) => service_b.append_value(s),
                    None => service_b.append_null(),
                }

                // pensieve.subject / pensieve.tenant / remaining attributes
                let (subject, tenant, attrs_json) = split_pensieve_attrs(&span.attributes);
                match subject {
                    Some(s) => subject_b.append_value(&s),
                    None => subject_b.append_null(),
                }
                match tenant {
                    Some(t) => tenant_b.append_value(&t),
                    None => tenant_b.append_null(),
                }
                attrs_b.append_value(&attrs_json);
                resource_b.append_value(&resource_json);
            }
        }
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

    RecordBatch::try_new(otel_traces_schema(), arrays)
        .map_err(|e| Status::internal(format!("build batch: {e}")))
}

// -------- service -------------------------------------------------------

use pensieve_core::catalog::Catalog;
use pensieve_ingest_core::WritePath;
use opentelemetry_proto::tonic::collector::trace::v1::{
    trace_service_server::{TraceService, TraceServiceServer},
    ExportTracePartialSuccess, ExportTraceServiceResponse,
};
use tonic::{Request, Response};
use tracing::{debug, info};

/// The OTLP traces service implementation.
pub struct OtlpTraceService {
    catalog: Arc<dyn Catalog>,
    write_path: WritePath,
    database: String,
}

impl OtlpTraceService {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        write_path: WritePath,
        database: impl Into<String>,
    ) -> Self {
        Self {
            catalog,
            write_path,
            database: database.into(),
        }
    }

    pub fn into_server(self) -> TraceServiceServer<Self> {
        TraceServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl TraceService for OtlpTraceService {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, tonic::Status> {
        let req = request.into_inner();
        let batch = request_to_batch(&req)?;
        let total = batch.num_rows();
        debug!(resource_spans = req.resource_spans.len(), total, "otlp trace export");
        if total == 0 {
            return Ok(Response::new(ExportTraceServiceResponse::default()));
        }
        let table_ref = crate::ensure_otel_table(
            &self.catalog,
            &self.database,
            OTEL_TRACES_TABLE,
            otel_traces_schema(),
        )
        .await?;
        let ack = self
            .write_path
            .ingest(&self.database, &table_ref, vec![batch])
            .await
            .map_err(|e| tonic::Status::internal(format!("ingest: {e}")))?;
        ::metrics::counter!("pensieve_otlp_spans_total").increment(ack.rows_ingested);
        info!(rows = ack.rows_ingested, "otlp trace export committed");
        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: if ack.rows_ingested == total as u64 {
                None
            } else {
                Some(ExportTracePartialSuccess {
                    rejected_spans: (total as i64 - ack.rows_ingested as i64).max(0),
                    error_message: String::new(),
                })
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Array, Int64Array, StringArray, TimestampNanosecondArray};
    use opentelemetry_proto::tonic::common::v1::{any_value::Value as V, AnyValue, KeyValue};
    use opentelemetry_proto::tonic::resource::v1::Resource;
    use opentelemetry_proto::tonic::trace::v1::{span, ResourceSpans, ScopeSpans, Span, Status};

    fn kv(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: Some(AnyValue {
                value: Some(V::StringValue(value.into())),
            }),
        }
    }

    fn sample_request() -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![kv("service.name", "pensieve-server")],
                    dropped_attributes_count: 0,
                }),
                scope_spans: vec![ScopeSpans {
                    scope: None,
                    spans: vec![Span {
                        trace_id: vec![0xaa; 16],
                        span_id: vec![0xbb; 8],
                        trace_state: String::new(),
                        parent_span_id: vec![],
                        name: "memory.recall".into(),
                        kind: span::SpanKind::Internal as i32,
                        start_time_unix_nano: 1_700_000_000_000_000_000,
                        end_time_unix_nano: 1_700_000_000_250_000_000,
                        attributes: vec![
                            kv("pensieve.subject", "ws-mbp-shaked"),
                            kv("pensieve.tenant", "default"),
                            kv("memory.query", "okta sso"),
                        ],
                        dropped_attributes_count: 0,
                        events: vec![],
                        dropped_events_count: 0,
                        links: vec![],
                        dropped_links_count: 0,
                        status: Some(Status {
                            message: String::new(),
                            code: 1,
                        }),
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        }
    }

    #[test]
    fn maps_spans_to_rows() {
        let batch = request_to_batch(&sample_request()).expect("batch");
        assert_eq!(batch.num_rows(), 1);
        let schema = batch.schema();
        let col = |name: &str| schema.index_of(name).unwrap();

        let start = batch
            .column(col("start_time"))
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .unwrap();
        assert_eq!(start.value(0), 1_700_000_000_000_000_000);
        let dur = batch
            .column(col("duration_ns"))
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(dur.value(0), 250_000_000);

        let s = |name: &str| {
            batch
                .column(col(name))
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0)
                .to_string()
        };
        assert_eq!(s("trace_id"), "aa".repeat(16));
        assert_eq!(s("span_id"), "bb".repeat(8));
        let parents = batch
            .column(col("parent_span_id"))
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert!(parents.is_null(0)); // root span
        assert_eq!(s("name"), "memory.recall");
        assert_eq!(s("kind"), "INTERNAL");
        assert_eq!(s("status_code"), "OK");
        assert_eq!(s("service_name"), "pensieve-server");
        assert_eq!(s("subject"), "ws-mbp-shaked");
        assert_eq!(s("tenant"), "default");
        // pensieve.* promoted OUT of attributes_json; the rest stays in.
        let attrs = s("attributes_json");
        assert!(attrs.contains("memory.query"));
        assert!(!attrs.contains("pensieve.subject"));
    }

    #[test]
    fn empty_request_yields_no_batch() {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![],
        };
        assert!(request_to_batch(&req).expect("ok").num_rows() == 0);
    }
}
