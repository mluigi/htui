# Blueprint: MOD-79 - MCP token off claude's argv

**Plan**: `.claude/plans/mod-79-mcp-token-off-argv.plan.md` (confirmed 2026-10-06, Windows option A)
**Scope**: design within D1-D3 and option A. Nothing here reopens them.

## Plan corrections (found while reading the source)

| # | Plan claim | What the source says | Effect |
|---|---|---|---|
| PC-1 | `argv` has 8 test call sites (`tests/cli_driver.rs:171,207,249,472,473,543,582,590`) | **13.** The plan's search matched single-line calls only. Five calls are split across lines: `:100`, `:147`, `:188`, `:430`, `:556` | T3 updates 13 sites, listed in full below |
| PC-2 | `RedactedEnv` is at `launch.rs:452-461` | `launch.rs:452-461` is `ResolvedLaunch`'s `Debug` impl. `RedactedEnv` is `driver.rs:325`, and the `REDACTED` const it prints is **private** at `driver.rs:29` | T4 also touches `driver.rs` (one visibility change). The lanes are still disjoint |
| PC-3 | Files table: no manifest change | `htui-agent`'s `uuid` has `["v7","serde"]` only (`Cargo.toml:55`). `Uuid::new_v4()` compiles today only because `htui-mcp` adds `v4` (`crates/htui-mcp/Cargo.toml:28`, blueprint H-9 there). `cargo check -p htui-agent` on its own would fail | T1 adds `features = ["v4"]` to `crates/htui-agent/Cargo.toml`. That is a feature, not a crate: `Cargo.lock` does not move and "no new dependency" still holds |
| PC-4 | Guard mechanism "SessionOptions field **or** `open_session` parameter" | A parameter threaded into `run_session` gives it 8 arguments. That fires `clippy::too_many_arguments` (default threshold 7; `run_session` has exactly 7, `cli/mod.rs:882-890`) | The parameter is moved into a wrapper future in `open_session`. `run_session`'s signature does not change (see G-1) |

All the other verified claims in the plan hold: the `SessionOptions` literal sites, the timeout-arm abort at
`cli/mod.rs:841-853`, `unsafe_code = "forbid"`, `tempfile` dev-only, and `htui-mcp → htui-agent`.

## Architecture: MCP config as a session-scoped private file

### Design decisions

- **G-1: the guard is an `open_session` parameter, moved into the task's future.**
  `open_session(io, spec, prompt, options, mcp_config: Option<McpConfigFile>)`. `open_session` spawns
  `async move { run_session(io, spec, prompt, options, ready_tx, events_tx, commands_rx).await; drop(mcp_config); }`.
  Against the four constraints:
  - *SessionOptions derives `Debug + Clone`.* A field forces `Arc<McpConfigFile>` to keep `Clone`, and then any
    clone silently extends the file's life past the session. That breaks D3's "a session-scoped guard is the
    invariant a reader can check". With a parameter the guard is move-only, the compiler proves it has one owner,
    and `SessionOptions` stays plain configuration.
  - *Literal construction sites.* Both mechanisms touch exactly two sites. A field touches `cli/mod.rs:501` and
    `tests/cli_driver.rs:1447`. A parameter touches the `open_session` calls at `cli/mod.rs:510` and
    `tests/cli_driver.rs:1458`. The churn is equal, so it does not decide anything.
  - *The abort in the timeout arm must drop it.* The guard is a capture of the spawned future, beside the
    `ChildGuard` that `run_session` creates. `task.abort(); let _ = task.await;` drops the future, so it drops the
    guard before `open_session` returns `Err`. This is the same mechanism exit (1) already relies on for the child.
  - *A failed spawn in `io()` must drop it.* `io()` owns the guard as a local until it returns `Ok((io, guard))`. A
    `?` on `launch::spawn` or `ChildIo::from_spawned` drops it.
  - The order on a normal end is: `run_session` returns only after `kill(&child).await`, then `drop(mcp_config)`.
    The file is removed after the CLI is gone, never before.
- **G-2: `McpConfigFile` is `pub`, re-exported as `htui_agent::cli::McpConfigFile`.** It is declared in the private
  `mod mcp_file;`. The re-export is needed for three reasons:
  1. `open_session` is `pub`, and its parameter type must be nameable for rustdoc.
  2. The exit-(1) test builds a real guard and hands it to `open_session`. The production timeout is a fixed 60 s,
     so that arm cannot be reached through `CliDriver::start` in a test.
  3. It keeps T2's commit free of dead code under featureless clippy (H-8).
- **G-3: `argv` takes the path and no longer reads `spec.mcp`.** It is pure over its arguments. `io()` is the one
  place that derives the file from `spec.mcp`. `mcp_config` stays `pub` and unchanged, and still pins the JSON shape.
- **G-4: synchronous `std::fs` in `McpConfigFile::write`.** This mirrors `channel.rs` `bind`. The file is a few
  hundred bytes, and `Drop` has to be synchronous anyway. Do not use `tokio::fs`.
- **G-5: Windows (option A) means `#[cfg(not(unix))]`, not `#[cfg(windows)]`.** Every non-Unix target then compiles
  the plain `create_dir` body, and the code has no hole for a third platform.

### Files to create

