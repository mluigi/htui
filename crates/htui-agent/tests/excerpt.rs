//! The filesystem half of §4.5 and the provider deadline (ANA-5 §12 criteria 12, 13, 14).
//!
//! Two harnesses, on purpose. `FsRepoReader` is tested against a real throwaway tree, because the
//! six skip rules are claims about `std::fs` and a fake cannot fail them. Everything above the
//! reader — the tiers, the denylist, the merge, the audit — is tested against `FakeRepoReader`,
//! because hazard H-22 is precisely a rule that passes on the filesystem and is absent from every
//! double.
#![cfg(feature = "test-support")]

use std::collections::BTreeMap;
use std::sync::Arc;

use htui_agent::excerpt::{FsRepoReader, SkipRule, run_providers};
use htui_core::prompt::TokenEstimator;
use htui_core::prompt::excerpt::{
    BUILTIN_ID, BuiltinRanker, ExcerptCandidate, ExcerptCaps, ExcerptProvider, ExcerptReason,
    ExcerptRequest, OwnedExcerptRequest, PathPrefix, ProviderError, RepoPath, RepoReader, RepoRoot,
    RootSource, select,
};

// ---------------------------------------------------------------------------------------------
// Doubles
// ---------------------------------------------------------------------------------------------

/// An in-memory tree, so a `select` case names no path that exists on any box.
#[derive(Debug, Default)]
struct FakeRepoReader {
    files: BTreeMap<(String, String), String>,
}

impl FakeRepoReader {
    fn with(files: &[(&str, &str, &str)]) -> Self {
        Self {
            files: files
                .iter()
                .map(|(repo, path, body)| {
                    (((*repo).to_owned(), (*path).to_owned()), (*body).to_owned())
                })
                .collect(),
        }
    }
}

impl RepoReader for FakeRepoReader {
    fn list(&self, root: &RepoRoot, _cap: u32) -> Result<(Vec<String>, bool), ProviderError> {
        Ok((
            self.files
                .keys()
                .filter(|(repo, _)| *repo == root.repo)
                .map(|(_, path)| path.clone())
                .collect(),
            false,
        ))
    }

    fn read(&self, root: &RepoRoot, path: &str) -> Result<String, ProviderError> {
        self.files
            .get(&(root.repo.clone(), path.to_owned()))
            .cloned()
            .ok_or_else(|| ProviderError::new("fake", format!("no such path `{path}`")))
    }
}

/// The four behaviours ANA-5 §12 criterion 13 names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    Candidates,
    Error,
    Panic,
    PastDeadline,
}

/// A provider that does exactly one of the four, so each is a case rather than a mock expectation.
#[derive(Debug)]
struct FakeExcerptProvider {
    name: String,
    behaviour: Behaviour,
}

impl FakeExcerptProvider {
    fn new(name: &str, behaviour: Behaviour) -> Self {
        Self {
            name: name.to_owned(),
            behaviour,
        }
    }
}

impl ExcerptProvider for FakeExcerptProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        "0.1"
    }

    fn propose(&self, req: &ExcerptRequest<'_>) -> Result<Vec<ExcerptCandidate>, ProviderError> {
        match self.behaviour {
            Behaviour::Candidates => Ok(vec![ExcerptCandidate {
                repo: "htui".to_owned(),
                path: "docs/proposed.rs".to_owned(),
                lines: None,
                weight: 95,
                // A provider that spells a tier name still renders as itself: `run_providers`
                // rewrites every reason to the provider that returned it.
                reason: "touched_path".to_owned(),
            }]),
            Behaviour::Error => Err(ProviderError::new(&self.name, "the index is cold")),
            Behaviour::Panic => panic!("a provider that panics must not take the prompt with it"),
            Behaviour::PastDeadline => {
                std::thread::sleep(req.deadline * 8);
                Ok(vec![ExcerptCandidate {
                    repo: "htui".to_owned(),
                    path: "docs/too_late.rs".to_owned(),
                    lines: None,
                    weight: 100,
                    reason: "late".to_owned(),
                }])
            }
        }
    }
}

