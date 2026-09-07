# htui

`htui` is a keyboard-driven terminal UI for a single developer's cross-box workflow store: it opens
on a **workspace**, lists that workspace's items grouped by project, and shows the selected item's
body, runs, link graph, documents and notes side by side. The product contract it implements is
`docs/REQUIREMENTS.md` (`R-TUI-1..3` for the surfaces, `R-NF-1` for the platforms, `R-NF-3` for
"the UI never blocks"), and the data it shows is shaped by `docs/ANA-9.md`, the concluded data
model: every view reads through the ANA-9 §6.1 store seam (`ReadStore` / `WriteStore` / `Backend`)
rather than through a database handle, so the in-memory store of MOD-1, the Postgres store and the
per-box SQLite cache all sit behind the same seam without a single view changing.

The store lives on **Postgres 16 or newer** (the schema uses `UNIQUE NULLS NOT DISTINCT`, a
Postgres 15 feature; 16 is the floor the suite runs against). A mirror of what you browse is kept
in a local SQLite file, so `htui` starts and renders with the server unreachable.

## Build

Requires **rustc 1.98+** (edition 2024; `rust-toolchain.toml` pins 1.98.1 exactly, so `rustup` will
fetch it on first build).

`workspace.package.rust-version` says `1.98` too, and the two numbers are deliberately the same.
The declared MSRV had been `1.85`, which was never true: the locked `sqlx-core 0.9.0` declares
`rust-version = "1.94.0"` and `ratatui 0.30.2` declares `1.88.0`, so nothing below 1.94 has been
able to build this workspace for some time. `docs/ANA-4.md` §4.2 proposed `1.88` (the ACP SDK's own
floor); MOD-2 raised it to the toolchain pin instead, on the reasoning that with an *exact* pin the
only MSRV that is simultaneously true and checkable is the one everyone actually runs. `clippy.toml`
carries the same number, or clippy silently lints against the older one.

```
cargo build --release
```

The binary lands in `target/release/htui` (`target/release/htui.exe` on Windows).

## Run

```
cargo run -p htui -- --demo
```

| Flag | Meaning |
|---|---|
| `--demo` | Load the deterministic demo fixture (two workspaces, three projects, items, runs, notes and documents) into memory instead of connecting. |
| `--offline` | Open from the local cache and never attempt a connection. |
| `--set-dsn` | Read a Postgres DSN from stdin, store it in the OS keyring and exit. |
| `--clear-dsn` | Remove the stored DSN from the OS keyring and exit. |
| `--log <PATH>` | Append `tracing` output to a file. Never stdout — stdout is the TUI. Also read from `HTUI_LOG`; `HTUI_LOG_FILTER` overrides the default `info` level. |
| `--help`, `--version` | Print usage / version and exit. |

With `--demo` the shell enters the first workspace by name (`Graphics`) and opens on the Backlog
tab. With an empty store nothing can be entered, so the workspace switcher stays up over the shell
reading `no workspaces` — creating one is MOD-15.

## The database connection

### Storing the DSN

The DSN lives in the **OS keyring** — Windows Credential Manager, macOS Keychain, the Linux secret
service — under service `htui`, user `postgres-dsn`. There is no environment-variable fallback and
no configuration file for it (`R-STO-1`), so the DSN never reaches `argv` or a shell history.

```
htui --set-dsn        # then paste the DSN and press Enter
htui --clear-dsn      # removes the entry again
```

Both flags exit before the TUI starts. The DSN is read from stdin, so it can also be piped in:

```
echo postgres://postgres:htui@localhost:5439/htui | htui --set-dsn
```

TLS is whatever the DSN asks for (`?sslmode=require`, …); nothing overrides it (`R-STO-2`).

### A development server

`compose.yaml` at the repo root runs the minimum supported server, `postgres:16`, on host port
**5439**, so a locally installed Postgres on 5432 never collides:

```
docker compose up -d
echo postgres://postgres:htui@localhost:5439/htui | htui --set-dsn
```

The container's `htui` database is the one to point the TUI at; `postgres` is the maintenance
database the test suite creates its throwaway databases through.

### Startup, offline mode and the cache

`htui` opens the local mirror first and renders from it immediately, then connects on a background
task (ANA-9 §4.4): the UI never waits for the network. The store field of the top bar says which
state it is in.

| Top bar | Meaning |
|---|---|
| `memory` | `--demo`: an in-process store, no server and no cache. |
| `connecting` | The first connection attempt has not answered yet. |
| `online` | Connected. Reads go to Postgres and the cursor pass keeps the mirror warm in the background. |
| `offline · 3m` | No connection for three minutes (`s` / `m` / `h`). Reads come from the mirror and there is no write path at all. |

An offline shell retries every 30 seconds and switches to `online` on its own. `--offline` skips
connecting entirely, which is the way to look at the cache on purpose.

The cache lives next to the box identity under the user's configuration directory —
`%APPDATA%\htui` on Windows, `~/.config/htui` on Linux, `~/Library/Application Support/htui` on
macOS:

