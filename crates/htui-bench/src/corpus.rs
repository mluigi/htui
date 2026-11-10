//! The corpus (MOD-162 T3): `HANDOFF.md` open items, `docs/REQUIREMENTS.md` requirements and
//! `docs/decisions/**` decisions read from a pinned commit (or the work tree), parsed into
//! [`CorpusDoc`]s and into the rows `htui_store::vector_sync` builds points from.

use std::collections::BTreeSet;
use std::path::Path;

use htui_core::model::{
    Document, Item, Priority, ProjectId, Requirement, RequirementState, Resolution,
};
use htui_store::vector::ConceptPoint;
use uuid::Uuid;

use crate::types::{CorpusDoc, CorpusId};

/// ANA-37's corpus commit.
pub const DEFAULT_PIN: &str = "d15e9fea";
/// `--corpus-ref` value meaning "the files on disk", not a commit.
pub const WORKTREE: &str = "WORKTREE";

/// The three sources of one corpus, before parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    /// The resolved full commit sha; `None` for the work tree.
    pub commit: Option<String>,
    /// `HANDOFF.md`.
    pub handoff: String,
    /// `docs/REQUIREMENTS.md`.
    pub requirements: String,
    /// `(repo-relative path, text)` of every `docs/decisions/**/*.md`, sorted by path.
    pub decisions: Vec<(String, String)>,
}

/// An open item of `HANDOFF.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItem {
    /// `MOD-99`.
    pub key: String,
    /// The bold title after `KEY - `, verbatim.
    pub title: String,
    /// The rest of the header line and its continuation lines.
    pub body: String,
    /// 1-based line of the header.
    pub line: usize,
}

/// A requirement bullet of `docs/REQUIREMENTS.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementBullet {
    /// `R-STO-8`.
    pub key: String,
    /// `STO`.
    pub area_code: String,
    /// `8`.
    pub number: i32,
    /// From the header's parenthesis (blueprint §3.2 rule 2).
    pub priority: Priority,
    /// `Withdrawn` for `(withdrawn)`, else `Active`.
    pub state: RequirementState,
    /// The rest of the header line and its continuation lines.
    pub body: String,
    /// 1-based line of the header.
    pub line: usize,
}

/// A decision file's first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionHeader {
    /// `ANA-4` (no `DEC:`).
    pub key: String,
    /// The title, without the status group.
    pub title: String,
    /// From the status group's first word; `Done` when there is none.
    pub resolution: Resolution,
}

/// The corpus: documents for L, G and the query targets; rows for H and the point builders.
#[derive(Debug, Clone, PartialEq)]
pub struct Corpus {
    /// As requested: a rev, or [`WORKTREE`].
    pub corpus_ref: String,
    /// Resolved commit; `None` for the work tree.
    pub commit: Option<String>,
    /// The one synthetic project (`bench_uuid("project")`).
    pub project: ProjectId,
    /// Open items in file order, then requirements in file order, then decisions in path order.
    pub docs: Vec<CorpusDoc>,
    /// Open items, then decision items, in `docs` order.
    pub items: Vec<Item>,
    /// In `docs` order.
    pub requirements: Vec<Requirement>,
    /// One `summary` per decision item, in the same order.
    pub documents: Vec<Document>,
}

/// Points by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct PointCounts {
    /// Item points.
    pub item: usize,
    /// Requirement points.
    pub requirement: usize,
    /// Document (section) points.
    pub document: usize,
}

impl Corpus {
    /// Every document's ID.
    pub fn ids(&self) -> BTreeSet<CorpusId> {
        let _ = &self.docs;
        BTreeSet::new()
    }

    /// The document with this ID.
    pub fn doc(&self, id: &CorpusId) -> Option<&CorpusDoc> {
        let _ = (&self.docs, id);
        None
    }

    /// `item_point` for every item, `document_points(item, doc)` for every summary,
    /// `requirement_point` for every requirement, in that order; `htui_store::vector_sync`
    /// unchanged (P3).
    pub fn points(&self) -> Vec<ConceptPoint> {
        let _ = (&self.items, &self.documents, &self.requirements);
        Vec::new()
    }

    /// Counts by `ConceptPoint::point_type()`.
    pub fn point_counts(&self) -> PointCounts {
        let _ = &self.items;
        PointCounts::default()
    }
}

/// Namespaced UUID (P3): the first 16 bytes of sha256("htui-bench/v1\0" + name), stamped as a
/// version-8 (custom) UUID by `uuid::Builder::from_custom_bytes(..).into_uuid()`.
pub fn bench_uuid(name: &str) -> Uuid {
    let _ = name;
    Uuid::nil()
}

/// `git -C repo rev-parse --verify <rev>^{commit}`, then `git show <sha>:HANDOFF.md`,
/// `git show <sha>:docs/REQUIREMENTS.md`, `git ls-tree -r --name-only <sha> -- docs/decisions`
/// (only `*.md`) and `git show` of each. An unreachable rev is an error that names the rev and says
/// "unreachable in this clone (shallow?); pass --corpus-ref".
pub fn read_git(repo: &Path, rev: &str) -> anyhow::Result<Sources> {
    let _ = (repo, rev);
    anyhow::bail!("corpus::read_git: not implemented yet (MOD-162 T3)")
}

/// The same three sources from disk under `root`; decisions found by a recursive `std::fs` walk
/// (no new crate), sorted by repo-relative path with `/` separators.
pub fn read_worktree(root: &Path) -> anyhow::Result<Sources> {
    let _ = root;
    anyhow::bail!("corpus::read_worktree: not implemented yet (MOD-162 T3)")
}

/// Pure: parse, build rows and documents. An error names a decision file it cannot key.
pub fn build_from_sources(corpus_ref: &str, sources: Sources) -> anyhow::Result<Corpus> {
    let _ = (corpus_ref, sources);
    anyhow::bail!("corpus::build_from_sources: not implemented yet (MOD-162 T3)")
}

/// `read_git` then `build_from_sources`.
pub fn build_from_git(repo: &Path, rev: &str) -> anyhow::Result<Corpus> {
    let _ = (repo, rev);
    anyhow::bail!("corpus::build_from_git: not implemented yet (MOD-162 T3)")
}

/// [`WORKTREE`] gives `read_worktree`, anything else `build_from_git`.
pub fn build(repo: &Path, corpus_ref: &str) -> anyhow::Result<Corpus> {
    let _ = (repo, corpus_ref);
    anyhow::bail!("corpus::build: not implemented yet (MOD-162 T3)")
}

/// The open items of `HANDOFF.md` (blueprint §3.2 rule 1).
pub fn parse_open_items(handoff: &str) -> Vec<OpenItem> {
    let _ = handoff;
    Vec::new()
}

/// The requirement bullets of `docs/REQUIREMENTS.md` (blueprint §3.2 rule 2).
pub fn parse_requirements(text: &str) -> Vec<RequirementBullet> {
    let _ = text;
    Vec::new()
}

/// A decision file's first line (blueprint §3.2 rule 3); `None` when it carries no key.
pub fn parse_decision_header(first_line: &str) -> Option<DecisionHeader> {
    let _ = first_line;
    None
}

/// `Resolution::from_str(word)`, falling back to `Done` (shipped, decided, status, anything).
pub fn resolution_of(word: &str) -> Resolution {
    let _ = word;
    Resolution::Done
}
