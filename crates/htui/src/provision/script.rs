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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every script with its name, for messages.
    const ALL: [(&str, &str); 5] = [
        ("PREFLIGHT", PREFLIGHT),
        ("PREPARE", PREPARE),
        ("INSTALL", INSTALL),
        ("INSTALL_ROOT", INSTALL_ROOT),
        ("VERIFY", VERIFY),
    ];

    /// The guide's sample unit (`docs/htui-worker.md`, "Running it as a systemd service") for the
    /// user `you`, with the binary in `~/.local/bin` (D294, D295).
    const GUIDE_UNIT: &str = r"[Unit]
Description=htui worker
Wants=network-online.target
After=network-online.target

[Service]
Type=exec
User=you
ExecStart=/home/you/.local/bin/htui worker --log /home/you/.local/state/htui/worker.log
LoadCredentialEncrypted=htui-dsn:/etc/credstore.encrypted/htui-dsn
Restart=on-failure
RestartSec=10s

[Install]
WantedBy=multi-user.target
";

    /// The index of the one line of `script` containing `needle`.
    fn line_of(script: &str, needle: &str) -> usize {
        let hits: Vec<usize> = script
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(needle))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "expected one line containing {needle:?}, got {hits:?}"
        );
        hits[0]
    }

    /// Whether `line` holds `word` as a whole shell word (`pw`, `$pw`, `"$pw"`).
    fn has_word(line: &str, word: &str) -> bool {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|token| token == word)
    }

    /// INSTALL_ROOT's heredoc body, the line after `<<EOF` up to the closing `EOF`, and the
    /// indices of the opening and closing lines.
    fn heredoc() -> (String, usize, usize) {
        let lines: Vec<&str> = INSTALL_ROOT.lines().collect();
        let open = line_of(INSTALL_ROOT, "<<EOF");
        let close = lines
            .iter()
            .enumerate()
            .skip(open + 1)
            .find(|(_, line)| **line == "EOF")
            .map(|(i, _)| i)
            .expect("the heredoc is closed");
        let mut body = String::new();
        for line in &lines[open + 1..close] {
            body.push_str(line);
            body.push('\n');
        }
        (body, open, close)
    }

    /// `true` when `program` can be spawned at all.
    fn on_path(program: &str) -> bool {
        std::process::Command::new(program)
            .args(["-c", "true"])
            .status()
            .is_ok()
    }

    fn parses_under(shell: &str) {
        for (name, script) in ALL {
            let out = std::process::Command::new(shell)
                .args(["-n", "-c", script])
                .output()
                .expect("the shell runs");
            assert!(
                out.status.success(),
                "{shell} -n refused {name}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[test]
    fn every_script_starts_with_set_eu() {
        for (name, script) in ALL {
            assert!(
                script.starts_with("set -eu\n"),
                "{name} does not start with set -eu"
            );
        }
    }

    #[test]
    fn no_script_holds_a_backslash_or_a_single_quote() {
        for (name, script) in ALL {
            assert!(!script.contains('\\'), "{name} holds a backslash");
            assert!(!script.contains('\''), "{name} holds a single quote");
        }
    }

    #[test]
    fn every_script_parses_under_sh_n() {
        parses_under("sh");
        if on_path("dash") {
            parses_under("dash");
        } else {
            println!("dash is not on PATH; skipping dash -n");
        }
        if on_path("bash") {
            parses_under("bash");
        } else {
            println!("bash is not on PATH; skipping bash -n");
        }
    }

    #[test]
    fn remote_command_round_trips_through_shell_words() {
        let args = ["", "alice", "/home/alice", "0", "password", INSTALL_ROOT];
        let command = remote_command(INSTALL, &args);
        let split = shell_words::split(&command).expect("the command splits");
        let mut expected = vec!["sh", "-c", INSTALL, ARG0];
        expected.extend(args);
        assert_eq!(split, expected);

        let command = remote_command(PREFLIGHT, &[""]);
        let split = shell_words::split(&command).expect("the command splits");
        assert_eq!(split, ["sh", "-c", PREFLIGHT, ARG0, ""]);
    }

    #[test]
    fn remote_command_is_backslash_free() {
        let install = remote_command(
            INSTALL,
            &["", "alice", "/home/alice", "0", "password", INSTALL_ROOT],
        );
        assert!(!install.contains('\\'));
        for (name, script) in ALL {
            let command = remote_command(script, &["", "/home/alice", "1", "2"]);
            assert!(
                !command.contains('\\'),
                "{name}'s command holds a backslash"
            );
        }
    }

    #[test]
    fn install_mentions_pw_only_on_its_three_lines() {
        let lines: Vec<&str> = INSTALL
            .lines()
            .filter(|line| has_word(line, "pw"))
            .map(str::trim)
            .collect();
        assert_eq!(
            lines,
            [
                "IFS= read -r pw || exit 4",
                r#"printf "%s%s" "$pw" "$nl" | sudo -S -p "" -v || exit 4"#,
                "unset pw",
            ]
        );
    }

    #[test]
    fn install_reads_the_password_before_any_sudo() {
        let read = line_of(INSTALL, "read -r pw");
        let forget = line_of(INSTALL, "sudo -k");
        let validate = line_of(INSTALL, r#"sudo -S -p "" -v"#);
        let run = line_of(INSTALL, "sudo -n");
        assert!(read < forget, "read {read} < sudo -k {forget}");
        assert!(forget < validate, "sudo -k {forget} < sudo -v {validate}");
        assert!(validate < run, "sudo -v {validate} < sudo -n {run}");
        let first_sudo = INSTALL
            .lines()
            .position(|line| line.contains("sudo"))
            .expect("INSTALL calls sudo");
        assert!(read < first_sudo);
    }

    #[test]
    fn install_never_execs_sudo_n() {
        let lines: Vec<&str> = INSTALL.lines().collect();
        let run = line_of(INSTALL, "sudo -n");
        assert!(lines[run].ends_with("|| rc=$?"), "{:?}", lines[run]);
        assert_eq!(lines.get(run + 1).copied(), Some(r#"exit "$rc""#));
        assert_eq!(run + 2, lines.len(), "exit \"$rc\" is the last line");
        assert!(INSTALL.ends_with("exit \"$rc\"\n"));
    }

    #[test]
    fn install_root_writes_the_credential_then_the_unit_then_starts() {
        let order = [
            "install -d -m 0700",
            "systemd-creds encrypt",
            r#"cat > "$tmp" <<EOF"#,
            r#"mv -f -- "$tmp" "$unit""#,
            "systemctl daemon-reload",
            "systemctl enable --now",
            "systemctl restart",
        ];
        let at: Vec<usize> = order.iter().map(|n| line_of(INSTALL_ROOT, n)).collect();
        for pair in at.windows(2) {
            assert!(pair[0] < pair[1], "{order:?} out of order: {at:?}");
        }

        let marker = line_of(INSTALL_ROOT, &format!("echo {ROOT_MARKER}"));
        let first_write = INSTALL_ROOT
            .lines()
            .position(|line| {
                [
                    "install ",
                    "rm ",
                    "mv ",
                    "mkdir ",
                    "cat ",
                    "chmod ",
                    "systemd-creds",
                    "systemctl",
                ]
                .iter()
                .any(|w| line.trim_start().starts_with(w))
            })
            .expect("INSTALL_ROOT writes");
        assert!(
            marker < first_write,
            "marker {marker} < first write {first_write}"
        );
    }

    #[test]
    fn the_unit_is_the_guides_sample() {
        let (body, _, _) = heredoc();
        let unit = body.replace("$user", "you").replace("$home", "/home/you");
        assert_eq!(unit, GUIDE_UNIT);
    }

    #[test]
    fn no_privileged_line_touches_home() {
        let (_, open, close) = heredoc();
        for (i, line) in INSTALL_ROOT.lines().enumerate() {
            if (open + 1..close).contains(&i) {
                continue;
            }
            assert!(
                !line.contains("$home"),
                "INSTALL_ROOT line {i} touches home: {line:?}"
            );
        }
    }

    #[test]
    fn verify_waits_inside_one_session() {
        assert!(VERIFY.contains("while "));
        assert!(VERIFY.contains(r#"sleep "$pause""#));
        assert!(VERIFY.contains("exit 5"));
        assert!(!VERIFY.contains("sudo"));
    }
}
