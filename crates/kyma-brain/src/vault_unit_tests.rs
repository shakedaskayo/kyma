use crate::registry::{BrainConfig, RealmSelector};
use crate::types::{EdgeRow, Manifest, ManifestEntry, NoteRow};
use crate::vault::plan_vault;

fn cfg_flat() -> BrainConfig {
    BrainConfig::new("team", RealmSelector::Realms(vec!["kyma".into()]), "2026-07-08T00:00:00Z")
        .unwrap()
}

fn cfg_multi() -> BrainConfig {
    BrainConfig::new(
        "team",
        RealmSelector::Realms(vec!["kyma".into(), "global".into()]),
        "2026-07-08T00:00:00Z",
    )
    .unwrap()
}

fn node(id: &str, title: &str, ty: &str, realm: &str) -> NoteRow {
    NoteRow {
        id: id.into(),
        realm: realm.into(),
        memory_type: ty.into(),
        title: title.into(),
        content: format!("Content of {title}."),
        tags: String::new(),
        importance: 0.5,
        status: "active".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-06-01T00:00:00Z".into(),
        valid_at: None,
        invalid_at: None,
        topic_key: None,
    }
}

#[test]
fn plan_is_deterministic_under_input_permutation() {
    let cfg = cfg_multi();
    let a = node("aaaa1111-0000-0000-0000-000000000001", "Alpha", "fact", "kyma");
    let b = node("bbbb2222-0000-0000-0000-000000000002", "Beta", "decision", "global");
    let e = EdgeRow {
        src: format!("memory:{}", a.id),
        dst: format!("memory:{}", b.id),
        edge_type: "RELATES_TO".into(),
    };
    let plan1 =
        plan_vault(&cfg, &Manifest::default(), &[a.clone(), b.clone()], &[e.clone()]).unwrap();
    let plan2 = plan_vault(&cfg, &Manifest::default(), &[b, a], &[e]).unwrap();
    assert_eq!(plan1.files, plan2.files);
}

#[test]
fn flat_vs_realm_layout_paths() {
    let n = node("aaaa1111-0000-0000-0000-000000000001", "Alpha", "fact", "kyma");
    let flat = plan_vault(&cfg_flat(), &Manifest::default(), &[n.clone()], &[]).unwrap();
    assert!(flat.files.iter().any(|f| f.path == "notes/facts/alpha-aaaa1111.md"));
    let multi = plan_vault(&cfg_multi(), &Manifest::default(), &[n], &[]).unwrap();
    assert!(multi.files.iter().any(|f| f.path == "notes/kyma/facts/alpha-aaaa1111.md"));
    assert!(multi.files.iter().any(|f| f.path == "notes/kyma/index.md"));
}

#[test]
fn prior_manifest_path_wins_over_title_change() {
    let cfg = cfg_flat();
    let mut n = node("aaaa1111-0000-0000-0000-000000000001", "Alpha", "fact", "kyma");
    let first = plan_vault(&cfg, &Manifest::default(), &[n.clone()], &[]).unwrap();
    let minted = "notes/facts/alpha-aaaa1111.md";
    assert!(first.manifest.entries.iter().any(|e| e.path == minted));

    n.title = "Alpha renamed completely".into();
    let second = plan_vault(&cfg, &first.manifest, &[n], &[]).unwrap();
    // Same path survives the rename; new title only in content/frontmatter.
    let f = second.files.iter().find(|f| f.path == minted).expect("path kept");
    assert!(String::from_utf8_lossy(&f.bytes).contains("Alpha renamed completely"));
}

#[test]
fn filters_exclude_archived_and_low_importance() {
    let mut cfg = cfg_flat();
    cfg.include.min_importance = Some(0.6);
    let mut keep = node("aaaa1111-0000-0000-0000-000000000001", "Keep", "fact", "kyma");
    keep.importance = 0.9;
    let mut low = node("bbbb2222-0000-0000-0000-000000000002", "Low", "fact", "kyma");
    low.importance = 0.1;
    let mut archived = node("cccc3333-0000-0000-0000-000000000003", "Gone", "fact", "kyma");
    archived.importance = 0.9;
    archived.status = "archived".into();
    let plan = plan_vault(&cfg, &Manifest::default(), &[keep, low, archived], &[]).unwrap();
    assert_eq!(plan.note_count, 1);
    let all: String = plan.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>().join("\n");
    assert!(all.contains("keep-"));
    assert!(!all.contains("low-"));
    assert!(!all.contains("gone-"));
}

#[test]
fn wiki_topic_key_routes_to_wiki_folder() {
    let cfg = cfg_flat();
    let mut n = node("aaaa1111-0000-0000-0000-000000000001", "Auth overview", "summary", "kyma");
    n.topic_key = Some("wiki:team:auth".into());
    let plan = plan_vault(&cfg, &Manifest::default(), &[n], &[]).unwrap();
    assert!(plan.files.iter().any(|f| f.path == "wiki/auth.md"));
    // index.md lists it under Start here.
    let idx = plan.files.iter().find(|f| f.path == "index.md").unwrap();
    assert!(String::from_utf8_lossy(&idx.bytes).contains("[[wiki/auth|Auth overview]]"));
}

#[test]
fn edges_render_related_links_both_directions() {
    let cfg = cfg_flat();
    let a = node("aaaa1111-0000-0000-0000-000000000001", "Alpha", "fact", "kyma");
    let b = node("bbbb2222-0000-0000-0000-000000000002", "Beta", "decision", "kyma");
    let e = EdgeRow {
        src: format!("memory:{}", a.id),
        dst: format!("memory:{}", b.id),
        edge_type: "REFERENCES".into(),
    };
    let plan = plan_vault(&cfg, &Manifest::default(), &[a, b], &[e]).unwrap();
    let alpha = plan.files.iter().find(|f| f.path.contains("alpha")).unwrap();
    let beta = plan.files.iter().find(|f| f.path.contains("beta")).unwrap();
    assert!(String::from_utf8_lossy(&alpha.bytes)
        .contains("[[notes/decisions/beta-bbbb2222|Beta]] — REFERENCES"));
    assert!(String::from_utf8_lossy(&beta.bytes)
        .contains("[[notes/facts/alpha-aaaa1111|Alpha]] — REFERENCES"));
}

#[test]
fn manifest_lists_every_generated_path_and_itself() {
    let cfg = cfg_flat();
    let n = node("aaaa1111-0000-0000-0000-000000000001", "Alpha", "fact", "kyma");
    let plan = plan_vault(&cfg, &Manifest::default(), &[n], &[]).unwrap();
    let owned = plan.manifest.owned_paths();
    for f in &plan.files {
        assert!(owned.contains(&f.path), "{} missing from manifest", f.path);
    }
    assert!(owned.contains(".kyma/manifest.json"));
    // Note entry carries its memory id.
    assert_eq!(
        plan.manifest.memory_ids_by_path().get("notes/facts/alpha-aaaa1111.md").map(String::as_str),
        Some("aaaa1111-0000-0000-0000-000000000001")
    );
}

#[test]
fn manifest_round_trips_bytes() {
    let m = Manifest {
        version: 1,
        entries: vec![
            ManifestEntry { path: "b.md".into(), memory_id: Some("x".into()) },
            ManifestEntry { path: "a.md".into(), memory_id: None },
        ],
    };
    let bytes = m.to_bytes();
    let back = Manifest::from_bytes(&bytes).unwrap();
    // Entries sorted by path on serialization.
    assert_eq!(back.entries[0].path, "a.md");
    assert_eq!(back.entries[1].memory_id.as_deref(), Some("x"));
}
