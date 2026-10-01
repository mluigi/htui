# htui

`htui` is a keyboard-driven terminal app for running a backlog of work through AI coding agents.
It keeps your work items, their documents, the runs agents performed and full chat transcripts in
one Postgres database, and lets you browse and drive all of it from the terminal, across several
projects, repositories and machines.

- **One place for everything.** Items, documents, notes, runs, requirements, skills and prompt
  templates live in Postgres. Credentials live in your OS keyring, never in a config file.
- **Works offline.** A local copy of what you browse is kept on each machine, so `htui` opens
  instantly and stays readable when the server is out of reach.
- **Talks to real agents.** Chat with Claude Code or Google's Antigravity (`agy`) from inside the
  app, and replay any recorded conversation later.
- **Drives runs step by step.** Start a run on an item, approve or reject each step, retry, cancel
  and close out, all from the Backlog.
- **Never blocks.** Network, database and agent work happen in the background, so the UI stays
  responsive.

It runs on Windows 10+, Linux and macOS.

## Contents

- [Quick start](#quick-start)
- [Building from source](#building-from-source)
- [Connecting to your database](#connecting-to-your-database)
- [Using htui](#using-htui)
- [Setting up agents](#setting-up-agents)
- [Search](#search)
- [Where htui keeps its files](#where-htui-keeps-its-files)
- [Troubleshooting](#troubleshooting)
- [Development](#development)
- [Further reading](#further-reading)

## Quick start

Try it without a database, using built-in sample data:

```
cargo run -p htui -- --demo
```

The demo opens on the **Graphics** workspace with a few projects, items, runs, notes and
documents to look around. Press `?` at any time to see the keys for the screen you are on, and `q`
to quit.

## Building from source

You need:

- **Rust 1.98.1.** The exact version is pinned in `rust-toolchain.toml`, so `rustup` downloads it
  on your first build.
- **On Linux:** the D-Bus development files and `pkg-config`, which the keyring integration builds
  against (`sudo apt install libdbus-1-dev pkg-config` on Debian and Ubuntu).
- **Network access on first use of the concepts index.** Nothing is downloaded while building.
  The embedding model (about 134 MB) is downloaded into your user cache directory the first time
  you index or search, and checked against a pinned hash. An existing download from an earlier
  htui is reused.
- **`git`** on your `PATH`, for running work in repositories.

Then build:

```
cargo build --release
```

The binary is `target/release/htui` (`target/release/htui.exe` on Windows).

### Toolchain

`workspace.package.rust-version` in `Cargo.toml` and `clippy.toml` both say `1.98`, matching the
pinned toolchain on purpose: with an exact pin, the only minimum version that is both true and
checkable is the one everybody actually builds with. Keep all three in step when you move the pin.

### Command-line options

| Option | What it does |
|---|---|
| `--demo` | Start with the built-in sample data in memory instead of connecting to a database. |
| `--offline` | Open from the local copy only and never try to connect. |
| `--set-dsn` | Read a Postgres connection string from standard input, save it in the OS keyring and exit. |
| `--clear-dsn` | Remove the saved connection string from the OS keyring and exit. |
| `--dsn-stdin` | Read the connection string for this session from standard input; it is never stored. For a TUI over `ssh -t` on a machine with no keyring. |
| `--index-items` | Update the search index from the database, then exit. See [Search](#search). |
| `--search-items <QUERY>` | Search items, their documents and requirements, print the results and exit. |
| `--project <SLUG>` | With `--index-items` or `--search-items`: only this project. |
| `--decisions` | With `--search-items`: only decisions, meaning items closed as done, concluded or rejected, and their documents. |
| `--limit <N>` | With `--search-items`: how many results to print, from 1 to 1000 (default 10). |
| `--log <PATH>` | Append logs to a file. Also read from `HTUI_LOG`; `HTUI_LOG_FILTER` changes the level (default `info`). Logs never go to the terminal, since the terminal is the app (`htui worker`, which has none, logs to standard error without it). |
| `worker [--pool-size N] [--dsn-stdin] [--log PATH]` | Run this machine's runs with no terminal, so they outlive the TUI: `htui worker`. Needs the box's executor set to `worker` in **Settings › Boxes**; no other option may be given beside it, and `--log` goes after `worker`. See [`docs/htui-worker.md`](docs/htui-worker.md). |
| `provision <DESTINATION> [--dsn-stdin] [--replace-credential] [--log PATH]` | Install this build as the `htui-worker` service on another Linux machine over `ssh`, with the DSN encrypted there by `systemd-creds`, and set that box's executor to `worker`. See [`docs/htui-worker.md`](docs/htui-worker.md#provisioning-a-remote-box). |
| `--help`, `--version` | Print usage or the version and exit. |

## Connecting to your database

`htui` needs **Postgres 16 or newer**.

### Run a local server

The repository includes a `compose.yaml` that starts Postgres 16 on port **5439**, so it does not
clash with a Postgres you may already have on 5432:

```
docker compose up -d
echo postgres://postgres:htui@localhost:5439/htui | htui --set-dsn
```

The same file also starts Qdrant, which is only needed for [Search](#search).

### Save your connection string

The connection string (DSN) is stored in your **OS keyring**: Windows Credential Manager, the macOS
Keychain, or the Secret Service on Linux. There is no environment variable or config file for it,
so it never ends up in your shell history or process list.

You can save it in either of two ways:

- **From inside the app:** open **Settings › Connection** and press `e`. The input is masked. Press
  `Enter` to save, and `htui` connects straight away without a restart. A machine with no saved
  connection opens on this screen automatically.
- **From the command line:** run `htui --set-dsn`, paste the DSN and press `Enter`. You can also
  pipe it in, as in the example above. `htui --clear-dsn` removes it again.

TLS follows whatever your DSN asks for, for example `?sslmode=require`.

### Online and offline

`htui` opens from its local copy first and connects in the background. The top bar shows where it
stands:

| Top bar | Meaning |
|---|---|
| `memory` | Running with `--demo`. No server and no local copy. |
| `connecting` | The first connection attempt has not answered yet. |
| `online` | Connected. The local copy is kept up to date in the background. |
| `offline · 3m` | No connection for three minutes. You can browse what was copied locally, but nothing can be changed and no chat or run can start. |

While offline, `htui` retries every 30 seconds and goes back online on its own. Use `--offline` to
look at the local copy on purpose.

### Database upgrades

When a new version of `htui` needs to upgrade your database, it asks first and never upgrades on
its own:

```
3 schema migrations are pending. Apply them now?

y apply · n / Esc stay offline
```

Press `y` to upgrade and go online, or `n` / `Esc` to leave the database untouched and keep
browsing the local copy. If the database is *newer* than your copy of `htui`, `htui` stays offline
and says why on the status line; update `htui` to connect.

## Using htui

Everything happens in a **workspace**. Press `w` to switch workspaces. If the database has none
yet, create one in **Settings › Hierarchy** with `N`.

`htui` has five tabs. Switch with `Tab` / `Shift+Tab` or the number keys `1` to `5`.

| Tab | What it is for |
|---|---|
| **1 Backlog** | The workspace's items, grouped by project, with details on the right. This is also where you drive runs. |
| **2 Skills** | Your skill library and prompt templates: browse versions, compare them, edit, import, and attach skills to projects and repositories. |
| **3 Requirements** | Each project's requirements, grouped by area: what each one says, which items cite it and how it changed over time. The project's owner adds, amends and withdraws them here. |
| **4 Settings** | Agents, workspaces and projects, item kinds, prompt settings, the database connection, the search server and this machine's profile. |
| **5 Chat** | A live conversation with an agent, and replays of recorded ones. |

**Press `?` on any screen for its full list of keys.** The ones below are the essentials.

### Everywhere

| Key | Action |
|---|---|
| `q` | Quit |
| `Tab` / `Shift+Tab` | Next / previous tab |
| `1` … `5` | Go to a tab |
| `w` | Switch workspace |
| `?` | Show or hide the key help |
| `Esc` | Close the open pop-up |

Most lists use `j` / `k` (or the arrow keys) to move, and `h` / `l` (or `[` / `]`) to switch
between sub-tabs or sections.

### Backlog

Select an item on the left to see it on the right. The detail pane has seven sub-tabs: **Body**,
**Runs**, **Graph** (linked items), **Docs**, **Notes**, **Prompt** and **Reqs** (the requirements
the item cites).

| Key | Action |
|---|---|
| `j` / `k`, `g` / `G` | Move, jump to the first / last row |
| `Enter` on a project | Fold or unfold the project |
| `h` / `l` | Previous / next detail sub-tab |
| `J` / `K`, `PageUp` / `PageDown` | Scroll the detail pane |

**Runs.** The Runs sub-tab lists an item's runs and their steps. Move between steps with `J` / `K`.
If an action isn't allowed right now, the status line says why and nothing happens.

| Key | Action |
|---|---|
| `R` | Start a run on the item |
| `a` / `x` | Approve / reject the step (rejecting asks for a note) |
| `r` | Retry the step |
| `s` | Pick this step as the winner of a fan-out |
| `A` | Accept the step's output |
| `o` | Read the step's output document |
| `p` | Continue the step as a chat |
| `Enter` | Replay the step's conversation in the Chat tab |
| `c` | Cancel the run |
| `T` | Retry the run's cleanup |
| `u` | Unblock the item |
| `C` | Close out the item: pick how it ends with `←` / `→`, press `y`, then type the item's key to confirm |

When you close out an item, the picker offers only the endings that fit its status. A `done` item
starts on `done` and can end as done, concluded, rejected, withdrawn, superseded or duplicate. An
open, blocked or failed item starts on `withdrawn` and can also end as rejected, superseded or
duplicate.

Runs need a connection to the database.

**Prompt.** The Prompt sub-tab shows exactly what an agent would be sent for this item: the
assembled text, its token budget, what was trimmed to fit, and which files were picked as
context. `n` / `p` switch between the project's templates. Looking at a prompt changes nothing and
starts nothing.

**Reqs.** The Reqs sub-tab lists the requirements the item cites, and how: it *addresses* a
requirement, *reserves* it for later, or is the item that *amends* or *withdraws* it. A citation is
marked `! suspect` when the requirement has changed since the item cited it.

| Key | Action |
|---|---|
| `J` / `K` | Move between citations |
| `r` | Re-confirm a suspect citation against the requirement as it reads now |
| `c` | Cite another requirement of the item's project (`Enter` picks it, then `a` addresses or `v` reserves) |
| `u` | Remove the citation (asks you to confirm with `y`) |

A citation made by amending or withdrawing a requirement is part of that decision and stays.
Citing needs a connection to the database.

### Requirements

The left pane lists the workspace's projects, their areas and each area's requirements, with
their priority (`must` or `later`). Select a requirement to see its full text and rationale, which
items cite it (a `! suspect` marker means the item cited an older version) and its revision
history. Withdrawn requirements stay in the list, dimmed and marked `✕`.

| Key | Action |
|---|---|
| `j` / `k`, `g` / `G` | Move, jump to the first / last row |
| `Enter` on a project or area | Fold or unfold it |
| `J` / `K`, `PageUp` / `PageDown` | Scroll the detail pane |
| `/` | Filter the list as you type (`Enter` keeps the filter, `Esc` clears it) |
| `a` | Add an area to the project |
| `n` | Write a new requirement in the selected area |
| `e` | Amend the requirement |
| `W` | Withdraw the requirement (asks you to type its key to confirm) |
| `r` | Read the requirements again |

In the forms, `Tab` moves between fields and `Ctrl+S` saves. An amend or a withdraw names the
item that decided it, and that item then cites the requirement.

A project's requirements belong to whoever first added an area or a requirement to it. Anyone
can read them, but only the owner can add, amend or withdraw; for everyone else, and while offline,
the write keys are greyed out and the status line says why. If someone else changed a requirement
while you were amending it, your text is kept and `Ctrl+S` saves over the new version.

### Chat

| Key | Action |
|---|---|
| `i` / `Enter` | Write a message (`Enter` sends, `Esc` stops writing) |
| `a` | Pick another agent (before the first message) |
| `1` … `9` | Answer the agent's permission request |
| `Esc` `Esc` | End the conversation |
| `t` | Show or hide the agent's thinking |
| `j` / `k`, `g` / `G` | Scroll |

The header shows the agent, the model and the project. If the agent can't do something, such as
ask for permission or propose edits, a banner says so before you start typing.

Chats need a connection to the database. When you quit, `htui` stops every running agent and waits
for it to exit.

**Replays** open read-only over the Chat tab. `Esc` closes the replay and brings back your live
conversation exactly as it was.

### Settings

| Section | What you do there |
|---|---|
| **Agents** | See which agents work on this machine (`r` checks again), install an agent's adapter (`i`), and log in (`a`). |
| **Hierarchy** | Create and edit workspaces (`N`), projects and repositories (`n`), and tell `htui` where each repository lives on this machine (`b`, or `i` to detect them). |
| **Kinds** | Item kinds, their phases and step graphs. |
| **Prompt** | Prompt settings such as the token budget. |
| **Connection** | The database connection string (`e` edit, `c` clear) and rebuilding the local copy (`R`). |
| **Qdrant** | The search server's address and API key (`e` edit, `c` clear). |
| **Boxes** | This machine's profile: tags (`t`), quirks (`e`), who runs its runs, the TUI or [`htui worker`](docs/htui-worker.md) (`w`), and a fresh check of its tools (`p`). |

Deleting in **Hierarchy** shows what would be removed and asks you to type a confirmation, because
it cannot be undone.

## Setting up agents

`htui` comes with three agents:

| Agent | What it is |
|---|---|
| `claude` | Claude Code over ACP, the Agent Client Protocol. **The recommended choice.** |
| `claude-cli` | Claude Code through its command-line JSON stream. A fallback for when the ACP adapter can't be installed. |
| `agy` | Google Antigravity, through Google's ACP server. |

Check **Settings › Agents** to see which ones work on this machine.

### Installing an adapter

Some agents need a separate adapter program. For `agy`, that is Google's `agy_acp_server`, which
the `agy` command-line tool does not install. In **Settings › Agents**, select the agent and press
`i`. Before anything is downloaded, `htui` shows what it will install: the version, where it comes
from, its size, where it will go, its licence, and whether its checksum can be verified. Press `y`
to go ahead, `n` to decline, or `x` to stop an install in progress.

Adapters are installed under:

- Linux: `~/.local/share/htui/agents` (or `$XDG_DATA_HOME/htui/agents`)
- macOS: `~/Library/Application Support/htui/agents`
- Windows: `%LOCALAPPDATA%\htui\agents`

Set `HTUI_AGENTS_ROOT` to install somewhere else. If `htui` can't download the adapter (no network,
a proxy), it shows the steps to install it by hand instead. Unpack the **whole** archive, not just
the server, and make sure the server file is executable.

### Logging in

An installed adapter still has to be logged in. For `agy`, logging into the `agy` command-line tool
is not enough: the adapter keeps its own credential. In **Settings › Agents**, press `a` on the
agent, pick a login method with `j` / `k` and `Enter`, and press `o` to open the link the agent
gives you. `x` cancels.

`htui` never sees or stores your agent credentials. It starts the agent's own login flow and then
checks the agent again, so what **Settings › Agents** shows is what actually works.

Two of `agy`'s methods, **Gemini API key** and **Gemini Enterprise Agent Platform**, are not
browser logins. They need an environment variable (for example `GEMINI_API_KEY`) set before you
start `htui`, and the agent tells you which one.

### Claude Code without ACP

Use `claude` when you can. `claude-cli` uses the same Claude Code login but has fewer abilities:

- It can't ask you for permission. Its permission mode setting decides instead, and refusals show
  up in the transcript.
- It shows edits as the tool calls that made them, not as diffs you can review.
- It doesn't produce plans.

### Spending limits

A project can cap how much a single run may spend, with the `per_token_cap_run` project setting.
The value is in **millionths of a dollar**, so `5000000` means $5. When a run reaches the cap, the
agent is stopped and the run fails with a clear message. An invalid value refuses the chat rather
than being ignored.

The figures are the agent's own estimates, so treat a cap as a safety net, not as your bill.
`per_token_cap_batch` can be set but is not enforced yet.

**Settings › Agents** also shows the remaining allowance per agent where the agent reports one,
such as a subscription's usage window. It updates as you chat.

## Search

`htui` can search your items, their documents and your requirements by meaning and by exact words.
This is optional and needs a [Qdrant](https://qdrant.tech/) server, which `docker compose up -d`
also starts.

1. In **Settings › Qdrant**, press `e` and enter the server's gRPC address (with the included
   `compose.yaml`, that is `http://localhost:6334`), then an API key if your server needs one.
   Both are kept in the OS keyring.
2. Build the index:

   ```
   htui --index-items
   ```

   The first run downloads the embedding model (about 134 MB) into your user cache directory and
   checks it against a pinned hash.
3. Search:

   ```
   htui --search-items "retry after timeout"
   htui --search-items "database choice" --decisions --project my-project --limit 5
   ```

Each result says what matched: an item, one of its documents, or a requirement. A closed item also
shows how it closed, for example `item (rejected)`, and a withdrawn requirement shows
`requirement (withdrawn)`.

Run `--index-items` again to pick up changes. After upgrading from a version that did not index
requirements, the first `--index-items` rebuilds the whole index into a new collection,
`htui_concepts_v2`. The old `htui_concepts_v1` collection is no longer read; you can drop it with
`curl -X DELETE http://localhost:6333/collections/htui_concepts_v1` (Qdrant's HTTP port, not the
gRPC one).

If a later htui embeds with another model, it refuses the old index and says so; delete it with
`curl -X DELETE http://localhost:6333/collections/htui_concepts_v2` and run `htui --index-items`
again.

## Where htui keeps its files

Everything local lives in your user configuration directory: `~/.config/htui` on Linux,
`~/Library/Application Support/htui` on macOS, `%APPDATA%\htui` on Windows.

| Path | What it is |
|---|---|
| `box.toml` | This machine's identity. It survives a hostname change. |
| `cache/<id>/cache.sqlite` | The local copy of one database. Each server gets its own folder, and the name never contains your credentials. |

Deleting `cache/` is safe: it is refilled from the server. You can also rebuild it from
**Settings › Connection** with `R`.

## Troubleshooting

**Logging in an agent on a remote machine.** When `htui` runs on a server, the agent's login link
redirects your browser to `127.0.0.1` on *your* computer, where nothing is listening. Copy the
failed address (`http://127.0.0.1:<port>/?code=…&state=…`) from the browser's address bar and
open it on the server:

```bash
curl -s "http://127.0.0.1:<port>/?code=…&state=…"
```

Forwarding the port with `ssh -L` also works on some setups.

**Garbled lines or boxes on Windows.** Use **Windows Terminal**. The old console (`conhost.exe`,
or `cmd.exe` outside Windows Terminal) works, but box-drawing characters depend on its font and
code page.

**An agent reads `failed` or `unauthenticated`.** Press `r` in **Settings › Agents** to check again,
`i` to reinstall the adapter, or `a` to log in.

**The agent program isn't found.** Point `htui` at it with an `HTUI_TOOL_<NAME>` environment
variable, for example `HTUI_TOOL_NODE` or `HTUI_TOOL_CLAUDE_AGENT_ACP`.

**Something went wrong and you want details.** Start `htui` with `--log htui.log` and look at the
file.

## Development

### Tests

```
cargo test --workspace --all-features
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

Tests that need Postgres read `HTUI_TEST_DATABASE_URL`, a DSN for a user allowed to create
databases. Each test creates its own `htui_test_<hex>` database and drops it when it finishes.
Without the variable those tests print `skipped: HTUI_TEST_DATABASE_URL not set` and pass.

```
docker compose up -d
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test --workspace --all-features
```

The Qdrant tests read `HTUI_TEST_QDRANT_URL` the same way (`http://localhost:6334` with the
included `compose.yaml`).

The test suite never touches your real configuration directory or keyring entry. A test run that is
killed halfway can leave `htui_test_…` databases behind; drop them by hand.

Keep an eye on free disk space. `target/` grows quickly, and a full disk makes Postgres tests fail
in confusing ways (`the database system is in recovery mode`). Deleting
`target/debug/incremental` is usually enough.

### UI snapshots

The UI tests compare rendered screens against `insta` snapshots in `crates/htui/tests/snapshots/`
and `crates/htui/src/snapshots/`. When you change a screen on purpose, re-record them with
`INSTA_UPDATE=always cargo test --workspace --all-features`, run the suite again without the
variable to check they match, and commit the `.snap` files.

### Changing SQL queries

Postgres queries are checked at compile time against the committed data in
`crates/htui-store/.sqlx/`, so building never needs a database. After adding or changing a
`query!`, regenerate that data from inside the crate:

```
cargo install sqlx-cli --no-default-features --features postgres,sqlite   # once
psql postgres://postgres:htui@localhost:5439/postgres -c "CREATE DATABASE htui_sqlx;"
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx migrate run --source crates/htui-store/migrations

cd crates/htui-store
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features
```

Keep `--all-targets --all-features`: without them, the queries used only by tests and the demo are
dropped. `cargo sqlx prepare --check` verifies the data is current.

### Keeping raw agent messages

Set `HTUI_KEEP_RAW_EVENTS=1` to store the original wire message alongside every recorded chat
event. It is off by default.

### Windows-specific code

Only `htui-agent` has Windows-only code (process supervision and command lookup). It can't be
lint-checked from Linux without a C toolchain that targets MSVC, so check it by running
`cargo test -p htui-agent` on Windows, including the ignored live tests.

### Running several `/handoff-run` sessions at once

`scripts/hr` gives each run its own container set: a fresh clone on branch `hr/<ITEM>`, a private
Postgres and Qdrant, and the full toolchain, so three or four runs can work side by side without
getting in each other's way. That guards against accidents, not against a hostile agent: every run
shares your Claude login, settings, plugins and skills, this project's Claude memory and a few other
directories with the host read-write; other projects' transcripts and your prompt history are masked
(see the threat model in the guide). Linux only; needs Docker. `gum` is needed only for the menus: every verb takes plain
arguments. See [`docs/hr-sandbox.md`](docs/hr-sandbox.md).

## Further reading

- [`CONCEPTS.md`](CONCEPTS.md): what `htui` is, and the design decisions behind it.
- [`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md): the full requirements.
