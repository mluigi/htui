//! Terminal lifetime (plan D8).
//!
//! Two independent guarantees that the terminal is given back: a [`Drop`] impl for the normal and
//! the error path, and a panic hook for the third one. The PRD's "terminal left raw after a
//! panic" risk is closed by having both, not by remembering to call `restore`.
//!
//! The hook is a *decision* — [`restores_the_terminal`] may answer `false` — and a decision only
//! counts if it is the outermost hook in the process. So [`init`] builds the terminal itself
//! instead of calling `ratatui::init`, which installs a hook of its own that restores the
//! terminal unconditionally *before* calling the one it wrapped (`ratatui-0.30.2/src/init.rs:398`,
//! `:566-572`); behind that one, this predicate is an opinion with no way to act on it (MOD-56
//! D217). Nothing here may take a ratatui init again, and `Suspend::enter` has to say so for
//! itself: the temptation arrives mid-session, with a terminal already up and no ratatui init
//! running to defer to.
//!
//! **Bracketed paste** (MOD-22 review M-1) is on for exactly as long as the alternate screen is:
//! [`init`] and `Suspend::enter` turn it on with the screen, and every way the terminal is given
//! back — [`restore_terminal`] (the panic hook and [`TerminalGuard`]'s restore) and
//! `Suspend::leave` — turns it off first. With it on, a paste is one `Event::Paste` the shell
//! routes to a capturing field or drops; without it, a paste is replayed as keystrokes and runs as
//! commands wherever no field is open.
//!
//! **Mouse capture** (MOD-71 D1-D3) is on only while the view on screen wants the mouse, because
//! it takes the terminal's own text selection away. The event loop asks the app after every step,
//! and [`TerminalGuard::set_mouse_capture`] writes only a change. [`init`] and `Suspend::enter`
//! never turn it on; every way the terminal is given back turns it off first: [`restore_terminal`]
//! unconditionally and best effort, `Suspend::leave` when the guard turned it on (review H1).
//! Capture is button-event reporting (`?1000h ?1002h ?1015h ?1006h`, MOD-74 D5): there is no
//! any-motion mode (`?1003h`), so the terminal sends no hover stream. Windows keeps crossterm's
//! WinAPI path.

use ratatui::{DefaultTerminal, Terminal, backend::CrosstermBackend};

/// Owns the terminal for as long as the shell runs.
#[derive(Debug)]
pub struct TerminalGuard {
    /// The terminal the event loop draws on.
    terminal: DefaultTerminal,
    /// Whether the raw mode and the alternate screen have already been given back.
    restored: bool,
    /// MOD-71 D2: whether mouse capture is on, as this guard last set it. Only
    /// [`TerminalGuard::set_mouse_capture`] turns it on; `Suspend::leave` clears it (D3).
    mouse: bool,
}

/// Installs the panic hook and takes the terminal over.
///
/// Panics if the terminal cannot be put into raw mode, cannot be given the alternate screen, cannot
/// take bracketed paste for a reason other than not supporting it, or cannot be measured: the
/// four `expect`s below. That is [`ratatui::init()`]'s contract — it is `try_init().expect(...)`
/// — now covering the steps this crate performs itself. There is no usable TUI in any of these
/// cases and `lib.rs::run` has no error path for one. A terminal that does not support bracketed
/// paste (crossterm's legacy Windows console answers `Unsupported`) is not one of them: the TUI
/// runs without paste mode there (review R2-L6).
///
/// The panic is safe because of the order, which is the whole of MOD-56 (D217): the hook goes in
/// before the first `expect`, so it is already the outermost one in the process when any of the
/// three fires, and the restore runs ahead of the unwind reaching the default hook underneath it.
#[must_use]
pub fn init() -> TerminalGuard {
    install_panic_hook();
    // `ratatui::try_init`'s body, minus its `set_panic_hook` (`init.rs:398-402`). Written out
    // because the order is the fix: the hook above is the outermost one from here to the end of
    // the process, and a second one — restoring unconditionally ahead of it — would put this
    // crate back where MOD-56 found it, with the predicate right and the terminal gone anyway.
    // `tests/panic_hook_order.rs` is what keeps this shape.
    crossterm::terminal::enable_raw_mode().expect("htui cannot put the terminal into raw mode");
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)
        .expect("htui cannot enter the alternate screen");
    enable_bracketed_paste().expect("htui cannot enable bracketed paste");
    let backend = CrosstermBackend::new(std::io::stdout());
    TerminalGuard {
        terminal: Terminal::new(backend).expect("htui cannot measure the terminal"),
        restored: false,
        // MOD-71 D1: capture starts off; the loop turns it on for a view that wants it.
        mouse: false,
    }
}

