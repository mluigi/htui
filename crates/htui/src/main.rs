//! The `htui` binary: parse, run, map the outcome to an exit code.

use clap::Parser;
use std::process::ExitCode;

/// Builds the runtime, runs the shell on it, and bounds the runtime's own shutdown.
///
/// By hand rather than `#[tokio::main]` (MOD-64 review 1): that runtime's drop waits without a
/// bound for every blocking task, and a first embedding-model download runs in an uncancellable
/// `spawn_blocking(FastEmbedder::new)`, so `q` would hang until the download ended. The builder
/// calls are the ones the attribute expands to; only the shutdown differs.
fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed building the Runtime");
    let code = runtime.block_on(shell());
    runtime.shutdown_timeout(htui::SHUTDOWN);
    code
}

/// Parses the command line and runs the shell.
///
/// The error is printed after [`run`](htui::run) has restored the terminal, so a failure is
/// readable instead of being drawn over the last frame.
async fn shell() -> ExitCode {
    let mut options = sentry::ClientOptions::default();
    options.dsn = "https://47539c499d6747008e7561dbbe1129cd@glitchtip.sette.mluigi.it/1"
        .parse()
        .ok();
    options.release = sentry::release_name!();
    options.attach_stacktrace = true;
    options.in_app_include = vec!["htui", "htui_core", "htui_store", "htui_agent", "htui_orch"];

    let _sentry = sentry::init(options);

    let args = htui::cli::Args::parse();
    match htui::run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            sentry_anyhow::capture_anyhow(&error);
            eprintln!("htui: {error:#}");
            ExitCode::FAILURE
        }
    }
}
