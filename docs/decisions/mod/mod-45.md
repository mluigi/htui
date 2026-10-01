# MOD-45 - Remote box provisioning over SSH (done, 2026-10-01)

**Requirements:** `R-BOX-1`, `R-BOX-4`, `R-AGT-9`, `R-ORCH-12`, and `R-STO-1`, which this item
amends (a TUI session may read the DSN once from stdin).
**Origin:** ANA-16 §8 item 6 (`docs/ANA-16.md`). It depends on MOD-41 (`htui worker`), MOD-7
(id-keyed `register_box`, the box probe) and MOD-22 (loopback paste-back), all done.
**Artifacts:**
- PRD `.claude/prds/mod-45-remote-box-provisioning.prd.md`: decisions D1–D5, two milestones.
- Plan `.claude/plans/mod-45-remote-box-provisioning.plan.md`: decisions D291–D314, 38
  fact-checked claims (tree and external tools), 10 of them amended before CONFIRM.
- Blueprint `.claude/plans/mod-45-remote-box-provisioning.blueprint.md`: hazards E-1…E-24. The
  orchestrator added E-25 and E-26 during implementation.
- Review record `.claude/plans/mod-45-review-findings.md`.

**Routing:** routed as **PRD** (C2 and C4 fired, low confidence at the threshold). Ultracode was
used for the implement phase: one workflow per task (implement → conformance and adversarial
lenses → repair), run in waves A (T1, T5) → T2 → T3 → T4. The review fix round ran as a workflow
of the same shape. Sandbox run `hr/MOD-45`.

## What was built

`htui provision <destination> [--dsn-stdin] [--replace-credential]` turns a Linux host reachable
over ssh into an executing box with one command. It drives the user's system `ssh` through four
short sessions, and htui never opens `ssh` to that host again afterwards.

1. **PREFLIGHT** (unprivileged, read-only). Facts come back as `htui.<key>=<value>` lines, and
   login-shell noise is ignored. The facts are: OS, arch, systemd version, `systemd-creds`,
   `sha256sum`, user, group, home, the installed binary's sha, the unit, `is-active`, the sudo
   mode (`sudo -k -n true` → nopasswd / password / none), and whether `printf` is a builtin.
   `plan::decide` is pure and table-tested. It refuses with exit 2, before any remote write, on:
   - not Linux, an arch mismatch, or systemd < 250;
   - no `systemd-creds`, no `sha256sum` or no sudo;
   - a password sudo whose `sh` has no `printf` builtin;
   - a worker already running from a unit htui did not write;
   - a different build already installed.
2. **PREPARE** (unprivileged) creates `~/.local/state/htui` and `~/.local/bin`. When the sha
   differs, it streams the local binary (`/proc/self/exe`) on stdin. The upload is checked with
   `sha256sum` (exit 6) and must run `--version` on the host (exit 3, the loader's last stderr
   line) before `mv` puts it in place.
3. **INSTALL**. With a password sudo, the script reads line 1 itself, then runs `sudo -k` and
   `printf "%s%s" "$pw" "$nl" | sudo -S -p "" -v`. sudo's retries read EOF from that pipe, so
   the DSN, still unread on stdin, can never become a second password attempt. Then
   `sudo -n sh -c INSTALL_ROOT … || rc=$?` runs. `sudo -n` is never the script's last command,
   because bash as `/bin/sh` would exec it and lose the ppid-keyed timestamp (E-4). As root,
   INSTALL_ROOT does four things:
   - prints `htui.root=start`, which tells a sudo refusal apart from a root-script failure;
   - encrypts the remaining stdin (the DSN) with `systemd-creds encrypt` into
     `/etc/credstore.encrypted/htui-dsn`, or discards it when a credential exists and
     `--replace-credential` was not given;
   - writes `/etc/systemd/system/htui-worker.service` (the guide's system unit with `User=`, since
     user-scoped encrypted credentials need systemd 256);
   - runs `daemon-reload` and `enable --now` (plus `restart` on a replaced credential).
