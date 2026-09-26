//! `htui_orch::infer`'s walk over real directories (plan D113, blueprint D131).
//!
//! Every repository here is built by `isolate::git::testkit::repo_with_one_commit`, which uses
//! `gix` alone, and its remote is a `[remote "origin"]` section appended to `.git/config` by hand:
//! no case needs a `git` binary, so none is skipped.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use htui_orch::infer::{Checkout, Limits, Scan, find_checkouts, find_checkouts_with};
use htui_orch::isolate::git::testkit::repo_with_one_commit;

/// A real repository at `root/rel`, with `remote` as its `origin` when given.
fn checkout(root: &Path, rel: &str, remote: Option<&str>) -> PathBuf {
    let dir = root.join(rel);
    fs::create_dir_all(&dir).expect("the checkout's directory is created");
    repo_with_one_commit(&dir);
    if let Some(url) = remote {
        let mut config = fs::OpenOptions::new()
            .append(true)
            .open(dir.join(".git").join("config"))
            .expect("the repository's config opens");
        write!(
            config,
            "[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
        )
        .expect("the remote is written");
    }
    dir
}

/// A plain directory at `root/rel`.
fn plain(root: &Path, rel: &str) -> PathBuf {
    let dir = root.join(rel);
    fs::create_dir_all(&dir).expect("the directory is created");
    dir
}

/// The names of `scan`'s checkouts, in order.
fn names(scan: &Scan) -> Vec<&str> {
    scan.checkouts
        .iter()
        .map(|checkout| checkout.name.as_str())
        .collect()
}

#[test]
fn one_checkout_is_found_with_its_remote_key() {
    let root = tempfile::tempdir().expect("a tempdir");
    let path = checkout(root.path(), "core", Some("git@github.com:o/core"));

    let scan = find_checkouts(root.path());

    assert_eq!(
        scan,
        Scan {
            checkouts: vec![Checkout {
                path,
                name: "core".to_owned(),
                remote_keys: vec!["github.com/o/core".to_owned()],
            }],
            truncated: false,
        }
    );
}

#[test]
fn two_checkouts_and_none() {
    let root = tempfile::tempdir().expect("a tempdir");
    let docs = checkout(root.path(), "docs", None);
    let core = checkout(root.path(), "core", Some("https://github.com/o/core.git"));

    let scan = find_checkouts(root.path());
    assert!(!scan.truncated);
    assert_eq!(names(&scan), ["core", "docs"]);
    assert_eq!(scan.checkouts[0].path, core);
    assert_eq!(scan.checkouts[1].path, docs);
    assert_eq!(scan.checkouts[1].remote_keys, Vec::<String>::new());

    let empty = tempfile::tempdir().expect("a tempdir");
    assert_eq!(find_checkouts(empty.path()), Scan::default());
}

#[test]
fn a_checkout_at_depth_three_is_found_and_at_depth_four_is_not() {
    let root = tempfile::tempdir().expect("a tempdir");
    checkout(root.path(), "a/b/c", None);
    checkout(root.path(), "x/y/z/deep", None);

    let scan = find_checkouts(root.path());

    assert_eq!(names(&scan), ["c"]);
    assert_eq!(scan.checkouts[0].path, root.path().join("a/b/c"));
    assert!(!scan.truncated);
}

#[test]
fn a_checkout_inside_a_checkout_is_not_listed() {
    let root = tempfile::tempdir().expect("a tempdir");
    let outer = checkout(root.path(), "outer", None);
    checkout(root.path(), "outer/inner", None);

    let scan = find_checkouts(root.path());

    assert_eq!(names(&scan), ["outer"]);
    assert_eq!(scan.checkouts[0].path, outer);
}

#[cfg(unix)]
#[test]
fn a_symlinked_checkout_is_not_followed() {
    let root = tempfile::tempdir().expect("a tempdir");
    let elsewhere = tempfile::tempdir().expect("a tempdir");
    let real = checkout(elsewhere.path(), "real", Some("git@github.com:o/real"));
    let parent = checkout(elsewhere.path(), "holder/nested", None);
    std::os::unix::fs::symlink(&real, root.path().join("link")).expect("the link is made");
    std::os::unix::fs::symlink(
        parent.parent().expect("a parent"),
        root.path().join("dirlink"),
    )
    .expect("the link is made");

    assert_eq!(find_checkouts(root.path()), Scan::default());
}

