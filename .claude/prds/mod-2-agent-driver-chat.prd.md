# MOD-2 — Agent driver + chat tab

> Routed as **PRD** by `/handoff-run MOD-2` (criteria C2, C3, C4 fired). Ultracode recommended and
> accepted for the implement and review phases. Design is already concluded upstream: this document
> captures *what must be true*, not *how* — `docs/ANA-4.md` and `docs/ANA-5.md` are the design
> authority and are cited, never restated.

## Problem

`htui` today can read and write its own domain (items, projects, runs) against Postgres and render a
backlog, but it cannot talk to a coding agent. Every capability the product exists for — running a
step graph, promoting a step to chat, judging fan-out candidates, recording what an agent did —
sits behind one missing seam: a driver that starts an agent session, streams typed events back, and
records them durably. Until that seam exists, `htui` is a backlog viewer with a database behind it,
and MOD-4, MOD-10, MOD-11 and MOD-12 are all blocked on it.

The second half of the same gap is the prompt: an agent session is only as good as the single
self-contained prompt it receives. `htui` has no assembler, so even with a live transport there is
nothing to send beyond text a human typed.

## Evidence

- **Design closed against primary sources, not recall.** ANA-4 re-checked 48 load-bearing claims
  against the ACP spec and the installed crates (9 refuted and corrected); ANA-5 re-checked 48 more
  (17 corrected). `docs/decisions/ana/ana-4.md`, `docs/decisions/ana/ana-5.md`.
- **Live probes on this box, not assumptions.** `claude`, `agy`, `agy_acp_server` and a rustc
  dyn-compatibility probe were all run during ANA-4; a Rust spawn probe proved the bare
  `claude-code-acp` shim name fails on Windows (`program not found`) while the `node <entry>` form
  works. That failure mode is exactly what a design written from documentation would have shipped.
- **Blocking evidence from the backlog itself.** Five open items name MOD-2 as their blocker
  (`HANDOFF.md`); MOD-4's build steps 1–4 are the only work in the tree that needs nothing from it.
- **Requirement coverage is contractual, not inferred.** `R-AGT-1..8`, `R-PRM-1..3`, `R-TUI-6`,
  `R-TUI-8`, `R-HIS-1..2` are `must` in `docs/REQUIREMENTS.md`, and `R-NF-4` obliges every item to
  cite the IDs it addresses.

## Users

- **Primary**: the maintainer running `htui` as their own workflow tool — one operator, one box,
  driving `claude` and `agy` against real repositories. The need is triggered the moment they want
  an agent to do the work an item describes rather than copying the item's text into a terminal by
  hand.
- **Also served**: every downstream `htui` feature that consumes the driver seam — the orchestrator
  (MOD-4), the MCP server (MOD-11), the auto-mode queue runner (MOD-12).
- **Not for**: teams (`R-USR-3` is later tier), remote or headless execution (`R-ORCH-12`, later
  tier), or agents beyond `claude` and `agy` in this version — though see the extensibility
  hypothesis below, which exists precisely so those arrive without touching this code.

## Hypothesis

We believe **one transport-agnostic agent driver with a durable event log, an extensible registry,
and a deterministic prompt assembler** will **turn `htui` from a backlog viewer into a tool that
runs agents** for **the maintainer and every downstream `htui` feature**.

We'll know we're right when **a live `claude` session streams into the chat tab, its events survive
a restart and replay identically, and a third agent — one nobody wrote code for — reaches a working
session through a registry row and at most one stream adapter.**

