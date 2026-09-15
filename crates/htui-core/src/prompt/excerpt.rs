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

// -------------------------------------------------------------------------------------------
// The seam (§4.5 `:1141-1203`) and the five-tier ranker (§4.5 `:1020-1063`)
// -------------------------------------------------------------------------------------------

/// The built-in ranker's provider name. Never an agent's or a vendor's name (`R-AGT-5`).
pub const BUILTIN_NAME: &str = "builtin";
/// The built-in ranker's version. A tier change is a new version, because it changes the prompt.
pub const BUILTIN_VERSION: &str = "1";
/// `builtin@1`: the first entry of every `provider_set`, and the one that is never dropped.
pub const BUILTIN_ID: &str = "builtin@1";

/// Tier 1 (§4.5 `:1026`): a path under a `touched_paths` prefix.
pub const TIER1_TOUCHED: u16 = 100;
/// Tier 2 (`:1027`): a path the previous attempt changed.
pub const TIER2_PREV_DIFF: u16 = 90;
/// Tier 3 (`:1028`): a path-like token in the item body or a resolved input document.
pub const TIER3_MENTIONED: u16 = 70;
/// Tier 4 (`:1029`): a path component matching a mentioned identifier.
pub const TIER4_IDENTIFIER: u16 = 50;
/// Tier 5's floor (`:1030`): `10 + a 0..9 lexical score`.
pub const TIER5_BASE: u16 = 10;
/// Tier 5's span: the lexical score is quantised into `0..=9` before anything is compared.
pub const TIER5_SPAN: u16 = 9;

/// Aider's identifier filter (`repomap.py:493-494`): shorter tokens are noise.
const MIN_IDENT_LEN: usize = 8;
/// The shortest word part a split identifier contributes. `gpt` is three (§4.5 `:1029`).
const MIN_PART_LEN: usize = 3;
/// How many bytes of a file the lexical tier reads (§4.5 `:1030`).
pub const LEXICAL_HEAD_BYTES: usize = 4_096;
/// The scale the tier-5 float is quantised on before it becomes a weight (blueprint G rule 5).
const LEXICAL_SCALE: f64 = 1_000.0;

/// The extensions tier 3 accepts on a path-like token (§4.5 `:1028`), a closed list.
///
/// Closed and not "anything after the last dot" so a version string such as `1.2.3` or a sentence
/// ending in `etc./foo.` cannot become a file mention.
const SOURCE_EXTENSIONS: [&str; 30] = [
    "c", "cc", "cfg", "cpp", "cs", "css", "go", "h", "hpp", "html", "java", "js", "json", "jsx",
    "kt", "md", "mjs", "php", "py", "rb", "rs", "scss", "sh", "sql", "toml", "ts", "tsx", "txt",
    "yaml", "yml",
];

/// Everything a provider is asked to propose against (§4.5 `:1147-1157`).
///
/// Borrowed rather than owned so the caller's `PromptSpec` inputs are not cloned per provider;
/// [`OwnedExcerptRequest`] is the `'static` mirror `htui_agent::excerpt::run_providers` moves into
/// a thread.
#[derive(Debug, Clone, Copy)]
pub struct ExcerptRequest<'a> {
    /// `<project.slug>:<item.key>`.
    pub item_key: &'a str,
    /// `item.body`, verbatim. Tiers 3, 4 and 5 read it.
    pub item_body: &'a str,
    /// `ResolvedPhase.name`.
    pub phase: &'a str,
    /// The resolved input documents' bodies; tier 3 reads them too (§4.5 `:1028`).
    pub document_bodies: &'a [String],
    /// `item.touched_paths`, already truncated and repo-qualified.
    pub touched_prefixes: &'a [PathPrefix],
    /// The previous attempt's changed paths; empty on attempt 1.
    pub changed_paths: &'a [RepoPath],
    /// One per repo in scope, with the root §4.5 step 1 resolved.
    pub roots: &'a [RepoRoot],
    /// The residual budget from §4.4 step 6. `0` or less means "no room", and nothing is selected.
    pub budget_tokens: i64,
    /// The four `excerpt_*` caps, recorded verbatim in the audit.
    pub caps: ExcerptCaps,
    /// `app_setting.excerpt_max_scan_files`: the walk's cap, and the `scan_truncated` trigger.
    pub scan_cap: u32,
    /// How long a provider has to answer before it is dropped and recorded (§4.5 rule 2).
    pub deadline: core::time::Duration,
}

/// An owned [`ExcerptRequest`], so a provider can run on a thread that outlives its deadline.
///
/// Hazard H-20: a provider that misses its deadline is *dropped*, not waited for, so the thread it
/// runs on may outlive the call. A scoped thread cannot express that — it joins at the end of the
/// scope — so the data a provider sees has to be `'static`, which is what this type is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedExcerptRequest {
    /// See [`ExcerptRequest::item_key`].
    pub item_key: String,
    /// See [`ExcerptRequest::item_body`].
    pub item_body: String,
    /// See [`ExcerptRequest::phase`].
    pub phase: String,
    /// See [`ExcerptRequest::document_bodies`].
    pub document_bodies: Vec<String>,
    /// See [`ExcerptRequest::touched_prefixes`].
    pub touched_prefixes: Vec<PathPrefix>,
    /// See [`ExcerptRequest::changed_paths`].
    pub changed_paths: Vec<RepoPath>,
    /// See [`ExcerptRequest::roots`].
    pub roots: Vec<RepoRoot>,
    /// See [`ExcerptRequest::budget_tokens`].
    pub budget_tokens: i64,
    /// See [`ExcerptRequest::caps`].
    pub caps: ExcerptCaps,
    /// See [`ExcerptRequest::scan_cap`].
    pub scan_cap: u32,
    /// See [`ExcerptRequest::deadline`].
    pub deadline: core::time::Duration,
}

impl OwnedExcerptRequest {
    /// Clone every borrowed field, once, before the threads start.
    #[must_use]
    pub fn from_request(req: &ExcerptRequest<'_>) -> Self {
        Self {
            item_key: req.item_key.to_owned(),
            item_body: req.item_body.to_owned(),
            phase: req.phase.to_owned(),
            document_bodies: req.document_bodies.to_vec(),
            touched_prefixes: req.touched_prefixes.to_vec(),
            changed_paths: req.changed_paths.to_vec(),
            roots: req.roots.to_vec(),
            budget_tokens: req.budget_tokens,
            caps: req.caps,
            scan_cap: req.scan_cap,
            deadline: req.deadline,
        }
    }

    /// Borrow it back as the request every provider is called with.
    #[must_use]
    pub fn as_request(&self) -> ExcerptRequest<'_> {
        ExcerptRequest {
            item_key: &self.item_key,
            item_body: &self.item_body,
            phase: &self.phase,
            document_bodies: &self.document_bodies,
            touched_prefixes: &self.touched_prefixes,
            changed_paths: &self.changed_paths,
            roots: &self.roots,
            budget_tokens: self.budget_tokens,
            caps: self.caps,
            scan_cap: self.scan_cap,
            deadline: self.deadline,
        }
    }
}

