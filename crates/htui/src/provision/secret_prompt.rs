//! The sudo password, read with no echo (MOD-45 D302): crossterm raw mode, key events up to
//! `Enter`, into a pre-sized `Zeroizing<String>`. crossterm falls back to `/dev/tty` when stdin is
//! not a terminal, so `--dsn-stdin` and a password prompt work together. A guard's `Drop` restores
//! the terminal. The prompt line itself is written to `/dev/tty` too when it opens, else to stderr
//! ([`PasswordPrompt::prompt_writer`]).

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::future::BoxFuture;
use zeroize::Zeroizing;

/// Longer input is refused, so the buffer never reallocates (residue: MOD-41 R-5).
pub const PASSWORD_MAX: usize = 1024;

/// Why there is no password. Neither variant carries input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptError {
    /// `Esc`, `Ctrl-C`, or longer than [`PASSWORD_MAX`].
    Aborted,
    /// No `/dev/tty`: raw mode could not be entered.
    NoTerminal,
}

/// Reads one password (E-6: a boxed future, so `&dyn PasswordPrompt` is formable).
pub trait PasswordPrompt: core::fmt::Debug + Send + Sync {
    /// The caller has already printed the prompt.
    fn ask(&self) -> BoxFuture<'_, Result<Zeroizing<String>, PromptError>>;

    /// Where the caller prints the prompt and the newline after it: the terminal [`Self::ask`]
    /// reads from, so the prompt is seen even when stderr is redirected (review finding 10).
    /// `None`, the default, means the caller's stderr.
    fn prompt_writer(&self) -> Option<Box<dyn std::io::Write + Send>> {
        None
    }
}

/// The terminal one. Runs `crossterm::event::read` on a thread of its own and answers through a
/// oneshot, like `worker_cmd::read_dsn_apart`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TtyPrompt;

impl PasswordPrompt for TtyPrompt {
    fn ask(&self) -> BoxFuture<'_, Result<Zeroizing<String>, PromptError>> {
        Box::pin(async {
            let (answer, answered) = tokio::sync::oneshot::channel();
            std::thread::Builder::new()
                .name("htui-provision-password".to_owned())
                .spawn(move || drop(answer.send(read_password())))
                .map_err(|_| PromptError::NoTerminal)?;
            answered.await.unwrap_or(Err(PromptError::Aborted))
        })
    }

    /// `/dev/tty` when it opens for writing; `None` (stderr) otherwise.
    fn prompt_writer(&self) -> Option<Box<dyn std::io::Write + Send>> {
        std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/tty")
            .ok()
            .map(|tty| Box::new(tty) as Box<dyn std::io::Write + Send>)
    }
}

/// Leaves raw mode when dropped, on every path out of [`read_password`].
#[derive(Debug)]
struct RawMode;

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// The blocking read: raw mode, then key events until [`apply_key`] ends it.
fn read_password() -> Result<Zeroizing<String>, PromptError> {
    crossterm::terminal::enable_raw_mode().map_err(|_| PromptError::NoTerminal)?;
    let _raw = RawMode;
    let mut buffer = Zeroizing::new(String::with_capacity(PASSWORD_MAX));
    loop {
        let event = crossterm::event::read().map_err(|_| PromptError::NoTerminal)?;
        let Event::Key(key) = event else { continue };
        match apply_key(&mut buffer, &key) {
            KeyStep::More => {}
            KeyStep::Done => return Ok(buffer),
            KeyStep::Abort => return Err(PromptError::Aborted),
        }
    }
}

/// One key's effect, pure so it is testable without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStep {
    /// Keep reading.
    More,
    /// `Enter`.
    Done,
    /// `Esc`, `Ctrl-C`, or the cap.
    Abort,
}

