//! The `$EDITOR` handoff (MOD-9 D8, D9): resolve the command, write a temp file, run the editor
//! with the TUI suspended, read the file back.
//!
//! Nothing here knows which view asked. A tab emits `Action::EditExternally` with an
//! [`ExternalEdit`]; the event loop takes it, calls [`run_suspended`] with the real terminal, and
//! hands the [`ExternalEditOutcome`] back to the tab (MOD-9 D10).

use std::io::{self, Write as _};
use std::path::Path;
use std::time::{Duration, Instant};

use htui_core::prompt::render::normalise_newlines;

/// The in-pane editor's process side (MOD-57 M1).
pub mod pane;

/// Below this, an unchanged return is reported as "quick" (MOD-9 blueprint D24): the shape of a
/// GUI editor started without `--wait`, which returns at once and leaves the file untouched.
pub const QUICK_EXIT: Duration = Duration::from_secs(1);

/// The notice after an `$EDITOR` return with text in it: the view holds the text, unsaved (MOD-9;
/// shared by the Skills views and the Backlog item form, MOD-13 milestone 4 D8).
pub const EDITED: &str = "edited in $EDITOR \u{2014} Ctrl+S saves";

/// The notice after an `$EDITOR` return that changed nothing (MOD-13 milestone 4 D8). The
/// Library also gives it for a rename that changes nothing.
pub const NO_CHANGES: &str = "no changes";

/// Appended to [`NO_CHANGES`] when the editor returned within [`QUICK_EXIT`] (MOD-9 blueprint
/// D24, R-3).
pub const WAIT_FLAG: &str = " \u{2014} a GUI editor needs its wait flag, e.g. `code --wait`";

/// The longest temp-file stem [`TempEdit::create`] keeps (blueprint D25), in chars.
const STEM_MAX: usize = 32;

/// The editor used when neither `$VISUAL` nor `$EDITOR` names one.
#[cfg(not(windows))]
const FALLBACK: &str = "vi";
/// The editor used when neither `$VISUAL` nor `$EDITOR` names one.
#[cfg(windows)]
const FALLBACK: &str = "notepad";

/// The variable that chooses the in-pane editor (MOD-57 plan P3), read next to `$VISUAL`/`$EDITOR`.
pub const PANE_VAR: &str = "HTUI_EDITOR_PANE";

/// The answer to a second edit while an in-pane editor is alive (MOD-57 P6, PD-8).
pub const EDITOR_BUSY: &str = "an editor is already open: return to it or abort it first";

/// The answer to `editor.abort` (MOD-57 P8): the editor is killed and nothing is read back.
pub const EDITOR_ABORTED: &str = "the editor was aborted; nothing was changed";

/// How `E`/`Ctrl+E` run the editor (MOD-57 P3, PRD D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    /// Today's handoff: the TUI is suspended while the editor runs. The default.
    Suspend,
    /// The editor runs on a pseudo-terminal drawn inside htui.
    Pane,
}

impl EditorMode {
    /// [`PANE_VAR`] through `lookup`: `1`, `true` or `yes` (trimmed, ASCII case-insensitive) is
    /// `Pane`; anything else, unset or blank included, is `Suspend`.
    pub fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let on = lookup(PANE_VAR).is_some_and(|value| {
            let value = value.trim();
            ["1", "true", "yes"]
                .into_iter()
                .any(|on| value.eq_ignore_ascii_case(on))
        });
        if on { Self::Pane } else { Self::Suspend }
    }
}

/// A resolved editor command (D8). `value` is handed to the platform shell verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    /// The command line, as the user's shell would read it: `code --wait`, `nvim -u NONE`.
    value: String,
    /// Whether `value` is the platform default because neither variable was set (D23).
    fallback: bool,
}

impl EditorCommand {
    /// `VISUAL`, then `EDITOR` (first non-blank after trim), else `vi` (unix) / `notepad`
    /// (windows) with `fallback: true`.
    pub fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Self {
        ["VISUAL", "EDITOR"]
            .into_iter()
            .filter_map(lookup)
            .find_map(|value| {
                let value = value.trim();
                (!value.is_empty()).then(|| Self {
                    value: value.to_owned(),
                    fallback: false,
                })
            })
            .unwrap_or_else(|| Self {
                value: FALLBACK.to_owned(),
                fallback: true,
            })
    }

    /// [`resolve`](Self::resolve) over `std::env::var`.
    #[must_use]
    pub fn from_env() -> Self {
        Self::resolve(|key| std::env::var(key).ok())
    }

    /// The command line shown in messages: `value`.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The process that edits `file`.
    ///
    /// Unix: `sh -c "exec <value> \"$1\"" htui-editor <file>`, so the shell parses `value` as the
    /// user's own shell would (git runs `$EDITOR` the same way) and the file is a positional
    /// argument, never spliced into the script. `exec` makes the editor htui's own child: `dash`
    /// forks for a lone command, and a waiting `sh` dies of the Ctrl-C an editor such as `ed`
    /// traps, which would read as a failure and remove the file it is about to save; the kill on
    /// drop would also reach only the shell. So `value` is a command and its arguments: a leading
    /// `VAR=x` or a compound (`a && b`) fails to start, or runs only its first part.
    ///
    /// Windows: `cmd /S /C "<value> "<file>""` as one raw argument; `/S` makes `cmd` strip exactly
    /// the outer pair of quotes. `cmd` has no `exec`. It handles Ctrl-C itself while it waits on
    /// its child rather than dying of it, so an edit that survives the key survives `cmd` too;
    /// but the kill on drop ends only `cmd`, and the editor lives on (not exercised: no Windows
    /// test runs here).
    #[must_use]
    pub fn command(&self, file: &Path) -> std::process::Command {
        #[cfg(not(windows))]
        {
            let mut command = std::process::Command::new("sh");
            command
                .arg("-c")
                .arg(format!("exec {} \"$1\"", self.value))
                .arg("htui-editor")
                .arg(file);
            command
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            let mut command = std::process::Command::new("cmd");
            command.raw_arg(format!("/S /C \"{} \"{}\"\"", self.value, file.display()));
            command
        }
    }

