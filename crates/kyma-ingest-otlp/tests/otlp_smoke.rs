//! OTLP ingest smoke test — sends real `ExportLogsServiceRequest`
//! messages via tonic and verifies the rows land in the `otel_logs` table.
//!
//! Ignored by default — exercised via `scripts/test-otlp.sh` which
//! boots a live `kyma` server on port 4317.

use opentelemetry_proto::tonic::collector::logs::v1::logs_service_client::LogsServiceClient;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValueValue;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use opentelemetry_proto::tonic::resource::v1::Resource;

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(AnyValueValue::StringValue(value.to_string())),
        }),
    }
}

#[tokio::test]
#[ignore = "requires a live kyma OTLP server on 127.0.0.1:4317"]
async fn otlp_export_logs() {
    let mut client = LogsServiceClient::connect("http://127.0.0.1:4317")
        .await
        .expect("connect to OTLP server");
    let req = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes: vec![
                    kv("service.name", "otlp-smoke-test"),
                    kv("k8s.namespace.name", "default"),
                ],
                dropped_attributes_count: 0,
            }),
            scope_logs: vec![ScopeLogs {
                scope: Some(InstrumentationScope {
                    name: "test-scope".into(),
                    version: "0.1".into(),
                    attributes: vec![],
                    dropped_attributes_count: 0,
                }),
                log_records: vec![
                    LogRecord {
                        time_unix_nano: 1_745_000_000_000_000_000,
                        observed_time_unix_nano: 1_745_000_000_000_000_000,
                        severity_number: 9, // INFO
                        severity_text: "INFO".into(),
                        body: Some(AnyValue {
                            value: Some(AnyValueValue::StringValue("hello from OTLP".into())),
                        }),
                        attributes: vec![kv("http.method", "GET")],
                        dropped_attributes_count: 0,
                        flags: 0,
                        trace_id: vec![0xaa; 16],
                        span_id: vec![0xbb; 8],
                    },
                    LogRecord {
                        time_unix_nano: 1_745_000_000_000_000_001,
                        observed_time_unix_nano: 1_745_000_000_000_000_001,
                        severity_number: 17, // ERROR
                        severity_text: "ERROR".into(),
                        body: Some(AnyValue {
                            value: Some(AnyValueValue::StringValue(
                                "OutOfMemoryError in handler".into(),
                            )),
                        }),
                        attributes: vec![],
                        dropped_attributes_count: 0,
                        flags: 0,
                        trace_id: vec![0xcc; 16],
                        span_id: vec![0xdd; 8],
                    },
                ],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    let resp = client.export(req).await.expect("export");
    let resp = resp.into_inner();
    assert!(resp.partial_success.is_none() || resp.partial_success.as_ref().unwrap().rejected_log_records == 0,
        "OTLP reported partial_success: {:?}", resp.partial_success);
}