/// One proposed file, before `htui` ranks, windows, caps, renders and digests it (§4.5 `:1159-1166`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptCandidate {
    /// The repo slug, never a path.
    pub repo: String,
    /// The repo-relative path.
    pub path: String,
    /// The window the proposer wants, 1-based and inclusive; `None` means "no opinion".
    pub lines: Option<(u32, u32)>,
    /// `0..=100`. `htui` re-normalises against its own tiers.
    pub weight: u16,
    /// One of the five tier names, or the proposing provider's name — which
    /// [`select`] renders as `reason="provider:<name>"`.
    pub reason: String,
}

/// A provider that declined, in the one shape `provider_set` can record.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("excerpt provider `{provider}`: {message}")]
pub struct ProviderError {
    /// The provider's [`name`](ExcerptProvider::name), or the reader's own.
    pub provider: String,
    /// What went wrong, already free of secrets: it lands in `trim_record.notes`.
    pub message: String,
}

impl ProviderError {
    /// The two-field constructor, so a caller does not spell the struct out per call site.
    #[must_use]
    pub fn new(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            message: message.into(),
        }
    }
}

/// A source of excerpt candidates (`R-LATER-7`), with §4.5's five rules (`:1177-1199`).
///
/// 1. **Providers propose candidates; `htui` renders.** Otherwise the prompt bytes, and therefore
///    the digest, depend on a third party's formatting.
/// 2. **Fail-open with a deadline.** Absent, erroring, panicking or slow means drop the provider,
///    record it in `provider_set` with its status, and continue (`R-NF-2`).
/// 3. **A provider's participation is recorded and is allowed to change the digest.** A provider
///    that changes the excerpt set genuinely changes the prompt, so pretending otherwise would make
///    the digest a lie.
/// 4. **Read-only, `R-ID-4`.** The trait has no write method and [`RepoRoot`] carries no writable
///    handle. A provider that writes a directory into the project breaks `R-ID-4` itself.
/// 5. **Non-LLM, `R-ID-6`.** Stated here, unenforceable by the type system, and therefore an ANA-3
///    review criterion rather than a compile error.
///
/// Synchronous by design: a provider is a lookup, and the deadline runner
/// (`htui_agent::excerpt::run_providers`) is what gives it a thread.
pub trait ExcerptProvider: Send + Sync + core::fmt::Debug {
    /// The provider's stable name, as it appears in `provider_set` and in `reason="provider:…"`.
    fn name(&self) -> &str;
    /// The provider's version, as it appears in `provider_set`.
    fn version(&self) -> &str;
    /// Propose candidates. Never a model call (rule 5), never a write (rule 4).
    ///
    /// # Errors
    ///
    /// Anything at all: the caller drops the provider and records it, which is rule 2.
    fn propose(&self, req: &ExcerptRequest<'_>) -> Result<Vec<ExcerptCandidate>, ProviderError>;
}

/// Read-only by type (ANA-5 invariant 6): list and read, nothing else.
///
/// The filesystem implementation is `htui_agent::excerpt::FsRepoReader`; this crate names no
/// `std::fs` (§4.8), so a `htui-core` test drives [`select`] through a double and touches no disk.
pub trait RepoReader: Send + Sync + core::fmt::Debug {
    /// Repo-relative `/`-separated paths in byte order at every level, content-skip rules already
    /// applied. The `bool` is `scan_truncated`: the cap bit and the listing is partial.
    ///
    /// # Errors
    ///
    /// An unreadable root. A repo that cannot be listed contributes nothing and is noted, which is
    /// §4.5 step 1's fail-open.
    fn list(&self, root: &RepoRoot, cap: u32) -> Result<(Vec<String>, bool), ProviderError>;
    /// The file's text, LF-normalised.
    ///
    /// # Errors
    ///
    /// A path the reader cannot read is an error, never a panic.
    fn read(&self, root: &RepoRoot, path: &str) -> Result<String, ProviderError>;
}

/// One listed candidate as the **pure** ranker sees it (§4.5 step 2's output).
///
/// [`head`](Self::head) is the first [`LEXICAL_HEAD_BYTES`] of the file when tier 5 will be asked
/// about it, and empty otherwise — which is what keeps [`rank`] a pure function over a supplied
/// listing rather than something that needs a reader.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listed {
    /// The repo slug.
    pub repo: String,
    /// The repo-relative path.
    pub path: String,
    /// The file's first bytes, for tier 5; empty when unread.
    pub head: String,
}

/// The built-in ranker, `builtin@1` (§4.5 `:1201-1203`).
///
/// Registered first and never removable, so "no providers configured" and "every provider failed"
/// are the same code path and are exercised by the same tests. Its `propose` returns nothing on
/// purpose: the built-in's candidates come from the **listing**, which only [`select`] holds, and
/// [`ExcerptRequest`] deliberately carries no reader (rule 4 — a provider is given signals, never a
/// handle). The impl exists so registration, ordering and the `provider_set` grammar have exactly
/// one implementation; the tiers themselves are [`rank`], which is pure and unit-tested.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinRanker;

impl ExcerptProvider for BuiltinRanker {
    fn name(&self) -> &str {
        BUILTIN_NAME
    }

    fn version(&self) -> &str {
        BUILTIN_VERSION
    }

    fn propose(&self, _req: &ExcerptRequest<'_>) -> Result<Vec<ExcerptCandidate>, ProviderError> {
        Ok(Vec::new())
    }
}

/// The signals tiers 3, 4 and 5 read, derived once from the item body and its documents.
#[derive(Debug, Default)]
struct Signals {
    /// Path-like tokens: `[A-Za-z0-9_./-]+` containing `/`, ending in a known source extension.
    mentioned: std::collections::BTreeSet<String>,
    /// Lowercase word parts of every identifier of at least [`MIN_IDENT_LEN`] characters.
    idents: std::collections::BTreeSet<String>,
}

impl Signals {
    /// §4.5 step 3, over the item body and every resolved input document.
    fn of(req: &ExcerptRequest<'_>) -> Self {
        let mut signals = Self::default();
        signals.absorb(req.item_body);
        for body in req.document_bodies {
            signals.absorb(body);
        }
        signals
    }

    fn absorb(&mut self, text: &str) {
        for token in tokens(text) {
            if token.contains('/') && has_source_extension(token) {
                self.mentioned.insert(token.to_owned());
            }
            if is_identifier(token) {
                for part in word_parts(token) {
                    self.idents.insert(part);
                }
            }
        }
    }
}

/// Every `[A-Za-z0-9_./-]+` run in `text`, with trailing punctuation trimmed.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')))
        .map(|token| token.trim_matches(|c| matches!(c, '.' | '-' | '/')))
        .filter(|token| !token.is_empty())
}

/// Whether a path-like token ends in one of [`SOURCE_EXTENSIONS`].
fn has_source_extension(token: &str) -> bool {
    token
        .rsplit_once('.')
        .is_some_and(|(_, ext)| SOURCE_EXTENSIONS.contains(&ext))
}

