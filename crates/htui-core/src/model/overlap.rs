//! ANA-2 §4.7's overlap predicate (`docs/ANA-2.md:1059-1089`), pure, shared by both stores'
//! admission (plan D80). The prefixes are `prompt::excerpt::PathPrefix::prefix` strings (D104):
//! this module parses nothing.
//!
//! The predicate lives in `htui-core` rather than beside the orchestrator's resolution half
//! (`htui_orch::overlap`, D81) because both stores evaluate it inside `claim_run`'s critical
//! section, and the stores cannot see `htui-orch`. One implementation that `MemStore` and
//! `PgStore` share is what keeps the conformance suite from ever seeing them disagree.

use core::fmt;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{RepoId, RunId};

/// Plan D79: `GraphSnapshot.scope`, the scope a run was resolved to at `StartRun`. Keys equal
/// `run.repo_scope`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunScope {
    /// Every repo the run may touch, with how it touches it.
    #[serde(default)]
    pub repos: BTreeMap<RepoId, RepoScope>,
}

/// One repo of a [`RunScope`]. Every field defaults to the conservative value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoScope {
    /// Every phase is `worktree | copy` for this repo (`:1053-1055`).
    #[serde(default)]
    pub isolated: bool,
    /// Any phase is `local` (`:1046`).
    #[serde(default)]
    pub local: bool,
    /// `PathPrefix::prefix` values; **empty = unknown = the whole repo** (`:1072`).
    #[serde(default)]
    pub prefixes: Vec<String>,
}

impl RunScope {
    /// D80: every repo of `repo_scope`, `isolated = false`, `local = false`, `prefixes = []`,
    /// which is exactly the pre-milestone-5 "any shared repo overlaps".
    #[must_use]
    pub fn conservative(repo_scope: &[RepoId]) -> Self {
        Self {
            repos: repo_scope
                .iter()
                .map(|repo| (*repo, RepoScope::default()))
                .collect(),
        }
    }
}

/// Rules L, I, P in §4.7's order (`:1063-1066`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlapRule {
    /// Rule L: either run is `local` in the repo. Unconditional.
    Local,
    /// Rule I: either run is not isolated in the repo (`R-ORCH-9` clause 1).
    NotIsolated,
    /// Rule P: both are isolated, but their path prefixes intersect (`R-ORCH-9` clause 2).
    Paths,
}

impl fmt::Display for OverlapRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Local => "local",
            Self::NotIsolated => "not_isolated",
            Self::Paths => "paths",
        })
    }
}

/// §4.7's `overlaps(A, B)`: for each repo of the intersection in `RepoId` order, check L, then I,
/// then P, and return the first hit. `None` = no overlap.
#[must_use]
pub fn overlaps(a: &RunScope, b: &RunScope) -> Option<OverlapRule> {
    // Both maps iterate in `RepoId` order, so the first hit is the lowest shared repo's.
    a.repos.iter().find_map(|(repo, x)| {
        let y = b.repos.get(repo)?;
        if x.local || y.local {
            Some(OverlapRule::Local)
        } else if !(x.isolated && y.isolated) {
            Some(OverlapRule::NotIsolated)
        } else if intersect(&x.prefixes, &y.prefixes) {
            Some(OverlapRule::Paths)
        } else {
            None
        }
    })
}

/// §4.7's `intersect(xs, ys)`: an empty list is unknown and overlaps everything; otherwise some
/// prefix of one side is a prefix of some prefix of the other. Bytes, no case folding
/// (`excerpt.rs`'s `PathPrefix::matches` rule).
fn intersect(xs: &[String], ys: &[String]) -> bool {
    xs.is_empty()
        || ys.is_empty()
        || xs.iter().any(|x| {
            ys.iter().any(|y| {
                x.as_bytes().starts_with(y.as_bytes()) || y.as_bytes().starts_with(x.as_bytes())
            })
        })
}

/// D80, D109: decode `snapshot["scope"]`. If it is absent, `null` or undecodable, the answer is
/// [`RunScope::conservative`]`(repo_scope)`. A decoded scope also gains a conservative entry for
/// every `repo_scope` repo it lacks (the safe direction).
#[must_use]
pub fn scope_of(snapshot: &Value, repo_scope: &[RepoId]) -> RunScope {
    let decoded = snapshot
        .get("scope")
        .filter(|scope| !scope.is_null())
        .and_then(|scope| RunScope::deserialize(scope).ok());
    let Some(mut scope) = decoded else {
        return RunScope::conservative(repo_scope);
    };
    for repo in repo_scope {
        scope.repos.entry(*repo).or_default();
    }
    scope
}

