//! The excerpt value types the prompt carries and the record audits (ANA-5 §4.5 `:990-1211`).
//!
//! T64 lands the **data**: what a selected excerpt is, and what `trim_record.excerpts` says about
//! the pass that selected it. T66 lands the machinery in this same file — `PathPrefix`,
//! `ExcerptRequest`, the `ExcerptProvider` and `RepoReader` seams, `skip_by_path`, `BuiltinRanker`
//! and `select` — plus its filesystem half in `htui_agent::excerpt`. The split is the one ANA-5
//! §4.8 draws: `htui-core` contains no `std::fs` at all, so everything here is a value a caller
//! hands the assembler, already read and already windowed.
//!
//! Two properties are enforced by type rather than by review. [`Excerpt`] carries a repo **slug**
//! and a repo-**relative** path, never a root, so §4.2 rule 5's "no absolute path in any rendered
//! byte" cannot be violated by a renderer. And [`Excerpt::content`] is the windowed text without
//! line numbers, so the `N | ` prefix is the renderer's and a re-window never has to strip one.

use serde::Serialize;

/// The glob metacharacters ANA-2 §4.7 truncates a `touched_paths` entry at (`docs/ANA-2.md:1074`).
const GLOB_META: [char; 4] = ['*', '?', '[', '{'];

/// §4.5's secret denylist (`:1071`), split into the four match shapes it needs: whole file names.
///
/// A **selection** rule and not a scrubbing rule: `R-SEC-3` fails closed on residue at persist
/// time, which is after the bytes reached the agent. Excluding the class at selection removes the
/// class (§4.5 `:1077-1085`).
const SECRET_EXACT: [&str; 7] = [
    ".env",
    ".netrc",
    ".npmrc",
    ".pypirc",
    "credentials",
    "credentials.json",
    "id_rsa",
];
/// Secret file names by prefix: `.env.*`, `id_rsa*`, `id_ed25519*`.
const SECRET_PREFIX: [&str; 3] = [".env.", "id_rsa", "id_ed25519"];
/// Secret file names by extension.
const SECRET_EXT: [&str; 5] = [".pem", ".key", ".p12", ".pfx", ".kdbx"];
/// The one secret extension that is not a suffix of the name but of the whole `*.keystore` form.
const SECRET_KEYSTORE: &str = ".keystore";
/// §4.5's last rule (`:1075`): high token cost, near-zero signal.
const NOISE_SUFFIX: [&str; 4] = [".lock", ".min.js", ".min.css", ".map"];

/// One repo-qualified, wildcard-free path prefix, derived from a `touched_paths` glob.
///
/// ANA-2 §4.7's truncation (`docs/ANA-2.md:1074-1077`): the glob is cut at its first `*?[{` and
/// then at the last `/`, so `src/**/*.rs` becomes `src/` and a full path stays whole. `**` becomes
/// the **empty** prefix, which is a prefix of everything — deliberately, because that is what makes
/// a `**` declaration equivalent to no declaration.
///
/// Repo-qualified per `docs/ANA-2.md:1025`: `repo:glob`, a bare glob meaning the primary repo. Two
/// repos each holding a `src/` therefore do not collide, which is the defect the qualifier exists
/// to fix.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathPrefix {
    /// The repo slug the prefix is relative to.
    pub repo: String,
    /// The wildcard-free prefix, `/`-separated and possibly empty.
    pub prefix: String,
}

impl PathPrefix {
    /// Parse one `touched_paths` entry against the project's primary repo slug.
    ///
    /// The repo qualifier is the text before the **first** `:` when that text is non-empty and
    /// holds no `/` — a `touched_paths` value is a repo-relative glob (`docs/ANA-9.md:591`), so a
    /// `:` after a path separator is part of a file name rather than a qualifier.
    #[must_use]
    pub fn parse(touched: &str, primary_repo: &str) -> Self {
        let (repo, glob) = match touched.split_once(':') {
            Some((repo, rest)) if !repo.is_empty() && !repo.contains('/') => (repo, rest),
            _ => (primary_repo, touched),
        };
        let prefix = match glob.find(GLOB_META) {
            // No metacharacter: the whole path is the prefix and stays whole.
            None => glob.to_owned(),
            Some(cut) => {
                let head = &glob[..cut];
                match head.rfind('/') {
                    Some(slash) => head[..=slash].to_owned(),
                    None => String::new(),
                }
            }
        };
        Self {
            repo: repo.to_owned(),
            prefix,
        }
    }

    /// Whether a repo-relative path sits under this prefix, on **raw bytes**.
    ///
    /// Bytes and not characters, and no case folding: a case-insensitive match would make the
    /// selected set depend on the filesystem the walk ran on, which invariant 2 forbids.
    #[must_use]
    pub fn matches(&self, repo: &str, path: &str) -> bool {
        self.repo == repo && path.as_bytes().starts_with(self.prefix.as_bytes())
    }
}

