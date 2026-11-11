//! The query set (MOD-162 T5, P6): `queries/ana37-r2.json`, frozen by its `.sha256` sidecar,
//! parsed and shape-checked before any backend runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use sha2::{Digest as _, Sha256};

use crate::types::{CorpusId, QueryClass};

/// The frozen set, compiled-in path (for `cargo run`).
pub const DEFAULT_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/queries/ana37-r2.json");

/// The word `crates/htui-agent/tests/extensibility.rs` refuses anywhere under `crates/` (blueprint
/// F6), spelled in halves so this file does not contain it.
const FORBIDDEN_LITERAL: &str = concat!("z", "eta");

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
    use std::fmt::Write as _;

    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// `<file>.sha256`.
pub fn sidecar_path(file: &Path) -> PathBuf {
    let mut path = file.as_os_str().to_owned();
    path.push(".sha256");
    PathBuf::from(path)
}

/// `"{sha}  {file_name}\n"`.
pub fn sidecar_line(sha256: &str, file_name: &str) -> String {
    format!("{sha256}  {file_name}\n")
}

/// First whitespace-separated token; must be 64 lowercase hex chars.
pub fn parse_sidecar(text: &str) -> anyhow::Result<String> {
    let Some(token) = text.split_whitespace().next() else {
        anyhow::bail!("the sidecar is empty; expected `<sha256>  <file name>`");
    };
    let is_lower_hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
    if token.len() != 64 || !token.chars().all(is_lower_hex) {
        anyhow::bail!("the sidecar's hash {token:?} is not 64 lowercase hex characters");
    }
    Ok(token.to_owned())
}

/// Pure freeze check: `sha256_hex(bytes) == parse_sidecar(sidecar)`, else an error that names both
/// hashes and says the query file changed after its freeze. Returns the hash.
pub fn check_frozen(bytes: &[u8], sidecar: &str) -> anyhow::Result<String> {
    let frozen = parse_sidecar(sidecar)?;
    let actual = sha256_hex(bytes);
    if actual != frozen {
        anyhow::bail!(
            "the query file changed after its freeze: it hashes to {actual}, its sidecar says \
             {frozen}"
        );
    }
    Ok(actual)
}

/// The query file's JSON.
pub fn parse(bytes: &[u8]) -> anyhow::Result<QuerySet> {
    serde_json::from_slice(bytes).context("the query file is not a query set")
}

/// Every problem in one error, one per line: counts per class 24/20/16/12; ids unique and of the
/// form `<class letter><two digits>`, numbered 01..=count; non-empty text; class `a` exactly one
/// target with neither a `DEC:` nor an `R-` prefix (an open item); `b` and `c` at least one
/// target; `d` none; `completed` only on class a/b; no text or target containing the forbidden
/// literal (case-insensitive; spelled in code as two concatenated halves).
pub fn check_shape(set: &QuerySet) -> anyhow::Result<()> {
    let mut problems = Vec::new();

    for class in QueryClass::ALL {
        let ids: BTreeSet<&str> = set
            .queries
            .iter()
            .filter(|q| q.class == class)
            .map(|q| q.id.as_str())
            .collect();
        let count = set.queries.iter().filter(|q| q.class == class).count();
        let expected = class.expected_count();
        if count != expected {
            problems.push(format!(
                "class {}: {count} queries, expected {expected}",
                class.letter()
            ));
        }
        let missing: Vec<String> = (1..=expected)
            .map(|n| format!("{}{n:02}", class.letter()))
            .filter(|id| !ids.contains(id.as_str()))
            .collect();
        if !missing.is_empty() {
            problems.push(format!(
                "class {}: no query numbered {}",
                class.letter(),
                missing.join(", ")
            ));
        }
    }

    let mut seen = BTreeSet::new();
    for query in &set.queries {
        let id = query.id.as_str();
        if !seen.insert(id) {
            problems.push(format!("{id}: duplicate id"));
        }
        if let Some(problem) = id_problem(query) {
            problems.push(format!("{id}: id {problem}"));
        }
        if query.text.trim().is_empty() {
            problems.push(format!("{id}: empty text"));
        }
        problems.extend(
            target_problems(query)
                .into_iter()
                .map(|p| format!("{id}: {p}")),
        );
        if query.completed && !matches!(query.class, QueryClass::A | QueryClass::B) {
            problems.push(format!(
                "{id}: completed is only allowed on class a or b, not class {}",
                query.class.letter()
            ));
        }
        if contains_forbidden(&query.text) {
            problems.push(format!(
                "{id}: text contains the word the extensibility test refuses"
            ));
        }
        for target in &query.targets {
            if contains_forbidden(target.as_str()) {
                problems.push(format!(
                    "{id}: target {target} contains the word the extensibility test refuses"
                ));
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "the query set {:?} has {} shape problem(s):\n{}",
            set.set,
            problems.len(),
            problems.join("\n")
        )
    }
}

/// Why `query.id` is not `<its class letter><two digits>` numbered from 01, if it is not.
fn id_problem(query: &Query) -> Option<String> {
    let letter = query.class.letter();
    let Some(digits) = query.id.strip_prefix(letter) else {
        return Some(format!("does not start with its class letter {letter:?}"));
    };
    if digits.len() != 2 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("is not {letter:?} and two digits"));
    }
    let number: usize = digits.parse().ok()?;
    let count = query.class.expected_count();
    if number == 0 || number > count {
        return Some(format!("is outside {letter}01..={letter}{count:02}"));
    }
    None
}

