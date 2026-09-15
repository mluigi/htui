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
//! No new dependency (§4.5 `:1007`): the walk is `std::fs` plus a hand matcher, in the shape
//! [`crate::probe`]'s glob walker already established, and the `.gitignore` matcher is a declared
//! **subset** rather than a glob crate.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use tracing::warn;

use htui_core::prompt::excerpt::{
    ExcerptCandidate, ExcerptProvider, ExcerptRequest, OwnedExcerptRequest, ProviderError,
    RepoReader, RepoRoot, skip_by_path,
};
use htui_core::prompt::settings::DEFAULTS;

/// How many bytes of a file the binary test reads (§4.5 `:1073`).
pub const BINARY_PROBE_BYTES: usize = 8_192;

/// The highest weight a provider may claim; `htui` re-normalises against its own tiers
/// (§4.5 `:1164`).
const MAX_PROVIDER_WEIGHT: u16 = 100;

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
/// `max_file_bytes` is carried on the reader rather than passed to `list`, because
/// [`RepoReader::list`] takes only the scan cap and §4.5's fifth skip rule needs the size limit at
/// the same moment the file is stat-ed. [`Default`] is `DEFAULTS.excerpt_max_file_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsRepoReader {
    /// `app_setting.excerpt_max_file_bytes`: skip rule 5.
    pub max_file_bytes: u64,
}

impl Default for FsRepoReader {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULTS.excerpt_max_file_bytes,
        }
    }
}

impl FsRepoReader {
    /// A reader with an explicit size limit, as `settings::resolve_excerpt_caps` resolved it.
    #[must_use]
    pub const fn new(max_file_bytes: u64) -> Self {
        Self { max_file_bytes }
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
            let child = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
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
                // Rule 1, at the directory: nothing under `.git/` is even stat-ed.
                if name == ".git" || self.skip(&child, ignores, None).is_some() {
                    continue;
                }
                if self.walk(base, &child, ignores, out, examined, cap)? {
                    truncated = true;
                    break;
                }
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            *examined += 1;
            if *examined > cap {
                truncated = true;
                break;
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
        let Ok(meta) = std::fs::metadata(path) else {
            return Some(SkipRule::Binary);
        };
        if has_nul(path) {
            return Some(SkipRule::Binary);
        }
        if meta.len() > self.max_file_bytes {
            return Some(SkipRule::TooLarge);
        }
        if skip_by_path(relative) == Some("lockfile_or_minified") {
            return Some(SkipRule::LockfileOrMinified);
        }
        None
    }
}

/// §4.5's binary test: a NUL in the first [`BINARY_PROBE_BYTES`]. An unreadable file reads as one.
fn has_nul(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return true;
    };
    let mut buffer = [0u8; BINARY_PROBE_BYTES];
    match file.read(&mut buffer) {
        Ok(read) => buffer[..read].contains(&0),
        Err(_) => true,
    }
}

impl RepoReader for FsRepoReader {
    fn list(&self, root: &RepoRoot, cap: u32) -> Result<(Vec<String>, bool), ProviderError> {
        let mut out = Vec::new();
        let mut ignores = Vec::new();
        let mut examined = 0u32;
        let truncated = self.walk(&root.root, "", &mut ignores, &mut out, &mut examined, cap)?;
        Ok((out, truncated))
    }

    fn read(&self, root: &RepoRoot, path: &str) -> Result<String, ProviderError> {
        // Repo-relative by contract: a caller that reached outside the root would be rendering an
        // absolute path, which §4.2 rule 5 forbids and hazard H-1 names.
        if path.split('/').any(|segment| segment == "..") || path.starts_with('/') {
            return Err(ProviderError::new(
                "fs",
                format!("`{path}` is not repo-relative"),
            ));
        }
        let bytes = std::fs::read(root.root.join(path))
            .map_err(|error| ProviderError::new("fs", format!("read: {error}")))?;
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

/// What one provider's thread reported back.
enum Outcome {
    Proposed(Vec<ExcerptCandidate>),
    Failed(ProviderError),
    Panicked,
}

/// Run every provider concurrently under one wall-clock deadline (§4.5 rule 2).
///
/// Returns the merged candidate list and `provider_set` in blueprint P-11's grammar:
/// `name@version` when the provider answered, `name@version:error`, `:panic` or `:timeout` when it
/// did not. Registration order is preserved, so a caller that puts
/// [`BuiltinRanker`](htui_core::prompt::excerpt::BuiltinRanker) first gets `builtin@1` first —
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
/// clamped to `0..=100`: a provider may propose, and it may not claim a tier — or another
/// provider's name — by spelling one.
#[must_use]
pub fn run_providers(
    providers: &[Arc<dyn ExcerptProvider>],
    req: &ExcerptRequest<'_>,
) -> (Vec<ExcerptCandidate>, Vec<String>) {
    let owned = Arc::new(OwnedExcerptRequest::from_request(req));
    let (sender, receiver) = mpsc::channel::<(usize, Outcome)>();
    for (index, provider) in providers.iter().enumerate() {
        let provider = Arc::clone(provider);
        let request = Arc::clone(&owned);
        let sender = sender.clone();
        // Detached on purpose: see H-20 above.
        std::thread::spawn(move || {
            let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                provider.propose(&request.as_request())
            })) {
                Ok(Ok(candidates)) => Outcome::Proposed(candidates),
                Ok(Err(error)) => Outcome::Failed(error),
                Err(_) => Outcome::Panicked,
            };
            // The receiver may already have given up; that is the timeout case, not an error.
            let _ = sender.send((index, outcome));
        });
    }
    drop(sender);

    let mut outcomes: Vec<Option<Outcome>> = (0..providers.len()).map(|_| None).collect();
    let started = Instant::now();
    let mut pending = providers.len();
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
            root: std::path::PathBuf::from("/tmp"),
            source: htui_core::prompt::excerpt::RootSource::RunStepTree,
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
