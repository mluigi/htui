# MOD-29 - Bugsink integration via Sentry crate

## Context
See `docs/ANA-15.md` for context. The goal was to add Sentry integration configured to point to a self-hosted Bugsink instance in order to capture crashes and unhandled errors.

## Decision
- We added `sentry` and `sentry-anyhow` (version 0.34.0) to `[workspace.dependencies]` and `crates/htui/Cargo.toml`.
- In `crates/htui/src/lib.rs::run`, we initialized `sentry` with the Bugsink DSN right after tracing is initialized.
- Unhandled `anyhow` errors returned from `htui::run` are captured using `sentry_anyhow::capture_anyhow` before exiting.
- The default Sentry panic hook automatically captures panics.

## Consequences
- The binary now requires an internet connection on startup if Sentry is to send events, but it operates non-blockingly.
- Crashes and unhandled errors during application runtime are sent to the Bugsink instance.