/// One repo-qualified, repo-relative path: a changed file, or a listed candidate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPath {
    /// The repo slug, never a path.
    pub repo: String,
    /// The repo-relative, `/`-separated path.
    pub path: String,
}

/// One repo's readable root, and where it came from (§4.5 step 1).
///
/// [`root`](Self::root) is the one absolute path in this module and it is **never rendered**: §4.2
/// rule 5 puts no absolute path in a digested byte, and [`Excerpt`] carries a slug and a relative
/// path precisely so a renderer cannot reach this field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRoot {
    /// The repo slug.
    pub repo: String,
    /// The absolute root the walk reads under.
    pub root: std::path::PathBuf,
    /// Which rung of §4.5 step 1 answered.
    pub source: RootSource,
}

/// §4.5's path-only skip rules, applied by the ranker over the listing rather than only by the
/// reader (hazard H-22).
///
/// Path-only so a [`RepoReader`] test double exercises them too: a rule that lived in
/// `htui_agent::excerpt::FsRepoReader` alone would pass on the filesystem and be absent from every
/// fake, which is exactly the hole ANA-5 §12 criterion 14 is written to catch.
///
/// Returns the **name** of the first rule that fired, in §4.5's own order (`:1066-1075`): `.git/`,
/// then the secret denylist, then lockfiles and minified assets. The three content rules — the
/// gitignore subset, the NUL test and `max_file_bytes` — need bytes and belong to the reader.
#[must_use]
pub fn skip_by_path(path: &str) -> Option<&'static str> {
    if path == ".git" || path.starts_with(".git/") || path.contains("/.git/") {
        return Some("git");
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    if SECRET_EXACT.contains(&name)
        || SECRET_PREFIX.iter().any(|head| name.starts_with(head))
        || SECRET_EXT.iter().any(|ext| name.ends_with(ext))
        || name.ends_with(SECRET_KEYSTORE)
    {
        return Some("secret_denylist");
    }
    if NOISE_SUFFIX.iter().any(|suffix| name.ends_with(suffix)) {
        return Some("lockfile_or_minified");
    }
    None
}

/// Where the walk's root for a repo came from (§4.5; `trim_record.excerpts.roots[]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RootSource {
    /// The run step's own isolated tree.
    RunStepTree,
    /// `repo_box_path` for this repo on this box.
    RepoBoxPath,
    /// No readable root was resolved, so nothing was scanned for this repo.
    NoPath,
}

/// Why a file was selected, as the `reason="…"` attribute and the audit key (§4.5 `:1113-1114`).
///
/// A closed enum and not prose: a provider that could write the attribute would put a third
/// party's formatting into `prompt_digest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcerptReason {
    /// Tier 1: the item's `touched_paths` prefix matched.
    TouchedPath,
    /// Tier 2: the previous attempt's diff changed it.
    PrevDiff,
    /// Tier 3: the item body or an input document names the path.
    Mentioned,
    /// Tier 4: an identifier in the item body occurs in it.
    Identifier,
    /// Tier 5: lexical overlap with the item body.
    Lexical,
    /// An [`ExcerptProvider`](self) other than the built-in ranker proposed it.
    Provider,
}

impl ExcerptReason {
    /// The `reason="…"` value for an excerpt selected for this reason.
    ///
    /// [`ExcerptReason::Provider`] renders `provider:<name>` (§4.5 `:1114`), which is why the
    /// provider name is an argument: the enum carries the class and [`Excerpt::provider`] carries
    /// the instance. A `Provider` excerpt with no name renders the bare `provider`, which is the
    /// fail-soft direction — a missing name must not make the section unrenderable.
    #[must_use]
    pub fn render(self, provider: Option<&str>) -> String {
        match (self, provider) {
            (Self::TouchedPath, _) => "touched_path".to_owned(),
            (Self::PrevDiff, _) => "prev_diff".to_owned(),
            (Self::Mentioned, _) => "mentioned".to_owned(),
            (Self::Identifier, _) => "identifier".to_owned(),
            (Self::Lexical, _) => "lexical".to_owned(),
            (Self::Provider, Some(name)) => format!("provider:{name}"),
            (Self::Provider, None) => "provider".to_owned(),
        }
    }
}

