//! The benchmark's shared vocabulary (MOD-162 T2): corpus IDs, documents, ranked rows, backends,
//! query classes, and the conversion of production search hits into ranked lists.

use std::collections::HashSet;

use htui_core::model::Item;
use htui_store::vector::Hit;

/// Depth of every ranked list recorded per query and backend (MRR@10, recall@5, report ranks).
pub const RANK_DEPTH: usize = 10;
/// `S`, `S-dense`, `S-sparse`: hits asked of Qdrant before collapsing (ANA-37 §5.1).
pub const S_LIMIT: u64 = 40;
/// `S10` and `S-points`: htui's default limit (`crates/htui/src/concepts.rs` `DEFAULT_LIMIT`).
pub const S10_LIMIT: u64 = 10;
/// Prefix of a closed item's (a decision's) corpus ID (ANA-37 §5.3).
pub const DECISION_PREFIX: &str = "DEC:";

/// A corpus document's ID, as queries name targets: `MOD-99` (open item), `R-STO-8`
/// (requirement), `DEC:ANA-4` (decision, the closed item and its summary document).
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct CorpusId(String);

impl CorpusId {
    /// The ID as written.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// `DEC:<key>`: a decision's ID.
    pub fn decision(key: &str) -> Self {
        Self(format!("{DECISION_PREFIX}{key}"))
    }

    /// The ID as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is a decision's ID (`DEC:` prefix).
    pub fn is_decision(&self) -> bool {
        self.0.starts_with(DECISION_PREFIX)
    }

    /// P3: `DEC:<key>` when `hit.resolution.is_some()`, else `hit.key`.
    pub fn of_hit(hit: &Hit) -> Self {
        if hit.resolution.is_some() {
            Self::decision(&hit.key)
        } else {
            Self::new(hit.key.as_str())
        }
    }

    /// `DEC:<key>` for a closed item (`resolution.is_some()`), else its key.
    pub fn of_item(item: &Item) -> Self {
        if item.resolution.is_some() {
            Self::decision(&item.key)
        } else {
            Self::new(item.key.as_str())
        }
    }
}

impl std::fmt::Display for CorpusId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which source a document comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    /// An open item of `HANDOFF.md`.
    OpenItem,
    /// A requirement bullet of `docs/REQUIREMENTS.md`.
    Requirement,
    /// A decision file under `docs/decisions/`.
    Decision,
}

/// One corpus document: what L and G search, what queries target.
#[derive(Debug, Clone, PartialEq)]
pub struct CorpusDoc {
    /// The document's ID.
    pub id: CorpusId,
    /// Its source.
    pub kind: DocKind,
    /// `MOD-99`, `R-STO-8`, `ANA-4` (no `DEC:`).
    pub key: String,
    /// Item or decision title; empty for a requirement.
    pub title: String,
    /// The lexical text (§3.2): for items and requirements the point text exactly, for a
    /// decision the whole file.
    pub text: String,
    /// `HANDOFF.md:553`, `docs/REQUIREMENTS.md:290`, `docs/decisions/ana/ana-4.md` (report only).
    pub source: String,
}

/// One row of a ranked list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Ranked {
    /// The document ranked.
    pub id: CorpusId,
    /// The backend's score; higher is better, comparable only within one list.
    pub score: f64,
}

/// Every backend, in report order. Serialised as its `name()`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Backend {
    /// The production hybrid search, 40 hits collapsed to one row per document.
    #[serde(rename = "S")]
    S,
    /// The production hybrid search at htui's default limit of 10, collapsed.
    #[serde(rename = "S10")]
    S10,
    /// The production hybrid search at limit 10, one row per point (ANA-37 §11).
    #[serde(rename = "S-points")]
    SPoints,
    /// The dense arm alone, collapsed.
    #[serde(rename = "S-dense")]
    SDense,
    /// The sparse (BM25) arm alone, collapsed.
    #[serde(rename = "S-sparse")]
    SSparse,
    /// Postgres full text, `websearch_to_tsquery` as parsed (AND).
    #[serde(rename = "L-and")]
    LAnd,
    /// Postgres full text, the tsquery's `&` rewritten to `|`.
    #[serde(rename = "L-or")]
    LOr,
    /// `L-and` with length normalisation.
    #[serde(rename = "L-and-len")]
    LAndLen,
    /// `L-or` with length normalisation.
    #[serde(rename = "L-or-len")]
    LOrLen,
    /// Keyword grep, ties by occurrences.
    #[serde(rename = "G")]
    G,
    /// Keyword grep, ties by density.
    #[serde(rename = "G-dens")]
    GDens,
    /// htui's no-Qdrant filter, mirrored.
    #[serde(rename = "H")]
    H,
}

