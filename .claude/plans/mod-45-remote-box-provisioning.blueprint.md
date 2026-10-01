# Blueprint: MOD-45, remote box provisioning over SSH

**Status**: PROPOSED 2026-10-01 by the code-architect, from the plan CONFIRMED the same day (OQ-1
answered (a): a TUI `--dsn-stdin`; OQ-2 accepted: provisioning sets the new box's executor to
`worker`). Hazards E-1…E-24 (§0.2) are this blueprint's. **Major** means a script, signature, test
or behaviour in the plan is wrong, cannot be written as stated, or is ambiguous enough that two
implementers would build different things. **Minor** is placement, wording, a gate, or a hazard with
a cheap guard. Where §0.2 amends the plan, this file wins. **No decision IDs are minted here**
(plan R-7): a resolution is cited by its E-number.

**Plan**: `.claude/plans/mod-45-remote-box-provisioning.plan.md` (CONFIRMED, fact-checked). Its
D291–D314, R-1…R-9, Tasks, waves and "Verified claims" table are binding and are not reopened,
except where E-1…E-24 show that the plan's own text cannot do what it says or leaves a choice open.
**PRD**: `.claude/prds/mod-45-remote-box-provisioning.prd.md`.

**Verified at**: HEAD `ce115bc` on `hr/MOD-45`. `git diff --stat de177dd HEAD -- crates Cargo.toml
Cargo.lock docs README.md` is empty, so the plan's line numbers are HEAD's. **Line numbers are
pre-edit**: once a task commits, a citation into a file it edits has moved. `df -h /`: **61 GB free
(87 % used)**. rustc 1.98.1. `/bin/sh` is **dash 0.5.12**; bash 5.2.21 is installed. There is no
`ssh`, `sudo`, `systemctl` or `journalctl` here (`HR_SANDBOX=1`); `sha256sum`, `install`, `mktemp`,
`id` and `uname` exist.

**Tooling**: the Gortex daemon calls this checkout INACTIVE but answered every symbol, source and
file query used here (`cli.rs`, `lib.rs`, `main.rs`, `worker_cmd.rs`, `secret.rs`, `connect.rs`,
`dsn.rs`, `identity.rs`, `connection.rs::snapshot`, `concepts.rs::open`, `pg/mod.rs`
`register_box`/`touch_box`, `pg/write.rs::edit_box`, `box_.rs`, `traits.rs`, `worker_pg.rs`). Docs
were read with `Read`. The sqlx-postgres 0.9.0 (`options/parse.rs`, `options/mod.rs`) and
shell-words 1.1 (`quote`, `escape_style`) sources were read in `~/.cargo/registry`. **The five
scripts in §3.2 were extracted from this file after it was written and checked with `dash -n` and
`bash -n`**. PREFLIGHT and VERIFY were also run read-only under dash and bash. Three behaviours were
observed on this machine: bash 5.2 execs the last simple command of an `sh -c` string while dash
does not (E-4); `IFS= read -r` takes exactly one line from a pipe under both shells; and an `EXIT`
trap keeps the exit status. No file outside this blueprint was written.

**Coupling verdict.** The plan's waves hold: **Wave A = T1 ∥ T2 ∥ T5**, then **T3**, then **T4**.
- The three Wave A file sets are pairwise disjoint (§1). T2 uses nothing from T1: the DSN class is
  T3's.
- If Wave A runs in worktrees, each worktree builds its own `target/` (~10 GB; 61 GB free). Delete
  each worktree before its branch.
