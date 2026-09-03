//! The `htui` terminal UI.
//!
//! The crate is a library with a thin binary on top: an integration test can only link a lib
//! target, and T4/T5's snapshot tests live under `tests/` (blueprint A.1).
//!
//! Shape, from the outside in: [`store_worker`] owns the only [`Backend`]
//! and answers over two unbounded channels; [`app`] holds the state and is the only place a state
//! change happens; [`ui`] draws and emits actions. No view holds a store handle or a channel,
//! which is `R-NF-3` by construction rather than by convention (plan D4).
#![warn(missing_docs)]

pub mod app;
pub mod cli;
pub mod event_loop;
pub mod keymap;
pub mod store_worker;
pub mod terminal;
pub mod ui;

#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

use std::path::Path;

use htui_core::store::{Backend, MemStore};
use tokio::sync::mpsc;

use crate::app::App;
use crate::keymap::Keymap;

/// Runs the whole application, terminal included.
///
/// `main` is [`cli::Args::parse`](clap::Parser::parse), this, and the exit-code mapping.
///
/// # Errors
///
/// Fails when the log file cannot be opened or when drawing to the terminal fails. The terminal
/// is restored on every path out of here, error included (plan D8).
pub async fn run(args: cli::Args) -> anyhow::Result<()> {
    init_tracing(args.log.as_deref())?;

    let store = if args.demo {
        MemStore::demo()
    } else {
        MemStore::new()
    };
    let backend = Backend::memory(store);
    let label = backend.label();

    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let (reply_tx, reply_rx) = mpsc::unbounded_channel();
    // The backend moves into the worker here and is unreachable from the UI afterwards (D4).
    let worker = store_worker::spawn(backend, request_rx, reply_tx);

    let mut app = App::new(request_tx, Keymap::default_global());
    app.top_bar.store = label;
    app::register_all(&mut app);
    app.start();

    let mut term = terminal::init();
    let outcome = event_loop::run(&mut term, &mut app, reply_rx).await;
    term.restore();
    worker.abort();

    outcome.map_err(anyhow::Error::from)
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
