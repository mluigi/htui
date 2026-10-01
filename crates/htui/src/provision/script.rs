//! The remote scripts (MOD-45 D297): POSIX `sh`, each starting with `set -eu`, with neither a
//! backslash nor a single quote anywhere (E-3, E-5), so `shell_words::quote` wraps each in a plain
//! `'…'` and the whole command line the remote login shell parses is backslash-free. Values arrive
//! only as positional arguments, the root prefix always first (`""` in production; a temporary
//! directory under test, D309). The DSN and the sudo password are never arguments.
//!
//! Exit codes a script chooses: 3, the uploaded binary does not run (PREPARE); 4, sudo refused
//! the password (INSTALL); 5, the service did not come up (VERIFY); 6, the upload's sha256 is not
//! the payload's (PREPARE). A script with a temporary file removes it on exit and on HUP, INT and
//! TERM (which exit 1). Anything else is the failing command's own status.

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
/// `printf` is `builtin` when this `sh` has it built in, `external` otherwise (review finding 5:
/// INSTALL hands the sudo password to `printf`, which must not be a process of its own).
pub const PREFLIGHT: &str = r#"set -eu
root=$1
say() {
  printf "htui.%s=%s" "$1" "$2"
  echo
}
home=${HOME:-}
say os "$(uname -s)"
say arch "$(uname -m)"
v=
if command -v systemctl >/dev/null 2>&1; then
  v=$(systemctl --version 2>/dev/null || true)
  v=${v#"systemd "}
  v=${v%%[!0-9]*}
fi
say systemd "${v:-none}"
if command -v systemd-creds >/dev/null 2>&1; then
  say creds yes
else
  say creds no
fi
say user "$(id -un)"
say group "$(id -gn)"
say home "$home"
sha=none
if [ -n "$home" ] && [ -f "$home/.local/bin/htui" ]; then
  sha=$(sha256sum < "$home/.local/bin/htui" 2>/dev/null || true)
  sha=${sha%% *}
fi
say bin_sha "${sha:-unknown}"
if [ -e "$root/etc/systemd/system/htui-worker.service" ]; then
  say unit yes
else
  say unit no
fi
a=
if command -v systemctl >/dev/null 2>&1; then
  a=$(systemctl is-active htui-worker.service 2>/dev/null || true)
fi
say active "${a:-unknown}"
if ! command -v sudo >/dev/null 2>&1; then
  say sudo none
elif sudo -k -n true >/dev/null 2>&1; then
  say sudo nopasswd
else
  say sudo password
fi
p=$(command -v printf || true)
case $p in
  printf) say printf builtin ;;
  *) say printf external ;;
esac
"#;

/// Unprivileged (D304): the log directory, then, with `send=1`, the binary on stdin, whose sha256
/// must be `$4` and which must run `--version` before it replaces anything. `$1` root (unused),
/// `$2` home, `$3` send (`0`|`1`), `$4` the payload's lowercase hex sha256. Exit 6: the upload is
/// not the payload (review finding 11); exit 3: it does not run here. Either way, and on a
/// signal, the temporary file is removed.
pub const PREPARE: &str = r#"set -eu
home=$2
send=$3
sha=$4
bin="$home/.local/bin"
tmp="$bin/.htui.provision.$$"
cleanup() {
  rm -f -- "$tmp"
}
trap cleanup EXIT
trap "exit 1" HUP INT TERM
mkdir -p "$home/.local/state/htui" "$bin"
if [ "$send" = 1 ]; then
  cat > "$tmp"
  got=$(sha256sum < "$tmp")
  if [ "${got%% *}" != "$sha" ]; then
    exit 6
  fi
  chmod 0755 "$tmp"
  if ! "$tmp" --version; then
    exit 3
  fi
  mv -f -- "$tmp" "$bin/htui"
fi
"#;

/// Unprivileged wrapper around the privileged part (D301; E-3, E-4, E-7, E-8). `$1` root, `$2`
/// user, `$3` home, `$4` replace (`0`|`1`), `$5` mode (`password`|`nopasswd`), `$6` the text of
/// [`INSTALL_ROOT`]. Stdin: with `mode=password`, the password line, then the DSN line; otherwise
/// the DSN line alone. Exit 4: sudo refused the password; the DSN was never read.
pub const INSTALL: &str = r#"set -eu
root=$1
user=$2
home=$3
replace=$4
mode=$5
installer=$6
nl="
"
if [ "$mode" = password ]; then
  IFS= read -r pw || exit 4
  sudo -k || exit 4
  printf "%s%s" "$pw" "$nl" | sudo -S -p "" -v || exit 4
  unset pw
