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
  written. Start the TUI once to apply migrations, or upgrade the binary.
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
- Stops on SIGINT or SIGTERM (Ctrl-C, Ctrl-Break, closing the console or a system shutdown on
  Windows): it cancels its walks with a short grace period and gives their leases back, then exits
  0. A step it interrupted is reset and retried by the next sweep on the box.

What it does not do yet:

- **No permission answers until MOD-42.** An engine-driven ACP step fails on its first permission
  request, exactly as it does under the TUI today. Cancelling a run the worker is walking also
  waits for MOD-42 (see [`c` below](#the-tui-on-a-worker-box)).
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
| `p` promote | Unchanged. A parked step promotes and its chat runs in the TUI; a step the worker is walking refuses, because its lease is held. | — |
| `c` cancel | A `queued` or parked run cancels as before. A run the worker is walking is refused: "the worker on this box is walking run …; cancelling a live run needs MOD-42's cancel command". Nothing is written. | Keeps walking. |
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
| `2` | A startup refusal: no DSN, a connection that could not be made, a schema or build the database refuses, a `--log` file that cannot be opened, a signal handler that cannot be installed, or a command line `htui` refuses (a usage error, such as `htui --log PATH worker`). A schema or build refusal writes nothing to the database. Other refusals may not be clean: a first start mints `box.toml` before it connects, and a refusal while seeding or registering this box comes after some of those rows are written, as the TUI's start would write them. |
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
cancel falls back to the leased path, which refuses a live walk and writes nothing. When the cancel
wins, it takes two writes: the run becomes `cancelled`, then the item goes back from `queued` to
`open`. If the process or the database dies between the two, the item is left `queued` with no
active run. No command recovers that: `R` needs the item `open` or `failed`, and `u` needs it
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
`worker`, `--log` goes after `provision`.

**What the remote host needs.**

- Linux, with the same CPU architecture as this machine: x86_64 or aarch64 (`arm64` counts as
  aarch64). Provisioning ships the binary you run, so this machine must run a Linux build too. The
  binary is uploaded unless the host already has the same build, and its size is printed; a
  release build is much smaller than a debug one.
- systemd 250 or later, with `systemd-creds` on the `PATH`.
- sudo (a password prompt is fine). When sudo wants a password, `htui` asks for it here, with no
  echo: `[sudo] password for <user> on <destination>:`. `Esc` or `Ctrl-C` cancels before anything
  is written. A sudo that asks a second question (a one-time code) or insists on a tty
  (`requiretty`) fails safely: exit 1, and nothing privileged is written. Give a host that asks
  for a one-time code NOPASSWD. NOPASSWD does not get around `requiretty`; turn it off for that
  user instead (`Defaults:<user> !requiretty`).
- A POSIX login shell on the remote user (sh, bash, zsh, ksh); fish and csh are refused.
- A user and group name matching `[a-z_][a-z0-9_-]{0,31}`, and a home directory whose path uses
  only `A-Z a-z 0-9 _ . / -` with no `..`. The user and the home are written into the unit file;
  the group is checked the same way.

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
`box.toml` within 60 s. The box is then checked in Postgres from this machine, which registers or
refreshes this machine's own box, as `htui --index-items` does. When the new box shows up, or its
`last_seen_at` moves past what it was before the install, its executor is set to `worker` and
`htui` prints, on standard output:

```
box <id> (<hostname>) provisioned on <destination>
```

Otherwise the same line is printed, the exit is still 0, and a warning says what was not done:

- `warning: service active, box <id>; not verified in Postgres from here: <reason>`, followed by
  `set box <id>'s executor to worker in Settings › Boxes`. The reason is either why this machine
  could not use the DSN (it reaches Postgres only from the remote network, say), or
  `box <id> did not check in within 60 s; see journalctl -u htui-worker and
  ~/.local/state/htui/worker.log on <destination>`.
- `warning: box <id>'s executor is not set to worker: <reason>; set it in Settings › Boxes`, when
  the box checked in but the write did not apply.

An active service with a `box.toml` does not prove the worker connected: it writes `box.toml`
before it connects, and a worker refused at start (exit 2) is restarted every 10 seconds and looks
active in between. When the box did not check in, read the worker's log on the host
(`journalctl -u htui-worker` may need sudo; `~/.local/state/htui/worker.log` does not), fix the
cause, and flip the executor in **Settings › Boxes** once the box shows up there.

**Re-running.** On a host already provisioned with this build and running, it says so and changes
nothing. `--replace-credential` re-encrypts the DSN and restarts the service. A host with a
different build, or one set up by hand, is refused: upgrading is not supported yet. The binary in
`~/.local/bin/htui` is replaced on a host without the unit. The first case prints
`<destination> is already provisioned with this build; nothing was changed`. A run that failed
partway is completed by running it again: the binary is not sent twice, an existing credential is
kept unless you pass `--replace-credential`, and a unit that is installed but not running is
written again and started. A run that failed while waiting for the worker may leave the service
running; the re-run then says it is already provisioned and does not set the executor. Set it in
**Settings › Boxes**, or re-run with `--replace-credential`, which goes through the check again.

**Exit codes.**

| Code | Meaning |
|---|---|
| `0` | Provisioned, or already provisioned. A warning on standard error may still ask you to set the executor by hand. |
| `2` | Refused; nothing was written on the remote host. The sentence starts `not provisioning <destination>:` and names the reason, among them: a destination that is empty, starts with `-` or contains whitespace, no DSN, a keyring that could not be read, a DSN the remote host cannot use, a local `htui` binary that cannot be read or is not a Linux build, `ssh` that could not be run (`cannot run ssh: …`) or could not connect (`ssh to <destination> failed (exit 255): …`), a login shell that did not run the preflight, the host's OS, architecture, systemd, `systemd-creds` or sudo, a different build already installed, a user, group or home that cannot go into a unit file, or a cancelled password prompt. |
| `1` | Failed after a remote write; re-running completes it (see **Re-running** for the executor). The sentence starts `provisioning <destination> failed:`. |

The failures:

- **The binary does not run there** (often an older glibc):
  `the htui binary does not run on <destination>: <its last error line>`. The uploaded copy is
  removed; a binary already installed stays.
- **sudo refused the password:** `sudo refused the password (or needs a tty or a second factor) on
  <destination>; nothing privileged was written`. The DSN was not read. A session that drops
  before the privileged part starts reports the same sentence.
- **The service did not start:** `the htui-worker service on <destination> did not start with a
  box.toml within 60 s (systemctl says <state>); its last journal and log lines follow:`, then the
  last lines of `journalctl -u htui-worker` and of `~/.local/state/htui/worker.log`.
- **ssh dropped**, or a step failed for another reason: `preparing <destination> failed (exit
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
