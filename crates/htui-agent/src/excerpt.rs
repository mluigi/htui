//! The filesystem and threading half of ANA-5 §4.5 (`docs/ANA-5.md:990-1211`).
//!
//! `htui-core` owns the excerpt *values*, the seam and the pure ranker, and names no `std::fs` at
//! all (§4.8 `:1458-1477`). Everything that touches a disk or a thread lives here: [`FsRepoReader`]
//! walks a repository root, and [`run_providers`] gives each [`ExcerptProvider`] a thread and a
//! deadline. The split is what lets every tier, denylist and windowing case in
//! [`htui_core::prompt::excerpt`] be a unit test over a double.
//!
//! Both halves are **read-only by type** (ANA-5 invariant 6, `R-ID-4`): [`RepoReader`] has `list`
//! and `read` and nothing else, and `htui` writes no file into a managed repository.
//!
//! Read-only is not the same as safe, and [`FsRepoReader::read`] does **not** trust its caller:
//! `htui_core::prompt::excerpt::select` hands it *provider* candidates, which never went through
//! the walk. So `read` re-runs the rules the listing ran — it descends one component at a time and
//! refuses a symlink at any of them, refuses anything over `max_file_bytes` before it allocates,
//! and refuses a NUL in the first [`BINARY_PROBE_BYTES`] — because a repo-relative path that
//! reaches a symlink is exactly how hazard H-1 arrives through a filesystem, and the listing alone
//! was never the gate it looked like.
//!
//! No new dependency (§4.5 `:1007`): the walk is `std::fs` plus a hand matcher, in the shape
//! [`crate::probe`]'s glob walker already established, and the `.gitignore` matcher is a declared
//! **subset** rather than a glob crate.
//!
//! MOD-7 milestone 4 wires the pass. [`excerpts_for`] is the one function both the engine's phase
//! prompt and the Backlog preview call, so the bytes a maintainer previews and the bytes a run sends
//! cannot drift (MOD-2 D103).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use tracing::warn;

use htui_core::model::{BoxId, Repo, RepoBoxPath, RepoId, RunStepTree};
use htui_core::prompt::excerpt::{
    BUILTIN_ID, BuiltinRanker, ExcerptAudit, ExcerptCandidate, ExcerptCaps, ExcerptProvider,
    ExcerptRequest, ExcerptSet, OwnedExcerptRequest, PathPrefix, ProviderError, RepoReader,
    RepoRoot, RootRecord, RootSource, select, skip_by_path,
};
use htui_core::prompt::settings::resolve_excerpt_caps;
use htui_core::prompt::{
    Placeholder, PromptSpec, TokenEstimator, drop_unmaskable_excerpts, excerpt_residual, parse,
    withhold_unmaskable_notes,
};
use htui_core::scrub::Scrubber;

/// How many bytes of a file the binary test reads (§4.5 `:1073`).
pub const BINARY_PROBE_BYTES: usize = 8_192;

/// The highest weight a provider may claim; `htui` re-normalises against its own tiers
/// (§4.5 `:1164`).
///
/// Re-exported from `htui-core` rather than spelled again here. The ceiling is a property of
/// [`ExcerptCandidate`], which that crate owns, and
/// [`select`] clamps to it as well — two independent `100`s
/// were how finding F-100 arrived, with the invariant enforced only by the impure crate.
pub use htui_core::prompt::excerpt::MAX_PROVIDER_WEIGHT;

/// §4.5's six skip rules (`:1066-1075`), in the order they are evaluated.
///
/// The order is a stated fact rather than an implementation detail: a file that is both a secret
/// and gitignored is excluded *as a secret*, and a maintainer reading a note wants the rule that
/// actually fired. [`ORDER`](Self::ORDER) is what pins it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipRule {
    /// The path is inside `.git/`. Never source, and pruned at the directory.
    Git,
    /// The path matches the secret denylist (`:1071`). A selection rule, not a scrubbing rule.
    SecretDenylist,
    /// A `.gitignore` entry matched, under the declared subset [`GitignoreSubset`] implements.
    Gitignored,
    /// The first [`BINARY_PROBE_BYTES`] contain a NUL.
    Binary,
    /// The file is larger than `max_file_bytes`.
    TooLarge,
    /// A lockfile or a minified asset (`:1075`).
    LockfileOrMinified,
}

impl SkipRule {
    /// The six rules in §4.5's own evaluation order.
    pub const ORDER: [Self; 6] = [
        Self::Git,
        Self::SecretDenylist,
        Self::Gitignored,
        Self::Binary,
        Self::TooLarge,
        Self::LockfileOrMinified,
    ];

    /// The rule's name, sharing the spelling
    /// [`htui_core::prompt::excerpt::skip_by_path`] returns for the three path-only rules.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::SecretDenylist => "secret_denylist",
            Self::Gitignored => "gitignored",
            Self::Binary => "binary",
            Self::TooLarge => "too_large",
            Self::LockfileOrMinified => "lockfile_or_minified",
        }
    }
}

/// One `.gitignore` line this matcher understands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IgnoreRule {
    /// `*.ext` — matched against the file name's tail.
    Suffix(String),
    /// A literal path relative to the file's own directory: `/build`, or `a/b`.
    Anchored(String),
    /// A bare name matched against any path component: `target`, `node_modules`.
    Name(String),
}

