use std::path::Path;
use std::time::Duration;

use crate::cc_writeback::{apply_actions, WritebackConfig};
use kyma_server::agent::cc_curate::{FileAction, IndexEntry};

fn write(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, content).expect("write");
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// A rendered kyma file whose stamp matches its body (untouched on disk).
fn promoted_file(name: &str, body: &str) -> (String, String) {
    let hash = kyma_ccmem::hash::content_hash(name, Some("project"), body);
    let content = kyma_ccmem::frontmatter::render(&kyma_ccmem::frontmatter::MemoryFile {
        front: kyma_ccmem::frontmatter::Frontmatter {
            name: Some(name.to_string()),
            description: Some("desc".to_string()),
            cc_type: Some("project".to_string()),
            source: Some("kyma".to_string()),
            kyma_memory_id: Some("memory:abc".to_string()),
            content_hash: Some(hash.clone()),
            ..kyma_ccmem::frontmatter::Frontmatter::default()
        },
        body: body.to_string(),
    });
    (content, hash)
}

fn write_action(file: &str, content: &str, hash: &str) -> FileAction {
    FileAction::WriteMemoryFile {
        file: file.to_string(),
        content: content.to_string(),
        node_id: "memory:abc".to_string(),
        content_hash: hash.to_string(),
    }
}

const NOW: &str = "2026-06-05T12:00:00+00:00";

#[test]
fn writes_archives_and_reindexes_idempotently() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    write(
        &mem.join("MEMORY.md"),
        "# Memory index\n\n- [User entry](user-note.md) — keep me\n",
    );
    write(
        &mem.join("old-note.md"),
        "---\nname: old-note\nmetadata:\n  type: project\n---\n\nThis was true once.\n",
    );

    let (content, hash) = promoted_file("kyma-auth", "Tokens win.\n");
    let actions = vec![
        write_action("kyma-auth.md", &content, &hash),
        FileAction::ArchiveFile {
            file: "old-note.md".to_string(),
            reason: "superseded in kyma".to_string(),
            node_id: None,
        },
        FileAction::SetIndex {
            entries: vec![IndexEntry {
                title: "Auth".to_string(),
                file: "kyma-auth.md".to_string(),
                hook: "tokens".to_string(),
            }],
        },
    ];
    let cfg = WritebackConfig::default();
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("apply");
    assert_eq!(report.written, 1);
    assert_eq!(report.archived, 1);
    assert!(report.index_updated);

    // Promoted file landed.
    assert_eq!(read(&mem.join("kyma-auth.md")), content);
    // Archive is a move with a tombstone — body fully preserved, original gone.
    assert!(!mem.join("old-note.md").exists());
    let tomb = kyma_ccmem::frontmatter::parse(&read(&mem.join("archive/old-note.md")))
        .expect("tombstone parses");
    assert_eq!(tomb.front.archived_at.as_deref(), Some(NOW));
    assert_eq!(tomb.front.archived_reason.as_deref(), Some("superseded in kyma"));
    assert!(tomb.body.contains("This was true once."));
    // Index: user line intact, managed region appended.
    let idx = read(&mem.join("MEMORY.md"));
    assert!(idx.contains("- [User entry](user-note.md) — keep me"));
    assert!(idx.contains(kyma_ccmem::MANAGED_BEGIN));
    assert!(idx.contains("- [Auth](kyma-auth.md) — tokens"));

    // Re-applying the same plan changes nothing.
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("re-apply");
    assert_eq!(report.written, 0, "identical content → no rewrite");
    assert_eq!(report.archived, 0, "file already archived");
    assert!(!report.index_updated, "index byte-identical");
}

#[test]
fn user_edited_kyma_file_is_never_overwritten() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    // On disk: a kyma file whose body was hand-edited (stamp ≠ body hash).
    let (content, _) = promoted_file("kyma-auth", "Original body.\n");
    let edited = content.replace("Original body.", "USER IMPROVED THIS.");
    write(&mem.join("kyma-auth.md"), &edited);

    let (new_content, new_hash) = promoted_file("kyma-auth", "Refreshed by kyma.\n");
    let actions = vec![write_action("kyma-auth.md", &new_content, &new_hash)];
    let report =
        apply_actions(&mem, None, &actions, &WritebackConfig::default(), NOW).expect("apply");
    assert_eq!(report.skipped_user_edited, 1);
    assert_eq!(report.written, 0);
    assert!(read(&mem.join("kyma-auth.md")).contains("USER IMPROVED THIS."));
}

