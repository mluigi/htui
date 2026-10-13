# htui from Claude Code: the `htui` plugin and `htui claude emit`

`htui claude emit` writes a Claude Code plugin named `htui`. It is for people who work in a plain
`claude` session beside htui and want to see what htui waits on without switching to the TUI. The
plugin holds two parts:

- **A skill**, `htui`. It teaches Claude Code to read htui's state with the three read commands of
  [`docs/htui-cli.md`](htui-cli.md) (`htui status --json`, `htui waiting --json` and
  `htui --search-items=<QUERY> --json`), and to hand work to htui instead of doing it inline.
- **A mod**: a band above the prompt with the waiting-on-you count, a `/htui` pane listing the
  rows, the turnless commands `/htui status`, `/htui waiting` and `/htui search <query>`, and a
  line that says when the plugin and the `htui` binary are different versions.

The plugin reads htui's state only by running `htui`; it writes nothing of htui's. This is MOD-93.
The rules every htui mod follows are in [`docs/ANA-29.md`](ANA-29.md) §6.3.

- [Write the plugin: `htui claude emit`](#write-the-plugin-htui-claude-emit)
- [Install it](#install-it)
- [Options: `panel` and `poll_seconds`](#options-panel-and-poll_seconds)
- [After upgrading htui](#after-upgrading-htui)
- [What the mod shows](#what-the-mod-shows)
- [What the skill does](#what-the-skill-does)
- [Sessions htui drives](#sessions-htui-drives)
- [Claude Code versions](#claude-code-versions)
- [What it never does](#what-it-never-does)
- [Windows](#windows)

## Write the plugin: `htui claude emit`

```
htui claude emit [--dir <PATH>] [--force]
```

`emit` writes a directory marketplace, also named `htui`, holding the plugin `htui`. The plugin's
version is the binary's: `emit` stamps it into the plugin's `plugin.json` and into the mod. Then it
prints the two commands that install it. It runs neither of them.

| Flag | Meaning |
|---|---|
| `--dir <PATH>` | Write here instead of the default directory below. |
| `--force` | Replace a directory an earlier `htui claude emit` wrote. A directory holding anything else is refused, with or without `--force`. |
| `--log <PATH>` | Append logs to a file. It goes after `claude emit` (`htui claude emit --log x.log`). |

`htui claude` alone is a usage error, and so is a TUI flag beside it (`htui --demo claude emit`).

### The default directory

Without `--dir`, `emit` writes to `htui/claude-plugin` under your local data directory:

| OS | Default directory |
|---|---|
| Linux | `~/.local/share/htui/claude-plugin` (`$XDG_DATA_HOME/htui/claude-plugin` when `XDG_DATA_HOME` is set) |
| macOS | `~/Library/Application Support/htui/claude-plugin` |
| Windows | `%LOCALAPPDATA%\htui\claude-plugin` |

Claude Code reads the installed plugin from this directory, not from a copy (see
[Install it](#install-it)), so keep it where it is, and do not edit it: the next `emit --force`
replaces it.

### Which directories `emit` writes into

| The target is | Without `--force` | With `--force` |
|---|---|---|
| Missing | Created, with its parent directories, and written | The same |
| An empty directory | Written | The same |
| A directory an earlier `emit` wrote | Refused | Replaced |
| A directory holding anything else | Refused | Refused |
| A file or a symbolic link | Refused | Refused |

A directory counts as one `emit` wrote when it holds nothing but `.claude-plugin` and `htui`, and
its `.claude-plugin/marketplace.json` names the marketplace `htui`. A directory with one more
file, another marketplace, no `marketplace.json`, or a `marketplace.json` that is not JSON is left
exactly as it is. So `--force` never touches a directory that holds anything beside
`.claude-plugin` and `htui`. The check looks only at those two top-level names: inside them,
`--force` replaces everything, so keep nothing of your own there, and never point `--dir` at
htui's source tree (`crates/htui/claude-plugin` passes the check).

`emit` writes the new copy into a sibling directory, `<name>.emit-<pid>`, then moves it into
place. With `--force`, the old copy is first moved aside to `<name>.old-<pid>`, the new copy takes
its place, and the old one is removed. If the move fails, the old copy is put back. If the old copy
cannot be removed afterwards, the report names it, and the new plugin is in place.

### What `emit` prints

```
Wrote htui's Claude Code plugin 0.1.0 to /home/you/.local/share/htui/claude-plugin.
The mod is included (claude 2.1.296).
Install it once:
  claude plugin marketplace add /home/you/.local/share/htui/claude-plugin
  claude plugin install htui@htui
After upgrading htui: htui claude emit --force, then /reload-plugins in Claude Code.
```

The first line names the version written and the directory, as an absolute path. The second line
is one of three (see [Claude Code versions](#claude-code-versions)):

| `claude --version` | Second line |
|---|---|
| 2.1.287 or later | `The mod is included (claude <version>).` |
| Below 2.1.287 | `The mod is left out: claude <version> is below 2.1.287, so only the skill was written.` |
| No answer | `The mod is included; claude's version could not be checked (no answer from claude --version).` |

The directory in the `marketplace add` line is quoted for the shell when it needs it (a path with
a space). When the old copy could not be removed after a `--force`, one more line follows:
`The old copy could not be removed: <path>.`

### Exit codes

| Exit | Meaning |
|---|---|
| 0 | Written. |
| 2 | Refused: the target holds the htui plugin and `--force` is missing, holds files `emit` did not write, or is a file; or there is no local data directory and no `--dir`. A usage error also exits 2. |
| 1 | Failed: a directory could not be read, created, moved or replaced, a file could not be written, or the report could not be printed. If moving the new copy into place fails, the old copy is put back. |

A refusal prints its sentence on standard error, as `htui: <sentence>`, and writes nothing:

| Case | Sentence |
|---|---|
| An earlier `emit`, without `--force` | `<dir> already holds the htui plugin; pass --force to replace it` |
| Anything else in the directory | `<dir> holds files htui claude emit did not write (<why>); it is left as it is. Choose another --dir.` |
| A file | `<dir> is a file, not a directory` |
| A path with no directory name | `<dir> names no directory to write` |
| No local data directory | `this machine has no local data directory; pass --dir PATH` |

`<why>` names the first thing that is not `emit`'s: a foreign entry, `no marketplace.json`,
`marketplace.json is not htui's`, or `a symbolic link`.

No `emit` ending is sent to htui's crash reporter, whatever its code: its sentences carry paths in
your home directory.

## Install it

Run the two commands `emit` printed, once:

```
claude plugin marketplace add ~/.local/share/htui/claude-plugin
claude plugin install htui@htui
```

The first adds the directory as a marketplace named `htui`; the second installs the plugin `htui`
from it. Claude Code reads a plugin installed from a directory marketplace from that directory,
not from a copy, so a later `emit --force` and `/reload-plugins` are all an upgrade needs.

## Options: `panel` and `poll_seconds`

| Option | Values | Default | Meaning |
|---|---|---|---|
| `panel` | `auto`, `command`, `off` | `auto` | `auto`: a band above the prompt when something waits, and `/htui` opens a pane. `command`: no band, and `/htui` opens the pane. `off`: no band and no pane; `/htui` prints the rows as text. |
| `poll_seconds` | a number | `30` | Seconds between `htui waiting` polls while the band or the pane shows. Below 5 is read as 5. |

Set them in a session with `/plugin configure htui@htui`, or when installing:

```
claude plugin install htui@htui --config panel=command --config poll_seconds=60
```

A change reloads the mod with the new values.

The mod polls only while something can show the result, so a poll (one `htui` process and one
query) costs nothing when nothing shows it:

| `panel` | When it polls |
|---|---|
| `auto` | Once when the session starts, then every `poll_seconds` for the session. |
| `command` | From the first `/htui` until a poll finds the pane closed. |
| `off` | Never. Each `/htui` runs `htui waiting` once. |

A poll that is still running when the next one is due is not overlapped: the next one is skipped.

## After upgrading htui

The plugin's version is the binary's. After upgrading htui, rewrite the plugin and reload it:

```
htui claude emit --force
```

then `/reload-plugins` in Claude Code. No reinstall is needed.

Until then, the mod shows a skew line in the band, in the pane and under `/htui status`:

```
htui 0.2.0 ≠ plugin 0.1.0: run htui claude emit --force, then /reload-plugins
```

The mod checks `htui --version` each time the session starts and each time the mod is reloaded, so
the line clears after `/reload-plugins`. When `htui --version` gives no answer, there is no skew
line; the poll reports why.

When `htui` answers with a schema version newer than the plugin reads, every command and the band
show `htui: answer version <N> is newer than this plugin: run htui claude emit --force, then /reload-plugins`.

## What the mod shows

The mod runs these commands, each with a time bound:

| Command | Bound | For |
|---|---|---|
| `htui waiting --json` | 10 s | The band, the pane, `/htui waiting` |
| `htui status --json` | 10 s | `/htui status` |
| `htui --version` | 10 s | The skew line, once per session start |
| `htui --search-items=<query> --json` | 60 s (the first search may load the embedding model) | `/htui search` |

`status` and `waiting` read the first workspace, the one the TUI opens on
([`docs/htui-cli.md`](htui-cli.md#scope-what-a-command-reads)). The query rides inside one
argument after `=`, so a query that starts with `-` is not read as a flag. Control characters are
removed from htui's stored text before the mod shows it.

### The band

Under `panel` `auto`, a band above the prompt shows, one line each and only when present:

- `htui: <n> waiting on you`, when at least one row waits;
- the skew line;
- the last poll's failure line, dimmed.

With no rows, no skew and no failure, there is no band. There is also none while Claude Code shows a
survey, and none under `command` or `off`.

### The pane

`/htui` opens a pane named `htui` (under `auto` and `command`). It shows:

- a header, `<workspace> · <n> waiting` (`no workspace` when the store has none);
- the skew line and the last poll's failure line, when present;
- one line per row: the reason, the item key, the step (`-` when there is none) and the age in
  columns, then the row's text, cut to the pane's width with a trailing `…`;
- `Nothing waits on you.` when there are no rows, or `htui: no answer yet` before the first poll
  answers.

The reason is written as the TUI's overlay writes it: `judge_failed` reads `judge failed`. The age
is the largest whole unit: `42s`, `5m`, `3h`, `2d`. When Claude Code cannot place the pane, `/htui`
answers `htui: the pane waits: <reason>`.

### The commands

The commands are turnless: each costs no model turn. Typed while Claude is idle, a command answers
at once; typed during a turn, it runs when that turn ends. The answer is the command's output row
in the transcript, and like any command's output the model reads that row on its next turn, so the
item keys, reasons, permission text and search snippets a command prints reach the model then.
`/htui` prints no row when it opens the pane.

| Command | Prints |
|---|---|
| `/htui status` | The top bar's line, `<workspace> · <box> · <store> · <n> working · <n> waiting`, as `htui status` prints it (`no workspace` and `no box` stand in for a missing one). Then `permission requests could not be listed; the count may be low` when htui could not list them. Then the skew line, if any. |
| `/htui waiting` | One line per row: `<item key>  <reason>  <step>  <age>  <text>`, with `-` for no step. No rows: `Nothing waits on you.` |
| `/htui search <query>` | One line per hit: `<key>  <place>[ <document kind>][ (<resolution>)]  <snippet>`. No index yet: `No concepts index yet: run htui --index-items.` No hits: `No hits.` |
| `/htui` | Opens the pane; under `panel` `off`, prints what `/htui waiting` prints. |

`/htui search` with no query prints `usage: /htui search <query>`, and any other word prints
`usage: /htui [status|waiting|search <query>]`.

For example:

```
/htui status
Platform · desk · online · 1 working · 2 waiting

/htui waiting
MOD-4  gate  prd 0.1  18h  approve the PRD
MOD-7  unblock  -  2d  blocked, no active run: u reopens it

/htui search closed items
ANA-11  item  a decision is a closed item
MOD-34  document prd (done)  the concepts index
```

### When `htui` fails

Every failure is one line in place of the answer; none stops the session. The band shows the last
poll's line, and the commands print theirs (`/htui status` adds the skew line after it).

| What happened | Line |
|---|---|
| `htui` is not on `PATH`, or could not start | `htui: not found on PATH` |
| `htui` ran past its bound | `htui: timed out` |
| `htui` printed nothing | `htui printed nothing (exit <code>)` |
| The answer is not a JSON document | `htui: unreadable answer (exit <code>)` |
| The answer's schema version is newer | `htui: answer version <N> is newer than this plugin: run htui claude emit --force, then /reload-plugins` |
| A failure document (the store offline, a refusal, a failed read) | `htui: <error.message>`, for example `htui: cannot reach Postgres: store unreachable: connection refused` |
| Any other non-zero exit | `htui: exit <code>` |

The failure documents and their messages are in
[`docs/htui-cli.md`](htui-cli.md#failure-documents).

## What the skill does

Claude Code uses the skill `htui` when you mention htui, ask what waits on you or what htui is doing, ask to
hand something to htui, or ask whether an htui item covers something. It tells Claude Code:

- to run `htui status --json`, `htui waiting --json` and `htui --search-items='<query>' --json`,
  and how to read their documents, exit codes and failures;
- when a request matches an open htui item or belongs in htui's queue, to say so, name the item
  by its key, and point you at the TUI, never working the request inline;
- never to try to write htui's store, never to connect to htui's Postgres or Qdrant, and to run no
  other htui command. Building the concepts index (`htui --index-items`) is your step.

The skill works on its own, without the mod.

## Sessions htui drives

Every Claude Code session that htui starts has `HTUI_SESSION=1` in its environment (MOD-91). Such
a session leaves out your mods and skills unless the agent's `mods (y/n)` is `y` (see
[Your Claude Code mods, hooks and skills](../README.md#your-claude-code-mods-hooks-and-skills)).
When it is `y`, the mod stands down: with `HTUI_SESSION` set, it registers no `/htui` command,
starts no poll, runs no `htui`, and draws no band. The stand-down is the mod's; the skill has none.

## Claude Code versions

Mods need Claude Code 2.1.287 or later. `emit` runs `claude --version` (bounded at 15 s) and
decides what to write:

- **2.1.287 or later**: the skill and the mod.
- **Below 2.1.287**: the skill alone. The plugin has no `hooks/` and no `types/`, and its
  `plugin.json` has no `types` and no options. The skill works without the mod. After upgrading
  Claude Code, run `htui claude emit --force` and `/reload-plugins` to add the mod.
- **Not checked** (`claude` is not on `PATH`, or printed no version): the skill and the mod, and the
  report says the version was not checked. If your Claude Code is older than 2.1.287, run `emit`
  again with `claude` on `PATH`, with `--force`.

## What it never does

- **`emit` never edits `~/.claude`** and never runs `claude plugin` itself: it prints the commands.
  It writes nothing outside the target directory and its two siblings, `<name>.emit-<pid>` and
  `<name>.old-<pid>` (and creates the target's parent directories).
- **No writes to htui's store.** The mod and the skill read through `htui status`, `htui waiting`
  and `htui --search-items`, which only read (see
  [`docs/htui-cli.md`](htui-cli.md#no-writes)). Neither connects to Postgres or Qdrant, and
  neither reads the DSN.
- **No model calls.** The mod makes none, and nothing it shows is decided by a model.
- **Reach limited to running `htui`.** The mod runs processes (`$.process.run`, only `htui`),
  keeps a timer, reads one environment variable (`HTUI_SESSION`), registers `/htui`, keeps its own
  state and draws its band and pane. It makes no network call, writes no file, and never
  submits a prompt or starts a turn; its only transcript rows are the `/htui` answers above.
  `claude plugin validate --strict <dir>/htui` lists what it calls:

  ```
  ./register.tsx calls: $.clock.every, $.clock.now, $.command.register, $.env.get, $.process.run, $.state.get, $.state.set, $.ui.open, $.ui.panes, $.ui.resolve
  ./register.tsx env reads: HTUI_SESSION
  ./register.tsx env writes: nothing
  ```

  (`validate` adds which function makes each call.)

## Windows

Unconfirmed. `emit`'s default directory and the install commands are the same on Windows, and the
mod reaches `htui.exe` through `$.process.run`, but this has not been run on Windows yet. The
check is part of MOD-16 in `HANDOFF.md`.