- T3 is the only task that touches `Cargo.lock`.
- T4 may need to correct a script in `provision/script.rs` (T2's file). That is allowed and is
  T4's only file outside `tests/` (§5.8).

**Scope at a glance**:
- **New**: `crates/htui/src/provision/{mod,plan,preflight,script,remote,secret_prompt,verify}.rs`,
  `crates/htui/tests/provision.rs`, `crates/htui/tests/provision_pg.rs`.
- **Changed**:
  - `crates/htui-store/src/identity.rs` and `crates/htui-store/src/dsn.rs`.
  - `crates/htui/src/{cli,lib,main}.rs` and `crates/htui/Cargo.toml`.
  - `Cargo.lock`: htui's dependency list gains `"sha2 0.10.9"`. No `[[package]]` is added.
  - `docs/htui-worker.md` and `README.md`.
- **Unchanged**:
  - `htui-worker`, `htui-orch`, `htui-agent` and `htui-core` (D314).
  - `htui-store`'s `lib.rs` and `connect.rs` (E-14).
  - Every migration, `.sqlx` and snapshot.

**House style (carried)**:
- Lints and formatting:
  - `unsafe_code = "forbid"`.
  - `missing_docs`, `missing_debug_implementations` and `unused_qualifications` warn.
  - clippy `all` at `-D warnings`, never `#[allow]` to get green. The one allowed attribute is
    `async_fn_in_trait` with a reason, which has precedent at `concepts.rs:210` (E-6).
  - rustdoc denies broken and **private** intra-doc links.
  - `max_width = 100`.
- Every new `pub` item has a doc comment and a `Debug` impl. A `Debug` impl never prints a DSN, a
  password or the payload bytes.
- Commits:
  - Red tests come first. A `todo!()` body goes only where no existing path calls it.
  - Every commit compiles, and no test is loosened.
  - Implementers stage their own paths only. Never `git add -A`, `stash` or `--amend`.
  - Commit after each green step.
- **Sentinels**:
  - DSN `postgres://u:SENTINEL-DSN-PW@db.example:5432/htui`, password `SENTINEL-SUDO-PW` (D310).
  - No test name, assertion message, `tracing` call or panic prints either one.
  - Every test that holds one asserts it is absent from the outputs it checks.

---

## 0. Environment, hazards

### 0.1 Environment and gates (this sandbox)

```bash
# Already set by the sandbox: HTUI_TEST_DATABASE_URL (Postgres at localhost:5439). Nothing to export.
pg_isready -h localhost -p 5439   # only T4's provision_pg and the final gate need it
df -h /                           # before each task; target/ fills the disk (dev Postgres crash loop)
```

Every `cargo test` line runs `--all-features -- --test-threads=1`. The htui suite's green depends on
scheduling, because the keyring fake is process-wide. Postgres suites run only in T4 and in the final
gate. Before believing a Postgres failure, run `df -h /` and re-run the case alone.

### 0.2 Hazards: where the plan is wrong or underspecified

| # | Severity | Task | Plan says | Tree / reading | Resolution |
|---|---|---|---|---|---|
| **E-1** | **Major** | T1 | Test row `…@/db?host=/var/run/postgresql` is a `Socket`. | `scan` (`dsn.rs:173`) takes the host from the URL authority. That authority is `u:p@`, so the host is empty and `Dsn::parse` returns `Err(DsnError::NoHost)`. `host_class` is never reached. | That row expects `Err(NoHost)`. The `Socket` rows become `…@localhost/db?host=/var/run/postgresql` and `…@%2Fvar%2Frun%2Fpostgresql/db`. Full table in §2.3. |
| **E-2** | **Major** | T1 | D306: classify the host, "also checks `hostaddr`". | sqlx applies query `host=` and `hostaddr=` **after** the authority, in order (`options/parse.rs` `"host"`/`"hostaddr"` arms). So `…@db.example/db?host=localhost` dials loopback. A query `host=/…` sets `socket` and nothing clears it. `PgConnectOptions::from_str` reads the environment only for defaults that the URL host then overwrites, and `scan` has already guaranteed a URL host. | `host_class` re-parses with `PgConnectOptions::from_str`, as `summary` does, safe because `scan` passed. `get_socket().is_some()` gives `Socket`. Otherwise `classify_host(get_host())`. This covers `host=`, `hostaddr=` and percent-encoded socket hosts in one place, and never sees an environment default (§2.2). |
| **E-3** | **Major** | T2 | D301: `printf '%s\n' "$pw" \| sudo -S -p '' -v`. | `'%s\n'` contains a backslash, so it breaks D297's backslash-free rule. | INSTALL sets `nl="<newline>"` (a literal newline inside double quotes, at column 0), then runs `printf "%s%s" "$pw" "$nl"`. `printf` is a builtin in dash, bash and busybox, so the password is in no `argv`. |
| **E-4** | **Major** | T2, T4 | D301: `sudo -n sh -c "$INSTALL_ROOT" …` reuses the timestamp `-v` wrote, because "both sudo calls share the parent shell". | When `/bin/sh` is bash (Fedora, RHEL, Arch, openSUSE), bash 5.2 **execs** the last simple command of a `-c` string even after other commands. Observed here: `bash -c 'echo $$; true; bash -c "echo \$PPID"'` prints the *grandparent's* pid. Dash 0.5.12 does not. An exec'd `sudo -n` has a different ppid from the forked `sudo -v`. With `timestamp_type=tty` and no tty, the record is keyed on ppid (V-5), so `sudo -n` answers "a password is required" on **every password host whose `/bin/sh` is bash**. A pipeline element (`printf … \| sudo -v`) is always forked, which was observed too. | INSTALL's last two lines are `sudo -n sh -c … \|\| rc=$?` and `exit "$rc"`, so `sudo -n` is never the last command and is always forked (observed: ppid = the shell). A T2 test pins the guard. T4 scenario S3 runs the whole chain with `sh` → bash, and its stub `sudo` keys the timestamp on `$PPID`. |
| **E-5** | **Major** | T2 | D297: scripts are "free of backslashes". D301 writes `-p ''`. | `shell_words::quote` turns every `'` into `'\''` (shell-words 1.1 `quote`, `EscapeStyle::Mixed`). So a single quote in a script puts backslashes into the command line the login shell parses. | **Scripts contain neither a backslash nor a single quote.** The empty prompt is written `-p ""`. Then every quoted piece is a plain `'…'`, and the whole `remote_command` output is backslash-free. That is tested (§3.6). |
| **E-6** | **Major** | T3 | D309: `prompter: &dyn PasswordPrompt` and `verifier: &dyn Verifier` with async methods. D298: `trait Remote { async fn run(…) }`. | `async fn` in a trait is not dyn-compatible. rustc 1.98 also warns `async_fn_in_trait` on a pub trait, which fails `-D warnings`. | `Verifier` and `PasswordPrompt` return `futures::future::BoxFuture<'_, …>`; `futures` is already an htui dependency, and `htui_orch::Verifier` sets the precedent. `Remote` keeps `async fn` under `#[allow(async_fn_in_trait, reason = "no dyn Remote is formed: run_with is generic")]` (precedent `concepts.rs:210`). It is used only as `R: Remote`. |
| **E-7** | **Major** | T3 | Exit 4 means "sudo refused". `sudo -n` failing is a "safe failure". | `sudo -n`'s own refusal exits **1**: a per-command NOPASSWD rule that does not cover `sh`, a missed timestamp, `requiretty`. Under `set -e`, INSTALL_ROOT's own failures exit with the failing command's status, often 1. The two cannot be told apart. | INSTALL_ROOT's first output line is `htui.root=start`. Locally: exit 4, **or** non-zero with no `htui.root=start` in stdout, is the D301 sudo sentence ("nothing privileged was written"). Non-zero **with** the marker is "installing the service on <dest> failed (exit N): …". |
| E-8 | Minor | T2 | INSTALL gets `root user group home replace mode`. | D303's amendment dropped `install -o/-g`, and the guide's unit has no `Group=`, so `group` has no use. | INSTALL's arguments are `root user home replace mode installer` (§3.3). Preflight still reports `group` and D296 still validates it: D299 and D296 are unchanged. |
| E-9 | Minor | T2 | D299: `sudo` is `nopasswd` or `password`. | With no sudo installed, preflight reports `password`. PREPARE writes, then INSTALL fails with exit 4. The PRD's "Clear refusal" metric names "no sudo" as a refusal **before any remote write**. | Preflight reports `sudo=none` when `command -v sudo` fails. `Facts.sudo` is `Option<SudoMode>`, and `decide` refuses `None` in its step 1 (§3.5). |
| E-10 | Minor | T2, T4 | D305: VERIFY "loops every 2 s for up to 60 s". | A literal 60 s loop makes T4's timeout scenario take 60 s. | VERIFY takes `tries` and `pause` as positional values: 30 and 2 in production, from `Ctx.timings`. The root prefix stays the only test hook in the script *text*. |
| E-11 | Minor | T3 | D305: VERIFY succeeds when `is-active` is `active` and `box.toml` exists. | `worker_cmd::connect` calls `load_or_mint` **before** `connect_headless`, and `Type=exec` reports `active` at exec. So a worker that exits 2 and restarts every 10 s (`Restart=on-failure`) still passes VERIFY. Only `box_seen` proves the worker connected. | Kept as planned: a `box_seen` timeout is a warning and exit 0 (D305). Its sentence says the worker has not checked in and names `journalctl -u htui-worker` and `~/.local/state/htui/worker.log`. VERIFY's exit-5 output also includes the user-owned `worker.log` tail, because `journalctl` may be denied to an unprivileged user. **For the maintainer**: making "baseline present, box never seen" a failure (exit 1) would be a D305 change. It is not made here. |
| E-12 | Minor | T3 | D309: `run(args, log)`. | D307 moved the dispatch after `init_tracing`, so `log` is unused. | `pub async fn run(args: ProvisionArgs) -> Result<(), ProvisionExit>`. |
| E-13 | Minor | T3 | OQ-1 (a): `--dsn-stdin` into `StartOptions.dsn`, "wrapped where read"; conflicts with `--demo`/`--offline`/`--set-dsn`/`--clear-dsn`. | `StartOptions.dsn` is `Option<String>`, and `StartOptions` **derives `Debug`**, which would print it (`connect.rs:200-213`). `start` wraps the String in `Zeroizing` at once (`:328-330`). `reconnect_over` keeps its own `Zeroizing` copy for every reconnect (`:470-483`). **No reconnect re-reads the keyring.** `--index-items`/`--search-items` read the keyring themselves (`concepts.rs:92`) and would ignore the flag. sqlx logs an unrecognised parameter's value at `warn` into `--log`. | `lib.rs::session_dsn`: `read_dsn_line`, then `Dsn::parse` (the scan, before sqlx sees the text), then `std::mem::take(&mut *line)`. That moves the allocation without a copy, and `start` re-wraps it. `StartOptions` is never formatted. The flag also conflicts with `--index-items` and `--search-items` (§4.6, §4.7). |
| E-14 | Minor | T1, T3 | D314: htui-store changes only `identity.rs` and `dsn.rs`. | `htui-store/src/lib.rs:35` re-exports `Dsn, DsnError` but not the new enum. | No re-export, because that would edit `lib.rs`. Callers write `htui_store::dsn::DsnHost`; `pub mod dsn` is at `lib.rs:17`. |
| E-15 | Minor | T5 | OQ-1 (a): a TUI over `ssh -t` with `--dsn-stdin`. | Settings › Connection reads the **keyring** for its DSN row (`connection.rs:158`). On such a session it shows "not stored" or "unreadable" while online, and its `SetDsn` would try the absent keyring. | Not fixed in MOD-45: the fix would touch the TUI beyond OQ-1's footprint. The guide says so in one sentence (§6). Follow-up candidate at close-out. |
| E-16 | Minor (security) | T5 | D301/D303: the DSN reaches `systemd-creds` on `sudo`'s stdin. | sudoers `log_input` (off by default) makes sudo record the command's stdin in its I/O log (`/var/log/sudo-io`), so the DSN would sit there. An unprivileged preflight cannot see this in password mode. | The guide states it: "do not provision through a sudo that has `log_input` set". The live check confirms the host's default. A FIFO hand-off that bypasses sudo's stdin would be a D301/D303 change, so it is deferred. |
| E-17 | Minor | T4 | D309: the `id` stub "returns the real test user and group, so the real `install` and `mv` work without root". | Since D303 there is no `install -o/-g`, so names are irrelevant. A real CI user name may also fail D296's `^[a-z_][a-z0-9_-]{0,31}$` (here it is `mluigi`). `/tmp` may be `noexec`, and the stub `htui` must execute. | The `id` stub answers a canned `provtest` (overridable per scenario). Every T4 directory is created under `env!("CARGO_TARGET_TMPDIR")` (`<repo>/target/tmp`), which must also pass D296's home charset. |
| E-18 | Minor | T1 | Gate `--lib -- identity:: dsn::`. | `dsn.rs` has no `#[cfg(test)]` module today (its cases live in `tests/dsn.rs`), so `dsn::` would match 0 tests and report ok. | T1 adds `#[cfg(test)] mod tests` to `dsn.rs`. The gate must show a non-zero count for both filters. |
| E-19 | Minor | T3 | Gate `cargo test -p htui --all-features provision`. | The filter misses the TUI `--dsn-stdin` tests, the `cli::` guards and `main.rs`. It also compiles every integration binary. | Explicit lines in §4.11. |
| E-20 | Minor | T3 | D293: the payload is `std::env::current_exe()`. | On Linux, `current_exe` is the `/proc/self/exe` link's text. After a `cargo build` while the binary runs, that text ends in ` (deleted)` and the read fails. | `Payload::this_binary()` reads `/proc/self/exe` directly. The open works on a deleted file and gives the same bytes. The arch is still `std::env::consts::ARCH`. |
| E-21 | Minor | T3 | D304: exit 3 shows the **first** stderr line. | ssh warnings and login-script output come **before** the loader's error. The loader's `GLIBC_… not found` is one line, and it is the last. | Exit 3 shows the **last non-empty** stderr line. |
| E-22 | Minor | T3, T5 | "A re-run prints 'already provisioned'". | `AlreadyProvisioned` stops after the preflight, so no box id is known. With password ssh auth, every run asks up to four times, once per session. | Stdout: `<dest> is already provisioned with this build; nothing was changed`. The PRD's "same box id" is checked by T4 S4 (`box.toml` byte-identical). The guide recommends keys or `ControlMaster` (§6). |
| E-23 | Minor | T3 | D313: "`Stale` is retried once after a re-read". | `CasOutcome::Stale(row)` already carries the current row (`traits.rs:2138`). A re-run on a box already set to `worker` would write again and bump `edit_version` for nothing. | Retry once with `row.edit_version` from the `Stale` row, with no extra read. Skip the write when `Executor::of(&row.settings) == Executor::Worker`. |
| E-24 | Minor | live check | Plan "Live check". | Not modelled by stubs: sudo's `use_pty`, and how sudo-rs passes stdin through; a host whose `/bin/sh` is bash (E-4); `log_input` (E-16). | Add to the maintainer's live check: one host where `/bin/sh` is bash (Fedora/RHEL); `sudo -l` / `/etc/sudoers` shows no `log_input`; the remote `ps` during INSTALL shows the scripts and no secret. |

---

## 1. Build order and validation, at a glance

| Task | Wave | Files (complete) | Commits | Gate (all `--all-features -- --test-threads=1`) |
|---|---|---|---|---|
| T1 store helpers | A | `crates/htui-store/src/identity.rs`, `crates/htui-store/src/dsn.rs` | 2 (§2.5) | fmt; `-p htui-store --lib -- identity:: dsn::`; clippy `-p htui-store` |
| T2 skeleton + scripts + parse + decide | A | `crates/htui/src/lib.rs` (one line), `crates/htui/src/provision/{mod,script,preflight,plan}.rs` | 4 (§3.7) | fmt; `-p htui --lib provision::`; clippy `-p htui` |
| T5 guide | A | `docs/htui-worker.md`, `README.md` | 1 (§6) | `validate-workflow-docs.sh`; links |
| T3 command, transport, prompt, verifier, CLI | after A | `provision/{mod,remote,secret_prompt,verify}.rs`, `cli.rs`, `lib.rs`, `main.rs`, `crates/htui/Cargo.toml`, `Cargo.lock` | 5 (§4.11) | fmt; `--lib -- provision:: cli:: tests::session_dsn`; `--bin htui`; clippy; `cargo tree` |
| T4 end to end | after T3 | `crates/htui/tests/provision.rs`, `crates/htui/tests/provision_pg.rs` (+ `script.rs` only to fix a script, §5.8) | 2 (§5.8) | fmt; `--test provision --test provision_pg`; then §7 |
| end | — | — | — | §7 workspace gate |

---

## 2. T1: store helpers (D305, D306; E-1, E-2, E-14, E-18)

**Files (complete)**: `crates/htui-store/src/identity.rs`, `crates/htui-store/src/dsn.rs`.

### 2.1 `identity.rs`: `parse_box_toml`

```rust
/// `box.toml`'s text as an [`Identity`], with the hostname the file records (MOD-45 D305).
///
/// `htui provision` reads a remote box's file over ssh and has no path to name, so the error is
/// path-free. [`load_or_mint`] shares the parser and keeps its own sentence, which names the path.
///
/// # Errors
///
/// [`StoreError::Backend`]: `not valid box.toml: <the parser's reason>`.
pub fn parse_box_toml(text: &str) -> Result<Identity>;

/// The one parser both readers share. Private: `BoxToml` stays private (`:36-39`).
fn parse_toml(text: &str) -> core::result::Result<BoxToml, toml::de::Error>;
```

- `parse_box_toml` maps `parse_toml`'s error to `StoreError::Backend(format!("not valid box.toml:
  {e}"))` and returns `Identity { box_id: BoxId::from_uuid(parsed.box_id), hostname:
  parsed.hostname }`. The hostname comes **from the file**, not from `gethostname`.
- `load_or_mint` (`:74-77`) calls `parse_toml(&text)` and keeps its error sentence byte for byte:
  `format!("{} is not valid box.toml: {e}", path.display())`. Nothing else in it changes.

### 2.2 `dsn.rs`: `DsnHost`, `Dsn::host_class`

```rust
/// Where a DSN's server is, as another machine would read the DSN (MOD-45 D306).
///
/// Carries no text. `htui provision` refuses `Loopback` and `Socket`: the remote host would reach
/// itself, not this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsnHost {
    /// A host name or an address another machine can mean the same server by.
    Remote,
    /// `localhost` (any case, trailing dot, `*.localhost`), `127.0.0.0/8`, `::1`, an IPv4-mapped
    /// loopback, or the unspecified `0.0.0.0` / `::`, which also reach this machine.
    Loopback,
    /// A Unix-socket directory (`?host=/…` or a percent-encoded `/` host).
    Socket,
}

impl Dsn {
    /// The class of the host sqlx would dial (E-2): the URL authority's host, unless a later
    /// `host=` or `hostaddr=` query parameter replaces it, and `Socket` once any `host=/…` set a
    /// socket.
    ///
    /// Re-parses with `PgConnectOptions::from_str`, as [`Dsn::summary`] does. That is safe because
    /// `scan` already refused every parameter sqlx would log. It is never an environment default
    /// (`PGHOST`, a socket probe): `scan` refused a DSN without an authority host
    /// ([`DsnError::NoHost`]) before this value existed. The unreachable parse failure answers
    /// `Loopback`, the refusing direction.
    #[must_use]
    pub fn host_class(&self) -> DsnHost;
}

/// `host` as sqlx holds it: brackets stripped, then the name and address rules of [`DsnHost`].
fn classify_host(host: &str) -> DsnHost;
```

`classify_host`:
1. Strip one `[`…`]` pair, then a single trailing `.`.
2. If the result is `localhost` or ends in `.localhost` (ASCII, case-insensitive) → `Loopback`.
3. If it parses as `std::net::IpAddr`:
   - V4 `is_loopback() || is_unspecified()` → `Loopback`.
   - V6 `is_loopback() || is_unspecified() || to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())`
     → `Loopback`.
4. Everything else → `Remote`.

### 2.3 T1 tests (red first)

`identity.rs` `mod tests`:
- `parse_box_toml_reads_what_store_writes`: `store(root, &Identity { box_id: BoxId::new(), hostname:
  "remote-host".into() })`, then `read_to_string`, then `parse_box_toml`. The result equals the
  input, hostname included, and the hostname is not this machine's.
- `parse_box_toml_refuses_malformed_text_without_a_path`: `parse_box_toml("box_id = 7\n")` is `Err`.
  Its text contains `not valid box.toml` and contains no `/`.
- `load_or_mint_still_names_the_path`: a malformed `box.toml` under a temp root. The error contains
  the root's path and `is not valid box.toml`.

`dsn.rs` new `#[cfg(test)] mod tests` (E-18). One table, `Dsn::parse(text).map(|d| d.host_class())`:

