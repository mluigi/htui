//! The history model (MOD-115 M2 D5, blueprint §8): browser-style back and forward through the
//! places visited, 50 entries, a push only when the coarse place (tab, sub-tab, pane) changes.
//!
//! MOD-115 M2 Task 0 declares the module so the parallel tasks never touch `ui/shell/mod.rs`;
//! Task 3 writes it.
