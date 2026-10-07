//! The in-pane editor's process side: PTY, threads, VT screen and key encoder (MOD-57 M1, plan P7).
//!
//! What lives here:
//!
//! - [`PtyChild`]: a child on a pseudo-terminal and three threads per pane. The **reader**
//!   (`htui-pane-read`) sends what the child draws as [`PaneEvent::Output`]; the **writer**
//!   (`htui-pane-write`) writes queued input, so a paste into a stalled child never blocks the
//!   caller (PD-2); the **wait** thread (`htui-pane-wait`) owns the child, polls it, and sends
//!   [`PaneEvent::Exited`] once it is reaped.
//! - The kill path (B5): [`PaneChild::kill`], or dropping the [`PtyChild`], asks the wait thread,
//!   which runs portable-pty's escalating kill on the child it owns (SIGHUP, a 200 ms grace, then
//!   SIGKILL; `TerminateProcess` on Windows) and reaps it. No pid-based signal is sent from the
//!   caller, so a reaped pid that the system reuses is never hit.
//! - [`PaneScreen`]: the VT screen a pane's output is parsed into (`vt100`), and the replies the
//!   child is owed for its terminal queries (DSR, DA1).
//! - [`encode_key`] and [`encode_paste`]: crossterm's keys and pastes as the bytes a legacy xterm
//!   sends (no kitty protocol, PRD Q4).
//! - [`PaneId`], [`PaneSize`], [`PaneEvent`]: what the shell and the pane's threads exchange.
//!
//! Nothing here blocks the caller: `write` queues, `resize` is one ioctl, `kill` sends on a
//! channel (R-NF-3). Bytes and screen contents are never logged or `Debug`ged: only lengths and
//! sizes.
//!
//! [`PtyChild`]: crate::editor::pane::PtyChild
//! [`PaneEvent::Output`]: crate::editor::pane::PaneEvent::Output
//! [`PaneEvent::Exited`]: crate::editor::pane::PaneEvent::Exited
//! [`PaneChild::kill`]: crate::editor::pane::PaneChild::kill
//! [`PaneScreen`]: crate::editor::pane::PaneScreen
//! [`encode_key`]: crate::editor::pane::encode_key
//! [`encode_paste`]: crate::editor::pane::encode_paste
//! [`PaneId`]: crate::editor::pane::PaneId
//! [`PaneSize`]: crate::editor::pane::PaneSize
//! [`PaneEvent`]: crate::editor::pane::PaneEvent

use std::fmt;
use std::io::{self, Read as _, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{Child, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc::UnboundedSender;
use zeroize::Zeroizing;

/// How often the wait thread polls the child while no kill is asked for.
const POLL: Duration = Duration::from_millis(25);

/// The reader thread's buffer: one `Output` carries at most this many bytes.
const READ_CHUNK: usize = 8192;

/// One in-pane editor's identity. Every event carries it, so the shell drops events of an editor
/// it has already finished or aborted (MOD-57 B4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(u64);

impl PaneId {
    /// A fresh id: a process-wide `AtomicU64` counter.
    #[must_use]
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A pane's size in cells. Never 0 in either dimension (`vt100` and the kernel both dislike it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSize {
    /// Rows.
    pub rows: u16,
    /// Columns.
    pub cols: u16,
}

impl PaneSize {
    /// Before anything was drawn: 24x80.
    pub const DEFAULT: Self = Self { rows: 24, cols: 80 };

    /// `rows`/`cols`, each at least 1.
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows: rows.max(1),
            cols: cols.max(1),
        }
    }

    /// The same size, floored again: the fields are public, so a literal can hold a 0.
    fn floored(self) -> Self {
        Self::new(self.rows, self.cols)
    }

    /// As portable-pty wants it (no pixel size).
    fn pty(self) -> PtySize {
        let Self { rows, cols } = self.floored();
        PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// What a pane's threads tell the shell. `Debug` prints lengths, never bytes.
pub enum PaneEvent {
    /// The child drew: raw bytes from the PTY, in order.
    Output {
        /// The pane.
        id: PaneId,
        /// What was read (at most 8 KiB).
        bytes: Vec<u8>,
    },
    /// The child ended and was reaped.
    Exited {
        /// The pane.
        id: PaneId,
        /// Its status, or why waiting failed.
        status: io::Result<portable_pty::ExitStatus>,
        /// Since the spawn, measured by the wait thread.
        elapsed: Duration,
    },
}

impl PaneEvent {
    /// The pane the event is about.
    #[must_use]
    pub fn id(&self) -> PaneId {
        match self {
            Self::Output { id, .. } | Self::Exited { id, .. } => *id,
        }
    }
}

impl fmt::Debug for PaneEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Output { id, bytes } => f
                .debug_struct("Output")
                .field("id", id)
                .field("len", &bytes.len())
                .finish(),
            Self::Exited {
                id,
                status,
                elapsed,
            } => f
                .debug_struct("Exited")
                .field("id", id)
                .field("status", status)
                .field("elapsed", elapsed)
                .finish(),
        }
    }
}