    /// The same process as [`command`](Self::command), on a pseudo-terminal (MOD-57 P9, PD-6).
    ///
    /// Unix: the same `sh -c "exec <value> \"$1\"" htui-editor <file>` argv, for the same
    /// reasons (see [`command`](Self::command)). The child is a session leader on its own PTY
    /// (portable-pty `setsid` + `TIOCSCTTY`), so `ctrl-c` reaches only it; no `Interrupts` are
    /// needed. `TERM` is `xterm-256color`, what the pane's VT parser speaks. The cwd is htui's
    /// own, as the suspended handoff inherits it (portable-pty would default to `$HOME`), and
    /// `LINES`/`COLUMNS` are removed so a stale value cannot override the PTY's size.
    ///
    /// Windows: `cmd /S /C "<value> <file name>"` with the cwd set to the file's directory.
    /// portable-pty quotes every argument and has no `raw_arg`, so the file is named bare; the
    /// temp file's name holds no space and no quote, and `/S` strips the one pair of quotes
    /// around the last argument. A `value` holding `"` is MOD-16's to verify.
    ///
    /// The builder carries the whole environment: never `Debug` or log it.
    #[must_use]
    pub fn pty_command(&self, file: &Path) -> portable_pty::CommandBuilder {
        #[cfg(not(windows))]
        {
            let mut command = portable_pty::CommandBuilder::new("sh");
            command.arg("-c");
            command.arg(format!("exec {} \"$1\"", self.value));
            command.arg("htui-editor");
            command.arg(file);
            command.env("TERM", "xterm-256color");
            command.env_remove("LINES");
            command.env_remove("COLUMNS");
            if let Ok(dir) = std::env::current_dir() {
                command.cwd(dir);
            }
            command
        }
        #[cfg(windows)]
        {
            let mut command = portable_pty::CommandBuilder::new("cmd");
            command.arg("/S");
            command.arg("/C");
            let name = file
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            command.arg(format!("{} {name}", self.value));
            if let Some(dir) = file.parent() {
                command.cwd(dir);
            }
            command.env("TERM", "xterm-256color");
            command
        }
    }
}

/// What the view asks the shell to edit (D10). Text is never `Debug`ged.
#[derive(Clone, PartialEq, Eq)]
pub struct ExternalEdit {
    /// The body handed to the editor.
    pub text: String,
    /// The temp file's name stem, normally the template name; sanitised by [`run`] (D25).
    pub stem: String,
}

impl core::fmt::Debug for ExternalEdit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExternalEdit")
            .field("text_len", &self.text.len())
            .field("stem", &self.stem)
            .finish()
    }
}

/// What came back (D9, D24). The edited text is never `Debug`ged.
#[derive(Clone, PartialEq, Eq)]
pub enum ExternalEditOutcome {
    /// The file changed; the normalised text ([`normalise_newlines`]).
    Edited(String),
    /// Byte-identical after normalising both sides.
    Unchanged {
        /// The editor returned within [`QUICK_EXIT`], the "GUI editor without `--wait`" shape
        /// (R-3).
        quick: bool,
    },
    /// Nothing changed and why, one sentence for the notice line.
    Failed(String),
}

/// `Edited` prints its length, as [`ExternalEdit`] does; the other two carry no body.
impl core::fmt::Debug for ExternalEditOutcome {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Edited(text) => f
                .debug_struct("Edited")
                .field("text_len", &text.len())
                .finish(),
            Self::Unchanged { quick } => f.debug_struct("Unchanged").field("quick", quick).finish(),
            Self::Failed(message) => f.debug_tuple("Failed").field(message).finish(),
        }
    }
}

/// D5: `returned` without the one final `\n` an editor adds on save (vim's `fixeol`, nano, VS
/// Code), when `handed`, the text the field handed out, had none. Exactly one is dropped:
/// `"abc\n\n"` back from `"abc"` is `"abc\n"`. When `handed` ends in `\n`, `returned` is kept
/// whole. `returned` is already LF-only (`editor::run` normalises it).
///
/// Shared by the item form and the detail compose area (MOD-13 milestone 5 D7).
#[must_use]
pub(crate) fn strip_added_newline<'a>(handed: &str, returned: &'a str) -> &'a str {
    if handed.ends_with('\n') {
        returned
    } else {
        returned.strip_suffix('\n').unwrap_or(returned)
    }
}

/// Review L2: `returned` without its control characters, as
/// [`TextArea::on_paste`](crate::ui::TextArea::on_paste) drops them from a paste, so an escape
/// sequence an editor or a script left in the file is not saved into the item, where typing and
/// pasting cannot put one. `\n` is kept, and so is `\t`: unlike a
/// paste, an editor's text is often indented code, and the area draws a `\t` as spaces to its
/// next tab stop with the cursor counted the same way (`text_area.rs` `drawn`, pinned by
/// `a_tab_draws_as_spaces_to_the_next_stop`). `returned` is already LF-only (`editor::run`
/// normalises it), so no `\r` is lost.
///
/// Shared by the item form and the detail compose area (MOD-13 milestone 5 D7).
#[must_use]
pub(crate) fn without_controls(returned: &str) -> String {
    returned
        .chars()
        .filter(|c| matches!(c, '\n' | '\t') || !c.is_control())
        .collect()
}

/// Leaving and re-entering the TUI's terminal state (D9). `TerminalGuard` is the real one; tests
/// use a recording fake.
pub trait Suspend {
    /// Give the terminal to a child: show the cursor, leave raw mode and the alternate screen.
    ///
    /// # Errors
    ///
    /// Whatever the terminal refused; the state may be half-left.
    fn leave(&mut self) -> io::Result<()>;
    /// Take it back: raw mode, alternate screen, clear so the next draw repaints whole.
    ///
    /// # Errors
    ///
    /// Whatever the terminal refused; the TUI cannot keep drawing.
    fn enter(&mut self) -> io::Result<()>;
}

/// How an editor process ended, whichever runner ran it (MOD-57 P8): [`run`]'s child process or
/// the in-pane editor's pseudo-terminal child. [`TempEdit::finish`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorExit {
    /// It exited with this code.
    Code(i32),
    /// A signal ended it.
    Signal,
}

/// [`run`]'s child: a status without a code is a signal's.
impl From<std::process::ExitStatus> for EditorExit {
    fn from(status: std::process::ExitStatus) -> Self {
        status.code().map_or(Self::Signal, Self::Code)
    }
}

/// The pseudo-terminal child's: a named signal is a signal; a code past `i32` (a Windows
/// `NTSTATUS`) saturates to `i32::MAX`, never wraps negative.
impl From<&portable_pty::ExitStatus> for EditorExit {
    fn from(status: &portable_pty::ExitStatus) -> Self {
        if status.signal().is_some() {
            Self::Signal
        } else {
            Self::Code(i32::try_from(status.exit_code()).unwrap_or(i32::MAX))
        }
    }
}

