//! `htui worker` (MOD-41 plan D14, D15): the headless connect, then `htui_worker::worker::run`.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use htui_core::qdrant::QdrantUrl;
use htui_store::pg::{CONNECT_TIMEOUT, PoolSize};
use htui_store::secret::{self, DsnSources};
use htui_store::{PgStore, connect, identity};
use htui_worker::worker::WorkerConfig;
use htui_worker::{Role, RunRuntime, Unaddressed};
use zeroize::Zeroizing;

use crate::cli::WorkerArgs;
use crate::concepts;
use crate::concepts_worker::{self, KeySource, QdrantCheck, QdrantSource};

/// MOD-158 D4: `--qdrant-key-stdin` was given and its line was blank, or missing. Not "nothing was
/// read": with `--dsn-stdin` the DSN line before it was (MOD-158 R1, B-L5).
const NO_KEY_ON_STDIN: &str = "no Qdrant API key on stdin: the key line was blank or missing";

/// The DSN and, with `--qdrant-key-stdin`, the key line, both wiped when dropped.
type StartupSecrets = (Zeroizing<String>, Option<Zeroizing<String>>);

/// How `htui worker` ends (plan D14): `main` maps it to the exit code.
#[derive(Debug)]
pub enum WorkerExit {
    /// Exit 2: nothing ran (no DSN, connect or schema refused, signals not installable).
    Refused(String),
    /// Exit 1: reserved for a failure after the worker started. This build produces none: a store
    /// outage is logged and waited out, not an exit (MOD-41 review R-3). A panic exits 101.
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
/// A refusal is logged too, at `warn` (MOD-41 E-1): `main` prints it on stderr, and an `error`
/// line would become a GlitchTip report, which a refusal must not.
///
/// # Errors
///
/// [`WorkerExit`]; a clean signal shutdown is `Ok`.
pub async fn run(args: WorkerArgs, log: Option<&Path>) -> Result<(), WorkerExit> {
    init_worker_tracing(log).map_err(|err| WorkerExit::Refused(format!("{err:#}")))?;
    let ended = serve(args).await;
    if let Err(WorkerExit::Refused(sentence)) = &ended {
        tracing::warn!("{sentence}");
    }
    ended
}

/// [`run`] once logging is up. The signal handlers go in first, and the startup (the DSN and key
/// reads, the keyring, the connect, the old keyring URL, the first read of the URL row) races them:
/// installing a handler replaces the default termination, so a signal the startup did not watch
/// would be swallowed until the loop began. A signal during the startup is a clean exit 0, with
/// nothing written after it.
async fn serve(args: WorkerArgs) -> Result<(), WorkerExit> {
    let shutdown = shutdown_signal().map_err(|err| {
        WorkerExit::Refused(format!("the signal handlers could not be installed: {err}"))
    })?;
    let mut shutdown = std::pin::pin!(shutdown);
    // Before any setting is read (PRD D7): the pool is the flag's, clamped.
    let pool = PoolSize::clamped(args.pool_size);
    let (dsn, key_line) = tokio::select! {
        biased;
        () = &mut shutdown => return stopped_before_ready(),
        secrets = read_dsn_apart(args.dsn_stdin, args.qdrant_key_stdin) => secrets?,
    };
    let root = identity::config_root().map_err(|err| WorkerExit::Refused(err.to_string()))?;
    let pg = tokio::select! {
        biased;
        () = &mut shutdown => return stopped_before_ready(),
        pg = connect(&dsn, &root, pool) => pg?,
    };
    // Wiped here: nothing after the connect needs it.
    drop(dsn);
    // MOD-158 D4, D6: the process-wide index (the one `search_concepts` and the index job share)
    // reads the `qdrant_url` row through this pool and the key through the headless sources.
    let qdrant = concepts_worker::shared_qdrant();
    qdrant.source().set_key_source(KeySource::Headless {
        stdin: key_line,
        credentials_dir: credentials_dir(),
    });
    qdrant.source().publish(Some(Arc::new(pg.clone())));
    // MOD-158 D10: an old per-box keyring URL is said once; nothing reads it for anything else.
    tokio::select! {
        biased;
        () = &mut shutdown => return stopped_before_ready(),
        old = concepts::apart("htui-worker-old-url", secret::get_qdrant_url) => {
            log_old_keyring_url(old);
        }
    }
    // MOD-160 D5: one awaited read of the URL row, so the first session knows whether
    // `search_concepts` is offered; the source caches it, and later reads refresh it.
    tokio::select! {
        biased;
        () = &mut shutdown => return stopped_before_ready(),
        _ = qdrant.source().url() => {}
    }
    // MOD-158 D7: the job starts whatever the setting says and resolves it every cycle; it shares
    // the pool, never the runtime.
    let index_job = concepts::spawn_index_job(pg.clone());
    // MOD-160 D9: one model-free look at Qdrant, said once; never awaited, so a Qdrant that does
    // not answer never delays "htui worker ready".
    let start_check = tokio::spawn(start_check(Arc::clone(qdrant.source())));
    // MOD-11 D11: a worker that cannot host tools would fail every document phase, so it is
    // refused before it claims anything.
    let tools = htui_mcp::McpHost::new(pg.clone())
        .map_err(|err| WorkerExit::Refused(format!("htui's MCP tools cannot be hosted: {err}")))?;
    // MOD-160 D5: attached always, as in the TUI; the cached offer decides per session, so a URL
    // set after start is offered without a restart (supersedes MOD-158 D7, Q3).
    let tools = match crate::mcp_search::production() {
        Some(search) => tools.with_search(search),
        None => tools,
    };
    let runtime = RunRuntime::<PgStore, Unaddressed>::production()
        .with_role(Role::Worker)
        .with_tool_host(Arc::new(tools))
        // MOD-10 D15: this process's one secret source; the worker hosts no chats.
        .with_secret_source(Arc::new(crate::secrets::KeyringInfisical::new()));
    tracing::info!(box_id = %pg.this_box(), pool = pool.get(), "htui worker ready");
    htui_worker::worker::run(pg, runtime, WorkerConfig::PRODUCTION, shutdown).await;
    index_job.abort();
    start_check.abort();
    tracing::info!("htui worker stopped");
    Ok(())
}

/// MOD-160 D9: the worker's start check of `source`'s Qdrant, logged once by [`start_line`].
async fn start_check(source: Arc<QdrantSource>) {
    let check = concepts_worker::check_qdrant(&source, htui_store::vector::COLLECTION).await;
    match start_line(check.as_ref()) {
        Some((tracing::Level::WARN, line)) => tracing::warn!("{line}"),
        Some((_, line)) => tracing::info!("{line}"),
        None => {}
    }
}

/// MOD-160 D9: the start line for `check`. A warning state is a `warn` line,
/// `knowledge search unavailable: <state>: <cause>`; a Qdrant that answers without a key is an
/// `info` line; anything else says nothing. Never an `error`: an `error` line becomes a GlitchTip
/// report (`docs/htui-worker.md`), and the worker starts either way.
fn start_line(check: Option<&QdrantCheck>) -> Option<(tracing::Level, String)> {
    let check = check?;
    if let Some(warning) = check.warning() {
        return Some((
            tracing::Level::WARN,
            htui_core::qdrant::unavailable(&warning),
        ));
    }
    check
        .info()
        .map(|info| (tracing::Level::INFO, info.to_owned()))
}

/// MOD-158 D10: the keyring's old `qdrant-url`, when there is one, is said once at `info`. A URL
/// that is not a usable one is not printed: it may hold a password. A keyring that cannot be read
/// (a service host has none) is no entry.
fn log_old_keyring_url(old: Option<htui_core::store::Result<Option<String>>>) {
    let Some(Ok(Some(old))) = old else {
        return;
    };
    let shown = QdrantUrl::parse(&old).map_or_else(
        |_| "a value that is not a usable URL".to_owned(),
        |url| url.to_string(),
    );
    tracing::info!(
        "this box's keyring holds an old Qdrant URL ({shown}) that htui no longer reads; use it \
         for every box in Settings > Qdrant or with `htui --set-qdrant-url`"
    );
}

/// `$CREDENTIALS_DIRECTORY`: the directory systemd decrypts the unit's credentials into, never a
/// secret.
fn credentials_dir() -> Option<PathBuf> {
    std::env::var_os("CREDENTIALS_DIRECTORY").map(PathBuf::from)
}

/// A signal won the race against the startup: a clean exit.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the startup's `return`, beside its `?`"
)]
fn stopped_before_ready() -> Result<(), WorkerExit> {
    tracing::info!("htui worker stopped before it was ready");
    Ok(())
}