/// The process side of an in-pane editor (MOD-57 B2). [`PtyChild`] is the real one; `App` tests
/// use a recording fake. No method blocks.
pub trait PaneChild: fmt::Debug {
    /// Queues `bytes` for the child's input.
    fn write(&mut self, bytes: &[u8]);
    /// Tells the kernel, and so the child (`SIGWINCH`), the new size.
    fn resize(&mut self, size: PaneSize);
    /// Asks for the child to end: SIGHUP, then SIGKILL after a grace (B5). Idempotent.
    fn kill(&mut self);
}

/// A child on a pseudo-terminal, with its reader, writer and wait threads. Dropping it kills the
/// child. `Debug` prints the pid only.
pub struct PtyChild {
    /// Kept for `resize`. Dropped after the kill request (`Drop` runs before the fields drop).
    master: Box<dyn MasterPty + Send>,
    /// The writer thread's queue; dropping it ends the thread, which drops the PTY writer then
    /// (F-10: that writes a newline and `VEOF`, so it must not happen to a live editor first).
    input: mpsc::Sender<Zeroizing<Vec<u8>>>,
    /// The wait thread's kill request; `None` once sent.
    kill: Option<mpsc::Sender<()>>,
    /// For `Debug` and tests.
    pid: Option<u32>,
}

impl PtyChild {
    /// Opens a PTY of `size`, spawns `command` on it, drops the slave, starts the threads.
    ///
    /// The caller never `Debug`s `command`: it carries the whole environment.
    ///
    /// # Errors
    /// The PTY could not be opened, the command could not be spawned, or a thread could not be
    /// started (the child, if spawned, is then killed by the wait thread or, if that thread never
    /// started, by a SIGHUP taken before it).
    pub fn spawn(
        command: portable_pty::CommandBuilder,
        size: PaneSize,
        id: PaneId,
        events: UnboundedSender<PaneEvent>,
    ) -> io::Result<Self> {
        let pair = native_pty_system()
            .openpty(size.pty())
            .map_err(|err| io::Error::other(err.to_string()))?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|err| io::Error::other(err.to_string()))?;
        // At once: while htui holds the slave, the reader never sees EOF.
        drop(pair.slave);
        let master = pair.master;
        let pid = child.process_id();
        let started = Instant::now();

        // The wait thread first: from here on, it owns the child and every kill goes through it.
        let (kill_tx, kill_rx) = mpsc::channel::<()>();
        // Only for the one path where the thread cannot start: `Builder::spawn` then drops the
        // closure, and the child with it, unkilled. The child is not reaped yet, so its pid is
        // still its own.
        let mut fallback = child.clone_killer();
        let wait_events = events.clone();
        let waiting = thread::Builder::new()
            .name("htui-pane-wait".to_owned())
            .spawn(move || wait_for(child, &kill_rx, id, started, &wait_events));
        if let Err(err) = waiting {
            let _ = fallback.kill();
            return Err(err);
        }
        drop(fallback);

        // Any error below drops `kill_tx`, and the wait thread kills the child.
        let mut reader = master
            .try_clone_reader()
            .map_err(|err| io::Error::other(err.to_string()))?;
        let mut writer = master
            .take_writer()
            .map_err(|err| io::Error::other(err.to_string()))?;

        thread::Builder::new()
            .name("htui-pane-read".to_owned())
            .spawn(move || {
                let mut buf = [0u8; READ_CHUNK];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let output = PaneEvent::Output {
                                id,
                                bytes: buf[..n].to_vec(),
                            };
                            if events.send(output).is_err() {
                                break;
                            }
                        }
                        Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                        // EIO once the child is gone (Linux), or a real failure: the pane is over.
                        Err(_) => break,
                    }
                }
            })?;

        let (input, input_rx) = mpsc::channel::<Zeroizing<Vec<u8>>>();
        thread::Builder::new()
            .name("htui-pane-write".to_owned())
            .spawn(move || {
                while let Ok(chunk) = input_rx.recv() {
                    if writer.write_all(&chunk).is_err() || writer.flush().is_err() {
                        break;
                    }
                }
                // `writer` drops here, after `input` did: the pane is being dropped (F-10).
            })?;

        Ok(Self {
            master,
            input,
            kill: Some(kill_tx),
            pid,
        })
    }

    /// The child's pid, if the platform has one.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
}

/// The wait thread (B5): polls `child` every [`POLL`] until it ends or a kill is asked for (or
/// the asking side is gone), then reaps it and sends `Exited`. A failed send (the loop is gone)
/// is ignored.
fn wait_for(
    mut child: Box<dyn Child + Send + Sync>,
    kill: &mpsc::Receiver<()>,
    id: PaneId,
    started: Instant,
    events: &UnboundedSender<PaneEvent>,
) {
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(err) => break Err(err),
            Ok(None) => {}
        }
        match kill.recv_timeout(POLL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                // The owned child's kill escalates: SIGHUP, a 200 ms grace, SIGKILL.
                let _ = child.kill();
                break child.wait();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    let _ = events.send(PaneEvent::Exited {
        id,
        status,
        elapsed: started.elapsed(),
    });
}

