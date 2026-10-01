//! MOD-45 T4 (blueprint §5, plan D309–D310): `htui provision` end to end over the real scripts.
//!
//! `run_with` drives the five committed scripts through a [`LocalShell`]: each remote command runs
//! as `sh -c <command>` on this machine, exactly as sshd hands it to the login shell, through the
//! same `remote::run_piped` the production `SshRemote` uses. The remote tools a host would provide
//! (`sudo`, `systemctl`, `systemd-creds`, `uname`, `id`, `journalctl`) are stub scripts first on
//! `PATH`; everything else (`sh`, `install`, `mv`, `cat`, `sha256sum`, …) is the real tool. Every
//! script gets a temporary root prefix as `$1`, so the "privileged" writes land under it.
//!
//! Every scenario ends with the D310 sweep: the DSN and the sudo password reach nothing but the
//! remote stdin and the encrypted credential. Every directory lives under `CARGO_TARGET_TMPDIR`
//! (E-17), never under `/tmp` or `~/.config/htui`, and every spawned process is waited for.
//!
//! No Postgres is needed: the verifier and the password prompt are fakes. `provision_pg.rs` runs
//! the Postgres verifier.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use htui::provision::remote::{Remote, RemoteOutput, run_piped};
use htui::provision::script::INSTALL_ROOT;
use htui::provision::secret_prompt::{PasswordPrompt, PromptError};
use htui::provision::verify::{Baseline, Poll, Verifier};
use htui::provision::{Ctx, Outcome, Payload, ProvisionExit, Timings, run_with};
use htui_core::model::BoxId;
use sha2::Digest as _;
use zeroize::Zeroizing;

/// The DSN every scenario ships (D310).
const DSN: &str = "postgres://u:SENTINEL-DSN-PW@db.example:5432/htui";
/// S5's second DSN: the same password, another host.
const DSN2: &str = "postgres://u:SENTINEL-DSN-PW@db2.example:5432/htui";
/// The sudo password the stub `sudo` accepts (D310).
const PASSWORD: &str = "SENTINEL-SUDO-PW";
/// The two sentinels the sweep looks for. The wrong password of S6 contains the second.
const SENTINELS: [&str; 2] = ["SENTINEL-DSN-PW", "SENTINEL-SUDO-PW"];
/// The ssh destination, as typed.
const DEST: &str = "stub-dest";

/// The `box.toml` the stub `systemctl enable --now` "starts the worker" with.
const BOX_TOML: &str = r#"box_id = "0190f0e0-0000-7000-8000-000000000045"
hostname = "stub-host"
"#;
/// [`BOX_TOML`]'s id, as `run_with` prints it.
const BOX_ID: &str = "0190f0e0-0000-7000-8000-000000000045";

/// A payload that runs `--version`.
const GOOD_HTUI: &str = "#!/bin/sh\necho \"htui 0.0.0-stub\"\n";
/// A payload that fails like a binary linked against a newer glibc.
const BAD_HTUI: &str =
    "#!/bin/sh\necho \"version GLIBC_2.99 not found (required by htui)\" >&2\nexit 1\n";

/// MOD-45 T4 stub sudo (blueprint §5.2): the timestamp is keyed on `$PPID`, as sudo's
/// `timestamp_type=tty` keys it without a tty (V-5), so an exec'd `sudo -n` is refused (E-4).
const STUB_SUDO: &str = r##"#!/bin/sh
# MOD-45 T4 stub sudo: only the flags PREFLIGHT and INSTALL use (plan D309, R-2).
set -u
d=$STUB_DIR
printf '%s\n' "sudo $*" >> "$d/argv.log"
env >> "$d/env.log"
k=0; n=0; S=0; v=0
while [ $# -gt 0 ]; do
  case $1 in
    -k) k=1 ;;
    -n) n=1 ;;
    -S) S=1 ;;
    -v) v=1 ;;
    -p) shift ;;
    -*) echo "stub sudo: unsupported flag $1" >&2; exit 64 ;;
    *) break ;;
  esac
  shift
