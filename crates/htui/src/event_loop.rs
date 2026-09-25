//! The event loop: three `select!` arms over three transports, forever (plan D4, blueprint F).
//!
//! A new tab, overlay, action or store request adds no arm here. The arms are the terminal, the
//! worker's replies and the tick; everything else is an [`Action`] that
//! [`App::update`](crate::app::App::update) applies. One post-step, not an arm, suspends the
//! terminal for `$EDITOR` (MOD-9 D9).

use std::time::Duration;

use futures::StreamExt;
use tokio::sync::mpsc;

use crate::app::{Action, App};
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
            _ = ticker.tick() => app.update(Action::Tick),
        }
        if app.should_quit {
            break;
        }
        // MOD-9 D9: a view asked for `$EDITOR`. The stream goes first: crossterm 0.29 parks a
        // thread in `poll_internal(None, ..)` that reads the tty until the stream is dropped
        // (`crossterm-0.29.0/src/event/stream.rs:44-55`, `:140-145`), and it would take the
        // editor's keys. A fresh stream after; `reset` so a long edit does not replay a burst of
        // ticks (`interval` is `Burst`). Replies queue in the unbounded channel meanwhile. A
        // terminal that cannot be taken back is an error: `lib.rs` restores and exits (D21).
        if let Some((tab, edit)) = app.take_external_edit() {
            drop(events);
            let outcome = crate::editor::run_suspended(
                term,
                &crate::editor::EditorCommand::from_env(),
                &edit,
            )
            .await?;
            events = crossterm::event::EventStream::new();
            ticker.reset();
            app.finish_external_edit(tab, outcome);
        }
        if std::mem::take(&mut app.dirty) {
            term.terminal_mut().draw(|frame| app.render(frame))?;
        }
    }
    Ok(())
}
