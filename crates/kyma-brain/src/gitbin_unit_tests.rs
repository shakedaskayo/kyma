//! Real-git tests: skipped when no `git` binary is on PATH (CI images and
//! dev machines have one; the guard keeps `cargo test` green elsewhere).

use crate::gitbin::GitBin;
use crate::types::VaultFile;

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

fn files(pairs: &[(&str, &str)]) -> Vec<VaultFile> {
    pairs
        .iter()
        .map(|(p, c)| VaultFile { path: (*p).to_string(), bytes: c.as_bytes().to_vec() })
        .collect()
}

#[tokio::test]
async fn init_commit_read_roundtrip() {
    let git = require_git!();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("t.git");
    git.init_bare(&repo).await.unwrap();

    let out = git
        .fast_import_commit(
            &repo,
            "main",
            &files(&[("a.md", "alpha\n"), ("dir/b.md", "beta\n")]),
            None,
            "first",
            1_750_000_000,
        )
        .await
        .unwrap();
    assert!(!out.noop);

    let paths = git.ls_tree_paths(&repo, "main").await.unwrap();
    assert_eq!(paths, vec!["a.md", "dir/b.md"]);
    assert_eq!(git.cat_file(&repo, "main", "a.md").await.unwrap(), b"alpha\n");
}

#[tokio::test]
async fn identical_tree_is_noop_and_rolls_back() {
    let git = require_git!();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("t.git");
    git.init_bare(&repo).await.unwrap();

    let fs = files(&[("a.md", "alpha\n")]);
    let first = git.fast_import_commit(&repo, "main", &fs, None, "one", 1).await.unwrap();
    let second =
        git.fast_import_commit(&repo, "main", &fs, Some(&first.commit), "two", 2).await.unwrap();
    assert!(second.noop);
    assert_eq!(second.commit, first.commit);
    // Branch still points at the first commit.
    let head = git.rev_parse(&repo, "refs/heads/main").await.unwrap().unwrap();
    assert_eq!(head, first.commit);
}

#[tokio::test]
async fn diff_name_status_reports_changes() {
    let git = require_git!();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("t.git");
    git.init_bare(&repo).await.unwrap();

    let one = git
        .fast_import_commit(&repo, "main", &files(&[("a.md", "1\n"), ("b.md", "1\n")]), None, "1", 1)
        .await
        .unwrap();
    let two = git
        .fast_import_commit(
            &repo,
            "main",
            &files(&[("a.md", "2\n"), ("c.md", "1\n")]),
            Some(&one.commit),
            "2",
            2,
        )
        .await
        .unwrap();
    let mut changes = git.diff_name_status(&repo, &one.commit, &two.commit).await.unwrap();
    changes.sort_by(|a, b| a.1.cmp(&b.1));
    use crate::gitbin::ChangeKind::*;
    assert_eq!(
        changes,
        vec![(Modified, "a.md".into()), (Deleted, "b.md".into()), (Added, "c.md".into())]
    );
}
