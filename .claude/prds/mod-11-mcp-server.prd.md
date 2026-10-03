# MOD-11 — htui MCP server

> Routed as **PRD** by `/handoff-run MOD-11` (C2, C3 and C4 fired; accepted by the maintainer
> 2026-10-03, sandbox run `hr/MOD-11`). Ultracode accepted for the implement and review phases.
> Requirements: `R-MCP-1..4` (`spawn_subagent` of `R-MCP-2` excluded, see Scope). Origin:
> `docs/ANA-2.md` §4.2, §8, §9 row MOD-11, risk 11; `docs/ANA-5.md` §4.2; `docs/ANA-16.md` §8.
> Consumers waiting on it: MOD-4 (judge, `approve`/`accept`), MOD-41 D5 (sink fence), MOD-33 D277,
> MOD-34/MOD-50 (`search_concepts`), MOD-2 (`permission_request` over CLI), MOD-12, MOD-27, MOD-43.

## Problem

No agent htui launches can act on the backlog. Every production `SessionSpec` carries
`mcp: Vec::new()`, no transport forwards an MCP server to the agent, and no MCP server exists. So no
step can write its `output_kind` document outside a test: every production judge fails and parks
the fan-out on a human pick, production `approve`/`accept` stay greyed in the Runs pane, agents
cannot propose links or leave notes, and heavy commands run unthrottled through the agent's own
shell. Until this lands, a production run cannot complete a phase whose output is a document, and
MOD-12 (auto mode) and MOD-27 (swarm) have nothing to build on.

## Evidence

Read at `30493f39` (branch `hr/MOD-11`). Paths relative to the repo root.

- **No server, no wiring.** `McpServerSpec` (`crates/htui-agent/src/driver.rs:224-234`, doc :219-222
  "MOD-11 fills it") is read by no transport: ACP `NewSessionRequest` sets no `mcp_servers`
  (`crates/htui-agent/src/acp/mod.rs:1108-1109`); `cli::argv` emits no `--mcp-config` /
  `--permission-prompt-tool` (`crates/htui-agent/src/cli/mod.rs:116-178`). Every production spec
  passes `mcp: Vec::new()` (`crates/htui-orch/src/engine.rs:5921`,
  `crates/htui/src/agent_worker.rs:1047`, `:2086`). No MCP crate in the workspace.
- **Judge fails structurally.** The judge phase (`crates/htui-orch/src/fanout.rs:284-301`,
  `output_kind = "judge"`) needs a new `judge` document per call; with none,
  `JudgeFailure::MissingDocument { call }` (`engine.rs:5077-5086`). Two calls (forward and
  reversed) share one step, so one step must be able to write two `judge` versions. MOD-4 M4 OQ-4
  (`docs/decisions/mod/mod-4.md:235-236`).
- **Approve/accept greyed (MOD-4 R-50,** `mod-4.md:359-361`**).** Production sink is
  `ProgressSink { author: None }` (`crates/htui-worker/src/views.rs:519-552`,
  `crates/htui-worker/src/runtime.rs:919-923`) — not `NoSink` as HANDOFF and `mod-4.md` say; the
  outcome is the same: no production `StepAuthor` (`views.rs:510-513`).
- **Sink write is unfenced (MOD-41 D5).** `ProgressSink::after_done` calls `write_document` with no
  `StepFence` (`views.rs:543-546`); `StepFence::{Lease, Unleased}` exists
  (`crates/htui-core/src/store/traits.rs:2494-2512`) but `write_document` is not a fenced method.
- **Write paths.** `write_document` allocates `max(version)+1` under the item row lock in the
  store (`traits.rs:1466-1473`) — HANDOFF's "orchestrator-allocated" is stale. `add_note` with
  `NewNote.via_step_id` exists (`traits.rs:1589-1594`, `crates/htui-core/src/model/note.rs:45-46`).
  **No `item_link` write exists** — only a read (`traits.rs:95`) and the demo loader; schema has
  `proposed_by_step_id` and a `deleted_at` tombstone (`crates/htui-store/migrations/0001_init.sql:357-368`).
  R-ENT-9: links come only from agents via MCP and the importer.
- **`box_profile`.** `BoxProfile::project` lives in `crates/htui-core/src/model/box_.rs:395-421`
  (not `htui_core::prompt`), drops `box_tool.path`, caps at 24 tools; the prompt renders it with a
  `HostnameLine` driven by `project.settings.box_hostname` (`crates/htui-core/src/prompt/render.rs:402-409`,
  `engine.rs:4837`). D277 (`.claude/plans/mod-33.plan.md:146`) leaves the switch to this item.
