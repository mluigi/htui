//! Repo-path inference's pure half (PRD D5; plan D111–D113, D123): remote-URL normalisation, a
//! bounded checkout walk, and the remote-first, name-second choice.
//!
//! Nothing here writes, and nothing here touches a store. `crate::hierarchy` in `crates/htui`
//! serves the request: it resolves this box's workspace root, runs [`find_checkouts`] under
//! `spawn_blocking`, calls [`choose`] once per repo with no row on this box, and writes each single
//! match through the insert-if-absent writer.
//!
//! The walk never follows a link, so every candidate under a canonical root is itself canonical
//! (F-102 by construction). A truncated scan infers nothing, because an unseen second clone would
//! turn a wrong single match into a write. Remote URLs never leave this module: a fetch URL can
//! carry `user:token@`, so [`Checkout`] carries normalised keys only, and a key holds no userinfo.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use htui_core::model::Repo;

/// How deep below the workspace root a checkout is looked for; the root itself is depth 0 and may
/// be a checkout (plan D113).
pub const MAX_DEPTH: usize = 3;
/// How many directories one scan examines before it stops and reports `truncated` (plan D113).
pub const MAX_DIRS: usize = 5_000;

/// The walk's two bounds. [`find_checkouts`] uses [`Limits::DEFAULT`]; tests pass smaller ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Deepest level descended to, the root being 0.
    pub max_depth: usize,
    /// Directories examined before the scan stops, the root included.
    pub max_dirs: usize,
}

impl Limits {
    /// [`MAX_DEPTH`] and [`MAX_DIRS`].
    pub const DEFAULT: Self = Self {
        max_depth: MAX_DEPTH,
        max_dirs: MAX_DIRS,
    };
}

/// One git checkout found under the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The directory holding `.git`, under the canonical root, reached without following a link.
    pub path: PathBuf,
    /// Its file name, which the name rung compares with `repo.name` byte for byte.
    pub name: String,
    /// Every configured remote's fetch URL, **normalised** ([`normalise_remote`]), sorted and
    /// deduplicated. Never a raw URL: a fetch URL can carry `user:token@`. Empty when the checkout
    /// has no remote, or `gix` could not open it.
    pub remote_keys: Vec<String>,
}

/// What one scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    /// Every checkout, in depth-first order with each level sorted by file-name bytes.
    pub checkouts: Vec<Checkout>,
    /// The scan stopped at [`Limits::max_dirs`]. The caller must infer nothing (plan D113).
    pub truncated: bool,
}

/// Which rung chose a checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedBy {
    /// A remote's normalised key equals the repo's.
    Remote,
    /// The directory name equals `repo.name`, and no remote contradicts it (plan OQ-31).
    Name,
}

impl MatchedBy {
    /// `remote` or `name`, the word the Hierarchy section's notice uses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Name => "name",
        }
    }
}

/// [`choose`]'s answer for one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Exactly one candidate at the first rung that had any.
    Inferred {
        /// The checkout's path, not yet re-canonicalised.
        path: PathBuf,
        /// Which rung.
        by: MatchedBy,
    },
    /// Two or more candidates at the deciding rung: nothing is written (PRD D5).
    Ambiguous {
        /// How many.
        candidates: usize,
    },
    /// No candidate at either rung.
    NoMatch,
}

/// The comparison key of a remote URL (plan D111), or `None` for an empty or unparseable one.
///
/// `https://github.com/o/r.git`, `git@github.com:o/r`, `ssh://git@github.com:22/o/r/` and
/// `https://user:tok@GitHub.com/o/r` are all `github.com/o/r`. The host is lowercased and the path
/// is kept byte-exact, so a case mismatch only falls through to the name rung, which is the safe
/// direction. The key never carries userinfo: `gix` keeps `user:tok@` in a fetch URL, so this
/// strips it itself.
#[must_use]
pub fn normalise_remote(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let s = trimmed.trim_end_matches('/');
    if s.is_empty() {
        return None;
    }

    if let Some((scheme, rest)) = s.split_once("://") {
        return match scheme.to_ascii_lowercase().as_str() {
            "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git" => network_key(rest),
            "file" => file_key(rest),
            _ => None,
        };
    }
    if s.starts_with('/') {
        return file_key(s);
    }
    if has_drive(s) {
        return file_key(&s.replace('\\', "/"));
    }
    scp_key(s)
}

