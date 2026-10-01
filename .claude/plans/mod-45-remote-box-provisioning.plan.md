# Plan: MOD-45 — Remote box provisioning over SSH

**Source PRD**: `.claude/prds/mod-45-remote-box-provisioning.prd.md`
**Selected Milestone**: 1 (Provision a host) and 2 (Safe re-run and refusals), planned together.
Milestone 2 is the preflight and re-run branches of the same command plus the guide; it does not
split cleanly from milestone 1.
**Complexity**: Medium
**Base**: `de177dd` (`main`), branch `hr/MOD-45`. Decision IDs D291–D314 (the highest in the tree and
on the host is D290, MOD-22).
**Status**: fact-checked (both passes below), CONFIRMED 2026-10-01 (OQ-1 (a), OQ-2 accepted).

## Summary

A new subcommand, `htui provision <destination> [--dsn-stdin] [--replace-credential]`, drives the
user's system `ssh` through four short remote sessions:

1. **Preflight** (unprivileged, read-only).
2. **Prepare** (unprivileged): the log directory, and the binary on stdin when its hash differs.
3. **Install** (privileged). The script reads the sudo password line itself (when one is needed) and
   validates it with `sudo -S -v`. Only then does it run `sudo -n`, whose child encrypts the
   remaining stdin (the DSN) with `systemd-creds encrypt`, writes the unit and starts the service.
4. **Verify**: an in-script loop waits until the service is active and `box.toml` exists, then
   prints it.

Locally, the command then:
- records a database-side baseline before Install and waits for the new box's row to appear or its
  `last_seen_at` to move;
- sets that box's executor to `worker`, so the worker actually claims runs.

Every remote script is a backslash-free constant. Values reach a script only as shell-quoted
positional arguments. The DSN and the sudo password travel only on stdin, never in any `argv`.

## Data flow

```
htui provision host
  local:  refuse unless cfg linux; validate destination
          DSN := keyring (get_dsn → Zeroizing) | --dsn-stdin (read_dsn_line)
          Dsn::parse(dsn)?.host_class(): refuse Loopback | Socket | NoHost
          payload := current_exe() bytes; sha := sha256; arch := consts::ARCH
  ssh#1   sh -c PREFLIGHT root                      stdin: ∅
            → htui.os/arch/systemd/creds/user/group/home/bin_sha/unit/active/sudo lines
              (non-`htui.` lines ignored: ~/.bashrc noise)
          plan::decide(facts, local, replace) → Refuse | AlreadyProvisioned | Steps{upload, sudo}
          [sudo = Password] read the password on /dev/tty, no echo (Zeroizing)
          baseline := local connect (worker_cmd::connect, local root) → boxes() → {id: last_seen_at}
                      (connect fails → baseline None; verification later reports "not verified")
  ssh#2   sh -c PREPARE root home send             stdin: payload | ∅
            → mkdir -p $home/.local/state/htui $home/.local/bin (unprivileged, user-owned)
              [send=1] cat > tmp; chmod; tmp --version || exit 3; mv -f tmp htui
  ssh#3   sh -c INSTALL root user group home replace mode
                                                    stdin: [password\n] DSN\n
            → [mode=password] IFS= read -r pw; sudo -k; printf '%s\n' "$pw" | sudo -S -p '' -v
                              || exit 4; unset pw        (the DSN is still unread)
              sudo -n sh -c INSTALL_ROOT …          (inherits the rest of stdin = the DSN)
                credstore dir; [absent|replace] systemd-creds encrypt - …new && mv | cat >/dev/null
                unit (tmp+mv); daemon-reload; enable --now; [replace] restart
  ssh#4   sh -c VERIFY root home                   stdin: ∅
            → loop ≤ 60 s: is-active (|| true) = active and box.toml present → print both
              else exit 5 + last 20 journal lines
  local:  parse_box_toml(text) → id; poll boxes() ≤ 60 s: id new vs baseline, or last_seen_at >
          baseline[id]; then edit_box(id, edit_version, executor = Worker)
          stdout: "box <id> (<hostname>) provisioned on <dest>"; stderr: agent-login hint
```

## Design decisions (settled here, not in code review)

- **D291 — Subcommand, not a flag.** `Command::Provision(ProvisionArgs)` sits beside
  `Command::Worker` (`cli.rs:71-77`).
  - `destination: String` is positional.
  - `--dsn-stdin` reads one line from local stdin. It is needed when the remote host reaches
    Postgres at a different address than the keyring DSN.
  - `--replace-credential` re-encrypts the DSN on a host that already has one.
  - No argument reads the environment. A test like `no_worker_argument_reads_the_environment`
    (`cli.rs:192-217`) guards `provision` too.
  - The global `--log` goes after the subcommand (`cli.rs:222-231`).
- **D292 — Exit codes mirror `WorkerExit`.** `ProvisionExit { Refused(String) = 2, Failed(String) = 1 }`
  in `provision/mod.rs`. `main.rs` `body()` downcasts to it next to `WorkerExit` (`main.rs:69-76`).
  - A refusal is anything decided **before the first remote write**: local checks, preflight, a
    password prompt aborted. A refusal is not a Sentry report (MOD-41 E-1, `code != 2`).
  - A failure is anything after that, including a sudo rejection inside Install (exit 4). That one
    still happens before any privileged write, and its sentence says so.
- **D293 — Only the local Linux build is shipped (PRD D2).**
  - A non-Linux local build refuses: "provisioning ships this htui binary, which is not a Linux
    build".
  - The payload is `std::env::current_exe()`, hashed with `sha2::Sha256`. `sha2` goes into
    `crates/htui/Cargo.toml` from the workspace (`Cargo.toml:55`). 0.10.9 is already in htui's
    graph, so no new crate is compiled.
  - The architecture check compares `std::env::consts::ARCH` with the remote `uname -m`. Only
    `x86_64` and `aarch64` are accepted, with `arm64` normalised to `aarch64`.
