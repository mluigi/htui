//! The `htui` binary: parse, run, map the outcome to an exit code.

use clap::Parser;
use std::panic::AssertUnwindSafe;
use std::process::ExitCode;
use std::time::Duration;

/// Parses the command line and runs the shell, or `htui worker`.
///
/// Exit codes: 0 on success; 2 for `htui worker`'s startup refusals, `htui provision`'s
/// refusals (MOD-45 D292), `htui mcp` without its environment, a key file htui refuses (MOD-67)
/// and clap's usage errors; 3 for an `htui mcp` the host refused (MOD-11 D6); 1 for every other
/// failure (MOD-41 plan D14). The error is printed after [`run`](htui::run) has restored the
/// terminal, so a failure is readable instead of being drawn over the last frame.
///
/// MOD-41 review R-4: the runtime is built by hand, not by `#[tokio::main]`, so its teardown is
/// bounded by [`htui::SHUTDOWN`], whether the body returns or panics: a blocking task that never
/// ends cannot hold the exit.
fn main() -> ExitCode {
    let args = htui::cli::Args::parse();
    let grace = teardown_grace(&args);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("the tokio runtime builds");
    run_bounded(runtime, grace, body(args))
}

/// How long the runtime's teardown may wait for blocking tasks once `body` has returned.
///
/// [`htui::SHUTDOWN`], except for `htui mcp` (MOD-11 D6), which gets none: its stdin is read by a
/// blocking read on fd 0 that nothing can cancel, so when the host ends the session first the
/// teardown would wait out the whole grace with the agent's stdout still open. Nothing of the
/// relay is left to finish by then: it returns only after flushing stdout.
fn teardown_grace(args: &htui::cli::Args) -> Duration {
    if matches!(args.command, Some(htui::cli::Command::Mcp)) {
        Duration::ZERO
    } else {
        htui::SHUTDOWN
    }
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
async fn body(args: htui::cli::Args) -> ExitCode {
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
        "htui_mcp",
    ];

    let _sentry = sentry::init(options);

    match htui::run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // MOD-41 plan D14: `htui worker` exits 2 on a startup refusal, 1 on a failure, and
            // `htui provision` (MOD-45 D292) likewise; every other error is 1 as before. Never
            // `process::exit`: the `_sentry` guard must flush.
            let code = exit_code(&error);
            if reports_to_sentry(&error, code) {
                sentry_anyhow::capture_anyhow(&error);
            }
            eprintln!("htui: {error:#}");
            ExitCode::from(code)
        }
    }
}

/// The exit code `error` ends `body` with: a [`WorkerExit`](htui::worker_cmd::WorkerExit)'s, a
/// [`ProvisionExit`](htui::provision::ProvisionExit)'s or an [`McpExit`](htui::mcp_cmd::McpExit)'s
/// own code (MOD-11 B-13: 2, 3 or 1), a [`KeysError`](htui::keys::KeysError)'s (MOD-67 M2 D9:
/// 2), else 1.
fn exit_code(error: &anyhow::Error) -> u8 {
    error
        .downcast_ref::<htui::worker_cmd::WorkerExit>()
        .map(htui::worker_cmd::WorkerExit::code)
        .or_else(|| {
            error
                .downcast_ref::<htui::provision::ProvisionExit>()
                .map(htui::provision::ProvisionExit::code)
        })
        .or_else(|| {
            error
                .downcast_ref::<htui::mcp_cmd::McpExit>()
                .map(htui::mcp_cmd::McpExit::code)
        })
        .or_else(|| {
            error
                .downcast_ref::<htui::keys::KeysError>()
                .map(htui::keys::KeysError::code)
        })
        .unwrap_or(1)
}

