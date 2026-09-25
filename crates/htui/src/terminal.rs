//! Terminal lifetime (plan D8).
//!
//! Two independent guarantees that the terminal is given back: a [`Drop`] impl for the normal and
//! the error path, and a panic hook for the third one. The PRD's "terminal left raw after a
//! panic" risk is closed by having both, not by remembering to call `restore`.

use ratatui::DefaultTerminal;

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
/// Panics if the terminal cannot be put into raw mode, which is
/// [`ratatui::init()`]'s contract; there is no usable TUI in that case.
#[must_use]
pub fn init() -> TerminalGuard {
    install_panic_hook();
    TerminalGuard {
        terminal: ratatui::init(),
        restored: false,
    }
}

/// Chains a hook that restores the terminal before the previous hook prints the panic.
///
/// Without it a panic leaves the alternate screen up and raw mode on, and the backtrace is drawn
/// over the TUI's last frame.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if restores_the_terminal() {
            ratatui::restore();
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
    /// Not `ratatui::init()`: that would stack another panic hook (`init.rs:397-403`).
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