/// One edit's temp file (MOD-57 P8): written by [`create`](Self::create), read back by
/// [`finish`](Self::finish), removed when it drops, on every path. Shared by [`run`] and the
/// in-pane editor. `Debug` prints the path and the handed text's length, never the text.
pub struct TempEdit {
    /// Removed on drop (`tempfile::TempPath`).
    path: tempfile::TempPath,
    /// The text handed out, for the unchanged comparison.
    handed: String,
}

impl core::fmt::Debug for TempEdit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TempEdit")
            .field("path", &self.path.display())
            .field("text_len", &self.handed.len())
            .finish()
    }
}

impl TempEdit {
    /// [`run`]'s first half: a `.md` temp file named `htui-<sanitised stem>-<random>` (D25),
    /// holding `text`, its handle closed (a Windows editor could not write over it otherwise).
    ///
    /// # Errors
    ///
    /// `Failed("could not create a temp file: …")` or `Failed("could not write the temp file:
    /// …")`: nothing was handed out, and nothing is left behind.
    pub fn create(text: &str, stem: &str) -> Result<Self, ExternalEditOutcome> {
        let stem = sanitise(stem);
        let mut file = tempfile::Builder::new()
            .prefix(&format!("htui-{stem}-"))
            .suffix(".md")
            .tempfile()
            .map_err(|err| {
                ExternalEditOutcome::Failed(format!("could not create a temp file: {err}"))
            })?;
        file.write_all(text.as_bytes()).map_err(|err| {
            ExternalEditOutcome::Failed(format!("could not write the temp file: {err}"))
        })?;
        Ok(Self {
            path: file.into_temp_path(),
            handed: text.to_owned(),
        })
    }

    /// The file the editor is handed.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// [`run`]'s second half: how the editor ended, then the file read back (D9, D23, D24).
    ///
    /// `exit` is `Err` when the suspended editor could not be started at all ([`run`]); the
    /// in-pane editor never passes one, since its start is the spawn and a failed wait after it
    /// is answered by the pane ("lost track of …", MOD-57 R1 L-1). A non-zero code or a signal
    /// is `Failed`, the platform shell's "could not start" codes with the `$VISUAL`/`$EDITOR`
    /// sentence. Otherwise the file is read back: unreadable or not UTF-8 is `Failed`; equal to
    /// the handed text after [`normalise_newlines`] on both sides is `Unchanged` (`quick` when
    /// `elapsed` is under [`QUICK_EXIT`]); anything else is `Edited` with the normalised text.
    /// Consumes `self`: the file is removed when this returns.
    #[must_use]
    pub fn finish(
        self,
        cmd: &EditorCommand,
        exit: io::Result<EditorExit>,
        elapsed: Duration,
    ) -> ExternalEditOutcome {
        use ExternalEditOutcome::{Edited, Failed, Unchanged};

        match exit {
            Err(err) => return Failed(start_failure(cmd, &err.to_string())),
            Ok(EditorExit::Code(0)) => {}
            // The platform shell started and could not start the editor (F-D, D23).
            Ok(EditorExit::Code(code)) if is_start_failure(code) => {
                return Failed(start_failure(cmd, "not found or not executable"));
            }
            Ok(exit) => {
                let code = match exit {
                    EditorExit::Code(code) => code.to_string(),
                    EditorExit::Signal => "a signal".to_owned(),
                };
                return Failed(format!(
                    "`{}` exited with {code}; nothing was changed",
                    cmd.value()
                ));
            }
        }

        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) => {
                return Failed(format!(
                    "could not read the edited file back ({err}); nothing was changed"
                ));
            }
        };
        let Ok(read) = String::from_utf8(bytes) else {
            return Failed("the edited file is not UTF-8; nothing was changed".to_owned());
        };
        let edited = normalise_newlines(&read);
        if edited == normalise_newlines(&self.handed) {
            Unchanged {
                quick: elapsed < QUICK_EXIT,
            }
        } else {
            Edited(edited)
        }
    }
}

/// Writes `text` to a temp file named after `stem`, runs `cmd` on it and reads it back (D9), over
/// a [`TempEdit`].
///
/// Stdio is inherited: the caller has already given the terminal away ([`run_suspended`]). The
/// temp file is removed on every path out, a dropped future included (the [`TempEdit`] drops
/// with it), and the editor child is killed if the future is dropped (D25; on Windows only `cmd`
/// is, see [`EditorCommand::command`]). While the editor runs, the terminal's interrupt keys
/// reach the editor and not htui (`Interrupts`, private).
pub async fn run(cmd: &EditorCommand, text: &str, stem: &str) -> ExternalEditOutcome {
    let temp = match TempEdit::create(text, stem) {
        Ok(temp) => temp,
        Err(outcome) => return outcome,
    };

    // Held until the editor has returned, so Ctrl-C at a cooked-mode editor cannot end htui.
    let interrupts = Interrupts::hold();
    let started = Instant::now();
    let status = tokio::process::Command::from(cmd.command(temp.path()))
        .kill_on_drop(true)
        .status()
        .await;
    let elapsed = started.elapsed();
    drop(interrupts);
    temp.finish(cmd, status.map(EditorExit::from), elapsed)
}

/// [`run`] with the terminal given to the editor and taken back (D9, D21).
///
/// A failed `leave` re-enters once, to undo a half-leave, and answers
/// [`ExternalEditOutcome::Failed`] without spawning anything (F-O). If the future is dropped while
/// the editor runs, a guard re-enters (never while panicking: the panic hooks restore instead).
///
/// # Errors
///
/// A failed `enter`: the terminal could not be taken back, and the caller must not keep drawing
/// (F-I).
pub async fn run_suspended<S: Suspend>(
    term: &mut S,
    cmd: &EditorCommand,
    edit: &ExternalEdit,
) -> io::Result<ExternalEditOutcome> {
    if let Err(err) = term.leave() {
        // Undo a half-leave: `try_restore` may have left raw mode and failed on the screen (F-O).
        term.enter()?;
        return Ok(ExternalEditOutcome::Failed(format!(
            "could not leave the TUI: {err}"
        )));
    }
    let resume = Resume { term, armed: true };
    let outcome = run(cmd, &edit.text, &edit.stem).await;
    resume.finish()?;
    Ok(outcome)
}

/// Re-enters the TUI if [`run_suspended`]'s future is dropped while the editor runs (D21).
struct Resume<'a, S: Suspend> {
    /// The terminal that was left.
    term: &'a mut S,
    /// Whether `Drop` still owes an `enter`.
    armed: bool,
}

impl<S: Suspend> Resume<'_, S> {
    /// The normal path: disarm, then `enter` with its error reported (F-I).
    fn finish(mut self) -> io::Result<()> {
        self.armed = false;
        self.term.enter()
    }
}

