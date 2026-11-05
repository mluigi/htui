//! htui's own log (MOD-122): what every `tracing` event that passed the global filter becomes.
//!
//! One subscriber, always (plan D1): a global [`EnvFilter`] that can be reloaded at run time, a
//! [`MemoryLayer`] into the [`LogBuffer`] the Logs tab reads, a `fmt` layer writing through a
//! file that can be switched on, moved or off at run time, and the Sentry layer **only** when a
//! log file was given at startup (`--log`/`HTUI_LOG`), exactly as before MOD-122.
//!
//! [`buffer`] keeps the last [`CAPACITY`] lines in memory; [`source`] maps an event's target to
//! the part of htui it came from. [`init`] is `run`'s and nothing else's; a test builds a
//! [`Logging`] and runs its events under `tracing::dispatcher::with_default` (D7), and the
//! harnesses' `App` holds a [`Logging::detached`] one, which has no subscriber at all.

pub mod buffer;
pub mod source;

pub use buffer::{CAPACITY, Counts, LogBuffer, LogLine, MemoryLayer, Since, SourceCounts};
pub use source::Source;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tracing::Dispatch;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::{EnvFilter, Registry, reload};

/// The filter when neither `HTUI_LOG_FILTER` nor a saved setting gives one.
pub const DEFAULT_FILTER: &str = "info";
/// The variable whose directive wins over the saved filter for the session.
pub const FILTER_VAR: &str = "HTUI_LOG_FILTER";
/// What Settings › Logging names as the override of the log file.
pub const SET_BY_FLAG: &str = "`--log`";
/// What Settings › Logging names as the override of the filter.
pub const SET_BY_VAR: &str = "`HTUI_LOG_FILTER`";

/// What the process was started with (flag and environment). Each wins over its saved value for
/// the session (D5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Startup {
    /// `--log`/`HTUI_LOG` (clap reads both).
    pub file: Option<PathBuf>,
    /// `HTUI_LOG_FILTER`, when it holds a usable directive.
    pub filter: Option<String>,
}

impl Startup {
    /// `run`'s: `--log`/`HTUI_LOG` and `HTUI_LOG_FILTER`.
    #[must_use]
    pub fn from_env(log: Option<&Path>) -> Self {
        Self::from_parts(log, std::env::var(FILTER_VAR).ok())
    }

    /// A blank or unparsable variable counts as unset, as before MOD-122 (it fell back to
    /// `info`).
    #[must_use]
    pub fn from_parts(log: Option<&Path>, filter_var: Option<String>) -> Self {
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

/// D7: what [`init`] hands back; `App` holds it.
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
    pub fn build(startup: Startup) -> anyhow::Result<Self> {
        let initial = startup.filter.as_deref().unwrap_or(DEFAULT_FILTER);
        let (filter, handle) = reload::Layer::new(parse_filter(initial)?);
        let file = startup.file.as_deref().map(open_append).transpose()?;
        let switch = FileSwitch::new(file);
        let buffer = LogBuffer::new();
        // D1: Sentry rides a startup log file only, exactly as before (MOD-99 owns widening it).
        let sentry = startup.file.is_some();
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(MemoryLayer::new(buffer.clone()))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(switch.clone())
                    .with_ansi(false),
            )
            .with(sentry.then(sentry_tracing::layer));
        Ok(Self {
            buffer,
            control: LoggingControl::new(Some(handle), switch, &startup),
            dispatch: Some(Dispatch::new(subscriber)),
            sentry,
        })
    }

