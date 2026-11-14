//! The activity feed (MOD-122 milestone 3, plan D1-D5, D7): one row per transition a person
//! should hear about, written by the store inside the transition (plan D2), never by a caller.
//!
//! [`ActivityKind`] names the transition (plan D3's table), [`ActivityEvent`] is a stored row as
//! the panel reads it, and [`ActivityPage`] is one read: the events after a cursor and every
//! needs-you event still waiting ([`ActivityPage::pinned`], derived at read time, plan D4).
//! [`park_kind`] is D3's run-level park rule, the one Rust copy both `MemStore`'s writer and the
//! parity tests read; the Postgres trigger implements the same rule in plpgsql.
//!
//! Parity is two links (blueprint F1). `htui-worker`'s `views.rs` test
//! `park_kind_agrees_with_waiting` pins [`park_kind`] against the waiting-on-you classifier
//! (`htui_worker::waiting`) over its fixtures, with a named list of divergences; the conformance
//! case `run_park_kinds`, run on both stores, pins the store's written kind against
//! [`park_kind`] over the steps read back.

use chrono::{DateTime, Utc};

use crate::model::ids::{ItemId, PermissionId, ProjectId, RunId, StepId, WorkspaceId};
use crate::model::run::{RunStep, StepStatus};

str_enum!(
    /// Plan D3: one activity event kind, by its stored name (`chk_activity_event_kind`).
    ActivityKind {
        /// A step parked at its gate (`run_step.status` → `awaiting_approval`).
        Gate => "gate",
        /// A parked run whose fan-out slot's judge failed with a note: pick a winner.
        JudgeFailed => "judge_failed",
        /// A parked run whose newest fan-out slot waits on a pick.
        Selection => "selection",
        /// An item moved to `blocked`.
        Unblock => "unblock",
        /// A parked run no other rule names (the engine's `park_interrupted`).
        Interrupted => "interrupted",
        /// A permission request parked `pending`.
        Permission => "permission",
        /// A step moved to `failed`.
        StepFailed => "step_failed",
        /// A run moved to `failed`.
        RunFailed => "run_failed",
        /// A graph run moved `queued` → `running`.
        RunStarted => "run_started",
        /// A graph run moved to `done` over no open item.
        RunFinished => "run_finished",
        /// A graph run moved to `done` over an item that is not `closed`: it is ready to close.
        CloseOut => "close_out",
    }
);

/// How the panel treats an [`ActivityKind`] (plan D3's class column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityClass {
    /// Waits on a person; pinned while its subject still waits (plan D4).
    NeedsYou,
    /// A failure: kept when its item goes (plan D7).
    Failure,
    /// Neither.
    Plain,
}

impl ActivityKind {
    /// Plan D3's class of this kind.
    #[must_use]
    pub const fn class(self) -> ActivityClass {
        match self {
            Self::Gate
            | Self::JudgeFailed
            | Self::Selection
            | Self::Unblock
            | Self::Interrupted
            | Self::Permission => ActivityClass::NeedsYou,
            Self::StepFailed | Self::RunFailed => ActivityClass::Failure,
            Self::RunStarted | Self::RunFinished | Self::CloseOut => ActivityClass::Plain,
        }
    }

    /// The panel's words: "gate", "judge failed", … , "ready to close".
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gate => "gate",
            Self::JudgeFailed => "judge failed",
            Self::Selection => "selection",
            Self::Unblock => "unblock",
            Self::Interrupted => "interrupted",
            Self::Permission => "permission",
            Self::StepFailed => "step failed",
            Self::RunFailed => "run failed",
            Self::RunStarted => "run started",
            Self::RunFinished => "run finished",
            Self::CloseOut => "ready to close",
        }
    }
}

