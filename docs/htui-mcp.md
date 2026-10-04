# htui's MCP server: the tools an agent session gets

Every agent session htui starts gets one MCP server, named `htui`. Its tools let the agent write
its step's output document, add notes, propose links and status changes, look up the machine it
runs on, search the project, and run heavy commands through a per-box queue. The agent sees them
as `mcp__htui__<tool>`. This is MOD-11 (`docs/REQUIREMENTS.md` R-MCP-1 to R-MCP-4).

The server belongs to htui, not to the agent: the htui process that started the session (the TUI,
or `htui worker`) answers every call with its own database connection, under the session's own
scope. The agent never gets a database connection string, and it never names a run, a step, a
project or a box.

- [How it works](#how-it-works)
- [The tools](#the-tools)
- [Which session gets which tools](#which-session-gets-which-tools)
- [Scope: what a session can touch](#scope-what-a-session-can-touch)
- [The output instruction in the prompt](#the-output-instruction-in-the-prompt)
- [`command_run` and the command queue](#command_run-and-the-command-queue)
- [Heavy commands outside the queue (R-MCP-4)](#heavy-commands-outside-the-queue-r-mcp-4)
- [Permission prompts for `claude-cli`](#permission-prompts-for-claude-cli)
- [Personas can hide htui's tools](#personas-can-hide-htuis-tools)
- [The socket](#the-socket)
- [`htui mcp`: the relay](#htui-mcp-the-relay)
- [Troubleshooting](#troubleshooting)

## How it works

1. When htui starts a session, it registers the session with its **tool host**, which mints a
   random 256-bit token for that session alone.
2. The session's agent is told to start an MCP server over stdio: the command `<htui binary> mcp`
   with two environment variables, `HTUI_MCP_ADDR` (where the host listens) and `HTUI_MCP_TOKEN`
   (the session's token).
   - An ACP agent gets it in its `session/new` request, as a stdio entry of `mcpServers`.
   - `claude-cli` gets it as one argument, `--mcp-config=<json>`. htui does not pass
     `--strict-mcp-config`, so the MCP servers you configured for Claude Code yourself stay
     available.
   - The binary is htui's own: on Linux `/proc/<pid>/exe` of the host process, which stays valid
     for the host's life even if the file on disk is replaced; elsewhere the path htui was started
     from.
3. `htui mcp` connects to the host's [socket](#the-socket), presents the token, and from then on
   copies bytes between its stdin/stdout and the socket. It never reads the MCP it carries.
4. The host process answers the MCP: newline-delimited JSON-RPC 2.0, MCP protocol versions
   `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05` (the newest when the agent asks for
   another). A message longer than 1 MiB is refused with a parse error, and the connection goes
   on.
5. When the session ends, its token is unregistered. Any later call answers `session ended`.

Which processes host the tools:

- **The TUI** runs one tool host for its walks and its chats. If the host cannot be created, the
  TUI logs a warning ("htui's MCP tools are not hosted this session") and runs without tools; a
  step that must write a document then fails `missing_output`.
- **`htui worker`** runs one for the runs it walks. A worker that cannot create it refuses to start
  (exit 2, "htui's MCP tools cannot be hosted: …"), because every document phase would fail.

Creating the host only resolves htui's own binary, so it fails only off Linux, when that binary
has no absolute path; on Linux it never fails. The [socket](#the-socket) is bound later, at the
first session: a socket directory that cannot be created or is not private does not stop the TUI
or the worker from starting, and instead fails each step with
[`agent spawn failed: htui's MCP listener could not start: …`](#troubleshooting).

## The tools

Every answer is one text item. A successful call answers compact JSON; a refused one answers a
one-line reason with `isError: true`. Every argument object except `permission_prompt`'s refuses
keys it does not list, so a `run_id`, `item_id` or `box_id` argument is refused as
`invalid arguments: unknown field …`. `permission_prompt`'s object stays open, because the Claude
Code CLI may add keys to its call: an extra key there is ignored.

| Tool | Arguments | Answer |
|---|---|---|
| `box_profile` | none | `{"profile": {…}, "text": "…"}`: the box's OS, CPU, RAM, GPU and detected tools, as JSON and as the prompt's `box` section renders it |
| `document_write` | `body` (Markdown), `title` (optional; the output kind when omitted) | `{"document_id", "kind", "version"}` |
| `note_add` | `body`, at most 16384 bytes | `{"note_id"}` |
| `item_status` | `status`, `reason`, `resolution` (optional, only with `closed`) | `{"note_id"}` |
| `item_link` | `op` (`add` or `remove`), `to` (an item key such as `FEAT-12`), `kind` | `{"from", "to", "kind", "live"}` |
| `search_concepts` | `query`, `types` (optional), `statuses` (optional), `limit` (optional, 1 to 20, default 10) | `{"hits": […]}` |
| `command_run` | `class` (`build`, `test` or `run`), `command`, `cwd` (optional), `timeout_secs` (optional, 1 to 1800, default 1800) | `{"exit_code", "status", "output", "truncated", "command_run_id"}` |
| `permission_prompt` | `tool_name`, `input`, `tool_use_id` (the Claude Code CLI's contract) | `{"behavior": "allow", "updatedInput": …}` or `{"behavior": "deny", "message": …}` |

**`box_profile`** describes the box the session runs on. When the project's `box_hostname` switch
is off, the JSON has no `hostname` key and the text starts at `os:`, exactly like the prompt's
`box` section. A box with no stored profile answers `not found: box <id>`.

**`document_write`** writes the step's output document on the run's item, with the kind the phase
names (`output_kind`); the agent cannot choose either. Each call writes a new version, and the
newest one is the step's document. The title and the body are
[scrubbed](#scope-what-a-session-can-touch) first.

**`note_add`** adds a note to the run's item, recorded as written through this step (and on this
box). The limit is counted after scrubbing: a longer body is refused as
`refused: note is <n> bytes, the limit is 16384`.

**`item_status`** asks for a status change; it never makes one. It writes a note
`status request: <status> (<resolution>) — <reason>` (without the parenthesis when there is no
resolution) on the run's item, through the same path as `note_add`. `status` is one of `open`,
`queued`, `in_progress`, `awaiting_approval`, `blocked`, `done`, `failed` or `closed`;
`resolution` is one of `done`, `concluded`, `rejected`, `withdrawn`, `superseded` or `duplicate`,
and goes with `closed` only (`refused: a resolution goes with closed only`). The item's status is
moved by a person, or by the engine's own rules, never by this tool.

**`item_link`** proposes (`add`) or withdraws (`remove`) a link that reads
`<this item> <kind> <to>`, where `kind` is `blocked_by`, `origin`, `relates` or `supersedes`. The
link always starts at the run's item. `to` is a key looked up in the session's project only; a key
the project does not have answers `out of scope: <key> is not an item of this project`.
`add` revives a withdrawn link, and leaves a live link exactly as it was, proposer included. A run
can withdraw only a link one of its own steps proposed; a live link another run (or a person)
proposed answers `not yours: <from> <kind> <to> was not proposed by this run`. A `remove` of a
link that is not live (it never existed, or it is already withdrawn) answers
`not found: item_link <from-id>-<kind>-><to-id>`, with the two items' ids. `live` says whether
the link is in force after the call.

**`search_concepts`** searches the concepts index (see the README's
[Search](../README.md#search)) within the session's project only. `types` keeps `item`,
`document` or `requirement` points (an unknown name answers `invalid arguments: unknown type
<name>`); `statuses` keeps items in those statuses, and naming any status leaves requirements out.
A hit carries `point_type`, `owner_kind` (`item` or `requirement`), `key`, `score`, `snippet`, and
when they apply `document_kind`, `resolution` (a closed item's) and `state` (a requirement's). The
`snippet` is the start of the indexed text as stored (whitespace folded, control characters
dropped, at most 280 characters), not scrubbed. It looks nothing else up. An index that cannot
answer (no Qdrant URL stored, Qdrant down, the model not loaded) answers
`search unavailable: <cause>`.

**`command_run`**: see [below](#command_run-and-the-command-queue).

**`permission_prompt`**: see [below](#permission-prompts-for-claude-cli). Claude Code hides this
tool from the model; only the CLI calls it.

## Which session gets which tools

A tool a session is not offered is absent from its `tools/list`, and calling it anyway is a
JSON-RPC error (`-32602`, `unknown tool: <name>`).

| Tool | Offered when |
|---|---|
| `box_profile` | always |
| `document_write` | the session has an item **and** an output kind |
| `note_add`, `item_status`, `item_link` | the session has an item |
| `search_concepts` | the host has a concepts index (below) |
| `command_run` | the step's command queue is exposed ([below](#when-command_run-is-offered)) |
| `permission_prompt` | the agent uses the CLI transport (`claude-cli`) |

What that gives each kind of session:

- **An engine step** (a phase of a run, in the TUI or the worker) has its run's item. It gets
  `document_write` when its phase names an output kind, and `command_run` when the phase exposes
  the queue. A fan-out judge call writes its verdict document with `document_write` and never gets
  `command_run`.
- **A fresh chat** has no item: `box_profile` and `search_concepts`, plus `permission_prompt` for
  `claude-cli`.
- **A promoted step's chat** has the step's item, so it also gets `note_add`, `item_status` and
  `item_link`, and `document_write` when the phase's output kind can be read from the run's
  snapshot (a snapshot that does not decode withholds that one tool, never the chat). A chat never
  gets `command_run`.

`search_concepts` is offered by the TUI always; a call answers `search unavailable: …` while no
Qdrant URL is stored (Settings › Qdrant). `htui worker` offers it only when the keyring held a
Qdrant URL at start, the same condition under which it keeps the index current (see
[`docs/htui-worker.md`](htui-worker.md#the-concepts-index)); a provisioned worker has no keyring,
so it never does.

## Scope: what a session can touch

- **The token is the scope.** The session's run, step, item, project, box, working directory and
  fence come from the token, never from arguments.
- **Every item write is fenced.** An engine step writes under its walk's lease: once another
  process has taken the run over (an adoption after a crash, a lapsed lease), every
  `document_write`, `note_add`, `item_status` and `item_link` answers `fenced: lease lost` and
  writes nothing. A chat writes without a lease, and is refused the same way once a walk holds the
  run again. A step only ever writes on its own run's item.
- **`command_run` is not fenced.** Its queue rows take no lease, so a step that has lost its lease
  can still queue and run a command, and record it, until its walk notices the loss (at its next
  lease renewal) and drops the step. The agent's connection then closes, and the command is killed
  and its row `cancelled`, as when the agent exits (see
  [Liveness](#liveness-crashes-and-cancellation)).
- **No agent moves a status** (R-ENT-8): see `item_status`.
- **Every text an agent writes, and every command's output, is scrubbed first, and fails
  closed** (R-ID-7, R-SEC-3): note and document bodies, document titles, command lines and output
  tails. A text that matches one of htui's credential rules (an API key's prefix, for example) is
  refused as `refused: the text matched credential rule <rule>; nothing was written`; an output
  tail is [withheld](#what-a-call-does) instead. The answer names the rule, never the
  text. What a tool reads back from htui (`box_profile`'s profile, `search_concepts`' snippets) is
  returned as stored, unscrubbed.
- **A token outlives nothing.** It is registered for exactly the session's life and never logged.
  When the session ends (the agent exits, the step settles, the chat ends, htui quits), the token
  is unregistered, and a call on a connection still open answers `session ended`.
- **The token is visible in `claude-cli`'s command line.** `--mcp-config=<json>` carries
  `HTUI_MCP_TOKEN`, so another local user can read it from the process list (`/proc/<pid>/cmdline`).
  It grants nothing on its own: the socket lives in a directory only your user can enter (`0700`),
  so nobody else can connect to present it, and it dies with the session. An ACP agent receives it
  over its stdin instead.

## The output instruction in the prompt

When a session is offered `document_write`, its prompt ends with one more protected section,
`<section name="output">`, holding exactly:

```
Write your `<kind>` document by calling the `document_write` tool of the `htui` MCP server; text left only in your reply is not recorded.
```

with the phase's output kind in place of `<kind>`. The engine adds it to phase prompts when it has
a tool host and the phase names an output kind, and to every judge prompt when it has a tool host.
A promoted step's handoff prompt keeps it. The section is protected: no token budget trims it,
and it is part of the prompt digest. A session that is not offered `document_write` gets no such
section. The Backlog prompt preview does not show it.

There is **no fallback**: a session that writes no document leaves no document, whatever its reply
says, and the step fails `missing_output` as before.

## `command_run` and the command queue

`command_run` runs a build, test or run command through a queue shared by every htui process on
the box, so several agents do not run heavy builds at once. The command runs on the box of the htui
process that hosts the session, as your user, with that process's environment.

### When `command_run` is offered

A phase's `command_queue` setting decides (shown in Settings › Kinds):

| `command_queue` | `command_run` offered |
|---|---|
| `off` | never |
| `always` | always |
| `fan_out_only` | when the phase fans out to more than one agent, or the item carries the tag `heavy_build` |

Every seeded phase is `fan_out_only`, so a single-agent step gets the queue only on an item tagged
`heavy_build` (the `tags` field of the item's form in Backlog). That field is the item's required
box tags (`R-ORCH-10`), so the tag is also a requirement: the item runs only on a box whose tags
include `heavy_build`. No probe sets it, so declare it on each box meant to take such items
(**Settings › Boxes**, `t`); on any other box a run is refused and the item is blocked with the
note `missing tags: heavy_build`, and auto mode does not pick the item there. A phase whose
`command_queue` is `always` gives a single-agent step the queue without tying the item to a box;
Settings › Kinds shows that setting but does not edit it (it is the `step_graph_phase` row's
`command_queue` column). A persona with `command-run: false`, or one that denies the `execute`
kind, takes it away (see
[`docs/personas.md`](personas.md#narrow-only-what-a-persona-can-and-cannot-change)), and an
engine that does not host htui's tools offers it to no step, nor refuses the heavy commands
below. The prompt's `command_queue` section follows the same rules, so the prompt never names a
tool the session lacks. Judge calls and chats never get it. Before MOD-11 a `fan_out_only` phase
rendered that section on every step; a single-agent step without `heavy_build` no longer does, so
its prompt digest differs from the same step's before the upgrade.

### Limits

Each `(box, class)` pair runs at most its limit of commands at once. The limit is the app setting
`command_limits`, seeded `{"build":1,"test":4,"verify":1}`, overlaid key by key with the box's own
`command_limits` object in the box row's `settings`. A class missing from both (`run`, with the
seed) is 1, and so is 0. An entry that is not a whole number is skipped (a box's `command_limits`
that does not parse is also warned about in the log). The limits are read again on every call.
No screen edits them yet: set them in Postgres, for example:

```sql
UPDATE app_setting SET value = '{"build":2,"test":4,"run":2,"verify":1}' WHERE key = 'command_limits';
```

The class `verify` belongs to the orchestrator's own verification: `command_run` refuses it
(`refused: verify is the orchestrator's`), and any class other than `build`, `test` or `run`.

### What a call does

1. **Checks.** `cwd`, when given, must be a relative path with no `..` that names an existing
   directory under the session's working directory; the default is that directory itself. The
   command may not be empty. `timeout_secs` must be 1 to 1800. A session runs one `command_run`
   at a time: a call while its previous one is still queued or running is refused at once with
   `refused: a command_run is already queued or running in this session`, and queues nothing.
2. **Queues.** A `queued` row is written to `command_run`, with the command line scrubbed (the
   shell still runs what the agent wrote).
3. **Waits for admission.** Every second the call asks for a slot. Rows of one `(box, class)` are
   admitted oldest first, while fewer than the limit are running.
4. **Runs.** `sh -c <command>` (`cmd /C` on Windows) in that directory, with stdin closed and stdout
   and stderr merged. When the timeout elapses, the command and everything it started are killed
   (the whole process group; a job object on Windows). When the shell exits, output still in
   flight is read for up to 2 more seconds, then whatever the command left running in the
   background is killed the same way, so `server & echo started` neither holds the call nor
   outlives it (a process that left the group, for example with `setsid`, is not reached).
5. **Answers.** The last 64 KiB of output is kept, scrubbed and stored on the row, then returned.
   When more was printed, `truncated` is `true` and the tail opens with the line
   `[… earlier output truncated]`. A timed-out command's output opens with the line
   `[killed: the <n> s timeout elapsed]`, before that one. Output that cannot be masked is
   replaced by `[output withheld: it matched credential rule <rule>]`.

`status` is `done` when the command exited (check `exit_code`: a failing build is still `done`),
and `failed` when it timed out, was killed by a signal, could not start, or was reaped (see
below); `exit_code` is then `null`. `cancelled` means the queue cancelled the row while the
command ran.

### Liveness, crashes and cancellation

- A running command's claim is renewed every 10 seconds. A `running` row whose renewal is older
  than 30 seconds belongs to a host that died: the next claim of the same `(box, class)` marks it
  `failed` and appends `reaped: its host stopped heartbeating (MOD-11 OQ-3)` to its output, so its
  slot is free again. A `queued` row whose caller stopped asking for 30 seconds is `cancelled` the
  same way. A crashed TUI or worker therefore holds a slot only until a command of the same class
  asks for it more than 30 seconds later.
- If the host's own command is reaped or cancelled while it runs, it is killed, and the call
  answers the row's status with what the command printed so far.
- If the agent cancels the call, or the agent exits (its connection closes) while the call is
  queued or running, the row is `cancelled` and the command is killed.
- An agent that asked for progress (`_meta.progressToken`) gets an MCP progress notification every
  second while the call waits, and every 10 seconds while the command runs.

**Long calls and the agent's own timeout.** A command may run for 30 minutes. Some MCP clients
give up on a tool call earlier; the progress notifications help those that wait as long as
progress arrives. If your agent abandons long `command_run` calls, raise its MCP tool timeout (for
Claude Code, the `MCP_TOOL_TIMEOUT` environment variable, in milliseconds, set in the environment
htui is started from) or pass a shorter `timeout_secs`.

## Heavy commands outside the queue (R-MCP-4)

While a step has `command_run`, htui refuses the heavy commands an agent might run in its own shell
instead, so it routes them through the queue. A command is refused when it **starts with** one of
these, as a whole word (the command ends there or a space follows, so `make` refuses `make` and
`make -j8` but not `makepkg`, and `cargo build` does not refuse `cargo build-sbf`):

`cargo build`, `cargo test`, `cargo nextest`, `cargo clippy`, `cmake --build`, `ctest`, `make`,
`ninja`, `msbuild`, `dotnet build`, `dotnet test`, `npm test`, `pnpm test`, `go build`, `go test`.

- **ACP agents:** the step's permission policy opens with one `reject_once` rule per prefix, ahead
  of the persona's and the agent's own rules. The agent reads the reason
  ``run `<prefix>` through htui's `command_run` tool (R-MCP-4)``.
- **`claude-cli`:** `Bash(<prefix>:*)` per prefix is added to `--disallowedTools`.

This is a nudge, not a fence. On ACP a rule compares the start of the command string only, so
`cd app && cargo build`, `env cargo build` or a script that runs `make` are not refused, and only
a command the agent asks permission for can be refused at all. On
`claude-cli` the CLI matches `Bash(<prefix>:*)` by its own rules, and a script that runs `make`
still passes. The list is fixed in this build; it is not a setting.

## Permission prompts for `claude-cli`

Claude Code without ACP has no permission channel of its own. When htui hosts the tools, it starts
`claude-cli` with `--permission-prompt-tool mcp__htui__permission_prompt`, so every tool call the
CLI's own permission mode does not settle (the seeded `claude-cli` runs in `acceptEdits`, so file
edits proceed and shell commands ask) is sent to htui as an ordinary permission request:

- **In an engine step**, the step's policy answers it first (the agent's rules and the persona's,
  as for ACP); otherwise the step parks and the **Runs** pane asks you, in the TUI or for a worker
  (see [`docs/htui-worker.md`](htui-worker.md#permission-requests-on-worker-steps)).
- **In a chat**, the chat asks you with the usual `1` … `9`, and no banner says the agent cannot
  ask.

**htui's own tools are pre-approved.** On both transports, in a step and in a chat, the session's
policy answers `allow_once` for every htui tool the session is offered except `command_run` (and
`permission_prompt`, the channel itself), so `document_write` or `note_add` never waits for you.
These rules come after the R-MCP-4 refusals, the persona's rules and the agent's own, just ahead of
the remembered choices and the default, so they only replace the default `ask`: a persona that
rejects the kind htui's tools carry (`other`), or a rule of yours that names one of them, still
wins. A policy whose default is not `ask` gets none of them: under a persona's or an agent's
`deny` default htui's tools stay rejected, and under `allow` the default already allows them.
`command_run` runs any command, so under the default `ask` it still asks.

The options are **Allow** and **Reject**. An allow lets the call run with its input unchanged; a
reject answers the CLI `denied in htui`. A cancel answers every pending prompt
`the session was cancelled`, and a session that ends answers them `the session ended`. A denied
call is recorded once, by its answer.

**Gated phases.** A phase with a gate other than `never` needs an agent that can be asked inline.
Before MOD-11 every `claude-cli` candidate was skipped there (`missing_capability:
inline_approval`). An engine with a tool host now admits it, because the prompt tool asks inline.
If such a session nevertheless starts without the prompt tool, the step fails at once with
`agent spawn failed: missing_capability: inline_approval — a gated CLI step needs htui's
permission_prompt tool` (unlike the selection-time skip above, it carries the spawn prefix).

## Personas can hide htui's tools

A persona's `disallowed-tools` reaches `claude-cli`'s `--disallowedTools` unchanged. An entry that
covers htui's tools, such as `mcp__*` or `mcp__htui__*`, hides them from the agent, including
`document_write`. htui honours this as the persona's deliberate choice and does not refuse such a
persona; a step bound to it that must write a document then fails `missing_output`. To keep the
tools, name only the MCP tools you mean to remove. ACP agents do not enforce tool names, so there
the entry has no effect. See [`docs/personas.md`](personas.md#htuis-own-mcp-tools).

## The socket

Nothing listens on the network: there is no TCP port. Each htui process that hosts tools opens one
local listener, at its first session, and the token gates every connection.

- **Linux and macOS:** a Unix socket named `s` inside a new directory
  `htui-mcp-<pid>-<8 hex>`, created with mode `0700` (htui refuses to use it if it is not
  private). The directory is under `$XDG_RUNTIME_DIR` when that is set to an existing absolute
  directory, otherwise under the temporary directory (`$TMPDIR`, else `/tmp`). A worker run as a
  systemd system service has no `XDG_RUNTIME_DIR`, so its directory is in `/tmp`.
- **Windows:** a named pipe `\\.\pipe\htui-mcp-<32 hex>`, created with Windows' default security for
  your account and refusing remote clients. This path is compiled but not yet exercised on Windows
  (MOD-16).

The socket and its directory are removed when the htui process exits normally. A crash (`kill -9`,
a power cut) leaves the directory behind; nothing sweeps it. It is safe to delete once no htui
process with that pid runs:

```
ls -d "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}"/htui-mcp-*
```

A connection must send its token within 5 seconds, or it is closed.

## `htui mcp`: the relay

```
htui mcp
```

It is started by the agent htui launched, never by hand. It reads `HTUI_MCP_ADDR` and
`HTUI_MCP_TOKEN`, writes only MCP to standard output, logs nothing, and exits when the agent closes
its standard input or the host ends the connection. A failure is one line on standard error,
`htui: <sentence>`, and is never sent as an error report.

| Code | Meaning |
|---|---|
| `0` | The agent closed stdin, or the host ended the connection. |
| `1` | The host could not be reached (`cannot reach the htui host at <address>: …`), or the stream broke (`relay i/o: …`). |
| `2` | A variable is unset or empty (`HTUI_MCP_ADDR is not set: …never by hand`), the address is not UTF-8, or the token is not 64 lowercase hex characters. The sentence never prints the token. |
| `3` | The host refused the handshake: `the htui host refused this relay: <reason>`, the reason being `no live htui session has this token (it ended, or it never existed)`, a version mismatch, or `malformed handshake`. |

## Troubleshooting

**The agent has no `htui` tools, and a step fails `missing_output`.** The agent could not start or
keep the server. Look at the agent's own MCP log or status (Claude Code reports each server as
`connected` or `failed`) for the relay's line on standard error and its exit code (see the
[table](#htui-mcp-the-relay)). Also check:

- A **sandboxed agent** that does not share the host's `/proc` (bubblewrap, a container) cannot
  start `/proc/<pid>/exe`, nor reach a socket outside its sandbox.
- The TUI logged "htui's MCP tools are not hosted this session" at start.
- A [persona](#personas-can-hide-htuis-tools) hides the tools.

**A step fails with `agent spawn failed: htui's tools cannot be hosted offline: no store to write
to`.** The TUI had
no database connection when the session started. Reconnect, then retry.

**A step fails with `agent spawn failed: htui's MCP listener could not start: …`.** The socket
directory could not be created or is not private: check that `$XDG_RUNTIME_DIR` (or the temporary
directory) is writable and yours.

**`session ended`.** The call came after its session ended: the step settled or was cancelled,
the chat ended, or htui quit. Nothing was written.

**`fenced: lease lost`.** Another process took the run over while this session was still running
(it had lost its lease, for example after a long database outage). Nothing was written; the run
goes on in the process that holds it. `command_run` never answers this: a command the session
started meanwhile is [not fenced](#scope-what-a-session-can-touch).

**`version mismatch: the host is htui <x>, this relay is <y>; restart the agent`** (exit 3). The
binary the agent started is not the build the host runs: htui was upgraded on disk while it was
running, with a release or relay change between the two builds. On Linux the agent always starts
the host's own binary (`/proc/<pid>/exe`), so this happens on other platforms. Restart htui, so
that the host and the binary on disk are the same build.

**`no live htui session has this token` (exit 3).** The agent restarted its MCP server after its
session ended, or someone ran `htui mcp` with an old token.

**`cannot reach the htui host at <address>: …` (exit 1).** The htui process that started the
session has exited or crashed, or its socket directory was deleted while it ran.

**`HTUI_MCP_ADDR is not set` (exit 2).** `htui mcp` was run by hand, or by an agent that does not
pass the server's environment.

**`search unavailable: <cause>`.** The concepts index cannot answer. Store a Qdrant URL in
Settings › Qdrant and build the index (`htui --index-items`); see the README's
[Search](../README.md#search).

**A long `command_run` call is abandoned by the agent.** Its MCP client timed out before the
command finished: see [Long calls](#liveness-crashes-and-cancellation). The queue cancels the row
and kills the command when the call is cancelled or the session ends.

**`command_run` waits for ever.** Another command of the same class on the box holds the slot.
Its row is reaped 30 seconds after its host stops renewing it; a live one ends when it finishes or
times out (at most 30 minutes). Raise the class's [limit](#limits) to run more at once.
