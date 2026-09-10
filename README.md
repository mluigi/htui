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
| `J` / `K` | Scroll the detail pane one row down / up — in **Runs**, move the cursor over the runs and their steps |
| `PageDown` / `PageUp` | Scroll the detail pane ten rows |
| `Enter` (in **Runs**) | Replay the selected step in the Chat tab, read-only |

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
| `Esc` `Esc` | End the session (the first `Esc` arms it; any other key, and opening a replay, disarms) |
| `t` | Fold or unfold the agent's thoughts |
| `j` / `k` / `g` / `G` | Scroll the transcript |

The header names the agent, the model, the project and the agent-side session id. A banner under
it lists what the session **cannot** do — permission requests, edit proposals, plans — whenever
the transport reports less than the full profile; it is computed from the driver's own
capabilities, not from `agent.transport`.

A chat can be started while the shell is offline. Its events go to a JSON-lines buffer under
`cache/<fingerprint>/pending/`, the header says `buffered · uploads when the store returns`, and
the next successful connection inserts the run, its step and every event in one transaction.
Ending the app cancels every live session and waits for its process tree to die before the process
exits.

**Replay.** `Enter` on a step in the Backlog tab's Runs pane reopens that step's recorded log over
the Chat tab, rendered by the same transcript the live view uses (`R-HIS-2`). It is read-only: the
tab sends nothing while it is open, `1`..`9` answer nothing, and `Esc` closes it and puts the live
conversation back exactly as it was. A step this box has never synced says so rather than reading
as a conversation that said nothing.

| Key | Action (while a replay is open) |
|---|---|
| `Esc` | Leave the replay |
| `t` | Fold or unfold the thoughts |
| `j` / `k` | Scroll the replayed transcript |

Two environment knobs, both off by default:

| Variable | Effect |
|---|---|
| `HTUI_KEEP_RAW_EVENTS=1` | Keep the verbatim wire message on every recorded row. `project.settings.keep_raw_events` replaces this once the project editor exists |
| `HTUI_TOOL_<NAME>` | Override one `${name}` placeholder of `agent.launch` — `HTUI_TOOL_NODE`, `HTUI_TOOL_CLAUDE_AGENT_ACP`, … — for a box whose layout the built-in resolver does not understand |

## Installing an agent's adapter

`claude` reaches ACP through an npm adapter the probe can find on its own. `agy` does not: Google
ships a separate first-party server, `agy_acp_server`, that is not on `PATH` and not installed by
the `agy` CLI. Since `R-AGT-10` `htui` installs it for you. In **Settings > Agents**, `j`/`k` pick
a row and `i` installs it. There is no code path per agent: the source is the row's own
`agent.launch.discovery.install`, which names an ACP registry id, and the installer writes where
that row's glob already looks (`R-AGT-5`). A row that declares no source says so instead.

`i` spends one registry read and one `HEAD`, then draws a consent pane — the agent and the version,
the archive's URL and its size, the directory it unpacks into, the licence and its terms URL,
whether the digest will be verified or cannot be, whether the disk holds it, and which installed
versions this one replaces. `y` accepts, `n` / `Esc` declines, `x` stops a running install.
**Nothing is downloaded before `y`.** Where the registry publishes a `sha256` it is checked before
anything is unpacked and a mismatch refuses the install; `antigravity-acp` publishes none, which
the pane says before the download, so `htui` records the digest of what it actually received and a
later re-install of the same version that differs is detectable. `agy_acp_server` is proprietary,
which is why the terms are on screen before a byte is fetched; consent is remembered per box in
`<install root>/<id>/manifest.json`.

The install root is `HTUI_AGENTS_ROOT`, seeded from this platform's local data directory:
`~/.local/share/htui/agents` on Linux (`$XDG_DATA_HOME` when it is set),
`~/Library/Application Support/htui/agents` on macOS, `%LOCALAPPDATA%\htui\agents` on Windows.
Setting the variable moves the tree, which is the knob for a box that wants 1.9 GB per version
somewhere else; `HTUI_TOOL_<NAME>` still wins over anything installed, for a layout no pattern
describes. Afterwards `htui` **re-probes**, and the `on this box` cell is the probe's answer rather
than the installer's claim: a tree that unpacked perfectly but cannot handshake reads `failed`.

