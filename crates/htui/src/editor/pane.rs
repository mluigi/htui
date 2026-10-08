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
//!   which escalates on the child it owns and then reaps it. On unix the child leads its own
//!   session and process group (portable-pty runs `setsid`), and the whole group gets SIGHUP, a
//!   grace of up to 200 ms (cut short once the child has exited), then SIGKILL: a grandchild that
//!   ignores SIGHUP (an `$EDITOR` wrapper that does not `exec`) cannot outlive the pane and keep
//!   the slave, and so the reader, alive. A child that exits on its own is noticed unreaped
//!   (`waitid` with `WNOWAIT`), and the rest of its group gets the same SIGHUP and SIGKILL. Every
//!   signal is sent before the child is reaped, so its pid, which is the group's id, cannot have
//!   been reused. On Windows: portable-pty's kill (`TerminateProcess`). No signal is sent from
//!   the caller's thread.
//! - [`PaneScreen`]: the VT screen a pane's output is parsed into (`vt100`), and the replies the
//!   child is owed for its terminal queries (DSR, DA1). The output passes a CSI clamp first
//!   (`CsiClamp`, R1 H-1): `vt100` repeats ICH, IL and SD as many times as their count asks, on
//!   the UI task, so their counts are held to the screen's size.
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

/// How often the wait thread polls the child while no kill is asked for, or during the grace.
const POLL: Duration = Duration::from_millis(25);

/// How long the child's group has between SIGHUP and SIGKILL (unix).
#[cfg(unix)]
const GRACE: Duration = Duration::from_millis(200);

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

/// The smallest side of a pane, in cells. `vt100` 0.16.2 underflows on a 1-row or 1-column grid
/// (`Grid::col_wrap`: a 1-row grid panics once a line wraps, a 1-column one on a wide character),
/// and the kernel dislikes 0.
const MIN_SIDE: u16 = 2;