impl PaneChild for PtyChild {
    fn write(&mut self, bytes: &[u8]) {
        if self.input.send(Zeroizing::new(bytes.to_vec())).is_err() {
            tracing::debug!(
                len = bytes.len(),
                "pane input dropped: the writer has ended"
            );
        }
    }

    fn resize(&mut self, size: PaneSize) {
        let size = size.floored();
        if self.master.resize(size.pty()).is_err() {
            tracing::debug!(rows = size.rows, cols = size.cols, "pane resize failed");
        }
    }

    fn kill(&mut self) {
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
    }
}

impl Drop for PtyChild {
    fn drop(&mut self) {
        self.kill();
    }
}

impl fmt::Debug for PtyChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PtyChild")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

/// The VT screen of a pane and the replies it owes the child (DSR, DA1). `Debug` prints the size.
pub struct PaneScreen {
    /// The grid, its modes, and the replies queued while parsing.
    parser: vt100::Parser<Replies>,
}

/// The callbacks: replies queued while parsing.
struct Replies {
    /// Bytes owed to the child, in order; taken by `PaneScreen::feed`.
    out: Vec<u8>,
}

impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|param| param.first()).copied();
        match (i1, i2, c, first) {
            // DSR, cursor position: 1-based row and column.
            (None, None, 'n', Some(6)) => {
                let (row, col) = screen.cursor_position();
                let _ = write!(
                    self.out,
                    "\x1b[{};{}R",
                    u32::from(row) + 1,
                    u32::from(col) + 1
                );
            }
            // DSR, status: OK.
            (None, None, 'n', Some(5)) => self.out.extend_from_slice(b"\x1b[0n"),
            // DA1: a VT220 with ANSI colour.
            (None, None, 'c', None | Some(0)) => self.out.extend_from_slice(b"\x1b[?62;22c"),
            // DA2, OSC queries, XTGETTCAP: unanswered (nvim falls back after its DA1 sentinel).
            _ => {}
        }
    }
}

impl PaneScreen {
    /// A blank screen of `size`, no scrollback.
    #[must_use]
    pub fn new(size: PaneSize) -> Self {
        let size = size.floored();
        Self {
            parser: vt100::Parser::new_with_callbacks(
                size.rows,
                size.cols,
                0,
                Replies { out: Vec::new() },
            ),
        }
    }

    /// Parses `bytes`; returns the replies to write back to the child (often empty).
    #[must_use]
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.parser.process(bytes);
        std::mem::take(&mut self.parser.callbacks_mut().out)
    }

    /// The screen, for the widget and the modes.
    #[must_use]
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// The current size.
    #[must_use]
    pub fn size(&self) -> PaneSize {
        let (rows, cols) = self.parser.screen().size();
        PaneSize { rows, cols }
    }

    /// Resizes the grid (`Screen::set_size`).
    pub fn resize(&mut self, size: PaneSize) {
        let size = size.floored();
        self.parser.screen_mut().set_size(size.rows, size.cols);
    }
}

impl fmt::Debug for PaneScreen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaneScreen")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

/// The xterm modifier parameter: `1 + shift(1) + alt(2) + ctrl(4)`.
fn modifier_param(modifiers: KeyModifiers) -> u8 {
    1 + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL))
}

/// `c` with CONTROL held, as a legacy terminal sends it.
fn control_char(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        ' ' | '@' | '2' => Some(0x00),
        '[' | '3' => Some(0x1B),
        '\\' | '4' => Some(0x1C),
        ']' | '5' => Some(0x1D),
        '^' | '6' => Some(0x1E),
        '_' | '7' | '/' => Some(0x1F),
        '?' | '8' => Some(0x7F),
        _ => None,
    }
}

/// ESC (when `alt`) then `body`: the prefix ALT adds to a single-key encoding.
fn alt_prefixed(alt: bool, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 1);
    if alt {
        out.push(0x1B);
    }
    out.extend_from_slice(body);
    out
}

