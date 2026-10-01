//! The preflight's answer (MOD-45 D299): `htui.<key>=<value>` lines. Every other line (a login
//! script's greeting, `~/.bashrc` noise, V-7) is ignored, and so is an `htui.` key this build
//! does not know.

use std::fmt;

use crate::provision::plan::SudoMode;

/// The thirteen keys, in the order [`crate::provision::script::PREFLIGHT`] prints them.
pub const KEYS: [&str; 13] = [
    "os",
    "arch",
    "systemd",
    "creds",
    "user",
    "group",
    "home",
    "bin_sha",
    "unit",
    "active",
    "sudo",
    "printf",
    "sha256sum",
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
    /// `printf=builtin`: the remote `sh` has `printf` built in, so INSTALL's `printf` of the sudo
    /// password is no process of its own (review finding 5). `external` is `false`.
    pub printf_builtin: bool,
    /// `sha256sum` is on `PATH`; PREPARE checks the upload with it (review finding 11).
    pub sha256sum: bool,
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
        let mut values: [Option<&str>; KEYS.len()] = [None; KEYS.len()];
        for line in stdout.lines() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let Some((key, value)) = line
                .strip_prefix("htui.")
                .and_then(|rest| rest.split_once('='))
            else {
                continue;
            };
            let Some(at) = KEYS.iter().position(|k| *k == key) else {
                continue;
            };
            if values[at].is_some() {
                return Err(FactsError::Duplicate(KEYS[at]));
            }
            values[at] = Some(value);
        }
        if values.iter().all(Option::is_none) {
            return Err(FactsError::NoPreflight);
        }
        if let Some(at) = values.iter().position(Option::is_none) {
            return Err(FactsError::Missing(KEYS[at]));
        }
        let get = |key: &'static str| -> &str {
            KEYS.iter()
                .position(|k| *k == key)
                .and_then(|at| values[at])
                .unwrap_or_default()
        };
        let yes_no = |key: &'static str| match get(key) {
            "yes" => Ok(true),
            "no" => Ok(false),
            _ => Err(FactsError::Unreadable(key)),
        };
        let systemd = match get("systemd") {
            "none" => None,
            v if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) => Some(
                v.parse::<u32>()
                    .map_err(|_| FactsError::Unreadable("systemd"))?,
            ),
            _ => return Err(FactsError::Unreadable("systemd")),
        };
        let sudo = match get("sudo") {
            "nopasswd" => Some(SudoMode::NoPassword),
            "password" => Some(SudoMode::Password),
            "none" => None,
            _ => return Err(FactsError::Unreadable("sudo")),
        };
        let printf_builtin = match get("printf") {
            "builtin" => true,
            "external" => false,
            _ => return Err(FactsError::Unreadable("printf")),
        };
        let bin_sha = match get("bin_sha") {
            "none" => None,
            v => Some(v.to_owned()),
        };
        Ok(Self {
            os: get("os").to_owned(),
            arch: get("arch").to_owned(),
            systemd,
            creds: yes_no("creds")?,
            user: get("user").to_owned(),
            group: get("group").to_owned(),
            home: get("home").to_owned(),
            bin_sha,
            unit: yes_no("unit")?,
            active: get("active").to_owned(),
            sudo,
            printf_builtin,
            sha256sum: yes_no("sha256sum")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete answer, as `(key, value)` pairs in [`KEYS`] order.
    fn pairs() -> Vec<(&'static str, &'static str)> {
        vec![
            ("os", "Linux"),
            ("arch", "x86_64"),
            ("systemd", "255"),
            ("creds", "yes"),
            ("user", "alice"),
            ("group", "staff"),
            ("home", "/home/alice"),
            ("bin_sha", "none"),
            ("unit", "no"),
            ("active", "inactive"),
            ("sudo", "password"),
            ("printf", "builtin"),
            ("sha256sum", "yes"),
        ]
    }

    fn render(pairs: &[(&str, &str)]) -> String {
        pairs
            .iter()
            .map(|(k, v)| format!("htui.{k}={v}\n"))
            .collect()
    }

    fn with(key: &str, value: &'static str) -> String {
        let mut all = pairs();
        for pair in &mut all {
            if pair.0 == key {
                pair.1 = value;
            }
        }
        render(&all)
    }

    fn expected() -> Facts {
        Facts {
            os: "Linux".into(),
            arch: "x86_64".into(),
            systemd: Some(255),
            creds: true,
            user: "alice".into(),
            group: "staff".into(),
            home: "/home/alice".into(),
            bin_sha: None,
            unit: false,
            active: "inactive".into(),
            sudo: Some(SudoMode::Password),
            printf_builtin: true,
            sha256sum: true,
        }
    }

    #[test]
    fn the_keys_are_the_fields_in_order() {
        let keys: Vec<&str> = pairs().iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, KEYS);
    }

    #[test]
    fn a_complete_answer_parses() {
        assert_eq!(Facts::parse(&render(&pairs())), Ok(expected()));

        let facts = Facts::parse(&render(&[
            ("os", "Linux"),
            ("arch", "aarch64"),
            ("systemd", "none"),
            ("creds", "no"),
            ("user", "bob"),
            ("group", "bob"),
            ("home", ""),
            ("bin_sha", "unknown"),
            ("unit", "yes"),
            ("active", "active"),
            ("sudo", "nopasswd"),
            ("printf", "external"),
            ("sha256sum", "no"),
        ]))
        .expect("parses");
        assert!(!facts.printf_builtin);
        assert!(!facts.sha256sum);
        assert_eq!(facts.systemd, None);
        assert!(!facts.creds);
        assert_eq!(facts.home, "");
        assert_eq!(facts.bin_sha.as_deref(), Some("unknown"));
        assert!(facts.unit);
        assert_eq!(facts.sudo, Some(SudoMode::NoPassword));
    }

    #[test]
    fn the_value_is_everything_after_the_first_equals() {
        let facts = Facts::parse(&with("active", "a=b")).expect("parses");
        assert_eq!(facts.active, "a=b");
    }

    #[test]
    fn noise_is_ignored() {
        let mut text = String::from("Welcome to host\r\nhtui-motd: hi\r\n\r\n\n");
        for (k, v) in pairs() {
            text.push_str(&format!("htui.{k}={v}\r\n"));
            text.push_str("  \r\n");
        }
        text.push_str("bye");
        assert_eq!(Facts::parse(&text), Ok(expected()));
    }

    #[test]
    fn an_unknown_key_is_ignored() {
        let text = format!("htui.extra=1\n{}htui.extra=2\n", render(&pairs()));
        assert_eq!(Facts::parse(&text), Ok(expected()));
    }

    #[test]
    fn each_missing_key_is_named() {
        for key in KEYS {
            let rest: Vec<(&str, &str)> = pairs().into_iter().filter(|(k, _)| *k != key).collect();
            assert_eq!(
                Facts::parse(&render(&rest)),
                Err(FactsError::Missing(key)),
                "{key}"
            );
        }
    }

    #[test]
    fn a_key_twice_is_named() {
        let text = format!("{}htui.home=\n", render(&pairs()));
        assert_eq!(Facts::parse(&text), Err(FactsError::Duplicate("home")));
        // Duplicates come before absences.
        let text = "htui.home=/a\nhtui.home=/b\n";
        assert_eq!(Facts::parse(text), Err(FactsError::Duplicate("home")));
    }

    #[test]
    fn no_preflight_line_is_no_preflight() {
        assert_eq!(Facts::parse(""), Err(FactsError::NoPreflight));
        assert_eq!(
            Facts::parse("fish: Unknown command\nWelcome to host\n"),
            Err(FactsError::NoPreflight)
        );
    }

    #[test]
    fn unreadable_values_are_named() {
        for (key, value) in [
            ("systemd", "25x"),
            ("systemd", ""),
            ("systemd", "99999999999"),
            ("creds", "maybe"),
            ("unit", "maybe"),
            ("sudo", "sometimes"),
            ("printf", "maybe"),
            ("printf", ""),
            ("sha256sum", "maybe"),
        ] {
            let key: &'static str = KEYS.iter().find(|k| **k == key).expect("a key");
            assert_eq!(
                Facts::parse(&with(key, value)),
                Err(FactsError::Unreadable(key)),
                "{key}={value}"
            );
        }
    }

    #[test]
    fn sudo_none_is_none() {
        let facts = Facts::parse(&with("sudo", "none")).expect("parses");
        assert_eq!(facts.sudo, None);
    }

    #[test]
    fn errors_read_as_sentences() {
        assert!(
            FactsError::NoPreflight
                .to_string()
                .contains("POSIX login shell")
        );
        assert_eq!(
            FactsError::Missing("os").to_string(),
            "the preflight did not report `os`"
        );
        assert_eq!(
            FactsError::Duplicate("home").to_string(),
            "the preflight reported `home` twice"
        );
        assert_eq!(
            FactsError::Unreadable("sudo").to_string(),
            "the preflight reported an unreadable `sudo`"
        );
    }
}