/// The declared `.gitignore` **subset** of blueprint B.7 — not a glob crate, and not git.
///
/// Understood: comments and blank lines; a trailing `/` (directory-only, honoured as "this name and
/// everything under it"); a leading `/` or an embedded `/` (anchored to the file's own directory);
/// `*.ext` suffixes; bare names matched against any path component. **Ignored: `!` negations**, and
/// every other glob form. The subset is declared rather than approximated because a partial glob
/// engine that silently disagreed with git would make the selected set a surprise; the failure
/// direction here is "a build artefact is excerpted", which costs tokens and leaks nothing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GitignoreSubset {
    rules: Vec<IgnoreRule>,
}

impl GitignoreSubset {
    /// Parse one `.gitignore` file's text.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            // `!` is ignored, not honoured: re-including a path needs git's full ordering
            // semantics, and half of them is worse than none.
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                continue;
            }
            let pattern = line.trim_end_matches('/');
            if pattern.is_empty() {
                continue;
            }
            if let Some(ext) = pattern.strip_prefix("*.") {
                rules.push(IgnoreRule::Suffix(format!(".{ext}")));
            } else if let Some(anchored) = pattern.strip_prefix('/') {
                rules.push(IgnoreRule::Anchored(anchored.to_owned()));
            } else if pattern.contains('/') {
                rules.push(IgnoreRule::Anchored(pattern.to_owned()));
            } else if pattern.contains('*') {
                // Any other wildcard form is outside the subset and is not guessed at.
            } else {
                rules.push(IgnoreRule::Name(pattern.to_owned()));
            }
        }
        Self { rules }
    }

    /// Whether `relative` — a path relative to the directory this file sits in — is ignored.
    #[must_use]
    pub fn matches(&self, relative: &str) -> bool {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        self.rules.iter().any(|rule| match rule {
            IgnoreRule::Suffix(ext) => name.ends_with(ext.as_str()),
            IgnoreRule::Anchored(path) => {
                relative == path || relative.starts_with(&format!("{path}/"))
            }
            IgnoreRule::Name(component) => relative.split('/').any(|segment| segment == component),
        })
    }

    /// Whether this matcher has anything to say at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// §4.5's repository walk: `std::fs`, depth-first, sorted by name at every level.
///
/// Deterministic by construction — every `read_dir` is sorted on the entry's **file-name bytes**
/// before it is descended, because `read_dir` order is the filesystem's and two boxes would
/// otherwise list the same tree differently and assemble different prompts (invariant 2).
///
/// The size limit is carried on the reader rather than passed to `list`, because
/// [`RepoReader::list`] takes only the scan cap and §4.5's fifth skip rule needs the size limit at
/// the same moment the file is stat-ed.
///
/// It is carried as the whole [`ExcerptCaps`] rather than as a loose `u64`, which is review finding
/// F-101. There are two enforcements of `max_file_bytes` and there always will be: this reader has
/// to refuse a file *before* it allocates it, and `select` re-measures what it gets back — "a
/// reader is a public seam and the two defences are deliberately independent"
/// ([`RepoReader::read`]). What went wrong was that nothing tied the two **numbers**: a reader built
/// with a limit below `caps.max_file_bytes` bound first and silently, because the walk simply never
/// listed the file, and `audit.caps` went on naming the configured cap — one that had stopped
/// nothing.
///
/// Taking an [`ExcerptCaps`] means the one `resolve_excerpt_caps` call that mints the caps the pass
/// records is the same call that mints the reader's limit; there is no second number to get wrong.
/// [`RepoReader::max_file_bytes`] then reports it, so `select` records the cap that actually bound
/// even for a reader built some other way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsRepoReader {
    /// The caps this reader was resolved under. Only `max_file_bytes` is a rule of the walk's
    /// (skip rule 5); the rest are the pass's and are carried so the two cannot be resolved apart.
    pub caps: ExcerptCaps,
}

impl Default for FsRepoReader {
    /// The reader for the caps an empty `app_setting` table resolves to — from
    /// `resolve_excerpt_caps`, not from a second reading of [`DEFAULTS`], so even the default is
    /// the one resolver's answer (F-101).
    ///
    /// [`DEFAULTS`]: htui_core::prompt::settings::DEFAULTS
    fn default() -> Self {
        Self::new(resolve_excerpt_caps(&BTreeMap::new()).0)
    }
}

impl FsRepoReader {
    /// A reader for the caps `settings::resolve_excerpt_caps` resolved.
    ///
    /// The **same** value must reach `ExcerptRequest::caps`; that is the whole point of the
    /// argument being an [`ExcerptCaps`] rather than a number (F-101).
    #[must_use]
    pub const fn new(caps: ExcerptCaps) -> Self {
        Self { caps }
    }

