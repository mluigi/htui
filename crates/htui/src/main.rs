//! The `htui` binary: parse, run, map the outcome to an exit code.

use clap::Parser;
use std::process::ExitCode;

/// Parses the command line and runs the shell.
///
/// The error is printed after [`run`](htui::run) has restored the terminal, so a failure is
/// readable instead of being drawn over the last frame.
#[tokio::main]
async fn main() -> ExitCode {
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
