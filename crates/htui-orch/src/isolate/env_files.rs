//! The env-file scanner: which dotenv-style files a session's trees hold (MOD-89 D1, D2, D7).
//!
//! A session reads whatever its `cwd` and `extra_dirs` hold, and a gitignored `.env` sits in the
//! primary checkout `local` and `shared_serialized` run in, rides along in a `copy`, and reaches a
//! `worktree` only when it is committed. This module answers which of those files exist — by
//! **file name only, never contents** — so the engine can refuse a provider project's session
//! (D3) or note the files for a provider-less one (D6).
//!
//! The scan **fails open**: an unreadable entry is skipped with a `warn`, a missing root answers
//! nothing, and a walk past [`SCAN_CAP`] entries stops and says so rather than erring. It is
//! hygiene, not a boundary; the trust boundary stays `SessionSpec.env` (MOD-10).
//!
//! Everything here is synchronous and touches no store and no `gix`; it is called by
//! `GixIsolator::env_files` under `spawn_blocking`, as [`copy`](super::copy)'s walk is.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::copy::Exclude;

/// D2: entries the walk visits across all roots before it stops and reports `truncated`.
pub const SCAN_CAP: usize = 100_000;

/// D7: how many paths a sentence names before `and N more`.
pub const LISTED: usize = 5;

/// D1: `.env.<suffix>` suffixes that are committed templates, compared ASCII case-insensitively.
///
/// A template is committed on purpose and holds names, not values, so it is not a finding. The
/// suffix is compared whole: `.env.local.example`'s suffix is `local.example`, which is no
/// template word, so that file is a finding.
pub const TEMPLATE_SUFFIXES: [&str; 6] = [
    "example", "sample", "template", "dist", "defaults", "schema",
];

/// What [`scan`] found (MOD-89 D2): root-relative, `/`-separated, sorted by bytes, deduplicated;
/// a path under an `extra_dirs` root carries that root's last component first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvFiles {
    /// The paths.
    pub paths: Vec<String>,
    /// The walk stopped at [`SCAN_CAP`] entries; more may exist.
    pub truncated: bool,
}

impl EnvFiles {
    /// Whether the scan found nothing. A cut walk that found nothing is still empty: the engine
    /// has no file to name, and the `warn` the cut logged is the only trace (D2).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// D1: `.env`, `.envrc`, or `.env.<suffix>` with a non-empty suffix that is not a
/// [`TEMPLATE_SUFFIXES`] word; ASCII case-insensitive, so Windows and macOS read as Linux does.
///
/// A name, not a path: whether the entry is a file is [`scan`]'s question.
#[must_use]
pub fn is_env_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == ".env" || lower == ".envrc" {
        return true;
    }
    lower
        .strip_prefix(".env.")
        .is_some_and(|suffix| !suffix.is_empty() && !TEMPLATE_SUFFIXES.contains(&suffix))
}

/// D2: walks `cwd`, then each of `extra_dirs`, for env files. Never fails.
///
/// Links are never followed; a symlink is a finding when its own name is one. `.git` is pruned
/// whatever its type, and so is a *directory* named like an env file (a Python venv called
/// `.env/`) or matched by one of `excludes` — directories only, so a project that excludes `.env*`
/// from its copies does not hide a real `.env` from this check. `cap` bounds the entries visited
/// across all roots together, one budget per session.
#[must_use]
pub fn scan(cwd: &Path, extra_dirs: &[PathBuf], excludes: &[Exclude], cap: usize) -> EnvFiles {
    let mut found = Vec::new();
    let mut visited = 0_usize;
    let mut truncated = false;

    // `cwd`'s paths are bare; an `extra_dirs` root's carry its last component (D7), so two repos'
    // `.env` files read apart. A root with no last component (`/`) gets no prefix.
    let roots = std::iter::once((cwd, None)).chain(extra_dirs.iter().map(|dir| {
        (
            dir.as_path(),
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        )
    }));
    'roots: for (root, prefix) in roots {
        // A missing root answers nothing (D2). `is_dir` follows a symlinked root, as `WalkDir`
        // does for its own root.
        if !root.is_dir() {
            continue;
        }
        let walk = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| keep(entry, excludes));
        for item in walk {
            let entry = match item {
                Ok(entry) => entry,
                Err(err) => {
                    tracing::warn!(
                        path = ?err.path(),
                        error = %err,
                        "the env-file scan skipped an entry it could not read"
                    );
                    continue;
                }
            };
            // The budget is spent on what the walk yields after pruning, across every root.
            if visited == cap {
                truncated = true;
                break 'roots;
            }
            visited += 1;
            if entry.depth() == 0 || entry.file_type().is_dir() {
                continue;
            }
            if !is_env_file(&entry.file_name().to_string_lossy()) {
                continue;
            }
            // The walk only yields paths under its own root, so the prefix always strips.
            let Ok(relative) = entry.path().strip_prefix(root) else {
                continue;
            };
            let relative = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            found.push(match &prefix {
                Some(prefix) => format!("{prefix}/{relative}"),
                None => relative,
            });
        }
    }

    if truncated {
        tracing::warn!(cap, "the env-file scan stopped at its cap");
    }
    found.sort();
    found.dedup();
    EnvFiles {
        paths: found,
        truncated,
    }
}