/// One selected file excerpt, read and windowed, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    /// The repo slug, never a path.
    pub repo: String,
    /// The repo-relative, `/`-separated path.
    pub path: String,
    /// The first line number of the window, 1-based.
    pub first_line: u32,
    /// The last line number of the window, 1-based and inclusive.
    pub last_line: u32,
    /// Whether the window is a cut of a longer file.
    pub truncated: bool,
    /// Lines the cut removed; `0` when [`truncated`](Self::truncated) is false.
    pub elided_lines: u32,
    /// Bytes the cut removed; `0` when [`truncated`](Self::truncated) is false.
    pub elided_bytes: u64,
    /// The ranker's 1-based rank, the order the trimmer drops files in (lowest rank last).
    pub rank: u32,
    /// The ranker's weight, `0..=100`.
    pub weight: u16,
    /// Why it was selected.
    pub reason: ExcerptReason,
    /// The proposing provider's name, when [`reason`](Self::reason) is
    /// [`ExcerptReason::Provider`].
    pub provider: Option<String>,
    /// The windowed lines, LF, **unnumbered**: the `N | ` prefix belongs to the renderer.
    pub content: String,
}

/// The caps the selection pass ran under (§5.3's `excerpt_*` keys), recorded verbatim.
///
/// `Default` is all zeroes and means "no selection pass ran", which is what an empty
/// [`ExcerptSet`] says: a prompt with no excerpt section must still produce a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ExcerptCaps {
    /// `app_setting.excerpt_max_files`.
    pub max_files: u32,
    /// `app_setting.excerpt_file_line_cap`: above this a file is windowed rather than taken whole.
    pub file_line_cap: u32,
    /// `app_setting.excerpt_head_lines`: the head half of the window.
    pub head_lines: u32,
    /// `app_setting.excerpt_max_file_bytes`: above this a file is skipped entirely.
    pub max_file_bytes: u64,
}

/// One repo's root, as the audit records it (§4.5 `:1124`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RootRecord {
    /// The repo slug.
    pub repo: String,
    /// Where the root came from — including [`RootSource::NoPath`], which is a fact and not a
    /// failure: an unresolved root is why a repo contributed nothing.
    pub source: RootSource,
    /// Whether `excerpt_max_scan_files` cut the listing short.
    pub scan_truncated: bool,
}

/// One file that reached the prompt, as the audit records it (§4.5 `:1128-1132`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileRecord {
    /// The repo slug.
    pub repo: String,
    /// The repo-relative path.
    pub path: String,
    /// `<first>-<last>`, the same string the `lines="…"` attribute carries.
    pub lines: String,
    /// The ranker's rank.
    pub rank: u32,
    /// The ranker's weight.
    pub weight: u16,
    /// Why it was selected.
    pub reason: ExcerptReason,
    /// Whether the window is a cut.
    pub truncated: bool,
    /// The rendered content's byte length.
    pub bytes: u64,
    /// `sha256` over the **rendered content bytes**, not the whole file (§4.5 `:1136-1137`): a
    /// reader can prove which bytes the model saw without the record storing them.
    pub sha256: String,
}

/// `trim_record.excerpts`: what the selection pass considered, chose and paid for (§4.5 `:1119`).
///
/// `selected` and `files` are deliberately allowed to disagree. `selected` is what the ranker
/// chose; `files` is what survived the trimmer, so a dropped `excerpts` section leaves `selected:
/// 3` beside an empty `files`. That asymmetry is the point: a reader can see that three files were
/// paid for and none reached the model.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ExcerptAudit {
    /// `name@version` per provider, suffixed `:error`, `:panic` or `:timeout` when one was dropped
    /// (blueprint P-11, for criterion 13's "a non-`ok` status per failing provider").
    pub provider_set: Vec<String>,
    /// One per repo the pass was given.
    pub roots: Vec<RootRecord>,
    /// How many candidate files the pass looked at.
    pub considered: u32,
    /// How many the ranker chose.
    pub selected: u32,
    /// The caps it ran under.
    pub caps: ExcerptCaps,
    /// What survived into the prompt; empty when the section was dropped whole.
    pub files: Vec<FileRecord>,
}

