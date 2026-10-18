//! The leave dialog (MOD-115 M2 D6, blueprint §9): what leaving htui stops, `Enter` (or a second
//! `ctrl-c`) leaves, `Esc` stays. It replaces the quit question, and `global.quit` always opens
//! it (`R-TUI-18`).
//!
//! MOD-115 M2 Task 0 declares the type and its key stack so the parallel tasks never touch
//! `ui/overlay/mod.rs`; Task 4 writes the dialog, Task 6 registers it and deletes
//! `quit_confirm.rs`.

use crate::keys::{Stack, views};

/// The leave dialog. Task 4 gives it its state and its `Overlay` impl.
#[derive(Debug, Default)]
pub struct LeaveDialog;

impl LeaveDialog {
    /// The dialog's key stack: its own two answers, then `global ∩ {help}`.
    #[must_use]
    pub const fn key_stack() -> Stack<'static> {
        views::LEAVE
    }
}
