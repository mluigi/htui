//! The remote scripts (MOD-45 D297): POSIX `sh`, each starting with `set -eu`, with neither a
//! backslash nor a single quote anywhere (E-3, E-5), so `shell_words::quote` wraps each in a plain
//! `'…'` and the whole command line the remote login shell parses is backslash-free. Values arrive
//! only as positional arguments, the root prefix always first (`""` in production; a temporary
//! directory under test, D309). The DSN and the sudo password are never arguments.
//!
//! Exit codes a script chooses: 3, the uploaded binary does not run (PREPARE); 4, sudo refused
//! the password (INSTALL); 5, the service did not come up (VERIFY). Anything else is the failing
//! command's own status.

/// `$0` of every script but [`INSTALL_ROOT`], so a remote error line reads `htui-provision: …`.
pub const ARG0: &str = "htui-provision";
/// The marker [`INSTALL_ROOT`] prints first: its absence on a failed INSTALL means sudo refused (E-7).
pub const ROOT_MARKER: &str = "htui.root=start";
/// The markers around `box.toml` in [`VERIFY`]'s output.
pub const BOX_BEGIN: &str = "htui.box.begin";
/// See [`BOX_BEGIN`].
pub const BOX_END: &str = "htui.box.end";

/// Read-only facts about the host (D299): `htui.<key>=<value>` lines, nothing else written.
/// `$1` root. Never fails on a missing tool: a missing tool is a value (`none`, `no`, `unknown`).
pub const PREFLIGHT: &str = "";

/// Unprivileged (D304): the log directory, then, with `send=1`, the binary on stdin, which must
/// run `--version` before it replaces anything. `$1` root (unused), `$2` home, `$3` send (`0`|`1`).
/// Exit 3: the uploaded binary does not run here; the temporary file is removed.
pub const PREPARE: &str = "";

/// Unprivileged wrapper around the privileged part (D301; E-3, E-4, E-7, E-8). `$1` root, `$2`
/// user, `$3` home, `$4` replace (`0`|`1`), `$5` mode (`password`|`nopasswd`), `$6` the text of
/// [`INSTALL_ROOT`]. Stdin: with `mode=password`, the password line, then the DSN line; otherwise
/// the DSN line alone. Exit 4: sudo refused the password; the DSN was never read.
pub const INSTALL: &str = "";

/// Runs as root under `sudo -n` (D303; E-7). `$1` root, `$2` user, `$3` home, `$4` replace. Stdin:
/// the DSN line, encrypted when the credential is absent or `replace=1`, otherwise discarded.
/// Credential, then unit, then start. Its first line of output is the marker `htui.root=start`.
pub const INSTALL_ROOT: &str = "";

/// Waits inside one session (D305; E-10): up to `tries` checks, `pause` seconds apart, until the
/// service is `active` and `box.toml` exists, then prints both between markers. `$1` root
/// (unused), `$2` home, `$3` tries, `$4` pause. Exit 5: it never did; the journal and log tails
/// go to stderr.
pub const VERIFY: &str = "";

/// `sh -c '<script>' htui-provision '<arg>'…`, every piece through `shell_words::quote`. The
/// remote login shell parses it once (D297, V-7).
#[must_use]
pub fn remote_command(script: &str, args: &[&str]) -> String {
    let _ = (script, args);
    todo!()
}