    /// One directory level: sorted entries, the nested `.gitignore` read once, then recursion.
    fn walk(
        &self,
        base: &Path,
        relative: &str,
        ignores: &mut Vec<(String, GitignoreSubset)>,
        out: &mut Vec<String>,
        examined: &mut u32,
        cap: u32,
    ) -> Result<bool, ProviderError> {
        let dir = if relative.is_empty() {
            base.to_path_buf()
        } else {
            base.join(relative)
        };
        let mut names: Vec<std::ffi::OsString> = std::fs::read_dir(&dir)
            .map_err(|error| ProviderError::new("fs", format!("read_dir: {error}")))?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        // Byte order at every level (§4.5 step 2), on the raw name rather than on a lossy string.
        names.sort_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));

        // The directory's own `.gitignore`, read once and scoped to this subtree.
        let pushed = match std::fs::read_to_string(dir.join(".gitignore")) {
            Ok(text) => {
                let rules = GitignoreSubset::parse(&text);
                let empty = rules.is_empty();
                if !empty {
                    ignores.push((relative.to_owned(), rules));
                }
                !empty
            }
            Err(_) => false,
        };

        let mut truncated = false;
        for name in names {
            let Some(name) = name.to_str() else {
                // A name that is not UTF-8 cannot be a repo-relative path in a rendered byte, so
                // it is skipped rather than lossily renamed.
                continue;
            };
            // Rule 1, above the `is_dir` branch and above the stat: `.git` is never source,
            // whatever kind of entry it is. A **submodule's** `sub/.git` is a file holding
            // `gitdir: /abs/path`, and a prune that ran only on the directory branch listed it and
            // read it — an absolute path in a rendered byte, which is hazard H-1.
            // `skip_by_path` refuses the same name at the same breadth, so neither crate is the
            // only one holding rule 1 (H-22).
            if name == ".git" {
                continue;
            }
            let child = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            // The cap is charged **here**, for every entry and before it is stat-ed, because the
            // cap bounds the walk's *cost* and a `read_dir` plus a `.gitignore` open per level is
            // what a directory costs. Counting only regular files let a tree of nested directories
            // walk unbounded while presenting nothing to count. Charging skipped entries too keeps
            // the truncated listing a **prefix** of the untruncated one, which is what makes
            // `scan_truncated` mean "there is more below this".
            *examined += 1;
            if *examined > cap {
                truncated = true;
                break;
            }
            let path = base.join(&child);
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_symlink() {
                // A symlink can leave the root, and a root the walk left is an absolute path in a
                // rendered byte waiting to happen (hazard H-1). Not followed, ever.
                continue;
            }
            if meta.is_dir() {
                if self.skip(&child, ignores, None).is_some() {
                    continue;
                }
                // A subdirectory this box cannot open contributes nothing and does **not** fail the
                // repo: §4.5 step 1's fail-open applies below the root as well as at it, and one
                // unreadable directory losing a whole repo's excerpts would be a permission bit
                // silently changing a prompt. Only the root itself is an error, because a root
                // that cannot be read is the "resolve a readable root" step failing.
                match self.walk(base, &child, ignores, out, examined, cap) {
                    Ok(true) => {
                        truncated = true;
                        break;
                    }
                    Ok(false) | Err(_) => continue,
                }
            }
            if !meta.is_file() {
                continue;
            }
            if self.skip(&child, ignores, Some(&path)).is_none() {
                out.push(child);
            }
        }
        if pushed {
            ignores.pop();
        }
        Ok(truncated)
    }

    /// The six rules of [`SkipRule::ORDER`], evaluated in that order.
    ///
    /// `path` is `None` for a directory, which can only fall to rules 1–3.
    fn skip(
        &self,
        relative: &str,
        ignores: &[(String, GitignoreSubset)],
        path: Option<&Path>,
    ) -> Option<SkipRule> {
        // Rules 1, 2 and 6 are `htui-core`'s, so the ranker and the reader cannot disagree (H-22).
        // They are re-ordered here rather than taken as one verdict, because §4.5 evaluates the
        // gitignore, binary and size rules *between* the denylist and the lockfile rule.
        match skip_by_path(relative) {
            Some("git") => return Some(SkipRule::Git),
            Some("secret_denylist") => return Some(SkipRule::SecretDenylist),
            _ => {}
        }
        for (base, rules) in ignores {
            let scoped = if base.is_empty() {
                relative
            } else {
                match relative.strip_prefix(&format!("{base}/")) {
                    Some(scoped) => scoped,
                    None => continue,
                }
            };
            if rules.matches(scoped) {
                return Some(SkipRule::Gitignored);
            }
        }
        let path = path?;
        // Rules 4 and 5 from **one** open descriptor. Two `stat`s and a separate `File::open` used
        // to answer them, so a name swapped between the walk's `symlink_metadata` and either of
        // them was decided about as one file and read as another; `open_regular` refuses that.
        // An unreadable file, and a name that changed underneath, both read as binary.
        let Some((mut file, meta)) = open_regular(path) else {
            return Some(SkipRule::Binary);
        };
        match probe(&mut file) {
            Ok(head) if head.contains(&0) => return Some(SkipRule::Binary),
            Err(_) => return Some(SkipRule::Binary),
            Ok(_) => {}
        }
        if meta.len() > self.caps.max_file_bytes {
            return Some(SkipRule::TooLarge);
        }
        if skip_by_path(relative) == Some("lockfile_or_minified") {
            return Some(SkipRule::LockfileOrMinified);
        }
        None
    }
}

/// The real path a repo-relative string names under `root`, with **no symlink at any component**.
///
/// `std::fs::read(root.join(path))` follows a symlink at *every* component, so the walk's "symlinks
/// are never followed" protected only the listing. A provider proposing `docs/notes.md` where that
/// is a link to `~/.ssh/id_rsa`, or `vendor/x.rs` where `vendor` is a link to `/`, would put bytes
/// from outside the repository into a prompt under a repo-relative path — hazard H-1, arriving
/// through the filesystem rather than through a rendered string. So the descent is one component at
/// a time with a `symlink_metadata` at each, the shape [`crate::install::archive`]'s own `descend`
/// already established in this crate.
///
/// A link is refused rather than resolved-and-compared: where it points can change between the
/// check and the open, and "inside the root" is not a property this reader can hold still.
///
/// The **root itself** is deliberately not checked. It is a maintainer-configured path rather than
/// repository content, `list` reads through it the same way, and `/tmp` is a symlink on more than
/// one box — refusing it would refuse a legitimate checkout without closing anything.
fn descend(root: &Path, path: &str) -> Result<PathBuf, ProviderError> {
    let bytes = path.as_bytes();
    let rooted = bytes.first().is_some_and(|c| *c == b'/' || *c == b'\\');
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if path.is_empty() || rooted || drive {
        return Err(not_repo_relative(path));
    }
    // Every segment is judged **before** the first `stat`, so the refusal names the path's own
    // shape rather than whichever component happened not to exist. An empty, `.` or `..` segment
    // is refused rather than normalised: the caller named a path it cannot have listed, and
    // normalising one is how a `..` survives a textual check.
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(not_repo_relative(path));
    }
    let mut at = root.to_path_buf();
    for segment in path.split('/') {
        at.push(segment);
        let Ok(meta) = std::fs::symlink_metadata(&at) else {
            return Err(ProviderError::new(
                "fs",
                format!("`{path}` does not exist under this root"),
            ));
        };
        if meta.is_symlink() {
            return Err(ProviderError::new(
                "fs",
                // The link's target is never named: it is an absolute path outside the root, and
                // this message becomes a note on the excerpt set.
                format!("`{path}` is reached through a symlink, which is never followed"),
            ));
        }
    }
    Ok(at)
}