fn base_request(touched: &[&str], roots: Vec<RepoRoot>) -> OwnedExcerptRequest {
    OwnedExcerptRequest {
        item_key: "htui:MOD-2".to_owned(),
        item_body: "Rework the prompt assembler.\n".to_owned(),
        phase: "implement".to_owned(),
        document_bodies: Vec::new(),
        touched_prefixes: touched
            .iter()
            .map(|glob| PathPrefix::parse(glob, "htui"))
            .collect(),
        changed_paths: Vec::new(),
        roots,
        budget_tokens: 100_000,
        caps: ExcerptCaps {
            max_files: 12,
            file_line_cap: 400,
            head_lines: 200,
            max_file_bytes: 524_288,
        },
        scan_cap: 20_000,
        deadline: core::time::Duration::from_millis(40),
    }
}

fn fake_root() -> RepoRoot {
    RepoRoot {
        repo: "htui".to_owned(),
        root: std::path::PathBuf::from("/does/not/exist"),
        source: RootSource::RunStepTree,
    }
}

// ---------------------------------------------------------------------------------------------
// Criterion 13 — the four provider behaviours
// ---------------------------------------------------------------------------------------------

#[test]
fn every_provider_failure_leaves_a_valid_prompt() {
    // ANA-5 §12 criterion 13, and §4.5 rule 2: absent, erroring, panicking or slow means drop the
    // provider, record it with its status, and continue. The panicking case prints one unwind line
    // to stderr, which is the runtime's and not a failure.
    let owned = base_request(&["src/"], vec![fake_root()]);
    // The two provider paths sit **outside** the touched prefix, so a tier-1 built-in candidate
    // cannot be mistaken for the provider's contribution.
    let reader = FakeRepoReader::with(&[
        ("htui", "src/a.rs", "fn a() {}\n"),
        ("htui", "docs/proposed.rs", "fn proposed() {}\n"),
        ("htui", "docs/too_late.rs", "fn late() {}\n"),
    ]);

    for (behaviour, status) in [
        (Behaviour::Candidates, "serena@0.1"),
        (Behaviour::Error, "serena@0.1:error"),
        (Behaviour::Panic, "serena@0.1:panic"),
        (Behaviour::PastDeadline, "serena@0.1:timeout"),
    ] {
        let providers: Vec<Arc<dyn ExcerptProvider>> = vec![
            Arc::new(BuiltinRanker),
            Arc::new(FakeExcerptProvider::new("serena", behaviour)),
        ];
        let (merged, provider_set) = run_providers(&providers, &owned.as_request());
        assert_eq!(
            provider_set,
            vec![BUILTIN_ID.to_owned(), status.to_owned()],
            "`builtin` is always first and never dropped; {behaviour:?} is recorded as `{status}`"
        );

        let set = select(
            &reader,
            &owned.as_request(),
            merged,
            provider_set.clone(),
            TokenEstimator::DEFAULT,
        );
        // The prompt is valid in every case: the built-in's own output is unaffected.
        assert!(
            set.files.iter().any(|file| file.path == "src/a.rs"),
            "{behaviour:?} cost the built-in ranker a file: {:?}",
            set.files
        );
        assert_eq!(set.audit.provider_set, provider_set);
        assert!(
            set.audit.provider_set[0] == BUILTIN_ID,
            "the built-in leads the record"
        );
        let attributed: Vec<&_> = set
            .files
            .iter()
            .filter(|file| file.reason == ExcerptReason::Provider)
            .collect();
        if behaviour == Behaviour::Candidates {
            assert_eq!(
                attributed.len(),
                1,
                "one candidate, attributed once: {:?} / {:?}",
                set.files,
                set.notes
            );
            let proposed = attributed[0];
            assert_eq!(proposed.path, "docs/proposed.rs");
            assert_eq!(proposed.provider.as_deref(), Some("serena"));
            assert_eq!(
                proposed.weight, 95,
                "a provider may not claim a tier by spelling one"
            );
            assert_eq!(
                proposed.rank, 2,
                "95 sits under tier 1's 100 and over the whole lexical tier"
            );
        } else {
            assert!(
                attributed.is_empty(),
                "{behaviour:?} contributed a candidate anyway: {attributed:?}"
            );
        }
        assert!(
            set.files
                .iter()
                .all(|file| file.provider.as_deref() != Some("serena")
                    || behaviour == Behaviour::Candidates),
            "a candidate that arrived past the deadline is dropped, not raced in"
        );
        // And nothing here is an error: the set is usable whatever the provider did.
        assert!(set.audit.selected > 0);
    }
}