4. **VERIFY** (unprivileged) runs an in-script loop until `is-active` is `active` and `box.toml`
   exists, then prints `box.toml`. On timeout it exits 5 with the journal and `worker.log` tails.

Back on the local side, a Postgres baseline is taken **after INSTALL**, so that an old worker or a
TUI open on the box cannot fake a check-in. The command then waits for the new box id to appear, or
for its `last_seen_at` to move past the baseline. Both readings are the database's clock. The
deadline is 90 s, which is `BOX_HEARTBEAT` + 30 s. It then sets that box's executor to `worker` with
`edit_box` (CAS, skipped when it is already `worker`). If Postgres is unreachable from the
provisioning machine, the run warns and exits 0.

A re-run on a host that already has this build runs PREFLIGHT and VERIFY only, then sets the
executor (E-25). This way a run that failed while waiting for the worker still gets finished.

**Secrets.** The DSN and the sudo password reach the remote only on ssh stdin. The scripts are
constants with no backslash and no single quote. Values reach them only as `shell_words`-quoted
positional arguments. The DSN and password are held in `Zeroizing` locally. `Ctx`, `Payload` and
`StartOptions` have redacting `Debug` impls. Remote text is stripped of control characters before
it is printed. No `htui provision` failure is reported to Sentry, because its sentences carry
`user@host` and remote log tails.

**Robustness.** ssh runs with `--` before the destination, `ConnectTimeout=15` and
`ServerAliveInterval=15`. Every session has a bound (PREPARE allows 2 s/MiB). A session past its
bound gets SIGTERM, sent through `sh -c 'kill'` (no `unsafe`), before `kill_on_drop` kills it.
stdin is written while stdout and stderr drain. The scripts' temp files are removed by
EXIT/HUP/INT/TERM traps.

**The TUI's `--dsn-stdin` (OQ-1 (a)).** `htui --dsn-stdin` reads one line, scans it with
`Dsn::parse` before sqlx sees it, and hands it to `StartOptions.dsn` without a copy. Reconnects
reuse that copy and never read the keyring. This is what makes agent login on a provisioned box
possible:

```
ssh -t <dest> '~/.local/bin/htui' --dsn-stdin
```

then MOD-22's paste-back in Settings › Agents. `R-STO-1` was amended for it.

**Store helpers.** `identity::parse_box_toml`, plus `Dsn::host_class` → `DsnHost { Remote, Loopback,
Socket }`, which classifies the host sqlx will actually dial, including `?host=`/`?hostaddr=`
overrides. Loopback also covers `localhost`, `127/8` with `inet_aton` shorthands (`127.1`,
`2130706433`), `::1`, `::`, and mapped `::ffff:127.x`/`::ffff:0.0.0.0`. A loopback, socket or
hostless DSN is refused before any ssh session.

**Guide.** `docs/htui-worker.md` has a new section, "Provisioning a remote box". It covers:
requirements (a POSIX login shell, sudo, systemd ≥ 250, `sha256sum`, a builtin `printf` with a
password sudo, no `log_input`); the paths; the exit codes 0/2/1 and their sentences; re-running;
the executor; agent login. `README.md` has `provision` and `--dsn-stdin` rows.

## Where it deviates from the plan, and why

- **The baseline moved from before PREPARE to after INSTALL** (review finding 3). The deadline
  rose from 60 s to 90 s so that a fast new worker in the baseline is still seen at its next
  heartbeat. When the box was already in the baseline, a `note:` says the check-in is best-effort.
- **E-25:** an already-provisioned re-run verifies the box and sets the executor, instead of
  stopping after PREFLIGHT. `Verifier::set_executor` returns `Result<bool, String>`.
- **D314's scope guard was amended:** `htui-store/src/connect.rs` gained the redacting
  `StartOptions` `Debug` (review finding 7). `htui-worker`, `htui-orch`, `htui-agent` and
  `htui-core` have no functional change (`7e5dd59` is only rustfmt drift left by the MOD-41 merge).
- **The HANDOFF item and ANA-16 say "a user service".** The install is a **system** unit with
  `User=`, per the existing guide (PRD D1, plan "Where the HANDOFF or tree disagree" 1).

## Verification

