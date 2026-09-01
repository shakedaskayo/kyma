//! Query side: BM25 top-k search over an opened FTS sidecar index.

use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{IndexRecordOption, Value};
use tantivy::{Index, TantivyDocument, Term};

use pensieve_core::index_sidecar::RowAddress;

use crate::build::{unpack_addr, ADDR_FIELD, BODY_FIELD};
use crate::file::FtsError;
use crate::tokenizer::tokenize;

type Result<T> = std::result::Result<T, FtsError>;

/// Run a BM25 top-`top_k` search for `query` over an opened sidecar index.
///
/// The query is analyzed with the same `pensieve-word-v1` tokenizer used at build
/// time (so a query term matches a document term iff their analyzed forms are
/// equal), then run as an OR over the resulting terms — standard bag-of-words
/// BM25. Returns `(RowAddress, score)` pairs, highest score first. An
/// all-stopword / empty query returns an empty result.
pub fn bm25_search(index: &Index, query: &str, top_k: usize) -> Result<Vec<(RowAddress, f32)>> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    let schema = index.schema();
    let body = schema
        .get_field(BODY_FIELD)
        .map_err(|e| FtsError::Tantivy(format!("body field: {e}")))?;
    let addr_field = schema
        .get_field(ADDR_FIELD)
        .map_err(|e| FtsError::Tantivy(format!("addr field: {e}")))?;

    let terms = tokenize(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    // OR over the analyzed query terms. Each clause scores via BM25; tantivy
    // sums clause scores per document.
    let clauses: Vec<(Occur, Box<dyn Query>)> = terms
        .into_iter()
        .map(|t| {
            let term = Term::from_field_text(body, &t);
            let q: Box<dyn Query> = Box::new(TermQuery::new(
                term,
                IndexRecordOption::WithFreqsAndPositions,
            ));
            (Occur::Should, q)
        })
        .collect();
    let query = BooleanQuery::new(clauses);

    let reader = index
        .reader()
        .map_err(|e| FtsError::Tantivy(format!("reader: {e}")))?;
    let searcher = reader.searcher();
    let hits = searcher
        .search(&query, &TopDocs::with_limit(top_k))
        .map_err(|e| FtsError::Tantivy(format!("search: {e}")))?;

    let mut out = Vec::with_capacity(hits.len());
    for (score, doc_addr) in hits {
        let doc: TantivyDocument = searcher
            .doc(doc_addr)
            .map_err(|e| FtsError::Tantivy(format!("doc fetch: {e}")))?;
        let packed = doc
            .get_first(addr_field)
            .and_then(|v| v.as_u64())
            .ok_or_else(|| FtsError::Corrupt("doc missing addr field".into()))?;
        out.push((unpack_addr(packed), score));
    }
    Ok(out)
}