#[test]
fn foreign_file_at_target_path_is_never_overwritten() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    // A user file (not kyma-authored) occupies the target name.
    write(
        &mem.join("kyma-auth.md"),
        "---\nname: kyma-auth\nmetadata:\n  type: user\n---\n\nMine, hands off.\n",
    );
    let (content, hash) = promoted_file("kyma-auth", "Kyma wants this spot.\n");
    let report = apply_actions(
        &mem,
        None,
        &[write_action("kyma-auth.md", &content, &hash)],
        &WritebackConfig::default(),
        NOW,
    )
    .expect("apply");
    assert_eq!(report.skipped_user_edited, 1);
    assert!(read(&mem.join("kyma-auth.md")).contains("Mine, hands off."));
}

#[test]
fn fresh_lock_blocks_stale_lock_is_reclaimed() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem).expect("mkdir");
    let lock = mem.join(".kyma-curate.lock");
    std::fs::write(&lock, "pid").expect("lock");

    let (content, hash) = promoted_file("kyma-auth", "Body.\n");
    let actions = vec![write_action("kyma-auth.md", &content, &hash)];
    let cfg = WritebackConfig::default();
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("apply");
    assert!(report.skipped_locked);
    assert!(!mem.join("kyma-auth.md").exists());

    // Make the lock stale → reclaimed, pass proceeds, lock released after.
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    let f = std::fs::File::options().write(true).open(&lock).expect("open lock");
    f.set_times(std::fs::FileTimes::new().set_modified(old)).expect("set mtime");
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("apply 2");
    assert!(!report.skipped_locked);
    assert_eq!(report.written, 1);
    assert!(!lock.exists(), "lock released on completion");
}

#[test]
fn active_session_defers_writeback() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem).expect("mkdir");
    let sessions = tmp.path().join("sessions");
    let project = tmp.path().join("the-project");
    #[allow(clippy::cast_possible_truncation)]
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch")
        .as_millis() as i64;
    write(
        &sessions.join("123.json"),
        &format!(
            r#"{{"pid":123,"cwd":"{}","updatedAt":{now_ms},"status":"busy"}}"#,
            project.display()
        ),
    );

    let (content, hash) = promoted_file("kyma-auth", "Body.\n");
    let actions = vec![write_action("kyma-auth.md", &content, &hash)];
    let cfg = WritebackConfig {
        sessions_dir: Some(sessions.clone()),
        ..WritebackConfig::default()
    };
    let report = apply_actions(&mem, Some(&project), &actions, &cfg, NOW).expect("apply");
    assert!(report.skipped_quiet);
    assert!(!mem.join("kyma-auth.md").exists());

    // A long-idle session does not block.
    write(
        &sessions.join("123.json"),
        &format!(
            r#"{{"pid":123,"cwd":"{}","updatedAt":{},"status":"busy"}}"#,
            project.display(),
            now_ms - 3_600_000
        ),
    );
    let report = apply_actions(&mem, Some(&project), &actions, &cfg, NOW).expect("apply 2");
    assert!(!report.skipped_quiet);
    assert_eq!(report.written, 1);
}

#[test]
fn dry_run_touches_nothing_but_audits() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem).expect("mkdir");
    let audit = tmp.path().join("cc-curation.log");

    let (content, hash) = promoted_file("kyma-auth", "Body.\n");
    let actions = vec![write_action("kyma-auth.md", &content, &hash)];
    let cfg = WritebackConfig {
        dry_run: true,
        audit_log: Some(audit.clone()),
        ..WritebackConfig::default()
    };
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("apply");
    assert_eq!(report.written, 1, "dry run reports what it would do");
    assert!(!mem.join("kyma-auth.md").exists(), "but writes nothing");

    let log = read(&audit);
    let line: serde_json::Value =
        serde_json::from_str(log.lines().next().expect("one line")).expect("json");
    assert_eq!(line["dry_run"], true);
    assert_eq!(line["actions"][0]["action"], "write_memory_file");
}

#[test]
fn index_defers_to_user_entries() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    // The user already lists kyma-auth.md themselves.
    write(
        &mem.join("MEMORY.md"),
        "# Memory index\n\n- [My own pointer](kyma-auth.md) — user's words\n",
    );
    let actions = vec![FileAction::SetIndex {
        entries: vec![
            IndexEntry {
                title: "Auth".to_string(),
                file: "kyma-auth.md".to_string(),
                hook: "tokens".to_string(),
            },
            IndexEntry {
                title: "Other".to_string(),
                file: "kyma-other.md".to_string(),
                hook: "other".to_string(),
            },
        ],
    }];
    apply_actions(&mem, None, &actions, &WritebackConfig::default(), NOW).expect("apply");
    let idx = read(&mem.join("MEMORY.md"));
    assert!(idx.contains("- [My own pointer](kyma-auth.md) — user's words"));
    assert!(idx.contains("- [Other](kyma-other.md) — other"));
    let managed_region = idx
        .split(kyma_ccmem::MANAGED_BEGIN)
        .nth(1)
        .expect("managed region");
    assert!(
        !managed_region.contains("kyma-auth.md"),
        "user-listed file must not be double-indexed"
    );
}
