//! Deterministic test doubles behind `test-support`: `FakeIsolator`, `TestClock`,
//! `FakeGraphSource` and `FakeOrchestrator` over `MemStore` + `FakeDriver` + `FakeIsolator`.
//!
//! **Empty on purpose.** T3 fills this file, mirroring `htui_agent::fake`'s determinism rules
//! (`crates/htui-agent/src/fake.rs:99-286`). Note for whoever writes `FakeGraphSource`:
//! `MemStore::phase_agents` returns `Vec::new()` unconditionally
//! (`crates/htui-core/src/store/mem.rs:470-473`) and every demo agent is `enabled`, so rungs 1 and
//! 3 of ANA-2 §4.1's candidate chain are both empty on `MemStore::demo()` — the fake carries an
//! explicit per-phase candidates map as its documented stand-in (plan D20, blueprint A-1).
