use std::collections::BTreeMap;

use crate::gitbin::ChangeKind;
use crate::ingest::{plan_push_ingest, IngestOp};
use crate::registry::{BrainConfig, RealmSelector};

fn cfg() -> BrainConfig {
    BrainConfig::new("team", RealmSelector::Realms(vec!["pensieve".into()]), "2026-07-08T00:00:00Z")
        .unwrap()
}

fn cfg_multi() -> BrainConfig {
    BrainConfig::new(
        "team",
        RealmSelector::Realms(vec!["pensieve".into(), "ops".into()]),
        "2026-07-08T00:00:00Z",
    )
    .unwrap()
}

const EXPORTED: &str = "---\ntitle: Alpha\npensieve_memory_id: aaaa1111-0000-0000-0000-000000000001\ntype: fact\nrealm: pensieve\n---\n\n# Alpha\n\nEdited body.\n";

#[test]
fn edit_of_exported_note_updates_same_memory() {
    let mut prior = BTreeMap::new();
    prior.insert(
        "notes/facts/alpha-aaaa1111.md".to_string(),
        "aaaa1111-0000-0000-0000-000000000001".to_string(),
    );
    let plan = plan_push_ingest(
        &cfg(),
        &[(ChangeKind::Modified, "notes/facts/alpha-aaaa1111.md".into())],
        |_| Some(EXPORTED.as_bytes().to_vec()),
        &prior,
    );
    assert_eq!(plan.ops.len(), 1);
    match &plan.ops[0] {
        IngestOp::UpdateExisting { memory_id, note, .. } => {
            assert_eq!(memory_id, "aaaa1111-0000-0000-0000-000000000001");
            assert_eq!(note.body, "Edited body.");
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

#[test]
fn forged_id_on_exported_path_keeps_original_identity() {
    let mut prior = BTreeMap::new();
    prior.insert("notes/facts/alpha-aaaa1111.md".to_string(), "real-id".to_string());
    let forged = EXPORTED.replace("aaaa1111-0000-0000-0000-000000000001", "forged-id");
    let plan = plan_push_ingest(
        &cfg(),
        &[(ChangeKind::Modified, "notes/facts/alpha-aaaa1111.md".into())],
        |_| Some(forged.as_bytes().to_vec()),
        &prior,
    );
    match &plan.ops[0] {
        IngestOp::UpdateExisting { memory_id, .. } => assert_eq!(memory_id, "real-id"),
        other => panic!("unexpected op: {other:?}"),
    }
    assert_eq!(plan.warnings.len(), 1);
}

#[test]
fn new_inbox_note_creates_memory_with_topic_key() {
    let plan = plan_push_ingest(
        &cfg(),
        &[(ChangeKind::Added, "inbox/idea.md".into())],
        |_| Some(b"# An idea\n\nBody text.".to_vec()),
        &BTreeMap::new(),
    );
    match &plan.ops[0] {
        IngestOp::CreateNew { topic_key, realm, note, .. } => {
            assert_eq!(topic_key, "brain:team:inbox/idea.md");
            assert_eq!(realm, "pensieve");
            assert_eq!(note.title.as_deref(), Some("An idea"));
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

#[test]
fn realm_derived_from_folder_in_multi_realm_layout() {
    let plan = plan_push_ingest(
        &cfg_multi(),
        &[(ChangeKind::Added, "notes/ops/facts/new-thing.md".into())],
        |_| Some(b"# New thing\n\nBody.".to_vec()),
        &BTreeMap::new(),
    );
    match &plan.ops[0] {
        IngestOp::CreateNew { realm, .. } => assert_eq!(realm, "ops"),
        other => panic!("unexpected op: {other:?}"),
    }
}

#[test]
fn deleted_exported_note_archives_memory() {
    let mut prior = BTreeMap::new();
    prior.insert("notes/facts/alpha-aaaa1111.md".to_string(), "the-id".to_string());
    let plan = plan_push_ingest(
        &cfg(),
        &[(ChangeKind::Deleted, "notes/facts/alpha-aaaa1111.md".into())],
        |_| None,
        &prior,
    );
    assert_eq!(
        plan.ops,
        vec![IngestOp::ArchiveDeleted {
            memory_id: "the-id".into(),
            rel_path: "notes/facts/alpha-aaaa1111.md".into()
        }]
    );
}

#[test]
fn generated_and_non_note_paths_are_ignored() {
    let plan = plan_push_ingest(
        &cfg(),
        &[
            (ChangeKind::Modified, "index.md".into()),
            (ChangeKind::Modified, ".pensieve/manifest.json".into()),
            (ChangeKind::Added, "assets/diagram.png".into()),
            (ChangeKind::Modified, "README.md".into()),
            (ChangeKind::Modified, "notes/pensieve/index.md".into()),
        ],
        |_| Some(b"x".to_vec()),
        &BTreeMap::new(),
    );
    assert!(plan.ops.is_empty());
}

#[test]
fn stripped_id_keeps_identity_with_warning() {
    let mut prior = BTreeMap::new();
    prior.insert("notes/facts/alpha-aaaa1111.md".to_string(), "the-id".to_string());
    let plan = plan_push_ingest(
        &cfg(),
        &[(ChangeKind::Modified, "notes/facts/alpha-aaaa1111.md".into())],
        |_| Some(b"# Alpha\n\nNo frontmatter anymore.".to_vec()),
        &prior,
    );
    match &plan.ops[0] {
        IngestOp::UpdateExisting { memory_id, .. } => assert_eq!(memory_id, "the-id"),
        other => panic!("unexpected op: {other:?}"),
    }
    assert_eq!(plan.warnings.len(), 1);
}
