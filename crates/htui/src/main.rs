//! The `htui` binary: parse, run, map the outcome to an exit code.

use clap::Parser;
use std::process::ExitCode;

/// Parses the command line and runs the shell.
///
/// The error is printed after [`run`](htui::run) has restored the terminal, so a failure is
/// readable instead of being drawn over the last frame.
#[tokio::main]
async fn main() -> ExitCode {
    let args = htui::cli::Args::parse();
    match htui::run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("htui: {error:#}");
            ExitCode::FAILURE
        }
    }
}