/// [`read_startup_secrets`] on a thread of its own, so the startup can race it against a signal.
/// Not `spawn_blocking`: the runtime waits for its blocking tasks when it is dropped, and a read
/// of a stdin nobody closes never ends. The thread is left behind on a signal and dies with the
/// process. The key line (MOD-158 D4) is read on the same thread, after the DSN.
async fn read_dsn_apart(dsn_stdin: bool, key_stdin: bool) -> Result<StartupSecrets, WorkerExit> {
    let (answer, answered) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("htui-worker-dsn".to_owned())
        .spawn(move || drop(answer.send(read_startup_secrets(dsn_stdin, key_stdin))))
        .map_err(|err| WorkerExit::Refused(format!("the DSN could not be read: {err}")))?;
    answered.await.unwrap_or_else(|_| {
        Err(WorkerExit::Refused(
            "the DSN could not be read: its reader stopped".to_owned(),
        ))
    })
}

/// [`read_startup_secrets_from`] over this process's stdin. A prompt goes to stderr, and only when
/// stdin is a terminal.
fn read_startup_secrets(dsn_stdin: bool, key_stdin: bool) -> Result<StartupSecrets, WorkerExit> {
    let credentials = credentials_dir();
    let stdin = std::io::stdin();
    let terminal = std::io::IsTerminal::is_terminal(&stdin);
    let mut lock = stdin.lock();
    read_startup_secrets_from(
        &mut lock,
        dsn_stdin,
        key_stdin,
        credentials.as_deref(),
        &mut |what| {
            if terminal {
                eprintln!("paste the {what} and press Enter (it will be visible):");
            }
        },
    )
}

