//! `htui-worker`: run supervision without a UI (MOD-41 plan D6), the runtime the TUI and
//! `htui worker` share.
//!
//! `htui-orch` never names `htui-store` (ANA-2 invariant 10), so everything that joins the
//! orchestrator to a store host lives here. The crate links no terminal crate: never `ratatui`,
//! never `crossterm` (`tests/deps.rs`).
#![warn(missing_docs)]