/// Chains a hook that restores the terminal before the previous hook prints the panic.
///
/// Without it a panic leaves the alternate screen up and raw mode on, and the backtrace is drawn
/// over the TUI's last frame.
pub fn install_panic_hook() {
    install_panic_hook_restoring(restore_terminal);
}

/// Gives the terminal back: mouse capture off (MOD-71 D3), bracketed paste off (MOD-22 review
/// M-1), then `ratatui::restore` — raw mode off and the alternate screen left. Best effort, as
/// `ratatui::restore` is: a stdout that cannot take one escape sequence is not a reason to stop
/// giving the rest back. A free function, so it cannot know whether capture is on: it turns it
/// off unconditionally, which a terminal that never had it ignores.
pub fn restore_terminal() {
    let _ = disable_mouse_capture();
    let _ = disable_bracketed_paste();
    ratatui::restore();
}

/// The hook, with the restore it performs as a parameter.
///
/// [`install_panic_hook`] is this with [`restore_terminal`], which is the only restore this crate
/// ever wants. The seam is public for one reason: the test that has to prove the *decision*
/// reaches the restore runs the real chain, and it is an integration test in another binary that
/// cannot see a private function (MOD-56 D218). Before the seam it could only assert
/// [`restores_the_terminal`], which is what made the defect invisible: the predicate was right
/// and the terminal was still torn down underneath the event loop, by a hook outside it.
pub fn install_panic_hook_restoring(restore: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if restores_the_terminal() {
            restore();
        }
        previous(info);
    }));
}

/// Whether a panic reaching the hook *right now* should give the terminal back.
///
/// Review finding M1. The hook fires on **any** panic on **any** thread, including one that a
/// `catch_unwind` further down the stack is about to swallow — `catch_unwind` does not stop the
/// hook, it only stops the unwind. `htui_agent::excerpt::run_providers` catches exactly such a
/// panic on purpose (hazard H-20: a provider that panics is dropped and recorded, and the prompt
/// is assembled without it), so restoring there left the process with no alternate screen and no
/// raw mode while the event loop carried on drawing — the wedged-UI shape, caused by a defect the
/// program had already decided to survive.
///
/// The question is asked of the agent crate because that is the crate that does the catching; this
/// one does not get to guess which panics are fatal. Every other panic is fatal, so the answer is
/// `true` for all of a normal run.
#[must_use]
pub fn restores_the_terminal() -> bool {
    !htui_agent::excerpt::panic_is_contained()
}

impl TerminalGuard {
    /// The terminal, for drawing.
    pub fn terminal_mut(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }

    /// Gives the terminal back. Idempotent: [`Drop`] calls it again and it does nothing.
    pub fn restore(&mut self) {
        if !self.restored {
            restore_terminal();
            self.restored = true;
        }
    }

    /// MOD-71 D2: mouse capture on or off, written only when `on` differs from what this guard
    /// last set, so the event loop can ask after every step for nothing. The loop is the only
    /// caller, with `App::mouse_capture` (MOD-74 D1), which also tells the tabs when capture goes
    /// off. A terminal without mouse reporting answers `Unsupported`, which is recorded as done so
    /// it is not asked again every step: the view runs keyboard-only (review R2-L6's shape,
    /// blueprint H-14).
    ///
    /// # Errors
    ///
    /// A terminal write that failed for any other reason: the loop ends and `lib.rs` restores
    /// (MOD-9 D21).
    pub fn set_mouse_capture(&mut self, on: bool) -> std::io::Result<()> {
        if on == self.mouse {
            return Ok(());
        }
        if on {
            enable_mouse_capture()?;
        } else {
            disable_mouse_capture()?;
        }
        self.mouse = on;
        Ok(())
    }
}

