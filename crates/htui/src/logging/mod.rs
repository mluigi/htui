//! htui's own log (MOD-122): what every `tracing` event that passes the filter becomes.
//!
//! One subscriber, always (plan D1): a [`MemoryLayer`] into the [`LogBuffer`] the Logs tab reads
//! and a `fmt` layer writing through a file that can be switched on, moved or off at run time,
//! both behind one `EnvFilter` that can be reloaded at run time; and the Sentry layer **only**
//! when a log file was given at startup (`--log`/`HTUI_LOG`), behind a filter of its own fixed
//! at startup, so it sees exactly what it saw before MOD-122 whatever Settings › Logging sets.
//!
//! [`buffer`] keeps the last [`CAPACITY`] lines in memory; [`source`] maps an event's target to
//! the part of htui it came from. `init` is `run`'s and nothing else's; a test builds a
//! [`Logging`] and runs its events under `tracing::dispatcher::with_default` (D7), and the
//! harnesses' `App` holds a [`Logging::detached`] one, which has no subscriber at all.

pub mod buffer;
pub mod source;

pub use buffer::{CAPACITY, Counts, LogBuffer, LogLine, MemoryLayer, Since, SourceCounts};
pub use source::Source;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tracing::Dispatch;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::layer::{Layer as _, Layered, SubscriberExt as _};
use tracing_subscriber::{EnvFilter, Registry, filter, reload};

/// The filter when neither `HTUI_LOG_FILTER` nor a saved setting gives one.
pub(crate) const DEFAULT_FILTER: &str = "info";
/// The variable whose directive wins over the saved filter for the session.
const FILTER_VAR: &str = "HTUI_LOG_FILTER";
/// What Settings › Logging names as the override of the log file.
pub(crate) const SET_BY_FLAG: &str = "`--log`";
/// What Settings › Logging names as the override of the filter.
pub(crate) const SET_BY_VAR: &str = "`HTUI_LOG_FILTER`";

/// The Sentry layer's slot (D1), boxed so a test can stand a layer of its own in for it.
type SentryLayer = Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync>;

/// What the buffer and the file layers sit on: the registry and the Sentry slot.
type Base = Layered<Option<SentryLayer>, Registry>;

/// What the process was started with (flag and environment). Each wins over its saved value for
/// the session (D5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Startup {
    /// `--log`/`HTUI_LOG` (clap reads both).
    pub(crate) file: Option<PathBuf>,
    /// `HTUI_LOG_FILTER`, when it holds a usable directive.
    pub(crate) filter: Option<String>,
}

impl Startup {
    /// `run`'s: `--log`/`HTUI_LOG` and `HTUI_LOG_FILTER`.
    #[must_use]
    pub(crate) fn from_env(log: Option<&Path>) -> Self {
        Self::from_parts(log, std::env::var(FILTER_VAR).ok())
    }

    /// A blank or unparsable variable counts as unset, as before MOD-122 (it fell back to
    /// `info`).
    #[must_use]
    pub(crate) fn from_parts(log: Option<&Path>, filter_var: Option<String>) -> Self {
        let filter = filter_var
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty() && parse_filter(value).is_ok());
        Self {
            file: log.map(Path::to_path_buf),
            filter,
        }
    }
}

/// The settings saved in `ui-state.toml`'s `[logging]` table (D5). `None` = the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoggingPrefs {
    /// The filter directive.
    pub filter: Option<String>,
    /// The log file.
    pub file: Option<PathBuf>,
}

impl LoggingPrefs {
    /// Nothing is saved.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.filter.is_none() && self.file.is_none()
    }
}

/// D7: what `init` hands back; `App` holds it.
pub struct Logging {
    buffer: LogBuffer,
    control: LoggingControl,
    dispatch: Option<Dispatch>,
    sentry: bool,
}

impl Logging {
    /// The subscriber (D1) as a [`Dispatch`], not installed.
    ///
    /// # Errors
    ///
    /// The startup log file cannot be opened.
    pub(crate) fn build(startup: Startup) -> anyhow::Result<Self> {
        Self::build_with(startup, || Box::new(sentry_tracing::layer()))
    }