fi
rc=0
sudo -n sh -c "$installer" htui-provision-root "$root" "$user" "$home" "$replace" || rc=$?
exit "$rc"
"#;

/// Runs as root under `sudo -n` (D303; E-7). `$1` root, `$2` user, `$3` home, `$4` replace. Stdin:
/// the DSN line, encrypted when the credential is absent or `replace=1`, otherwise discarded.
/// Credential, then unit, then start. Its first line of output is the marker `htui.root=start`.
pub const INSTALL_ROOT: &str = r#"set -eu
root=$1
user=$2
home=$3
replace=$4
echo htui.root=start
umask 022
creds="$root/etc/credstore.encrypted"
cred="$creds/htui-dsn"
units="$root/etc/systemd/system"
unit="$units/htui-worker.service"
tmp="$unit.htui-provision.$$"
cleanup() {
  rm -f -- "$tmp" "$cred.new"
}
trap cleanup EXIT
trap "exit 1" HUP INT TERM
install -d -m 0700 "$creds"
if [ ! -e "$cred" ] || [ "$replace" = 1 ]; then
  rm -f -- "$cred.new"
  systemd-creds encrypt --name=htui-dsn - "$cred.new"
  mv -f -- "$cred.new" "$cred"
else
  cat > /dev/null
fi
mkdir -p "$units"
cat > "$tmp" <<EOF
[Unit]
Description=htui worker
Wants=network-online.target
After=network-online.target

[Service]
Type=exec
User=$user
ExecStart=$home/.local/bin/htui worker --log $home/.local/state/htui/worker.log
LoadCredentialEncrypted=htui-dsn:/etc/credstore.encrypted/htui-dsn
Restart=on-failure
RestartSec=10s

[Install]
WantedBy=multi-user.target
EOF
chmod 0644 "$tmp"
mv -f -- "$tmp" "$unit"
systemctl daemon-reload
systemctl enable --now htui-worker.service
if [ "$replace" = 1 ]; then
  systemctl restart htui-worker.service
fi
"#;

/// Waits inside one session (D305; E-10): up to `tries` checks, `pause` seconds apart, until the
/// service is `active` and `box.toml` exists, then prints both between markers. `$1` root
/// (unused), `$2` home, `$3` tries, `$4` pause. Exit 5: it never did; the journal and log tails
/// go to stderr.
pub const VERIFY: &str = r#"set -eu
home=$2
tries=$3
pause=$4
box="$home/.config/htui/box.toml"
n=0
a=
while [ "$n" -lt "$tries" ]; do
  a=$(systemctl is-active htui-worker.service 2>/dev/null || true)
  if [ "$a" = active ] && [ -f "$box" ]; then
    echo htui.active=active
    echo htui.box.begin
    cat -- "$box"
    echo
    echo htui.box.end
    exit 0
  fi
  n=$((n + 1))
  if [ "$n" -lt "$tries" ]; then
    sleep "$pause"
  fi
done
echo "htui.active=${a:-unknown}"
journalctl -u htui-worker.service -n 20 --no-pager >&2 || true
tail -n 20 -- "$home/.local/state/htui/worker.log" >&2 || true
exit 5
"#;

