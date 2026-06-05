//! Normalized content hashing for change detection.
//!
//! The hash covers what a memory *means* — name, type, body — normalized so
//! that mtime-only touches and trailing-whitespace churn don't register as
//! changes. Used both as the ingest skip-index value and as the
//! `content_hash` kyma stamps into promoted files (a mismatch on disk means
//! the user edited the file).

/// Hash a memory's identity + normalized body (blake3, hex).
pub fn content_hash(name: &str, cc_type: Option<&str>, body: &str) -> String {
    let mut normalized = String::with_capacity(body.len());
    for line in body.lines() {
        normalized.push_str(line.trim_end());
        normalized.push('\n');
    }
    let normalized = normalized.trim_end_matches('\n');

    let mut hasher = blake3::Hasher::new();
    hasher.update(name.as_bytes());
    hasher.update(b"\0");
    hasher.update(cc_type.unwrap_or("\0none").as_bytes());
    hasher.update(b"\0");
    hasher.update(normalized.as_bytes());
    hasher.finalize().to_hex().to_string()
}
