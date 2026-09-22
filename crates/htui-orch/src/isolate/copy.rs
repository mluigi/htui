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

use std::path::{Path, PathBuf};

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

/// Bytes of every file [`copy_tree`] would copy: a `walkdir` over `src` pruning any directory an
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

/// The one walk both [`measure`] and [`copy_tree`] run: depth-first, links never followed, any
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

/// D35: the source has no `.git` at all, so the copy would not be a checkout.
#[must_use]
pub fn not_a_git_checkout() -> String {
    "not a git checkout".to_owned()
}

/// D35: the source's `.git` is a *file*, so the source is itself a linked worktree and a copy of
/// it would point back at the original's `worktrees/` entry.
#[must_use]
pub fn linked_worktree_source() -> String {
    "source is a linked worktree".to_owned()
}

/// Whether `src` is a checkout this mode can copy — D35's two refusals, taken before the first
/// byte is written.
///
/// A missing `.git` is the PRD's open question (`:378-380`), adopted as refused: a directory that
/// is not a checkout has no `HEAD` to record as `before_hash` and nothing for `reconcile` to merge
/// back. A `.git` that is a *file* is the `gitdir:` pointer `git worktree add` writes, so the
/// source is itself a linked worktree and the copy would share the original's `worktrees/` entry.
///
/// # Errors
/// [`IsolateError::Refused`] with [`not_a_git_checkout`] or [`linked_worktree_source`];
/// [`IsolateError::Io`] when `.git` cannot be stat-ed at all.
pub fn check_source(src: &Path) -> Result<(), IsolateError> {
    match std::fs::metadata(src.join(".git")) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(IsolateError::Refused(linked_worktree_source())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Err(IsolateError::Refused(not_a_git_checkout()))
        }
        Err(err) => Err(IsolateError::Io(err)),
    }
}

/// The half-built sibling of `dst`: the same path with `.partial` appended to its last component.
///
/// A sibling and not a child, and not a path under some other root, for one reason: the last step
/// of a copy is a `rename` onto `dst`, and `rename` is only atomic — only *possible*, on several
/// filesystems — within one directory (blueprint H-26).
#[must_use]
pub fn partial_path(dst: &Path) -> PathBuf {
    let mut name = dst.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    dst.with_file_name(name)
}

/// `src` → `dst` minus `excludes`: files, directories, symlinks re-created as symlinks with their
/// target verbatim, Unix permission bits preserved. `dst` must not exist.
///
/// A symlink is never followed, by the walk or by this: resolving one would copy the pointed-at
/// tree into the copy, and a link into `target/` would defeat the exclusion list outright. What a
/// relative link means inside the copy is what it meant inside the source.
///
/// Directory modes are applied **after** the walk and deepest-first, because a source directory
/// the owner narrowed to `0o500` would otherwise refuse the very children we are about to write
/// into it. File modes come from `std::fs::copy`, which carries them.
///
/// This is byte copying and nothing cleverer — no reflink, no rename, no hard link — so a
/// `scratch_root` on a different mount from the repository is the same code path as one beside it
/// (blueprint H-26).
///
/// # Errors
/// [`IsolateError::Refused`] from [`check_source`], before anything is created;
/// [`IsolateError::Io`] for an existing `dst` or any failure of the walk or the copy.
pub fn copy_tree(src: &Path, dst: &Path, excludes: &[Exclude]) -> Result<(), IsolateError> {
    check_source(src)?;
    if dst.exists() {
        return Err(IsolateError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("copy destination {} already exists", dst.display()),
        )));
    }
    std::fs::create_dir_all(dst)?;

    let mut directories: Vec<(PathBuf, std::fs::Permissions)> = Vec::new();
    for entry in walk(src, excludes) {
        let entry = entry.map_err(walk_error)?;
        let relative = entry
            .path()
            .strip_prefix(src)
            .map_err(|_| unreachable_prefix(entry.path(), src))?;
        let target = dst.join(relative);
        let kind = entry.file_type();

        if kind.is_symlink() {
            recreate_symlink(entry.path(), &target)?;
        } else if kind.is_dir() {
            std::fs::create_dir_all(&target)?;
            directories.push((target, entry.metadata().map_err(walk_error)?.permissions()));
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }

    for (path, permissions) in directories.into_iter().rev() {
        std::fs::set_permissions(&path, permissions)?;
    }
    Ok(())
}