impl crate::editor::Suspend for TerminalGuard {
    /// Show the cursor (every draw hid it), mouse capture off (MOD-71 D3) and bracketed paste off
    /// so the editor gets its own mouse and paste (MOD-22 review M-1), then `ratatui::try_restore`
    /// (MOD-9 D22). `restored` is not touched: this is a pause, not the end. The loop's next
    /// `set_mouse_capture` re-asserts what the app wants once the editor is gone — one place
    /// decides.
    ///
    /// Capture is turned off only when this guard turned it on (review H1): on Windows the disable
    /// goes through the console API, which errors — not `Unsupported` — when no enable ever
    /// ran, and would fail every editor handoff before the flow view was first opened. `mouse` is
    /// cleared only once the write succeeded (review L1), so the flag never says off while the
    /// terminal is still captured.
    fn leave(&mut self) -> std::io::Result<()> {
        self.terminal.show_cursor()?;
        if self.mouse {
            disable_mouse_capture()?;
            self.mouse = false;
        }
        disable_bracketed_paste()?;
        ratatui::try_restore()
    }

    /// Raw mode and the alternate screen back, then `Terminal::clear`, which resets the back
    /// buffer so the next draw is whole (`ratatui-core-0.1.2/src/terminal/buffers.rs:147-173`).
    /// Not `ratatui::init()`, and now for two reasons: it would stack another panic hook
    /// (`init.rs:398`), and since MOD-56 the hook it would wrap is the one that decides whether a
    /// panic gives the terminal back at all — so on every editor suspend it would re-break the
    /// defect this file just closed (D221). Mouse capture is not turned back on here (MOD-71 D3):
    /// the loop's next `set_mouse_capture` decides, in one place.
    fn enter(&mut self) -> std::io::Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        enable_bracketed_paste()?;
        self.terminal.clear()
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Bracketed paste on, in its own `execute!` (review R2-L6): a terminal that does not support it
/// runs without it rather than failing the TUI. Without paste mode a paste is replayed as keys,
/// as it was before MOD-22's review M-1 — so the one thing said is that, and nothing of the
/// terminal's own error text.
fn enable_bracketed_paste() -> std::io::Result<()> {
    let enabled = tolerate_unsupported(crossterm::execute!(
        std::io::stdout(),
        crossterm::event::EnableBracketedPaste
    ))?;
    if !enabled {
        tracing::info!("this terminal has no bracketed paste; a paste arrives as keystrokes");
    }
    Ok(())
}

/// Bracketed paste off, in its own `execute!`, `Unsupported` tolerated as on the way in.
fn disable_bracketed_paste() -> std::io::Result<()> {
    tolerate_unsupported(crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableBracketedPaste
    ))
    .map(|_| ())
}

/// MOD-71 D2: mouse capture on, in its own `execute!` (review R2-L6's shape). It issues
/// `EnableButtonMouseCapture` (MOD-74 D5), crossterm's `EnableMouseCapture` minus any-motion
/// `?1003h`, so the terminal sends no hover stream. `App::on_mouse` still drops `Moved`: Windows
/// reports it, and a terminal may ignore the narrower mode. `Unsupported` is a terminal with no
/// mouse reporting, which runs keyboard-only.
fn enable_mouse_capture() -> std::io::Result<()> {
    let enabled = tolerate_unsupported(crossterm::execute!(
        std::io::stdout(),
        EnableButtonMouseCapture
    ))?;
    if !enabled {
        tracing::debug!("this terminal has no mouse reporting; the flow view is keyboard-only");
    }
    Ok(())
}

/// Mouse capture off, in its own `execute!`, `Unsupported` tolerated as on the way in (MOD-71 D3).
fn disable_mouse_capture() -> std::io::Result<()> {
    tolerate_unsupported(crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture
    ))
    .map(|_| ())
}

/// `Ok(true)` when the command ran, `Ok(false)` when the terminal does not support it, and every
/// other error as it came.
fn tolerate_unsupported(result: std::io::Result<()>) -> std::io::Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::Unsupported => Ok(false),
        Err(err) => Err(err),
    }
}

/// MOD-74 D5: crossterm's `EnableMouseCapture` (`crossterm-0.29.0/src/event.rs:321-345`) minus
/// any-motion `?1003h`: press/release (`?1000h`), drag (`?1002h`), RXVT coordinates past 223
/// (`?1015h`, kept for terminals without SGR) and SGR (`?1006h`). `DisableMouseCapture` clears
/// every one of them. On Windows it is crossterm's own WinAPI command, `is_ansi_code_supported`
/// false as crossterm's is, so the console path never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableButtonMouseCapture;

