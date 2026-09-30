//! The `htui` binary: parse, run, map the outcome to an exit code.

use clap::Parser;
use std::panic::AssertUnwindSafe;
use std::process::ExitCode;
use std::time::Duration;

/// Parses the command line and runs the shell, or `htui worker`.
///
/// Exit codes: 0 on success; 2 for `htui worker`'s startup refusals (and clap's usage errors);
/// 1 for every other failure (MOD-41 plan D14). The error is printed after [`run`](htui::run) has restored the terminal, so a failure is
/// readable instead of being drawn over the last frame.
///
/// MOD-41 review R-4: the runtime is built by hand, not by `#[tokio::main]`, so its teardown is
/// bounded by [`htui::SHUTDOWN`], whether the body returns or panics: a blocking task that never
/// ends cannot hold the exit.
fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("the tokio runtime builds");
    run_bounded(runtime, htui::SHUTDOWN, body())
}

/// Runs `future` to its end on `runtime`, then tears the runtime down within `grace`.
///
/// MOD-41 review RF-4: a panic out of `future` is caught, the runtime is torn down within
/// `grace`, and the panic resumes: unwinding past a live runtime would drop it, and its drop
/// waits for every blocking task with no bound.
fn run_bounded<T>(
    runtime: tokio::runtime::Runtime,
    grace: Duration,
    future: impl Future<Output = T>,
) -> T {
    let out = std::panic::catch_unwind(AssertUnwindSafe(|| runtime.block_on(future)));
    runtime.shutdown_timeout(grace);
    match out {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// `main` inside the runtime. The Sentry guard lives here, so it flushes before the runtime's
/// teardown begins.
async fn body() -> ExitCode {
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

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::run_bounded;

    /// MOD-41 review RF-4: a body that panics while a blocking task never ends still leaves
    /// within the grace, so the panic reaches its exit (101) and a unit's `Restart=on-failure`.
    #[test]
    fn a_panicking_body_still_bounds_the_teardown() {
        let (release, blocked) = mpsc::channel::<()>();
        let (done, finished) = mpsc::channel();
        let runner = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("the tokio runtime builds");
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                run_bounded(runtime, Duration::from_millis(100), async move {
                    let (started, running) = tokio::sync::oneshot::channel();
                    tokio::task::spawn_blocking(move || {
                        let _ = started.send(());
                        // Held until the case releases it: a blocking task that does not end.
                        let _ = blocked.recv();
                    });
                    running.await.expect("the blocking task starts");
                    panic!("the scripted body panics");
                })
            }));
            let _ = done.send(outcome.is_err());
        });

        let panicked = finished.recv_timeout(Duration::from_secs(10));
        drop(release);
        runner.join().expect("the runner thread ends");
        assert_eq!(
            panicked,
            Ok(true),
            "the panic leaves within the grace instead of waiting on the blocking task"
        );
    }
}
