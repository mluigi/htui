# ANA-2 - Orchestration design: step graphs, phase contract, status machines, fan-out, isolation, resume (done, 2026-09-06)

## Summary

Concluded how `htui` runs an item through a step graph: the step graph data shape and its
semantics, the phase contract (inputs, output document kind, gate, retry, verification), the
status machines on `item`, `run` and `run_step`, the review-to-implement loop, fan-out selection
and the judge contract, the four isolation modes with per-repo commit capture, the overlap rule for
concurrent items, promotion to chat, and resume after process death or an offline window. Design
in [`docs/ANA-2.md`](../../ANA-2.md). Requirements addressed: `R-ORCH-1..11`, `R-ENT-6`,
`R-ENT-8`; touched at their seams: `R-AGT-7..8`, `R-TUI-4`, `R-TUI-9`, `R-MCP-2..3`, `R-HIS-1..2`.

Method: five parallel Opus readers (requirements and prior ANAs, codebase seams, the in-house
`/handoff-run` workflow as prior art, a web survey of agent orchestrators and judges, a web survey
of durable execution, worktree isolation and queue admission), then one Opus writer producing the
document from the five dossiers with its own repo reads. Six agents total, per the standing cap.
The run was killed twice by the session usage limit and resumed from the workflow cache each time
(readers replayed, only the failed agents re-ran). An inline review of the finished document found
four internal inconsistencies (a loopable reject parking the run instead of continuing; a missing
run-table row for it; promotion of a `failed` step contradicting the step table's terminal set; the
recovery sweep selecting `queued` runs by an executing box they do not have yet) and fixed them in
place.

## What was decided

1. **Step graph shape (§4.1).** ANA-9 §5.4 suffices for `R-ORCH-1`; it is byte-identical to the
   shipped `0001_init.sql`. Migration `0003_orchestration.sql` adds five columns and one table and
   drops, renames or retypes nothing. Positions are dense and linear; the only back-edge is the
   `R-ORCH-3` loop expressed as `attempt + 1`. Every nullable phase field has one fallback chain.
   The item override clone is deep over `step_graph_phase` and `phase_agent`, not over
   `skill_binding`; `step_graph.is_override` hides it from the graph list. The orchestrator reads a
   `ResolvedPhase` off `run.graph_snapshot`, never the live tables.
2. **Phase contract (§4.2).** A phase is a total function from resolved inputs to exactly one output
   document, run in six stages (admit, prepare, prompt, session, settle, gate). `input_kinds`
   resolves to the latest version of each kind whose producing step is not a fan-out loser,
   preferring this run's own output; a missing kind hard-fails before a token is spent. The
   orchestrator allocates `document.version` inside the insert transaction; MOD-11's
   `document_write` calls that path. `verify_command` runs after the session and before the gate
   with three outcomes (`pass`, `fail`, `unavailable`) in new columns, leaving `run_step.exit_code`
   to `R-ORCH-11`. `attempt` is 1-based; `retry_limit` is additional attempts.
3. **Status machines (§4.3).** Three compare-and-set transition tables (item 8 states, run 6, step
   7) enforced in Rust by `const fn can_move_to` predicates and the conformance suite, not by
   triggers. Stored `blocked` means "a human must clear this" (capability refusal, `R-ORCH-3`
   escalation, secret-provider outage), always with an `item_note`; dependency-blocked stays
   derived per ANA-9 §7.4. Run status derives from steps except `queued` and `cancelled`; item
   status derives from its non-terminal runs; `awaiting_approval` propagates step to run to item
   in one transaction. `failed` upstream still blocks; the escape is `close`.
4. **Review loop (§4.4).** Bound by the implement phase's `retry_limit`, re-run at the same
   `position` with `attempt + 1`; the reviewed implement step and the rejecting review step become
   `superseded`. The loop carries the review document (seeded into `implement.input_kinds`), the
   failing verification output and the previous attempt's diff, never the transcript. A no-progress
   predicate (identical `after_hash` or identical review body hash, two attempts running) stops it
   early. Escalation is run `awaiting_approval` plus item `blocked`, never run `failed`.
5. **Fan-out and judge (§4.5).** `judge_agent_id` and `judge_model` on `step_graph_phase`; NULL
   means human selection whenever `fan_out > 1`. The judge is a real `run_step` at the same
   position with `fanout_index = -1`. Candidates failing verification are eliminated first when at
   least two survive; the judge sees documents, diffs and verification results, never transcripts,
   and is called twice with reversed order; disagreement, an unparseable verdict or an out-of-range
   index escalates to a human. Losers keep their documents and trees and are invisible downstream.
   Caps `max_fan_out = 4`, `max_agents_per_run = 8` (6 as concluded; raised by MOD-4 milestone 4's
   `0004` migration, amended by MOD-4 milestone 6, 2026-09-25), refused loudly. Task fan-out (the maintainer's
   own practice) is not `R-ORCH-7` and is left open.
6. **Isolation (§4.6).** Per repo through a new `run_step_tree(run_step_id, repo_id, mode, path,
   base_ref, dirty)` table. `worktree` branches from the primary repo's HEAD into a scratch root
   outside every `repo_box_path` with `--lock` and branch `htui/<step_id>`, retries git writes with
   backoff, and is refused for submodule repos; `copy` is re-justified on non-git, submodule/LFS and
   path-anchored-cache trees rather than on Rust build caches, with a mandatory exclusion list;
   `shared_serialized` holds a Postgres advisory lock per `(box, repo)`; `local` forces
   `fan_out = 1` and is refused when another run holds any tree in that repo. `before_hash` is
   captured per repo in every mode (with `dirty = true` on a dirty tree), `after_hash` at step end.
   The winner is merged into the primary tree, never copied; a conflict parks the run. Cleanup only
   at run termination.