/// [`scan`]'s prune rule, asked of every entry before the walk yields or enters it: the root is
/// always kept; `.git` never is, whatever its type; a directory is pruned when its name is an env
/// file's (D1) or an exclude matches it (D2). A file is always kept, so an exclude never hides one.
fn keep(entry: &walkdir::DirEntry, excludes: &[Exclude]) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    if name == ".git" {
        return false;
    }
    !(entry.file_type().is_dir()
        && (is_env_file(&name) || excludes.iter().any(|exclude| exclude.matches(&name))))
}

/// D7's `<list>`: the first [`LISTED`] paths backticked and joined with `, `, then
/// ` and {n} more`, then ` (scan cut at {SCAN_CAP} entries)` when truncated.
///
/// A control character renders as `?`: Linux allows a newline in a file name, and `run.failure`
/// and an item note are single-line in the TUI.
#[must_use]
pub fn listed(files: &EnvFiles) -> String {
    let mut list = files
        .paths
        .iter()
        .take(LISTED)
        .map(|path| {
            let shown: String = path
                .chars()
                .map(|ch| if ch.is_control() { '?' } else { ch })
                .collect();
            format!("`{shown}`")
        })
        .collect::<Vec<_>>()
        .join(", ");
    if files.paths.len() > LISTED {
        list.push_str(&format!(" and {} more", files.paths.len() - LISTED));
    }
    if files.truncated {
        list.push_str(&format!(" (scan cut at {SCAN_CAP} entries)"));
    }
    list
}

/// D7's refusal sentence: a provider project's session trees hold env files.
#[must_use]
pub fn refused(provider: &str, files: &EnvFiles) -> String {
    format!(
        "env_files_refused: {} in the session's trees; the project resolves its secrets from \
         {provider}. Remove them, or set allow_env_files.",
        listed(files)
    )
}