| File | Purpose | Task |
|---|---|---|
| `crates/htui-agent/src/private_dir.rs` | `create(prefix)`: the per-process private directory (moved from `channel.rs`) | T1 |
| `crates/htui-agent/src/cli/mcp_file.rs` | `McpConfigFile`: writes the `0600` `mcp.json` in a private dir; `Drop` removes both | T2 |

### Files to modify

| File | Change | Task |
|---|---|---|
| `crates/htui-agent/Cargo.toml` | `uuid = { workspace = true, features = ["v4"] }`, with a comment (PC-3) | T1 |
| `crates/htui-agent/src/lib.rs` | `pub mod private_dir;` between `persona` and `probe` (rustfmt `reorder_modules`); one crate-doc paragraph | T1 |
| `crates/htui-mcp/src/channel.rs` | unix `platform`: delete `base`/`private_dir` and their now-unused imports; `bind` calls the helper | T1 |
| `crates/htui-agent/src/cli/mod.rs` | `mod mcp_file; pub use mcp_file::McpConfigFile;` (T2). `argv` signature and docs, `io()`, `start`, `open_session` (T3) | T2, T3 |
| `crates/htui-agent/tests/cli_driver.rs` | 13 `argv` sites, 1 `open_session` site, three process tests, `Path` import unconditional | T3 |
| `crates/htui-agent/src/driver.rs` | `const REDACTED` → `pub(crate) const REDACTED` | T4 |
| `crates/htui-agent/src/launch.rs` | `RedactedArgs` wrapper; `ResolvedLaunch`'s `Debug` uses it | T4 |
| `crates/htui-agent/tests/launch.rs` | three redaction tests; `ResolvedLaunch` moves to the unconditional import | T4 |
| `docs/htui-mcp.md` | the `:187-191` bullet; the crash note and `ls` line in "The socket"; one troubleshooting entry | T5 |

---

## T1: `htui_agent::private_dir`

### API (`crates/htui-agent/src/private_dir.rs`)

```rust
//! Private per-process directories, for a file another local account must not read (MOD-79 D2).
//!
//! One helper, two callers: `htui-mcp`'s socket directory (`htui-mcp-<pid>-<8 hex>`) and the CLI
//! driver's MCP config (`htui-cli-<pid>-<8 hex>`, [`crate::cli::McpConfigFile`]). Both live under
//! the same base, so whatever isolation lets the relay child reach the socket also lets the CLI read
//! its config.

/// Creates `<base>/<prefix>-<pid>-<8 hex>` and returns its path. The caller owns the directory and
/// removes it; after a success this function never does.
///
/// On Unix, `<base>` is `$XDG_RUNTIME_DIR` when that is set to an existing absolute directory, else
/// [`std::env::temp_dir`]. The directory is created with mode `0700` and its mode is read back. A
/// directory that grants anything to group or others is removed and refused.
///
/// Elsewhere (Windows), `<base>` is [`std::env::temp_dir`] (`%LOCALAPPDATA%\Temp` for a user), and
/// the directory inherits that directory's ACL: the account, SYSTEM and Administrators (MOD-79,
/// option A). Nothing checks it at run time; that is MOD-16's.
///
/// # Errors
///
/// The `create` error (the name exists, the base is not writable), the metadata read's, or, on Unix
/// only, `"<dir> is not private (mode <octal>)"`.
pub fn create(prefix: &str) -> std::io::Result<std::path::PathBuf>
```

### Bodies

- `#[cfg(unix)] fn base() -> PathBuf { base_from(std::env::var_os("XDG_RUNTIME_DIR")) }`
- `#[cfg(unix)] fn base_from(xdg: Option<OsString>) -> PathBuf`. This is `channel.rs`'s `base()` body moved
  verbatim, with the env read lifted out so a test can drive it (H-7).
- `#[cfg(not(unix))] fn base() -> PathBuf { std::env::temp_dir() }`
- `create`: the name is
  `format!("{prefix}-{}-{}", std::process::id(), &uuid::Uuid::new_v4().simple().to_string()[..8])`. Then:
  - `#[cfg(unix)]`: `channel.rs:497-510` verbatim. That is `DirBuilder::new().mode(0o700).create(&dir)?`, then the
    mode is re-read. If `mode & 0o077 != 0`, it calls `remove_dir` and returns `Err(io::Error::other(format!("{} is
    not private (mode {mode:o})", dir.display())))`. Gate the `DirBuilderExt` and `PermissionsExt` imports with
    `#[cfg(unix)]`.
  - `#[cfg(not(unix))]`: `std::fs::create_dir(&dir)?`.
  - `Ok(dir)`.

### `channel.rs` switch (unix `platform` module only)

- `bind`: `let dir = htui_agent::private_dir::create("htui-mcp")?;`. Everything after it stays byte-identical. A
  failed `UnixListener::bind` still calls `remove(&dir)`. A non-UTF-8 socket path still calls `remove(&dir)` and
  returns `"the socket path is not UTF-8"`. `remove()` stays: file `SOCKET_NAME`, then the dir.
- Delete `fn base()` and `fn private_dir()`. Delete `DirBuilderExt` and `PermissionsExt` from the imports.
  `Path`/`PathBuf` stay, because `remove` and `bind`'s return type use them.
