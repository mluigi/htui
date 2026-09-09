//! Plan MOD-21 D16 and D17: the two halves of "who opens the browser".
//!
//! **The adapter must not.** The observed hijack (the PRD's evidence) is an adapter whose own
//! opener, on a box with no display, fell through to a terminal browser — which wrote alt-screen
//! sequences into the JSON-RPC channel it shares with `htui` and killed the login with a decode
//! error nobody could act on. [`BrowserPolicy::apply`] puts one variable in front of the login's
//! own child, and `tests/auth.rs`'s regression pair is that hijack with and without it.
//!
//! **`htui` does, and only when the user asks.** [`open_url`] is what the pane's `o` key reaches:
//! a plain [`tokio::process::Command`] with all three streams at `/dev/null`, spawned and let go.
//! Never through [`crate::launch::spawn`] — an opener made a process-group leader `htui` owns is
//! an opener `htui` can kill, and `xdg-open` may have exec'd the user's real browser as its own
//! child (H-4).
//!
//! **Windows is written here and verified by MOD-16** (plan D22). The `cfg(windows)` arms below
//! are reviewed by eye and lint-checked on no box: this workspace cannot build the MSVC target
//! (TOOL-3), so there is no `--target x86_64-pc-windows-msvc` line in this item anywhere. What is
//! deferred to MOD-16 by name: whether any Windows opener honours `BROWSER` at all, whether
//! `cmd.exe /c exit 0` is a value one accepts, and that `Start-Process` reaches the default
//! browser from a non-interactive PowerShell started with `CREATE_NO_WINDOW`. The PowerShell
//! spelling itself is borrowed verbatim from the `open 5.4.3` crate, which is not a dependency:
//! `unsafe_code = "forbid"` rules out `ShellExecuteW`, and putting the URL in the environment is
//! the one spelling that never quotes an OAuth link full of `&` and `%` onto a command line.

use std::process::Stdio;

use tokio::process::Command;

use crate::auth::{BrowserPolicy, OpenerCommand};
use crate::error::{DriverError, Result};
use crate::launch::ResolvedLaunch;

/// The variable an opener reads before it starts guessing (plan D16).
const BROWSER_VAR: &str = "BROWSER";

/// Plan D17's Windows spelling: the URL travels in the child's environment, never on a command
/// line something downstream would have to re-parse.
#[cfg(windows)]
const OPEN_URL_VAR: &str = "HTUI_OPEN_URL";

impl BrowserPolicy {
    /// Writes this policy into the login spawn's environment.
    ///
    /// [`Neutralised`](BrowserPolicy::Neutralised) inserts exactly one variable, `BROWSER`, whose
    /// value is a no-op that **exists and exits 0**: that is what makes a chain-style opener stop
    /// rather than fall through to the next candidate, and a value naming nothing would be worse
    /// than no policy at all. [`Inherit`](BrowserPolicy::Inherit) writes nothing.
    ///
    /// `DISPLAY` and `WAYLAND_DISPLAY` are left exactly as inherited either way — clearing them
    /// would send an opener looking for a terminal browser, which is the failure this exists to
    /// prevent.
    ///
    /// Applied to the login's own copy of the launch, which dies with the flow (blueprint H-11):
    /// nothing here reaches a chat's spawn, the probe's, or `agent_box.probe.resolved`.
    pub fn apply(self, launch: &mut ResolvedLaunch) {
        match self {
            Self::Neutralised => {
                launch.env.insert(BROWSER_VAR.to_owned(), neutraliser());
            }
            Self::Inherit => {}
        }
    }
}

/// The no-op an opener is meant to stop at.
///
/// Resolved rather than spelled: the same program lives in different places on the two unixes this
/// runs on, and a hard-coded path that is wrong on one of them is a hijack on that one. The lookup
/// is a handful of `stat`s and runs on the login task, never on the worker loop.
#[cfg(unix)]
fn neutraliser() -> String {
    which::which("true")
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "true".to_owned())
}