7. **Overlap (§4.7).** A decidable predicate over a stored `run.repo_scope`, per-repo isolation
   and repo-qualified `touched_paths` (`repo_name:glob`, bare glob = primary repo, empty = unknown
   = the whole primary repo). Intersection is non-wildcard prefix containment, no glob crate.
   Admission is one transaction holding `FOR UPDATE` on the box row and counting `running` only.
   `BoxSettings` and `ProjectSettings` become typed with `app_setting` defaults.
8. **Promotion (§4.8).** In place: the same `run_step`, continued with `follow_up` events,
   `run.kind` stays `graph`, one new `run_step.promoted_at`. Context is preserved by `follow_up` in
   session, else by ANA-4's `session_started` row and `session/load` or `--resume`, else by an
   ANA-5 handoff prompt into a fresh session against the same tree. "The chat yields the artifact"
   is a document of `output_kind` plus an explicit `accept artifact` action, which then verifies,
   captures `after_hash`, resolves the gate and resumes at `position + 1`.
9. **Resume (§4.9).** A run-level lease (`lease_box_id`, `lease_owner`, `lease_expires_at`, 120 s
   TTL refreshed at 60 s, refreshed by CAS on the owner). The recovery sweep adopts local-box runs
   with an expired lease and decides per step by artefact: `after_hash` plus an output document
   means `done`; otherwise reset the tree to `before_hash` and retry; a dirty `local` or
   `shared_serialized` tree is never reset but parked. No prompt-digest replay. Offline, a live run
   stalls into the existing pending buffer and the reconnect sweep adjudicates.
10. **Capability, records, close-out, auto mode, caps (§4.10, §7).** The `R-ORCH-10` predicate is
    evaluated at queue and claim time and persists as `blocked` plus a note. The `R-ORCH-6`
    downgrade is applied at snapshot time, so the snapshot carries `gate` and `gate_effective`;
    `gate_outcome = 'skipped'` is its trace. Close-out is one transaction writing the `summary`
    document with the commit table and moving the item to `closed`; no new commit table. ANA-4 §7's
    candidate predicate is adopted verbatim with three MOD-4 skip conditions and an empty-candidate
    fallback chain ending in `no_candidate_agent`.
11. **Crate layout (§8) and schema (§9).** New fifth crate `htui-orch` (policy) plus
    `crates/htui/src/run_worker.rs` (runtime), mirroring ANA-4's split; sixteen `WriteStore`
    methods and the unmirrored reads as `PgStore` inherent methods; `FakeOrchestrator` over
    `MemStore`, `FakeDriver` and a `FakeIsolator`. Migration `0003_orchestration.sql` (full SQL in
    §9) and `cache_migrations/0002_orchestration.sql`; ANA-9's "migration `0002`" for ANA-2 is
    superseded, and MOD-4 must not apply `0003` before MOD-2 lands `0002`. Twelve `app_setting`
    keys reserved (§5.4). Default git library `gix`.

## Open for the maintainer (defaults adopted, MOD-4 not blocked on any)

Listed in `docs/ANA-2.md` §10: rival fan-out only in v1 (task fan-out is a requirement change);
`max_fan_out = 4` and `max_agents_per_run = 8` (6 as concluded; raised by MOD-4 milestone 4's `0004`
migration, amended by MOD-4 milestone 6, 2026-09-25); `max_concurrent_items = 2`; `copy` offered on
Windows with the size shown; `gate_hard` seeded on `prd`, `plan` and `verdict`; lease 120 s / 60 s;
`gix` over `git2`; close-out allowed on a `failed` item.

## Residual gaps (named, not solved)

- The in-house "live coordinates" recap and "watch items for later modules" note have no database
  home (`docs/ANA-2.md` §4.10, risk 10).
- `document_write` does not exist until MOD-11, which is blocked on MOD-4; MOD-4 ships the
  `accept artifact` action and hand-written documents as the interim producers (risk 4).
- `fanout_index = -1` relies on the absence of a CHECK constraint (risk 12).

## Downstream items

- **MOD-4** (orchestrator, manual mode) now blocked on MOD-2 only; build order in `docs/ANA-2.md`
  §9, steps 1 to 4 need nothing from MOD-2.
- **MOD-12** (auto mode) adds a caller, not machinery: `ready_items`, the same `claim_run`
  admission, batch caps, escalation surfacing, the `scheduler_window` key.
- **MOD-2** must land `0002_agent_probe.sql` before `0003`; MOD-4 consumes `DriverCaps`,
  `SessionSpec.cwd`/`extra_dirs`, `session_ref` and the `session_started` row.
- **MOD-11**: `document_write` calls `write_document`; `command_run` accepts class `verify`; an
  `item_status` request becomes an `item_note`, never a transition.
- **MOD-7**: real `probed_tags`/`declared_tags`, `repo_box_path` rows, `agent_box.probe.status`.
- **MOD-15**: seed per §4.1 (`review` in `implement`/`fix` `input_kinds`, `gate_hard` seed,
  `is_override = false`); whichever of MOD-15 and MOD-4 lands second owns the seed amendment.
- **MOD-13**: the `touched_paths` edit path accepts the `repo_name:glob` qualification.
- **ANA-5** must supply three templates: `judge`, the review-loop forwarded set, the promotion
  handoff prompt, plus a stable serialisation for `prompt_digest`.

## Commits

- docs(ana-2): conclude orchestration design analysis (this close-out).
