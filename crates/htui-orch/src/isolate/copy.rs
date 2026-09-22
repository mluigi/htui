//! The `copy` mode's walker and its guards (plan D35; ANA-2 `:915-933`).
//!
//! `copy` is the mode for a repository `git worktree` cannot serve — submodules, LFS, a
//! path-anchored toolchain cache — and ANA-2 `:919-926` is blunt about what it costs: the copy is
//! a byte-for-byte one, so the exclusion list is *mandatory* and the size is measured before the
//! first byte. Both live here, and nothing else does: this module knows about directories and
//! bytes, never about runs, steps or `gix`.
//!
//! The copy carries the source's `.git` (ANA-2 `:915`, the mode table at plan `:204`), which is
//! what makes it a real checkout the agent can commit in and what `reconcile` later fetches the
//! range out of. So `.git` is never excluded and [`measure`] counts it.
//!
//! Everything here is synchronous; milestone 3's `isolate::real::GixIsolator` calls it under
//! `tokio::task::spawn_blocking` (it is named in text rather than linked because T5 creates it
//! after this module). The two git steps that finish a copy — `git reset --hard` for a dirty
//! source (plan D47) and the `htui/<step>` label of blueprint A-2 — belong to
//! [`Cli`](super::git::Cli) and [`create_branch`](super::git::create_branch) and are never
//! re-implemented here.

use std::path::Path;

use walkdir::WalkDir;

use super::IsolateError;

/// ANA-2 `:923-924`'s seven entries, applied when `ProjectSettings.copy_exclude` is empty (D35).
pub const DEFAULT_COPY_EXCLUDE: [&str; 7] = [
    "target/",
    "build/",
    "cmake-build-*/",
    "node_modules/",
    ".venv/",
    "out/",
    "dist/",
];

/// The measured refusal of ANA-2 `:930-932`: the size is named, so is the cap.
#[must_use]
pub fn copy_over_cap(need: u64, cap: u64) -> String {
    format!("copy would need {need} bytes; cap is {cap}")
}

/// One exclusion entry: a path **component** with at most one trailing `*`.
///
/// "No glob crate" is ANA-2 `:1783`'s own rule, so this is the whole matcher: no `**`, no
/// character class, no separator. A build directory is named by its directory name at any depth —
/// `crate/node_modules` is excluded by `node_modules/` exactly as the top-level one is — which is
/// what the seven entries of [`DEFAULT_COPY_EXCLUDE`] mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclude {
    /// The literal text before the `*`, or the whole component when there is none.
    prefix: String,
    /// Whether the entry ended in `*`.
    wildcard: bool,
}

impl Exclude {
    /// Parses one entry, or `None` when it is not one.
    ///
    /// A trailing `/` is stripped — every entry ANA-2 names a directory that way. `None` for an
    /// empty entry, for one containing a separator (it is then not a component) and for one whose
    /// `*` is anywhere but last. A caller that has a list parses it through [`excludes`], which
    /// skips and warns rather than refusing the step over a typo in a project setting.
    #[must_use]
    pub fn parse(entry: &str) -> Option<Self> {
        let entry = entry.strip_suffix('/').unwrap_or(entry);
        if entry.is_empty() || entry.contains('/') || entry.contains('\\') {
            return None;
        }
        let wildcard = entry.ends_with('*');
        let prefix = if wildcard {
            &entry[..entry.len() - 1]
        } else {
            entry
        };
        if prefix.contains('*') {
            return None;
        }
        Some(Self {
            prefix: prefix.to_owned(),
            wildcard,
        })
    }

    /// Whether one path component is excluded by this entry.
    #[must_use]
    pub fn matches(&self, component: &str) -> bool {
        if self.wildcard {
            component.starts_with(&self.prefix) && !component.contains('/')
        } else {
            component == self.prefix
        }
    }
}

