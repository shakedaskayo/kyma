use crate::notes::*;
use crate::types::NoteRow;

fn row() -> NoteRow {
    NoteRow {
        id: "9f3a2b1c-4e21-4b8a-9c3e-1f2a3b4c5d6e".into(),
        realm: "pensieve".into(),
        memory_type: "decision".into(),
        title: "Auth model uses stateless JWT".into(),
        content: "Sessions are stateless JWTs.\n\nWhy: horizontal scale.".into(),
        tags: "auth, architecture".into(),
        importance: 0.8,
        status: "active".into(),
        created_at: "2026-05-14T09:12:00Z".into(),
        updated_at: "2026-07-01T18:40:00Z".into(),
        valid_at: Some("2026-05-14T09:12:00Z".into()),
        invalid_at: None,
        topic_key: None,
    }
}

#[test]
fn render_is_deterministic_and_parses_back() {
    let related = vec![RelatedLink {
        target: "notes/decisions/refresh-tokens-1b2c3d4e".into(),
        title: "Refresh tokens rotate daily".into(),
        edge_type: "RELATES_TO".into(),
    }];
    let a = render_note(&row(), &related, true);
    let b = render_note(&row(), &related, true);
    assert_eq!(a, b);

    let parsed = parse_note(&a);
    assert_eq!(parsed.title.as_deref(), Some("Auth model uses stateless JWT"));
    assert_eq!(parsed.pensieve_memory_id.as_deref(), Some("9f3a2b1c-4e21-4b8a-9c3e-1f2a3b4c5d6e"));
    assert_eq!(parsed.memory_type.as_deref(), Some("decision"));
    assert_eq!(parsed.realm.as_deref(), Some("pensieve"));
    assert_eq!(parsed.tags, vec!["auth", "architecture"]);
    // Managed block stripped from the editable body.
    assert!(!parsed.body.contains("Related"));
    assert!(parsed.body.contains("Sessions are stateless JWTs."));
}

#[test]
fn content_hash_ignores_managed_blocks_and_whitespace() {
    let r = row();
    let rendered = render_note(
        &r,
        &[RelatedLink { target: "x".into(), title: "X".into(), edge_type: "REFERENCES".into() }],
        true,
    );
    let parsed = parse_note(&rendered);
    // Hash in frontmatter matches recomputation over the editable region.
    assert_eq!(
        parsed.content_hash.as_deref(),
        Some(content_hash(&r.title, &parsed.body).as_str())
    );
}

#[test]
fn private_spans_are_stripped() {
    assert_eq!(strip_private_spans("a <private>secret</private>b"), "a b");
    assert_eq!(strip_private_spans("a <private>unclosed"), "a ");
    assert_eq!(strip_private_spans("no spans"), "no spans");
}

#[test]
fn note_stem_is_stable_and_windows_safe() {
    assert_eq!(
        note_stem("Auth model uses stateless JWT", "9f3a2b1c-4e21-4b8a"),
        "auth-model-uses-stateless-jwt-9f3a2b1c"
    );
    // Reserved device name gets replaced.
    assert!(note_stem("CON", "9f3a2b1c").starts_with("note-"));
    // Empty title still yields a stem.
    assert!(note_stem("!!!", "9f3a2b1c").starts_with("note-"));
}

#[test]
fn yaml_special_titles_round_trip() {
    let mut r = row();
    r.title = "fix: \"quoted\" #tags & colons: yes".into();
    let rendered = render_note(&r, &[], true);
    let parsed = parse_note(&rendered);
    assert_eq!(parsed.title.as_deref(), Some(r.title.as_str()));
}

#[test]
fn plain_markdown_without_frontmatter_parses() {
    let parsed = parse_note("# A note\n\nJust text with a [[Wiki Link]].");
    assert_eq!(parsed.title.as_deref(), Some("A note"));
    assert!(parsed.pensieve_memory_id.is_none());
    assert_eq!(parsed.links, vec!["Wiki Link"]);
    assert_eq!(parsed.body, "Just text with a [[Wiki Link]].");
}