That last clause is the load-bearing one. `R-AGT-5` is the requirement, and the maintainer's scope
call made it the MVP's proof obligation rather than an aspiration: extensibility that is not
demonstrated by an agent the codebase does not know about is not extensibility.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| New agent added without code change | 1 registry row, 0 changed source files, working session | A test registers an agent absent from the codebase and drives it through the shared conformance `CASES`; `R-AGT-5` |
| Transports passing one conformance list | 3 (fake, ACP, CLI) with 0 transport-specific cases | `ANA-4 §11` criterion 1 |
| Event durability | 100% of session events persisted after scrubbing; replay of a fixture is byte-identical twice | `ANA-4 §11` criteria 2–4; `R-HIS-1` |
| Past step reopened read-only and replayed | Any recorded step, including one recorded offline and uploaded | `ANA-4 §11` criterion 12; `R-HIS-2` |
| Design criteria closed | `ANA-4 §11` 1–13 and `ANA-5 §12` 1–20 all pass | Test suite + live run |
| Unverified items closed by probe or live run | `ANA-4 §11.14` (11 items) and `ANA-5 §12` criterion 21 (5 items) — all resolved or explicitly re-deferred with a named owner | Recorded in the MOD-2 decision document |
| Prompt reproducibility | Same `PromptSpec` yields the same digest on any box, and the same prompt under fan-out | `ANA-5 §12`; `R-PRM-1` |
| Secret leakage into the store | 0 unmasked values; a residue refuses the write rather than persisting it | Fail-closed scrubber check on every persist path; `R-SEC-3` |

## Scope

**MVP** — a driver seam that three transports satisfy identically, a chat tab that streams a live
session and answers permissions inline, an event log that survives restart and replays, a registry
whose extension point is proven by an agent the code does not know, quota and caps that actually
cancel a session, and a prompt assembler with a read-only preview.

Concretely in scope:

- The `AgentDriver` / `AgentSession` seam and its event model, per `docs/ANA-4.md` §4.1.
- ACP transport for `claude` (via `claude-agent-acp`) and for `agy` (via `agy_acp_server`), per §4.4
  and §4.5.
- The degraded CLI transport for `claude`, with a capability banner naming what it cannot do, per
  §4.4 and §6.2 — `R-AGT-3` is a `must` and the structural gaps (`edit_proposal`,
  `permission_request`, `plan`) are declared, not hidden.
- Agent registry, its Settings tab section, autodiscovery probes, and quota display — `R-AGT-4`,
  `R-AGT-6`, `R-TUI-8`.
- Quota tracking and cap enforcement by session cancellation — `R-AGT-7`, `R-AGT-8`.
- Streamed chat tab with follow-ups, collapsed thoughts, tool calls and results, edit proposals as
  diffs, and inline permission answers — `R-TUI-6`.
- Event persistence, offline buffering, and read-only replay — `R-HIS-1`, `R-HIS-2`.
- **Chat run ownership.** MOD-2 owns creating the `run` (`kind = 'chat'`, `item_id NULL`) and
  `run_step` (`phase_name = 'chat'`) rows a free-standing chat records against, with the same
  client-side UUIDv7 mint the offline path uses (`docs/ANA-9.md` §4.3). MOD-4's `claim_run` is
  graph-only and does not cover this.
- **A minimal fail-closed scrubber.** ANA-7 has not concluded and MOD-10 is blocked on it, but
  `R-SEC-3` gates every persist path and `docs/ANA-5.md` §4.5 already refuses a run whose excerpt
  trips the scrubber. MOD-2 ships the `Scrubber` seam ANA-4 §9 specifies plus a minimal built-in:
  exact-match masking of every `SessionSpec.env` value and of known credential prefixes. It is
  fail-closed from day one and MOD-10 replaces the implementation behind the unchanged trait.
- The prompt assembler and its supporting store methods, per `docs/ANA-5.md` §8, including
  `WriteStore::set_step_prompt`.
- **A read-only prompt preview.** Pick an item and a phase; see the assembled prompt, its section
  list, its trim record and its digest. Without it the assembler ships correct but never executed
  by the running binary, since MOD-4 is its only production caller and does not exist yet. Preview
  renders; it does not send.
- Migration `0002_agent_probe.sql`, carrying ANA-4's `agent_box.probe` column and ANA-5's appended
  comment and `app_setting` seed sections.

**Out of scope**

- Step-graph execution, gates, retries, the review loop, fan-out and judging — MOD-4 owns them; the
  driver only supplies `DriverCaps`, `SessionSpec` and the recorded rows they consume.
- The real secret provider and environment injection — MOD-10, gated on ANA-7. MOD-2 ships the seam
  and a minimal implementation behind it, deliberately.
- The MCP server — MOD-11. Its permission-prompt tool is the CLI transport's only route to a
  `permission_request`, so that gap stays declared until MOD-11 lands.