/// `host/path` from a scheme URL's text after `://`: userinfo and a numeric port dropped, the host
/// lowercased, the path's repeated `/` collapsed and its leading `/` removed.
fn network_key(rest: &str) -> Option<String> {
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    // Userinfo ends at the last `@`: everything up to it, credentials included, is dropped here.
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = match host_port.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => host,
        _ => host_port,
    };
    host_path(host, path)
}

/// `host/path` from an scp-like `[user@]host:path`, or `None` when the text is not one: a `:` must
/// come before any `/`.
fn scp_key(s: &str) -> Option<String> {
    let (before, path) = s.split_once(':')?;
    if before.contains('/') {
        return None;
    }
    let host = before.rsplit_once('@').map_or(before, |(_, host)| host);
    host_path(host, path)
}

/// `host/path`, the host lowercased and the path collapsed without its leading `/`; `None` when
/// either is empty.
fn host_path(host: &str, path: &str) -> Option<String> {
    let path = collapse_slashes(path);
    let path = path.trim_start_matches('/');
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{}/{path}", host.to_ascii_lowercase()))
}

/// `file:` and the collapsed path. A `/` in front of a drive (`file:///C:/…`) is dropped so the
/// URL and the bare drive path share a key.
fn file_key(path: &str) -> Option<String> {
    let path = collapse_slashes(path);
    let path = match path.strip_prefix('/') {
        Some(rest) if has_drive(rest) => rest,
        _ => path.as_str(),
    };
    if path.is_empty() || path == "/" {
        return None;
    }
    Some(format!("file:{path}"))
}

/// `^[A-Za-z]:[\\/]`: a Windows drive path.
fn has_drive(s: &str) -> bool {
    matches!(
        s.as_bytes(),
        [letter, b':', b'\\' | b'/', ..] if letter.is_ascii_alphabetic()
    )
}

/// `s` with every run of `/` folded to one.
fn collapse_slashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut previous_slash = false;
    for ch in s.chars() {
        let slash = ch == '/';
        if !(slash && previous_slash) {
            out.push(ch);
        }
        previous_slash = slash;
    }
    out
}

/// `find_checkouts_with(root, Limits::DEFAULT)`. Synchronous `std::fs` plus one `gix::open` per
/// checkout: call it under `tokio::task::spawn_blocking` (plan D113, R-45).
#[must_use]
pub fn find_checkouts(root: &Path) -> Scan {
    find_checkouts_with(root, Limits::DEFAULT)
}

/// The checkouts under `root`, depth-first, each level sorted by file-name bytes (the
/// `FsRepoReader` walk's determinism rule).
///
/// A directory holding a `.git` entry (a directory, or a file beginning `gitdir:`) is a checkout
/// and is **not** descended into, so a nested checkout is never listed. Entries whose name starts
/// with `.` are skipped. Links are never followed (`symlink_metadata`). An unreadable directory
/// contributes nothing, and a name that is not UTF-8 is skipped. Every directory examined, the root
/// included, counts towards `limits.max_dirs`, and the first one past it stops the scan with
/// `truncated`.
#[must_use]
pub fn find_checkouts_with(root: &Path, limits: Limits) -> Scan {
    let _ = (root, limits);
    todo!()
}

