use super::hash::content_hash;

#[test]
fn stable_for_identical_input() {
    let a = content_hash("auth", Some("project"), "body text\n");
    let b = content_hash("auth", Some("project"), "body text\n");
    assert_eq!(a, b);
    assert_eq!(a.len(), 64); // blake3 hex
}

#[test]
fn insensitive_to_trailing_whitespace() {
    let a = content_hash("auth", Some("project"), "line one  \nline two\n\n\n");
    let b = content_hash("auth", Some("project"), "line one\nline two");
    assert_eq!(a, b);
}

#[test]
fn sensitive_to_meaningful_change() {
    let base = content_hash("auth", Some("project"), "body");
    assert_ne!(base, content_hash("auth", Some("project"), "different body"));
    assert_ne!(base, content_hash("other-name", Some("project"), "body"));
    assert_ne!(base, content_hash("auth", Some("user"), "body"));
    assert_ne!(base, content_hash("auth", None, "body"));
}