    /// [`build`](Self::build) with `sentry` making the layer for the Sentry slot, so a test can
    /// watch what reaches it.
    fn build_with(startup: Startup, sentry: impl FnOnce() -> SentryLayer) -> anyhow::Result<Self> {
        let initial = startup.filter.as_deref().unwrap_or(DEFAULT_FILTER);
        let (filter, handle) = reload::Layer::new(parse_filter(initial)?);
        let file = startup.file.as_deref().map(open_append).transpose()?;
        let switch = FileSwitch::new(file);
        let buffer = LogBuffer::new();
        // D1: Sentry rides a startup log file only, exactly as before (MOD-99 owns widening it),
        // behind its own copy of the startup filter, which nothing reloads (MOD-122 R1): a filter
        // set in Settings changes what htui keeps, never what reaches Sentry.
        let sentry_filter = startup
            .file
            .is_some()
            .then(|| parse_filter(initial))
            .transpose()?;
        let sentry_level = sentry_filter
            .as_ref()
            .map_or(LevelFilter::OFF, |filter| widest(filter.max_level_hint()));
        let sentry = sentry_filter.map(|filter| sentry().with_filter(filter).boxed());
        let on = Arc::clone(&switch.on);
        // MOD-122 R1: with no file the `fmt` layer is filtered out, so no line is formatted for
        // nothing; asked per event, since the file switches at run time.
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(switch.clone())
            .with_ansi(false)
            .with_filter(filter::dynamic_filter_fn(move |_, _| {
                on.load(Ordering::Relaxed)
            }));
        let local = MemoryLayer::new(buffer.clone())
            .and_then(file_layer)
            .with_filter(filter);
        let with_sentry = sentry.is_some();
        let subscriber = tracing_subscriber::registry().with(sentry).with(local);
        Ok(Self {
            buffer,
            control: LoggingControl::new(Some(handle), switch, sentry_level, &startup),
            dispatch: Some(Dispatch::new(subscriber)),
            sentry: with_sentry,
        })
    }

    /// No subscriber at all: `App::new`'s and the harnesses'. The control still records the
    /// filter and switches the file, so a test can watch what an action did.
    #[must_use]
    pub fn detached() -> Self {
        Self {
            buffer: LogBuffer::new(),
            control: LoggingControl::new(
                None,
                FileSwitch::default(),
                LevelFilter::OFF,
                &Startup::default(),
            ),
            dispatch: None,
            sentry: false,
        }
    }

    /// The in-memory log the Logs tab reads.
    #[must_use]
    pub const fn buffer(&self) -> &LogBuffer {
        &self.buffer
    }

    /// The run-time handle Settings › Logging and `App` share.
    #[must_use]
    pub const fn control(&self) -> &LoggingControl {
        &self.control
    }

    /// The subscriber, when this was built rather than detached.
    #[must_use]
    pub const fn dispatch(&self) -> Option<&Dispatch> {
        self.dispatch.as_ref()
    }

    /// Whether the Sentry layer is in the subscriber (D1 pin). Test-only:
    /// nothing outside the pin reads it.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn sentry(&self) -> bool {
        self.sentry
    }
}

impl fmt::Debug for Logging {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Logging")
            .field("buffer", &self.buffer)
            .field("control", &self.control)
            .field("installed", &self.dispatch.is_some())
            .field("sentry", &self.sentry)
            .finish()
    }
}

/// [`Logging::build`], then installed process-wide (the same call as before MOD-122, which also
/// installs the `log` bridge). `run` calls it once; nothing else may.
///
/// # Errors
///
/// The startup log file cannot be opened, or a subscriber is already installed.
pub(crate) fn init(startup: Startup) -> anyhow::Result<Logging> {
    use tracing_subscriber::util::SubscriberInitExt as _;

    let logging = Logging::build(startup)?;
    if let Some(dispatch) = logging.dispatch.clone() {
        dispatch
            .try_init()
            .map_err(|err| anyhow::anyhow!("could not install the log subscriber: {err}"))?;
        logging.control.0.installed.store(true, Ordering::Relaxed);
        logging.control.bridge_log();
    }
    Ok(logging)
}

/// D7: the run-time handle the Settings section and `App` share. Clone = the same state.
#[derive(Clone)]
pub struct LoggingControl(Arc<ControlInner>);

struct ControlInner {
    /// `None` when detached: the filter is recorded, nothing reloads.
    reload: Option<reload::Handle<EnvFilter, Base>>,
    file: FileSwitch,
    /// The widest level the Sentry layer's fixed filter lets through; `OFF` without the layer.
    sentry: LevelFilter,
    /// `init` installed this subscriber process-wide, so the `log` crate's max level is its to
    /// keep (MOD-122 R1). Never set in a test: nothing process-global moves there.
    installed: AtomicBool,
    state: Mutex<LoggingState>,
}

/// What Settings › Logging shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingState {
    /// The filter directive in force.
    pub filter: String,
    /// The log file in force.
    pub file: Option<PathBuf>,
    /// `Some(SET_BY_VAR)` when `HTUI_LOG_FILTER` set the filter for this session.
    pub filter_set_by: Option<&'static str>,
    /// `Some(SET_BY_FLAG)` when `--log`/`HTUI_LOG` set the file for this session.
    pub file_set_by: Option<&'static str>,
    /// Why the last file asked for could not be opened; a good switch clears it.
    pub file_error: Option<String>,
}

