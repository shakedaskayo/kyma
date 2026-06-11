//! OpenTelemetry Protocol (OTLP) ingest frontend — logs MVP.
//!
//! Implements `opentelemetry_proto::tonic::collector::logs::v1::LogsService`.
//! The OpenTelemetry Collector's OTLP-gRPC exporter points at our port
//! 4317 (the OTLP default) and we route `ExportLogsServiceRequest`s into
//! the fixed `otel_logs` table in the configured database.
//!
//! # Schema (auto-created on first export if missing)
//!
//! ```text
//! otel_logs:
//!   timestamp        timestamp
//!   severity_number  int
//!   severity_text    string
//!   body             string
//!   service_name     string
//!   trace_id         string     (hex-encoded)
//!   span_id          string     (hex-encoded)
//!   attributes_json  string     (flattened KeyValue[] as JSON string)
//! ```
//!
//! # What's not here (phase A)
//!
//! - Trace + metric signals (`ExportTraceService`, `ExportMetricsService`)
//! - Histogram metric rollups (delta vs cumulative encoding)
//! - Span events → nested dynamic-column JSON (needs the real `dynamic` type)
//! - Flattening all resource/scope attributes into typed columns with
//!   aliases (`k8s_namespace`, `host_name`, …)
//!
//! Those are planned extensions; this crate keeps a single clean entry
//! point so adding them is a modification, not a rewrite.

#![forbid(unsafe_code)]

use arrow_array::builder::{Int32Builder, StringBuilder, TimestampNanosecondBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use kyma_core::catalog::{Catalog, TableConfig};
use kyma_core::types::DatabaseId;
use kyma_ingest_core::WritePath;
use opentelemetry_proto::tonic::collector::logs::v1::{
    logs_service_server::{LogsService, LogsServiceServer},
    ExportLogsPartialSuccess, ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::{
    any_value::Value as AnyValueValue, AnyValue, KeyValue,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::{debug, info};

pub mod self_export;
pub mod traces;

const OTEL_LOGS_TABLE: &str = "otel_logs";

fn otel_logs_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new("severity_number", DataType::Int32, true),
        Field::new("severity_text", DataType::Utf8, true),
        Field::new("body", DataType::Utf8, true),
        Field::new("service_name", DataType::Utf8, true),
        Field::new("trace_id", DataType::Utf8, true),
        Field::new("span_id", DataType::Utf8, true),
        Field::new("attributes_json", DataType::Utf8, true),
    ]))
}

/// The OTLP logs service implementation.
pub struct OtlpLogsService {
    catalog: Arc<dyn Catalog>,
    write_path: WritePath,
    /// Target database for all OTLP-received logs. Configurable at startup.
    database: String,
}

impl OtlpLogsService {
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

    pub fn into_server(self) -> LogsServiceServer<Self> {
        LogsServiceServer::new(self)
    }

    /// Ensure the `otel_logs` table exists in the target database.
    /// Idempotent — returns the existing `TableRef` if already present.
    async fn ensure_table(&self) -> Result<kyma_core::catalog::TableRef, Status> {
        ensure_otel_table(&self.catalog, &self.database, OTEL_LOGS_TABLE, otel_logs_schema()).await
    }
}

#[tonic::async_trait]
impl LogsService for OtlpLogsService {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let req = request.into_inner();
        let table_ref = self.ensure_table().await?;
        let total_records: usize = req
            .resource_logs
            .iter()
            .flat_map(|rl| rl.scope_logs.iter())
            .map(|sl| sl.log_records.len())
            .sum();
        debug!(
            resource_logs = req.resource_logs.len(),
            total_records, "otlp export"
        );

        if total_records == 0 {
            return Ok(Response::new(ExportLogsServiceResponse::default()));
        }

        // One RecordBatch built column-at-a-time from the whole request.
        let mut ts_b = TimestampNanosecondBuilder::with_capacity(total_records);
        let mut sev_num_b = Int32Builder::with_capacity(total_records);
        let mut sev_text_b = StringBuilder::with_capacity(total_records, total_records * 8);
        let mut body_b = StringBuilder::with_capacity(total_records, total_records * 64);
        let mut service_b = StringBuilder::with_capacity(total_records, total_records * 16);
        let mut trace_b = StringBuilder::with_capacity(total_records, total_records * 32);
        let mut span_b = StringBuilder::with_capacity(total_records, total_records * 16);
        let mut attrs_b = StringBuilder::with_capacity(total_records, total_records * 64);

