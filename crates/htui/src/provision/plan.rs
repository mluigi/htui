//! What to do on a host, decided from its [`Facts`] alone (MOD-45 D300), and the value checks
//! that make the remote values safe in a unit file (D296). Pure: no I/O.

use crate::provision::preflight::Facts;

/// How INSTALL gets root (D301).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SudoMode {
    /// `sudo -k -n true` failed: the password is read here and sent as INSTALL's first line.
    Password,
    /// NOPASSWD: INSTALL's stdin is the DSN line alone.
    NoPassword,
}

/// The local build being shipped, as `decide` compares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFacts {
    /// `std::env::consts::ARCH`.
    pub arch: String,
    /// Lowercase hex sha256 of the payload.
    pub sha: String,
}

/// `decide`'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Refuse with this sentence; nothing has been written.
    Refuse(String),
    /// This build, its unit, running, and no `--replace-credential`: nothing to do.
    AlreadyProvisioned,
    /// Run PREPARE (uploading iff `upload`) and INSTALL with `sudo`.
    Steps {
        /// The remote binary's hash differs from the payload's.
        upload: bool,
        /// How INSTALL gets root.
        sudo: SudoMode,
    },
}

/// `LoadCredentialEncrypted=` and `systemd-creds` arrived in 250 (V-1).
pub const MIN_SYSTEMD: u32 = 250;

/// D300, in order: refusals (OS, architecture, systemd, `systemd-creds`, sudo), the
/// different-build refusal, already provisioned, else the steps.
#[must_use]
pub fn decide(facts: &Facts, local: &LocalFacts, replace_credential: bool) -> Decision {
    let _ = (facts, local, replace_credential);
    todo!()
}

/// `arm64` → `aarch64`; anything else unchanged (D293).
#[must_use]
pub fn normalise_arch(arch: &str) -> &str {
    let _ = arch;
    todo!()
}

/// D296: non-empty, no leading `-`, no whitespace or control character.
///
/// # Errors
///
/// The refusal sentence (it never repeats the destination).
pub fn validate_destination(destination: &str) -> Result<(), String> {
    let _ = destination;
    todo!()
}

/// D296: `^[a-z_][a-z0-9_-]{0,31}$`; `field` is `user` or `group`.
///
/// # Errors
///
/// The refusal sentence, naming `field`.
pub fn validate_account(field: &'static str, name: &str) -> Result<(), String> {
    let _ = (field, name);
    todo!()
}

/// D296: absolute, bytes in `[A-Za-z0-9_./-]`, no `..`.
///
/// # Errors
///
/// The refusal sentence.
pub fn validate_home(home: &str) -> Result<(), String> {
    let _ = home;
    todo!()
}
