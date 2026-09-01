//! The `pensieve-word-v1` tokenizer.
//!
//! This crate's tokenizer is a **superset-compatible match** for the extent
//! writer's token rule (`pensieve-format-tlm`'s `writer::tokenize`), so the
//! catalog's existing word-level token prune (`tokens @> ["needle"]`) can never
//! falsely exclude an extent that has an FTS sidecar: every term the writer
//! records, tantivy also indexes.
//!
//! ## The rule (identical to the writer)
//!
//! Walk the string char-by-char. Accumulate runs of **ASCII-alphanumeric**
//! characters (`[0-9A-Za-z]`), lowercased; any other character (ASCII
//! punctuation, whitespace, or **any non-ASCII char**, since `is_ascii_…` is
//! false for those) terminates the current run. Emit a run as a token only when
//! it is **≥ 2 chars** long. Single-char runs and empty runs are dropped.
//!
//! Concretely, for `"OutOfMemoryError: retry_count=3 — café"` both the writer
//! and this tokenizer produce `["outofmemoryerror", "retry", "count", "3"...]`
//! — wait, `"3"` is one char so it is dropped; `"café"` splits at the non-ASCII
//! `é` into `"caf"` (the `é` is not ASCII-alphanumeric). The point is that the
//! two implementations apply byte-for-byte the same predicate
//! (`char::is_ascii_alphanumeric` + `to_ascii_lowercase` + len ≥ 2), so their
//! token *sets* are identical, which is stronger than the superset property the
//! prune requires.
//!
//! ## Why a custom tantivy tokenizer (not `SimpleTokenizer` + filters)
//!
//! tantivy's `SimpleTokenizer` splits on `!char::is_alphanumeric()`, which is
//! **Unicode**-aware: it keeps `é`, `日`, `①` inside tokens. That is a *superset*
//! of our ASCII rule for non-ASCII text (it would index `café` as one token
//! where we index `caf`), and `LowerCaser` + a `RemoveLongFilter` won't narrow
//! it back to the ASCII boundary. To guarantee the writer's tokens are a subset
//! of tantivy's terms we tokenize with the writer's exact predicate. We register
//! it under the name [`TOKENIZER_NAME`].

use tantivy::tokenizer::{BoxTokenStream, Token, TokenStream, Tokenizer};

/// Stable registry name for the tokenizer. Persisted into the sidecar `params`
/// (`{"tokenizer":"pensieve-word-v1", …}`) and used by both the build and search
/// paths so the same analysis applies to documents and queries.
pub const TOKENIZER_NAME: &str = "pensieve-word-v1";

/// Minimum token length (chars). Runs shorter than this are dropped — matches
/// the writer's `cur.len() >= 2` check (a token is pure ASCII so byte-len ==
/// char-count).
pub const MIN_TOKEN_LEN: usize = 2;

/// Standalone reference tokenizer: split `s` into the same lowercased,
/// ASCII-alphanumeric, len-≥2 tokens the extent writer records.
///
/// This is the function the parity tests compare against the writer's rule. It
/// is also the exact logic the tantivy [`PensieveWordTokenizer`] applies, so the
/// indexed terms equal `tokenize(doc)` and the query terms equal
/// `tokenize(query)`.
pub fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::with_capacity(16);
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c.to_ascii_lowercase());
        } else if !cur.is_empty() {
            if cur.len() >= MIN_TOKEN_LEN {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if cur.len() >= MIN_TOKEN_LEN {
        out.push(cur);
    }
    out
}

/// tantivy [`Tokenizer`] applying the `pensieve-word-v1` rule (see module docs).
#[derive(Clone, Default)]
pub struct PensieveWordTokenizer;

impl Tokenizer for PensieveWordTokenizer {
    type TokenStream<'a> = BoxTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        // We precompute tokens with byte offsets so phrase positions are
        // correct. Building the Vec up front keeps the predicate identical to
        // `tokenize` (the parity oracle) rather than re-deriving it lazily.
        let mut tokens = Vec::new();
        let mut pos = 0usize;
        let mut cur = String::with_capacity(16);
        let mut start_byte = 0usize;
        let mut byte = 0usize;
        for c in text.chars() {
            let clen = c.len_utf8();
            if c.is_ascii_alphanumeric() {
                if cur.is_empty() {
                    start_byte = byte;
                }
                cur.push(c.to_ascii_lowercase());
            } else if !cur.is_empty() {
                if cur.len() >= MIN_TOKEN_LEN {
                    tokens.push(Token {
                        offset_from: start_byte,
                        offset_to: byte,
                        position: pos,
                        text: std::mem::take(&mut cur),
                        position_length: 1,
                    });
                    pos += 1;
                } else {
                    cur.clear();
                }
            }
            byte += clen;
        }
        if cur.len() >= MIN_TOKEN_LEN {
            tokens.push(Token {
                offset_from: start_byte,
                offset_to: byte,
                position: pos,
                text: cur,
                position_length: 1,
            });
        }
        BoxTokenStream::new(VecTokenStream { tokens, idx: 0 })
    }
}

/// A [`TokenStream`] over a precomputed `Vec<Token>`.
struct VecTokenStream {
    tokens: Vec<Token>,
    idx: usize,
}

impl TokenStream for VecTokenStream {
    fn advance(&mut self) -> bool {
        if self.idx < self.tokens.len() {
            self.idx += 1;
            true
        } else {
            false
        }
    }

    fn token(&self) -> &Token {
        // `advance` was called before `token`, so idx is in 1..=len.
        &self.tokens[self.idx - 1]
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.idx - 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::tokenizer::Tokenizer;

    /// Collect the terms a tantivy `PensieveWordTokenizer` emits for `text`.
    fn tantivy_terms(text: &str) -> Vec<String> {
        let mut tok = PensieveWordTokenizer;
        let mut stream = tok.token_stream(text);
        let mut out = Vec::new();
        while stream.advance() {
            out.push(stream.token().text.clone());
        }
        out
    }

    #[test]
    fn tantivy_stream_matches_standalone_tokenize() {
        let cases = [
            "OutOfMemoryError: retry_count=3",
            "CamelCaseWord and snake_case_word",
            "  leading and trailing  ",
            "café über naïve",
            "a bb ccc d",
            "123 45 6",
            "mixed123abc-XYZ",
            "",
            "!!!",
            "日本語 text here",
        ];
        for c in cases {
            assert_eq!(tantivy_terms(c), tokenize(c), "mismatch for {c:?}");
        }
    }
}
