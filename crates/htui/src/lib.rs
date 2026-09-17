//! The `htui` terminal UI.
//!
//! The crate is a library with a thin binary on top: an integration test can only link a lib
//! target, and T4/T5's snapshot tests live under `tests/` (blueprint A.1).
//!
//! Shape, from the outside in: [`store_worker`] owns the only `Backend`
//! and answers over two unbounded channels; [`app`] holds the state and is the only place a state
//! change happens; [`ui`] draws and emits actions. No view holds a store handle or a channel,
//! which is `R-NF-3` by construction rather than by convention (plan D4).
#![warn(missing_docs)]

pub mod agent_worker;
pub mod app;
pub mod catalogue;
pub mod cli;
pub mod connection;
pub mod event_loop;
pub mod hierarchy;
pub mod keymap;
pub mod preview;
pub mod prompt_settings;
pub mod store_worker;
pub mod terminal;
pub mod ui;

#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

/// How long the shell waits for the store worker to end every live chat before giving up.
///
/// Long enough for a cancel's grace window plus a process-tree kill, short enough that a wedged
/// agent cannot hold the terminal hostage after `q`.
const SHUTDOWN: std::time::Duration = std::time::Duration::from_secs(5);

use std::io::BufRead as _;
use std::path::Path;

use htui_core::store::MemStore;
use htui_store::{Backend, StartOptions, Started, connect, identity, secret};
use tokio::sync::mpsc;

use crate::app::App;
use crate::keymap::Keymap;

/// Runs the whole application, terminal included.
///
/// Startup order (blueprint D.4), and it matters: logging, then the two keyring flags — which exit
/// **before** any terminal work, so the DSN is typed into a normal shell and never into a raw-mode
/// terminal — then the backend, then the worker, then the shell, then the terminal.
///
/// The backend is always available immediately: `connect::start` opens the local mirror and hands
/// back an offline backend, and the connection runs on its own task (ANA-9 §4.4). A missing DSN, an
/// unreachable server and a pending schema are all states the shell renders, not startup failures.
///
/// `main` is [`cli::Args::parse`](clap::Parser::parse), this, and the exit-code mapping.
///
/// # Errors
///
/// Fails when the log file cannot be opened, when the keyring refuses a `--set-dsn` /
/// `--clear-dsn`, when the config directory or the cache file cannot be opened, or when drawing to
/// the terminal fails. The terminal is restored on every path out of here, error included
/// (MOD-1 plan D8).
pub async fn run(args: cli::Args) -> anyhow::Result<()> {
    init_tracing(args.log.as_deref())?;

    if args.set_dsn {
        return set_dsn_from_stdin();
    }
    if args.clear_dsn {
        secret::clear_dsn()?;
        eprintln!("DSN removed from the OS keyring.");
        return Ok(());
    }

    let started = if args.demo {
        Started::detached(Backend::memory(MemStore::demo()))
    } else {
        connect::start(StartOptions {
            offline: args.offline,
            ..StartOptions::new(identity::config_root()?)
        })
        .await?
    };
    let label = started.backend.label();

    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let (reply_tx, reply_rx) = mpsc::unbounded_channel();
    // The backend moves into the worker here and is unreachable from the UI afterwards (D4).
    let worker = store_worker::spawn(started, request_rx, reply_tx);

    let mut app = App::new(request_tx, Keymap::default_global());
    app.top_bar.store = label;
    app::register_all(&mut app);
    app.start();

    let mut term = terminal::init();
    let outcome = event_loop::run(&mut term, &mut app, reply_rx).await;
    term.restore();

    // Quit order, and it matters (MOD-2 milestone 3). Dropping the shell closes the request
    // channel; the worker sees that, cancels every live chat and waits for each session task to
    // kill its process tree, and only then returns. Aborting it here instead — which is what this
    // did before there were sessions — drops every task at its first await and orphans the agent
    // processes they spawned (`docs/ANA-4.md` §11 criterion 11). The abort stays as the backstop
    // for a worker that will not stop.
    drop(app);
    if tokio::time::timeout(SHUTDOWN, worker).await.is_err() {
        tracing::warn!("the store worker did not stop within the shutdown window");
    }

    outcome.map_err(anyhow::Error::from)
}

/// `--set-dsn`: one line from stdin into the OS keyring, then exit (plan D7, `R-STO-1`).
///
/// Stdin rather than an argument so the DSN never reaches `argv`, a shell history or a dotfile,
/// and before the terminal is put into raw mode so the shell's own line editing applies. Echo
/// suppression is not attempted — that would be another dependency — so the prompt says as much.
/// Everything printed goes to **stderr**: stdout belongs to the TUI, and a maintainer piping a DSN
/// in should still see the confirmation.
fn set_dsn_from_stdin() -> anyhow::Result<()> {
    eprintln!("paste the DSN and press Enter (it will be visible):");
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let dsn = line.trim();
    anyhow::ensure!(!dsn.is_empty(), "no DSN on stdin; nothing was stored");
    secret::set_dsn(dsn)?;
    eprintln!(
        "DSN stored in the OS keyring ({}/{}).",
        secret::SERVICE,
        secret::USER
    );
    Ok(())
}

/// Sends `tracing` to a file, or nowhere at all.
///
/// Never to stdout: stdout is the TUI (plan Patterns/Logging). `HTUI_LOG_FILTER` overrides the
/// default `info` level.
fn init_tracing(path: Option<&Path>) -> anyhow::Result<()> {
    let Some(path) = path else { return Ok(()) };
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let filter = tracing_subscriber::EnvFilter::try_from_env("HTUI_LOG_FILTER")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_writer(file)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init()
        .map_err(|err| anyhow::anyhow!("could not install the log subscriber: {err}"))
}
