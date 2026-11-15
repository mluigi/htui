//! MOD-133 (ANA-34 §7): how a finished run's work reaches its repository's remote.
//!
//! A repo's [`IntegrateMode`] is `repo.integrate`, else the project's `integrate` setting
//! ([`IntegrateMode::project_default`]), else [`IntegrateMode::Local`]: today's behaviour, where
//! the work stays on the box's local target branch. A non-`local` repo of a run's scope is frozen
//! into the run's snapshot (`GraphSnapshot::integrate`), and `finish_run(Done)` writes one
//! [`Integration`] record per such repo the run changed, in the same transaction. The executing
//! box **lands** a record (`Isolator::land`): verify, then push; a record whose run is `done` but
//! is not [`IntegrationState::is_landed`] holds the run's dependents (ANA-35 §7.1, MOD-133 D7).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{ProjectId, RepoId, RunId};

str_enum!(
    /// `repo.integrate` and `integration.mode` (MOD-133 D1): how a finished run's work reaches
    /// the remote. `integration.mode` never holds `local`: a `local` repo writes no record.
    IntegrateMode {
        /// The work stays on the box's local target branch (today's behaviour, the default).
        Local => "local",
        /// The executing box fast-forwards the remote's target branch with its own credentials.
        Push => "push",
        /// The run's steps reconcile into a run branch ([`run_branch_name`]) in the primary
        /// repository instead of its checked-out branch; after `done` the executing box pushes
        /// that branch (never forced) and the record stays `pending` with the pushed head in
        /// `landed_hash` (MOD-133 milestone 3). The pull request against the target branch waits
        /// for the forge adapter (MOD-134).
        PullRequest => "pull_request",
    }
);

impl IntegrateMode {
    /// The `project.settings` key the project default lives under (MOD-133 D2).
    pub const PROJECT_SETTING: &'static str = "integrate";

    /// MOD-133 D2: the project default, read **strictly and on its own** out of a
    /// `project.settings` document, as [`ProjectCaps::from_settings`](crate::model::ProjectCaps)
    /// reads its keys. Not a [`ProjectSettings`](crate::model::ProjectSettings) field: a bad value
    /// there would fail the whole document's decode and reset every key.
    ///
    /// Absent or `null` (and a document that is not an object, which holds no key) is
    /// [`IntegrateMode::Local`]; `"local"`, `"push"` and `"pull_request"` are that mode.
    ///
    /// # Errors
    /// Anything else, as the value's JSON text (`"bogus"` with its quotes, `3`, `true`), for the
    /// run-start refusal to name.
    pub fn project_default(settings: &Value) -> Result<Self, String> {
        match settings.get(Self::PROJECT_SETTING) {
            None | Some(Value::Null) => Ok(Self::Local),
            Some(Value::String(text)) => text
                .parse()
                .map_err(|_| Value::from(text.as_str()).to_string()),
            Some(other) => Err(other.to_string()),
        }
    }

    /// MOD-133 M2 E8: why a Settings form refuses to set this mode, or `None` when it may.
    /// [`IntegrateMode::PullRequest`] waits for the forge adapter (MOD-134), whatever the remote;
    /// [`IntegrateMode::Push`] needs a remote URL (`has_remote`); [`IntegrateMode::Local`] is
    /// always selectable. The form is the guard: the stores keep no such check, and a run refuses
    /// at start (M1 D3).
    #[must_use]
    pub fn selectable_refusal(self, has_remote: bool) -> Option<String> {
        match self {
            Self::PullRequest => Some(
                "`pull_request` needs the forge adapter (MOD-134): choose `local` or `push`"
                    .to_owned(),
            ),
            Self::Push if !has_remote => {
                Some("`push` needs a remote URL: set one, or choose `local`".to_owned())
            }
            Self::Local | Self::Push => None,
        }
    }
}

str_enum!(
    /// `integration.state` (MOD-133 D1). The PR states are written by MOD-134; they are declared
    /// now so the `CHECK` never moves.
    IntegrationState {
        /// Written by `finish_run(Done)`; waits for its box to land it.
        Pending => "pending",
        /// Pushed: the remote's target branch holds the run's work.
        Landed => "landed",
        /// A pull request is open for the run branch (MOD-134).
        PrOpen => "pr_open",
        /// The pull request merged (MOD-134).
        PrMerged => "pr_merged",
        /// The pull request closed unmerged (MOD-134).
        PrClosed => "pr_closed",
        /// Landing was refused for a typed reason ([`Integration::refusal`]); not retried by a
        /// sweep. Unblock retries it (MOD-133 M2):
        /// [`WriteStore::retry_landing`](crate::store::WriteStore::retry_landing) puts it back to
        /// `pending` for its box to land again.
        Refused => "refused",
    }
);