Offline, behind a proxy that refuses, or against a registry that will not answer, the action
degrades to manual steps derived from the same row — the registry URL, the entry id, this box's
platform key, the directory to unpack the whole archive into, the file to make executable, and the
`HTUI_TOOL_<NAME>` override. Three things hold whichever way the adapter arrives. Unpack the
**whole** archive rather than the server alone: `localharness_external` ships beside it, and though
the handshake does not need it (verified) a live turn may. **The executable bit matters** —
`launch::spawn` runs `which` even on an absolute path, which rejects a file without it. And **the
Linux argument is mandatory**: `agent.launch` appends `--uid=` on Linux with its value deliberately
empty; without it the server's startup path tries to drop privileges to a `nobody` group and aborts
(`Check failed: LookupGIDByGroupName(…)`) before it reads a byte of stdin. The probe applies the
append, which is why `Settings > r` is what makes a chat launch a working `agy`.

## Logging an agent in

An installed adapter is not a usable one. `agy_acp_server` keeps its credential under
`$GEMINI_HOME/antigravity-acp/` (default `~/.gemini/antigravity-acp/`) — a *sibling of, and
separate from*, the `agy` CLI's own directory, so being logged into the CLI does not log in the
adapter, and until that directory holds a token the probe records `unauthenticated`.

Since `R-AGT-9` that state is actionable from the app. In **Settings > Agents**, `a` on the
highlighted row starts a login. The agent is asked which methods it accepts and answers with its
own words — `agy` offers "Log in with Google", "Log in with Gemini Enterprise", "Gemini API key"
and "Gemini Enterprise Agent Platform" — `j`/`k` and `Enter` choose one, `o` opens the link the
agent prints, and `x` cancels. Where the agent advertises logging out, the chooser offers that too.
No agent name and no method id appears anywhere in the code: the list comes off the wire
(`R-AGT-5`).

**`htui` never reads, holds or stores the credential.** It triggers the agent's own flow and
watches what happens; the token is written by the vendor, where the vendor keeps it. What the app
records afterwards is a **re-probe**, so the `on this box` cell is the probe's verdict and not the
login's claim — a flow that reported success into a box holding no credential still reads
`unauthenticated`. Authentication is a fact about a box, never about a registry row: the `agent`
row is byte-identical before and after.

Two of the four methods are not browser flows at all. `gemini-api-key` and `agent-platform` want a
variable in the environment the adapter is *launched from*, and the agent says so itself
(`The GEMINI_API_KEY environment variable must be set…`). `htui` relays that rather than hiding the
method; injecting the value is the secret provider's job (`R-SEC-1..4`, MOD-10), not this flow's.

A login is paced by a human, so it has no timeout — only `x`, and a cap on **silence** that ends a
flow nobody is watching. While it waits, the rest of the app keeps working: the flow runs off the
worker loop like every other long operation (`R-NF-3`).

One thing the app cannot fix for you yet. The agent opens its own redirect listener on
**this box's** loopback, on a fresh port per attempt. If you run `htui` on a server, the link your
browser opens redirects to `127.0.0.1` on *your* machine, where nothing is listening. Until MOD-22
lands, finish the flow by copying the failed `http://127.0.0.1:<port>/?code=…&state=…` out of the
address bar and re-issuing it on the box:

```bash
curl -s "http://127.0.0.1:<port>/?code=…&state=…"
```

The adapter takes the code from its own loopback and the login completes. Forwarding the port
(`ssh -L`) is the other route and works on some setups; the paste-back works on all of them.

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

### Checking the Windows-only code from Linux

`htui-agent` is the only crate with `#[cfg(windows)]` code — the job object, `CREATE_NO_WINDOW`
and the `PATHEXT`-aware command lookup of `launch.rs`. On a Linux box that code is never
compiled by an ordinary build, so it is checked against the Windows target instead:

```bash
rustup target add x86_64-pc-windows-msvc
cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings
```

No linker is involved, so this needs nothing but the target's standard library. It compiles and
lints every Windows branch, and it has already caught two things a Linux build cannot see (an
enum whose variants are lopsided only on Windows, and a binding used only under `cfg(unix)`).

**It proves the code builds, not that it behaves.** The job object's kill-on-close guarantee, the
`.cmd` shim that `CreateProcess` refuses, and `CREATE_NO_WINDOW` are runtime facts about Windows
and are verified by running the suite there — `cargo test -p htui-agent` plus the `#[ignore]` live
tests. The rest of the workspace does not cross-check from Linux: `sqlx`'s `ring` dependency
builds C code and wants an MSVC-compatible compiler.

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
