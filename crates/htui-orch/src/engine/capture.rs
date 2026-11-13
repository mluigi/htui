//! MOD-164 D11: the run-end capture step, [`Engine::capture`].
//!
//! [`Engine::end_run`] calls it before the run's `finish_run` when the run is not ending
//! `cancelled`, its snapshot froze a capture and it has an item. It runs at most once per run, on
//! one `stage` row at [`CAPTURE_POSITION`] keyed [`CAPTURE_KEY`]: a one-off light-model turn
//! shaped like the judge's (`run_judge`), shown the run's evidence pack and asked for at most
//! [`MEMORY_RUN_WRITE_CAP`](htui_core::model::MEMORY_RUN_WRITE_CAP) memory entries, which are
//! checked against the pack and written through `write_run_memory`. Whatever the turn meets settles
//! the capture row; the run's own status never moves.

use std::collections::BTreeMap;

use futures::future::{Either, select};

use htui_core::capture::{
    CAPTURE_TEMPLATE, CaptureOutput, EvidencePack, capture_prompt, capture_trigger, parse_capture,
};
use htui_core::model::{
    CAPTURE_KEY, CAPTURE_POSITION, CommandQueue, Gate, Isolation, ItemId, MemoryEntryId,
    NewRunStep, NodeRole, Run, RunId, RunMemoryWrite, RunStep, SnapshotCandidate, SnapshotCapture,
    SnapshotCaptureModel, SnapshotPhase, SnapshotTemplate, StepId, StepRole, StepStatus,
};
use htui_core::store::StepFence;

use super::{
    AgentSelector, Engine, EngineError, GraphSource, Isolator, SessionKey, SessionSink, Verifier,
    failure_text, is_fenced,
};
use crate::command::stale_step;
use crate::gate;
use crate::isolate::Clock;

/// MOD-164 D11: the capture turn's deadline. The judge is untimed; a capture is not.
pub(super) const CAPTURE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// `run_step.gate_note` of a capture row passed without a turn (D11 step 3).
pub(super) const NO_LIGHT_MODEL: &str = "skipped: no light model configured";

/// `run_step.gate_note` of a capture row a crash left `running` (D11 step 1): settled, not re-run.
pub(super) const CAPTURE_INTERRUPTED: &str =
    "interrupted: the capture step did not finish; it is not run again";

/// The capture turn's output kind: what `open_tools` offers `document_write` for.
pub(super) const CAPTURE_KIND: &str = "memory_capture";

/// What a successful capture did, for its item note.
#[derive(Debug, Default)]
struct Written {
    /// Entries created, superseding or not.
    wrote: usize,
    /// Of those, the ones that superseded a run's entry.
    superseded: usize,
    /// Entries a person's live entry on the subject kept out.
    kept: usize,
    /// Entries dropped, by [`htui_core::capture::DropReason::as_str`] (or `over_cap` from the
    /// store's own cap).
    dropped: Vec<&'static str>,
}

impl Written {
    /// `memory capture: wrote N (superseded M), kept a person's entry K, dropped D (reasons)`.
    fn note(&self) -> String {
        let mut note = format!(
            "memory capture: wrote {} (superseded {}), kept a person's entry {}, dropped {}",
            self.wrote,
            self.superseded,
            self.kept,
            self.dropped.len()
        );
        if !self.dropped.is_empty() {
            note.push_str(&format!(" ({})", self.dropped.join(", ")));
        }
        note
    }
}