- **D294 — Remote layout.**

  | What | Path |
  |---|---|
  | Binary | `$HOME/.local/bin/htui` |
  | Log | `$HOME/.local/state/htui/worker.log` |
  | Unit | `/etc/systemd/system/htui-worker.service` |
  | Credential | `/etc/credstore.encrypted/htui-dsn` |

  These are the guide's paths (`docs/htui-worker.md:253-316`), except the binary moves from
  `~/.cargo/bin` to `~/.local/bin` because the target has no cargo.
- **D295 — The unit text.** The guide's sample (`docs/htui-worker.md:274-290`), byte for byte, with:
  - `User=<user>`
  - `ExecStart=<home>/.local/bin/htui worker --log <home>/.local/state/htui/worker.log`
  - `LoadCredentialEncrypted=htui-dsn:/etc/credstore.encrypted/htui-dsn`

  It is written from a heredoc inside the `INSTALL_ROOT` constant. `<user>`, `<group>` and `<home>`
  come from preflight and must pass D296 before anything is written. systemd sets `$HOME` for
  `User=` units, so the worker's `dirs::config_dir()` is `<home>/.config` (fact-check V-8).
- **D296 — Value validation.**
  - The remote user and group must match `^[a-z_][a-z0-9_-]{0,31}$`.
  - The home must be absolute, use only bytes in `[A-Za-z0-9_./-]`, and contain no `..`.
  - Anything else is a refusal naming the field, so these values are safe unquoted in a unit file.
  - The destination must not start with `-` and must not contain whitespace or control bytes.
- **D297 — Scripts are constants, values are positional.**
  - The scripts are `const &str` in `provision/script.rs`: POSIX `sh`, **free of backslashes**, and
    each starts with `set -eu`.
  - A remote command is `sh -c '<SCRIPT>' htui-provision <root> <arg>…`, with every value passed
    through `shell_words::quote` (`crates/htui/Cargo.toml:66`).
  - The DSN and the password are never arguments.
  - The remote user's login shell parses this command line once (sshd runs `$SHELL -c`, V-7), so it
    must be POSIX-like: sh, bash, dash, zsh or ksh. fish and csh/tcsh break multi-line single-quoted
    strings. If the preflight output has none of its keys, the refusal reads "the remote login shell
    did not run the preflight; htui provision needs a POSIX login shell (sh, bash, zsh, ksh)". The
    guide says the same.
- **D298 — The `Remote` seam.**
  - `trait Remote { async fn run(&self, command: &str, stdin: &[u8]) -> io::Result<RemoteOutput> }`,
    where `RemoteOutput` is `status` plus `stdout`/`stderr`, each bounded at 64 KiB.
  - `SshRemote` spawns `ssh -T -o ConnectTimeout=15 -- <destination> <command>`. The `--` matters:
    ssh parses options again after the destination (V-7).
  - It uses `stdin(piped)` and `kill_on_drop(true)`. The payload is written from its own task
    **while** stdout/stderr are drained, and stdin is then closed. Writing ~100 MiB first could
    deadlock on a full pipe (V-6).
  - `ssh`'s own prompts go to `/dev/tty` and stay interactive (V-6, PRD D4).
  - `SshRemote` is the only `ssh` spawner in `htui` (PRD metric "SSH not reused"). A pure
    `ssh_argv(dest, command) -> Vec<OsString>` is unit-tested.
- **D299 — Preflight output.** Lines of `htui.<key>=<value>`; lines without the `htui.` prefix are
  ignored (`~/.bashrc` noise, V-7). The keys:

  | Key | Value |
  |---|---|
  | `os` | `uname -s` |
  | `arch` | `uname -m` |
  | `systemd` | major version from `systemctl --version`, or `none` |
  | `creds` | whether `systemd-creds` is on `PATH` |
  | `user` | `id -un` |
  | `group` | `id -gn` |
  | `home` | `$HOME` |
  | `bin_sha` | `sha256sum` of the installed binary, or `none` |
  | `unit` | `yes`/`no` |
  | `active` | `systemctl is-active htui-worker \|\| true` (V-11) |
  | `sudo` | `sudo -k -n true` → `nopasswd`, or `password` (V-4) |

  Using `-k` stops a cached global timestamp from being mistaken for NOPASSWD. A missing key, or
  one listed twice, is a refusal naming it. `Facts::parse` lives in `provision/preflight.rs`.
- **D300 — `plan::decide` is pure and table-tested.** It takes `(facts, local: LocalFacts { arch, sha },
  replace_credential)`. `LocalFacts`, `Decision` and `SudoMode` all live in `plan.rs`. Checked in
  this order:
  1. `Refuse` when:
     - `os ≠ Linux`;
     - the architectures differ;
     - `systemd` is `none` or below 250 (`LoadCredentialEncrypted=` and `systemd-creds` arrived in
       250, V-1; RHEL 9.0 ships exactly 250, V-10);
     - there is no `systemd-creds`.
  2. `Refuse` "already provisioned with a different build; upgrading is not supported yet" when
     `unit = yes` and `bin_sha ≠ local sha`. A host set up by hand from the guide (binary in
     `~/.cargo/bin`) reports `bin_sha = none`, so the refusal adds "or set up by hand".
  3. `AlreadyProvisioned` when `unit = yes`, the sha is the same, `active = active`, and
     `--replace-credential` is not given.
  4. Otherwise `Steps { upload: bin_sha ≠ local sha, sudo: SudoMode::{Password, NoPassword} }`.