/// [`copy_tree`] into [`partial_path`], which is where a copy is built (blueprint H-6).
///
/// A copy only becomes a tree once `git reset --hard` has cleaned it (plan D47) and the
/// `htui/<step>` label of blueprint A-2 names its base — and both of those are git's, not this
/// module's. Until they have run the directory is a half-made checkout that D38's idempotence
/// branch ("`<tree>/.git` exists → reuse it") would happily hand to an agent. So it is built under
/// a name D38 does not look at, and [`finish_partial`] is what makes it visible.
///
/// A `.partial` found on entry is a previous attempt that died before its rename; it is deleted
/// whole, never copied into and never reused. An attempt interrupted here therefore costs the
/// bytes of one copy and nothing else: the destination still does not exist, so the next `prepare`
/// takes this same path again from the beginning.
///
/// # Errors
/// As [`copy_tree`], plus [`IsolateError::Io`] when the stale `.partial` cannot be removed.
pub fn copy_into_partial(
    src: &Path,
    dst: &Path,
    excludes: &[Exclude],
) -> Result<PathBuf, IsolateError> {
    let partial = partial_path(dst);
    match std::fs::remove_dir_all(&partial) {
        Ok(()) => tracing::warn!(
            partial = %partial.display(),
            "a half-finished copy from an earlier attempt was removed before this one started"
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(IsolateError::Io(err)),
    }
    copy_tree(src, &partial, excludes)?;
    Ok(partial)
}

/// Renames [`partial_path`] onto `dst`: the last step of a copy and the only one that makes it
/// reusable (blueprint H-6).
///
/// Same-directory by construction ([`partial_path`]), so it is one `rename` and cannot half
/// happen. Called only once the copy is a checkout an agent could be handed.
///
/// # Errors
/// [`IsolateError::Io`] when the rename fails — a `dst` that appeared in the meantime among them.
pub fn finish_partial(dst: &Path) -> Result<(), IsolateError> {
    std::fs::rename(partial_path(dst), dst)?;
    Ok(())
}

/// A symlink at `source` re-created at `target`, its link text taken verbatim.
fn recreate_symlink(source: &Path, target: &Path) -> Result<(), IsolateError> {
    let link = std::fs::read_link(source)?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&link, target)?;
    }
    #[cfg(windows)]
    {
        // Windows needs to know whether the link is to a directory, and needs Developer Mode or
        // elevation to create one at all; what to do when it refuses is MOD-16's call (blueprint
        // §10, the `copy_tree` row), not this milestone's.
        if std::fs::metadata(source).is_ok_and(|meta| meta.is_dir()) {
            std::os::windows::fs::symlink_dir(&link, target)?;
        } else {
            std::os::windows::fs::symlink_file(&link, target)?;
        }
    }
    Ok(())
}

