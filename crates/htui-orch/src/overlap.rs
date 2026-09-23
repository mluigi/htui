//! ANA-2 §4.7's resolution half (plan D81): `touched_paths` + the project's repos + the snapshot's
//! per-phase isolation → `(run.repo_scope, RunScope)`. The predicate is `htui_core`'s (D80).
//!
//! [`resolve`] runs once, at `StartRun`, inside [`crate::graph::resolve`], and its answer is
//! written into `GraphSnapshot.scope` (D79). Both stores' `claim_run` read that scope back through
//! `htui_core::model::scope_of` and never resolve a path themselves, so this module is the only
//! place a `touched_paths` glob meets a `repo` row (plan D119).
//!
//! It is deliberately **not** re-exported at the crate root (blueprint F-G): `graph::resolve`
//! already owns that name there, so callers write `htui_orch::overlap::resolve`.

use std::collections::BTreeMap;

use htui_core::model::{Isolation, Item, Repo, RepoId, RepoScope, RunScope, SnapshotPhase};
use htui_core::prompt::excerpt::PathPrefix;

use crate::graph::ResolveError;

/// `(run.repo_scope, GraphSnapshot.scope)` for an item (ANA-2 §4.7, plan D119).
///
/// Each `touched_paths` entry is parsed by `PathPrefix::parse` against the primary repo's name. A
/// bare glob, or one qualified with the primary's name, belongs to the primary; a qualified glob
/// belongs to the repo of that name. With no primary, a bare glob maps nowhere and is dropped.
///
/// `requested` `None` derives `repo_scope`: every repo an entry names, plus the primary when no
/// glob is declared at all, in `RepoId` order. `Some(scope)` is kept **as given**, order included,
/// and only the entries of its repos contribute prefixes.
///
/// Per repo of `repo_scope`: `isolated` when every phase is `worktree | copy`, `local` when any
/// phase is `local`, and `prefixes` its entries' prefixes, deduplicated and sorted (`[]`, the whole
/// repo, when it has none). The judge's synthesized phase is not a snapshot phase and is not an
/// input.
///
/// # Errors
/// [`ResolveError::UnknownTouchedRepo`] for a qualifier no project repo carries, with or without
/// `requested`; [`ResolveError::EmptyScopeWithPrimary`] (D14, kept) for `Some([])` on a project
/// with an `is_primary` repo.
pub fn resolve(
    item: &Item,
    repos: &[Repo],
    phases: &[SnapshotPhase],
    requested: Option<&[RepoId]>,
) -> Result<(Vec<RepoId>, RunScope), ResolveError> {
    let _ = (item, repos, phases, requested);
    todo!("T4: overlap::resolve")
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{
        GraphSnapshot, Isolation, Item, Repo, RepoId, RepoScope, RunScope, SnapshotPhase,
    };
    use uuid::Uuid;

    use super::resolve;
    use crate::graph::ResolveError;

    /// [`GraphSnapshot`] under another name, so no struct literal of it appears here (D106's
    /// pattern in `recover.rs`).
    type Snapshot = GraphSnapshot;

    /// A repo whose id sorts by `n`.
    fn repo(n: u128, name: &str, is_primary: bool) -> Repo {
        let at = chrono::DateTime::UNIX_EPOCH;
        Repo {
            id: RepoId::from_uuid(Uuid::from_u128(n)),
            project_id: ids::PROJECT_HTUI,
            name: name.to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary,
            created_at: at,
            updated_at: at,
        }
    }

    /// `core` (primary) and `docs`, with `core`'s id the lower.
    fn two_repos() -> Vec<Repo> {
        vec![repo(1, "core", true), repo(2, "docs", false)]
    }

    /// `HTUI_FEAT-1` declaring `paths`.
    fn item(paths: &[&str]) -> Item {
        let mut item = demo_data()
            .items
            .into_iter()
            .find(|row| row.id == ids::HTUI_FEAT_1)
            .expect("the fixture holds HTUI_FEAT-1");
        item.touched_paths = paths.iter().map(|path| (*path).to_owned()).collect();
        item
    }

    /// One phase per isolation, each a copy of the `feature` graph's first phase.
    fn phases(isolations: &[Isolation]) -> Vec<SnapshotPhase> {
        let run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        let snapshot: Snapshot =
            serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
                .expect("the fixture snapshot is a `GraphSnapshot`");
        isolations
            .iter()
            .map(|isolation| SnapshotPhase {
                isolation: *isolation,
                ..snapshot.phases[0].clone()
            })
            .collect()
    }

    /// Every phase `worktree`, the seeded default.
    fn worktree() -> Vec<SnapshotPhase> {
        phases(&[Isolation::Worktree, Isolation::Worktree])
    }

    fn entry(isolated: bool, local: bool, prefixes: &[&str]) -> RepoScope {
        RepoScope {
            isolated,
            local,
            prefixes: prefixes.iter().map(|prefix| (*prefix).to_owned()).collect(),
        }
    }

    fn scope(entries: &[(&Repo, RepoScope)]) -> RunScope {
        RunScope {
            repos: entries
                .iter()
                .map(|(repo, entry)| (repo.id, entry.clone()))
                .collect(),
        }
    }

    #[test]
    fn a_bare_glob_is_the_primary_repo() {
        let repos = two_repos();
        let (repo_scope, run_scope) =
            resolve(&item(&["src/**/*.rs"]), &repos, &worktree(), None).expect("resolves");
        assert_eq!(repo_scope, vec![repos[0].id]);
        assert_eq!(
            run_scope,
            scope(&[(&repos[0], entry(true, false, &["src/"]))])
        );

        // Qualifying with the primary's own name is the same entry.
        let (repo_scope, run_scope) =
            resolve(&item(&["core:src/**/*.rs"]), &repos, &worktree(), None).expect("resolves");
        assert_eq!(repo_scope, vec![repos[0].id]);
        assert_eq!(
            run_scope,
            scope(&[(&repos[0], entry(true, false, &["src/"]))])
        );
    }

    #[test]
    fn a_qualified_glob_names_its_repo() {
        let repos = two_repos();
        let (repo_scope, run_scope) = resolve(
            &item(&["docs:guide/**", "src/lib.rs"]),
            &repos,
            &worktree(),
            None,
        )
        .expect("resolves");
        assert_eq!(repo_scope, vec![repos[0].id, repos[1].id], "RepoId order");
        assert_eq!(
            run_scope,
            scope(&[
                (&repos[0], entry(true, false, &["src/lib.rs"])),
                (&repos[1], entry(true, false, &["guide/"])),
            ])
        );

        // Only a qualified entry: the primary is not in the scope.
        let (repo_scope, run_scope) =
            resolve(&item(&["docs:guide/**"]), &repos, &worktree(), None).expect("resolves");
        assert_eq!(repo_scope, vec![repos[1].id]);
        assert_eq!(
            run_scope,
            scope(&[(&repos[1], entry(true, false, &["guide/"]))])
        );

        // No primary: the bare glob maps nowhere and is dropped, the qualified one still lands.
        let docs = vec![repo(2, "docs", false)];
        let (repo_scope, run_scope) = resolve(
            &item(&["src/**", "docs:guide/**"]),
            &docs,
            &worktree(),
            None,
        )
        .expect("resolves");
        assert_eq!(repo_scope, vec![docs[0].id]);
        assert_eq!(
            run_scope,
            scope(&[(&docs[0], entry(true, false, &["guide/"]))])
        );
    }

    #[test]
    fn an_unknown_repo_name_is_refused() {
        let repos = two_repos();
        let item = item(&["src/**", "web:app/**"]);
        let expected = ResolveError::UnknownTouchedRepo {
            item: item.id,
            name: "web".to_owned(),
        };
        assert_eq!(
            expected.to_string(),
            format!(
                "item {} touches repo `web`, which the project does not carry (ANA-2 §4.7 `:1025`)",
                item.id
            )
        );
        assert_eq!(
            resolve(&item, &repos, &worktree(), None),
            Err(expected.clone())
        );
        // The name is wrong regardless of the scope the caller asked for (D119).
        assert_eq!(
            resolve(&item, &repos, &worktree(), Some(&[repos[0].id])),
            Err(expected)
        );
    }

    #[test]
    fn no_declaration_is_the_whole_primary_repo() {
        let repos = two_repos();
        let (repo_scope, run_scope) =
            resolve(&item(&[]), &repos, &worktree(), None).expect("resolves");
        assert_eq!(repo_scope, vec![repos[0].id]);
        assert_eq!(run_scope, scope(&[(&repos[0], entry(true, false, &[]))]));
        assert!(run_scope.repos[&repos[0].id].prefixes.is_empty());
    }

    #[test]
    fn a_requested_scope_is_honoured_and_its_paths_are_filtered() {
        let repos = two_repos();
        let declared = item(&["docs:guide/**", "src/**", "docs:api/x.md", "src/**/*.rs"]);

        // Order kept as given, prefixes deduplicated and sorted.
        let (repo_scope, run_scope) = resolve(
            &declared,
            &repos,
            &worktree(),
            Some(&[repos[1].id, repos[0].id]),
        )
        .expect("resolves");
        assert_eq!(repo_scope, vec![repos[1].id, repos[0].id]);
        assert_eq!(
            run_scope,
            scope(&[
                (&repos[0], entry(true, false, &["src/"])),
                (&repos[1], entry(true, false, &["api/x.md", "guide/"])),
            ])
        );

        // A repo outside the requested scope contributes nothing.
        let (repo_scope, run_scope) =
            resolve(&declared, &repos, &worktree(), Some(&[repos[1].id])).expect("resolves");
        assert_eq!(repo_scope, vec![repos[1].id]);
        assert_eq!(
            run_scope,
            scope(&[(&repos[1], entry(true, false, &["api/x.md", "guide/"]))])
        );

        // A requested repo no entry names is the whole repo.
        let (_, run_scope) = resolve(
            &item(&["docs:guide/**"]),
            &repos,
            &worktree(),
            Some(&[repos[0].id]),
        )
        .expect("resolves");
        assert_eq!(run_scope, scope(&[(&repos[0], entry(true, false, &[]))]));

        // D14, kept: an explicitly empty scope on a project with a primary.
        assert_eq!(
            resolve(&declared, &repos, &worktree(), Some(&[])),
            Err(ResolveError::EmptyScopeWithPrimary {
                item: declared.id,
                repo: repos[0].id,
            })
        );
    }

    #[test]
    fn every_isolated_phase_makes_the_repo_isolated() {
        let repos = two_repos();
        let (_, run_scope) = resolve(
            &item(&["src/**"]),
            &repos,
            &phases(&[Isolation::Worktree, Isolation::Copy, Isolation::Worktree]),
            None,
        )
        .expect("resolves");
        assert_eq!(
            run_scope,
            scope(&[(&repos[0], entry(true, false, &["src/"]))])
        );
    }

    #[test]
    fn one_local_phase_makes_the_repo_local() {
        let repos = two_repos();
        let (_, run_scope) = resolve(
            &item(&["src/**", "docs:guide/**"]),
            &repos,
            &phases(&[Isolation::Worktree, Isolation::Local]),
            None,
        )
        .expect("resolves");
        assert_eq!(
            run_scope,
            scope(&[
                (&repos[0], entry(false, true, &["src/"])),
                (&repos[1], entry(false, true, &["guide/"])),
            ])
        );
    }

    #[test]
    fn one_shared_phase_makes_it_not_isolated() {
        let repos = two_repos();
        let (_, run_scope) = resolve(
            &item(&["src/**"]),
            &repos,
            &phases(&[Isolation::Copy, Isolation::SharedSerialized]),
            None,
        )
        .expect("resolves");
        assert_eq!(
            run_scope,
            scope(&[(&repos[0], entry(false, false, &["src/"]))])
        );
    }

    #[test]
    fn a_project_without_repos_resolves_to_nothing() {
        for paths in [&[][..], &["src/**"][..]] {
            let (repo_scope, run_scope) =
                resolve(&item(paths), &[], &worktree(), None).expect("resolves");
            assert_eq!(repo_scope, Vec::<RepoId>::new());
            assert_eq!(run_scope, RunScope::default());
        }
        // No primary, so an explicitly empty scope is legitimate (D14).
        let (repo_scope, run_scope) =
            resolve(&item(&["src/**"]), &[], &worktree(), Some(&[])).expect("resolves");
        assert_eq!(repo_scope, Vec::<RepoId>::new());
        assert_eq!(run_scope, RunScope::default());
    }
}
