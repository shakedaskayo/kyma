//! Minimal gRPC-web smoke test — sends a Flight DoGet over HTTP/1.1 and
//! asserts we get Arrow-framed body back.
#![cfg(feature = "web-ui")]

use bytes::{BufMut, BytesMut};

#[tokio::test]
async fn grpc_web_do_get_returns_arrow_stream() {
    let server = kyma_server::test_support::start_test_server_with_seeded_data().await;
    let base = server.http_base_url(); // e.g. http://127.0.0.1:PORT
    let ticket_json = serde_json::json!({
        "database": "obs",
        "query": "otel_logs | take 1",
        "language": "kql"
    }).to_string();

    // Flight.Ticket protobuf: { bytes ticket = 1 }. Field 1, wire type 2 (length-delimited).
    let mut proto = BytesMut::new();
    proto.put_u8(0x0a); // tag: field=1, wire=2
    prost::encoding::encode_varint(ticket_json.len() as u64, &mut proto);
    proto.extend_from_slice(ticket_json.as_bytes());

    // gRPC-web frame: [0x00][len u32 BE][proto]
    let mut frame = BytesMut::with_capacity(5 + proto.len());
    frame.put_u8(0x00);
    frame.put_u32(proto.len() as u32);
    frame.extend_from_slice(&proto);

    let client = reqwest::Client::builder().http1_only().build().unwrap();
    let resp = client.post(format!("{base}/flight/arrow.flight.protocol.FlightService/DoGet"))
        .header("authorization", "Bearer test-read-token")
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .body(frame.freeze())
        .send().await.unwrap();

    assert!(resp.status().is_success(), "status: {}", resp.status());
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/grpc-web+proto",
    );
    let body = resp.bytes().await.unwrap();
    assert!(body.len() > 5, "expected at least one gRPC-web frame; got {} bytes", body.len());
    server.shutdown().await;
}