- `SOCKET_NAME` stays in both `platform` modules (`Listener::close` names it) (H-3).
- The Windows `platform` module is untouched.

### Tests

Unit tests in `private_dir.rs` `#[cfg(test)] mod tests`. They need no feature. Each test removes the
directories it creates.

| Test | Asserts |
|---|---|
| `a_private_dir_is_named_after_its_prefix_and_pid` | `file_name` is `<prefix>-<pid>-` plus 8 lowercase hex; `parent() == Some(base())`; the dir exists |
| `two_private_dirs_never_share_a_name` | two `create("mod79-test")` calls give different paths |
| `#[cfg(unix)] a_private_dir_has_mode_0700` | `metadata.permissions().mode() & 0o777 == 0o700` |
| `#[cfg(unix)] the_base_is_xdg_runtime_dir_only_when_absolute_and_a_directory` | `base_from(None)`, `Some("relative")` and `Some("/nonexistent-mod79")` give `temp_dir()`; `Some(tempdir)` gives that dir |

Regression guard for the switch: `crates/htui-mcp/tests/channel.rs` already pins it with
`the_socket_directory_is_private` (name prefix and `0700`) and `close_removes_the_socket_and_its_directory`.
No new `htui-mcp` test is needed. The refusal branch (`& 0o077 != 0`) cannot be produced from a test, because a
umask only clears bits. It stays covered by being moved verbatim.

Validate: `cargo test -p htui-agent private_dir`, `cargo test -p htui-mcp --all-features channel`, and
`cargo check -p htui-agent` **on its own** (PC-3).

---

## T2: `McpConfigFile` (`crates/htui-agent/src/cli/mcp_file.rs`)

```rust
//! A CLI session's `--mcp-config` file (MOD-79): the JSON that used to ride on the argv, where only
//! the session's own account can read it.

/// The directory's name prefix: `htui-cli-<pid>-<8 hex>`.
const DIR_PREFIX: &str = "htui-cli";
/// The file's name inside it.
const FILE_NAME: &str = "mcp.json";

/// [`super::mcp_config`]'s JSON in `<dir>/mcp.json`, with `<dir>` a fresh
/// [`crate::private_dir::create`] directory. On Unix the file has mode `0600`.
///
/// The session owns it for its whole life (MOD-79 D3): [`super::open_session`] moves it into the
/// session task, so every exit drops it, the abort of a session that never sent `system/init`
/// included. `Drop` removes the file and then the directory, best effort. `Debug` prints the
/// path, never the contents. Move-only on purpose: a copy would outlive the session.
pub struct McpConfigFile { path: PathBuf, dir: PathBuf }

impl McpConfigFile {
    /// Writes `servers`' config. `Ok(None)` for an empty slice, with no directory created.
    ///
    /// # Errors
    /// [`crate::private_dir::create`]'s, a path that is not UTF-8 (the argv is `String`s), or the
    /// file's create/write error. Every error removes what was already created.
    pub fn write(servers: &[McpServerSpec]) -> std::io::Result<Option<Self>>;

    /// The file, for `--mcp-config=`.
    #[must_use]
    pub fn path(&self) -> &Path;
}
```

`write` body, in this order (H-10):
1. `let Some(json) = super::mcp_config(servers) else { return Ok(None) };`
2. `let dir = crate::private_dir::create(DIR_PREFIX)?;`
3. `let file = Self { path: dir.join(FILE_NAME), dir };`. This builds **the guard before anything else can fail**,
   so every `?` below removes the dir (and any partial file).
4. If `file.path.to_str()` is `None`, return `Err(io::Error::other("the MCP config path is not UTF-8"))`. This
   mirrors `channel.rs`'s socket-path check.
5. `let mut options = std::fs::OpenOptions::new(); options.write(true).create_new(true);` then
   `#[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt as _; options.mode(0o600); }`.
6. `options.open(&file.path)?.write_all(json.as_bytes())?;` then `Ok(Some(file))`.

`Drop`: `let _ = std::fs::remove_file(&self.path); let _ = std::fs::remove_dir(&self.dir);`.

`Debug`: `f.debug_struct("McpConfigFile").field("path", &self.path).finish()`.

Not `Clone`.