/// Applies `key` (press events only; release and repeat are `More`): a character typed with at most
/// `Shift` pushes, Backspace pops, `Enter` is `Done`, `Esc` is `Abort`. Raw mode delivers control
/// bytes as `Ctrl`+letter, which are read like sudo's own prompt: `Ctrl-H` erases, `Ctrl-U` kills
/// the line, `Ctrl-J` (a raw newline) ends it, `Ctrl-C` and `Ctrl-D` abort; any other `Ctrl`- or
/// `Alt`-modified key is ignored, never typed.
#[must_use]
pub fn apply_key(buffer: &mut Zeroizing<String>, key: &KeyEvent) -> KeyStep {
    if key.kind != KeyEventKind::Press {
        return KeyStep::More;
    }
    let typed = KeyModifiers::SHIFT.contains(key.modifiers);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Enter => KeyStep::Done,
        KeyCode::Esc => KeyStep::Abort,
        KeyCode::Char('c' | 'd') if ctrl => KeyStep::Abort,
        KeyCode::Char('j') if ctrl => KeyStep::Done,
        KeyCode::Char('h') if ctrl => {
            buffer.pop();
            KeyStep::More
        }
        KeyCode::Char('u') if ctrl => {
            // Wipes the bytes in place; the capacity stays.
            zeroize::Zeroize::zeroize(&mut **buffer);
            KeyStep::More
        }
        KeyCode::Char(c) if typed => {
            if buffer.len() + c.len_utf8() > PASSWORD_MAX {
                return KeyStep::Abort;
            }
            buffer.push(c);
            KeyStep::More
        }
        KeyCode::Backspace => {
            buffer.pop();
            KeyStep::More
        }
        _ => KeyStep::More,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn buffer() -> Zeroizing<String> {
        Zeroizing::new(String::with_capacity(PASSWORD_MAX))
    }

    #[test]
    fn apply_key_builds_and_ends_the_password() {
        let mut typed = buffer();
        let capacity = typed.capacity();
        for c in ['p', 'w', 'x'] {
            assert_eq!(
                apply_key(&mut typed, &press(KeyCode::Char(c))),
                KeyStep::More
            );
        }
        assert_eq!(
            apply_key(&mut typed, &press(KeyCode::Backspace)),
            KeyStep::More
        );
        assert_eq!(
            apply_key(
                &mut typed,
                &KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT)
            ),
            KeyStep::More
        );
        assert_eq!(apply_key(&mut typed, &press(KeyCode::Enter)), KeyStep::Done);
        assert_eq!(typed.as_str(), "pwD");
        assert_eq!(typed.capacity(), capacity, "the buffer never reallocated");
    }

    #[test]
    fn apply_key_aborts_on_escape_ctrl_c_and_the_cap() {
        let mut typed = buffer();
        assert_eq!(apply_key(&mut typed, &press(KeyCode::Esc)), KeyStep::Abort);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(apply_key(&mut typed, &ctrl_c), KeyStep::Abort);
        assert!(typed.is_empty(), "Ctrl-C pushes nothing");

        let mut full = buffer();
        let capacity = full.capacity();
        for _ in 0..PASSWORD_MAX {
            assert_eq!(
                apply_key(&mut full, &press(KeyCode::Char('a'))),
                KeyStep::More
            );
        }
        assert_eq!(
            apply_key(&mut full, &press(KeyCode::Char('a'))),
            KeyStep::Abort
        );
        assert_eq!(full.len(), PASSWORD_MAX);
        assert_eq!(
            full.capacity(),
            capacity,
            "the cap keeps the buffer in place"
        );
    }

    /// Raw mode turns control bytes into `Ctrl`+letter (crossterm 0.29): `^H` erases, `^U` kills
    /// the line, `^D` and `^J` end it like sudo's own prompt, and no other modified key is typed.
    #[test]
    fn apply_key_never_types_a_ctrl_or_alt_key() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let mut typed = buffer();
        for c in ['p', 'w', 'x'] {
            assert_eq!(
                apply_key(&mut typed, &press(KeyCode::Char(c))),
                KeyStep::More
            );
        }
        assert_eq!(apply_key(&mut typed, &ctrl('h')), KeyStep::More);
        assert_eq!(typed.as_str(), "pw", "Ctrl-H erases like Backspace");
        for c in ['w', 'z', 'a', '4', ' '] {
            assert_eq!(apply_key(&mut typed, &ctrl(c)), KeyStep::More);
        }
        let alt = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(apply_key(&mut typed, &alt), KeyStep::More);
        let ctrl_shift = KeyEvent::new(
            KeyCode::Char('W'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(apply_key(&mut typed, &ctrl_shift), KeyStep::More);
        assert_eq!(typed.as_str(), "pw", "no modified key is typed");

        let capacity = typed.capacity();
        assert_eq!(apply_key(&mut typed, &ctrl('u')), KeyStep::More);
        assert!(typed.is_empty(), "Ctrl-U kills the line");
        assert_eq!(typed.capacity(), capacity, "in place");
        assert_eq!(
            apply_key(&mut typed, &press(KeyCode::Char('q'))),
            KeyStep::More
        );
        assert_eq!(apply_key(&mut typed, &ctrl('j')), KeyStep::Done);
        assert_eq!(typed.as_str(), "q", "Ctrl-J (a raw newline) ends it");

        let mut typed = buffer();
        assert_eq!(apply_key(&mut typed, &ctrl('d')), KeyStep::Abort);
    }

    #[test]
    fn apply_key_ignores_releases() {
        let mut typed = buffer();
        let mut release = press(KeyCode::Char('a'));
        release.kind = KeyEventKind::Release;
        assert_eq!(apply_key(&mut typed, &release), KeyStep::More);
        let mut enter = press(KeyCode::Enter);
        enter.kind = KeyEventKind::Release;
        assert_eq!(apply_key(&mut typed, &enter), KeyStep::More);
        let mut repeat = press(KeyCode::Char('b'));
        repeat.kind = KeyEventKind::Repeat;
        assert_eq!(apply_key(&mut typed, &repeat), KeyStep::More);
        assert!(typed.is_empty());
    }
}
