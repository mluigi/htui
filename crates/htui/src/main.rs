//! The `htui` binary: parse, run, map the outcome to an exit code.

use clap::Parser;
use std::process::ExitCode;

/// Parses the command line and runs the shell.
///
/// The error is printed after [`run`](htui::run) has restored the terminal, so a failure is
/// readable instead of being drawn over the last frame.
#[tokio::main]
async fn main() -> ExitCode {
    let _sentry = sentry::init((
        "https://31abc84ddc174750a380b54b21c45f1c@bugsink.sette.mluigi.it/2",
        sentry::ClientOptions {
            release: sentry::release_name!(),
            attach_stacktrace: true,
            in_app_include: vec!["htui", "htui_core", "htui_store", "htui_agent", "htui_orch"],
            ..Default::default()
        },
    ));

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
