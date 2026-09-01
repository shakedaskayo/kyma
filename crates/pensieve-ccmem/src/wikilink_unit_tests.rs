use super::wikilink::{extract, to_wikilink};

#[test]
fn extracts_in_order_deduped() {
    let body = "See [[alpha]] then [[beta]], and [[alpha]] again.";
    assert_eq!(extract(body), vec!["alpha".to_string(), "beta".to_string()]);
}

#[test]
fn trims_and_drops_empty() {
    assert_eq!(extract("[[ spaced-name ]] and [[]]"), vec!["spaced-name".to_string()]);
}

#[test]
fn ignores_unterminated() {
    assert!(extract("broken [[link without close").is_empty());
    assert!(extract("no links at all").is_empty());
}

#[test]
fn renders_wikilink() {
    assert_eq!(to_wikilink("auth-model"), "[[auth-model]]");
}
