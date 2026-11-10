//! The in-process lexical baselines (MOD-162 T4): G and G-dens (ANA-37 §5.4's keyword grep) and
//! H (htui's no-Qdrant filter, mirrored).

use htui_core::model::{Item, Requirement};

use crate::types::{CorpusDoc, CorpusId, Ranked};

/// Postgres' `english.stop` (snowball, 127 words), verbatim, so G drops what L drops.
pub const STOPWORDS: &[&str] = &[];

/// G's keywords: lowercase; split on every char that is not alphanumeric, `-` or `_`; trim `-`/`_`
/// at both ends; drop tokens shorter than 2 chars and stopwords; dedupe, first occurrence order.
/// `"R-STO-8"` → `["r-sto-8"]`; `"Plackett-Luce"` → `["plackett-luce"]`.
pub fn keywords(query: &str) -> Vec<String> {
    let _ = query;
    Vec::new()
}

/// G's tie-break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GTie {
    /// G.
    Occurrences,
    /// G-dens.
    Density,
}

/// Lower-cased corpus texts, built once (timing G measures the search, not the lowercasing).
#[derive(Debug, Clone)]
pub struct GrepIndex {
    /// `docs[i].id`.
    pub ids: Vec<CorpusId>,
    /// `docs[i].text.to_lowercase()`.
    pub lower: Vec<String>,
    /// `max(1, whitespace word count)` of `docs[i].text`.
    pub words: Vec<usize>,
}

impl GrepIndex {
    /// `lower[i] = docs[i].text.to_lowercase()`, `words[i] = max(1, whitespace word count)`.
    pub fn new(docs: &[CorpusDoc]) -> Self {
        let _ = docs;
        Self {
            ids: Vec::new(),
            lower: Vec::new(),
            words: Vec::new(),
        }
    }

    /// G / G-dens over the docs. For each doc: `matched` = keywords k with `lower.contains(k)`,
    /// `occurrences` = Σ `lower.matches(k).count()`, `density` = occurrences / words. Docs with
    /// matched == 0 are dropped. Sort by (matched desc, then occurrences or density desc, then doc
    /// order asc); take `limit`. `Ranked.score` = matched / keywords.len() (the §5.4 "fraction
    /// matched"). No keywords → empty.
    pub fn search(&self, query: &str, tie: GTie, limit: usize) -> Vec<Ranked> {
        let _ = (&self.ids, &self.lower, &self.words, query, tie, limit);
        Vec::new()
    }
}

/// H (P4): htui's no-Qdrant filter, mirrored. needle = `query.to_lowercase()` (not trimmed: the
/// production filter passes the text as typed). Items whose `key` or `title` lowercased contains
/// it, ordered by (`key_prefix` bytes, `key_number`); then requirements whose `key` or `body`
/// does, ordered by (`area_code` bytes, `number`). Item IDs by `CorpusId::of_item`. Score 1.0.
/// First `limit`.
pub fn htui_filter(
    items: &[Item],
    requirements: &[Requirement],
    query: &str,
    limit: usize,
) -> Vec<Ranked> {
    let _ = (items, requirements, query, limit);
    Vec::new()
}