/// The project's list, or [`DEFAULT_COPY_EXCLUDE`] when it is empty (D35).
///
/// A project list **replaces** the default rather than extending it: ANA-2 `:923` makes
/// `copy_exclude` the list, and a project that deliberately copies its `target/` would have no way
/// to say so otherwise. An entry [`Exclude::parse`] refuses is skipped with a `warn` — one bad
/// row of a settings table must not make every `copy` step of the project unrunnable.
#[must_use]
pub fn excludes(project: &[String]) -> Vec<Exclude> {
    if project.is_empty() {
        return DEFAULT_COPY_EXCLUDE
            .iter()
            .filter_map(|entry| Exclude::parse(entry))
            .collect();
    }
    project
        .iter()
        .filter_map(|entry| {
            let parsed = Exclude::parse(entry);
            if parsed.is_none() {
                tracing::warn!(
                    entry = %entry,
                    "copy_exclude entry is not a path component with at most one trailing `*`; skipped"
                );
            }
            parsed
        })
        .collect()
}

/// Bytes of every file `copy_tree` would copy: a `walkdir` over `src` pruning any directory an
/// exclude names (`htui_store::vector_sync`'s bounded walk, blueprint F-H).
///
/// `.git/` is counted, because the copy is a real checkout and carries it (the mode table, plan
/// `:204`; ANA-2 `:915`). Symlinks are counted as links and never followed, so a link into
/// `target/` costs its own few bytes and cannot walk the tree twice.
///
/// # Errors
/// [`IsolateError::Io`] when the walk cannot read the tree.
pub fn measure(src: &Path, excludes: &[Exclude]) -> Result<u64, IsolateError> {
    let mut total = 0_u64;
    for entry in walk(src, excludes) {
        let entry = entry.map_err(walk_error)?;
        let kind = entry.file_type();
        if kind.is_dir() {
            continue;
        }
        total = total.saturating_add(entry.metadata().map_err(walk_error)?.len());
    }
    Ok(total)
}

/// [`measure`], refused above `cap`.
///
/// ANA-2 `:930-932`: the tree is measured before the first copy of a run and the refusal names the
/// size. `cap` is `app_setting.copy_max_total_bytes` times the number of copies — one, this
/// milestone — so the multiplication is the caller's.
///
/// # Errors
/// [`IsolateError::Io`] from the walk; [`IsolateError::Refused`] with [`copy_over_cap`] when the
/// measured size is above `cap`.
pub fn measure_within_cap(src: &Path, excludes: &[Exclude], cap: u64) -> Result<u64, IsolateError> {
    let need = measure(src, excludes)?;
    if need > cap {
        return Err(IsolateError::Refused(copy_over_cap(need, cap)));
    }
    Ok(need)
}

/// The one walk both [`measure`] and `copy_tree` run: depth-first, links never followed, any
/// directory (or file) whose name an exclude matches pruned whole, the root itself never pruned.
fn walk(
    src: &Path,
    excludes: &[Exclude],
) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> {
    let owned: Vec<Exclude> = excludes.to_vec();
    WalkDir::new(src)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| {
            // `filter_entry` is asked about the root too, and a repository that happens to be
            // called `build` is still the thing we were told to copy.
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !owned.iter().any(|exclude| exclude.matches(&name))
        })
}

