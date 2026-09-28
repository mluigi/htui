//! Terminal lifetime (plan D8).
//!
//! Two independent guarantees that the terminal is given back: a [`Drop`] impl for the normal and
//! the error path, and a panic hook for the third one. The PRD's "terminal left raw after a
//! panic" risk is closed by having both, not by remembering to call `restore`.
//!
//! The hook is a *decision* — [`restores_the_terminal`] may answer `false` — and a decision only
//! counts if it is the outermost hook in the process. So [`init`] builds the terminal itself
//! instead of calling `ratatui::init`, which installs a hook of its own that restores the
//! terminal unconditionally *before* calling the one it wrapped (`ratatui-0.30.2/src/init.rs:398`,
//! `:566-572`); behind that one, this predicate is an opinion with no way to act on it (MOD-56
//! D217). Nothing here may take a ratatui init again, and `Suspend::enter` has to say so for
//! itself: the temptation arrives mid-session, with a terminal already up and no ratatui init
//! running to defer to.

use ratatui::{DefaultTerminal, Terminal, backend::CrosstermBackend};

/// Owns the terminal for as long as the shell runs.
#[derive(Debug)]
pub struct TerminalGuard {
    /// The terminal the event loop draws on.
    terminal: DefaultTerminal,
    /// Whether the raw mode and the alternate screen have already been given back.
    restored: bool,
}

/// Installs the panic hook and takes the terminal over.
///
/// Panics if the terminal cannot be put into raw mode, cannot be given the alternate screen, or
/// cannot be measured: the three `expect`s below. That is [`ratatui::init()`]'s contract — it is
/// `try_init().expect(...)` — now covering the two steps this crate performs itself. There is no
/// usable TUI in any of the three cases and `lib.rs::run` has no error path for one.
///
/// The panic is safe because of the order, which is the whole of MOD-56 (D217): the hook goes in
/// before the first `expect`, so it is already the outermost one in the process when any of the
/// three fires, and the restore runs ahead of the unwind reaching the default hook underneath it.
#[must_use]
pub fn init() -> TerminalGuard {
    install_panic_hook();
    // `ratatui::try_init`'s body, minus its `set_panic_hook` (`init.rs:398-402`). Written out
    // because the order is the fix: the hook above is the outermost one from here to the end of
    // the process, and a second one — restoring unconditionally ahead of it — would put this
    // crate back where MOD-56 found it, with the predicate right and the terminal gone anyway.
    // `tests/panic_hook_order.rs` is what keeps this shape.
    crossterm::terminal::enable_raw_mode().expect("htui cannot put the terminal into raw mode");
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)
        .expect("htui cannot enter the alternate screen");
    let backend = CrosstermBackend::new(std::io::stdout());
    TerminalGuard {
        terminal: Terminal::new(backend).expect("htui cannot measure the terminal"),
        restored: false,
    }
}

/// Chains a hook that restores the terminal before the previous hook prints the panic.
///
/// Without it a panic leaves the alternate screen up and raw mode on, and the backtrace is drawn
/// over the TUI's last frame.
pub fn install_panic_hook() {
    install_panic_hook_restoring(ratatui::restore);
}

/// The hook, with the restore it performs as a parameter.
///
/// [`install_panic_hook`] is this with `ratatui::restore`, which is the only restore this crate
/// ever wants. The seam is public for one reason: the test that has to prove the *decision*
/// reaches the restore runs the real chain, and it is an integration test in another binary that
/// cannot see a private function (MOD-56 D218). Before the seam it could only assert
/// [`restores_the_terminal`], which is what made the defect invisible: the predicate was right
/// and the terminal was still torn down underneath the event loop, by a hook outside it.
pub fn install_panic_hook_restoring(restore: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if restores_the_terminal() {
            restore();
        }
        previous(info);
    }));
}

/// Whether a panic reaching the hook *right now* should give the terminal back.
///
/// Review finding M1. The hook fires on **any** panic on **any** thread, including one that a
/// `catch_unwind` further down the stack is about to swallow — `catch_unwind` does not stop the
/// hook, it only stops the unwind. `htui_agent::excerpt::run_providers` catches exactly such a
/// panic on purpose (hazard H-20: a provider that panics is dropped and recorded, and the prompt
/// is assembled without it), so restoring there left the process with no alternate screen and no
/// raw mode while the event loop carried on drawing — the wedged-UI shape, caused by a defect the
/// program had already decided to survive.
///
/// The question is asked of the agent crate because that is the crate that does the catching; this
/// one does not get to guess which panics are fatal. Every other panic is fatal, so the answer is
/// `true` for all of a normal run.
#[must_use]
pub fn restores_the_terminal() -> bool {
    !htui_agent::excerpt::panic_is_contained()
}

impl TerminalGuard {
    /// The terminal, for drawing.
    pub fn terminal_mut(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }

    /// Gives the terminal back. Idempotent: [`Drop`] calls it again and it does nothing.
    pub fn restore(&mut self) {
        if !self.restored {
            ratatui::restore();
            self.restored = true;
        }
    }
}

impl crate::editor::Suspend for TerminalGuard {
    /// Show the cursor (every draw hid it), then `ratatui::try_restore` (MOD-9 D22). `restored` is
    /// not touched: this is a pause, not the end.
    fn leave(&mut self) -> std::io::Result<()> {
        self.terminal.show_cursor()?;
        ratatui::try_restore()
    }

    /// Raw mode and the alternate screen back, then `Terminal::clear`, which resets the back
    /// buffer so the next draw is whole (`ratatui-core-0.1.2/src/terminal/buffers.rs:147-173`).
    /// Not `ratatui::init()`, and now for two reasons: it would stack another panic hook
    /// (`init.rs:398`), and since MOD-56 the hook it would wrap is the one that decides whether a
    /// panic gives the terminal back at all — so on every editor suspend it would re-break the
    /// defect this file just closed (D221).
    fn enter(&mut self) -> std::io::Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        self.terminal.clear()
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
