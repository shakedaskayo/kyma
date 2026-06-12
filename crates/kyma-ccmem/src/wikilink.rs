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

/// Extract Obsidian-flavored wikilink targets, normalized to the bare note
/// name: embeds (`![[…]]`) are caught by the plain `[[` scan; alias
/// (`|alias`), heading (`#heading`), and block (`^block`) suffixes are
/// stripped. First-occurrence order, de-duplicated after normalization.
pub fn extract_normalized(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in extract(body) {
        let t = raw.split(['|', '#', '^']).next().unwrap_or("").trim();
        if !t.is_empty() && !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// Render a wikilink for a memory name.
pub fn to_wikilink(name: &str) -> String {
    format!("[[{name}]]")
}

#[cfg(test)]
mod normalized_tests {
    use super::*;

    #[test]
    fn normalizes_obsidian_link_flavors() {
        let body =
            "See [[Plain]], [[Target|an alias]], [[Note#Heading]], [[Other^block1]], and ![[Embedded Note]].";
        assert_eq!(
            extract_normalized(body),
            vec!["Plain", "Target", "Note", "Other", "Embedded Note"]
        );
    }

    #[test]
    fn dedupes_after_normalization_and_drops_empty() {
        // `[[A|x]]` and `[[A#y]]` normalize to the same target; `[[|alias]]`
        // and blank targets are dropped.
        let body = "[[A|x]] [[A#y]] [[|alias]] [[ ]]";
        assert_eq!(extract_normalized(body), vec!["A"]);
    }
}