/// Whether `body` sends `error`, ending in exit `code`, to Sentry.
///
/// MOD-41 E-1: a startup refusal (2) is a configuration state the user reads on stderr and in the
/// log, not a crash report. MOD-45 review finding 1: no `htui provision` end is either, whatever
/// its code: its sentences carry `user@host`, remote paths and the host's journal and log lines,
/// and each describes that host's state. MOD-11 B-13: no `htui mcp` end is either; an agent's
/// relay ending is the session's state, not a crash. MOD-67 M2 D9: no key-file refusal either,
/// whatever its code: the report carries the user's home path.
fn reports_to_sentry(error: &anyhow::Error, code: u8) -> bool {
    code != 2
        && error
            .downcast_ref::<htui::provision::ProvisionExit>()
            .is_none()
        && error.downcast_ref::<htui::mcp_cmd::McpExit>().is_none()
        && error.downcast_ref::<htui::keys::KeysError>().is_none()
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::{reports_to_sentry, run_bounded, teardown_grace};
    use htui::keys::{KeyFileError, KeysError};
    use htui::mcp_cmd::McpExit;
    use htui::provision::ProvisionExit;
    use htui::worker_cmd::WorkerExit;

    /// MOD-45 review finding 1: no `htui provision` end reaches Sentry, whatever its code; a
    /// worker refusal does not either, and every other failure still does.
    #[test]
    fn only_crashes_are_reported_to_sentry() {
        let failed = anyhow::Error::from(ProvisionExit::Failed("provisioning h failed".into()));
        assert!(!reports_to_sentry(&failed, 1), "a provision failure");
        let refused = anyhow::Error::from(ProvisionExit::Refused("not provisioning h".into()));
        assert!(!reports_to_sentry(&refused, 2), "a provision refusal");
        let wrapped = failed.context("while provisioning");
        assert!(
            !reports_to_sentry(&wrapped, 1),
            "a wrapped provision failure"
        );

        let refused = anyhow::Error::from(WorkerExit::Refused("no DSN".into()));
        assert!(!reports_to_sentry(&refused, 2), "a worker refusal");
        let failed = anyhow::Error::from(WorkerExit::Failed("lost".into()));
        assert!(reports_to_sentry(&failed, 1), "a worker failure");
        assert!(
            reports_to_sentry(&anyhow::anyhow!("boom"), 1),
            "any other error"
        );

        // MOD-11 B-13: an agent's relay ending is never a crash report, whatever its code.
        let missing = anyhow::Error::from(McpExit::MissingEnv("HTUI_MCP_ADDR is not set".into()));
        assert!(!reports_to_sentry(&missing, 2), "a relay missing its env");
        let refused = anyhow::Error::from(McpExit::Refused("unknown session".into()));
        assert!(!reports_to_sentry(&refused, 3), "a refused relay");
        let failed = anyhow::Error::from(McpExit::Failed("cannot reach the host".into()));
        assert!(!reports_to_sentry(&failed, 1), "a failed relay");
    }

    /// MOD-67 M2 D9: a key file htui refuses exits 2 and never reaches Sentry, by its type and
    /// not only by its code (the report carries the user's home path), wrapped or not.
    #[test]
    fn a_key_file_refusal_exits_2_and_never_reaches_sentry() {
        let refusals = || {
            [
                KeysError::Invalid {
                    path: "/home/u/.config/htui/keys.toml".into(),
                    errors: vec![KeyFileError {
                        line: 3,
                        message: "[global] quit = \"ctrl-c\": ctrl-c always quits".to_owned(),
                    }],
                },
                KeysError::Unreadable {
                    path: "/home/u/k.toml".into(),
                    source: std::io::ErrorKind::NotFound.into(),
                },
            ]
        };
        for refusal in refusals() {
            let error = anyhow::Error::from(refusal);
            assert_eq!(super::exit_code(&error), 2, "{error}");
            assert!(!reports_to_sentry(&error, 2), "{error}");
            assert!(!reports_to_sentry(&error, 1), "refused by type: {error}");
            let wrapped = error.context("while starting");
            assert_eq!(super::exit_code(&wrapped), 2, "{wrapped:#}");
            assert!(!reports_to_sentry(&wrapped, 1), "{wrapped:#}");
        }
    }

    /// MOD-11 D6, B-13: `htui mcp` exits 2 without its environment, 3 when the host refuses it, 1
    /// on any other failure; `body` reads the code off the error the same way.
    #[test]
    fn mcp_exit_codes_are_2_3_1() {
        assert_eq!(McpExit::MissingEnv(String::new()).code(), 2);
        assert_eq!(McpExit::Refused(String::new()).code(), 3);
        assert_eq!(McpExit::Failed(String::new()).code(), 1);
        for exit in [
            McpExit::MissingEnv("m".into()),
            McpExit::Refused("r".into()),
            McpExit::Failed("f".into()),
        ] {
            let expected = exit.code();
            let error = anyhow::Error::from(exit);
            assert_eq!(super::exit_code(&error), expected, "{error}");
        }
        assert_eq!(super::exit_code(&anyhow::anyhow!("boom")), 1);
    }

    /// MOD-11 D6: `htui mcp` tears its runtime down at once (its stdin read cannot be
    /// cancelled); every other command keeps the bounded grace.
    #[test]
    fn only_mcp_skips_the_teardown_grace() {
        use clap::Parser;
        let grace = |argv: &[&str]| {
            teardown_grace(&htui::cli::Args::try_parse_from(argv).expect("the arguments parse"))
        };
        assert_eq!(grace(&["htui", "mcp"]), Duration::ZERO);
        assert_eq!(grace(&["htui"]), htui::SHUTDOWN);
        assert_eq!(grace(&["htui", "--clear-dsn"]), htui::SHUTDOWN);
    }

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