/// The one spelling of §4.2 rule 5's refusal, so every caller says it identically.
fn not_repo_relative(path: &str) -> ProviderError {
    ProviderError::new("fs", format!("`{path}` is not repo-relative"))
}

/// An `io::Error` from a read, as a `ProviderError`. `std` names no path in its `Display`, and this
/// message ends up in `ExcerptSet::notes`, so none is added.
fn read_failed(error: std::io::Error) -> ProviderError {
    ProviderError::new("fs", format!("read: {error}"))
}

/// Open `path` for reading, refusing anything that is not the regular file just stat-ed.
///
/// Returns the handle **and** the metadata taken from the descriptor, so the size, the binary probe
/// and the bytes are all answered by one open file. That is what closes the `symlink_metadata` →
/// `metadata` / `File::open` window: a name swapped for a symlink between the two used to be
/// stat-ed as a file and opened as its target.
fn open_regular(path: &Path) -> Option<(std::fs::File, std::fs::Metadata)> {
    let before = std::fs::symlink_metadata(path).ok()?;
    if before.is_symlink() || !before.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let after = file.metadata().ok()?;
    if !after.is_file() || !is_same_file(&before, &after) {
        // The name was swapped between the stat and the open. Whatever is behind it now is not
        // what was decided about, so it is refused rather than read.
        return None;
    }
    Some((file, after))
}

/// Whether two [`std::fs::Metadata`] describe the same file, for [`open_regular`]'s re-check.
#[cfg(unix)]
fn is_same_file(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    before.dev() == after.dev() && before.ino() == after.ino()
}

/// Elsewhere there is no `std` equivalent of `(dev, ino)`. The `symlink_metadata` refusal above
/// stands on its own; only the race window between it and the open stays open.
#[cfg(not(unix))]
const fn is_same_file(_before: &std::fs::Metadata, _after: &std::fs::Metadata) -> bool {
    true
}

/// The first [`BINARY_PROBE_BYTES`] of an already-open file, for §4.5's binary test (a NUL in
/// them).
///
/// Loops to the cap or to EOF rather than trusting one `read` to fill the buffer: a short read is
/// not an end of file, and a single `read` that returned sixteen bytes used to declare a whole file
/// NUL-free. The bytes are returned rather than a verdict because [`FsRepoReader::read`] needs them
/// anyway and re-reading them from the same handle is not possible.
fn probe(file: &mut std::fs::File) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut head = Vec::with_capacity(BINARY_PROBE_BYTES);
    file.by_ref()
        .take(BINARY_PROBE_BYTES as u64)
        .read_to_end(&mut head)?;
    Ok(head)
}

impl RepoReader for FsRepoReader {
    /// The limit skip rule 5 and [`Self::read`] enforce, so `select` records the cap that actually
    /// bound rather than one it merely asked for (review finding F-101).
    ///
    /// `caps.max_file_bytes` and nothing else: a reader built by [`FsRepoReader::new`] from the
    /// caller's own `resolve_excerpt_caps` result reports exactly the number that call resolved, so
    /// the reconciliation in `select` is a no-op and `audit.caps` is the caps verbatim. It stops
    /// being a no-op only when a caller resolved the two apart, which is the case the finding
    /// describes and the one this method makes visible.
    fn max_file_bytes(&self) -> u64 {
        self.caps.max_file_bytes
    }

    /// The cap counts every **entry** the walk examines — files, directories and skipped ones
    /// alike — because the cap exists to bound the walk's cost, a stat of a lockfile costs what a
    /// stat of a source file costs, and a directory costs a `read_dir` and a `.gitignore` open on
    /// top. A cap that counted only regular files walked a tree of nested empty directories
    /// unbounded, which is what `the_scan_cap_counts_directories_too` pins.
    ///
    /// A truncated listing is therefore still always a prefix of the untruncated one, never a
    /// different set — which is what makes `scan_truncated` mean "there is more below this", and
    /// what `scan_cap_sets_truncated` pins.
    fn list(&self, root: &RepoRoot, cap: u32) -> Result<(Vec<String>, bool), ProviderError> {
        let mut out = Vec::new();
        let mut ignores = Vec::new();
        let mut examined = 0u32;
        let truncated = self.walk(&root.root, "", &mut ignores, &mut out, &mut examined, cap)?;
        Ok((out, truncated))
    }