```
%APPDATA%\htui\box.toml                                # this box's id (UUIDv7) and its hostname
%APPDATA%\htui\cache\<fingerprint>\cache.sqlite        # the mirror, one per server
%APPDATA%\htui\cache\<fingerprint>\pending\*.jsonl     # offline chat buffers, uploaded on connect
```

`<fingerprint>` is `sha256(host:port/dbname)` and never contains credentials, so pointing `htui` at
a second server gives it a second mirror rather than a mixed one. `box.toml` is minted on first
launch and its id survives a hostname change (`R-BOX-4`). Deleting `cache/` is safe: it is refilled
from the server, and it is rebuilt automatically when the server's schema version changes.

### The migration prompt

A schema that is behind this binary is **never migrated without being asked** (`R-STO-5`). When the
connected database is behind, a modal box appears:

```
3 schema migrations are pending. Apply them now?

y apply · n / Esc stay offline
```

`y` applies them and goes online; `n` and `Esc` leave the database untouched and the shell reading
from the cache. The question is asked once per session. A database whose schema is *newer* than
this binary, or whose applied migrations have different checksums, is refused outright: the top bar
stays offline and the reason is on the status line.

## Keys

Keys are a table, not a `match`: a key reaches the topmost overlay first, then the overlay's
bindings, then the active tab, then the tab's bindings, then the global table.

### Global

| Key | Action |
|---|---|
| `q` | Quit |
| `Tab` / `Shift+Tab` | Next / previous tab |
| `1` … `9` | Select a tab by position (`1` Backlog, `2` Skills, `3` Settings) |
| `w` | Open the workspace switcher |
| `?` | Toggle the key help box |

### Workspace switcher (overlay)

| Key | Action |
|---|---|
| `j` / `Down` | Next workspace |
| `k` / `Up` | Previous workspace |
| `Enter` | Enter the selected workspace (changes the scope) |
| `Esc` | Close the overlay |

`Esc` closes any overlay; the switcher is modal, so a key it does not handle never reaches the tab
underneath.

### Schema prompt (overlay)

| Key | Action |
|---|---|
| `y` | Apply the pending migrations and go online |
| `n` / `Esc` | Leave the database untouched and stay on the cache |

The prompt has no key that opens it: it is opened by the shell when the connected database reports
pending migrations, and never by the user.

### Backlog tab

| Key | Action |
|---|---|
| `j` / `Down` | Next row |
| `k` / `Up` | Previous row |
| `g` / `Home` | First row |
| `G` / `End` | Last row |
| `Enter` | Fold or unfold the project group, when the cursor is on a project header |
| `l` / `]` / `Right` | Next detail sub-tab |
| `h` / `[` / `Left` | Previous detail sub-tab |
| `J` / `K` | Scroll the detail pane one row down / up |
| `PageDown` / `PageUp` | Scroll the detail pane ten rows |

The five detail sub-tabs are **Body**, **Runs**, **Graph**, **Documents** and **Notes**
(`R-TUI-3`). The Skills tab is still a placeholder; the Settings tab lists the agent registry.

### Chat tab

A live conversation with an agent (`R-TUI-6`). The first prompt starts a session, records it
against a `chat` run, and streams what the agent does back into the transcript.

| Key | Action |
|---|---|
| `i` / `Enter` | Compose; `Enter` sends, `Esc` leaves the composer |
| `a` | Next enabled agent, before the first prompt |
| `1`..`9` | Answer the permission request the agent is waiting on |
| `Esc` `Esc` | End the session (the first `Esc` arms it, any other key disarms) |
| `t` | Fold or unfold the agent's thoughts |
| `j` / `k` / `g` / `G` | Scroll the transcript |

The header names the agent, the model, the project and the agent-side session id. A banner under
it lists what the session **cannot** do — permission requests, edit proposals, plans — whenever
the transport reports less than the full profile; it is computed from the driver's own
capabilities, not from `agent.transport`.

A chat needs a writable store: while the shell is offline it refuses to start one, because the
offline event buffer arrives in a later milestone. Ending the app cancels every live session and
waits for its process tree to die before the process exits.

Two environment knobs, both off by default:

| Variable | Effect |
|---|---|
| `HTUI_KEEP_RAW_EVENTS=1` | Keep the verbatim wire message on every recorded row. `project.settings.keep_raw_events` replaces this once the project editor exists |
| `HTUI_TOOL_<NAME>` | Override one `${name}` placeholder of `agent.launch` — `HTUI_TOOL_NODE`, `HTUI_TOOL_CLAUDE_AGENT_ACP`, … — for a box whose layout the built-in resolver does not understand |

## Platforms

`R-NF-1` is Windows 10+, Linux and macOS, and nothing in the crate is platform-specific: the
terminal layer is `crossterm` and the drawing layer is `ratatui`.

- **Windows**: **Windows Terminal** is the target and the only configuration the rendering is
  tuned for — it has the Unicode box-drawing characters, the middle dot in the top bar and the
  colour depth the theme assumes.
