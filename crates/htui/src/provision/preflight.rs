//! The preflight's answer (MOD-45 D299): `htui.<key>=<value>` lines. Every other line (a login
//! script's greeting, `~/.bashrc` noise, V-7) is ignored, and so is an `htui.` key this build
//! does not know.

use std::fmt;

use crate::provision::plan::SudoMode;

/// The eleven keys, in the order [`crate::provision::script::PREFLIGHT`] prints them.
pub const KEYS: [&str; 11] = [
    "os", "arch", "systemd", "creds", "user", "group", "home", "bin_sha", "unit", "active", "sudo",
];

/// What the remote host said about itself. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// `uname -s`.
    pub os: String,
    /// `uname -m`, as printed (normalised by `plan::normalise_arch`).
    pub arch: String,
    /// The major version from `systemctl --version`; `None` for `none`.
    pub systemd: Option<u32>,
    /// `systemd-creds` is on `PATH`.
    pub creds: bool,
    /// `id -un`.
    pub user: String,
    /// `id -gn`.
    pub group: String,
    /// `$HOME`.
    pub home: String,
    /// `sha256sum` of `~/.local/bin/htui`; `None` for `none`. `unknown` stays `Some("unknown")`
    /// and matches no local hash.
    pub bin_sha: Option<String>,
    /// `/etc/systemd/system/htui-worker.service` exists (under the root prefix).
    pub unit: bool,
    /// `systemctl is-active htui-worker.service`, as printed (`unknown` without systemctl).
    pub active: String,
    /// `None` for `none`: sudo is not installed (E-9).
    pub sudo: Option<SudoMode>,
}

/// Why the answer is not a preflight. Each variant names its key; none carries a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactsError {
    /// No `htui.` key at all: the login shell did not run the script (D297).
    NoPreflight,
    /// A key is absent.
    Missing(&'static str),
    /// A key came twice.
    Duplicate(&'static str),
    /// A key's value is not one this build reads (`systemd=25x`, `unit=maybe`, `sudo=?`).
    Unreadable(&'static str),
}

impl fmt::Display for FactsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPreflight => f.write_str(
                "the remote login shell did not run the preflight; htui provision needs a POSIX \
                 login shell (sh, bash, zsh, ksh)",
            ),
            Self::Missing(k) => write!(f, "the preflight did not report `{k}`"),
            Self::Duplicate(k) => write!(f, "the preflight reported `{k}` twice"),
            Self::Unreadable(k) => write!(f, "the preflight reported an unreadable `{k}`"),
        }
    }
}

impl std::error::Error for FactsError {}

impl Facts {
    /// Parses `stdout`; lines may end in `\r`; the value is everything after the first `=`.
    ///
    /// # Errors
    ///
    /// [`FactsError`], first by line order for duplicates, then by [`KEYS`] order for absences.
    pub fn parse(stdout: &str) -> Result<Self, FactsError> {
        let _ = stdout;
        todo!()
    }
}
