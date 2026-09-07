//! The way into a replay (MOD-2 milestone 4, T19).
//!
//! What is under test here is the *addressing*, not the rendering: a key pressed in the Backlog
//! tab's Runs pane has to end with the step's rows in a tab that never saw the key (plan D39).
//! The spy tab below stands in for the Chat tab's replay mode, which is what proves the shell
//! names its replay tab by id rather than by type; what that mode then *draws* is `tests/chat.rs`
//! (`chat__replay_*`), beside the live snapshots it has to match (T20, `R-TUI-6`).
#![cfg(feature = "testkit")]

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::KeyEvent;
use htui::app::{Action, Ctx, Handled};
use htui::store_worker::{StoreReply, StoreRequest};
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui::ui::tabs::{ChatTab, Tab, TabId};
use htui_core::fixtures::ids;
use htui_core::model::{Scope, StepId, WorkspaceSummary};
use htui_core::store::MemStore;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Every `StepEvents` reply a [`Spy`] was handed: the step and how many rows came with it.
type Seen = Rc<RefCell<Vec<(StepId, Option<usize>)>>>;

/// A tab that asks for nothing and remembers every `StepEvents` reply addressed to it.
#[derive(Debug, Default)]
struct Spy {
    seen: Seen,
}

impl Spy {
    const ID: TabId = TabId("spy");
}

impl Tab for Spy {
    fn id(&self) -> TabId {
        Self::ID
    }
    fn title(&self) -> &str {
        "Spy"
    }
    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }
    fn on_scope_change(&mut self, _scope: &Scope) {}
    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::StepEvents { step_id, events } = reply {
            self.seen
                .borrow_mut()
                .push((*step_id, events.as_ref().map(Vec::len)));
        }
    }
    fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
}

/// The workspace of the demo fixture with this slug, read out of a throw-away store.
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// Rows down from the arrival row to htui `FEAT-1`, the only item with a recorded step.
const TO_FEAT_1: usize = 3;

/// Moves a settled harness onto `FEAT-1` and opens the Runs pane on it.
async fn on_the_runs_pane(harness: &mut Harness) {
    harness.settle().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.settle().await;
    for _ in 0..TO_FEAT_1 {
        harness.key("j");
        harness.settle().await;
    }
    harness.key("l");
    harness.settle().await;
}

/// The Backlog tab beside a tab that only records what it is sent, named as the replay tab.
async fn with_spy() -> (Harness, Seen) {
    let seen = Seen::default();
    let mut harness = Harness::demo()
        .with_tab(Box::new(BacklogTab::new()))
        .with_tab(Box::new(Spy {
            seen: Rc::clone(&seen),
        }))
        .with_replay_tab(Spy::ID);
    on_the_runs_pane(&mut harness).await;
    (harness, seen)
}

#[tokio::test]
async fn enter_on_a_step_addresses_its_rows_to_the_replay_tab() {
    let (mut harness, seen) = with_spy().await;

    // The cursor arrives on the first step; `J` moves it to `plan`, the recorded one.
    harness.key("J");
    harness.key("enter");
    harness.settle().await;

    assert_eq!(
        harness.app().tabs.active_id(),
        Some(Spy::ID),
        "the replay tab is focused, not the Backlog tab the key was pressed in"
    );
    assert_eq!(
        *seen.borrow(),
        vec![(ids::STEP_PLAN, Some(8))],
        "the reply is delivered to the tab the shell addressed it to"
    );
}

#[tokio::test]
async fn a_step_that_recorded_nothing_arrives_as_a_missing_log() {
    let (mut harness, seen) = with_spy().await;

    // The cursor starts on `prd`, which the fixture has no `session_event` row for.
    harness.key("enter");
    harness.settle().await;

    assert_eq!(
        *seen.borrow(),
        vec![(ids::STEP_PRD, None)],
        "no rows is `None` — the view says `not on this box`, not `an empty conversation`"
    );
}

#[tokio::test]
async fn a_second_replay_supersedes_the_first() {
    let (mut harness, seen) = with_spy().await;

    harness.key("enter");
    harness.settle().await;
    // Back to the Backlog tab, one step down, and replay that one instead.
    harness
        .app()
        .update(Action::Tab(htui::app::TabAction::Focus(BacklogTab::ID)));
    harness.key("J");
    harness.key("enter");
    harness.settle().await;

    assert_eq!(
        seen.borrow().last(),
        Some(&(ids::STEP_PLAN, Some(8))),
        "the newest replay is the one the tab is holding"
    );
    assert_eq!(seen.borrow().len(), 2, "and both were delivered in order");
}

#[tokio::test]
async fn a_shell_that_registered_no_replay_tab_says_so() {
    let mut harness = Harness::demo().with_tab(Box::new(BacklogTab::new()));
    on_the_runs_pane(&mut harness).await;

    harness.app().update(Action::Replay {
        step_id: ids::STEP_PLAN,
    });
    harness.settle().await;

    assert_eq!(
        harness.app().status.as_deref(),
        Some("no tab can replay a step"),
        "a build that can replay nothing refuses on the status line, it does not panic"
    );
}

/// The registration under test is the real one: `register_all` names the Chat tab and binds the
/// Backlog tab's `Enter`.
async fn registered() -> Harness {
    let mut harness = Harness::demo();
    htui::app::register_all(harness.app());
    on_the_runs_pane(&mut harness).await;
    harness
}

#[tokio::test]
async fn the_registered_shell_replays_into_the_chat_tab() {
    let mut harness = registered().await;
    harness.key("J");
    harness.key("enter");
    harness.drive().await;

    assert_eq!(
        harness.app().tabs.active_id(),
        Some(ChatTab::ID),
        "`register_all` names the Chat tab as the replay tab (D39)"
    );
    assert_eq!(
        harness.app().status,
        None,
        "the pane consumed the key, so the keymap's refusal never fired"
    );
}

#[tokio::test]
async fn enter_on_a_pane_that_cannot_replay_says_where_to_press_it() {
    let mut harness = registered().await;
    // Back to the Body pane, which has no step under any cursor.
    harness.key("h");
    harness.key("enter");
    harness.drive().await;

    assert_eq!(
        harness.app().status.as_deref(),
        Some("select a step in the Runs pane (J/K) to replay it")
    );
    assert!(
        harness
            .app()
            .keymap
            .help_line(&htui::keymap::KeyScope::Tab(BacklogTab::ID))
            .contains("Enter replay step"),
        "and the binding is on the Backlog tab's help line"
    );
}

#[tokio::test]
async fn enter_on_a_project_header_still_folds() {
    let mut harness = registered().await;
    // `g` lands on the first project header; the detail pane never sees that `Enter`.
    harness.key("g");
    harness.settle().await;
    harness.key("enter");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        !frame.contains("TUI scaffold"),
        "the group folded, exactly as it did before the pane was offered `Enter`"
    );
    assert_eq!(harness.app().status, None, "and no refusal was raised");
}

#[tokio::test]
async fn the_runs_pane_shows_its_steps_with_a_cursor() {
    let mut harness = Harness::demo().with_tab(Box::new(BacklogTab::new()));
    on_the_runs_pane(&mut harness).await;
    harness.key("J");
    insta::assert_snapshot!("runs_step_selected", harness.render());
}