/// The excerpt half of a [`PromptSpec`](crate::prompt::PromptSpec): the files and their audit.
///
/// `Default` is the empty set, which is what a caller that resolved no readable root passes — the
/// preview does exactly that (plan D103) — and what a phase with no `{{excerpts}}` placeholder
/// never looks at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExcerptSet {
    /// The selected excerpts. Rendered in `(repo, path)` byte order (§4.7 rule 5), whatever order
    /// the ranker produced them in.
    pub files: Vec<Excerpt>,
    /// The audit half, filled by the ranker; `files` is completed by the assembler.
    pub audit: ExcerptAudit,
    /// Notes the pass produced that are not errors — a skipped repo, a dropped provider. Copied
    /// into `trim_record.notes`.
    pub notes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reason_vocabulary_is_closed_and_provider_carries_its_name() {
        assert_eq!(ExcerptReason::TouchedPath.render(None), "touched_path");
        assert_eq!(ExcerptReason::PrevDiff.render(None), "prev_diff");
        assert_eq!(ExcerptReason::Mentioned.render(None), "mentioned");
        assert_eq!(ExcerptReason::Identifier.render(None), "identifier");
        assert_eq!(ExcerptReason::Lexical.render(None), "lexical");
        assert_eq!(
            ExcerptReason::Provider.render(Some("serena")),
            "provider:serena"
        );
        assert_eq!(
            ExcerptReason::Provider.render(None),
            "provider",
            "a missing name must not make the section unrenderable"
        );
        // A non-provider reason ignores the name rather than appending it.
        assert_eq!(
            ExcerptReason::Lexical.render(Some("serena")),
            "lexical",
            "only `provider` is qualified"
        );
    }

    #[test]
    fn an_empty_set_still_serialises_an_audit() {
        let set = ExcerptSet::default();
        assert!(set.files.is_empty());
        let audit = serde_json::to_value(&set.audit).expect("plain data");
        assert_eq!(audit["considered"], 0);
        assert_eq!(audit["selected"], 0);
        assert_eq!(audit["caps"]["max_files"], 0);
        assert_eq!(audit["files"], serde_json::json!([]));
        assert_eq!(audit["provider_set"], serde_json::json!([]));
    }

    #[test]
    fn a_touched_glob_truncates_at_its_first_wildcard_then_at_the_last_slash() {
        // ANA-2 §4.7 `:1074-1077`, verbatim: `src/**/*.rs` becomes `src/`, a full path stays
        // whole, and `**` becomes the empty prefix, which is a prefix of everything.
        for (glob, prefix) in [
            ("src/**/*.rs", "src/"),
            (
                "crates/htui-core/src/model/item.rs",
                "crates/htui-core/src/model/item.rs",
            ),
            ("**", ""),
            ("src/htui*.rs", "src/"),
            ("a/b/c?.txt", "a/b/"),
            ("a/b[0-9]/c", "a/"),
            ("a/{x,y}/c", "a/"),
        ] {
            let parsed = PathPrefix::parse(glob, "htui");
            assert_eq!(parsed.repo, "htui", "a bare glob means the primary repo");
            assert_eq!(parsed.prefix, prefix, "`{glob}` truncates to `{prefix}`");
        }
    }

    #[test]
    fn a_touched_glob_may_name_its_repo() {
        let parsed = PathPrefix::parse("agy:src/**", "htui");
        assert_eq!(parsed.repo, "agy");
        assert_eq!(parsed.prefix, "src/");
        assert!(parsed.matches("agy", "src/main.rs"));
        assert!(
            !parsed.matches("htui", "src/main.rs"),
            "the prefix is repo-qualified, so two repos' `src/` do not collide"
        );
        assert!(!parsed.matches("agy", "tests/main.rs"));
        // The empty prefix is a prefix of everything, in its own repo only.
        let all = PathPrefix::parse("**", "htui");
        assert!(all.matches("htui", "anything/at/all"));
        assert!(!all.matches("agy", "anything/at/all"));
    }

    #[test]
    fn the_path_skip_rules_name_the_rule_that_fired() {
        for path in [
            ".git/config",
            "crates/.git/HEAD",
            ".env",
            ".env.local",
            "deploy/prod.pem",
            "certs/server.key",
            "a/b.p12",
            "a/b.pfx",
            "home/id_rsa",
            "home/id_ed25519.pub",
            "a/release.keystore",
            ".netrc",
            "sub/.npmrc",
            ".pypirc",
            "aws/credentials",
            "gcp/credentials.json",
            "vault/secrets.kdbx",
            "Cargo.lock",
            "web/app.min.js",
            "web/app.min.css",
            "web/app.js.map",
        ] {
            assert!(
                skip_by_path(path).is_some(),
                "`{path}` is excluded at selection, not at scrub time (§4.5 `:1077-1085`)"
            );
        }
        assert_eq!(skip_by_path(".git/config"), Some("git"));
        assert_eq!(skip_by_path(".env.production"), Some("secret_denylist"));
        assert_eq!(skip_by_path("Cargo.lock"), Some("lockfile_or_minified"));
        for path in [
            "crates/htui-core/src/prompt/mod.rs",
            "src/environment.rs",
            "docs/keys.md",
            "src/credentials_test.rs",
            "web/app.js",
        ] {
            assert_eq!(skip_by_path(path), None, "`{path}` is ordinary source");
        }
    }

    #[test]
    fn the_audit_may_record_more_selected_than_it_carries_files() {
        // §4.5 `:1128-1132`: three files were paid for and none reached the model.
        let audit = ExcerptAudit {
            selected: 3,
            ..ExcerptAudit::default()
        };
        assert_eq!(audit.selected, 3);
        assert!(audit.files.is_empty());
    }
}
