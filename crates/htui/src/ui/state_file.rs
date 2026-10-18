//! The UI-state file (MOD-115 M2 D1, blueprint §5): `<config root>/ui-state.toml`, per machine,
//! versioned and written atomically the moment the about panel or a tab's layout changes. A
//! missing file is the first launch; an unreadable or invalid one falls back to the defaults with
//! a reason, and never stops htui starting. The testkit keeps it in memory.
//!
//! MOD-115 M2 Task 0 declares the module so the parallel tasks never touch `ui/mod.rs`; Task 1
//! writes it.