| DSN | Expected |
|---|---|
| `postgres://u:p@localhost/db` | `Ok(Loopback)` |
| `postgres://u:p@LOCALHOST./db` | `Ok(Loopback)` |
| `postgres://u:p@127.0.0.5/db` | `Ok(Loopback)` |
| `postgres://u:p@[::1]:5432/db` | `Ok(Loopback)` |
| `postgres://u:p@0.0.0.0/db` | `Ok(Loopback)` |
| `postgres://u:p@localhost/db?host=/var/run/postgresql` | `Ok(Socket)` |
| `postgres://u:p@%2Fvar%2Frun%2Fpostgresql/db` | `Ok(Socket)` |
| `postgres://u:p@/db?host=/var/run/postgresql` | `Err(DsnError::NoHost)` (E-1) |
| `postgres:///db` | `Err(DsnError::NoHost)` |
| `postgres://u:p@db.example/db` | `Ok(Remote)` |
| `postgres://u:p@10.0.0.3/db` | `Ok(Remote)` |
| `postgres://u:p@[fd00::3]/db` | `Ok(Remote)` |
| `postgres://u:p@db.example/db?hostaddr=127.0.0.1` | `Ok(Loopback)` |
| `postgres://u:p@db.example/db?host=localhost` | `Ok(Loopback)` (E-2) |
| `postgres://u:p@localhost/db?host=db.example` | `Ok(Remote)` (E-2) |

Also:
- `nothing_prints_the_password`: for `postgres://u:SENTINEL-DSN-PW@localhost/db`, neither
  `format!("{:?}", dsn)` nor `format!("{:?}", dsn.host_class())` contains `SENTINEL-DSN-PW`. For
  `…SENTINEL-DSN-PW@/db`, neither the error's `Display` nor its `Debug` does.
- `classify_host_strips_brackets_and_the_trailing_dot`: a direct table over `classify_host`.

### 2.4 T1 gate

```bash
cargo fmt --all -- --check
cargo test -p htui-store --all-features --lib -- identity:: dsn:: --test-threads=1   # both filters > 0 tests
cargo clippy -p htui-store --all-targets --all-features -- -D warnings
```

### 2.5 T1 commits

1. `test(mod-45): T1 red — parse_box_toml and Dsn::host_class`. Contains the tests,
   `parse_box_toml` and `host_class` with `todo!()` bodies, and `DsnHost` complete.
2. `feat(mod-45): T1 — parse_box_toml, DsnHost and Dsn::host_class`.

---

## 3. T2: skeleton, scripts, preflight parse, decide (D296, D297, D299–D301, D303, D304; E-3…E-5, E-8…E-10)

**Files (complete)**: `crates/htui/src/lib.rs` (`pub mod provision;` between `prompt_settings` and
`qdrant_settings_info`), and `crates/htui/src/provision/{mod,script,preflight,plan}.rs` (new).

### 3.1 `provision/mod.rs` skeleton and module doc

T2 writes the module doc in full, with `pub mod plan; pub mod preflight; pub mod script;`. T3 adds
the rest. The doc's key sentences, which T3 keeps:

```rust
//! `htui provision <destination>` (MOD-45): installs this htui build as the `htui-worker` system
//! service on a Linux host over the user's own `ssh`, with the DSN encrypted on that host by
//! `systemd-creds` and nowhere else (`R-STO-1`, PRD D1–D3).
//!
//! Four short sessions: the preflight (read-only), prepare (unprivileged: the log directory and the
//! binary), install (privileged: the credential, the unit, the start) and verify (wait for the
//! service and its `box.toml`). Then, from this machine, the box is checked in Postgres against a
//! baseline taken before the install, and its executor is set to `worker` (D313).
//!
//! Every remote script is a constant in [`script`]. Values reach a script only as quoted
//! positional arguments. The DSN and the sudo password travel only on the ssh child's stdin:
//! never in an `argv`, the environment, a log or a file on either side.
//!
//! A refusal (exit 2) is anything decided before the first remote write. A failure (exit 1) is
//! anything after it. `remote::SshRemote` is the only place `htui` spawns `ssh`.
```

The last sentence uses a code span in T2, because `remote` does not exist yet. T3 turns it into an
intra-doc link.

### 3.2 The five scripts, exactly as committed (`provision/script.rs`)

Rules:
- POSIX `sh`; dash and bash both parse every script. **No backslash and no single quote** (E-3,
  E-5).
- Each starts with `set -eu`.
- `$0` is `htui-provision`, or `htui-provision-root` for INSTALL_ROOT. `$1` is the root prefix:
  `""` in production, a temp directory under test (D309).
- Rust raw strings, `r#"set -eu` with nothing before `set`. No script contains the sequence `"#`.

**Dash compatibility**:
- The constructs used are `${v#"…"}`, `${v%%[!0-9]*}`, `$((n + 1))`, `command -v`, `trap … EXIT`,
  functions, `if !`, and an unquoted heredoc. All are POSIX and supported by dash, bash, busybox
  ash and ksh.
- `read` from a pipe consumes one byte at a time in dash and bash, so `IFS= read -r pw` takes line 1
  only. This was observed: the rest of the pipe reached the next reader.
- `printf` is a builtin everywhere it matters, so `$pw` is never in an `argv`.

```rust
/// Read-only facts about the host (D299): `htui.<key>=<value>` lines, nothing else written.
/// `$1` root. Never fails on a missing tool: a missing tool is a value (`none`, `no`, `unknown`).
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
"#;

/// Unprivileged (D304): the log directory, then, with `send=1`, the binary on stdin, which must
/// run `--version` before it replaces anything. `$1` root (unused), `$2` home, `$3` send (`0`|`1`).
/// Exit 3: the uploaded binary does not run here; the temporary file is removed.
pub const PREPARE: &str = r#"set -eu
home=$2
send=$3
bin="$home/.local/bin"
tmp="$bin/.htui.provision.$$"
cleanup() {
  rm -f -- "$tmp"
}
trap cleanup EXIT
mkdir -p "$home/.local/state/htui" "$bin"
if [ "$send" = 1 ]; then
  cat > "$tmp"
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
```

Notes the implementer must not "fix":
- **How INSTALL_ROOT reaches `sudo -n sh -c`**:
  - It is INSTALL's sixth positional argument (`$6`, held in `installer`).
  - It is quoted once by `remote_command`, and the login shell un-quotes it. INSTALL passes it
    unchanged as `sh -c "$installer"`.
  - Its own positionals follow `htui-provision-root`.
  - The text is visible in the remote `ps` and in sudo's command log. It holds no secret.
- **The unit heredoc**:
  - The delimiter `<<EOF` is unquoted, so `$user` and `$home` expand. Both passed D296, so they are
    safe unquoted in a unit file.
  - The body and the closing `EOF` are at column 0, so there is no `<<-` and no tabs.
  - The heredoc feeds `cat`'s stdin, never the outer stdin, which holds the DSN.
  - `LoadCredentialEncrypted=` names the **production** path, never `$root`. The root prefix only
    moves where the file is written.
- **`rc=0` / `|| rc=$?` / `exit "$rc"`** is E-4's guard. Without it, bash execs `sudo -n`.
- **`|| true`**:
  - It follows `systemctl --version` and every `is-active`, because `is-active` exits 3 when the
    unit is not active (V-11).
  - It follows the journal and log tails.
  - It follows `sha256sum`: a missing tool is the value `unknown`, never an aborted preflight.
- **An `EXIT` trap keeps the exit status** in dash and bash (observed: `exit 3` with a trap still
  exits 3). `cleanup` is a function, so the trap needs no quoting of `$tmp`.
- **Arguments that are not dollar-expanded**:
  - VERIFY's `$1` and PREPARE's `$1` are never read. The root stays first by rule.
  - VERIFY's `tries` and `pause` are integers from `Ctx.timings`.

### 3.3 `script.rs` API

```rust
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

/// `sh -c '<script>' htui-provision '<arg>'…`, every piece through `shell_words::quote`. The
/// remote login shell parses it once (D297, V-7).
#[must_use]
pub fn remote_command(script: &str, args: &[&str]) -> String;
```

The argument lists, which T3 builds:

| Script | Arguments after `$0` | Stdin |
|---|---|---|
| PREFLIGHT | `root` | ∅ |
| PREPARE | `root home send` (`1` iff `upload`) | the payload iff `send=1`, else ∅ |
| INSTALL | `root user home replace mode INSTALL_ROOT` | `pw\nDSN\n` (password) / `DSN\n` (nopasswd) |
| VERIFY | `root home tries pause` | ∅ |

### 3.4 `preflight.rs`

```rust
//! The preflight's answer (MOD-45 D299): `htui.<key>=<value>` lines. Every other line (a login
//! script's greeting, `~/.bashrc` noise, V-7) is ignored, and so is an `htui.` key this build
//! does not know.

/// The eleven keys, in the order [`crate::provision::script::PREFLIGHT`] prints them.
pub const KEYS: [&str; 11] =
    ["os", "arch", "systemd", "creds", "user", "group", "home", "bin_sha", "unit", "active", "sudo"];

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

impl Facts {
    /// Parses `stdout`; lines may end in `\r`; the value is everything after the first `=`.
    ///
    /// # Errors
    ///
    /// [`FactsError`], first by line order for duplicates, then by [`KEYS`] order for absences.
    pub fn parse(stdout: &str) -> Result<Self, FactsError>;
}
```

`FactsError` implements `Display` by hand (htui has no `thiserror`):
- `NoPreflight`: "the remote login shell did not run the preflight; htui provision needs a POSIX
  login shell (sh, bash, zsh, ksh)". This is D297's sentence.
- `Missing(k)`: "the preflight did not report `{k}`".
- `Duplicate(k)`: "the preflight reported `{k}` twice".
- `Unreadable(k)`: "the preflight reported an unreadable `{k}`".

Value rules:
- `systemd` is `none` or ASCII digits.
- `creds` and `unit` are `yes` or `no`.
- `sudo` is `nopasswd`, `password` or `none`.
- Every other value is taken as is, including empty. D296 judges `user`, `group` and `home`.

### 3.5 `plan.rs`

