# CLEAN-10 - `cargo doc` fails on private intra-doc links (done, 2026-10-13)

**Origin:** MOD-12 M2's gate.

**What was done.** Run in a sandbox on `hr/CLEAN-10`, routed as a plan (no criteria fired). Docs only: no code
or behaviour changed.

The item text said 17 errors in `htui-core` and `htui-store`. Measured at the start of the run with
`cargo doc --workspace --no-deps --all-features --keep-going` it was **33 errors in `htui-store` (16),
`htui-agent` (2) and `htui` (15)**, none in `htui-core`; the rest had accrued since M2.

- **Private targets** (`cas_miss`, `step_scope`, `revise_requirement`, `finish_run_on`, `StepLock::*`,
  `HOOKS_OFF`, `give_way`, `TABS`/`MODAL`, `prepare`/`install`, ...) became plain code spans; the prose is unchanged.
- **Redundant targets** (`[`views`](super::views)` in `keys/stack.rs`) were shortened to ``[`views`]``.
- **Unresolved links.** `ui/tabs/settings/queue.rs` module docs resolve in the parent's scope, so
  `parse_usd`, `format_usd`, `parse_window` and `QueueSetting::validate` now carry their `htui_core::model::` path.
  The same quirk broke `ensure_model` in `htui-store/src/model.rs` (`//!` after an outer `///` on `pub mod model`),
  now ``[`ensure_model`](crate::model::ensure_model)``.
- **Without features.** `cargo doc --workspace --no-deps` also failed on three `HashEmbedder` links in
  `htui-store/src/embed.rs` (the type is `test-support` only); they are code spans now.

**Gate.** `cargo doc --workspace --no-deps` with and without `--all-features`: 0 diagnostics. `cargo fmt --check`,
`cargo clippy --workspace -- -D warnings` and `cargo test -p htui-agent --all-features -- --test-threads=1` green.

Commit: `ef6515ee`.
