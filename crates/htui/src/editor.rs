//! The `$EDITOR` handoff (MOD-9 D8, D9): resolve the command, write a temp file, run the editor
//! with the TUI suspended, read the file back.
//!
//! Nothing here knows which view asked. A tab emits `Action::EditExternally` with an
//! [`ExternalEdit`]; the event loop takes it, calls [`run_suspended`] with the real terminal, and
//! hands the [`ExternalEditOutcome`] back to the tab (MOD-9 D10).

use std::io;
use std::path::Path;
use std::time::Duration;

/// Below this, an unchanged return is reported as "quick" (MOD-9 blueprint D24): the shape of a
/// GUI editor started without `--wait`, which returns at once and leaves the file untouched.
pub const QUICK_EXIT: Duration = Duration::from_secs(1);

/// The longest temp-file stem [`run`] keeps (blueprint D25), in chars.
const STEM_MAX: usize = 32;

/// The editor used when neither `$VISUAL` nor `$EDITOR` names one.
#[cfg(not(windows))]
const FALLBACK: &str = "vi";
/// The editor used when neither `$VISUAL` nor `$EDITOR` names one.
#[cfg(windows)]
const FALLBACK: &str = "notepad";

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
        let _ = lookup;
        todo!()
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
    /// Unix: `sh -c "<value> \"$1\"" htui-editor <file>`, so the shell parses `value` as the
    /// user's own shell would (git runs `$EDITOR` the same way) and the file is a positional
    /// argument, never spliced into the script. Windows: `cmd /S /C ""<value>" "<file>""` as one raw
    /// argument; `/S` makes `cmd` strip exactly the outer pair of quotes.
    #[must_use]
    pub fn command(&self, file: &Path) -> std::process::Command {
        let _ = file;
        todo!()
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

/// What came back (D9, D24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalEditOutcome {
    /// The file changed; the normalised text
    /// ([`normalise_newlines`](htui_core::prompt::render::normalise_newlines)).
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

/// Writes `text` to a temp file named after `stem`, runs `cmd` on it and reads it back (D9).
///
/// Stdio is inherited: the caller has already given the terminal away ([`run_suspended`]). The
/// temp file is removed on every path out, a dropped future included, and the editor child is
/// killed if the future is dropped (D25).
pub async fn run(cmd: &EditorCommand, text: &str, stem: &str) -> ExternalEditOutcome {
    let _ = (cmd, text, stem);
    todo!()
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
    let _ = (term, cmd, edit);
    todo!()
}

/// The temp-file stem: `[A-Za-z0-9_-]`, anything else `_`, at most [`STEM_MAX`] chars (D25).
fn sanitise(stem: &str) -> String {
    let _ = stem;
    todo!()
}

/// The sentence for an editor that could not start (D23): it names `$VISUAL`/`$EDITOR`, and says
/// neither is set when `cmd` is the platform fallback (F-M).
fn start_failure(cmd: &EditorCommand, why: &str) -> String {
    let _ = (cmd, why);
    todo!()
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
                "code --wait \"$1\"",
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
    fn stems_are_sanitised_and_capped() {
        assert_eq!(sanitise("implement"), "implement");
        assert_eq!(sanitise("a/b ..é-1_x"), "a_b____-1_x");
        assert_eq!(sanitise(&"x".repeat(40)).chars().count(), STEM_MAX);
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
            let started = std::time::Instant::now();
            let result = tokio::time::timeout(
                Duration::from_millis(50),
                run_suspended(&mut term, &cmd, &edit()),
            )
            .await;
            assert!(result.is_err(), "the editor was still running");
            assert!(started.elapsed() < Duration::from_secs(4));
            assert_eq!(term.calls, ["leave", "enter"]);
        }
    }
}