```rust
//! What to do on a host, decided from its [`Facts`] alone (MOD-45 D300), and the value checks
//! that make the remote values safe in a unit file (D296). Pure: no I/O.

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
pub fn decide(facts: &Facts, local: &LocalFacts, replace_credential: bool) -> Decision;

/// `arm64` → `aarch64`; anything else unchanged (D293).
#[must_use]
pub fn normalise_arch(arch: &str) -> &str;

/// D296: non-empty, no leading `-`, no whitespace or control character.
///
/// # Errors
///
/// The refusal sentence (it never repeats the destination).
pub fn validate_destination(destination: &str) -> Result<(), String>;

/// D296: `^[a-z_][a-z0-9_-]{0,31}$`; `field` is `user` or `group`.
///
/// # Errors
///
/// The refusal sentence, naming `field`.
pub fn validate_account(field: &'static str, name: &str) -> Result<(), String>;

/// D296: absolute, bytes in `[A-Za-z0-9_./-]`, no `..`.
///
/// # Errors
///
/// The refusal sentence.
pub fn validate_home(home: &str) -> Result<(), String>;
```

`decide`'s sentences, exact. `run_with` prefixes `not provisioning <dest>: `.

| Step | Condition | Sentence |
|---|---|---|
| 1a | `os != "Linux"` | `the remote host runs {os}, not Linux` |
| 1b | local arch ∉ {`x86_64`, `aarch64`} | `this htui build is {local}; provisioning supports x86_64 and aarch64` |
| 1c | `normalise_arch(remote) != local` | `the remote host is {remote} and this htui build is {local}; provisioning ships this binary, so the two must match` |
| 1d | `systemd == None` | `the remote host has no systemd` |
| 1e | `systemd < 250` | `the remote host has systemd {v}; provisioning needs 250 or later for LoadCredentialEncrypted=` |
| 1f | `!creds` | `the remote host has no systemd-creds on its PATH` |
| 1g | `sudo == None` (E-9) | `the remote host has no sudo; provisioning installs a system service and needs it` |
| 2 | `unit && bin_sha == None` | `already provisioned with a different build, or set up by hand; upgrading is not supported yet` |
| 2 | `unit && bin_sha != Some(local.sha)` | `already provisioned with a different build; upgrading is not supported yet` |
| 3 | `unit && same sha && active == "active" && !replace` | → `AlreadyProvisioned` |
| 4 | otherwise | → `Steps { upload: bin_sha != Some(local.sha), sudo }` |

Validation sentences:
- `validate_destination`: `the ssh destination must not be empty, start with "-", or contain
  whitespace or control characters`.
- `validate_account(field, name)`: `the remote {field} {name:?} is not a name htui provision writes
  into a unit file (it must match [a-z_][a-z0-9_-]{0,31})`.
- `validate_home(home)`: `the remote home {home:?} is not a path htui provision writes into a unit
  file (absolute, only A-Z a-z 0-9 _ . / -, no "..")`.

### 3.6 T2 tests (red first)

`script.rs`:
- `every_script_starts_with_set_eu`: each of the five `starts_with("set -eu\n")`.
- `no_script_holds_a_backslash_or_a_single_quote` (E-3, E-5).
- `every_script_parses_under_sh_n`: `std::process::Command::new("sh").args(["-n", "-c",
  script])` succeeds for each script; here `sh` is dash. The same runs under `bash -n` when `bash`
  is on `PATH`, and is skipped with a printed note otherwise.
- `remote_command_round_trips_through_shell_words`: `remote_command(INSTALL, &["", "alice",
  "/home/alice", "0", "password", INSTALL_ROOT])` passed through `shell_words::split` gives exactly
  `["sh", "-c", INSTALL, "htui-provision", "", "alice", "/home/alice", "0", "password",
  INSTALL_ROOT]`. One more case: PREFLIGHT with `""`.
- `remote_command_is_backslash_free`: that output, and every script's command, contain no `\`.
- `install_mentions_pw_only_on_its_three_lines`: the INSTALL lines matching the word `pw` are exactly
  `IFS= read -r pw || exit 4`, the `printf "%s%s" "$pw" "$nl" | …` line and `unset pw`.
- `install_reads_the_password_before_any_sudo`: line order is `read -r pw` < `sudo -k` <
  `sudo -S -p "" -v` < `sudo -n`.
- `install_never_execs_sudo_n` (E-4):
  - the `sudo -n` line ends with `|| rc=$?`;
  - the next line is `exit "$rc"`;
  - it is the last line.
- `install_root_writes_the_credential_then_the_unit_then_starts`: line order is `install -d -m
  0700` < `systemd-creds encrypt` < `cat > "$tmp" <<EOF` < `mv -f -- "$tmp" "$unit"` <
  `daemon-reload` < `enable --now` < `restart`. Also, `echo htui.root=start` comes before every
  write.
- `the_unit_is_the_guides_sample` (D295):
  1. Take the heredoc body of INSTALL_ROOT.
  2. Replace `$user` → `you` and `$home` → `/home/you`.
  3. Assert it equals the guide's sample (`docs/htui-worker.md:275-289`), with the binary at
     `/home/you/.local/bin/htui`. The expected text is a test constant.
- `no_privileged_line_touches_home`: outside the heredoc, no INSTALL_ROOT line mentions `$home` (D303:
  the log directory is PREPARE's).
- `verify_waits_inside_one_session`: VERIFY has a `while` loop, `sleep "$pause"` and `exit 5`, and
  never calls `sudo`.

`preflight.rs`:
- A complete answer parses: every field, `systemd=255` → `Some(255)`, `bin_sha=none` → `None`,
  `sudo=password` → `Some(Password)`.
- Noise is ignored: `Welcome to host`, `htui-motd: hi`, blank lines and CRLF endings.
- An unknown `htui.extra=1` is ignored.
- Each key missing in turn gives `Missing(key)`.
- `htui.home=` twice gives `Duplicate("home")`.
- No `htui.` line gives `NoPreflight`.
- `systemd=25x`, `unit=maybe` and `sudo=sometimes` give `Unreadable`.
- `sudo=none` gives `None`.

`plan.rs`:
- `decide_follows_d300_in_order`: one row per line of §3.5's table, plus:
  - `arm64` versus a local `aarch64` → steps;
  - `systemd` 250 passes and 249 refuses;
  - a row failing 1a and 1c reports 1a;
  - `active=failed` with this build → `Steps { upload: false }`;
  - `--replace-credential` on an already-provisioned host → `Steps { upload: false }`;
  - `unit=no` with `bin_sha=unknown` → `Steps { upload: true }`.
- `validators_accept_and_refuse`:
  - accounts: `alice`, `_svc` and `a-b_9` pass. A 32-character name passes and a 33-character one
    is refused. `Alice`, `9lives`, `""`, `a b`, `a.b` and `a$b` are refused.
  - homes: `/home/alice`, `/var/lib/w-1` and `/` pass. `home`, `""`, `/home/a b`, `/home/../etc`,
    `/home/a..b` and `/home/al%ice` are refused.
  - destinations: `host`, `u@host`, `alias` and `[fd00::3]` pass. `""`, `-oProxyCommand=x`, `a b`,
    `a\tb` and `host\n` are refused.

### 3.7 T2 gate and commits

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib provision:: -- --test-threads=1
cargo clippy -p htui --all-targets --all-features -- -D warnings
```

1. `feat(mod-45): T2 skeleton — the provision module`: `mod.rs`, `lib.rs`, and the three files with
   their docs and empty bodies (`todo!()`). It compiles with no tests.
2. `test(mod-45): T2 red — scripts, preflight parse, decide, validators`. The script constants are
   `""` here, so the script tests fail.
3. `feat(mod-45): T2 — the five scripts and remote_command`.
4. `feat(mod-45): T2 — Facts::parse, decide and the validators`.

---

## 4. T3: command, transport, prompt, verifier, CLI (D291–D295, D298, D302, D305, D307, D309, D313; OQ-1 (a); E-6, E-7, E-11…E-13, E-19…E-23)

**Files (complete)**:
- `crates/htui/src/provision/mod.rs` (extended).
- `crates/htui/src/provision/{remote,secret_prompt,verify}.rs` (new).
- `crates/htui/src/cli.rs`, `crates/htui/src/lib.rs`, `crates/htui/src/main.rs`.
- `crates/htui/Cargo.toml`, `Cargo.lock`.

### 4.1 Manifest

`crates/htui/Cargo.toml` `[dependencies]`, after `shell-words`:

```toml
# MOD-45 D293: the payload's hash, compared with the remote binary's. 0.10.9 is already in htui's
# graph (via htui-store), so nothing new is compiled.
sha2                   = { workspace = true }
```

Afterwards, `cargo tree -p htui -i sha2@0.10.9` shows htui as a direct dependent. `git diff
Cargo.lock` is one added line in `htui`'s `dependencies`.

### 4.2 `provision/mod.rs`: the public surface

```rust
pub mod plan;
pub mod preflight;
pub mod remote;
pub mod script;
pub mod secret_prompt;
pub mod verify;

/// How `htui provision` ends (D292): `main` maps it to the exit code.
#[derive(Debug)]
pub enum ProvisionExit {
    /// Exit 2: refused before the first remote write (local checks, preflight, an aborted prompt).
    Refused(String),
    /// Exit 1: failed after it (an upload that does not run, sudo refused inside INSTALL, a
    /// service that never came up, ssh dropping mid-way).
    Failed(String),
}
impl ProvisionExit { #[must_use] pub const fn code(&self) -> u8 { /* 2 / 1 */ } }
impl core::fmt::Display for ProvisionExit { /* the sentence, and nothing else */ }
impl std::error::Error for ProvisionExit {}

/// The local build being shipped (D293). `Debug` prints its length, arch and hash, never the bytes.
#[derive(Clone)]
pub struct Payload { bytes: Vec<u8>, arch: String, sha: String }
impl Payload {
    /// `bytes` with their sha256 computed here (tests build a stub binary this way).
    #[must_use] pub fn new(bytes: Vec<u8>, arch: &str) -> Self;
    /// This process's own binary, `/proc/self/exe` (E-20), and `std::env::consts::ARCH`.
    ///
    /// # Errors
    ///
    /// The read's error.
    pub fn this_binary() -> std::io::Result<Self>;
    /// The bytes PREPARE streams.
    #[must_use] pub fn bytes(&self) -> &[u8];
    /// `std::env::consts::ARCH` at build time, or the test's value.
    #[must_use] pub fn arch(&self) -> &str;
    /// Lowercase hex sha256 of [`Payload::bytes`].
    #[must_use] pub fn sha(&self) -> &str;
}

/// The waits (D305; E-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timings {
    /// VERIFY's checks inside its one session.
    pub verify_tries: u32,
    /// Seconds between them.
    pub verify_pause_secs: u32,
    /// The local `box_seen` poll.
    pub poll: verify::Poll,
}
impl Timings {
    /// 30 checks 2 s apart; poll every 2 s for up to 60 s.
    pub const PRODUCTION: Self = /* … */;
}

/// Everything `run_with` needs (D309). `Debug` redacts the DSN and summarises the payload.
pub struct Ctx<'a, R> {
    /// The ssh destination, as typed.
    pub destination: &'a str,
    /// The root prefix every script gets as `$1`: `""` in production (D309).
    pub root: &'a str,
    /// `--replace-credential`.
    pub replace_credential: bool,
    /// The transport.
    pub remote: &'a R,
    /// The binary to ship.
    pub payload: Payload,
    /// The DSN to encrypt on the host; the only copy `run_with` holds.
    pub dsn: zeroize::Zeroizing<String>,
    /// Reads the sudo password with no echo (D302).
    pub prompter: &'a dyn secret_prompt::PasswordPrompt,
    /// Baseline, `box_seen`, `set_executor` (D305, D313).
    pub verifier: &'a dyn verify::Verifier,
    /// See [`Timings`].
    pub timings: Timings,
    /// The success line.
    pub out: &'a mut dyn std::io::Write,
    /// Progress, warnings, prompts, the hint.
    pub err: &'a mut dyn std::io::Write,
}