impl IntegrationState {
    /// Whether the record's work is on the remote's target branch: [`Self::Landed`] or
    /// [`Self::PrMerged`]. A blocker's `done` run releases its dependents only when every record
    /// of it is landed (MOD-133 D7).
    #[must_use]
    pub const fn is_landed(self) -> bool {
        matches!(self, Self::Landed | Self::PrMerged)
    }
}

/// A row of `integration` (MOD-133 D5): one finished run's landing in one repo.
///
/// The lease columns (`lease_box_id`, `lease_owner`, `lease_expires_at`) are not fields, as
/// `run.lease_owner` is not a [`Run`](crate::model::Run) field: only the store compares them
/// ([`WriteStore::take_integrations`](crate::store::WriteStore::take_integrations)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Integration {
    /// `integration.run_id`.
    pub run_id: RunId,
    /// `integration.repo_id`.
    pub repo_id: RepoId,
    /// `integration.project_id`: the run's project.
    pub project_id: ProjectId,
    /// `integration.mode`, frozen from the run's snapshot; never [`IntegrateMode::Local`].
    pub mode: IntegrateMode,
    /// `integration.remote`: the repo's `remote_url` at run start.
    pub remote: String,
    /// `integration.target_branch`: the repo's `default_branch` at run start.
    pub target_branch: String,
    /// `integration.state`.
    pub state: IntegrationState,
    /// `integration.refusal`: the typed reason, `Some` exactly when `state` is `refused`.
    pub refusal: Option<String>,
    /// `integration.landed_hash`: the target branch's tip the landing pushed (or found there);
    /// for a `pull_request` record, the run branch's pushed head ([`LandOutcome::Pushed`]).
    pub landed_hash: Option<String>,
    /// `integration.pr_ref` (MOD-134).
    pub pr_ref: Option<String>,
    /// `integration.pr_url` (MOD-134).
    pub pr_url: Option<String>,
    /// `integration.created_at`.
    pub created_at: DateTime<Utc>,
    /// `integration.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// MOD-133 M3 D1: the run branch a `pull_request` run of the item keyed `key` reconciles into
/// and pushes: `htui/item/<key>`. It cannot collide with the `htui/<step-uuid>` step labels. Two
/// projects that share one checkout and have equal item keys share the branch, which is accepted.
///
/// A guard only: real keys (an uppercase prefix, a dash and a number) always pass.
///
/// # Errors
/// Why `key` makes no branch name: it is empty, holds a character outside `[A-Za-z0-9._-]`, holds
/// `..`, starts or ends with `.`, or ends with `.lock` (the last three are what `git
/// check-ref-format` refuses of a ref component).
pub fn run_branch_name(key: &str) -> Result<String, String> {
    if key.is_empty() {
        return Err("an empty item key makes no branch name".to_owned());
    }
    if let Some(bad) = key
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "item key `{key}` holds `{bad}`, which a branch name does not allow"
        ));
    }
    if key.contains("..") || key.starts_with('.') || key.ends_with('.') || key.ends_with(".lock") {
        return Err(format!(
            "item key `{key}` holds `..`, starts or ends with `.`, or ends with `.lock`, which a \
             branch name does not allow"
        ));
    }
    Ok(format!("htui/item/{key}"))
}