#[test]
fn a_provider_set_of_one_is_the_same_path_as_every_provider_failing() {
    // §4.5 `:1201-1203`: the "no providers configured" path and the "every provider failed" path
    // are the same code path, which is what the built-in being a provider buys.
    let owned = base_request(&["src/"], vec![fake_root()]);
    let reader = FakeRepoReader::with(&[("htui", "src/a.rs", "fn a() {}\n")]);
    let alone: Vec<Arc<dyn ExcerptProvider>> = vec![Arc::new(BuiltinRanker)];
    let (merged, set_alone) = run_providers(&alone, &owned.as_request());
    assert!(merged.is_empty());
    assert_eq!(set_alone, vec![BUILTIN_ID.to_owned()]);
    let only_builtin = select(
        &reader,
        &owned.as_request(),
        Vec::new(),
        set_alone,
        TokenEstimator::DEFAULT,
    );

    let broken: Vec<Arc<dyn ExcerptProvider>> = vec![
        Arc::new(BuiltinRanker),
        Arc::new(FakeExcerptProvider::new("serena", Behaviour::Error)),
    ];
    let (merged, set_broken) = run_providers(&broken, &owned.as_request());
    let all_failed = select(
        &reader,
        &owned.as_request(),
        merged,
        set_broken,
        TokenEstimator::DEFAULT,
    );

    assert_eq!(
        only_builtin.files, all_failed.files,
        "the same files by the same ranker"
    );
    assert_ne!(
        only_builtin.audit.provider_set, all_failed.audit.provider_set,
        "and the record still says a provider was asked and declined"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 14 — the denylist is a selection rule
// ---------------------------------------------------------------------------------------------

#[test]
fn a_denied_file_under_a_touched_prefix_is_never_selected_and_is_noted() {
    // ANA-5 §12 criterion 14: "never selected even when it is the only path under a
    // `touched_paths` prefix, and its absence is recorded rather than silent."
    let mut owned = base_request(&["config/"], vec![fake_root()]);
    owned.changed_paths = vec![RepoPath {
        repo: "htui".to_owned(),
        path: "config/.env".to_owned(),
    }];
    let reader = FakeRepoReader::with(&[
        ("htui", "config/.env", "DATABASE_URL=postgres://u:p@h/db\n"),
        (
            "htui",
            "config/id_rsa",
            "-----BEGIN OPENSSH PRIVATE KEY-----\n",
        ),
        ("htui", "config/server.pem", "-----BEGIN CERTIFICATE-----\n"),
    ]);
    let set = select(
        &reader,
        &owned.as_request(),
        Vec::new(),
        vec![BUILTIN_ID.to_owned()],
        TokenEstimator::DEFAULT,
    );

    assert!(
        set.files.is_empty(),
        "the only paths under the prefix are denied: {:?}",
        set.files
    );
    assert_eq!(set.audit.considered, 0, "a denied path is not a candidate");
    assert_eq!(set.audit.selected, 0);
    assert!(set.audit.files.is_empty());
    // The whole point of the rule: no byte of the secret reaches the prompt or the record.
    let record = serde_json::to_string(&set.audit).expect("plain data");
    assert!(!record.contains("DATABASE_URL"));
    assert!(!record.contains("BEGIN OPENSSH"));

    // And its absence is recorded rather than silent.
    assert!(
        set.notes.iter().any(|note| note.contains("config/.env")
            && note.contains("secret_denylist")
            && note.contains("declared")),
        "the declared-and-denied file is named: {:?}",
        set.notes
    );
    assert!(
        set.notes
            .iter()
            .any(|note| note.contains("3 path(s) skipped by rule `secret_denylist`")),
        "and the aggregate count is there too: {:?}",
        set.notes
    );

    // A provider cannot smuggle one in either (§4.5 rule 1: `htui` ranks, windows and renders).
    let smuggled = vec![ExcerptCandidate {
        repo: "htui".to_owned(),
        path: "config/.env".to_owned(),
        lines: None,
        weight: 100,
        reason: "serena".to_owned(),
    }];
    let set = select(
        &reader,
        &owned.as_request(),
        smuggled,
        vec![BUILTIN_ID.to_owned(), "serena@0.1".to_owned()],
        TokenEstimator::DEFAULT,
    );
    assert!(
        set.files.is_empty(),
        "a provider may propose; it may not win"
    );
    assert!(
        set.notes
            .iter()
            .any(|note| note.contains("candidate") && note.contains("config/.env")),
        "and that refusal is recorded: {:?}",
        set.notes
    );
}

// ---------------------------------------------------------------------------------------------
// `FsRepoReader` — the one place this milestone touches a real filesystem
// ---------------------------------------------------------------------------------------------

/// Writes a file and every directory above it, under a throwaway root.
fn write(root: &std::path::Path, path: &str, body: &[u8]) {
    let full = root.join(path);
    std::fs::create_dir_all(full.parent().expect("a parent")).expect("mkdir");
    std::fs::write(full, body).expect("write");
}

fn fs_root(dir: &std::path::Path) -> RepoRoot {
    RepoRoot {
        repo: "htui".to_owned(),
        root: dir.to_path_buf(),
        source: RootSource::RunStepTree,
    }
}

#[test]
fn fs_reader_lists_in_byte_order_at_every_level() {
    // §4.5 step 2: "depth-first, entries sorted by file name byte order at every level so the
    // traversal is deterministic."
    let dir = tempfile::tempdir().expect("a throwaway root");
    for path in [
        "z/z.rs", "z/a.rs", "a/z.rs", "a/a.rs", "m.rs", "B.rs", "a/b/c.rs",
    ] {
        write(dir.path(), path, b"fn x() {}\n");
    }
    let reader = FsRepoReader::default();
    let (paths, truncated) = reader
        .list(&fs_root(dir.path()), 20_000)
        .expect("the root is readable");
    assert!(!truncated);
    assert_eq!(
        paths,
        vec![
            "B.rs".to_owned(),
            "a/a.rs".to_owned(),
            "a/b/c.rs".to_owned(),
            "a/z.rs".to_owned(),
            "m.rs".to_owned(),
            "z/a.rs".to_owned(),
            "z/z.rs".to_owned(),
        ],
        "byte order, so `B.rs` precedes `a/` and the walk is depth-first inside it"
    );
    assert_eq!(
        reader.read(&fs_root(dir.path()), "m.rs").expect("readable"),
        "fn x() {}\n"
    );
}

#[test]
fn fs_reader_normalises_line_endings() {
    // The reader's contract (blueprint B.7) and hazard H-10: CRLF is normalised before anything
    // counts a byte, or `elided_bytes` and therefore the digest would depend on the checkout.
    let dir = tempfile::tempdir().expect("a throwaway root");
    write(dir.path(), "crlf.rs", b"\xEF\xBB\xBFone\r\ntwo\r\n");
    let text = reader_read(dir.path(), "crlf.rs");
    assert_eq!(text, "one\ntwo\n", "CRLF folded, BOM dropped");
}

fn reader_read(dir: &std::path::Path, path: &str) -> String {
    FsRepoReader::default()
        .read(&fs_root(dir), path)
        .expect("readable")
}

#[test]
fn fs_reader_skips_git_gitignored_binary_large_and_lockfiles_in_order() {
    // §4.5 `:1066-1075`, all six rules, in the order the ANA evaluates them.
    assert_eq!(
        SkipRule::ORDER.map(SkipRule::as_str),
        [
            "git",
            "secret_denylist",
            "gitignored",
            "binary",
            "too_large",
            "lockfile_or_minified",
        ],
        "the order is the ANA's and is a fact, not an implementation detail"
    );

    let dir = tempfile::tempdir().expect("a throwaway root");
    write(dir.path(), "src/keep.rs", b"fn keep() {}\n");
    write(dir.path(), "src/big.rs", &vec![b'x'; 4_096]);
    // 1 — `.git/` is pruned at the directory, so nothing under it is even stat-ed.
    write(dir.path(), ".git/config", b"[core]\n");
    write(dir.path(), ".git/objects/ab/cdef", b"whatever\n");
    // 2 — the secret denylist.
    write(dir.path(), ".env", b"TOKEN=hunter2\n");
    write(dir.path(), "certs/server.pem", b"-----BEGIN-----\n");
    // 3 — the gitignore subset: a literal directory, an anchored path and a `*.ext` suffix.
    write(
        dir.path(),
        ".gitignore",
        b"# a comment\n\ntarget/\n/build\n*.tmp\n!target/keep.rs\n",
    );
    write(dir.path(), "target/debug/thing.rs", b"fn thing() {}\n");
    write(dir.path(), "target/keep.rs", b"fn keep() {}\n");
    write(dir.path(), "build/out.rs", b"fn out() {}\n");
    write(dir.path(), "src/scratch.tmp", b"scratch\n");
    // A nested `.gitignore` applies to its own subtree only.
    write(dir.path(), "web/.gitignore", b"dist/\n");
    write(dir.path(), "web/dist/bundle.js", b"var a=1;\n");
    write(dir.path(), "dist/not-ignored.rs", b"fn d() {}\n");
    // 4 — a NUL in the first 8 KB.
    write(dir.path(), "src/image.dat", b"PNG\x00\x01\x02binary\n");
    // 6 — lockfiles and minified assets.
    write(dir.path(), "Cargo.lock", b"[[package]]\n");
    write(dir.path(), "web/app.min.js", b"var a=1;\n");
    write(dir.path(), "web/app.js.map", b"{}\n");

    // 5 — `max_file_bytes` bites at 1 KB, so `src/big.rs` goes and `src/keep.rs` stays.
    let reader = FsRepoReader::new(1_024);
    let (paths, truncated) = reader
        .list(&fs_root(dir.path()), 20_000)
        .expect("the root is readable");
    assert!(!truncated);
    assert_eq!(
        paths,
        vec![
            ".gitignore".to_owned(),
            "dist/not-ignored.rs".to_owned(),
            "src/keep.rs".to_owned(),
            "web/.gitignore".to_owned(),
        ],
        "everything else fell to one of the six rules"
    );
    // `!` is ignored rather than honoured (blueprint B.7), so a negation does not resurrect a file.
    assert!(!paths.iter().any(|path| path == "target/keep.rs"));
}

#[test]
fn scan_cap_sets_truncated() {
    // §4.5 step 2: "Cap the walk at `app_setting.excerpt_max_scan_files` … and record
    // `scan_truncated` when the cap bites."
    let dir = tempfile::tempdir().expect("a throwaway root");
    for n in 0..20 {
        write(dir.path(), &format!("src/f{n:02}.rs"), b"fn x() {}\n");
    }
    let reader = FsRepoReader::default();
    let (all, truncated) = reader.list(&fs_root(dir.path()), 20_000).expect("readable");
    assert_eq!(all.len(), 20);
    assert!(!truncated, "a cap that does not bite is not a truncation");

    let (capped, truncated) = reader.list(&fs_root(dir.path()), 5).expect("readable");
    assert!(truncated, "the cap bit");
    assert_eq!(capped.len(), 5);
    assert_eq!(
        capped,
        all[..5].to_vec(),
        "a truncated listing is a prefix of the full one, never a different set"
    );

    // And the audit carries it, which is what makes a thin excerpt section explicable.
    let owned = base_request(&["src/"], vec![fs_root(dir.path())]);
    let set = select(
        &reader,
        &owned.as_request(),
        Vec::new(),
        vec![BUILTIN_ID.to_owned()],
        TokenEstimator::DEFAULT,
    );
    assert_eq!(set.audit.roots.len(), 1);
    assert!(!set.audit.roots[0].scan_truncated, "20 000 does not bite");

    let mut capped_request = base_request(&["src/"], vec![fs_root(dir.path())]);
    capped_request.scan_cap = 5;
    let set = select(
        &reader,
        &capped_request.as_request(),
        Vec::new(),
        vec![BUILTIN_ID.to_owned()],
        TokenEstimator::DEFAULT,
    );
    assert!(set.audit.roots[0].scan_truncated);
    assert!(
        set.notes.iter().any(|note| note.contains("scan cap")),
        "and it is a note as well as a flag: {:?}",
        set.notes
    );
}

#[test]
fn an_unreadable_root_is_a_note_and_not_an_error() {
    // §4.5 step 1's fail-open, from the `std::fs` side: MOD-7 writes `repo_box_path` rows and has
    // not landed, so a root that is not there is a normal condition.
    let owned = base_request(&["src/"], vec![fake_root()]);
    let reader = FsRepoReader::default();
    assert!(reader.list(&fake_root(), 20_000).is_err());
    let set = select(
        &reader,
        &owned.as_request(),
        Vec::new(),
        vec![BUILTIN_ID.to_owned()],
        TokenEstimator::DEFAULT,
    );
    assert!(set.files.is_empty());
    assert_eq!(set.audit.roots.len(), 1, "the repo is still recorded");
    assert!(
        set.notes
            .iter()
            .any(|note| note.contains("could not be listed")),
        "{:?}",
        set.notes
    );
}