/// What a successful run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// D300 step 3: nothing was written.
    AlreadyProvisioned,
    /// Installed and started.
    Provisioned {
        /// From the remote `box.toml`.
        box_id: htui_core::model::BoxId,
        /// From the remote `box.toml`.
        hostname: String,
        /// `box_seen` answered within the poll.
        verified: bool,
        /// `set_executor` wrote (or found) `worker`.
        executor_set: bool,
    },
}

/// `htui provision`: builds the production [`Ctx`] and runs [`run_with`] (D309; E-12).
///
/// # Errors
///
/// [`ProvisionExit`].
pub async fn run(args: crate::cli::ProvisionArgs) -> Result<(), ProvisionExit>;

/// The whole provisioning, over injected parts (D309).
///
/// # Errors
///
/// [`ProvisionExit`]: `Refused` before the first remote write, `Failed` after it.
pub async fn run_with<R: remote::Remote>(ctx: Ctx<'_, R>) -> Result<Outcome, ProvisionExit>;
```

**`run`** (production wiring, not unit-tested):
1. `if !cfg!(target_os = "linux")` → `Refused("not provisioning {dest}: provisioning ships this htui
   binary, which is not a Linux build")` (D293).
2. `validate_destination`, so a bad destination is refused before stdin or the keyring is touched.
3. The DSN:
   - With `--dsn-stdin`: when stdin is a terminal, print `paste the DSN {dest} should use and press
     Enter (it will be visible):` to stderr. Then `secret::read_dsn_line(&mut stdin().lock())`.
     `None` → `Refused("… no DSN on stdin; nothing was sent")`.
   - Otherwise `secret::get_dsn()`, wrapped in `Zeroizing` **at once** (`secret.rs:113`):
     - `Ok(None)` → `Refused("… no Postgres DSN in the OS keyring; run `htui --set-dsn` or pass
       --dsn-stdin")`.
     - `Err(e)` → `Refused("… the OS keyring could not be read ({e}); pass the DSN with
       --dsn-stdin")`.
4. `Payload::this_binary()`. `Err` → `Refused("… this htui binary could not be read: {e}")`.
5. `identity::config_root()` is the **local** root for the baseline. `Err` → `Refused`.
6. Build `SshRemote::new(dest)`, `TtyPrompt`, `PgVerifier::new(root)`, `Timings::PRODUCTION`,
   `root: ""`, `stdout`/`stderr` locked, then run `run_with(…)`. `Ok(_)` → `Ok(())`.

**`run_with`, step by step.** Every refusal reads `not provisioning {dest}: {sentence}` and every
failure reads `provisioning {dest} failed: {sentence}`. Progress lines go to `err` as
`provisioning {dest}: …` (D307). Writes to `out`/`err` ignore their own errors.

| # | Step | On error |
|---|---|---|
| 1 | `plan::validate_destination(dest)` | Refused |
| 2 | `htui_store::Dsn::parse(&dsn)`: `NoHost` → `the DSN names no host; pass one {dest} can reach with --dsn-stdin`; any other `DsnError` → `the DSN cannot be used: {e}`. Then `host_class()`: `Loopback` → `the DSN's host is loopback, which would mean {dest} itself; pass the address {dest} reaches Postgres at with --dsn-stdin`; `Socket` → `the DSN names a Unix socket, which would be a socket on {dest}; pass a TCP address with --dsn-stdin`. **Steps 1–2 make no `Remote` call.** | Refused |
| 3 | `err`: `… preflight`. `remote.run(&remote_command(PREFLIGHT, &[root]), b"")` | `io::Error` → Refused `cannot run ssh: {e}` |
| 4 | `Facts::parse(stdout)`. `NoPreflight` with `code != Some(0)` → `ssh to {dest} failed (exit {code}): {last stderr line}`. `NoPreflight` with exit 0 → the D297 sentence. Other `FactsError`s → their sentence, plus `: {last stderr line}` when stderr is not empty. | Refused |
| 5 | `validate_account("user")`, `validate_account("group")`, `validate_home` | Refused |
| 6 | `decide(&facts, &LocalFacts { arch: payload.arch, sha: payload.sha }, replace)`. `Refuse(s)` → Refused. `AlreadyProvisioned` → `out`: `{dest} is already provisioned with this build; nothing was changed`, `err`: `(--replace-credential re-encrypts the DSN and restarts the service)`, then return `Ok(AlreadyProvisioned)` (E-22). | Refused |
| 7 | With `SudoMode::Password`: `err`: `[sudo] password for {user} on {dest}: `, then `prompter.ask().await`, then `err`: newline. `Aborted` → `the sudo password prompt was cancelled; nothing was written`. `NoTerminal` → D302's sentence: `sudo needs a password and there is no terminal; configure NOPASSWD or run interactively`. | Refused |
| 8 | `verifier.baseline(&dsn).await`: `Ok(b)` → `Some(b)`. `Err(r)` → `None`; keep `r` for step 12. | never fails |
| 9 | With `upload`, `err`: `… uploading htui ({n} MiB)` (rounded up). `remote.run(&remote_command(PREPARE, &[root, home, send]), payload-or-∅)`. **From here every exit is `Failed`.** Exit 3 → `the htui binary does not run on {dest}: {last non-empty stderr line}` (D304, E-21). Other non-zero → `preparing {dest} failed (exit {code}): {stderr tail, 5 lines}`. | Failed |
| 10 | `err`: `… installing the service`. stdin = `install_stdin(password.as_deref(), &dsn)`. Drop the password **right after this call**. Exit 4, **or** non-zero with no `htui.root=start` line in stdout → `sudo refused the password (or needs a tty or a second factor) on {dest}; nothing privileged was written` (D301, E-7). Other non-zero → `installing the service on {dest} failed (exit {code}): {stderr tail, 5 lines}`. | Failed |
| 11 | `err`: `… waiting for the worker`. `remote.run(&remote_command(VERIFY, &[root, home, tries, pause]), b"")`. Exit 0 → the text between `htui.box.begin` and `htui.box.end` → `htui_store::identity::parse_box_toml`; absent or invalid → `the worker on {dest} wrote a box.toml htui cannot read: {e}`. Exit 5 → `the htui-worker service on {dest} did not start with a box.toml within {tries × pause} s (systemctl says {state}); its last journal and log lines follow:\n{stderr}`, where `state` is the `htui.active=` value. Other non-zero → `waiting for the worker on {dest} failed (exit {code}): …`. | Failed |
| 12 | With `Some(b)`: `verifier.box_seen(id, &b, timings.poll)`. If `Ok`: `verifier.set_executor(id)`; `Ok` → `err`: `… executor set to worker`; `Err(w)` → `err`: `warning: box {id}'s executor is not set to worker: {w}; set it in Settings › Boxes` (D313). If `box_seen` is `Err(r)`, or the baseline was `None(r)`: `err`: `warning: service active, box {id}; not verified in Postgres from here: {r}` (D305; E-11 wording when it timed out: `box {id} did not check in within {deadline} s; see journalctl -u htui-worker and ~/.local/state/htui/worker.log on {dest}`), then `err`: `set box {id}'s executor to worker in Settings › Boxes` (D313). | warning, exit 0 |
| 13 | `out`: `box {id} ({hostname}) provisioned on {dest}`. `err` (OQ-1 (a)): `to log an agent in on {dest}: ssh -t {dest} ~/.local/bin/htui --dsn-stdin, paste the DSN, then Settings › Agents` | — |

Private helpers in `mod.rs`:
- `fn install_stdin(password: Option<&str>, dsn: &str) -> Zeroizing<Vec<u8>>`: capacity computed
  up front, so there is no reallocation residue.
- `fn box_text(stdout: &str) -> Option<&str>`.
- `fn read_dsn(dsn_stdin: bool, dest: &str) -> Result<Zeroizing<String>, ProvisionExit>`.
- `fn refused(dest, sentence)` and `fn failed(dest, sentence)`.

### 4.3 `provision/remote.rs`

```rust
//! The ssh seam (D298). [`SshRemote`] is the only `ssh` spawner in htui (PRD "SSH not reused").
//! The payload is written from a future joined with the two output drains, never before them: a
//! 100 MiB write into a full pipe whose reader is not draining would deadlock (V-6).

/// Each captured stream keeps its first 64 KiB; the rest is drained and dropped.
pub const OUTPUT_CAP: usize = 64 * 1024;

/// What one remote session left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteOutput {
    /// The exit code; `None` when a signal ended it (ssh's own failures are 255).
    pub code: Option<i32>,
    /// At most [`OUTPUT_CAP`] bytes.
    pub stdout: Vec<u8>,
    /// At most [`OUTPUT_CAP`] bytes.
    pub stderr: Vec<u8>,
}
impl RemoteOutput {
    /// Lossy UTF-8 of `stdout`.
    #[must_use] pub fn stdout_text(&self) -> std::borrow::Cow<'_, str>;
    /// The last non-empty stderr line, trimmed; `""` when there is none.
    #[must_use] pub fn last_stderr_line(&self) -> String;
    /// The last `n` non-empty stderr lines, joined by `\n`.
    #[must_use] pub fn stderr_tail(&self, n: usize) -> String;
}

/// One remote session: run `command` under the remote login shell, feed it `stdin`, then close it.
#[allow(async_fn_in_trait, reason = "no dyn Remote is formed: run_with is generic (E-6)")]
pub trait Remote {
    /// # Errors
    ///
    /// The spawn's, or a write's other than a broken pipe (a script that exits without reading).
    async fn run(&self, command: &str, stdin: &[u8]) -> std::io::Result<RemoteOutput>;
}

/// The user's own `ssh` (PRD D4): `~/.ssh/config`, `ProxyJump` and the agent apply; its prompts go
/// to `/dev/tty`.
#[derive(Debug, Clone)]
pub struct SshRemote { destination: String }
impl SshRemote { #[must_use] pub fn new(destination: &str) -> Self; }
impl Remote for SshRemote { /* Command::new("ssh").args(ssh_argv(..)) then run_piped */ }

/// `-T -o ConnectTimeout=15 -- <destination> <command>` (V-7: `--` stops ssh from reading options
/// after the destination). Pure.
#[must_use]
pub fn ssh_argv(destination: &str, command: &str) -> Vec<std::ffi::OsString>;

/// Spawns `command` with all three streams piped and `kill_on_drop(true)`, then joins
/// (`tokio::join!`, one task) the stdin write and close with both capped drains, then waits. A
/// `BrokenPipe` on the write is not an error. `pub` so the T4 `LocalShell` exercises the same code.
///
/// # Errors
///
/// The spawn's, the wait's, or a write's other than `BrokenPipe`.
pub async fn run_piped(command: tokio::process::Command, stdin: &[u8]) -> std::io::Result<RemoteOutput>;
```

The plan's "from its own task" is met by `tokio::join!` inside one task. The three futures make
progress concurrently, and nothing needs a `'static` copy of the payload.

### 4.4 `provision/secret_prompt.rs`

```rust
//! The sudo password, read with no echo (D302): crossterm raw mode, key events up to `Enter`, into
//! a pre-sized `Zeroizing<String>`. crossterm falls back to `/dev/tty` when stdin is not a terminal,
//! so `--dsn-stdin` and a password prompt work together. A guard's `Drop` restores the terminal.

/// Longer input is refused, so the buffer never reallocates (residue: MOD-41 R-5).
pub const PASSWORD_MAX: usize = 1024;

/// Why there is no password. Neither variant carries input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptError {
    /// `Esc`, `Ctrl-C`, or longer than [`PASSWORD_MAX`].
    Aborted,
    /// No `/dev/tty`: raw mode could not be entered.
    NoTerminal,
}

/// Reads one password (E-6: a boxed future, so `&dyn PasswordPrompt` is formable).
pub trait PasswordPrompt: core::fmt::Debug + Send + Sync {
    /// The caller has already printed the prompt.
    fn ask(&self) -> futures::future::BoxFuture<'_, Result<zeroize::Zeroizing<String>, PromptError>>;
}

/// The terminal one. Runs `crossterm::event::read` on a thread of its own and answers through a
/// oneshot, like `worker_cmd::read_dsn_apart`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TtyPrompt;

/// One key's effect, pure so it is testable without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStep {
    /// Keep reading.
    More,
    /// `Enter`.
    Done,
    /// `Esc`, `Ctrl-C`, or the cap.
    Abort,
}

/// Applies `key` (press events only; release and repeat are `More`): a character pushes, Backspace
/// pops, `Enter` is `Done`, `Esc` and `Ctrl-C` are `Abort`.
#[must_use]
pub fn apply_key(buffer: &mut zeroize::Zeroizing<String>, key: &crossterm::event::KeyEvent) -> KeyStep;
```

### 4.5 `provision/verify.rs`

```rust
//! Did the new box reach Postgres, judged against a baseline taken before INSTALL (D305), and its
//! executor (D313). Both timestamps are the database's `clock_timestamp()`; no local clock is used.
//!
//! The production verifier connects through `worker_cmd::connect` with this machine's own config
//! root, which registers or refreshes **this** box's row, as `htui --index-items` does (R-6). It
//! never connects with the remote identity.

/// `boxes()` as `{id → last_seen_at}`.
pub type Baseline = std::collections::HashMap<htui_core::model::BoxId, chrono::DateTime<chrono::Utc>>;

/// The local poll's cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Poll {
    /// Between reads.
    pub interval: std::time::Duration,
    /// Overall.
    pub deadline: std::time::Duration,
}

/// The seam `run_with` verifies through (E-6: boxed futures, so `&dyn Verifier` is formable).
pub trait Verifier: core::fmt::Debug + Send + Sync {
    /// Connects (once) with `dsn` and lists the boxes. `Err` is the reason no baseline exists (the
    /// DSN reaches Postgres only from the remote network, a pending schema, …), never the DSN.
    fn baseline<'a>(&'a self, dsn: &'a str) -> BoxFuture<'a, Result<Baseline, String>>;
    /// Polls until `id` is new against `baseline` or its `last_seen_at` is later; `Err` is the
    /// reason it was not seen.
    fn box_seen<'a>(&'a self, id: BoxId, baseline: &'a Baseline, poll: Poll) -> BoxFuture<'a, Result<(), String>>;
    /// D313: `executor = worker`, `Stale` retried once with the row it carries, skipped when already
    /// `worker` (E-23); `Err` is the warning.
    fn set_executor(&self, id: BoxId) -> BoxFuture<'_, Result<(), String>>;
}

/// The Postgres one.
#[derive(Debug)]
pub struct PgVerifier { root: std::path::PathBuf, store: tokio::sync::OnceCell<htui_store::PgStore> }
impl PgVerifier {
    /// `root` is this machine's config root (`identity::config_root()`; a temp dir in tests).
    #[must_use] pub fn new(root: std::path::PathBuf) -> Self;
}

/// Whether `records` show `id` new against `baseline` or checked in since. Pure.
#[must_use]
pub fn seen_against(records: &[htui_core::model::BoxRecord], id: BoxId, baseline: &Baseline) -> bool;
```

How `PgVerifier` works:
- `baseline`: `worker_cmd::connect(dsn, &self.root, PoolSize::clamped(2))`. A `WorkerExit` becomes
  `Err(sentence)`; its sentences never carry the DSN. Store the `PgStore` in the cell, then
  `boxes()` (`use htui_core::store::WriteStore as _;`).
- `box_seen`: no store → `Err("no connection from here")`. Otherwise loop on `boxes()` +
  `seen_against` until the deadline, sleeping `poll.interval`. Keep the last read error for the
  sentence.
- `set_executor`:
  1. Find the record. Missing → `Err`.
  2. `Executor::of(&row.settings) == Worker` → `Ok`.
  3. `edit_box(id, row.edit_version, BoxEdit { executor: Some(Executor::Worker),
     ..BoxEdit::default() })`.
  4. `Applied` → `Ok`. `Stale(row)` → step 2 then step 3 once more, with that row. Then any
     `Stale` or `Err` → `Err(text)`.

### 4.6 `cli.rs`

```rust
// In `Command`, after `Worker`:
    /// Install this htui build as the `htui-worker` service on a Linux host over your own `ssh`
    /// (system unit, sudo), with the DSN encrypted on that host by `systemd-creds`; then set the
    /// new box's executor to `worker`. The DSN comes from the OS keyring or `--dsn-stdin`; never
    /// argv or the environment.
    Provision(ProvisionArgs),

/// `htui provision`'s arguments. None reads the environment (D291).
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ProvisionArgs {
    /// The ssh destination: a host, `user@host`, or a `~/.ssh/config` alias.
    #[arg(value_name = "DESTINATION")]
    pub destination: String,
    /// Read the DSN the remote host should use from one line of stdin instead of the keyring
    /// (when it reaches Postgres at another address).
    #[arg(long)]
    pub dsn_stdin: bool,
    /// Re-encrypt the DSN on a host that already has one, and restart the service.
    #[arg(long)]
    pub replace_credential: bool,
}

// In `Args`, after `clear_dsn` (OQ-1 (a); E-13):
    /// Read a Postgres DSN for this session from one line of stdin, held in memory only and never
    /// stored (`R-STO-1`): for a TUI over `ssh -t` on a box with no keyring.
    #[arg(long, conflicts_with_all = ["set_dsn", "clear_dsn", "demo", "offline", "index_items", "search_items"])]
    pub dsn_stdin: bool,
```

The two `dsn_stdin` fields do not collide. One belongs to the top-level command and the other to
`worker`. With `args_conflicts_with_subcommands = true`, `htui --dsn-stdin worker` is refused, and
`htui worker --dsn-stdin` parses as the worker's flag. `Args` derives `Default`, so the new `bool`
needs nothing.

### 4.7 `lib.rs`

Dispatch, D307: right after `init_tracing(args.log.as_deref())?;` and before `if args.set_dsn`:

```rust
    if let Some(cli::Command::Provision(provision)) = args.command.clone() {
        // MOD-45 D307: after `init_tracing`, so `--log` works the ordinary way.
        return provision::run(provision).await.map_err(anyhow::Error::from);
    }
```

`run`'s "Startup order" doc gains: "…then `htui provision` (MOD-45), which prints to the shell and
exits…".

The TUI's `--dsn-stdin` (OQ-1 (a); E-13). In the `else` arm that calls `connect::start`:

```rust
        let dsn = if args.dsn_stdin {
            if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                eprintln!("paste the DSN for this session and press Enter (it will be visible; it is not stored):");
            }
            Some(session_dsn(&mut std::io::stdin().lock())?)
        } else {
            None
        };
        connect::start(StartOptions {
            offline: args.offline,
            dsn,
            ..StartOptions::new(identity::config_root()?)
        })
        .await?
```

```rust
/// `--dsn-stdin` for the TUI (MOD-45 OQ-1 (a), `R-STO-1` as amended): one line, validated by
/// `Dsn::parse` before sqlx can log a parameter of it, held for this session only. The keyring is
/// neither read nor written. The line's buffer moves into `StartOptions` without a copy, and
/// `connect::start` wraps it in `Zeroizing` again. Reconnects reuse its own zeroizing copy and never
/// re-read the keyring (`connect.rs` `reconnect_over`).
///
/// # Errors
///
/// A blank line or end of input, a read error, or a DSN `Dsn::parse` refuses; never the DSN's text.
fn session_dsn(reader: &mut dyn std::io::BufRead) -> anyhow::Result<String> {
    let Some(mut line) = secret::read_dsn_line(reader)? else {
        anyhow::bail!("no DSN on stdin; nothing was read");
    };
    htui_store::Dsn::parse(&line)
        .map_err(|err| anyhow::anyhow!("the DSN on stdin cannot be used: {err}"))?;
    Ok(std::mem::take(&mut *line))
}
```

The DSN is read **before** `terminal::init`, like `--set-dsn`, so the shell's cooked line editing
applies. Over `ssh -t` stdin is the pty. When stdin is a pipe, crossterm uses `/dev/tty` afterwards.

### 4.8 `main.rs`

```rust
            let code = error
                .downcast_ref::<htui::worker_cmd::WorkerExit>()
                .map(htui::worker_cmd::WorkerExit::code)
                .or_else(|| {
                    error
                        .downcast_ref::<htui::provision::ProvisionExit>()
                        .map(htui::provision::ProvisionExit::code)
                })
                .unwrap_or(1);
```

The comment above it gains "and `htui provision` (MOD-45 D292)". `main`'s doc "Exit codes:" line
gains `htui provision`'s refusals. `code != 2` keeps a refusal out of GlitchTip, as for the worker
(MOD-41 E-1).

### 4.9 T3 tests (red first)

`cli.rs`:
- `provision_parses_its_destination_and_both_flags`: `htui provision box1 --dsn-stdin
  --replace-credential` → `ProvisionArgs { destination: "box1", dsn_stdin: true,
  replace_credential: true }`. Bare `htui provision box1` → both `false`.
- `provision_needs_a_destination_and_takes_no_dsn_argument`: `htui provision` is an error, and so
  is `htui provision box1 --dsn x`.
- `no_provision_argument_reads_the_environment`: mirrors `no_worker_argument_reads_the_environment`
  (`:192-217`).
- `log_goes_after_provision`: `htui provision box1 --log x` parses, and `htui --log x provision box1`
  does not.
- `the_tui_dsn_stdin_flag_parses_alone_and_excludes_the_keyring_flags`:
  - `htui --dsn-stdin` parses with `command == None`.
  - Each of `--set-dsn`, `--clear-dsn`, `--demo`, `--offline`, `--index-items` and `--search-items
    q` beside it is an error.
  - `htui --dsn-stdin worker` is an error.
  - `htui worker --dsn-stdin` is `Worker(WorkerArgs { dsn_stdin: true, .. })` with `args.dsn_stdin
    == false`.
- `no_tui_argument_but_log_reads_the_environment`: every top-level argument of `Args::command()` has
  `get_env() == None` except `log`.

`lib.rs`, new `#[cfg(test)] mod tests`:
- `session_dsn_reads_one_validated_line`: a `Cursor` holding `"  postgres://u:p@db.example/htui
  \nnext\n"` gives the trimmed DSN, and `next` is left unread.
- `session_dsn_refuses_a_blank_line`: the text is `no DSN on stdin; nothing was read`.
- `session_dsn_refuses_an_unusable_dsn_without_echoing_it`: input
  `postgres://u:SENTINEL-DSN-PW@db.example/htui?bogus=SENTINEL-DSN-PW`. The error contains
  `unrecognised parameter` and not the sentinel.

`provision/mod.rs` `mod tests`. The fakes:
- `ScriptedRemote`: a `Mutex<VecDeque<io::Result<RemoteOutput>>>` of answers and a
  `Mutex<Vec<(String, Vec<u8>)>>` of calls.
- `FakeVerifier`: scripted baseline, seen and executor results, plus an ordered call log.
- `FakePrompt`: a scripted answer and a call count.
- `Timings`: 1 try, pause 0, poll 10 ms / 50 ms.

The cases:
- `provision_exit_codes`: `Refused` is 2 and `Failed` is 1. `Display` is the sentence alone.
- `a_fresh_password_host_runs_four_sessions_with_their_stdin`:
  - The stdins are, in order: ∅, the payload, `SENTINEL-SUDO-PW\n` + DSN + `\n`, ∅.
  - The PREPARE command carries `send` = 1. INSTALL's carries `password` and INSTALL_ROOT's text.
  - The `Outcome` is `Provisioned { verified: true, executor_set: true }`.
  - `out` is exactly `box <id> (<host>) provisioned on <dest>\n`.
  - The verifier log is `baseline`, then `box_seen(id)`, then `set_executor(id)`, each once.
- `a_nopasswd_host_sends_only_the_dsn_and_never_prompts`: INSTALL's stdin is DSN + `\n`, and the
  prompt count is 0.
- `matching_hashes_skip_the_upload`: PREPARE's stdin is empty, `send` is `0`, and no `uploading`
  line appears.
- `already_provisioned_runs_only_the_preflight`: one call, the `Outcome` is `AlreadyProvisioned`,
  and the verifier is never called.
- `local_refusals_make_no_remote_call`: a bad destination, a loopback DSN, a socket DSN
  (`…@localhost/db?host=/run/postgresql`) and a hostless DSN. Each is `Refused`, with zero calls.
- `preflight_refusals_are_refusals`:
  - noise with no keys and exit 0 → the D297 sentence;
  - ssh exit 255 with no keys → `ssh to … failed (exit 255)`;
  - an architecture mismatch;
  - a bad `home`.
  Each is code 2, with one call.
- `an_aborted_prompt_is_a_refusal_before_any_write`: `FakePrompt` answers `Aborted`. The result is
  code 2 after one call (the preflight).
- `exit_codes_map_to_their_sentences`:
  - PREPARE 3 → `does not run on`, with the last stderr line.
  - INSTALL 4 → the D301 sentence.
  - INSTALL 1 without the marker → the D301 sentence (E-7).
  - INSTALL 1 with `htui.root=start` → `installing the service on`.
  - VERIFY 5 → `did not start with a box.toml`, with the stderr included.
  All are code 1.
- `no_baseline_is_a_warning_and_exit_0`: the baseline is `Err("unreachable")`. The result is
  `verified: false` and `executor_set: false`. `err` holds `not verified in Postgres from here:
  unreachable` and `Settings › Boxes`. `box_seen` and `set_executor` are never called.
- `a_box_never_seen_is_a_warning_and_exit_0`: `box_seen` is `Err`, and `set_executor` is never
  called.
- `the_sentinels_reach_only_stdin`: in the password and fresh case, neither sentinel is in any
  command string, `out` or `err`. Both are in INSTALL's stdin.

`provision/remote.rs`:
- `ssh_argv_puts_the_separator_before_the_destination`: the argv is exactly `["-T", "-o",
  "ConnectTimeout=15", "--", dest, command]`.
- `ssh_argv_carries_no_sentinel`: build it for each of the four sessions with the arguments
  `run_with` uses. Neither sentinel appears.
- `run_piped_writes_stdin_while_draining_output`: `sh -c cat` with 4 MiB of stdin.
  - Code 0.
  - `stdout.len() == OUTPUT_CAP`.
  - It finishes within 10 s: a sequential write would deadlock.
- `run_piped_tolerates_a_child_that_never_reads`: `sh -c "exit 4"` with 4 MiB of stdin →
  `Ok(code: Some(4))`.

`provision/secret_prompt.rs`:
- `apply_key_builds_and_ends_the_password`: characters, Backspace and `Enter`.
- `apply_key_aborts_on_escape_ctrl_c_and_the_cap`.
- `apply_key_ignores_releases`.

`provision/verify.rs`:
- `seen_against_new_moved_and_unchanged`:
  - absent from the baseline → `true`;
  - same `last_seen_at` → `false`;
  - later → `true`;
  - not in `records` → `false`.

### 4.10 Note on `run_with`'s generic and `Send`

`run_with` is awaited only via `block_on`: `main`'s runtime and `#[tokio::test]`. It is never
`tokio::spawn`ed, so neither its future nor `Ctx` needs to be `Send`. Do not add
`tokio::spawn(run_with(..))`.

### 4.11 T3 gate and commits

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib -- provision:: cli:: tests::session_dsn --test-threads=1
cargo test -p htui --all-features --bin htui -- --test-threads=1
cargo clippy -p htui --all-targets --all-features -- -D warnings
cargo tree -p htui -i sha2@0.10.9      # one 0.10 instance, htui a direct dependent
```

1. `build(mod-45): T3 — sha2 for htui`: `Cargo.toml` and `Cargo.lock`.
2. `test(mod-45): T3 red — CLI, ProvisionExit, ssh_argv, run_with over fakes`: types and signatures
   complete, `todo!()` bodies only where no existing path calls them.
3. `feat(mod-45): T3 — SshRemote, the no-echo prompt and the Postgres verifier`.
4. `feat(mod-45): T3 — run_with, run, the provision subcommand and its exit code`: `mod.rs`,
   `cli.rs`, `lib.rs` dispatch, `main.rs`.
5. `feat(mod-45): T3 — the TUI's --dsn-stdin (OQ-1 a)`: `cli.rs` flag, `lib.rs` `session_dsn`,
   their tests.

---

## 5. T4: end to end over the real scripts (D309–D311; E-4, E-10, E-17)

**Files**: `crates/htui/tests/provision.rs` (`#![cfg(unix)]`) and `crates/htui/tests/provision_pg.rs`.
`crates/htui/src/provision/script.rs` changes only if a scenario exposes a script bug (§5.8).

### 5.1 The world one scenario runs in

```rust
/// One scenario's directories, all under `env!("CARGO_TARGET_TMPDIR")` (E-17: `/tmp` may be
/// `noexec`, and every path must pass D296's home charset).
struct World {
    _base: tempfile::TempDir,
    root: PathBuf,   // `$1` of every script: `<root>/etc/...` is where the "root" writes land
    home: PathBuf,   // `HOME` of every remote process
    stub: PathBuf,   // the stub binaries and their state files; first on `PATH`
}
```

`World::new()` creates the three directories and writes the six stubs (§5.2) with mode 0755. It
also writes these state files:

| File | Default |
|---|---|
| `os` | `Linux` |
| `arch` | `std::env::consts::ARCH` |
| `systemd.version` | `255` |
| `user`, `group` | `provtest` |
| `sudo.mode` | per scenario |
| `sudo.pwhash` | lowercase hex sha256 of `SENTINEL-SUDO-PW`, computed in Rust with `sha2` |
| `box.toml` | `BOX_TOML` |

The password itself is **never** written to any file. `BOX_TOML` reads:

```toml
box_id = "0190f0e0-0000-7000-8000-000000000045"
hostname = "stub-host"
```

The two payloads, built with `Payload::new(bytes, std::env::consts::ARCH)`:
- `GOOD_HTUI`: `#!/bin/sh` followed by `echo "htui 0.0.0-stub"`.
- `BAD_HTUI`: `#!/bin/sh`, then `echo "version GLIBC_2.99 not found (required by htui)" >&2`, then
  `exit 1`.

### 5.2 The stubs, exactly

Each is a Rust `r##"…"##` constant (they contain quotes and backslashes, which are allowed in test
files). Each logs its `argv` to `$STUB_DIR/argv.log`. The four that run under `sudo` or touch
system state also log `env` to `$STUB_DIR/env.log`. They model only the flags the scripts use (R-2);
anything else exits 64, so a script change that leaves the model fails loudly.

`sudo`. The timestamp is keyed on `$PPID`, modelling V-5, so E-4 is pinned:

```sh
#!/bin/sh
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
```

How each call the scripts make behaves:

| Call | Behaviour |
|---|---|
| `sudo -k -n true` (preflight) | Ignores the timestamp. `nopasswd` → runs `true` (0). `password` → exit 1 |
| `sudo -k` | Removes the timestamp |
| `printf … \| sudo -S -p "" -v` | Reads at most three lines, logging one `line` per read and never the text. A match writes `$PPID` |
| `sudo -n sh -c …` | Runs only when `nopasswd`, or when the timestamp equals **its own** `$PPID` |

An exec'd (unguarded) `sudo -n` would have the outer shell as its `$PPID` and be refused. `printf`
is dash's builtin here, so `$line` is never in an `argv`.

`systemctl`:

```sh
#!/bin/sh
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
```

`systemd-creds`:

```sh
#!/bin/sh
# MOD-45 T4 stub systemd-creds: "encrypts" by wrapping stdin in a marker.
set -u
d=$STUB_DIR
printf '%s\n' "systemd-creds $*" >> "$d/argv.log"
env >> "$d/env.log"
{ [ $# -eq 4 ] && [ "$1" = encrypt ] && [ "$2" = --name=htui-dsn ] && [ "$3" = - ]; } || exit 64
{ printf 'ENCRYPTED('; cat; printf ')'; } > "$4"
```

`uname`, `id`, `journalctl`:

```sh
#!/bin/sh
d=$STUB_DIR
printf '%s\n' "uname $*" >> "$d/argv.log"
case "${1:-}" in -s) cat "$d/os" ;; -m) cat "$d/arch" ;; *) exit 64 ;; esac
```

```sh
#!/bin/sh
d=$STUB_DIR
printf '%s\n' "id $*" >> "$d/argv.log"
case "${1:-}" in -un) cat "$d/user" ;; -gn) cat "$d/group" ;; *) exit 64 ;; esac
```

```sh
#!/bin/sh
d=$STUB_DIR
printf '%s\n' "journalctl $*" >> "$d/argv.log"
echo "stub journal: htui-worker.service: Main process exited, status=2/INVALIDARGUMENT"
echo "stub journal: htui: no Postgres DSN"
```

`sha256sum`, `install`, `mkdir`, `mv`, `chmod`, `cat`, `tail`, `sleep` and `sh` are the real
ones, from `/usr/bin:/bin`. The exception is S3, where `$STUB_DIR/sh` is a symlink to `bash`.

### 5.3 `LocalShell: Remote`

```rust
/// Runs each remote command as `sh -c <command>`, which is what sshd does with the login shell (V-7),
/// with a cleared environment: `HOME` = the world's home, `PATH` = `<stub>:/usr/bin:/bin`,
/// `STUB_DIR`, `LC_ALL=C`. Records every command for the sweep. With `noise`, the command is
/// prefixed by `echo "Welcome to stub-host"; echo "htui-motd: hi";`, as a chatty `~/.bashrc` would
/// print before it.
#[derive(Debug)]
struct LocalShell { home: PathBuf, stub: PathBuf, noise: bool, commands: std::sync::Mutex<Vec<String>> }

impl Remote for LocalShell {
    async fn run(&self, command: &str, stdin: &[u8]) -> std::io::Result<RemoteOutput> {
        // record `command` (not the noisy one), build `tokio::process::Command::new("sh")` with
        // `.env_clear()` and the four variables, then `htui::provision::remote::run_piped(cmd, stdin)`.
    }
}
```

`Command::new("sh")` with an explicit `PATH` resolves `sh` through that `PATH` (std on Unix), so S3's
symlink applies to the outer shell too.

### 5.4 Fakes

These are the same shapes as T3's, rewritten here because integration tests cannot see `#[cfg(test)]`
items:
- `FakePrompt`: answers a fixed `Result` and counts its calls.
- `FakeVerifier`:
  - `baseline` → `Ok(Baseline::new())`, and records the `dsn` argument only as "called"; the
    argument is the DSN and must not be stored.
  - `box_seen` → `Ok`.
  - `set_executor` → `Ok`.
  - Every call is logged in order.

`Timings { verify_tries: 2, verify_pause_secs: 0, poll: Poll { interval: 10 ms, deadline: 100 ms } }`.
`DSN = "postgres://u:SENTINEL-DSN-PW@db.example:5432/htui"`, `DEST = "stub-dest"`.

### 5.5 Scenarios (each ends with the §5.6 sweep)

| # | Name | Set-up | Asserts |
|---|---|---|---|
| S1 | `a_fresh_password_host_is_provisioned` | `sudo.mode = password`, prompt → `SENTINEL-SUDO-PW` | (a) The `Outcome` is `Provisioned { box_id: 0190f0e0-…-045, hostname: "stub-host", verified: true, executor_set: true }`. (b) `out` == `box 0190f0e0-0000-7000-8000-000000000045 (stub-host) provisioned on stub-dest\n`. (c) `home/.local/bin/htui` == `GOOD_HTUI`, mode 0755, and `home/.local/state/htui/` exists. (d) `root/etc/credstore.encrypted` is mode 0700, and its `htui-dsn` == `ENCRYPTED(` + DSN + `\n)`. (e) The unit file == the D295 text with `User=provtest` and the world's home, mode 0644. (f) `argv.log` holds `systemctl daemon-reload` before `systemctl enable --now htui-worker.service`, and no `restart`. (g) `sudo.reads` holds exactly 1 line, so sudo never read the DSN. (h) The verifier log is `baseline`, `box_seen`, `set_executor`. (i) No `.htui.provision.*` or `*.htui-provision.*` remains |
| S2 | `a_fresh_nopasswd_host_never_prompts` | `nopasswd` | As S1 (a)–(f). The prompt count is 0, and `sudo.reads` is absent |
| S3 | `a_host_whose_sh_is_bash_keeps_the_sudo_timestamp` (E-4) | as S1, plus `stub/sh → $(command -v bash)`. Skip with a printed note when bash is absent | As S1 (a), (d), (g). This proves `sudo -n` is forked: its `$PPID` equals the `-v` caller's |
| S4 | `a_re_run_changes_nothing` | S2, then a second `run_with` in the same world | The second run's `Outcome` is `AlreadyProvisioned`, after exactly **one** command (the preflight). `out` contains `already provisioned`. The binary, credential, unit and `box.toml` bytes are unchanged |
| S5 | `replace_credential_re_encrypts_and_restarts` | S2, then a second run with `replace_credential: true` and DSN2 = `…SENTINEL-DSN-PW@db2.example:5432/htui` | PREPARE's stdin is empty and its `send` is `0`. The credential contains `db2.example`. `argv.log`'s last `systemctl` line is `restart htui-worker.service`. The `Outcome` is `Provisioned` |
| S6 | `a_wrong_sudo_password_writes_nothing_privileged` | `password`, prompt → `SENTINEL-SUDO-PW-WRONG` | `Err(Failed)`, code 1, holding `sudo refused the password`. `root/etc` **does not exist**. No `systemd-creds` line in `argv.log`. `sudo.reads` holds exactly 1 line. The binary *was* installed (PREPARE ran: a failure, not a refusal, D292) |
| S7 | `an_architecture_mismatch_is_refused_before_any_write` | `arch` = `aarch64` when local is `x86_64`, else `x86_64` | `Err(Refused)`, code 2. One command. `home/.local` does not exist |
| S8 | `a_binary_that_does_not_run_is_removed` | payload `BAD_HTUI`, `nopasswd` | `Err(Failed)` == `provisioning stub-dest failed: the htui binary does not run on stub-dest: version GLIBC_2.99 not found (required by htui)`. `home/.local/bin` holds no `htui` and no `.htui.provision.*`. `root/etc` does not exist. No `systemctl` line |
| S9 | `a_different_installed_build_is_refused` | seed `home/.local/bin/htui` = `b"other"` and an empty `root/etc/systemd/system/htui-worker.service` | `Err(Refused)` containing `different build; upgrading is not supported yet`. One command. The seeded binary is unchanged |
| S10 | `a_chatty_login_shell_is_still_parsed` | `nopasswd`, `noise: true` | As S2 (a), (b) |
| S11 | `a_service_that_never_starts_times_out_with_its_journal` | `nopasswd`, `never-active` present | `Err(Failed)` containing `did not start with a box.toml within 0 s` (2 tries × 0 s), `systemctl says activating` and `stub journal: htui: no Postgres DSN`. `box_seen` was never called |

### 5.6 The D310 sweep, after every scenario

```rust
/// D310: the DSN and the password reach nothing but stdin and the encrypted credential.
fn sweep(world: &World, shell: &LocalShell, out: &[u8], err: &[u8]);
```

1. Neither `SENTINEL-DSN-PW` nor `SENTINEL-SUDO-PW` appears in any `shell.commands` entry, in `out`
   or in `err`.
2. Neither appears in any file under `world.stub`: `argv.log`, `env.log`, `sudo.reads` and the
   state files.
3. Every file under `world.root` and `world.home`, walked recursively: the DSN sentinel appears
   only in `root/etc/credstore.encrypted/htui-dsn`, and the password sentinel appears in none.

The command check also covers S6: its wrong password contains the sentinel string, so a leak of it
is caught too.

### 5.7 `provision_pg.rs` (D311)

Gated like `worker_pg.rs:16-17`: `testkit::demo_db()` returns `None` and prints `SKIP` without
`HTUI_TEST_DATABASE_URL`, and panics under `CI`. Every connect uses a temp root, never
`~/.config/htui`. Each case ends with `db.drop_db().await`. The poll is `Poll { interval: 20 ms,
deadline: 10 s }` for "seen" and 300 ms for "not seen".

- `a_box_registered_after_the_baseline_is_seen`:
  1. `PgVerifier::new(local_root)`, then `baseline(&db.url)`.
  2. `worker_cmd::connect(&db.url, remote_root, PoolSize::clamped(2))`; its `this_box()` is
     `remote`.
  3. `box_seen(remote, &baseline, poll)` is `Ok`.
- `an_unchanged_box_in_the_baseline_is_not_seen`: register `remote` first, take the baseline, then
  `box_seen(remote, …, 300 ms)` is `Err`.
- `a_touch_after_the_baseline_is_seen`: as above, then `db.store.touch_box(remote)` (`pg/mod.rs:639`),
  then `box_seen` is `Ok`.
- `set_executor_writes_worker_and_keeps_every_other_key`:
  1. Write `UPDATE box SET settings = settings || '{"max_concurrent_items": 3}'` for `remote`
     (runtime `sqlx::query`).
  2. `set_executor(remote)` is `Ok`.
  3. Re-read with `boxes()`: `Executor::of(&row.settings) == Executor::Worker`, and
     `row.settings["max_concurrent_items"] == 3`.
  4. A second `set_executor` is `Ok` and leaves `edit_version` unchanged (E-23).

### 5.8 T4 gate and commits

```bash
cargo fmt --all -- --check
pg_isready -h localhost -p 5439
cargo test -p htui --all-features --test provision --test provision_pg -- --test-threads=1
```

1. `test(mod-45): T4 — provisioning end to end over stubbed remote tools`.
2. `test(mod-45): T4 — the verifier against Postgres`.

A script bug that a scenario exposes is fixed in `script.rs`, in its own commit `fix(mod-45): T4 —
<script>: <what>`. The fix keeps every T2 test green, re-runs §3.6, and is recorded in the handoff
notes.

---

## 6. T5: the guide and the README (D312; E-11, E-15, E-16, E-22)

**Files**: `docs/htui-worker.md`, `README.md`. Gate: `bash
.claude/skills/handoff-run/scripts/validate-workflow-docs.sh`, plus every new anchor resolving.
Commit: `docs(mod-45): T5 — provisioning a remote box`.

`docs/htui-worker.md`:
- **`:62-63`** becomes: "It does not install itself; `htui provision <host>` installs it on another
  Linux machine (see [Provisioning a remote box](#provisioning-a-remote-box)). The [sample
  unit](#running-it-as-a-systemd-service) is for doing it by hand."
- **`:255`** becomes: "This is the unit `htui provision` writes (with the binary in
  `~/.local/bin`); use it by hand on a host where sudo over ssh is not available."
- **TOC (`:12-22`)**: add `- [Provisioning a remote box](#provisioning-a-remote-box)` before the
  systemd entry.
- **New section `## Provisioning a remote box`**, before `## Running it as a systemd service`. The
  sentences in quotes are required. The rest is free prose.
  1. **Command**: `htui provision <destination> [--dsn-stdin] [--replace-credential] [--log PATH]`,
     where the destination is anything `ssh` accepts. "It runs your own `ssh`, so `~/.ssh/config`,
     `ProxyJump` and your agent apply. It opens four short sessions; with password logins you are
     asked once per session, so use a key or `ControlMaster`."
  2. **Requirements**: Linux on both ends, the same CPU architecture (x86_64 or aarch64), systemd
     250 or later with `systemd-creds`, sudo (a password prompt is fine), and "a POSIX login shell
     on the remote user (sh, bash, zsh, ksh); fish and csh are refused."
  3. **What is written where**: the four D294 paths, and "the DSN is encrypted by `systemd-creds` on
     that host and is never in a command line, the environment, a log or a plain file on either
     machine."
  4. **The DSN**: "From your OS keyring, or one line of stdin with `--dsn-stdin` when the remote
     host reaches Postgres at another address. A DSN whose host is `localhost`, a loopback address
     or a Unix socket is refused." **Do not provision through a sudo configured with `log_input`**:
     it would record the DSN in sudo's I/O log (E-16).
  5. **Verification and the executor**: the service must be active with a `box.toml` within 60 s.
     The box is then checked in Postgres from this machine, which "registers or refreshes this
     machine's own box, as `htui --index-items` does". If the box checks in, its executor is set to
     `worker`. Otherwise a warning says how to set it in **Settings › Boxes** (E-11 wording, D313).
  6. **Re-running**: "On a host already provisioned with this build and running, it says so and
     changes nothing. `--replace-credential` re-encrypts the DSN and restarts the service. A host
     with a different build, or one set up by hand, is refused: upgrading is not supported yet."
     The binary in `~/.local/bin/htui` is replaced on a host without the unit.
  7. **Exit codes**: 0, 2 (refused, nothing written), 1 (failed after a remote write; re-running
     completes it). The four failure sentences are listed: the binary does not run; sudo refused,
     "nothing privileged was written"; the service did not start, with the journal and log tails;
     ssh dropped.
  8. **Agent login on the new box** (OQ-1 (a)): "`ssh -t <host> ~/.local/bin/htui --dsn-stdin`,
     paste the DSN, then **Settings › Agents**. The DSN is held in memory for that session only."
     Settings › Connection there shows the keyring's state, not this session's DSN (E-15).
  9. **Not done**: upgrades, uninstalling, non-Linux targets, user units.
- The manual sample stays.

`README.md`:
- `:87`'s table gains a row after `worker`: `` `provision <DESTINATION> [--dsn-stdin]
  [--replace-credential] [--log PATH]` `` | "Install this build as the `htui-worker` service on
  another Linux machine over `ssh`, with the DSN encrypted there by `systemd-creds`, and set that
  box's executor to `worker`. See [`docs/htui-worker.md`](docs/htui-worker.md#provisioning-a-remote-box)."
- A flag row after `--clear-dsn`: `` `--dsn-stdin` `` | "Read the connection string for this session
  from standard input; it is never stored. For a TUI over `ssh -t` on a machine with no keyring."

---

## 7. Final gate and close-out

```bash
df -h /
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
cargo tree -p htui -i sha2@0.10.9
git diff --stat de177dd -- crates/htui-worker crates/htui-orch crates/htui-agent crates/htui-core   # empty (D314)
git diff --stat de177dd -- crates/htui-store      # exactly identity.rs and dsn.rs (D314, E-14)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Close-out (main thread, not an implementer):
- The `R-STO-1` amendment text from the plan's OQ block.
- The HANDOFF wording fixes the plan lists under "Where the HANDOFF or tree disagree".
- The maintainer's live check, with E-24's additions, recorded in `docs/decisions/mod/mod-45.md`.
- E-15 as a follow-up item candidate.
- E-11's open question (exit 1 on "never checked in") put to the maintainer.