/// D7's warning sentence (the item note's body): a provider-less project's session trees, or a
/// provider project's under `allow_env_files`, hold env files.
#[must_use]
pub fn warning(files: &EnvFiles) -> String {
    format!(
        "env files in the session's trees: {}; the agent can read them",
        listed(files)
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::copy::excludes;
    use super::{EnvFiles, SCAN_CAP, is_env_file, listed, refused, scan, warning};

    /// An empty file at `root/rel`, its parents created.
    fn touch(root: &Path, rel: &str) {
        let full = root.join(rel);
        std::fs::create_dir_all(full.parent().expect("a parent")).expect("the parent is created");
        std::fs::write(&full, b"").expect("the file is written");
    }

    /// A finding list from string literals.
    fn found(paths: &[&str]) -> EnvFiles {
        EnvFiles {
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
            truncated: false,
        }
    }

    /// D1: the dotenv family and direnv's `.envrc`, case-insensitively, minus the six template
    /// suffixes compared whole; nothing that merely contains `env`.
    #[test]
    fn env_file_names_are_the_dotenv_family_and_direnv() {
        for name in [
            ".env",
            ".ENV",
            ".Env.Local",
            ".env.local",
            ".env.production",
            ".envrc",
            ".ENVRC",
            ".env.local.example",
        ] {
            assert!(is_env_file(name), "{name} is an env file");
        }
        for name in [
            ".env.example",
            ".env.sample",
            ".env.template",
            ".env.dist",
            ".env.defaults",
            ".env.schema",
            ".ENV.EXAMPLE",
            ".env.",
            "env",
            ".environment",
            "x.env",
            ".envrc.bak",
            ".venv",
        ] {
            assert!(!is_env_file(name), "{name} is not an env file");
        }
    }

    /// D2: a monorepo keeps `.env` per package, so the walk is recursive and names a nested file
    /// by its root-relative, `/`-separated path.
    #[test]
    fn a_nested_env_file_is_found_by_its_root_relative_path() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        for rel in [".env", "web/.env.local", "web/src/app.ts"] {
            touch(dir.path(), rel);
        }

        let files = scan(dir.path(), &[], &excludes(&[]), SCAN_CAP);

        assert_eq!(files.paths, [".env", "web/.env.local"]);
        assert!(!files.truncated);
    }

    /// D2: `.git` and the default build-directory list are never entered, at any depth.
    #[test]
    fn git_and_the_build_directories_are_pruned() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        for rel in [
            ".git/.env",
            "node_modules/pkg/.env",
            "target/.env",
            "crate/target/.env.local",
        ] {
            touch(dir.path(), rel);
        }

        assert_eq!(
            scan(dir.path(), &[], &excludes(&[]), SCAN_CAP),
            EnvFiles::default()
        );
    }

    /// D1: a directory named like an env file (a Python venv called `.env/`) is not a finding and
    /// is not descended into.
    #[test]
    fn a_directory_named_like_an_env_file_is_neither_found_nor_entered() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        touch(dir.path(), ".env/.env");
        touch(dir.path(), ".envrc/x");

        assert_eq!(
            scan(dir.path(), &[], &excludes(&[]), SCAN_CAP),
            EnvFiles::default()
        );
    }

    /// The exclude list prunes directories only: a project that keeps `.env*` out of its copies
    /// must not hide a real `.env` from the check in `local` mode.
    #[test]
    fn a_project_exclude_prunes_directories_only() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        touch(dir.path(), ".env");
        touch(dir.path(), ".envdir/.env");

        let files = scan(dir.path(), &[], &excludes(&[".env*".into()]), SCAN_CAP);

        assert_eq!(files.paths, [".env"]);
    }

    /// D1: a symlink named `.env` is a finding by its own name and is never followed, and a link
    /// named anything else does not lead the walk into the directory it points at.
    #[cfg(unix)]
    #[test]
    fn a_symlink_named_env_is_found_and_not_followed() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("root");
        std::fs::create_dir(&root).expect("the root is created");
        touch(dir.path(), "elsewhere/.env.production");
        touch(dir.path(), "other/.env");
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), root.join(".env"))
            .expect("the link is made");
        std::os::unix::fs::symlink(dir.path().join("other"), root.join("link"))
            .expect("the link is made");

        let files = scan(&root, &[], &excludes(&[]), SCAN_CAP);

        assert_eq!(files.paths, [".env"]);
    }

    /// D2: the cap cuts the walk and sets `truncated`; under the real cap the same tree is walked
    /// whole.
    #[test]
    fn the_cap_cuts_the_walk_and_says_so() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        for index in 0..10 {
            touch(dir.path(), &format!("a{index}"));
        }
        touch(dir.path(), ".env");

        assert!(scan(dir.path(), &[], &excludes(&[]), 3).truncated);

        let whole = scan(dir.path(), &[], &excludes(&[]), SCAN_CAP);
        assert_eq!(whole.paths, [".env"]);
        assert!(!whole.truncated);
    }

    /// D2: a root that does not exist answers nothing rather than an error.
    #[test]
    fn a_missing_root_answers_nothing() {
        let dir = tempfile::tempdir().expect("a temporary directory");

        let files = scan(
            &dir.path().join("gone"),
            &[dir.path().join("also-gone")],
            &excludes(&[]),
            SCAN_CAP,
        );

        assert_eq!(files, EnvFiles::default());
    }

    /// D7: a path under an `extra_dirs` root is prefixed with that root's last component, so two
    /// repos' `.env` files read apart; a `cwd` path is not.
    #[test]
    fn an_extra_dir_is_named_by_its_last_component() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let cwd = dir.path().join("cwd");
        touch(&cwd, ".env");
        touch(dir.path(), "other/docs/.envrc");
        let extra: Vec<PathBuf> = vec![dir.path().join("other/docs")];

        let files = scan(&cwd, &extra, &excludes(&[]), SCAN_CAP);

        assert_eq!(files.paths, [".env", "docs/.envrc"]);
    }

    /// D2: an unreadable directory is skipped with a `warn`, and the rest of the walk still
    /// answers.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_is_skipped() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("a temporary directory");
        touch(dir.path(), ".env");
        touch(dir.path(), "locked/.env.local");
        let locked = dir.path().join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
            .expect("the directory's mode is set");
        let restore = || {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
                .expect("the directory's mode is restored");
        };
        if std::fs::read_dir(&locked).is_ok() {
            // Running as root, which ignores the mode: nothing here is unreadable.
            restore();
            return;
        }

        let files = scan(dir.path(), &[], &excludes(&[]), SCAN_CAP);
        restore();

        assert_eq!(files.paths, [".env"]);
    }

    /// D7's `<list>`: five names, then a count; the cut marker last; a control character as `?`.
    #[test]
    fn the_list_names_five_then_counts_the_rest() {
        assert_eq!(listed(&found(&[".env"])), "`.env`");
        assert_eq!(
            listed(&found(&["a", "b", "c", "d", "e"])),
            "`a`, `b`, `c`, `d`, `e`"
        );
        assert_eq!(
            listed(&found(&["a", "b", "c", "d", "e", "f", "g"])),
            "`a`, `b`, `c`, `d`, `e` and 2 more"
        );
        assert_eq!(
            listed(&EnvFiles {
                truncated: true,
                ..found(&[".env"])
            }),
            "`.env` (scan cut at 100000 entries)"
        );
        assert_eq!(listed(&found(&["a\nb/.env"])), "`a?b/.env`");
    }

    /// D7's two sentences, byte for byte.
    #[test]
    fn the_sentences_are_d7s_bytes() {
        let files = found(&[".env", "web/.env.local"]);

        assert_eq!(
            refused("infisical", &files),
            "env_files_refused: `.env`, `web/.env.local` in the session's trees; the project \
             resolves its secrets from infisical. Remove them, or set allow_env_files."
        );
        assert_eq!(
            warning(&files),
            "env files in the session's trees: `.env`, `web/.env.local`; the agent can read them"
        );
    }
}
