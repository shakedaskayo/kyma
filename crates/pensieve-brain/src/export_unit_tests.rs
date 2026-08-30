//! Export-pass tests against a real bare repo (skip when git missing).
//! The load-bearing case: user-pushed files the exporter does not own must
//! survive a full-state `deleteall` commit.

use crate::export::run_export;
use crate::gitbin::GitBin;
use crate::registry::{BrainConfig, RealmSelector};
use crate::types::{NoteRow, VaultFile};

macro_rules! require_git {
    () => {
        match GitBin::detect().await {
            Some(g) => g,
            None => {
                eprintln!("skipping: git binary not found");
                return;
            }
        }
    };
}

fn cfg() -> BrainConfig {
    BrainConfig::new("team", RealmSelector::Realms(vec!["pensieve".into()]), "2026-07-08T00:00:00Z")
        .unwrap()
}

fn node(id: &str, title: &str) -> NoteRow {
    NoteRow {
        id: id.into(),
        realm: "pensieve".into(),
        memory_type: "fact".into(),
        title: title.into(),
        content: format!("Body of {title}."),
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

#[tokio::test]
async fn first_export_seeds_and_commits_vault() {
    let git = require_git!();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("team.git");
    git.init_bare(&repo).await.unwrap();

    let out = run_export(
        &git,
        &repo,
        &cfg(),
        &[node("aaaa1111-0000-0000-0000-000000000001", "Alpha")],
        &[],
        1_750_000_000,
    )
    .await
    .unwrap();
    assert!(!out.noop);
    assert_eq!(out.note_count, 1);

    let paths = git.ls_tree_paths(&repo, "main").await.unwrap();
    for expected in [
        "README.md",
        "CONTRIBUTING.md",
        "index.md",
        ".pensieve/manifest.json",
        ".gitignore",
        ".obsidian/graph.json",
        "inbox/README.md",
        "notes/facts/alpha-aaaa1111.md",
    ] {
        assert!(paths.contains(&expected.to_string()), "{expected} missing: {paths:?}");
    }
}

#[tokio::test]
async fn unchanged_memory_state_is_noop() {
    let git = require_git!();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("team.git");
    git.init_bare(&repo).await.unwrap();

    let nodes = [node("aaaa1111-0000-0000-0000-000000000001", "Alpha")];
    let first = run_export(&git, &repo, &cfg(), &nodes, &[], 1).await.unwrap();
    assert!(!first.noop);
    let second = run_export(&git, &repo, &cfg(), &nodes, &[], 2).await.unwrap();
    assert!(second.noop, "same memory state must not publish a commit");
    assert_eq!(second.commit, first.commit);
}

#[tokio::test]
async fn user_files_survive_export_and_seeded_files_stay_editable() {
    let git = require_git!();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("team.git");
    git.init_bare(&repo).await.unwrap();

    let nodes = [node("aaaa1111-0000-0000-0000-000000000001", "Alpha")];
    let first = run_export(&git, &repo, &cfg(), &nodes, &[], 1).await.unwrap();

    // Simulate a push adding a user file and editing seeded .gitignore.
    let head_paths = git.ls_tree_paths(&repo, "main").await.unwrap();
    let mut files: Vec<VaultFile> = Vec::new();
    for p in &head_paths {
        files.push(VaultFile {
            path: p.clone(),
            bytes: git.cat_file(&repo, "main", p).await.unwrap(),
        });
    }
    files.push(VaultFile { path: "assets/diagram.png".into(), bytes: vec![1, 2, 3] });
    for f in &mut files {
        if f.path == ".gitignore" {
            f.bytes.extend_from_slice(b"my-custom-ignore\n");
        }
    }
    let pushed = git
        .fast_import_commit(&repo, "main", &files, Some(&first.commit), "user push", 2)
        .await
        .unwrap();
    assert!(!pushed.noop);

    // Next export must preserve both.
    let second = run_export(&git, &repo, &cfg(), &nodes, &[], 3).await.unwrap();
    assert!(!second.noop || second.commit == pushed.commit);
    let paths = git.ls_tree_paths(&repo, "main").await.unwrap();
    assert!(paths.contains(&"assets/diagram.png".to_string()), "user file destroyed by export");
    let gi = git.cat_file(&repo, "main", ".gitignore").await.unwrap();
    assert!(
        String::from_utf8_lossy(&gi).contains("my-custom-ignore"),
        "user edit to seeded file clobbered"
    );
}

#[tokio::test]
async fn removed_memory_disappears_from_tree() {
    let git = require_git!();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("team.git");
    git.init_bare(&repo).await.unwrap();

    let a = node("aaaa1111-0000-0000-0000-000000000001", "Alpha");
    let b = node("bbbb2222-0000-0000-0000-000000000002", "Beta");
    run_export(&git, &repo, &cfg(), &[a.clone(), b], &[], 1).await.unwrap();
    assert!(git
        .ls_tree_paths(&repo, "main")
        .await
        .unwrap()
        .contains(&"notes/facts/beta-bbbb2222.md".to_string()));

    // Beta archived → out of the filter set → gone from the next tree.
    let out = run_export(&git, &repo, &cfg(), &[a], &[], 2).await.unwrap();
    assert!(!out.noop);
    let paths = git.ls_tree_paths(&repo, "main").await.unwrap();
    assert!(!paths.contains(&"notes/facts/beta-bbbb2222.md".to_string()));
    assert!(paths.contains(&"notes/facts/alpha-aaaa1111.md".to_string()));
}