        for rl in &req.resource_logs {
            // Pull service.name + remaining attributes from resource.
            let (service_name, resource_attrs_json) = match &rl.resource {
                Some(r) => split_service_and_json(&r.attributes),
                None => (None, "{}".to_string()),
            };
            for sl in &rl.scope_logs {
                for rec in &sl.log_records {
                    // timestamp
                    let ts = if rec.time_unix_nano > 0 {
                        Some(rec.time_unix_nano as i64)
                    } else if rec.observed_time_unix_nano > 0 {
                        Some(rec.observed_time_unix_nano as i64)
                    } else {
                        None
                    };
                    match ts {
                        Some(v) => ts_b.append_value(v),
                        None => ts_b.append_null(),
                    }
                    // severity
                    if rec.severity_number != 0 {
                        sev_num_b.append_value(rec.severity_number);
                    } else {
                        sev_num_b.append_null();
                    }
                    if rec.severity_text.is_empty() {
                        sev_text_b.append_null();
                    } else {
                        sev_text_b.append_value(&rec.severity_text);
                    }
                    // body
                    match rec.body.as_ref().and_then(any_value_to_string) {
                        Some(s) => body_b.append_value(&s),
                        None => body_b.append_null(),
                    }
                    // service
                    match &service_name {
                        Some(s) => service_b.append_value(s),
                        None => service_b.append_null(),
                    }
                    // trace/span — hex-encode the raw bytes.
                    if rec.trace_id.is_empty() {
                        trace_b.append_null();
                    } else {
                        trace_b.append_value(&hex_encode(&rec.trace_id));
                    }
                    if rec.span_id.is_empty() {
                        span_b.append_null();
                    } else {
                        span_b.append_value(&hex_encode(&rec.span_id));
                    }
                    // attributes: resource + scope + record, merged to one JSON.
                    let merged = merge_attrs_json(
                        &resource_attrs_json,
                        sl.scope.as_ref().map(|s| &s.attributes[..]).unwrap_or(&[]),
                        &rec.attributes,
                    );
                    attrs_b.append_value(&merged);
                }
            }
        }

        let arrays: Vec<ArrayRef> = vec![
            Arc::new(ts_b.finish()),
            Arc::new(sev_num_b.finish()),
            Arc::new(sev_text_b.finish()),
            Arc::new(body_b.finish()),
            Arc::new(service_b.finish()),
            Arc::new(trace_b.finish()),
            Arc::new(span_b.finish()),
            Arc::new(attrs_b.finish()),
        ];
        let batch = RecordBatch::try_new(otel_logs_schema(), arrays)
            .map_err(|e| Status::internal(format!("build batch: {e}")))?;

        // Route through the same WritePath that REST ingest uses.
        let ack = self
            .write_path
            .ingest(&self.database, &table_ref, vec![batch])
            .await
            .map_err(|e| Status::internal(format!("ingest: {e}")))?;

        ::metrics::counter!("kyma_otlp_log_records_total").increment(ack.rows_ingested);
        info!(rows = ack.rows_ingested, "otlp export committed");

        // Standard OTLP response — partial_success is for soft-errors we
        // don't use yet.
        Ok(Response::new(ExportLogsServiceResponse {
            partial_success: if ack.rows_ingested == total_records as u64 {
                None
            } else {
                Some(ExportLogsPartialSuccess {
                    rejected_log_records: (total_records as i64 - ack.rows_ingested as i64).max(0),
                    error_message: String::new(),
                })
            },
        }))
    }
}

// -------- public bootstrap helpers ------------------------------------

/// Pre-create the `otel_traces` table so it exists immediately on fresh
/// install, before the first self-trace batch is flushed. Called from
/// `kyma-bin` right after the self-trace exporter is wired to storage.
pub async fn ensure_traces_table(catalog: &Arc<dyn Catalog>, database: &str) {
    if let Err(e) = ensure_otel_table(
        catalog,
        database,
        traces::OTEL_TRACES_TABLE,
        traces::otel_traces_schema(),
    )
    .await
    {
        tracing::warn!(error = %e, "could not pre-create otel_traces table");
    }
}

// -------- helpers -----------------------------------------------------

/// Ensure `table` exists in `database` with `schema`, creating the database
/// on first use (OTLP has no separate "create" step). Idempotent.
pub(crate) async fn ensure_otel_table(
    catalog: &Arc<dyn Catalog>,
    database: &str,
    table: &str,
    schema: Arc<Schema>,
) -> Result<kyma_core::catalog::TableRef, Status> {
    match catalog.lookup_table(database, table).await {
        Ok(t) => Ok(t),
        Err(_) => {
            let db_id = match find_database_id(&**catalog, database).await {
                Some(id) => id,
                None => catalog
                    .create_database(database)
                    .await
                    .map_err(|e| Status::internal(format!("create_database: {e}")))?,
            };
            catalog
                .create_table(db_id, table, schema, TableConfig::default())
                .await
                .map_err(|e| Status::internal(format!("create_table: {e}")))?;
            catalog
                .lookup_table(database, table)
                .await
                .map_err(|e| Status::internal(format!("lookup after create: {e}")))
        }
    }
}

