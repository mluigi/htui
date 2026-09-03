//! The event loop: three `select!` arms over three transports, forever (plan D4, blueprint F).
//!
//! A new tab, overlay, action or store request adds no arm here. The arms are the terminal, the
//! worker's replies and the tick; everything else is an [`Action`] that
//! [`App::update`](crate::app::App::update) applies.

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
            Some(event) = events.next() => app.on_terminal_event(event?),
            Some(envelope) = replies.recv() => app.update(Action::Reply(envelope)),
            _ = ticker.tick() => app.update(Action::Tick),
            else => break,
        }
        if app.should_quit {
            break;
        }
        if std::mem::take(&mut app.dirty) {
            term.terminal_mut().draw(|frame| app.render(frame))?;
        }
    }
    Ok(())
}