- **Workspace gate** at `bd66bff`, run on the real tree:
  - `cargo fmt --check` and `cargo clippy --workspace --all-targets --all-features -D warnings`
    are clean.
  - `cargo test --workspace --all-features -- --test-threads=1` passes: **3184 passed, 0 failed**
    across 103 binaries, against Postgres on `localhost:5439`.
  - There is one `sha2` 0.10.9, and the docs validator is clean.
- **MOD-45 tests:**
  - 84 lib tests (`provision::`, `cli::`, `tests::session_dsn`).
  - `tests/provision.rs`: 14 end-to-end scenarios. They run the **real** scripts through a
    local-shell `Remote` against stub `sudo`, `systemctl`, `systemd-creds`, `uname`, `id`,
    `journalctl` and `printf`. A sentinel sweep after every scenario checks that the DSN and the
    password appear only in the encrypted credential: never in a command, argv/env log, output or
    other file. The scenarios cover password/NOPASSWD sudo, `/bin/sh` = bash (E-4, which proves it
    bites), re-run, replace-credential, a wrong password (nothing privileged written, DSN never
    read), arch mismatch, a binary that does not run, a corrupted upload, a different build, a
    chatty login shell, a service that never starts, a `sh` without a `printf` builtin, and a hung
    session.
  - `tests/provision_pg.rs`: 5 Postgres cases for the baseline, `box_seen` and `set_executor`.
- **Review gate:** `rust-reviewer` said "approve with fixes", with no CRITICAL or HIGH findings. All
  14 findings were applied (3 MEDIUM: Sentry egress, unbounded sessions, false "verified"). Two
  verifier lenses then found 5 more lows, all fixed.

## Not done here

- **The live check on a real host.** The sandbox has no ssh, sudo or systemd, so this is the
  maintainer's job after `scripts/hr collect MOD-45`. Record the result here when it has run.
  - Run `htui provision <LAN host>` on a host with C sudo, and on one with sudo-rs if available
    (Ubuntu 25.10+/26.04), and on a host whose `/bin/sh` is bash (Fedora/RHEL).
  - Expect `htui worker ready` in the journal and the box in Settings › Boxes with executor
    `worker`.
  - A re-run prints `already provisioned with this build; box <id> …`.
  - A wrong sudo password fails with nothing under `/etc`.
  - `ps -ef` / `/proc/*/cmdline` on both ends never show the DSN or the password.
  - `sudo -l` shows no `log_input`.
  - Run `ssh -t <host> '~/.local/bin/htui' --dsn-stdin`, then log an agent in through paste-back.
- **Out of scope by PRD D5:** upgrading an installed worker, uninstall, non-Linux targets (MOD-16),
  and a TUI entry point.
- **Follow-up candidates, not minted.** Each is the maintainer's call.
  - E-15: on a `--dsn-stdin` TUI session, Settings › Connection still reads the keyring and shows
    "not stored".
  - E-11: should "service active but the box never checked in" be exit 1 rather than a warning?
  - E-16: sudoers `log_input` would record the DSN in sudo's I/O log. The guide warns; a FIFO
    hand-off would remove the exposure.
  - An existing workspace test (not MOD-45's) leaves an empty `~/.config/htui/trees/` behind
    after `cargo test --workspace`.

## Commits

`fcada1d` PRD · `4b41f61`, `ce115bc` plan · `4b4486a` blueprint · T1 `5079445` `8495e99` `33e5f3c`
`5bf946a` `1088f31` · T5 `3b51340` `aedbfcb` `24cca68` · T2 `2722f38` `9017d38` `09a0527` `abbc84f` ·
T3 `f6852e6` `be5cf18` `2194269` `9f23ddf` `2221a70` `f28a4aa` `8a02839` `bc66793` `f02d8df` · T4
`163efb2` `d2803e7` · review `b9c22fa` `3d341ce` `b45e8cc` `cdce7ee` `01bc3fa` `d8b436c` `5fdad77`
`2579479` `cd2093d` `a10cc2c` `51e88d2` `6077c8b` `bd66bff` · style `7e5dd59`.
