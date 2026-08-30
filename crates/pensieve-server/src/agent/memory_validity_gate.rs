//! Optional validity gate for newly-extracted/proposed memories (M8.3b) —
//! rejects trivial/filler content before it becomes a stored memory.
//!
//! Two call sites, each with different available signal:
//! - The realtime extraction pipeline ([`super::memory`]) has the extraction
//!   model's own `confidence` score to reuse for free — [`extraction_reject`].
//! - Dreaming's `save_memory`/`save_memories` tools don't — the
//!   [`super::dreaming::ValidityGatedTool`] decorator falls through to the
//!   heuristic, then an optional second-opinion LLM judge.
//!
//! Tiers, cheapest first: (1) reuse `confidence` if present — free; (2) a
//! pure length + filler-phrase heuristic — cheap, no I/O; (3) an **optional**
//! single-turn LLM judge via [`super::runner::run_oneshot`], only reached when
//! `llm_escalation_enabled` and the heuristic alone flagged the content.

use serde::Deserialize;

use super::memory_extract::{extract_json_object, ExtractedMemory};
use super::memory_settings::ValidityGateSettings;
use super::runner;
use super::state::AgentState;

/// Cheap heuristic pre-filter, pure — no I/O. `None` = passes.
fn heuristic_reject(content: &str, min_chars: usize) -> Option<&'static str> {
    let trimmed = content.trim();
    if trimmed.chars().count() < min_chars {
        return Some("too short to be a durable memory");
    }
    const FILLER: &[&str] = &[
        "ok",
        "okay",
        "sure",
        "got it",
        "thanks",
        "thank you",
        "sounds good",
        "no problem",
        "yes",
        "no",
        "done",
        "great",
        "cool",
        "nice",
        "understood",
    ];
    if FILLER.contains(&trimmed.to_ascii_lowercase().as_str()) {
        return Some("conversational filler");
    }
    None
}

/// Realtime extraction pipeline gate: reuse `confidence` (free), then the
/// heuristic. `None` = keep. Disabled (`settings.enabled == false`) is a
/// pure passthrough — zero behavior change until an operator turns it on.
pub fn extraction_reject(m: &ExtractedMemory, settings: &ValidityGateSettings) -> Option<String> {
    if !settings.enabled {
        return None;
    }
    if let Some(c) = m.confidence {
        if c < settings.min_confidence {
            return Some(format!(
                "extraction confidence {c:.2} below {}",
                settings.min_confidence
            ));
        }
    }
    heuristic_reject(&m.content, settings.min_content_chars).map(str::to_string)
}

const VALIDITY_SYSTEM: &str = "You judge whether a piece of text is worth keeping as a \
durable, reusable memory for an AI coding agent — a genuine fact/decision/preference/learning, \
not conversational filler, an unresolved question, or content already obvious from context. \
Return STRICT JSON: {\"keep\": true|false, \"reason\": \"one short sentence\"}. Output ONLY the \
JSON object.";

#[derive(Debug, Deserialize)]
struct KeepVerdict {
    keep: bool,
}

/// Tier 3: a second-opinion LLM judge. Any failure (engine unavailable, bad
/// JSON) defaults to `true` (keep) — the gate must never silently drop
/// genuinely new information over a transient LLM hiccup.
async fn llm_judge_keep(state: &AgentState, content: &str) -> bool {
    let text = match runner::run_oneshot(
        state,
        "kyma-validity-gate",
        "Judges whether content is worth keeping as a durable memory.",
        VALIDITY_SYSTEM,
        content,
    )
    .await
    {
        Ok(t) => t,
        Err(_) => return true,
    };
    extract_json_object(&text)
        .and_then(|j| serde_json::from_str::<KeepVerdict>(&j).ok())
        .map(|v| v.keep)
        .unwrap_or(true)
}

/// Tool-side gate (dreaming's `save_memory`/`save_memories`, which have no
/// `confidence` field to reuse): heuristic first; when it flags the content
/// AND `llm_escalation_enabled`, get a second opinion rather than trusting a
/// length/filler-list check alone. `None` = keep.
pub async fn tool_reject_reason(
    state: &AgentState,
    content: &str,
    settings: &ValidityGateSettings,
) -> Option<String> {
    if !settings.enabled {
        return None;
    }
    let heuristic = heuristic_reject(content, settings.min_content_chars)?;
    if !settings.llm_escalation_enabled {
        return Some(heuristic.to_string());
    }
    if llm_judge_keep(state, content).await {
        None
    } else {
        Some(format!("{heuristic} (confirmed by validity-gate judge)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(min_chars: usize) -> ValidityGateSettings {
        ValidityGateSettings {
            enabled: true,
            llm_escalation_enabled: false,
            min_confidence: 0.4,
            min_content_chars: min_chars,
        }
    }

    fn extracted(content: &str, confidence: Option<f32>) -> ExtractedMemory {
        ExtractedMemory {
            content: content.to_string(),
            title: None,
            kind: "fact".to_string(),
            importance: 0.5,
            valid_at: None,
            entity_mentions: vec![],
            confidence,
        }
    }

    #[test]
    fn disabled_is_passthrough() {
        let mut s = settings(20);
        s.enabled = false;
        assert_eq!(extraction_reject(&extracted("ok", Some(0.1)), &s), None);
    }

    #[test]
    fn low_confidence_rejects_even_long_content() {
        let m = extracted("a perfectly reasonable and detailed statement", Some(0.1));
        assert!(extraction_reject(&m, &settings(20)).is_some());
    }

    #[test]
    fn missing_confidence_falls_through_to_heuristic() {
        let m = extracted("a perfectly reasonable and detailed statement", None);
        assert_eq!(extraction_reject(&m, &settings(20)), None);
    }

    #[test]
    fn short_content_rejected_by_heuristic() {
        let m = extracted("kyma", Some(0.9));
        assert!(extraction_reject(&m, &settings(20)).is_some());
    }

    #[test]
    fn filler_phrase_rejected_regardless_of_length_setting() {
        let m = extracted("thanks", Some(0.9));
        assert!(extraction_reject(&m, &settings(1)).is_some());
    }

    #[test]
    fn genuine_fact_passes() {
        let m = extracted("kyma uses DataFusion for query execution", Some(0.9));
        assert_eq!(extraction_reject(&m, &settings(20)), None);
    }
}