/// Aider's filter (`repomap.py:493-494`): at least [`MIN_IDENT_LEN`] characters **and** snake,
/// kebab or camel case, so a long ordinary word is not an identifier.
fn is_identifier(token: &str) -> bool {
    if token.chars().count() < MIN_IDENT_LEN {
        return false;
    }
    if token.contains(['_', '-', '/', '.']) {
        return true;
    }
    token
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0].is_ascii_lowercase() && pair[1].is_ascii_uppercase())
}

/// Split on `_`, `-`, `/`, `.` and on every lower→upper transition, lowercased (Sweep's rule, so
/// `ChatGPT`, `chat_gpt` and `chatGPT` all yield `chat` and `gpt`).
fn word_parts(token: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for ch in token.chars() {
        if matches!(ch, '_' | '-' | '/' | '.') {
            if current.chars().count() >= MIN_PART_LEN {
                parts.push(core::mem::take(&mut current));
            } else {
                current.clear();
            }
            previous_lower = false;
            continue;
        }
        if ch.is_ascii_uppercase() && previous_lower {
            if current.chars().count() >= MIN_PART_LEN {
                parts.push(core::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
        current.push(ch.to_ascii_lowercase());
        previous_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    if current.chars().count() >= MIN_PART_LEN {
        parts.push(current);
    }
    parts
}

/// Every suffix of a path at a `/` boundary, longest first — what tier 3 tests membership of.
fn path_suffixes(path: &str) -> Vec<&str> {
    let mut out = vec![path];
    let mut rest = path;
    while let Some((_, tail)) = rest.split_once('/') {
        out.push(tail);
        rest = tail;
    }
    out
}

/// Tiers 1–4, which need no file bytes at all.
fn tier_of(
    req: &ExcerptRequest<'_>,
    signals: &Signals,
    entry: &Listed,
) -> Option<(u16, &'static str)> {
    if req
        .touched_prefixes
        .iter()
        .any(|prefix| prefix.matches(&entry.repo, &entry.path))
    {
        return Some((TIER1_TOUCHED, "touched_path"));
    }
    if req
        .changed_paths
        .iter()
        .any(|changed| changed.repo == entry.repo && changed.path == entry.path)
    {
        return Some((TIER2_PREV_DIFF, "prev_diff"));
    }
    if path_suffixes(&entry.path)
        .into_iter()
        .any(|suffix| signals.mentioned.contains(suffix))
    {
        return Some((TIER3_MENTIONED, "mentioned"));
    }
    if entry
        .path
        .split('/')
        .flat_map(word_parts)
        .any(|part| signals.idents.contains(&part))
    {
        return Some((TIER4_IDENTIFIER, "identifier"));
    }
    None
}

/// The five tiers as one pure function over a supplied listing (§4.5 steps 3–5).
///
/// **Max, not sum** (`:1020-1022`): a file takes the highest tier it qualifies for, so one file
/// cannot outrank another by accumulating weak evidence. Tier 5 runs only when tiers 1–4 leave
/// `max_files` unspent and never contributes more than a third of it (`:1032-1033`). The result is
/// sorted `(weight desc, repo asc, path asc)` on raw bytes, with tier 5's float quantised to an
/// integer **before** anything is compared (`:1048-1051`).
///
/// Pure over `listing`: no reader, no filesystem, no clock. [`Listed::head`] is what tier 5 scores,
/// and a listing with empty heads simply scores every tier-5 candidate on its path.
#[must_use]
pub fn rank(req: &ExcerptRequest<'_>, listing: &[Listed]) -> Vec<ExcerptCandidate> {
    let signals = Signals::of(req);
    let mut out = Vec::new();
    let mut leftovers = Vec::new();
    for entry in listing {
        match tier_of(req, &signals, entry) {
            Some((weight, reason)) => out.push(ExcerptCandidate {
                repo: entry.repo.clone(),
                path: entry.path.clone(),
                lines: None,
                weight,
                reason: reason.to_owned(),
            }),
            None => leftovers.push(entry),
        }
    }
    let max_files = req.caps.max_files as usize;
    let tier5_cap = (max_files / 3).min(max_files.saturating_sub(out.len()));
    if tier5_cap > 0 {
        out.extend(lexical(&signals, &leftovers, tier5_cap));
    }
    sort_candidates(&mut out);
    out
}

/// §4.5 step 5's composite key: `(weight desc, repo asc, path asc)` on raw bytes.
fn sort_candidates(candidates: &mut [ExcerptCandidate]) {
    candidates.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| a.repo.as_bytes().cmp(b.repo.as_bytes()))
            .then_with(|| a.path.as_bytes().cmp(b.path.as_bytes()))
    });
}

/// Tier 5: a hand-rolled TF-IDF over the candidate set, quantised to `0..=9` (§4.5 `:1030`).
///
/// No crate and nothing learned: Repoformer found that neither UniXCoder nor CodeBLEU outperformed
/// Jaccard similarity (<https://arxiv.org/html/2403.10059v1>), which removes the argument for
/// anything heavier. The one float in the whole module lives here and dies here — the score is
/// multiplied by [`LEXICAL_SCALE`], truncated to an integer, and only integers are ever compared.
fn lexical(signals: &Signals, leftovers: &[&Listed], cap: usize) -> Vec<ExcerptCandidate> {
    if signals.idents.is_empty() || leftovers.is_empty() {
        // No query terms: every candidate scores zero, so the floor weight and path order decide.
        return leftovers
            .iter()
            .take(cap)
            .map(|entry| lexical_candidate(entry, 0))
            .collect();
    }
    // One term-frequency map per candidate, restricted to the query terms.
    let mut frequencies: Vec<std::collections::BTreeMap<&str, u32>> = Vec::new();
    let mut document_frequency: std::collections::BTreeMap<&str, u32> =
        std::collections::BTreeMap::new();
    for entry in leftovers {
        let mut counts: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
        for part in document_terms(entry) {
            if let Some(term) = signals.idents.get(&part) {
                *counts.entry(term.as_str()).or_default() += 1;
            }
        }
        for term in counts.keys() {
            *document_frequency.entry(term).or_default() += 1;
        }
        frequencies.push(counts);
    }
    let total = leftovers.len() as f64;
    let scores: Vec<u64> = frequencies
        .iter()
        .map(|counts| {
            let score: f64 = counts
                .iter()
                .map(|(term, count)| {
                    let df = f64::from(document_frequency.get(term).copied().unwrap_or(1)).max(1.0);
                    (1.0 + f64::from(*count).ln()) * (1.0 + total / df).ln()
                })
                .sum();
            // The one quantisation: everything downstream is integer arithmetic.
            if score.is_finite() && score > 0.0 {
                (score * LEXICAL_SCALE) as u64
            } else {
                0
            }
        })
        .collect();
    let peak = scores.iter().copied().max().unwrap_or(0);
    let mut candidates: Vec<ExcerptCandidate> = leftovers
        .iter()
        .zip(&scores)
        .map(|(entry, score)| {
            let lex = (score * u64::from(TIER5_SPAN))
                .checked_div(peak)
                .map_or(0, |scaled| u16::try_from(scaled).unwrap_or(TIER5_SPAN));
            lexical_candidate(entry, lex.min(TIER5_SPAN))
        })
        .collect();
    sort_candidates(&mut candidates);
    candidates.truncate(cap);
    candidates
}

/// One tier-5 candidate at a quantised `0..=9` lexical score.
fn lexical_candidate(entry: &Listed, lex: u16) -> ExcerptCandidate {
    ExcerptCandidate {
        repo: entry.repo.clone(),
        path: entry.path.clone(),
        lines: None,
        weight: TIER5_BASE + lex,
        reason: "lexical".to_owned(),
    }
}

/// A tier-5 document's terms: the path's word parts plus the head's, the same split both sides.
fn document_terms(entry: &Listed) -> Vec<String> {
    let mut terms: Vec<String> = entry.path.split('/').flat_map(word_parts).collect();
    let head = &entry.head[..entry.head.len().min(LEXICAL_HEAD_BYTES)];
    // A truncation at 4 KB can land inside a multi-byte character; the lossy tail is one term at
    // most and the score is a ranking signal, not a checksum.
    for token in tokens(head) {
        terms.extend(word_parts(token));
    }
    terms
}

// -------------------------------------------------------------------------------------------
// §4.5 steps 1–10, over a reader (`:1035-1064`)
// -------------------------------------------------------------------------------------------

/// How many file heads the lexical tier will read before it scores the rest on their paths alone.
///
/// ANA-5 bounds the *walk* (`excerpt_max_scan_files`, 20 000) and does not bound the tier-5
/// **read**, which would be 20 000 file opens for a signal worth `10 + 0..9`. The cap is recorded
/// in `notes` when it bites, so a thin lexical tier is visible rather than inferred.
pub const LEXICAL_HEAD_READS: usize = 400;

/// How many per-file denylist notes `select` writes before it falls back to the aggregate count.
const DENIED_NOTE_CAP: usize = 20;

/// One audit row for a rendered excerpt, hashing the bytes the model was given (§4.5 `:1136-1137`).
///
/// `rendered` is the excerpt's `<file>` block exactly as it lands in the prompt — which is why this
/// takes it rather than re-deriving it: [`crate::prompt::assemble`] scrubs before it digests, so the
/// bytes a reader can prove are the scrubbed ones (plan D100), and a record that hashed the
/// pre-scrub rendering would describe a string nobody was sent.
#[must_use]
pub fn file_record(excerpt: &Excerpt, rendered: &str) -> FileRecord {
    FileRecord {
        repo: excerpt.repo.clone(),
        path: excerpt.path.clone(),
        lines: format!("{}-{}", excerpt.first_line, excerpt.last_line),
        rank: excerpt.rank,
        weight: excerpt.weight,
        reason: excerpt.reason,
        truncated: excerpt.truncated,
        bytes: rendered.len() as u64,
        sha256: crate::prompt::digest::sha256_hex(rendered),
    }
}

/// Whether a candidate path is repo-relative, as §4.2 rule 5 requires (hazard H-1).
///
/// A provider that proposed `/etc/passwd` or `C:\secrets` would put an absolute path into a
/// digested byte, so the guard lives here rather than in a renderer that has no way to refuse.
fn is_repo_relative(path: &str) -> bool {
    let bytes = path.as_bytes();
    if path.is_empty() || bytes[0] == b'/' || bytes[0] == b'\\' {
        return false;
    }
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return false;
    }
    !path.split('/').any(|segment| segment == "..")
}

