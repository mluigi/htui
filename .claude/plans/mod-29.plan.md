# Plan: MOD-29 - Bugsink integration via Sentry crate

**Status:** Done (2026-09-19)

## Tasks

### 1. Add Dependencies
Add `sentry` and `sentry-anyhow` to the workspace `Cargo.toml` and to `crates/htui/Cargo.toml`.
- Root `Cargo.toml`: Add `sentry = "0.34"` and `sentry-anyhow = "0.34"` to `[workspace.dependencies]`. (Or whatever the latest 0.x is, let's say "0.34" or "0.36").
- `crates/htui/Cargo.toml`: Add them to `[dependencies]` using `{ workspace = true }`.

### 2. Wire Sentry Initialization
In `crates/htui/src/lib.rs`, initialize the Sentry client at the start of `run(...)`:
- Call `sentry::init` with the provided DSN: `https://31abc84ddc174750a380b54b21c45f1c@bugsink.sette.mluigi.it/2`
- Keep the `_guard` returned by `sentry::init` alive for the duration of `run`.
- Where `run` returns `Err(error)`, use `sentry_anyhow::capture_anyhow(&error)` to capture the anyhow error before returning it.

## Fact-check claims
| Claim | Verdict | Evidence |
|---|---|---|
| `sentry` and `sentry-anyhow` can be added to the workspace `Cargo.toml` | Verified | The workspace `Cargo.toml` manages all dependencies. |
| The entry point `run` in `crates/htui/src/lib.rs` is the right place for initialization | Verified | `main.rs` calls `htui::run(args)` which controls the application lifecycle. |

## Execution
Since this is an independent, narrow task, it can be executed serially by an implementer.
