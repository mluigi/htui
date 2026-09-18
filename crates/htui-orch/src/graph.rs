//! Resolution: a live `step_graph` becomes the typed `GraphSnapshot` a run is decided by.
//!
//! ANA-2 §4.1's field chains (`docs/ANA-2.md:278-288`), the `topology` digest the snapshot row
//! reserves for this milestone (`crates/htui-core/src/model/run.rs:453`), the scope resolution
//! that refuses an empty scope for an item with a primary repo (plan D14), and the override clone
//! that is deep over `step_graph_phase` and never over `skill_binding` (`docs/ANA-2.md:290-295`).
//!
//! **Empty on purpose until T2's third commit.**