/// A pane's size in cells. Never under 2 in either dimension (`vt100` cannot take 1, the kernel
/// dislikes 0).
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

    /// `rows`/`cols`, each at least 2.
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows: rows.max(MIN_SIDE),
            cols: cols.max(MIN_SIDE),
        }
    }

    /// The same size, floored again: the fields are public, so a literal can hold a 0 or a 1.
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
    /// The wait, reader and writer threads, so a test can see them end (the real-child tests
    /// are unix-only).
    #[cfg(all(test, unix))]
    threads: Vec<thread::JoinHandle<()>>,
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
        // Dropped by the wait thread once the child is reaped; the writer thread waits for it.
        let (reaped_tx, reaped_rx) = mpsc::channel::<()>();
        // Only for the one path where the thread cannot start: `Builder::spawn` then drops the
        // closure, and the child with it, unkilled. The child is not reaped yet, so its pid is
        // still its own.
        let mut fallback = child.clone_killer();
        let wait_events = events.clone();
        let waiting = thread::Builder::new()
            .name("htui-pane-wait".to_owned())
            .spawn(move || {
                wait_for(child, &kill_rx, id, started, &wait_events);
                drop(reaped_tx);
            });
        let waiting = match waiting {
            Ok(waiting) => waiting,
            Err(err) => {
                let _ = fallback.kill();
                return Err(err);
            }
        };
        drop(fallback);

        // Any error below drops `kill_tx`, and the wait thread kills the child.
        let mut reader = master
            .try_clone_reader()
            .map_err(|err| io::Error::other(err.to_string()))?;
        let mut writer = master
            .take_writer()
            .map_err(|err| io::Error::other(err.to_string()))?;

        let reading = thread::Builder::new()
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
        let writing = thread::Builder::new()
            .name("htui-pane-write".to_owned())
            .spawn(move || {
                while let Ok(chunk) = input_rx.recv() {
                    if writer.write_all(&chunk).is_err() || writer.flush().is_err() {
                        break;
                    }
                }
                // `writer` drops after `input` did (the pane is being dropped, F-10) **and** after
                // the child is reaped: its drop types `\n` and VEOF, which a child that ignores
                // SIGHUP would still read in the grace before SIGKILL (T2 verify round 3).
                let _ = reaped_rx.recv();
            })?;

        let threads = vec![waiting, reading, writing];
        // Detached outside tests: each thread ends on its own (EOF, a closed queue, a reap).
        #[cfg(not(all(test, unix)))]
        drop(threads);
        Ok(Self {
            master,
            input,
            kill: Some(kill_tx),
            pid,
            #[cfg(all(test, unix))]
            threads,
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
        if let Some(status) = ended(&mut *child) {
            break status;
        }
        match kill.recv_timeout(POLL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break end(&mut *child),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    let _ = events.send(PaneEvent::Exited {
        id,
        status,
        elapsed: started.elapsed(),
    });
}

/// Whether `child` has ended on its own: `None` while it runs; once it has exited, the rest of
/// its process group is ended as on a kill ([`end`]: SIGHUP, then SIGKILL at once, the child
/// being gone) and the child reaped. The kernel's own SIGHUP at the child's exit does not end a
/// job that ignores it, and that job would hold the slave, and so the reader, open. The probe
/// does not reap, so the group's id stays the child's while the signals go out.
#[cfg(unix)]
fn ended(child: &mut (dyn Child + Send + Sync)) -> Option<io::Result<portable_pty::ExitStatus>> {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

    let Some(pid) = child
        .process_id()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(Pid::from_raw)
    else {
        return child.try_wait().transpose();
    };
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
    match waitid(WaitId::Pid(pid), options) {
        Ok(None) => None,
        Ok(Some(_)) => Some(end(child)),
        // Not ours to probe (never expected): reap as before, without signalling a group whose
        // id may no longer be the child's.
        Err(_) => child.try_wait().transpose(),
    }
}

/// Whether `child` has ended on its own; if so, reaped.
#[cfg(not(unix))]
fn ended(child: &mut (dyn Child + Send + Sync)) -> Option<io::Result<portable_pty::ExitStatus>> {
    child.try_wait().transpose()
}

/// Ends the child and its process group, then reaps the child (B5). `child` is not reaped yet.
#[cfg(unix)]
fn end(child: &mut (dyn Child + Send + Sync)) -> io::Result<portable_pty::ExitStatus> {
    use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};

    let Some(group) = child
        .process_id()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(Pid::from_raw)
    else {
        // No pid: portable-pty's own escalation, on the child alone.
        let _ = child.kill();
        return child.wait();
    };
    // The child is a session leader, so its pid is its group's id; unreaped, nobody else has it.
    let _ = kill_process_group(group, Signal::HUP);
    // Has the child exited? `NOWAIT` leaves it unreaped, so the group id stays ours.
    let exited = || {
        let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
        matches!(waitid(WaitId::Pid(group), options), Ok(Some(_)) | Err(_))
    };
    let deadline = Instant::now() + GRACE;
    while !exited() && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    // Whatever is left of the group: the child, if it ignored SIGHUP, and any descendant.
    let _ = kill_process_group(group, Signal::KILL);
    child.wait()
}

/// Ends the child, then reaps it (B5): portable-pty's kill, `TerminateProcess`.
#[cfg(not(unix))]
fn end(child: &mut (dyn Child + Send + Sync)) -> io::Result<portable_pty::ExitStatus> {
    let _ = child.kill();
    child.wait()
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
    /// What the child's bytes go through before the parser (R1 H-1).
    clamp: CsiClamp,
    /// The clamped bytes of one feed, kept for the next.
    clamped: Vec<u8>,
}

/// ESC, CAN and SUB: the bytes that end a sequence from inside it (`vte`'s "anywhere").
const ESC: u8 = 0x1B;
/// See [`ESC`].
const CAN: u8 = 0x18;
/// See [`ESC`].
const SUB: u8 = 0x1A;

/// `vte` 0.15 keeps at most this many parameters and subparameters (`params.rs`, `MAX_PARAMS`);
/// what follows them changes nothing it dispatches.
const VTE_MAX_PARAMS: usize = 32;

/// Where the clamp is in the child's output: `vte` 0.15's states, as far as a CSI needs them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ClampState {
    /// Text, or any sequence that is not a CSI (OSC, DCS, an `ESC` final): only an ESC matters.
    #[default]
    Ground,
    /// After an ESC (already passed on).
    Escape,
    /// After `ESC [`: the `[` and the parameters are held until the final byte.
    Csi,
    /// A CSI with a private marker or an intermediate: passed on as it comes.
    Passing,
}

/// R1 H-1: `vt100` 0.16.2 runs ICH (`CSI n @`, quadratic in the row), IL (`CSI n L`) and SD
/// (`CSI n T`) `n` times, uncapped, on the UI task: one `CSI 65535 @` took half a second. Past a
/// screen's width (ICH) or height (IL, SD) every count is the same edit, so the clamp rewrites
/// that count to the screen's size before `vt100` sees it.
///
/// Stateful, because `vte` is: a sequence split across two reads is one sequence. It mirrors
/// `vte`'s transitions: an ESC restarts from anywhere (inside an OSC or a DCS too); `ESC [`
/// starts a CSI; inside one, a C0 byte other than CAN, SUB and ESC is executed in place, so it is
/// passed on at once, ahead of the held CSI (the order `vte` executes them in); CAN and SUB
/// abort it; a private marker (`0x3C`-`0x3F`) or an intermediate (`0x20`-`0x2F`) makes it a
/// sequence `vt100` does not read as ICH, IL or SD, passed on as it comes; DEL and bytes from
/// `0x80` are ignored. No digit is held raw (leading zeros would grow the buffer without bound):
/// each parameter is counted as `vte` counts it (a saturating `u16`) and written back in decimal,
/// at most [`VTE_MAX_PARAMS`] of them. Every other byte is passed on unchanged.
#[derive(Debug, Default)]
struct CsiClamp {
    /// Where the last byte left it.
    state: ClampState,
    /// The held CSI's first parameter; `None` before its first digit.
    first: Option<u16>,
    /// The held CSI's further parameters: each separator (`;` or `:`) and its number.
    rest: Vec<(u8, Option<u16>)>,
}

