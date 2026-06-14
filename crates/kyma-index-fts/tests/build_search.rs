//! Build → serialize → open → BM25-search round-trip and relevance tests.

use kyma_core::index_sidecar::RowAddress;
use kyma_core::segment_format::BlockId;
use kyma_index_fts::{bm25_search, build_fts, open_index, read_header};

fn addr(block: u32, row: u32) -> RowAddress {
    RowAddress {
        block: BlockId(block),
        row,
    }
}

/// Build an index from `(text)` docs at sequential addresses, returning the blob.
fn build(docs: &[&str]) -> bytes::Bytes {
    let rows: Vec<(RowAddress, String)> = docs
        .iter()
        .enumerate()
        .map(|(i, s)| (addr(0, i as u32), (*s).to_string()))
        .collect();
    let (blob, params) = build_fts(rows).unwrap();
    assert_eq!(params["tokenizer"], "kyma-word-v1");
    assert_eq!(params["docs"], docs.len() as u64);
    blob
}

#[test]
fn round_trip_finds_doc_and_decodes_addr() {
    let rows = vec![
        (addr(0, 0), "the quick brown fox".to_string()),
        (addr(1, 7), "lazy dog sleeps".to_string()),
        (addr(2, 3), "quick foxes run".to_string()),
    ];
    let (blob, _p) = build_fts(rows).unwrap();
    let _h = read_header(&blob).unwrap();
    let index = open_index(&blob).unwrap();

    // "dog" appears only in the (block=1,row=7) doc.
    let hits = bm25_search(&index, "dog", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, addr(1, 7), "RowAddress must round-trip");
    assert!(hits[0].1 > 0.0, "BM25 score should be positive");
}

#[test]
fn bm25_ranks_rare_term_doc_first() {
    // 200 filler docs + a few containing a rare term.
    let mut docs: Vec<String> = (0..200)
        .map(|i| format!("common log line number {i} status ok"))
        .collect();
    docs[42] = "common log line OutOfMemoryError status crash".to_string();
    docs[150] = "another OutOfMemoryError happened here".to_string();

    let rows: Vec<(RowAddress, String)> = docs
        .iter()
        .enumerate()
        .map(|(i, s)| (addr(0, i as u32), s.clone()))
        .collect();
    let (blob, _p) = build_fts(rows).unwrap();
    let index = open_index(&blob).unwrap();

    let hits = bm25_search(&index, "outofmemoryerror", 5).unwrap();
    assert!(!hits.is_empty(), "rare term must be found");
    // Both docs containing the term should rank at the top.
    let top_rows: std::collections::HashSet<u32> =
        hits.iter().take(2).map(|(a, _)| a.row).collect();
    assert!(
        top_rows.contains(&42) && top_rows.contains(&150),
        "the two docs with the rare term should rank first, got {top_rows:?}"
    );
}

#[test]
fn multi_term_query_and_no_match() {
    let blob = build(&[
        "alpha beta gamma",
        "beta delta",
        "gamma epsilon",
        "completely unrelated words",
    ]);
    let index = open_index(&blob).unwrap();

    // OR semantics: "alpha gamma" matches docs 0 and 2 (and 1? no — doc1 has
    // neither). doc0 has both terms → should outscore doc2 (one term).
    let hits = bm25_search(&index, "alpha gamma", 10).unwrap();
    let rows: Vec<u32> = hits.iter().map(|(a, _)| a.row).collect();
    assert!(rows.contains(&0) && rows.contains(&2));
    assert_eq!(hits[0].0.row, 0, "doc with both terms ranks first");

    // No-match query → empty.
    assert!(bm25_search(&index, "zzzznotpresent", 10)
        .unwrap()
        .is_empty());
    // Query that tokenizes to nothing (all single-char / punctuation) → empty.
    assert!(bm25_search(&index, "a ! ?", 10).unwrap().is_empty());
}

#[test]
fn corrupt_blob_errs_not_panics() {
    assert!(open_index(b"not a real sidecar blob").is_err());
    assert!(read_header(&[0u8; 3]).is_err());
}
