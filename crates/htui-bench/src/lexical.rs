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

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use htui_core::model::{
        ItemId, ItemKindId, Priority, ProjectId, RequirementAreaId, RequirementId,
        RequirementState, Resolution, Status, UserId,
    };

    use super::*;
    use crate::types::DocKind;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_760_000_000, 0).expect("in range")
    }

    fn doc(id: &str, text: &str) -> CorpusDoc {
        CorpusDoc {
            id: CorpusId::new(id),
            kind: DocKind::OpenItem,
            key: id.to_owned(),
            title: String::new(),
            text: text.to_owned(),
            source: format!("fixture:{id}"),
        }
    }

    fn item(key: &str, title: &str, body: &str, resolution: Option<Resolution>) -> Item {
        let (prefix, number) = key.rsplit_once('-').expect("a key");
        Item {
            id: ItemId::default(),
            project_id: ProjectId::default(),
            kind_id: ItemKindId::default(),
            key_prefix: prefix.to_owned(),
            key_number: number.parse().expect("a number"),
            key: key.to_owned(),
            title: title.to_owned(),
            body: body.to_owned(),
            status: if resolution.is_some() {
                Status::Closed
            } else {
                Status::Open
            },
            priority: 0,
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            step_graph_id: None,
            version: 1,
            created_by: UserId::default(),
            created_at: at(),
            updated_at: at(),
            closed_at: resolution.map(|_| at()),
            resolution,
        }
    }

    fn requirement(area: &str, number: i32, body: &str) -> Requirement {
        Requirement {
            id: RequirementId::default(),
            project_id: ProjectId::default(),
            area_id: RequirementAreaId::default(),
            area_code: area.to_owned(),
            number,
            key: format!("R-{area}-{number}"),
            body: body.to_owned(),
            rationale: String::new(),
            priority: Priority::Must,
            state: RequirementState::Active,
            version: 1,
            created_by: UserId::default(),
            created_at: at(),
            updated_at: at(),
        }
    }

    fn ids(ranked: &[Ranked]) -> Vec<&str> {
        ranked.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn stopwords_are_postgres_english_stop() {
        assert_eq!(STOPWORDS.len(), 127);
        assert_eq!(STOPWORDS.first(), Some(&"i"));
        assert_eq!(STOPWORDS.last(), Some(&"now"));
        let unique: std::collections::HashSet<_> = STOPWORDS.iter().collect();
        assert_eq!(unique.len(), 127, "no duplicates");
        for w in ["the", "and", "s", "t", "don", "should", "against", "yourselves"] {
            assert!(STOPWORDS.contains(&w), "{w}");
        }
    }

    #[test]
    fn keywords_drop_stopwords_and_keep_hyphenated_identifiers() {
        assert_eq!(keywords("R-STO-8"), vec!["r-sto-8"]);
        assert_eq!(keywords("Plackett-Luce"), vec!["plackett-luce"]);
        assert_eq!(
            keywords("What is the Plackett-Luce model for R-STO-8?"),
            vec!["plackett-luce", "model", "r-sto-8"]
        );
    }

    #[test]
    fn keywords_trim_dedupe_and_drop_short_tokens() {
        assert_eq!(
            keywords("--queue_ and a x QUEUE, queue_; snake_case -dash- 7 42"),
            vec!["queue", "snake_case", "dash", "42"]
        );
        assert!(keywords("the and of a").is_empty());
        assert!(keywords("").is_empty());
    }

    #[test]
    fn g_ranks_by_matched_count_first() {
        let docs = [
            doc("ONE", "alpha alpha alpha alpha"),
            doc("TWO", "alpha beta"),
            doc("NONE", "unrelated"),
        ];
        let index = GrepIndex::new(&docs);
        let ranked = index.search("alpha beta", GTie::Occurrences, 10);
        assert_eq!(ids(&ranked), vec!["TWO", "ONE"]);
        assert_eq!(ranked[0].score, 1.0);
        assert_eq!(ranked[1].score, 0.5);
    }

    #[test]
    fn g_breaks_ties_by_occurrences_and_g_dens_by_density() {
        let filler = vec!["filler"; 200].join(" ");
        let long = format!("alpha beta {filler} alpha beta alpha");
        let docs = [doc("SHORT", "Alpha beta"), doc("LONG", &long)];
        let index = GrepIndex::new(&docs);
        assert_eq!(index.words, vec![2, 205]);
        assert_eq!(index.lower[0], "alpha beta");

        let g = index.search("alpha beta", GTie::Occurrences, 10);
        assert_eq!(ids(&g), vec!["LONG", "SHORT"]);
        let dens = index.search("alpha beta", GTie::Density, 10);
        assert_eq!(ids(&dens), vec!["SHORT", "LONG"]);
        assert!(dens.iter().all(|r| r.score == 1.0));
    }

    #[test]
    fn g_full_ties_keep_doc_order_and_respect_the_limit() {
        let docs = [doc("A", "alpha"), doc("B", "alpha"), doc("C", "alpha")];
        let index = GrepIndex::new(&docs);
        assert_eq!(
            ids(&index.search("alpha", GTie::Occurrences, 10)),
            vec!["A", "B", "C"]
        );
        assert_eq!(ids(&index.search("alpha", GTie::Density, 2)), vec!["A", "B"]);
    }

    #[test]
    fn g_without_keywords_gives_no_rows_and_scores_the_fraction_matched() {
        let docs = [doc("A", "the and alpha"), doc("B", "beta gamma")];
        let index = GrepIndex::new(&docs);
        assert!(index.search("the and", GTie::Occurrences, 10).is_empty());
        assert!(index.search("", GTie::Density, 10).is_empty());
        let ranked = index.search("alpha beta gamma", GTie::Occurrences, 10);
        assert_eq!(ids(&ranked), vec!["B", "A"]);
        assert!((ranked[0].score - 2.0 / 3.0).abs() < 1e-12);
        assert!((ranked[1].score - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn grep_index_counts_at_least_one_word() {
        let index = GrepIndex::new(&[doc("EMPTY", "   ")]);
        assert_eq!(index.words, vec![1]);
        assert_eq!(index.ids, vec![CorpusId::new("EMPTY")]);
    }

    #[test]
    fn h_is_case_insensitive_on_item_key_and_title_but_not_body() {
        let items = [
            item("MOD-1", "Queue Starvation", "nothing", None),
            item("MOD-2", "other", "queue starvation in the body", None),
            item("TOOL-3", "unrelated", "", None),
        ];
        assert_eq!(ids(&htui_filter(&items, &[], "queue STARVATION", 10)), vec!["MOD-1"]);
        assert_eq!(ids(&htui_filter(&items, &[], "tool-3", 10)), vec!["TOOL-3"]);
        assert!(htui_filter(&items, &[], "in the body", 10).is_empty());
    }

    #[test]
    fn h_matches_requirement_key_and_body_and_puts_items_first() {
        let items = [item("MOD-5", "Store sync", "", None)];
        let requirements = [
            requirement("STO", 8, "The store keeps a sync journal."),
            requirement("ORCH", 2, "Unrelated."),
        ];
        let ranked = htui_filter(&items, &requirements, "SYNC", 10);
        assert_eq!(ids(&ranked), vec!["MOD-5", "R-STO-8"]);
        assert!(ranked.iter().all(|r| r.score == 1.0));
        assert_eq!(ids(&htui_filter(&items, &requirements, "r-orch-2", 10)), vec!["R-ORCH-2"]);
    }

    #[test]
    fn h_maps_a_closed_item_to_its_decision() {
        let items = [item("ANA-4", "Vector store choice", "", Some(Resolution::Concluded))];
        assert_eq!(ids(&htui_filter(&items, &[], "vector", 10)), vec!["DEC:ANA-4"]);
    }

    #[test]
    fn h_orders_by_prefix_bytes_then_number() {
        let items = [
            item("MOD-10", "x widget", "", None),
            item("TOOL-1", "x widget", "", None),
            item("ANA-3", "x widget", "", None),
            item("MOD-9", "x widget", "", None),
        ];
        let requirements = [
            requirement("TUI", 1, "widget"),
            requirement("ORCH", 15, "widget"),
            requirement("ORCH", 7, "widget"),
        ];
        assert_eq!(
            ids(&htui_filter(&items, &requirements, "widget", 10)),
            vec!["ANA-3", "MOD-9", "MOD-10", "TOOL-1", "R-ORCH-7", "R-ORCH-15", "R-TUI-1"]
        );
        assert_eq!(
            ids(&htui_filter(&items, &requirements, "widget", 3)),
            vec!["ANA-3", "MOD-9", "MOD-10"]
        );
    }

    #[test]
    fn h_treats_the_whole_query_as_one_needle() {
        let items = [item("MOD-1", "beta alpha", "", None)];
        let requirements = [requirement("STO", 1, "beta alpha")];
        assert!(htui_filter(&items, &requirements, "alpha beta", 10).is_empty());
        assert_eq!(htui_filter(&items, &requirements, "beta alpha", 10).len(), 2);
        // Not trimmed: a trailing space is part of the needle.
        assert!(htui_filter(&items, &requirements, "alpha ", 10).is_empty());
    }
}