done
mode=$(cat "$d/sudo.mode")
if [ $# -eq 0 ] && [ $v -eq 0 ]; then
  rm -f "$d/sudo.ts"
  exit 0
fi
if [ $v -eq 1 ]; then
  if [ "$mode" = nopasswd ]; then echo "$PPID" > "$d/sudo.ts"; exit 0; fi
  if [ $S -eq 0 ]; then echo "sudo: a terminal is required to read the password" >&2; exit 1; fi
  tries=0
  while [ $tries -lt 3 ]; do
    if ! IFS= read -r line; then echo "sudo: no password was provided" >&2; exit 1; fi
    tries=$((tries + 1))
    echo line >> "$d/sudo.reads"
    if [ "$(printf '%s' "$line" | sha256sum | cut -d ' ' -f 1)" = "$(cat "$d/sudo.pwhash")" ]; then
      echo "$PPID" > "$d/sudo.ts"
      exit 0
    fi
    echo "Sorry, try again." >&2
  done
  echo "sudo: 3 incorrect password attempts" >&2
  exit 1
fi
if [ $n -eq 0 ]; then echo "stub sudo: a command without -n is not modelled" >&2; exit 64; fi
if [ "$mode" = nopasswd ]; then exec "$@"; fi
if [ $k -eq 0 ] && [ -f "$d/sudo.ts" ] && [ "$(cat "$d/sudo.ts")" = "$PPID" ]; then exec "$@"; fi
echo "sudo: a password is required" >&2
exit 1
"##;

/// MOD-45 T4 stub systemctl (blueprint §5.2).
const STUB_SYSTEMCTL: &str = r##"#!/bin/sh
# MOD-45 T4 stub systemctl.
set -u
d=$STUB_DIR
printf '%s\n' "systemctl $*" >> "$d/argv.log"
env >> "$d/env.log"
case "${1:-}" in
  --version)
    v=$(cat "$d/systemd.version")
    printf 'systemd %s (%s-stub)\n+PAM +AUDIT +SELINUX\n' "$v" "$v"
    ;;
  is-active)
    [ "${2:-}" = htui-worker.service ] || exit 64
    state=$(cat "$d/active" 2>/dev/null || echo inactive)
    echo "$state"
    [ "$state" = active ]
    ;;
  daemon-reload)
    ;;
  enable)
    { [ "${2:-}" = --now ] && [ "${3:-}" = htui-worker.service ]; } || exit 64
    if [ -f "$d/never-active" ]; then
      echo activating > "$d/active"
    else
      mkdir -p "$HOME/.config/htui"
      [ -f "$HOME/.config/htui/box.toml" ] || cp "$d/box.toml" "$HOME/.config/htui/box.toml"
      echo active > "$d/active"
    fi
    ;;
  restart)
    [ "${2:-}" = htui-worker.service ] || exit 64
    ;;
  *)
    echo "stub systemctl: unsupported $*" >&2
    exit 64
    ;;
esac
"##;

/// MOD-45 T4 stub systemd-creds (blueprint §5.2).
const STUB_SYSTEMD_CREDS: &str = r##"#!/bin/sh
# MOD-45 T4 stub systemd-creds: "encrypts" by wrapping stdin in a marker.
set -u
d=$STUB_DIR
printf '%s\n' "systemd-creds $*" >> "$d/argv.log"
env >> "$d/env.log"
{ [ $# -eq 4 ] && [ "$1" = encrypt ] && [ "$2" = --name=htui-dsn ] && [ "$3" = - ]; } || exit 64
{ printf 'ENCRYPTED('; cat; printf ')'; } > "$4"
"##;

/// MOD-45 T4 stub uname (blueprint §5.2).
const STUB_UNAME: &str = r##"#!/bin/sh
d=$STUB_DIR
printf '%s\n' "uname $*" >> "$d/argv.log"
case "${1:-}" in -s) cat "$d/os" ;; -m) cat "$d/arch" ;; *) exit 64 ;; esac
"##;

/// MOD-45 T4 stub id (blueprint §5.2; E-17: a canned `provtest`).
const STUB_ID: &str = r##"#!/bin/sh
d=$STUB_DIR
printf '%s\n' "id $*" >> "$d/argv.log"
case "${1:-}" in -un) cat "$d/user" ;; -gn) cat "$d/group" ;; *) exit 64 ;; esac
"##;

/// MOD-45 T4 stub journalctl (blueprint §5.2).
const STUB_JOURNALCTL: &str = r##"#!/bin/sh
d=$STUB_DIR
printf '%s\n' "journalctl $*" >> "$d/argv.log"
echo "stub journal: htui-worker.service: Main process exited, status=2/INVALIDARGUMENT"
echo "stub journal: htui: no Postgres DSN"
"##;

/// One scenario's directories, all under `CARGO_TARGET_TMPDIR` (E-17: `/tmp` may be `noexec`, and
/// every path must pass D296's home charset).
struct World {
    _base: tempfile::TempDir,
    /// `$1` of every script: `<root>/etc/...` is where the "root" writes land.
    root: PathBuf,
    /// `HOME` of every remote process.
    home: PathBuf,
    /// The stub binaries and their state files; first on `PATH`.
    stub: PathBuf,
}

