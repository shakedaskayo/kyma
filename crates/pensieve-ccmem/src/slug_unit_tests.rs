use super::slug::{memory_filename, path_slug, realm_for_path, resolve_project_path};
use super::topic_key;
use std::path::Path;

#[test]
fn topic_key_is_prefixed_slug_and_name() {
    assert_eq!(
        topic_key("-Users-x-projects-pensieve", "auth-model"),
        "claude-md:-Users-x-projects-pensieve/auth-model"
    );
}

#[test]
fn slugifies_like_claude_code() {
    assert_eq!(
        path_slug(Path::new("/Users/shakedaskayo/projects/agentcylabs/pensieve")),
        "-Users-shakedaskayo-projects-agentcylabs-pensieve"
    );
    // Every non-alphanumeric character becomes a dash (dots, underscores).
    assert_eq!(path_slug(Path::new("/tmp/my_app.v2")), "-tmp-my-app-v2");
}

#[test]
fn resolves_slug_via_known_paths() {
    let known = vec![
        "/Users/x/projects/pensieve".to_string(),
        "/Users/x/projects/other".to_string(),
    ];
    assert_eq!(
        resolve_project_path("-Users-x-projects-pensieve", &known)
            .as_deref()
            .map(Path::to_str),
        Some(Some("/Users/x/projects/pensieve"))
    );
    assert_eq!(resolve_project_path("-Users-x-unknown", &known), None);
}

#[test]
fn realm_is_basename() {
    assert_eq!(realm_for_path(Path::new("/Users/x/projects/pensieve")), "pensieve");
}

#[test]
fn memory_filename_is_prefixed_kebab() {
    assert_eq!(memory_filename("Auth Model Decision"), "pensieve-auth-model-decision.md");
    // Collapses runs of non-alphanumerics, trims edge dashes.
    assert_eq!(memory_filename("  Weird -- name!! "), "pensieve-weird-name.md");
}