impl LoggingControl {
    fn new(
        reload: Option<reload::Handle<EnvFilter, Base>>,
        file: FileSwitch,
        sentry: LevelFilter,
        startup: &Startup,
    ) -> Self {
        let state = LoggingState {
            filter: startup
                .filter
                .clone()
                .unwrap_or_else(|| DEFAULT_FILTER.to_owned()),
            file: startup.file.clone(),
            filter_set_by: startup.filter.is_some().then_some(SET_BY_VAR),
            file_set_by: startup.file.is_some().then_some(SET_BY_FLAG),
            file_error: None,
        };
        Self(Arc::new(ControlInner {
            reload,
            file,
            sentry,
            installed: AtomicBool::new(false),
            state: Mutex::new(state),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, LoggingState> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What is in force now.
    #[must_use]
    pub fn state(&self) -> LoggingState {
        self.lock().clone()
    }

    /// Reloads the buffer's and the file's filter with `directive`. The state's lock is held from
    /// the override check to the record, so two changes never interleave (MOD-122 R1).
    ///
    /// # Errors
    ///
    /// [`LoggingError::Overridden`] while `HTUI_LOG_FILTER` sets it; [`LoggingError::Filter`]
    /// for a directive `parse_filter` refuses (nothing changes); [`LoggingError::Gone`] when
    /// the subscriber was dropped, [`LoggingError::Poisoned`] when its filter's lock is.
    pub fn set_filter(&self, directive: &str) -> Result<(), LoggingError> {
        let mut state = self.lock();
        if let Some(by) = state.filter_set_by {
            return Err(LoggingError::Overridden(by));
        }
        let directive = directive.trim();
        let filter = parse_filter(directive)?;
        if let Some(handle) = &self.0.reload {
            handle.reload(filter).map_err(|err| {
                if err.is_poisoned() {
                    LoggingError::Poisoned
                } else {
                    LoggingError::Gone
                }
            })?;
        }
        state.filter = directive.to_owned();
        drop(state);
        self.bridge_log();
        Ok(())
    }

    /// Switches the log file: `Some` is opened (create, append) **before** the swap, and the old
    /// file is closed once the writer's lock is let go; `None` is no file. The state's lock is
    /// held throughout, as [`set_filter`](Self::set_filter)'s.
    ///
    /// # Errors
    ///
    /// [`LoggingError::Overridden`] while `--log` sets it; [`LoggingError::File`] when the file
    /// cannot be opened or is not a regular file (the previous one stays, and `file_error` says
    /// why).
    pub fn set_file(&self, path: Option<&Path>) -> Result<(), LoggingError> {
        let mut state = self.lock();
        if let Some(by) = state.file_set_by {
            return Err(LoggingError::Overridden(by));
        }
        let file = match path.map(open_regular).transpose() {
            Ok(file) => file,
            Err(err) => {
                let err = LoggingError::File {
                    path: path.map(Path::to_path_buf).unwrap_or_default(),
                    why: one_line(&err.to_string()),
                };
                state.file_error = Some(err.to_string());
                return Err(err);
            }
        };
        let old = self.0.file.swap(file);
        drop(old);
        state.file = path.map(Path::to_path_buf);
        state.file_error = None;
        Ok(())
    }

    /// The `log` crate's max level this subscriber wants (MOD-122 R1): the widest of the run-time
    /// filter and the Sentry layer's. `log` drops a record above its max level before the bridge
    /// sees it, and `init` sets it once, from the startup filter.
    fn log_level(&self) -> log::LevelFilter {
        let local = self.0.reload.as_ref().map_or(LevelFilter::OFF, |handle| {
            handle
                .with_current(|filter| widest(filter.max_level_hint()))
                .unwrap_or(LevelFilter::OFF)
        });
        as_log(local.max(self.0.sentry))
    }

    /// Raises or lowers the `log` crate's max level to [`log_level`](Self::log_level), only for
    /// the subscriber `init` installed.
    fn bridge_log(&self) {
        if self.0.installed.load(Ordering::Relaxed) {
            log::set_max_level(self.log_level());
        }
    }

    /// D5 at startup: each saved field `HTUI_LOG_FILTER` or `--log` does not override is applied.
    /// The first that cannot apply comes back as one line for the status line; the other field
    /// still applies.
    pub fn apply_saved(&self, saved: Option<&LoggingPrefs>) -> Option<String> {
        let saved = saved?;
        let state = self.state();
        let mut notices = Vec::new();
        if let Some(filter) = saved.filter.as_deref()
            && state.filter_set_by.is_none()
            && let Err(err) = self.set_filter(filter)
        {
            let why = match err {
                LoggingError::Filter { why, .. } => why,
                other => other.to_string(),
            };
            notices.push(format!(
                "{}: logging.filter `{filter}` not applied: {why}; using {}",
                crate::ui::state_file::FILE_NAME,
                self.state().filter,
            ));
        }
        if let Some(file) = saved.file.as_deref()
            && state.file_set_by.is_none()
            && let Err(err) = self.set_file(Some(file))
        {
            let why = match err {
                LoggingError::File { why, .. } => why,
                other => other.to_string(),
            };
            notices.push(format!(
                "{}: logging.file {} not applied: {why}; no log file",
                crate::ui::state_file::FILE_NAME,
                file.display(),
            ));
        }
        for notice in &notices {
            tracing::warn!(%notice, "a saved logging setting was not applied");
        }
        notices.into_iter().next()
    }
}

impl fmt::Debug for LoggingControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LoggingControl")
            .field(&self.state())
            .finish()
    }
}

/// Why a logging setting was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggingError {
    /// The directive is not an `EnvFilter` one; nothing changed.
    Filter {
        /// The directive refused.
        directive: String,
        /// Why, on one line.
        why: String,
    },
    /// The file could not be opened; the previous one stays.
    File {
        /// The file refused.
        path: PathBuf,
        /// Why, on one line.
        why: String,
    },
    /// A flag or variable sets this setting for the session (`SET_BY_FLAG`, `SET_BY_VAR`).
    Overridden(&'static str),
    /// The subscriber the filter belonged to was dropped.
    Gone,
    /// The filter's lock was poisoned by a thread that panicked holding it; nothing changed.
    Poisoned,
}

impl fmt::Display for LoggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Filter { directive, why } => {
                write!(f, "`{directive}` is not a log filter: {why}")
            }
            Self::File { path, why } => write!(
                f,
                "cannot open the log file {}: {why}; the previous one stays",
                path.display()
            ),
            Self::Overridden(by) => write!(f, "set by {by} for this session"),
            Self::Gone => f.write_str("the log subscriber is gone"),
            Self::Poisoned => f.write_str("the log filter's lock is poisoned"),
        }
    }
}

