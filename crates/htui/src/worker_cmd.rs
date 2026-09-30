//! `htui worker` (MOD-41 plan D14, D15): the headless connect, then `htui_worker::worker::run`.

use std::future::Future;
use std::path::{Path, PathBuf};

use htui_store::pg::{CONNECT_TIMEOUT, PoolSize};
use htui_store::secret::{self, DsnSources};
use htui_store::{PgStore, connect, identity};
use htui_worker::worker::WorkerConfig;
use htui_worker::{Role, RunRuntime, Unaddressed};
use zeroize::Zeroizing;

use crate::cli::WorkerArgs;
use crate::concepts;

/// How `htui worker` ends (plan D14): `main` maps it to the exit code.
#[derive(Debug)]
pub enum WorkerExit {
    /// Exit 2: nothing ran (no DSN, connect or schema refused, signals not installable).
    Refused(String),
    /// Exit 1: the worker ran and failed.
    Failed(String),
}

impl WorkerExit {
    /// 2 for a refusal, 1 for a failure.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Refused(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl core::fmt::Display for WorkerExit {
    /// The sentence, and nothing else.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(sentence) | Self::Failed(sentence) => f.write_str(sentence),
        }
    }
}

impl std::error::Error for WorkerExit {}

/// `htui worker`: logging, the signal handlers, the DSN, the headless connect, then the loop
/// until SIGINT or SIGTERM (plan D14). Everything before the loop is a startup refusal.
///
/// # Errors
///
/// [`WorkerExit`]; a clean signal shutdown is `Ok`.
pub async fn run(args: WorkerArgs, log: Option<&Path>) -> Result<(), WorkerExit> {
    init_worker_tracing(log).map_err(|err| WorkerExit::Refused(format!("{err:#}")))?;
    let shutdown = shutdown_signal().map_err(|err| {
        WorkerExit::Refused(format!("the signal handlers could not be installed: {err}"))
    })?;
    // Before any setting is read (PRD D7): the pool is the flag's, clamped.
    let pool = PoolSize::clamped(args.pool_size);
    let dsn = read_dsn(args.dsn_stdin)?;
    let root = identity::config_root().map_err(|err| WorkerExit::Refused(err.to_string()))?;
    let pg = connect(&dsn, &root, pool).await?;
    // Wiped here: nothing after the connect needs it.
    drop(dsn);
    let runtime = RunRuntime::<PgStore, Unaddressed>::production().with_role(Role::Worker);
    tracing::info!(box_id = %pg.this_box(), pool = pool.get(), "htui worker ready");
    htui_worker::worker::run(pg, runtime, WorkerConfig::PRODUCTION, shutdown).await;
    tracing::info!("htui worker stopped");
    Ok(())
}

/// PRD D3's sources, in order. The prompt goes to stderr, and only when stdin is a terminal.
fn read_dsn(dsn_stdin: bool) -> Result<Zeroizing<String>, WorkerExit> {
    let credentials = std::env::var_os("CREDENTIALS_DIRECTORY").map(PathBuf::from);
    let stdin = std::io::stdin();
    if dsn_stdin && std::io::IsTerminal::is_terminal(&stdin) {
        eprintln!("paste the DSN and press Enter (it will be visible):");
    }
    let mut lock = stdin.lock();
    secret::headless_dsn(DsnSources {
        stdin: dsn_stdin.then_some(&mut lock as &mut dyn std::io::BufRead),
        credentials_dir: credentials.as_deref(),
    })
    .map_err(|err| WorkerExit::Refused(err.to_string()))
}

/// The headless connect and the registration write-back (plan D14): every refusal before any
/// write, with `concepts::headless_refusal`'s sentences. `pub` for `worker_pg.rs` case 4.
///
/// # Errors
///
/// [`WorkerExit::Refused`].
pub async fn connect(dsn: &str, root: &Path, pool: PoolSize) -> Result<PgStore, WorkerExit> {
    let presented =
        identity::load_or_mint(root).map_err(|err| WorkerExit::Refused(err.to_string()))?;
    let pg = PgStore::connect_headless(dsn, &presented, CONNECT_TIMEOUT, pool)
        .await
        .map_err(|err| WorkerExit::Refused(format!("{:#}", concepts::headless_refusal(err))))?;
    connect::persist_registration(root, &presented, &pg)
        .map_err(|err| WorkerExit::Refused(err.to_string()))?;
    Ok(pg)
}

/// `--log PATH` as the TUI writes it (appended, no ANSI); without it, stderr, with ANSI only on a
/// terminal. Both add `sentry_tracing::layer()`; `HTUI_LOG_FILTER` overrides the default `info`
/// level, as for the TUI.
fn init_worker_tracing(log: Option<&Path>) -> anyhow::Result<()> {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = tracing_subscriber::EnvFilter::try_from_env("HTUI_LOG_FILTER")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let fmt_layer = if let Some(path) = log {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|err| anyhow::anyhow!("cannot open the log {}: {err}", path.display()))?;
        tracing_subscriber::fmt::layer()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            .boxed()
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(sentry_tracing::layer())
        .try_init()
        .map_err(|err| anyhow::anyhow!("could not install the log subscriber: {err}"))
}

/// SIGINT or SIGTERM on unix; Ctrl-C, Ctrl-Break, Ctrl-Close and Ctrl-Shutdown on Windows
/// (uncompiled here, R-8). Installed before connecting, so a refusal to install is a startup
/// refusal, and a signal that arrives during the connect is not lost.
fn shutdown_signal() -> std::io::Result<impl Future<Output = ()> + Send + 'static> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate())?;
        let mut int = signal(SignalKind::interrupt())?;
        Ok(async move {
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
        })
    }
    #[cfg(windows)]
    {
        use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close, ctrl_shutdown};
        let (mut c, mut b, mut cl, mut sd) =
            (ctrl_c()?, ctrl_break()?, ctrl_close()?, ctrl_shutdown()?);
        Ok(async move {
            tokio::select! {
                _ = c.recv() => {}
                _ = b.recv() => {}
                _ = cl.recv() => {}
                _ = sd.recv() => {}
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerExit;

    /// Plan D14: a startup refusal exits 2, a failure after start exits 1.
    #[test]
    fn worker_exit_codes() {
        assert_eq!(WorkerExit::Refused("no DSN".to_owned()).code(), 2);
        assert_eq!(WorkerExit::Failed("lost".to_owned()).code(), 1);
    }
}