impl<S: Suspend> Drop for Resume<'_, S> {
    fn drop(&mut self) {
        // A dropped future: nothing can report the error, so the best effort is to re-enter. Not
        // while panicking: both hooks and `TerminalGuard::drop` restore on that path, and
        // re-entering here would leave the alternate screen up behind the panic message.
        if self.armed && !std::thread::panicking() {
            let _ = self.term.enter();
        }
    }
}

/// Ctrl-C and Ctrl-\ (Ctrl-Break on Windows) while an editor has the terminal: htui survives them,
/// the editor still gets them. Git's `editor.c` does the same around its editor.
///
/// `leave` hands the editor a cooked terminal, so those keys signal the whole foreground process
/// group, htui included. The default disposition would end htui with no destructor run: the temp
/// file stays behind and `lib.rs`'s shutdown order for the store worker is skipped. A listener
/// replaces that default while it is held. It is a handler and not `SIG_IGN`, so the `exec`ed
/// editor starts at the default and still gets the key. Agents, verify commands and git are
/// spawned as their own group leaders (`ProcessGroup::leader`), so the key never reaches them.
///
/// On unix tokio never uninstalls a handler: after the first handoff, an interrupt that reaches
/// htui outside an editor is swallowed rather than fatal. In raw mode the keyboard cannot send one
/// (no `ISIG`); `kill -INT` from elsewhere can, and `SIGTERM` still ends htui. On Windows the
/// console default is back as soon as the listeners drop.
struct Interrupts {
    /// The listeners; their existence is the whole effect.
    #[cfg(unix)]
    _held: Vec<tokio::signal::unix::Signal>,
    /// The listeners; their existence is the whole effect.
    #[cfg(windows)]
    _held: (
        Option<tokio::signal::windows::CtrlC>,
        Option<tokio::signal::windows::CtrlBreak>,
    ),
}

impl Interrupts {
    /// Registers the listeners. One that cannot be registered is logged and skipped: the edit
    /// still runs, with that key as fatal as it was before.
    fn hold() -> Self {
        /// `result`'s listener, or `None` with a warning.
        fn listener<T>(result: io::Result<T>, which: &str) -> Option<T> {
            result
                .inspect_err(|err| {
                    tracing::warn!("could not hold {which} off htui during the edit: {err}");
                })
                .ok()
        }

        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Self {
                _held: [
                    (SignalKind::interrupt(), "SIGINT"),
                    (SignalKind::quit(), "SIGQUIT"),
                ]
                .into_iter()
                .filter_map(|(kind, which)| listener(signal(kind), which))
                .collect(),
            }
        }
        #[cfg(windows)]
        {
            use tokio::signal::windows::{ctrl_break, ctrl_c};
            Self {
                _held: (
                    listener(ctrl_c(), "Ctrl-C"),
                    listener(ctrl_break(), "Ctrl-Break"),
                ),
            }
        }
    }
}

/// Whether an exit code is the platform shell saying the editor could not start (D23): `sh`'s 127
/// (not found) and 126 (not executable), `cmd`'s 9009.
fn is_start_failure(code: i32) -> bool {
    if cfg!(windows) {
        code == 9009
    } else {
        code == 127 || code == 126
    }
}

