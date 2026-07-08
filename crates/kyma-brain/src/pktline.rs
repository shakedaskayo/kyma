//! Minimal pkt-line framing for the smart-HTTP ref advertisement: the
//! `GET info/refs` response must prefix git's own output with a service
//! banner pkt-line followed by a flush packet.

/// Encode one pkt-line: 4-hex length (including the header) + payload.
pub fn pkt_line(payload: &str) -> Vec<u8> {
    let len = payload.len() + 4;
    assert!(len <= 0xffff, "pkt-line payload too large");
    let mut out = format!("{len:04x}").into_bytes();
    out.extend_from_slice(payload.as_bytes());
    out
}

/// The flush packet.
pub const FLUSH: &[u8] = b"0000";

/// The `# service=…` banner + flush that precedes the advertisement body.
pub fn service_banner(service: &str) -> Vec<u8> {
    let mut out = pkt_line(&format!("# service={service}\n"));
    out.extend_from_slice(FLUSH);
    out
}