impl Backend {
    /// Every backend, in report order.
    pub const ALL: [Self; 12] = [
        Self::S,
        Self::S10,
        Self::SPoints,
        Self::SDense,
        Self::SSparse,
        Self::LAnd,
        Self::LOr,
        Self::LAndLen,
        Self::LOrLen,
        Self::G,
        Self::GDens,
        Self::H,
    ];

    /// The report's name: `"S"`, `"S-points"`, `"L-or-len"`, …
    pub const fn name(self) -> &'static str {
        match self {
            Self::S => "S",
            Self::S10 => "S10",
            Self::SPoints => "S-points",
            Self::SDense => "S-dense",
            Self::SSparse => "S-sparse",
            Self::LAnd => "L-and",
            Self::LOr => "L-or",
            Self::LAndLen => "L-and-len",
            Self::LOrLen => "L-or-len",
            Self::G => "G",
            Self::GDens => "G-dens",
            Self::H => "H",
        }
    }

    /// L-*, G, G-dens, H: the baselines ("every lexical" in the report).
    pub const fn is_lexical(self) -> bool {
        match self {
            Self::S | Self::S10 | Self::SPoints | Self::SDense | Self::SSparse => false,
            Self::LAnd
            | Self::LOr
            | Self::LAndLen
            | Self::LOrLen
            | Self::G
            | Self::GDens
            | Self::H => true,
        }
    }

    /// The Postgres variant of an `L-*` backend; `None` for every other.
    pub const fn l_variant(self) -> Option<LVariant> {
        match self {
            Self::LAnd => Some(LVariant::And),
            Self::LOr => Some(LVariant::Or),
            Self::LAndLen => Some(LVariant::AndLen),
            Self::LOrLen => Some(LVariant::OrLen),
            Self::S
            | Self::S10
            | Self::SPoints
            | Self::SDense
            | Self::SSparse
            | Self::G
            | Self::GDens
            | Self::H => None,
        }
    }
}

impl std::str::FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|backend| backend.name() == s)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|backend| backend.name()).collect();
                format!(
                    "unknown backend {s:?}; expected one of {}",
                    names.join(", ")
                )
            })
    }
}

/// The four Postgres full-text variants (P4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LVariant {
    /// `websearch_to_tsquery` as parsed.
    And,
    /// Its `&` rewritten to `|`.
    Or,
    /// `And` with length normalisation.
    AndLen,
    /// `Or` with length normalisation.
    OrLen,
}

impl LVariant {
    /// `ts_rank_cd` normalisation: 32 for And/Or, 34 (= 2|32) for the `-len` pair.
    pub const fn normalization(self) -> i32 {
        match self {
            Self::And | Self::Or => 32,
            Self::AndLen | Self::OrLen => 2 | 32,
        }
    }

    /// Or, OrLen: the tsquery's `&` rewritten to `|`.
    pub const fn is_or(self) -> bool {
        matches!(self, Self::Or | Self::OrLen)
    }
}

/// Query classes (ANA-37 §5.1).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum QueryClass {
    /// Duplicates: a paraphrase of one open item.
    #[serde(rename = "a")]
    A,
    /// Questions whose answer uses other words.
    #[serde(rename = "b")]
    B,
    /// Identifiers: keys and rare terms.
    #[serde(rename = "c")]
    C,
    /// No target: plausible requests the corpus does not answer.
    #[serde(rename = "d")]
    D,
}

impl QueryClass {
    /// Every class, in report order.
    pub const ALL: [Self; 4] = [Self::A, Self::B, Self::C, Self::D];

    /// `"a"` … `"d"`.
    pub const fn letter(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
            Self::D => "d",
        }
    }

    /// `"duplicates"`, `"questions"`, `"identifiers"`, `"no target"`.
    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "duplicates",
            Self::B => "questions",
            Self::C => "identifiers",
            Self::D => "no target",
        }
    }

    /// Queries of this class in a set: 24, 20, 16, 12.
    pub const fn expected_count(self) -> usize {
        match self {
            Self::A => 24,
            Self::B => 20,
            Self::C => 16,
            Self::D => 12,
        }
    }
}

impl std::str::FromStr for QueryClass {
    type Err = String;

    /// By [`QueryClass::letter`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|class| class.letter() == s)
            .ok_or_else(|| format!("unknown query class {s:?}; expected one of a, b, c, d"))
    }
}