/// The temp-file stem: `[A-Za-z0-9_-]`, anything else `_`, at most [`STEM_MAX`] chars (D25).
fn sanitise(stem: &str) -> String {
    stem.chars()
        .take(STEM_MAX)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The sentence for an editor that could not start (D23): it names `$VISUAL`/`$EDITOR`, and says
/// neither is set when `cmd` is the platform fallback (F-M).
fn start_failure(cmd: &EditorCommand, why: &str) -> String {
    if cmd.fallback {
        format!(
            "no $VISUAL or $EDITOR is set and `{}` could not start ({why})",
            cmd.value
        )
    } else {
        format!(
            "could not start `{}` ({why}) — set $VISUAL or $EDITOR",
            cmd.value
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `lookup` over fixed pairs.
    fn vars(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn visual_wins_over_editor() {
        let cmd = EditorCommand::resolve(vars(&[("VISUAL", "nvim -u NONE"), ("EDITOR", "nano")]));
        assert_eq!(cmd.value(), "nvim -u NONE");
        assert!(!cmd.fallback);

        let cmd = EditorCommand::resolve(vars(&[("EDITOR", "nano")]));
        assert_eq!(cmd.value(), "nano");
        assert!(!cmd.fallback);
    }

    #[test]
    fn blank_values_fall_through() {
        let cmd = EditorCommand::resolve(vars(&[("VISUAL", "   "), ("EDITOR", " code --wait ")]));
        assert_eq!(cmd.value(), "code --wait");
        assert!(!cmd.fallback);

        let cmd = EditorCommand::resolve(vars(&[("VISUAL", ""), ("EDITOR", " \t")]));
        assert!(cmd.fallback, "{cmd:?}");
    }

    #[cfg(unix)]
    #[test]
    fn unix_falls_back_to_vi() {
        let cmd = EditorCommand::resolve(|_| None);
        assert_eq!(
            cmd,
            EditorCommand {
                value: "vi".to_owned(),
                fallback: true
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_falls_back_to_notepad() {
        let cmd = EditorCommand::resolve(|_| None);
        assert_eq!(
            cmd,
            EditorCommand {
                value: "notepad".to_owned(),
                fallback: true
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_unix_command_passes_the_file_as_dollar_one() {
        let cmd = EditorCommand {
            value: "code --wait".to_owned(),
            fallback: false,
        };
        let file = Path::new("/tmp/htui-implement-x.md");
        let command = cmd.command(file);
        assert_eq!(command.get_program(), "sh");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "-c",
                "exec code --wait \"$1\"",
                "htui-editor",
                "/tmp/htui-implement-x.md"
            ]
        );
    }

    #[test]
    fn the_start_failure_reads_the_fallback_flag() {
        let set = EditorCommand {
            value: "hx".to_owned(),
            fallback: false,
        };
        assert_eq!(
            start_failure(&set, "not found or not executable"),
            "could not start `hx` (not found or not executable) — set $VISUAL or $EDITOR"
        );
        let unset = EditorCommand {
            value: FALLBACK.to_owned(),
            fallback: true,
        };
        assert_eq!(
            start_failure(&unset, "boom"),
            format!("no $VISUAL or $EDITOR is set and `{FALLBACK}` could not start (boom)")
        );
    }

    #[test]
    fn debug_prints_lengths_not_text() {
        let edit = ExternalEdit {
            text: "secret body".to_owned(),
            stem: "implement".to_owned(),
        };
        let shown = format!("{edit:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("text_len: 11"), "{shown}");
    }

    #[test]
    fn an_edited_outcome_debug_prints_its_length_not_its_text() {
        let shown = format!(
            "{:?}",
            ExternalEditOutcome::Edited("secret body".to_owned())
        );
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("text_len: 11"), "{shown}");
        // The other two carry no body, and print as they are.
        assert_eq!(
            format!("{:?}", ExternalEditOutcome::Unchanged { quick: true }),
            "Unchanged { quick: true }"
        );
        assert_eq!(
            format!("{:?}", ExternalEditOutcome::Failed("boom".to_owned())),
            "Failed(\"boom\")"
        );
    }

    #[test]
    fn stems_are_sanitised_and_capped() {
        assert_eq!(sanitise("implement"), "implement");
        assert_eq!(sanitise("a/b ..é-1_x"), "a_b____-1_x");
        assert_eq!(sanitise(&"x".repeat(40)).chars().count(), STEM_MAX);
    }

    #[test]
    fn strip_added_newline_drops_exactly_one_editor_newline() {
        for (handed, returned, result) in [
            ("abc", "abc\n", "abc"),
            ("abc", "abd\n", "abd"),
            ("abc", "abc\n\n", "abc\n"),
            ("abc\n", "abc\n\n", "abc\n\n"),
            ("", "\n", ""),
            ("abc", "abc", "abc"),
        ] {
            assert_eq!(
                strip_added_newline(handed, returned),
                result,
                "{handed:?} -> {returned:?}"
            );
        }
    }

    #[test]
    fn without_controls_keeps_line_breaks_and_tabs() {
        assert_eq!(without_controls("a\tb\n\u{1b}[31mc\u{7}"), "a\tb\n[31mc");
    }

    #[test]
    fn editor_exit_from_both_runners() {
        use portable_pty::ExitStatus;

        assert_eq!(
            EditorExit::from(&ExitStatus::with_exit_code(0)),
            EditorExit::Code(0)
        );
        assert_eq!(
            EditorExit::from(&ExitStatus::with_exit_code(3)),
            EditorExit::Code(3)
        );
        assert_eq!(
            EditorExit::from(&ExitStatus::with_signal("Hangup")),
            EditorExit::Signal
        );
        // A code past `i32` (a Windows NTSTATUS) saturates rather than wrapping negative.
        assert_eq!(
            EditorExit::from(&ExitStatus::with_exit_code(u32::MAX)),
            EditorExit::Code(i32::MAX)
        );

        #[cfg(unix)]
        {
            let status = |script: &str| {
                std::process::Command::new("sh")
                    .args(["-c", script])
                    .status()
                    .expect("run sh")
            };
            assert_eq!(EditorExit::from(status("exit 4")), EditorExit::Code(4));
            assert_eq!(
                EditorExit::from(status("kill -TERM $$")),
                EditorExit::Signal
            );
        }
    }

    #[test]
    fn temp_edit_create_writes_the_text_under_the_sanitised_stem() {
        let temp = TempEdit::create("hello\n", "../a/b c").expect("create the temp file");
        let path = temp.path().to_owned();
        assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
        let name = path.file_name().and_then(|name| name.to_str()).unwrap();
        assert!(name.starts_with("htui-___a_b_c-"), "{name}");
        assert!(name.ends_with(".md"), "{name}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
        drop(temp);
        assert!(!path.exists(), "{}", path.display());
    }

    /// What a [`TempEdit::finish`] row expects.
    enum Want {
        /// Exactly this outcome.
        Is(ExternalEditOutcome),
        /// `Failed`, with each of these in its sentence.
        FailedWith(&'static [&'static str]),
    }

    #[test]
    fn temp_edit_finish_maps_every_exit() {
        use ExternalEditOutcome::{Edited, Failed, Unchanged};
        use Want::{FailedWith, Is};

        /// What the "editor" leaves in the file before `finish` reads it back.
        enum File {
            /// Untouched.
            Kept,
            /// Rewritten with these bytes.
            Rewritten(&'static [u8]),
            /// Removed.
            Gone,
        }

        let cmd = EditorCommand {
            value: "hx".to_owned(),
            fallback: false,
        };
        let start_failure = if cfg!(windows) { 9009 } else { 127 };
        let quick = Duration::from_millis(10);
        let slow = Duration::from_secs(2);
        let rows: Vec<(&str, File, io::Result<EditorExit>, Duration, Want)> = vec![
            (
                "appended",
                File::Rewritten(b"hello\nmore\n"),
                Ok(EditorExit::Code(0)),
                quick,
                Is(Edited("hello\nmore\n".to_owned())),
            ),
            (
                "untouched, quick",
                File::Kept,
                Ok(EditorExit::Code(0)),
                quick,
                Is(Unchanged { quick: true }),
            ),
            (
                "untouched, slow",
                File::Kept,
                Ok(EditorExit::Code(0)),
                slow,
                Is(Unchanged { quick: false }),
            ),
            (
                "exit 3",
                File::Rewritten(b"hello\nmore\n"),
                Ok(EditorExit::Code(3)),
                slow,
                FailedWith(&["`hx` exited with 3; nothing was changed"]),
            ),
            (
                "signal",
                File::Kept,
                Ok(EditorExit::Signal),
                slow,
                FailedWith(&["`hx` exited with a signal; nothing was changed"]),
            ),
            (
                "not found",
                File::Kept,
                Ok(EditorExit::Code(start_failure)),
                quick,
                FailedWith(&["$VISUAL or $EDITOR", "not found or not executable"]),
            ),
            (
                "spawn error",
                File::Kept,
                Err(io::Error::other("boom")),
                quick,
                FailedWith(&["could not start `hx` (boom)"]),
            ),
            (
                "not utf-8",
                File::Rewritten(b"\xff\xfe"),
                Ok(EditorExit::Code(0)),
                slow,
                Is(Failed(
                    "the edited file is not UTF-8; nothing was changed".to_owned(),
                )),
            ),
            (
                "crlf rewrite",
                File::Rewritten(b"hello\r\nmore\r\n"),
                Ok(EditorExit::Code(0)),
                slow,
                Is(Edited("hello\nmore\n".to_owned())),
            ),
            (
                "crlf, same text",
                File::Rewritten(b"hello\r\n"),
                Ok(EditorExit::Code(0)),
                slow,
                Is(Unchanged { quick: false }),
            ),
            (
                "file removed",
                File::Gone,
                Ok(EditorExit::Code(0)),
                slow,
                FailedWith(&[
                    "could not read the edited file back (",
                    "nothing was changed",
                ]),
            ),
        ];
        for (name, file, exit, elapsed, want) in rows {
            let temp = TempEdit::create("hello\n", "implement").expect("create the temp file");
            let path = temp.path().to_owned();
            match file {
                File::Kept => {}
                File::Rewritten(bytes) => std::fs::write(&path, bytes).unwrap(),
                File::Gone => std::fs::remove_file(&path).unwrap(),
            }
            let outcome = temp.finish(&cmd, exit, elapsed);
            match want {
                Is(want) => assert_eq!(outcome, want, "{name}"),
                FailedWith(parts) => {
                    let Failed(message) = &outcome else {
                        panic!("{name}: expected Failed, got {outcome:?}");
                    };
                    for part in parts {
                        assert!(message.contains(part), "{name}: {message}");
                    }
                }
            }
            assert!(!path.exists(), "{name} left {}", path.display());
        }
    }

    #[test]
    fn a_temp_edit_debug_prints_lengths_not_text() {
        let temp = TempEdit::create("secret body", "implement").unwrap();
        let shown = format!("{temp:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("text_len: 11"), "{shown}");
        assert!(shown.contains("htui-implement-"), "{shown}");
    }

    #[test]
    fn editor_mode_reads_htui_editor_pane() {
        assert_eq!(EditorMode::resolve(|_| None), EditorMode::Suspend);
        for (value, mode) in [
            ("", EditorMode::Suspend),
            (" ", EditorMode::Suspend),
            ("0", EditorMode::Suspend),
            ("no", EditorMode::Suspend),
            ("false", EditorMode::Suspend),
            ("on", EditorMode::Suspend),
            ("1", EditorMode::Pane),
            ("true", EditorMode::Pane),
            ("TRUE", EditorMode::Pane),
            (" yes ", EditorMode::Pane),
        ] {
            let lookup = |key: &str| (key == "HTUI_EDITOR_PANE").then(|| value.to_owned());
            assert_eq!(EditorMode::resolve(lookup), mode, "{value:?}");
        }
        // Only `HTUI_EDITOR_PANE` is read.
        assert_eq!(PANE_VAR, "HTUI_EDITOR_PANE");
        assert_eq!(
            EditorMode::resolve(vars(&[("EDITOR", "1")])),
            EditorMode::Suspend
        );
    }

    #[test]
    fn the_pane_sentences_are_pinned() {
        assert_eq!(
            EDITOR_BUSY,
            "an editor is already open: return to it or abort it first"
        );
        assert_eq!(
            EDITOR_ABORTED,
            "the editor was aborted; nothing was changed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pty_command_matches_command_s_argv() {
        let cmd = EditorCommand {
            value: "code --wait".to_owned(),
            fallback: false,
        };
        let file = Path::new("/tmp/htui-implement-x.md");
        let command = cmd.pty_command(file);
        assert_eq!(
            *command.get_argv(),
            [
                "sh",
                "-c",
                "exec code --wait \"$1\"",
                "htui-editor",
                "/tmp/htui-implement-x.md"
            ]
        );
        // The same argv as the suspended handoff's.
        let suspended = cmd.command(file);
        assert_eq!(command.get_argv()[0], suspended.get_program());
        assert!(command.get_argv()[1..].iter().eq(suspended.get_args()));
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new("xterm-256color"))
        );
        assert_eq!(command.get_env("LINES"), None);
        assert_eq!(command.get_env("COLUMNS"), None);
        assert_eq!(
            command.get_cwd().map(std::ffi::OsString::as_os_str),
            Some(std::env::current_dir().unwrap().as_os_str())
        );
    }

    /// The `LINES`/`COLUMNS` half of test 7 with the variables really inherited: a test cannot
    /// plant them in its own process (`set_var` is `unsafe`), so it re-runs itself, alone, as a
    /// child test binary that has them, and the child asserts `pty_command` drops them.
    #[cfg(unix)]
    #[test]
    fn pty_command_drops_inherited_lines_and_columns() {
        const CHILD: &str = "HTUI_EDITOR_TEST_ENV_CHILD";
        const NAME: &str = "editor::tests::pty_command_drops_inherited_lines_and_columns";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .args([NAME, "--exact", "--test-threads=1"])
                .env(CHILD, "1")
                .env("LINES", "7")
                .env("COLUMNS", "9")
                .output()
                .expect("re-run the test binary");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "child failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            // A filter that matched nothing would also succeed.
            assert!(stdout.contains("1 passed"), "child ran no test:\n{stdout}");
            return;
        }
        // The child: the variables are inherited, so the builder's base environment has them.
        assert_eq!(std::env::var_os("LINES"), Some("7".into()));
        assert_eq!(std::env::var_os("COLUMNS"), Some("9".into()));
        let cmd = EditorCommand {
            value: "vi".to_owned(),
            fallback: false,
        };
        let command = cmd.pty_command(Path::new("/tmp/htui-implement-x.md"));
        assert_eq!(command.get_env("LINES"), None);
        assert_eq!(command.get_env("COLUMNS"), None);
    }

    #[cfg(windows)]
    #[test]
    fn pty_command_names_the_file_in_its_directory() {
        let cmd = EditorCommand {
            value: "code --wait".to_owned(),
            fallback: false,
        };
        let command = cmd.pty_command(Path::new(r"C:\Temp\htui-implement-x.md"));
        assert_eq!(
            *command.get_argv(),
            ["cmd", "/S", "/C", "code --wait htui-implement-x.md"]
        );
        assert_eq!(
            command.get_cwd().map(std::ffi::OsString::as_os_str),
            Some(std::ffi::OsStr::new(r"C:\Temp"))
        );
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new("xterm-256color"))
        );
    }

    /// Fake editors: `#!/bin/sh` scripts in a temp dir. No test launches a real editor.
    #[cfg(unix)]
    mod scripts {
        use std::os::unix::fs::PermissionsExt as _;
        use std::path::{Path, PathBuf};

        use tempfile::TempDir;

        use super::super::*;

        /// Writes `body` as an executable script and returns the command that runs it.
        ///
        /// `std::fs::write` closes the handle at once, so a fork elsewhere cannot hold it open
        /// into our `exec` (`ETXTBSY`, R-13).
        pub(super) fn script(dir: &TempDir, name: &str, body: &str) -> EditorCommand {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod the script");
            EditorCommand {
                value: format!("'{}'", path.display()),
                fallback: false,
            }
        }

        /// A script that records the path it was handed in `side`, then runs `then`.
        pub(super) fn recording(dir: &TempDir, name: &str, then: &str) -> (EditorCommand, PathBuf) {
            let side = dir.path().join(format!("{name}.path"));
            let body = format!("printf '%s' \"$1\" > '{}'\n{then}", side.display());
            (script(dir, name, &body), side)
        }

        /// The path a [`recording`] script was handed.
        pub(super) fn recorded(side: &Path) -> PathBuf {
            PathBuf::from(std::fs::read_to_string(side).expect("the script ran"))
        }

        #[tokio::test]
        async fn a_fake_editor_that_appends_returns_edited() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "append", "printf 'more\\n' >> \"$1\"");
            let outcome = run(&cmd, "hello\n", "implement").await;
            assert_eq!(
                outcome,
                ExternalEditOutcome::Edited("hello\nmore\n".to_owned())
            );
        }

        #[tokio::test]
        async fn a_fake_editor_that_does_nothing_returns_unchanged() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "noop", "exit 0");
            let outcome = run(&cmd, "hello\n", "implement").await;
            assert_eq!(outcome, ExternalEditOutcome::Unchanged { quick: true });
        }

        #[tokio::test]
        async fn a_crlf_writing_editor_is_normalised() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "crlf", "printf 'a\\r\\nb\\r\\n' > \"$1\"");
            let outcome = run(&cmd, "a\nb\nc\n", "implement").await;
            assert_eq!(outcome, ExternalEditOutcome::Edited("a\nb\n".to_owned()));

            // Rewriting the same text with CRLF endings is no change at all (D35).
            let cmd = script(&dir, "same", "printf 'a\\r\\nb\\r\\n' > \"$1\"");
            let outcome = run(&cmd, "a\nb\n", "implement").await;
            assert!(
                matches!(outcome, ExternalEditOutcome::Unchanged { .. }),
                "{outcome:?}"
            );
        }

        #[tokio::test]
        async fn a_non_zero_exit_is_failed_and_names_the_code() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "three", "printf 'x' >> \"$1\"\nexit 3");
            let outcome = run(&cmd, "hello\n", "implement").await;
            let ExternalEditOutcome::Failed(message) = outcome else {
                panic!("expected Failed, got {outcome:?}");
            };
            assert!(message.contains("exited with 3"), "{message}");
            assert!(message.contains("nothing was changed"), "{message}");
        }

        #[tokio::test]
        async fn a_missing_program_is_failed_and_names_visual_and_editor() {
            // `sh` itself starts and answers 127 (F-D): the start-failure sentence, not the exit.
            let cmd = EditorCommand {
                value: "/nonexistent/ed".to_owned(),
                fallback: false,
            };
            let outcome = run(&cmd, "hello\n", "implement").await;
            let ExternalEditOutcome::Failed(message) = outcome else {
                panic!("expected Failed, got {outcome:?}");
            };
            assert!(message.contains("$VISUAL or $EDITOR"), "{message}");
            assert!(message.contains("`/nonexistent/ed`"), "{message}");
            assert!(message.contains("not found or not executable"), "{message}");
        }

        #[tokio::test]
        async fn a_non_utf8_file_is_failed() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "latin1", "printf '\\377\\376' > \"$1\"");
            let outcome = run(&cmd, "hello\n", "implement").await;
            assert_eq!(
                outcome,
                ExternalEditOutcome::Failed(
                    "the edited file is not UTF-8; nothing was changed".to_owned()
                )
            );
        }

        #[tokio::test]
        async fn the_temp_file_is_gone_after_every_outcome() {
            let dir = TempDir::new().unwrap();
            for (name, then) in [
                ("edited", "printf 'more\\n' >> \"$1\""),
                ("unchanged", "exit 0"),
                ("failed", "exit 3"),
            ] {
                let (cmd, side) = recording(&dir, name, then);
                let outcome = run(&cmd, "hello\n", "implement").await;
                let path = recorded(&side);
                assert!(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(
                            |name| name.starts_with("htui-implement-") && name.ends_with(".md")
                        ),
                    "{name}: {}",
                    path.display()
                );
                assert!(
                    !path.exists(),
                    "{name} ({outcome:?}) left {}",
                    path.display()
                );
            }
        }

        #[tokio::test]
        async fn a_stem_with_a_path_separator_stays_in_the_temp_dir() {
            let dir = TempDir::new().unwrap();
            let (cmd, side) = recording(&dir, "stem", "exit 0");
            let outcome = run(&cmd, "hello\n", "../a/b c").await;
            assert!(
                matches!(outcome, ExternalEditOutcome::Unchanged { .. }),
                "{outcome:?}"
            );
            let path = recorded(&side);
            assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
            let name = path.file_name().and_then(|name| name.to_str()).unwrap();
            assert!(name.starts_with("htui-___a_b_c-"), "{name}");
        }

        /// Ctrl-C or Ctrl-\ at an editor that keeps the tty cooked (`code --wait`, `ed`) signals
        /// the whole foreground group, htui included. The script stands in for the terminal by
        /// signalling this process by pid: never the group, which holds `cargo` too.
        #[tokio::test]
        async fn an_interrupt_while_the_editor_runs_does_not_kill_htui() {
            let dir = TempDir::new().unwrap();
            let me = std::process::id();
            let cmd = script(
                &dir,
                "interrupt",
                &format!("kill -INT {me}\nkill -QUIT {me}\nexit 0"),
            );
            let outcome = run(&cmd, "hello\n", "implement").await;
            assert!(
                matches!(outcome, ExternalEditOutcome::Unchanged { .. }),
                "{outcome:?}"
            );
            // Still here: the default disposition would have ended this process above.
            let outcome = run(&cmd, "hello\n", "implement").await;
            assert!(
                matches!(outcome, ExternalEditOutcome::Unchanged { .. }),
                "{outcome:?}"
            );
        }

        /// An editor that traps Ctrl-C (`ed`, `ex`) keeps running and saves. The terminal signals
        /// the whole group, so the editor's parent gets the key too: were that a shell waiting on
        /// the editor (`sh -c` without `exec`), the shell would die, and htui would report a
        /// failure and remove the file the editor is about to save. `$PPID` is that parent.
        #[tokio::test]
        async fn an_editor_that_survives_the_interrupt_keeps_its_edit() {
            let dir = TempDir::new().unwrap();
            let cmd = script(
                &dir,
                "trapping",
                "trap '' INT\nkill -INT $PPID\nsleep 0.2\nprintf 'more\\n' >> \"$1\"",
            );
            let outcome = run(&cmd, "hello\n", "implement").await;
            assert_eq!(
                outcome,
                ExternalEditOutcome::Edited("hello\nmore\n".to_owned())
            );
        }

        /// htui catches the interrupt; it does not ignore it. A caught signal is back at its
        /// default in an `exec`ed child, so the editor still gets Ctrl-C (a `SIG_IGN` would be
        /// inherited, and the script below would live on to `exit 0` and answer `Unchanged`).
        #[tokio::test]
        async fn the_editor_still_gets_the_interrupt() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "self", "kill -INT $$\nexit 0");
            let outcome = run(&cmd, "hello\n", "implement").await;
            let ExternalEditOutcome::Failed(message) = outcome else {
                panic!("expected Failed, got {outcome:?}");
            };
            // The editor is htui's own child (`exec`), so its death reads as a signal, not 130.
            assert!(message.contains("exited with a signal"), "{message}");
        }
    }

    /// Suspension over a recording fake: the real terminal is never touched.
    #[cfg(unix)]
    mod suspension {
        use tempfile::TempDir;

        use super::super::*;
        use super::scripts::script;

        #[derive(Debug, Default)]
        struct FakeTerminal {
            calls: Vec<&'static str>,
            fail_leave: bool,
            fail_enter: bool,
        }

        impl Suspend for FakeTerminal {
            fn leave(&mut self) -> io::Result<()> {
                self.calls.push("leave");
                if self.fail_leave {
                    return Err(io::Error::other("leave refused"));
                }
                Ok(())
            }
            fn enter(&mut self) -> io::Result<()> {
                self.calls.push("enter");
                if self.fail_enter {
                    return Err(io::Error::other("enter refused"));
                }
                Ok(())
            }
        }

        fn edit() -> ExternalEdit {
            ExternalEdit {
                text: "hello\n".to_owned(),
                stem: "implement".to_owned(),
            }
        }

        #[tokio::test]
        async fn leave_then_enter_on_success() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "append", "printf 'more\\n' >> \"$1\"");
            let mut term = FakeTerminal::default();
            let outcome = run_suspended(&mut term, &cmd, &edit()).await.unwrap();
            assert_eq!(
                outcome,
                ExternalEditOutcome::Edited("hello\nmore\n".to_owned())
            );
            assert_eq!(term.calls, ["leave", "enter"]);
        }

        #[tokio::test]
        async fn enter_runs_after_a_failed_editor() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "three", "exit 3");
            let mut term = FakeTerminal::default();
            let outcome = run_suspended(&mut term, &cmd, &edit()).await.unwrap();
            assert!(
                matches!(outcome, ExternalEditOutcome::Failed(_)),
                "{outcome:?}"
            );
            assert_eq!(term.calls, ["leave", "enter"]);
        }

        #[tokio::test]
        async fn a_failed_leave_spawns_nothing_and_enters_once() {
            let dir = TempDir::new().unwrap();
            let marker = dir.path().join("spawned");
            let cmd = script(&dir, "marker", &format!("touch '{}'", marker.display()));
            let mut term = FakeTerminal {
                fail_leave: true,
                ..FakeTerminal::default()
            };
            let outcome = run_suspended(&mut term, &cmd, &edit()).await.unwrap();
            let ExternalEditOutcome::Failed(message) = outcome else {
                panic!("expected Failed, got {outcome:?}");
            };
            assert!(message.contains("could not leave the TUI"), "{message}");
            assert!(message.contains("leave refused"), "{message}");
            assert_eq!(
                term.calls,
                ["leave", "enter"],
                "a half-leave is undone once"
            );
            assert!(!marker.exists(), "nothing was spawned");
        }

        #[tokio::test]
        async fn a_failed_enter_is_an_io_error() {
            let dir = TempDir::new().unwrap();
            let cmd = script(&dir, "noop", "exit 0");
            let mut term = FakeTerminal {
                fail_enter: true,
                ..FakeTerminal::default()
            };
            let err = run_suspended(&mut term, &cmd, &edit())
                .await
                .expect_err("a terminal that cannot be taken back is an error");
            assert_eq!(err.to_string(), "enter refused");
            assert_eq!(
                term.calls,
                ["leave", "enter"],
                "the guard is disarmed: no second enter on drop"
            );
        }

        #[tokio::test]
        async fn a_dropped_future_still_enters() {
            let dir = TempDir::new().unwrap();
            // `exec`, so the child `kill_on_drop` reaps is the `sleep` itself.
            let cmd = script(&dir, "slow", "exec sleep 5");
            let mut term = FakeTerminal::default();
            let started = Instant::now();
            let result = tokio::time::timeout(
                Duration::from_millis(50),
                run_suspended(&mut term, &cmd, &edit()),
            )
            .await;
            assert!(result.is_err(), "the editor was still running");
            assert!(started.elapsed() < Duration::from_secs(4));
            assert_eq!(term.calls, ["leave", "enter"]);
        }

        /// `kill_on_drop` reaches the editor itself, not a shell between htui and it. The script
        /// does not `exec` its `sleep`: the script is the editor, and its pid is what must die.
        #[tokio::test]
        async fn a_dropped_future_kills_the_editor() {
            let dir = TempDir::new().unwrap();
            let pid_file = dir.path().join("editor.pid");
            let cmd = script(
                &dir,
                "slow",
                &format!("printf '%s' $$ > '{}'\nsleep 5", pid_file.display()),
            );
            let mut term = FakeTerminal::default();
            let edit = edit();
            let mut running = Box::pin(run_suspended(&mut term, &cmd, &edit));
            let pid = loop {
                tokio::select! {
                    outcome = &mut running => panic!("the editor returned: {outcome:?}"),
                    () = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
                if let Some(pid) = std::fs::read_to_string(&pid_file)
                    .ok()
                    .filter(|pid| !pid.is_empty())
                {
                    break pid;
                }
            };
            drop(running);
            assert_eq!(term.calls, ["leave", "enter"]);

            // SIGKILL is sent on drop; tokio reaps in the background, so a zombie counts as gone.
            let started = Instant::now();
            while alive(&pid) {
                assert!(
                    started.elapsed() < Duration::from_secs(3),
                    "the editor (pid {pid}) outlived the dropped future"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        /// Whether `pid` is a live process: `ps` knows it and it is not a zombie.
        fn alive(pid: &str) -> bool {
            let out = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", pid])
                .output()
                .expect("run ps");
            let stat = String::from_utf8_lossy(&out.stdout);
            let stat = stat.trim();
            !stat.is_empty() && !stat.starts_with('Z')
        }
    }
}
