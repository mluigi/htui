//! MOD-7 milestone 4 (T4) against live Postgres: `InferRepoPaths` served by
//! `htui::store_worker::serve` over a `Backend::Online`.
//!
//! `tests/hierarchy.rs` proves the inference over a `MemStore`. What only this file can prove is
//! what Postgres does with the rows: a manual `upsert_repo_box_path` row is reported `AlreadySet`
//! and left exactly as it was (the insert-if-absent statement's conflict clause itself is pinned by
//! the store conformance case over `PgStore`), and the canonical path the worker computed
//! round-trips through the `TEXT` column byte for byte. The row's box is compared with what
//! `backend.box_info()` answers rather than with a constant (blueprint D144): the demo database
//! repoints `this_box` at the fixture, and this file does not rely on which id that is.
//!
//! The stack is `templates_pg.rs`'s without the shell: a throwaway database with the demo world
//! (`testkit::demo_db`), a throwaway mirror (`CacheStore`) and `Backend::Online` over the two. No
//! keyring guard: nothing on this path reads `secret::`.
//!
//! The checkouts are real repositories built by `repo_with_one_commit` with `gix` alone, each
//! remote a `[remote "origin"]` section appended to `.git/config` by hand: no `git` binary.
//!
//! Each case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics
//! instead when `CI` is set, like every other Postgres-backed suite. The cases write files under a
//! temporary root, so they are unix-only, as the other `_pg` suites that touch the disk are.
#![cfg(feature = "testkit")]
#![cfg(unix)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use htui::hierarchy::{HierarchySnapshot, InferOutcome, InferReport};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_core::fixtures::ids;
use htui_core::model::RepoId;
use htui_core::store::WriteStore as _;
use htui_orch::infer::MatchedBy;
use htui_orch::isolate::git::testkit::repo_with_one_commit;
use htui_store::{Backend, CacheStore, PgStore, testkit};

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

/// `path`, canonical, as the worker stores it.
fn canonical(path: &Path) -> String {
    path.canonicalize()
        .expect("the path resolves")
        .into_os_string()
        .into_string()
        .expect("a tempdir path is UTF-8")
}

/// The tree a reply carries, or a panic naming what came back instead.
#[track_caller]
fn tree(reply: StoreReply) -> HierarchySnapshot {
    match reply {
        StoreReply::Hierarchy(Some(tree)) => *tree,
        other => panic!("expected a tree: {other:?}"),
    }
}

/// The report an inference answers, or a panic naming what came back instead.
#[track_caller]
fn report(reply: StoreReply) -> InferReport {
    match reply {
        StoreReply::RepoPathsInferred { report, .. } => report,
        other => panic!("expected an inference: {other:?}"),
    }
}

/// What the report says about the repo named `name`.
#[track_caller]
fn outcome<'a>(report: &'a InferReport, name: &str) -> &'a InferOutcome {
    &report
        .repos
        .iter()
        .find(|line| line.name == name)
        .unwrap_or_else(|| panic!("`{name}` is in the report: {report:?}"))
        .outcome
}

/// A repo named `name` in `vulkan-tutorials`, created through the worker; its id.
async fn create_repo(backend: &Backend, name: &str, remote: Option<&str>) -> RepoId {
    let created = tree(
        serve(
            backend,
            &StoreRequest::CreateRepo {
                project: ids::PROJECT_VULKAN,
                name: name.to_owned(),
                remote_url: remote.map(str::to_owned),
                default_branch: "main".to_owned(),
                is_primary: false,
            },
        )
        .await,
    );
    created.projects[0]
        .repos
        .iter()
        .find(|entry| entry.repo.name == name)
        .expect("the new repo is in the tree")
        .repo
        .id
}

/// One inference writes one canonical row on the server, under this box, and leaves the manual
/// row beside it untouched; a second pass writes nothing at all.
#[tokio::test]
async fn inference_writes_one_row_and_keeps_a_manual_one_on_postgres() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let mirror = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(mirror.path(), "hierarchy-pg", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let backend = Backend::Online {
        pg: db.store.clone(),
        cache: cache.clone(),
    };

    let root = tempfile::tempdir().expect("a throwaway workspace root");
    let core_dir = checkout(root.path(), "core", Some("git@example.com:o/core"));
    checkout(root.path(), "docs", None);
    let manual = root.path().join("manual");
    fs::create_dir(&manual).expect("the manual path exists");

    let core = create_repo(&backend, "core", Some("https://example.com/o/core.git")).await;
    let docs = create_repo(&backend, "docs", None).await;
    let _ = tree(
        serve(
            &backend,
            &StoreRequest::SetWorkspaceRoot {
                id: ids::WORKSPACE_GRAPHICS,
                path: root.path().display().to_string(),
            },
        )
        .await,
    );
    let _ = tree(
        serve(
            &backend,
            &StoreRequest::SetRepoPath {
                project: ids::PROJECT_VULKAN,
                repo: docs,
                path: manual.display().to_string(),
            },
        )
        .await,
    );

    let first = report(
        serve(
            &backend,
            &StoreRequest::InferRepoPaths(ids::WORKSPACE_GRAPHICS),
        )
        .await,
    );
    assert_eq!(first.root, Some(canonical(root.path())));
    assert_eq!(
        outcome(&first, "core"),
        &InferOutcome::Inferred {
            path: canonical(&core_dir),
            by: MatchedBy::Remote,
        }
    );
    assert_eq!(outcome(&first, "docs"), &InferOutcome::AlreadySet);

    let this_box = backend
        .box_info()
        .await
        .expect("the server answers")
        .expect("the demo database has this box")
        .box_id;
    let core_rows = db
        .store
        .repo_box_paths(core)
        .await
        .expect("the server answers");
    assert_eq!(core_rows.len(), 1, "exactly one row: {core_rows:?}");
    assert_eq!(
        core_rows[0].local_path,
        canonical(&core_dir),
        "the canonical path round-trips through `TEXT`"
    );
    assert_eq!(core_rows[0].box_id, this_box, "the row is this box's");
    let docs_rows = db
        .store
        .repo_box_paths(docs)
        .await
        .expect("the server answers");
    assert_eq!(
        docs_rows
            .iter()
            .map(|row| row.local_path.as_str())
            .collect::<Vec<_>>(),
        vec![canonical(&manual).as_str()],
        "the manual row is untouched"
    );

    let second = report(
        serve(
            &backend,
            &StoreRequest::InferRepoPaths(ids::WORKSPACE_GRAPHICS),
        )
        .await,
    );
    assert_eq!(outcome(&second, "core"), &InferOutcome::AlreadySet);
    assert_eq!(outcome(&second, "docs"), &InferOutcome::AlreadySet);
    assert_eq!(
        db.store
            .repo_box_paths(core)
            .await
            .expect("the server answers"),
        core_rows,
        "a second pass writes nothing"
    );
    assert_eq!(
        db.store
            .repo_box_paths(docs)
            .await
            .expect("the server answers"),
        docs_rows,
        "and leaves the manual row as it was"
    );

    cache.close().await;
    db.drop_db().await;
}