/// Remote first, name second, ambiguity is failure (PRD D5, plan D112), for one repo with no row on
/// this box.
///
/// Paths in `held` (other repos' rows on this box) are excluded before either rung. Rung 1, when
/// `repo.remote_url` normalises to a key: the candidates with **any** remote key equal to it. One
/// is `Inferred { by: Remote }`, several are `Ambiguous`, and none falls to rung 2. Rung 2: the
/// candidates whose name equals `repo.name` byte for byte. When the repo has a key, only candidates
/// with **no** remote are accepted: a same-named checkout whose remotes all normalise elsewhere is
/// a fork or an unrelated project (OQ-31). One is `Inferred { by: Name }`, several `Ambiguous`,
/// none `NoMatch`. A `remote_url` that does not normalise counts as none.
#[must_use]
pub fn choose(repo: &Repo, checkouts: &[Checkout], held: &BTreeSet<PathBuf>) -> Choice {
    let free: Vec<&Checkout> = checkouts
        .iter()
        .filter(|checkout| !held.contains(&checkout.path))
        .collect();
    let key = repo.remote_url.as_deref().and_then(normalise_remote);

    if let Some(key) = &key {
        let by_remote: Vec<&Checkout> = free
            .iter()
            .copied()
            .filter(|checkout| checkout.remote_keys.iter().any(|theirs| theirs == key))
            .collect();
        if let Some(choice) = decide(&by_remote, MatchedBy::Remote) {
            return choice;
        }
    }

    // OQ-31: when the repo has a key, a same-named checkout with any remote is one whose remotes
    // all normalise elsewhere (rung 1 found none equal), so it is a fork or another project.
    let by_name: Vec<&Checkout> = free
        .iter()
        .copied()
        .filter(|checkout| checkout.name == repo.name)
        .filter(|checkout| key.is_none() || checkout.remote_keys.is_empty())
        .collect();
    decide(&by_name, MatchedBy::Name).unwrap_or(Choice::NoMatch)
}