- **D301 — The password never reaches a sudo that could pass the DSN on as a retry.** A wrong
  password makes `sudo -S` read the next line as another attempt, up to `passwd_tries` = 3
  (V-3). So the INSTALL script separates the two:
  - **`mode=password`**:
    1. `IFS= read -r pw` takes line 1 only (`read` from a pipe consumes one byte at a time).
    2. `sudo -k` then `printf '%s\n' "$pw" | sudo -S -p '' -v`. `printf` is a builtin, so the
       password is not in any `argv`. sudo's retries read EOF from that pipe and stop (V-3).
    3. Failure → `exit 4`, with the DSN still unread on the outer stdin. Then `unset pw`.
    4. `sudo -n sh -c "$INSTALL_ROOT" …` reuses the timestamp just written. Both sudo calls share
       the parent shell and the session, which is how `timestamp_type=tty` with no tty is keyed
       (V-5). The `sudo -n` child inherits the outer stdin: the DSN.
  - **`mode=nopasswd`**: `sudo -n` directly. No password line is sent.
  - **Safe failures.** A per-command NOPASSWD rule, a second PAM question (OTP) or
    `timestamp_timeout=0` fails `-v` or `-n` safely: the DSN is never read. So does `requiretty`
    (V-9), which fails before any read.
  - Exit 4 maps to "sudo refused the password (or needs a tty or a second factor) on <destination>;
    nothing privileged was written".
  - Session 1b of the first draft is gone.
- **D302 — The local no-echo read.**
  - It uses `crossterm`, already a dependency: raw mode, then `KeyCode::Char` events up to `Enter`
    into a `Zeroizing<String>`. `Esc` or `Ctrl-C` aborts, and an abort is a refusal.
  - Raw mode is restored by a guard's `Drop`.
  - crossterm falls back to `/dev/tty` when stdin is not a terminal, so `--dsn-stdin` and a sudo
    password work together. The read is refused only when `/dev/tty` cannot be opened: "sudo needs
    a password and there is no terminal; configure NOPASSWD or run interactively". This is ssh's
    own rule.
  - The blocking `event::read` runs on a thread of its own, like `read_dsn_apart`
    (`worker_cmd.rs:119-130`).
- **D303 — `INSTALL_ROOT` runs as root, with `set -eu`.**
  1. `install -d -m 0700 /etc/credstore.encrypted`.
  2. If the credential is absent or `replace=1`: `systemd-creds encrypt --name=htui-dsn - …/htui-dsn.new`,
     then `mv -f` it into place. `encrypt` reads stdin to EOF (V-2). The trailing newline is kept
     and trimmed by `headless_dsn`. Otherwise `cat >/dev/null` discards the DSN.
  3. The unit, through a temp file + `mv`.
  4. `systemctl daemon-reload`, then `systemctl enable --now htui-worker.service`, then
     `systemctl restart htui-worker.service` when the credential was replaced.

  The log directory is **not** created here. PREPARE creates it unprivileged (D304), so no
  root-owned `~/.local/state` is left behind, and root never needs `install -o/-g`. A failed
  session is completed by a re-run (milestone 2).
- **D304 — PREPARE is unprivileged and runs unless `AlreadyProvisioned`.**
  1. `mkdir -p "$home/.local/state/htui" "$home/.local/bin"`.
  2. With `send=1`: `cat > "$home/.local/bin/.htui.provision.$$"`, then `chmod 0755`, then run
     `"$tmp" --version`. If that fails, remove `tmp` and `exit 3`. Otherwise `mv -f` it to
     `$home/.local/bin/htui`.
  3. Exit 3 maps to "the htui binary does not run on <destination>: <first stderr line>" (PRD open
     question 2). PREPARE is the first remote write, so this is a failure (D292).
- **D305 — Verification against a database-side baseline.**
  - **Baseline.** Before INSTALL, `verify::baseline(dsn, local_root)` connects through
    `worker_cmd::connect(dsn, local_root, PoolSize::clamped(2))` (`worker_cmd.rs:158`) and records
    `boxes()` as `{id → last_seen_at}`. `use htui_core::store::WriteStore as _;` provides
    `boxes()`, the call `worker_pg.rs:37` also uses.
    - `boxes()` lists the single `app_user`'s boxes (`pg/write.rs:1441`; `seed_if_empty_as` returns
      the oldest `app_user`, `pg/mod.rs:503-506`), so the remote OS user name does not matter.
    - This connect registers or refreshes the **local** box row and persists its registration. That
      is exactly what `htui --index-items` does, and the guide says so.
    - It never connects with the remote identity: a fingerprint mismatch would make registration
      answer `Copied` and mint a stray row.
    - If the connect fails (the DSN reaches Postgres only from the remote network), the baseline is
      `None`.
  - **Remote.** Session 4's script loops every 2 s for up to 60 s **inside one session**, so a
    password-auth user sees one prompt, not thirty. When `is-active` says `active` and `box.toml`
    exists, it prints both. On a timeout it exits 5 with the last 20 lines of
    `journalctl -u htui-worker`, which is a failure.
  - **Local.** `parse_box_toml` reads the id. `verify::box_seen(store, id, baseline)` polls
    `boxes()` for up to 60 s until the id is absent from the baseline or its `last_seen_at` is
    greater than the baseline's. Both timestamps are the database's `clock_timestamp()`, so no
    local clock is involved. The worker stamps `last_seen_at` at registration and on start
    (`htui-worker/src/worker.rs:40-66`).
  - **Baseline `None`, or the poll times out.** A warning, and exit 0: "service active, box <id>;
    not verified in Postgres from here: <reason>".