/// `key` as the bytes a legacy xterm sends (blueprint §4.5); `None` for a key it has no encoding
/// for.
///
/// SHIFT on a character is ignored (crossterm already sent the shifted character). CONTROL maps
/// a character to its C0 byte where xterm has one; ALT prefixes ESC. Cursor keys, Home and End
/// switch to SS3 in application-cursor mode (DECCKM) when unmodified; any modifier gives the
/// `CSI 1;m` form, `m` being `1 + shift + 2·alt + 4·ctrl`.
#[must_use]
pub fn encode_key(key: KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    let modifiers = key.modifiers;
    let alt = modifiers.contains(KeyModifiers::ALT);
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let m = modifier_param(modifiers);
    let modified = m > 1;

    // A cursor-style key: `CSI final` / `SS3 final` unmodified, `CSI 1;m final` modified.
    let cursor = |last: char| {
        if modified {
            format!("\x1b[1;{m}{last}").into_bytes()
        } else if application_cursor {
            format!("\x1bO{last}").into_bytes()
        } else {
            format!("\x1b[{last}").into_bytes()
        }
    };
    // An SS3 function key (F1-F4): SS3 in both modes unmodified.
    let ss3 = |last: char| {
        if modified {
            format!("\x1b[1;{m}{last}").into_bytes()
        } else {
            format!("\x1bO{last}").into_bytes()
        }
    };
    // A tilde key: `CSI n ~` / `CSI n;m ~`.
    let tilde = |n: u8| {
        if modified {
            format!("\x1b[{n};{m}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    let bytes = match key.code {
        KeyCode::Char(c) => {
            let mut utf8 = [0u8; 4];
            let body = match control_char(c).filter(|_| ctrl) {
                Some(byte) => vec![byte],
                None => c.encode_utf8(&mut utf8).as_bytes().to_vec(),
            };
            alt_prefixed(alt, &body)
        }
        KeyCode::Enter => alt_prefixed(alt, b"\r"),
        KeyCode::Tab => alt_prefixed(alt, b"\t"),
        KeyCode::BackTab => alt_prefixed(alt, b"\x1b[Z"),
        KeyCode::Backspace => alt_prefixed(alt, if ctrl { &[0x08] } else { &[0x7F] }),
        KeyCode::Esc => alt_prefixed(alt, b"\x1b"),
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Right => cursor('C'),
        KeyCode::Left => cursor('D'),
        KeyCode::Home => cursor('H'),
        KeyCode::End => cursor('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => ss3(char::from(b'P' + (n - 1))),
        KeyCode::F(n @ 5..=12) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde(CODES[usize::from(n - 5)])
        }
        _ => return None,
    };
    Some(bytes)
}

/// The bracketed-paste start and end markers (xterm mode 2004).
const PASTE_START: &[u8] = b"\x1b[200~";
/// See [`PASTE_START`].
const PASTE_END: &[u8] = b"\x1b[201~";

/// `bytes` without any [`PASTE_END`], into a buffer that never reallocates (so no unzeroized copy
/// is left behind). Removing one can join two halves into another, hence the caller's loop.
fn without_paste_end(bytes: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::with_capacity(bytes.len()));
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(PASTE_END) {
            at += PASTE_END.len();
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}

/// A paste as the child should receive it (blueprint §4.6). Zeroized on drop: a paste may be a
/// credential (MOD-22).
///
/// Every `ESC [201~` is removed (a paste must not end the bracket early), `\r\n` and a lone `\n`
/// become `\r` (what a terminal sends for a pasted newline; a raw `\n` is `ctrl-j`), and, when
/// the child asked for bracketed paste, the result is wrapped in `ESC [200~` … `ESC [201~`.
#[must_use]
pub fn encode_paste(text: &str, bracketed: bool) -> Zeroizing<Vec<u8>> {
    let mut stripped = without_paste_end(text.as_bytes());
    while stripped
        .windows(PASTE_END.len())
        .any(|window| window == PASTE_END)
    {
        stripped = without_paste_end(&stripped);
    }

    let extra = if bracketed {
        PASTE_START.len() + PASTE_END.len()
    } else {
        0
    };
    let mut out = Zeroizing::new(Vec::with_capacity(stripped.len() + extra));
    if bracketed {
        out.extend_from_slice(PASTE_START);
    }
    let mut at = 0;
    while at < stripped.len() {
        match stripped[at] {
            b'\r' if stripped.get(at + 1) == Some(&b'\n') => {
                out.push(b'\r');
                at += 2;
            }
            b'\n' => {
                out.push(b'\r');
                at += 1;
            }
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }
    if bracketed {
        out.extend_from_slice(PASTE_END);
    }
    out
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    const SECRET: &str = "hunter2-SECRET";

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn enc(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
        encode_key(key(code, modifiers), false)
    }

    fn enc_app(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
        encode_key(key(code, modifiers), true)
    }

    fn bytes(s: &str) -> Option<Vec<u8>> {
        Some(s.as_bytes().to_vec())
    }

    #[test]
    fn the_legacy_key_table() {
        use KeyCode::*;
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let alt = KeyModifiers::ALT;
        let shift = KeyModifiers::SHIFT;

        // Characters: UTF-8; SHIFT is already in the character.
        assert_eq!(enc(Char('a'), none), bytes("a"));
        assert_eq!(enc(Char('A'), shift), bytes("A"));
        assert_eq!(enc(Char('é'), none), bytes("é"));
        assert_eq!(enc(Char('é'), none), Some(vec![0xC3, 0xA9]));

        // Ctrl + character.
        assert_eq!(enc(Char('c'), ctrl), Some(vec![0x03]));
        assert_eq!(enc(Char('a'), ctrl), Some(vec![0x01]));
        assert_eq!(enc(Char('z'), ctrl), Some(vec![0x1A]));
        assert_eq!(enc(Char('A'), ctrl), Some(vec![0x01]));
        assert_eq!(enc(Char('Z'), ctrl | shift), Some(vec![0x1A]));
        assert_eq!(enc(Char('h'), ctrl), Some(vec![0x08]));
        for c in [' ', '@', '2'] {
            assert_eq!(enc(Char(c), ctrl), Some(vec![0x00]), "ctrl-{c}");
        }
        for c in ['[', '3'] {
            assert_eq!(enc(Char(c), ctrl), Some(vec![0x1B]), "ctrl-{c}");
        }
        for c in ['\\', '4'] {
            assert_eq!(enc(Char(c), ctrl), Some(vec![0x1C]), "ctrl-{c}");
        }
        for c in [']', '5'] {
            assert_eq!(enc(Char(c), ctrl), Some(vec![0x1D]), "ctrl-{c}");
        }
        for c in ['^', '6'] {
            assert_eq!(enc(Char(c), ctrl), Some(vec![0x1E]), "ctrl-{c}");
        }
        for c in ['_', '7', '/'] {
            assert_eq!(enc(Char(c), ctrl), Some(vec![0x1F]), "ctrl-{c}");
        }
        for c in ['?', '8'] {
            assert_eq!(enc(Char(c), ctrl), Some(vec![0x7F]), "ctrl-{c}");
        }
        // Any other character: CONTROL dropped.
        assert_eq!(enc(Char('1'), ctrl), bytes("1"));
        assert_eq!(enc(Char('é'), ctrl), bytes("é"));
        assert_eq!(enc(Char('a'), ctrl | alt), Some(vec![0x1B, 0x01]));
        assert_eq!(enc(Char('4'), ctrl | alt), Some(vec![0x1B, 0x1C]));

        // Alt + character.
        assert_eq!(enc(Char('x'), alt), bytes("\x1bx"));
        assert_eq!(enc(Char('X'), alt | shift), bytes("\x1bX"));
        assert_eq!(enc(Char('é'), alt), bytes("\x1bé"));

        // Enter, Tab, BackTab, Backspace, Esc.
        assert_eq!(enc(Enter, none), bytes("\r"));
        assert_eq!(enc(Enter, alt), bytes("\x1b\r"));
        assert_eq!(enc(Tab, none), bytes("\t"));
        assert_eq!(enc(Tab, alt), bytes("\x1b\t"));
        assert_eq!(enc(BackTab, shift), bytes("\x1b[Z"));
        assert_eq!(enc(BackTab, alt | shift), bytes("\x1b\x1b[Z"));
        assert_eq!(enc(Backspace, none), Some(vec![0x7F]));
        assert_eq!(enc(Backspace, ctrl), Some(vec![0x08]));
        assert_eq!(enc(Backspace, alt), Some(vec![0x1B, 0x7F]));
        assert_eq!(enc(Backspace, ctrl | alt), Some(vec![0x1B, 0x08]));
        assert_eq!(enc(Esc, none), Some(vec![0x1B]));
        assert_eq!(enc(Esc, alt), bytes("\x1b\x1b"));

        // Cursor keys, Home, End.
        for (code, last) in [(Up, 'A'), (Down, 'B'), (Right, 'C'), (Left, 'D')] {
            assert_eq!(enc(code, none), bytes(&format!("\x1b[{last}")));
            assert_eq!(enc_app(code, none), bytes(&format!("\x1bO{last}")));
            assert_eq!(enc(code, shift), bytes(&format!("\x1b[1;2{last}")));
            assert_eq!(enc_app(code, shift), bytes(&format!("\x1b[1;2{last}")));
            assert_eq!(enc(code, alt), bytes(&format!("\x1b[1;3{last}")));
            assert_eq!(enc(code, ctrl), bytes(&format!("\x1b[1;5{last}")));
            assert_eq!(
                enc(code, ctrl | alt | shift),
                bytes(&format!("\x1b[1;8{last}"))
            );
        }
        assert_eq!(enc(Up, shift), bytes("\x1b[1;2A"));
        assert_eq!(enc_app(Up, shift), bytes("\x1b[1;2A"));
        assert_eq!(enc(Right, ctrl), bytes("\x1b[1;5C"));
        assert_eq!(enc(Home, none), bytes("\x1b[H"));
        assert_eq!(enc(End, none), bytes("\x1b[F"));
        assert_eq!(enc_app(Home, none), bytes("\x1bOH"));
        assert_eq!(enc_app(End, none), bytes("\x1bOF"));
        assert_eq!(enc(Home, ctrl), bytes("\x1b[1;5H"));
        assert_eq!(enc_app(End, shift), bytes("\x1b[1;2F"));

        // The tilde keys.
        for (code, n) in [(Insert, 2), (Delete, 3), (PageUp, 5), (PageDown, 6)] {
            assert_eq!(enc(code, none), bytes(&format!("\x1b[{n}~")));
            assert_eq!(enc_app(code, none), bytes(&format!("\x1b[{n}~")));
            assert_eq!(enc(code, shift), bytes(&format!("\x1b[{n};2~")));
        }
        assert_eq!(enc(Delete, ctrl), bytes("\x1b[3;5~"));

        // Function keys.
        for (n, last) in [(1, 'P'), (2, 'Q'), (3, 'R'), (4, 'S')] {
            assert_eq!(enc(F(n), none), bytes(&format!("\x1bO{last}")));
            assert_eq!(enc_app(F(n), none), bytes(&format!("\x1bO{last}")));
            assert_eq!(enc(F(n), ctrl), bytes(&format!("\x1b[1;5{last}")));
        }
        for (n, code) in [
            (5, 15),
            (6, 17),
            (7, 18),
            (8, 19),
            (9, 20),
            (10, 21),
            (11, 23),
            (12, 24),
        ] {
            assert_eq!(enc(F(n), none), bytes(&format!("\x1b[{code}~")));
            assert_eq!(enc(F(n), alt), bytes(&format!("\x1b[{code};3~")));
        }
        assert_eq!(enc(F(5), shift), bytes("\x1b[15;2~"));

        // No encoding.
        for code in [
            F(13),
            F(0),
            Null,
            CapsLock,
            ScrollLock,
            NumLock,
            PrintScreen,
            Pause,
            Menu,
            KeypadBegin,
            Media(crossterm::event::MediaKeyCode::Play),
            Modifier(crossterm::event::ModifierKeyCode::LeftShift),
        ] {
            assert_eq!(enc(code, none), None, "{code:?}");
        }
    }

    #[test]
    fn application_cursor_mode_changes_unmodified_arrows_and_home_end_only() {
        use KeyCode::*;
        let none = KeyModifiers::NONE;
        let changed = [Up, Down, Right, Left, Home, End];
        for code in changed {
            assert_ne!(enc(code, none), enc_app(code, none), "{code:?}");
        }
        let modifiers = [
            KeyModifiers::NONE,
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ];
        let codes = [
            Up,
            Down,
            Right,
            Left,
            Home,
            End,
            Char('a'),
            Char('4'),
            Enter,
            Tab,
            BackTab,
            Backspace,
            Esc,
            Insert,
            Delete,
            PageUp,
            PageDown,
            F(1),
            F(4),
            F(5),
            F(12),
            F(13),
        ];
        for code in codes {
            for modifiers in modifiers {
                if changed.contains(&code) && modifiers == KeyModifiers::NONE {
                    continue;
                }
                assert_eq!(
                    enc(code, modifiers),
                    enc_app(code, modifiers),
                    "{code:?} {modifiers:?}"
                );
            }
        }
    }

    #[test]
    fn a_paste_is_normalised_bracketed_and_cannot_close_the_bracket() {
        assert_eq!(encode_paste("a\nb", false).as_slice(), b"a\rb");
        assert_eq!(encode_paste("a\r\nb\n", false).as_slice(), b"a\rb\r");
        assert_eq!(
            encode_paste("a\r\nb", true).as_slice(),
            b"\x1b[200~a\rb\x1b[201~"
        );
        let pasted = encode_paste("x\x1b[201~y", true);
        assert_eq!(pasted.as_slice(), b"\x1b[200~xy\x1b[201~");
        // A closing bracket split around another one is still removed.
        let pasted = encode_paste("x\x1b[20\x1b[201~1~y", true);
        assert_eq!(pasted.as_slice(), b"\x1b[200~xy\x1b[201~");
        assert_eq!(encode_paste("x\x1b[201~y", false).as_slice(), b"xy");
        assert_eq!(encode_paste("", true).as_slice(), b"\x1b[200~\x1b[201~");
    }

    #[test]
    fn the_screen_answers_dsr_and_da1() {
        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        assert_eq!(screen.feed(b"ab\x1b[6n"), b"\x1b[1;3R");
        assert_eq!(screen.feed(b"\r\n\x1b[6n"), b"\x1b[2;1R");
        assert_eq!(screen.feed(b"\x1b[c"), b"\x1b[?62;22c");
        assert_eq!(screen.feed(b"\x1b[0c"), b"\x1b[?62;22c");
        assert_eq!(screen.feed(b"\x1b[5n"), b"\x1b[0n");
        assert!(screen.feed(b"\x1b[>c").is_empty());
        assert!(screen.feed(b"plain text").is_empty());
        // Two queries in one read: both answered, in order.
        assert_eq!(screen.feed(b"\x1b[5n\x1b[c"), b"\x1b[0n\x1b[?62;22c");
        assert!(screen.screen().contents().contains("plain text"));
    }

    #[test]
    fn the_screen_reports_its_modes() {
        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        assert!(!screen.screen().application_cursor());
        assert!(!screen.screen().bracketed_paste());
        assert!(!screen.screen().hide_cursor());
        assert!(!screen.screen().alternate_screen());
        let _ = screen.feed(b"\x1b[?1h\x1b[?2004h\x1b[?25l");
        assert!(screen.screen().application_cursor());
        assert!(screen.screen().bracketed_paste());
        assert!(screen.screen().hide_cursor());
        let _ = screen.feed(b"\x1b[?1049h");
        assert!(screen.screen().alternate_screen());
    }

    #[test]
    fn resize_changes_the_grid_and_never_to_zero() {
        assert_eq!(PaneSize::new(0, 0), PaneSize { rows: 1, cols: 1 });
        assert_eq!(PaneSize::new(0, 7), PaneSize { rows: 1, cols: 7 });
        assert_eq!(PaneSize::new(5, 0), PaneSize { rows: 5, cols: 1 });
        assert_eq!(PaneSize::DEFAULT, PaneSize { rows: 24, cols: 80 });

        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        assert_eq!(screen.size(), PaneSize::DEFAULT);
        assert_eq!(screen.screen().size(), (24, 80));
        screen.resize(PaneSize::new(30, 100));
        assert_eq!(screen.size(), PaneSize::new(30, 100));
        assert_eq!(screen.screen().size(), (30, 100));
        screen.resize(PaneSize::new(0, 0));
        assert_eq!(screen.size(), PaneSize::new(1, 1));
        // A literal zero (the fields are public) is floored too.
        screen.resize(PaneSize { rows: 0, cols: 0 });
        assert_eq!(screen.screen().size(), (1, 1));
        let screen = PaneScreen::new(PaneSize { rows: 0, cols: 3 });
        assert_eq!(screen.size(), PaneSize::new(1, 3));
    }

    #[test]
    fn nothing_debugs_bytes_or_contents() {
        let id = PaneId::next();
        let event = PaneEvent::Output {
            id,
            bytes: SECRET.as_bytes().to_vec(),
        };
        let debug = format!("{event:?}");
        assert!(!debug.contains(SECRET), "{debug}");
        assert!(debug.contains("len"), "{debug}");
        assert!(debug.contains(&SECRET.len().to_string()), "{debug}");
        assert_eq!(event.id(), id);

        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        let _ = screen.feed(SECRET.as_bytes());
        assert!(screen.screen().contents().contains(SECRET));
        let debug = format!("{screen:?}");
        assert!(!debug.contains(SECRET), "{debug}");
        assert!(debug.contains("24"), "{debug}");

        let exited = PaneEvent::Exited {
            id,
            status: Ok(portable_pty::ExitStatus::with_exit_code(0)),
            elapsed: Duration::from_millis(5),
        };
        assert_eq!(exited.id(), id);
        assert_ne!(PaneId::next(), id);
    }

    /// Real children on a real PTY (blueprint §4.7 tests 8-14): real time, never paused. Each test
    /// holds its `PtyChild`, so a failing assertion kills the child on drop.
    #[cfg(unix)]
    mod real {
        use std::path::Path;
        use std::time::{Duration, Instant};

        use portable_pty::CommandBuilder;
        use tempfile::TempDir;
        use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

        use super::super::*;

        /// `body` written to a script in `dir` (as `editor.rs`'s `scripts::script` does) and run
        /// as `sh <script> <file>`: `$1` is `file`. Run through `sh` rather than executed, so a
        /// concurrent fork elsewhere in the test binary cannot make it `ETXTBSY`.
        fn script(dir: &TempDir, body: &str, file: &Path) -> CommandBuilder {
            let path = dir.path().join("editor.sh");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
            let mut cmd = CommandBuilder::new("sh");
            cmd.arg(&path);
            cmd.arg(file);
            cmd.env("TERM", "xterm-256color");
            cmd.env_remove("LINES");
            cmd.env_remove("COLUMNS");
            cmd.cwd(dir.path());
            cmd
        }

        /// A spawned pane, the receiving end of its events and the screen they are parsed into.
        struct Pane {
            child: PtyChild,
            rx: UnboundedReceiver<PaneEvent>,
            screen: PaneScreen,
            id: PaneId,
        }

        impl Pane {
            fn spawn(command: CommandBuilder) -> Self {
                let (tx, rx) = unbounded_channel();
                let id = PaneId::next();
                // The sender moves into the pane: the test keeps none (test 13).
                let child = PtyChild::spawn(command, PaneSize::DEFAULT, id, tx).expect("spawn");
                Self {
                    child,
                    rx,
                    screen: PaneScreen::new(PaneSize::DEFAULT),
                    id,
                }
            }

            /// The next event within `limit`; `None` once every sender is gone.
            async fn next(&mut self, limit: Duration) -> Option<PaneEvent> {
                tokio::time::timeout(limit, self.rx.recv())
                    .await
                    .expect("an event in time")
            }

            /// Feeds an `Output` into the screen and writes the replies back.
            fn absorb(&mut self, event: &PaneEvent) {
                assert_eq!(event.id(), self.id);
                if let PaneEvent::Output { bytes, .. } = event {
                    let replies = self.screen.feed(bytes);
                    if !replies.is_empty() {
                        self.child.write(&replies);
                    }
                }
            }

            /// Pumps output until the screen shows `text`, within 10 s.
            async fn until_shown(&mut self, text: &str) {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !self.screen.screen().contents().contains(text) {
                    let left = deadline.saturating_duration_since(Instant::now());
                    let event = self.next(left).await.expect("the pane is still running");
                    assert!(
                        matches!(event, PaneEvent::Output { .. }),
                        "exited before showing {text:?}: {event:?}"
                    );
                    self.absorb(&event);
                }
            }

            /// Pumps output until `Exited`, within `limit`.
            async fn exited(&mut self, limit: Duration) -> io::Result<portable_pty::ExitStatus> {
                let deadline = Instant::now() + limit;
                loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    match self.next(left).await.expect("an Exited before the end") {
                        PaneEvent::Exited { id, status, .. } => {
                            assert_eq!(id, self.id);
                            return status;
                        }
                        output => self.absorb(&output),
                    }
                }
            }
        }

        /// As `editor.rs`'s `suspension::alive`: a pid that `ps` no longer lists, or lists as a
        /// zombie, is gone.
        fn alive(pid: &str) -> bool {
            let out = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", pid])
                .output()
                .expect("run ps");
            let stat = String::from_utf8_lossy(&out.stdout);
            let stat = stat.trim();
            !stat.is_empty() && !stat.starts_with('Z')
        }

        #[tokio::test]
        async fn a_scripted_editor_draws_reads_a_line_and_exits() {
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("note.md");
            std::fs::write(&file, "hello\n").unwrap();
            let mut pane = Pane::spawn(script(
                &dir,
                "printf ready; IFS= read -r line; printf '%s\\n' \"$line\" > \"$1\"",
                &file,
            ));
            assert!(pane.child.pid().is_some());
            pane.until_shown("ready").await;
            pane.child.write(b"edited\r");
            let status = pane.exited(Duration::from_secs(10)).await.expect("reaped");
            assert!(status.success(), "{status:?}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "edited\n");
        }

        #[tokio::test]
        async fn the_child_sees_the_size_and_a_resize() {
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("note.md");
            let mut pane = Pane::spawn(script(&dir, "stty size; read x; stty size; read y", &file));
            pane.until_shown("24 80").await;
            let size = PaneSize::new(30, 100);
            pane.child.resize(size);
            pane.screen.resize(size);
            pane.child.write(b"\r");
            pane.until_shown("30 100").await;
            pane.child.write(b"\r");
            let status = pane.exited(Duration::from_secs(10)).await.expect("reaped");
            assert!(status.success(), "{status:?}");
        }

        #[tokio::test]
        async fn kill_ends_the_child_and_exited_follows() {
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("note.md");
            let mut pane = Pane::spawn(script(&dir, "printf ready; exec sleep 30", &file));
            pane.until_shown("ready").await;
            pane.child.kill();
            pane.child.kill(); // idempotent
            let status = pane.exited(Duration::from_secs(3)).await.expect("reaped");
            assert!(!status.success(), "{status:?}");
        }

        #[tokio::test]
        async fn a_hup_ignoring_child_is_killed_by_escalation() {
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("note.md");
            let mut pane = Pane::spawn(script(
                &dir,
                "trap '' HUP; printf ready; while :; do sleep 0.1; done",
                &file,
            ));
            // The trap is set before `ready`, so SIGHUP alone cannot end it.
            pane.until_shown("ready").await;
            pane.child.kill();
            let status = pane.exited(Duration::from_secs(3)).await.expect("reaped");
            assert!(!status.success(), "{status:?}");
        }

        #[tokio::test]
        async fn dropping_the_child_leaves_no_process() {
            let dir = TempDir::new().unwrap();
            let pidfile = dir.path().join("pid");
            let pane = Pane::spawn(script(
                &dir,
                "printf '%s' $$ > \"$1\"; exec sleep 30",
                &pidfile,
            ));
            let deadline = Instant::now() + Duration::from_secs(10);
            let pid = loop {
                let pid = std::fs::read_to_string(&pidfile).unwrap_or_default();
                if !pid.is_empty() {
                    break pid;
                }
                assert!(Instant::now() < deadline, "the script never wrote its pid");
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            assert_eq!(
                pane.child.pid().map(|pid| pid.to_string()),
                Some(pid.clone())
            );
            assert!(alive(&pid), "{pid} is not running yet");
            drop(pane);
            let deadline = Instant::now() + Duration::from_secs(3);
            while alive(&pid) {
                assert!(Instant::now() < deadline, "{pid} outlived its pane");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        #[tokio::test]
        async fn both_threads_end_after_exit() {
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("note.md");
            let mut pane = Pane::spawn(script(&dir, "exit 0", &file));
            let status = pane.exited(Duration::from_secs(10)).await.expect("reaped");
            assert!(status.success(), "{status:?}");
            // The reader saw EOF (the slave was dropped) and the wait thread ended: every sender
            // is gone while the pane itself is still held.
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match pane.next(left).await {
                    None => break,
                    Some(event) => {
                        assert!(matches!(event, PaneEvent::Output { .. }), "{event:?}");
                    }
                }
            }
        }

        #[tokio::test]
        async fn a_dsr_from_the_child_is_answered() {
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("reply");
            let mut pane = Pane::spawn(script(
                &dir,
                "stty -icanon -echo; printf '\\033[6n'; dd bs=1 count=6 2>/dev/null > \"$1\"",
                &file,
            ));
            let status = pane.exited(Duration::from_secs(10)).await.expect("reaped");
            assert!(status.success(), "{status:?}");
            assert_eq!(std::fs::read(&file).unwrap(), b"\x1b[1;1R");
        }
    }
}