/// `sh -c '<script>' htui-provision '<arg>'…`, every piece through `shell_words::quote`. The
/// remote login shell parses it once (D297, V-7).
#[must_use]
pub fn remote_command(script: &str, args: &[&str]) -> String {
    ["sh", "-c", script, ARG0]
        .into_iter()
        .chain(args.iter().copied())
        .map(shell_words::quote)
        .collect::<Vec<_>>()
        .join(" ")
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

    /// A fresh `<home>` for PREPARE, under the system temp directory (nothing is executed there).
    fn prepare_home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("htui-prepare-")
            .tempdir()
            .expect("a temporary home")
    }

    /// The names left in `<home>/.local/bin`.
    fn bin_entries(home: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(home.join(".local/bin"))
            .map(|entries| {
                entries
                    .map(|e| {
                        e.expect("an entry")
                            .file_name()
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Review finding 11: an upload whose sha256 is not the expected one exits 6, removes the
    /// temporary file and installs nothing.
    #[test]
    fn prepare_refuses_a_corrupted_upload() {
        use std::io::Write as _;
        let home = prepare_home();
        let home_text = home.path().to_str().expect("a UTF-8 home");
        let wrong = "0".repeat(64);
        let mut child = std::process::Command::new("sh")
            .args(["-c", PREPARE, ARG0, "", home_text, "1", wrong.as_str()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("sh runs");
        let mut stdin = child.stdin.take().expect("stdin");
        stdin
            .write_all(b"#!/bin/sh\necho htui 0.0.0\n")
            .expect("write the upload");
        drop(stdin);
        let status = child.wait().expect("PREPARE ends");
        assert_eq!(status.code(), Some(6), "a corrupted upload exits 6");
        assert!(
            bin_entries(home.path()).is_empty(),
            "left behind: {:?}",
            bin_entries(home.path())
        );
    }

    /// Review finding 12: a signal while the upload is being read still removes the temporary
    /// file, under every shell at hand.
    #[test]
    fn prepare_removes_its_temporary_file_on_a_signal() {
        for shell in ["sh", "dash", "bash"] {
            if !on_path(shell) {
                println!("{shell} is not on PATH; skipping");
                continue;
            }
            let home = prepare_home();
            let home_text = home.path().to_str().expect("a UTF-8 home");
            let sha = "0".repeat(64);
            let mut child = std::process::Command::new(shell)
                .args(["-c", PREPARE, ARG0, "", home_text, "1", sha.as_str()])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("the shell runs");
            let stdin = child.stdin.take().expect("stdin");
            let started = std::time::Instant::now();
            while !bin_entries(home.path())
                .iter()
                .any(|name| name.starts_with(".htui.provision."))
            {
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(10),
                    "{shell}: PREPARE never created its temporary file"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let killed = std::process::Command::new("kill")
                .args(["-TERM", &child.id().to_string()])
                .status()
                .expect("kill runs");
            assert!(killed.success());
            // `cat` still holds the pipe; closing it lets the shell run its trap.
            drop(stdin);
            let status = child.wait().expect("the shell ends");
            assert!(!status.success(), "{shell}: {status:?}");
            assert!(
                bin_entries(home.path()).is_empty(),
                "{shell} left {:?}",
                bin_entries(home.path())
            );
        }
    }

    /// Review finding 12: every script with a cleanup trap also exits on HUP, INT and TERM, so
    /// the EXIT trap runs on a signal too.
    #[test]
    fn every_cleanup_trap_covers_the_signals() {
        let mut seen = 0;
        for (name, script) in ALL {
            let lines: Vec<&str> = script.lines().collect();
            if let Some(at) = lines.iter().position(|line| *line == "trap cleanup EXIT") {
                seen += 1;
                assert_eq!(
                    lines.get(at + 1).copied(),
                    Some(r#"trap "exit 1" HUP INT TERM"#),
                    "{name}"
                );
            }
        }
        assert_eq!(seen, 2, "PREPARE and INSTALL_ROOT clean up");
    }

    /// Review finding 11: PREPARE compares the upload's sha256 before it runs it.
    #[test]
    fn prepare_checks_the_sha_before_it_runs_the_upload() {
        let read = line_of(PREPARE, r#"cat > "$tmp""#);
        let hash = line_of(PREPARE, r#"sha256sum < "$tmp""#);
        let refuse = line_of(PREPARE, "exit 6");
        let run = line_of(PREPARE, r#""$tmp" --version"#);
        assert!(read < hash && hash < refuse && refuse < run);
    }

    /// Review finding 5: PREFLIGHT says whether `printf` is a builtin of the remote `sh`.
    #[test]
    fn preflight_reports_whether_printf_is_a_builtin() {
        assert!(PREFLIGHT.contains("command -v printf"));
        assert!(PREFLIGHT.contains("say printf builtin"));
        assert!(PREFLIGHT.contains("say printf external"));
        let probe = "p=$(command -v printf || true)\ncase $p in\n  printf) echo builtin ;;\n  *) echo external ;;\nesac\n";
        assert!(PREFLIGHT.contains(&probe.replace("echo ", "say printf ")));
        for shell in ["sh", "dash", "bash"] {
            if !on_path(shell) {
                continue;
            }
            let out = std::process::Command::new(shell)
                .args(["-c", probe])
                .output()
                .expect("the shell runs");
            assert_eq!(String::from_utf8_lossy(&out.stdout), "builtin\n", "{shell}");
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