/// The synthetic phase the capture turn runs as: named [`CAPTURE_KEY`], ungated, single, writing a
/// [`CAPTURE_KIND`] document from no inputs, verifying nothing, in no tree and with no command
/// queue, on the internal [`CAPTURE_TEMPLATE`], with the frozen light model as its only candidate.
fn capture_phase(model: &SnapshotCaptureModel) -> SnapshotPhase {
    SnapshotPhase {
        position: CAPTURE_POSITION,
        name: CAPTURE_KEY.to_owned(),
        fan_out: 1,
        gate: Gate::Never,
        gate_effective: Gate::Never,
        gate_hard: false,
        retry_limit: 0,
        input_kinds: Vec::new(),
        output_kind: CAPTURE_KIND.to_owned(),
        isolation: Isolation::Local,
        command_queue: CommandQueue::Off,
        verify_command: None,
        deadline_seconds: None,
        template: SnapshotTemplate {
            name: CAPTURE_TEMPLATE.to_owned(),
            version: 1,
        },
        token_budget: None,
        candidates: vec![SnapshotCandidate {
            agent_id: model.agent_id,
            agent_name: model.agent_name.clone(),
            model: model.model.clone(),
        }],
        judge: None,
        persona: None,
        role: NodeRole::default(),
        palette: false,
        plan_scope: None,
        divide: None,
    }
}

/// Whether `err` leaves the capture unsettled for its owner: a lost fence or lease, or a cancel,
/// whose run another path ends. Every other error settles the capture row.
pub(super) const fn passes_through(err: &EngineError) -> bool {
    is_fenced(err)
        || matches!(
            err,
            EngineError::Cancelled { .. } | EngineError::LeaseLost { .. }
        )
}