    /// `read` re-runs the rules the listing ran, because its caller is not the listing.
    ///
    /// `select` reaches this method with **provider** candidates, which never went through
    /// [`Self::list`]. A textual `..`-and-leading-`/` test was therefore the only thing between a
    /// proposed path and `std::fs::read`, which follows a symlink at every component, allocates the
    /// whole file before anything measures it, and refuses only non-UTF-8 — and a NUL is UTF-8. So
    /// the order here is §4.5's: repo-relative, then no symlink at any component (rule 1's spirit),
    /// then `max_file_bytes` (rule 5) and the NUL probe (rule 4) **before** the bytes are taken.
    fn read(&self, root: &RepoRoot, path: &str) -> Result<String, ProviderError> {
        use std::io::Read as _;

        let full = descend(&root.root, path)?;
        let Some((mut file, meta)) = open_regular(&full) else {
            return Err(ProviderError::new(
                "fs",
                format!("`{path}` is not a readable regular file"),
            ));
        };
        // Rule 5 before the allocation, not after it: `select` measured `text.len()` only once the
        // file was already in memory, so a multi-gigabyte file cost its own size to refuse.
        if meta.len() > self.caps.max_file_bytes {
            return Err(ProviderError::new(
                "fs",
                format!(
                    "`{path}` is {} bytes, over max_file_bytes ({})",
                    meta.len(),
                    self.caps.max_file_bytes
                ),
            ));
        }
        // Rule 4, from the same handle, before the rest of the file is read.
        let mut bytes = probe(&mut file).map_err(read_failed)?;
        if bytes.contains(&0) {
            return Err(ProviderError::new(
                "fs",
                format!("`{path}` has a NUL in its first {BINARY_PROBE_BYTES} bytes; binary"),
            ));
        }
        // `meta.len()` is a promise from before the read, so the read is bounded again by one byte
        // past the cap: a file that grew under the reader is refused, never quietly truncated into
        // a prompt whose digest would then describe bytes nobody chose.
        let room = self.caps.max_file_bytes.saturating_sub(bytes.len() as u64) + 1;
        file.take(room)
            .read_to_end(&mut bytes)
            .map_err(read_failed)?;
        if bytes.len() as u64 > self.caps.max_file_bytes {
            return Err(ProviderError::new(
                "fs",
                format!(
                    "`{path}` grew past max_file_bytes ({}) while it was read",
                    self.caps.max_file_bytes
                ),
            ));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| ProviderError::new("fs", format!("`{path}` is not UTF-8")))?;
        Ok(normalise(&text))
    }
}

/// `\r\n` and lone `\r` become `\n`; a leading U+FEFF is dropped.
///
/// The same normalisation `htui_core::prompt::render` applies, done here as well because the
/// reader's contract says LF-normalised and hazard H-10 is a CRLF checkout changing `elided_bytes`
/// and therefore the digest. Duplicated rather than exported: it is six lines, and a `pub` helper
/// in the pure crate for the benefit of the impure one would invert the dependency §4.8 draws.
fn normalise(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// What one provider reported back.
enum Outcome {
    Proposed(Vec<ExcerptCandidate>),
    Failed(ProviderError),
    Panicked,
}

/// What [`run_providers`] names the thread it gives a provider, followed by the provider's name.
///
/// A convention a backtrace can be read against: an unwind line from `excerpt-provider:serena` says
/// whose defect it is without the reader having to know this function exists.
pub const PROVIDER_THREAD_PREFIX: &str = "excerpt-provider:";

thread_local! {
    /// Whether this thread is inside a [`propose_caught`] window right now.
    static CONTAINED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a panic on **this** thread, at this instant, is one [`run_providers`] is going to catch.
///
/// Review finding M1, and the one question a process-wide panic hook cannot answer for itself:
/// [`std::panic::catch_unwind`] does not stop the hook, and the hook runs on the panicking thread
/// before the unwind reaches the `catch_unwind` that swallows it. So `htui`'s hook — which leaves
/// the alternate screen and disables raw mode — used to fire for a provider panic that the
/// assembler had already decided to survive, tearing the terminal down under a running event loop.
///
/// A thread-local rather than a [`PROVIDER_THREAD_PREFIX`] name test, because since finding M2
/// `providers[0]` runs on the **caller's** thread, and a name test would miss exactly that one.
/// Both are set: the flag is the mechanism, the name is the diagnostic.
///
/// False on every thread that is not inside a provider call, which is every thread in the process
/// for all of a normal run. A panic the process does not survive still gives the terminal back.
#[must_use]
pub fn panic_is_contained() -> bool {
    CONTAINED.with(std::cell::Cell::get)
}

/// Sets [`CONTAINED`] for the lifetime of a provider call, restoring the **previous** value rather
/// than clearing it, so a nested `run_providers` cannot un-contain its caller's window.
struct Contained(bool);

impl Contained {
    fn enter() -> Self {
        Self(CONTAINED.with(|flag| flag.replace(true)))
    }
}

impl Drop for Contained {
    fn drop(&mut self) {
        CONTAINED.with(|flag| flag.set(self.0));
    }
}

/// One `propose` call with its unwind caught, shared by the inline provider and the spawned ones.
///
/// Inline too, and not only on a thread: `providers[0]` runs on the caller's thread since finding
/// M2, so an unwind there would take the assembler down instead of being "dropped and recorded"
/// (hazard H-20).
///
/// The [`Contained`] guard spans the `catch_unwind` and not just the `propose`, because the hook
/// fires *during* the unwind — `panic_is_contained` has to still be true when it is asked.
fn propose_caught(provider: &dyn ExcerptProvider, req: &ExcerptRequest<'_>) -> Outcome {
    let _contained = Contained::enter();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.propose(req))) {
        Ok(Ok(candidates)) => Outcome::Proposed(candidates),
        Ok(Err(error)) => Outcome::Failed(error),
        Err(_) => Outcome::Panicked,
    }
}

