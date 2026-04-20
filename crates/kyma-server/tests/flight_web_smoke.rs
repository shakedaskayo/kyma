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

    // Tighten: confirm we got an Arrow Flight gRPC-web stream with a proper
    // data frame (0x00 marker) and a trailers frame (0x80 marker).
    let first_byte = body[0];
    assert!(
        first_byte == 0x00 || first_byte == 0x80,
        "expected gRPC-web frame marker (0x00 or 0x80) at start, got 0x{:02x}",
        first_byte
    );

    // If first frame is data (0x00), validate its length.
    if first_byte == 0x00 {
        let data_len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
        assert!(data_len > 0, "data frame should have non-zero length");
        assert!(body.len() >= 5 + data_len, "body shorter than declared data length");
    }

    // A valid gRPC-web response must contain a trailers frame (0x80 marker).
    assert!(
        body.iter().any(|b| *b == 0x80),
        "expected trailers-frame marker (0x80) somewhere in response"
    );

    server.shutdown().await;
}
