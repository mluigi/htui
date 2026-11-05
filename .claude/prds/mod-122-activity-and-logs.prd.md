# MOD-122 — Activity panel and Logs tab

> Routed as **PRD** by `/handoff-run MOD-122` (criteria C2 and C4 fired, at threshold; accepted by
> the maintainer 2026-11-05, sandbox run `hr/MOD-122`). Ultracode accepted for the implement phase.
> Origin: `docs/ui-redesign-findings-2026-10-15.md` §4.4, §4.7, §7 rounds 4 and 6.
> Requirements: `R-TUI-15` (Logs tab), `R-TUI-16` (activity panel); touches `R-TUI-12` (layout
> presets), `R-TUI-13` (history), `R-TUI-8` (Settings). MOD-115 is done (`docs/decisions/mod/mod-115.md`
> D2: the activity slot is a placeholder until this item).
> Scope extended by the maintainer at framing (2026-11-05): logging configurable from Settings,
> always captured in memory, and connected workers' logs visible from the main instance.

## Problem

The maintainer runs many items through agents at once, from the TUI's own runtime and from
`htui worker` processes on other boxes. When something stops for them (a permission request, a gate,
a selection) or fails, the only way to notice is to visit tabs and items one by one, and only the
current workspace is ever looked at. When htui itself misbehaves there is often no log at all: the
TUI logs only when started with `--log`, and a worker's log is a file on its own box, reachable only
by SSH. Problems go unnoticed or undiagnosable, and the activity slot MOD-115 laid out stays empty.

## Evidence

Read at `38949409` (branch `hr/MOD-122`). Paths relative to the repo root.

- **Maintainer requests in the UI review.** Round 4: "what does activity do?" (the first skeleton
  had no features) and a Logs tab; round 6: activity newest first, and a way back after Enter
  (`docs/ui-redesign-findings-2026-10-15.md` §7).
- **Maintainer report (2026-11-05), all three observed in use:** a problem left no log because
  `--log` was not set; a worker problem could be read only by SSHing in to `worker.log`; a wait or a
  failure went unseen while the maintainer was on another tab or workspace.
- **No log without `--log`.** `init_tracing` returns early when no path is given
  (`crates/htui/src/lib.rs:359`); the filter is `HTUI_LOG_FILTER`, default `info` (`lib.rs:368`).
  No in-memory buffer, tracing layer or log viewer exists in `crates/`.
- **Worker logs stay on the worker's box.** `init_worker_tracing` writes to its `--log` file or
  stderr (`crates/htui/src/worker_cmd.rs:281`); the provisioned unit writes
  `~/.local/state/htui/worker.log` (`crates/htui/src/provision/script.rs:209`). The TUI also runs a
  run runtime in-process (`crates/htui/src/run_worker.rs`), so htui's own log spans processes.
- **"Needs you" reaches the current workspace only.** Waiting reasons (`WaitingReason`:
  gate, judge failed, selection, unblock, interrupted, permission —
  `crates/htui-worker/src/views.rs:487`) feed the Ctrl+W overlay and the top-bar count, polled for
  the one current `Scope` (`crates/htui-core/src/model/scope.rs:8`, `crates/htui/src/app/update.rs:24`).
- **Failures are not stored as events.** They travel as transient `FrameKind::Error` run frames
  (`views.rs:150`) to a per-item subscriber; there is no activity or run-event table.
- **The activity slot is a placeholder.** It renders "Activity appears here once activity panel
  lands" (`crates/htui/src/app/state.rs:780-811`, `:1751-1754`).

## Users

- **Primary**: the maintainer, running several items through agents across workspaces and boxes,
  who needs to see what waits on them or failed, and to diagnose htui or a worker, without leaving
  the TUI or opening a shell on another box.
- **Not for**: team or multi-user audit (phase 2, MOD-47); external log aggregation or alerting.

## Hypothesis

We believe **an always-on in-app log that includes connected workers, plus a live activity feed
across every workspace with "needs you" pinned**, will **let problems be noticed and diagnosed
without leaving htui or SSHing into a box** for **the maintainer**.
We'll know we're right when **every wait and failure in any workspace appears in the activity panel
within a few seconds of happening, and a worker problem is diagnosed from the Logs tab without a
shell on the worker's box**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Wait/failure → visible in the activity panel | ≤ 2 poll intervals (~2 s), any workspace | test pinning a stored event's appearance; manual check across two workspaces |
| Log lines available in the TUI with no `--log` | 100 % of lines at or above the configured level, up to the buffer cap | test: a line emitted with no log file appears in the Logs tab |
| Worker line → visible in the main instance's Logs tab | within one upload batch interval + one poll | test with a worker and a TUI on one database |
| Worker problems needing SSH to diagnose | 0 in use after M2 | maintainer report — TBD, needs validation in use |
| Secrets in any log line (memory, file, shipped) | 0 | test over known secret-bearing code paths; review |