/// §4.5 step 8: the window one file contributes, and what it cost to cut it.
///
/// Returns `(content, first_line, last_line, truncated, elided_lines, elided_bytes)`. `want` is a
/// provider's window opinion; `None` means the whole file, and either way the line cap applies.
fn window(text: &str, caps: ExcerptCaps, want: Option<(u32, u32)>) -> Option<Windowed> {
    let text = crate::prompt::render::normalise_newlines(text);
    let all: Vec<&str> = text.lines().collect();
    if all.is_empty() {
        return None;
    }
    let total = u32::try_from(all.len()).unwrap_or(u32::MAX);
    let (first, last) = match want {
        Some((from, to)) => (from.max(1).min(total), to.max(1).min(total)),
        None => (1, total),
    };
    let (first, last) = if first > last {
        (1, total)
    } else {
        (first, last)
    };
    let slice = &all[(first as usize - 1)..(last as usize)];
    let cap = caps.file_line_cap.max(1) as usize;
    if slice.len() <= cap {
        return Some(Windowed {
            content: joined(slice),
            first_line: first,
            last_line: last,
            truncated: false,
            elided_lines: 0,
            elided_bytes: 0,
        });
    }
    let head = (caps.head_lines.max(1) as usize).min(slice.len().saturating_sub(1));
    let dropped = &slice[head..];
    Some(Windowed {
        content: joined(&slice[..head]),
        first_line: first,
        last_line: first + u32::try_from(head).unwrap_or(u32::MAX) - 1,
        truncated: true,
        elided_lines: u32::try_from(dropped.len()).unwrap_or(u32::MAX),
        // `+ 1` per line for the LF the split removed, the same arithmetic `trim.rs` bills a
        // head+tail elision with, so the two markers mean one thing.
        elided_bytes: dropped.iter().map(|line| line.len() as u64 + 1).sum(),
    })
}

/// [`window`]'s six results, named rather than positional.
struct Windowed {
    content: String,
    first_line: u32,
    last_line: u32,
    truncated: bool,
    elided_lines: u32,
    elided_bytes: u64,
}