/// Plan D83: `WriteStore::claim_run`'s verdict.
#[must_use = "a refused claim wrote nothing; the caller must act on why"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// The run is now `running` on the box.
    Admitted,
    /// Not `queued`, or `target_box_id != box`.
    NotClaimable,
    /// The box already runs `limit` runs.
    SlotFull {
        /// Runs counted against the box's slot.
        running: u64,
        /// The box's `max_concurrent_items`.
        limit: u32,
    },
    /// The first overlapping live run in `(queued_at, id)` order.
    Overlaps {
        /// The holding run.
        with: RunId,
        /// The first rule that fired.
        rule: OverlapRule,
    },
}

impl Claim {
    /// Whether the claim was [`Claim::Admitted`].
    #[must_use]
    pub const fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted)
    }
}

impl fmt::Display for Claim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admitted => f.write_str("admitted"),
            Self::NotClaimable => f.write_str("not claimable"),
            Self::SlotFull { running, limit } => {
                write!(f, "box full ({running} of {limit} running)")
            }
            Self::Overlaps { with, rule } => write!(f, "overlaps run {with} ({rule})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::excerpt::PathPrefix;
    use serde_json::json;
    use uuid::Uuid;

    fn repo(n: u128) -> RepoId {
        RepoId::from_uuid(Uuid::from_u128(n))
    }

    fn entry(isolated: bool, local: bool, prefixes: &[&str]) -> RepoScope {
        RepoScope {
            isolated,
            local,
            prefixes: prefixes.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    fn scope(entries: &[(RepoId, RepoScope)]) -> RunScope {
        RunScope {
            repos: entries.iter().cloned().collect(),
        }
    }

    fn isolated(prefixes: &[&str]) -> RunScope {
        scope(&[(repo(1), entry(true, false, prefixes))])
    }

    /// `RepoScope.prefixes` holds what `PathPrefix::parse` produces (D104): the glob cut at its
    /// first `*?[{` and then at the last `/`, a metacharacter-free path kept whole, and `**` (or
    /// any glob with no `/` before its first metacharacter) the empty prefix.
    #[test]
    fn prefixes_come_from_the_excerpt_path_prefix() {
        let prefix = |glob: &str| PathPrefix::parse(glob, "htui").prefix;
        assert_eq!(prefix("src/**/*.rs"), "src/");
        assert_eq!(
            prefix("crates/htui-core/src/model/item.rs"),
            "crates/htui-core/src/model/item.rs"
        );
        assert_eq!(prefix("**"), "");
        assert_eq!(prefix("src/{a,b}"), "src/");
        assert_eq!(prefix("a?b"), "");
    }

    #[test]
    fn disjoint_repos_never_overlap() {
        let a = scope(&[(repo(1), entry(false, true, &[]))]);
        let b = scope(&[(repo(2), entry(false, true, &[]))]);
        assert_eq!(overlaps(&a, &b), None);
        assert_eq!(overlaps(&b, &a), None);
    }

    #[test]
    fn rule_l_local_overlaps_even_when_the_other_is_isolated() {
        let a = scope(&[(repo(1), entry(false, true, &["src/"]))]);
        let b = scope(&[(repo(1), entry(true, false, &["docs/"]))]);
        assert_eq!(overlaps(&a, &b), Some(OverlapRule::Local));
        assert_eq!(overlaps(&b, &a), Some(OverlapRule::Local));
    }

    #[test]
    fn rule_i_shared_serialized_overlaps_an_isolated_run_on_the_same_repo() {
        let a = scope(&[(repo(1), entry(false, false, &["src/"]))]);
        let b = isolated(&["docs/"]);
        assert_eq!(overlaps(&a, &b), Some(OverlapRule::NotIsolated));
        assert_eq!(overlaps(&b, &a), Some(OverlapRule::NotIsolated));
    }

    #[test]
    fn rule_p_two_isolated_runs_overlap_on_intersecting_prefixes() {
        let a = isolated(&["src/"]);
        let b = isolated(&["src/lib/"]);
        assert_eq!(overlaps(&a, &b), Some(OverlapRule::Paths));
        assert_eq!(overlaps(&b, &a), Some(OverlapRule::Paths));
    }

    /// Criterion 15's parallel half: two worktree runs on one repo, disjoint paths.
    #[test]
    fn two_isolated_runs_with_disjoint_prefixes_do_not_overlap() {
        let a = isolated(&["src/"]);
        let b = isolated(&["docs/"]);
        assert_eq!(overlaps(&a, &b), None);
        assert_eq!(overlaps(&b, &a), None);
    }

    #[test]
    fn an_empty_prefix_list_overlaps_everything_in_its_repo() {
        let a = isolated(&[]);
        let b = isolated(&["docs/"]);
        assert_eq!(overlaps(&a, &b), Some(OverlapRule::Paths));
        assert_eq!(overlaps(&b, &a), Some(OverlapRule::Paths));
    }

    #[test]
    fn a_double_star_is_the_same_as_no_declaration() {
        let a = isolated(&[""]);
        let b = isolated(&["docs/"]);
        assert_eq!(overlaps(&a, &b), Some(OverlapRule::Paths));
        assert_eq!(overlaps(&b, &a), Some(OverlapRule::Paths));
    }

    #[test]
    fn rules_are_reported_in_l_i_p_order() {
        // One repo tripping L, I and P at once.
        let a = scope(&[(repo(1), entry(false, true, &["src/"]))]);
        let b = scope(&[(repo(1), entry(false, false, &["src/"]))]);
        assert_eq!(overlaps(&a, &b), Some(OverlapRule::Local));

        // One repo tripping I and P.
        let a = scope(&[(repo(1), entry(false, false, &["src/"]))]);
        let b = scope(&[(repo(1), entry(true, false, &["src/"]))]);
        assert_eq!(overlaps(&a, &b), Some(OverlapRule::NotIsolated));

        // r1 < r2: r1 trips P, r2 trips L; the per-repo loop returns at the first repo.
        let a = scope(&[
            (repo(1), entry(true, false, &["src/"])),
            (repo(2), entry(false, true, &[])),
        ]);
        let b = scope(&[
            (repo(1), entry(true, false, &["src/lib/"])),
            (repo(2), entry(true, false, &["docs/"])),
        ]);
        assert_eq!(overlaps(&a, &b), Some(OverlapRule::Paths));
        assert_eq!(overlaps(&b, &a), Some(OverlapRule::Paths));
    }

    #[test]
    fn scope_of_a_snapshot_without_scope_is_conservative() {
        let r = repo(1);
        let got = scope_of(&json!({"v": 1}), &[r]);
        assert_eq!(got, scope(&[(r, RepoScope::default())]));
        assert_eq!(got, RunScope::conservative(&[r]));
        let other = isolated(&["docs/"]);
        assert_eq!(overlaps(&got, &other), Some(OverlapRule::NotIsolated));
        assert_eq!(overlaps(&other, &got), Some(OverlapRule::NotIsolated));
    }

    #[test]
    fn scope_of_an_undecodable_scope_is_conservative() {
        let r = repo(1);
        let conservative = RunScope::conservative(&[r]);
        assert_eq!(scope_of(&json!({"scope": 7}), &[r]), conservative);
        assert_eq!(
            scope_of(&json!({"scope": {"repos": 3}}), &[r]),
            conservative
        );
        assert_eq!(scope_of(&json!({"scope": null}), &[r]), conservative);
    }

    #[test]
    fn scope_of_fills_a_repo_the_scope_forgot() {
        let r = repo(1);
        assert_eq!(
            scope_of(&json!({"scope": {"repos": {}}}), &[r]),
            scope(&[(r, RepoScope::default())])
        );

        // A decoded entry is kept as written; only the missing repo is filled conservatively.
        let (r1, r2) = (repo(1), repo(2));
        let written = isolated(&["src/"]);
        let snapshot = json!({ "scope": written });
        assert_eq!(
            scope_of(&snapshot, &[r1, r2]),
            scope(&[
                (r1, entry(true, false, &["src/"])),
                (r2, RepoScope::default()),
            ])
        );
    }

    #[test]
    fn claim_display_names_the_rule_and_the_holding_run() {
        assert_eq!(OverlapRule::Local.to_string(), "local");
        assert_eq!(OverlapRule::NotIsolated.to_string(), "not_isolated");
        assert_eq!(OverlapRule::Paths.to_string(), "paths");
        assert_eq!(Claim::Admitted.to_string(), "admitted");
        assert_eq!(Claim::NotClaimable.to_string(), "not claimable");
        assert_eq!(
            Claim::SlotFull {
                running: 3,
                limit: 2
            }
            .to_string(),
            "box full (3 of 2 running)"
        );
        let with = RunId::from_uuid(Uuid::from_u128(7));
        assert_eq!(
            Claim::Overlaps {
                with,
                rule: OverlapRule::NotIsolated
            }
            .to_string(),
            format!("overlaps run {with} (not_isolated)")
        );
        assert!(Claim::Admitted.is_admitted());
        assert!(!Claim::NotClaimable.is_admitted());
        assert!(
            !Claim::SlotFull {
                running: 3,
                limit: 2
            }
            .is_admitted()
        );
        assert!(
            !Claim::Overlaps {
                with,
                rule: OverlapRule::Paths
            }
            .is_admitted()
        );
    }
}