impl std::error::Error for LoggingError {}

/// B3: the one validator of a filter directive (`EnvFilter::try_new`), which the control, the
/// startup apply and Settings › Logging all call, so a refusal reads the same everywhere. A blank
/// directive is refused too: it would filter out everything.
///
/// # Errors
///
/// [`LoggingError::Filter`], `why` on one line.
pub fn parse_filter(directive: &str) -> Result<EnvFilter, LoggingError> {
    let refuse = |why: String| LoggingError::Filter {
        directive: directive.to_owned(),
        why,
    };
    if directive.trim().is_empty() {
        return Err(refuse("it is empty".to_owned()));
    }
    EnvFilter::try_new(directive).map_err(|err| refuse(one_line(&err.to_string())))
}

/// A filter's max level hint, `None` (no hint) read as the widest.
fn widest(hint: Option<LevelFilter>) -> LevelFilter {
    hint.unwrap_or(LevelFilter::TRACE)
}

/// `level` as the `log` crate's.
fn as_log(level: LevelFilter) -> log::LevelFilter {
    match level.into_level() {
        None => log::LevelFilter::Off,
        Some(tracing::Level::ERROR) => log::LevelFilter::Error,
        Some(tracing::Level::WARN) => log::LevelFilter::Warn,
        Some(tracing::Level::INFO) => log::LevelFilter::Info,
        Some(tracing::Level::DEBUG) => log::LevelFilter::Debug,
        Some(_) => log::LevelFilter::Trace,
    }
}

/// `text` with every run of whitespace, line breaks included, as one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Opens `path` to append, creating it, as the `--log` file always was: a regular file or a
/// device (`/dev/null`, `/dev/stderr`). On Unix the open itself never blocks (MOD-122 R1):
/// opening a FIFO to write blocks until a reader comes, so it opens `O_NONBLOCK`, cleared once
/// open, and a FIFO no one reads is refused at once instead of hanging startup.
fn open_append(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        rustix::fs::OFlags::NONBLOCK.bits().cast_signed(),
    );
    let file = options.open(path).map_err(|err| unread_fifo(path, err))?;
    #[cfg(unix)]
    rustix::fs::fcntl_setfl(
        &file,
        rustix::fs::fcntl_getfl(&file)? - rustix::fs::OFlags::NONBLOCK,
    )?;
    Ok(file)
}

