//! MOD-42 plan D6: one turn of a session, with the permission relay and the run's control.

use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{
    OpenPermission, PermissionChoice, PermissionId, PermissionStatus, RelaySessionId, RunId,
    StepId, StepPermission,
};
use htui_core::store::Result as StoreResult;
use tokio::sync::watch;
use uuid::Uuid;

use super::{Recorder, enforce_breach};
use crate::driver::{AgentSession, PermissionPolicy};
use crate::error::DriverError;
use crate::event::{DoneEvent, DriverEvent};

/// D8: how often a parked request's row is read. One primary-key read per tick, only while parked.
pub const RELAY_POLL: Duration = Duration::from_secs(1);
/// B-16: the grace a cancel `drive` discovers itself (a `stale`/`cancelled` row) is given.
pub const RELAY_GRACE: Duration = Duration::from_secs(2);

/// What a walk is asked to do (D10, D11). `Copy`, read with [`Control::signal`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Signal {
    /// Carry on.
    #[default]
    Run,
    /// Stop gracefully: answer parked requests `cancelled`, `session.cancel(grace)`, drain.
    Cancel {
        /// The window `AgentSession::cancel` is given.
        grace: Duration,
    },
}

impl Signal {
    /// Whether this is [`Signal::Cancel`].
    #[must_use]
    pub const fn is_cancel(self) -> bool {
        matches!(self, Self::Cancel { .. })
    }
}

/// One walk's view of its run's [`Signal`] (D10): a `watch` receiver, or none ("never").
#[derive(Debug, Clone)]
pub struct Control(Option<watch::Receiver<Signal>>);

impl Control {
    /// A control no signal ever reaches: `pump`, and every engine no runtime cancels.
    #[must_use]
    pub const fn never() -> Self {
        Self(None)
    }

    /// Over a receiver of the run's channel.
    #[must_use]
    pub const fn new(receiver: watch::Receiver<Signal>) -> Self {
        Self(Some(receiver))
    }

    /// The current signal. R-8: `changed()` is edge-triggered, so every wait reads this first.
    #[must_use]
    pub fn signal(&self) -> Signal {
        self.0.as_ref().map_or(Signal::Run, |rx| *rx.borrow())
    }

    /// Resolves when the value changes. Pending for ever for [`Control::never`] and once the
    /// sender is gone (a dropped sender is not a cancel).
    pub async fn changed(&mut self) {
        // Not a match guard: a binding is immutable inside its guard, and `changed` takes `&mut`.
        if let Some(rx) = &mut self.0
            && rx.changed().await.is_ok()
        {
            return;
        }
        std::future::pending::<()>().await;
    }
}

/// A fresh channel at [`Signal::Run`] and a control over it (tests, the fake orchestrator).
#[must_use]
pub fn control_channel() -> (watch::Sender<Signal>, Control) {
    let (sender, receiver) = watch::channel(Signal::Run);
    (sender, Control::new(receiver))
}

/// What `drive` needs to relay a stage-3 request (D6). Built per session by the engine.
pub struct Relay<'a, R: htui_core::store::RelayStore> {
    /// The executor's store.
    pub store: &'a R,
    /// The executor's lease owner (`EngineParts::owner`), the row's `owner` (I-2).
    pub owner: Uuid,
    /// The run.
    pub run: RunId,
    /// The step the session drives.
    pub step: StepId,
    /// This session (minted per `drive_once`).
    pub session: RelaySessionId,
    /// The agent's policy, stages 1-2 (D9).
    pub policy: &'a PermissionPolicy,
    /// Parked-row poll interval: [`RELAY_POLL`] in production (D8, B-18).
    pub poll: Duration,
    /// B-16's grace.
    pub grace: Duration,
    /// The instant source for the rows `htui` authors (`record_permission_answer`'s `at`).
    pub now: &'a (dyn Fn() -> DateTime<Utc> + Sync),
}

impl<R: htui_core::store::RelayStore> core::fmt::Debug for Relay<'_, R> {
    /// The ids and the timings. The store, the policy and the clock are not printed: a store need
    /// not be `Debug`, and neither of the other two is what a relay log is about.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Relay")
            .field("owner", &self.owner)
            .field("run", &self.run)
            .field("step", &self.step)
            .field("session", &self.session)
            .field("poll", &self.poll)
            .field("grace", &self.grace)
            .finish_non_exhaustive()
    }
}

/// `pump`'s relay type (D6): uninhabited, so `None::<&Relay<'_, NoRelay>>` names a relay that
/// cannot exist.
#[derive(Debug, Clone, Copy)]
pub enum NoRelay {}

impl htui_core::store::RelayStore for NoRelay {
    async fn open_permission(&self, _open: OpenPermission) -> StoreResult<PermissionId> {
        match *self {}
    }

    async fn permission(&self, _id: PermissionId) -> StoreResult<Option<StepPermission>> {
        match *self {}
    }

    async fn apply_permission(
        &self,
        _id: PermissionId,
        _owner: Uuid,
    ) -> StoreResult<Option<PermissionChoice>> {
        match *self {}
    }

    async fn settle_permissions(
        &self,
        _session: RelaySessionId,
        _to: PermissionStatus,
    ) -> StoreResult<u64> {
        match *self {}
    }
}

/// D6: one turn of `session` into `recorder`, ending at its `done`, a graceful cancel, or an error.
///
/// MOD-42 T2 red: today's pump loop; the relay and the control are not read yet.
///
/// # Errors
/// [`DriverError::Closed`] for a stream that ended without `done`; the recorder's failures.
pub async fn drive<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: Option<&Relay<'_, R>>,
    control: &mut Control,
) -> Result<DoneEvent, DriverError> {
    let _ = (relay, control);
    loop {
        let Some(envelope) = session.next_event().await? else {
            return Err(DriverError::Closed);
        };
        let done = match &envelope.event {
            DriverEvent::Done(done) => Some(*done),
            _ => None,
        };
        if let Some(breach) = recorder.record(envelope).await? {
            return enforce_breach(session, recorder, breach).await;
        }
        if let Some(done) = done {
            return Ok(done);
        }
    }
}