#[test]
fn a_dot_directory_is_skipped() {
    let root = tempfile::tempdir().expect("a tempdir");
    checkout(root.path(), ".cache/core", None);
    checkout(root.path(), ".hidden", None);

    assert_eq!(find_checkouts(root.path()), Scan::default());
}

#[test]
fn a_gitdir_file_checkout_is_listed() {
    let root = tempfile::tempdir().expect("a tempdir");
    let elsewhere = tempfile::tempdir().expect("a tempdir");
    let real = checkout(elsewhere.path(), "real", None);
    let wt = plain(root.path(), "wt");
    fs::write(
        wt.join(".git"),
        format!("gitdir: {}\n", real.join(".git").display()),
    )
    .expect("the gitdir file is written");

    let scan = find_checkouts(root.path());

    assert_eq!(names(&scan), ["wt"]);
    assert_eq!(scan.checkouts[0].path, wt);
}

#[test]
fn the_order_is_byte_order_at_every_level() {
    let root = tempfile::tempdir().expect("a tempdir");
    checkout(root.path(), "a", None);
    checkout(root.path(), "a/inner", None);
    checkout(root.path(), "B", None);
    checkout(root.path(), "z/y", None);
    checkout(root.path(), "z/X", None);

    let first = find_checkouts(root.path());
    let second = find_checkouts(root.path());

    assert_eq!(names(&first), ["B", "a", "X", "y"]);
    assert_eq!(first, second);
}

#[test]
fn the_directory_cap_sets_truncated() {
    let root = tempfile::tempdir().expect("a tempdir");
    for name in ["d1", "d2", "d3", "d4", "d5"] {
        plain(root.path(), name);
    }
    let small = Limits {
        max_depth: 3,
        max_dirs: 3,
    };

    assert!(find_checkouts_with(root.path(), small).truncated);
    assert!(
        !find_checkouts_with(
            root.path(),
            Limits {
                max_depth: 3,
                max_dirs: 6
            }
        )
        .truncated,
        "the root and five siblings fit in six"
    );
    assert!(!find_checkouts(root.path()).truncated);
}

#[test]
fn the_root_itself_may_be_a_checkout() {
    let root = tempfile::tempdir().expect("a tempdir");
    checkout(root.path(), "", Some("https://github.com/o/root"));
    checkout(root.path(), "below", None);

    let scan = find_checkouts(root.path());

    assert_eq!(scan.checkouts.len(), 1, "{scan:?}");
    assert_eq!(scan.checkouts[0].path, root.path());
    assert_eq!(scan.checkouts[0].remote_keys, ["github.com/o/root"]);
    assert!(!scan.truncated);
    // Nothing below the root is examined, so a cap of one directory is enough.
    let one = Limits {
        max_depth: 3,
        max_dirs: 1,
    };
    assert_eq!(find_checkouts_with(root.path(), one), scan);
}

#[test]
fn an_unopenable_checkout_has_no_remote_keys() {
    let root = tempfile::tempdir().expect("a tempdir");
    let broken = plain(root.path(), "broken");
    plain(root.path(), "broken/.git");

    let scan = find_checkouts(root.path());

    assert_eq!(
        scan.checkouts,
        [Checkout {
            path: broken,
            name: "broken".to_owned(),
            remote_keys: Vec::new(),
        }]
    );
}

#[test]
fn a_remote_with_a_token_leaves_only_its_key() {
    let root = tempfile::tempdir().expect("a tempdir");
    checkout(root.path(), "r", Some("https://user:tok@github.com/o/r"));

    let scan = find_checkouts(root.path());

    assert_eq!(names(&scan), ["r"]);
    assert_eq!(scan.checkouts[0].remote_keys, ["github.com/o/r"]);
    // The tempdir's own path is ours and may contain anything, so it is masked first (H-7).
    let root_text = root.path().to_str().expect("a UTF-8 tempdir");
    let shown = format!("{scan:?}").replace(root_text, "<root>");
    assert!(!shown.contains("tok"), "{shown}");
    assert!(!shown.contains("user"), "{shown}");
}
