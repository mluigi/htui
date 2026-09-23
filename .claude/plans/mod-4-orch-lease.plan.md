# Plan: MOD-4 milestone 5 — two runs do not collide, and a crash is survivable

**Source**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`, milestone 5 (`:309`): "The overlap
predicate, the admission transaction, the lease and its refresh, and the recovery sweep's artefact
test — including the refusal to reset a dirty tree." Success-metric rows `:190` (criterion 12),
`:191` (criteria 15, 16) and `:192` (criterion 18); risk rows `:390` (a dirty tree is never reset)
and `:400` (two processes adopting each other's runs); the scope bullet `:241-243`. Design
authority: the PRD's D1–D8 (cited **PRD Dn**; this plan's own decisions are plain **Dn**,
milestone 1's **M1 Dn**, milestone 2's **M2 Dn**, milestone 3's **M3 Dn**, milestone 4's **M4
Dn**), `docs/ANA-2.md` invariants 5, 6 and 9 (`:124-127`, `:128-131`, `:139-142`), §4.3's run and
step tables (`:603`, `:605`, `:611`, `:618-621`, `:642-643`), §4.6's commit-capture table
(`:955-968`: heading, table `:957-960`, the `before_hash`/`dirty` prose `:962-968`) and base-ref rule (`:948-953`), **§4.7** (`:1009-1158`: `RunScope` `:1041-1055`, the
predicate `:1059-1072`, `PathPrefix` `:1074-1077`, the three cautions `:1079-1089`, admission
`:1091-1113`), **§4.9** (`:1242-1340`: the lease `:1276-1282`, the sweep `:1284-1307`, completed
siblings `:1317-1323`, the offline window `:1325-1340`), §5.4 (`:1518-1519`), §8 (`:1669`,
`:1671`, `:1681-1682`, `:1740-1742`), **§9 build step 7** (`:1811-1813`), §10 row 6 (`:2054`), §11
risks 8 and 13 (`:2071`, `:2076`), §12 criteria 3, 12, 15, 16, 18 (`:2090-2091`, `:2118-2119`,
`:2125-2130`, `:2135-2137`) and criterion 19 (`:2138-2140`, re-deferred); `docs/REQUIREMENTS.md`
`R-ORCH-9` (`:211-213`) and `R-HIS-1` (`:225-227`).

**Requirements**: `R-ORCH-9` (overlap serialises, isolation parallelises, a per-box limit),
`R-HIS-1` (nothing about a run exists only on one box — the lease and the artefact test are how a
run survives the process that started it), `R-ORCH-8` (the four isolation modes, whose reset rules ANA-2 §4.6/§4.9 define),
`R-ORCH-11` (every run records per-step commit hashes and status — the record the artefact test reads), `R-ORCH-12`'s v1 half by
construction (a box adopts and claims only its own runs).

**Complexity**: Large. Two new modules (`htui-core/src/model/overlap.rs`,
`htui-orch/src/recover.rs`) plus `htui-orch/src/overlap.rs`; one changed `WriteStore` signature
(`claim_run` answers *why*), two new `WriteStore` writers (`take_lease`, `interrupt_step`), one
narrowed contract (`adopt_runs` never adopts the sweeper's own lease); a pure overlap predicate
evaluated inside both stores' admission critical section; two new `Isolator` verbs (`reset`,
`release`); a heartbeat around every walk; the recovery sweep; ~16 new `htui-orch` conformance
cases and 3 new store conformance cases. **No migration** (D79), therefore no `schema_version`
bump, no mirror rebuild and no `cache_migrations` companion; **`.sqlx` does change** (the new and
changed `PgStore` queries).

**Routing**: routed as **plan** by `/handoff-run MOD-4` (the PRD and its milestone table exist, so
this milestone enters the chain at `plan`). **Staffing: Opus 5.5 for every step — plan,
fact-check, architect, implementers, verifiers and reviewer (`rust-reviewer`,
`.claude/workflow-config.json:2`); Fable is not used (maintainer standing instruction,
2026-09-23).** Ultracode for the implementers only, one workflow per task, verify fan-out per
round; the architect and the reviewer stay plain agents.

**Numbering**: milestone 4's plan and blueprint end at **D78**
(`.claude/plans/mod-4-orch-fanout.plan.md:217`; blueprint A-1..A-7 were accepted as D72–D78,
`:43`), so this plan starts at **D79**. New risks continue after **R-9** as **R-10…**.

**Status**: **fact-checked (194 claims: 137 confirmed, 55 partial, 2 falsified; all resolved)
and CONFIRMED by the maintainer 2026-09-23; OQ-1..OQ-11 take the adopted default.** The two falsified claims changed design: C10 rewrote D82 and added
OQ-11/R-20; C129 redefined D97's frontier and added a T7 case. The independence check added
D104–D106 and reshaped the waves. Branch `mod-4-m5`, cut from `main` at
`d854ff1`. **Blueprint ACCEPTED by the maintainer 2026-09-23**
(`mod-4-orch-lease.blueprint.md`, D107–D120): every finding F-A..F-W is
accepted with its fix, and every proposed change A-1..A-8 is accepted. A-8
supersedes F-C's test rewrite and closes R-21; R-22, R-23 and R-24 are not
incurred. Implementation in progress.

**Graphify note**: `graphify-out/GRAPH_REPORT.md` was built from `3e34610` (`GRAPH_REPORT.md:12`),
which predates `htui-orch` and the `0003`/`0004` migrations (`git ls-tree 3e34610
crates/htui-store/migrations/` lists `0001` and `0002` only). A `grep -c` of `graph.json` for
`claim_run`, `refresh_lease`, `adopt_runs`, `repo_scope`, `touched_paths`, `BoxSettings` and
`max_concurrent_items` returns **0** for every one of them, so the graph answered nothing this
milestone needs; every `htui-orch`, `htui-store` and `htui-core` store fact below was read from
the tree at `d854ff1`.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked.

- [ ] **OQ-1 — Where does the resolved `RunScope` live? (R-2)** §4.7 wants per-repo `isolated`,
      `local` and normalised `paths` "resolved once at queue time and stored on the run"
      (`docs/ANA-2.md:1035`, `:1040-1048`); the `run` row stores only `repo_scope`
      (`0003_orchestration.sql:40`), which is why milestone 1's `claim_run` can only answer "any
      shared repo overlaps" (M1 blueprint R-2, `mod-4-orch-seam.blueprint.md:1365`). **Default
      adopted (D79):** store it **inside `run.graph_snapshot`**, as `GraphSnapshot.scope:
      Option<RunScope>` with `#[serde(default)]` — the snapshot *is* the run's queue-time record
      (invariant 2), `topology` hashes `phases[]` only (`crates/htui-orch/src/graph.rs:223-233`),
      and M4 D53 set the precedent of a defaulted field without a `GraphSnapshot::V` bump —
      `SnapshotJudge.template`, nested inside the snapshot (`crates/htui-core/src/model/run.rs:553-557`);
      `scope` would be the first defaulted *top-level* `GraphSnapshot` field (`:448-461` are all
      required today). No migration, no `schema_version` bump
      (`PgStore::schema_version` is the highest embedded migration, `crates/htui-store/src/pg/mod.rs:389-391`,
      so a `0005` would rebuild every box's mirror — PRD risk `:393`). **Alternative:** R-2
      option (a), a `0005` adding typed per-run scope columns and evaluating the whole predicate in
      SQL. What a `0005` would have to touch is enumerated under Validation so the choice is
      costed.
- [ ] **OQ-2 — Does overlap cross boxes? (R-2's third question.)** Both stores filter the overlap
      set by `executing_box_id = box` (`crates/htui-core/src/store/mem.rs:3235-3242`,
      `crates/htui-store/src/pg/write.rs:2535-2549`); §4.7's predicate has no box term
      (`:1060-1067`) while its admission SQL does count per box (`:1096-1099`). **Default adopted
      (D82):** keep the box filter as a deliberate narrowing — a checkout is per box
      (`repo_box_path`), `R-ORCH-12` v1 executes only locally, and two boxes' trees can collide only
      at a push, which `htui` never does. **Alternative:** drop the filter so a run on box A
      serialises behind an overlapping run on box B. *(Amended after the fact-check, C10: the
      box filter is **not** backed by an item-level guarantee — see OQ-11.)*
- [ ] **OQ-11 — Two live runs of one item (fact-check C10).** `create_run` refuses an item that is
      not `open | failed` (`traits.rs:679-689`, `:685`), but `WriteStore::transition` has no
      live-run guard (`traits.rs:205-211`) and `Status::can_move_to` allows `queued → open` and
      `in_progress → open` (`crates/htui-core/src/model/item.rs:49-52`), so the seam admits a second
      live run on one item after such a walk-back — the shipped case
      `finish_run_moves_run_and_item_together` does exactly that and claims **both runs on one box**
      (`crates/htui-core/src/store/conformance.rs:5958-5985`), and `finish_run`'s "only when no
      other run of the item is non-terminal" (`traits.rs:932`, `:942`) exists because of it. The
      engine never walks an item back while its run is live (`cancel_run` finishes the run before
      the item returns to `open`, `crates/htui-orch/src/engine.rs:820-826`), so the only producers
      are a direct seam caller or two processes racing a cancel. **Default adopted (D82, R-20):
      a recorded known gap** — no same-item check in `claim_run`. **Alternative:** a cross-box
      same-item rule inside `claim_run` (`Claim::SameItem { with }`: refuse while any other
      non-terminal run of the same item exists on any box), which would also refuse the shipped
      case's leg 2 on one box, so that leg would have to be rewritten to claim one run and assert
      the refusal of the other — one more rule in both stores and one changed store case.
- [ ] **OQ-3 — `claim_run` answers why it refused.** Today it is `Result<bool>`
      (`crates/htui-core/src/store/traits.rs:711-718`) and the engine reports
      `ClaimRefused { run }` with "the box is full or the scope overlaps"
      (`crates/htui-orch/src/command.rs:235-242`), while §4.7 wants rule L's refusal to name "the
      holding run" (`:1085-1087`) and criteria 15/16 are about *which* rule held a run back.
      **Default adopted (D83):** `claim_run -> Result<Claim>`, `Claim::{Admitted, NotClaimable,
      SlotFull { running, limit }, Overlaps { with: RunId, rule: OverlapRule }}`, decided in the
      same order and the same transaction as today. Five implementors change
      (`mem.rs:4461` inside `impl WriteStore` from `:4180`, `pg/write.rs:2464`, `crates/htui-store/src/writer.rs:673`,
      `crates/htui-agent/src/conformance.rs:927`, `crates/htui-agent/tests/recorder.rs:621`).
      **Alternative:** keep `bool` and re-derive the reason after a refusal — impossible from the
      engine, which has no trait read of runs across items (by box or repo scope): `overlapping_runs`
      is inherent (`mem.rs:659`, `crates/htui-store/src/pg/read.rs:1729`), and `ReadStore::runs(item)`
      (`traits.rs:78`) sees only the same item's runs, so neither the slot count nor an overlap with
      another item's run can be re-derived.
- [ ] **OQ-4 — A refused claim leaves the run `queued`; who claims it later?** Manual mode has no
      queue (MOD-12's), and a `queued` run nobody re-claims holds its item at `queued` forever.
      **Default adopted (D84):** every refusal, rule L included, leaves the run `queued` and
      returns the reason; `Engine::claim(run)` (new, `pub`) is the one re-attempt path — claim
      then a leased walk — and milestone 6's `run_worker` calls it when one of its runs rests and
      at start; `CancelRun` already accepts a `queued` run (`command.rs:556-565`). **Deviation:**
      §4.7 says rule L is "**refused**" rather than silently queued (`:1085-1087`); here it is
      reported with the holding run and left queued, because §4.3's only refusal-at-claim exit is
      run `failed` (`:605`) and `finish_run`'s item mirror has no `queued → *` row for `failed`
      (`traits.rs:935-941`), so failing it would strand the item at `queued`. **Alternative:**
      cancel the run on a rule-L refusal (`finish_run(Cancelled)` mirrors `queued → open`).
- [ ] **OQ-5 — The artefact test for a no-commit step.** §4.9: "`after_hash` present for every
      repo in scope **and** a document of `output_kind` produced by this step" (`:1297`). A `prd`
      or `plan` step that writes its document and commits nothing legitimately leaves `after_hash`
      NULL (`:960`), so the literal test re-runs a finished no-commit step. **Default adopted
      (D90):** finished ⇔ the output document exists **and** (`run_step.finished_at` is set, or
      every repo in `run.repo_scope` has an `after_hash`), **scoped to a `running` step with
      `fanout_index >= 0`** (the judge is D95's). For a step still `running`, `finished_at` is set
      only by `finish_step`, which on a single step or a candidate runs after stage 5's `capture` +
      `record_commits` (`crates/htui-orch/src/engine.rs:1369-1401`, `:1939-1967`), so a set
      `finished_at` means the NULLs are real. The judge's `finish_step` (`engine.rs:2400-2411`) has
      no capture, and terminal transitions and `answer_gate` also stamp `finished_at`
      (`traits.rs:751-752`, `:790`) — neither reaches a `running` row. **Alternative:** the literal test, which re-runs such a step once (the safe
      direction, at the cost of a session).
- [ ] **OQ-6 — The sweep parks the item at `awaiting_approval`, not `blocked`.** §4.9's second row
      blocks the item when the retry budget is spent and its third row always does (`:1298-1299`), but `Status::can_move_to` has no road
      from `blocked` back to `in_progress` (`crates/htui-core/src/model/item.rs:55`; carried R-4),
      so `RetryStep` — which unparks through `awaiting_approval → in_progress`
      (`engine.rs:3324-3336`) — could never resume a swept run. **Default adopted (D94):** run
      and item both `awaiting_approval`, the reason in the step's `gate_note` and an `item_note`,
      exactly M4's OQ-3/D50 answer for the judge. **Alternative:** `blocked` as written, resumable
      only by milestone 6's `Unblock` (item `blocked → open`, the run abandoned).
- [ ] **OQ-7 — A clean-at-start `shared_serialized`/`local` tree that is dirty *now*.** §4.9
      resets it (`dirty = false` at stage 2 is the whole test, `:1298`). In `local` mode the
      checkout is the maintainer's own working tree, so uncommitted edits made *during* the run may
      be the maintainer's. **Default adopted (D92, D93):** the isolator re-reads `is_dirty` at
      reset time and refuses — the step parks `interrupted, tree not reset` — so recovery never
      runs `reset --hard` over uncommitted work of any origin (PRD risk `:390`). **Alternative:**
      the literal rule, which destroys the agent's partial edits and anyone else's with them.
- [ ] **OQ-8 — Are `worktree`/`copy` trees reset?** Criterion 18 says the unfinished step "has its
      worktree reset to `before_hash` and is retried" (`:2136-2137`). A retry is a new step with its
      own `htui/<step_id>` tree (M3 D23), so the old tree is never re-entered, and resetting it
      moves `htui/<old_step>` off the agent's commits (PRD risk `:389` is about losing committed
      work). **Default adopted (D92):** `worktree`/`copy` trees are left as they are; the retry's
      `before_hash` equals the interrupted step's (the primary never merged it), which is what the
      criterion's "reset" protects. `shared_serialized`/`local` — the checkout the retry *does*
      re-enter — are labelled `htui/<step_id>` at their current `HEAD` (a `gix` ref write, the
      label M3 D26 writes at capture) and then `reset --hard <before_hash>`, so nothing committed is
      lost. **Alternative:** reset every tree literally, naming the discarded tips in the note.
- [ ] **OQ-9 — A finished step is re-settled, not marked `done`.** §4.9's first row says
      `status = 'done'` (`:1297`); but a gated phase would then skip its human (invariant 5,
      `:124-127`), and a step whose `verify_outcome = 'fail'` would land `done` against §4.3
      `:637`. **Default adopted (D91):** rebuild the settle from the artefacts and hand it to
      `gate::apply` (`crates/htui-orch/src/gate.rs:373-438`), so the gate table decides. The M2
      warning about `:450` (`HANDOFF.md:251-254`) is honoured: the exhausted-`failed` cell stays
      the shipped `retry_or_fail` → `finish_run(Failed)` (`gate.rs:526-558`, `finish_run(Failed)` at `:545-552`), not `:450`'s
      escalation. **Alternative:** the literal `done`.
- [ ] **OQ-10 — Two new seam writers.** `take_lease` (D87) and `interrupt_step` (D89) take
      `WriteStore` from 63 to 65 methods and each needs both stores, the `Writer` arm, both spies,
      a conformance case and `.sqlx`. **Default adopted:** add them. **Alternatives:** for
      `interrupt_step`, M4 D51's two writes (`running → awaiting_approval`, then
      `answer_gate(Rejected, note)`), whose crash window leaves an `awaiting_approval` step under
      a `running` run that the sweep never touches (`:1306`) and whose `gate_outcome = rejected`
      misreports a crash as a rejection; for `take_lease`, unpark first and let `adopt_runs` take
      the expired lease, which adopts every other abandoned run on the box in the same call.

---

## Summary

Two things are missing for a run to be safe beside another run and to outlive its process.

**Overlap.** `claim_run` already locks the box row, counts `running` runs against
`max_concurrent_items`, and refuses any shared repo with a live run on the box
(`pg/write.rs:2464-2589`, run row `FOR UPDATE` at `:2479` then box row at `:2492`; `mem.rs:3209-3280`) — rules L and I collapsed into "same repo" and rule
P absent, so two worktree-isolated runs on one repo are refused (R-2). This milestone resolves a
`RunScope` at `StartRun` from `item.touched_paths`, the project's repos and the snapshot's
per-phase isolation, stores it in the snapshot (D79), and evaluates §4.7's predicate — rules L, I
and P over the non-wildcard `PathPrefix` — as **one pure function in `htui-core`** that both
stores call inside the same critical section, after the `repo_scope &&` prefilter they already run
(D80). `claim_run` says why it refused (D83); a refused run stays `queued` and `Engine::claim`
re-attempts it (D84). Criteria 15 and 16 follow.

**Crash.** The lease columns, `claim_run`'s lease, `refresh_lease` and `adopt_runs` all exist in the
seam — the lease columns on `Run` (`crates/htui-core/src/model/run.rs:201-206`; `lease_owner` is
deliberately not projected, `:203`, and `MemStore` keeps it in a `lease_owners` map), `claim_run`,
`refresh_lease` and `adopt_runs` (`traits.rs:691-741`) — and nothing calls the last two; the engine claims with a hard-coded
120 s lease and never refreshes it (`engine.rs:84`, `:446-457`), so any session longer than two
minutes is adoptable by a second process while it runs (PRD risk `:400`). This milestone puts
every walk under a heartbeat that refreshes at `lease_refresh_seconds` and **abandons** the walk
on a zero-row refresh (D85, D86), releases the lease when a walk parks and takes it back on unpark
(D87), and never lets a process adopt its own lease (D88). The sweep (`Engine::sweep`, D98) adopts
expired runs on this box and adjudicates every `running` step from its artefacts: finished steps
are re-settled through the gate table (D90, D91); unfinished ones are reset where resettable and
retried (D92), or — on a dirty `shared_serialized`/`local` tree — failed `interrupted, tree not
reset` and parked **without touching the tree** (D93, criterion 12). Fan-out candidates and the
judge are adjudicated without re-attempts (D95); a half-written park is completed (D96); a winner
whose reconcile was lost is re-reconciled idempotently before the walk goes on (D97). The adopted
runs are then walked by `Engine::resume`, criterion 3's path, under the heartbeat.

**Where milestone 5 hands off.** Milestone 6's `run_worker.rs` owns *when*: it calls `Engine::sweep`
at start and every TTL (`:1284`), `Engine::resume` for each adopted run, and `Engine::claim` for
queued runs when a slot frees; it also renders the reasons this milestone writes (the Runs tab's
"stalled, offline" label, ANA-2 risk 9 `:2072`). Criterion 19's offline window needs that worker
and MOD-6's `upload_pending` and is **milestone 6's** (D103).

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D79 | **The resolved scope is stored in the snapshot: `GraphSnapshot.scope: Option<RunScope>`, `#[serde(default)]`, written by `graph::resolve` at `StartRun`; `GraphSnapshot::V` is not bumped and `topology` is unchanged.** `RunScope { repos: BTreeMap<RepoId, RepoScope> }`, `RepoScope { isolated: bool, local: bool, prefixes: Vec<String> }` (empty `prefixes` = unknown = the whole repo). `run.repo_scope` stays the `UUID[]` prefilter and equals `scope.repos`' keys. A snapshot with `scope: None` (every row written before this milestone, the fixture runs, chat runs) is read **conservatively** (D80). | ANA-2 `:1035` ("resolved at queue time and stored on the run") and invariant 2: resolving at claim time from the live `item.touched_paths` would let an edit (MOD-13's editor) change a running run's admission answer. The snapshot is already that record, per run, immutable, on both stores; `topology` hashes `phases[]` alone (`graph.rs:223-233`), so `FEATURE_TOPOLOGY` and every stored digest are unaffected. OQ-1 gives the migration alternative and why it costs a mirror rebuild. |
| D80 | **The predicate is `htui_core::model::overlap::overlaps(a: &RunScope, b: &RunScope) -> Option<OverlapRule>`, pure, and both stores call it inside `claim_run`'s critical section**: `MemStore` over its in-memory rows (`mem.rs:3209-3280`), `PgStore` over the `graph_snapshot` of the rows its existing prefilter returns — `executing_box_id = $box AND status IN ('running','awaiting_approval') AND repo_scope && $2` (`pg/write.rs:2535-2549`), now selecting `id, graph_snapshot` rather than `EXISTS` — with the box row still `FOR UPDATE` (`:2491-2502`). `OverlapRule::{Local, NotIsolated, Paths}` is rules L, I, P in §4.7's order (`:1063-1066`). `scope_of(snapshot: &Value, repo_scope: &[RepoId]) -> RunScope` decodes the `scope` key and, when absent or undecodable, answers the conservative scope: every repo of `repo_scope` `isolated = false`, `prefixes = []` — which is exactly today's "shared repo overlaps". | R-2 option (b)'s decisive argument (`mod-4-orch-seam.blueprint.md:1365`): the rule stays one implementation that `MemStore` and `PgStore` share, so the conformance suite cannot see them disagree; and D79 means (b) no longer has to read live item rows under the lock. The predicate cannot live in `htui-orch` (`overlap.rs`, where ANA-2 `:1669` sketches it) because the stores cannot see it: the orchestrator depends on `htui-core` and never on `htui-store` (invariant 10, `:143-146`: one cache writer, and the orchestrator reaches Postgres only through `WriteStore`), so a rule both stores evaluate has to live in `htui-core`; `htui-orch/src/overlap.rs` keeps the resolution half (D81). The conservative fallback keeps `claim_run_admits_one_and_refuses_the_second` (`crates/htui-core/src/store/conformance.rs:4057-4223`, doc from `:4051`, whose runs carry a scope-less `run_snapshot()`, `:3760-3780`) green with the same admit/refuse outcomes, its assertions rewritten from `bool` to `Claim` per T2. |
| D81 | **`htui_orch::overlap::resolve(item, repos, phases, requested) -> Result<(Vec<RepoId>, RunScope), ResolveError>`** replaces `graph::resolve_scope` (`graph.rs:245-260`) as what `graph::resolve` calls (`:351-352`). Each `touched_paths` entry is split at its first `:`; a left side naming a repo of the project is qualified, anything else is a bare glob of the primary repo (`:1025`); a qualified name no repo carries is `ResolveError::UnknownTouchedRepo { item, name }` at `StartRun`. `repos` = every repo named by a qualified entry, plus the primary when any bare glob or no glob is declared (`:1051-1052`); a `requested` scope (`Command::StartRun { repo_scope }`, `command.rs:50`) is honoured as given, with D14's empty-with-primary refusal kept (`graph.rs:251-255`). Per repo, `isolated` = every phase's resolved isolation (`graph.rs:518`) is `Worktree \| Copy`, `local` = any phase is `Local` (`:1053-1055` defines `isolated[r]`; `local[r]` is only the field comment at `:1046`; a phase's isolation applies to every repo in scope). **The prefix is the existing `htui_core::prompt::excerpt::PathPrefix::parse(touched, primary_repo)`** (`crates/htui-core/src/prompt/excerpt.rs:43-89`, D104), not a second implementation: no metacharacter → the whole path; otherwise truncate before the first of `*?[{` (`GLOB_META`, `:18`) and keep up to and including the last `/` (none → `""`), so `**` is `""` (`:1074-1077`). Its qualifier rule — the text before the first `:` when non-empty and free of `/` — is the one `overlap::resolve` uses, and the parsed repo slug is then mapped to a `RepoId` by `repo.name`. | §4.7's inputs, verbatim. Refusing an unknown repo name is the loud direction: silently reading `other:src/**` as a primary-repo glob would under-report overlap in the repo the author meant. `ResolveError::UnknownTouchedRepo` joins D63/D64's start-time rules in `resume`'s not-comparable list (`engine.rs:957-961`, the arm through `:970`), so, by extending M4 behaviour 4 (`HANDOFF.md:319-321`: `resume` keeps walking the snapshot when the live graph no longer resolves under lowered caps) to `UnknownTouchedRepo`, an edit to `touched_paths` after queue time never stops a run. |
| D82 | **The overlap set stays per box (OQ-2), and two live runs of one item are a recorded known gap (OQ-11, R-20) — no same-item rule in `claim_run`.** | See OQ-2 and OQ-11. *Rewritten after the fact-check (C10):* the draft claimed `create_run`'s `open \| failed → queued` guard (`traits.rs:679-689`) means "one item never has two live runs"; it does not — the seam admits a second live run after a `queued → open` or `in_progress → open` walk-back (`item.rs:49-52`; the shipped `conformance.rs:5958-5985` claims both on one box). The normal engine path queues one run per item and walks an item back only after finishing its run (`engine.rs:820-826`), so the gap is reachable only by a direct seam caller or a cancel race between two processes; the per-box overlap set therefore does not, by itself, serialise two runs of one item on different boxes. OQ-11 prices the check. |
| D83 | **`claim_run -> Result<Claim>` (OQ-3).** Decision order unchanged: `NotFound` for run then box (`traits.rs:707-710`), `NotClaimable` (not `queued`, or `target_box_id != box`), `SlotFull { running, limit }`, then the first overlapping run in `(queued_at, id)` order as `Overlaps { with, rule }`, then `Admitted` with today's writes. `EngineError::ClaimRefused { run, claim }` renders the reason: `claim refused: box full (2 of 2 running)`, `claim refused: overlaps run <id> (<rule>)`. | One transaction already decides all of this (`pg/write.rs:2464-2589`); returning the verdict it computed costs a type, not a query. Rule L's "a message naming the holding run" (`:1086-1087`) and criteria 15/16's assertions need the reason. `Claim` lives in `htui_core::model::overlap` beside `OverlapRule`, so `traits.rs` names it without a new module. |
| D84 | **A refused claim leaves the run `queued`; `pub async fn Engine::claim(run) -> Result<CommandOutcome, EngineError>` re-attempts it** — the tail of `start_run` (`engine.rs:446-460`) factored out: `claim_run`, then the leased walk (D86), `CommandOutcome::Started { run, rest }`. No new `Command` variant (§6.2 has none, `docs/ANA-2.md:1558-1571`); milestone 6 decides the verb. | OQ-4. |
| D85 | **Lease timings come from `app_setting.lease_ttl_seconds` and `lease_refresh_seconds`** (`docs/ANA-2.md:1518-1519`; seeded `120`/`60` by `0003_orchestration.sql:135-136`), read from `EngineParts.app` (`engine.rs:278-280`) through a `recover::LeaseTimes::from_app` with the built-in `120`/`60` fallbacks; a refresh `>= ttl` or `0` reads as `ttl / 2`. `const LEASE_SECONDS` (`engine.rs:83-84`) is removed — its doc cites `:1408`, which is §4.10's timer-approval text, not the lease (`:1276-1282`). | PRD D2 adopts §10's 120 s / 60 s (`:337-339`) as *defaults*; the keys exist so they are settable. A refresh interval at or above the TTL would let a live lease expire between beats. |
| D86 | **Every walk runs under a heartbeat.** A private `walk_leased(run, walk)` polls `futures::future::select(walk, recover::heartbeat(..))` in the same task — **both futures pinned** (`select` needs `Future + Unpin`: `Box::pin`, or `std::pin::pin!` inside a scope that ends right after the `select`), and on `Either::Right` the returned still-pending walk is **dropped explicitly** (or its pinned scope left) *before* `isolator.release` runs, because dropping the `select`'s output does not drop a stack-pinned walk (the fact-check's probe, rustc 1.98.1 / futures 0.3.34) — no `'static` bound; `futures` is already a dependency, `crates/htui-orch/Cargo.toml` "MOD-4 milestone 4 (plan D58)"). The heartbeat sleeps `refresh` with `tokio::time::sleep` (the `time` feature is on, same manifest), then `refresh_lease(run, owner, clock.now() + ttl)`: `Ok(true)` → loop; `Ok(false)` → `Abandoned`; `Err` → `tracing::warn!` and loop — an inference from the offline window (`:1325-1336`: the lease is left to expire offline because the refresh is a write, and the reconnect sweep adjudicates), which implies a refresh error is not a taken lease; ANA-2 does not say it in those words. On `Abandoned` the walk future is **dropped where it stands**, `isolator.release(run)` drops this process's guards for the run (D99), nothing else is written, and the caller gets `EngineError::LeaseLost { run }`. Every entry that walks goes through it: `start_run`/`claim`, `answer_gate`, `retry_step`, `retry_group`, `select_fanout` and `resume` — the eight `run_to_rest` call sites at `engine.rs:459`, `:529`, `:575`, `:661`, `:703`, `:761`, `:969`, `:998` — while `run_to_rest` itself (`:847`, `pub`) stays the unleased primitive for callers that manage the lease. | ANA-2 `:1280-1282`: "a zero-row refresh means the lease was taken and the orchestrator abandons the run without writing further". Dropping the future is the only way to stop *all* further writes, including a candidate mid-`join_all` (M4 behaviour 5, `HANDOFF.md:321-322`); the adopter's sweep (D95) adjudicates what the drop left. A dropped session's agent child is killed by `ChildGuard::drop` (`crates/htui-agent/src/launch.rs:977-993`, `start_kill` at `:989`). `FakeDriver` never suspends (`crates/htui-agent/src/fake.rs:393-408`, M4 D58), so every shipped case finishes before the first beat and no shipped case changes. |
| D87 | **A walk that rests with the run not `running` releases the lease** — `refresh_lease(run, owner, now)`, the existing writer, so `lease_expires_at = now` and the run reads as expired — **and every unpark takes the lease first** through a new `WriteStore::take_lease(run, box, owner, now, until) -> Result<bool>`: `lease_owner = owner, lease_box_id = box, lease_expires_at = until` where `status IN ('running','awaiting_approval') AND executing_box_id = box AND (lease_owner = owner OR lease_owner IS NULL OR lease_expires_at IS NULL OR lease_expires_at <= now)`. `Ok(false)` → `EngineError::LeaseHeld { run }`, nothing unparked. The four `unpark` call sites (`engine.rs:517`, `:650`, `:697`, `:754`) take it before `unpark` (`:3324-3336`). | A run parked for a week keeps the lease its claim wrote; a gate answered after a restart comes from a process with a new `owner`, so `refresh_lease` (a CAS on the owner, `traits.rs:720-725`) answers `false` and the walk would abandon itself. Taking the lease while the run is still parked, and only when it is ours or expired, means unpark → `running` never exposes an expired lease to another sweep, and never displaces a live orchestrator (`:2076`). Releasing at a park makes the take succeed across processes without waiting a TTL. |
| D88 | **`adopt_runs` never adopts a run whose `lease_owner` is the sweeper**: `AND lease_owner IS DISTINCT FROM $owner` on `PgStore` (`pg/write.rs:2643-2692`), the same clause on `MemStore` (`mem.rs:3302-3333`), and one sentence in the contract (`traits.rs:727-742`). | Otherwise a process whose heartbeat stalled past the TTL (a long synchronous stretch, a suspended laptop) would adopt its own live run in its own sweep and walk it twice under one owner, where the CAS cannot tell the two walkers apart. The cost — a run whose walking *task* died inside a live process is not re-adopted until that process restarts — is recorded as R-12. |
| D89 | **New `WriteStore::interrupt_step(step, note, at) -> Result<bool>`: `running → failed`, `gate_note = note`, `finished_at = COALESCE(finished_at, at)`, `gate_outcome` untouched (NULL); `Ok(false)` when the step is not `running`; `NotFound` for an unknown step.** One compare-and-set on both stores. **No migration**: `lease_owner`/`lease_box_id`/`lease_expires_at` exist since `0003` (`:41-43`); `run_step.gate_note`/`gate_outcome`/`finished_at` and status `failed` since `0001_init.sql:481-492`. | §4.9 writes `status = 'failed'` with `gate_note = 'interrupted'` (`:1298-1299`). No shipped writer sets `gate_note` on the way from `running` to `failed` (M4 D51's premise, `mod-4-orch-fanout.plan.md:190`); M4's two-write workaround has the crash window OQ-10 names, which is fatal exactly here, and records `gate_outcome = rejected` for a crash. |
| D90 | **The artefact test (OQ-5): a `running` step with `fanout_index >= 0` is *finished* when a document of `phase.output_kind` produced by it exists (`Engine::output_of`, `engine.rs:3466`) and either `finished_at` is set or every repo of `run.repo_scope` has a `run_step_commit` row with `after_hash` present.** Empty `repo_scope` (the demo fixture) makes the second half vacuous. | §4.9 `:1297`, refined per OQ-5. The document half is the invariant-5 artefact (`:124-127`); the commit half is §4.6's stage-5 row (`:960`; `:959` is the stage-2 row). |
| D91 | **A finished step is re-settled and handed to the gate table (OQ-9).** `recover::resettle` builds `gate::SettleInput` — all eight fields (`gate.rs:203-231`) — with `driver: Ok(DoneEvent { stop_reason: EndTurn })`, `cap_breach: None`, `started_at` = the row's `started_at`, `now` = the sweep clock's instant (inert, since `deadline_seconds: None`), the output document, `is_review` by `phase.name` (as stage 5 does, `engine.rs:1381`), and `verify_outcome` = the row's when `finished_at` is set, else the step's last `command_run` mapped `Done`+0 → `pass`, `Done`+non-zero → `fail`, `Failed` → `unavailable`, none → `None` (verify runs before capture, `engine.rs:1356-1370`, so an `after_hash` implies the row); then `gate::apply(ctx, step, phase, settle)` (`gate.rs:373`), and `Landing::Advance` → `reconcile_done_step` (`engine.rs:2857-2889`), `Retry` → `admit`, `Rest` → the rest — the exact tail of `walk_live_step` (`engine.rs:1403-1426`). | A step that finished and lost only its bookkeeping (`:1297`, the River case `:1301-1304`) deserves the outcome its artefacts earn, and the gate table is the one place that decides outcomes. The deadline is dropped because the sweep's clock is not the step's; the cap is dropped because the recorder's breach is not durable (the partial `usage` is, `:1272-1274`, but the breach decision is not). Recorded as R-13. |
| D92 | **An unfinished step (D90 false) is reset where resettable and retried.** Resettable per §4.9 `:1298`: every `run_step_tree` row is `worktree \| copy`, or `shared_serialized \| local` with `dirty = false`. The engine calls `isolator.reset(step, &trees)` (D99), which is **all-or-nothing** — it checks every row before touching any — and for `worktree`/`copy` touches nothing (OQ-8), for `shared_serialized`/`local` refuses a checkout that is dirty *now* (OQ-7) and otherwise labels `htui/<step_id>` at the current `HEAD` when it differs from the row's `base_ref` (the stage-2 `before_hash`; `RunStepTree` has no `before_hash` field, `model/run.rs:426-439`) and runs `git reset --hard <base_ref>` (M3's fifth verb, `crates/htui-orch/src/isolate/git.rs:609`). Then `interrupt_step(step, "interrupted")`, an `item_note` naming the step, each tree, its `before_hash` and any label written, and §4.4's retry admission: `may_attempt(attempt + 1, retry_limit)` (`crates/htui-orch/src/status.rs:27-29`) → `admit(run, snapshot, phase, attempt + 1)` (`engine.rs:1012`), else D94's park with reason `interrupted: retry budget spent`. A step with no tree rows at all (a crash between `pending → running` and `upsert_step_tree`, `engine.rs:1264-1311`) is vacuously resettable. | §4.9 `:1298` and criterion 18's second half (`:2136-2137`). Out of budget, §4.9 parks (`:1298`; §4.3 `:611` is the sweep's park of an unrecoverable step after lease expiry) rather than failing the run — an interruption is not the agent's failed settle, so M2's `:450` warning does not apply (that cell stays `retry_or_fail`'s, D91). |
| D93 | **A tree that must not be reset is never touched.** A `shared_serialized`/`local` row with `dirty = true`, or an `isolator.reset` refusal (live dirt, OQ-7), fails the step through `interrupt_step(step, "interrupted, tree not reset")`, parks per D94, and writes an `item_note` naming every tree's path and `before_hash` — the plan's addition: `:1299` has the Runs tab name them, which is milestone 6's render. No `git` write runs on that path. | Criterion 12 (`:2118-2119`) and PRD risk `:390`. The dirt is recorded at stage 2 by M3 D24 (`is_dirty`, untracked excluded, `git.rs:1256-1263`), which is the record `:964-968` makes the non-resettable flag. |
| D94 | **Every park the sweep writes is run `running → awaiting_approval` then item `in_progress → awaiting_approval` then the note (OQ-6)** — `engine.rs:2900-2933`'s `park_run` order (doc `:2891-2899`) with the step already `failed`; `run.failure` stays NULL (R-3); `Rest.failure` is a new `RunFailure::Interrupted { phase, reset: bool }`, `Display` `interrupted: <phase>` / `interrupted, tree not reset: <phase>`. `RetryStep` on the failed step resumes it (`retry_enabled` accepts `failed`, ANA-2 `:1562`). | OQ-6. `Rest.failure` is the typed stop reason of the transition that caused it (`engine.rs:3338-3347`), and the sweep is that transition. |
| D95 | **Fan-out rows are adjudicated without re-attempts.** A `running` candidate (`fanout_index >= 0` in a `fan_out > 1` phase) that is finished becomes `done` with its `verify_outcome` recorded and not applied — M4 D48's candidate settle — and one that is not becomes `failed` through `interrupt_step(…, "interrupted")` with **no reset** and no park: the group's cursor (M4 D59) then drives or selects over the survivors exactly as it does after a failed candidate. A `running` judge (`fanout_index = -1`) is `interrupt_step(…, "interrupted")`, and the `Select` arm re-parks the slot for a human (M4 D59: a failed judge re-parks rather than re-judges). | §4.9 `:1317-1319` ("completed siblings survive … selection then runs over whatever survived"). Its "only the failed index is re-attempted" clause stays unimplemented, M4 D48's recorded deviation, for M4 D65's reason: a re-attempt of index *i* must be `(position, attempt + 1, i)` under `UNIQUE (run_id, position, attempt, fanout_index)` and would put two attempts in one comparison. A `shared_serialized` sibling left dirty makes the next sibling refuse `dirty_tree_not_reset` (M4 D72), which is readable and resets nothing. This closes M4 behaviour 5 (`HANDOFF.md:321-322`): the rows a dropped `drive_group` left are adjudicated, and the guards go with the process (D99). |
| D96 | **On adoption the run's status is re-derived from its steps** (`:618-621`: "it is what the recovery sweep re-runs on adoption"): a `running` run with no `running` step and a step at `awaiting_approval` completes the park — `transition_run(running → awaiting_approval)`, item `in_progress → awaiting_approval` — and is not walked. | `gate::park` is three compare-and-sets (`gate.rs:489-518`, M2 H-10, carried R-5); a crash after the first leaves a waiting step under a running run, which `run_to_rest` would report as a rest while the run stays `running` and adoptable forever. This closes R-5's crash half. |
| D97 | **On adoption the frontier winner is re-reconciled before the walk goes on. The frontier ignores retired rows** *(redefined after the fact-check, C129)*: it is the latest-attempt `done` winner (`selected`, or the one row of a `fan_out = 1` slot) at the highest position *p* that has one, **provided every row at a position > *p* is `superseded` or `cancelled`**; `reconcile_done_step(run, step, siblings)` (`engine.rs:2857-2889`) runs again, and a refusal parks through `park_run` (the R-7 shape). | Stage 6 moves the step `done` (`gate.rs:387-392`) *before* `reconcile_done_step` merges it (`engine.rs:1409-1412`); a crash between them leaves an unmerged winner that the cursor passes (`done` completes a position, `status.rs:223-256`) — and at the last position `Cursor::Finished` would `finish_run(Done)` over an unmerged primary (`engine.rs:871-883`). **Why retired rows are ignored:** the draft's "no row at the next position" is false after a review rejection — `review_loop` retires rows `target..=review` (`gate.rs:711`, `retire_slot` `:884-901`: the rejected review is `failed`, `traits.rs:789`, and is cancelled) and only then does the walk create implement `attempt + 1` (`:713-714`), so when that attempt reaches `done`, the retired review of attempt 1 already sits at the next position and the draft rule found no frontier. Every **live** row at a later position is created by `run_to_rest` after that position's reconcile returned `Ok` (first attempts; fan-out winners are reconciled before the next `admit`, `engine.rs:758`, `:2150`); retired rows predate the winner. Reconcile is crash-idempotent by M3 H-3 (`crates/htui-orch/src/isolate/real.rs:1001-1006`, `:1036-1043`: a `HEAD` whose parents are `[before, after]` is answered, not refused) and the identity for `shared_serialized`/`local`. |
| D98 | **`pub async fn Engine::sweep() -> Result<Vec<Adopted>, EngineError>`**: `adopt_runs(box, owner, now, now + ttl)` (`traits.rs:735-741`), then per adopted run, in `queued_at` order, D95/D90–D93 for every `running` step, then D96, then D97; `Adopted { run, next: Next::{Walk, Parked(Rest), Finished(Rest)} }`. It **does not walk**: a `Next::Walk` run is walked by the caller through `Engine::resume(run)` (`engine.rs:940`, criterion 3's topology check), which D86 puts under the heartbeat. Steps at `awaiting_approval` are never touched (`:1306-1307`); `pending` steps are left for the walk (`:1307`). The sweep releases (D87) the lease of every run it leaves parked. | §4.9 `:1284-1307`. Walking inside the sweep would serialise every adopted run behind the first one's sessions; milestone 6's `run_worker` "spawns one engine task per active run" (`:1681-1682`). |
| D99 | **Two new `Isolator` verbs** (`crates/htui-orch/src/isolate.rs:126-207`): `reset(step, trees) -> IsolatorFuture<ResetReport>` (D92's rule; `ResetReport { labelled: Vec<(RepoId, String)>, refused: Vec<(RepoId, String)> }`, a refusal carrying `dirty_tree_not_reset: <path>`, `real.rs:114-116`) and `release(run) -> IsolatorFuture<()>` (drop every guard any step of `run` holds, touch no tree — `GixIsolator::release_run`, `real.rs:513-527`, today reachable only through `cleanup` (`real.rs:1387`), which also removes trees — `release` exposes it on its own; the fake drops its single `serial` guard for the run's steps, `fake.rs:86-91`). `GixIsolator` and `FakeIsolator` are the only implementors (`real.rs:1214`, `crates/htui-orch/src/fake.rs:280`). No new `git` verb: the label is `git::create_branch` (a `gix` ref write, `git.rs:1337`) and the reset is `Cli::reset_hard`. | `cleanup(run, _)` (`isolate.rs:203-206`; `GixIsolator`'s impl at `real.rs:1356-1399` removes the worktree/copy trees and the run dir) removes trees, which would destroy what the adopter must read; an abandoning process needs only its guards gone, or its own later `shared_serialized` steps on that repo would wait on a guard nobody releases (M3 D43's guards are in-process, `real.rs:197-198`, `:219-229`). |
| D100 | **A crash in the harness is a dropped future; a restart is a second orchestrator over the same store.** `FakeOrchestrator::stall_after_done(phase, attempt, write_output: bool)` makes `after_done` (`engine.rs:1348-1353`) write the output document or not and then never return; a case polls `dispatch` once with `futures::FutureExt::now_or_never` (the fakes never suspend otherwise, D86), which leaves the step `running` exactly where a killed process would. `FakeOrchestrator::restarted(&self) -> Self` clones the `MemStore` (a `Clone` handle over `Arc<RwLock<State>>`, so a clone sees the same rows; `crates/htui-core/src/store/mem.rs:55-58`), mints a fresh `owner`, a fresh `FakeIsolator` and a clock advanced past the TTL. The `Orchestrate` trait (`crates/htui-orch/src/conformance.rs:55-113`) gains `sweep`, `claim`, `restarted` and `stall_after_done`. "Both artefacts present" is the stalled state plus a direct `record_commits` of an `after_hash`, i.e. capture landed and the settle did not. | Criterion 18 (`:2135-2137`) against `FakeDriver` + `FakeIsolator`, deterministically, without a process kill. |
| D101 | **"No orphaned agent process" is proven for the in-process abandon only.** A dropped walk kills its agent through `ChildGuard::drop`'s `start_kill` on the process group (`launch.rs:977-993`, `start_kill` at `:989`; `:1128`). A `SIGKILL`ed orchestrator cannot run that `Drop`, and its agent — a process-group leader, so not even in the terminal's group — survives; nothing records its pid for the adopter to signal. | Recorded as **R-10**, not closed: signalling a stale pid from another process risks killing a reused one, and a death signal needs `unsafe` or a new dependency (`unsafe_code = "forbid"`). The cheap half of criterion 18 is provable now; the other half is a milestone 6 / MOD-16 decision. |
| D102 | **Recovery runs no `git gc` and no `git worktree prune`.** The sweep's only `git` write is D92's `reset --hard` after its label. | PRD risk `:389`; M3 D46 (`git.rs:19`, `real.rs:1395`). An acceptance grep pins it. |
| D103 | **Criterion 19 (the offline window, `:2138-2140`) is milestone 6's.** Milestone 5 makes its engine half true — the heartbeat does not abandon on a store error (D86) and the sweep adjudicates by artefacts — but the proof needs `run_worker`, `go_offline`, MOD-6's `upload_pending` and a reconnect, none of which exist in `htui-orch`. | ANA-2 `:1338-1340` ("nothing new is built for this"). The PRD lists only criterion 18 for this milestone (`:192`). |
| D104 | **(fact-check, independence) One `PathPrefix`.** T1 writes no prefix parser: `RepoScope.prefixes` stores the `prefix` strings `htui_core::prompt::excerpt::PathPrefix::parse` already produces (`excerpt.rs:43-89`), `overlaps`' rule P compares two stored strings with `x.starts_with(y) \|\| y.starts_with(x)` on bytes (`:1071`), and T4's `overlap::resolve` is the one caller of `parse`. | The existing type already implements ANA-2 `:1074-1077` and the `repo:glob` qualifier (`:1025`) and is `pub`; two implementations of one rule can drift. `htui-core` holds both modules, so no new dependency edge. |
| D105 | **(fact-check, independence) Wave A runs as shape (b): T1 first and alone until green on the real tree; then T3 and T5 in parallel, each in its own git worktree (`isolation: "worktree"`), merged T3 then T5, with `cargo test -p htui-orch --all-features -- --test-threads=1` re-run on the real tree after each merge.** | The three sets are file-disjoint but not build-disjoint: T1 is a dependency of `htui-orch`, and T3's red commit (tests calling `reset`/`release`) or mid-edit state (trait methods with no default body, before both implementors have them) stops `htui-orch` compiling, which fails T5's gate for T3's reasons and vice versa. T1 is small and pure, so serialising it costs little and removes the only cross-crate coupling; T3 ∥ T5 then share no file and no type, so separate worktrees make each one's red state private. Shape (a) (three worktrees) would add a third merge for no parallelism gain. |
| D106 | **(fact-check, independence) T5 is pinned so it cannot couple to T1 or T2.** `classify`'s scope argument is `run_scope: &[RepoId]` (`run.repo_scope`, D90), never T1's `RunScope`. `heartbeat` takes the refresh as a closure — `heartbeat<F, Fut>(mut refresh: F, clock, times) -> Heartbeat` with `F: FnMut(DateTime<Utc>) -> Fut`, `Fut: Future<Output = Result<bool, StoreError>>` — so its tests script `Ok(true)` / `Ok(false)` / `Err` with no store and no `claim_run`; the engine (T6) passes `\|until\| store.refresh_lease(run, owner, until)`. T5's tests build `GraphSnapshot`s by decoding JSON (as `status.rs:334` does), never by struct literal, and never call `claim_run`. T3 does **not** touch `lib.rs`: `ResetReport` is re-exported by T6's pass beside the other isolate types (`lib.rs:48-51`). | Without the pins, T5's heartbeat tests would need a leased running run — which only `claim_run` writes (`mem.rs:6083` pattern), whose return type T2 changes — and a hand-built snapshot literal would break when T2 adds `scope`; either would make `recover.rs` a silent member of T2's file set. `recover.rs` is added to T2's list anyway as a guard (T2 greps it and must find neither), so the file-set intersection stays honest. |

## What this milestone touches from the carried list

| Carried | Disposition |
|---|---|
| **R-2** (`claim_run`'s repo-set predicate; rules I and P unevaluable; the box filter) — M1 blueprint §4.6 (`mod-4-orch-seam.blueprint.md:1365`), deferred to milestone 5 (`HANDOFF.md:214-217`, `:229-230`) | **Resolved here.** D79 stores the scope, D80 evaluates L/I/P in both stores' critical section, D81 resolves it, D82 answers the box question, D83 names the rule. Criterion 15's "two worktree-isolated runs on one repo with disjoint paths" now runs concurrently. |
| **R-3** (`run.failure` NULL on a parked run) — M2 blueprint `:27`, `:956` | **Touched, re-deferred to milestone 6.** D94's parks follow the rule (on the graph-run path `finish_run` is the only writer of `failure`, M2 D7; `fail_run`, `traits.rs:918-925`, is the chat path's run-only writer and the engine never calls it); the reason is the step's `gate_note` and the `item_note`, which milestone 6's Runs tab renders. Nothing in milestone 5 reads `run.failure` of a parked run. |
| **R-4** (a `blocked` item cannot resume) — M2 blueprint `:28`, `:956` | **Avoided, re-deferred to milestone 6.** D94 parks at `awaiting_approval` for exactly this reason (OQ-6); `Unblock` is milestone 6's (§6.2 `:1568-1569`). |
| **R-5** (the three-write park; no `gate_outcome = skipped` writer) — M2 blueprint `:29`, H-9 `:913`, H-10 `:914` | **Crash half closed** by D96 (a half-written park is completed on adoption). **The `skipped` writer is re-deferred to milestone 6**, whose step list renders "gate state" (PRD `:195`) and is the first reader. |
| **R-6** (`phase_agent` copy and `is_override` writers) — M2 blueprint `:30`, `:956` | **Untouched, re-deferred.** No criterion of milestone 5 or 6 needs them; the owner is the next item that edits override graphs (MOD-15's phase editor). |
| **R-7** (a reconcile-refusal park has no resume verb) — M3 blueprint F-G `:25`, `:784` | **Re-deferred to milestone 6.** The sweep never touches a parked run (`:1306-1307`), so it cannot be the resume verb; `command.rs:432`'s "milestone 5's sweep owns that retry" is wrong against ANA-2 and is rewritten (T6) to name milestone 6's `Unblock`-shaped verb. Milestone 5 closes only R-7's *crash* half: a reconcile lost to a crash is redone on adoption (D97). |
| **R-9** (`NoProgressReview` unreachable) — M4 blueprint F-B `:29`, `:895` | **Re-deferred.** Unrelated to overlap or recovery; fixing it changes which stop reason shipped loop cases reach. Proposed for milestone 6's close-out or its own `CLEAN` item. |
| **M4 behaviour 5** (`drive_group` is not cancel-safe, `HANDOFF.md:321-322`) | **Closed** by D86 (a drop is now the *intended* abandon), D95 (the rows it leaves are adjudicated) and D99 (`release(run)`). `engine.rs:1590-1595`'s doc is updated to name them. |
| **M2's `:450` vs `:575`/`:608`** (`HANDOFF.md:251-254`; `mod-4-orch-engine.plan.md:223`) | **Honoured.** D91 routes a re-settled failure through the shipped `retry_or_fail`; D92's out-of-budget park is §4.9's own rule for an interruption, not `:450`'s cell. |
| **M3: `dirty` at stage 2; never reset a dirty tree; no `worktree prune`; no `gc`** | **D92, D93, D102.** OQ-7 goes further than §4.9 (live dirt is also never reset). |
| **M3 D43** (in-process `shared_serialized` guard) vs PRD scope `:229-230` ("a Postgres advisory lock per `(box, repo)`") | **Relied on, recorded.** Cross-process exclusion of a shared checkout is now rule I's job (D80: `shared_serialized` is not isolated, so two runs on one repo overlap), which is why D99's `release` is enough. |
| **`0004` and migration numbering** | **No `0005`** (D79). What a `0005` would touch is listed under Validation. |
| **MOD-36** (weighted candidate assignment) | **Untouched**: `AgentSelector::select` is not called by the sweep except through `admit` on a retry (D92), with its `fanout_index` (M4 D71). |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| Admission in one transaction, box row locked, refusal writes nothing | `PgStore::claim_run` | `crates/htui-store/src/pg/write.rs:2464-2589` |
| The same rule on `MemStore`, decided before the first write | `State::claim_run` | `crates/htui-core/src/store/mem.rs:3209-3280` |
| Compare-and-set writer with `Ok(false)` + `NotFound` told apart by one follow-up read | `PgStore::refresh_lease` | `pg/write.rs:2601-2631` |
| `UPDATE … RETURNING` ordered through a CTE | `PgStore::adopt_runs` | `pg/write.rs:2643-2692` |
| Delegating `Writer` arm | `Writer::claim_run` | `crates/htui-store/src/writer.rs:673-705` |
| Store conformance case with both-store pins | `lease_refresh_is_a_cas_on_owner` + `EXPECTED_CASES` + `mem_store.rs` | `crates/htui-core/src/store/conformance.rs:4231`; `crates/htui-store/tests/pg_conformance.rs:19`; `crates/htui-core/tests/mem_store.rs:35-43` |
| Postgres race proven with two pools | `admission_is_serialised_by_the_box_row_lock` | `crates/htui-store/tests/pg_criteria.rs:585-664` |
| A defaulted snapshot field without a `V` bump | `SnapshotJudge.template` | `crates/htui-core/src/model/run.rs:553-557` (the struct `:545-558`) |
| Pure verdict over rows, not a `bool` | `quota::Availability` / `SkipReason`; `fanout::Route` | `crates/htui-core/src/model/quota.rs:349-381`; `crates/htui-orch/src/fanout.rs` |
| Park with the step already settled: run, item, note | `park_run` | `crates/htui-orch/src/engine.rs:2900-2933` |
| Stage 6's tail after a settle | `walk_live_step`'s gate match | `engine.rs:1403-1426` |
| Resume that re-resolves and walks | `Engine::resume` | `engine.rs:940-999` |
| An isolator verb with per-mode behaviour and a `blocking` `gix` read | `reconcile` / `reconcile_isolated` | `crates/htui-orch/src/isolate/real.rs:1006-1063` (the trait `reconcile` at `:1322`) |
| A `git` write through the wrapper, retried | `Cli::reset_hard` + `git::with_retry` | `crates/htui-orch/src/isolate/git.rs:609`, `real.rs:1114` (`with_retry("reset --hard", …)`) |
| Git-backed test that skips without `git` | `let Some(git) = skip_without_git!() else { return; };` | `crates/htui-orch/tests/gix_isolator.rs` (its existing cases) |
| `htui-orch` conformance case + two count pins | `CASES` + `cases_are_unique_and_thirty_six` + `cases_len_is_thirty_six` | `crates/htui-orch/src/conformance.rs:186-268`, `:3153-3172`; `crates/htui-orch/tests/fake_conformance.rs:14-17` |
| Paused-clock timing test | `with_retry`'s 200/400/800 ms under `test-util` | `crates/htui-orch/Cargo.toml` dev-dependencies (`tokio` `test-util`) |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/overlap.rs` | create | T1 | `RunScope`, `RepoScope`, `OverlapRule`, `overlaps`, `scope_of`, `Claim` (D79, D80, D83; prefixes from `prompt::excerpt::PathPrefix`, D104) |
| `crates/htui-core/src/model/mod.rs` | edit | T1 | `pub mod overlap;` + re-exports (`:80-134`) |
| `crates/htui-core/src/model/run.rs` | edit | T2 | `GraphSnapshot.scope` (D79, `:448-466`) |
| `crates/htui-core/src/store/traits.rs` | edit | T2 | `claim_run -> Result<Claim>` (D83), `take_lease` (D87), `adopt_runs` own-owner clause (D88), `interrupt_step` (D89) |
| `crates/htui-core/src/store/mem.rs` | edit | T2 | the four behaviours, both halves (`State` `:3209-3333`, `impl WriteStore` methods `:4461-4487`, the impl block from `:4180`); the `GraphSnapshot` literal at `:5794`; its unit tests `:5948` and `a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it` `:6069-6173` (adopt/refresh asserts `:6125-6172`) |
| `crates/htui-core/src/store/conformance.rs` | edit | T2 | 3 new cases; `CASES` 49 → 52 (`:36-86`); `run_snapshot()` literal (`:3760-3780`); `claim_run` call sites (13) |
| `crates/htui-core/src/fixtures.rs` | edit | T2 | the `GraphSnapshot` literal (`:1328`) gains `scope: None` |
| `crates/htui-core/tests/mem_store.rs` | edit | T2 | the `CASES` pin and its message (`:35-43`) |
| `crates/htui-store/src/pg/write.rs` | edit | T2 | `claim_run`, `take_lease`, `adopt_runs`, `interrupt_step` |
| `crates/htui-store/src/writer.rs` | edit | T2 | the two new delegating arms; `claim_run`'s return type (`:673-705`) |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T2 | `EXPECTED_CASES` 49 → 52 (`:19`) |
| `crates/htui-store/tests/pg_criteria.rs` | edit | T2 | `claim_run` call sites (`:624-625`, `:3357`), the `GraphSnapshot` literal (`:691`), two new races (T2's test list) |
| `crates/htui-store/.sqlx/*` | regenerate | T2 | the offline query cache (224 files at `d854ff1`): it changes with the `claim_run`/`adopt_runs` SQL and the new `take_lease`/`interrupt_step` queries; `pg_criteria.rs` is a test file whose own `query!` macros are cached there too |
| `crates/htui-agent/src/conformance.rs` | edit | T2 | `UsageSpy`'s `WriteStore` impl (`:673`, `:927-955`) |
| `crates/htui-agent/tests/recorder.rs` | edit | T2 | `SpyStore`'s `WriteStore` impl (`:353`, `:621-649`) |
| `crates/htui-orch/src/engine.rs` | edit | T2, T6, T7 | T2: `start_run`'s `claim_run` call site only (`:450-457`); T6: lease, heartbeat, `claim`, `take_lease`, release, `ClaimRefused` reason, `resume`'s list; T7: `sweep` |
| `crates/htui-orch/src/graph.rs` | edit | T2, T4 | T2: the `GraphSnapshot` literal (`:338-349`) gains `scope: None`; T4: `resolve` fills it via `overlap::resolve`, `ResolveError::UnknownTouchedRepo`, `resolve_scope` removed or delegated |
| `crates/htui-orch/src/isolate.rs` | edit | T3 | `reset`, `release`, `ResetReport` (D99) |
| `crates/htui-orch/src/isolate/real.rs` | edit | T3 | `GixIsolator::reset` / `release` per mode |
| `crates/htui-orch/src/fake.rs` | edit | T3, T6, T7 | T3: `FakeIsolator::reset`/`release` + scripting; T6: `restarted`, lease plumbing in the harness; T7: `stall_after_done` |
| `crates/htui-orch/src/recover.rs` | create | T5 | `LeaseTimes`, `heartbeat`, `Heartbeat`, `Adjudication`, `classify`, `resettle`, `frontier` (D85, D86, D90, D91, D97) |
| `crates/htui-orch/src/lib.rs` | edit | T5, T4, T6 | `pub mod recover;` + the crate doc's "not created" sentence (`:15-16`) (T5); `pub mod overlap;` (T4); re-exports (T6) |
| `crates/htui-orch/src/overlap.rs` | create | T4 | `resolve` (D81) |
| `crates/htui-orch/tests/fixtures.rs` | edit | T4 | only if its assertions need the scope spelled out; the JSON below is what moves |
| `crates/htui-orch/tests/fixtures/feature.snapshot.json`, `feature-with-verify.snapshot.json` | regenerate | T4 | `resolve` now writes `scope` (D79); `topology` inside them must **not** change |
| `crates/htui-orch/src/command.rs` | edit | T6 | `EngineError::{LeaseLost, LeaseHeld}`, `ClaimRefused { run, claim }` (`:235-242`, test `:901`), `select_enabled`'s R-7 sentence (`:432-433`) |
| `crates/htui-orch/src/status.rs` | edit | T7 | `RunFailure::Interrupted` + its `Display` test |
| `crates/htui-orch/src/conformance.rs` | edit | T6, T7 | new cases; `Orchestrate` additions; `CASES` 36 → 42 → 52 and its in-file pin |
| `crates/htui-orch/tests/fake_conformance.rs` | edit | T6, T7 | the out-of-crate pin (`:14-17`) |
| `crates/htui-orch/tests/gix_isolator.rs` | edit | T8 | real-git criteria 12 (sweep half) and 18, and the frontier reconcile |

**Not touched, on purpose:** every migration and `cache_migrations/` file (D79); every `.snap`
(no render changes; milestone 6 re-records the Runs tab); `crates/htui/**` (milestone 6's
`run_worker.rs`); `crates/htui-core/src/prompt/**`; `isolate/git.rs` (no new verb, D99);
`isolate/copy.rs`; `select.rs`, `fanout.rs`, `gate.rs`, `verify.rs` (read, not changed);
`queue.rs` is not created (MOD-12's); `docs/ANA-2.md`, `HANDOFF.md`, the PRD (the deviations in
OQ-4, OQ-5, OQ-6, OQ-7, OQ-8, OQ-9 are the main thread's to record).

## Tasks

**T1 alone first (D105). Then Wave A′: T3 ∥ T5, each in its own git worktree, merged T3 then T5
with the `htui-orch` gate re-run on the real tree after each merge. Then T2 (needs T1). Then T4
(needs T2 and T5: both edit `lib.rs`). Then T6 (needs T2, T3, T4, T5), then T7, then T8.**
Independence is decided by intersecting the file sets, not by prose — and, since the fact-check,
by build coupling too:

| Task | Files (complete list) | Parallel |
|---|---|---|
| T1 | `crates/htui-core/src/model/overlap.rs`, `crates/htui-core/src/model/mod.rs` | first, alone, until green (D105) |
| T3 | `crates/htui-orch/src/isolate.rs`, `crates/htui-orch/src/isolate/real.rs`, `crates/htui-orch/src/fake.rs` (**not** `lib.rs`, D106) | Wave A′, own worktree, merged first |
| T5 | `crates/htui-orch/src/recover.rs`, `crates/htui-orch/src/lib.rs` | Wave A′, own worktree, merged after T3 |
| T2 | `crates/htui-core/src/model/run.rs`, `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/src/fixtures.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/pg_criteria.rs`, `crates/htui-store/.sqlx/*`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`, `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/graph.rs`, `crates/htui-orch/src/recover.rs` (guard only: D106 says T5 leaves nothing here for T2 to fix) | serial, after T1 and after Wave A′ merges |
| T4 | `crates/htui-orch/src/overlap.rs`, `crates/htui-orch/src/graph.rs`, `crates/htui-orch/src/lib.rs`, `crates/htui-orch/tests/fixtures.rs`, `crates/htui-orch/tests/fixtures/feature.snapshot.json`, `crates/htui-orch/tests/fixtures/feature-with-verify.snapshot.json` | serial, after T2 (and after T5: both edit `lib.rs`) |
| T6 | `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/command.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/lib.rs`, `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs` | serial, after T2–T5 |
| T7 | `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs` | serial, after T6 |
| T8 | `crates/htui-orch/tests/gix_isolator.rs` | serial, after T7 |

The three early sets are pairwise disjoint (T1 is `htui-core/src/model/{overlap,mod}.rs`; T3 is
the isolator trio; T5 is `recover.rs` + `lib.rs`). **File-disjoint is not build-disjoint**
(fact-check): T1 is a dependency of `htui-orch`, and any task's red or mid-edit state stops
`htui-orch` compiling for the others, hence D105's shape. **Compile coupling, checked:** T1 adds a
module and re-exports only, so nothing else moves; T3 adds two trait methods with no default body — every
implementor is in T3's own set (`real.rs:1214`, `fake.rs:280`; `grep -rn 'impl.*Isolator for'
crates/` finds those two), and no caller exists yet; T5 is a new module with no store call at all (the refresh is a closure,
D106) and whose only gate call is the unchanged `gate::settle` (`gate.rs:239`). **Why T2 is one serial task:** adding a field to `GraphSnapshot` breaks its five
struct literals (`graph.rs:338`, `crates/htui-store/tests/pg_criteria.rs:691`,
`crates/htui-core/src/fixtures.rs:1328`, `mem.rs:5794`, `conformance.rs:3761`; the other
`GraphSnapshot` producers — e.g. `status.rs:334`, `gate.rs:1004`, `tests/review_loop.rs:28`,
`command.rs:588`, `gate.rs:1321`, `fanout.rs:380`, `engine.rs:5565` — all decode JSON and are
unaffected), and changing `claim_run`'s return type breaks all five
implementors and every caller (call sites — `grep -rn 'claim_run(' crates/` minus the five impl signatures and `traits.rs:711`'s
declaration: 13 in the store conformance, 10 in `mem.rs` (9 tests + the `State` delegation at
`:4470`), 3 in `pg_criteria.rs`, 2 in `writer.rs`, 1 each in the two spies and `engine.rs:453`) —
so every one of those files is in T2 and T2 must leave the workspace compiling in each commit.
**Hidden coupling, checked:** only T2 changes a query (`.sqlx`); only T4 changes
`tests/fixtures/*.snapshot.json`; no task changes a seed, a `.snap`, `Cargo.toml` or `Cargo.lock`
(`futures`, `tokio` `time` and `test-util` are already there); `CASES` lists move only in T2
(store) and T6/T7 (orch), never concurrently.

Every implementer prompt carries: PRD D1–D8 win over this plan where they disagree; graphify is
stale for everything here (Graphify note), so **read the tree**; no migration; nothing sets
`updated_at` by hand; the only `git` subprocesses in `src/` stay `isolate/git.rs`'s `Cli` (six
verbs) and `verify.rs`; **commit incrementally** — uncommitted subagent work does not survive the
session, and there is no stash on a shared tree; T3 and T5 run in their own worktrees (D105) and
their gates are re-run on the real tree after each merge; verify your gate with
`--test-threads=1`.

### Task 1: `htui-core` — the overlap predicate, pure (D79, D80, D83)
- **Files**: `crates/htui-core/src/model/overlap.rs` (new), `crates/htui-core/src/model/mod.rs`.
- **Test first** (unit tests in `overlap.rs`): `prefixes_come_from_the_excerpt_path_prefix` (D104:
  over `PathPrefix::parse`'s outputs — `src/**/*.rs` → `src/`, `crates/htui-core/src/model/item.rs`
  whole, `**` → `""`, `src/{a,b}` → `src/`, `a?b` → `""` — the rule stated as "no metacharacter:
  whole path; else truncate before the first of `*?[{` and keep up to and including the last `/`,
  none → `""`"); `disjoint_repos_never_overlap`; `rule_l_local_overlaps_even_when_the_other_is_isolated`;
  `rule_i_shared_serialized_overlaps_an_isolated_run_on_the_same_repo`;
  `rule_p_two_isolated_runs_overlap_on_intersecting_prefixes`;
  `two_isolated_runs_with_disjoint_prefixes_do_not_overlap` (criterion 15's parallel half);
  `an_empty_prefix_list_overlaps_everything_in_its_repo` (criterion 15's third clause, `:2126-2127`);
  `a_double_star_is_the_same_as_no_declaration`; `rules_are_reported_in_l_i_p_order`;
  `scope_of_a_snapshot_without_scope_is_conservative` (every repo `isolated = false`, `prefixes =
  []`, so it overlaps any shared repo); `scope_of_an_undecodable_scope_is_conservative`;
  `claim_display_names_the_rule_and_the_holding_run`.
- **Action**: `RunScope { repos: BTreeMap<RepoId, RepoScope> }`, `RepoScope { isolated, local,
  prefixes: Vec<String> }` (both `Serialize`/`Deserialize`/`#[serde(default)]` per field),
  no prefix parser of its own (D104), `OverlapRule::{Local, NotIsolated, Paths}` with `Display`
  (`local`, `not_isolated`, `paths`), `pub fn overlaps(a, b) -> Option<OverlapRule>` over the
  intersection of `a.repos` and `b.repos` in `RepoId` order, `pub fn scope_of(snapshot: &Value,
  repo_scope: &[RepoId]) -> RunScope`, `Claim::{Admitted, NotClaimable, SlotFull { running: u64,
  limit: u32 }, Overlaps { with: RunId, rule: OverlapRule }}` with `Display`. Doc cites
  `docs/ANA-2.md:1059-1089`. `mod.rs`: `pub mod overlap;` and `pub use overlap::{Claim,
  OverlapRule, RepoScope, RunScope};`.
- **Mirror**: `crates/htui-core/src/model/quota.rs:349-463`.
- **Validate**: `cargo test -p htui-core --all-features --lib model::overlap`; `cargo clippy -p
  htui-core --all-targets --all-features -- -D warnings`.

### Task 2: the seam — admission, the lease writers, `interrupt_step`, both stores (D79–D83, D87–D89)
- **Files**: the T2 row above (14 paths plus `.sqlx`).
- **Test first**: three new store conformance cases (`CASES` 49 → 52), each failing for its stated
  reason on both stores:
  `claim_run_applies_the_isolation_and_path_rules` — a two-repo project (two `create_repo`s,
  `traits.rs` `create_repo`), runs whose snapshots carry `scope: Some(..)`: two worktree-isolated
  runs on `core` with prefixes `src/` and `docs/` are **both** admitted; a third with `src/lib/` is
  `Overlaps { rule: Paths }` naming the first; a `shared_serialized` run on `core` is
  `Overlaps { rule: NotIsolated }`; a `local` run on `core` is `Overlaps { rule: Local }`; a run on
  the second repo only is admitted up to the slot, then `SlotFull { running: 2, limit: 2 }`;
  an `awaiting_approval` run still overlaps and holds no slot (criterion 16's two halves, `:2128-2130`).
  `take_lease_moves_only_our_own_or_an_expired_lease` — a stranger's live lease refuses, an
  expired one is taken, our own is renewed, a parked run's lease is takeable, a `queued` or
  terminal run refuses, another box refuses, unknown run `NotFound`.
  `interrupt_step_is_a_cas_on_running` — `running → failed` with `gate_note`, `gate_outcome`
  NULL, `finished_at` set; a `pending`/`awaiting_approval`/`done` step answers `Ok(false)` and is
  unchanged; unknown step `NotFound`.
  Extended in place (no count change): `claim_run_admits_one_and_refuses_the_second` asserts the
  `Claim` values instead of `bool`s; `lease_refresh_is_a_cas_on_owner` gains the D88 leg (the
  owner's own expired lease is **not** adopted by its own sweep; the case is `:4231-4339`, its
  `adopt_runs` section `:4285-4338`, and the leg goes after the "expired lease adopted" leg at
  `:4307-4316`).
  `pg_criteria.rs`: `two_sweeps_adopt_each_expired_run_once` (two pools, two owners, one
  `adopt_runs` each, the union is the set and the intersection empty) and
  `a_claim_and_a_take_do_not_both_win_a_parked_run` — mirroring
  `admission_is_serialised_by_the_box_row_lock` (`:585-664`).
- **Action**: `GraphSnapshot.scope` (`model/run.rs:448-461`, doc: D79, not in `topology`, `V`
  unchanged) and `scope: None` at the five literals; `traits.rs`: the four contracts with their
  docs (`claim_run`'s at `:691-718` rewritten around rules L/I/P and `Claim`); `MemStore`: the
  predicate via `htui_core::model::overlap::{scope_of, overlaps}`, `take_lease`, `interrupt_step`,
  `adopt_runs`'s owner clause; `PgStore`: `claim_run` selects `graph_snapshot` for the claimant
  and `id, graph_snapshot` for the prefiltered set (still under the box `FOR UPDATE`), decides in
  Rust in `(queued_at, id)` order, `take_lease` and `interrupt_step` as single `UPDATE`s with
  `refresh_lease`'s `NotFound` follow-up, `adopt_runs` `AND lease_owner IS DISTINCT FROM $2`;
  `Writer` arms; both spies forward; `engine.rs:450-457` becomes `match … { Claim::Admitted =>
  {}, other => return Err(EngineError::ClaimRefused { run: id }) }` (the reason is T6's). `.sqlx`
  regenerated from `crates/htui-store` against a migrated scratch database.
- **Mirror**: `pg/write.rs:2464-2692`; `mem.rs:3209-3333`.
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; `USERNAME=htui-ci
  HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features -- --test-threads=1`; `cargo
  test -p htui-agent --all-features -- --test-threads=1`; `cargo test -p htui-orch --all-features
  -- --test-threads=1` (all 36 orch `CASES` unchanged); `cargo sqlx prepare --check` (Validation);
  clippy on the workspace.

### Task 3: `htui-orch` — `Isolator::reset` and `release` (D92, D93, D99)
- **Files**: `crates/htui-orch/src/isolate.rs`, `crates/htui-orch/src/isolate/real.rs`,
  `crates/htui-orch/src/fake.rs`.
- **Test first** (`real.rs`, git-backed ones behind `skip_without_git!()`):
  `reset_leaves_worktree_and_copy_trees_untouched` (OQ-8: `HEAD`, branch and files unchanged, the
  report empty); `reset_labels_then_resets_a_clean_shared_checkout` (an agent commit on the
  checked-out branch → `htui/<step>` names it, `HEAD == before_hash`, the commit still reachable);
  `reset_of_a_shared_checkout_at_before_hash_writes_no_label`;
  `reset_refuses_a_live_dirty_local_checkout_and_touches_nothing` (OQ-7; the edit survives
  byte-for-byte, `HEAD` unmoved, no label); `reset_is_all_or_nothing_across_repos` (repo A clean,
  repo B dirty → neither reset); `reset_of_a_vanished_tree_is_not_an_error`;
  `release_frees_a_shared_guard_without_removing_a_tree` (a second `prepare` on the same `(box,
  repo)` proceeds; the tree directory still exists). `fake.rs`:
  `the_fake_reset_is_scripted_and_empty_by_default`, `the_fake_release_frees_its_serial_lock`.
- **Action**: `isolate.rs` — `ResetReport`, the two verbs with docs citing D92/D99 and ANA-2
  `:1298-1299`; `real.rs` — per mode: `worktree`/`copy` return an empty report; `shared_serialized`/
  `local` first read `is_dirty` for every row (`git.rs:1263`) and refuse with
  `dirty_tree_not_reset` (`real.rs:114`) if any is dirty, then per row `create_branch(htui/<step>,
  HEAD)` when `HEAD != tree.base_ref` and `with_retry("reset --hard", reset_hard(tree.base_ref))` (the pattern at `real.rs:1114`); `release` calls
  `release_run` (`:513`); `fake.rs` — `FakeIsolator::script_reset_refusal`, `resets()` and
  `releases()` counters; `release` drops the fake's one whole-fake `serial` guard (`fake.rs:86-91`,
  held per step in `held`) for the run's steps — the fake has one lock, not one per repo.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy;
  `grep -rn 'Command::new' crates/htui-orch/src/` still hits `isolate/git.rs` and `verify.rs`
  only.

### Task 4: `htui-orch` — resolving the scope at `StartRun` (D79, D81)
- **Files**: `crates/htui-orch/src/overlap.rs` (new), `crates/htui-orch/src/graph.rs`,
  `crates/htui-orch/src/lib.rs`, `crates/htui-orch/tests/fixtures.rs`,
  `crates/htui-orch/tests/fixtures/feature.snapshot.json`,
  `crates/htui-orch/tests/fixtures/feature-with-verify.snapshot.json`.
- **Test first** (`overlap.rs` unit tests over plain `Item`/`Repo` values):
  `a_bare_glob_is_the_primary_repo`; `a_qualified_glob_names_its_repo`;
  `an_unknown_repo_name_is_refused`; `no_declaration_is_the_whole_primary_repo`;
  `a_requested_scope_is_honoured_and_its_paths_are_filtered`;
  `every_isolated_phase_makes_the_repo_isolated`; `one_local_phase_makes_the_repo_local`;
  `one_shared_phase_makes_it_not_isolated`. `graph.rs`:
  `resolve_writes_the_scope_into_the_snapshot`, `feature_snapshot_topology_is_pinned`
  **unchanged**, and the D14 refusal test `empty_scope_with_primary_is_refused` (`graph.rs:1286-1353`) unchanged in meaning.
- **Action**: `overlap::resolve` per D81, calling `htui_core::prompt::excerpt::PathPrefix::parse` (D104); `graph::resolve` calls it where it calls
  `resolve_scope` (`graph.rs:351-352`) and sets `snapshot.scope`; `ResolveError::UnknownTouchedRepo`;
  `resolve_scope` is removed from `lib.rs:45`'s re-export or kept as a thin wrapper (the
  implementer chooses; no caller outside `graph.rs` exists — `grep -rn resolve_scope crates/`);
  `lib.rs`: `pub mod overlap;`; the two JSON fixtures regenerated, and `tests/fixtures.rs`
  asserts their `topology` equals `FEATURE_TOPOLOGY`'s pin.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy.

### Task 5: `htui-orch` — `recover.rs`, pure (D85, D86, D90, D91, D97)
- **Files**: `crates/htui-orch/src/recover.rs` (new), `crates/htui-orch/src/lib.rs`.
- **Test first**: `lease_times_read_the_two_app_settings`; `lease_times_fall_back_to_120_and_60`;
  `a_refresh_at_or_above_the_ttl_reads_as_half`; `heartbeat_refreshes_every_interval` and
  `heartbeat_returns_abandoned_on_a_zero_row_refresh` and `heartbeat_survives_a_store_error`
  (`#[tokio::test(start_paused = true)]` over a scripted refresh closure — no store, no
  `claim_run`, D106); `classify_*` over hand-built rows — finished by `finished_at`, finished by every
  `after_hash`, not finished without the document, not finished with one repo missing,
  resettable/non-resettable per mode × `dirty`, a candidate, a judge, a vacuous no-tree step;
  `resettle_maps_command_runs_to_verify_outcomes`; `resettle_reads_a_review_verdict`;
  `frontier_is_the_highest_done_winner_without_a_successor`, `frontier_is_none_mid_position` and
  `frontier_ignores_retired_rows_after_a_review_rejection` (C129: implement attempt 2 `done` at *p*,
  the review of attempt 1 `cancelled` at *p* + 1 → the frontier is implement attempt 2). Snapshots
  are decoded from JSON, never built as literals (D106).
- **Action**: `LeaseTimes { ttl: TimeDelta, refresh: Duration }::from_app`; `pub async fn
  heartbeat<F, Fut, C: Clock + ?Sized>(refresh: F, clock, times) -> Heartbeat` (D106)
  (`Heartbeat::Abandoned` only; it never returns otherwise); `Adjudication::{Finished { verify:
  Option<VerifyOutcome> }, Reset, NeverReset { trees: Vec<(RepoId, String, String)> }}` and
  `StepKind::{Plain, Candidate, Judge}`; `classify(step, phase, run_scope: &[RepoId], trees,
  commits, output_present, command_runs) -> (StepKind, Adjudication)`; `resettle(output, verify, is_review)
  -> Settle` via `gate::settle` (`gate.rs:239`); `frontier(snapshot, steps) -> Option<StepId>`.
  `lib.rs`: `pub mod recover;` and the doc sentence at `:15-16` ("`overlap.rs`, `recover.rs` and
  `queue.rs` … are deliberately not created") rewritten for milestone 5.
- **Validate**: `cargo test -p htui-orch --all-features --lib -- recover::`; clippy.

### Task 6: the engine holds its lease; overlap reaches the walk (D83–D88; criteria 15, 16)
- **Files**: `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/command.rs`,
  `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/lib.rs`,
  `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs`.
- **Test first**, six `CASES` (36 → 42), each failing for its stated reason:
  `overlapping_touched_paths_serialise` (**criterion 15a**: two items declaring `src/**` on one
  repo; the second `StartRun` is `ClaimRefused` with `Overlaps { rule: Paths }` naming the first
  and stays `queued`; after the first finishes, `Engine::claim` walks it);
  `the_same_paths_in_two_repos_run_concurrently` (**15b**: a two-repo project, each item qualified
  to its own repo, both claimed); `an_undeclared_item_holds_its_whole_primary_repo` (**15c**);
  `a_third_run_waits_for_a_slot_and_a_parked_run_still_blocks_overlap` (**criterion 16**);
  `a_parked_run_releases_its_lease_and_an_answer_takes_it` (D87: after a gate park
  `lease_expires_at == now`; `AnswerGate` from a `restarted()` orchestrator succeeds and the run's
  lease is the new owner's); `a_live_lease_blocks_an_answer_from_another_process` (D87's
  `LeaseHeld`). Unit tests in `engine.rs`: `a_walk_whose_lease_is_taken_is_abandoned_and_writes_nothing`
  (a walk future that never completes, a stranger's `adopt_runs`, `start_paused`, the walk
  dropped, `isolator.releases() == 1`, no row changed after the steal);
  `lease_times_come_from_app_settings`; `claim_refused_names_the_rule`.
- **Action**: `EngineParts` unchanged in shape (the timings come from `app`); `walk_leased`
  around the eight `run_to_rest` sites (D86); `take_lease` before each `unpark` (D87); release on a
  non-`running` rest (D87); `pub async fn claim(run)` (D84); `start_run` returns
  `ClaimRefused { run, claim }`; `resume`'s not-comparable list gains
  `ResolveError::UnknownTouchedRepo` (`engine.rs:957-961`, the arm through `:970`); `LEASE_SECONDS` removed (D85);
  `drive_group`'s cancel-safety doc (`:1588-1595`) updated. `command.rs`:
  `EngineError::{LeaseLost { run }, LeaseHeld { run }}`, `ClaimRefused { run, claim: Claim }`
  with its test (`:901`), `select_enabled`'s R-7 sentence (`:432-433`) now naming milestone 6.
  `fake.rs`: `FakeOrchestrator::restarted`, `claim`, and the owner/clock plumbing; `lib.rs`:
  re-exports (`recover::{LeaseTimes, Heartbeat}`, `overlap::resolve`, and `isolate::ResetReport`,
  which T3 deliberately left out, D106); the heartbeat is handed `|until| store.refresh_lease(run,
  owner, until)` (D106). Both count pins move in
  the same commit: `tests/fake_conformance.rs:14-17` (`cases_len_is_thirty_six` → renamed) and
  `conformance.rs:3153-3172`, whose **test name** and enumerating message are rewritten, not just
  the number, and the `CASES` doc (`:164-185`). **Recount, do not append**: both existing
  enumerations miscount today (the doc's categories sum to 37, and thirteen fan-out cases are
  called "twelve").
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy.

### Task 7: the recovery sweep (D89–D98, D100; criteria 12 and 18 over the fakes)
- **Files**: `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/status.rs`,
  `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/conformance.rs`,
  `crates/htui-orch/tests/fake_conformance.rs`.
- **Test first**: `status.rs` — `run_failure_display_is_ana2s_bytes` extended with
  `interrupted: implement` and `interrupted, tree not reset: implement`. Ten `CASES` (42 → 52):
  `a_finished_step_is_adopted_through_its_gate` (**criterion 18a**: stall after the document,
  `record_commits` an `after_hash`, `restarted().sweep()` → an ungated phase is `done` and the run
  walks on to `done`; a gated one parks at `awaiting_approval`, not `done`);
  `an_unfinished_step_is_reset_and_retried` (**18b**: stall before the document; the sweep fails
  it `interrupted` with `gate_note`, calls `reset`, admits attempt 2 whose `before_hash` equals
  attempt 1's, and `resume` walks it to `done`);
  `an_interrupted_step_out_of_budget_parks` (`retry_limit = 0` via `conformance.rs`'s private `repoint` helper, `:455`, so the case lives in `conformance.rs`; run and item
  `awaiting_approval`, `RetryStep` then resumes it);
  `a_dirty_tree_is_never_reset` (**criterion 12**, fake half: a `local` phase whose tree row says
  `dirty = true`; `interrupted, tree not reset`, `resets() == 0`, the note names path and
  `before_hash`); `a_crash_before_reconcile_is_reconciled_on_adoption` (D97: stall is not needed —
  a `FakeIsolator` scripted to fail `reconcile` once, then `restarted().sweep()` reconciles the
  frontier and the walk finishes; the `reconciles()` call log (`fake.rs:208`) shows the second call; scripting via `fail_reconcile` (`:153`));
  `a_crash_between_done_and_reconcile_after_a_review_rejection_is_reconciled` (C129, D97: the
  review rejects attempt 1, implement attempt 2 reaches `done`, the process stalls before its
  reconcile — a `FakeIsolator` scripted to fail that `reconcile` stands in; after
  `restarted().sweep()` the frontier is implement attempt 2 despite the cancelled review row at
  *p* + 1, it is reconciled once, and the new review attempt runs over the merged primary);
  `an_interrupted_candidate_fails_alone` (a 3-way group, candidate 1 stalled; after the sweep 0
  and 2 are `done`, 1 is `failed` with `interrupted`, and selection runs over 0 and 2);
  `an_interrupted_judge_parks_for_selection`; `a_half_written_park_is_completed` (D96: a step
  moved `running → awaiting_approval` by hand under a `running` run; the sweep moves run and item);
  `the_sweep_never_touches_a_parked_run_or_a_live_lease` (`:1306`, D88: a parked run and a run
  with an unexpired lease are both absent from `sweep()`'s answer and byte-unchanged).
- **Action**: `pub async fn Engine::sweep` and a private `recover_run` per D98, using T5's
  `classify`/`resettle`/`frontier`, T2's `interrupt_step`, T3's `reset`, and the existing `admit`,
  `gate::apply`, `reconcile_done_step`, `park_run`-shaped park (D94); `RunFailure::Interrupted`;
  `fake.rs`: `stall_after_done`; `conformance.rs`: the `Orchestrate` trait gains `sweep`, `claim`,
  `restarted`, `stall_after_done`; both count pins move again (52).
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy; then the
  workspace gate.

### Task 8: real git — criteria 12 and 18 over trees, and the lost merge (D92, D93, D97)
- **Files**: `crates/htui-orch/tests/gix_isolator.rs`.
- **Test first** (each opens with `let Some(git) = skip_without_git!() else { return; };`, except
  the `local` one, which needs no `git` exactly as `criterion_12_a_local_step_on_a_dirty_tree` does,
  `:515-522`): `criterion_12_the_sweep_parks_a_dirty_local_step_without_resetting` (a stalled
  `local` step on a dirty checkout; after the sweep the edit is byte-identical, `HEAD` unmoved, no
  `htui/` branch, the step `failed` with `interrupted, tree not reset`);
  `criterion_18_an_unfinished_worktree_step_is_retried_from_the_same_base` (`worktree`; a
  `CommittingSink` that commits and then stalls; after `sweep` + `resume` the retry's tree
  branches from the interrupted step's `before_hash`, the interrupted `htui/<step>` still names its
  commit, `git worktree list --porcelain` shows both trees until the run ends);
  `criterion_18_a_finished_worktree_step_is_adopted_and_merged` (capture landed, settle lost; the
  sweep re-settles and D97 merges it once — one two-parent merge in the primary, not two);
  `a_clean_shared_checkout_is_labelled_then_reset_and_retried`;
  `a_lost_merge_is_recognised_not_repeated` (the merge happened, `record_commits` did not: the
  sweep's reconcile answers the existing merge commit via the parents check, `real.rs:1036-1041`).
- **Action**: the cases, with a stalling variant of the file's `CommittingSink` (`:369-414`) and a
  second `EngineParts` over the same `MemStore` with a new owner and a new `GixIsolator`.
- **Validate**: `cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1`,
  then the workspace gate.

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named in the task.

**Store conformance (`htui-core/src/store/conformance.rs`), run on both `MemStore`
(`crates/htui-core/tests/mem_store.rs`) and `PgStore` (`crates/htui-store/tests/pg_conformance.rs`):**
three new cases — `claim_run_applies_the_isolation_and_path_rules`,
`take_lease_moves_only_our_own_or_an_expired_lease`, `interrupt_step_is_a_cas_on_running` — and two
extended in place — `claim_run_admits_one_and_refuses_the_second` (asserts `Claim`),
`lease_refresh_is_a_cas_on_owner` (D88's leg). `READ_CASES` (9) is unchanged.

**`htui-orch` conformance (`FakeDriver` + `FakeIsolator` + `MemStore`):** sixteen new cases, T6's
six and T7's ten, by criterion: 15 (three), 16 (one), 12 (one, fake half), 18 (two, plus four
recovery shapes: out of budget, candidate, judge, half-written park), and D87's two, D88's one and D97's two (3+1+1+2+4+5 = 16).

**Postgres-only races (`pg_criteria.rs`):** `two_sweeps_adopt_each_expired_run_once`,
`a_claim_and_a_take_do_not_both_win_a_parked_run`.

**Real git (`tests/gix_isolator.rs`):** criteria 12 (sweep half) and 18 (both halves), the shared
reset, the lost merge.

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| `htui-core` store `CASES` | 49 | 52 | `crates/htui-core/src/store/conformance.rs:36-86` (counted: 49 entries); pinned with its enumerating message at `crates/htui-core/tests/mem_store.rs:35-43` |
| `htui-store` `EXPECTED_CASES` | 49 | 52 | `crates/htui-store/tests/pg_conformance.rs:19` |
| `htui-orch` `CASES` | 36 | 42 (T6) → 52 (T7) | `crates/htui-orch/src/conformance.rs:186-268` (counted: 36 entries); `cases_are_unique_and_thirty_six` at `:3153-3172` (name and message); `crates/htui-orch/tests/fake_conformance.rs:14-17` (`cases_len_is_thirty_six`) |
| `WriteStore` method count | 63 | 65 | `crates/htui-core/src/store/traits.rs:195-991` (counted: 63 `async fn` in the trait body); not pinned by a test |
| `ReadStore` method count | 16 | 16 | `traits.rs:66-183` (counted: 16) |
| Migration pins | `vec![1, 2, 3, 4]`, `Pending(4)` | unchanged | `crates/htui-store/tests/migrations.rs:74`, `:591`, `:623`; `crates/htui-store/tests/connect.rs:95` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-10** — A `SIGKILL`ed orchestrator's agent survives: the child is a process-group leader (`launch.rs:1128`) and `ChildGuard::drop` cannot run in a killed process, so criterion 18's "no orphaned agent process" holds only for an in-process abandon (D101) | Certain on a hard kill | Recorded; the in-process half is proven (T6's abandon test). A pid-in-`session_started` + signal-on-adoption design, or a death signal, is milestone 6's / MOD-16's to decide, with the reused-pid hazard stated |
| **R-11** — A crash between `pending → running` and the session start is re-run as "interrupted" even though no token was spent, and burns one retry of the budget | Low | Recorded; the vacuous no-tree case (D92) is the correct *state*, and a budget refund would need a "never started" marker the schema does not have (`session_started` is an event, not a column) |
| **R-12** — D88 means a run whose walking task died inside a *live* process is not re-adopted until that process restarts | Low | The same process can `resume` it directly (its own lease refreshes); milestone 6's `run_worker` supervises its tasks and is where that belongs |
| **R-13** — D91 drops the deadline and cap rules when re-settling, so a step that finished late or over its cap before the crash is judged only on its document and verify | Low | Recorded; the partial `usage` is durable (`:1272-1274`) and a later refinement can read it, but the breach decision itself was never persisted |
| **R-14** — The scope is resolved at `StartRun` from `touched_paths` as they are then; a later edit (MOD-13) changes nothing for that run, so a run can be admitted on a declaration its item no longer makes | Medium | That is invariant 2's reading (D79) and the same as every other snapshot field; the Runs tab can show the snapshot's scope (milestone 6) |
| **R-15** — §4.7's prefix rule over-serialises (`src/**` vs `src/**`, ANA-2 risk 8 `:2071`) and the seeded default isolation is `worktree` for every phase, so ordinary items with no declaration still serialise on the primary repo | Certain | By design (`:1029`, declaration buys parallelism); criterion 15c pins it |
| **R-16** — D97's frontier reconcile parks a healthy run whose primary moved (a human commit) between a completed reconcile and the crash | Low | The park names `primary_moved` and the `before_hash` (R-7's shape); nothing is lost; the alternative (skip the re-reconcile) is the unmerged-primary failure D97 exists to prevent |
| **R-17** — `PgStore::claim_run` now decodes one snapshot per prefiltered run under the box lock | Low | The prefilter (`repo_scope &&`, GIN-indexed, `0003_orchestration.sql:52`) bounds it to live runs sharing a repo on one box — at `max_concurrent_items = 2` running plus the parked ones; only the `scope` key is decoded |
| **R-18** — Deviations the main thread must record in ANA-2 / the PRD, or a later milestone re-reads the text literally: OQ-4 (rule L queued, `:1085-1087`), OQ-5 (`:1297`), OQ-6 (`:1298-1299`), OQ-7 (`:1298`), OQ-8 (`:2136-2137`), OQ-9 (`:1297`), D95 (`:1318`), PRD scope `:229-230` vs M3 D43 | Medium | Listed here and in the carried table; criteria cases fail loudly if the literal reading is reintroduced |
| **R-19** — The heartbeat relies on the walk yielding; a long synchronous stretch in the engine task starves it past the TTL | Low | Every `gix` call runs on the blocking pool and every `git` verb child is a `tokio` process (M3 D22); the one synchronous child is the bounded `git --version` probe at `GixIsolator` construction (`git.rs:215`), outside the walk; D88 stops the process adopting itself, and a stranger's adoption makes our next beat abandon |
| **R-20** — Two live runs of one item are not refused by `claim_run` (OQ-11, D82): a `queued → open` / `in_progress → open` walk-back through `transition` (`traits.rs:205-211`) followed by a second `create_run` lets both be claimed, on one box or two | Low (the engine never does it) | Recorded; OQ-11 offers `Claim::SameItem`; `finish_run`'s "no other live run" mirror rule (`traits.rs:932`, `:942`) keeps the item's status coherent meanwhile |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
# from crates/htui-store, against a migrated scratch database (the compose `htui` one is empty):
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features
cargo doc --workspace --no-deps --keep-going     # baseline: exactly the two errors below, no new one
cargo doc -p htui-orch --no-deps --all-features  # this milestone's own doc check: must exit 0
cargo doc -p htui-core --no-deps --all-features  # fails at baseline on MIRRORED_TABLES: no *new* error
```

`--test-threads=1` is not optional (the keyring fake is process-wide). **`cargo sqlx prepare
--check` is part of this gate** — T2 changes `claim_run` and `adopt_runs` and adds `take_lease` and
`interrupt_step`, so `.sqlx` must be regenerated and committed with the queries. The **`cargo doc`
baseline is confirmed at `d854ff1`** by the fact-check: with `--keep-going`, exactly two errors —
`htui-core`'s unresolved `crate::store::MIRRORED_TABLES` (`traits.rs:887`) and `htui-store`'s
`record_command_run` linking the private `step_exists` (`pg/write.rs:3316`); without
`--keep-going` cargo stops after the first. `cargo doc -p htui-orch --no-deps --all-features` exits
0 today; `cargo doc -p htui-core --no-deps --all-features` exits 101 today on the same
`MIRRORED_TABLES` link, so it is held to "no new error", not a clean exit. Git-backed cases skip on a box without `git` ≥ 2.33.0; this box has 2.43.0 (M3
fact ledger), so the gate runs them.

**If OQ-1 is answered "migration" instead**, a `0005` would touch: a new
`crates/htui-store/migrations/0005_*.sql`; the applied-version vectors at
`crates/htui-store/tests/migrations.rs:74` and `:591` (`vec![1, 2, 3, 4]`); `MigrationState::Pending(4)`
at `migrations.rs:623` and `crates/htui-store/tests/connect.rs:95`; the "exactly twenty-five
commented columns" pin (`migrations.rs:381-409`) if the column is commented (`0003` comments 19 of its
columns but leaves `lease_box_id`, `lease_expires_at`, `verify_exit_code` and the `run_step_tree`
columns uncommented, so the count moves only if the new column gets a `COMMENT`); `PgStore::schema_version` (`pg/mod.rs:389-391`), which rebuilds every box's mirror on
first launch (PRD risk `:393`); a `cache_migrations/0004_*.sql` companion **only if** the column is
mirrored — it need not be, because `refresh_run` selects named columns (`RUN_COLUMNS`,
`crates/htui-store/src/cache/refresh.rs:1075-1094`, and the `refresh_run` `SELECT` at `:1103-1112`, SQL `:1106-1109`), exactly as `lease_owner` is
left out; `Run`, every `Run`-returning query and `pg/rows.rs` only if the column is surfaced on
`Run`; and the `.sqlx` files of every query naming it.

## Acceptance

- [ ] ANA-2 §12 criteria 15 (all three clauses) and 16 pass as `htui-orch` conformance cases; the
      store's `claim_run_applies_the_isolation_and_path_rules` passes on `MemStore` and `PgStore`
      identically; two worktree-isolated runs on one repo with disjoint declared paths run
      concurrently (R-2 closed).
- [ ] Criterion 12's sweep half passes over a real `local` checkout and the fake: the dirt
      survives byte-for-byte, no `git` write ran, the step is `failed` with `interrupted, tree not
      reset`, the note names path and `before_hash`.
- [ ] Criterion 18 passes over the fakes and over real worktrees: a step with both artefacts is
      adopted through its gate; a step with neither is failed `interrupted`, retried from the same
      base, and walked to `done`; a lost merge is recognised, not repeated.
- [ ] Every walk refreshes its lease; a zero-row refresh drops the walk and writes nothing after
      it; a parked run's lease is released; an unpark takes it or refuses `LeaseHeld`; no process
      adopts its own lease; two concurrent sweeps adopt each run once (Postgres).
- [ ] The sweep never touches an `awaiting_approval` run, never resets a dirty tree, completes a
      half-written park, and re-reconciles only the frontier.
- [ ] `htui-orch` `CASES` is 52 and both pins agree; `htui-core` `CASES` is 52, pinned at
      `mem_store.rs` and `EXPECTED_CASES`; `READ_CASES` is 9.
- [ ] No migration, no `cache_migrations` file, no `.snap` change; `.sqlx` regenerated and
      `cargo sqlx prepare --check` clean; `grep -rn 'Command::new' crates/htui-orch/src/` hits
      `isolate/git.rs` and `verify.rs` only; `grep -rnE '"gc"|worktree prune' crates/htui-orch/src/isolate/git.rs`
      finds no spawned verb.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D
      warnings` clean; `cargo test --workspace --all-features -- --test-threads=1` green with
      Postgres up; `cargo doc -p htui-orch --no-deps --all-features` exits 0 and `cargo doc
      --workspace --no-deps --keep-going` shows only the two baseline errors.

## Where the PRD, ANA-2 or HANDOFF disagree with the tree

1. **`engine.rs:83`'s `LEASE_SECONDS` doc cites `docs/ANA-2.md:1408`**, which is §4.10's
   timer-approval paragraph; the lease is `:1276-1282` and its keys `:1518-1519` (D85 removes the
   constant).
2. **`command.rs:432-433` says a reconcile-refused group's retry is "milestone 5's sweep"**; ANA-2
   `:1306-1307` says the sweep never touches a parked run, and `HANDOFF.md:291-293` offers "milestone
   5's sweep **or** milestone 6's `Unblock`-shaped verb". This plan re-defers R-7 to milestone 6
   and T6 rewrites the sentence.
3. **PRD scope `:229-230` says `shared_serialized` is "a Postgres advisory lock per `(box, repo)`"**;
   the tree's guard is an in-process `tokio` mutex (M3 D43, `real.rs:197-198`). Cross-process
   exclusion therefore rests on rule I (D80).
4. **PRD constraint `:280` and `:272` ("the migration set is exactly `0003` …"; "any second
   migration … is the wrong decision")** are already false at `d854ff1` (`0004_max_agents_per_run_default.sql`,
   `HANDOFF.md:324-331`); this milestone adds none.
5. **ANA-2 `:999` says "`git worktree prune` is run once after every cleanup"**; the tree never
   runs it (M3 D46, `git.rs:19`), and `:1777`'s amendment records that. The recovery sweep keeps to
   the tree (D102).
6. **ANA-2 `:1298-1299` (item `blocked`), `:1297` (`done` directly) and `:2136-2137` ("worktree
   reset")** are read differently here — OQ-6, OQ-9, OQ-8 — each for a reason in the tree
   (`item.rs:55`; `gate.rs:373-438` and invariant 5; M3 D23's per-step tree).
7. **ANA-2 §8 (`:1740`) says `claim_run` returns `false` on a refusal**; D83 changes the shape
   (OQ-3).
8. **`HANDOFF.md:214-217` describes R-2 as "two worktree-isolated runs on one repo are refused"**;
   in the tree the predicate also ignores `local` vs `shared_serialized` entirely (both are
   collapsed into "same repo"), so a `local` run is refused with no rule-L sentence — the refusal
   is right, its reason is lost (D83).

---

## Verified claims

One independent fact-check pass against the tree at `d854ff1` (full verdicts in
`/tmp/mod4m5-factcheck.json`, which is not part of the repo). **194 claims: 137 confirmed, 55
partial, 2 falsified; every non-confirmed claim was re-opened at its cited line before being
resolved, and none was rejected.** The two falsified claims (C10, C129) changed decisions, not
only prose. The independence check found Wave A file-disjoint but not build-disjoint; that is
resolved by D105 (shape (b)) and D106 (T5's pins, T3 not touching `lib.rs`, `recover.rs` as a
guard entry in T2's list), and the duplicate `PathPrefix` by D104.

| Claim | Verdict | Evidence |
|---|---|---|
| C1: claim_run is Result<bool> at traits.rs:711-718 | confirmed | crates/htui-core/src/store/traits.rs:711 `async fn claim_run(` ... :718 `) -> Result<bool>;` |
| C2: MemStore filters overlap set by executing_box_id = box at mem.rs:3235-3242 | confirmed | mem.rs:3235-3242 `let live ... .filter(\|row\| row.executing_box_id == Some(box_id) && matches!(row.status, Running \| AwaitingApproval)).collect()`. The same set is used for the slot count (:3243-3249) and the overlap check (:3253-3259). |
| C3: MemStore WriteStore claim_run at mem.rs:4461; impl WriteStore region :4461-4486 | partial | mem.rs:4461 `async fn claim_run(` is right. But `impl WriteStore for MemStore` begins at :4180. The claim_run/refresh_lease/adopt_runs span inside it is 4461-4487: adopt_runs closes at :4487, not :4486. **Resolution:** Amendment applied: Say `impl WriteStore` methods claim_run/refresh_lease/adopt_runs at `:4461-4487` (the impl block itself starts at :4180). |
| C4: overlapping_runs is an inherent method at mem.rs:659 | confirmed | mem.rs:659 `pub async fn overlapping_runs(&self, scope: &[RepoId])` inside `impl MemStore {` (opens at :149), not a trait impl. |
| C5: finish_run's item mirror has no queued -> * row for failed (traits.rs:935-941) | confirmed | traits.rs:938 `\| failed \| in_progress -> failed, awaiting_approval -> failed \|`. Only the cancelled row (:939) has queued -> open. The table runs :935-939 and :941 says every other status is left alone. |
| C6: Status::can_move_to has no road from blocked back to in_progress (item.rs:55) | confirmed | crates/htui-core/src/model/item.rs:55 `Self::Blocked => matches!(to, Self::Open \| Self::Closed)` |
| C7: M4 D53 precedent of a defaulted GraphSnapshot field without V bump at run.rs:463-466 | partial | run.rs:463-466 is only `impl GraphSnapshot { pub const V: u32 = 1; }`. The D53 precedent is `SnapshotJudge.template`, a nested field, at run.rs:553-557 (doc :553-555 says 'GraphSnapshot::V is not bumped for it'). GraphSnapshot's own top-level fields (:448-461) are all required today, so none of them has a serde default. **Resolution:** Amendment applied: Cite `crates/htui-core/src/model/run.rs:553-557` (SnapshotJudge.template, D53: a defaulted field nested inside the snapshot, V not bumped). Scope would be the first defaulted *top-level* GraphSnapshot field. |
| C8: State::claim_run at mem.rs:3209-3280 counts running vs max_concurrent_items, refuses shared repo with live run on the box, decided before first write | confirmed | mem.rs:3208 doc 'decided before the first write so a refusal writes nothing'. fn at :3209 closes at :3280. Checks in order: NotFound (:3218-3224), not queued or wrong target (:3225-3227), running count >= max_concurrent_items (:3243-3250), any repo intersection with live running\|awaiting_approval runs on the box (:3254-3260), and only then the writes (:3262+). The 'lock' is MemStore's write RwLock (the box-row FOR UPDATE is Pg's). |
| C9: Lease columns, claim_run's lease, refresh_lease and adopt_runs all exist in the seam at traits.rs:689-742 | partial | traits.rs:689 is create_run. The claim_run doc starts at :691, the lease writes are named at :698-699, refresh_lease is :720-725 and adopt_runs is :727-741. The lease columns themselves are model fields at crates/htui-core/src/model/run.rs:201-206: `lease_box_id` and `lease_expires_at`, while `lease_owner` is deliberately not projected (run.rs:203; MemStore keeps it in a separate `lease_owners` map). **Resolution:** Amendment applied: Cite `traits.rs:691-741` for claim_run/refresh_lease/adopt_runs and `model/run.rs:201-206` for the lease columns, noting that lease_owner is not on `Run`. |
| C10: create_run moves the item open \| failed -> queued (traits.rs:678-689), so one item never has two live runs | falsified | The cited fact holds: traits.rs:679-689, with :685 saying 'only open and failed can'. The conclusion does not. WriteStore::transition (traits.rs:205-211) has no live-run guard, and can_move_to allows queued->open and in_progress->open (item.rs:49-52). The seam's own conformance case finish_run_moves_run_and_item_together (conformance.rs:5958-5985) puts two live runs on one item: create_run, then transition queued->open, then create_run again, then both are claimed. finish_run's 'only when no other run of the item is non-terminal' (traits.rs:932, :942) exists because of this. **Resolution:** **Design changed.** D82 rewritten (no item-level guarantee claimed); OQ-11 added (known gap vs a cross-box `Claim::SameItem` rule), default = recorded known gap; R-20 added. |
| C11: claim_run decision order: NotFound run then box (traits.rs:707-710), then not claimable, slot full, overlap | confirmed | traits.rs:708-710 '# Errors ... NotFound for an unknown run ("run") or box ("box"), the run looked up first'. mem.rs:3218 require_run, :3219-3224 box NotFound, :3225-3227 status/target, :3248-3250 slot, :3254-3260 overlap. |
| C12: MemStore adopt_runs is at mem.rs:3302-3332 | partial | mem.rs:3302 `fn adopt_runs(` is right, but :3332 is `adopted` and the closing brace is :3333. The doc comment is :3301. **Resolution:** Amendment applied: Cite `mem.rs:3302-3333` (and the State range as `:3209-3333`). |
| C13: adopt_runs contract at traits.rs:727-742; signature (box, owner, now, until) at :735-741 | confirmed | traits.rs:727-734 doc (running + executing_box_id = box + lease NULL or expired at now; queued_at,id order; unknown box -> Ok(vec![])). :735-741 `async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, now: DateTime<Utc>, lease_until: DateTime<Utc>) -> Result<Vec<Run>>;`. :742 is blank. |
| C14: refresh_lease is a CAS on the owner (traits.rs:720-725) | confirmed | traits.rs:720-721 'UPDATE run SET lease_expires_at = until WHERE id = run AND lease_owner = owner. Ok(false) = zero rows'. :725 is the signature. mem.rs:3290-3292 compares lease_owners. |
| C15: MemStore is shared as an Arc (mem.rs:55-58) | partial | mem.rs:55-58 `#[derive(Debug, Clone, Default)] pub struct MemStore { state: Arc<RwLock<State>> }`. MemStore is not itself an Arc. It is a Clone handle whose clones share one Arc<RwLock<State>>. **Resolution:** Amendment applied: Say 'shares the MemStore (a Clone handle over `Arc<RwLock<State>>`, so a clone sees the same rows; mem.rs:55-58)'. restarted() can clone the store rather than wrap it in an Arc. |
| C16: SnapshotJudge.template is a defaulted snapshot field without V bump at run.rs:545-556 | partial | The struct SnapshotJudge spans run.rs:545-558 (doc :543-544). The template field is :553-557, with `#[serde(default)]` at :556 and `pub template: Option<SnapshotTemplate>` at :557, so the cited range stops one line short of the field. Doc :555 confirms 'GraphSnapshot::V is not bumped for it'. **Resolution:** Amendment applied: Cite `crates/htui-core/src/model/run.rs:553-557` (the field) or `:545-558` (the struct). |
| C17: quota::Availability / SkipReason at quota.rs:352-378 (T1 mirror range :352-456) | partial | quota.rs:349-352 doc, :353-359 Availability, :361-381 SkipReason, so the SkipReason enum closes at :381, not :378. `available()` spans :383-463 (fn at :427, closes :463), so :456 cuts mid-match. **Resolution:** Amendment applied: Cite `quota.rs:349-381` for Availability/SkipReason and `:349-463` for the full verdict + `available` mirror. |
| C18: model/mod.rs re-exports are at :80-134 | confirmed | model/mod.rs:80-95 holds the `pub mod` declarations and :97-134 the `pub use` re-exports (the last is `pub use user::...` at :134). :136 is #[cfg(test)]. Both edits T1 needs (pub mod overlap; plus re-exports) fall in :80-134. |
| C19: GraphSnapshot definition at model/run.rs:448-466 (T2 says :448-461) | confirmed | run.rs:448 `pub struct GraphSnapshot {` closes at :461; :463-466 `impl GraphSnapshot { const V = 1 }`. Both ranges are correct for what they cover, and the doc comment starts at :441. |
| C20: GraphSnapshot literal at mem.rs:5794; mem.rs unit tests at :5948 and :6125-6170 | partial | mem.rs:5793 `fn test_snapshot() -> GraphSnapshot`, :5794 `GraphSnapshot {` is correct. :5948 `async fn claim_run_refuses_an_overlapping_scope_and_a_full_box()` is correct. :6125-6170 is only the adopt_runs/refresh_lease asserts inside `a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it`, which spans :6069-6173. **Resolution:** Amendment applied: Cite the lease test as `mem.rs:6069-6173` (adopt/refresh asserts at :6125-6172). |
| C21: GraphSnapshot literal at fixtures.rs:1328 | confirmed | crates/htui-core/src/fixtures.rs:1328 `serde_json::to_value(GraphSnapshot {`, which is the only GraphSnapshot literal in that file. |
| C22: claim_run doc in traits.rs at :691-718 | confirmed | traits.rs:691 '/// ANA-2 §4.7's admission, ...'. The doc runs to :710 and the signature is :711-718. |
| C23: traits.rs has a create_repo writer usable to create a two-repo project | confirmed | traits.rs:465-471 `async fn create_repo(&self, new: NewRepo) -> Result<Repo>;`. NewRepo (model/hierarchy.rs:149-161) carries id, project_id, name, remote_url, default_branch and is_primary, and only (project_id, name) must be unique, so a second repo per project is allowed. |
| C24: GraphSnapshot has a constant V and a topology hash | confirmed | run.rs:465 `pub const V: u32 = 1;` and run.rs:453-454 `pub topology: String` ('sha256:… over the canonical phases[]'). |
| C25: No shipped writer sets gate_note on the way from running to failed | confirmed | There are three gate_note writers on the seam. answer_gate (traits.rs:788-801) is a CAS on awaiting_approval, with Rejected -> failed. select_fanout's judge (traits.rs:803-813) goes pending\|running\|awaiting_approval -> done. finish_step's StepOutcome (run.rs:409-421) has no gate_note. transition_step (traits.rs:774-780) writes no note. The pg demo insert (pg/demo.rs:430) is a seed, not a transition. |
| C26: finish_run is the only writer of run.failure (M2 D7) | partial | WriteStore::fail_run (traits.rs:918-925: 'status = failed, failure = failure') also writes run.failure. traits.rs:945-946 says transition_run and fail_run 'remain the run-only writers the chat path uses'. Implementations: mem.rs:4577, pg/write.rs:3537. It has no caller in htui-orch, so on the graph/engine path finish_run is the only writer. **Resolution:** Amendment applied: Say 'on the graph-run path finish_run is the only writer of run.failure (fail_run, traits.rs:918-925, is the chat path's run-only writer and the engine never calls it)'. |
| C27: Adding GraphSnapshot.scope with #[serde(default)] needs no V bump and leaves topology/FEATURE_TOPOLOGY and stored digests unchanged | confirmed | GraphSnapshot (run.rs:447) has no deny_unknown_fields, so an old reader ignores the key and a new reader defaults a missing one. topology hashes `serde_json::to_string(phases)` only (htui-orch/src/graph.rs:223-233), so FEATURE_TOPOLOGY (graph.rs:746) does not move. No stored digest covers the whole snapshot: prompt_digest is over prompt text (prompt/digest.rs). One caveat: tests/fixtures.rs:26-33 compares the whole typed snapshot, so once T4 fills scope the two *.snapshot.json fixtures must change. The plan already lists them under T4. |
| C28: Claim can live in htui_core::model::overlap and traits.rs can name it without a new module / dependency cycle | confirmed | traits.rs:33 already imports `use crate::model::{...}`, and no model/*.rs file imports crate::store (grep is empty), so no layering cycle appears. model/overlap.rs does not exist yet (model/ lists 16 modules plus mod.rs), but T1 creates it anyway (plan line 276). D83's 'without a new module' means no extra module beyond that one, which holds. |
| C29: run row stores only repo_scope (0003_orchestration.sql:40) | confirmed | 0003_orchestration.sql:40 `ALTER TABLE run ADD COLUMN repo_scope UUID[] NOT NULL DEFAULT '{}'`; the only other run columns 0003 adds are lease_box_id/lease_owner/lease_expires_at (:41-43). None of them holds per-repo isolated/local/paths. |
| C30: PgStore::schema_version is the highest embedded migration (pg/mod.rs:389-391), so a 0005 would rebuild every box's mirror | confirmed | pg/mod.rs:389-391 `pub fn schema_version() -> i64 { MIGRATOR.iter().map(\|m\| m.version).max().unwrap_or(0) }`; cache/mod.rs:138 rebuilds when `meta.schema_version != schema_version`. |
| C31: PgStore overlap prefilter at pg/write.rs:2535-2549 filters executing_box_id = box AND status IN ('running','awaiting_approval') AND repo_scope && $2, currently an EXISTS | confirmed | write.rs:2535 builds `scope`; :2541-2545 `SELECT EXISTS (SELECT 1 FROM run WHERE executing_box_id = $1 AND status IN ('running','awaiting_approval') AND repo_scope && $2::uuid[])`. The statement actually ends at :2552 and the refusal is :2553-2555, but the cited range contains the SQL. |
| C32: PgStore::claim_run at pg/write.rs:2464(-2599), one transaction, box row locked, refusal writes nothing | partial | The fn starts at write.rs:2464 but its closing brace is at :2589 (:2591-2600 is refresh_lease's doc comment). One tx (`pool.begin()` :2472, commit :2587); run row FOR UPDATE (:2479) and then box row FOR UPDATE (:2492); every refusal is an early `return Ok(false)` before any UPDATE (:2503-2505, :2531-2533, :2553-2555), so the tx rolls back on drop. **Resolution:** Amendment applied: Cite pg/write.rs:2464-2589. Also note the run row is locked FOR UPDATE (:2479) before the box row (:2492). |
| C33: Box row is locked FOR UPDATE at pg/write.rs:2491-2502 | confirmed | write.rs:2491-2501 `sqlx::query_scalar!("SELECT settings FROM box WHERE id = $1 FOR UPDATE", ...)...NotFound{entity:"box"}`; :2502 is blank. |
| C34: Writer::claim_run delegating arm at writer.rs:673(-705) | confirmed | writer.rs:287 `impl WriteStore for Writer`; :673 `async fn claim_run(` delegates Memory/Online; claim_run ends at :685, and :673-705 covers claim_run, refresh_lease and adopt_runs together. |
| C35: UsageSpy WriteStore claim_run at conformance.rs:927 (impl at :673, :927-955) | confirmed | crates/htui-agent/src/conformance.rs:673 `impl<S: WriteStore> WriteStore for UsageSpy<'_, S>`; :927 `async fn claim_run(` (ends :938); refresh_lease :939-946; adopt_runs :947-955. |
| C36: SpyStore WriteStore claim_run at recorder.rs:621 (impl at :353, :621-649) | confirmed | crates/htui-agent/tests/recorder.rs:353 `impl WriteStore for SpyStore`; :621 `async fn claim_run(` (ends :632); refresh_lease :633; adopt_runs through ~:649, followed by create_step. |
| C37: overlapping_runs is inherent on PgStore at pg/read.rs:1729 | confirmed | read.rs:843 `impl PgStore {` (inherent; the trait impl is at :60); :1729 `pub async fn overlapping_runs(&self, scope: &[RepoId]) -> Result<Vec<Run>>`. |
| C38: PgStore::refresh_lease at pg/write.rs:2601-2633 is a CAS with Ok(false)/NotFound told apart by one follow-up read | partial | The behaviour is right: UPDATE ... WHERE id=$1 AND lease_owner=$2 (:2602-2603), then on zero rows one `SELECT 1 FROM run WHERE id = $1` (:2617) returns Ok(false) or NotFound. But the fn runs :2601-2631, and :2633-2642 is adopt_runs's doc comment. **Resolution:** Amendment applied: Cite pg/write.rs:2601-2631. |
| C39: PgStore::adopt_runs at pg/write.rs:2643-2699 is UPDATE ... RETURNING ordered through a CTE | partial | write.rs:2653 `WITH swept AS (UPDATE run SET lease_owner=$2, lease_box_id=$1, lease_expires_at=$4 WHERE executing_box_id=$1 AND status='running' AND (lease_expires_at IS NULL OR lease_expires_at <= $3) RETURNING *)` followed by SELECT ... FROM swept ORDER BY queued_at, id. But the fn ends at :2692; :2694-2699 is create_step's doc comment. **Resolution:** Amendment applied: Cite pg/write.rs:2643-2692. |
| C40: lease_ttl_seconds and lease_refresh_seconds seeded 120/60 by 0003_orchestration.sql:135-136 | confirmed | 0003_orchestration.sql:135 `('lease_ttl_seconds', '120'::jsonb)`, :136 `('lease_refresh_seconds', '60'::jsonb)`, inside an INSERT ... ON CONFLICT DO NOTHING. |
| C41: EXPECTED_CASES pin at pg_conformance.rs:19 is 49 | confirmed | pg_conformance.rs:19 `const EXPECTED_CASES: usize = 49;`. Counting the string entries of `pub const CASES` in crates/htui-core/src/store/conformance.rs gives 49. (The module doc at :7 still says 'twenty CREATE DATABASE pairs', which is stale but outside the claim.) |
| C42: admission_is_serialised_by_the_box_row_lock at pg_criteria.rs:585-667, a two-pool Postgres race | partial | pg_criteria.rs:585 `#[tokio::test(flavor = "multi_thread")]`, :586 fn; the closing brace is at :664, not :667. It is two pools: the doc at :583-584 says so, and the body opens a second `PgStore::connect` ("second pool") and races claim_run at :624-625. **Resolution:** Amendment applied: Cite pg_criteria.rs:585-664. |
| C43: pg_criteria.rs claim_run call sites at :624-625 and :3357; GraphSnapshot literal at :691 | confirmed | pg_criteria.rs:624 `.claim_run(first, ids::BOX, ...)`, :625 `other.claim_run(second, ...)`, :3357 `.claim_run(run, ids::BOX, owner, at, until)`, :691 `graph_snapshot: GraphSnapshot {`. |
| C44: repo_scope is GIN-indexed at 0003_orchestration.sql:52 | confirmed | 0003_orchestration.sql:52 `CREATE INDEX idx_run_repo_scope ON run USING GIN (repo_scope);` |
| C45: vec![1, 2, 3, 4] at migrations.rs:74 and :591; Pending(4) at migrations.rs:623 and connect.rs:95 | confirmed | tests/migrations.rs:74 and :591 `vec![1, 2, 3, 4]`; migrations.rs:623 and tests/connect.rs:95 `MigrationState::Pending(4)`. |
| C46: 'exactly twenty-five commented columns' pin at migrations.rs:381-409, and every 0003 column is commented | partial | The pin is right: migrations.rs:381 comment, :382-400 col_description query, :407-409 assert 'exactly the twenty-five commented columns', and ANA_COLUMN_COMMENTS (:168) has 25 entries (6 from 0002 plus 19 from 0003). But not every 0003 column is commented. 0003 adds run.lease_box_id (:41), run.lease_expires_at (:43), run_step.verify_exit_code (:69) and the run_step_tree columns (:93-98), and none of these has a COMMENT ON COLUMN. The 19 commented columns are is_override, judge_agent_id, judge_model, deadline_seconds, verify_command, input_kinds, repo_scope, lease_owner, graph_snapshot, verify_outcome, exit_code, fanout_index, selected, attempt, isolation_path, promoted_at, touched_paths, box.settings, project.settings. **Resolution:** Amendment applied: Replace 'if the column is commented, as every 0003 column is' with 'if the column is commented (0003 comments 19 of its columns but leaves lease_box_id, lease_expires_at, verify_exit_code and the run_step_tree columns uncommented)'. The count only changes if the new column gets a COMMENT. |
| C47: refresh_run selects named columns RUN_COLUMNS at refresh.rs:1074-1093, :1104-1108, and lease_owner is left out | partial | The content is right: RUN_COLUMNS has 18 names and no lease_owner, and the refresh_run SELECT lists named columns with the comment '`lease_owner` is deliberately absent (plan D7)'. The lines are off by one or two: RUN_COLUMNS is refresh.rs:1075-1094 (:1074 is blank), and the query! is :1103-1112 (comment :1104-1105, SQL :1106-1109). **Resolution:** Amendment applied: Cite RUN_COLUMNS at cache/refresh.rs:1075-1094 and the refresh_run SELECT at :1103-1112 (SQL text :1106-1109). |
| C48: git ls-tree 3e34610 migrations lists 0001 and 0002 only; at d854ff1 migrations are 0001..0004 including 0004_max_agents_per_run_default.sql | confirmed | `git ls-tree --name-only 3e34610 crates/htui-store/migrations/` gives 0001_init.sql and 0002_agent_probe.sql. At d854ff1 it gives 0001_init, 0002_agent_probe, 0003_orchestration and 0004_max_agents_per_run_default.sql. |
| C49: Existing pg_criteria / .sqlx are offline query caches that change when claim_run/adopt_runs SQL changes and new writers are added (sqlx query! macros used in pg/write.rs) | partial | crates/htui-store/.sqlx holds 224 offline query JSONs keyed by SQL hash. pg/write.rs has 110 sqlx::query!/query_as!/query_scalar! uses, including claim_run's (:2474, :2491, :2512, :2523, :2541, :2557, :2578) and adopt_runs's query_as! (:2650). So changing that SQL or adding take_lease/interrupt_step macros changes .sqlx. But pg_criteria.rs is not a cache: it is a test file that itself uses 19 compile-checked query! macros (e.g. :49, :82, :122), which also have .sqlx entries. **Resolution:** Amendment applied: Say '.sqlx is the offline query cache; it changes with the claim_run/adopt_runs SQL and the new take_lease/interrupt_step queries. pg_criteria.rs is a test file whose own query! macros are also cached there.' |
| C50: adopt_runs owner clause and new take_lease/interrupt_step UPDATEs need no migration (lease_owner, lease_box_id, lease_expires_at, gate_note, finished_at, gate_outcome already exist in 0003) | partial | The no-migration conclusion holds, but gate_outcome, gate_note and finished_at are not 0003 columns. They come from 0001_init.sql:483-492 (run_step), and 'failed' is already in run_step.status's CHECK (0001_init.sql:481-482). Only the lease_* columns are 0003's (:41-43). No 0003 CHECK blocks the planned values. **Resolution:** Amendment applied: Say 'lease_owner/lease_box_id/lease_expires_at exist since 0003 (:41-43); run_step.gate_note/gate_outcome/finished_at and status 'failed' exist since 0001_init.sql:481-492'. |
| C51: cache_migrations companion would be 0004_*.sql (cache migrations currently end at 0003) | confirmed | `git ls-tree d854ff1 crates/htui-store/cache_migrations/` gives 0001_mirror.sql, 0002_agent_mirror.sql and 0003_orchestration.sql. There is no 0004, so the next one is 0004_*.sql. |
| C52: Engine reports ClaimRefused { run } with 'the box is full or the scope overlaps' at command.rs:235-242 (test at :901) | confirmed | command.rs:235-237 doc, :238 #[error("claim refused: the box is full or the scope overlaps (ANA-2 §4.7)")], :239-242 ClaimRefused { run: RunId }; test assert at command.rs:901. Raised at engine.rs:456. |
| C53: CancelRun already accepts a queued run (command.rs:556-565) | confirmed | command.rs:556-565 cancel_enabled returns Ok if run.status.is_active(); htui-core/src/model/run.rs:53-55 is_active = Queued \| Running \| AwaitingApproval; engine.rs:804-806 cancel_run calls cancel_enabled. |
| C54: finished_at is written by finish_step only after stage 5's capture + record_commits (engine.rs:1369-1401) | partial | True for walk_live_step: capture :1369, record_commits :1370, finish_step :1385-1401 with finished_at :1398; fan-out candidates likewise (capture :1939, record_commits :1941, finish_step :1958). But settle_judge's finish_step (engine.rs:2400-2411, finished_at :2408) runs with no capture, and finished_at is also stamped outside finish_step: transition_step to a terminal status (traits.rs:751-752, mem.rs:3454-3455) and answer_gate (traits.rs:790). **Resolution:** Applied to OQ-5 and D90: scoped to `running` steps with `fanout_index >= 0`. |
| C55: RetryStep unparks through awaiting_approval -> in_progress via unpark at engine.rs:3324-3336 | confirmed | engine.rs:3324-3336 unpark: transition_run AwaitingApproval->Running (:3327), item transition AwaitingApproval->InProgress (:3332). retry_step calls it at :650, retry_group at :697. |
| C56: Engine claims with a hard-coded 120 s lease and never refreshes it (engine.rs:84, :446-457); const LEASE_SECONDS at engine.rs:83-84 | confirmed | engine.rs:83 doc, :84 const LEASE_SECONDS: i64 = 120; :446 lease = now + LEASE_SECONDS; :450-457 claim_run + ClaimRefused. No refresh_lease/adopt_runs call in crates/htui-orch/src (only a doc mention of adopt_runs at :935). |
| C57: LEASE_SECONDS doc at engine.rs:83 cites docs/ANA-2.md:1408 | confirmed | engine.rs:83: '/// How long a claim's lease runs before ANA-2 §4.9's sweep may adopt the run (`:1408`).' docs/ANA-2.md:1406-1410 is the timer-auto-approval rejection text, not the lease, as the plan says. |
| C58: EngineParts.app exists at engine.rs:278-280 | confirmed | engine.rs:278-279 doc, :280 pub app: BTreeMap<String, Value>. |
| C59: futures is already a dependency of htui-orch (comment 'MOD-4 milestone 4 (plan D58)'), tokio time feature on; tokio test-util in dev-deps | confirmed | crates/htui-orch/Cargo.toml: '# MOD-4 milestone 4 (plan D58)...' then futures = { workspace = true }; tokio features ["sync","rt","process","io-util","time","fs"]; dev-dependencies tokio features include "test-util". Workspace futures = "0.3" (lock 0.3.34), tokio 1.53.1. |
| C60: Eight run_to_rest call sites at engine.rs:459,:529,:575,:661,:703,:761,:969,:998; run_to_rest pub at :847 | confirmed | grep 'run_to_rest(' in engine.rs gives 9 hits = the 8 listed calls + definition at :847 (pub async fn). No callers outside engine.rs anywhere in crates/. |
| C61: ChildGuard::drop kills the agent child's process group via start_kill (launch.rs:977-990); child is a process-group leader (launch.rs:1128) | partial | impl Drop for ChildGuard spans launch.rs:977-993 (start_kill at :989); :969 doc says on unix start_kill is a killpg at the group; :1128 wrapped.wrap(ProcessGroup::leader()). Only the range end is off. **Resolution:** Amendment applied: Cite launch.rs:977-993 (start_kill at :989). |
| C62: FakeDriver never suspends (fake.rs:393-408, M4 D58) | confirmed | crates/htui-agent/src/fake.rs:393 next_event returns Box::pin(async move {...}) that pops a queue / returns Ok/Err synchronously; grep finds no .await, sleep, Notify or yield in fake.rs at all. (The M4 D58 attribution is not checked against the tree.) |
| C63: futures::future::select over two non-'static futures in the same task needs no 'static bound; dropping the select drops the walk future | partial | Probe in /tmp/selprobe (rustc 1.98.1, futures 0.3.34, tokio 1.53.1): no 'static bound is needed, but select requires A: Future + Unpin and B: Future + Unpin (E0277 for a bare async fn future), so both must be pinned (std::pin::pin! or Box::pin). When the heartbeat wins, select returns Either::Right((out, walk)) handing back the still-pending walk; with Box::pin, dropping that value dropped the walk right away; with pin!, dropping the returned Pin<&mut> did NOT drop the walk, which was dropped only at the end of the enclosing scope. **Resolution:** Applied to D86: both futures pinned, the returned walk dropped explicitly before `isolator.release`. |
| C64: futures::FutureExt::now_or_never polls a future once and returns None if pending | confirmed | Probe: async { sleep(1s).await; 1 }.now_or_never() -> None; async { 2 }.now_or_never() -> Some(2) (futures 0.3.34). It consumes the future, so a pending future is dropped on return. |
| C65: Four unpark call sites at engine.rs:517, :650, :697, :754 | confirmed | engine.rs:517 (answer_gate), :650 (retry_step), :697 (retry_group), :754 (select_fanout). The other 'unpark' hits (:601, :5796) are comments; the definition is at :3324. |
| C66: Engine::output_of at engine.rs:3466 | confirmed | engine.rs:3466 async fn output_of( |
| C67: stage 5 decides is_review by phase.name at engine.rs:1381; verify runs before capture (1356-1370) | confirmed | engine.rs:1381 is_review: phase.name == REVIEW_PHASE; verify at :1356-1366, stage-5 capture at :1369, record_commits at :1370. |
| C68: reconcile_done_step at engine.rs:2857(-2898); walk_live_step's gate-match tail at 1403-1426 | partial | reconcile_done_step spans engine.rs:2857-2889 (park_run's doc begins :2891). Gate tail: stage 6 comment :1403, match arms :1409-1425, match closes :1426. That part is correct. **Resolution:** Amendment applied: reconcile_done_step at engine.rs:2857-2889. |
| C69: admit(run, snapshot, phase, attempt) at engine.rs:1012 | confirmed | engine.rs:1012-1018 async fn admit(&self, run: &Run, snapshot: &GraphSnapshot, phase: &SnapshotPhase, attempt: i32) -> Result<Option<Rest>, EngineError>. |
| C70: pending -> running then upsert_step_tree spans engine.rs:1264-1311 | confirmed | engine.rs:1264 'if !self', :1267 transition_step(Pending, Running), :1311 upsert_step_tree(step.id, &trees) in walk_live_step. |
| C71: may_attempt(attempt + 1, retry_limit) at status.rs:27-29 | confirmed | status.rs:27-29 pub const fn may_attempt(next_attempt: i32, retry_limit: i32) -> bool { next_attempt <= retry_limit + 1 }. The attempt+1 call is at command.rs:411. |
| C72: park_run at engine.rs:2900-2946 orders run -> item -> note with the step already settled | partial | park_run spans engine.rs:2900-2933: transition_run Running->AwaitingApproval (:2902-2905), item InProgress->AwaitingApproval (:2909), add_note (:2911-2926); the doc :2891-2899 says the step is already done. :2935-2947 is cleanup_run's doc. **Resolution:** Amendment applied: park_run at engine.rs:2900-2933 (doc :2891-2899). |
| C73: Rest.failure is the typed stop reason of the transition that caused it (engine.rs:3338-3347) | confirmed | engine.rs:3338-3347 (resting's doc): '[`Rest::failure`] is the typed [`RunFailure`] of the transition that *caused* the stop'. resting is always None, and callers that did not see the stop read run.failure. |
| C74: cursor: done completes a position (status.rs:223-260); Cursor::Finished leads to finish_run(Done) at engine.rs:871-883 | partial | cursor spans status.rs:223-256 (Done => {} continues at :239, Finished at :255); :258-260 is group_cursor's doc. engine.rs:871-883 Cursor::Finished => finish_run(run, Done, None, now) + cleanup_run + Rest{Done}. That part is correct. **Resolution:** Amendment applied: cursor at status.rs:223-256. |
| C75: Stage 6: reconcile_done_step merges after gate moves step done (engine.rs:1409-1412) | confirmed | engine.rs:1409 Landing::Advance => {, :1410 re-reads step, :1411 reconcile_done_step(&row, &step, &[]), :1412 }. |
| C76: Engine::resume at engine.rs:940(-999) does criterion 3's topology check; not-comparable list at 957-963 | partial | resume spans engine.rs:940-999 (topology compare at :972). The not-comparable ResolveError list (FanOutCap, AgentCap, ReviewFanOut, LocalFanOut, NoCandidate) is at :957-961. :962 is ') => {' and :963 starts the note body. **Resolution:** Amendment applied: Not-comparable list at engine.rs:957-961 (arm through :970). |
| C77: after_done at engine.rs:1348-1353 is a hook FakeOrchestrator can stall | confirmed | engine.rs:1348-1353 calls self.parts.sink.after_done(...) after a done session and before verify/capture. engine.rs:3851-3862 impl SessionSink for FakeOrchestrator forwards to FakeOrchestrator::after_done (htui-orch/src/fake.rs:1221), an async fn, so a never-returning future can be added there. No stall exists today: it only advances the clock and writes a document, so stall_after_done is new work, as D100 plans. |
| C78: start_run's claim tail at engine.rs:446-460 (claim_run call at :450-457) returns CommandOutcome::Started { run, rest } | confirmed | engine.rs:446 lease, :450-454 `if !self.parts.store.claim_run(id, box_id, owner, now, lease).await?`, :456 ClaimRefused, :459 run_to_rest, :460 Ok(CommandOutcome::Started { run: id, rest }). |
| C79: drive_group's cancel-safety doc at engine.rs:1588-1595 / :1590-1595 | confirmed | engine.rs:1590 '**Not cancel-safe.**' through :1595 '...milestone 5's sweep releases them'; fn drive_group at :1596. :1584-1588 is the join_all/'static rationale. |
| C80: command.rs:432-433 (select_enabled) says 'milestone 5's sweep owns that retry' | confirmed | command.rs:431-433: '**A parked run has no other resume verb** (blueprint R-7): ... refused here as [`EngineError::AlreadySelected`]; milestone 5's sweep owns that retry.' |
| C81: status.rs has test run_failure_display_is_ana2s_bytes, RunFailure has Display; retry_enabled accepts failed | confirmed | status.rs:349 fn run_failure_display_is_ana2s_bytes; status.rs:75 impl fmt::Display for RunFailure; command.rs:401-405 retry_enabled accepts StepStatus::AwaitingApproval \| StepStatus::Failed. |
| C82: The workspace has unsafe_code = "forbid" | confirmed | Cargo.toml:113 unsafe_code = "forbid"; htui-orch has [lints] workspace = true. |
| C83: Engine has a clock, an owner and a box id available to pass to refresh_lease / adopt_runs (clock.now()) | confirmed | EngineParts.clock engine.rs:268, box_id :284, owner: Uuid :286; Engine::now() = self.parts.clock.now() at :3303-3305. traits.rs:725 refresh_lease(run, owner: Uuid, until) and adopt_runs (:735-741) takes box/owner/now/lease_until. |
| C84: Refusal-at-claim: the engine has no trait read of other runs, so it cannot re-derive why claim_run refused | partial | The conclusion holds: overlapping_runs is inherent only (htui-core/src/store/mem.rs:659, htui-store/src/pg/read.rs:1729), and there is no trait read of runs by box or by scope. But the trait does have ReadStore::runs(item) -> Vec<RunSummary> (traits.rs:78) and run(id) (:142), so the engine can read its own item's other runs, just not other items' runs on the box or in the scope. **Resolution:** Amendment applied: Reword: 'the engine has no trait read of runs across items (by box or repo scope); `runs(item)` (traits.rs:78) sees only the same item's runs, so neither rule S's slot count nor an overlap with another item's run can be re-derived.' |
| C85: Cli::reset_hard is M3's fifth git verb at git.rs:609 | confirmed | crates/htui-orch/src/isolate/git.rs:609 `pub async fn reset_hard(&self, tree: &Path, target: &str)`. The module doc at git.rs:11-14 lists the six verbs in this order: worktree add, worktree remove, merge --no-ff, merge --abort, reset --hard, diff. reset --hard is the fifth. |
| C86: is_dirty (untracked excluded) at git.rs:1256-1263 (T3 cites :1263) | confirmed | git.rs:1255-1262 is the doc comment. 'untracked files excluded' is at :1257-1258. `pub fn is_dirty(path: &Path)` is at :1263 and the body ends at :1268. Both citations land on the function. |
| C87: Isolator trait at isolate.rs:126-207; cleanup(run,_) at isolate.rs:201-206 removes trees | partial | isolate.rs:126 opens `pub trait Isolator` and :207 closes it, so that range is right. Line 201 is the tail of reconcile's signature (`) -> IsolatorFuture<'a, Vec<RunStepCommit>>;`). cleanup's doc is at :203-205 and its signature at :206. The tree removal is in GixIsolator's impl at real.rs:1356-1399: worktree remove, remove_directory for copy, and the run dir. **Resolution:** Amendment applied: Cite cleanup as isolate.rs:203-206 (impl at real.rs:1356-1399, which removes worktree/copy trees and the run dir). |
| C88: dirty_tree_not_reset refusal string at real.rs:114-116 | confirmed | real.rs:113 #[must_use]; :114 `pub fn dirty_tree_not_reset(path: &Path) -> String {`; :115 `format!("dirty_tree_not_reset: {}", path.display())`; :116 `}`. |
| C89: GixIsolator::release_run exists at real.rs:512-527 and is currently unreachable from the trait | partial | real.rs:512 is the doc line and :513-527 is `fn release_run(&self, run: RunId)`, so the location is right. But it is reachable from the trait: GixIsolator's Isolator::cleanup calls `self.release_run(run)` at real.rs:1387. What is missing is a trait path that releases the guards without also removing trees. **Resolution:** Amendment applied: Say: release_run (real.rs:513-527) is reachable only through Isolator::cleanup (real.rs:1387), which also removes trees. D99's `release` exposes it on its own; it does not make an unreachable function reachable. |
| C90: GixIsolator and FakeIsolator are the only Isolator implementors (real.rs:1214, fake.rs:280) | confirmed | `grep -rn 'Isolator for' crates --include='*.rs'` returns only crates/htui-orch/src/isolate/real.rs:1214 `impl Isolator for GixIsolator` and crates/htui-orch/src/fake.rs:280 `impl Isolator for FakeIsolator`. There are no blanket impls and nothing in tests. |
| C91: git::create_branch is a gix ref write at git.rs:1337 | confirmed | git.rs:1337 `pub fn create_branch(path, name, target)` opens the repo and calls `repo.reference(format!("refs/heads/{name}"), id, PreviousValue::MustNotExist, "htui: label")`, which is a gix ref write. Callers go through with_retry plus blocking (real.rs:845-847, :901-903). |
| C92: M3 D43's shared_serialized guards are in-process tokio mutexes at real.rs:197-198, :219-229 | confirmed | real.rs:197 is the doc and :198 is `type Guards = Mutex<BTreeMap<(BoxId, RepoId), Arc<tokio::sync::Mutex<()>>>>;`. The `held: Held` field doc is at :219-226 and the field at :227 (Held = Mutex<BTreeMap<(RunId, StepId), Vec<OwnedMutexGuard<()>>>>, :206). These are process-local only. |
| C93: Reconcile crash-idempotent: HEAD with parents [before, after] answered not refused (real.rs:1001-1006, :1036-1043; T8 :1036-1041) | confirmed | real.rs:1001-1005 is the doc that describes the H-3 crash case, and :1006 is `async fn reconcile_isolated(`. At :1036-1043, if head != base_ref it reads head_parents. When parents == [base_ref, after] it returns Ok(Some(head)) at :1040-1041; otherwise it refuses with primary_moved at :1043. T8's :1036-1041 is a correct subset. |
| C94: reconcile / reconcile_isolated at real.rs:1007-1060 with a blocking gix read; git::with_retry used at real.rs:945-947 | partial | reconcile_isolated's doc is at real.rs:~990-1005, the fn is at :1006 and it closes at :1063. Line 1007 is `&self`. The trait-level `reconcile` impl is at :1322, not in that range. The blocking gix reads (e.g. :1019, :1032, :1036) are in range. real.rs:945-947 is `self.under_admin(.., \|\| git::with_retry("worktree remove", ..))`, a worktree remove under the admin lock, not reset_hard. The plan pairs it with Cli::reset_hard, so a closer example is real.rs:1114 `git::with_retry("reset --hard", \|\| git.reset_hard(&local, &target))` (or :413, :714). **Resolution:** Amendment applied: Patterns rows: reconcile_isolated at real.rs:1006-1063 (trait reconcile at :1322). For the 'git write through wrapper, retried' row, cite real.rs:1114 (with_retry("reset --hard", reset_hard)) instead of :945-947. |
| C95: M3 D46: tree never runs git worktree prune or gc (git.rs:19, real.rs:1395) | confirmed | git.rs:19 `//! git worktree prune is never spawned (plan D46)` and real.rs:1395 `// D46, last: git worktree prune is never spawned`. No `"gc"` or `"prune"` argv appears anywhere in crates/htui-orch/src. |
| C96: gix_isolator.rs uses `let Some(git) = skip_without_git!() else { return; };` in its existing cases | confirmed | tests/gix_isolator.rs:40 `use htui_orch::skip_without_git;`. There are 16 uses, e.g. :420, :1109-1111, :1170. One case binds `_git` (:1439). The macro is #[macro_export]ed from git.rs:1632-1643, and its doc gives this exact idiom. The local-mode criterion 12 case deliberately does not use it. |
| C97: criterion_12_a_local_step_on_a_dirty_tree exists at gix_isolator.rs:514-521 and needs no git | partial | gix_isolator.rs:514 is blank. The doc is at :515-520, #[tokio::test] at :521 and `async fn criterion_12_a_local_step_on_a_dirty_tree()` at :522. It uses Fixture::new(Isolation::Local, None) with no skip_without_git, and its doc says 'No git needed'. **Resolution:** Amendment applied: Cite gix_isolator.rs:515-522 (fn at :522). |
| C98: CommittingSink in gix_isolator.rs at :369-414 | confirmed | gix_isolator.rs:369 `struct CommittingSink<'a>`, :379 `impl SessionSink for CommittingSink<'_>`, and :414 closes the impl. |
| C99: Only git subprocesses in htui-orch src are isolate/git.rs's Cli (six verbs) and verify.rs; grep 'Command::new' hits only those two | confirmed | `grep -rn 'Command::new' crates/htui-orch/src/` hits isolate/git.rs:215 (std::process git --version in Cli::probe_within), :315 (tokio Cli::run_capturing), :2825 (test oracle inside #[cfg(test)] mod tests from :1806) and verify.rs:326 (tokio shell). No other file matches. Note that Cli also spawns a synchronous `--version` probe on top of the six verbs. |
| C100: grep -rnE '"gc"\|worktree prune' crates/htui-orch/src/isolate/git.rs finds no spawned verb | confirmed | The grep's only matches are the doc comments at git.rs:19 and :1403, both saying prune is never run. There is no `"gc"` and no spawned prune. The grep output is not empty, so the acceptance check has to be read by eye; an 'empty output' gate would fail. |
| C101: FakeIsolator has a per-repo lock and a reconciles() counter / reconcile scripting | partial | fake.rs:86-89 `serial: Arc<tokio::sync::Mutex<()>>` is documented as 'Plan D28's per-repo lock, one for the whole fake'. It is a single lock, not one per repo, and it is held per step in `held` (:91). reconciles() (fake.rs:208) returns a Vec<(StepId, Vec<StepId>)> call log, not a counter. Scripting exists: refuse_reconcile (:143), fail_reconcile (:153), reconcile_failures queue (:66). **Resolution:** Amendment applied: Say FakeIsolator has one whole-fake serial lock (fake.rs:86-91), a reconciles() call log (fake.rs:208) and refuse_reconcile/fail_reconcile scripting (:143, :153). T7's 'per-repo lock released per run' means releasing that single `serial` guard for the run's steps. |
| C102: run_step_tree rows carry isolation mode, dirty flag, path and before_hash; M3 D23 own htui/<step_id> tree per step; M3 D26 label at capture | partial | RunStepTree (crates/htui-core/src/model/run.rs:426-439) has run_step_id, repo_id, mode, path, base_ref and dirty. It has no before_hash field; before_hash lives on RunStepCommit (run.rs:585, 0001_init.sql:504). The isolate.rs:127 trait doc calls the HEAD carried in base_ref 'before_hash', and with a fan-out slot base_ref is the slot base (real.rs slot_base :118-). D23 per-step worktree on htui/<step_id> is confirmed (real.rs:601, :1973), and so is the D26 label at capture (real.rs:882-903). **Resolution:** Applied to D92 and T3: the reset target is `tree.base_ref`. |
| C103: Every gix call runs on the blocking pool and every git child is a tokio process (M3 D22) | partial | Every gix call in real.rs outside tests goes through blocking(..) (real.rs:363-1163; git::unborn_head at :366 only builds a string), and the git.rs module doc :5-10 says the same. All verb children are tokio::process (git.rs:315). The exception is Cli::probe_within, which spawns `git --version` with std::process::Command (git.rs:215). It runs synchronously at worker start/GixIsolator::new (git.rs:42), bounded by PROBE_TIMEOUT, outside the engine walk. **Resolution:** Amendment applied: R-19 mitigation: 'every git verb child is a tokio process; the one synchronous child is the bounded git --version probe at GixIsolator construction (git.rs:215), outside the walk'. The risk rating is unchanged. |
| C104: Adding two trait methods without default bodies to Isolator compiles as long as only real.rs and fake.rs implement it and no caller exists yet | confirmed | The only implementors are real.rs:1214 and fake.rs:280, with no blanket impls anywhere in crates. The trait is used as &dyn Isolator (isolate.rs:37) and as `I: Isolator + ?Sized` (engine.rs:251, :298, :324, :362). New methods of the form `fn reset<'a>(&'a self, ..) -> IsolatorFuture<'a, _>` stay dyn-compatible, like the existing ones (IsolatorFuture is a boxed dyn Future, isolate.rs:40). The methods must avoid generic type parameters to keep &dyn Isolator valid. |
| C105: topology hashes phases[] only (graph.rs:223-233) | confirmed | crates/htui-orch/src/graph.rs:223 `pub fn topology(phases: &[SnapshotPhase])`, which serialises `phases` and sha256es it. The function closes at :233. Nothing else goes into the digest. |
| C106: gate::apply at gate.rs:373(-438) | confirmed | gate.rs:373 `pub async fn apply<S: WriteStore, C: Clock + ?Sized>(`. The match closes at :438 and the fn brace is at :439. |
| C107: exhausted-failed cell is retry_or_fail -> finish_run(Failed) at gate.rs:526-556 | partial | gate.rs:526 `async fn retry_or_fail`. The finish_run(Failed) call is at :545-552 and the Rest it returns is at :553-557. The fn ends at :558, not :556. Its doc comment starts at :520. **Resolution:** Amendment applied: Cite gate.rs:526-558 (finish_run(Failed) at :545-552). The behaviour described is correct. |
| C108: graph::resolve_scope at graph.rs:245-260, called by graph::resolve at :351-352; D14's refusal at :251-255 | confirmed | graph.rs:245-260 is resolve_scope. :351 fetches `store.repos(...)` and :352 is `let repo_scope = resolve_scope(item, &repos, requested_scope)?`. :251 is `match (requested, primary)` and :252-255 is the `(Some([]), Some(repo)) => Err(EmptyScopeWithPrimary{..})` arm. |
| C109: Command::StartRun has a repo_scope field (command.rs:50) | confirmed | crates/htui-orch/src/command.rs:43 `StartRun {` and :50 `repo_scope: Option<Vec<RepoId>>,`. |
| C110: A phase's resolved isolation is at graph.rs:518 | confirmed | graph.rs:518 `let isolation = phase.isolation.unwrap_or(settings.default_isolation);` |
| C111: claim_run_admits_one_and_refuses_the_second at store/conformance.rs:4057-4229; scope-less run_snapshot() at :3760-3780 | partial | crates/htui-core/src/store/conformance.rs:4057 starts the fn, and it closes at :4223. Lines :4225-4230 are the doc comment of lease_refresh_is_a_cas_on_owner, which starts at :4231. run_snapshot() runs from :3760 to :3780, and its GraphSnapshot literal (no scope field) starts at :3761. Scopes go to new_run separately: :4077-4080 has [repo], [repo], [], []. **Resolution:** Amendment applied: Cite conformance.rs:4057-4223 (the fn doc starts at :4051). run_snapshot() at :3760-3780 is correct. |
| C112: gate::park is three compare-and-sets at gate.rs:489-518 (M2 H-10) | confirmed | gate.rs:488 has the doc 'The three compare-and-sets of a gate park, in step → run → item order (blueprint H-10)'. The fn runs :489-518: transition_step at :495-502, transition_run at :503-510, move_item at :511-512. |
| C113: Stage 6 moves the step done at gate.rs:387-392 | confirmed | gate.rs:387 is the arm `(Gate::OnFailure \| Gate::Never, Settle::Ok { note })`. :389-391 is `transition_step(step.id, Running, Done, now)` and :392 is `Ok(Landing::Advance)`. The caller reconciles afterwards at engine.rs:1409-1412. |
| C114: Orchestrate trait at htui-orch/src/conformance.rs:55-113 | confirmed | conformance.rs:54 `#[allow(async_fn_in_trait)]`, :55 `pub trait Orchestrate {`, and :113 `}`. The impl for FakeOrchestrator starts at :115. |
| C115: lease_refresh_is_a_cas_on_owner at store conformance.rs:4231 (D88 leg lands at :4287-4336) | partial | The fn starts at conformance.rs:4231 and ends at :4339. Its adopt_runs legs run from :4285 (`assert!(` of 'live lease not abandoned') to :4338 (the unknown-box leg ends there). :4287 and :4336 both fall inside asserts, not on statement boundaries. **Resolution:** Amendment applied: Say the adopt_runs section of the case is :4285-4338 (fn :4231-4339), and the D88 leg is added there, e.g. after the 'expired lease adopted' leg at :4307-4316. The case location at :4231 is correct. |
| C116: mem_store.rs CASES pin with enumerating message at crates/htui-core/tests/mem_store.rs:35-42 | partial | mem_store.rs:35 `assert_eq!(`, :36 `conformance::CASES.len(),`, :37 `49,`, and the message is at :38-42. The closing `);` is at :43. The message counts 15+5+2+1+12+1+11+1+1 = 49, which matches. **Resolution:** Amendment applied: Cite mem_store.rs:35-43. The pin value of 49 and its enumeration are correct. |
| C117: htui-orch CASES at conformance.rs:186-268; cases_are_unique_and_thirty_six at :3153-3170; CASES doc at :165-185; fake_conformance.rs:14-17 has cases_len_is_thirty_six | partial | CASES runs :186-268 and I counted 36 string entries. The test has #[test] at :3153 and the fn at :3154, and it ends at :3172; its message is :3162-3170 and the assert closes at :3171. The CASES doc starts at :164 ('Case names in run order') and ends at :185. tests/fake_conformance.rs:14-17 is `#[test] fn cases_len_is_thirty_six() { assert_eq!(CASES.len(), 36); }`, which is correct. Side note: both enumerating prose texts are internally inconsistent. The doc's categories sum to 37 (7+4+1+3+1+3+5+12+1). Entries 24-36 are 13 fan-out-milestone cases, yet the test message calls them 'twelve'. Whoever rewrites them in T6 should recount. **Resolution:** Applied to T6, with a "recount, do not append" instruction. |
| C118: GraphSnapshot struct literal at graph.rs:338(-349); run_snapshot() literal at store conformance.rs:3761 | confirmed | graph.rs:338 `let snapshot = GraphSnapshot {` closes at :349. htui-core/src/store/conformance.rs:3761 `GraphSnapshot {` sits inside fn run_snapshot() at :3760. A grep for `GraphSnapshot {` finds exactly five literals: graph.rs:338, pg_criteria.rs:691, fixtures.rs:1328, mem.rs:5794 and conformance.rs:3761. |
| C119: Other GraphSnapshot producers at status.rs:334, gate.rs:1004 and tests/review_loop.rs:28 decode RUN_1's JSON (not struct literals) | partial | status.rs:334, gate.rs:1004 and tests/review_loop.rs:28 each find RUN_1 in demo_data() and call serde_json::from_value on it; none is a literal. The list is not exhaustive, though. Other decoders exist at command.rs:588, gate.rs:1321, fanout.rs:380 and engine.rs:5565 (snapshot_of decodes the engine-written snapshot). None of these is a literal either, so the 'five literals' conclusion holds. **Resolution:** Amendment applied: Say 'other GraphSnapshot producers (e.g. status.rs:334, gate.rs:1004, tests/review_loop.rs:28, command.rs:588, gate.rs:1321, fanout.rs:380, engine.rs:5565) all decode JSON and are unaffected'. The five-literal count is unchanged. |
| C120: gate::settle at gate.rs:239; SettleInput has fields driver, cap_breach, deadline_seconds, output doc, is_review, verify_outcome; Landing::{Advance, Retry, Rest} | partial | gate.rs:239 `pub fn settle(input: &SettleInput<'_>) -> Settle` is correct. SettleInput (:203-231) has eight fields: driver (:205), cap_breach (:207), started_at (:211), now (:213), deadline_seconds (:215), output (:217), verify_outcome (:224), is_review (:230). The claim leaves out started_at and now; deadline_elapsed reads them. Landing at :342-354 is {Advance, Retry{position, attempt}, Rest(Rest)}, which is correct. **Resolution:** Applied to D91: all eight `SettleInput` fields, with `started_at`/`now` supplied. |
| C121: D14 refusal tests at graph.rs:1300-1352 | partial | This is one test, not several. Its doc comment is at graph.rs:1286, the #[tokio::test] attribute at :1287 and `async fn empty_scope_with_primary_is_refused()` at :1288. It ends at about :1353, after the final `assert_eq!(resolved.repo_scope, vec![repo.id])` at :1352. Lines 1300-1301 are arguments inside its first resolve() call. **Resolution:** Amendment applied: Cite 'the D14 refusal test empty_scope_with_primary_is_refused (graph.rs:1286-1353)'. |
| C122: resolve_scope is re-exported at lib.rs:45 and has no caller outside graph.rs | confirmed | lib.rs:44-46 is `pub use graph::{GraphSource, ResolveError, Resolved, override_graph, resolve, resolve_scope, topology};`, with resolve_scope on :45. Running grep -rn resolve_scope over crates/ finds graph::resolve_scope only in graph.rs (:109 doc, :245 def, :352 call) and lib.rs:45. Caveat: the same grep also hits an unrelated private method `GixIsolator::resolve_scope(&self, scope)` in isolate/real.rs:328, called at :1224 and :1268. It is not a caller, but it means the plan's grep does not come back clean. |
| C123: lib.rs:15-16 says overlap.rs, recover.rs and queue.rs are deliberately not created | confirmed | lib.rs:15-16 reads '`overlap.rs`, `recover.rs` and `queue.rs` are named by ANA-2 §8 and belong to milestone 5 and MOD-12; they are deliberately not created, not even empty (plan D1).' The sentence starts at the end of :15 with 'D53).'. |
| C124: feature_snapshot_topology_is_pinned test exists and fixtures feature.snapshot.json / feature-with-verify.snapshot.json exist; FEATURE_TOPOLOGY pin exists | confirmed | graph.rs:885 `async fn feature_snapshot_topology_is_pinned()` and graph.rs:746 `const FEATURE_TOPOLOGY: &str`, which is also used at :891, :1034 and :1442. Both crates/htui-orch/tests/fixtures/feature.snapshot.json and feature-with-verify.snapshot.json exist. |
| C125: fanout::Route exists in crates/htui-orch/src/fanout.rs as a pure verdict | confirmed | fanout.rs:242 is `pub enum Route` (Human(HumanReason), ...), and fanout.rs:258 is `pub fn route(gate: Gate, has_judge: bool, pre: &Prefilter) -> Route`, which is sync and does no I/O. It is re-exported at lib.rs:40. |
| C126: FakeOrchestrator exists in fake.rs with repoint (retry_limit via repoint) | partial | FakeOrchestrator is at fake.rs:1014 (`pub struct FakeOrchestrator`). repoint is not in fake.rs. It is a private helper in conformance.rs:455, `async fn repoint<O: Orchestrate>(orch, item, mutate: impl Fn(&mut StepGraphPhase))`. It clones the graph and can set retry_limit, as used at conformance.rs:1268 (`phase.retry_limit = 3`). **Resolution:** Applied to T7: the case lives in `conformance.rs` to use its private `repoint`. |
| C127: UNIQUE (run_id, position, attempt, fanout_index) constraint on run_step; judge uses fanout_index = -1 | confirmed | crates/htui-store/migrations/0001_init.sql:494 is `UNIQUE (run_id, position, attempt, fanout_index)`, and 0003_orchestration.sql:65 says '-1 is the judge step'. htui-orch/src/status.rs:162 is `const JUDGE_FANOUT_INDEX: i32 = -1;`. |
| C128: The conservative scope_of fallback (every repo isolated=false, prefixes=[]) reproduces today's 'shared repo overlaps' so claim_run_admits_one_and_refuses_the_second stays green | partial | The admission outcomes are preserved. The case's runs carry run_snapshot() with no scope, and their scopes are [repo], [repo], [], [] (conformance.rs:4076-4080). Today MemStore refuses on any shared repo_scope entry among running/awaiting runs (mem.rs:3253-3259, after the slot check at :3247-3249). Under the fallback, a shared repo with isolated=false is rule I, so first/second overlap and the empty scopes overlap nothing. That gives the same refusals at :4143, :4172 and :4191 and the same admissions at :4161 and :4198. However, D80 says 'byte-for-byte green', and that cannot hold: D83 changes claim_run to return Claim, and plan line 399-400 itself edits this case to assert Claim values. Side note: the case's doc at :4051 says overlap 'refuses before slot count does', but both MemStore (mem.rs:3247-3259) and D83 check the slot first. That doc sentence is already inaccurate. **Resolution:** Amendment applied: Replace 'byte-for-byte green' in D80 with 'green with the same admit/refuse outcomes, its assertions rewritten from bool to Claim per T2'. Optionally fix the :4051 doc to match the slot-then-overlap order. |
| C129: Only the frontier step's merge can be missing because every earlier position's reconcile ran before a row at the next position was created | falsified | The review loop is a counterexample. gate.rs review_loop (:650) calls retire(ctx, steps, target, review.position) (:711, which supersedes or cancels rows at target..=review via retire_slot :884-901) and then creates attempt+1 at the target position (:713-714). A rejected review is `failed` (answer_gate Rejected -> failed, traits.rs:789; mem.rs:3511), and retire cancels it. So when implement attempt 2 at position p becomes done, rows at p+1 (the retired review attempt 1) already exist, created long before this winner's reconcile. If the process crashes between gate::apply's Done move (gate.rs:389-391) and reconcile_done_step (engine.rs:1409-1412), D97's frontier rule ('the done winner at the highest position ... when no row exists at the next position') finds no frontier. The cursor then passes the unmerged implement and walks review attempt 2 over an unmerged primary. The invariant does hold for first attempts and for fan-out, where the winner is reconciled (engine.rs:758, :2150) before run_to_rest's admit. **Resolution:** **Design changed.** D97 redefined: the frontier ignores `superseded`/`cancelled` rows at later positions; T5 gains `frontier_ignores_retired_rows_after_a_review_rejection`; T7 gains `a_crash_between_done_and_reconcile_after_a_review_rejection_is_reconciled` (orch `CASES` 51 → 52). |
| C130: ANA-2 invariants 5, 6, 9 at :124-127, :128-131, :139-142; invariant 10 at :143-146 (stores may not depend on orchestrator) | partial | docs/ANA-2.md:124-127 (inv 5), :128-131 (inv 6), :139-142 (inv 9), :143-146 (inv 10). All line ranges are right. The paraphrase of inv 10 is backwards: :145-146 say 'Enforced by the orchestrator crate depending on htui-core, not on htui-store'. The rule is that the orchestrator must not depend on the store crate. The claim says the stores must not depend on the orchestrator. **Resolution:** Amendment applied: Invariant 10 (:143-146): one cache writer, and MOD-4 never opens the mirror. The orchestrator crate depends on htui-core and not on htui-store, so it reaches Postgres only through WriteStore. |
| C131: §4.3 run/step tables at :603, :605, :611, :618-621, :642-643; :605 only refusal-at-claim exit; :611 out-of-budget park; :618-621 recovery sweep re-runs on adoption; :637 verify_outcome fail not done | partial | :603 is the claim row. :605 'queued \| refusal at claim \| capability, secret or agent-selection refusal \| failed' is the only refusal row in the run table (the item table has its own at :572, which goes to blocked). :620 has 'it is what the recovery sweep re-runs on adoption'. :637 has the guard `verify_outcome != 'fail'` for done. :642-643 are the sweep rows. :611, however, says 'running \| lease expired and the step is unrecoverable \| recovery sweep (§4.9) \| awaiting_approval'. It says nothing about budget. The budget-exhausted escalation park is :609. **Resolution:** Amendment applied: :611 is the recovery-sweep park of an unrecoverable step after lease expiry. The out-of-budget reading comes from §4.9 :1298 ('else the run parks at awaiting_approval'), not from :611. Cite :611 together with :1298. |
| C132: §4.6 commit-capture table :955-968, base-ref rule :948-953; :960 after_hash NULL for no-commit step; :959-960 stage-5 row; :964-968 dirty as non-resettable flag | partial | :948-953 is the base-ref rule. :955 is the heading, :957-960 the table and :962-968 the prose after it. :960 is the stage-5 row ('left NULL when the step committed nothing'). :964-968 cover dirty=true and 'non-resettable (§4.9)' at :968. :959 is the stage-2 (prepare) row, not part of the stage-5 row. **Resolution:** Amendment applied: The stage-2 row is :959 (run_step_tree + before_hash) and the stage-5 row is :960 alone. The table body is :957-960, and :955-968 means heading + table + before_hash/dirty prose. |
| C133: §4.7 spans :1009-1158: RunScope :1041-1055, predicate :1059-1072, PathPrefix :1074-1077, three cautions :1079-1089, admission :1091-1113 | confirmed | The §4.7 heading is at :1009 and §4.8 starts at :1160, so :1159 is blank. The RunScope struct is :1041-1048 and its prose :1051-1055. The predicate block is :1059-1072, PathPrefix :1074-1077, the three cautions :1079-1089 and admission (text + SQL + rationale) :1091-1113. |
| C134: ANA-2 :1035 says RunScope is 'resolved at queue time and stored on the run'; :1040-1048 per-repo isolated/local/paths | confirmed | :1035 reads '*Inputs, all resolved at queue time and stored on the run.*' :1040 is the doc comment 'resolved once at queue time and stored'. :1045-1047 are the BTreeMap<RepoId,..> fields isolated, local and paths. |
| C135: §4.7 predicate has no box term (:1060-1067) while admission SQL counts per box (:1096-1099); rules L, I, P order at :1063-1066 | confirmed | The predicate at :1060-1067 uses only repos, local, isolated and paths. RunScope carries box_id (:1043), but the predicate never reads it. The prose for rule L (:1086) does say 'on that box'. The admission SQL at :1096-1099 locks the box row and counts `executing_box_id = $box AND status = 'running'`. Rules L, I and P sit at :1064, :1065 and :1066 inside the for-each loop at :1063. |
| C136: Rule L is 'refused' with a message naming the holding run (:1085-1087) | confirmed | :1085-1087 read '...is instead **refused** when another non-terminal run holds any tree in that repo on that box, with a message naming the holding run.' |
| C137: :1025 bare glob = primary repo; :1051-1052 repos = qualified + primary; :1053-1055 isolated/local per repo; :1029 declaration buys parallelism | partial | :1025 (bare glob means the primary repo), :1051-1052 (repos = every qualified entry, plus primary when a bare glob or no glob is declared) and :1029 ('Declaration becomes the way to buy parallelism') are all right. :1053-1055 define only isolated[r] (true when every phase uses worktree or copy for r). local[r] is not defined there. It appears only as the field comment at :1046 ('the `local` mode specifically'). :1052-1053 add that isolation resolved per project applies to every repo in scope. **Resolution:** Amendment applied: Write it as ':1053-1055 isolated[r] per repo (every phase worktree\|copy); local[r] is only the field comment at :1046'. |
| C138: §4.9 spans :1242-1340: lease :1276-1282, sweep :1284-1307, completed siblings :1317-1323, offline window :1325-1340; :1272-1274 partial usage is durable | confirmed | The §4.9 heading is at :1242 and §4.10 at :1342. Lease :1276-1282, sweep SQL + table + River + untouched steps :1284-1307, siblings :1317-1323, offline window :1325-1340 and partial usage :1272-1274 (quoting ANA-4:1105-1107) all match. |
| C139: :1280-1282 says 'a zero-row refresh means the lease was taken and the orchestrator abandons the run without writing further' | confirmed | The quote is verbatim at docs/ANA-2.md:1280-1282, followed by 'which is the single-writer rule of invariant 1 applied to processes'. |
| C140: :1297 artefact test: after_hash present for every repo in scope and a document of output_kind; first row status='done' | confirmed | :1297 reads '`after_hash` present for every repo in scope **and** a document of `output_kind` produced by this step exists \| ... `status = 'done'`, then re-derive the run status and continue the walk'. |
| C141: :1298-1299 second/third rows move the item to blocked, gate_note='interrupted', status failed; :1298 dirty=false at stage 2 is the whole reset test; :1299 note names paths/before_hash | partial | Three details are off. (1) Row 2 (:1298) blocks the item only when the budget is spent. It says 'a new attempt if budget remains, else the run parks at awaiting_approval and the item goes to blocked'. Row 3 (:1299) always parks and blocks. (2) The gate_note differs by row: 'interrupted' at :1298 and 'interrupted, tree not reset' at :1299. (3) At :1299 the Runs tab, not a note, names the tree and before_hash: 'the Runs tab names the tree and the `before_hash`'. The reset test at :1298 does match the claim: worktree\|copy, or shared_serialized\|local with dirty=false. :1298 does not say 'stage 2', but dirty is written at stage 2 per :959. **Resolution:** Amendment applied: Row 2 (:1298): failed + gate_note 'interrupted', then a new attempt if budget remains, else park at awaiting_approval + item blocked. Row 3 (:1299): failed + gate_note 'interrupted, tree not reset', always park + blocked, and the Runs tab (not the note) names the tree and before_hash. If the item_note must name trees and before_hash, as D92 plans, that is the plan's addition, not ANA-2 text. |
| C142: :1301-1304 is the River case; :1306-1307 the sweep never touches awaiting_approval/parked runs and leaves pending steps for the walk; :1284 sweep at start and every TTL | confirmed | :1301-1304 is the River quote. :1306-1307 read 'Steps in `awaiting_approval` are never touched by the sweep ... a run parked for a week is not a crashed run. Steps in `pending` are simply re-scheduled.' :1284 reads 'runs at orchestrator start and every TTL thereafter'. What actually excludes parked runs is the sweep SQL `WHERE status = 'running'` (:1288). |
| C143: :1317-1319 'completed siblings survive … selection then runs over whatever survived'; :1318 'only the failed index is re-attempted' | confirmed | :1317 has 'Completed siblings survive a failed sibling.' :1318 has 'only the failed index is re-attempted'. :1318-1319 have 'selection then runs over whatever survived'. |
| C144: :1325-1336 offline window: an unreachable store is not a taken lease; :1338-1340 'nothing new is built for this' | partial | The offline-window list is at :1325-1336, and :1338 reads 'Nothing new is built for this'. The phrase 'an unreachable store is not a taken lease' does not appear anywhere in ANA-2. :1328-1329 and :1333 say the orchestrator writes nothing offline and the lease will expire because the refresh is a write. Treating a refresh Err as 'keep going' is therefore the plan's inference (D86). **Resolution:** Amendment applied: Word it as an inference: ':1325-1336 (the lease is left to expire offline, the refresh is a write; the reconnect sweep adjudicates) implies a refresh error is not a taken lease'. Do not present it as ANA-2 text. |
| C145: :1408 is §4.10's timer-approval paragraph, not the lease | confirmed | §4.10 starts at :1342. :1407-1410 is 'Timer-based auto-approval is rejected. Jules auto-approves a plan on a timer...', and :1408 sits inside it. Nothing there is about leases. |
| C146: §5.4 :1518-1519 defines app_setting lease_ttl_seconds and lease_refresh_seconds | confirmed | docs/ANA-2.md:1518 `lease_ttl_seconds` \| integer \| 120 \| §4.9; :1519 `lease_refresh_seconds` \| integer \| 60 \| §4.9 (the §5.4 heading is at :1507). |
| C147: §6.2 commands at :1558-1571 has no claim verb; :1562 retry_enabled accepts failed; :1568-1569 Unblock | confirmed | The table at :1558-1566 lists AnswerGate x2, RetryStep, PromoteStep, CancelRun/CancelStep, OpenArtifact and SelectFanout. :1568-1571 add AcceptArtifact, Unblock, CloseOut and StartRun. There is no claim verb. :1562 enables retry for 'step `awaiting_approval` or `failed`, and `attempt <= retry_limit + 1`'. `Unblock { item }` is at :1569. |
| C148: §8 :1669 sketches overlap.rs in htui-orch; :1671; :1681-1682 run_worker 'spawns one engine task per active run'; :1740-1742 (:1740 says claim_run returns false on refusal) | confirmed | :1669 is 'src/overlap.rs RunScope, the predicate, PathPrefix normalisation, admission transaction' under crates/htui-orch/ (:1659). :1671 is recover.rs. :1681-1682 describe run_worker.rs: 'one per box, owns the lease refresh, spawns one engine task per active run'. :1740 has claim_run 'returning `false` when the slot or the overlap check refuses', then refresh_lease at :1741 and adopt_runs at :1742. |
| C149: :1777 amendment records that worktree prune is not run; :999 says 'git worktree prune is run once after every cleanup' | confirmed | :999 reads '`git worktree prune` is run once after every cleanup.' The 2026-09-22 supersession note at :1777 ends '`git worktree prune` is never run, because it refuses locked entries and cannot be scoped'. |
| C150: §9 build step 7 at :1811-1813; §10 row 6 at :2054 | confirmed | :1811-1813 is step 7, 'overlap.rs and recover.rs: the predicate, the admission transaction, the lease and the recovery sweep...'. :2054 is §10 row 6, 'Lease TTL and refresh interval \| 120 s and 60 s'. |
| C151: §11 risk 8 at :2071 (prefix over-serialises src/** vs src/**), risk 9 at :2072 (stalled, offline label), risk 13 at :2076 (never displace a live orchestrator) | confirmed | :2071 is risk 8 ('`src/**` and `src/**` in one repo always overlap'). :2072 is risk 9 (the Runs tab 'labels the run "stalled, offline"'). :2076 is risk 13 ('Adoption requires an expired lease, so a live orchestrator is never displaced'). |
| C152: §12 criteria 3, 12, 15, 16, 18 at :2090-2091, :2118-2119, :2125-2130 (15's third clause :2126-2127; 16 :2128-2130), :2135-2137; criterion 19 at :2138-2140; :2136-2137 'has its worktree reset to before_hash and is retried' | confirmed | Criterion 3 is :2090-2091, 12 is :2118-2119, 15 is :2125-2127 (the empty-touched_paths clause is :2126-2127), 16 is :2128-2130, 18 is :2135-2137 and 19 is :2138-2140. :2136-2137 read 'a step with neither has its worktree reset to `before_hash` and is retried'. |
| C153: ANA-2 :450 escalation cell vs :575/:608 (M2 warning) | confirmed | The :450 `never` x settle-failed cell reads 'else step to `failed` and run to `awaiting_approval` (escalation)'. :575 (item -> failed) and :608 (run 'a step reached failed with no retry budget and no gate' -> failed) disagree with it. HANDOFF.md:251-254 records the M2 warning that milestone 5 must not read :450 literally. |
| C154: ANA-2 invariant 2 (snapshot is run's queue-time record) | confirmed | docs/ANA-2.md:109-113 invariant 2: 'The executing copy of a graph is `run.graph_snapshot`, never the live tables' and 'the snapshot being taken in the same transaction that inserts the `run` row'. The insert is the queue event (:602). |
| C155: The sweep never touches parked (awaiting_approval) runs per ANA-2 :1306, so it cannot be R-7's resume verb, and an awaiting_approval step under a running run is never touched | confirmed | :1306 says awaiting_approval steps are never touched by the sweep and 'a run parked for a week is not a crashed run'. The sweep SQL at :1287-1290 selects only `status = 'running'` runs whose lease has expired, so a parked run is never adopted and the sweep cannot serve as a resume verb for it. Inside an adopted running run, the per-step table at :1293-1299 acts only on `running` steps, which confirms that an awaiting_approval step is left alone. |
| C156: PRD milestone 5 at :309 quoted verbatim | confirmed | mod-4-orchestrator-manual-mode.prd.md:309 row 5 reads exactly 'The overlap predicate, the admission transaction, the lease and its refresh, and the recovery sweep's artefact test — including the refusal to reset a dirty tree.' (status pending) |
| C157: PRD success rows :190 (crit 12), :191 (15,16), :192 (18 only) | confirmed | PRD:190 'A dirty tree is never reset' -> criterion 12; :191 'Overlap serialises, isolation parallelises' -> criteria 15, 16; :192 'A killed orchestrator loses no finished work' -> criterion 18 only |
| C158: PRD risk rows :390, :400, :389, :393 | confirmed | PRD:389 git contention loses committed work; :390 dirty local/shared_serialized tree reset by recovery; :393 schema_version bump silently rebuilds mirror; :400 two htui processes adopt each other's runs |
| C159: PRD scope bullet :241-243; :229-230 shared_serialized = Postgres advisory lock per (box, repo) | confirmed | PRD:241-243 'Overlap, admission and the lease: RunScope, PathPrefix, SELECT … FOR UPDATE admission..., lease_owner..., recovery sweep's artefact test.'; PRD:229-230 '`shared_serialized` (a / Postgres advisory lock per `(box, repo)`)' |
| C160: PRD D2 adopts 120 s / 60 s at :337-339 | confirmed | D2 header at PRD:335; 'lease TTL 120 s refreshed at 60 s' at PRD:338, inside the cited 337-339 range |
| C161: PRD :195 step list renders 'gate state' | confirmed | PRD:195 'The Runs tab is complete' row: 'the step list shows agent, model, gate state, usage and duration' (criterion 21) |
| C162: PRD :280 / :272 migration constraints already false given 0004 (HANDOFF:324-331) | confirmed | PRD:272 '**Any second migration.** If a decision seems to need `0004`, it is the wrong decision.'; PRD:280 'The migration set is exactly `0003_orchestration.sql` + `cache_migrations/0003_orchestration.sql`.'; HANDOFF.md:324-331 records 0004_max_agents_per_run_default.sql landing at M4 review |
| C163: R-ORCH-9 :211-213, R-HIS-1 :225-227; R-ORCH-8/11/12 exist with described content | partial | REQUIREMENTS.md:211-213 R-ORCH-9 and :225-227 R-HIS-1 ('Nothing about a run exists only on one box') are exact; R-ORCH-12 :218-220 ('Version one stores the target box and executes only when it is the local box') matches. But R-ORCH-8 (:207-210) only lists the four isolation modes and says nothing about reset rules, and R-ORCH-11 (:216-217) is a list of what every run records (incl. commit hashes), not a statement that resume is decided from hashes and documents **Resolution:** Amendment applied: Reword the glosses: 'R-ORCH-8 (the four isolation modes, whose reset rules ANA-2 §4.6/§4.9 define)', 'R-ORCH-11 (every run records per-step commit hashes and status — the record the artefact test reads)'. |
| C164: M4 plan/blueprint end at D78 (fanout.plan.md:217); A-1..A-7 = D72-D78 (:43); prior risks end at R-9 | confirmed | mod-4-orch-fanout.plan.md:217 is the D78 row (blueprint A-7); :43 'its A-1..A-7 accepted as D72–D78'; highest D in the plan is D78; R-9 is the newest carried risk (mod-4-orch-fanout.blueprint.md:895 'R-9 (new)'), no R-10 exists in prior plans/blueprints |
| C165: workflow-config.json:2 names rust-reviewer | confirmed | .claude/workflow-config.json is 3 lines; line 2 is '"reviewer": "rust-reviewer"' |
| C166: GRAPH_REPORT.md:12 built from 3e34610 predating htui-orch; graph.json grep counts are 0 | confirmed | GRAPH_REPORT.md:12 '- Built from commit: `3e346107`'; git ls-tree 3e34610 crates/ has no htui-orch, migrations only 0001/0002; grep -c in graph.json returns 0 for claim_run, refresh_lease, adopt_runs, repo_scope, touched_paths, BoxSettings, max_concurrent_items |
| C167: Branch mod-4-m5 cut from main at d854ff1 | confirmed | git rev-parse main HEAD both d854ff17805978b94084cd5e03f0d0389e8e6748; merge-base is the same |
| C168: M1 blueprint R-2 at seam.blueprint.md:1365 with options (a)/(b) and box-filter question | confirmed | mod-4-orch-seam.blueprint.md:1365 R-2: predicate collapsed to 'any shared repo overlaps'; '(a) Store the rest of RunScope at queue time … (a milestone-2 migration'; (b) evaluate in shared Rust predicate so MemStore and PgStore share it; 'The box filter is a third question' |
| C169: HANDOFF:214-217, :229-230 defer R-2 to M5; 'two worktree-isolated runs on one repo are refused' | confirmed | HANDOFF.md:214-217 'claim_run's overlap predicate is repo-set-on-one-box … two worktree-isolated runs on one repo are refused, which is safe but narrower than R-ORCH-9'; :229-230 '**R-2 is still deferred to milestone 5**' |
| C170: HANDOFF:251-254 carries M2 warning about ANA-2 :450; engine.plan.md:223 | confirmed | HANDOFF.md:251-254 'A second §4.2/§4.3 disagreement … ANA-2 :450 … Milestone 5 must not re-read :450 literally'; mod-4-orch-engine.plan.md:223 is the risk row on :450 read literally at milestone 5 |
| C171: HANDOFF:291-293 offers 'milestone 5's sweep or milestone 6's Unblock-shaped verb' for R-7 | confirmed | HANDOFF.md:291-293 'R-7 — a run parked by a reconcile refusal (park_run) has no resume verb, so milestone 5's sweep or milestone 6's `Unblock`-shaped verb must provide one' |
| C172: HANDOFF:319-321 is M4 behaviour 4 (edit to touched_paths after queue time doesn't stop resume); :321-323 behaviour 5 (drive_group not cancel-safe) | partial | HANDOFF.md:319-321 behaviour 4 is '`resume` keeps walking the snapshot when the live graph no longer resolves under lowered caps (FanOutCap, AgentCap, ReviewFanOut, NoCandidate) and writes a note — invariant 2'; it says nothing about touched_paths (that extension is the plan's own D81 adding UnknownTouchedRepo to the list). Behaviour 5 is at :321-322 (322-323 is behaviour 6, StaleSlot), so :321-323 slightly overshoots but is fine **Resolution:** Amendment applied: In D81 say: 'so, by extending M4 behaviour 4 (HANDOFF.md:319-321: resume keeps walking the snapshot when the live graph no longer resolves under lowered caps) to UnknownTouchedRepo, an edit to touched_paths after queue time never stops the run'. Behaviour 5 citation may be tightened to :321-322. |
| C173: HANDOFF:324-331 records 0004_max_agents_per_run_default.sql | confirmed | HANDOFF.md:324-331; filename at :327, commits b86b62c/bcce4b9 at :328-329 |
| C174: fanout.plan.md:190 states D51 premise; :602-606 cargo doc baseline of two errors at 01feaff | confirmed | mod-4-orch-fanout.plan.md:190 D51 row: 'No writer moves a `running` step to `failed` with a `gate_note`'; :602-606 '`cargo doc` baseline at `01feaff`: two pre-existing errors … MIRRORED_TABLES … record_command_run linking the private step_exists' |
| C175: Carried risk citations in M2/M3/M4 blueprints | confirmed | mod-4-orch-engine.blueprint.md:27 F-I, :28 F-J, :29 F-K, :30 F-L, :913 H-9, :914 H-10, :956 carries R-3 (F-I), R-4 (F-J), R-5 (F-K/H-9), R-6 (F-L); mod-4-orch-tree.blueprint.md:25 F-G, :784 carries R-7; mod-4-orch-fanout.blueprint.md:29 F-B, :895 R-9 (new) |
| C176: M3 fact ledger records git 2.43.0; git-backed cases skip below 2.33.0 | confirmed | mod-4-orch-tree.plan.md:627 ledger row 'git 2.43.0 is on this box at /usr/bin/git'; :529-533 and :580 skip below 2.33.0; `git --version` here prints 2.43.0 |
| C177: Prior milestone decisions cited say what the plan attributes | confirmed | Spot-checked each: tree.plan D22 (gix on blocking pool, git verbs as tokio::process), D23 (worktree add -b htui/<step_id>), D24 (is_dirty, untracked excluded), D26 (shared_serialized label at capture), D43 (in-process tokio Mutex per (box, repo), not advisory lock), D46 (no worktree prune, HANDOFF:276); tree.blueprint:695 H-3 (reconcile idempotent on head_parents==[before,after]); fanout.plan D48 (candidate settle + recorded deviation on re-attempt, citing D65's argument), D50/OQ-3 (park at awaiting_approval), D51, D53 (serde(default) template, V not bumped), D58 (FakeDriver never suspends), D59 (group cursor; failed judge re-parks), D63/D64 (start-time refusals), D65 (two attempts in one comparison), D71 (fanout_index on select), D72 (dirty_tree_not_reset for every sibling); engine.plan D7 (finish_run writes failure), D14 (empty-with-primary refused), engine.blueprint:914 H-10 (three-write park) |
| C178: htui-core store CASES has 49 entries at conformance.rs:36-86; READ_CASES has 9 | confirmed | crates/htui-core/src/store/conformance.rs:36 is `pub const CASES`, entries run from :37 to :85 and `];` is at :86. I counted 49 entries. READ_CASES is at :204-214 and has 9 entries. The existing pins agree: mem_store.rs:36-37 asserts 49 and pg_conformance.rs:19 has EXPECTED_CASES = 49. |
| C179: htui-orch CASES has 36 entries at conformance.rs:186-268 | confirmed | crates/htui-orch/src/conformance.rs:186 is `pub const CASES`, the last entry is at :267 and `];` is at :268. I counted 36 string entries in that range. They are pinned at :3153-3170 (`cases_are_unique_and_thirty_six`, which asserts 36) and at tests/fake_conformance.rs:14-17 (`cases_len_is_thirty_six`). |
| C180: WriteStore has 63 async fn at traits.rs:195-991, not pinned by a test; ReadStore has 16 at traits.rs:66-183 | confirmed | traits.rs:195 is `pub trait WriteStore: ReadStore {` and it closes at :991. Inside it there are 63 lines starting with `async fn` and no plain `fn`. traits.rs:66 is `pub trait ReadStore` and it closes at :183, with 16 fns. Searching crates/ for 'sixty-three' or '63 methods' finds nothing, so no test pins the WriteStore count. |
| C181: Five claim_run implementors (mem.rs, pg/write.rs, writer.rs, UsageSpy, SpyStore) | confirmed | `grep 'impl.*WriteStore for'` finds exactly five: mem.rs:4180 MemStore (claim_run at :4461), pg/write.rs:381 PgStore (claim_run at :2464), writer.rs:287 Writer (claim_run at :673), htui-agent/src/conformance.rs:673 UsageSpy (claim_run at :927) and htui-agent/tests/recorder.rs:353 SpyStore (claim_run at :621). crates/htui has no other impl. |
| C182: grep -rn 'claim_run(' crates/: 13 store conformance, 10 mem.rs, 3 pg_criteria.rs, 2 writer.rs, 1 each in the two spies and engine.rs | partial | The literal grep gives per-file line counts of: conformance.rs 13, mem.rs 12, pg_criteria.rs 3, writer.rs 3, agent conformance.rs 2, recorder.rs 2, engine.rs 1, and it also hits traits.rs 1 and pg/write.rs 1. The plan's numbers are call sites with the fn definitions removed. mem.rs has definitions at :3209 and :4461; its 10 call sites are :4470 (delegation to State) and :5972-:7195. writer.rs has its definition at :673 and delegating calls at :682-683. The spies have definitions at :927/:621 and calls at :936/:630. pg_criteria's calls are at :624, :625 and :3357, and engine.rs's is at :453. **Resolution:** Amendment applied: Say these are call sites, not grep lines: "grep -rn 'claim_run(' crates/ minus the five impl signatures and traits.rs:711's declaration: 13 in the store conformance, 10 in mem.rs (9 tests + State delegation at :4470), 3 in pg_criteria.rs, 2 in writer.rs, 1 each in the two spies and engine.rs:453". The file set T2 must touch is unchanged. |
| C183: GraphSnapshot has exactly five struct literals: graph.rs:338, pg_criteria.rs:691, fixtures.rs:1328, mem.rs:5794, conformance.rs:3761 | confirmed | `grep -rnE 'GraphSnapshot \{' crates` finds literals only at graph.rs:338, pg_criteria.rs:691, htui-core/src/fixtures.rs:1328, mem.rs:5794 and store conformance.rs:3761. The other hits are fn signatures, the struct definition at run.rs:448 and the impl at :463. status.rs:334, gate.rs:1004 and tests/review_loop.rs:28 all start with `let run = demo_data().runs...`, meaning they decode, as the plan says. Nothing uses `Self {` inside `impl GraphSnapshot`. |
| C184: T2's file list is 15 paths plus .sqlx | partial | Plan line 324, the T2 row, lists run.rs, traits.rs, mem.rs, store conformance.rs, fixtures.rs, mem_store.rs, pg/write.rs, writer.rs, pg_conformance.rs, pg_criteria.rs, .sqlx/*, agent conformance.rs, recorder.rs, engine.rs and graph.rs. That is 15 entries including .sqlx, so 14 paths plus .sqlx. **Resolution:** Amendment applied: Line 383: "the T2 row above (14 paths plus `.sqlx`)". |
| C185: ~15 new orch cases (T6 six 36->42; T7 nine 42->51), 3 new store cases 49->52; breakdown 15 = 3+1+1+2+4+3 (D87/D88/D97) | partial | The totals are right: T6 names 6 cases (plan :497-505), T7 names 9 (:531-549), 36+6=42, 42+9=51, and the store goes 49+3=52. The breakdown on Test plan line 590-592 only sums to 3+1+1+2+4+3=14, because D87 has two cases (`a_parked_run_releases_its_lease_and_an_answer_takes_it` and `a_live_lease_blocks_an_answer_from_another_process`). D88 has one (`the_sweep_never_touches_a_parked_run_or_a_live_lease`) and D97 has one (`a_crash_before_reconcile_is_reconciled_on_adoption`), so that bucket holds four. **Resolution:** Applied to the Test plan breakdown (now 16 with C129's added case). |
| C186: T7 names nine cases and T6 names six | confirmed | T6 (plan :497-505) names overlapping_touched_paths_serialise, the_same_paths_in_two_repos_run_concurrently, an_undeclared_item_holds_its_whole_primary_repo, a_third_run_waits_for_a_slot_and_a_parked_run_still_blocks_overlap, a_parked_run_releases_its_lease_and_an_answer_takes_it and a_live_lease_blocks_an_answer_from_another_process: 6. T7 (:531-549) names a_finished_step_is_adopted_through_its_gate, an_unfinished_step_is_reset_and_retried, an_interrupted_step_out_of_budget_parks, a_dirty_tree_is_never_reset, a_crash_before_reconcile_is_reconciled_on_adoption, an_interrupted_candidate_fails_alone, an_interrupted_judge_parks_for_selection, a_half_written_park_is_completed and the_sweep_never_touches_a_parked_run_or_a_live_lease: 9. The engine.rs unit tests are listed separately and are not CASES. |
| C187: Wave A file sets pairwise disjoint; T4 and T5 both edit lib.rs; only T2 touches .sqlx; only T4 touches snapshot JSON; no task edits Cargo.toml/Cargo.lock/seeds/.snap | confirmed | The Tasks table (:321-328) gives T1 {model/overlap.rs, model/mod.rs}, T3 {isolate.rs, isolate/real.rs, fake.rs} and T5 {recover.rs, lib.rs}; no path appears in two of them. lib.rs is in T4, T5 and T6. .sqlx appears only in T2 and the snapshot JSON only in T4. No row names Cargo.toml, Cargo.lock, seeds/ or .snap. Cargo.toml already has what the plan relies on: htui-orch/Cargo.toml:31 tokio with `time`, :47 futures, and :60 dev-dep tokio with `test-util`. The three .snap files that match 'topology' are TUI text renders, not graph_snapshot. tests/fixtures.rs:33 compares the whole decoded snapshot, so T2's `scope: None` still passes and only T4's `Some(..)` moves the JSON, as the plan says. One risk outside this claim: T5's `classify(.., run_scope, ..)` would depend on T1's `RunScope` if it uses that type, which would couple T5 to T1 in Wave A. The plan should say `run_scope` is `&[RepoId]`, or order T5 after T1. |
| C188: cargo sqlx prepare --check -- --all-targets --all-features from crates/htui-store against a migrated scratch DB validates .sqlx; the compose htui database is empty | confirmed | I ran it from crates/htui-store with DATABASE_URL=.../htui_prepare_check (sqlx-cli 0.9.0, CARGO_TARGET_DIR under /tmp) and it exited 0 at d854ff1. .sqlx is per-crate at crates/htui-store/.sqlx (224 files) and there is no workspace-level .sqlx. The compose `htui` database has 0 tables in public. htui_prepare_check already exists but its _sqlx_migrations stops at version 3. 0004 is a data-only UPDATE of app_setting, so the query types are unaffected. |
| C189: cargo doc --workspace --no-deps baseline at d854ff1 still has exactly the two pre-existing errors; cargo doc -p htui-orch --no-deps --all-features exits 0 | partial | `cargo doc -p htui-orch --no-deps --all-features` exits 0. Plain `cargo doc --workspace --no-deps` exits 101 but reports only one error, htui-core's unresolved `crate::store::MIRRORED_TABLES` (traits.rs:887), then prints 'build failed, waiting for other jobs'. The htui-store error (`record_command_run` linking the private `step_exists`, pg/write.rs:3316) only shows up with `--keep-going`, which reports exactly those two errors. Also, plan line 638 `cargo doc -p htui-core --no-deps --all-features` exits 101 at baseline on the same MIRRORED_TABLES error, but the plan lists it as a check with no note. **Resolution:** Applied to Validation/Acceptance: `--keep-going` baseline of two errors recorded; `htui-core` doc held to "no new error". |
| C190: #[tokio::test(start_paused = true)] requires tokio test-util and paused-clock sleep auto-advances; with_retry's 200/400/800 ms test uses it | confirmed | crates/htui-orch/src/isolate/git.rs:2374 has `#[tokio::test(start_paused = true)]` on `with_retry_retries_three_times_on_a_lock_error_and_not_on_others`, and git.rs:796-803 defines the 200/400/800 ms schedule. The dev-dep tokio with `test-util` is at htui-orch/Cargo.toml:60. start_paused needs test-util and the current_thread runtime (the default flavour), and a paused runtime auto-advances time when idle. htui-agent/tests/install.rs:12-15 warns that auto-advance can skip past real-IO timeouts; MemStore-based tests have no real IO, but the heartbeat's injected Clock does not advance with tokio's paused time. |
| C191: PathPrefix::of truncating at first of *?[{ then at last '/' gives the stated test vectors | partial | Applying the rule literally: `src/**/*.rs` gives `src/`, `**` gives `""`, `src/{a,b}` gives `src/`, and `a?b` gives `a` and then `""`, which all match the vectors. But `crates/htui-core/src/model/item.rs` has no wildcard, and 'then at the last /' would cut it to `crates/htui-core/src/model/`, not keep it whole as plan :359 and ANA-2 :1074-1076 both say. The vectors only hold if the cut to the last '/' happens only when a wildcard was found. The plan also does not say whether the '/' is kept (the vectors imply it is kept). **Resolution:** Applied in D81/T1, and T1 now reuses `prompt::excerpt::PathPrefix::parse`, which already implements the stated rule (D104). |
| C192: Claim::SlotFull { running: u64, limit: u32 } matches the types of the running count and max_concurrent_items | confirmed | BoxSettings.max_concurrent_items is `Option<u32>` (box_.rs:114). MemStore::max_concurrent_items returns u32 (mem.rs:3022), and mem.rs:3247 compares `rows(running) >= u64::from(self.max_concurrent_items(box_id))`. In PgStore, the limit is u32 (u32::try_from, pg/write.rs:2507-2520) and it compares `rows(running) >= u64::from(limit)` (:2531). So running is u64 through rows() and limit is u32 in both stores. |
| C193: GraphSnapshot scope stored in JSONB run.graph_snapshot requires no schema change | confirmed | The column is `graph_snapshot JSONB` (0001_init.sql:457). The only constraint is ck_run_graph_snapshot `CHECK (kind <> 'graph' OR graph_snapshot IS NOT NULL)` (0003_orchestration.sql:47-48), and nothing checks keys. The mirror stores it as TEXT (cache_migrations/0001_mirror.sql:106). GraphSnapshot's doc (run.rs:441-446) says optional fields carry #[serde(default)], so an added `scope: Option<RunScope>` with serde(default) needs no DDL. |
| C194: Files to Change and T-task file lists are consistent | confirmed | I cross-checked the Files table (:276-305) against the Tasks table (:321-328) and the per-task Files lines. engine.rs is in T2, T6 and T7 in both tables; fake.rs in T3, T6 and T7; lib.rs in T5, T4 and T6; graph.rs in T2 and T4; orch conformance.rs and fake_conformance.rs in T6 and T7; status.rs in T7; command.rs in T6; recover.rs in T5; orch overlap.rs, tests/fixtures.rs and the two JSONs in T4; isolate.rs and real.rs in T3; gix_isolator.rs in T8; core overlap.rs and mod.rs in T1. All 13 T2 paths in the Files table, plus .sqlx, appear in the T2 row, and every path in the Tasks table appears in the Files table. The only mismatch is the count in C184. |
