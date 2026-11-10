//! The query set (MOD-162 T5, P6): `queries/ana37-r2.json`, frozen by its `.sha256` sidecar,
//! parsed and shape-checked before any backend runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::types::{CorpusId, QueryClass};

/// The frozen set, compiled-in path (for `cargo run`).
pub const DEFAULT_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/queries/ana37-r2.json");

/// One query.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    /// `a01`: class letter and two digits.
    pub id: String,
    /// The query's class.
    pub class: QueryClass,
    /// The text searched.
    pub text: String,
    /// The corpus documents that answer it; empty for class `d`.
    pub targets: Vec<CorpusId>,
    /// A §5.3 text quoted truncated and completed by the author (a01, a04, a20).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub completed: bool,
}

/// The file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySet {
    /// `ana37-r2`.
    pub set: String,
    /// The corpus commit the targets name documents of.
    pub corpus_ref: String,
    /// The queries, in file order.
    pub queries: Vec<Query>,
}

/// A set whose bytes matched its sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenQueries {
    /// The query file read.
    pub path: PathBuf,
    /// Its sha256, lowercase hex.
    pub sha256: String,
    /// Its parsed, shape-checked content.
    pub set: QuerySet,
}

/// A target that names no corpus document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTarget {
    /// The query's ID.
    pub query: String,
    /// The target that is not a corpus ID.
    pub target: CorpusId,
}

impl std::fmt::Display for UnknownTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} is not a corpus ID", self.query, self.target)
    }
}

/// Lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let _ = bytes;
    String::new()
}

/// `<file>.sha256`.
pub fn sidecar_path(file: &Path) -> PathBuf {
    let _ = file;
    PathBuf::new()
}

/// `"{sha}  {file_name}\n"`.
pub fn sidecar_line(sha256: &str, file_name: &str) -> String {
    let _ = (sha256, file_name);
    String::new()
}

/// First whitespace-separated token; must be 64 lowercase hex chars.
pub fn parse_sidecar(text: &str) -> anyhow::Result<String> {
    let _ = text;
    anyhow::bail!("queries::parse_sidecar: not implemented yet (MOD-162 T5)")
}

/// Pure freeze check: `sha256_hex(bytes) == parse_sidecar(sidecar)`, else an error that names both
/// hashes and says the query file changed after its freeze. Returns the hash.
pub fn check_frozen(bytes: &[u8], sidecar: &str) -> anyhow::Result<String> {
    let _ = (bytes, sidecar);
    anyhow::bail!("queries::check_frozen: not implemented yet (MOD-162 T5)")
}

/// The query file's JSON.
pub fn parse(bytes: &[u8]) -> anyhow::Result<QuerySet> {
    let _ = bytes;
    anyhow::bail!("queries::parse: not implemented yet (MOD-162 T5)")
}

/// Every problem in one error, one per line: counts per class 24/20/16/12; ids unique and of the
/// form `<class letter><two digits>`, numbered 01..=count; non-empty text; class `a` exactly one
/// target with neither a `DEC:` nor an `R-` prefix (an open item); `b` and `c` at least one
/// target; `d` none; `completed` only on class a/b; no text or target containing the forbidden
/// literal (case-insensitive; spelled in code as two concatenated halves).
pub fn check_shape(set: &QuerySet) -> anyhow::Result<()> {
    let _ = set;
    anyhow::bail!("queries::check_shape: not implemented yet (MOD-162 T5)")
}

/// Read file and sidecar, `check_frozen`, `parse`, `check_shape`.
pub fn load_frozen(path: &Path) -> anyhow::Result<FrozenQueries> {
    let _ = path;
    anyhow::bail!("queries::load_frozen: not implemented yet (MOD-162 T5)")
}

/// Every target, in file order, that is not in `corpus_ids` (empty means valid).
pub fn validate(queries: &[Query], corpus_ids: &BTreeSet<CorpusId>) -> Vec<UnknownTarget> {
    let _ = (queries, corpus_ids);
    Vec::new()
}