/// `ENXIO` from a non-blocking open of a FIFO means no one reads it; anything else stays as is.
fn unread_fifo(path: &Path, err: io::Error) -> io::Error {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt as _;

        if rustix::io::Errno::from_io_error(&err) == Some(rustix::io::Errno::NXIO)
            && std::fs::metadata(path).is_ok_and(|meta| meta.file_type().is_fifo())
        {
            return io::Error::other("a FIFO no one reads");
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    err
}

/// [`open_append`] for a file set at run time (Settings › Logging, `ui-state.toml`): a regular
/// file only (MOD-122 R1). A FIFO or a device written from the event loop can stall it, and a
/// terminal would write over the TUI, so a path that exists and is something else is refused
/// before it is opened, and one that turns into something else in between is refused after.
fn open_regular(path: &Path) -> io::Result<std::fs::File> {
    let not_regular = || io::Error::other("not a regular file");
    if std::fs::metadata(path).is_ok_and(|meta| !meta.is_file()) {
        return Err(not_regular());
    }
    let file = open_append(path)?;
    if !file.metadata()?.is_file() {
        return Err(not_regular());
    }
    Ok(file)
}

/// The `fmt` layer's writer (D1): the current file, or none (writes dropped). Clone = the same
/// slot.
#[derive(Clone, Default)]
struct FileSwitch {
    slot: Arc<Mutex<Option<std::fs::File>>>,
    /// Whether the slot holds a file: the `fmt` layer's filter (MOD-122 R1).
    on: Arc<AtomicBool>,
    /// How many writers the `fmt` layer asked for, so a test sees whether it formatted.
    #[cfg(test)]
    made: Arc<std::sync::atomic::AtomicUsize>,
}

impl FileSwitch {
    fn new(file: Option<std::fs::File>) -> Self {
        let switch = Self::default();
        drop(switch.swap(file));
        switch
    }

    /// Puts `file` in the slot and hands back the one it replaces, to be dropped (closed) after
    /// the lock is let go.
    fn swap(&self, file: Option<std::fs::File>) -> Option<std::fs::File> {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        self.on.store(file.is_some(), Ordering::Relaxed);
        std::mem::replace(&mut *slot, file)
    }

    /// How many writers the `fmt` layer asked for.
    #[cfg(test)]
    fn writers(&self) -> usize {
        self.made.load(Ordering::Relaxed)
    }
}

/// One write's hold on the slot: the `fmt` layer formats into its own buffer first and holds this
/// for one `write_all`.
struct SwitchWriter<'a>(MutexGuard<'a, Option<std::fs::File>>);