impl World {
    /// The three directories, the six stubs and their default state; `sudo_mode` is `password` or
    /// `nopasswd`.
    fn new(sudo_mode: &str) -> Self {
        let base = tempfile::Builder::new()
            .prefix("provision-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .expect("a scenario directory under CARGO_TARGET_TMPDIR");
        let root = base.path().join("root");
        let home = base.path().join("home");
        let stub = base.path().join("stub");
        for dir in [&root, &home, &stub] {
            std::fs::create_dir(dir).expect("create a scenario directory");
        }
        for (name, text) in [
            ("sudo", STUB_SUDO),
            ("systemctl", STUB_SYSTEMCTL),
            ("systemd-creds", STUB_SYSTEMD_CREDS),
            ("uname", STUB_UNAME),
            ("id", STUB_ID),
            ("journalctl", STUB_JOURNALCTL),
        ] {
            let path = stub.join(name);
            std::fs::write(&path, text).expect("write a stub");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("make a stub executable");
        }
        let world = Self {
            _base: base,
            root,
            home,
            stub,
        };
        world.state("os", "Linux");
        world.state("arch", std::env::consts::ARCH);
        world.state("systemd.version", "255");
        world.state("user", "provtest");
        world.state("group", "provtest");
        world.state("sudo.mode", sudo_mode);
        world.state("sudo.pwhash", &hex_sha256(PASSWORD.as_bytes()));
        world.state("box.toml", BOX_TOML);
        world
    }

    /// Writes one stub state file.
    fn state(&self, name: &str, value: &str) {
        std::fs::write(self.stub.join(name), value).expect("write a stub state file");
    }

    /// `$1` of every script.
    fn root_text(&self) -> &str {
        self.root.to_str().expect("a UTF-8 root")
    }

    /// The stubs' shared `argv.log`, one line per call. The stub `sudo` logs `sudo -n sh -c`'s
    /// argument, INSTALL_ROOT's text, over many lines; it is folded to `<INSTALL_ROOT>` so a line of
    /// the script never reads as a call.
    fn argv_log(&self) -> Vec<String> {
        std::fs::read_to_string(self.stub.join("argv.log"))
            .unwrap_or_default()
            .replace(INSTALL_ROOT, "<INSTALL_ROOT>")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The `systemctl` calls INSTALL_ROOT makes: every one but the read-only `--version` and
    /// `is-active` that PREFLIGHT and VERIFY make.
    fn systemctl_writes(&self) -> Vec<String> {
        self.argv_log()
            .into_iter()
            .filter(|line| line.starts_with("systemctl "))
            .filter(|line| {
                line != "systemctl --version" && !line.starts_with("systemctl is-active ")
            })
            .collect()
    }

    /// How many password lines the stub `sudo -v` read; `None` when it never read one.
    fn sudo_reads(&self) -> Option<usize> {
        std::fs::read_to_string(self.stub.join("sudo.reads"))
            .ok()
            .map(|text| text.lines().count())
    }

    fn binary(&self) -> PathBuf {
        self.home.join(".local/bin/htui")
    }

    fn credential(&self) -> PathBuf {
        self.root.join("etc/credstore.encrypted/htui-dsn")
    }

    fn unit(&self) -> PathBuf {
        self.root.join("etc/systemd/system/htui-worker.service")
    }

    fn box_toml(&self) -> PathBuf {
        self.home.join(".config/htui/box.toml")
    }

    /// The unit INSTALL_ROOT must write (D295) for `provtest` and this world's home.
    fn expected_unit(&self) -> String {
        let home = self.home.display();
        format!(
            "[Unit]\nDescription=htui worker\nWants=network-online.target\n\
             After=network-online.target\n\n[Service]\nType=exec\nUser=provtest\n\
             ExecStart={home}/.local/bin/htui worker --log {home}/.local/state/htui/worker.log\n\
             LoadCredentialEncrypted=htui-dsn:/etc/credstore.encrypted/htui-dsn\n\
             Restart=on-failure\nRestartSec=10s\n\n[Install]\nWantedBy=multi-user.target\n"
        )
    }
}

/// Runs each remote command as `sh -c <command>`, which is what sshd does with the login shell
/// (V-7), with a cleared environment: `HOME` = the world's home, `PATH` = `<stub>:/usr/bin:/bin`,
/// `STUB_DIR`, `LC_ALL=C`. Records every command for the sweep. With `noise`, the command is
/// prefixed by two greeting lines, as a chatty `~/.bashrc` would print before it.
#[derive(Debug)]
struct LocalShell {
    home: PathBuf,
    stub: PathBuf,
    noise: bool,
    /// Every command as `run_with` built it (never the noisy one), with its stdin's length.
    commands: Mutex<Vec<(String, usize)>>,
}

impl LocalShell {
    fn new(world: &World) -> Self {
        Self {
            home: world.home.clone(),
            stub: world.stub.clone(),
            noise: false,
            commands: Mutex::new(Vec::new()),
        }
    }

    /// The commands so far.
    fn commands(&self) -> Vec<(String, usize)> {
        self.commands.lock().expect("the command log").clone()
    }
}

impl Remote for LocalShell {
    async fn run(&self, command: &str, stdin: &[u8]) -> std::io::Result<RemoteOutput> {
        self.commands
            .lock()
            .expect("the command log")
            .push((command.to_owned(), stdin.len()));
        let line = if self.noise {
            format!("echo \"Welcome to stub-host\"; echo \"htui-motd: hi\"; {command}")
        } else {
            command.to_owned()
        };
        let mut sh = tokio::process::Command::new("sh");
        sh.arg("-c")
            .arg(line)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", format!("{}:/usr/bin:/bin", self.stub.display()))
            .env("STUB_DIR", &self.stub)
            .env("LC_ALL", "C");
        run_piped(sh, stdin).await
    }
}

/// Answers one fixed password (or refusal) and counts its calls. `Debug` never prints it.
struct FakePrompt {
    answer: Result<String, PromptError>,
    calls: AtomicUsize,
}

impl core::fmt::Debug for FakePrompt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FakePrompt").finish_non_exhaustive()
    }
}