/// The walk only ever yields paths under its own root, so this is unreachable; it exists because
/// an `unwrap` in a filesystem loop is a panic in production for a case nobody can name.
fn unreachable_prefix(path: &Path, src: &Path) -> IsolateError {
    IsolateError::Io(std::io::Error::other(format!(
        "{} is not under the copy source {}",
        path.display(),
        src.display()
    )))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        DEFAULT_COPY_EXCLUDE, Exclude, copy_into_partial, copy_tree, excludes, finish_partial,
        measure, partial_path,
    };

    /// Every relative path under `root`, sorted, directories with a trailing `/`.
    fn layout_of(root: &Path) -> Vec<String> {
        let mut seen: Vec<String> = walkdir::WalkDir::new(root)
            .follow_links(false)
            .min_depth(1)
            .into_iter()
            .map(|entry| entry.expect("the walk reads the copy"))
            .map(|entry| {
                let rel = entry
                    .path()
                    .strip_prefix(root)
                    .expect("every entry is under the root")
                    .to_string_lossy()
                    .into_owned();
                if entry.file_type().is_dir() {
                    format!("{rel}/")
                } else {
                    rel
                }
            })
            .collect();
        seen.sort();
        seen
    }

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

    /// The mode table (plan `:204`): "a filesystem copy including `.git/` minus `copy_exclude`".
    /// The `.git` half is not a detail — a copy without it is not a checkout, has no `HEAD` to
    /// reset to and nothing for `reconcile` to fetch a range out of.
    #[test]
    fn copy_tree_reproduces_the_layout_minus_excludes_and_keeps_dot_git() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        lay_out(
            &src,
            &[
                (".git/HEAD", 41),
                (".git/objects/pack/p", 100),
                ("README.md", 12),
                ("crate/src/lib.rs", 30),
                ("crate/target/debug/huge", 4_096),
                ("node_modules/left-pad/index.js", 20),
                ("cmake-build-debug/CMakeCache.txt", 90),
                ("empty/", 0),
            ],
        );

        let dst = dir.path().join("copy");
        copy_tree(&src, &dst, &excludes(&[])).expect("the tree copies");

        assert_eq!(
            layout_of(&dst),
            vec![
                ".git/",
                ".git/HEAD",
                ".git/objects/",
                ".git/objects/pack/",
                ".git/objects/pack/p",
                "README.md",
                "crate/",
                "crate/src/",
                "crate/src/lib.rs",
                "empty/",
            ]
        );
        assert_eq!(
            std::fs::read(dst.join(".git/HEAD")).expect("the copied file reads"),
            vec![b'x'; 41],
            "content is copied, not just the name"
        );
    }

    /// D35's first refusal, the PRD's open question answered (`:378-380`): a directory that is not
    /// a checkout has no `before_hash` and nothing to reconcile, so it is refused rather than
    /// copied into a tree the walk would then have to explain.
    #[test]
    fn a_source_without_dot_git_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        lay_out(&src, &[("README.md", 3)]);

        let dst = dir.path().join("copy");
        let err = copy_tree(&src, &dst, &excludes(&[])).expect_err("no .git, no copy");
        assert_eq!(err.to_string(), "isolation refused: not a git checkout");
        assert!(!dst.exists(), "refused before the first byte");
    }

    /// D35's second: `git worktree add` writes a `gitdir:` *file* at `.git`, so a source with one
    /// is already a linked worktree and its copy would point back at the original's `worktrees/`
    /// entry — two checkouts sharing one administrative directory.
    #[test]
    fn a_source_with_a_dot_git_file_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).expect("the source is created");
        std::fs::write(src.join(".git"), "gitdir: /elsewhere/.git/worktrees/w\n")
            .expect("the gitfile is written");

        let dst = dir.path().join("copy");
        let err = copy_tree(&src, &dst, &excludes(&[])).expect_err("a gitfile is not a checkout");
        assert_eq!(
            err.to_string(),
            "isolation refused: source is a linked worktree"
        );
        assert!(!dst.exists(), "refused before the first byte");
    }

    /// A symlink is re-created as a symlink with its target verbatim: resolving it would copy the
    /// pointed-at tree into the copy (and a link into `target/` would defeat the exclusion list
    /// entirely), and a relative link keeps working inside the copy.
    #[cfg(unix)]
    #[test]
    fn a_symlink_is_copied_as_a_symlink() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        lay_out(
            &src,
            &[(".git/HEAD", 41), ("real.txt", 5), ("target/big", 900)],
        );
        std::os::unix::fs::symlink("real.txt", src.join("link.txt")).expect("the link is made");
        std::os::unix::fs::symlink("target", src.join("to-target")).expect("the link is made");

        let dst = dir.path().join("copy");
        copy_tree(&src, &dst, &excludes(&[])).expect("the tree copies");

        let link = dst.join("link.txt");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("the link exists")
                .is_symlink(),
            "copied as a link, not as its target's bytes"
        );
        assert_eq!(
            std::fs::read_link(&link).expect("the link reads"),
            Path::new("real.txt"),
            "the target is kept verbatim"
        );
        assert!(
            std::fs::symlink_metadata(dst.join("to-target"))
                .expect("the link exists")
                .is_symlink(),
            "a link named after nothing excluded survives even when it points at an excluded name"
        );
        assert!(
            !dst.join("target").exists(),
            "and following it did not drag `target/` in"
        );
    }

    /// Unix permission bits are the copy's too: a `.git/hooks/pre-commit` that lost its execute
    /// bit is a hook that silently stops running, and a `0o700` directory copied as `0o755` is a
    /// permission the source deliberately narrowed.
    #[cfg(unix)]
    #[test]
    fn unix_permissions_survive_the_copy() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        lay_out(
            &src,
            &[(".git/HEAD", 41), ("hook.sh", 9), ("private/inner", 1)],
        );
        std::fs::set_permissions(src.join("hook.sh"), std::fs::Permissions::from_mode(0o755))
            .expect("the file's mode is set");
        std::fs::set_permissions(src.join("private"), std::fs::Permissions::from_mode(0o700))
            .expect("the directory's mode is set");

        let dst = dir.path().join("copy");
        copy_tree(&src, &dst, &excludes(&[])).expect("the tree copies");

        let mode = |path: &Path| {
            std::fs::metadata(path)
                .expect("the copy reads")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(&dst.join("hook.sh")), 0o755);
        assert_eq!(
            mode(&dst.join("private")),
            0o700,
            "a narrowed directory mode is applied after its children are written, not before"
        );
        assert!(dst.join("private/inner").exists());
    }

    /// `dst` must not exist: a copy over a populated directory is not a copy of the source, and
    /// D38's reuse branch is the only thing allowed to look at an existing tree.
    #[test]
    fn copy_tree_refuses_a_destination_that_already_exists() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        lay_out(&src, &[(".git/HEAD", 41)]);
        let dst = dir.path().join("copy");
        lay_out(&dst, &[("stale", 1)]);

        let err = copy_tree(&src, &dst, &excludes(&[])).expect_err("the destination is there");
        assert_eq!(
            err.to_string().lines().next().unwrap_or_default(),
            format!("copy destination {} already exists", dst.display()),
            "unexpected sentence: {err}"
        );
        assert!(dst.join("stale").exists(), "and nothing of it was touched");
    }

    /// Blueprint H-6: a copy interrupted before its `git reset --hard` and its `htui/<step>` label
    /// leaves a directory with a `.git` in it, which is exactly what D38's idempotence branch
    /// reuses. So the copy is built under `<name>.partial` and renamed last, a `.partial` found on
    /// entry is deleted, and the destination never exists in a half-finished state.
    #[test]
    fn a_half_finished_copy_is_never_reusable() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let src = dir.path().join("src");
        lay_out(&src, &[(".git/HEAD", 41), ("f", 4)]);
        let dst = dir.path().join("trees").join("repo");

        // The crash: a previous attempt left a half-copy behind and never renamed it.
        let stale = partial_path(&dst);
        lay_out(&stale, &[(".git/HEAD", 41), ("leftover", 1)]);
        assert!(
            !dst.exists(),
            "the crashed attempt never reached the rename"
        );

        let partial = copy_into_partial(&src, &dst, &excludes(&[])).expect("the tree copies");
        assert_eq!(partial, stale, "one name, and it is a sibling of the tree");
        assert!(
            !partial.join("leftover").exists(),
            "the stale partial is deleted, not copied into"
        );
        assert!(
            !dst.exists(),
            "the destination appears only once the caller says the copy is finished"
        );

        finish_partial(&dst).expect("the copy is renamed into place");
        assert!(dst.join(".git").is_dir() && dst.join("f").is_file());
        assert!(!partial.exists(), "and the partial name is gone");
    }
}