/// One rung's verdict: `None` when it had no candidate, so the next rung decides.
fn decide(candidates: &[&Checkout], by: MatchedBy) -> Option<Choice> {
    match candidates {
        [] => None,
        [only] => Some(Choice::Inferred {
            path: only.path.clone(),
            by,
        }),
        several => Some(Choice::Ambiguous {
            candidates: several.len(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use htui_core::fixtures::ids;
    use htui_core::model::{Repo, RepoId};
    use uuid::Uuid;

    use super::{Checkout, Choice, MatchedBy, choose, normalise_remote};

    /// The key every spelling of `o/r` on GitHub folds to.
    const KEY: &str = "github.com/o/r";

    /// A repo named `name` with `remote_url`.
    fn repo(name: &str, remote_url: Option<&str>) -> Repo {
        let at = chrono::DateTime::UNIX_EPOCH;
        Repo {
            id: RepoId::from_uuid(Uuid::from_u128(1)),
            project_id: ids::PROJECT_HTUI,
            name: name.to_owned(),
            remote_url: remote_url.map(str::to_owned),
            default_branch: "main".to_owned(),
            is_primary: false,
            created_at: at,
            updated_at: at,
        }
    }

    /// A checkout at `/w/<dir>/<name>` whose remotes normalise to `keys`.
    fn checkout(dir: &str, name: &str, keys: &[&str]) -> Checkout {
        Checkout {
            path: PathBuf::from(format!("/w/{dir}/{name}")),
            name: name.to_owned(),
            remote_keys: keys.iter().map(|key| (*key).to_owned()).collect(),
        }
    }

    #[test]
    fn normalise_remote_folds_the_four_spellings_of_one_repo() {
        for url in [
            "https://github.com/o/r.git",
            "git@github.com:o/r",
            "ssh://git@github.com:22/o/r/",
            "https://user:tok@GitHub.com/o/r",
            "http://github.com/o/r",
            "git://github.com/o/r.git",
            "git+ssh://git@github.com/o/r",
            "https://github.com//o//r/",
            "  HTTPS://github.com/o/r.git/  ",
        ] {
            assert_eq!(normalise_remote(url).as_deref(), Some(KEY), "{url}");
        }
    }

    #[test]
    fn normalise_remote_keeps_other_hosts_and_paths_apart() {
        for (url, key) in [
            ("https://gitlab.com/o/r", "gitlab.com/o/r"),
            ("git@github.com:o/other.git", "github.com/o/other"),
            ("https://github.com/O/R", "github.com/O/R"),
        ] {
            let got = normalise_remote(url);
            assert_eq!(got.as_deref(), Some(key), "{url}");
            assert_ne!(got.as_deref(), Some(KEY), "{url}");
        }
    }

    #[test]
    fn normalise_remote_refuses_empty_and_unparseable() {
        for url in [
            "",
            "   ",
            "relative/dir",
            "bare",
            "ftp://h/o/r",
            "https://github.com",
            "https:///o/r",
            "git@github.com:",
            "/",
        ] {
            assert_eq!(normalise_remote(url), None, "{url:?}");
        }
    }

    #[test]
    fn normalise_remote_never_keeps_credentials() {
        for url in [
            "https://user:tok@github.com/o/r",
            "ssh://me:pw@host:2222/o/r",
        ] {
            let key = normalise_remote(url).expect("the URL normalises");
            for secret in ["user", "tok", "me", "pw", "@"] {
                assert!(!key.contains(secret), "{secret:?} survived in a key");
            }
        }
        assert_eq!(
            normalise_remote("ssh://me:pw@host:2222/o/r").as_deref(),
            Some("host/o/r")
        );
    }

    #[test]
    fn normalise_remote_maps_local_paths_to_file_keys() {
        for url in ["/srv/git/r.git", "file:///srv/git/r", "/srv//git/r/"] {
            assert_eq!(
                normalise_remote(url).as_deref(),
                Some("file:/srv/git/r"),
                "{url}"
            );
        }
        assert_eq!(
            normalise_remote(r"C:\git\r").as_deref(),
            Some("file:C:/git/r")
        );
        assert_eq!(
            normalise_remote("file:///C:/git/r.git").as_deref(),
            Some("file:C:/git/r")
        );
    }

    #[test]
    fn choose_takes_the_one_remote_match() {
        let a = checkout("a", "core", &[KEY]);
        let b = checkout("b", "core", &["github.com/o/other"]);
        let choice = choose(
            &repo("r", Some("https://github.com/o/r")),
            &[a.clone(), b],
            &BTreeSet::new(),
        );
        assert_eq!(
            choice,
            Choice::Inferred {
                path: a.path,
                by: MatchedBy::Remote
            }
        );
    }

    #[test]
    fn choose_refuses_two_remote_matches_without_falling_back_to_the_name() {
        let named = checkout("a", "r", &[KEY]);
        let other = checkout("b", "clone", &["gitlab.com/x/y", KEY]);
        let choice = choose(
            &repo("r", Some("git@github.com:o/r.git")),
            &[named, other],
            &BTreeSet::new(),
        );
        assert_eq!(choice, Choice::Ambiguous { candidates: 2 });
    }

    #[test]
    fn choose_falls_back_to_the_name_when_no_remote_matches() {
        let named = checkout("a", "r", &[]);
        let other = checkout("b", "else", &["github.com/o/else"]);
        let choice = choose(
            &repo("r", Some("https://github.com/o/r")),
            &[other, named.clone()],
            &BTreeSet::new(),
        );
        assert_eq!(
            choice,
            Choice::Inferred {
                path: named.path,
                by: MatchedBy::Name
            }
        );
    }

    #[test]
    fn choose_rejects_a_name_match_whose_remote_contradicts() {
        let fork = checkout("a", "r", &["github.com/fork/r"]);
        let choice = choose(
            &repo("r", Some("https://github.com/o/r")),
            &[fork],
            &BTreeSet::new(),
        );
        assert_eq!(choice, Choice::NoMatch);
    }

    #[test]
    fn choose_accepts_a_name_match_for_a_repo_with_no_remote_url() {
        let named = checkout("a", "r", &["github.com/anyone/r"]);
        let other = checkout("b", "R", &[]);
        for remote_url in [None, Some("not a url")] {
            let choice = choose(
                &repo("r", remote_url),
                &[named.clone(), other.clone()],
                &BTreeSet::new(),
            );
            assert_eq!(
                choice,
                Choice::Inferred {
                    path: named.path.clone(),
                    by: MatchedBy::Name
                },
                "{remote_url:?}"
            );
        }
    }

    #[test]
    fn choose_refuses_two_name_matches() {
        let one = checkout("a", "r", &[]);
        let two = checkout("b", "r", &[]);
        let choice = choose(
            &repo("r", Some("https://github.com/o/r")),
            &[one, two],
            &BTreeSet::new(),
        );
        assert_eq!(choice, Choice::Ambiguous { candidates: 2 });
    }

    #[test]
    fn choose_excludes_a_path_another_repo_holds() {
        let held_match = checkout("a", "r", &[KEY]);
        let held = BTreeSet::from([held_match.path.clone()]);
        let choice = choose(
            &repo("r", Some("https://github.com/o/r")),
            &[held_match],
            &held,
        );
        assert_eq!(choice, Choice::NoMatch);
    }

    #[test]
    fn matched_by_names_its_rung() {
        assert_eq!(MatchedBy::Remote.as_str(), "remote");
        assert_eq!(MatchedBy::Name.as_str(), "name");
    }
}