/// Lines back into text, LF-terminated, which is what [`Excerpt::content`] is.
fn joined(lines: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// §4.5 steps 1–10 over a reader and a candidate list already merged from every provider.
///
/// Pure over `reader`: with an in-memory double no filesystem is touched, which is what makes the
/// tier, denylist and windowing cases unit tests rather than integration tests (hazard H-22).
///
/// `providers` is the `provider_set` [`htui_agent::excerpt::run_providers`] produced, in P-11's
/// grammar; [`BUILTIN_ID`] is forced to the front if the caller left it out, because §4.5 `:1201`
/// makes the built-in un-removable and a record that omitted it would claim a prompt no code path
/// can produce.
///
/// Every failure here is a **note**, never an error (§4.5 step 1's fail-open): an unresolved root,
/// an unlistable repo, a denied file, an unreadable candidate and an exhausted budget all leave a
/// valid prompt with a smaller excerpt section and a record that says why.
#[must_use]
pub fn select(
    reader: &dyn RepoReader,
    req: &ExcerptRequest<'_>,
    merged: Vec<ExcerptCandidate>,
    providers: Vec<String>,
    est: crate::prompt::TokenEstimator,
) -> ExcerptSet {
    let mut notes = Vec::new();
    let mut provider_set = providers;
    if !provider_set.iter().any(|entry| entry == BUILTIN_ID) {
        provider_set.insert(0, BUILTIN_ID.to_owned());
    }

    // -- steps 1 and 2: resolve a root per repo, list under it, apply the path-only skip rules --
    let mut roots: Vec<&RepoRoot> = req.roots.iter().collect();
    roots.sort_by(|a, b| a.repo.as_bytes().cmp(b.repo.as_bytes()));
    let mut root_records = Vec::new();
    let mut listing: Vec<Listed> = Vec::new();
    let mut skipped: std::collections::BTreeMap<&'static str, u32> =
        std::collections::BTreeMap::new();
    let mut denied_notes = Vec::new();
    for root in &roots {
        if root.source == RootSource::NoPath {
            root_records.push(RootRecord {
                repo: root.repo.clone(),
                source: RootSource::NoPath,
                scan_truncated: false,
            });
            notes.push(format!(
                "excerpt: no readable root for repo `{}`; nothing was scanned",
                root.repo
            ));
            continue;
        }
        let (paths, truncated) = match reader.list(root, req.scan_cap) {
            Ok(listed) => listed,
            Err(error) => {
                root_records.push(RootRecord {
                    repo: root.repo.clone(),
                    source: root.source,
                    scan_truncated: false,
                });
                notes.push(format!(
                    "excerpt: repo `{}` could not be listed: {}",
                    root.repo, error.message
                ));
                continue;
            }
        };
        root_records.push(RootRecord {
            repo: root.repo.clone(),
            source: root.source,
            scan_truncated: truncated,
        });
        if truncated {
            notes.push(format!(
                "excerpt: repo `{}` hit the scan cap of {} files; the listing is partial",
                root.repo, req.scan_cap
            ));
        }
        for path in paths {
            if let Some(rule) = skip_by_path(&path) {
                *skipped.entry(rule).or_default() += 1;
                let declared = req
                    .touched_prefixes
                    .iter()
                    .any(|prefix| prefix.matches(&root.repo, &path))
                    || req
                        .changed_paths
                        .iter()
                        .any(|changed| changed.repo == root.repo && changed.path == path);
                if declared && denied_notes.len() < DENIED_NOTE_CAP {
                    // ANA-5 §12 criterion 14: a denied file that was *declared* is the case a
                    // maintainer has to be told about, because otherwise the declaration looks
                    // honoured and is not.
                    denied_notes.push(format!(
                        "excerpt: `{}:{path}` is declared but excluded by rule `{rule}`; never selected",
                        root.repo
                    ));
                }
                continue;
            }
            if !is_repo_relative(&path) {
                notes.push(format!(
                    "excerpt: listed path `{}:{path}` is not repo-relative; dropped",
                    root.repo
                ));
                continue;
            }
            listing.push(Listed {
                repo: root.repo.clone(),
                path,
                head: String::new(),
            });
        }
    }
    notes.extend(denied_notes);
    for (rule, count) in &skipped {
        notes.push(format!("excerpt: {count} path(s) skipped by rule `{rule}`"));
    }
    let considered = u32::try_from(listing.len()).unwrap_or(u32::MAX);

    // -- steps 3 to 5: the tiers. Heads are read only for the files tier 5 will actually score. --
    fill_lexical_heads(reader, req, &roots, &mut listing, &mut notes);
    let builtin = rank(req, &listing);

    // -- step 6: merge the providers' candidates by `(repo, path)`, keeping the higher weight -----
    let mut by_path: std::collections::BTreeMap<(String, String), ExcerptCandidate> =
        std::collections::BTreeMap::new();
    for candidate in builtin {
        by_path.insert((candidate.repo.clone(), candidate.path.clone()), candidate);
    }
    for candidate in merged {
        if !is_repo_relative(&candidate.path) {
            notes.push(format!(
                "excerpt: candidate `{}:{}` is not repo-relative; dropped",
                candidate.repo, candidate.path
            ));
            continue;
        }
        if let Some(rule) = skip_by_path(&candidate.path) {
            notes.push(format!(
                "excerpt: candidate `{}:{}` is excluded by rule `{rule}`; never selected",
                candidate.repo, candidate.path
            ));
            continue;
        }
        let key = (candidate.repo.clone(), candidate.path.clone());
        match by_path.get(&key) {
            // A tie keeps the built-in's tier reason: two orders for one weight would make the
            // rendered `reason=` depend on the order providers happened to answer in.
            Some(existing) if existing.weight >= candidate.weight => {}
            _ => {
                by_path.insert(key, candidate);
            }
        }
    }
    let mut candidates: Vec<ExcerptCandidate> = by_path.into_values().collect();
    sort_candidates(&mut candidates);

    // -- steps 7 and 8: take in rank order until a cap binds, windowing each file ----------------
    let mut files: Vec<Excerpt> = Vec::new();
    let mut spent = 0i64;
    let max_files = req.caps.max_files as usize;
    if req.budget_tokens <= 0 && !candidates.is_empty() {
        notes.push("excerpt: the residual budget is zero; no file was taken".to_owned());
    }
    let mut not_taken = 0u32;
    for candidate in candidates {
        if files.len() >= max_files || req.budget_tokens <= 0 {
            not_taken += 1;
            continue;
        }
        let Some(root) = roots.iter().find(|root| root.repo == candidate.repo) else {
            notes.push(format!(
                "excerpt: candidate `{}:{}` names a repo with no root; dropped",
                candidate.repo, candidate.path
            ));
            continue;
        };
        let text = match reader.read(root, &candidate.path) {
            Ok(text) => text,
            Err(error) => {
                notes.push(format!(
                    "excerpt: `{}:{}` could not be read: {}",
                    candidate.repo, candidate.path, error.message
                ));
                continue;
            }
        };
        let bytes = text.len() as u64;
        if bytes > req.caps.max_file_bytes {
            // §4.5 step 8: skipped rather than windowed — a file this large that is not tier 1 or
            // 2 is almost always generated.
            notes.push(format!(
                "excerpt: `{}:{}` is {bytes} bytes, over max_file_bytes ({}); skipped",
                candidate.repo, candidate.path, req.caps.max_file_bytes
            ));
            continue;
        }
        let Some(windowed) = window(&text, req.caps, candidate.lines) else {
            continue;
        };
        let (reason, provider) = reason_of(&candidate.reason);
        let excerpt = Excerpt {
            repo: candidate.repo,
            path: candidate.path,
            first_line: windowed.first_line,
            last_line: windowed.last_line,
            truncated: windowed.truncated,
            elided_lines: windowed.elided_lines,
            elided_bytes: windowed.elided_bytes,
            rank: u32::try_from(files.len() + 1).unwrap_or(u32::MAX),
            weight: candidate.weight,
            reason,
            provider,
            content: windowed.content,
        };
        let cost = est.estimate(&crate::prompt::render::file_block(&excerpt));
        if spent + cost > req.budget_tokens {
            not_taken += 1;
            continue;
        }
        spent += cost;
        files.push(excerpt);
    }
    if not_taken > 0 {
        notes.push(format!(
            "excerpt: {not_taken} candidate(s) not taken; the residual budget of {} tokens and \
             max_files of {max_files} bound first",
            req.budget_tokens
        ));
    }

    // -- step 10: the audit ---------------------------------------------------------------------
    let audit = ExcerptAudit {
        provider_set,
        roots: root_records,
        considered,
        selected: u32::try_from(files.len()).unwrap_or(u32::MAX),
        caps: req.caps,
        files: files
            .iter()
            .map(|file| file_record(file, &crate::prompt::render::file_block(file)))
            .collect(),
    };
    ExcerptSet {
        files,
        audit,
        notes,
    }
}

/// A candidate's `reason` string back into the closed enum, or into a provider attribution.
///
/// A string that is not one of the five tier names **is** a provider name: `run_providers` rewrites
/// every returned candidate's reason to the provider that returned it, so a provider cannot claim
/// a tier — or another provider's name — by spelling one.
fn reason_of(reason: &str) -> (ExcerptReason, Option<String>) {
    match reason {
        "touched_path" => (ExcerptReason::TouchedPath, None),
        "prev_diff" => (ExcerptReason::PrevDiff, None),
        "mentioned" => (ExcerptReason::Mentioned, None),
        "identifier" => (ExcerptReason::Identifier, None),
        "lexical" => (ExcerptReason::Lexical, None),
        other => (ExcerptReason::Provider, Some(other.to_owned())),
    }
}

/// Read the first [`LEXICAL_HEAD_BYTES`] of the files tier 5 is going to score, and no others.
///
/// Bounded by [`LEXICAL_HEAD_READS`] and noted when the bound bites. Files past it keep an empty
/// head and are scored on their path alone, which is a weaker tier-5 signal rather than an absent
/// candidate.
fn fill_lexical_heads(
    reader: &dyn RepoReader,
    req: &ExcerptRequest<'_>,
    roots: &[&RepoRoot],
    listing: &mut [Listed],
    notes: &mut Vec<String>,
) {
    let max_files = req.caps.max_files as usize;
    if max_files / 3 == 0 {
        return;
    }
    let signals = Signals::of(req);
    let leftovers: Vec<usize> = listing
        .iter()
        .enumerate()
        .filter(|(_, entry)| tier_of(req, &signals, entry).is_none())
        .map(|(index, _)| index)
        .collect();
    if listing.len() - leftovers.len() >= max_files {
        // Tiers 1–4 already fill `max_files`, so tier 5 will not run and no head is worth reading.
        return;
    }
    let mut read = 0usize;
    for index in &leftovers {
        if read >= LEXICAL_HEAD_READS {
            break;
        }
        let entry = &listing[*index];
        let Some(root) = roots.iter().find(|root| root.repo == entry.repo) else {
            continue;
        };
        if let Ok(text) = reader.read(root, &entry.path) {
            // Cut on a character boundary at or below the byte cap, so the head is still UTF-8.
            let cut = if text.len() <= LEXICAL_HEAD_BYTES {
                text.len()
            } else {
                (0..=LEXICAL_HEAD_BYTES)
                    .rev()
                    .find(|at| text.is_char_boundary(*at))
                    .unwrap_or(0)
            };
            listing[*index].head = text[..cut].to_owned();
        }
        read += 1;
    }
    if leftovers.len() > read {
        notes.push(format!(
            "excerpt: the lexical tier read {read} file head(s); {} candidate(s) were scored on \
             their path alone",
            leftovers.len() - read
        ));
    }
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

    /// A listing entry with no head bytes: tiers 1–4 need none.
    fn listed(repo: &str, path: &str) -> Listed {
        Listed {
            repo: repo.to_owned(),
            path: path.to_owned(),
            head: String::new(),
        }
    }

    /// A request carrying only the signals a tier test needs.
    fn request(body: &str, touched: &[&str], changed: &[&str]) -> OwnedExcerptRequest {
        OwnedExcerptRequest {
            item_key: "htui:MOD-2".to_owned(),
            item_body: body.to_owned(),
            phase: "implement".to_owned(),
            document_bodies: Vec::new(),
            touched_prefixes: touched
                .iter()
                .map(|glob| PathPrefix::parse(glob, "htui"))
                .collect(),
            changed_paths: changed
                .iter()
                .map(|path| RepoPath {
                    repo: "htui".to_owned(),
                    path: (*path).to_owned(),
                })
                .collect(),
            roots: Vec::new(),
            budget_tokens: 100_000,
            caps: ExcerptCaps {
                max_files: 12,
                file_line_cap: 400,
                head_lines: 200,
                max_file_bytes: 524_288,
            },
            scan_cap: 20_000,
            deadline: core::time::Duration::from_millis(1_500),
        }
    }

    fn weight_of(candidates: &[ExcerptCandidate], path: &str) -> Option<u16> {
        candidates
            .iter()
            .find(|candidate| candidate.path == path)
            .map(|candidate| candidate.weight)
    }

    #[test]
    fn tier_weights_are_max_not_sum() {
        // §4.5 `:1020-1022`: "the **maximum** tier weight it qualifies for, not a sum, so one file
        // cannot outrank another by accumulating weak evidence."
        let owned = request(
            "Rework the prompt_assembler over crates/htui-core/src/prompt/mod.rs.\n",
            &["crates/htui-core/src/prompt/**"],
            &["crates/htui-core/src/prompt/mod.rs"],
        );
        let req = owned.as_request();
        let listing = [
            // Qualifies for tiers 1, 2, 3 and 4 at once.
            listed("htui", "crates/htui-core/src/prompt/mod.rs"),
            // Tier 2 only: changed but outside the prefix and unmentioned.
            listed("htui", "crates/htui-store/src/pg/read.rs"),
            // Tier 4 only: a path component matches the `prompt_assembler` identifier.
            listed("agy", "src/assembler/main.rs"),
        ];
        let ranked = rank(&req, &listing);
        assert_eq!(
            weight_of(&ranked, "crates/htui-core/src/prompt/mod.rs"),
            Some(100),
            "four signals still weigh 100, not 310"
        );
        assert!(
            ranked.iter().all(|candidate| candidate.weight <= 100),
            "no weight may exceed tier 1's: {ranked:?}"
        );
        assert_eq!(
            weight_of(&ranked, "src/assembler/main.rs"),
            Some(50),
            "a path component matching a mentioned identifier is tier 4"
        );
    }

    #[test]
    fn ties_break_on_path_bytes() {
        // §4.5 step 5: `(weight desc, repo_slug asc, path asc)` on raw bytes.
        let owned = request("nothing in particular\n", &["**"], &[]);
        let req = owned.as_request();
        let listing = [
            listed("htui", "b.rs"),
            listed("agy", "z.rs"),
            listed("htui", "a.rs"),
            listed("agy", "a.rs"),
        ];
        let ranked = rank(&req, &listing);
        // `**` is the empty prefix, so all four are tier 1 in the primary repo only; `agy` gets
        // whatever the lexical tier gives it, which is strictly less than 100.
        let keys: Vec<(u16, &str, &str)> = ranked
            .iter()
            .map(|candidate| {
                (
                    candidate.weight,
                    candidate.repo.as_str(),
                    candidate.path.as_str(),
                )
            })
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
                .then_with(|| a.2.as_bytes().cmp(b.2.as_bytes()))
        });
        assert_eq!(
            keys, sorted,
            "weight desc, then repo bytes, then path bytes"
        );
        assert_eq!(
            keys.first().map(|key| (key.1, key.2)),
            Some(("htui", "a.rs")),
            "the two tier-1 files lead, in path byte order"
        );
    }

    #[test]
    fn tier_five_scores_are_quantised_before_the_sort() {
        // §4.5 step 5: "Float scores from tier 5 are quantised to integers before sorting, so
        // ordering never depends on `f64` comparison."
        let owned = request(
            "Fix the token_estimator and the prompt_digest of the recorder_pump.\n",
            &[],
            &[],
        );
        let req = owned.as_request();
        let listing: Vec<Listed> = [
            ("src/aaa.txt", "token estimator estimator estimator\n"),
            ("src/bbb.txt", "token estimator digest\n"),
            ("src/ccc.txt", "unrelated prose about nothing at all\n"),
            ("src/ddd.txt", "digest digest pump pump recorder\n"),
        ]
        .into_iter()
        .map(|(path, head)| Listed {
            repo: "htui".to_owned(),
            path: path.to_owned(),
            head: head.to_owned(),
        })
        .collect();
        let ranked = rank(&req, &listing);
        assert!(!ranked.is_empty(), "the lexical tier produced nothing");
        for candidate in &ranked {
            assert!(
                (TIER5_BASE..=TIER5_BASE + TIER5_SPAN).contains(&candidate.weight),
                "a tier-5 weight is `10 + a 0..9 lexical score`, got {candidate:?}"
            );
            assert_eq!(candidate.reason, "lexical");
        }
        // The order is the composite integer key and nothing else.
        let keys: Vec<(u16, &str)> = ranked
            .iter()
            .map(|candidate| (candidate.weight, candidate.path.as_str()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
        });
        assert_eq!(keys, sorted);
    }

    #[test]
    fn tier_five_never_exceeds_a_third_of_max_files() {
        // §4.5 `:1032-1033`: tier 5 "never contributes more than a third of `max_files`", and runs
        // only when tiers 1 to 4 leave budget unspent.
        let mut owned = request("a body naming nothing\n", &[], &[]);
        owned.caps.max_files = 12;
        let listing: Vec<Listed> = (0..40)
            .map(|n| listed("htui", &format!("src/f{n:02}.rs")))
            .collect();
        let ranked = rank(&owned.as_request(), &listing);
        assert_eq!(ranked.len(), 4, "12 / 3 = 4, and every file is tier 5");

        // Tiers 1–4 filling `max_files` leave tier 5 nothing to do.
        let mut full = request("a body naming nothing\n", &["src/"], &[]);
        full.caps.max_files = 3;
        let ranked = rank(&full.as_request(), &listing);
        assert!(
            ranked.iter().all(|candidate| candidate.weight == 100),
            "tier 1 already fills `max_files`, so tier 5 does not run: {ranked:?}"
        );
    }

    /// A [`RepoReader`] over an in-memory tree: no filesystem, so every `select` case below is a
    /// unit test (§4.8 — `htui-core` names no `std::fs`).
    #[derive(Debug, Default)]
    struct MapReader {
        files: std::collections::BTreeMap<(String, String), String>,
        truncated: bool,
    }

    impl MapReader {
        fn with(files: &[(&str, &str, &str)]) -> Self {
            Self {
                files: files
                    .iter()
                    .map(|(repo, path, body)| {
                        (((*repo).to_owned(), (*path).to_owned()), (*body).to_owned())
                    })
                    .collect(),
                truncated: false,
            }
        }
    }

    impl RepoReader for MapReader {
        fn list(&self, root: &RepoRoot, _cap: u32) -> Result<(Vec<String>, bool), ProviderError> {
            Ok((
                self.files
                    .keys()
                    .filter(|(repo, _)| *repo == root.repo)
                    .map(|(_, path)| path.clone())
                    .collect(),
                self.truncated,
            ))
        }

        fn read(&self, root: &RepoRoot, path: &str) -> Result<String, ProviderError> {
            self.files
                .get(&(root.repo.clone(), path.to_owned()))
                .cloned()
                .ok_or_else(|| ProviderError::new("map", format!("no such path `{path}`")))
        }
    }

    fn root(repo: &str) -> RepoRoot {
        RepoRoot {
            repo: repo.to_owned(),
            root: std::path::PathBuf::from("/nowhere"),
            source: RootSource::RunStepTree,
        }
    }

    #[test]
    fn render_order_is_path_order_not_rank_order() {
        // §4.5 step 9: the rank decides what survives, the path decides where it sits.
        let mut owned = request("nothing\n", &["src/z.rs", "src/a.rs"], &[]);
        owned.roots = vec![root("htui")];
        let reader = MapReader::with(&[
            ("htui", "src/a.rs", "fn a() {}\n"),
            ("htui", "src/z.rs", "fn z() {}\n"),
        ]);
        let set = select(
            &reader,
            &owned.as_request(),
            Vec::new(),
            vec![BUILTIN_ID.to_owned()],
            crate::prompt::TokenEstimator::DEFAULT,
        );
        assert_eq!(set.files.len(), 2);
        // `src/z.rs` is the first declared prefix, so it ranks first...
        assert_eq!(set.files[0].path, "src/a.rs", "the ranker is path-ordered");
        let rendered = crate::prompt::render::excerpts(&set.files).expect("two files");
        let a = rendered.content.find("src/a.rs").expect("a is rendered");
        let z = rendered.content.find("src/z.rs").expect("z is rendered");
        assert!(a < z, "...and the render is `(repo, path)` byte order");
    }

    #[test]
    fn a_file_over_the_line_cap_is_head_windowed_with_the_marker() {
        // §4.5 step 8: whole file at or under `file_line_cap`, otherwise the first `head_lines`
        // with §4.4's elision marker.
        let mut owned = request("nothing\n", &["src/"], &[]);
        owned.roots = vec![root("htui")];
        owned.caps.file_line_cap = 4;
        owned.caps.head_lines = 2;
        let long: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        let reader = MapReader::with(&[
            ("htui", "src/long.rs", &long),
            ("htui", "src/short.rs", "one\ntwo\n"),
        ]);
        let set = select(
            &reader,
            &owned.as_request(),
            Vec::new(),
            vec![BUILTIN_ID.to_owned()],
            crate::prompt::TokenEstimator::DEFAULT,
        );
        let long = set
            .files
            .iter()
            .find(|file| file.path == "src/long.rs")
            .expect("the long file survived");
        assert_eq!((long.first_line, long.last_line), (1, 2));
        assert!(long.truncated);
        assert_eq!(long.elided_lines, 8);
        assert_eq!(long.content, "line 1\nline 2\n");
        // `line 3\n` .. `line 10\n` is eight lines: seven of seven bytes and one of eight, the LF
        // billed per line exactly as `trim.rs` bills a head+tail elision.
        assert_eq!(long.elided_bytes, 7 * 7 + 8);
        let block = crate::prompt::render::file_block(long);
        assert!(
            block.contains(&crate::prompt::render::elision_marker(
                long.elided_lines,
                long.elided_bytes
            )),
            "the marker's two numbers are the record's: {block}"
        );
        let short = set
            .files
            .iter()
            .find(|file| file.path == "src/short.rs")
            .expect("the short file survived");
        assert!(!short.truncated);
        assert_eq!((short.first_line, short.last_line), (1, 2));
    }

    #[test]
    fn no_roots_means_no_section_and_a_no_path_root_per_repo() {
        // ANA-5 §12 criterion 12, and plan D103's preview: an unresolved root is a fact, not a
        // failure.
        let mut owned = request("nothing\n", &["**"], &[]);
        owned.roots = vec![
            RepoRoot {
                repo: "htui".to_owned(),
                root: std::path::PathBuf::new(),
                source: RootSource::NoPath,
            },
            RepoRoot {
                repo: "agy".to_owned(),
                root: std::path::PathBuf::new(),
                source: RootSource::NoPath,
            },
        ];
        let reader = MapReader::default();
        let set = select(
            &reader,
            &owned.as_request(),
            Vec::new(),
            vec![BUILTIN_ID.to_owned()],
            crate::prompt::TokenEstimator::DEFAULT,
        );
        assert!(set.files.is_empty(), "nothing was scanned, so nothing is");
        assert!(crate::prompt::render::excerpts(&set.files).is_none());
        assert_eq!(
            set.audit
                .roots
                .iter()
                .map(|record| (record.repo.as_str(), record.source))
                .collect::<Vec<_>>(),
            vec![("agy", RootSource::NoPath), ("htui", RootSource::NoPath)],
            "one `no_path` record per repo, in repo byte order"
        );
        assert_eq!(set.audit.considered, 0);
        assert_eq!(set.audit.selected, 0);
        assert_eq!(set.audit.provider_set, vec![BUILTIN_ID.to_owned()]);
        assert_eq!(
            set.notes.len(),
            2,
            "each skipped repo says so: {:?}",
            set.notes
        );
    }

    #[test]
    fn sha256_is_over_the_rendered_block() {
        // §4.5 `:1136-1137`: over the excerpt's rendered content bytes, not the whole file, so a
        // reader can prove which bytes the model saw without the record storing them.
        let mut owned = request("nothing\n", &["src/"], &[]);
        owned.roots = vec![root("htui")];
        let body = "fn a() {}\nfn b() {}\n";
        let reader = MapReader::with(&[("htui", "src/a.rs", body)]);
        let set = select(
            &reader,
            &owned.as_request(),
            Vec::new(),
            vec![BUILTIN_ID.to_owned()],
            crate::prompt::TokenEstimator::DEFAULT,
        );
        let excerpt = &set.files[0];
        let block = crate::prompt::render::file_block(excerpt);
        let record = &set.audit.files[0];
        assert_eq!(record.sha256, crate::prompt::digest::sha256_hex(&block));
        assert_eq!(record.bytes, block.len() as u64);
        assert_eq!(record.lines, "1-2");
        assert_ne!(
            record.sha256,
            crate::prompt::digest::sha256_hex(body),
            "the whole file's hash is not the rendered block's"
        );
        assert_eq!(record.sha256.len(), 64);
    }

    #[test]
    fn the_residual_budget_binds_before_max_files() {
        // §4.5 step 7: `max_files`, the per-file caps, or the residual budget — whichever binds.
        let mut owned = request("nothing\n", &["src/"], &[]);
        owned.roots = vec![root("htui")];
        owned.budget_tokens = 40;
        let body: String = (1..=20).map(|n| format!("a line of prose {n}\n")).collect();
        let files: Vec<(&str, String, String)> = (0..6)
            .map(|n| ("htui", format!("src/f{n}.rs"), body.clone()))
            .collect();
        let reader = MapReader::with(
            &files
                .iter()
                .map(|(repo, path, body)| (*repo, path.as_str(), body.as_str()))
                .collect::<Vec<_>>(),
        );
        let set = select(
            &reader,
            &owned.as_request(),
            Vec::new(),
            vec![BUILTIN_ID.to_owned()],
            crate::prompt::TokenEstimator::DEFAULT,
        );
        assert!(
            set.files.len() < 6,
            "the budget bound before `max_files` did"
        );
        assert!(
            set.notes
                .iter()
                .any(|note| note.contains("residual budget")),
            "a cap that bit is recorded: {:?}",
            set.notes
        );
        let est = crate::prompt::TokenEstimator::DEFAULT;
        let spent: i64 = set
            .files
            .iter()
            .map(|file| est.estimate(&crate::prompt::render::file_block(file)))
            .sum();
        assert!(spent <= 40, "{spent} tokens is over the residual budget");
    }

    #[test]
    fn a_provider_candidate_merges_by_repo_and_path_keeping_the_higher_weight() {
        // §4.5 step 6, and rule 3: a provider that changes the excerpt set changes the prompt.
        let mut owned = request("nothing\n", &[], &[]);
        owned.roots = vec![root("htui")];
        let reader = MapReader::with(&[
            ("htui", "src/a.rs", "fn a() {}\n"),
            ("htui", "src/b.rs", "fn b() {}\n"),
        ]);
        let merged = vec![
            ExcerptCandidate {
                repo: "htui".to_owned(),
                path: "src/b.rs".to_owned(),
                lines: None,
                weight: 95,
                reason: "serena".to_owned(),
            },
            // An absolute path from a provider is dropped, never rendered (hazard H-1).
            ExcerptCandidate {
                repo: "htui".to_owned(),
                path: "/etc/passwd".to_owned(),
                lines: None,
                weight: 100,
                reason: "serena".to_owned(),
            },
        ];
        let set = select(
            &reader,
            &owned.as_request(),
            merged,
            vec![BUILTIN_ID.to_owned(), "serena@0.1".to_owned()],
            crate::prompt::TokenEstimator::DEFAULT,
        );
        let b = set
            .files
            .iter()
            .find(|file| file.path == "src/b.rs")
            .expect("the provider's candidate was taken");
        assert_eq!(b.weight, 95);
        assert_eq!(b.reason, ExcerptReason::Provider);
        assert_eq!(b.provider.as_deref(), Some("serena"));
        assert_eq!(b.rank, 1, "95 outranks the lexical tier");
        assert!(
            crate::prompt::render::file_block(b).contains("reason=\"provider:serena\""),
            "§4.5 `:1114` renders a provider's reason qualified"
        );
        assert!(
            set.files.iter().all(|file| !file.path.starts_with('/')),
            "an absolute path never reaches a rendered byte"
        );
        assert!(
            set.notes.iter().any(|note| note.contains("/etc/passwd")),
            "and its exclusion is recorded: {:?}",
            set.notes
        );
    }

    #[test]
    fn the_builtin_is_registered_as_builtin_at_1() {
        let builtin = BuiltinRanker;
        assert_eq!(builtin.name(), "builtin");
        assert_eq!(builtin.version(), "1");
        assert_eq!(BUILTIN_ID, "builtin@1");
    }

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
