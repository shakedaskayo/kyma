use super::frontmatter::{parse, render, Frontmatter, MemoryFile};

const CC_FILE: &str = "---\nname: auth-model\ndescription: \"How auth works: session tokens\"\nmetadata:\n  node_type: memory\n  type: project\n  originSessionId: 11111111-1111-1111-1111-111111111111\n---\n\nBody with **Why:** because.\n\nSee [[other-mem]].\n";

#[test]
fn parses_claude_code_frontmatter() {
    let f = parse(CC_FILE).expect("parses");
    assert_eq!(f.front.name.as_deref(), Some("auth-model"));
    assert_eq!(
        f.front.description.as_deref(),
        Some("How auth works: session tokens")
    );
    assert_eq!(f.front.cc_type.as_deref(), Some("project"));
    assert_eq!(
        f.front.origin_session_id.as_deref(),
        Some("11111111-1111-1111-1111-111111111111")
    );
    assert!(!f.is_kyma_authored());
    assert!(f.body.starts_with("Body with **Why:**"));
    assert!(f.body.contains("[[other-mem]]"));
}

#[test]
fn detects_kyma_authored_files() {
    let raw = "---\nname: promoted\nmetadata:\n  source: kyma\n  kyma_memory_id: memory:abc\n  content_hash: deadbeef\n---\nbody\n";
    let f = parse(raw).expect("parses");
    assert!(f.is_kyma_authored());
    assert_eq!(f.front.kyma_memory_id.as_deref(), Some("memory:abc"));
    assert_eq!(f.front.content_hash.as_deref(), Some("deadbeef"));
}

#[test]
fn no_frontmatter_returns_none() {
    assert!(parse("just a markdown file\n").is_none());
    assert!(parse("").is_none());
}

#[test]
fn malformed_yaml_returns_none() {
    assert!(parse("---\n: : :\n  - [\n---\nbody\n").is_none());
}

#[test]
fn missing_name_is_tolerated() {
    let f = parse("---\ndescription: nameless\n---\nbody\n").expect("parses");
    assert_eq!(f.front.name, None);
    assert_eq!(f.front.description.as_deref(), Some("nameless"));
}

#[test]
fn render_parse_round_trips() {
    let file = MemoryFile {
        front: Frontmatter {
            name: Some("auth-model".into()),
            description: Some("How auth works: session tokens".into()),
            cc_type: Some("project".into()),
            origin_session_id: Some("11111111-1111-1111-1111-111111111111".into()),
            source: Some("kyma".into()),
            kyma_memory_id: Some("memory:abc".into()),
            content_hash: Some("deadbeef".into()),
            archived_at: None,
            archived_reason: None,
        },
        body: "Line one.\n\nLine two with [[link]].\n".into(),
    };
    let rendered = render(&file);
    let back = parse(&rendered).expect("re-parses");
    assert_eq!(back, file);
    // Deterministic: rendering again is byte-identical.
    assert_eq!(render(&back), rendered);
}

#[test]
fn render_omits_absent_fields() {
    let file = MemoryFile {
        front: Frontmatter {
            name: Some("minimal".into()),
            ..Frontmatter::default()
        },
        body: "b\n".into(),
    };
    let rendered = render(&file);
    assert!(!rendered.contains("description"));
    assert!(!rendered.contains("archived_at"));
    assert!(!rendered.contains("source"));
}

#[test]
fn archive_tombstone_round_trips() {
    let file = MemoryFile {
        front: Frontmatter {
            name: Some("old".into()),
            archived_at: Some("2026-06-05T00:00:00Z".into()),
            archived_reason: Some("superseded by [[new]]".into()),
            ..Frontmatter::default()
        },
        body: "original body kept in full\n".into(),
    };
    let back = parse(&render(&file)).expect("re-parses");
    assert_eq!(back, file);
}