impl io::Write for SwitchWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.as_mut() {
            Some(file) => file.write(buf),
            // No file: the line is dropped.
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.as_mut().map_or(Ok(()), io::Write::flush)
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileSwitch {
    type Writer = SwitchWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        #[cfg(test)]
        self.made.fetch_add(1, Ordering::Relaxed);
        SwitchWriter(self.slot.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        LogBuffer, Logging, LoggingError, LoggingPrefs, MemoryLayer, SET_BY_FLAG, SET_BY_VAR,
        SentryLayer, Startup, parse_filter,
    };

    /// Runs `f` under `logging`'s own subscriber, scoped to this thread (D7).
    fn under(logging: &Logging, f: impl FnOnce()) {
        let dispatch = logging.dispatch().expect("a built logging");
        tracing::dispatcher::with_default(dispatch, f);
    }

    fn texts(logging: &Logging) -> Vec<String> {
        logging
            .buffer()
            .since(0)
            .lines
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn the_filter_reloads_and_a_refused_directive_changes_nothing() {
        let logging = Logging::build(Startup::default()).expect("built");
        let control = logging.control().clone();
        assert_eq!(control.state().filter, "info");
        under(&logging, || {
            tracing::debug!(target: "htui::test", "hidden");
            tracing::info!(target: "htui::test", "shown");
        });
        assert_eq!(texts(&logging), ["shown"]);

        control.set_filter("debug").expect("a valid directive");
        under(
            &logging,
            || tracing::debug!(target: "htui::test", "debug 1"),
        );
        assert_eq!(texts(&logging), ["shown", "debug 1"]);

        let err = control.set_filter("htui=[").expect_err("refused");
        assert!(matches!(err, LoggingError::Filter { .. }), "{err:?}");
        assert!(
            err.to_string()
                .starts_with("`htui=[` is not a log filter: "),
            "{err}"
        );
        assert!(!err.to_string().contains('\n'));
        assert_eq!(control.state().filter, "debug");
        under(
            &logging,
            || tracing::debug!(target: "htui::test", "debug 2"),
        );
        assert_eq!(texts(&logging), ["shown", "debug 1", "debug 2"]);
        assert!(parse_filter("  ").is_err());
    }

    #[test]
    fn the_file_switches_on_moves_and_off() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (a, b) = (dir.path().join("a.log"), dir.path().join("b.log"));
        let logging = Logging::build(Startup::default()).expect("built");
        let control = logging.control().clone();
        assert_eq!(control.state().file, None);

        control.set_file(Some(&a)).expect("a opens");
        assert_eq!(control.state().file.as_deref(), Some(a.as_path()));
        under(&logging, || tracing::info!(target: "htui::test", "first"));
        assert!(read(&a).contains("first"), "{}", read(&a));
        assert!(!read(&a).contains('\u{1b}'), "no ANSI in a file");

        control.set_file(Some(&b)).expect("b opens");
        assert_eq!(control.state().file.as_deref(), Some(b.as_path()));
        under(&logging, || tracing::info!(target: "htui::test", "second"));
        assert!(!read(&a).contains("second"));
        assert!(read(&b).contains("second"));

        control.set_file(None).expect("no file");
        assert_eq!(control.state().file, None);
        under(&logging, || tracing::info!(target: "htui::test", "third"));
        assert!(!read(&b).contains("third"));
        assert_eq!(texts(&logging), ["first", "second", "third"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_old_file_is_closed_when_the_file_moves() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (a, b) = (dir.path().join("a.log"), dir.path().join("b.log"));
        let logging = Logging::build(Startup::default()).expect("built");
        let control = logging.control();
        let open_on = |path: &Path| {
            std::fs::read_dir("/proc/self/fd")
                .expect("/proc/self/fd")
                .filter_map(Result::ok)
                .filter_map(|entry| std::fs::read_link(entry.path()).ok())
                .any(|link| link == path)
        };
        control.set_file(Some(&a)).expect("a opens");
        assert!(open_on(&a));
        control.set_file(Some(&b)).expect("b opens");
        assert!(!open_on(&a), "the old file is still open");
        assert!(open_on(&b));
        control.set_file(None).expect("no file");
        assert!(
            !open_on(&b),
            "the file is still open after switching it off"
        );
    }

    #[test]
    fn a_file_that_cannot_be_opened_leaves_the_previous_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let a = dir.path().join("a.log");
        let logging = Logging::build(Startup::default()).expect("built");
        let control = logging.control().clone();
        control.set_file(Some(&a)).expect("a opens");

        let err = control.set_file(Some(dir.path())).expect_err("a directory");
        let LoggingError::File { path, .. } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(path, dir.path());
        let shown = err.to_string();
        assert!(shown.contains(&dir.path().display().to_string()), "{shown}");
        assert!(shown.ends_with("; the previous one stays"), "{shown}");
        let state = control.state();
        assert_eq!(state.file.as_deref(), Some(a.as_path()));
        assert_eq!(state.file_error.as_deref(), Some(shown.as_str()));
        under(&logging, || tracing::info!(target: "htui::test", "still a"));
        assert!(read(&a).contains("still a"));

        control.set_file(Some(&a)).expect("a again");
        assert_eq!(control.state().file_error, None);
    }

    #[test]
    fn a_startup_file_and_filter_mark_their_overrides() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let a = dir.path().join("a.log");
        let logging = Logging::build(Startup {
            file: Some(a.clone()),
            filter: Some("debug".to_owned()),
        })
        .expect("built");
        let control = logging.control();
        let state = control.state();
        assert_eq!(state.filter, "debug");
        assert_eq!(state.file.as_deref(), Some(a.as_path()));
        assert_eq!(state.file_set_by, Some(SET_BY_FLAG));
        assert_eq!(state.filter_set_by, Some(SET_BY_VAR));
        assert_eq!(
            control.set_filter("info"),
            Err(LoggingError::Overridden(SET_BY_VAR))
        );
        assert_eq!(
            control.set_file(None),
            Err(LoggingError::Overridden(SET_BY_FLAG))
        );
        assert_eq!(
            LoggingError::Overridden(SET_BY_FLAG).to_string(),
            "set by `--log` for this session"
        );
        under(&logging, || tracing::debug!(target: "htui::test", "to a"));
        assert!(read(&a).contains("to a"));
    }

    #[test]
    fn an_unusable_filter_variable_counts_as_unset() {
        for value in ["[", "  ", ""] {
            let startup = Startup::from_parts(None, Some(value.to_owned()));
            assert_eq!(startup.filter, None, "{value:?}");
        }
        let startup = Startup::from_parts(Some(Path::new("/x.log")), Some(" htui=debug ".into()));
        assert_eq!(startup.filter.as_deref(), Some("htui=debug"));
        assert_eq!(startup.file.as_deref(), Some(Path::new("/x.log")));
        assert_eq!(Startup::from_parts(None, None), Startup::default());
    }

    /// D1: what reaches Sentry is MOD-99's call, so the layer rides a startup log file only, under
    /// the startup filter (MOD-122 R1): a saved file never brings it in, and a filter set at run
    /// time neither mutes nor widens it. A buffer stands in for Sentry, in the same slot.
    #[test]
    fn sentry_rides_only_a_startup_log_file_under_the_startup_filter() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let a = dir.path().join("a.log");
        let sentry = LogBuffer::new();
        let stand_in = || -> SentryLayer { Box::new(MemoryLayer::new(sentry.clone())) };
        let sent = || -> Vec<String> {
            sentry
                .since(0)
                .lines
                .into_iter()
                .map(|line| line.text)
                .collect()
        };

        let without = Logging::build_with(Startup::default(), stand_in).expect("built");
        assert!(!without.sentry());
        without.control().apply_saved(Some(&LoggingPrefs {
            filter: Some("trace".to_owned()),
            file: Some(a.clone()),
        }));
        under(&without, || tracing::error!(target: "htui::test", "kept"));
        assert_eq!(texts(&without), ["kept"]);
        assert!(sent().is_empty(), "a saved file never brings Sentry in");
        assert!(!Logging::detached().sentry());

        let with = Logging::build_with(
            Startup {
                file: Some(a),
                filter: None,
            },
            stand_in,
        )
        .expect("built");
        assert!(with.sentry());
        with.control().set_filter("off").expect("a valid directive");
        under(&with, || {
            tracing::info!(target: "htui::test", "to sentry");
            tracing::debug!(target: "htui::test", "under the startup filter");
        });
        assert_eq!(sent(), ["to sentry"], "`off` does not mute Sentry");
        assert!(texts(&with).is_empty(), "`off` mutes the buffer");
        with.control()
            .set_filter("trace")
            .expect("a valid directive");
        under(&with, || tracing::debug!(target: "htui::test", "debug"));
        assert_eq!(sent(), ["to sentry"], "`trace` does not widen Sentry");
        assert_eq!(texts(&with), ["debug"]);
    }