/// One stored activity event as the panel reads it (plan D3, D5, D7).
///
/// Every reference is `ON DELETE SET NULL`; `project_slug`, `item_key` and `step_name` are copies,
/// so a failure that outlives its item still reads (plan D7). Postgres's `txid` and `subject`
/// columns are not exposed (blueprint F3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityEvent {
    /// The identity column: rises with every event written.
    pub id: i64,
    /// When the store wrote it.
    pub at: DateTime<Utc>,
    /// What happened.
    pub kind: ActivityKind,
    /// Plan D5: the run's workspace, else the project's scope workspace, else the project's only
    /// workspace, else `None`.
    pub workspace: Option<WorkspaceId>,
    /// The item's (or the chat run's) project.
    pub project: Option<ProjectId>,
    /// The project's slug, copied.
    pub project_slug: String,
    /// The item, when there is one.
    pub item: Option<ItemId>,
    /// The item's key, copied.
    pub item_key: Option<String>,
    /// The run.
    pub run: Option<RunId>,
    /// `run.kind = 'chat'`.
    pub chat: bool,
    /// The step the event jumps to.
    pub step: Option<StepId>,
    /// The step's `phase_name`.
    pub step_name: Option<String>,
    /// The permission request, for a [`ActivityKind::Permission`] event.
    pub permission: Option<PermissionId>,
    /// Plan D3's text: a gate note, a permission summary or `run.failure`, cut to
    /// [`TEXT_MAX_CHARS`]; else empty.
    pub text: String,
}

/// One read of the feed (plan D1, D4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityPage {
    /// The events after the cursor, oldest first.
    pub events: Vec<ActivityEvent>,
    /// Every needs-you event still waiting, newest first, at most [`PINNED_LIMIT`].
    pub pinned: Vec<ActivityEvent>,
}

/// Plan D4: at most this many pinned events per read.
pub const PINNED_LIMIT: u32 = 200;

/// F7: one page of events: T3's `ACTIVITY_LIMIT` and T4's `CAPACITY` are defined from it.
pub const ACTIVITY_PAGE: u32 = 500;

/// Plan D3: an event's text is cut to this many chars, at a char boundary.
pub const TEXT_MAX_CHARS: usize = 500;

/// `text` cut to [`TEXT_MAX_CHARS`] chars, at a char boundary (Postgres's `left(text, 500)`).
#[must_use]
pub fn activity_text(text: &str) -> String {
    text.chars().take(TEXT_MAX_CHARS).collect()
}

/// The `fanout_index` of a fan-out slot's judge row (`htui-orch`'s `judge_at`).
const JUDGE_FANOUT_INDEX: i32 = -1;