/// What landing one [`Integration`] answered (MOD-133 D6, D8), settled by
/// [`WriteStore::settle_integration`](crate::store::WriteStore::settle_integration).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LandOutcome {
    /// The remote's target branch is at `hash`: `pending -> landed`.
    Landed {
        /// The target branch's tip, `integration.landed_hash`.
        hash: String,
    },
    /// MOD-133 M3 D5: a `pull_request` record's run branch is on the remote at `hash`. The record
    /// stays `pending` with `landed_hash` set, so it is not landed (its dependents keep waiting)
    /// and no sweep takes it again until MOD-134 opens the pull request. Settling it on a record
    /// whose mode is not [`IntegrateMode::PullRequest`] answers `false`.
    Pushed {
        /// The run branch's pushed head, `integration.landed_hash`.
        hash: String,
    },
    /// Landing was refused: `pending -> refused`.
    Refused {
        /// The typed reason (`not_on_target: …`, `non_fast_forward: …`), `integration.refusal`.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{IntegrateMode, IntegrationState, run_branch_name};

    /// MOD-133 M3 D1: a real item key is its run branch's last component.
    #[test]
    fn run_branch_name_accepts_real_keys() {
        for (key, branch) in [
            ("MOD-7", "htui/item/MOD-7"),
            ("AB12-300", "htui/item/AB12-300"),
            ("a.b_c-9", "htui/item/a.b_c-9"),
        ] {
            assert_eq!(run_branch_name(key).as_deref(), Ok(branch), "{key}");
        }
    }

    /// MOD-133 M3 D1: a key that would not make a branch name is refused, never mangled.
    #[test]
    fn run_branch_name_refuses_what_is_not_a_branch_name() {
        for key in [
            "a/b",
            "a..b",
            "a b",
            "a@{b",
            "",
            ".hidden",
            "trailing.",
            "x.lock",
            "a~1",
            "a^",
            "a:b",
            "a\\b",
            "é",
        ] {
            assert!(run_branch_name(key).is_err(), "{key:?} is refused");
        }
    }

    /// MOD-133 D2: absent and `null` are `local`; each wire name is its mode.
    #[test]
    fn project_default_reads_the_three_modes_and_defaults_to_local() {
        for (settings, mode) in [
            (json!({}), IntegrateMode::Local),
            (json!({ "integrate": null }), IntegrateMode::Local),
            (json!({ "integrate": "local" }), IntegrateMode::Local),
            (json!({ "integrate": "push" }), IntegrateMode::Push),
            (
                json!({ "integrate": "pull_request" }),
                IntegrateMode::PullRequest,
            ),
            (json!({ "default_isolation": "copy" }), IntegrateMode::Local),
        ] {
            assert_eq!(
                IntegrateMode::project_default(&settings),
                Ok(mode),
                "{settings}"
            );
        }
    }

    /// MOD-133 D2: any other value is an error naming it as JSON, never a silent `local`.
    #[test]
    fn project_default_refuses_any_other_value_naming_it() {
        for (settings, named) in [
            (json!({ "integrate": "bogus" }), r#""bogus""#),
            (json!({ "integrate": "Push" }), r#""Push""#),
            (json!({ "integrate": "" }), r#""""#),
            (json!({ "integrate": 3 }), "3"),
            (json!({ "integrate": true }), "true"),
            (json!({ "integrate": ["push"] }), r#"["push"]"#),
            (
                json!({ "integrate": { "mode": "push" } }),
                r#"{"mode":"push"}"#,
            ),
        ] {
            assert_eq!(
                IntegrateMode::project_default(&settings),
                Err(named.to_owned()),
                "{settings}"
            );
        }
    }

    /// A document that is not an object holds no key, so it reads as the default.
    #[test]
    fn project_default_of_a_non_object_is_local() {
        for settings in [json!(null), json!([]), json!("push")] {
            assert_eq!(
                IntegrateMode::project_default(&settings),
                Ok(IntegrateMode::Local),
                "{settings}"
            );
        }
    }

    /// MOD-133 M2 E8: `pull_request` waits for the forge adapter whatever the remote; `push` needs
    /// a remote URL; `local` is always selectable.
    #[test]
    fn selectable_refusal_refuses_pull_request_and_push_without_a_remote() {
        let forge = "`pull_request` needs the forge adapter (MOD-134): choose `local` or `push`";
        let remote = "`push` needs a remote URL: set one, or choose `local`";
        for (mode, has_remote, expected) in [
            (IntegrateMode::Local, true, None),
            (IntegrateMode::Local, false, None),
            (IntegrateMode::Push, true, None),
            (IntegrateMode::Push, false, Some(remote)),
            (IntegrateMode::PullRequest, true, Some(forge)),
            (IntegrateMode::PullRequest, false, Some(forge)),
        ] {
            assert_eq!(
                mode.selectable_refusal(has_remote).as_deref(),
                expected,
                "{mode}, has_remote {has_remote}"
            );
        }
    }

    /// MOD-133 D1: the wire names, and `is_landed` is `landed | pr_merged`.
    #[test]
    fn wire_names_and_is_landed() {
        assert_eq!(
            IntegrateMode::ALL
                .iter()
                .map(|mode| mode.as_str())
                .collect::<Vec<_>>(),
            ["local", "push", "pull_request"]
        );
        assert_eq!(
            IntegrationState::ALL
                .iter()
                .map(|state| (state.as_str(), state.is_landed()))
                .collect::<Vec<_>>(),
            [
                ("pending", false),
                ("landed", true),
                ("pr_open", false),
                ("pr_merged", true),
                ("pr_closed", false),
                ("refused", false),
            ]
        );
    }
}
