//! End-to-end smoke test for the Claude CLI engine.
//!
//! Spawns the real local `claude` binary, so it's `#[ignore]`d by default
//! (needs Claude Code installed + authenticated, makes a billable call). Run
//! explicitly with:
//!
//! ```sh
//! cargo test -p pensieve-server --test claude_cli_smoke -- --ignored --nocapture
//! ```
//!
//! It guards the regression where the stream aborted on the CLI's
//! `rate_limit_event` frame, dropping the terminal `result` and surfacing a
//! spurious error.

use pensieve_server::agent::engine::claude_cli::{run_stream, ClaudeEvent};

#[tokio::test]
#[ignore = "spawns the real claude CLI; run explicitly with --ignored"]
async fn streams_tokens_and_finishes_without_spurious_error() {
    if pensieve_server::agent::engine::claude_cli::locate_binary().is_none() {
        eprintln!("skipping: `claude` not on PATH");
        return;
    }

    let mut rx = run_stream(
        "Reply with exactly the word: hello",
        Some("default"),
        None,
        None,
        None,
    )
    .expect("spawn claude");

    let mut text = String::new();
    let mut got_result = false;
    let mut error: Option<String> = None;
    let mut session_id = String::new();

    while let Some(ev) = rx.recv().await {
        match ev {
            ClaudeEvent::Init { session_id: sid } => session_id = sid,
            ClaudeEvent::TextDelta { text: t, .. } => text.push_str(&t),
            ClaudeEvent::Result {
                is_error,
                session_id: sid,
                ..
            } => {
                got_result = true;
                assert!(!is_error, "result reported is_error");
                if session_id.is_empty() {
                    session_id = sid;
                }
            }
            ClaudeEvent::Error { message } => error = Some(message),
            _ => {}
        }
    }

    assert!(error.is_none(), "spurious error event: {error:?}");
    assert!(got_result, "never received a terminal Result frame");
    assert!(!session_id.is_empty(), "no session id captured for resume");
    assert!(
        text.to_lowercase().contains("hello"),
        "expected streamed text to contain 'hello', got: {text:?}"
    );
}