- The skill library editor and template editing UI — MOD-9 and MOD-15. MOD-2 owns the template
  *validator* they call and the types they share, not their screens.
- External context tools as excerpt providers — `R-LATER-7` / ANA-3. MOD-2 ships the
  `ExcerptProvider` seam and the built-in ranker only.
- Sending an assembled prompt into a live session from the preview — that is a run, and runs are
  MOD-4.
- Retention sweeps (`R-HIS-3`), remote dispatch and scheduling (`R-ORCH-12..13`).

## Constraints (decided upstream, not open)

- Workspace MSRV rises 1.85 → **1.98**, matching the exact toolchain pin `1.98.1`. This is a
  maintainer override of `docs/ANA-4.md` §4.2's 1.88, given at CONFIRM on 2026-09-06: 1.88 is below
  the locked `sqlx-core 0.9.0` floor of 1.94, so it would have been a second unbuildable
  declaration replacing the first. `agent-client-protocol` is pinned `=2.1.0` (`docs/ANA-4.md` §4.2).
- New workspace dependencies are exactly those ANA-4 §8 names; the prompt assembler adds none
  (`docs/ANA-5.md` §8).
- New crate `htui-agent`; everything pure in `htui-core::prompt`; no `htui-orch` (it does not exist
  until MOD-4).
- One migration, `0002_agent_probe.sql`, and it must land before MOD-4 writes `0003`.
- `unsafe_code = "forbid"` and the workspace lint set are unchanged; TDD per repo convention.

## Delivery Milestones

<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Driver seam + conformance | Any transport, real or fake, is provably interchangeable: one `CASES` list all of them pass, with events recorded, scrubbed and persisted through the fake alone. No process is spawned yet. | complete | [plan](../plans/mod-2-driver-seam-registry.plan.md) |
| 2 | Registry, launch and extensibility proof | The maintainer sees registered agents in Settings; an agent the codebase has never heard of reaches a working session from a registry row alone. `R-AGT-5` becomes a passing test rather than a claim. | complete | [plan](../plans/mod-2-driver-seam-registry.plan.md) |
| 3 | Live `claude` over ACP | The maintainer holds a real streamed conversation with `claude` inside `htui` — text, thoughts, tool calls, edit proposals as diffs, permissions answered inline. | complete | [plan](../plans/mod-2-live-acp-chat.plan.md) |
| 4 | Durable history and replay | Nothing about a session exists only in memory: events persist scrubbed, an offline session buffers and uploads idempotently, and any past step reopens read-only and replays. | complete | [plan](../plans/mod-2-durable-history-replay.plan.md) |
| 5 | Autodiscovery and box probe | The maintainer learns what this box can actually run without configuring anything: probes find agents and adapters, record versions, and mark per-box enablement. Migration `0002` lands here. | complete | [plan](../plans/mod-2-probe-autodiscovery.plan.md) |
| 6 | `agy` over ACP | A second agent works through the same code paths as the first, differing only by registry row and capability banner. | complete | [plan](../plans/mod-2-agy-acp.plan.md), [T34 close-out](../plans/mod-2-quota-caps.plan.md) |
| 7 | Quota and caps | Remaining allowance is visible per agent per box; a per-token cap breach cancels the session rather than being noticed on the invoice. | complete | [plan](../plans/mod-2-quota-caps.plan.md) |
| 8 | Degraded CLI transport | An agent without ACP is still usable, with the capability banner stating exactly what it cannot do. | in-progress | [plan](../plans/mod-2-cli-transport.plan.md) |
| 9 | Prompt assembler + preview | The maintainer can see the exact prompt a step would receive — sections, trim record, digest — and confirm the ten default template bodies before MOD-4 depends on them. | pending | — |

Milestone 6 went `complete` when `T34` closed inside milestone 7's plan. Milestone 7 also carried
`T46`, which was **not** in its original scope: the review gate's live evidence showed ANA-4 §11
criterion 6's `edit_proposal` dedup rule failing across a flush — a defect from milestone 3/4 that
had passed only because the fake transport never flushed mid-tool-call. It is fixed here (D77) with
no new store seam, so criterion 6 is now honest for MOD-2's close-out.