`cli/mod.rs` (T2's part): `mod mcp_file;` next to `pub mod claude;`, and `pub use mcp_file::McpConfigFile;`
(G-2). The re-export is what keeps this commit green under featureless clippy (H-8).

### Tests

Unit tests in `mcp_file.rs` `#[cfg(test)] mod tests`, with a local `fn server()` that builds a
`McpServerSpec` with token `"planted-mod79-token"`:

| Test | Asserts |
|---|---|
| `no_servers_write_no_file` | `write(&[])` is `Ok(None)` |
| `the_file_holds_the_clis_config_json` | `read_to_string(path) == mcp_config(&[server()]).unwrap()`; `path` ends in `mcp.json`; parent name starts `htui-cli-<pid>-` |
| `#[cfg(unix)] the_file_is_0600_in_a_0700_directory` | file mode `& 0o777 == 0o600`, dir `== 0o700` |
| `dropping_the_guard_removes_the_file_and_its_directory` | after `drop`, neither path exists |
| `debug_prints_the_path_and_never_the_contents` | `{:?}` contains the path, not `planted-mod79-token` and not `mcpServers` |

---

## T3: argv carries the path; the guard lives with the session

### `argv` (`cli/mod.rs:133`)

```rust
#[must_use]
pub fn argv(
    row_args: &[String],
    cli: &CliSettings,
    spec: &SessionSpec,
    session_id: &str,
    mcp_config: Option<&Path>,
) -> Vec<String>
```

`:181-184` becomes:

```rust
// MOD-11 D8: after the last pair, before the narrowing, so `extra_args` still wins. MOD-79: the
// file's path, never the JSON.
if let Some(config) = mcp_config {
    args.push(format!("--mcp-config={}", config.to_string_lossy()));
}
```

`to_string_lossy` mirrors `--add-dir` at `:174`. It is exact here, because `McpConfigFile::write` refuses a
non-UTF-8 path. Add `use std::path::Path;`.

### Doc text

Add this after the first paragraph of `argv`'s doc ("Pure, and unit-tested as a list rather than through a
process."):

```text
/// `mcp_config` is the path of the [`McpConfigFile`] the caller wrote for `spec.mcp`; `argv` does not
/// read `spec.mcp` itself, so the code that owns the file is the code that decides the flag.
```

Item 6¼ becomes:

```text
///    6¼. `htui`'s own MCP servers (MOD-11 D8, MOD-79): one `--mcp-config=<path>` argument when
///    `mcp_config` is `Some`, `=`-joined so the CLI's variadic parse cannot swallow the argument after
///    it. The path names the `0600` file [`McpConfigFile`] wrote [`mcp_config`]'s JSON into; the JSON
///    itself never reaches the argv, because its `env` carries `HTUI_MCP_TOKEN` and an argv is readable
///    by every account on the box (blueprint P-2's reason). No `--strict-mcp-config` — the operator's
///    own servers stay — and `--tools` is left as it is, because it never filters MCP tools;
```

`mcp_config`'s doc becomes:

```text
/// The MCP config JSON for `servers` (MOD-11 D8): the CLI's own
/// `{"mcpServers":{<name>:{"type":"stdio","command","args","env"}}}`, serialised compactly.
/// `None` for an empty slice, which is what keeps a session with no server on today's argv.
///
/// It is written to a file ([`McpConfigFile`], MOD-79) and never put on the argv: `env` carries the
/// session's `HTUI_MCP_TOKEN`.
///
/// Both maps are `BTreeMap`s, so the bytes are stable: servers by name, `env` by key.
```

### `CliDriver::io` (`:449`)

The signature becomes `async fn io(&self, spec: &SessionSpec, session_id: &str) -> Result<(ChildIo, Option<McpConfigFile>)>`.
Doc addition: "and the MCP config file its argv names, which the caller must keep for the session's life".

```rust
IoSource::Spawn { .. } => {
    let mut resolved = self.launch_for(spec).await?;
    // MOD-79: written after the launch resolves (a failure there leaves nothing on disk) and
    // before the spawn; a failed spawn drops it here.
    let config = McpConfigFile::write(&spec.mcp).map_err(|err| {
        DriverError::Spawn(format!("cannot write the MCP config: {err}"))
    })?;
    resolved.args = argv(
        &resolved.args, &self.cli_settings(), spec, session_id,
        config.as_ref().map(McpConfigFile::path),
    );
    let spawned = crate::launch::spawn(&resolved, &spec.cwd).await?;
    Ok((ChildIo::from_spawned(spawned)?, config))
}
#[cfg(feature = "test-support")]
IoSource::Prepared(slot) => /* unchanged chain */ .map(|io| (io, None)),
```

The user sees `agent spawn failed: cannot write the MCP config: <io error>` (`DriverError::Spawn`'s Display).

### `start` (`:500-510`)

`let (io, mcp_config) = self.io(&spec, &session_id).await?;` …
`open_session(io, spec, prompt, options, mcp_config).await?`. The `SessionOptions` literal does not change.

### `open_session` (`:814`)

- Add a trailing parameter `mcp_config: Option<McpConfigFile>`.
- The spawn becomes:
  ```rust
  // MOD-79 D3: the config file is the task's, beside the `ChildGuard` `run_session` makes: every
  // exit drops it, the abort in exit (1) included, and a normal end drops it only after
  // `run_session`'s final kill. Captured here rather than passed in, so `run_session` keeps its
  // seven arguments.
  let task = crate::contained::spawn(async move {
      run_session(io, spec, prompt, options, ready_tx, events_tx, commands_rx).await;
      drop(mcp_config);
  });
  ```
- Add a doc paragraph after the three exits: "`mcp_config` is the file the argv names (MOD-79). It moves into the
  task and is removed when the task ends, on all three failing exits as on the session's own end."
- Update the exit-(1) comment at `:843` to say "drops its `ChildGuard` and its MCP config file".

### Test call sites (`tests/cli_driver.rs`)

Add `const CONFIG_PATH: &str = "/run/htui-cli-1-abcd/mcp.json";` near `htui_server()`. Make
`use std::path::Path;` unconditional by dropping the `#[cfg(unix)]` at `:18` (the pure tests now use it on every
platform).

| Line | Test | Pass |
|---|---|---|
| 100 | `argv_is_the_ana4_line_in_order` | `None` |
| 147 | `the_prompt_has_no_place_on_the_command_line` | `None` |
| 171 | `a_resuming_session_names_the_old_id_and_mints_nothing` | `None` |
| 188 | `an_empty_permission_mode_passes_no_flag` | `None` |
| 207 | `a_budget_that_is_absent_or_zero_or_negative_passes_no_flag` | `None` |
| 249 | `narrowed_argv` helper | `None` |
| 430 | `argv_without_mcp_is_unchanged` | `None` |
| 472 | `argv_with_mcp_has_one_joined_config_before_tools` (`args`) | `Some(Path::new(CONFIG_PATH))`; add `assert_eq!(configs[0], &format!("--mcp-config={CONFIG_PATH}"))`. The positional checks stay as they are |
| 473 | same (`without`) | `None` |
| 543 | `the_mcp_config_is_the_clis_stdio_shape` | `Some(Path::new(CONFIG_PATH))`. **Rewrite the tail assertion**: `mcp_configs(&args) == [&format!("--mcp-config={CONFIG_PATH}")]`, and no argv element contains `token-value`, `/run/htui-mcp-1-abcd/s` or `mcpServers`. The JSON-shape assertions on `mcp_config` above it stay. Doc: "...and the argv carries only the file's path (MOD-79)" |
| 556 | `extra_args_stay_last_with_mcp` | `Some(Path::new(CONFIG_PATH))` |
| 582 | `argv_names_the_prompt_tool_only_with_a_port` (`without`) | `Some(Path::new(CONFIG_PATH))`, because the `rest == without` comparison needs the config in both |
| 590 | same (`args`) | `Some(Path::new(CONFIG_PATH))` |
| 1458 | `a_stream_with_no_init_times_out_and_kills_its_child` (`open_session`) | trailing `None` |

### Process tests

New section `// MOD-79: the MCP config travels as a file` at the end of `tests/cli_driver.rs`. Every item is
`#[cfg(unix)]`, matching the file's convention. **No feature is needed**: `cli_driver.rs` has no
`cfg(feature = ...)` gate, and `open_session`, `SessionOptions` and `ChildIo` are plain `pub`.

The script template gains one placeholder. Add `.replace("<COPY_CONFIG>", COPY_CONFIG)` to `script()`. The
config copy reuses the existing `$HTUI_GO_FILE.<suffix>` convention (`LATE_CALL_PROMPT_SCRIPT` already uses it),
so `scripted()` and every other row are left as they are.

```rust
/// MOD-79: copies the file `--mcp-config=` names to `$HTUI_GO_FILE.config`, proving the CLI can
/// read it while it runs.
#[cfg(unix)]
const COPY_CONFIG: &str = concat!(
    "for arg in \"$@\"; do\n",
    "  case \"$arg\" in\n",
    "    --mcp-config=*) cat \"${arg#--mcp-config=}\" > \"$HTUI_GO_FILE.config\" ;;\n",
    "  esac\n",
    "done\n",
);

/// [`TURN_SCRIPT`], after copying its MCP config.
#[cfg(unix)]
const CONFIG_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
<COPY_CONFIG>
printf '%s\n' '<INIT>'
while IFS= read -r line; do
  printf '%s\n' '<REPLY>'
  printf '%s\n' '<RESULT>'
done
exit 0
"#;

/// Copies its MCP config, then exits before `system/init`, as an unauthenticated CLI does.
#[cfg(unix)]
const CONFIG_THEN_EXIT_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
<COPY_CONFIG>
printf '%s\n' 'not logged in' >&2
exit 1
"#;
```

Factor `fn driver(row: &AgentRow) -> Box<dyn AgentDriver>` out of `start_with` (the factory, the registration and
the caps assertion). `start_with` calls it, and test 2 calls it directly.

1. **`the_cli_reads_its_mcp_config_from_a_private_file_and_never_from_its_argv`**
   - Setup: `scripted(tmp, CONFIG_SCRIPT)`, then `spec.mcp = vec![htui_server()]`, then `start_with`, then
     `next(.., "the banner")`. The script records and copies before it prints `init`, so both files exist once
     the banner arrives.
   - From the argv file: exactly one line starts with `--mcp-config`, and it is the `=`-joined form. For **every**
     value in `htui_server().env` (the planted `token-value` and the address), the whole argv text does not
     contain it. The argv text does not contain `mcpServers`.
   - The path: its file name is `mcp.json`, and its parent's name starts `htui-cli-<std::process::id()>-`. The
     parent exists while the session is live.
   - `tmp/go.config` equals `htui_agent::cli::mcp_config(&[htui_server()]).unwrap()` and contains `token-value`.
     This shows the CLI could read the token.
   - `session.cancel(Duration::ZERO)`, then assert that neither the file nor its dir exists. `cancel` joins the
     task, so this is deterministic (H-18).
2. **`a_cli_that_exits_before_init_leaves_no_mcp_config_behind`** (exit 2)
   - `scripted(tmp, CONFIG_THEN_EXIT_SCRIPT)`, then `spec.mcp = vec![htui_server()]`, then
     `let Err(err) = driver(&row).start(spec, "hello".into()).await else { panic!(..) };`. Use `let … else`,
     because `Box<dyn AgentSession>` has no `Debug`.
   - `err` is `DriverError::Transport` and its message contains `system/init`.
   - `go.config` holds `token-value`, so the file existed while the CLI ran.
   - The dir named by the recorded argv does not exist. `open_session`'s exit (2) awaits the task before it
     returns, so this is deterministic.
3. **`an_open_session_that_times_out_drops_its_mcp_config`** (exit 1, the abort)
   - `let config = McpConfigFile::write(&[htui_server()]).expect(..).expect(..)`, then
     `let dir = config.path().parent()…to_path_buf()`, then `assert!(dir.exists())`.
   - `ChildIo { reader, writer, child: None }` over a `tokio::io::duplex` whose agent end is held for the test's
     life, as `:1437-1445` does. A child is not needed to prove the guard drops.
   - The `SessionOptions` literal has `init_timeout: Duration::from_millis(300)`. Call
     `open_session(io, spec(..), "hi".into(), options, Some(config))`.
   - Assert `Err(DriverError::Transport(m))` with `m.contains("system/init")`, then `!dir.exists()`.

**Not tested by a process: the failed spawn in `io()`.** Its directory never reaches an argv the test could
record. Finding it means scanning the base dir for `htui-cli-<pid>-*`, which races every other test in the same
process. Coverage is by construction: the guard is an `io()` local, and the spawn's `?` returns. The reviewer
checks this ordering.

---

## T4: `ResolvedLaunch` `Debug` redaction

`driver.rs:29`: `const REDACTED` → `pub(crate) const REDACTED`. Then `launch.rs:33` imports
`crate::driver::{PermissionPolicy, REDACTED, RedactedEnv}`. One literal for both wrappers, following `RedactedEnv`'s
"two copies would be two places for the invariant to rot".

```rust
/// `ResolvedLaunch::args` for `Debug` (MOD-79): every `--mcp-config` value prints as `[REDACTED]`,
/// so neither a session's config path nor an operator's inline JSON reaches a log line.
///
/// `--mcp-config=<v>` prints as `--mcp-config=[REDACTED]`. After a bare `--mcp-config`, every
/// following argument up to the next `-`-prefixed one prints as `[REDACTED]`; that is how the CLI's
/// variadic flag consumes them. Every other argument prints unchanged, including flags that merely
/// share the prefix (`--mcp-config-x`).
struct RedactedArgs<'a>(&'a [String]);

impl core::fmt::Debug for RedactedArgs<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut list = f.debug_list();
        let mut values = false; // inside a bare `--mcp-config`'s values
        for arg in self.0 {
            if arg == "--mcp-config" {
                values = true;
                list.entry(arg);
            } else if arg.starts_with("--mcp-config=") {
                values = false;
                list.entry(&format!("--mcp-config={REDACTED}"));
            } else if values && !arg.starts_with('-') {
                list.entry(&REDACTED);
            } else {
                values = false;
                list.entry(arg);
            }
        }
        list.finish()
    }
}
```

`ResolvedLaunch`'s `Debug` changes `.field("args", &self.args)` to `.field("args", &RedactedArgs(&self.args))`.
`AgentLaunch`'s `Debug` (`launch.rs:90-99`) is out of scope (requirement 4 names `ResolvedLaunch`). See the note
at the end.

### Tests (`crates/htui-agent/tests/launch.rs`, after `debug_never_prints_an_environment_value`)

Move `ResolvedLaunch` from the `#[cfg(unix)]` import at `:23` into the unconditional `htui_agent::launch::{..}`
list at `:19`. The tests are platform-neutral and build `ResolvedLaunch` literally.

| Test | Input `args` | Asserts |
|---|---|---|
| `debug_redacts_a_joined_mcp_config_value` | `["-p", "--mcp-config={\"mcpServers\":{\"htui\":{\"env\":{\"HTUI_MCP_TOKEN\":\"s3cr3t-token\"}}}}", "--model", "sonnet"]`, then a second case with `--mcp-config=/run/htui-cli-1-abcd/mcp.json` | the output contains `--mcp-config=[REDACTED]`, `"-p"`, `"--model"` and `"sonnet"`, and contains neither `s3cr3t-token`, `mcpServers` nor `/run/htui-cli-1-abcd` |
| `debug_redacts_every_value_of_a_split_mcp_config` | `["--mcp-config", "a.json", "{\"t\":\"s3cr3t\"}", "--model", "x"]`, `env` empty | **exact**: `ResolvedLaunch { command: "claude", args: ["--mcp-config", "[REDACTED]", "[REDACTED]", "--model", "x"], env: {} }` |
| `debug_prints_arguments_without_mcp_config_unchanged` | `["--model", "x", "--add-dir", "/a", "--mcp-config-x", "y"]` | the args part equals `format!("{:?}", args)` |

---

## T5: `docs/htui-mcp.md`

**Replace `:187-191`** (the bullet "The token is visible in `claude-cli`'s command line.") with:

```markdown
- **The token is never on a command line.** For `claude-cli`, htui writes the session's MCP server
  list, token included, to a file `mcp.json` in a new directory `htui-cli-<pid>-<8 hex>`, and starts
  the CLI with `--mcp-config=<path>`, so the process list shows only the path. On Linux and macOS the
  file has mode `0600` and the directory `0700` (htui refuses a directory that is not private), under
  the same base directory as [the socket](#the-socket). On Windows the directory is under your
  temporary directory (usually `%LOCALAPPDATA%\Temp`) and inherits its permissions: your account,
  SYSTEM and Administrators. This path is compiled but not yet exercised on Windows (MOD-16). htui
  removes the file and its directory when the session ends. An ACP agent receives the token over its
  stdin instead.
```

**In "The socket"**, replace the paragraph "The socket and its directory are removed … with that pid runs:" and
the `ls` block with:

````markdown
The socket and its directory are removed when the htui process exits normally, and a `claude-cli`
session's `htui-cli-*` directory when the session ends. A crash (`kill -9`, a power cut) leaves them
behind; nothing sweeps them. A token left in an `htui-cli-*` directory is dead once its htui process
has exited. Both are safe to delete once no htui process with that pid runs:

```
ls -d "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}"/htui-mcp-* "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}"/htui-cli-*
```
````

**In "Troubleshooting"**, after the `htui's MCP listener could not start` entry, add:

```markdown
**A step fails with `agent spawn failed: cannot write the MCP config: …`.** htui could not create
the private directory or the file that hands `claude-cli` its MCP servers. Check that
`$XDG_RUNTIME_DIR` (or the temporary directory) is writable and yours.
```

---

## Hazards

- **H-1 `uuid` `v4` (PC-3).** Without the feature line, `cargo check -p htui-agent` fails on its own, even while
  the workspace build passes through feature unification. Run the standalone check.
- **H-2 Do not use `Uuid::now_v7()` for the suffix.** Its first 8 hex digits are the millisecond timestamp, not
  random bits, so two dirs made in the same millisecond by one pid would collide. Use `new_v4()`, as `channel.rs`
  does.
- **H-3 `SOCKET_NAME` stays.** `sun_path`'s length limit (MOD-11 H-12) concerns the socket only. The config dir's
  path length does not matter, but the socket's one-letter name and both `platform` copies of the constant must
  survive T1. `Listener::close` uses it on every platform.
- **H-4 Unused imports after the move.** `DirBuilderExt` and `PermissionsExt` in `channel.rs`'s unix `platform`
  become unused, and `-D warnings` fails on them. In `private_dir.rs` the unix-only imports and `base_from` must
  be `#[cfg(unix)]`, or the Windows check warns.
- **H-5 `#![warn(missing_docs)]` (`htui-agent/src/lib.rs`).** Every `pub` item in `private_dir.rs` and
  `mcp_file.rs` needs a doc, and each module a `//!`.
- **H-6 `missing_debug_implementations`.** `McpConfigFile` needs its hand-written `Debug` (path only).
  `#[derive(Debug)]` would also print only paths, but write it by hand so the "never the contents" rule is
  visible in the code.
- **H-7 `unsafe_code = "forbid"` on edition 2024.** `std::env::set_var` is `unsafe`, so no test may set
  `XDG_RUNTIME_DIR`. `base_from` exists so the base selection can be tested without it. No `unsafe` anywhere,
  including the Windows body (option A).
- **H-8 Featureless clippy (`cargo clippy --workspace -- -D warnings`).** Every task's commit must be green on
  its own. T2 must add the `pub use mcp_file::McpConfigFile;` re-export in the same commit as the module.
  Otherwise the type is dead until T3 wires it in. Do not add private helpers that only tests call.
- **H-9 Windows cross-check.** `CC=gcc AR=ar cargo check -p htui-mcp -p htui-agent --target x86_64-pc-windows-gnu`
  compiles the `cfg(not(unix))` bodies of `private_dir::create`/`base` and `McpConfigFile::write`. Every
  `std::os::unix` path must sit inside a `#[cfg(unix)]` block. The target must be added first
  (`rustup target add`), because the sandbox does not have it.
- **H-10 Guard before write.** `McpConfigFile` is constructed right after the directory exists, before the UTF-8
  check and the file write, so every later `?` removes the directory. Use `create_new(true)` so an existing file
  is never reused.
- **H-11 No `let _ = mcp_config;`.** That pattern drops at once. The guard is captured by the spawned future and
  dropped explicitly after `run_session(..).await`.
- **H-12 `run_session` arity (PC-4).** Do not add an 8th parameter: `clippy::too_many_arguments` fires above 7.
  The wrapper future in `open_session` is the design.
- **H-13 `io()` order.** Run `launch_for` first, then write, then `argv`, then spawn. Writing before `launch_for`
  would leave a file behind for a launch that cannot resolve (it is still dropped, but it is wasted I/O). The
  `Prepared` arm returns `None`.
- **H-14 `unused_qualifications`.** Do not write `std::path::Path::new` where `Path` is imported. In
  `tests/cli_driver.rs`, make the `Path` import unconditional instead of qualifying.
- **H-15 `tests/launch.rs` import.** `ResolvedLaunch` is imported only under `cfg(unix)` today. Move it to the
  unconditional list; leaving both lines would be a duplicate import.
- **H-16 `private_intra_doc_links = "deny"`.** `argv`'s and `mcp_config`'s docs link `[`McpConfigFile`]`. That
  link resolves only through the `pub use` (G-2). Run `cargo doc -p htui-agent --no-deps`.
- **H-17 The `RECORD` preamble writes one argv element per line.** The config path has no newline, so
  splitting on lines is exact. Read the argv and copy files only after the banner arrives (the script writes them
  before `init`).
- **H-18 Natural-end race.** On a session's own end, the event channel closes when `session_main` returns. That
  is before `run_session`'s kill and before the guard drops, so `drain()` returning `None` does not yet mean the
  dir is gone. Assert removal after `cancel` (it joins the task) or after `start`'s `Err` (exit (2) awaits the
  task). If a natural-end assertion is ever added, poll with a deadline.
- **H-19 Exit (2)'s bounded await.** `open_session` awaits the finished task with `timeout(init_timeout, task)`.
  If that ever timed out, the task, and with it the guard, would live on until the child ends, the same as the
  child itself. That is accepted, not a leak.
- **H-20 Windows delete-while-open.** On Windows `remove_file` fails while another process holds the file open
  without `FILE_SHARE_DELETE`. On the abort path the kill has been signalled but not reaped, so the dir can leak
  there. Best effort, as option A accepts; MOD-16 owns Windows runtime behaviour.
- **H-21 Do not touch `acp::open_session` or `tests/acp_driver.rs`.** It is a different function with the same
  name. The ACP path is unchanged (requirement 5).
- **H-22 MOD-78 overlap.** MOD-78 edits `htui-mcp` session/host. T1 changes only `channel.rs`'s unix `platform`
  module, so keep the diff there.
- **H-23 `mcp_config` stays `pub` and byte-identical.** `the_mcp_config_is_the_clis_stdio_shape` still pins its
  JSON, and `McpConfigFile::write` writes exactly its output.

## Data flow

1. `CliDriver::start` mints the session id.
2. `io()` resolves the launch and calls `McpConfigFile::write(&spec.mcp)`. That creates
   `<base>/htui-cli-<pid>-<8 hex>/` (`0700`) and writes `mcp.json` (`0600`) with the token.
3. `argv(.., Some(path))` pushes `--mcp-config=<path>`, then the child is spawned. A failed spawn drops the guard
   in `io()`.
4. `open_session(io, .., Some(guard))` moves the guard into the session task's future.
5. The CLI reads the file at start-up, and its relay child reads `HTUI_MCP_TOKEN` from the env the file
   declared.
6. On cancel, EOF or a handle drop, `run_session` kills the child and returns, and the guard drops: the file,
   then the dir. On the timeout abort, the dropped future drops it. On EOF before `init`, the awaited task drops it.
7. Separately, `ResolvedLaunch`'s `Debug` prints any `--mcp-config` value as `[REDACTED]`.

## Build order and lanes

| Lane | Tasks (serial within a lane) | File set |
|---|---|---|
| A (one implementer) | T1 → T2 → T3 | `crates/htui-agent/Cargo.toml`, `crates/htui-agent/src/private_dir.rs`, `crates/htui-agent/src/lib.rs`, `crates/htui-mcp/src/channel.rs`, `crates/htui-agent/src/cli/mcp_file.rs`, `crates/htui-agent/src/cli/mod.rs`, `crates/htui-agent/tests/cli_driver.rs` |
| B (parallel) | T4 | `crates/htui-agent/src/driver.rs`, `crates/htui-agent/src/launch.rs`, `crates/htui-agent/tests/launch.rs` |
| C (parallel) | T5 | `docs/htui-mcp.md` |

The three file sets are pairwise disjoint (B gained `driver.rs`, per PC-2; A gained `Cargo.toml`, per PC-3). Lane A
is serial because T2 and T3 share `cli/mod.rs` and each task builds on the one before. Commit after each task,
with explicit-path staging.

Hidden coupling to watch: none of the `.sqlx` files, snapshots or seeds move. Lanes A and B both compile
`htui-agent`, so a gate run while the other lane is mid-edit sees the other's half-done state. Verify the final
gate on the merged tree.

## Validation (the plan's, plus three lines)

```bash
cargo fmt --all --check
cargo check -p htui-agent                                   # H-1: no feature unification
cargo test -p htui-agent --features test-support --no-fail-fast -- --test-threads=1
cargo test -p htui-mcp --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                     # H-8
cargo doc -p htui-agent --no-deps                           # H-16
rustup target add x86_64-pc-windows-gnu
CC=gcc AR=ar cargo check -p htui-mcp -p htui-agent --target x86_64-pc-windows-gnu   # H-9
```

Live probe (plan): `/proc/<pid>/cmdline` of a real `claude-cli` session holds `--mcp-config=<…>/htui-cli-…/mcp.json`
and no token. After the session ends, the directory is gone.

## Note for the maintainer (not in scope, not designed)

`CliSettings` derives `Debug`, and `AgentLaunch`'s `Debug` prints `args` as they are. An operator's own inline
`--mcp-config=<json>` in `settings.cli.extra_args` or in the row's `launch.args` therefore still reaches those
types' `Debug`. Requirement 4 covers only `ResolvedLaunch`. If the same rule is wanted there, `RedactedArgs` is
reusable as written.