/// Hits to a ranked list of at most `depth` rows, IDs by [`CorpusId::of_hit`], score as f64.
/// `collapse`: one row per corpus ID, first occurrence wins (S, S10, S-dense, S-sparse);
/// otherwise one row per point, duplicates kept (S-points, ANA-37 §11).
pub fn ranked_from_hits(hits: &[Hit], collapse: bool, depth: usize) -> Vec<Ranked> {
    let mut seen = HashSet::new();
    hits.iter()
        .map(|hit| Ranked {
            id: CorpusId::of_hit(hit),
            score: f64::from(hit.score),
        })
        .filter(|row| !collapse || seen.insert(row.id.clone()))
        .take(depth)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::{DateTime, Utc};
    use htui_core::model::{
        DocumentId, Item, ItemId, ItemKindId, ProjectId, RequirementId, Resolution, Status, UserId,
    };
    use htui_store::vector::{Hit, Owner, PointType};

    use super::*;

    fn hit(key: &str, resolution: Option<Resolution>, score: f32) -> Hit {
        Hit {
            point_type: PointType::Item,
            owner: Owner::Item(ItemId::default()),
            key: key.to_owned(),
            document: None,
            resolution,
            state: None,
            score,
            snippet: format!("{key} snippet"),
        }
    }

    fn doc_hit(key: &str, resolution: Option<Resolution>, score: f32) -> Hit {
        Hit {
            point_type: PointType::Document,
            document: Some((DocumentId::default(), "summary".to_owned())),
            ..hit(key, resolution, score)
        }
    }

    fn requirement_hit(key: &str, score: f32) -> Hit {
        Hit {
            point_type: PointType::Requirement,
            owner: Owner::Requirement(RequirementId::default()),
            ..hit(key, None, score)
        }
    }

    fn item(key: &str, resolution: Option<Resolution>) -> Item {
        let at: DateTime<Utc> = DateTime::from_timestamp(1_760_000_000, 0).expect("in range");
        let (prefix, number) = key.rsplit_once('-').expect("a key");
        Item {
            id: ItemId::default(),
            project_id: ProjectId::default(),
            kind_id: ItemKindId::default(),
            key_prefix: prefix.to_owned(),
            key_number: number.parse().expect("a number"),
            key: key.to_owned(),
            title: "title".to_owned(),
            body: String::new(),
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
            created_at: at,
            updated_at: at,
            closed_at: resolution.map(|_| at),
            resolution,
        }
    }

    fn ids(ranked: &[Ranked]) -> Vec<&str> {
        ranked.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn corpus_ids_name_open_items_requirements_and_decisions() {
        let open = CorpusId::new("MOD-99");
        assert_eq!(open.as_str(), "MOD-99");
        assert_eq!(open.to_string(), "MOD-99");
        assert!(!open.is_decision());

        let decision = CorpusId::decision("ANA-4");
        assert_eq!(decision.as_str(), "DEC:ANA-4");
        assert!(decision.is_decision());
        assert_eq!(decision, CorpusId::new("DEC:ANA-4"));

        assert!(!CorpusId::new("R-STO-8").is_decision());
    }

    #[test]
    fn corpus_ids_serialise_as_a_bare_string() {
        let id = CorpusId::decision("MOD-41");
        let json = serde_json::to_string(&id).expect("serialises");
        assert_eq!(json, "\"DEC:MOD-41\"");
        assert_eq!(serde_json::from_str::<CorpusId>(&json).expect("parses"), id);
    }

    #[test]
    fn of_hit_maps_a_closed_items_hit_to_its_decision() {
        assert_eq!(
            CorpusId::of_hit(&hit("MOD-99", None, 1.0)).as_str(),
            "MOD-99"
        );
        assert_eq!(
            CorpusId::of_hit(&hit("ANA-4", Some(Resolution::Concluded), 1.0)).as_str(),
            "DEC:ANA-4"
        );
        assert_eq!(
            CorpusId::of_hit(&doc_hit("MOD-41", Some(Resolution::Done), 1.0)).as_str(),
            "DEC:MOD-41"
        );
        assert_eq!(
            CorpusId::of_hit(&requirement_hit("R-STO-8", 1.0)).as_str(),
            "R-STO-8"
        );
    }

    #[test]
    fn of_item_maps_a_closed_item_to_its_decision() {
        assert_eq!(CorpusId::of_item(&item("MOD-99", None)).as_str(), "MOD-99");
        assert_eq!(
            CorpusId::of_item(&item("ANA-4", Some(Resolution::Rejected))).as_str(),
            "DEC:ANA-4"
        );
    }

    #[test]
    fn ranked_from_hits_collapses_to_the_first_row_per_document() {
        let hits = [
            hit("MOD-1", None, 0.9),
            doc_hit("ANA-4", Some(Resolution::Done), 0.8),
            doc_hit("MOD-1", None, 0.7),
            hit("ANA-4", Some(Resolution::Done), 0.6),
            requirement_hit("R-STO-8", 0.5),
            hit("MOD-2", None, 0.4),
        ];

        let collapsed = ranked_from_hits(&hits, true, 10);
        assert_eq!(ids(&collapsed), ["MOD-1", "DEC:ANA-4", "R-STO-8", "MOD-2"]);
        // First occurrence wins, its score kept.
        assert_eq!(collapsed[0].score, f64::from(0.9_f32));
        assert_eq!(collapsed[1].score, f64::from(0.8_f32));

        // Depth counts collapsed rows, not hits.
        let shallow = ranked_from_hits(&hits, true, 3);
        assert_eq!(ids(&shallow), ["MOD-1", "DEC:ANA-4", "R-STO-8"]);
    }

    #[test]
    fn ranked_from_hits_keeps_duplicate_points_uncollapsed() {
        let hits = [
            hit("MOD-1", None, 0.9),
            doc_hit("MOD-1", None, 0.8),
            doc_hit("MOD-1", None, 0.7),
            hit("MOD-2", None, 0.6),
        ];

        let points = ranked_from_hits(&hits, false, 10);
        assert_eq!(ids(&points), ["MOD-1", "MOD-1", "MOD-1", "MOD-2"]);
        assert_eq!(points[2].score, f64::from(0.7_f32));

        let shallow = ranked_from_hits(&hits, false, 2);
        assert_eq!(ids(&shallow), ["MOD-1", "MOD-1"]);

        assert!(ranked_from_hits(&hits, false, 0).is_empty());
        assert!(ranked_from_hits(&[], true, 10).is_empty());
    }

    #[test]
    fn backend_names_round_trip_through_from_str_and_serde() {
        let names: Vec<&str> = Backend::ALL.iter().map(|b| b.name()).collect();
        assert_eq!(
            names,
            [
                "S",
                "S10",
                "S-points",
                "S-dense",
                "S-sparse",
                "L-and",
                "L-or",
                "L-and-len",
                "L-or-len",
                "G",
                "G-dens",
                "H"
            ]
        );
        for backend in Backend::ALL {
            assert_eq!(Backend::from_str(backend.name()), Ok(backend));
            let json = serde_json::to_string(&backend).expect("serialises");
            assert_eq!(json, format!("\"{}\"", backend.name()));
            assert_eq!(
                serde_json::from_str::<Backend>(&json).expect("parses"),
                backend
            );
        }
        assert!(Backend::from_str("s").is_err());
        assert!(Backend::from_str("L").is_err());
    }

    #[test]
    fn backend_all_is_sorted_in_report_order() {
        let mut sorted = Backend::ALL;
        sorted.sort();
        assert_eq!(sorted, Backend::ALL);
    }

    #[test]
    fn lexical_backends_are_the_baselines() {
        let lexical: Vec<Backend> = Backend::ALL
            .into_iter()
            .filter(|b| b.is_lexical())
            .collect();
        assert_eq!(
            lexical,
            [
                Backend::LAnd,
                Backend::LOr,
                Backend::LAndLen,
                Backend::LOrLen,
                Backend::G,
                Backend::GDens,
                Backend::H
            ]
        );
    }

    #[test]
    fn l_backends_carry_their_postgres_variant() {
        assert_eq!(Backend::LAnd.l_variant(), Some(LVariant::And));
        assert_eq!(Backend::LOr.l_variant(), Some(LVariant::Or));
        assert_eq!(Backend::LAndLen.l_variant(), Some(LVariant::AndLen));
        assert_eq!(Backend::LOrLen.l_variant(), Some(LVariant::OrLen));
        for backend in [Backend::S, Backend::SPoints, Backend::G, Backend::H] {
            assert_eq!(backend.l_variant(), None);
        }

        assert_eq!(LVariant::And.normalization(), 32);
        assert_eq!(LVariant::Or.normalization(), 32);
        assert_eq!(LVariant::AndLen.normalization(), 34);
        assert_eq!(LVariant::OrLen.normalization(), 34);
        assert!(!LVariant::And.is_or());
        assert!(LVariant::Or.is_or());
        assert!(!LVariant::AndLen.is_or());
        assert!(LVariant::OrLen.is_or());
    }

    #[test]
    fn query_classes_round_trip_through_from_str_and_serde() {
        let letters: Vec<&str> = QueryClass::ALL.iter().map(|c| c.letter()).collect();
        assert_eq!(letters, ["a", "b", "c", "d"]);
        let labels: Vec<&str> = QueryClass::ALL.iter().map(|c| c.label()).collect();
        assert_eq!(
            labels,
            ["duplicates", "questions", "identifiers", "no target"]
        );
        let counts: Vec<usize> = QueryClass::ALL.iter().map(|c| c.expected_count()).collect();
        assert_eq!(counts, [24, 20, 16, 12]);

        for class in QueryClass::ALL {
            assert_eq!(QueryClass::from_str(class.letter()), Ok(class));
            let json = serde_json::to_string(&class).expect("serialises");
            assert_eq!(json, format!("\"{}\"", class.letter()));
            assert_eq!(
                serde_json::from_str::<QueryClass>(&json).expect("parses"),
                class
            );
        }
        assert!(QueryClass::from_str("A").is_err());
        assert!(QueryClass::from_str("e").is_err());
    }
}