    /// No subscriber at all: `App::new`'s and the harnesses'. The control still records the
    /// filter and switches the file, so a test can watch what an action did.
    #[must_use]
    pub fn detached() -> Self {
        Self {
            buffer: LogBuffer::new(),
            control: LoggingControl::new(None, FileSwitch::default(), &Startup::default()),
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

/// [`Logging::build`], then installed process-wide (the same call as before MOD-122, so the
/// `log` bridge is unchanged). `run` calls it once; nothing else may.
///
/// # Errors
///
/// The startup log file cannot be opened, or a subscriber is already installed.
pub fn init(startup: Startup) -> anyhow::Result<Logging> {
    use tracing_subscriber::util::SubscriberInitExt as _;

    let logging = Logging::build(startup)?;
    if let Some(dispatch) = logging.dispatch.clone() {
        dispatch
            .try_init()
            .map_err(|err| anyhow::anyhow!("could not install the log subscriber: {err}"))?;
    }
    Ok(logging)
}

/// D7: the run-time handle the Settings section and `App` share. Clone = the same state.
#[derive(Clone)]
pub struct LoggingControl(Arc<ControlInner>);

struct ControlInner {
    /// `None` when detached: the filter is recorded, nothing reloads.
    reload: Option<reload::Handle<EnvFilter, Registry>>,
    file: FileSwitch,
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
        reload: Option<reload::Handle<EnvFilter, Registry>>,
        file: FileSwitch,
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

    /// Reloads the global filter with `directive`.
    ///
    /// # Errors
    ///
    /// [`LoggingError::Overridden`] while `HTUI_LOG_FILTER` sets it; [`LoggingError::Filter`]
    /// for a directive [`parse_filter`] refuses (nothing changes); [`LoggingError::Gone`] when
    /// the subscriber was dropped.
    pub fn set_filter(&self, directive: &str) -> Result<(), LoggingError> {
        if let Some(by) = self.lock().filter_set_by {
            return Err(LoggingError::Overridden(by));
        }
        let directive = directive.trim();
        let filter = parse_filter(directive)?;
        if let Some(handle) = &self.0.reload {
            handle.reload(filter).map_err(|_| LoggingError::Gone)?;
        }
        self.lock().filter = directive.to_owned();
        Ok(())
    }

    /// Switches the log file: `Some` is opened (create, append) **before** the swap, and the old
    /// file is closed once the writer's lock is let go; `None` is no file.
    ///
    /// # Errors
    ///
    /// [`LoggingError::Overridden`] while `--log` sets it; [`LoggingError::File`] when the file
    /// cannot be opened (the previous one stays, and `file_error` says why).
    pub fn set_file(&self, path: Option<&Path>) -> Result<(), LoggingError> {
        if let Some(by) = self.lock().file_set_by {
            return Err(LoggingError::Overridden(by));
        }
        let file = match path.map(open_append).transpose() {
            Ok(file) => file,
            Err(err) => {
                let err = LoggingError::File {
                    path: path.map(Path::to_path_buf).unwrap_or_default(),
                    why: one_line(&err.to_string()),
                };
                self.lock().file_error = Some(err.to_string());
                return Err(err);
            }
        };
        let old = self.0.file.swap(file);
        drop(old);
        let mut state = self.lock();
        state.file = path.map(Path::to_path_buf);
        state.file_error = None;
        Ok(())
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
    /// A flag or variable sets this setting for the session ([`SET_BY_FLAG`], [`SET_BY_VAR`]).
    Overridden(&'static str),
    /// The subscriber the filter belonged to was dropped.
    Gone,
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

/// `text` with every run of whitespace, line breaks included, as one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Opens `path` to append, creating it, as the `--log` file always was.
fn open_append(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// The `fmt` layer's writer (D1): the current file, or none (writes dropped). Clone = the same
/// slot.
#[derive(Clone, Default)]
struct FileSwitch(Arc<Mutex<Option<std::fs::File>>>);

impl FileSwitch {
    fn new(file: Option<std::fs::File>) -> Self {
        Self(Arc::new(Mutex::new(file)))
    }

    /// Puts `file` in the slot and hands back the one it replaces, to be dropped (closed) after
    /// the lock is let go.
    fn swap(&self, file: Option<std::fs::File>) -> Option<std::fs::File> {
        std::mem::replace(
            &mut *self.0.lock().unwrap_or_else(PoisonError::into_inner),
            file,
        )
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
        SwitchWriter(self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        Logging, LoggingError, LoggingPrefs, SET_BY_FLAG, SET_BY_VAR, Startup, parse_filter,
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

    /// D1: what reaches Sentry is MOD-99's call, so the layer rides a startup log file only.
    #[test]
    fn sentry_rides_only_a_startup_log_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let a = dir.path().join("a.log");
        let built = Logging::build(Startup::default()).expect("built");
        assert!(!built.sentry());
        built.control().apply_saved(Some(&LoggingPrefs {
            filter: None,
            file: Some(a.clone()),
        }));
        assert!(!built.sentry(), "a saved file never brings Sentry in");
        assert!(!Logging::detached().sentry());
        let with_file = Logging::build(Startup {
            file: Some(a),
            filter: None,
        })
        .expect("built");
        assert!(with_file.sentry());
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