/// The class rules on `query.targets`.
fn target_problems(query: &Query) -> Vec<String> {
    let targets = &query.targets;
    match query.class {
        QueryClass::A => {
            let mut problems = Vec::new();
            if targets.len() != 1 {
                problems.push(format!(
                    "class a needs exactly one target, found {}",
                    targets.len()
                ));
            }
            for target in targets {
                if target.is_decision() || target.as_str().starts_with("R-") {
                    problems.push(format!(
                        "class a target {target} is not an open item (no `DEC:` or `R-` prefix)"
                    ));
                }
            }
            problems
        }
        QueryClass::B | QueryClass::C if targets.is_empty() => {
            vec![format!(
                "class {} needs at least one target",
                query.class.letter()
            )]
        }
        QueryClass::B | QueryClass::C => Vec::new(),
        QueryClass::D if targets.is_empty() => Vec::new(),
        QueryClass::D => {
            let named: Vec<&str> = targets.iter().map(CorpusId::as_str).collect();
            vec![format!(
                "class d takes no targets, found {}",
                named.join(", ")
            )]
        }
    }
}

/// Whether `text` contains [`FORBIDDEN_LITERAL`], in any case.
fn contains_forbidden(text: &str) -> bool {
    text.to_lowercase().contains(FORBIDDEN_LITERAL)
}

/// Read file and sidecar, `check_frozen`, `parse`, `check_shape`.
pub fn load_frozen(path: &Path) -> anyhow::Result<FrozenQueries> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading query file {}", path.display()))?;
    let sidecar_file = sidecar_path(path);
    let sidecar = std::fs::read_to_string(&sidecar_file)
        .with_context(|| format!("reading its sidecar {}", sidecar_file.display()))?;
    let sha256 = check_frozen(&bytes, &sidecar)
        .with_context(|| format!("checking {} against its sidecar", path.display()))?;
    let set = parse(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    check_shape(&set).with_context(|| format!("checking the shape of {}", path.display()))?;
    Ok(FrozenQueries {
        path: path.to_path_buf(),
        sha256,
        set,
    })
}