impl<S, G, I, V, C, A, K> Engine<'_, S, G, I, V, C, A, K>
where
    S: htui_core::store::WorkerStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    V: Verifier + ?Sized,
    C: Clock + ?Sized,
    A: AgentSelector + ?Sized,
    K: SessionSink + ?Sized,
{
    /// MOD-164 D11: the run-end capture of `run`, whose snapshot froze `capture`.
    ///
    /// 1. At most once: a capture row a crash left `running` is interrupted, not re-run; a capture
    ///    row in any other state means it ran.
    /// 2. No D9 trigger over the run's steps: nothing, no row and no note.
    /// 3. No light model: the row is created and passed `skipped` with [`NO_LIGHT_MODEL`].
    /// 4. Otherwise the turn: the row on the frozen agent and model, its evidence pack and prompt,
    ///    one session under [`CAPTURE_DEADLINE`], and the answer checked and written.
    ///
    /// # Errors
    /// Only a lost fence or lease, or a cancel, which leave the row for its owner. Every other
    /// failure settles the row `failed` with its reason and an item note.
    pub(super) async fn capture(
        &self,
        run: &Run,
        capture: &SnapshotCapture,
    ) -> Result<(), EngineError> {
        let steps = self.parts.store.run_steps(run.id).await?;
        if let Some(row) = steps.iter().find(|step| {
            step.role == StepRole::Stage && step.node_key.as_deref() == Some(CAPTURE_KEY)
        }) {
            if row.status == StepStatus::Running {
                self.parts
                    .store
                    .interrupt_step(row.id, CAPTURE_INTERRUPTED, self.now())
                    .await?;
            }
            return Ok(());
        }
        if capture_trigger(&steps).is_none() {
            return Ok(());
        }
        let item = Self::item_of(run)?;
        let fence = StepFence::Lease(self.parts.owner);

        let Some(model) = &capture.model else {
            let row = self.capture_row(run, None).await?;
            let now = self.now();
            self.move_step(
                run.id,
                row.id,
                StepStatus::Pending,
                StepStatus::Running,
                now,
            )
            .await?;
            if !self
                .parts
                .store
                .pass_step(fence, row.id, Some(NO_LIGHT_MODEL), now)
                .await?
            {
                return Err(stale_step(
                    run.id,
                    row.id,
                    StepStatus::Running,
                    StepStatus::Done,
                ));
            }
            return Ok(());
        };

        let row = self.capture_row(run, Some(model)).await?;
        self.move_step(
            run.id,
            row.id,
            StepStatus::Pending,
            StepStatus::Running,
            self.now(),
        )
        .await?;
        self.step_started(run, row.id);
        let settled = match self.capture_turn(run, item, &steps, &row, model).await {
            Ok(Ok(written)) => self.capture_done(run, item, &row, &written).await,
            Ok(Err(reason)) => Err(reason),
            Err(err) if passes_through(&err) => return Err(err),
            Err(err) => Err(failure_text(CAPTURE_KEY, &err)),
        };
        if let Err(reason) = settled {
            let now = self.now();
            self.parts
                .store
                .interrupt_step(row.id, &reason, now)
                .await?;
            self.note(
                item,
                format!("memory capture failed: {reason}"),
                Some(row.id),
                now,
            )
            .await?;
        }
        Ok(())
    }

    /// The capture row (D3): `stage` at [`CAPTURE_POSITION`], keyed and named [`CAPTURE_KEY`],
    /// attempt 1, on the frozen agent and model when there is one.
    async fn capture_row(
        &self,
        run: &Run,
        model: Option<&SnapshotCaptureModel>,
    ) -> Result<RunStep, EngineError> {
        Ok(self
            .parts
            .store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run.id,
                position: CAPTURE_POSITION,
                attempt: 1,
                fanout_index: 0,
                phase_name: CAPTURE_KEY.to_owned(),
                agent_id: model.map(|model| model.agent_id),
                model: model.map(|model| model.model.clone()),
                role: StepRole::Stage,
                parent_step_id: None,
                node_key: Some(CAPTURE_KEY.to_owned()),
            })
            .await?)
    }

    /// D11 step 6's success: the row `running -> done` and the item note of what was written.
    async fn capture_done(
        &self,
        run: &Run,
        item: ItemId,
        row: &RunStep,
        written: &Written,
    ) -> Result<(), String> {
        let now = self.now();
        let done = self
            .move_step(run.id, row.id, StepStatus::Running, StepStatus::Done, now)
            .await;
        if let Err(err) = done {
            return Err(err.to_string());
        }
        // The row is settled: a note that cannot be written is warned, not a second settle.
        if let Err(err) = self.note(item, written.note(), Some(row.id), now).await {
            tracing::warn!(run = %run.id, step = %row.id, %err, "the capture's item note was not written");
        }
        Ok(())
    }

    /// D11 steps 4 and 5: the turn and the writes. The inner `Err` is a reason the row fails
    /// with; the outer one is an engine error, which [`Self::capture`] sorts.
    async fn capture_turn(
        &self,
        run: &Run,
        item: ItemId,
        steps: &[RunStep],
        row: &RunStep,
        model: &SnapshotCaptureModel,
    ) -> Result<Result<Written, String>, EngineError> {
        let prepared = self
            .parts
            .isolator
            .prepare(run.id, row.id, &[], Isolation::Local, None)
            .await?;
        let phase = capture_phase(model);
        let pack = EvidencePack::build(steps, &self.evidence_outputs(steps).await?);
        let prompt = capture_prompt(&pack);
        let trim = prompt
            .trim
            .to_value(self.parts.scrubber)
            .map_err(htui_agent::RecordError::from)?;
        let fence = StepFence::Lease(self.parts.owner);
        self.parts
            .store
            .set_step_prompt(fence, row.id, &prompt.digest, &trim)
            .await?;

        let (mut recorder, cap_batch) = self.open_recorder(run, row, &prompt).await?;
        let key = SessionKey::of(row);
        let driving = Box::pin(self.drive_once(
            run,
            row,
            &phase,
            None,
            &key,
            &prompt.text,
            prepared.cwd.clone(),
            prepared.extra_dirs.clone(),
            Some(tokio::time::Instant::now() + CAPTURE_DEADLINE),
            false,
            &mut recorder,
        ));
        // The run end is not a flight of the walk's scheduler, which is what copies a run cancel
        // into the walk's control: the capture forwards it itself while its session runs.
        let driven = match select(driving, Box::pin(self.forward_run_cancel(run.id))).await {
            Either::Left((driven, _)) => driven,
            Either::Right((never, _)) => match never {},
        };
        let finished = recorder.finish().await;
        let driven = driven?;
        let breach = finished?.cap_breach;
        if driven.cut {
            return Ok(Err(gate::StepFailure::DeadlineElapsed.to_string()));
        }
        let done = match driven.result {
            Ok(done) => done,
            Err(err) => return Ok(Err(format!("session failed: {err}"))),
        };
        if let Some(breach) = breach {
            return Ok(Err(format!(
                "{} ({breach:?})",
                gate::StepFailure::CapBreached { batch: cap_batch }
            )));
        }
        self.parts
            .sink
            .after_done(item, row, &phase, &key, &done)
            .await?;
        let Some(document) = self.output_of(item, &phase, row.id).await? else {
            return Ok(Err(gate::StepFailure::MissingOutput.to_string()));
        };
        let output = match parse_capture(&document.body, &pack, self.parts.scrubber) {
            Ok(output) => output,
            Err(refusal) => return Ok(Err(refusal.to_string())),
        };
        let commit = self.newest_commit(steps).await?;
        Ok(Ok(self.write_entries(row, output, commit).await?))
    }

    /// Copies a cancel on `run`'s own control into its open walk's, now and on every change, for
    /// as long as it is polled. Never ends.
    async fn forward_run_cancel(&self, run: RunId) -> core::convert::Infallible {
        let mut own = (self.parts.control)(run);
        loop {
            self.walks.forward(run, &own);
            own.changed().await;
        }
    }

    /// The latest `command_run.output` of every step the trigger reads (D9's rows), for the
    /// pack's tails. A win is an attempt past the first, so it is one of them.
    async fn evidence_outputs(
        &self,
        steps: &[RunStep],
    ) -> Result<BTreeMap<StepId, String>, EngineError> {
        let mut outputs = BTreeMap::new();
        for step in steps {
            if capture_trigger(std::slice::from_ref(step)).is_none() {
                continue;
            }
            let latest = self
                .parts
                .store
                .command_runs(step.id)
                .await?
                .into_iter()
                .filter_map(|command| command.output)
                .next_back();
            if let Some(output) = latest {
                outputs.insert(step.id, output);
            }
        }
        Ok(outputs)
    }

    /// D4's `commit`, which the engine fills and never the model: the newest `after_hash` of
    /// the run's step commits, the last finished step's first, or `None`.
    async fn newest_commit(&self, steps: &[RunStep]) -> Result<Option<String>, EngineError> {
        let mut ordered: Vec<&RunStep> = steps
            .iter()
            .filter(|step| step.role != StepRole::Stage)
            .collect();
        ordered.sort_by_key(|step| (step.finished_at, step.position, step.attempt));
        for step in ordered.into_iter().rev() {
            let newest = self
                .parts
                .store
                .step_commits(step.id)
                .await?
                .into_iter()
                .find_map(|commit| commit.after_hash);
            if newest.is_some() {
                return Ok(newest);
            }
        }
        Ok(None)
    }

    /// D11 step 5: each kept entry through `write_run_memory`, one by one (the Postgres write cap
    /// relies on a single sequential writer per step).
    async fn write_entries(
        &self,
        row: &RunStep,
        output: CaptureOutput,
        commit: Option<String>,
    ) -> Result<Written, EngineError> {
        let mut written = Written {
            dropped: output.dropped.iter().map(|(_, why)| why.as_str()).collect(),
            ..Written::default()
        };
        for entry in output.kept {
            let answer = self
                .parts
                .store
                .write_run_memory(
                    StepFence::Lease(self.parts.owner),
                    row.id,
                    entry.into_new(MemoryEntryId::new(), commit.clone()),
                )
                .await?;
            match answer {
                RunMemoryWrite::Created(_) => written.wrote += 1,
                RunMemoryWrite::Superseded { .. } => {
                    written.wrote += 1;
                    written.superseded += 1;
                }
                RunMemoryWrite::PersonHolds { .. } => written.kept += 1,
                RunMemoryWrite::CapReached { .. } => written.dropped.push("over_cap"),
            }
        }
        Ok(written)
    }
}
