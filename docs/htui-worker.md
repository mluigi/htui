# `htui worker`: running runs without a terminal

`htui worker` is the headless half of `htui`. It runs on a box (a machine with its own `box.toml`),
connects to your Postgres, and walks that box's runs with no terminal attached, so a run outlives
the TUI session that started it. It is part of `htui`, started by you and talking only to your
Postgres; it is not a hosted service (`docs/REQUIREMENTS.md` R-ID-2, R-ORCH-12).

```
htui worker [--pool-size N] [--dsn-stdin] [--log PATH]
```

- [What it does, and what it does not do yet](#what-it-does-and-what-it-does-not-do-yet)
- [Who executes on a box: the executor setting](#who-executes-on-a-box-the-executor-setting)
- [The TUI on a worker box](#the-tui-on-a-worker-box)
- [Permission requests on worker steps](#permission-requests-on-worker-steps)
- [htui's MCP tools on worker steps](#htuis-mcp-tools-on-worker-steps)
- [Cancelling a run the worker walks](#cancelling-a-run-the-worker-walks)
- [Upgrading: migrate from a TUI first](#upgrading-migrate-from-a-tui-first)
- [Where the DSN comes from](#where-the-dsn-comes-from)
- [Pool size](#pool-size)
- [Logs](#logs)
- [Exit codes](#exit-codes)
- [A run whose resume keeps failing](#a-run-whose-resume-keeps-failing)
- [A queued cancel interrupted halfway](#a-queued-cancel-interrupted-halfway)
- [Error reports](#error-reports)
- [Provisioning a remote box](#provisioning-a-remote-box)
- [Running it as a systemd service](#running-it-as-a-systemd-service)

## What it does, and what it does not do yet

What it does:

- Connects headless: it never upgrades the database and never asks anything. A pending, newer or
  dirty schema, or an `htui` build older than the database requires, is refused before anything is
  written. Start the TUI once to apply migrations, or upgrade the binary (see
  [Upgrading](#upgrading-migrate-from-a-tui-first)).
- Uses the same `box.toml` as the TUI (see [Where htui keeps its files](../README.md#where-htui-keeps-its-files)),
  so it **is** the TUI's box. Run it as the same OS user, with the same configuration directory,
  as the TUI whose runs it should take over. A worker that mints its own `box.toml` is another box,
  and never sees the TUI's runs.
- Every 5 seconds, when the box's executor is `worker`: claims the box's queued runs in the order
  they were queued, adopts the box's `running` runs that nobody holds (a lease given back or
  lapsed), and walks them. It refreshes each walk's lease and fences itself before its lease can
  lapse, exactly as the TUI does.
- Picks up a repo or checkout change (a repo added, a checkout path set or moved) at the next
  sweep when no run is walking, with no restart; the box's command limits and
  `copy_max_total_bytes` are read again at that point. While a run is walking, the worker keeps
  adopting but its sweep claims no queued run until its walks rest, so no walk has its checkouts
  moved under it. A run whose claim the worker already had to put back (its slot full, or its
  scope overlapping a walk) is still claimed when one of the walks rests, with the checkouts the
  worker read before the change. A change to the limits alone reaches the worker at its next
  restart.
- Marks the box as seen (`box.last_seen_at`) once at start and then every minute, whatever the
  executor.
- Logs `htui worker ready` once connected, with the box id and the pool size.
- Hosts htui's MCP tools for the sessions it starts, on a private local socket; nothing listens
  on the network (see [htui's MCP tools](#htuis-mcp-tools-on-worker-steps)).
- Parks a step whose agent asks for a permission its policy does not settle, until someone
  answers it from the **Runs** pane of any TUI that holds the DSN (see
  [Permission requests](#permission-requests-on-worker-steps)).
- Applies a cancel requested from any TUI: it reads requests every second and stops the walk
  gracefully (see [Cancelling](#cancelling-a-run-the-worker-walks)).
- Stops on SIGINT or SIGTERM (Ctrl-C, Ctrl-Break, closing the console or a system shutdown on
  Windows): it drops its walks at once, waits a short grace period for their tasks to end and
  gives their leases back, then exits 0. A step it interrupted is reset and retried by the next sweep on the box. This stop is a hard
  drop, not a graceful cancel: a step parked on a permission request is not answered first (see
  [below](#permission-requests-on-worker-steps)).

What it does not do yet:

- **No remote targeting until MOD-43.** A run targets the box that started it, and a worker only
  takes runs targeted at its own box. The TUI cannot yet send a run to another machine's worker,
  and nothing in the TUI tells you whether a worker is running.
- It does not install itself; `htui provision <host>` installs it on another Linux machine (see
  [Provisioning a remote box](#provisioning-a-remote-box)). The
  [sample unit](#running-it-as-a-systemd-service) is for doing it by hand.

## Who executes on a box: the executor setting

Each box has an **executor**, `tui` (the default) or `worker`, stored in the box's settings in
Postgres. Change it in **Settings › Boxes** with `w`, then `y` to confirm (`n` or `Esc` cancels).
`w` proposes the other value; on a value this build does not know, it proposes `tui`.
`htui provision` sets it to `worker` for the box it installs, once that box checks in (see
[Provisioning a remote box](#provisioning-a-remote-box)).

**The rule (I-1):** on a box, only the process whose role matches the executor claims queued runs,
adopts runs nobody holds, or sweeps. With `tui`, the TUI does; with `worker`, `htui worker` does.
Leases and fences, not this rule, are what keep the store correct: if two processes ever act on one
box (during a flip, or on a misconfigured box), the second one's writes are still refused. The rule
makes ownership single, so a TUI exit never interrupts a worker's run.

The executor is read on its own, so a malformed neighbouring setting never turns a `worker` box
back into a `tui` box. A value neither `tui` nor `worker` (written by a newer build, or by hand)
fails closed: **neither** process executes there. The TUI refuses `R` and the walking commands with
"box executor `…` is not known to htui <version>", and the worker idles.

**A worker on a `tui` box idles.** It keeps marking the box as seen, logs the executor once each
time it changes, re-checks it at every poll and exits 0 on a signal. So you can start the worker
before you flip the setting, and a flip in either direction needs no restart.

**Flipping never moves a live lease.**

- **`tui` → `worker`.** Walks the TUI already started keep their leases and heartbeats and run to
  their next rest in the TUI. The TUI's next sweep skips the box, its next command hands back (see
  below), and a run it had queued but not yet claimed stays queued for the worker. When the TUI
  exits, it gives its walks' leases back, and the worker adopts those runs at its next poll; a step
  that was interrupted is reset and retried.
- **`worker` → `tui`.** At its next poll the worker stops claiming and sweeping. Walks it already
  started keep heartbeating until they rest, then it idles. From its next sweep the TUI adopts runs
  whose lease was given back or has lapsed, and claims the box's queued rows, including those it
  queued for the worker, so nothing is stranded.

## The TUI on a worker box

On a box whose executor is `worker`, the TUI still records every decision you make, under the
run's lease, but it no longer walks what comes after. It gives the lease back instead (a
**hand-back**), and the worker adopts the run at its next poll, within about 5 seconds. The worker
finishes it exactly as it would finish a run whose process crashed at that point.

| Key | What the TUI does on a worker box | What the worker then does |
|---|---|---|
| `R` start | Creates the run `queued` for this box. The Runs pane shows `queued`. | Claims it and walks it. |
| `a` approve, `x` reject | Records the answer, gives the lease back. | An approval merges the step's work and walks on; a rejection loops back or fails the run, as the graph says. |
| `A` accept | Runs the verification in the TUI, records the acceptance, then as `a`. | As `a`. |
| `r` retry | Records the retry and creates the new attempt, gives the lease back. | Walks the new attempt. |
| `s` select | Records the chosen fan-out result, gives the lease back. | Merges the chosen result and walks on. |
| `u` unblock (resume) | Lifts the park only, gives the lease back. | Re-checks the live graph and walks on. `u` that reopens an item or follows a run is unchanged. |
| `p` promote | Unchanged. A parked step promotes and its chat runs in the TUI. A step the worker is walking is refused: "the worker on this box is walking run …; a live step is promoted only by the process that walks it". A live session cannot move between processes. | Keeps walking. |
| `c` cancel | A `queued` run, or one nobody holds, cancels at once as before. A run the worker is walking gets a **cancel request**: the status line says "cancel requested: the run's executor applies it", the Runs pane shows `cancel requested` under the run, and a second `c` says "a cancel is already requested". | Applies it within about a second, gracefully (see [Cancelling](#cancelling-a-run-the-worker-walks)). |
| `1` … `9` | On a step whose session asks for a permission: answers it with that option (see [Permission requests](#permission-requests-on-worker-steps)). | Applies the answer within about a second and the step goes on. |
| `C`, `T`, `o` | Unchanged. | — |

**Cleanup is not serialised with the worker.** The TUI's cleanup (`c` on a run that has rested,
and `T`) still runs `git worktree remove` in the TUI, and nothing orders it against the worker's
`git worktree add` on the same repository: the locks that order them are per process. A rare race
fails one candidate with a readable error, or leaves a tree behind that `T` removes when you run
it again. A lock across processes is deferred.

Two consequences of recovering "as after a crash":

- **A graph changed mid-run.** The worker re-checks the item's **live** step graph before it walks
  on. If the graph's shape was edited after the run started, the run parks with a
  "topology mismatch" note instead of walking on its original snapshot, as the TUI would have done
  in process. An approval or selection has already been merged by then. The park is sticky: `u`
  goes through the same check. Cancel the run (`c`) and start a new one.
- **A lost hand-back.** If giving the lease back fails (the database dropped at that moment), the
  worker cannot adopt the run until the lease lapses on its own, after the lease's full
  time-to-live instead of at its next poll. Nothing is lost; the run is just late.

**The Runs pane refreshes itself** every 5 seconds while the selected item has a `queued`,
`running` or `awaiting_approval` run, so a worker's progress shows up without re-selecting the
item. This is a re-read, not a liveness signal: a run queued on a worker box with no worker running
stays `queued`, and the pane cannot tell you why (MOD-43).

## If the worker crashes

A worker killed mid-run (`kill -9`, an OOM kill, a power cut) loses **at most the step it was
walking**. Nothing is resumed inside the agent's session: the session, its process and any
permission request it was parked on die with the worker. Once the run's lease lapses (its
time-to-live, `lease_ttl_seconds`, 120 s by default), the next worker or TUI on the box that sweeps
adopts the run and decides from what reached Postgres and the step's tree:

| The worker died | What recovery does |
|---|---|
| after the step's session started, before or after some of its log was written | The step fails `interrupted` (its log so far is kept), the item gets a note "… did not finish; … retrying as attempt N", and a new attempt runs in a fresh tree. With the retry budget spent, the run parks instead. |
| after the step's output document was written, before its work was captured | As above: the document alone does not finish a step. |
| after the step's work was captured (every repository's commit recorded) and its document written | The step is settled `done` without a new session, and the run walks on. |

A step whose checkout was used in place and was not clean when the step started is never reset:
the run parks with the tree's path and its starting commit in the note.

**A cancel survives a crash.** A cancel requested while the worker was walking the run, and not yet
applied when it died, is applied by the restarted worker **before** it recovers anything: the run
ends `cancelled`, never walked on and never finished by the recovery itself.

These outcomes are pinned by `crates/htui/tests/worker_crash_pg.rs`, which kills a real worker
process at each of these points and restarts it (MOD-24).

## Permission requests on worker steps

An agent may ask for permission before it runs a tool: an ACP agent over ACP, a `claude-cli` agent
through htui's `permission_prompt` tool (see
[`docs/htui-mcp.md`](htui-mcp.md#permission-prompts-for-claude-cli)). On a step the worker walks,
as on one the TUI walks:

1. **The agent's own policy decides first**, the same policy its chat uses: a rule that allows or
   rejects the call answers at once, and the answer is recorded in the step's log as the policy's.
2. **Otherwise the step parks and waits for a person.** The worker writes the request to Postgres
   (the tool and its title, scrubbed of secrets like the rest of the log, and the options the agent
   offers) and keeps the session open. The **Runs** pane of any TUI connected to the same database,
   on this box or another, shows it under the step: `asks: <what>`, then the options numbered as
   in a chat. Put the cursor on the step and press the option's digit. The worker reads the answer
   within about a second, hands it to the agent, records it in the step's log as yours, and the
   step goes on. The pane refreshes itself every 5 seconds, so a new request shows up within that.

Things to know:

- **One answer wins.** If two people answer the same request, the first is applied; the second is
  told what happened on the status line (for example "this permission request was already
  answered") and nothing of it is kept.
- **A parked step waits for ever.** There is no timeout. It holds its run's lease and its slot on
  the box meanwhile; cancel the run (`c`) to give both back.
- **Offline there is nothing to answer.** A TUI without a database connection shows no requests,
  and no error either.
- **Only the process walking the step can apply an answer.** A request whose process stopped or
  lost the run can no longer be answered ("the process that asked no longer holds the run") and
  drops out of the pane. When the step is walked again, its new session asks again.
- **Stopping the worker does not answer a parked request.** SIGINT or SIGTERM drops every walk at
  once, a parked one included; the request is left unanswerable and disappears from the pane once
  the lease is given back. The next sweep resets the step and retries it, and its session asks
  again. Cancel the run first if you want it to end as `cancelled` instead.

## htui's MCP tools on worker steps

Every session the worker starts gets htui's MCP server, `htui`, and the worker itself answers it:
the agent writes its document, notes and link proposals through the worker's own connection and
under the walk's lease, and never sees the DSN. What the tools do, and which session gets which, is
in [`docs/htui-mcp.md`](htui-mcp.md).

- **Nothing listens on the network.** The worker opens one local listener, at its first session: a
  Unix socket in a `0700` directory `htui-mcp-<pid>-<8 hex>` under `$XDG_RUNTIME_DIR`, else the
  temporary directory (a named pipe on Windows), and every connection must present its session's
  token. ANA-16 §5.4's "no worker listener socket" is about a control plane other machines could
  reach; this socket is per process, local, gated per session, and gone when the worker stops
  (MOD-11 OQ-1).
- **Under the [systemd unit](#running-it-as-a-systemd-service)** a system service has no
  `XDG_RUNTIME_DIR`, so the directory is in `/tmp`. A worker that is killed leaves it behind; it is
  safe to delete once that pid is gone (see [The socket](htui-mcp.md#the-socket)).
- **A worker that cannot host the tools refuses to start** (exit 2, "htui's MCP tools cannot be
  hosted: …"): every document phase would fail `missing_output` without them.
- **`search_concepts`** is offered only when the keyring held a Qdrant URL at start (see
  [The concepts index](#the-concepts-index)), so a provisioned worker never offers it.
- **`command_run`** runs on this box, as the worker's user and with the worker's environment, in
  the box's command queue, which the worker shares with a TUI on the same box. A worker killed
  mid-command holds its slot until the next command of that class asks, 30 seconds later at the
  earliest.

## Cancelling a run the worker walks

`c` on a run the worker is walking (on a `worker` box, or from a TUI on another box) does not stop
it from the TUI: a live session cannot be reached from another process. Instead the TUI writes a
**cancel request** to Postgres, which survives the TUI exiting, and the process that walks the run
applies it:

- **The worker reads cancel requests every second.** It stops the run's walk **gracefully**: a
  permission request the step is parked on is answered `cancelled` (and recorded in the step's
  log), the agent is asked to cancel its turn and given 2 seconds to wind down, and the walk is
  then dropped. The run and its live steps become `cancelled`, as an in-process cancel would make
  them, so a cancel usually takes effect within about 3 seconds (the 2-second grace plus one
  poll), and at most a second later if the agent does not wind down.
- **A request whose executor is not running waits.** It stays pending, with no timeout, and the
  Runs pane keeps showing `cancel requested`. If the worker died holding the run, the request waits
  until the run's lease lapses; from then on the next process on the run's box that reads cancel
  requests applies it: the worker when it starts again (before it recovers the run, see
  [If the worker crashes](#if-the-worker-crashes)), or a TUI open on that box. A run walked on
  another box is cancelled only by a process on that box.
- **A run that ended first is not cancelled.** If the run reached `done` (or failed) before its
  executor read the request, the request is refused with the run's actual status and the run is
  left as it ended.
- **Shutdown is not a cancel.** Stopping the worker drops its walks at once (see above); a pending
  cancel request is applied by the next process on the box that reads cancel requests.

A `c` on a run the TUI itself walks takes the same graceful path, at once, in the TUI.

## Upgrading: migrate from a TUI first

The worker never changes the database schema (it has no one to ask). So when a new `htui` brings a
migration, upgrade in this order:

1. **Upgrade the TUI and start it once.** It shows the pending migrations and applies them when you
   confirm.
2. **Then start (or restart) the upgraded worker.** Until the database is migrated, a worker built
   with the new migration refuses to start with exit 2 ("… schema migration(s) are pending, and a
   headless process never migrates; start `htui` once to apply them") and writes nothing. Under
   the [systemd unit](#running-it-as-a-systemd-service) below it is simply restarted every 10
   seconds and comes up once the TUI has migrated.

Upgrade the worker's binary too: a worker older than the database's schema refuses it ("schema is
newer than this htui") with the same exit 2.

For example, the permission and cancel relay above needs the migration
`0011_permission_relay.sql`: a worker built with it waits, refusing, until a TUI of the same
version has applied it.

## Where the DSN comes from

The worker reads the Postgres connection string from exactly one of these, in this order:

1. **`--dsn-stdin`:** one line from standard input, and no other source. A blank line or end of
   input is a refusal (exit 2), never a fall-through to the others. When standard input is a
   terminal, a prompt goes to standard error; the pasted text is visible.
2. **Otherwise the OS keyring entry the TUI uses**, the one `htui --set-dsn` and
   **Settings › Connection** write. A keyring that cannot be reached (a Linux host with no Secret
   Service session, typically a service) counts as "no keyring", not as an error.
3. **Otherwise, on Linux, the systemd credential `htui-dsn`:** the file
   `$CREDENTIALS_DIRECTORY/htui-dsn` that systemd decrypts for the unit (see
   [below](#running-it-as-a-systemd-service)). `CREDENTIALS_DIRECTORY` names the directory, never
   the secret.

With none of them, the worker exits 2 with a sentence naming all three. The DSN is **never** read
from `argv`, from any other environment variable or from a plain file, and it is never written to a
log or an error report. The copy htui reads is wiped once the connection is open; the Postgres
driver keeps the password in memory for the life of the process so it can reconnect.

## Pool size

`--pool-size N` sets how many Postgres connections the worker may hold. The default is 4; any value
is clamped to 2 through 8, with a warning in the log when it was moved. The pool is sized before
any setting is read from the database. The TUI keeps its own pool of 8, so keep the sum over every
running `htui` process below your server's `max_connections`.

## The concepts index

When the OS keyring holds a Qdrant URL (set in Settings > Qdrant, as for `htui --index-items`), the
worker keeps the concepts index in step with every project: once at start, then every
`concepts_sync_minutes` minutes. That is an `app_setting` key holding a positive whole number,
read again after every sync; without it, or with any other value, the interval is 15 minutes.

- The keyring is read **once**, at start. Without a URL there, or with a keyring that cannot be
  read, the worker logs one `info` line saying why and never indexes; a URL stored later is
  picked up at the next start. A host without a keyring, which is the systemd case, never indexes.
- The embedding model loads in the background, downloaded on first use exactly as for
  `--index-items`; runs never wait for it.
- A Qdrant that does not answer, a model that does not load or a sync that fails is a `warn` line,
  and the next interval tries again. None of it touches runs.
- The sync shares the worker's pool, one query at a time, and is stopped on shutdown.

## Logs

- `--log PATH` appends the log to that file, exactly as the TUI does. The directory must exist.
- Without it the worker, unlike the TUI, logs to standard error (in colour only on a terminal), so
  a systemd unit's journal has it.
- `HTUI_LOG` is read too, and `HTUI_LOG_FILTER` sets the level (default `info`).
- **`--log` goes after `worker`:** `htui worker --log PATH` works, `htui --log PATH worker` is
  refused. `HTUI_LOG` works either way. Beside `worker`, no flag of the TUI may be given
  (`htui --demo worker` is refused).

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Clean shutdown on a signal. |
| `1` | Reserved for a failure after the worker started. This build produces none: a store outage is logged, not an exit (below). |
| `2` | A startup refusal: no DSN, a connection that could not be made, a schema or build the database refuses, a `--log` file that cannot be opened, a signal handler that cannot be installed, htui's MCP tools that cannot be hosted, or a command line `htui` refuses (a usage error, such as `htui --log PATH worker`). A schema or build refusal writes nothing to the database. Other refusals may not be clean: a first start mints `box.toml` before it connects, and a refusal while seeding or registering this box comes after some of those rows are written, as the TUI's start would write them. |
| `101` | A panic that stops the process: a bug in `htui`. It is reported to GlitchTip (see [Error reports](#error-reports)). A panic inside a run's walk is reported too, but does not stop the worker: the run is adopted again, [backed off](#a-run-whose-resume-keeps-failing). |

A store outage while running is not an exit: the worker logs it and carries on, and its walks fence
themselves until the next poll succeeds.

## A run whose resume keeps failing

When the worker adopts a run whose live graph no longer resolves (the graph, a prompt template or
an agent row it needs is gone), the resume fails, the lease goes back, and the next poll would
adopt it again. So the worker backs off **per run**: it still adopts the run, but waits before
resuming it, first 5 seconds, then twice as long each time, up to 5 minutes. It logs
"a run's resume keeps failing; the worker waits before resuming it again" at `warn` once per step
up. A resume whose walk panics is backed off the same way. A resume that succeeds clears the delay; the delays are kept in memory, so a restart starts
over. A failing database is not backed off this way. Fix what the run needs, or cancel it.

## A queued cancel interrupted halfway

Cancelling a `queued` run is a compare-and-set on every box: if a worker claims the run first, the
cancel is refused ("another orchestrator holds a live lease") and writes nothing; press `c`
again, which now [requests a cancel](#cancelling-a-run-the-worker-walks) the worker applies. When
the cancel wins, it takes two writes: the run becomes `cancelled`, then the item goes back from
`queued` to `open`. If the process or the database dies between the two, the item is left `queued`
with no active run. No command recovers that: `R` needs the item `open` or `failed`, and `u` needs it
`blocked`.

**How to notice:** the item stays `queued` in the Backlog while its Runs pane shows its latest run
`cancelled`, and nothing is `queued`, `running` or `awaiting_approval`. **How to fix it**, in
`psql`, with the item's key and project slug:

```sql
UPDATE item SET status = 'open', closed_at = NULL
 WHERE key = 'FEAT-12'
   AND project_id = (SELECT id FROM project WHERE slug = 'my-project')
   AND status = 'queued'
   AND NOT EXISTS (SELECT 1 FROM run
                    WHERE run.item_id = item.id
                      AND run.status IN ('queued', 'running', 'awaiting_approval'));
```

This is the same write the cancel's second half makes; `UPDATE 1` means it applied.

## Error reports

The `htui` binary sends crash and error reports to the maintainer's GlitchTip (Sentry-compatible)
server; this build has no setting to turn that off. For the worker:

- A panic (exit 101) or a log line at `error` level is reported, with the recent `info` and `warn`
  lines as breadcrumbs.
- A startup refusal (exit 2) is **not** sent as a report: it is a configuration state, and it goes
  to standard error and the log only.
- The DSN is never in a log line, so never in a report either.

If you self-host and do not want reports to leave your network, block outbound traffic to
`glitchtip.sette.mluigi.it` for the worker's host.

## Provisioning a remote box

```
htui provision <destination> [--dsn-stdin] [--replace-credential] [--log PATH]
```

`htui provision` installs this `htui` build as the `htui-worker` system service on another Linux
machine, with the [unit below](#running-it-as-a-systemd-service), and sets the new box's executor to
`worker`. The destination is anything `ssh` accepts: a host, `user@host` or a `~/.ssh/config`
alias. It runs your own `ssh`, so `~/.ssh/config`, `ProxyJump` and your agent apply. It opens four
short sessions; with password logins you are asked once per session, so use a key or
`ControlMaster`. Progress goes to standard error and the result to standard output. As for
`worker`, `--log` goes after `provision`. Each session is bounded: `ssh` keepalives end one whose
host has gone silent for about a minute, and `htui` closes the preflight after 120 s, the upload
after 120 s plus 2 s per MiB, the install after 300 s, and the wait for the worker after its own
minute plus 120 s. The bounds count the time `ssh` spends asking for a password or passphrase; a
session past its bound is asked to stop before it is killed, so a prompt left waiting gives your
terminal back with echo on. Text from the remote host (error lines, journal lines, the hostname) is printed
with its control characters escaped.

**What the remote host needs.**

- Linux, with the same CPU architecture as this machine: x86_64 or aarch64 (`arm64` counts as
  aarch64). Provisioning ships the binary you run, so this machine must run a Linux build too. The
  binary is uploaded unless the host already has the same build, and its size is printed; a
  release build is much smaller than a debug one.
- systemd 250 or later, with `systemd-creds` on the `PATH`.
- `sha256sum` (coreutils) on the `PATH`: the upload is checked with it, and a host without it is
  refused before anything is written.
- sudo (a password prompt is fine). When sudo wants a password, `htui` asks for it here, on your
  terminal, with no echo: `[sudo] password for <user> on <destination>:`. `Esc` or `Ctrl-C`
  cancels before anything is written. A sudo that asks a second question (a one-time code) or
  insists on a tty (`requiretty`) fails safely: exit 1, and nothing privileged is written. Give a
  host that asks for a one-time code NOPASSWD. NOPASSWD does not get around `requiretty`; turn it
  off for that user instead (`Defaults:<user> !requiretty`).
- With a password sudo, a remote `sh` with `printf` built in (dash, bash, busybox and ksh93 have
  it; posh and mksh do not). The password is handed to sudo through `printf`, and an external one
  would show it in the process list, so such a host is refused: give that user NOPASSWD.
- A POSIX login shell on the remote user (sh, bash, zsh, ksh); csh and tcsh are refused. fish is
  untested and likely works, since every command is a plain `sh -c '…'`.
- No `htui-worker` service already running from a unit `htui` did not write (one that is not
  `/etc/systemd/system/htui-worker.service`): such a host is refused; stop and remove that unit
  first.
- A user and group name matching `[a-z_][a-z0-9_-]{0,31}`, and a home directory whose path uses
  only `A-Z a-z 0-9 _ . / -` with no `..`. The user and the home are written into the unit file;
  the group is only checked, and written nowhere.

**What is written where.** On the remote host, for the user you log in as:

| What | Path | Written by |
|---|---|---|
| Binary | `~/.local/bin/htui` | you |
| Log | `~/.local/state/htui/worker.log` | you (the directory); the worker writes the file |
| Unit | `/etc/systemd/system/htui-worker.service` | root, through sudo |
| Credential | `/etc/credstore.encrypted/htui-dsn` | root, through sudo |

The unit is the [sample below](#running-it-as-a-systemd-service) with that user's name and home
filled in and the binary in `~/.local/bin`. The worker mints its `box.toml` in `~/.config/htui/` at
its first start, so the new box belongs to that user. The DSN is encrypted by `systemd-creds` on
that host and is never in a command line, the environment, a log or a plain file on either
machine. The sudo password, likewise, travels only on `ssh`'s standard input. The notes under the
sample apply to this unit too; to add an `Environment=` line (`PATH`, say), use a drop-in
(`sudo systemctl edit htui-worker.service`), which a re-run leaves alone. Do not set
`XDG_CONFIG_HOME` there: provisioning assumes `~/.config`, and a worker that reads another
`box.toml` is another box, with its executor still `tui`, and a later re-run would wait on the
wrong file.

**The DSN.** From your OS keyring, or one line of stdin with `--dsn-stdin` when the remote host
reaches Postgres at another address. A DSN whose host is `localhost`, a loopback address or a Unix
socket is refused, including through a `host=` or `hostaddr=` parameter: the remote host would
reach itself, not your server. With `--dsn-stdin` on a terminal, `htui` asks for the line; the
pasted text is visible.

**Do not provision through a sudo configured with `log_input`.** The DSN reaches `systemd-creds` on
sudo's standard input, so such a sudo would record it in its I/O log (`/var/log/sudo-io`). It is
off by default; `sudo -l` and `/etc/sudoers` show it when it is set.

**Verification and the executor.** After the install, the service must be active with a
`box.toml` within about a minute. The box is then checked in Postgres from this machine, which
registers or refreshes this machine's own box, as `htui --index-items` does. When the new box shows
up, or its `last_seen_at` moves past what it was once the install had returned, its executor is set
to `worker` and `htui` prints, on standard output:

```
box <id> (<hostname>) provisioned on <destination>
```

Otherwise the same line is printed, the exit is still 0, and a warning says what was not done:

- `warning: service active, box <id>; not verified in Postgres from here: <reason>`, followed by
  `set box <id>'s executor to worker in Settings › Boxes`. The reason is either why this machine
  could not use the DSN (it reaches Postgres only from the remote network, say), or
  `box <id> did not check in within 90 s; see journalctl -u htui-worker and
  ~/.local/state/htui/worker.log on <destination>`.
- `warning: box <id>'s executor is not set to worker: <reason>; set it in Settings › Boxes`, when
  the box checked in but the write did not apply.

When the box is already in Postgres once the install has returned (a re-run with
`--replace-credential`, say, or a new worker that connected at once), only a later `last_seen_at`
counts, and an `htui` TUI on that host with the same `box.toml` moves it too. The check is then
best-effort, and a line on standard error says so: `note: box <id> was already in Postgres when the
install returned, so the check-in is best-effort: an htui TUI on <destination> with the same
box.toml would also count`.

An active service with a `box.toml` does not prove the worker connected: it writes `box.toml`
before it connects, and a worker refused at start (exit 2) is restarted every 10 seconds and looks
active in between. When the box did not check in, read the worker's log on the host
(`journalctl -u htui-worker` may need sudo; `~/.local/state/htui/worker.log` does not), fix the
cause, and flip the executor in **Settings › Boxes** once the box shows up there.

**Re-running.** On a host already provisioned with this build and running, nothing is installed
or restarted, and no sudo password is asked: `htui` reads the box's `box.toml` there (one more
unprivileged session), sets that box's executor to `worker` when it is not already, and prints
`<destination> is already provisioned with this build; box <id> (<hostname>)`, ending in
`; executor set to worker` when it changed it. When the box cannot be read there, the line is
`<destination> is already provisioned with this build; nothing was changed`; when this machine
cannot use the DSN, or the write does not apply, the same warnings as above say so. The exit is 0
either way. `--replace-credential` re-encrypts the DSN and restarts the service. A host with a
different build, or one set up by hand, is refused: upgrading is not supported yet. The binary in
`~/.local/bin/htui` is replaced on a host without the unit. A run that failed partway is completed
by running it again: the binary is not sent twice, an existing credential is kept unless you pass
`--replace-credential`, and a unit that is installed but not running is written again and started.
A run that failed while waiting for the worker may leave the service running; the re-run then
finds it already provisioned and sets the executor.

**Exit codes.**

| Code | Meaning |
|---|---|
| `0` | Provisioned, or already provisioned. A warning on standard error may still ask you to set the executor by hand. |
| `2` | Refused; nothing was written on the remote host. The sentence starts `not provisioning <destination>:` and names the reason, among them: a destination that is empty, starts with `-` or contains whitespace, no DSN, a keyring that could not be read, a DSN the remote host cannot use, a local `htui` binary that cannot be read or is not a Linux build, `ssh` that could not be run (`cannot run ssh: …`) or could not connect (`ssh to <destination> failed (exit 255): …`), a preflight that did not finish (`the preflight on <destination> did not finish within 120 s; its ssh session was closed`), a login shell that did not run the preflight, the host's OS, architecture, systemd, `systemd-creds`, `sha256sum` or sudo, a password sudo whose `sh` has no `printf` builtin, an `htui-worker` already running from a unit `htui` did not write, a different build already installed, a user, group or home that cannot go into a unit file, or a cancelled password prompt. |
| `1` | Failed after a remote write; re-running completes it, the executor included (see **Re-running**). The sentence starts `provisioning <destination> failed:`. |

The failures:

- **The binary does not run there** (often an older glibc):
  `the htui binary does not run on <destination>: <its last error line>`. The uploaded copy is
  removed; a binary already installed stays.
- **The upload arrived altered:** `the upload to <destination> was corrupted (its sha256 is not
  this build's); nothing was replaced`. The uploaded copy is checked before it runs, and removed.
- **sudo refused:** with a password, `sudo refused the password (or needs a tty or a second
  factor) on <destination> (<sudo's last line>); nothing privileged was written`; with NOPASSWD,
  `sudo refused to run the installer as root (or needs a tty) on <destination> (<sudo's last
  line>); nothing privileged was written`. The DSN was not read.
- **ssh dropped before the privileged part:** `the ssh session to <destination> dropped before the
  privileged part (exit <code>); nothing privileged was written: <its last line>`.
- **The service did not start:** `the htui-worker service on <destination> did not start with a
  box.toml within 58 s (systemctl says <state>); if the worker cannot use its DSN, fix it and re-run
  with --replace-credential; its last journal and log lines follow:`, then the last lines of
  `journalctl -u htui-worker` and of `~/.local/state/htui/worker.log`. A plain re-run keeps the
  credential already there.
- **A session did not finish in time:** `preparing <destination> did not finish within <bound>;
  its ssh session was closed`, and likewise `installing the service on <destination>` and
  `waiting for the worker on <destination>`.
- **ssh dropped later**, or a step failed for another reason: `preparing <destination> failed (exit
  <code>): …`, `installing the service on <destination> failed (exit <code>): …` or
  `waiting for the worker on <destination> failed (exit <code>): …`, with the last lines the
  session printed. `ssh`'s own failures are exit 255.

When sudo wants a password and no terminal can be opened, the run is refused (exit 2) instead:
`sudo needs a password and there is no terminal; configure NOPASSWD or run interactively`.

**Agent login on the new box.** After a success, `htui` prints the way in on standard error:
`ssh -t <host> '~/.local/bin/htui' --dsn-stdin`, paste the DSN, then **Settings › Agents**. Keep
the quotes: without them your own shell expands `~` to your home on this machine, which is the
wrong path when the login on `<host>` is another user. The DSN is held in memory for that session
only; it is neither read from nor written to a keyring, and systemd decrypts the service's
credential only for the unit. The TUI runs as the same user with the same `box.toml` (unless that
login sets `XDG_CONFIG_HOME`), so it is the same box (see
[The TUI on a worker box](#the-tui-on-a-worker-box)). **Settings › Connection** there shows the
keyring's state, not this session's DSN, and saving a DSN from it tries a keyring the box does not
have. `--dsn-stdin` cannot be combined with `--set-dsn`, `--clear-dsn`, `--demo`, `--offline`,
`--index-items` or `--search-items`.

**Not done.** Upgrades (a host with a different build is refused), uninstalling
(`sudo systemctl disable --now htui-worker.service`, then remove the four paths above by hand),
non-Linux targets on either end, and user units. The concepts index is not built on the box: the
service has no keyring, so no Qdrant URL (see [The concepts index](#the-concepts-index)).

## Running it as a systemd service

This is the unit `htui provision` writes (with the binary in `~/.local/bin`); use it by hand on a
host where sudo over ssh is not available. It is a **system** unit that runs as your user, because
user units can load an encrypted credential only from systemd 256 on, and Ubuntu 24.04 ships 255.
Check yours with `systemctl --version`.

**1. Encrypt the DSN** (as root). The credential is encrypted at rest and bound to this host (to its
TPM too, when it has one), and systemd decrypts it only for the unit:

```
sudo install -d -m 0700 /etc/credstore.encrypted
sudo systemd-creds encrypt --name=htui-dsn - /etc/credstore.encrypted/htui-dsn
```

Paste the DSN, press `Enter`, then `Ctrl-D`. Reading it from standard input keeps it out of your
shell history and the process list.

**2. Create the log directory** as your user: `mkdir -p ~/.local/state/htui`.

**3. The unit**, `/etc/systemd/system/htui-worker.service`, with `<you>` replaced by your user name:

```ini
[Unit]
Description=htui worker
Wants=network-online.target
After=network-online.target

[Service]
Type=exec
User=<you>
ExecStart=/home/<you>/.cargo/bin/htui worker --log /home/<you>/.local/state/htui/worker.log
LoadCredentialEncrypted=htui-dsn:/etc/credstore.encrypted/htui-dsn
Restart=on-failure
RestartSec=10s

[Install]
WantedBy=multi-user.target
```

```
sudo systemctl daemon-reload
sudo systemctl enable --now htui-worker.service
```

Notes:

- `User=` gives the service your home directory, so it reads the same `~/.config/htui/box.toml`
  as your TUI. If your shell sets `XDG_CONFIG_HOME`, add the same value as an
  `Environment=XDG_CONFIG_HOME=…` line.
- The unit's `PATH` is systemd's default, not your shell's. If `git` or your agents live elsewhere
  (for example `~/.local/bin`), add an `Environment=PATH=…` line.
- The service has no Secret Service session, so the keyring counts as absent and the credential is
  used. The DSN never appears in the unit file, the environment or the process list.
- `Restart=on-failure` restarts on any non-zero exit (2, or 101 after a panic), which also covers a database that is not
  up yet at boot; `RestartSec=` spaces the attempts. A clean stop (exit 0) is not restarted, and
  `systemctl stop` sends SIGTERM, which the worker handles.
- Then flip the box's executor to `worker` in **Settings › Boxes**.

**User unit, systemd 256 or later only.** User-scoped encrypted credentials need systemd 256. On
such a host, encrypt as your user with `systemd-creds --user encrypt --name=htui-dsn - <path>`,
point `LoadCredentialEncrypted=htui-dsn:<path>` at it from a unit in `~/.config/systemd/user/`,
drop `User=`, use `WantedBy=default.target`, and run `loginctl enable-linger <you>` so it starts
without a login. On an older host, use the system unit above, run the worker in the foreground with
`--dsn-stdin`, or rely on the keyring from a desktop session.
