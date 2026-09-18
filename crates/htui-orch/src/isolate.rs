//! The `Isolator` seam (plan D6): stage 2 of ANA-2 §4.2's walk, behind a trait so that milestone
//! 3's `gix` implementation lands without re-cutting `engine.rs`.
//!
//! **Empty on purpose.** T3 fills this file with the trait alone — a dyn-compatible
//! `Pin<Box<dyn Future … + Send + 'a>>` alias in the shape of
//! `htui_agent::driver::DriverFuture` (`crates/htui-agent/src/driver.rs:37`), never
//! `async_trait`. The four `R-ORCH-8` isolation modes are milestone 3's.