    /// MOD-122 R1: `log` records (sqlx's, among others) pass `log`'s own max level before the
    /// bridge sees them. `tracing-subscriber`'s reload sets that level from
    /// `LevelFilter::current()` (its `tracing-log` feature, on by default), so a run-time filter
    /// reaches them with nothing of htui's own; this pins it. `LevelFilter::current()` is the
    /// widest over every live subscriber, so only a raise is certain while other tests run.
    #[test]
    fn a_reload_raises_the_log_crates_level() {
        let logging = Logging::build(Startup::default()).expect("built");
        let control = logging.control();
        control
            .set_filter("warn,sqlx=trace")
            .expect("a valid directive");
        assert_eq!(log::max_level(), log::LevelFilter::Trace);
        drop(logging);

        let dir = tempfile::tempdir().expect("a temp dir");
        let with_sentry = Logging::build(Startup {
            file: Some(dir.path().join("a.log")),
            filter: None,
        })
        .expect("built");
        with_sentry
            .control()
            .set_filter("off")
            .expect("a valid directive");
        assert!(
            log::max_level() >= log::LevelFilter::Info,
            "Sentry's startup filter still wants info: {}",
            log::max_level()
        );
    }

    /// MOD-122 R1: with no file the `fmt` layer is filtered out, so nothing is formatted for a
    /// line no file will hold; the buffer still keeps every line.
    #[test]
    fn nothing_is_formatted_for_the_file_without_a_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let a = dir.path().join("a.log");
        let logging = Logging::build(Startup::default()).expect("built");
        let control = logging.control();
        let switch = control.0.file.clone();
        under(&logging, || tracing::info!(target: "htui::test", "no file"));
        assert_eq!(switch.writers(), 0, "formatted with no file");
        control.set_file(Some(&a)).expect("a opens");
        under(&logging, || tracing::info!(target: "htui::test", "to a"));
        assert_eq!(switch.writers(), 1);
        control.set_file(None).expect("no file");
        under(
            &logging,
            || tracing::info!(target: "htui::test", "no file again"),
        );
        assert_eq!(switch.writers(), 1, "formatted after the file went");
        assert_eq!(texts(&logging), ["no file", "to a", "no file again"]);
        assert!(read(&a).contains("to a"));
    }

    /// MOD-122 R1: the startup `--log` opens a device as it always did (`/dev/null` keeps Sentry
    /// on without writing a file); a file set at run time must be a regular one.
    #[cfg(unix)]
    #[test]
    fn a_startup_device_still_opens_and_a_run_time_one_is_refused() {
        let null = Path::new("/dev/null");
        let logging = Logging::build(Startup {
            file: Some(null.to_path_buf()),
            filter: None,
        })
        .expect("`--log /dev/null` opens");
        assert!(logging.sentry());
        under(&logging, || tracing::info!(target: "htui::test", "dropped"));
        assert_eq!(texts(&logging), ["dropped"]);

        let plain = Logging::build(Startup::default()).expect("built");
        let err = plain
            .control()
            .set_file(Some(null))
            .expect_err("a device is refused at run time");
        assert!(err.to_string().contains(": not a regular file;"), "{err}");
    }

    /// MOD-122 R1: a FIFO is refused at once: opening one to write blocks until a reader comes,
    /// on the event loop or at startup. At run time any path that is not a regular file is
    /// refused; at startup the open cannot block, so a FIFO no one reads is refused there.
    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let fifo = dir.path().join("pipe");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs");
        assert!(made.success());
        let logging = Logging::build(Startup::default()).expect("built");
        let err = logging
            .control()
            .set_file(Some(&fifo))
            .expect_err("a FIFO is refused");
        assert!(err.to_string().contains(": not a regular file;"), "{err}");
        assert_eq!(logging.control().state().file, None);
        let err = Logging::build(Startup {
            file: Some(fifo),
            filter: None,
        })
        .expect_err("at startup too");
        assert!(
            format!("{err:#}").contains("a FIFO no one reads"),
            "{err:#}"
        );
    }

    /// MOD-122 R1: a filter set after the subscriber was dropped says so.
    #[test]
    fn a_dropped_subscriber_is_gone() {
        let logging = Logging::build(Startup::default()).expect("built");
        let control = logging.control().clone();
        drop(logging);
        assert_eq!(control.set_filter("debug"), Err(LoggingError::Gone));
        assert_eq!(control.state().filter, "info", "nothing is recorded");
        assert_eq!(
            LoggingError::Poisoned.to_string(),
            "the log filter's lock is poisoned"
        );
    }

    #[test]
    fn saved_settings_apply_unless_overridden() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let a = dir.path().join("a.log");
        let saved = LoggingPrefs {
            filter: Some("warn".to_owned()),
            file: Some(a.clone()),
        };

        let plain = Logging::build(Startup::default()).expect("built");
        assert_eq!(plain.control().apply_saved(None), None);
        assert_eq!(plain.control().apply_saved(Some(&saved)), None);
        let state = plain.control().state();
        assert_eq!(state.filter, "warn");
        assert_eq!(state.file.as_deref(), Some(a.as_path()));
        under(&plain, || {
            tracing::info!(target: "htui::test", "below warn");
            tracing::warn!(target: "htui::test", "a warning");
        });
        assert_eq!(texts(&plain), ["a warning"]);

        let by_var = Logging::build(Startup {
            file: None,
            filter: Some("debug".to_owned()),
        })
        .expect("built");
        assert_eq!(by_var.control().apply_saved(Some(&saved)), None);
        let state = by_var.control().state();
        assert_eq!(state.filter, "debug", "the variable wins for the session");
        assert_eq!(state.file.as_deref(), Some(a.as_path()));
    }

    #[test]
    fn a_bad_saved_filter_is_a_notice_and_the_file_still_applies() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let a = dir.path().join("a.log");
        let logging = Logging::build(Startup::default()).expect("built");
        let notice = logging
            .control()
            .apply_saved(Some(&LoggingPrefs {
                filter: Some("htui=[".to_owned()),
                file: Some(a.clone()),
            }))
            .expect("a notice");
        assert!(
            notice.starts_with("ui-state.toml: logging.filter `htui=[` not applied: "),
            "{notice}"
        );
        assert!(notice.ends_with("; using info"), "{notice}");
        assert!(!notice.contains('\n'));
        let state = logging.control().state();
        assert_eq!(state.filter, "info");
        assert_eq!(state.file.as_deref(), Some(a.as_path()));
    }

    #[test]
    fn a_detached_logging_installs_nothing() {
        let logging = Logging::detached();
        assert!(logging.dispatch().is_none());
        assert!(logging.buffer().is_empty());
        logging.control().set_filter("debug").expect("recorded");
        assert_eq!(logging.control().state().filter, "debug");
        assert!(logging.control().set_filter("htui=[").is_err());
        assert_eq!(logging.control().state().filter, "debug");
        assert!(format!("{logging:?}").starts_with("Logging { buffer: LogBuffer { lines: 0 }"));
    }
}
