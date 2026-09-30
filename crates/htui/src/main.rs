//! The `htui` binary: parse, run, map the outcome to an exit code.

use clap::Parser;
use std::process::ExitCode;

/// Parses the command line and runs the shell, or `htui worker`.
///
/// Exit codes: 0 on success; 2 for `htui worker`'s startup refusals (and clap's usage errors);
/// 1 for every other failure (MOD-41 plan D14). The error is printed after [`run`](htui::run) has restored the terminal, so a failure is
/// readable instead of being drawn over the last frame.
#[tokio::main]
async fn main() -> ExitCode {
    let mut options = sentry::ClientOptions::default();
    options.dsn = "https://47539c499d6747008e7561dbbe1129cd@glitchtip.sette.mluigi.it/1"
        .parse()
        .ok();
    options.release = sentry::release_name!();
    options.attach_stacktrace = true;
    options.in_app_include = vec![
        "htui",
        "htui_core",
        "htui_store",
        "htui_agent",
        "htui_orch",
        "htui_worker",
    ];

    let _sentry = sentry::init(options);

    let args = htui::cli::Args::parse();
    match htui::run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // MOD-41 plan D14: `htui worker` exits 2 on a startup refusal, 1 on a failure; every
            // other error is 1 as before. Never `process::exit`: the `_sentry` guard must flush.
            let code = error
                .downcast_ref::<htui::worker_cmd::WorkerExit>()
                .map_or(1, htui::worker_cmd::WorkerExit::code);
            // MOD-41 E-1: a startup refusal is a configuration state the user reads on stderr
            // and in the log, not a crash report.
            if code != 2 {
                sentry_anyhow::capture_anyhow(&error);
            }
            eprintln!("htui: {error:#}");
            ExitCode::from(code)
        }
    }
}
