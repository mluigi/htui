//! MOD-125 M4 T4: `htui stop` and `htui resume` over **Postgres**, through `stop_cmd`'s cores
//! (`stop_on`, `resume_on`, `report`), never the binary: the keyring is never read.
//!
//! What only this file can prove: the verbs write and delete the `workspace_stop` row with the
//! connected user, and their report reads through `PgStore`'s `workspace_live_runs`, `boxes`,
//! `item`, `agents` (the caps), `run_steps` and `step_events` (a stopped step's forecast). Every
//! case runs over `testkit::demo_db()` and returns early, printing `testkit::SKIP`, with
//! `HTUI_TEST_DATABASE_URL` unset.
#![cfg(feature = "testkit")]

use std::collections::HashMap;

use chrono::{TimeDelta, Utc};
use htui::state_cmd::StateExit;
use htui::stop_cmd::{self, Ask, Context, STOP_POLL, STOP_WAIT};
use htui_agent::driver::DriverCaps;
use htui_agent::event::SESSION_STARTED;
use htui_core::fixtures::ids;
use htui_core::model::{
    AgentId, Claim, EventKind, EventRole, ItemId, NewItem, NewRun, NewRunStep, RunId, RunMode,
    SessionEvent, StepId, StepRole, StepStatus, StopMode, WorkspaceStopState, WorkspaceSummary,
};
use htui_core::store::{ReadStore as _, StepFence, WriteStore as _};
use htui_store::testkit;
use serde_json::json;
use uuid::Uuid;

/// What `run_*` would read from the store before the core.
struct Inputs {
    caps: HashMap<AgentId, DriverCaps>,
    workspaces: Vec<WorkspaceSummary>,
}

impl Inputs {
    async fn of(db: &testkit::TestDb) -> Self {
        Self {
            caps: stop_cmd::caps_map(db.store.agents().await.expect("agents")),
            workspaces: db.store.workspaces().await.expect("workspaces"),
        }
    }

    fn ctx(&self, db: &testkit::TestDb) -> Context<'_> {
        Context {
            user: db.store.this_user(),
            now: Utc::now,
            caps: &self.caps,
            workspaces: &self.workspaces,
            wait: STOP_WAIT,
            poll: STOP_POLL,
        }
    }
}

/// `--yes` without a terminal: nothing is asked, and nothing may be read.
async fn stop(
    db: &testkit::TestDb,
    inputs: &Inputs,
    mode: StopMode,
) -> (Result<(), StateExit>, String) {
    let mut input = std::io::empty();
    let mut err = Vec::new();
    let mut out = Vec::new();
    let mut ask = Ask {
        yes: true,
        interactive: false,
        input: &mut input,
        question: &mut err,
    };
    let ended = stop_cmd::stop_on(
        &db.store,
        &inputs.ctx(db),
        "graphics",
        mode,
        &mut ask,
        &mut out,
    )
    .await;
    assert_eq!(String::from_utf8(err).expect("utf-8"), "", "nothing asked");
    (ended, String::from_utf8(out).expect("utf-8"))
}

/// A graph run of a fresh `htui` item in `graphics`, claimed on the fixture box, with one running
/// `implement` step on an agent that resumes sessions, whose log holds a `session_started` banner,
/// and stopped by its executor (`stop_run`), as M2 stops one.
async fn stopped_on_the_box(db: &testkit::TestDb) -> RunId {
    let item = db
        .store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            title: "stopped on the box".to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            target_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("mint");
    let snapshot = db
        .store
        .run(ids::RUN_8)
        .await
        .expect("read")
        .and_then(|run| run.graph_snapshot)
        .expect("the demo's runs carry a snapshot");
    let run = db
        .store
        .create_run(NewRun {
            id: RunId::new(),
            project_id: ids::PROJECT_HTUI,
            item_id: item.id,
            mode: RunMode::Manual,
            target_box_id: ids::BOX,
            executing_box_id: None,
            started_by: ids::USER,
            graph_snapshot: serde_json::from_value(snapshot).expect("a snapshot decodes"),
            repo_scope: Vec::new(),
            queued_at: Utc::now(),
            batch_id: None,
            workspace_id: Some(ids::WORKSPACE_GRAPHICS),
            target: None,
        })
        .await
        .expect("create")
        .id;
    let owner = Uuid::now_v7();
    assert_eq!(
        db.store
            .claim_run(run, ids::BOX, owner, Utc::now(), TimeDelta::seconds(600))
            .await
            .expect("claim"),
        Claim::Admitted
    );
    let step = db
        .store
        .create_step(NewRunStep {
            id: StepId::new(),
            run_id: run,
            position: 0,
            attempt: 1,
            fanout_index: 0,
            phase_name: "implement".to_owned(),
            agent_id: Some(ids::AGENT_CLAUDE_CLI),
            model: None,
            role: StepRole::Step,
            parent_step_id: None,
            node_key: None,
        })
        .await
        .expect("create step")
        .id;
    assert!(
        db.store
            .transition_step(step, StepStatus::Pending, StepStatus::Running, Utc::now())
            .await
            .expect("start")
    );
    let rows: Vec<SessionEvent> = [
        (EventKind::Prompt, json!({ "text": "go" })),
        (
            EventKind::Other,
            json!({ "update": SESSION_STARTED, "body": { "session_id": "sess_1" } }),
        ),
    ]
    .into_iter()
    .zip(0..)
    .map(|((kind, payload), seq)| SessionEvent {
        run_step_id: step,
        seq,
        turn: 0,
        kind,
        role: EventRole::Agent,
        tool_call_id: None,
        payload,
        raw: None,
        at: Utc::now(),
    })
    .collect();
    db.store
        .append_events(StepFence::Lease(owner), &rows)
        .await
        .expect("append");
    assert!(db.store.stop_run(run, owner).await.expect("stop_run"));
    run
}

