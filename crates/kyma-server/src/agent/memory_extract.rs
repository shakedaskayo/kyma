//! LLM-driven memory extraction + conflict resolution — the "intelligent"
//! half of the ingestion pipeline.
//!
//! Best-in-class agentic memory (Mem0 / Zep-Graphiti) does NOT dump raw
//! transcripts: it (1) **extracts** atomic, self-contained memories plus the
//! entities and relationships they involve, then (2) **consolidates** each
//! candidate against semantically-similar existing memories via an
//! ADD / UPDATE / NOOP / INVALIDATE decision (Mem0's "A.U.D.N."). Both steps
//! are single structured LLM turns reusing the configured agent engine
//! ([`super::runner::run_oneshot`]); the calling pipeline applies the results
//! append-only with bi-temporal validity.
//!
//! Model JSON is unreliable (esp. local models that ignore response schemas),
//! so parsing is tolerant: it strips ```` ```json ```` fences and falls back to
//! locating the first balanced `{...}` object.

use serde::{Deserialize, Serialize};

use super::runner;
use super::state::AgentState;

// ── extracted shapes ─────────────────────────────────────────────────────────

/// One atomic memory the model proposes to remember.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtractedMemory {
    /// Self-contained statement (pronouns/"it"/"the project" already resolved).
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    /// One of: fact | decision | preference | learning | procedure.
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default = "default_importance")]
    pub importance: f32,
    /// When the fact became true (RFC3339), if the model can infer it.
    #[serde(default)]
    pub valid_at: Option<String>,
    /// Names of entities this memory is about (resolved to nodes downstream).
    #[serde(default)]
    pub entity_mentions: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
}

fn default_kind() -> String {
    "fact".to_string()
}
fn default_importance() -> f32 {
    0.5
}

/// An entity (person, repo, service, file, table, config, …) the model saw.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtractedEntity {
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// A directed relationship `(src) -[predicate]-> (dst)` between two entities.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtractedRel {
    pub src: String,
    pub predicate: String,
    pub dst: String,
}

/// The full structured extraction result.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ExtractedBundle {
    #[serde(default)]
    pub memories: Vec<ExtractedMemory>,
    #[serde(default)]
    pub entities: Vec<ExtractedEntity>,
    #[serde(default)]
    pub relationships: Vec<ExtractedRel>,
}

impl ExtractedBundle {
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty() && self.entities.is_empty() && self.relationships.is_empty()
    }
}

// ── conflict-resolution decision ─────────────────────────────────────────────

/// The operation chosen for one candidate against the existing store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictOp {
    Add,
    Update,
    Noop,
    Invalidate,
}