- **Legacy `conhost.exe`** (the old console host, and `cmd.exe` windows opened outside Windows
  Terminal) is **best-effort**: it runs, but box-drawing and the `·` separator depend on the
  console code page and the font, so expect replacement characters with a raster font.
- **Linux / macOS**: any `xterm`-compatible terminal with UTF-8. Not built on either in MOD-1
  (this box has only the Windows target installed); the crate uses only `crossterm` and
  `ratatui`, so nothing is expected to be platform-specific, but it is unverified.

The terminal is restored on every exit path, panics included: a panic hook runs `ratatui::restore()`
before the default hook prints, and the terminal guard restores again on drop.

## Tests

```
cargo test --workspace --all-features
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

### Tests that need a server

The Postgres and cache suites read **`HTUI_TEST_DATABASE_URL`**, a *maintenance* DSN whose user may
`CREATEDB`. Each test creates its own `htui_test_<hex>` database, migrates it and drops it on its
last line. With the variable unset every such test prints `skipped: HTUI_TEST_DATABASE_URL not set`
and passes, so the command above stays green on a box without a server.

```
docker compose up -d
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test --workspace --all-features
```

Nothing under `%APPDATA%\htui` is touched by the suite: every test mints its `box.toml` and opens
its mirror in a throwaway directory, and the keyring tests use their own `htui-test-<pid>` service
rather than the real entry. A test process killed mid-run can leave a database behind; they all
carry the `htui_test_` prefix, so `DROP DATABASE htui_test_…` is the cleanup.

### `sqlx` offline query data

Postgres queries are checked at compile time against the **committed** `crates/htui-store/.sqlx/`
data, with `SQLX_OFFLINE=true` in `.cargo/config.toml`, so `cargo build`, `clippy` and `doc` never
need a server. After adding or changing a `query!`, regenerate it **from inside the crate** —
`cargo sqlx prepare` has no `-p` flag and writes to the manifest directory:

```
cargo install sqlx-cli --no-default-features --features postgres,sqlite   # once
psql postgres://postgres:htui@localhost:5439/postgres -c "CREATE DATABASE htui_sqlx;"
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx migrate run --source crates/htui-store/migrations

cd crates/htui-store
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features
```

`--all-targets --all-features` is not optional: without it the queries inside `#[cfg(test)]`,
inside `tests/*.rs` and behind the `demo` feature are garbage-collected out of the cache.
`cargo sqlx prepare --check` is the CI form. The SQLite mirror is checked at run time instead and
contributes no files.

The UI tests are `insta` snapshots rendered against a `TestBackend` at 100x30 through
`htui::testkit::Harness`, which serves store requests inline: no sleeps, no spawned worker, so a
snapshot is byte-stable. Snapshots live next to their tests in `crates/htui/tests/snapshots/` and
`crates/htui/src/snapshots/`. When a change moves a rendering on purpose, re-record with
`INSTA_UPDATE=always cargo test --workspace --all-features`, then run the suite again without the
variable to confirm the recorded snapshots match, and commit the `.snap` files.

## Scope

MOD-1 was the **scaffold**: the shell, the key table, the tab and overlay registries, the store
worker seam and read-only views over an in-memory store loaded with demo fixtures.

**MOD-6** adds the real store: `crates/htui-store` with the ANA-9 §5 Postgres schema and its
backend (`PgStore`), the per-box SQLite mirror (`CacheStore`) with the background cursor refresh,
the box identity, the keyring DSN and the `Backend` enum the store worker holds. Item editing is
still MOD-13's: the write paths exist on `PgStore` and no view calls them yet.

**MOD-2** is landing `crates/htui-agent`: the `AgentDriver` / `AgentSession` seam of
`docs/ANA-4.md` §4.1, the driver event model, the session recorder (coalescing, `seq`/`turn`,
scrub-before-persist), a `FakeDriver` and one transport-neutral conformance list every transport
must pass. Milestone 2 adds the launch recipe — `agent.launch` / `agent.settings` as types,
`${tool}` resolution against a per-box tool map, and a supervised spawn (job object on Windows,
process group on unix) — plus the agent registry section of the Settings tab. No wire protocol
yet: `claude` over ACP is milestone 3, so the crate holds the SDK but starts no session.

The rest is tracked as its own item and lands as an additive module — a file plus one registration
line, with no change to the event loop:

| Deferred to | What |
|---|---|
| **MOD-13** | Filters and item editing (new, edit, close) |
| **MOD-14** | Navigable Graph traversal, re-rooting and multi-hop |
| **MOD-15** | Hierarchy management: creating workspaces, projects, repos and item kinds |
| **MOD-4** | Run actions: run, approve, reject, retry, cancel |
| **MOD-2** | The Chat tab |

The TUI scope is always a workspace (a single project still lives in one), and the store seam is
`docs/ANA-9.md` §6.1 — read it before changing a signature there.
