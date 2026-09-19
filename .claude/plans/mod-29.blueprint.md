# Blueprint: MOD-29 - Bugsink integration via Sentry crate

## Architecture
- Dependencies: `sentry` and `sentry-anyhow` added to workspace and `htui` crate.
- Initialization: In `crates/htui/src/lib.rs::run`, `sentry::init` is called with the Bugsink DSN right after `init_tracing`. The return value (a `ClientInitGuard`) must be bound to a variable (e.g. `let _sentry = sentry::init(...)`) so it is not dropped prematurely.

## Implementation Steps
1. **Cargo.toml updates**:
   - `Cargo.toml`: Add `sentry = "0.34"` and `sentry-anyhow = "0.34"` to `[workspace.dependencies]`.
   - `crates/htui/Cargo.toml`: Add `sentry = { workspace = true }` and `sentry-anyhow = { workspace = true }` to `[dependencies]`.

2. **Code changes in `crates/htui/src/lib.rs`**:
   - At the beginning of `pub async fn run(args: cli::Args) -> anyhow::Result<()>`, after `init_tracing`:
     ```rust
     let _sentry = sentry::init((
         "https://31abc84ddc174750a380b54b21c45f1c@bugsink.sette.mluigi.it/2",
         sentry::ClientOptions {
             release: sentry::release_name!(),
             ..Default::default()
         },
     ));
     ```
   - In the same `run` function, if an error is about to be returned, it can optionally be captured via `sentry_anyhow`, but by default Sentry will capture panics anyway. We will rely on panic capture for now unless specific anyhow errors need to be sent. We'll simply integrate `sentry::init`.

## Test Plan
- Run `cargo check` and `cargo test -p htui` to ensure no breakages.
