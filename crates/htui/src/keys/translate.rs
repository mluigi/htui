//! The `[field]` translation (MOD-115 M2 D7, blueprint §3): while the active stack captures text
//! ([`Stack::captures_text`](super::Stack::captures_text)), a chord bound to a `[field]` row
//! becomes that row's canonical editing event, so `TextField`, `TextArea` and `Composer` keep
//! matching their own literals and every editing key is still rebindable.
//!
//! MOD-115 M2 Task 0 declares the module so the parallel tasks never touch `keys/mod.rs`; Task 5
//! writes `FieldEvent` and `translate`, and Task 6 calls it from `App::on_key`.
