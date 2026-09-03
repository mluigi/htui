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
/// [`ratatui::init`]'s contract; there is no usable TUI in that case.
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
        ratatui::restore();
        previous(info);
    }));
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

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