impl CsiClamp {
    /// Passes `bytes` on to `out`, clamped for a screen of `size` (the size the sequence's final
    /// byte meets). A CSI not yet complete is held for the next call.
    fn filter(&mut self, bytes: &[u8], size: PaneSize, out: &mut Vec<u8>) {
        for &byte in bytes {
            match self.state {
                ClampState::Ground => {
                    out.push(byte);
                    if byte == ESC {
                        self.state = ClampState::Escape;
                    }
                }
                ClampState::Escape => match byte {
                    b'[' => {
                        self.first = None;
                        self.rest.clear();
                        self.state = ClampState::Csi;
                    }
                    // Another sequence (an `ESC` final, OSC, DCS, ...) or an abort.
                    CAN | SUB | 0x20..=0x7E => {
                        out.push(byte);
                        self.state = ClampState::Ground;
                    }
                    // ESC again, a C0 byte (executed), DEL or a high byte: still after an ESC.
                    _ => out.push(byte),
                },
                ClampState::Csi => match byte {
                    // Restart: the held CSI is dropped, as `vte` drops it. The ESC already
                    // passed on stays in `vt100`'s escape state, which this one continues.
                    ESC => {
                        out.push(byte);
                        self.state = ClampState::Escape;
                    }
                    CAN | SUB => {
                        out.push(byte);
                        self.state = ClampState::Ground;
                    }
                    0x00..=0x1F => out.push(byte),
                    b'0'..=b'9' => self.digit(byte - b'0'),
                    b':' | b';' => {
                        if self.rest.len() < VTE_MAX_PARAMS {
                            self.rest.push((byte, None));
                        }
                    }
                    0x20..=0x2F | 0x3C..=0x3F => {
                        self.release(out);
                        out.push(byte);
                        self.state = ClampState::Passing;
                    }
                    0x40..=0x7E => {
                        let limit = match byte {
                            b'@' => Some(size.cols),
                            b'L' | b'T' => Some(size.rows),
                            _ => None,
                        };
                        if let (Some(limit), Some(first)) = (limit, self.first.as_mut()) {
                            *first = (*first).min(limit);
                        }
                        self.release(out);
                        out.push(byte);
                        self.state = ClampState::Ground;
                    }
                    // DEL and high bytes: ignored inside a CSI.
                    _ => {}
                },
                ClampState::Passing => {
                    out.push(byte);
                    match byte {
                        ESC => self.state = ClampState::Escape,
                        CAN | SUB | 0x40..=0x7E => self.state = ClampState::Ground,
                        _ => {}
                    }
                }
            }
        }
    }

    /// One digit of the current parameter. Past [`VTE_MAX_PARAMS`] it changes nothing.
    fn digit(&mut self, digit: u8) {
        if self.rest.len() >= VTE_MAX_PARAMS {
            return;
        }
        let param = match self.rest.last_mut() {
            Some((_, param)) => param,
            None => &mut self.first,
        };
        let value = param.unwrap_or(0);
        *param = Some(value.saturating_mul(10).saturating_add(u16::from(digit)));
    }

    /// Passes the held `[` and parameters on.
    fn release(&mut self, out: &mut Vec<u8>) {
        out.push(b'[');
        if let Some(first) = self.first.take() {
            let _ = write!(out, "{first}");
        }
        for (separator, param) in self.rest.drain(..) {
            out.push(separator);
            if let Some(param) = param {
                let _ = write!(out, "{param}");
            }
        }
    }
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
            // DSR, cursor position: 1-based row and column. In the pending-wrap state `vt100`
            // holds the column one past the edge; xterm reports the last column. (Origin mode,
            // DECOM, is not honoured: `vt100` does not expose it or the scroll region.)
            (None, None, 'n', Some(6)) => {
                let (row, col) = screen.cursor_position();
                let col = col.min(screen.size().1.saturating_sub(1));
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
            clamp: CsiClamp::default(),
            clamped: Vec::new(),
        }
    }

    /// Parses `bytes`; returns the replies to write back to the child (often empty).
    #[must_use]
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.clamped.clear();
        self.clamp.filter(bytes, self.size(), &mut self.clamped);
        self.parser.process(&self.clamped);
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
    ///
    /// `vt100` 0.16.2 keeps a wide character whose second half falls off a narrower edge: a wide
    /// cell in the last column, which the next erase or write of that column indexes past (a
    /// panic on the UI task). A grid, shown or not, left holding one is cleared; the editor
    /// redraws on the `SIGWINCH` the same resize sends it.
    pub fn resize(&mut self, size: PaneSize) {
        let size = size.floored();
        let (_, cols) = self.parser.screen().size();
        let screen = self.parser.screen_mut();
        screen.set_size(size.rows, size.cols);
        if size.cols < cols {
            clear_split_wide(screen);
        }
    }
}