## Scope

**MVP** — three milestones (below): the Logs tab with in-memory capture and logging settings; worker
logs shipped to the main instance; the stored activity feed with the panel.

Decisions taken at the DECIDE gate (2026-11-05):

- **Worker upload is buffered and batched**: a worker keeps lines in a local buffer and uploads them
  in batches, at its configured level, rather than one write per line.
- **Retention**: a configurable maximum retention for stored logs and activity events; in addition,
  a deleted item's or worker's logs and events are deleted with it, **except errors**, which stay
  until the maximum retention.
- **Secrets**: htui never writes a secret into a log line — memory, file or shipped. The requirement
  is that no secret reaches a log, not a redaction pass after the fact.
- **Activity events**: *needs you* — every waiting reason (permission, gate, selection, unblock,
  interrupted, judge failed); *failures* — a step or a run failed; *plain* — a run started, a run
  finished, an item waiting for close-out.
- **Settings scope**: the TUI's own logging settings are local to it; each worker's shipping level is
  set from the main instance, per box.

**Out of scope**

- The background-workspace lifecycle (open / background / stopping) — MOD-116. The feed reads every
  workspace in the store regardless, so it does not wait on MOD-116.
- Terminal bell and OS notifications — MOD-114 (1); they may consume M3's events later.
- Push delivery (`LISTEN`/`NOTIFY`, relay) — MOD-46/MOD-47; the existing ~1 s poll suffices.
- Stored command logs — MOD-152.
- Log search, and log export beyond the existing log file.

## Delivery Milestones
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Logs tab and logging settings | A Logs tab shows htui's own log, always captured in memory with or without `--log`: sources (ui, store, orchestrator, agents, mcp, worker, secrets) with line and warning counts, lines coloured by level, minimum level `v`, follow `F`, scrolling up pauses following, `G` resumes, new lines stream in. A Settings section sets the level filter and the optional log file, applied without a restart. | complete | `.claude/plans/mod-122-activity-and-logs-m1.plan.md` |
| 2 | Worker logs in the main instance | Each connected worker uploads its log lines (buffered, batched, at its per-box level set from the main instance); they appear in the main instance's Logs tab as a source per worker/box. Stored lines obey the maximum retention and are deleted with their worker, errors excepted. | pending | — |
| 3 | Activity panel | Stored activity events from every workspace, the TUI's runtime and workers alike, shown newest first in the activity slot of the layout presets; needs-you events pinned on top, newest first, counted in the panel title; Enter jumps to the item or chat (across workspaces it says which workspace opens) and is recorded in back/forward history; `f` filters all workspaces · needs you · failures · this workspace; new events stream in. Events obey the maximum retention and are deleted with their item, errors excepted. | pending | — |

## Open Questions

All questions raised at DECIDE were answered (above). Left for `/plan`, not scope-changing:

- [ ] Batch size and upload interval for worker logs, and the worker-side buffer cap when the
  database is unreachable (what is dropped first) — TBD, needs validation via a load measurement.
- [ ] In-memory buffer cap in the TUI (lines or bytes) and whether it is a setting — TBD at `/plan`.
- [ ] Default maximum retention for logs and events, and whether it rides MOD-100's sweeper or its
  own — TBD at `/plan` (MOD-100 is open).
- [ ] How "errors" is defined for the deletion exception: error-level log lines and failure events
  only, or also warnings — assumption: error level and failure events only; confirm at `/plan`.
- [ ] Which tabs default to a preset showing the activity panel (today none does) — TBD at `/plan`.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Worker log upload floods Postgres or slows runs | Medium | High | Buffered batched upload at the per-box level; drop-oldest local buffer; measure before fixing the defaults |
| A secret reaches a shipped log line, readable by anyone with database access | Medium | High | Requirement "never log secrets"; tests on secret-bearing paths (DSN, keys, keyring); reviewer gate checks every new log call |
| Logging setting applied live breaks the subscriber or loses lines | Low | Medium | Reloadable filter/file layer; test switching level and file at run time |
| Activity events from the TUI runtime and workers duplicate or miss transitions | Medium | Medium | One writer per transition at the point the state changes; tests per event kind |
| Snapshot churn across tabs showing the activity slot | High | Low | Regenerate and review per milestone (memory: full insta run) |
| A new migration moves count pins (`Pending(N)`, `TABLES`) | High | Low | Known recipe; renumber at collect if a sibling run adds a migration |

---
*Status: M1 complete (`hr/MOD-122` `83dd23c0`, 2026-11-05); M2 and M3 pending `/plan`.*