impl PasswordPrompt for FakePrompt {
    fn ask(&self) -> BoxFuture<'_, Result<Zeroizing<String>, PromptError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = self.answer.clone().map(Zeroizing::new);
        Box::pin(async move { answer })
    }
}

/// Answers every check with success and logs each call by name, in order. The DSN `baseline`
/// receives is never stored.
#[derive(Debug, Default)]
struct FakeVerifier {
    log: Mutex<Vec<String>>,
}

impl FakeVerifier {
    fn note(&self, call: String) {
        self.log.lock().expect("the verifier log").push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.log.lock().expect("the verifier log").clone()
    }
}

impl Verifier for FakeVerifier {
    fn baseline<'a>(&'a self, _dsn: &'a str) -> BoxFuture<'a, Result<Baseline, String>> {
        self.note("baseline".to_owned());
        Box::pin(async { Ok(Baseline::new()) })
    }

    fn box_seen<'a>(
        &'a self,
        id: BoxId,
        _baseline: &'a Baseline,
        _poll: Poll,
    ) -> BoxFuture<'a, Result<(), String>> {
        self.note(format!("box_seen {id}"));
        Box::pin(async { Ok(()) })
    }

    fn set_executor(&self, id: BoxId) -> BoxFuture<'_, Result<bool, String>> {
        self.note(format!("set_executor {id}"));
        Box::pin(async { Ok(true) })
    }
}

/// Blueprint §5.4: two quick checks inside VERIFY, a short local poll.
const TIMINGS: Timings = Timings {
    verify_tries: 2,
    verify_pause_secs: 0,
    poll: Poll {
        interval: Duration::from_millis(10),
        deadline: Duration::from_millis(100),
    },
};

/// One `run_with` and everything it left behind.
struct Run {
    result: Result<Outcome, ProvisionExit>,
    out: Vec<u8>,
    err: Vec<u8>,
    prompt_calls: usize,
    verifier: Vec<String>,
}

impl Run {
    fn out(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }

    /// `err` as `main` would leave it: the progress lines, then the exit sentence.
    fn stderr(&self) -> Vec<u8> {
        let mut err = self.err.clone();
        if let Err(exit) = &self.result {
            err.extend_from_slice(exit.to_string().as_bytes());
            err.push(b'\n');
        }
        err
    }

    /// The `Outcome`; on an exit, a panic with its sentence (checked sentinel-free first).
    fn outcome(&self) -> &Outcome {
        match &self.result {
            Ok(outcome) => outcome,
            Err(exit) => {
                let sentence = exit.to_string();
                assert!(
                    !SENTINELS.iter().any(|s| sentence.contains(s)),
                    "run_with failed with a sentence that holds a sentinel"
                );
                panic!(
                    "run_with failed: {sentence}\nstderr: {}",
                    String::from_utf8_lossy(&self.err)
                );
            }
        }
    }