/// The Windows value — written here, verified by MOD-16 (plan D22, module doc).
#[cfg(windows)]
fn neutraliser() -> String {
    "cmd.exe /c exit 0".to_owned()
}

/// Opens `url` in whatever this box calls a browser, and returns without waiting for it.
///
/// The scheme is checked **before** anything is spawned: the scan that produced this string
/// (`crate::auth::url::first_url`) admits only `http` and `https`, and this is the second lock on
/// the same door — a `file:` or `javascript:` link that reached an opener would already have won.
///
/// The child gets `/dev/null` on all three streams (H-3: an opener on a display-less box may
/// itself fall through to a terminal browser, and one with no terminal anywhere dies at `initscr`
/// instead of seizing `htui`'s screen). It is handed to a spawned `wait()` so the operating system
/// reaps it, and it is never killed: `xdg-open` may exec the user's real browser as its own child,
/// and a timeout that tidied up would close the browser the human is logging in with.
///
/// [`OpenerCommand::Platform`] is `xdg-open` on unix, `open` on macOS, and PowerShell's
/// `Start-Process` on Windows; [`OpenerCommand::Custom`] is the program itself, with the URL as its
/// one argument.
///
/// # Errors
/// [`DriverError::Transport`] for any scheme but `http` or `https`, before a process exists;
/// [`DriverError::Spawn`] naming the program when the operating system refuses the spawn.
pub async fn open_url(url: &str, opener: &OpenerCommand) -> Result<()> {
    match scheme_of(url).as_deref() {
        Some("http" | "https") => {}
        Some(other) => {
            return Err(DriverError::Transport(format!(
                "only http and https links are opened; refused `{other}`"
            )));
        }
        None => {
            return Err(DriverError::Transport(
                "only http and https links are opened; refused a link with no scheme".to_owned(),
            ));
        }
    }

    let (program, mut command) = opener_command(url, opener);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| DriverError::Spawn(format!("`{program}`: {error}")))?;
    // Reaped, not supervised: nothing here holds the child, so nothing here can kill it.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

/// The scheme of `url`, lowercased, or `None` when it has none.
///
/// `split_once` rather than a parse: everything before the first `://` is the scheme by every
/// definition this needs, and the two names it is compared against contain no surprises.
fn scheme_of(url: &str) -> Option<String> {
    url.split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
}

/// The command to spawn and the name to put in a spawn failure.
fn opener_command(url: &str, opener: &OpenerCommand) -> (String, Command) {
    match opener {
        OpenerCommand::Platform => platform_command(url),
        OpenerCommand::Custom(program) => {
            let mut command = Command::new(program);
            command.arg(url);
            (program.to_string_lossy().into_owned(), command)
        }
    }
}

/// macOS's opener.
#[cfg(target_os = "macos")]
fn platform_command(url: &str) -> (String, Command) {
    let program = "open";
    let mut command = Command::new(program);
    command.arg(url);
    (program.to_owned(), command)
}

/// The freedesktop opener, which every unix but macOS uses.
#[cfg(all(unix, not(target_os = "macos")))]
fn platform_command(url: &str) -> (String, Command) {
    let program = "xdg-open";
    let mut command = Command::new(program);
    command.arg(url);
    (program.to_owned(), command)
}

/// Windows — written here, verified by MOD-16 (plan D22, module doc).
///
/// [`crate::launch::CREATE_NO_WINDOW`] rather than a second literal: a TUI must not flash a console
/// window, and a constant spelled twice is a constant that can drift.
#[cfg(windows)]
fn platform_command(url: &str) -> (String, Command) {
    let program = "powershell.exe";
    let mut command = Command::new(program);
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(format!("Start-Process -FilePath $env:{OPEN_URL_VAR}"))
        .env(OPEN_URL_VAR, url)
        .creation_flags(crate::launch::CREATE_NO_WINDOW);
    (program.to_owned(), command)
}
