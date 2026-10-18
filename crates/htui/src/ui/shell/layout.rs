//! The layout engine (MOD-115 M2 D2, blueprint §6.1): pure geometry from a tab's body region and
//! [`TabLayout`](super::TabLayout) to its [`PaneAreas`](super::PaneAreas), the visible panes'
//! focus order, mouse hits and the note a hidden pane leaves.
//!
//! MOD-115 M2 Task 0 declares the module so the parallel tasks never touch `ui/shell/mod.rs`;
//! Task 2 writes it.