/// Look up a database by name. Returns `None` if not found. Uses a raw SQL
/// query against the catalog's underlying pool to avoid plumbing yet
/// another trait method — OTLP is a bootstrapping path so this compromise
/// is acceptable for MVP.
async fn find_database_id(catalog: &dyn Catalog, name: &str) -> Option<DatabaseId> {
    use std::any::Any;
    let any_ref: &dyn Any = catalog.as_ref_any();
    let Some(pg) = any_ref.downcast_ref::<kyma_catalog::PostgresCatalog>() else {
        return None;
    };
    let row: Option<(uuid::Uuid,)> = sqlx::query_as("SELECT id FROM databases WHERE name = $1")
        .bind(name)
        .fetch_optional(pg.pool())
        .await
        .ok()
        .flatten();
    row.map(|(id,)| DatabaseId::from_uuid(id))
}

pub(crate) fn any_value_to_string(v: &AnyValue) -> Option<String> {
    let inner = v.value.as_ref()?;
    Some(match inner {
        AnyValueValue::StringValue(s) => s.clone(),
        AnyValueValue::BoolValue(b) => b.to_string(),
        AnyValueValue::IntValue(i) => i.to_string(),
        AnyValueValue::DoubleValue(d) => d.to_string(),
        AnyValueValue::ArrayValue(_)
        | AnyValueValue::KvlistValue(_)
        | AnyValueValue::BytesValue(_) => {
            // Serialize as JSON via serde for fidelity on complex bodies.
            serde_json::to_string(&keyvalue_to_json(&[KeyValue {
                key: "_".to_string(),
                value: Some(v.clone()),
            }]))
            .unwrap_or_default()
        }
    })
}

pub(crate) fn keyvalue_to_json(pairs: &[KeyValue]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for kv in pairs {
        map.insert(kv.key.clone(), any_value_to_json(kv.value.as_ref()));
    }
    serde_json::Value::Object(map)
}

fn any_value_to_json(v: Option<&AnyValue>) -> serde_json::Value {
    use serde_json::Value as J;
    let Some(v) = v else { return J::Null };
    match v.value.as_ref() {
        None => J::Null,
        Some(AnyValueValue::StringValue(s)) => J::String(s.clone()),
        Some(AnyValueValue::BoolValue(b)) => J::Bool(*b),
        Some(AnyValueValue::IntValue(i)) => J::Number((*i).into()),
        Some(AnyValueValue::DoubleValue(d)) => serde_json::Number::from_f64(*d)
            .map(J::Number)
            .unwrap_or(J::Null),
        Some(AnyValueValue::BytesValue(b)) => J::String(hex_encode(b)),
        Some(AnyValueValue::ArrayValue(a)) => J::Array(
            a.values
                .iter()
                .map(|v| any_value_to_json(Some(v)))
                .collect(),
        ),
        Some(AnyValueValue::KvlistValue(kv)) => keyvalue_to_json(&kv.values),
    }
}

pub(crate) fn split_service_and_json(attrs: &[KeyValue]) -> (Option<String>, String) {
    let mut service_name = None;
    let mut rest = Vec::with_capacity(attrs.len());
    for kv in attrs {
        if kv.key == "service.name" {
            if let Some(s) = kv.value.as_ref().and_then(any_value_to_string) {
                service_name = Some(s);
                continue;
            }
        }
        rest.push(kv.clone());
    }
    let json = serde_json::to_string(&keyvalue_to_json(&rest)).unwrap_or_else(|_| "{}".into());
    (service_name, json)
}

fn merge_attrs_json(
    resource_json: &str,
    scope_attrs: &[KeyValue],
    record_attrs: &[KeyValue],
) -> String {
    let mut resource_map: serde_json::Value =
        serde_json::from_str(resource_json).unwrap_or_else(|_| serde_json::json!({}));
    let scope_map = keyvalue_to_json(scope_attrs);
    let record_map = keyvalue_to_json(record_attrs);

    if let serde_json::Value::Object(root) = &mut resource_map {
        root.insert("_scope".to_string(), scope_map);
        root.insert("_record".to_string(), record_map);
    }
    serde_json::to_string(&resource_map).unwrap_or_else(|_| "{}".into())
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}