/// Run every provider concurrently under one wall-clock deadline (§4.5 rule 2).
///
/// Returns the merged candidate list and `provider_set` in blueprint P-11's grammar:
/// `name@version` when the provider answered, `name@version:error`, `:panic` or `:timeout` when it
/// did not. Registration order is preserved, so a caller that puts
/// [`BuiltinRanker`] first gets `builtin@1` first —
/// and the built-in is never dropped because its `propose` is infallible and returns at once.
///
/// **Hazard H-20, stated rather than hidden**: a provider past the deadline is *dropped*, not
/// joined, so its thread may outlive this call and its result is discarded when it finally
/// arrives. That is what "fail-open with a deadline" costs, and it is why the request is cloned
/// into an [`OwnedExcerptRequest`] rather than borrowed through a scoped thread — a scope would
/// block on exactly the provider the deadline exists to abandon. A provider is a lookup by
/// contract (`R-ID-6`: deterministic code, never a model call), so an unbounded one is a defect in
/// the provider.
///
/// Every returned candidate's `reason` is rewritten to the provider's own name and its weight
/// clamped to [`MAX_PROVIDER_WEIGHT`]: a provider may propose, and it may not claim a tier — or
/// another provider's name — by spelling one.
///
/// **`providers[0]` runs on the caller's thread and is not under the deadline** (review finding
/// M2). The paragraph above says the built-in is never dropped; running it on a spawned thread
/// under the same `recv_timeout` made that false, because `excerpt_provider_deadline_ms` accepts
/// any positive integer and a thread spawn plus a scheduling slice exceeds `1`. A `builtin@1` that
/// intermittently became `builtin@1:timeout` is the same inputs producing two different records —
/// the determinism invariant, broken by a setting an operator is allowed to write. The others are
/// spawned **first**, so they still overlap with it and the concurrency the deadline exists for is
/// unchanged.
#[must_use]
pub fn run_providers(
    providers: &[Arc<dyn ExcerptProvider>],
    req: &ExcerptRequest<'_>,
) -> (Vec<ExcerptCandidate>, Vec<String>) {
    let mut outcomes: Vec<Option<Outcome>> = (0..providers.len()).map(|_| None).collect();
    let (sender, receiver) = mpsc::channel::<(usize, Outcome)>();
    // The clone is paid for only when there is a thread to hand it to; the inline provider reads
    // the caller's own borrowed request.
    let mut deadlined = providers.len().saturating_sub(1);
    if deadlined > 0 {
        let owned = Arc::new(OwnedExcerptRequest::from_request(req));
        for (index, provider) in providers.iter().enumerate().skip(1) {
            let name = format!("{PROVIDER_THREAD_PREFIX}{}", provider.name());
            let provider = Arc::clone(provider);
            let request = Arc::clone(&owned);
            let sender = sender.clone();
            // Named so an unwind line says whose defect it is, and `Builder` rather than
            // `thread::spawn` so a refused spawn is this provider's `:error` rather than a panic
            // on the assembler's own thread.
            let spawned = std::thread::Builder::new().name(name).spawn(move || {
                // Detached on purpose: see H-20 above.
                let outcome = propose_caught(provider.as_ref(), &request.as_request());
                // The receiver may already have given up; that is the timeout case, not an error.
                let _ = sender.send((index, outcome));
            });
            if let Err(error) = spawned {
                outcomes[index] = Some(Outcome::Failed(ProviderError::new(
                    "excerpt",
                    format!("no thread for this provider: {error}"),
                )));
                deadlined -= 1;
            }
        }
    }
    drop(sender);

    // One wall clock over the whole concurrent set, started where the threads were: the inline
    // provider runs inside that window rather than beside it, so the call is still bounded and
    // `deadline` still means what §4.5 rule 2 says it means.
    let started = Instant::now();
    if let Some(first) = providers.first() {
        outcomes[0] = Some(propose_caught(first.as_ref(), req));
    }
    let mut pending = deadlined;
    while pending > 0 {
        let Some(left) = req.deadline.checked_sub(started.elapsed()) else {
            break;
        };
        match receiver.recv_timeout(left) {
            Ok((index, outcome)) => {
                outcomes[index] = Some(outcome);
                pending -= 1;
            }
            Err(_) => break,
        }
    }

    let mut merged = Vec::new();
    let mut provider_set = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for (provider, outcome) in providers.iter().zip(outcomes) {
        let id = format!("{}@{}", provider.name(), provider.version());
        match outcome {
            Some(Outcome::Proposed(candidates)) => {
                for candidate in candidates {
                    if candidate.repo.is_empty()
                        || candidate.path.is_empty()
                        || !seen.insert((candidate.repo.clone(), candidate.path.clone()))
                    {
                        continue;
                    }
                    merged.push(ExcerptCandidate {
                        weight: candidate.weight.min(MAX_PROVIDER_WEIGHT),
                        reason: provider.name().to_owned(),
                        ..candidate
                    });
                }
                provider_set.push(id);
            }
            Some(Outcome::Failed(error)) => {
                warn!(
                    provider = provider.name(),
                    message = %error.message,
                    "an excerpt provider declined; dropped and recorded"
                );
                provider_set.push(format!("{id}:error"));
            }
            Some(Outcome::Panicked) => {
                warn!(
                    provider = provider.name(),
                    "an excerpt provider panicked; dropped and recorded"
                );
                provider_set.push(format!("{id}:panic"));
            }
            None => {
                warn!(
                    provider = provider.name(),
                    deadline_ms = req.deadline.as_millis(),
                    "an excerpt provider missed its deadline; dropped and recorded"
                );
                provider_set.push(format!("{id}:timeout"));
            }
        }
    }
    (merged, provider_set)
}