/// A `walkdir` failure as this crate's error, keeping the `io::Error` when there is one.
fn walk_error(err: walkdir::Error) -> IsolateError {
    let path = err.path().map(Path::to_path_buf);
    match err.into_io_error() {
        Some(io) => IsolateError::Io(io),
        None => IsolateError::Io(std::io::Error::other(match path {
            Some(path) => format!("walking {}: a cycle of symbolic links", path.display()),
            None => "walking the copy source: a cycle of symbolic links".to_owned(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{DEFAULT_COPY_EXCLUDE, Exclude, excludes, measure};

    /// A directory tree from `(relative path, bytes)` pairs; a trailing `/` makes a directory.
    fn lay_out(root: &Path, entries: &[(&str, usize)]) {
        for (path, size) in entries {
            let full = root.join(path.trim_end_matches('/'));
            if path.ends_with('/') {
                std::fs::create_dir_all(&full).expect("the directory is created");
            } else {
                std::fs::create_dir_all(full.parent().expect("a parent"))
                    .expect("the parent is created");
                std::fs::write(&full, vec![b'x'; *size]).expect("the file is written");
            }
        }
    }

    /// ANA-2 `:923-924` fixes the list and calls it mandatory, and `ProjectSettings::default()`
    /// seeds nothing (`crates/htui-core/src/model/kind.rs:292`), so the default is this constant
    /// or it is nowhere.
    #[test]
    fn default_excludes_match_ana2s_seven_entries() {
        assert_eq!(
            DEFAULT_COPY_EXCLUDE,
            [
                "target/",
                "build/",
                "cmake-build-*/",
                "node_modules/",
                ".venv/",
                "out/",
                "dist/"
            ]
        );
    }

    /// D35: the default is what an empty `ProjectSettings.copy_exclude` means, and a project that
    /// names its own list gets that list and nothing added to it.
    #[test]
    fn an_empty_project_list_falls_back_to_the_default() {
        let fallback = excludes(&[]);
        assert_eq!(fallback.len(), DEFAULT_COPY_EXCLUDE.len());
        assert!(fallback.iter().any(|entry| entry.matches("node_modules")));

        let named = excludes(&["vendor".to_owned()]);
        assert_eq!(named.len(), 1);
        assert!(named[0].matches("vendor"));
        assert!(
            !named.iter().any(|entry| entry.matches("target")),
            "a project list replaces the default, it does not extend it"
        );
    }

    /// "No glob crate" (ANA-2 `:1783`): one trailing `*` over a single path component, and an
    /// entry that is not that shape is skipped rather than half-honoured.
    #[test]
    fn cmake_build_star_matches_by_prefix() {
        let entry = Exclude::parse("cmake-build-*/").expect("a component with a trailing star");
        assert!(entry.matches("cmake-build-debug"));
        assert!(entry.matches("cmake-build-"), "the prefix alone matches");
        assert!(
            !entry.matches("cmake-buildx/y"),
            "a nested path is never one component"
        );
        assert!(!entry.matches("xcmake-build-"), "the star is not a prefix");

        let plain = Exclude::parse("target/").expect("a bare component");
        assert!(plain.matches("target"));
        assert!(!plain.matches("target-two"), "no star, no prefix match");

        assert_eq!(
            Exclude::parse("a/b"),
            None,
            "a separator is not a component"
        );
        assert_eq!(Exclude::parse("no*star"), None, "the star must be last");
        assert_eq!(Exclude::parse(""), None);
        assert_eq!(Exclude::parse("/"), None);
    }

    /// D35 measures "what would be copied": every excluded directory is pruned whole, and `.git`
    /// is counted because the copy is a real checkout (the mode table, plan `:204`).
    #[test]
    fn measure_counts_only_what_would_be_copied() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        lay_out(
            &src,
            &[
                ("f", 10),
                (".git/objects/pack/p", 100),
                ("crate/src/lib.rs", 1_000),
                ("target/debug/deps/huge", 1024 * 1024),
                ("crate/node_modules/left-pad/index.js", 5_000),
                ("cmake-build-debug/CMakeCache.txt", 7_000),
            ],
        );

        let counted = measure(&src, &excludes(&[])).expect("the walk reads the tree");
        assert_eq!(
            counted, 1_110,
            "the three excluded directories are pruned whole; .git is counted"
        );
    }

    /// The cap is measured before the first byte is written (ANA-2 `:930-932`), and the refusal
    /// names both numbers so the operator can widen `copy_max_total_bytes` or the exclusion list.
    #[test]
    fn a_tree_over_the_cap_is_refused_with_both_numbers() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        lay_out(&src, &[("f", 4_096)]);

        let list = excludes(&[]);
        assert_eq!(
            super::measure_within_cap(&src, &list, 4_096).expect("exactly at the cap is allowed"),
            4_096
        );
        let err = super::measure_within_cap(&src, &list, 4_095).expect_err("one byte over");
        assert_eq!(
            err.to_string(),
            "isolation refused: copy would need 4096 bytes; cap is 4095"
        );
    }
}