Milestones 1–8 follow `docs/ANA-4.md` §9's build order; milestone 9 is `docs/ANA-5.md` §8's, and is
independent of 3–8 (it shares only milestone 1's store work). Milestone 9 may run in parallel with
3–8 or land last; the plan decides.

## Open Questions

Carried from the concluded analyses. None blocks the start; each must be closed or explicitly
re-deferred before MOD-2 is marked done.

- [ ] How is `R-PRM-1`'s "current project when no workspace" bound expressed, given the shipped
      `Scope` mandates a `workspace_id`? (`Scope::single_project` vs a separate `PromptScope`;
      `docs/ANA-5.md` §4.3 step 1)
- [ ] Can `conformance::run_case` be generalised over `ReadStore` so the four `ReadStore` additions
      run against `CacheStore`, or does the mirror comparison need its own harness? Generalising
      touches a shipped signature. (`docs/ANA-5.md` §8)
- [ ] Can `set_step_usage`'s `prompt_digest` parameter be dropped once `set_step_prompt` exists, or
      does the chat path still need it? (`docs/ANA-5.md` §4.4)
- [ ] Do `agy` and `claude` want different excerpt renderings (line numbers on or off)? If yes,
      `prompt_digest` becomes agent-dependent. (`docs/ANA-5.md` §10 item 9)
- [ ] The eleven `ANA-4 §11.14` items — `claude-agent-acp` model config ids and empty-option
      behaviour, `rate_limit_event` ordering, SIGINT vs SIGTERM cancellation semantics, CLI thinking
      block shape, the `--permission-prompt-tool` contract (deferred to MOD-11), whether
      `agy_acp_server` emits `usage_update` / issues `session/request_permission` / uses a
      vendor-specific edit shape, its `session/new` model list, its darwin/linux `.par` mechanics,
      and the `agy` CLI `statusLine` quota payload.
- [ ] Per-family `chars-v1` estimator constants are reasoned interpolations, not measurements.
      Calibration against real `run_step.usage` is deferred — confirm deferral survives MOD-2's
      first live runs. (`docs/ANA-5.md` §4.4)

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Third-party ACP surfaces (`claude-agent-acp`, `agy_acp_server`) behave differently from spec or move under us | High | Medium | Recorded transcripts as fixtures so behaviour is pinned once observed; `agent-client-protocol` pinned `=2.1.0`; capability banner surfaces what a transport cannot do rather than failing opaquely |
| The extensibility proof degenerates into "a second hard-coded agent" | Medium | High | The proof is a test driving an agent absent from the codebase through registry data alone; a code change to accommodate it fails the milestone |
| The minimal scrubber gives false confidence before MOD-10 | Medium | High | It is fail-closed, not fail-open: residue refuses the write. Its limits are recorded in the MOD-2 decision doc and MOD-10 is the named replacement behind an unchanged trait |
| The prompt assembler ships with no production caller and rots before MOD-4 | Medium | Medium | Milestone 9's preview exercises it end-to-end from the running binary; golden `insta` prompts pin the ten default bodies |
| `touched_paths` is empty and `repo_box_path` has no writer until MOD-13 and MOD-7 land | High | Low | Known and accepted: excerpt tier 1 and the fallback root contribute nothing at MOD-2 time; the ranker degrades rather than failing (`docs/ANA-5.md` §11) |
| MSRV bump and five new workspace dependencies destabilise the existing three crates | Low | Medium | Bump lands in milestone 2 with the dependency set, ahead of any transport work; `cargo tree` assertion that the SDK contributes no tokio edge (`ANA-4 §11` criterion 13) |
| Windows process-tree leaks (`node`, `claude`, `agy_acp_server` survive a kill) | Medium | Medium | Job object on Windows and process group on Linux, asserted by `ANA-4 §11` criterion 11; `htui` spawns the child itself because the SDK's group kill is unix-only |
| Nine milestones is a long single item; a session limit lands mid-flight | High | Low | Milestones are independently completable and each is a plan of its own; `HANDOFF.md` phase notes per `lifecycle.md` P1 carry state between sessions |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
