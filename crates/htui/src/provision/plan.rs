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

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A fresh host: nothing installed, a password sudo.
    fn fresh() -> Facts {
        Facts {
            os: "Linux".into(),
            arch: "x86_64".into(),
            systemd: Some(255),
            creds: true,
            user: "alice".into(),
            group: "alice".into(),
            home: "/home/alice".into(),
            bin_sha: None,
            unit: false,
            active: "inactive".into(),
            sudo: Some(SudoMode::Password),
        }
    }

    /// A host already running this build.
    fn provisioned() -> Facts {
        Facts {
            bin_sha: Some(SHA.into()),
            unit: true,
            active: "active".into(),
            ..fresh()
        }
    }

    fn local(arch: &str) -> LocalFacts {
        LocalFacts {
            arch: arch.into(),
            sha: SHA.into(),
        }
    }

    fn refuse(sentence: &str) -> Decision {
        Decision::Refuse(sentence.into())
    }

    fn steps(upload: bool, sudo: SudoMode) -> Decision {
        Decision::Steps { upload, sudo }
    }

    #[test]
    fn decide_follows_d300_in_order() {
        let x86 = local("x86_64");
        let rows: Vec<(&str, Facts, LocalFacts, bool, Decision)> = vec![
            (
                "1a",
                Facts {
                    os: "Darwin".into(),
                    ..fresh()
                },
                x86.clone(),
                false,
                refuse("the remote host runs Darwin, not Linux"),
            ),
            (
                "1b",
                fresh(),
                local("riscv64"),
                false,
                refuse("this htui build is riscv64; provisioning supports x86_64 and aarch64"),
            ),
            (
                "1c",
                Facts {
                    arch: "aarch64".into(),
                    ..fresh()
                },
                x86.clone(),
                false,
                refuse(
                    "the remote host is aarch64 and this htui build is x86_64; provisioning \
                     ships this binary, so the two must match",
                ),
            ),
            (
                "1d",
                Facts {
                    systemd: None,
                    ..fresh()
                },
                x86.clone(),
                false,
                refuse("the remote host has no systemd"),
            ),
            (
                "1e",
                Facts {
                    systemd: Some(249),
                    ..fresh()
                },
                x86.clone(),
                false,
                refuse(
                    "the remote host has systemd 249; provisioning needs 250 or later for \
                     LoadCredentialEncrypted=",
                ),
            ),
            (
                "1f",
                Facts {
                    creds: false,
                    ..fresh()
                },
                x86.clone(),
                false,
                refuse("the remote host has no systemd-creds on its PATH"),
            ),
            (
                "1g",
                Facts {
                    sudo: None,
                    ..fresh()
                },
                x86.clone(),
                false,
                refuse(
                    "the remote host has no sudo; provisioning installs a system service and \
                     needs it",
                ),
            ),
            (
                "2 by hand",
                Facts {
                    bin_sha: None,
                    ..provisioned()
                },
                x86.clone(),
                false,
                refuse(
                    "already provisioned with a different build, or set up by hand; upgrading \
                     is not supported yet",
                ),
            ),
            (
                "2 different build",
                Facts {
                    bin_sha: Some("ffff".into()),
                    ..provisioned()
                },
                x86.clone(),
                false,
                refuse(
                    "already provisioned with a different build; upgrading is not supported yet",
                ),
            ),
            (
                "2 unknown sha",
                Facts {
                    bin_sha: Some("unknown".into()),
                    ..provisioned()
                },
                x86.clone(),
                false,
                refuse(
                    "already provisioned with a different build; upgrading is not supported yet",
                ),
            ),
            (
                "3",
                provisioned(),
                x86.clone(),
                false,
                Decision::AlreadyProvisioned,
            ),
            (
                "4 fresh",
                fresh(),
                x86.clone(),
                false,
                steps(true, SudoMode::Password),
            ),
            (
                "4 nopasswd",
                Facts {
                    sudo: Some(SudoMode::NoPassword),
                    ..fresh()
                },
                x86.clone(),
                false,
                steps(true, SudoMode::NoPassword),
            ),
            (
                "arm64 is aarch64",
                Facts {
                    arch: "arm64".into(),
                    ..fresh()
                },
                local("aarch64"),
                false,
                steps(true, SudoMode::Password),
            ),
            (
                "systemd 250 passes",
                Facts {
                    systemd: Some(MIN_SYSTEMD),
                    ..fresh()
                },
                x86.clone(),
                false,
                steps(true, SudoMode::Password),
            ),
            (
                "1a before 1c",
                Facts {
                    os: "FreeBSD".into(),
                    arch: "amd64".into(),
                    ..fresh()
                },
                x86.clone(),
                false,
                refuse("the remote host runs FreeBSD, not Linux"),
            ),
            (
                "this build, not running",
                Facts {
                    active: "failed".into(),
                    ..provisioned()
                },
                x86.clone(),
                false,
                steps(false, SudoMode::Password),
            ),
            (
                "replace credential",
                provisioned(),
                x86.clone(),
                true,
                steps(false, SudoMode::Password),
            ),
            (
                "no unit, unknown sha",
                Facts {
                    bin_sha: Some("unknown".into()),
                    ..fresh()
                },
                x86.clone(),
                false,
                steps(true, SudoMode::Password),
            ),
            (
                "no unit, this build",
                Facts {
                    bin_sha: Some(SHA.into()),
                    ..fresh()
                },
                x86,
                false,
                steps(false, SudoMode::Password),
            ),
        ];
        for (name, facts, local, replace, expected) in rows {
            assert_eq!(decide(&facts, &local, replace), expected, "row {name}");
        }
    }

    #[test]
    fn normalise_arch_maps_arm64_only() {
        assert_eq!(normalise_arch("arm64"), "aarch64");
        assert_eq!(normalise_arch("aarch64"), "aarch64");
        assert_eq!(normalise_arch("x86_64"), "x86_64");
        assert_eq!(normalise_arch("amd64"), "amd64");
    }

    #[test]
    fn validators_accept_and_refuse() {
        let long_ok = format!("a{}", "b".repeat(31));
        let long_bad = format!("a{}", "b".repeat(32));
        for name in ["alice", "_svc", "a-b_9", long_ok.as_str()] {
            assert_eq!(validate_account("user", name), Ok(()), "{name:?}");
        }
        for name in [
            long_bad.as_str(),
            "Alice",
            "9lives",
            "",
            "a b",
            "a.b",
            "a$b",
            "-a",
        ] {
            assert!(validate_account("group", name).is_err(), "{name:?}");
        }
        assert_eq!(
            validate_account("group", "Alice"),
            Err(
                "the remote group \"Alice\" is not a name htui provision writes into a unit file \
                 (it must match [a-z_][a-z0-9_-]{0,31})"
                    .into()
            )
        );

        for home in ["/home/alice", "/var/lib/w-1", "/"] {
            assert_eq!(validate_home(home), Ok(()), "{home:?}");
        }
        for home in [
            "home",
            "",
            "/home/a b",
            "/home/../etc",
            "/home/a..b",
            "/home/al%ice",
        ] {
            assert!(validate_home(home).is_err(), "{home:?}");
        }
        assert_eq!(
            validate_home("home"),
            Err(
                "the remote home \"home\" is not a path htui provision writes into a unit file \
                 (absolute, only A-Z a-z 0-9 _ . / -, no \"..\")"
                    .into()
            )
        );

        for dest in ["host", "u@host", "alias", "[fd00::3]"] {
            assert_eq!(validate_destination(dest), Ok(()), "{dest:?}");
        }
        for dest in ["", "-oProxyCommand=x", "a b", "a\tb", "host\n", "a\u{7f}b"] {
            let refused = validate_destination(dest).expect_err(dest);
            assert_eq!(
                refused,
                "the ssh destination must not be empty, start with \"-\", or contain whitespace \
                 or control characters"
            );
        }
    }
}