/// Plan D3's run-level park rule over the run's steps, first match wins:
///
/// 1. **`judge_failed`**: a judge row (`fanout_index = -1`) `failed` with a `gate_note` (any text,
///    as `views.rs` reads it) in a slot `(position, attempt)` none of whose candidates is
///    `selected = true`. Several: the greatest `(position, attempt)`. The event's step is the
///    judge.
/// 2. **`selection`**: the newest slot holding two or more candidates (`fanout_index >= 0`),
///    newest being the greatest `(position, attempt)`, has none `selected = true` and none
///    `running` or `pending`. The event's step is its first candidate (lowest `fanout_index`).
/// 3. **`interrupted`**: the event's step is the most recently updated `failed` step (greatest
///    `(updated_at, id)`), or none.
///
/// `htui_worker::waiting` reads the snapshot and the cursor, which SQL cannot, so this is its
/// approximation; the parity test `park_kind_agrees_with_waiting` names where they differ.
#[must_use]
pub fn park_kind(steps: &[RunStep]) -> (ActivityKind, Option<StepId>) {
    let candidates = |position: i32, attempt: i32| {
        steps.iter().filter(move |step| {
            step.position == position && step.attempt == attempt && step.fanout_index >= 0
        })
    };
    let decided = |position: i32, attempt: i32| {
        candidates(position, attempt).any(|step| step.selected == Some(true))
    };

    let judge = steps
        .iter()
        .filter(|step| {
            step.fanout_index == JUDGE_FANOUT_INDEX
                && step.status == StepStatus::Failed
                && step.gate_note.is_some()
                && !decided(step.position, step.attempt)
        })
        .max_by_key(|step| (step.position, step.attempt));
    if let Some(judge) = judge {
        return (ActivityKind::JudgeFailed, Some(judge.id));
    }

    let newest = steps
        .iter()
        .filter(|step| step.fanout_index >= 0)
        .map(|step| (step.position, step.attempt))
        .filter(|&(position, attempt)| candidates(position, attempt).nth(1).is_some())
        .max();
    if let Some((position, attempt)) = newest {
        let waiting = candidates(position, attempt).all(|step| {
            step.selected != Some(true)
                && !matches!(step.status, StepStatus::Running | StepStatus::Pending)
        });
        if waiting
            && let Some(first) = candidates(position, attempt).min_by_key(|step| step.fanout_index)
        {
            return (ActivityKind::Selection, Some(first.id));
        }
    }

    let failed = steps
        .iter()
        .filter(|step| step.status == StepStatus::Failed)
        .max_by_key(|step| (step.updated_at, step.id));
    (ActivityKind::Interrupted, failed.map(|step| step.id))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeDelta, Utc};
    use uuid::Uuid;

    use super::{ActivityClass, ActivityKind, activity_text, park_kind};
    use crate::model::{RunId, RunStep, StepId, StepRole, StepStatus};

    /// A step of one run at `(position, attempt, fanout_index)`, at `status`, updated `minute`
    /// minutes after a fixed instant.
    fn step(n: u128, (position, attempt, fanout): (i32, i32, i32), status: StepStatus) -> RunStep {
        RunStep {
            id: StepId::from_uuid(Uuid::from_u128(n)),
            run_id: RunId::from_uuid(Uuid::from_u128(1)),
            position,
            attempt,
            fanout_index: fanout,
            phase_name: "research".to_owned(),
            agent_id: None,
            model: None,
            status,
            gate_outcome: None,
            gate_note: None,
            selected: None,
            exit_code: None,
            prompt_digest: None,
            trim_record: None,
            usage: None,
            isolation_path: None,
            started_at: None,
            finished_at: None,
            verify_outcome: None,
            verify_exit_code: None,
            promoted_at: None,
            branch: None,
            role: StepRole::Step,
            parent_step_id: None,
            node_key: None,
            updated_at: base(),
        }
    }

    fn base() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_220_800, 0).expect("in range")
    }

    fn noted(mut step: RunStep, note: &str) -> RunStep {
        step.gate_note = Some(note.to_owned());
        step
    }

    fn selected(mut step: RunStep, selected: bool) -> RunStep {
        step.selected = Some(selected);
        step
    }

    fn id(n: u128) -> Option<StepId> {
        Some(StepId::from_uuid(Uuid::from_u128(n)))
    }

    #[test]
    fn a_failed_noted_judge_in_an_unselected_slot_is_judge_failed() {
        let steps = [
            step(2, (0, 1, 0), StepStatus::Done),
            step(3, (0, 1, 1), StepStatus::Done),
            noted(step(4, (0, 1, -1), StepStatus::Failed), "judge: tie"),
        ];
        assert_eq!(park_kind(&steps), (ActivityKind::JudgeFailed, id(4)));
    }

    #[test]
    fn a_failed_judge_with_no_note_is_a_selection() {
        let steps = [
            step(2, (0, 1, 0), StepStatus::Done),
            step(3, (0, 1, 1), StepStatus::Done),
            step(4, (0, 1, -1), StepStatus::Failed),
        ];
        assert_eq!(park_kind(&steps), (ActivityKind::Selection, id(2)));
    }

    #[test]
    fn an_unselected_slot_of_two_done_candidates_is_a_selection_on_the_first() {
        // Listed out of `fanout_index` order: the first candidate is the lowest index.
        let steps = [
            step(3, (0, 1, 1), StepStatus::Done),
            step(2, (0, 1, 0), StepStatus::Done),
        ];
        assert_eq!(park_kind(&steps), (ActivityKind::Selection, id(2)));
    }

    #[test]
    fn a_newest_failed_step_with_no_slot_is_interrupted_on_that_step() {
        let mut older = step(2, (0, 1, 0), StepStatus::Failed);
        older.updated_at = base();
        let mut newer = step(3, (1, 1, 0), StepStatus::Failed);
        newer.updated_at = base() + TimeDelta::minutes(1);
        let steps = [newer, step(4, (2, 1, 0), StepStatus::Done), older];
        assert_eq!(park_kind(&steps), (ActivityKind::Interrupted, id(3)));
    }

    #[test]
    fn failed_steps_updated_together_break_the_tie_on_the_id() {
        let steps = [
            step(3, (1, 1, 0), StepStatus::Failed),
            step(2, (0, 1, 0), StepStatus::Failed),
        ];
        assert_eq!(park_kind(&steps), (ActivityKind::Interrupted, id(3)));
    }

    #[test]
    fn no_steps_is_interrupted_on_no_step() {
        assert_eq!(park_kind(&[]), (ActivityKind::Interrupted, None));
    }

    #[test]
    fn a_slot_with_a_selected_candidate_is_never_a_selection() {
        let steps = [
            selected(step(2, (0, 1, 0), StepStatus::Done), true),
            selected(step(3, (0, 1, 1), StepStatus::Superseded), false),
        ];
        assert_eq!(park_kind(&steps), (ActivityKind::Interrupted, None));

        // Nor is it a judge failure: the slot is decided.
        let judged = [
            selected(step(2, (0, 1, 0), StepStatus::Done), true),
            selected(step(3, (0, 1, 1), StepStatus::Superseded), false),
            noted(step(4, (0, 1, -1), StepStatus::Failed), "judge: tie"),
        ];
        assert_eq!(park_kind(&judged), (ActivityKind::Interrupted, id(4)));
    }

    #[test]
    fn a_slot_with_a_live_candidate_is_interrupted() {
        for live in [StepStatus::Running, StepStatus::Pending] {
            let steps = [
                step(2, (0, 1, 0), StepStatus::Done),
                step(3, (0, 1, 1), live),
            ];
            assert_eq!(
                park_kind(&steps),
                (ActivityKind::Interrupted, None),
                "{live}"
            );
        }
    }

    #[test]
    fn only_the_newest_slot_can_be_a_selection() {
        // The older slot is undecided, the newer decided: no selection.
        let steps = [
            step(2, (0, 1, 0), StepStatus::Done),
            step(3, (0, 1, 1), StepStatus::Done),
            selected(step(4, (1, 1, 0), StepStatus::Done), true),
            selected(step(5, (1, 1, 1), StepStatus::Superseded), false),
        ];
        assert_eq!(park_kind(&steps), (ActivityKind::Interrupted, None));

        // The older slot is decided, the newer undecided: a selection on the newer slot.
        let steps = [
            selected(step(2, (0, 1, 0), StepStatus::Done), true),
            selected(step(3, (0, 1, 1), StepStatus::Superseded), false),
            step(5, (1, 1, 1), StepStatus::Done),
            step(4, (1, 1, 0), StepStatus::Done),
        ];
        assert_eq!(park_kind(&steps), (ActivityKind::Selection, id(4)));
    }

    #[test]
    fn a_single_candidate_is_not_a_slot() {
        let steps = [step(2, (0, 1, 0), StepStatus::Done)];
        assert_eq!(park_kind(&steps), (ActivityKind::Interrupted, None));
    }

    #[test]
    fn the_judge_rule_wins_over_a_newer_selection() {
        let steps = [
            step(2, (0, 1, 0), StepStatus::Done),
            step(3, (0, 1, 1), StepStatus::Done),
            noted(step(4, (0, 1, -1), StepStatus::Failed), "judge: tie"),
            step(5, (1, 1, 0), StepStatus::Done),
            step(6, (1, 1, 1), StepStatus::Done),
        ];
        assert_eq!(park_kind(&steps), (ActivityKind::JudgeFailed, id(4)));
    }

    #[test]
    fn every_kind_has_its_class_and_label() {
        let table = [
            (ActivityKind::Gate, ActivityClass::NeedsYou, "gate"),
            (
                ActivityKind::JudgeFailed,
                ActivityClass::NeedsYou,
                "judge failed",
            ),
            (
                ActivityKind::Selection,
                ActivityClass::NeedsYou,
                "selection",
            ),
            (ActivityKind::Unblock, ActivityClass::NeedsYou, "unblock"),
            (
                ActivityKind::Interrupted,
                ActivityClass::NeedsYou,
                "interrupted",
            ),
            (
                ActivityKind::Permission,
                ActivityClass::NeedsYou,
                "permission",
            ),
            (
                ActivityKind::StepFailed,
                ActivityClass::Failure,
                "step failed",
            ),
            (
                ActivityKind::RunFailed,
                ActivityClass::Failure,
                "run failed",
            ),
            (
                ActivityKind::RunStarted,
                ActivityClass::Plain,
                "run started",
            ),
            (
                ActivityKind::RunFinished,
                ActivityClass::Plain,
                "run finished",
            ),
            (
                ActivityKind::CloseOut,
                ActivityClass::Plain,
                "ready to close",
            ),
        ];
        assert_eq!(table.len(), ActivityKind::ALL.len());
        for (kind, class, label) in table {
            assert_eq!((kind.class(), kind.label()), (class, label), "{kind}");
        }
    }

    #[test]
    fn text_is_cut_at_a_char_boundary() {
        let long = "é".repeat(600);
        let cut = activity_text(&long);
        assert_eq!(cut.chars().count(), 500);
        assert_eq!(cut, "é".repeat(500));
        assert_eq!(activity_text("short"), "short");
    }
}
