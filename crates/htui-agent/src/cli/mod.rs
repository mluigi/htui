//! The degraded CLI transport: an agent that speaks only its own headless JSON stream, reaching
//! the same chat tab, recorder, store rows and replay as an ACP one (`docs/ANA-4.md` §4.3, §4.4,
//! §6.2, §7; MOD-2 milestone 8).
//!
//! The split mirrors `acp/`: the supervisor and the session task live in this file, and the
//! wire → [`DriverEvent`] mapping lives alone in [`claude`], which imports no process type and is
//! unit-testable from a single recorded line.
//!
//! [`DriverEvent`]: crate::event::DriverEvent

pub mod claude;