- **`command_run`.** Table exists (`0001_init.sql:537-551`, queue index on
  `(box_id, class, status, queued_at)`), with `record_command_run` / `command_runs`
  (`traits.rs:1449`, `:1464`). Limits: `box.settings.command_limits`, app default
  `{"build":1,"test":4,"verify":1}` (`0003_orchestration.sql:129`) — but the worker falls back to
  `{"verify":1}` only (`runtime.rs:797-822`). `ShellVerifier` runs verify commands directly under an
  in-process semaphore (`crates/htui-orch/src/verify.rs:152-200`).
- **Exposure is not implemented.** `ToolExposure.command_run` is never true (`engine.rs:5900-5905`,
  `crates/htui-agent/src/persona.rs:87`); the prompt flag ignores `fan_out_only` and `heavy_build`
  (`engine.rs:5593-5595`), so `fan_out_only` behaves as `always`. `heavy_build` exists only as a box
  tag; R-MCP-3 puts it on the item (`item.required_tags`, `0001_init.sql:322`).
- **Vector search ready.** `VectorStore::search(&SearchQuery) -> Vec<Hit>`
  (`crates/htui-store/src/vector.rs:361`); empty `projects` finds nothing; `Hit.owner` is
  `Item | Requirement`.
- **Scoping inputs exist.** `drive_once` holds run, step, phase, `key` and the lease owner
  (`engine.rs:5875-5928`); `SessionSpec.step_id` (`driver.rs:255-256`); chat runs are
  `StepFence::Unleased`, and `run.item_id` may be `None`.
- **Process boundary.** `htui worker` drops the DSN after connecting
  (`crates/htui/src/worker_cmd.rs:88-89`); ANA-16 §5 says agents never write Postgres, the hosting
  htui process does. A stdio MCP child cannot open the store itself.
- **CLI permission gap.** CLI transport reports `permission_requests: false` and
  `answer_permission` is `Unsupported` (`cli/mod.rs:9-15`, `:584-596`); design in
  `docs/ANA-4.md:546-553`, open contract `:1385-1386`; deferred here by
  `.claude/prds/mod-2-agent-driver-chat.prd.md:213,222`.

## Users

- **Primary**: the maintainer driving runs from the TUI or `htui worker` — needs a production run to
  finish judge and document-output phases without a test sink, and needs agents to leave durable,
  attributed traces (notes, links, documents) instead of free text in the session log.
- **Primary (machine)**: the agent session inside a run step — needs a small, scoped tool surface to
  act on its own item and box.
- **Not for**: humans editing items (MOD-13), auto-mode scheduling (MOD-12), sub-agent spawning
  (MOD-27), remote/container launch (MOD-43, MOD-44), cross-run or cross-project agent access.

## Hypothesis

We believe a **step-scoped htui MCP server, advertised to every session htui launches** will **let
production runs complete document-output phases and give agents attributed, bounded write access to
their own item** for **the maintainer running htui**. We'll know we're right when **a production
run's judge writes both `judge` documents and resolves without a human pick, `approve`/`accept` are
live on a production step, and every tool call against an item, step or project outside the
session's run is refused by test.**

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Production judge success | Judge resolves on an agent-written `judge` document, no `MissingDocument` | Engine test on the production sink + `ProgressSink` with a production author |
| `approve`/`accept` on production steps | Enabled when the step wrote its `output_kind` document | TUI snapshot / Runs-pane test without a test `StepAuthor` |
| Scoping | 100% of cross-run / cross-project / stale-lease calls refused | Conformance tests per tool (MemStore + Postgres) |
| Fence | A `document_write` from a step whose lease was lost is refused | Fenced-write test (MOD-41 D5 closed) |
| Status never moves | `item_status` never changes `item.status` | Test asserting a note with `via_step_id`, status unchanged |
| `command_run` limits | Concurrent runs per `(box, class)` never exceed the limit | Queue test under contention |
| Exposure | Tool not advertised when the phase resolves to off | Session-spec / tools-list test per `off` / `fan_out_only` / `always` / `heavy_build` |

## Scope

**MVP** — the server, wired into both transports, with per-step scoping and the write tools
(`document_write`, `note_add`, `item_status`, `item_link`) plus the read tools (`box_profile`,
`search_concepts`). This is the slice that closes MOD-4's judge and R-50 and MOD-41 D5.
`command_run` with the queue and per-phase exposure follows as its own milestone.

**Out of scope**
- `spawn_subagent` (`R-MCP-2`) — MOD-27 owns the swarm `RunKind`; MOD-11 supplies the server it
  plugs into.
- Auto-mode queue runner — MOD-12.
- Remote/container reachability beyond a launchable stdio spec on the box that hosts the driver —
  MOD-44 (open) and MOD-43; this item must not preclude it.