impl ConflictOp {
    fn parse(s: &str) -> ConflictOp {
        match s.trim().to_ascii_uppercase().as_str() {
            "UPDATE" => ConflictOp::Update,
            "NOOP" => ConflictOp::Noop,
            "INVALIDATE" => ConflictOp::Invalidate,
            _ => ConflictOp::Add,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawDecision {
    op: String,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    merged_content: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// A parsed A.U.D.N. decision for a candidate memory.
#[derive(Debug, Clone)]
pub struct ConflictDecision {
    pub op: ConflictOp,
    /// The existing memory id this decision targets (for UPDATE / INVALIDATE).
    pub target_id: Option<String>,
    /// Merged/rewritten content (for UPDATE), if the model supplied one.
    pub merged_content: Option<String>,
    pub reason: Option<String>,
}

// ── prompts ──────────────────────────────────────────────────────────────────

const EXTRACTION_SYSTEM: &str = r#"You distill software-engineering activity into durable, reusable MEMORY for an AI agent's long-term store. You are given recent activity (and possibly a summary of earlier activity) for one project. Extract only what is worth remembering across sessions.

Return STRICT JSON (no prose, no markdown fences) with this exact shape:
{
  "memories": [
    {
      "content": "self-contained statement; resolve pronouns and 'it'/'the project' to concrete names",
      "title": "short label (optional)",
      "kind": "fact | decision | preference | learning | procedure",
      "importance": 0.0-1.0,
      "valid_at": "RFC3339 timestamp if the moment it became true is knowable, else omit",
      "entity_mentions": ["names of entities this memory is about"],
      "confidence": 0.0-1.0
    }
  ],
  "entities": [
    { "name": "canonical name", "kind": "person|repo|service|file|table|config|tool|other", "aliases": ["other names seen"] }
  ],
  "relationships": [
    { "src": "entity name", "predicate": "short verb phrase, e.g. depends_on / owns / calls", "dst": "entity name" }
  ]
}

Rules:
- Prefer ATOMIC memories: one fact/decision/preference/learning/procedure each.
- kind=decision for choices made; kind=preference for how the user likes things; kind=learning for "X failed because Y, fix is Z"; kind=procedure for conventions/runbooks; kind=fact otherwise.
- Do NOT invent. Do NOT record transient state, secrets, or trivia already obvious from the data.
- Every entity referenced in a memory's entity_mentions or a relationship MUST appear in "entities".
- If nothing is worth remembering, return {"memories":[],"entities":[],"relationships":[]}.
- Output ONLY the JSON object."#;

const CONFLICT_SYSTEM: &str = r#"You maintain an AI agent's long-term memory. Given a CANDIDATE memory and the most semantically-similar EXISTING memories, decide how to integrate the candidate. Return STRICT JSON:
{ "op": "ADD | UPDATE | NOOP | INVALIDATE", "target_id": "memory:... (for UPDATE/INVALIDATE)", "merged_content": "rewritten content (for UPDATE)", "reason": "one short sentence" }

Choose:
- ADD: the candidate is genuinely new information.
- UPDATE: the candidate refines/extends an existing memory that is still true — set target_id to that memory and merged_content to the improved statement.
- NOOP: the candidate is already captured (a duplicate/subset) — set target_id to the existing memory.
- INVALIDATE: the candidate CONTRADICTS an existing memory (the old one is no longer true) — set target_id to the now-false memory; the candidate will be added as its replacement.

Output ONLY the JSON object."#;

// ── public API ───────────────────────────────────────────────────────────────

/// Run the extraction LLM turn for one project window. `rolling_summary`
/// compresses earlier activity (Mem0-style context); `window` is the new
/// activity transcript since the last watermark.
pub async fn extract(
    state: &AgentState,
    realm: &str,
    rolling_summary: Option<&str>,
    window: &str,
) -> anyhow::Result<ExtractedBundle> {
    let mut input = String::new();
    if let Some(s) = rolling_summary {
        if !s.trim().is_empty() {
            input.push_str(&format!(
                "CONTEXT — summary of earlier activity in project '{realm}':\n{s}\n\n"
            ));
        }
    }
    input.push_str(&format!("NEW ACTIVITY in project '{realm}':\n{window}\n"));
    let text = runner::run_oneshot(
        state,
        "kyma-memory-extractor",
        "Extracts durable memories, entities, and relationships from activity.",
        EXTRACTION_SYSTEM,
        &input,
    )
    .await?;
    parse_bundle(&text)
}

/// Run the A.U.D.N. conflict-resolution turn for one candidate against its
/// nearest existing neighbours. `similar` is `(memory_id, content)` pairs.
pub async fn decide_conflict(
    state: &AgentState,
    candidate_content: &str,
    similar: &[(String, String)],
) -> anyhow::Result<ConflictDecision> {
    // No neighbours → trivially ADD without spending a turn.
    if similar.is_empty() {
        return Ok(ConflictDecision {
            op: ConflictOp::Add,
            target_id: None,
            merged_content: None,
            reason: Some("no similar memories".to_string()),
        });
    }
    let mut input = String::from("CANDIDATE:\n");
    input.push_str(candidate_content);
    input.push_str("\n\nEXISTING (most similar):\n");
    for (id, content) in similar {
        input.push_str(&format!("- [{id}] {content}\n"));
    }
    let text = runner::run_oneshot(
        state,
        "kyma-memory-conflict",
        "Decides ADD/UPDATE/NOOP/INVALIDATE for a candidate memory.",
        CONFLICT_SYSTEM,
        &input,
    )
    .await?;
    parse_decision(&text)
}

// ── tolerant JSON parsing ────────────────────────────────────────────────────

pub fn parse_bundle(text: &str) -> anyhow::Result<ExtractedBundle> {
    let cleaned = extract_json_object(text)
        .ok_or_else(|| anyhow::anyhow!("extraction returned no JSON object: {text:.200}"))?;
    serde_json::from_str::<ExtractedBundle>(&cleaned)
        .map_err(|e| anyhow::anyhow!("parse extraction bundle: {e}"))
}

fn parse_decision(text: &str) -> anyhow::Result<ConflictDecision> {
    let cleaned = extract_json_object(text)
        .ok_or_else(|| anyhow::anyhow!("decision returned no JSON object: {text:.200}"))?;
    let raw: RawDecision = serde_json::from_str(&cleaned)
        .map_err(|e| anyhow::anyhow!("parse conflict decision: {e}"))?;
    Ok(ConflictDecision {
        op: ConflictOp::parse(&raw.op),
        target_id: raw.target_id.filter(|s| !s.trim().is_empty()),
        merged_content: raw.merged_content.filter(|s| !s.trim().is_empty()),
        reason: raw.reason,
    })
}

/// Strip code fences and isolate the first balanced top-level JSON object.
/// Returns `None` if no `{...}` is present.
pub(crate) fn extract_json_object(text: &str) -> Option<String> {
    let t = text.trim();
    // Drop a leading ```json / ``` fence and any trailing fence.
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.trim_start();
    let start = t.find('{')?;
    let bytes = t.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(t[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_bundle() {
        let s = r#"{"memories":[{"content":"kyma uses pgvector","kind":"fact","entity_mentions":["kyma"]}],"entities":[{"name":"kyma","kind":"repo"}],"relationships":[]}"#;
        let b = parse_bundle(s).unwrap();
        assert_eq!(b.memories.len(), 1);
        assert_eq!(b.memories[0].kind, "fact");
        assert_eq!(b.memories[0].importance, 0.5); // default applied
        assert_eq!(b.entities[0].name, "kyma");
    }

    #[test]
    fn strips_code_fences_and_prose() {
        let s = "Here is the result:\n```json\n{\"memories\":[],\"entities\":[],\"relationships\":[]}\n```\nDone.";
        let b = parse_bundle(s).unwrap();
        assert!(b.is_empty());
    }

    #[test]
    fn isolates_object_with_nested_braces_and_strings() {
        let s = r#"prefix {"memories":[{"content":"uses {braces} and \"quotes\"","kind":"learning"}],"entities":[],"relationships":[]} suffix"#;
        let b = parse_bundle(s).unwrap();
        assert_eq!(b.memories.len(), 1);
        assert!(b.memories[0].content.contains("{braces}"));
    }

    #[test]
    fn parses_decision_variants() {
        let d = parse_decision(r#"{"op":"INVALIDATE","target_id":"memory:abc","reason":"contradicted"}"#).unwrap();
        assert_eq!(d.op, ConflictOp::Invalidate);
        assert_eq!(d.target_id.as_deref(), Some("memory:abc"));

        let d2 = parse_decision(r#"{"op":"noop","target_id":""}"#).unwrap();
        assert_eq!(d2.op, ConflictOp::Noop);
        assert_eq!(d2.target_id, None); // empty filtered out

        let d3 = parse_decision(r#"{"op":"weird"}"#).unwrap();
        assert_eq!(d3.op, ConflictOp::Add); // unknown → ADD
    }
}