- **D306 — DSN host class on the existing newtype.**
  - `Dsn::host_class(&self) -> DsnHost { Remote, Loopback, Socket }` in
    `crates/htui-store/src/dsn.rs`, reached as `Dsn::parse(text)?.host_class()`. `scan()` runs
    before sqlx sees the text, so no parameter value is logged (`dsn.rs:3-8`, `:173`).
  - `DsnError::NoHost` (`dsn.rs:54`) is a refusal.
  - `Socket` comes from sqlx's `get_socket()`, or a host starting with `/`.
  - `Loopback` means `localhost`, `127.0.0.0/8` or `::1`, with brackets stripped, and also checks
    `hostaddr`.
  - The class is never computed from `get_host()` defaults, which depend on the environment.
  - `Debug`, `Display` and every error are free of the DSN text.
- **D307 — Output.**
  - `Provision` is dispatched in `lib.rs` **after** `init_tracing(args.log)` and before the `set_dsn`
    branch, so `--log` works the ordinary way.
  - Progress goes to stderr: `provisioning <dest>: preflight`, `… uploading htui (<n> MiB)`,
    `… installing the service`, `… waiting for the worker`, `… executor set to worker`.
  - On success, one line on stdout: `box <uuid> (<hostname>) provisioned on <dest>`.
  - Then the agent-login hint on stderr (per OQ-1).
  - All output goes through injected writers (D309).
- **D308 — Agent login.** See **OQ-1**. It is blocked by one fact: a TUI opened over `ssh -t` on the
  new box has no DSN. The credential is root-owned and unit-only, and a headless ssh session has no
  Secret Service for `--set-dsn`.