/// D5, D6: `stop --yes` writes the row with the connected user and reports `stopped` with each
/// run (the box's hostname, the item keys); `resume --yes` deletes it and says so.
#[tokio::test]
async fn a_stop_and_a_resume_round_trip_on_postgres() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let run = stopped_on_the_box(&db).await;
    let inputs = Inputs::of(&db).await;

    let (ended, out) = stop(&db, &inputs, StopMode::Graceful).await;
    ended.expect("stopped");
    let row = db
        .store
        .workspace_stop(ids::WORKSPACE_GRAPHICS)
        .await
        .expect("read")
        .expect("a stop row");
    assert_eq!(row.mode, StopMode::Graceful);
    assert_eq!(row.requested_by, db.store.this_user());
    assert!(
        out.starts_with("workspace graphics: stopped (graceful, requested "),
        "{out}"
    );
    let run8 = &ids::RUN_8.to_string()[..8];
    let ours = &run.to_string()[..8];
    assert!(
        out.contains(&format!(
            "{run8}  stopped, no box          cannot be resumed: cancel it from the Runs pane"
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "{ours}  stopped on DESKTOP-HTUI  resumes: implement reopens its agent session"
        )),
        "{out}"
    );
    assert!(out.contains("  FEAT-1 "), "the item key: {out}");

    let graphics = inputs
        .workspaces
        .iter()
        .find(|ws| ws.workspace_id == ids::WORKSPACE_GRAPHICS)
        .expect("graphics");
    let report = stop_cmd::report(&db.store, &inputs.ctx(&db), graphics)
        .await
        .expect("report");
    assert_eq!(report.state, WorkspaceStopState::Stopped);
    assert_eq!(report.lines.len(), 2, "{report}");

    let mut input = std::io::empty();
    let mut err = Vec::new();
    let mut out = Vec::new();
    let mut ask = Ask {
        yes: true,
        interactive: false,
        input: &mut input,
        question: &mut err,
    };
    stop_cmd::resume_on(&db.store, &inputs.ctx(&db), "graphics", &mut ask, &mut out)
        .await
        .expect("resumed");
    assert_eq!(
        db.store
            .workspace_stop(ids::WORKSPACE_GRAPHICS)
            .await
            .expect("read"),
        None
    );
    let out = String::from_utf8(out).expect("utf-8");
    assert!(
        out.starts_with(
            "resume requested: each box resumes its stopped runs at its next command poll\n\
             workspace graphics: not stopped\n"
        ),
        "{out}"
    );
    db.drop_db().await;
}

/// D5: `--force` over a graceful stop escalates the row; a graceful request over the forced row
/// writes nothing and says it does not downgrade it.
#[tokio::test]
async fn a_forced_stop_escalates_on_postgres() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let inputs = Inputs::of(&db).await;
    let mode = async || {
        db.store
            .workspace_stop(ids::WORKSPACE_GRAPHICS)
            .await
            .expect("read")
            .map(|row| row.mode)
    };
    stop(&db, &inputs, StopMode::Graceful)
        .await
        .0
        .expect("graceful");
    assert_eq!(mode().await, Some(StopMode::Graceful));
    let (ended, out) = stop(&db, &inputs, StopMode::Forced).await;
    ended.expect("escalated");
    assert_eq!(mode().await, Some(StopMode::Forced));
    assert!(
        out.starts_with("workspace graphics: stopped (forced, "),
        "{out}"
    );
    let (ended, out) = stop(&db, &inputs, StopMode::Graceful).await;
    ended.expect("a repeat is answered");
    assert!(
        out.starts_with(
            "workspace graphics is already stopped (forced)\na graceful stop does not downgrade \
             a forced one\n"
        ),
        "{out}"
    );
    assert_eq!(mode().await, Some(StopMode::Forced));
    db.drop_db().await;
}