    /// The exit, with its code and sentence; panics on success.
    fn exit(&self) -> (u8, String) {
        match &self.result {
            Ok(outcome) => panic!("run_with succeeded: {outcome:?}"),
            Err(exit) => (exit.code(), exit.to_string()),
        }
    }
}

/// What one `run_with` is given beyond the world.
struct Setup<'a> {
    payload: &'a str,
    dsn: &'a str,
    password: Result<&'a str, PromptError>,
    replace_credential: bool,
}

impl Default for Setup<'_> {
    fn default() -> Self {
        Self {
            payload: GOOD_HTUI,
            dsn: DSN,
            password: Ok(PASSWORD),
            replace_credential: false,
        }
    }
}

/// `run_with` over `shell` in `world`, with fresh fakes.
async fn provision(world: &World, shell: &LocalShell, setup: Setup<'_>) -> Run {
    let prompter = FakePrompt {
        answer: setup.password.map(str::to_owned),
        calls: AtomicUsize::new(0),
    };
    let verifier = FakeVerifier::default();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = run_with(Ctx {
        destination: DEST,
        root: world.root_text(),
        replace_credential: setup.replace_credential,
        remote: shell,
        payload: Payload::new(setup.payload.as_bytes().to_vec(), std::env::consts::ARCH),
        dsn: Zeroizing::new(setup.dsn.to_owned()),
        prompter: &prompter,
        verifier: &verifier,
        timings: TIMINGS,
        out: &mut out,
        err: &mut err,
    })
    .await;
    Run {
        result,
        out,
        err,
        prompt_calls: prompter.calls.load(Ordering::SeqCst),
        verifier: verifier.calls(),
    }
}

/// Lowercase hex sha256.
fn hex_sha256(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `haystack` holds `needle`'s bytes.
fn holds(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Every regular file under `dir`, recursively; symlinks are not followed.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        let meta = std::fs::symlink_metadata(&path).expect("stat an entry");
        if meta.is_dir() {
            files.extend(files_under(&path));
        } else if meta.is_file() {
            files.push(path);
        }
    }
    files
}

/// The `mode & 0o777` of `path`.
fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).expect("stat").permissions().mode() & 0o777
}

/// The arguments after `$0` of a `remote_command`.
fn script_args(command: &str) -> Vec<String> {
    shell_words::split(command).expect("the command splits")[4..].to_vec()
}

/// D310: the DSN and the password reach nothing but stdin and the encrypted credential.
fn sweep(world: &World, shell: &LocalShell, out: &[u8], err: &[u8]) {
    for sentinel in SENTINELS {
        for (i, (command, _)) in shell.commands().iter().enumerate() {
            assert!(
                !command.contains(sentinel),
                "remote command {i} holds a sentinel"
            );
        }
        assert!(!holds(out, sentinel), "stdout holds a sentinel");
        assert!(!holds(err, sentinel), "stderr holds a sentinel");
        for file in files_under(&world.stub) {
            let bytes = std::fs::read(&file).expect("read a stub file");
            assert!(
                !holds(&bytes, sentinel),
                "{} holds a sentinel",
                file.display()
            );
        }
    }
    let credential = world.credential();
    for file in files_under(&world.root)
        .into_iter()
        .chain(files_under(&world.home))
    {
        let bytes = std::fs::read(&file).expect("read a remote file");
        assert!(
            !holds(&bytes, "SENTINEL-SUDO-PW"),
            "{} holds the password sentinel",
            file.display()
        );
        if file != credential {
            assert!(
                !holds(&bytes, "SENTINEL-DSN-PW"),
                "{} holds the DSN sentinel",
                file.display()
            );
        }
    }
}

/// [`sweep`] over one run.
fn sweep_run(world: &World, shell: &LocalShell, run: &Run) {
    sweep(world, shell, &run.out, &run.stderr());
}

/// The `Outcome` of a fresh, verified provisioning of [`BOX_TOML`].
fn provisioned() -> Outcome {
    let identity = htui_store::identity::parse_box_toml(BOX_TOML).expect("BOX_TOML parses");
    Outcome::Provisioned {
        box_id: identity.box_id,
        hostname: "stub-host".to_owned(),
        verified: true,
        executor_set: true,
    }
}