- **D309 — `run_with(ctx)`, the test seam.** `pub async fn run_with<R: Remote>(ctx: Ctx<'_, R>) ->
  Result<Outcome, ProvisionExit>`, where `Ctx` holds:
  - the remote;
  - `payload: Payload { bytes, arch, sha }`;
  - the DSN as `Zeroizing<String>`;
  - a `prompter: &dyn PasswordPrompt`;
  - a `verifier: &dyn Verifier`, whose `baseline` / `box_seen` / `set_executor` T4 fakes;
  - timings: interval and deadline;
  - writers: `out`, `err`.

  `run(args, log)` builds the production `Ctx`. The integration test drives `run_with` with a
  `LocalShell: Remote`, which runs `sh -c <command>` with a temp `HOME` and stub directories on
  `PATH`.
  - **Stubs.** They are `sh` files the test writes. Each logs its `argv` and `env`.

    | Stub | Behaviour |
    |---|---|
    | `sudo` | Handles `-k`, `-n`, `-S`, `-v` and `-p ''`. Keeps a per-test timestamp file. Checks a fixture password. Execs the rest |
    | `systemctl` | Records verbs. Answers `--version` and `is-active`. `enable --now` writes `box.toml` under the temp home |
    | `systemd-creds` | Writes stdin to the target path, wrapped in an `ENCRYPTED(` … `)` marker |
    | `uname`, `journalctl` | Canned answers |
    | `id` | Returns the real test user and group, so the real `install` and `mv` work without root |

  - **Root prefix.** Absolute paths under `/etc` go through the `root` positional argument (every
    script's first), which is `""` in production. It is the only test hook in production text, and
    it is not an environment variable.
- **D310 — Sentinel checks.**
  - The sentinels are `DSN = postgres://u:SENTINEL-DSN-PW@db.example:5432/htui` and the password
    `SENTINEL-SUDO-PW`.
  - After every scenario:
    - neither sentinel appears in any stub `argv`/`env` log, in any command string the `Remote`
      received, or in the `out`/`err` writers;
    - under the temp root and home, the DSN sentinel appears only inside
      `etc/credstore.encrypted/htui-dsn`;
    - the password sentinel appears in no file.
  - `ssh_argv` and `remote_command` are also unit-tested with both sentinels absent.
- **D311 — Postgres case.** `crates/htui/tests/provision_pg.rs` is gated on `HTUI_TEST_DATABASE_URL`,
  like `worker_pg.rs:16-17`. It checks:
  - a baseline taken, then a "remote" box registered through `worker_cmd::connect` with its own temp
    root → `box_seen` finds it;
  - an unchanged box already in the baseline is not accepted;
  - a baseline, then a touch of that box → accepted;
  - `set_executor` leaves `Executor::of(settings)` equal to `Worker` and keeps every other key.

  Every connect uses temp roots, never `~/.config/htui`.
- **D312 — Docs.**
  - `docs/htui-worker.md:62-63` ("does not install itself") and `:254` ("nothing installs it for
    you (MOD-45)") are reworded.
  - A new section, "Provisioning a remote box", covers: the flags; what is written where; the POSIX
    login-shell requirement; the refusals and exit codes; re-running; the executor being set to
    `worker`; verification registering the local box; agent login per OQ-1; and what is not done
    (upgrade, uninstall, non-Linux).
  - The table of contents (`:12-22`) gets the entry.
  - The manual systemd sample stays, for hosts without sudo over ssh.
  - `README.md:87`'s subcommand table gets a `provision` row.
- **D313 — The executor is set to `worker`.** Without this, the worker on a `tui` box idles
  (`docs/htui-worker.md:35,82-84`). After `box_seen`, `verifier.set_executor(id)` calls
  `WriteStore::edit_box(id, record.row.edit_version, BoxEdit { executor: Some(Executor::Worker),
  ..Default::default() })` (`traits.rs:511`; `BoxEdit` `box_.rs:157-165`, which writes that key
  only).
  - `Stale` is retried once after a re-read.
  - Any other outcome is a warning naming Settings › Boxes.
  - If verification was skipped, the hint says to set it by hand.
- **D314 — Scope guard.** `htui-worker`, `htui-orch`, `htui-agent` and `htui-core` are not modified.
  `htui-store` changes only `identity.rs` (`parse_box_toml`) and `dsn.rs` (`host_class`).

## Open questions for the maintainer (answer at CONFIRM)

**Answered 2026-10-01 at CONFIRM ("confirm" over the recommendations): OQ-1 → (a), OQ-2 → accept.**
The plan implements both. The `R-STO-1` amendment text is written by the main thread at close-out:
"…or be read once from the worker's or a TUI session's stdin and held only in memory."

- **OQ-1 — Agent login on the new box.** PRD D4 and the item text say "agent login via MOD-22's
  paste-back". But MOD-22's pane lives in the TUI, and a TUI started over `ssh -t` on the new box
  cannot connect: the DSN is a root-owned unit credential, and there is no Secret Service in an ssh
  session. Options:
  - **(a) Recommended.** Add `htui --dsn-stdin` for the TUI: one line from stdin, held in memory for
    that session and never stored. `StartOptions.dsn` already exists (`connect.rs:200-213`). This
    needs a one-sentence `R-STO-1` amendment extending "read once from the worker's stdin" to "or a
    TUI session's". It adds files `cli.rs` (already T3's), `lib.rs` (T3's) and a guard test, and
    the hint becomes `ssh -t <dest> ~/.local/bin/htui --dsn-stdin`, then Settings › Agents.
  - **(b)** Leave it out of MOD-45. The guide documents the vendor CLI's own login over `ssh -t` as
    the stopgap, and a follow-up item is minted for in-app login on a headless box. This is at odds
    with `R-AGT-9` "from the app" until that item lands.
- **OQ-2 — Executor default (D313).** The plan sets the new box's executor to `worker`
  automatically, because provisioning a worker that then idles defeats the PRD hypothesis. Accept,
  or keep it manual.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Subcommand + exit | `crates/htui/src/worker_cmd.rs:18-43`, `run`/`serve` | `Refused`/`Failed` sentences, `code()`, `Display` = the sentence only |
| Exit mapping | `crates/htui/src/main.rs:69-76` | `downcast_ref` → code; `code != 2` → Sentry |
| CLI | `crates/htui/src/cli.rs:71-90`, tests `:151-235` | clap derive; parse tests; "no argument reads the environment" |
| Blocking read beside async | `worker_cmd.rs:119-130` `read_dsn_apart` | own thread + oneshot |
| DSN in memory | `crates/htui-store/src/secret.rs:113`, `:225-232` | wrap `get_dsn` in `Zeroizing` at once; errors never carry the DSN |
| DSN parse | `crates/htui-store/src/dsn.rs:119` `Dsn::parse`, `:173` `scan` | scan before sqlx; redacting `Debug` |
| Headless connect | `worker_cmd.rs:158-166` `connect` | `load_or_mint(root)` → `connect_headless` → `persist_registration` |
| Child process | `crates/htui-orch/src/isolate/git.rs:316`, `:381` | `tokio::process::Command`, `kill_on_drop`, captured output |
| Tests (Pg) | `crates/htui/tests/worker_pg.rs:16-37` | env-gated; `WriteStore as _` |
| Sentinels | `crates/htui/src/agent_worker.rs:7109`, `:8101` (`SENTINEL-TOKEN-VALUE`) | asserted absent from every sink |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-store/src/identity.rs` | UPDATE | `parse_box_toml` (path-free error; `load_or_mint` wraps it with the path) | T1 |
| `crates/htui-store/src/dsn.rs` | UPDATE | `DsnHost`, `Dsn::host_class` (D306) | T1 |
| `crates/htui/src/provision/mod.rs` | CREATE (T2 skeleton) / UPDATE (T3) | T2: `pub mod plan; pub mod preflight; pub mod script;` + module doc. T3: `ProvisionExit`, `Ctx`, `run`, `run_with` | T2 → T3 |
| `crates/htui/src/lib.rs` | UPDATE | T2: `pub mod provision;`. T3: dispatch after `init_tracing` | T2 → T3 |
| `crates/htui/src/provision/script.rs` | CREATE | the constants + `remote_command` | T2 |
| `crates/htui/src/provision/preflight.rs` | CREATE | `Facts::parse` | T2 |
| `crates/htui/src/provision/plan.rs` | CREATE | `LocalFacts`, `Decision`, `SudoMode`, `decide`, validators | T2 |
| `crates/htui/src/provision/remote.rs` | CREATE | `Remote`, `RemoteOutput`, `SshRemote`, `ssh_argv` | T3 |
| `crates/htui/src/provision/secret_prompt.rs` | CREATE | `PasswordPrompt`, the crossterm no-echo prompt | T3 |
| `crates/htui/src/provision/verify.rs` | CREATE | `Verifier`, Pg impl: `baseline`, `box_seen`, `set_executor` | T3 |
| `crates/htui/src/cli.rs` | UPDATE | `Command::Provision`, `ProvisionArgs`, tests (+ OQ-1a's TUI `--dsn-stdin`) | T3 |
| `crates/htui/src/main.rs` | UPDATE | `ProvisionExit` downcast | T3 |
| `crates/htui/Cargo.toml` | UPDATE | `sha2 = { workspace = true }` | T3 |
| `crates/htui/tests/provision.rs` | CREATE | LocalShell + stubs, scenarios, sentinel sweep | T4 |
| `crates/htui/tests/provision_pg.rs` | CREATE | D311 | T4 |
| `docs/htui-worker.md` | UPDATE | D312 | T5 |
| `README.md` | UPDATE | `:87` row | T5 |
| `docs/REQUIREMENTS.md` | UPDATE | only if OQ-1 (a): the `R-STO-1` amendment (main thread, at close-out) | — |

**Independence (checked by intersecting file sets).**
- T1 = {identity.rs, dsn.rs}.
- T2 = {provision/mod.rs, lib.rs, script.rs, preflight.rs, plan.rs}.
- T5 = {docs/htui-worker.md, README.md}.
- These three are pairwise disjoint, so **Wave A** runs them in parallel.
- T3 overlaps T2 on `provision/mod.rs` and `lib.rs`, and uses T1's API, so it runs after Wave A.
- T4 uses T3's API, so it runs after T3.

## Tasks

TDD per task: tests first, red, then green. Each implementer commits its own work as it goes.

### Task 1: store helpers (D305, D306)
- **Action**: `pub fn parse_box_toml(text: &str) -> Result<Identity>`. The hostname comes from the
  file. `load_or_mint` uses it and keeps its error sentence (with the path). `pub enum DsnHost` and
  `Dsn::host_class`.
- **Tests first**:
  - `parse_box_toml`: round trip with `store`; malformed text → `Err` naming "box.toml" without a
    path.
  - Host classes for:
    - `postgres://u:p@localhost/db`
    - `…@127.0.0.5/db`
    - `…@[::1]:5432/db`
    - `…@/db?host=/var/run/postgresql`
    - `…@db.example/db`
    - `…@10.0.0.3/db`
    - `…@db.example/db?hostaddr=127.0.0.1`
    - a hostless URL (`NoHost`)
  - No `Debug`, `Display` or error contains `SENTINEL-DSN-PW`.
- **Mirror**: `Dsn::parse`, `split_host_port`.
- **Validate**: `cargo test -p htui-store --all-features --lib -- identity:: dsn:: --test-threads=1`.

### Task 2: skeleton, scripts, preflight parse, decide (D296, D297, D299, D300, D301, D303, D304)
- **Action**:
  - The `provision/mod.rs` skeleton and `pub mod provision;` in `lib.rs`. Every item is `pub` and
    documented, to satisfy `-D warnings` and `missing_docs`.
  - `script.rs`: `PREFLIGHT`, `PREPARE`, `INSTALL`, `INSTALL_ROOT` and `VERIFY`, plus
    `remote_command`.
  - `preflight.rs`: `Facts::parse`.
  - `plan.rs`: `LocalFacts`, `Decision`, `SudoMode`, `decide` and the validators.
- **Tests first**:
  - `decide`: one table row per D300 branch, in order.
  - The validators: accept/refuse tables.
  - `Facts::parse`: a missing key, a duplicate, noise lines ignored.
  - `remote_command`: round-trips through `shell_words::split`, with each argument exactly once.
  - The scripts: each contains no backslash, starts with `set -eu`, and passes `sh -n` (a unit
    test spawning `sh -n -c`; the sandbox `sh` is dash).
  - `INSTALL` never mentions `$pw` outside the `read` / `printf` / `unset` lines.
- **Validate**: `cargo test -p htui --all-features --lib provision:: -- --test-threads=1`.

### Task 3: command, transport, prompt, verifier, CLI (D291–D295, D298, D302, D305, D307, D309, D313; OQ-1)
- **Action**:
  - `ProvisionExit`, `Ctx`, `run` and `run_with`, doing the orchestration in the data-flow order.
  - `SshRemote` with concurrent stdin writing.
  - The crossterm prompt on its own thread.
  - `PgVerifier` over `worker_cmd::connect`.
  - The CLI, the `lib.rs` dispatch after `init_tracing`, the `main.rs` downcast, and `sha2`.
  - OQ-1 (a), if accepted: the TUI's `--dsn-stdin` into `StartOptions.dsn` (plain `String` today;
    wrapped where read), mutually exclusive with `--demo`/`--offline`/`--set-dsn`/`--clear-dsn`.
- **Tests first**:
  - `cli.rs`: `provision` parses its positional and both flags, and rejects a missing destination
    and `--dsn`. `no_provision_argument_reads_the_environment`.
  - `ProvisionExit::code`.
  - `ssh_argv` puts `--` before the destination, with no sentinel.
  - `run_with` over a scripted fake `Remote` + fake `Verifier`:
    - the stdin payload per session (`pw\nDSN\n`, `DSN\n`, payload, ∅);
    - the upload is skipped when the hashes match;
    - `AlreadyProvisioned` → no PREPARE or INSTALL;
    - a local refusal (loopback DSN, bad destination) → zero `Remote` calls;
    - exit 3/4/5 → the mapped sentence;
    - `set_executor` is called exactly once after `box_seen`.
- **Validate**: `cargo test -p htui --all-features provision -- --test-threads=1`; clippy.

### Task 4: end-to-end over the real scripts (D309–D311)
- **Action**: `tests/provision.rs` (`#![cfg(unix)]`). Scenarios:
  - fresh host with password sudo;
  - fresh host with NOPASSWD;
  - re-run → `AlreadyProvisioned`, no INSTALL;
  - `--replace-credential` → credential rewritten, `restart` recorded;
  - **wrong sudo password** → exit 4, nothing under `<root>/etc`, and `systemd-creds` never invoked;
  - arch mismatch → refusal, no remote write;
  - `--version` fails → exit 3, tmp removed, no unit;
  - different installed build → refusal;
  - preflight output wrapped in `.bashrc` noise → still parsed;
  - VERIFY timeout → exit 5 with the journal tail.

  Every scenario runs the D310 sweep. Add `tests/provision_pg.rs` per D311.
- **Validate**: `cargo test -p htui --all-features --test provision --test provision_pg --
  --test-threads=1`.

### Task 5: guide (D312; OQ-1's hint)
- **Action**: the edits listed in D312, quoting the sentences fixed in D292–D305 and D313.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; links resolve.

## Test plan

- **Unit**: T1, T2, T3.
- **Script end to end**: T4 `provision.rs`, where the real `sh` scripts run against stubs. It runs in
  the sandbox, which has no `ssh`, `systemctl` or `sudo`.
- **Postgres**: T4 `provision_pg.rs`.
- **Live check (maintainer, on the host after `scripts/hr collect MOD-45`)**. Run
  `htui provision <LAN host>` against at least one host with C sudo, and one with sudo-rs if
  available (Ubuntu 25.10+/26.04 ship it by default, V-10). Confirm:
  - The worker journal shows `htui worker ready`.
  - The box appears in Settings › Boxes with executor `worker`.
  - A re-run prints "already provisioned".
  - A deliberately wrong sudo password exits 4 and writes nothing under `/etc`.
  - `ps -ef` / `/proc/*/cmdline` on both machines never show the DSN or the password during a run.
  - With OQ-1 (a): `ssh -t <host> ~/.local/bin/htui --dsn-stdin` opens the TUI, and Settings ›
    Agents can log an agent in by paste-back.

  Record the results in `docs/decisions/mod/mod-45.md`.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — sudo reads more than one line | Low | Verified in source for C sudo (`tgetpass.c:403-425`) and sudo-rs (`rpassword.rs:188-250`, with the test at `:496-513`). D301 also never gives sudo the DSN's stdin until after `-v` succeeds. The live check is the proof |
| **R-2** — Stubs model sudo or systemd wrongly | Medium | Stubs model only the flags the scripts use. The live check is required before close-out |
| **R-3** — A non-POSIX login shell (fish, csh) on the target | Low | Preflight keys are missing → a named refusal (D297). The guide states the requirement |
| **R-4** — ~100+ MiB upload | Medium | Skipped when the hashes match. The size is printed. The guide recommends a release build |
| **R-5** — `ort` loads lazily, so `--version` passes while indexing fails | Low | Indexing on the box needs a Qdrant URL in *its* keyring, which the service lacks (`concepts.rs:279-310`). Stated in the guide |
| **R-6** — Verification writes the local box row | Accepted | Same as `--index-items`. Documented (D305) |
| **R-7** — Decision IDs collide with another sandbox | Medium | Renumber at merge |
| **R-8** — The htui suite is scheduling-dependent | Medium | Every gate runs `--test-threads=1` |
| **R-9** — The PAM stack asks a second question | Low | Fails safe (D301, exit 4). Documented as "needs NOPASSWD or a single-password sudo" |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
cargo tree -p htui -i sha2@0.10.9        # one 0.10 instance; 0.11.0 is also in the lock
git diff --stat de177dd -- crates/htui-worker crates/htui-orch crates/htui-agent crates/htui-core  # empty
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Acceptance

- [ ] With stubbed remote tools, `htui provision <dest>` on a fresh host does all of this: installs
      the binary, the credential and the unit; starts the service; verifies the box against the
      baseline; sets the executor to `worker`; reports the box id. The DSN and password sentinels
      are absent from every `argv`, `env`, log, output and plaintext file (T4).
- [ ] Every D300 refusal happens before any remote write and exits 2. A wrong sudo password exits
      4, and `systemd-creds` never runs.
- [ ] A re-run reports "already provisioned" with no PREPARE or INSTALL. `--replace-credential`
      rewrites the credential and restarts the service.
- [ ] `box_seen` and `set_executor` hold against Postgres (D311).
- [ ] The guide covers provisioning, refusals, re-running, the executor and agent login (per OQ-1).
      The validator is green.
- [ ] The workspace gate is green. D314's scope guard holds.
- [ ] The maintainer's live check is recorded in the write-up.

## Where the HANDOFF or tree disagree

1. The HANDOFF item (and ANA-16 §8 item 6) say "a **user** service". The tree's own guide uses a
   **system** unit with `User=`, because user units can load encrypted credentials only from
   systemd 256 (`docs/htui-worker.md:255-257`; V-2). The plan follows the guide (PRD D1). The
   close-out corrects the HANDOFF wording.
2. The item says agent login is "via MOD-22's paste-back", but a TUI on the provisioned box has no
   DSN (OQ-1).
3. The guide's sample binary path is `~/.cargo/bin`. D294 uses `~/.local/bin` for provisioned hosts.
4. `docs/htui-worker.md` presents the executor switch as a TUI action on the box itself. For a
   provisioned box, D313 does it from the provisioning machine.

---

## Verified claims

Checked 2026-10-01 against `de177dd` by two parallel passes: a tree pass and an external-tools pass
over upstream source and man pages. The first draft's falsified claims are amended above. Each row
names the decision or section it changed.

| claim | verdict | evidence |
|---|---|---|
| `Command` enum with `Worker` | verified | `cli.rs:71-77` |
| "no argument reads the environment" test exists for `worker` | verified | `cli.rs:192-217`; `provision` needs its own (D291) |
| `--log` is global and goes after the subcommand | verified | `cli.rs:25`, `:222-231` |
| `main.rs` downcasts only `WorkerExit`; code 2 skips Sentry | verified | `main.rs:69-76` |
| `WorkerExit` 2/1; `worker_cmd::connect` pub, used by integration tests | verified | `worker_cmd.rs:18-43`, `:158`; `lib.rs:37`; `tests/worker_pg.rs:25` |
| `Provision` dispatched "before `init_tracing` like `Worker`" while `--log` goes "through the ordinary `init_tracing`" | **false** (contradiction) | `lib.rs:79-85`; amended D307: dispatch after `init_tracing` |
| `shell-words`, `crossterm` are htui deps; `toml` is not | verified | `crates/htui/Cargo.toml:66`, `:39` |
| `sha2` adds no compiled crate | verified | 0.10.9 already in htui's graph |
| `cargo tree -p htui -i sha2` | **false** | ambiguous: 0.10.9 and 0.11.0 in `Cargo.lock:5923,5934`; amended Validation to `sha2@0.10.9` |
| `BoxToml` private; `load_or_mint` parses inline, error includes the path | verified | `identity.rs:36-39`, `:74-77`; `parse_box_toml` returns a path-free error (T1) |
| `dsn_host_class` via `PgConnectOptions::from_str` like `db_fingerprint` | **false / unsafe** | sqlx 0.9 logs unrecognised parameter values (`dsn.rs:3-8`); amended D306 to `Dsn::parse` + `host_class` |
| socket = host starting with `/` | partially true | sqlx routes `/` hosts to `get_socket()`; hostless URLs default by environment; amended D306 |
| keyring fake serves `get_dsn` | verified | `secret.rs:113-122`; `testkit.rs:300` |
| `connect_headless` signature; `PoolSize::clamped`; `CONNECT_TIMEOUT` | verified | `pg/mod.rs:284-289`, `:181`, `:46` |
| `boxes()` lists the single app user's boxes; `last_seen_at` on `BoxRow` | verified | `pg/write.rs:1441`; `pg/mod.rs:503-506`; `box_.rs:61` |
| `last_seen_at ≥ started_at − 5 s` is sound | **partially true** | server `clock_timestamp()` vs local clock; amended D305 to a DB-side baseline |
| a local `connect_headless` for verification is read-only | **false** | `bootstrap` registers the local box (`pg/mod.rs:711-721`); amended D305 to go through `worker_cmd::connect` with an injected root, documented as R-6 |
| worker stamps `last_seen_at` at start and every 60 s | verified | `htui-worker/src/worker.rs:24,40-66`; `connect.rs:38` |
| `spawn_index_job` needs a Qdrant URL in the keyring | verified | `concepts.rs:279-310` |
| `worker_pg.rs` is env-gated | verified | `worker_pg.rs:16-17` (no `required-features`) |
| guide systemd section facts | verified | `docs/htui-worker.md:253-316` |
| T5 covers every "nothing installs it" line | partially true | also `:62-63`; amended D312 |
| a provisioned worker runs runs | **false** | the executor defaults to `tui`, so a worker idles (`docs/htui-worker.md:35,82-84`); added D313 |
| `WriteStore::edit_box` can set another box's executor | verified | `traits.rs:511`; `BoxEdit.executor` `box_.rs:157-165` |
| T2 compiles without `mod.rs` via a `#[path]` scratch | **false** | `LocalFacts` was in T3's `mod.rs`; amended: T2 owns the skeleton, `LocalFacts` moves to `plan.rs` |
| T1 validation command | **false** | one TESTNAME before `--`; amended |
| no-echo prompt must refuse when stdin is not a tty | partially true | crossterm falls back to `/dev/tty` (`file_descriptor.rs:124-154`); amended D302 |
| log dir via root `install -d -o` | **false** | leaves a missing `~/.local/state` root-owned; amended D303/D304 (unprivileged PREPARE) |
| a TUI over `ssh -t` can log agents in | **false** | no DSN reachable (root-only credential, no Secret Service); raised as OQ-1 |
| V-1 `LoadCredentialEncrypted=` / `systemd-creds` since 250 | verified | systemd NEWS "CHANGES WITH 250"; `systemd-creds.xml` v250 tags |
| V-2 encrypt with the host key needs root before 256; `encrypt -` reads stdin to EOF | verified (stdin); partially (root) | `creds.c:576-581`; NEWS 256 `--user`; TPM-only edge irrelevant under sudo |
| V-3 `sudo -S` reads byte-wise to `\n`; wrong password reads more lines (3 tries) | verified | `tgetpass.c:403-425`; `sudo_auth.c:314`; `configure.ac:220`; sudo-rs `rpassword.rs:188-250` — drove D301's redesign |
| V-4 `-p ''`, `-k`, `-n` semantics | verified | `tgetpass.c:225-228`; `sudo.man.in` `-k`/`-n`; sudo-rs docs |
| V-5 no-tty timestamps key on ppid and session | verified | `timestamp.c:425-441`, `:909`; `global` crosses sessions; amended D299 (`sudo -k -n`) and D301 (`-k` before `-v`) |
| V-6 `ssh -T` forwards stdin, EOF propagates, prompts via `/dev/tty` | verified | `readpass.c:119-165`; `channels.c:2301-2307`; amended D298 (concurrent stdin write) |
| V-7 destination is the first non-option; the command is joined and run by `$SHELL -c` | partially true | options are re-parsed after the destination unless `--` is given (`ssh.c:1070-1072`, `:1112-1114`); non-login shell; fish/csh quoting; amended D297/D298/D299 |
| V-8 systemd sets `$HOME` for `User=` units | verified | `systemd.exec.xml:4229-4241`; `dirs` falls back to `getpwuid_r` |
| V-9 `requiretty` fails `sudo -S` under `ssh -T` | verified | `sudoers.c:454-457`, before any read; fails safe (D301) |
| V-10 Ubuntu 24.04 = 255, Debian 12 = 252, RHEL 9 ≥ 250 | partially true | RHEL 9.0/9.1 = 250, 9.2+ = 252; all ≥ 250; Ubuntu 25.10+/26.04 default to sudo-rs (live check covers it) |
| V-11 `systemctl is-active` prints `active` and exits 0 | verified | `systemctl.xml:211-219`; non-zero otherwise → `\|\| true` under `set -eu` (D299) |