/// Every target, in file order, that is not in `corpus_ids` (empty means valid).
pub fn validate(queries: &[Query], corpus_ids: &BTreeSet<CorpusId>) -> Vec<UnknownTarget> {
    queries
        .iter()
        .flat_map(|query| {
            query
                .targets
                .iter()
                .filter(|target| !corpus_ids.contains(*target))
                .map(|target| UnknownTarget {
                    query: query.id.clone(),
                    target: target.clone(),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMITTED: &[u8] = include_bytes!("../queries/ana37-r2.json");
    const COMMITTED_SIDECAR: &str = include_str!("../queries/ana37-r2.json.sha256");

    /// ANA-37 §5.3's texts, quoted in full there, kept byte for byte (plan P6).
    const VERBATIM: [(&str, &str); 18] = [
        (
            "a03",
            "save the process return status of every agent invocation",
        ),
        (
            "a08",
            "docs checker should fail when a write-up does not mention requirements",
        ),
        ("a09", "never two repositories flagged as the main one"),
        (
            "a11",
            "choose among assistants with odds proportional to strength",
        ),
        (
            "a15",
            "bring markdown backlog and decision log into the database",
        ),
        ("a18", "pause work in progress and pick it up later"),
        ("b01", "where do we decide which machine runs a step"),
        ("b03", "keep passwords out of the logs"),
        ("b04", "can two people work on the same backlog"),
        ("b05", "decide against an in-memory cache server"),
        (
            "b07",
            "which decision chose the protocol we use to talk to coding assistants",
        ),
        ("b09", "how are crash reports collected"),
        ("b11", "engine for finding similar tickets by meaning"),
        ("b12", "finished work merged back into the main branch"),
        (
            "b14",
            "big compile jobs queued so the computer is not overloaded",
        ),
        ("b15", "what happens to running work when I close the app"),
        (
            "b20",
            "how do agents split one task into pieces that run at the same time",
        ),
        ("c16", "R-STO-8"),
    ];

    /// ANA-37 §5.3's known targets (blueprint §3.4).
    const KNOWN_TARGETS: [(&str, &[&str]); 16] = [
        ("a01", &["MOD-99"]),
        ("a03", &["MOD-101"]),
        ("a04", &["MOD-97"]),
        ("a05", &["MOD-94"]),
        ("a08", &["TOOL-13"]),
        ("a09", &["MOD-106"]),
        ("a10", &["MOD-98"]),
        ("a11", &["MOD-36"]),
        ("a15", &["MOD-8"]),
        ("a18", &["MOD-125"]),
        ("a20", &["MOD-108"]),
        ("b01", &["ANA-42", "R-ORCH-15", "MOD-130"]),
        ("b07", &["DEC:ANA-4"]),
        ("b15", &["R-TUI-18", "DEC:MOD-41"]),
        ("b20", &["R-ORCH-7", "ANA-33"]),
        ("c16", &["R-STO-8"]),
    ];

    fn query<'a>(set: &'a QuerySet, id: &str) -> &'a Query {
        set.queries
            .iter()
            .find(|q| q.id == id)
            .unwrap_or_else(|| panic!("{id} is in the set"))
    }

    /// `first` occurs in `text`, and `second` after it.
    fn in_order(text: &str, first: &str, second: &str) -> bool {
        text.find(first)
            .is_some_and(|at| text[at + first.len()..].contains(second))
    }

    fn q(id: &str, class: QueryClass, targets: &[&str]) -> Query {
        Query {
            id: id.to_owned(),
            class,
            text: format!("text of {id}"),
            targets: targets.iter().map(|t| CorpusId::new(*t)).collect(),
            completed: false,
        }
    }

    /// A set that passes `check_shape`: 24/20/16/12, open-item targets on `a`.
    fn valid_set() -> QuerySet {
        let mut queries = Vec::new();
        for class in QueryClass::ALL {
            for n in 1..=class.expected_count() {
                let id = format!("{}{n:02}", class.letter());
                let target = match class {
                    QueryClass::A => vec![format!("MOD-{n}")],
                    QueryClass::B => vec![format!("R-STO-{n}"), format!("DEC:ANA-{n}")],
                    QueryClass::C => vec![format!("TOOL-{n}")],
                    QueryClass::D => Vec::new(),
                };
                let target: Vec<&str> = target.iter().map(String::as_str).collect();
                queries.push(q(&id, class, &target));
            }
        }
        QuerySet {
            set: "ana37-r2".to_owned(),
            corpus_ref: "d15e9fea".to_owned(),
            queries,
        }
    }

    fn shape_error(set: &QuerySet) -> String {
        check_shape(set)
            .expect_err("the shape check refuses")
            .to_string()
    }

    fn position(set: &QuerySet, id: &str) -> usize {
        set.queries
            .iter()
            .position(|q| q.id == id)
            .unwrap_or_else(|| panic!("{id} is in the set"))
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("htui-bench-queries-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn the_committed_set_hashes_to_its_sidecar() {
        let sha = check_frozen(COMMITTED, COMMITTED_SIDECAR).expect("the committed set is frozen");
        assert_eq!(sha, sha256_hex(COMMITTED));
        assert_eq!(
            COMMITTED_SIDECAR,
            sidecar_line(&sha, "ana37-r2.json"),
            "the sidecar is `sha256sum` output"
        );

        let frozen = load_frozen(Path::new(DEFAULT_FILE)).expect("the default file loads");
        assert_eq!(frozen.sha256, sha);
        assert_eq!(frozen.path, Path::new(DEFAULT_FILE));
        assert_eq!(frozen.set, parse(COMMITTED).expect("parses"));
    }

    #[test]
    fn the_committed_set_has_ana37s_shape() {
        let set = parse(COMMITTED).expect("the committed set parses");
        check_shape(&set).expect("the committed set has its shape");
        assert_eq!(set.set, "ana37-r2");
        assert_eq!(set.corpus_ref, "d15e9fea");

        for class in QueryClass::ALL {
            let count = set.queries.iter().filter(|q| q.class == class).count();
            assert_eq!(count, class.expected_count(), "class {}", class.letter());
        }
        assert_eq!(set.queries.len(), 72);

        for (id, text) in VERBATIM {
            assert_eq!(query(&set, id).text, text, "{id} is ANA-37's text verbatim");
        }
        for (id, targets) in KNOWN_TARGETS {
            let got: BTreeSet<&str> = query(&set, id)
                .targets
                .iter()
                .map(CorpusId::as_str)
                .collect();
            let want: BTreeSet<&str> = targets.iter().copied().collect();
            assert_eq!(got, want, "{id}'s targets");
        }

        let a01 = &query(&set, "a01").text;
        assert!(
            a01.starts_with("crash telemetry address is hard-wired"),
            "{a01}"
        );
        let a04 = &query(&set, "a04").text;
        assert!(
            in_order(a04, "flat-rate plan", "provider credentials from my shell"),
            "{a04}"
        );
        let a20 = &query(&set, "a20").text;
        assert!(
            in_order(
                a20,
                "check out a repository onto a machine",
                "keep the git login"
            ),
            "{a20}"
        );
        for (id, needle) in [("c09", "ConPTY"), ("c11", "Plackett-Luce")] {
            assert!(
                query(&set, id).text.contains(needle),
                "{id} looks up {needle}"
            );
        }

        let completed: Vec<&str> = set
            .queries
            .iter()
            .filter(|q| q.completed)
            .map(|q| q.id.as_str())
            .collect();
        assert_eq!(completed, ["a01", "a04", "a20"]);
    }

    #[test]
    fn sha256_hex_matches_the_fips_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256_hex(b"").len(), 64);
    }

    #[test]
    fn sidecar_path_and_line_follow_sha256sum() {
        assert_eq!(
            sidecar_path(Path::new("/x/queries/ana37-r2.json")),
            PathBuf::from("/x/queries/ana37-r2.json.sha256")
        );
        let sha = sha256_hex(b"abc");
        let line = sidecar_line(&sha, "ana37-r2.json");
        assert_eq!(line, format!("{sha}  ana37-r2.json\n"));
        assert_eq!(parse_sidecar(&line).expect("parses"), sha);
    }

    #[test]
    fn check_frozen_refuses_one_flipped_byte() {
        let mut bytes = COMMITTED.to_vec();
        let last = bytes.len() - 3;
        bytes[last] ^= 0x01;
        let err = check_frozen(&bytes, COMMITTED_SIDECAR)
            .expect_err("a flipped byte breaks the freeze")
            .to_string();
        let expected = parse_sidecar(COMMITTED_SIDECAR).expect("the sidecar parses");
        assert!(err.contains(&expected), "names the sidecar's hash: {err}");
        assert!(
            err.contains(&sha256_hex(&bytes)),
            "names the file's hash: {err}"
        );
        assert!(err.contains("changed after its freeze"), "{err}");
    }

    #[test]
    fn parse_sidecar_refuses_a_short_or_upper_case_hash() {
        let sha = sha256_hex(b"abc");
        assert!(parse_sidecar(&format!("{}  f.json\n", &sha[..63])).is_err());
        assert!(parse_sidecar(&format!("{}  f.json\n", sha.to_uppercase())).is_err());
        assert!(parse_sidecar(&format!("{sha}0  f.json\n")).is_err());
        assert!(parse_sidecar(&format!("{}g  f.json\n", &sha[..63])).is_err());
        assert!(parse_sidecar("").is_err());
        assert!(parse_sidecar("   \n").is_err());
        assert_eq!(parse_sidecar(&format!("{sha}\n")).expect("bare hash"), sha);
    }

    #[test]
    fn parse_refuses_unknown_fields_and_bad_classes() {
        let ok = br#"{"set": "s", "corpus_ref": "r", "queries": [{"id": "d01", "class": "d", "text": "t", "targets": []}]}"#;
        let set = parse(ok).expect("parses");
        assert!(!set.queries[0].completed, "completed defaults to false");

        let extra = br#"{"set": "s", "corpus_ref": "r", "queries": [{"id": "d01", "class": "d", "text": "t", "targets": [], "note": "x"}]}"#;
        assert!(parse(extra).is_err());
        let class = br#"{"set": "s", "corpus_ref": "r", "queries": [{"id": "e01", "class": "e", "text": "t", "targets": []}]}"#;
        assert!(parse(class).is_err());
        assert!(parse(b"not json").is_err());
    }

    #[test]
    fn check_shape_accepts_a_valid_set() {
        check_shape(&valid_set()).expect("valid");
    }

    #[test]
    fn check_shape_names_a_wrong_count() {
        let mut set = valid_set();
        let at = position(&set, "a24");
        set.queries.remove(at);
        let err = shape_error(&set);
        assert!(err.contains("class a: 23 queries, expected 24"), "{err}");
    }

    #[test]
    fn check_shape_names_a_duplicate_id() {
        let mut set = valid_set();
        let at = position(&set, "b02");
        set.queries[at].id = "b01".to_owned();
        let err = shape_error(&set);
        assert!(err.contains("b01: duplicate id"), "{err}");
        assert!(err.contains("b02"), "the gap is named too: {err}");
    }

    #[test]
    fn check_shape_names_a_malformed_or_misnumbered_id() {
        let mut set = valid_set();
        let at = position(&set, "c03");
        set.queries[at].id = "c3".to_owned();
        let err = shape_error(&set);
        assert!(err.contains("c3: id"), "{err}");

        let mut set = valid_set();
        let at = position(&set, "c03");
        set.queries[at].id = "d03".to_owned();
        let err = shape_error(&set);
        assert!(err.contains("d03: id"), "a letter not its class: {err}");

        let mut set = valid_set();
        let at = position(&set, "c16");
        set.queries[at].id = "c17".to_owned();
        let err = shape_error(&set);
        assert!(err.contains("c17"), "{err}");
        assert!(err.contains("c16"), "{err}");
    }

    #[test]
    fn check_shape_names_a_class_d_target() {
        let mut set = valid_set();
        let at = position(&set, "d05");
        set.queries[at].targets = vec![CorpusId::new("MOD-1")];
        let err = shape_error(&set);
        assert!(err.contains("d05"), "{err}");
        assert!(err.contains("MOD-1"), "{err}");
    }

    #[test]
    fn check_shape_names_an_a_target_that_is_not_an_open_item() {
        let mut set = valid_set();
        let at = position(&set, "a03");
        set.queries[at].targets = vec![CorpusId::decision("ANA-4")];
        let err = shape_error(&set);
        assert!(err.contains("a03"), "{err}");
        assert!(err.contains("DEC:ANA-4"), "{err}");

        let mut set = valid_set();
        let at = position(&set, "a04");
        set.queries[at].targets = vec![CorpusId::new("R-STO-8")];
        let err = shape_error(&set);
        assert!(err.contains("a04") && err.contains("R-STO-8"), "{err}");

        let mut set = valid_set();
        let at = position(&set, "a05");
        set.queries[at].targets.push(CorpusId::new("MOD-99"));
        let err = shape_error(&set);
        assert!(err.contains("a05"), "two targets on class a: {err}");
    }

    #[test]
    fn check_shape_names_missing_targets_empty_text_and_misplaced_completed() {
        let mut set = valid_set();
        let at = position(&set, "b07");
        set.queries[at].targets.clear();
        let err = shape_error(&set);
        assert!(err.contains("b07"), "{err}");

        let mut set = valid_set();
        let at = position(&set, "c02");
        set.queries[at].targets.clear();
        let err = shape_error(&set);
        assert!(err.contains("c02"), "{err}");

        let mut set = valid_set();
        let at = position(&set, "b03");
        set.queries[at].text = "  ".to_owned();
        let err = shape_error(&set);
        assert!(err.contains("b03") && err.contains("text"), "{err}");

        let mut set = valid_set();
        let at = position(&set, "c04");
        set.queries[at].completed = true;
        let err = shape_error(&set);
        assert!(err.contains("c04") && err.contains("completed"), "{err}");

        let mut set = valid_set();
        let at = position(&set, "b04");
        set.queries[at].completed = true;
        check_shape(&set).expect("completed is allowed on class b");
    }

    #[test]
    fn check_shape_refuses_the_forbidden_literal() {
        let word = FORBIDDEN_LITERAL.to_uppercase();

        let mut set = valid_set();
        let at = position(&set, "d02");
        set.queries[at].text = format!("route work to {word} first");
        let err = shape_error(&set);
        assert!(err.contains("d02"), "{err}");

        let mut set = valid_set();
        let at = position(&set, "c06");
        set.queries[at].targets = vec![CorpusId::new(format!("MOD-{word}"))];
        let err = shape_error(&set);
        assert!(err.contains("c06"), "{err}");
    }

    #[test]
    fn check_shape_lists_every_problem_one_per_line() {
        let mut set = valid_set();
        let at = position(&set, "d01");
        set.queries[at].targets = vec![CorpusId::new("MOD-1")];
        let at = position(&set, "a02");
        set.queries[at].targets = vec![CorpusId::decision("MOD-2")];
        let at = position(&set, "b05");
        set.queries[at].id = "b04".to_owned();
        let at = position(&set, "c16");
        set.queries.remove(at);

        let err = shape_error(&set);
        for needle in [
            "d01",
            "a02",
            "b04: duplicate id",
            "class c: 15 queries, expected 16",
        ] {
            let lines: Vec<&str> = err.lines().filter(|l| l.contains(needle)).collect();
            assert_eq!(lines.len(), 1, "{needle} on exactly one line of:\n{err}");
        }
        assert!(err.lines().count() >= 4, "{err}");
    }

    #[test]
    fn validate_names_every_unknown_target_in_file_order() {
        let queries = vec![
            q("a01", QueryClass::A, &["MOD-99"]),
            q("b01", QueryClass::B, &["MOD-999", "R-STO-8", "DEC:ANA-404"]),
            q("c01", QueryClass::C, &["TOOL-0"]),
            q("d01", QueryClass::D, &[]),
        ];
        let corpus: BTreeSet<CorpusId> = ["MOD-99", "R-STO-8", "DEC:ANA-4"]
            .into_iter()
            .map(CorpusId::new)
            .collect();

        let unknown = validate(&queries, &corpus);
        let shown: Vec<String> = unknown.iter().map(ToString::to_string).collect();
        assert_eq!(
            shown,
            [
                "b01: MOD-999 is not a corpus ID",
                "b01: DEC:ANA-404 is not a corpus ID",
                "c01: TOOL-0 is not a corpus ID",
            ]
        );
        assert_eq!(
            unknown[0],
            UnknownTarget {
                query: "b01".to_owned(),
                target: CorpusId::new("MOD-999"),
            }
        );

        let complete: BTreeSet<CorpusId> = queries
            .iter()
            .flat_map(|q| q.targets.iter().cloned())
            .collect();
        assert!(validate(&queries, &complete).is_empty());
    }

    #[test]
    fn load_frozen_reads_the_file_and_its_sidecar() {
        let dir = scratch_dir("load");
        let file = dir.join("ana37-r2.json");
        std::fs::write(&file, COMMITTED).expect("write set");
        std::fs::write(sidecar_path(&file), COMMITTED_SIDECAR).expect("write sidecar");

        let frozen = load_frozen(&file).expect("loads");
        assert_eq!(frozen.path, file);
        assert_eq!(frozen.sha256, sha256_hex(COMMITTED));
        assert_eq!(frozen.set.queries.len(), 72);

        // One edit after the freeze: refused.
        let mut edited = COMMITTED.to_vec();
        edited.push(b'\n');
        std::fs::write(&file, &edited).expect("rewrite set");
        let err = format!("{:#}", load_frozen(&file).expect_err("refused"));
        assert!(err.contains("changed after its freeze"), "{err}");

        // A missing sidecar: refused, and the path is named.
        std::fs::write(&file, COMMITTED).expect("restore set");
        std::fs::remove_file(sidecar_path(&file)).expect("remove sidecar");
        let err = format!("{:#}", load_frozen(&file).expect_err("refused"));
        assert!(err.contains("ana37-r2.json.sha256"), "{err}");

        // A frozen file of the wrong shape: refused after the hash matches.
        let mut set = parse(COMMITTED).expect("parses");
        set.queries.pop();
        let bytes = serde_json::to_vec_pretty(&set).expect("serialises");
        std::fs::write(&file, &bytes).expect("write short set");
        std::fs::write(
            sidecar_path(&file),
            sidecar_line(&sha256_hex(&bytes), "ana37-r2.json"),
        )
        .expect("write sidecar");
        let err = format!("{:#}", load_frozen(&file).expect_err("refused"));
        assert!(err.contains("class d: 11 queries, expected 12"), "{err}");

        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