- Human link editing — R-ENT-9 forbids a manual link action.
- `requirement_cite` tool (deferred `R-MCP-2` amendment, `REQUIREMENTS.md:16`) — needs a maintainer
  amendment to REQUIREMENTS first (OQ-7).
- Loopback HTTP MCP transport (OQ-1) — stdio plus the host relay covers every launch site.
- `verify_command` through the queue (OQ-6) — stays direct.

## Delivery Milestones

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Server + wiring + scoping | Every session htui launches (engine, chat, worker) sees an htui MCP server bound to its step; `box_profile` answers; out-of-scope calls refused | pending | — |
| 2 | Document and backlog writes | `document_write` (fenced, production `StepAuthor`), `note_add`, `item_status` as note, `item_link`; production judge resolves, `approve`/`accept` live | pending | — |
| 3 | `search_concepts` | Agents search items and requirements in the step's project | pending | — |
| 4 | `command_run` queue and exposure | Heavy commands queue under per-box class limits and return output; exposure honours `off` / `fan_out_only` / `always` / `heavy_build`; skill text and ACP denial per R-MCP-4 | pending | — |
| 5 | CLI `permission_request` | CLI-transport sessions route permission prompts through the MCP permission tool into the MOD-42 relay | pending | — |

## Open Questions

Resolved with the maintainer 2026-10-03 (OQ-2..4 as asked; OQ-5..7 by stated default, no
objection; OQ-1 see note).

- [x] **OQ-1 Process boundary → stdio front + host-side relay.** The maintainer leaned "both"
  because workers can run in sandboxes. Resolution: the agent-facing protocol is **stdio only** — an
  `htui mcp` child the agent launches; the tool handlers run in the **hosting** htui process (TUI
  engine, chat worker, or `htui worker`), which owns the store, the lease and the fence. The child
  reaches its host over a **per-session local channel with a session token**, and that channel is a
  seam. A sandboxed worker is not a problem: `htui worker` spawns its agents on its own box
  (`crates/htui-agent/src/launch.rs:1129-1172`), so the worker inside the sandbox *is* the host.
  Containers (MOD-44) bridge the same seam (bind-mounted socket or the `docker exec -i` stream)
  without a second agent-facing protocol. Loopback HTTP MCP is not built: it reaches nothing stdio
  plus the relay does not, and adds a protocol per agent. *Maintainer to confirm at plan CONFIRM.*
- [x] **OQ-2 CLI `permission_request` → in scope as M5** (`--permission-prompt-tool` into the
  MOD-42 relay; contract `docs/ANA-4.md:1385-1386`).
- [x] **OQ-3 `box_profile` hostname (D277) → honour the switch**: the tool uses the same
  `HostnameLine` resolution as the prompt (shown / stand-in / omitted), so tool and prompt agree.
- [x] **OQ-4 `item_link` → live on insert, own tombstone.** Stamped `proposed_by_step_id`; all four
  kinds; an agent may tombstone only a link its own run proposed. Both endpoints must be in the
  run's project.
- [x] **OQ-5 `document_write` → phase `output_kind` on the step's own item only**; the tool is not
  advertised when `run.item_id` is `None`. One step may write several versions (judge: one per call).
- [x] **OQ-6 `command_run` output → capped and truncated with a marker**, scrubbed before store
  and return. `verify_command` stays direct this item (follow-up if wanted); the worker's
  `command_limits()` fallback is fixed to read the `app_setting` default.
- [x] **OQ-7 `requirement_cite` → deferred**; needs a maintainer REQUIREMENTS amendment first.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Agent writes outside its run (scoping bypass) | Medium | High | Every tool resolves scope server-side from the bound step, never from tool arguments; per-tool refusal tests |
| Lost-lease step keeps writing documents | Medium | High | Fence `write_document` with `StepFence` (MOD-41 D5) |
| Agent moves an item status | Low | High | `item_status` writes a note only (ANA-2 risk 11); test pins status unchanged |
| Transport differences (ACP vs claude CLI MCP config) | Medium | Medium | Driver contract test per transport asserting the server is advertised |
| Command queue deadlock / starvation across workers | Medium | Medium | Postgres-side admission on the existing queue index; cancellation on lease loss |
| Secrets in `command_run` output | Medium | High | Store scrubbed output only (existing column contract); reuse the runtime scrubber |
| Stack headroom in `htui-orch` tests | Medium | Low | Box new engine futures (memory: htui-orch test stack headroom) |

---
*Status: APPROVED requirements (open questions resolved 2026-10-03). Implementation planning via /plan.*