/// S1 (a)–(f), shared by S2, S3 and S10.
fn assert_fresh_install(world: &World, run: &Run) {
    // (a), (b)
    assert_eq!(run.outcome(), &provisioned());
    assert_eq!(
        run.out(),
        format!("box {BOX_ID} (stub-host) provisioned on {DEST}\n")
    );
    // (c)
    assert_eq!(
        std::fs::read(world.binary()).expect("the binary"),
        GOOD_HTUI.as_bytes()
    );
    assert_eq!(mode(&world.binary()), 0o755);
    assert!(world.home.join(".local/state/htui").is_dir());
    // (d)
    assert_eq!(mode(&world.root.join("etc/credstore.encrypted")), 0o700);
    let credential = std::fs::read(world.credential()).expect("the credential");
    assert!(
        credential == format!("ENCRYPTED({DSN}\n)").as_bytes(),
        "the credential is not the DSN line, wrapped by the stub systemd-creds"
    );
    // (e)
    assert_eq!(
        std::fs::read_to_string(world.unit()).expect("the unit"),
        world.expected_unit()
    );
    assert_eq!(mode(&world.unit()), 0o644);
    // (f)
    assert_eq!(
        world.systemctl_writes(),
        [
            "systemctl daemon-reload",
            "systemctl enable --now htui-worker.service"
        ]
    );
    let sudo_n = world
        .argv_log()
        .into_iter()
        .filter(|line| line.starts_with("sudo -n sh -c <INSTALL_ROOT> htui-provision-root "))
        .count();
    assert_eq!(sudo_n, 1, "INSTALL_ROOT ran once, under sudo -n");
}

/// S1 (i): no temporary file of PREPARE or INSTALL_ROOT is left.
fn assert_no_leftovers(world: &World) {
    for file in files_under(&world.root)
        .into_iter()
        .chain(files_under(&world.home))
    {
        let name = file.file_name().expect("a name").to_string_lossy();
        assert!(
            !name.contains(".htui.provision.") && !name.contains(".htui-provision."),
            "a temporary file remains: {}",
            file.display()
        );
        assert!(!name.ends_with(".new"), "{} remains", file.display());
    }
}

#[tokio::test]
async fn a_fresh_password_host_is_provisioned() {
    let world = World::new("password");
    let shell = LocalShell::new(&world);
    let run = provision(&world, &shell, Setup::default()).await;

    assert_fresh_install(&world, &run);
    assert_eq!(run.prompt_calls, 1);
    // (g): sudo read the password line and never the DSN line.
    assert_eq!(world.sudo_reads(), Some(1));
    // (h)
    assert_eq!(
        run.verifier,
        [
            "baseline".to_owned(),
            format!("box_seen {BOX_ID}"),
            format!("set_executor {BOX_ID}")
        ]
    );
    // (i)
    assert_no_leftovers(&world);
    // Four sessions: PREFLIGHT, PREPARE (with the payload), INSTALL (password and DSN lines),
    // VERIFY.
    let commands = shell.commands();
    assert_eq!(commands.len(), 4);
    assert_eq!(
        script_args(&commands[1].0).last().map(String::as_str),
        Some("1")
    );
    assert_eq!(commands[1].1, GOOD_HTUI.len());
    assert_eq!(commands[2].1, PASSWORD.len() + 1 + DSN.len() + 1);
    assert_eq!(commands[3].1, 0);
    sweep_run(&world, &shell, &run);
}

#[tokio::test]
async fn a_fresh_nopasswd_host_never_prompts() {
    let world = World::new("nopasswd");
    let shell = LocalShell::new(&world);
    let run = provision(&world, &shell, Setup::default()).await;

    assert_fresh_install(&world, &run);
    assert_eq!(run.prompt_calls, 0);
    assert_eq!(world.sudo_reads(), None, "sudo -v never ran");
    assert_eq!(
        shell.commands()[2].1,
        DSN.len() + 1,
        "INSTALL's stdin is the DSN line"
    );
    assert_no_leftovers(&world);
    sweep_run(&world, &shell, &run);
}

/// The absolute path of `bash`, when there is one.
fn bash_path() -> Option<PathBuf> {
    let out = std::process::Command::new("sh")
        .args(["-c", "command -v bash"])
        .output()
        .ok()?;
    let path = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && path.starts_with('/')).then(|| PathBuf::from(path))
}

