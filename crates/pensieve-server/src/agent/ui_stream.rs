//! AI SDK **UI Message Stream** emitter.
//!
//! The web client talks to `/v1/agent/ask` with Vercel's `@ai-sdk/react`
//! `useChat`, which consumes the *UI Message Stream* protocol: a
//! `text/event-stream` carrying one JSON part per SSE `data:` frame (the part
//! type lives in a `"type"` field, **not** in the SSE `event:` field), plus a
//! required `x-vercel-ai-ui-message-stream: v1` response header and a trailing
//! `data: [DONE]` sentinel.
//!
//! This module owns the wire format so both engine paths (the Claude CLI path
//! and the adk-rust path) emit identical, spec-correct frames.
//!
//! Part shapes (per the AI SDK docs):
//! - `{"type":"start","messageId":…}` / `{"type":"finish"}`
//! - `{"type":"text-start","id":…}` / `text-delta` `{id,delta}` / `text-end`
//! - `{"type":"reasoning-start|delta|end", …}` (same shape as text)
//! - `{"type":"tool-input-available","toolCallId":…,"toolName":…,"input":…}`
//! - `{"type":"tool-output-available","toolCallId":…,"output":…}`
//! - `{"type":"tool-output-error","toolCallId":…,"errorText":…}`
//! - `{"type":"data-<name>","data":…}` — custom data parts (we use `data-session`,
//!   `data-usage`)
//! - `{"type":"error","errorText":…}`

use std::convert::Infallible;

use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// Thin wrapper over the SSE sender that serializes UI Message Stream parts.
///
/// Cloneable so the same stream can be written from multiple tasks; every send
/// is best-effort (a hung-up client just drops the frame).
#[derive(Clone)]
pub struct UiStream {
    tx: mpsc::UnboundedSender<Result<SseEvent, Infallible>>,
}

/// Create an emitter plus the receiver to hand to [`response`].
pub fn channel() -> (UiStream, mpsc::UnboundedReceiver<Result<SseEvent, Infallible>>) {
    let (tx, rx) = mpsc::unbounded_channel::<Result<SseEvent, Infallible>>();
    (UiStream { tx }, rx)
}

impl UiStream {
    fn part(&self, value: Value) {
        let body = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
        let _ = self.tx.send(Ok(SseEvent::default().data(body)));
    }

    /// `start` — opens the assistant message.
    pub fn start(&self, message_id: &str) {
        self.part(json!({ "type": "start", "messageId": message_id }));
    }

    /// `start-step` — opens a step within the message.
    pub fn start_step(&self) {
        self.part(json!({ "type": "start-step" }));
    }

    pub fn text_start(&self, id: &str) {
        self.part(json!({ "type": "text-start", "id": id }));
    }
    pub fn text_delta(&self, id: &str, delta: &str) {
        self.part(json!({ "type": "text-delta", "id": id, "delta": delta }));
    }
    pub fn text_end(&self, id: &str) {
        self.part(json!({ "type": "text-end", "id": id }));
    }

    pub fn reasoning_start(&self, id: &str) {
        self.part(json!({ "type": "reasoning-start", "id": id }));
    }
    pub fn reasoning_delta(&self, id: &str, delta: &str) {
        self.part(json!({ "type": "reasoning-delta", "id": id, "delta": delta }));
    }
    pub fn reasoning_end(&self, id: &str) {
        self.part(json!({ "type": "reasoning-end", "id": id }));
    }

    /// `tool-input-available` — a complete tool call (id, name, input).
    pub fn tool_input_available(&self, tool_call_id: &str, tool_name: &str, input: Value) {
        self.part(json!({
            "type": "tool-input-available",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "input": input,
        }));
    }
    /// `tool-output-available` — the tool's successful result.
    pub fn tool_output_available(&self, tool_call_id: &str, output: Value) {
        self.part(json!({
            "type": "tool-output-available",
            "toolCallId": tool_call_id,
            "output": output,
        }));
    }
    /// `tool-output-error` — the tool failed.
    pub fn tool_output_error(&self, tool_call_id: &str, error_text: &str) {
        self.part(json!({
            "type": "tool-output-error",
            "toolCallId": tool_call_id,
            "errorText": error_text,
        }));
    }

    /// Custom `data-<name>` part. `useChat` surfaces these as data parts the UI
    /// can read (we use `session` for resume and `usage` for run metadata).
    pub fn data(&self, name: &str, data: Value) {
        self.part(json!({ "type": format!("data-{name}"), "data": data }));
    }

    /// Transient error part — does not terminate the stream by itself.
    pub fn error(&self, error_text: &str) {
        self.part(json!({ "type": "error", "errorText": error_text }));
    }

    pub fn finish_step(&self) {
        self.part(json!({ "type": "finish-step" }));
    }
    pub fn finish(&self) {
        self.part(json!({ "type": "finish" }));
    }

    /// Emit the terminal `[DONE]` sentinel. Call exactly once, last.
    pub fn done(&self) {
        let _ = self.tx.send(Ok(SseEvent::default().data("[DONE]")));
    }
}

/// Build the SSE `Response` for a UI Message Stream, attaching the required
/// `x-vercel-ai-ui-message-stream: v1` header that tells `useChat` how to parse
/// the body.
pub fn response(rx: mpsc::UnboundedReceiver<Result<SseEvent, Infallible>>) -> Response {
    let stream = UnboundedReceiverStream::new(rx);
    let sse = Sse::new(stream).keep_alive(KeepAlive::default());
    ([("x-vercel-ai-ui-message-stream", "v1")], sse).into_response()
}