impl crossterm::Command for EnableButtonMouseCapture {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        crossterm::Command::execute_winapi(&crossterm::event::EnableMouseCapture)
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    /// What a command writes to an ANSI terminal.
    fn ansi(command: impl crossterm::Command) -> String {
        let mut out = String::new();
        command
            .write_ansi(&mut out)
            .expect("a String takes any write");
        out
    }

    /// This file with its comments removed, as `tests/panic_hook_order.rs` reads it.
    fn code() -> String {
        include_str!("terminal.rs")
            .lines()
            .map(|line| line.find("//").map_or(line, |at| &line[..at]))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The body of the item that starts at `head`, up to the next item at the same indent.
    fn body<'a>(code: &'a str, head: &str) -> &'a str {
        let start = code
            .find(head)
            .unwrap_or_else(|| panic!("`{head}` is in terminal.rs"));
        let rest = &code[start..];
        let end = ["\n    fn ", "\npub fn ", "\nfn ", "\nimpl "]
            .iter()
            .filter_map(|next| rest[1..].find(next))
            .min()
            .map_or(rest.len(), |at| at + 1);
        &rest[..end]
    }

    /// MOD-22 review M-1: bracketed paste is on wherever the alternate screen is taken and off
    /// wherever the terminal is given back. A paste replayed as keystrokes runs as commands, so a
    /// path that forgot either half is the credential leak this closes.
    #[test]
    fn every_path_that_takes_the_terminal_enables_bracketed_paste_and_every_restore_disables_it() {
        let code = code();
        for taking in ["pub fn init()", "fn enter(&mut self)"] {
            assert!(
                body(&code, taking).contains("enable_bracketed_paste()"),
                "`{taking}` takes the screen without bracketed paste"
            );
        }
        for giving in ["pub fn restore_terminal()", "fn leave(&mut self)"] {
            assert!(
                body(&code, giving).contains("disable_bracketed_paste()"),
                "`{giving}` gives the terminal back with bracketed paste still on"
            );
        }
        // Review R2-L6: each in its own `execute!`, so an unsupported paste mode cannot take the
        // alternate screen down with it.
        for (helper, command) in [
            ("fn enable_bracketed_paste()", "EnableBracketedPaste"),
            ("fn disable_bracketed_paste()", "DisableBracketedPaste"),
        ] {
            let body = body(&code, helper);
            assert!(body.contains(command), "`{helper}` issues `{command}`");
            assert!(
                body.contains("tolerate_unsupported("),
                "`{helper}` tolerates an unsupported terminal"
            );
            assert!(
                !body.contains("AlternateScreen"),
                "`{helper}` issues nothing else"
            );
        }
        assert!(
            body(&code, "pub fn install_panic_hook()").contains("restore_terminal"),
            "the panic hook restores through `restore_terminal`"
        );
        assert!(
            body(&code, "pub fn restore(&mut self)").contains("restore_terminal()"),
            "the guard restores through `restore_terminal`"
        );
    }

    /// MOD-71 D3: capture is the loop's alone to turn on, and every way the terminal is given back
    /// turns it off first: `restore_terminal` whatever the guard last set, `leave` when the guard
    /// set it (review H1). A path that forgot leaves the shell, or `$EDITOR`, printing an escape
    /// sequence for every mouse move.
    #[test]
    fn every_give_back_disables_mouse_capture_and_only_the_loop_enables_it() {
        let code = code();
        let restore = body(&code, "pub fn restore_terminal()");
        let off = restore
            .find("disable_mouse_capture()")
            .expect("`restore_terminal` turns capture off");
        let paste = restore
            .find("disable_bracketed_paste()")
            .expect("`restore_terminal` turns paste off");
        assert!(off < paste, "capture goes first (D3)");
        // Review H1, L1: `leave` writes the disable only for a capture this guard turned on (a
        // Windows console that never had it errors, not `Unsupported`), and forgets it only once
        // the write succeeded.
        let leave = body(&code, "fn leave(&mut self)");
        let guard = leave
            .find("if self.mouse {")
            .expect("`leave` asks the guard first");
        let off = leave
            .find("disable_mouse_capture()?")
            .expect("`leave` turns capture off");
        let forget = leave
            .find("self.mouse = false")
            .expect("`leave` forgets it");
        let paste = leave
            .find("disable_bracketed_paste()")
            .expect("`leave` turns paste off");
        assert!(
            guard < off && off < forget && forget < paste,
            "guard, write, forget, then paste: {leave}"
        );
        for taking in ["pub fn init()", "fn enter(&mut self)"] {
            let body = body(&code, taking);
            assert!(
                !body.contains("MouseCapture") && !body.contains("mouse_capture"),
                "`{taking}` leaves capture to the loop"
            );
        }
        // Review nit: the toggle returns on no change, writes, and records only after the write.
        let toggle = body(&code, "pub fn set_mouse_capture(&mut self, on: bool)");
        let same = toggle
            .find("if on == self.mouse {")
            .expect("the toggle writes only a change");
        let on = toggle
            .find("enable_mouse_capture()?")
            .expect("the toggle turns capture on");
        let off = toggle
            .find("disable_mouse_capture()?")
            .expect("the toggle turns capture off");
        let recorded = toggle
            .find("self.mouse = on;")
            .expect("the toggle records what it wrote");
        assert!(
            same < on.min(off) && on.max(off) < recorded,
            "no change returns, then the write, then the record: {toggle}"
        );
        // MOD-74 D5: on, button-only reporting; off, crossterm's superset, which also clears
        // any-motion.
        assert!(
            !body(&code, "fn enable_mouse_capture()").contains("event::EnableMouseCapture"),
            "the enable issues button-only capture, not crossterm's any-motion command"
        );
        for (helper, command) in [
            ("fn enable_mouse_capture()", "EnableButtonMouseCapture"),
            ("fn disable_mouse_capture()", "DisableMouseCapture"),
        ] {
            let body = body(&code, helper);
            assert!(body.contains(command), "`{helper}` issues `{command}`");
            assert!(
                body.contains("tolerate_unsupported("),
                "`{helper}` tolerates an unsupported terminal"
            );
            assert!(
                !body.contains("AlternateScreen") && !body.contains("BracketedPaste"),
                "`{helper}` issues nothing else"
            );
        }
    }

    /// MOD-74 D5: presses, drags and releases, with RXVT and SGR coordinates; no any-motion
    /// `?1003h`, so the terminal sends no hover stream.
    #[test]
    fn button_capture_reports_presses_drags_and_releases_and_no_motion() {
        let written = ansi(super::EnableButtonMouseCapture);
        assert_eq!(written, "\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h");
        assert!(!written.contains("?1003h"), "no any-motion: {written:?}");
    }

    /// MOD-74 D5: every give-back stays crossterm's `DisableMouseCapture`, so every mode the
    /// button-only enable sets must be one it clears.
    #[test]
    fn the_disable_clears_every_mode_button_capture_sets() {
        let enable = ansi(super::EnableButtonMouseCapture);
        let disable = ansi(crossterm::event::DisableMouseCapture);
        let modes: Vec<&str> = enable
            .split("\x1b[")
            .filter(|mode| !mode.is_empty())
            .collect();
        assert_eq!(modes.len(), 4, "four modes: {enable:?}");
        for mode in modes {
            let number = mode
                .strip_suffix('h')
                .unwrap_or_else(|| panic!("`{mode}` turns a mode on"));
            assert!(
                disable.contains(&format!("\x1b[{number}l")),
                "`{number}` is never cleared: {disable:?}"
            );
        }
    }

    /// Review R2-L6: crossterm's legacy Windows console answers `Unsupported` for bracketed paste;
    /// that is a terminal without paste mode, not a failure. Any other error still is one.
    #[test]
    fn an_unsupported_paste_mode_is_tolerated_and_nothing_else_is() {
        use std::io::{Error, ErrorKind};
        assert!(matches!(super::tolerate_unsupported(Ok(())), Ok(true)));
        assert!(matches!(
            super::tolerate_unsupported(Err(Error::new(ErrorKind::Unsupported, "legacy console"))),
            Ok(false)
        ));
        let other = super::tolerate_unsupported(Err(Error::new(ErrorKind::BrokenPipe, "gone")));
        assert_eq!(
            other.map_err(|err| err.kind()).err(),
            Some(ErrorKind::BrokenPipe)
        );
    }
}
