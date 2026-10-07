//! The event loop: four `select!` arms over four transports, forever (plan D4, blueprint F).
//!
//! A new tab, overlay, action or store request adds no arm here. The arms are the terminal, the
//! worker's replies, the in-pane editor's events (MOD-57 P7) and the tick; everything else is an
//! [`Action`] that [`App::update`](crate::app::App::update) applies. Post-steps, not arms: the
//! `$EDITOR` step suspends the terminal (MOD-9 D9) or opens the in-pane editor (MOD-57, with
//! `HTUI_EDITOR_PANE`); then mouse capture is set to what the view on screen wants (MOD-71 D2),
//! through `App::mouse_capture`, which tells the tabs when it goes off (MOD-74 D1); then the dirty
//! draw; then the pane resize (MOD-57 PD-1), which needs the rect the draw just claimed.

use std::time::Duration;

use futures::StreamExt;
use tokio::sync::mpsc;

use crate::app::{Action, App};
use crate::editor::pane::{PaneChild, PaneEvent, PtyChild};
use crate::editor::{EditorCommand, EditorMode};
use crate::store_worker::ReplyEnvelope;
use crate::terminal::TerminalGuard;

/// How often the shell wakes up on its own. Every fourth tick refreshes the top bar.
pub const TICK: Duration = Duration::from_millis(250);

/// Draws the first frame, then runs until `App::should_quit`.
///
/// `io::Result`, not `anyhow`: `anyhow` belongs to `main` (plan Patterns).
pub async fn run(
    term: &mut TerminalGuard,
    app: &mut App,
    mut replies: mpsc::UnboundedReceiver<ReplyEnvelope>,
) -> std::io::Result<()> {
    let mut events = crossterm::event::EventStream::new();
    let mut ticker = tokio::time::interval(TICK);
    // MOD-57 P7: the in-pane editor's transport. The loop keeps a sender, so `recv` never ends.
    let (pane_tx, mut pane_rx) = mpsc::unbounded_channel::<PaneEvent>();

    term.terminal_mut().draw(|frame| app.render(frame))?;
    loop {
        tokio::select! {
            event = events.next() => match event {
                // crossterm's `EventStream` does not end in practice - errors arrive as
                // `Some(Err(..))` - but matching `None` explicitly keeps exhausted input a
                // shutdown instead of an arm that silently disables itself. No `else` arm:
                // `ticker.tick()` is irrefutable, so `select!` can never run out of branches.
                Some(event) => app.on_terminal_event(event?),
                None => break,
            },
            Some(envelope) = replies.recv() => app.update(Action::Reply(envelope)),
            Some(event) = pane_rx.recv() => app.on_pane_event(event),
            _ = ticker.tick() => app.update(Action::Tick),
        }
        if app.should_quit {
            break;
        }
        // MOD-9 D9: a view asked for `$EDITOR`. MOD-57 P3: `$VISUAL`/`$EDITOR` and
        // `HTUI_EDITOR_PANE`, read once per edit through one lookup.
        if let Some((tab, edit)) = app.take_external_edit() {
            let lookup = |key: &str| std::env::var(key).ok();
            let cmd = EditorCommand::resolve(lookup);
            match EditorMode::resolve(lookup) {
                // MOD-57 P1: the editor runs on a pseudo-terminal drawn inside htui; its events
                // come back on the pane arm.
                EditorMode::Pane => app.open_editor(tab, &edit, cmd, |id, command, size| {
                    PtyChild::spawn(command, size, id, pane_tx.clone())
                        .map(|child| Box::new(child) as Box<dyn PaneChild>)
                }),
                // The stream goes first: crossterm 0.29 parks a thread in
                // `poll_internal(None, ..)` that reads the tty until the stream is dropped
                // (`crossterm-0.29.0/src/event/stream.rs:44-55`, `:140-145`), and it would take
                // the editor's keys. A fresh stream after; `reset` so a long edit does not replay
                // a burst of ticks (`interval` is `Burst`). Replies queue in the unbounded
                // channel meanwhile. A terminal that cannot be taken back is an error: `lib.rs`
                // restores and exits (D21).
                EditorMode::Suspend => {
                    drop(events);
                    let outcome = crate::editor::run_suspended(term, &cmd, &edit).await?;
                    events = crossterm::event::EventStream::new();
                    ticker.reset();
                    app.finish_external_edit(tab, outcome);
                }
            }
        }
        // MOD-71 D2: capture follows the view on screen. Asked after every step, the editor's
        // included, so a toggle `v` or `Esc` caused lands before the frame it changed; the guard
        // writes only a change. MOD-74 D1: `mouse_capture` also tells the tabs when capture goes
        // off.
        term.set_mouse_capture(app.mouse_capture())?;
        if std::mem::take(&mut app.dirty) {
            term.terminal_mut().draw(|frame| app.render(frame))?;
        }
        // MOD-57 PD-1: after the draw, which is what knows the pane's rect.
        app.resize_editor();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// This file up to its tests, with its comments removed.
    fn code() -> String {
        let file = include_str!("event_loop.rs");
        file[..file.find("#[cfg(test)]").unwrap_or(file.len())]
            .lines()
            .map(|line| line.find("//").map_or(line, |at| &line[..at]))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// MOD-71 D2, review H1: capture is asked for after the `$EDITOR` post-step, so the editor's
    /// `leave` cannot leave it off, and before the dirty draw, so a toggle lands before the frame
    /// it changed.
    #[test]
    fn the_loop_sets_mouse_capture_after_the_editor_and_before_the_draw() {
        let code = code();
        let editor = code
            .find("app.finish_external_edit(")
            .expect("the editor post-step");
        let capture = code
            .find("term.set_mouse_capture(app.mouse_capture())?;")
            .expect("the loop sets capture through the app's edge (MOD-74 D1)");
        assert!(
            !code.contains("app.wants_mouse()"),
            "the loop never bypasses the edge (MOD-74 D1)"
        );
        let draw = code
            .find("std::mem::take(&mut app.dirty)")
            .expect("the dirty draw");
        assert!(editor < capture && capture < draw);
        assert_eq!(
            code.matches("set_mouse_capture(").count(),
            1,
            "one place decides"
        );
    }

    /// MOD-57 P7, PD-1: the in-pane editor is a fourth arm, opened by the `$EDITOR` post-step
    /// (one environment read per edit) and resized after the draw, which is what knows its rect.
    #[test]
    fn the_pane_is_an_arm_opened_by_the_editor_step_and_resized_after_the_draw() {
        let code = code();
        assert!(
            code.contains("app.on_pane_event("),
            "the pane's transport is an arm"
        );
        let open = code.find("app.open_editor(").expect("the pane step");
        let capture = code
            .find("term.set_mouse_capture(app.mouse_capture())?;")
            .expect("capture");
        let draw = code
            .find("std::mem::take(&mut app.dirty)")
            .expect("the dirty draw");
        let resize = code.find("app.resize_editor();").expect("the resize step");
        assert!(open < capture && capture < draw && draw < resize);
        assert_eq!(code.matches("EditorMode::resolve(").count(), 1);
        assert_eq!(
            code.matches("std::env::var(").count(),
            1,
            "one lookup per edit"
        );
        assert!(!code.contains("EditorCommand::from_env()"));
    }
}