/// ANA-5 §4.5 step 1 (plan D107): one root per scope repo, by **row presence**.
///
/// The step's `run_step_tree` row for the repo (`RootSource::RunStepTree`), else this box's
/// `repo_box_path` row (`RootSource::RepoBoxPath`; rows of other boxes are ignored), else
/// `RootSource::NoPath` with an empty root. `repo` is the repo's name, the slug a rendered
/// `path="repo:…"` and `PathPrefix` use. No `stat`: an unreadable root is `select`'s "could not
/// be listed" note, which keeps this pure. In `scope` order.
#[must_use]
pub fn excerpt_roots(
    scope: &[(RepoId, String)],
    trees: &[RunStepTree],
    paths: &[RepoBoxPath],
    box_id: BoxId,
) -> Vec<RepoRoot> {
    scope
        .iter()
        .map(|(id, name)| {
            let (root, source) = if let Some(tree) = trees.iter().find(|tree| tree.repo_id == *id) {
                (PathBuf::from(&tree.path), RootSource::RunStepTree)
            } else if let Some(path) = paths
                .iter()
                .find(|path| path.repo_id == *id && path.box_id == box_id)
            {
                (PathBuf::from(&path.local_path), RootSource::RepoBoxPath)
            } else {
                (PathBuf::new(), RootSource::NoPath)
            };
            RepoRoot {
                repo: name.clone(),
                root,
                source,
            }
        })
        .collect()
}

/// `item.touched_paths` as §4.5's tier-1 prefixes, under `overlap::resolve`'s primary rule
/// (`crates/htui-orch/src/overlap.rs:43-54`, plan D119): a bare glob belongs to the project's
/// `is_primary` repo, and to the empty slug when there is none, which matches no repo.
#[must_use]
pub fn touched_prefixes(touched: &[String], repos: &[Repo]) -> Vec<PathPrefix> {
    let primary = repos
        .iter()
        .find(|repo| repo.is_primary)
        .map_or("", |repo| repo.name.as_str());
    touched
        .iter()
        .map(|glob| PathPrefix::parse(glob, primary))
        .collect()
}

/// What the shared pass needs beside the spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassInput {
    /// `excerpt_roots`' answer.
    pub roots: Vec<RepoRoot>,
    /// `touched_prefixes`' answer.
    pub touched_prefixes: Vec<PathPrefix>,
    /// The caller's own notes about the roots, which go first: a scope repo with no row, or
    /// D108's "no `run_step_tree` row yet".
    pub notes: Vec<String>,
}

/// §4.5 over the filesystem (plan D119): the built-in provider, then `select` over an
/// `FsRepoReader` built from the request's own caps (F-101). **Blocking**: `excerpts_for` calls it
/// under `spawn_blocking` whenever a root is readable.
#[must_use]
pub fn excerpt_pass(req: &OwnedExcerptRequest, est: TokenEstimator) -> ExcerptSet {
    let providers: Vec<Arc<dyn ExcerptProvider>> = vec![Arc::new(BuiltinRanker)];
    let request = req.as_request();
    let (merged, provider_set) = run_providers(&providers, &request);
    select(
        &FsRepoReader::new(req.caps),
        &request,
        merged,
        provider_set,
        est,
    )
}