#[tokio::test]
async fn a_host_whose_sh_is_bash_keeps_the_sudo_timestamp() {
    let Some(bash) = bash_path() else {
        println!("bash is not on PATH; skipping the E-4 scenario");
        return;
    };
    let world = World::new("password");
    std::os::unix::fs::symlink(&bash, world.stub.join("sh")).expect("link sh to bash");
    // The outer `sh -c` resolves through the cleared environment's PATH, so it is bash too.
    let probe = LocalShell::new(&world);
    let answer = probe
        .run("echo \"${BASH_VERSION:-none}\"", b"")
        .await
        .expect("the probe runs");
    assert_ne!(
        answer.stdout_text().trim(),
        "none",
        "the login shell is bash"
    );

    let shell = LocalShell::new(&world);
    let run = provision(&world, &shell, Setup::default()).await;
    // (a)
    assert_eq!(run.outcome(), &provisioned());
    // (d): `sudo -n` found the timestamp `sudo -v` wrote, so it was forked by the same shell.
    let credential = std::fs::read(world.credential()).expect("the credential");
    assert!(
        credential == format!("ENCRYPTED({DSN}\n)").as_bytes(),
        "the credential is not the DSN line, wrapped by the stub systemd-creds"
    );
    // (g)
    assert_eq!(world.sudo_reads(), Some(1));
    sweep_run(&world, &shell, &run);
}

#[tokio::test]
async fn a_re_run_changes_nothing() {
    let world = World::new("nopasswd");
    let shell = LocalShell::new(&world);
    let first = provision(&world, &shell, Setup::default()).await;
    assert_eq!(first.outcome(), &provisioned());
    sweep_run(&world, &shell, &first);

    let files = [
        world.binary(),
        world.credential(),
        world.unit(),
        world.box_toml(),
    ];
    let before: Vec<Vec<u8>> = files
        .iter()
        .map(|file| std::fs::read(file).expect("read a provisioned file"))
        .collect();
    let calls = shell.commands().len();

    let second = provision(&world, &shell, Setup::default()).await;
    assert_eq!(second.outcome(), &Outcome::AlreadyProvisioned);
    // E-25: PREFLIGHT, then VERIFY to read the box; no PREPARE, no INSTALL.
    let commands = shell.commands();
    assert_eq!(commands.len() - calls, 2, "a re-run runs two sessions");
    for (command, stdin) in &commands[calls..] {
        assert_eq!(*stdin, 0, "a re-run sends nothing on stdin");
        let args = script_args(command);
        assert!(
            args.len() == 1 || args.len() == 4,
            "only PREFLIGHT ($1) and VERIFY ($1-$4) run: {args:?}"
        );
    }
    assert!(
        second.out().contains(&format!(
            "already provisioned with this build; box {BOX_ID} (stub-host)"
        )),
        "out: {}",
        second.out()
    );
    assert_eq!(
        second.verifier,
        ["baseline".to_owned(), format!("set_executor {BOX_ID}")]
    );
    let after: Vec<Vec<u8>> = files
        .iter()
        .map(|file| std::fs::read(file).expect("read a provisioned file"))
        .collect();
    assert!(before == after, "a re-run changed a provisioned file");
    assert_eq!(
        world.systemctl_writes().len(),
        2,
        "no systemctl write on the re-run"
    );
    sweep_run(&world, &shell, &second);
}

#[tokio::test]
async fn replace_credential_re_encrypts_and_restarts() {
    let world = World::new("nopasswd");
    let shell = LocalShell::new(&world);
    let first = provision(&world, &shell, Setup::default()).await;
    assert_eq!(first.outcome(), &provisioned());
    sweep_run(&world, &shell, &first);
    let calls = shell.commands().len();

    let second = provision(
        &world,
        &shell,
        Setup {
            dsn: DSN2,
            replace_credential: true,
            ..Setup::default()
        },
    )
    .await;
    assert_eq!(second.outcome(), &provisioned());
    let commands = shell.commands();
    let (prepare, prepare_stdin) = &commands[calls + 1];
    assert_eq!(*prepare_stdin, 0, "the same build is not uploaded again");
    assert_eq!(script_args(prepare).last().map(String::as_str), Some("0"));
    let credential = std::fs::read(world.credential()).expect("the credential");
    assert!(
        holds(&credential, "db2.example"),
        "the credential is the new DSN"
    );
    assert_eq!(
        world.systemctl_writes().last().map(String::as_str),
        Some("systemctl restart htui-worker.service")
    );
    sweep_run(&world, &shell, &second);
}

