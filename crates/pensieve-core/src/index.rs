//! Index traits — bloom filters, inverted indices, path-presence bitmaps.
//!
//! Indices are internal to a [`SegmentFormat`][crate::segment_format::SegmentFormat]
//! implementation; engine code never constructs them directly. These traits
//! exist so exec-layer plumbing (cache, stats) can talk about indices
//! generically.

use crate::errors::Result;

/// Coarse classification of an index.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum IndexKind {
    /// Probabilistic "possibly present / definitely absent".
    Bloom,
    /// Full inverted index: term → posting list.
    Inverted,
    /// Per-path presence bitmap on a `dynamic` column.
    PathPresence,
    /// Per-block min/max for a typed column.
    MinMax,
}

/// A generic tagged index.
pub trait IndexProvider: Send + Sync {
    fn kind(&self) -> IndexKind;
}

/// A posting list: the set of rows (within a block) matching a term.
pub trait PostingList: Send + Sync {
    /// Approximate cardinality of the posting.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Intersect two posting lists; returns a new list.
    fn intersect(&self, other: &dyn PostingList) -> Box<dyn PostingList>;

    /// Iterate over row offsets within the owning block.
    fn iter_rows(&self) -> Box<dyn Iterator<Item = u32> + Send + '_>;
}

/// A term dictionary. For strings this is typically an FST; for small
/// dictionaries it may be a sorted byte-slice array.
pub trait TermDict: Send + Sync {
    /// Look up a term; returns its posting list if present.
    fn lookup(&self, term: &[u8]) -> Result<Option<Box<dyn PostingList>>>;

    /// Iterate terms matching a prefix (for `startswith` translation).
    fn prefix_scan<'a>(&'a self, prefix: &[u8]) -> Box<dyn Iterator<Item = Vec<u8>> + Send + 'a>;
}