/// The excerpt set for `spec` (plan D109, D118, D119; MOD-7 milestone 4 D126, D128).
///
/// 1. `resolve_excerpt_caps(app)`.
/// 2. `spec.body` parsed in `spec.role` does not place `{{excerpts}}` (or does not parse): no
///    pass. The roots are recorded unscanned, with the note
///    ``excerpt: template `name` places no {{excerpts}}; nothing was read``.
/// 3. No readable root (none, or all `NoPath`): [`excerpt_pass`] **inline**, with a zero budget.
///    It reads nothing and spawns nothing (H-9), and records each `NoPath` with `select`'s own "no
///    readable root" note.
/// 4. Otherwise the budget is `excerpt_residual(spec, scrubber)`. An `Err` records the roots
///    unscanned, because `assemble` will refuse the same way. Then [`excerpt_pass`] runs under
///    `tokio::task::spawn_blocking`; a `JoinError` records the roots unscanned with
///    `excerpt: the pass panicked; no excerpts`, or `excerpt: the pass was cancelled; no excerpts`
///    when the task never ran (§4.5 fail-open).
/// 5. `withhold_unmaskable_notes(&mut set.notes, scrubber)`: a pass note names `repo:path` as the
///    reader returned it, and `trim_record.notes` is persisted unscrubbed, so a note the scrubber
///    would mask or refuse is replaced by a fixed line that names nothing (P-2).
/// 6. `drop_unmaskable_excerpts(&mut set, scrubber)`, whose notes are built safe.
/// 7. `set.notes` = `input.notes`, then the pass's notes, then the drop notes.
///
/// The request is `item_key`, `item_body`, `phase` and the input documents' bodies from `spec`,
/// `input.touched_prefixes`, no `changed_paths` (D122), `input.roots`, the budget, and the caps,
/// `scan_cap` and `deadline` of step 1. `est` is `spec.estimator`.
pub async fn excerpts_for(
    spec: &PromptSpec,
    input: PassInput,
    app: &BTreeMap<String, serde_json::Value>,
    scrubber: &dyn Scrubber,
) -> ExcerptSet {
    let (caps, scan_cap, deadline) = resolve_excerpt_caps(app);
    let PassInput {
        roots,
        touched_prefixes,
        notes,
    } = input;

    let places = parse(spec.role, &spec.body)
        .is_ok_and(|parsed| parsed.used.contains(&Placeholder::Excerpts));
    if !places {
        let mut notes = notes;
        notes.push(format!(
            "excerpt: template `{}` places no {{{{excerpts}}}}; nothing was read",
            spec.template.name
        ));
        return unscanned(&roots, caps, notes);
    }

    let readable = roots.iter().any(|root| root.source != RootSource::NoPath);
    let budget_tokens = if readable {
        match excerpt_residual(spec, scrubber) {
            Ok(budget) => budget,
            // `assemble` refuses this spec the same way, so nothing read here could be sent.
            Err(_) => return unscanned(&roots, caps, notes),
        }
    } else {
        0
    };
    let request = OwnedExcerptRequest {
        item_key: spec.item_key.clone(),
        item_body: spec.item_body.clone(),
        phase: spec.phase.clone(),
        document_bodies: spec
            .documents
            .iter()
            .map(|document| document.body.clone())
            .collect(),
        touched_prefixes,
        // D122: the previous attempt's diff carries no repo-qualified path list.
        changed_paths: Vec::new(),
        roots,
        budget_tokens,
        caps,
        scan_cap,
        deadline,
    };
    let est = spec.estimator;

    let mut set = if readable {
        let roots = request.roots.clone();
        match tokio::task::spawn_blocking(move || excerpt_pass(&request, est)).await {
            Ok(set) => set,
            Err(error) => {
                let mut notes = notes;
                notes.push(
                    if error.is_panic() {
                        "excerpt: the pass panicked; no excerpts"
                    } else {
                        // The runtime shut down before the blocking task ran.
                        "excerpt: the pass was cancelled; no excerpts"
                    }
                    .to_owned(),
                );
                return unscanned(&roots, caps, notes);
            }
        }
    } else {
        // No readable root: `select` lists nothing and reads nothing, and the one provider runs
        // on this thread, so there is no I/O to move off the runtime (H-9).
        excerpt_pass(&request, est)
    };
    // The pass's own notes name `repo:path` as the reader returned it, so they are checked
    // before `drop_unmaskable_excerpts` appends its own, which are built safe (P-2).
    withhold_unmaskable_notes(&mut set.notes, scrubber);
    drop_unmaskable_excerpts(&mut set, scrubber);
    let mut all = notes;
    all.append(&mut set.notes);
    set.notes = all;
    set
}

/// The set a pass that read nothing records: the built-in registered, every root as given and
/// unscanned (sorted by repo bytes, as `select` sorts them), the caps, and `notes`.
fn unscanned(roots: &[RepoRoot], caps: ExcerptCaps, notes: Vec<String>) -> ExcerptSet {
    let mut records: Vec<RootRecord> = roots
        .iter()
        .map(|root| RootRecord {
            repo: root.repo.clone(),
            source: root.source,
            scan_truncated: false,
        })
        .collect();
    records.sort_by(|a, b| a.repo.as_bytes().cmp(b.repo.as_bytes()));
    ExcerptSet {
        files: Vec::new(),
        audit: ExcerptAudit {
            provider_set: vec![BUILTIN_ID.to_owned()],
            roots: records,
            considered: 0,
            selected: 0,
            caps,
            files: Vec::new(),
        },
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gitignore_subset_is_the_declared_one() {
        let rules = GitignoreSubset::parse(
            "# comment\n\ntarget/\n/build\n*.tmp\nnode_modules\na/b\n!target/keep\nsrc/*.gen.rs\n",
        );
        assert!(rules.matches("target"));
        assert!(rules.matches("target/debug/x.rs"));
        assert!(
            rules.matches("crates/target/x.rs"),
            "a bare name is unanchored"
        );
        assert!(rules.matches("build"));
        assert!(rules.matches("build/out.rs"));
        assert!(
            !rules.matches("crates/build/out.rs"),
            "a leading `/` anchors"
        );
        assert!(rules.matches("scratch.tmp"));
        assert!(rules.matches("src/scratch.tmp"));
        assert!(rules.matches("node_modules/pkg/index.js"));
        assert!(rules.matches("a/b"));
        assert!(rules.matches("a/b/c.rs"));
        assert!(!rules.matches("x/a/b"), "an embedded `/` anchors too");
        // `!` is ignored rather than honoured.
        assert!(rules.matches("target/keep"));
        // Any other wildcard form is outside the subset and is not guessed at.
        assert!(!rules.matches("src/thing.gen.rs"));
        assert!(GitignoreSubset::parse("# only a comment\n").is_empty());
    }

    #[test]
    fn the_reader_refuses_a_path_that_leaves_its_root() {
        let reader = FsRepoReader::default();
        let root = RepoRoot {
            repo: "htui".to_owned(),
            root: PathBuf::from("/tmp"),
            source: RootSource::RunStepTree,
        };
        for path in ["../etc/passwd", "a/../../etc/passwd", "/etc/passwd"] {
            let error = reader.read(&root, path).expect_err("refused");
            assert!(
                error.message.contains("repo-relative"),
                "`{path}`: {}",
                error.message
            );
        }
    }

    #[test]
    fn normalise_folds_crlf_and_drops_the_bom() {
        assert_eq!(normalise("\u{feff}a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(normalise(""), "");
    }
}
