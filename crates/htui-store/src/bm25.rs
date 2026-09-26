//! BM25 sparse vectors for the concepts index (MOD-34 D1, `docs/ANA-20.md` §3.3).
//!
//! The sparse half of hybrid search exists so that exact identifiers (`MOD-34`, `R-STO-1`) and code
//! tokens are found even where the dense model blurs them. This module computes the per-document
//! half of BM25, the saturated and length-normalised term frequency; the collection's sparse
//! vector is created with `Modifier::Idf`, so Qdrant supplies the inverse document frequency from
//! the corpus it holds. No model and no vocabulary file: a term's index is a hash of the term.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Term-frequency saturation. The usual BM25 default.
pub const K1: f32 = 1.2;
/// Length normalisation. The usual BM25 default.
pub const B: f32 = 0.75;
/// The document length (in tokens) treated as average. A constant rather than a corpus statistic
/// so a point's vector never depends on what else is indexed; Qdrant's own BM25 does the same.
pub const AVG_LEN: f32 = 256.0;

/// A sparse vector: sorted, unique `indices` with one weight each.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SparseVector {
    /// Term indices, ascending, no duplicates.
    pub indices: Vec<u32>,
    /// One weight per index.
    pub values: Vec<f32>,
}

/// Splits `text` into lowercase terms.
///
/// A run of alphanumerics joined by `-` or `_` is kept whole (`mod-34`, `item_key`) **and** as its
/// parts (`mod`, `34`), so a query for the whole key and a query for one part both match.
/// Single-character terms are dropped.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in text.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')) {
        let compound = raw.trim_matches(['-', '_']).to_lowercase();
        if compound.is_empty() {
            continue;
        }
        let parts: Vec<&str> = compound
            .split(['-', '_'])
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() > 1 {
            push_term(&mut terms, &compound);
        }
        for part in parts {
            push_term(&mut terms, part);
        }
    }
    terms
}

fn push_term(terms: &mut Vec<String>, term: &str) {
    if term.chars().count() > 1 {
        terms.push(term.to_owned());
    }
}

/// The sparse index of a term: the first four bytes of its SHA-256, little-endian. Stable across
/// runs, builds and platforms, which is what lets an index outlive the process that wrote it.
#[must_use]
pub fn term_index(term: &str) -> u32 {
    let digest = Sha256::digest(term.as_bytes());
    u32::from_le_bytes(digest[..4].try_into().expect("4 bytes"))
}

/// The document-side vector: per term, `tf·(k1+1) / (tf + k1·(1 − b + b·len/avg))`.
#[must_use]
pub fn document_vector(text: &str) -> SparseVector {
    let terms = tokenize(text);
    #[allow(clippy::cast_precision_loss)] // token counts are far below f32's exact range
    let len = terms.len() as f32;
    let mut tf: BTreeMap<u32, f32> = BTreeMap::new();
    for term in &terms {
        *tf.entry(term_index(term)).or_default() += 1.0;
    }
    let norm = K1 * (1.0 - B + B * len / AVG_LEN);
    let (indices, values) = tf
        .into_iter()
        .map(|(i, f)| (i, f * (K1 + 1.0) / (f + norm)))
        .unzip();
    SparseVector { indices, values }
}

/// The query-side vector: weight 1 per distinct term, so the score is the sum of the matched
/// terms' document weights times Qdrant's IDF.
#[must_use]
pub fn query_vector(text: &str) -> SparseVector {
    let indices: std::collections::BTreeSet<u32> =
        tokenize(text).iter().map(|t| term_index(t)).collect();
    let values = vec![1.0; indices.len()];
    SparseVector {
        indices: indices.into_iter().collect(),
        values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weight(v: &SparseVector, term: &str) -> f32 {
        let i = term_index(term);
        v.indices
            .iter()
            .position(|&x| x == i)
            .map_or(0.0, |p| v.values[p])
    }

    #[test]
    fn keys_are_kept_whole_and_split() {
        assert_eq!(
            tokenize("See MOD-34 and R-STO-1."),
            ["see", "mod-34", "mod", "34", "and", "r-sto-1", "sto"]
        );
    }

    #[test]
    fn punctuation_and_single_characters_are_dropped() {
        assert_eq!(
            tokenize("a (b) --c-- item_key!"),
            ["item_key", "item", "key"]
        );
        assert!(tokenize("  -- _ ").is_empty());
    }

    #[test]
    fn term_index_is_stable() {
        // Golden values: a change here orphans every sparse vector already indexed.
        assert_eq!(term_index("qdrant"), 0x185d_c3cf);
        assert_eq!(term_index("mod-34"), 0xb197_6ed9);
    }

    #[test]
    fn vectors_are_sorted_and_unique() {
        let v = document_vector("store store cache store migration");
        assert!(v.indices.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(v.indices.len(), v.values.len());
        assert_eq!(v.indices.len(), 3);
    }

    #[test]
    fn repeated_terms_saturate() {
        let once = weight(&document_vector("store"), "store");
        let twice = weight(&document_vector("store store"), "store");
        let many = weight(&document_vector(&"store ".repeat(50)), "store");
        assert!(twice > once);
        assert!(many < K1 + 1.0);
    }

    #[test]
    fn longer_texts_weigh_a_term_less() {
        let short = weight(&document_vector("qdrant search"), "qdrant");
        let long_text = format!("qdrant {}", "filler ".repeat(500));
        let long = weight(&document_vector(&long_text), "qdrant");
        assert!(short > long);
    }

    #[test]
    fn query_weights_are_one_per_distinct_term() {
        let q = query_vector("MOD-34 mod");
        assert_eq!(q.indices.len(), 3); // mod-34, mod, 34
        assert!(q.values.iter().all(|&w| (w - 1.0).abs() < f32::EPSILON));
    }
}
