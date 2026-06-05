//! `[[wikilink]]` extraction and emission for memory-file bodies.

/// Extract wikilink targets from a markdown body, first-occurrence order,
/// de-duplicated, whitespace-trimmed. Empty targets and unterminated `[[`
/// are ignored.
pub fn extract(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let target = after[..end].trim();
        if !target.is_empty() && !out.iter().any(|t| t == target) {
            out.push(target.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

/// Render a wikilink for a memory name.
pub fn to_wikilink(name: &str) -> String {
    format!("[[{name}]]")
}