/// PRD D3's DSN sources, in order, then (MOD-158 D4) with `key_stdin` the next line of `input` as
/// the Qdrant API key: a blank line or end of input is a refusal, never a fall-through. `prompt`
/// is told what is about to be read from `input`.
fn read_startup_secrets_from(
    input: &mut dyn std::io::BufRead,
    dsn_stdin: bool,
    key_stdin: bool,
    credentials_dir: Option<&Path>,
    prompt: &mut dyn FnMut(&str),
) -> Result<StartupSecrets, WorkerExit> {
    if dsn_stdin {
        prompt("DSN");
    }
    let dsn = secret::headless_dsn(DsnSources {
        stdin: dsn_stdin.then_some(&mut *input),
        credentials_dir,
    })
    .map_err(|err| WorkerExit::Refused(err.to_string()))?;
    if !key_stdin {
        return Ok((dsn, None));
    }
    prompt("Qdrant API key");
    // The DSN's reader: one line, trimmed, in a buffer wiped when dropped.
    let key = secret::read_dsn_line(input)
        .map_err(|err| WorkerExit::Refused(format!("stdin could not be read: {err}")))?
        .ok_or_else(|| WorkerExit::Refused(NO_KEY_ON_STDIN.to_owned()))?;
    Ok((dsn, Some(key)))
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
/// (uncompiled here, R-8). Installed before the DSN read, so a refusal to install is a startup
/// refusal, and `serve` races the startup against it.
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
    use std::io::Cursor;

    use htui_store::secret;
    use tracing::Level;

    use super::{WorkerExit, read_startup_secrets_from, start_line};
    use crate::concepts_worker::QdrantCheck;

    /// Plan D14: a startup refusal exits 2, a failure after start exits 1.
    #[test]
    fn worker_exit_codes() {
        assert_eq!(WorkerExit::Refused("no DSN".to_owned()).code(), 2);
        assert_eq!(WorkerExit::Failed("lost".to_owned()).code(), 1);
    }

    /// MOD-160 D9: the start line warns on each warning state and informs on a keyless Qdrant,
    /// with the sentence first; it is never an `error`, which would become a GlitchTip report.
    #[test]
    fn the_start_line_warns_or_informs_and_never_errors() {
        for check in [
            QdrantCheck::NotConfigured("no Qdrant URL is set".to_owned()),
            QdrantCheck::Unavailable("cannot reach Qdrant".to_owned()),
            QdrantCheck::Unreadable("the keyring is locked".to_owned()),
            QdrantCheck::KeyRefused("qdrant: access refused".to_owned()),
            QdrantCheck::NoKey("qdrant: access refused".to_owned()),
        ] {
            let (level, line) = start_line(Some(&check)).expect("a warning");
            assert_eq!(level, Level::WARN, "{check:?}");
            let warning = check.warning().expect("a warning state");
            assert_eq!(line, format!("knowledge search unavailable: {warning}"));
        }
        assert_eq!(
            start_line(Some(&QdrantCheck::Keyless)),
            Some((
                Level::INFO,
                "this Qdrant answers without an API key".to_owned()
            ))
        );
        assert_eq!(start_line(Some(&QdrantCheck::Ready)), None);
        assert_eq!(start_line(None), None, "nothing probed: offline");
    }

    /// The secrets `--dsn-stdin` and `--qdrant-key-stdin` read, in that order, with no prompt.
    fn read(
        input: &str,
        dsn_stdin: bool,
        key_stdin: bool,
    ) -> Result<(String, Option<String>), WorkerExit> {
        let mut input = Cursor::new(input.as_bytes().to_vec());
        read_startup_secrets_from(&mut input, dsn_stdin, key_stdin, None, &mut |_| {}).map(
            |(dsn, key)| {
                (
                    dsn.as_str().to_owned(),
                    key.map(|key| key.as_str().to_owned()),
                )
            },
        )
    }

    /// MOD-158 D4: with both flags the DSN is the first line and the key the second, trimmed.
    #[test]
    fn the_key_line_is_read_after_the_dsn_line() {
        let (dsn, key) = read("postgres://db/htui\n  the-key \nrest\n", true, true).expect("both");
        assert_eq!(dsn, "postgres://db/htui");
        assert_eq!(key.as_deref(), Some("the-key"));
        let (_, key) = read("postgres://db/htui\n", true, false).expect("the DSN alone");
        assert_eq!(key, None, "no flag, no key line read");
    }

    /// MOD-158 D4: a blank key line, or none, is a refusal, never a fall-through.
    #[test]
    fn a_blank_key_line_is_a_refusal() {
        for input in ["postgres://db/htui\n \n", "postgres://db/htui\n"] {
            let refused = read(input, true, true).expect_err("no key on stdin");
            assert!(matches!(refused, WorkerExit::Refused(_)), "{refused:?}");
            assert_eq!(
                refused.to_string(),
                "no Qdrant API key on stdin: the key line was blank or missing"
            );
        }
    }

    /// MOD-158 D4: `--qdrant-key-stdin` alone reads the key from the first line and leaves the DSN
    /// to the keyring. The keyring fake is one process-wide slot: run under `--test-threads=1`.
    #[tokio::test]
    async fn the_key_line_alone_leaves_the_dsn_to_the_keyring() {
        let _keyring = htui_store::testkit::mock_keyring().await;
        secret::set_dsn("postgres://keyring/htui").expect("the fake keyring stores");
        let (dsn, key) = read("the-key\n", false, true).expect("the keyring's DSN");
        assert_eq!(dsn, "postgres://keyring/htui");
        assert_eq!(key.as_deref(), Some("the-key"));
    }
}