/// Clears each grid of `screen` that holds a wide cell in its last column (see
/// [`PaneScreen::resize`]). The sequences go through a scratch parser that `screen` is swapped
/// into, so the pane's parser, maybe halfway through the child's escape sequence, is not fed
/// them, and no reply is queued. Mode 47 switches grids without saving, restoring or clearing
/// anything.
fn clear_split_wide(screen: &mut vt100::Screen) {
    fn clear_shown(scratch: &mut vt100::Parser) {
        let screen = scratch.screen();
        let (rows, cols) = screen.size();
        let split =
            (0..rows).any(|row| screen.cell(row, cols - 1).is_some_and(vt100::Cell::is_wide));
        if split {
            scratch.process(b"\x1b[2J");
        }
    }

    let mut scratch = vt100::Parser::new(MIN_SIDE, MIN_SIDE, 0);
    std::mem::swap(scratch.screen_mut(), screen);
    let alternate = scratch.screen().alternate_screen();
    let (other, back): (&[u8], &[u8]) = if alternate {
        (b"\x1b[?47l", b"\x1b[?47h")
    } else {
        (b"\x1b[?47h", b"\x1b[?47l")
    };
    clear_shown(&mut scratch);
    scratch.process(other);
    clear_shown(&mut scratch);
    scratch.process(back);
    std::mem::swap(scratch.screen_mut(), screen);
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
        // Floored at 2x2, not 1x1: see `MIN_SIDE`.
        assert_eq!(PaneSize::new(0, 0), PaneSize { rows: 2, cols: 2 });
        assert_eq!(PaneSize::new(1, 1), PaneSize { rows: 2, cols: 2 });
        assert_eq!(PaneSize::new(0, 7), PaneSize { rows: 2, cols: 7 });
        assert_eq!(PaneSize::new(5, 0), PaneSize { rows: 5, cols: 2 });
        assert_eq!(PaneSize::new(2, 2), PaneSize { rows: 2, cols: 2 });
        assert_eq!(PaneSize::DEFAULT, PaneSize { rows: 24, cols: 80 });

        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        assert_eq!(screen.size(), PaneSize::DEFAULT);
        assert_eq!(screen.screen().size(), (24, 80));
        screen.resize(PaneSize::new(30, 100));
        assert_eq!(screen.size(), PaneSize::new(30, 100));
        assert_eq!(screen.screen().size(), (30, 100));
        screen.resize(PaneSize::new(0, 0));
        assert_eq!(screen.size(), PaneSize::new(2, 2));
        // A literal zero or one (the fields are public) is floored too.
        screen.resize(PaneSize { rows: 0, cols: 1 });
        assert_eq!(screen.screen().size(), (2, 2));
        let screen = PaneScreen::new(PaneSize { rows: 1, cols: 3 });
        assert_eq!(screen.size(), PaneSize::new(2, 3));
        assert_eq!(PaneSize { rows: 0, cols: 1 }.pty().rows, 2);
        assert_eq!(PaneSize { rows: 0, cols: 1 }.pty().cols, 2);
    }

    #[test]
    fn the_smallest_grid_survives_wrapping_scrolling_and_wide_characters() {
        // `vt100` 0.16.2 underflows on a 1-row or 1-column grid (`grid.rs` `col_wrap`): a
        // 1-row grid panics once a line wraps, a 1-column one on a wide character.
        let floor = PaneSize::new(0, 0);
        assert!(floor.rows >= 2 && floor.cols >= 2, "{floor:?}");
        let wide = "\u{4e2d}".repeat(3);
        let line = "x".repeat(81);
        let cases: [(PaneSize, &[u8]); 7] = [
            (PaneSize::new(1, 80), line.as_bytes()),
            (PaneSize::new(1, 2), b"9Dd"),
            (PaneSize::new(1, 1), b"ab"),
            (PaneSize::new(24, 1), wide.as_bytes()),
            (PaneSize::new(0, 0), wide.as_bytes()),
            (PaneSize { rows: 1, cols: 1 }, b"abc\r\nd"),
            (PaneSize { rows: 0, cols: 1 }, wide.as_bytes()),
        ];
        for (size, bytes) in cases {
            let mut screen = PaneScreen::new(size);
            let _ = screen.feed(bytes);
            let _ = screen.feed(bytes);
        }
        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        screen.resize(PaneSize::new(0, 40));
        let _ = screen.feed(&[b'y'; 100]);
        screen.resize(PaneSize::new(24, 0));
        let _ = screen.feed(wide.as_bytes());

        // A seeded sweep of wrap-, scroll- and width-heavy tokens at the floor.
        let tokens: [&[u8]; 14] = [
            b"a",
            "\u{4e2d}".as_bytes(),
            b"\r",
            b"\n",
            b"\x08",
            b"\x1b[2;2r",
            b"\x1b[r",
            b"\x1b[L",
            b"\x1b[M",
            b"\x1b[9D",
            b"\x1b[9C",
            b"\x1bM",
            b"\x1b[?6h",
            b"\x1b[?6l",
        ];
        let mut seed: u64 = 0x5eed;
        let mut screen = PaneScreen::new(floor);
        for _ in 0..20_000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let pick = usize::try_from(seed >> 33).unwrap_or(0) % tokens.len();
            let _ = screen.feed(tokens[pick]);
        }
    }

    #[test]
    fn narrowing_past_a_wide_character_leaves_a_grid_that_takes_any_output() {
        // `vt100` 0.16.2's `set_size` keeps a wide character whose second half falls off the new
        // edge: a wide cell in the last column, and the next erase or write of it indexes past
        // the row. On either grid: the one not shown is resized too.
        let wide = format!("{}\u{4e2d}", "x".repeat(78));
        let after: [&[u8]; 4] = [
            b"\x1b[1;79H\x1b[K",
            b"\x1b[1;79H\x1b[1K",
            b"\x1b[1;79Hy",
            b"\x1b[1;79H\x1b[X",
        ];
        for (fill, then) in [
            (String::new(), String::new()),
            (String::new(), "\x1b[?1049h".to_owned()),
            ("\x1b[?1049h".to_owned(), "\x1b[?1049l".to_owned()),
        ] {
            for bytes in after {
                let mut screen = PaneScreen::new(PaneSize::DEFAULT);
                let _ = screen.feed(format!("{fill}{wide}").as_bytes());
                let alternate = screen.screen().alternate_screen();
                screen.resize(PaneSize::new(24, 79));
                assert_eq!(screen.screen().alternate_screen(), alternate);
                assert_eq!(screen.size(), PaneSize::new(24, 79));
                let _ = screen.feed(then.as_bytes());
                let _ = screen.feed(bytes);
            }
        }

        // Nothing is cleared when no wide character is split, and the modes survive.
        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        let _ = screen.feed(b"\x1b[?1h\x1b[?2004hkept\x1b[3;5H");
        screen.resize(PaneSize::new(24, 40));
        assert!(screen.screen().contents().contains("kept"));
        assert!(screen.screen().application_cursor());
        assert!(screen.screen().bracketed_paste());
        assert!(!screen.screen().alternate_screen());
        assert_eq!(screen.screen().cursor_position(), (2, 4));

        // A split mid-sequence: the live parser's half-read escape is not disturbed.
        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        let _ = screen.feed(wide.as_bytes());
        let _ = screen.feed(b"\x1b[?20");
        screen.resize(PaneSize::new(24, 79));
        let _ = screen.feed(b"04h");
        assert!(screen.screen().bracketed_paste());

        // A seeded sweep: wide characters, erases and writes with narrowing resizes between.
        let tokens: [&[u8]; 10] = [
            "\u{4e2d}".as_bytes(),
            b"a",
            b"\x1b[K",
            b"\x1b[1K",
            b"\x1b[1J",
            b"\x1b[X",
            b"\x1b[P",
            b"\x1b[9C",
            b"\r\n",
            b"\x1b[?1049h",
        ];
        let mut seed: u64 = 0x0057;
        let mut screen = PaneScreen::new(PaneSize::new(4, 9));
        for _ in 0..20_000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let pick = usize::try_from(seed >> 33).unwrap_or(0);
            if pick % 7 == 0 {
                let cols = u16::try_from(pick % 9).unwrap_or(0);
                screen.resize(PaneSize::new(4, cols));
            } else if pick % 11 == 0 {
                let _ = screen.feed(b"\x1b[?1049l");
            } else {
                let _ = screen.feed(tokens[pick % tokens.len()]);
            }
        }
    }

    #[test]
    fn a_dsr_in_the_pending_wrap_state_reports_the_last_column() {
        let mut screen = PaneScreen::new(PaneSize::new(24, 10));
        assert_eq!(screen.feed(b"0123456789\x1b[6n"), b"\x1b[1;10R");
        // The wrap happens on the next character.
        assert_eq!(screen.feed(b"a\x1b[6n"), b"\x1b[2;2R");
    }

    /// R1 H-1: `vt100` 0.16.2 runs ICH (`ESC [n@`, quadratically), IL (`ESC [nL`) and SD
    /// (`ESC [nT`) `n` times, uncapped: one `ESC [65535@` cost half a second on the UI task. The
    /// clamp holds each to a screen's worth, split across feeds and behind leading zeros too.
    #[test]
    fn huge_insert_and_scroll_counts_cost_a_screen_not_their_count() {
        let bound = Duration::from_secs(1);
        for last in ['@', 'L', 'T'] {
            let forms: [Vec<String>; 4] = [
                vec![format!("\x1b[65535{last}")],
                vec!["\x1b[655".to_owned(), format!("35{last}")],
                vec!["\x1b[".to_owned(), format!("65535{last}")],
                vec!["\x1b[0000000000".to_owned(), format!("000065535{last}")],
            ];
            for form in forms {
                let mut screen = PaneScreen::new(PaneSize::DEFAULT);
                let _ = screen.feed("x".repeat(80 * 24).as_bytes());
                let _ = screen.feed(b"\x1b[3;5H\x1b[2;20r\x1b[5;7H");
                let started = Instant::now();
                for round in 0..1000 {
                    for part in &form {
                        let _ = screen.feed(part.as_bytes());
                    }
                    let took = started.elapsed();
                    assert!(
                        took < bound,
                        "{form:?}: {took:?} after {} rounds",
                        round + 1
                    );
                }
            }
        }
    }

    /// The screen after `before` then `sequence` (fed in `parts`) through the clamp, against a
    /// plain `vt100` parser fed the same bytes unclamped: identical, cell by cell, and again
    /// after more output (the rows' wrap flags).
    fn assert_clamp_is_invisible(size: PaneSize, before: &str, parts: &[&str]) {
        let mut clamped = PaneScreen::new(size);
        let mut plain = vt100::Parser::new(size.rows, size.cols, 0);
        let _ = clamped.feed(before.as_bytes());
        plain.process(before.as_bytes());
        for part in parts {
            let _ = clamped.feed(part.as_bytes());
            plain.process(part.as_bytes());
        }
        let what = format!("{before:?} then {parts:?}");
        assert_same_screen(clamped.screen(), plain.screen(), &what);
        let after = "XY\u{4e2d}Z\r\nW";
        let _ = clamped.feed(after.as_bytes());
        plain.process(after.as_bytes());
        assert_same_screen(
            clamped.screen(),
            plain.screen(),
            &format!("{what}, then more"),
        );
    }

    fn assert_same_screen(clamped: &vt100::Screen, plain: &vt100::Screen, what: &str) {
        assert_eq!(clamped.contents(), plain.contents(), "{what}");
        assert_eq!(
            clamped.contents_formatted(),
            plain.contents_formatted(),
            "{what}"
        );
        assert_eq!(clamped.cursor_position(), plain.cursor_position(), "{what}");
        let (rows, cols) = plain.size();
        for row in 0..rows {
            for col in 0..cols {
                let cell = |screen: &vt100::Screen| {
                    screen.cell(row, col).map(|cell| {
                        (
                            cell.contents().to_owned(),
                            cell.is_wide(),
                            cell.is_wide_continuation(),
                            cell.bgcolor(),
                        )
                    })
                };
                assert_eq!(cell(clamped), cell(plain), "{what} at {row},{col}");
            }
        }
    }

    #[test]
    fn a_clamped_count_draws_what_the_unclamped_one_does() {
        // 6x10: every count from 1 past the screen up is the same edit, so the plain parser can
        // take them unclamped here (cheaply: the grid is small).
        let size = PaneSize::new(6, 10);
        let lines = "1aaaaaaaaa\r\n2bbbbbbbbb\r\n3ccccccccc\r\n4ddddddddd\r\n5eeeeeeeee\r\n6fffff";
        let befores = [
            // The cursor mid-row.
            format!("{lines}\x1b[1;4H"),
            // On a wide character's second half (ICH keeps it a continuation).
            format!("{lines}\x1b[1;1Hab\u{4e2d}cd\u{4e2d}ef\x1b[1;4H"),
            // In a scroll region (DECSTBM), and below one.
            format!("{lines}\x1b[2;4r\x1b[3;2H"),
            format!("{lines}\x1b[2;3r\x1b[5;2H"),
            // Pending wrap, with a background colour to carry.
            format!("{lines}\x1b[44m\x1b[6;1H0123456789"),
        ];
        for before in &befores {
            for last in ['@', 'L', 'T'] {
                let mut counts = vec!["", "0", "1", "5", "6", "9", "10", "11", "300", "1000"];
                if last != '@' {
                    // ICH is quadratic unclamped; IL and SD are linear, so the full count.
                    counts.push("65535");
                }
                for count in counts {
                    let whole = format!("\x1b[{count}{last}");
                    assert_clamp_is_invisible(size, before, &[&whole]);
                    // Split at every byte: the clamp keeps its state across feeds.
                    for at in 1..whole.len() {
                        assert_clamp_is_invisible(size, before, &[&whole[..at], &whole[at..]]);
                    }
                }
            }
        }

        // What `vte` does inside a CSI, kept: C0 runs in place, DEL and high bytes are ignored,
        // CAN/SUB abort, ESC restarts, leading zeros and saturation count as `vte` counts them,
        // a second parameter or a subparameter rides along, and a parameter list past `vte`'s 32
        // is cut where `vte` cuts it.
        let many = format!("\x1b[300{}@", ";1".repeat(40));
        let edge = [
            "\x1b[3\r00@",
            "\x1b[3\n00L",
            "\x1b[30\x7f0@",
            "\x1b[30\u{e9}0@",
            "\x1b[300\x18@",
            "\x1b[300\x1aL",
            "\x1b[300\x1b[2@",
            "\x1b\x07[300@",
            "\x1b[000000000000300@",
            "\x1b[99999999L",
            "\x1b[300;5@",
            "\x1b[300:5@",
            "\x1b[;300@",
            &many,
            "\x1b]0;[300@\x07",
            "\x1b]0;t\x1b[300@",
            "\x1bP[300@\x1b\\",
            "\x1b (300@",
            "\x1b[?300@",
            "\x1b[300 @",
            "\x1b[3?00@",
        ];
        for before in &befores {
            for sequence in edge {
                assert_clamp_is_invisible(size, before, &[sequence]);
                for at in 1..sequence.len() {
                    if sequence.is_char_boundary(at) {
                        assert_clamp_is_invisible(
                            size,
                            before,
                            &[&sequence[..at], &sequence[at..]],
                        );
                    }
                }
            }
        }
    }

    /// The bytes the clamp hands `vt100` for `parts`, fed in turn, at `size`.
    fn clamped(size: PaneSize, parts: &[&str]) -> Vec<String> {
        let mut clamp = CsiClamp::default();
        parts
            .iter()
            .map(|part| {
                let mut out = Vec::new();
                clamp.filter(part.as_bytes(), size, &mut out);
                String::from_utf8(out).expect("UTF-8 in, UTF-8 out")
            })
            .collect()
    }

    #[test]
    fn the_clamp_rewrites_only_the_count_of_ich_il_and_sd() {
        let size = PaneSize::DEFAULT;
        // Passed through byte for byte: every other sequence, the private and intermediate forms
        // of the three, and counts within the screen.
        for bytes in [
            "plain [65535@ text\r\n",
            "\x1b[?1049h",
            "\x1b[?2004h",
            "\x1b[?1h\x1b[?25l",
            "\x1b[38;2;1;2;3m",
            "\x1b[38:2::1:2:3m",
            "\x1b[38;5;196m\x1b[0m",
            "\x1b[2;20r",
            "\x1b[3;7H",
            "\x1b[8;30;100t",
            "\x1b[6n\x1b[5n\x1b[c\x1b[>c",
            "\x1b[>65535@",
            "\x1b[?65535L",
            "\x1b[65535 @",
            "\x1b[65535$T",
            "\x1b[80@\x1b[24L\x1b[24T",
            "\x1b[@\x1b[L\x1b[T\x1b[0@",
            "\x1b[65535M\x1b[65535S\x1b[65535P\x1b[65535X\x1b[65535C",
            "\x1b]0;[65535@\x07",
            "\x1bP[65535@\x1b\\",
            "\x1b(B\x1b7\x1b8\x1bM",
        ] {
            assert_eq!(clamped(size, &[bytes]).concat(), bytes, "{bytes:?}");
        }
        // Clamped: the count of ICH to the columns, of IL and SD to the rows.
        for (bytes, to) in [
            ("\x1b[65535@", "\x1b[80@"),
            ("\x1b[81@", "\x1b[80@"),
            ("\x1b[65535L", "\x1b[24L"),
            ("\x1b[25T", "\x1b[24T"),
            ("\x1b[00000065535;7@", "\x1b[80;7@"),
            ("\x1b[65535:2L", "\x1b[24:2L"),
            ("\x1b]0;t\x1b[65535@", "\x1b]0;t\x1b[80@"),
            ("\x1b[65\r535@", "\x1b\r[80@"),
        ] {
            assert_eq!(clamped(size, &[bytes]).concat(), to, "{bytes:?}");
        }
        // Across feeds: nothing of the count is passed on until its final byte.
        assert_eq!(
            clamped(size, &["ab\x1b[655", "35", "@cd"]),
            ["ab\x1b", "", "[80@cd"]
        );
        // The size at the final byte is the one that counts.
        let mut clamp = CsiClamp::default();
        let mut out = Vec::new();
        clamp.filter(b"\x1b[6553", PaneSize::DEFAULT, &mut out);
        clamp.filter(b"5L", PaneSize::new(30, 100), &mut out);
        assert_eq!(out, b"\x1b[30L");
    }

    #[test]
    fn the_clamp_keeps_the_screen_modes_and_replies() {
        let mut screen = PaneScreen::new(PaneSize::DEFAULT);
        let modes = b"\x1b[?1049h\x1b[?2004h\x1b[38;2;1;2;3mA\x1b[38:2::4:5:6mB\x1b[38;5;196mC";
        let _ = screen.feed(modes);
        let mut plain = vt100::Parser::new(24, 80, 0);
        plain.process(modes);
        assert!(screen.screen().alternate_screen());
        assert!(screen.screen().bracketed_paste());
        let fg = |screen: &vt100::Screen, col| screen.cell(0, col).map(vt100::Cell::fgcolor);
        assert_eq!(fg(screen.screen(), 0), Some(vt100::Color::Rgb(1, 2, 3)));
        assert_eq!(fg(screen.screen(), 2), Some(vt100::Color::Idx(196)));
        for col in 0..3 {
            assert_eq!(fg(screen.screen(), col), fg(plain.screen(), col), "{col}");
        }
        // A query split across feeds is answered once it is whole, as before.
        assert!(screen.feed(b"\x1b[").is_empty());
        assert_eq!(screen.feed(b"6n"), b"\x1b[1;4R");
        assert!(screen.feed(b"\x1b").is_empty());
        assert_eq!(screen.feed(b"[c"), b"\x1b[?62;22c");
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
        async fn dropping_the_pane_types_nothing_into_a_child_that_outlives_sighup() {
            // T2 verify round 3: the PTY writer's drop writes `\n` and VEOF. It must not happen
            // while a child that ignores SIGHUP is still reading, in the grace before SIGKILL.
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("note.md");
            let pane = Pane::spawn(script(
                &dir,
                "trap '' HUP; stty raw -echo; printf ready; dd bs=1 count=2 of=\"$1\" 2>/dev/null; exec sleep 30",
                &file,
            ));
            let mut pane = pane;
            pane.until_shown("ready").await;
            let Pane { child, mut rx, .. } = pane;
            drop(child);
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match tokio::time::timeout(left, rx.recv())
                    .await
                    .expect("Exited in time")
                {
                    Some(PaneEvent::Exited { .. }) => break,
                    Some(PaneEvent::Output { .. }) => {}
                    None => panic!("the pane ended without Exited"),
                }
            }
            let typed = std::fs::read(&file).unwrap_or_default();
            assert!(typed.is_empty(), "the dying child read {typed:?}");
        }

        /// Waits (up to 10 s) for the script to write a pid into `pidfile`.
        async fn read_pid(pidfile: &Path) -> String {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let pid = std::fs::read_to_string(pidfile).unwrap_or_default();
                let pid = pid.trim();
                if !pid.is_empty() {
                    return pid.to_owned();
                }
                assert!(Instant::now() < deadline, "the script never wrote its pid");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        /// Waits (up to 3 s) for `pid` to be gone.
        async fn gone(pid: &str) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while alive(pid) {
                assert!(Instant::now() < deadline, "{pid} outlived its pane");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        #[tokio::test]
        async fn a_hup_ignoring_grandchild_dies_with_the_pane() {
            // A wrapper that does not `exec`: the editor is a grandchild, in the child's process
            // group, and it ignores SIGHUP. It holds the slave, so while it lives the reader
            // never sees EOF.
            let dir = TempDir::new().unwrap();
            let pidfile = dir.path().join("pid");
            let mut pane = Pane::spawn(script(
                &dir,
                "sh -c 'trap \"\" HUP; printf %s $$ > \"$1\"; printf ready; exec sleep 30' sh \"$1\"; :",
                &pidfile,
            ));
            pane.until_shown("ready").await;
            let grandchild = read_pid(&pidfile).await;
            assert_ne!(
                pane.child.pid().map(|pid| pid.to_string()),
                Some(grandchild.clone())
            );
            assert!(alive(&grandchild), "{grandchild} is not running yet");
            pane.child.kill();
            let status = pane.exited(Duration::from_secs(3)).await.expect("reaped");
            assert!(!status.success(), "{status:?}");
            gone(&grandchild).await;
            // Every sender is gone: the reader saw EOF and the wait thread ended.
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
        async fn a_hup_ignoring_grandchild_dies_when_the_child_exits_on_its_own() {
            // The child exits by itself, leaving a job in its group that ignores the kernel's
            // SIGHUP and holds the slave: nothing calls `kill`, and the pane is still held.
            let dir = TempDir::new().unwrap();
            let pidfile = dir.path().join("pid");
            let mut pane = Pane::spawn(script(
                &dir,
                "sh -c 'trap \"\" HUP; printf %s $$ > \"$1\"; exec sleep 30' sh \"$1\" &\n\
                 while [ ! -s \"$1\" ]; do sleep 0.02; done; printf ready; exit 0",
                &pidfile,
            ));
            let status = pane.exited(Duration::from_secs(10)).await.expect("reaped");
            assert!(status.success(), "{status:?}");
            let grandchild = read_pid(&pidfile).await;
            gone(&grandchild).await;
            // Every sender is gone: the reader saw EOF and the wait thread ended.
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
        async fn a_large_write_to_a_stalled_child_never_blocks() {
            // PD-2 / R-NF-3: the child never reads, so the line discipline's queue fills after a
            // few KiB; `write` must still return at once.
            let dir = TempDir::new().unwrap();
            let pidfile = dir.path().join("pid");
            let mut pane = Pane::spawn(script(
                &dir,
                "printf '%s' $$ > \"$1\"; printf ready; exec sleep 30",
                &pidfile,
            ));
            pane.until_shown("ready").await;
            let pid = read_pid(&pidfile).await;
            // On a thread of its own, so a blocking `write` fails the test instead of hanging it.
            let Pane {
                mut child,
                rx,
                screen,
                id,
            } = pane;
            let (done_tx, done_rx) = mpsc::channel();
            thread::spawn(move || {
                let chunk = vec![b'x'; 2 * 1024 * 1024];
                let started = Instant::now();
                child.write(&chunk);
                child.write(&chunk);
                let _ = done_tx.send((child, started.elapsed()));
            });
            let (child, took) = done_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("write blocked on a stalled child");
            assert!(took < Duration::from_secs(1), "write took {took:?}");
            let mut pane = Pane {
                child,
                rx,
                screen,
                id,
            };

            // The writer is stuck in the master write. Kill, then drop: every thread ends.
            pane.child.kill();
            let status = pane.exited(Duration::from_secs(3)).await.expect("reaped");
            assert!(!status.success(), "{status:?}");
            gone(&pid).await;
            let threads = std::mem::take(&mut pane.child.threads);
            assert_eq!(threads.len(), 3);
            drop(pane);
            let deadline = Instant::now() + Duration::from_secs(3);
            while !threads.iter().all(thread::JoinHandle::is_finished) {
                assert!(Instant::now() < deadline, "a pane thread outlived the pane");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
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