#[tokio::test]
async fn a_wrong_sudo_password_writes_nothing_privileged() {
    let world = World::new("password");
    let shell = LocalShell::new(&world);
    let run = provision(
        &world,
        &shell,
        Setup {
            password: Ok("SENTINEL-SUDO-PW-WRONG"),
            ..Setup::default()
        },
    )
    .await;
    let (code, sentence) = run.exit();
    assert!(matches!(run.result, Err(ProvisionExit::Failed(_))));
    assert_eq!(code, 1);
    assert!(sentence.contains("sudo refused the password"), "{sentence}");
    assert!(
        !world.root.join("etc").exists(),
        "nothing privileged was written"
    );
    assert!(
        !world
            .argv_log()
            .iter()
            .any(|line| line.starts_with("systemd-creds")),
        "systemd-creds never ran"
    );
    assert_eq!(world.sudo_reads(), Some(1));
    assert_eq!(
        std::fs::read(world.binary()).expect("PREPARE installed the binary"),
        GOOD_HTUI.as_bytes()
    );
    sweep_run(&world, &shell, &run);
}

#[tokio::test]
async fn an_architecture_mismatch_is_refused_before_any_write() {
    let world = World::new("nopasswd");
    let other = if std::env::consts::ARCH == "x86_64" {
        "aarch64"
    } else {
        "x86_64"
    };
    world.state("arch", other);
    let shell = LocalShell::new(&world);
    let run = provision(&world, &shell, Setup::default()).await;
    let (code, sentence) = run.exit();
    assert!(matches!(run.result, Err(ProvisionExit::Refused(_))));
    assert_eq!(code, 2, "{sentence}");
    assert_eq!(shell.commands().len(), 1);
    assert!(!world.home.join(".local").exists());
    sweep_run(&world, &shell, &run);
}

#[tokio::test]
async fn a_binary_that_does_not_run_is_removed() {
    let world = World::new("nopasswd");
    let shell = LocalShell::new(&world);
    let run = provision(
        &world,
        &shell,
        Setup {
            payload: BAD_HTUI,
            ..Setup::default()
        },
    )
    .await;
    let (code, sentence) = run.exit();
    assert!(matches!(run.result, Err(ProvisionExit::Failed(_))));
    assert_eq!(code, 1);
    assert_eq!(
        sentence,
        "provisioning stub-dest failed: the htui binary does not run on stub-dest: version \
         GLIBC_2.99 not found (required by htui)"
    );
    let bin = world.home.join(".local/bin");
    let left: Vec<String> = std::fs::read_dir(&bin)
        .expect("PREPARE made the bin directory")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(left.is_empty(), "{} holds {left:?}", bin.display());
    assert!(!world.root.join("etc").exists());
    assert_eq!(world.systemctl_writes(), Vec::<String>::new());
    sweep_run(&world, &shell, &run);
}

#[tokio::test]
async fn a_different_installed_build_is_refused() {
    let world = World::new("nopasswd");
    std::fs::create_dir_all(world.home.join(".local/bin")).expect("seed the bin directory");
    std::fs::write(world.binary(), b"other").expect("seed another build");
    std::fs::create_dir_all(world.root.join("etc/systemd/system")).expect("seed the unit dir");
    std::fs::write(world.unit(), b"").expect("seed a unit");
    let shell = LocalShell::new(&world);
    let run = provision(&world, &shell, Setup::default()).await;
    let (code, sentence) = run.exit();
    assert_eq!(code, 2);
    assert!(
        sentence.contains("different build; upgrading is not supported yet"),
        "{sentence}"
    );
    assert_eq!(shell.commands().len(), 1);
    assert_eq!(
        std::fs::read(world.binary()).expect("the seeded binary"),
        b"other"
    );
    sweep_run(&world, &shell, &run);
}

#[tokio::test]
async fn a_chatty_login_shell_is_still_parsed() {
    let world = World::new("nopasswd");
    let shell = LocalShell {
        noise: true,
        ..LocalShell::new(&world)
    };
    let run = provision(&world, &shell, Setup::default()).await;
    assert_eq!(run.outcome(), &provisioned());
    assert_eq!(
        run.out(),
        format!("box {BOX_ID} (stub-host) provisioned on {DEST}\n")
    );
    sweep_run(&world, &shell, &run);
}

#[tokio::test]
async fn a_service_that_never_starts_times_out_with_its_journal() {
    let world = World::new("nopasswd");
    world.state("never-active", "");
    let shell = LocalShell::new(&world);
    let run = provision(&world, &shell, Setup::default()).await;
    let (code, sentence) = run.exit();
    assert_eq!(code, 1);
    for part in [
        "did not start with a box.toml within 0 s",
        "systemctl says activating",
        "stub journal: htui: no Postgres DSN",
    ] {
        assert!(sentence.contains(part), "{part:?} missing from: {sentence}");
    }
    assert_eq!(
        run.verifier,
        ["baseline".to_owned()],
        "box_seen was never called"
    );
    sweep_run(&world, &shell, &run);
}
