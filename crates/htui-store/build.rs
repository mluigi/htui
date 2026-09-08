//! Rebuild triggers for the two embedded migration sets.
//!
//! `sqlx::migrate!` (`src/lib.rs`) expands to the *contents* of the files present when the macro
//! ran, and registers a rerun-if-changed for each of those files — not for the directory holding
//! them. Adding a **new** `000N_*.sql` therefore does not invalidate a warm `target/`: the crate
//! is not recompiled, `MIGRATOR` still holds the old set, and the migration suite fails against a
//! schema the binary does not know it has. That was reproduced while landing
//! `0002_agent_probe.sql` (MOD-2 milestone 5) and it will fire again for MOD-4's `0003`.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=cache_migrations");
}
