# Plan: MOD-4 milestone 4 — three candidates, one winner

**Source**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`, milestone 4 (`:308`): "Fan-out with
the verification prefilter, the two-order judge call, the selection transaction, and `R-AGT-8`'s
walk with the three skip conditions and the empty-candidate fallback." Success metric row `:188`.
Design authority: the PRD's D1–D8 (cited **PRD Dn**; this plan's own decisions are plain **Dn**,
milestone 2's **M2 Dn**, milestone 3's **M3 Dn**), `docs/ANA-2.md` §4.2 (`:376-377`, `:383-389`),
§4.3 (`:576`, `:609`, `:634`, `:650`), §4.4 (`:754-758`), §4.5 (`:766-883`), §4.6 (`:916`,
`:1002-1007`), §4.9 (`:1317-1323`), §5.3 (`:1494-1505`), §5.4 (`:1515-1516`), §6.2 (`:1566`),
§7 (`:1598-1635`), §8 (`:1666`, `:1672`, `:1747`), §9 build step 6 (`:1808-1810`), §11 risks 5 and
12 (`:2068`, `:2075`), §12 criteria 8, 9, 10 (`:2105-2114`) and criterion 7's real-tree half
(`:2103-2104`, via R-8); `docs/ANA-5.md` §4.6a (`:1248-1273`), the verdict block (`:1340-1351`),
the prompt-kind table (`:1858-1870`) and §12 criterion 17 (`:2312-2315`, the engine half);
`docs/REQUIREMENTS.md` `R-ORCH-7` (`:203-206`), `R-AGT-8` (`:164-165`), `R-ORCH-8` (`:207-210`).

**Requirements**: `R-ORCH-7` (fan-out, human or judge selection, losers kept as history),
`R-AGT-8` (the candidate walk), `R-AGT-7` (its read side: quota and the per-run cap at
admission), `R-ORCH-8` (the fan-out halves of `worktree`, `copy` and `shared_serialized`),
`R-ORCH-3` (the loop over a fanned-out implement), `R-ORCH-11` (the judge is a recorded step);
`R-PRM-1` by construction (the judge sees documents, diffs and verify tails, never a transcript);
`R-ID-6` by construction (the judge decides one integer).

**Complexity**: Large. Two new modules (`select.rs`, `fanout.rs`), a group-aware cursor, a
concurrent candidate drive, a two-session judge step, two park shapes, a new command
(`SelectFanout`), four changes to the `Isolator` seam (a slot on `prepare`, `base`, `diff`, a
sibling list on `reconcile`) with their `git` halves, a narrow loosening in `htui-core`'s quota
predicate, the review-loop forwarded set M3 deferred here, and 17 new conformance cases. **No
seam writer, no migration, no `.sqlx` change, no `.snap` change.**

**Routing**: routed as **plan** by `/handoff-run MOD-4` (the PRD and its milestone table exist,
so this milestone enters the chain at `plan`). **Staffing: Opus 5.5 for every step — architect,
implementers and reviewer (maintainer instruction, 2026-09-22)**, which supersedes the standing
"architect Fable, reviewer Fable" note for this milestone. **Ultracode for the implementers
only**, one workflow per task, verify fan-out per round; the architect and the reviewer stay plain
agents. Reviewer: `rust-reviewer` (`.claude/workflow-config.json:2`).

**Numbering**: the tree plan's decision table runs to **D47** (`mod-4-orch-tree.plan.md:192`; no
higher `D` number appears in it or in its blueprint), so this plan starts at **D48**.

**Status**: **complete** (2026-09-23, `fa03782`..`7ee0ac6`; review round `20cb2fa`..`7ee0ac6`
plus the cap change `b86b62c`/`bcce4b9`). Fact-checked, amended and **CONFIRMED by the maintainer
2026-09-23** (answers under "Open questions"; OQ-7 adds D71 and spawns MOD-36/ANA-21). Blueprint
`.claude/plans/mod-4-orch-fanout.blueprint.md`; its A-1..A-7 accepted as D72–D78, its F-A..F-K
corrections apply as written. Branch `mod-4-m4`. The independent fact-check (318 claims) and how
every non-confirmed finding was resolved are in the last section.

**Graphify note**: `graphify-out/GRAPH_REPORT.md` was built from `3e34610` (2026-09-15,
`GRAPH_REPORT.md:12`), whose `crates/` holds four members and **no `htui-orch`** (`git ls-tree -d
3e34610 crates/`). It was used for `htui-core`'s quota and prompt communities only; every
`htui-orch` fact below was read from the tree at `01feaff`.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked.

**Answered at CONFIRM (maintainer, 2026-09-23):** OQ-1..OQ-6 and OQ-8 take the adopted default;
the recorded deviations D48 (no re-attempt of a failed candidate), D63 (retries not counted toward
`max_agents_per_run`) and OQ-3 (`awaiting_approval`, not `blocked`) are accepted. **OQ-7 is
overridden in direction, not in this milestone's behaviour**: the maintainer wants candidates spread
across agents, chosen by per-model weights. That is split out as **MOD-36** (weighted assignment)
blocked on **ANA-21** (weights derived from public sources); this milestone keeps one agent per
group but makes the agent choice per candidate (**D71**), so MOD-36 plugs in without re-cutting the
engine.

- [x] **OQ-1 — `max_agents_per_run`'s reading refuses a judged 3-way `implement` in the seeded
      `feature` graph at the default cap of 6.** ANA-2 counts "every candidate plus every judge
      plus every retry attempt planned so far" and refuses "at admission naming the cap and the
      requested figure" (`docs/ANA-2.md:870-875`). Read literally at `StartRun`, the four-phase
      `feature` graph with `implement` at `fan_out = 3` and a judge plans `1 + 1 + 3 + 1 + 1 = 7`
      agents; a 4-way judged fan-out (`max_fan_out`'s own default, `:868`) plans 8. §10's reason for
      6 — "the maintainer's own standing cap on subagents per run" after a run spawned 56 agents
      (`:2050`) — reads like a *concurrency* ceiling. **Default adopted (D63):** the literal
      reading, first pass only: `Σ fan_out + (1 per judged phase)` over the snapshot, where a
      *judged phase* is one with `fan_out > 1` **and** `judge.is_some()` (a single-candidate phase
      never runs a judge, and `graph.rs:483-490` puts the project's judge on every phase), checked
      once at `StartRun`. **Retries are deliberately not counted — a deviation from `:871-872`'s
      "plus every retry attempt planned so far"**: they are bounded by `retry_limit`; counting
      them live would refuse M2's shipped `an_intermediate_position_is_retired_by_the_loop` (8
      steps); counting them as planned (`Σ fan_out × (retry_limit + 1)`) would refuse the seeded
      `feature` graph itself at the default (`4 × 2 = 8 > 6`). Criterion 8's case uses the two-phase `analysis` graph (`3 + 1 + 1 = 5`).
      **Alternative:** cap the widest position (`max over phases of fan_out + judge`), which never
      bites at the defaults; or raise the default in `0003`'s seed (a data change, not a migration).
      **Revisited at the review gate (maintainer, 2026-09-23): the default is raised to 8.** The
      literal reading stays; `0004_max_agents_per_run_default.sql` moves an untouched seeded `6` to
      `8` (a user-changed value is left alone — `0003` is on `main` and cannot be edited) and
      `graph.rs`'s built-in fallback follows, so the judged 3-way `feature` graph (7) runs by
      default and a judged 4-way `implement` (8) sits exactly at the cap. `docs/ANA-2.md:871`,
      `:1472`, `:1516` and `:1992` still say 6 and are not amended here.
- [x] **OQ-2 — A diff renderer is needed now, and `gix` 0.87.1 ships only its primitives.** The
      judge's candidate block (`docs/ANA-2.md:828`) and the loop's `previous_diff` (M3 D32 moved it
      here) both need a `--stat` plus a unified diff over `before_hash..after_hash`. `gix` has tree
      diffs (`Repository::diff_tree_to_tree`, `gix-0.87.1/src/repository/diff.rs:50`, gated on
      `blob-diff`, which `status` already enables — `src/repository/mod.rs:30`), aggregate stats
      (`object/tree/diff/mod.rs:160`) and a hunk renderer (`gix-diff-0.67.1/src/blob/unified_diff/
      impls.rs:11`), but no porcelain: file headers, mode lines, binary markers and the per-file
      `--stat` table would be written here. **Default adopted (D55):** a sixth `git` verb,
      read-only — `git diff --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/`
      once with `--stat` and once without — through M3's `Cli`, the same reasoning that made
      `reset --hard` the fifth verb (M3 D47: do not re-implement a `git` verb from primitives).
      **Alternative:** compose it in `gix` (~200 lines, no new verb). Either way ANA-2 `:1777`'s
      amended text ("exactly `worktree add --lock`, `worktree remove`, `merge --no-ff` and
      `reset --hard`") is **already one verb behind the shipped code** — `merge --abort` runs today
      (`isolate/git.rs:691-697`, `abort_merge`) — so the main thread's amendment adds two verbs,
      `merge --abort` and `diff`.
- [x] **OQ-3 — A judge failure parks the item at `awaiting_approval`, not `blocked`.** ANA-2 says
      the item "goes to `blocked`" (`:861`; §4.3 `:576`), but `Status::can_move_to` has no road
      from `blocked` back to `in_progress` (`crates/htui-core/src/model/item.rs:55`, carried R-4),
      so the human `SelectFanout` criterion 10 requires (`:2113-2114`) could never resume the item
      it would leave behind. **Default adopted (D50):** run and item both `awaiting_approval`, the
      reason in the judge's `gate_note` and an `item_note`. **Alternative:** `blocked` as written,
      with `SelectFanout` moving the item `blocked → open` and the run finishing — which contradicts
      criterion 10's "completes it".
- [x] **OQ-4 — The verdict is read from a `judge` document, so until MOD-11 lands every production
      judge fails and every fan-out is decided by a human.** ANA-2 has the verdict in "a document of
      kind `judge` … parsed from a fenced JSON block" (`:837-846`), and PRD D8 records that no agent
      writes a document before MOD-11 (`:366-370`). **Default adopted (D52):** the document, as
      written; the harness's `SessionSink` stands in (M2 D13) so criteria 8–10 are provable now.
      **Alternative:** parse the last fenced `json` block of the judge session's final
      `assistant_text` event, which works today and needs no document — a second source for one
      fact, and a divergence from ANA-5's "emitted by the agent inside the document" (`:1348`).
- [x] **OQ-5 — A `review` phase with `fan_out > 1` is refused at snapshot time.** A review
      candidate's settle can be `rejected` (`crates/htui-orch/src/gate.rs:240`), and there is no
      reading of "select the winning rejection, then loop" that ANA-2 defines; the loop retires by
      position and a rejected winner cannot reach the automatic loop's `awaiting_approval → failed`
      path from `done` (`model/run.rs:127`). No seeded graph fans out (`seed.rs:221`). **Default
      adopted (D64):** `ResolveError::ReviewFanOut`. **Alternative:** a review group's settle is
      the winner's verdict, applied after selection — a second loop entry point.
- [x] **OQ-6 — `min_budget_for_new_attempt` has no key.** ANA-2 names the rule (`:1615`,
      `:1421-1424`) and reserves no `app_setting` for it (§5.4 `:1509-1522`); PRD `:272` forbids a
      `0004`. **Default adopted (D60):** read an unseeded `app_setting.min_budget_for_new_attempt`
      (USD micros) when present, default `0`, skipping a candidate when `cap − spent < min` with
      both known — at `0` it is subsumed by (strictly weaker than) `available()`'s rule 4, which
      already skips at `spent >= cap` (`quota.rs:446-455`), so the default changes no verdict.
      **Alternative:** defer the third skip condition to MOD-12, which owns batch accounting
      (`docs/ANA-2.md:1828-1829`).
- [x] **OQ-7 — Every candidate of a group runs on the one agent the walk selects.** ANA-2 says
      "N candidate sessions … on the same prompt" (`:770-773`) and never says whether they spread
      across `phase_agent` rows. **Default adopted (D60):** same agent, same model, N sessions
      (rival sampling; ANA-5's "identical across siblings: required", `:1870`). **Alternative:**
      round-robin over the eligible list, which makes the comparison partly an agent comparison.
      **Answer (2026-09-23):** spread across agents by weight — follow-up **MOD-36**, weights from
      **ANA-21**. This milestone: same agent for every candidate, chosen per candidate through D71's
      seam.
- [x] **OQ-8 — Phase-level judge columns stay unread (blueprint H-14, carried).** `0003` added
      `step_graph_phase.judge_agent_id`/`judge_model` (`0003_orchestration.sql:17-22`) but
      `StepGraphPhase` carries neither (`model/kind.rs:177-211`), so the judge resolves from
      `project.settings.judge_agent_id` only and `judge_model` is always `None`
      (`crates/htui-orch/src/graph.rs:481-490`). **Default adopted:** not fixed here — adding the
      fields touches every phase query (`.sqlx`), MOD-15's phase editor and its snapshots, for a
      rung criteria 8–10 do not need. **Alternative:** add them now.

---

## Summary

Milestone 3 made one step work in a real tree; this milestone makes a phase with `fan_out = N`
produce N candidates and exactly one winner. Stage 1 gains `R-AGT-8`'s walk (`select.rs`): each
candidate row is tested against ANA-4's four quota rules and MOD-4's three skip conditions, the
survivors are chosen in priority order, a refusal says why every candidate was skipped, and the
empty-candidate chain gains its missing rung 3 and its `no_candidate_agent` + `blocked` +
`item_note` refusal. `htui-core`'s `available()` stops skipping `allowed_warning` (PRD D3), which is
what makes the walk usable on this box.

A fanned-out phase creates its candidates at `fanout_index 0..N-1` together, assembles their one
prompt once, and drives them concurrently through stages 2–5, each from the same base commit;
`shared_serialized` siblings serialise on M3's guard and reset to that base first. A candidate
lands `done` or `failed` on its own and never parks; its `verify_outcome` is recorded and *not*
applied to its settle, because the prefilter is the thing that reads it (D48). Selection then
routes (D49): the gate, the prefilter (`verify_outcome = 'fail'` eliminated when ≥ 2 pass; exactly
one passing wins outright), then the judge — a real `run_step` at `fanout_index = -1` running two
fresh sessions, the second with the candidates reversed, whose verdicts must agree — or a human.
Every outcome lands through `select_fanout`, milestone 1's one-transaction writer (unchanged), and
the winner is reconciled by M3's `reconcile_done_step`. A judge failure or a human route parks the
run with every candidate as it settled and `selected` NULL; the new `SelectFanout` command
completes it (criterion 10). The review loop learns that a position can hold a group (retire the
whole group; compare winners), which also closes R-8.

Nothing crosses into the seam: `select_fanout`, `answer_gate`, `transition_step`,
`upsert_step_tree`, `record_commits` and `add_note` already do every write this milestone needs.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D48 | **A fan-out candidate settles on its own terms and never parks: `gate::settle` with `verify_outcome` forced to `None` decides `done` (settle `ok`) or `failed`; the real outcome is written to `run_step.verify_outcome`/`verify_exit_code` and the `command_run` row exactly as M3 writes it.** A candidate error after `pending → running` fails *that candidate* (`running → failed` plus an `item_note`), never the run. The gate table (`gate::apply`) is not applied per candidate; it is applied once to the group (D49). | Criterion 9 needs a verify-failed candidate to survive as a *selectable* row when fewer than two pass (`docs/ANA-2.md:2109-2111`; SWE-agent "disabling the filter when fewer than two candidates qualify", `:792`), and `select_fanout` accepts only an `awaiting_approval \| done` winner (`crates/htui-core/src/store/traits.rs:821-824`). Under M2's settle a `fail` is `Settle::Failed` (`gate.rs:261-263`), which would make the prefilter unreachable. "Completed siblings survive a failed sibling" (`:1317-1319`) and criterion 10's "every candidate `done`" (`:2112-2113`) both require the per-candidate landing to be terminal-or-done without a park. **Deliberate deviation:** the same passage goes on "only the failed index is re-attempted, and selection then runs over whatever survived" (`:1318-1319`); this milestone does **not** re-attempt a failed candidate — it selects over the survivors only — because a re-attempt of index *i* must be `(position, attempt + 1, i)` under `UNIQUE (run_id, position, attempt, fanout_index)` and would put two attempts in one comparison (D65's argument); the passage sits in §4.9's recovery section, whose sweep is milestone 5's. Recorded under Risks. M2's `walk_step → fail_hard` path (`engine.rs:844-853`, `:1102-1127`) fails the run and is therefore not reused for candidates. |
| D49 | **Group routing, in this order.** `pool` = candidates `done`; `passing` = `pool` whose `verify_outcome` is not `fail` (`pass`, `unavailable` or `NULL`). (1) `pool` empty, or `passing` empty → **group failed**, and §4.2's `failed` cell applies to the group: `always`/`on_failure` park for a human (D50); `never` retries the whole group at `attempt + 1` while `may_attempt` holds (`status.rs:27-29`), else `finish_run(Failed, "no_surviving_candidate: <phase>")`. (2) `gate_effective = always` → human (D50). (3) no judge → human (D50). (4) `passing` has exactly one → that candidate wins, **no judge step is created**, `select_fanout(winner, "the only candidate whose verify_command did not fail")`. (5) otherwise the judge compares `passing` (D51–D53) — **unless the judge prompt's trimmer dropped a candidate** (D53), in which case the group goes to D50's human park with the reason `judge_candidate_dropped: <fanout_index>`. | §4.5's routing table (`:807-812`) with its prefilter row (`:792`) and criterion 9 (`:2109-2111`). "`on_failure` that tripped" (`:810`) is read as *the group failed*: a group with a survivor has not failed. Zero passing is treated as a failed group rather than "filter disabled, judge everything" because §4.2 makes `verify_outcome = 'fail'` a failed settle (`:439`) and a group of nothing but failures has no `ok` member; recorded under Risks as a reading. Exactly-one-passing auto-wins only on the judge route: with no judge the table says human (`:812`). |
| D50 | **One park shape for human selection: every candidate as it settled, `selected` NULL, run `running → awaiting_approval`, item `in_progress → awaiting_approval`, one `item_note` naming the phase, the attempt, each candidate's `fanout_index`/status/`verify_outcome`, and the reason (`gated`, `no judge configured`, `no_surviving_candidate`, or the judge failure).** No step is at `awaiting_approval` — the same deviation from §4.3's propagation rule (`:659-663`) that M3's `park_run` already makes (`engine.rs:1179-1221`). `run.failure` stays NULL (R-3). | Criterion 10 fixes the candidates at `done` with `selected` NULL (`:2112-2113`); parking the candidates instead would earn a second shape for the gated case and an `AnswerGate` that could approve a candidate without selecting it. The item goes to `awaiting_approval` and **not** `blocked` (OQ-3): `blocked → in_progress` does not exist (`item.rs:55`), and the unpark `SelectFanout` needs is `awaiting_approval → in_progress` (`item.rs:54`). |
| D51 | **The judge is created lazily, only on route (5) or when the judge agent is skipped, as `create_step { fanout_index: -1, phase_name: "<phase>:judge", agent_id: judge.agent_id, model: judge.model or agent.default_model or agent.models[0] }` at the group's `(position, attempt)`.** It runs ungated (no inline-approval interlock — it answers no permission), in no tree: `prepare(run, judge, &[], Isolation::Local, None)` yields a scratch `cwd` and no row (`isolate/real.rs:392-403`). Success: `select_fanout(run, position, attempt, winner, Some(reasons[winner]))`, which moves the judge `running → done` with `gate_note` in the same transaction (`mem.rs:3575-3586`). Failure: `transition_step(judge, running → awaiting_approval)` then `answer_gate(judge, Rejected, Some(reason))` → `failed` with `gate_note` — the exact two-write pattern `gate::apply` uses for an automatic review rejection (`gate.rs:403-423`) — then D50's park. A judge whose agent the walk skips (quota, probe, missing row) is created, moved `pending → running`, and failed the same way with `judge_unavailable: <reason>`. | ANA-2 `:814-820` (the row), `:858-862` (failure: `failed` with `gate_note`, candidates `done`, `selected` NULL, human picks), `:854` (success: `done` + `gate_note`). `select_fanout` was built for exactly this: it leaves a judge that "already reached an outcome … alone entirely" so the human's pick does not overwrite the failure reason (`traits.rs:808-813`). No writer moves a `running` step to `failed` with a `gate_note`: `select_fanout` writes `gate_note` only on the way to `done` (`mem.rs:3577-3583`), and `answer_gate` only from `awaiting_approval` (`traits.rs:788-801`) — so the reason has to go through `awaiting_approval` and `answer_gate`; its side effect, `gate_outcome = rejected` on the judge, reads accurately as "the orchestrator rejected the judge's verdict" and is recorded rather than hidden. `pending → failed` is illegal (`model/run.rs:113`), hence the `running` hop. The judge has no tree because it edits nothing; ANA-2 gives it none. |
| D52 | **The two orderings are two fresh driver sessions on the one judge step, under one `Recorder`: call 0's text is `record_prompt` (seq 0, `prompt_digest`, `set_step_prompt`), call 1's reversed text is `record_follow_up` (turn 1).** Each call's verdict is the newest `judge`-kind document the judge step produced after that call's `SessionSink::after_done`; a call that produced no new document is `judge_missing_document`. The verdict is the **last** fenced `json` block of the body, parsed with `serde_json` into `{ winner: i32, reasons: { "<i>": String } }`; failures, each a `fanout::JudgeFailure` with a byte-fixed `Display`: `judge_unparseable: <detail>`, `judge_out_of_range: <winner> not in [<survivors>]`, `judge_disagreement: forward <a>, reversed <b>`, `judge_missing_document: call <n>`, `judge_session_failed: <err>`, `judge_unavailable: <skip reason>`. Missing `reasons` entries are not a failure (§4.5 does not list them, `:845-846`); the winner's missing reason becomes `judge: <winner>`. | A second call inside one session would see its own first answer and could not test position bias (`:790`). ANA-5 persists a handoff's fresh session "as a `follow_up` event at the next turn, not a second `prompt` row", keeping `prompt_digest` at seq 0 (`docs/ANA-5.md:1294-1300`); the judge's second call is the same shape. `UNIQUE (run_id, position, attempt, fanout_index)` allows one judge row per group (`:816-817`), so two rows are not an option. "Last block" is ANA-5's instruction to the agent (`:1347-1348`; the seeded body says "end the document with exactly one fenced json block, and nothing after it", `crates/htui-core/src/prompt/defaults.rs:189-190`). |
| D53 | **The judge prompt is `assemble()` over `PromptSpec { role: Judge, template: <pinned judge template>, judge: Some(JudgeInputs { task, candidates, reverse }) … }`.** `task` is the `payload.text` of the seq-0 `prompt` event of the passing candidate with the lowest `fanout_index` (`ReadStore::step_events`, `traits.rs:80`). One `JudgeCandidate` per passing candidate: `fanout_index`; `verify` = `Some(true)` for `pass`, `Some(false)` for `fail`, `None` otherwise; `exit_code` = `verify_exit_code`; `document` = its output document; `diff` = `Isolator::diff` over its rows (D54); `verification_tail` = the last `command_run.output` (already tail-capped at 64 KiB by M3 D30). Budget from the judged phase (`settings::resolve_budget(phase.token_budget, …)`). **The template version is pinned in the snapshot: `SnapshotJudge` gains `#[serde(default)] template: Option<SnapshotTemplate>`**, resolved by `graph::resolve` as the latest `judge` row at run creation; `None` (an older snapshot) falls back to the latest at judge time. `GraphSnapshot::V` is **not** bumped. | ANA-2 `:822-835` and ANA-5 §4.6a (`:1248-1273`): `task` replayed, not re-assembled; budget from the judged phase; the per-candidate isolate cap and the diff-stat-only fallback are the assembler's (`Trimmer::trim_candidates`, `crates/htui-core/src/prompt/trim.rs:810-847`; `prompt/mod.rs:573-580` only sorts and reverses). **The trimmer can also drop a candidate outright** when even its stat-only form exceeds its share (`trim.rs:841-845`, whose comment reads "MOD-4 reads a dropped candidate as 'escalate to human selection'"). The engine honours that: after assembling call 0 it checks `trim_record` for a dropped `judge_candidate:<i>` section, and on one it does **not** run the judge — the group goes to D50's park with `judge_candidate_dropped: <i>`; the second call is assembled from the same inputs reversed, so it drops the same candidate. ANA-5's table pins the judge template "beside the judged phase" (`:1862`), and invariant 2 says a run reads its snapshot. `topology` hashes the typed phases (M2 D9, `graph.rs:182-192`): a `None` judge serialises as `null` either way, so `FEATURE_TOPOLOGY` (`graph.rs:590-591`) and every stored snapshot without a judge keep their digest; a stored snapshot *with* a judge would recompute differently on resume, and none exists outside tests because nothing starts a run in production until milestone 6. |
| D54 | **Four changes to the `Isolator` seam (`isolate.rs:106-153`).** (a) `prepare` gains `slot: Option<FanoutSlot<'a>>`, `FanoutSlot { index: i32, width: i32, base: &'a BTreeMap<RepoId, String> }`: every candidate of a group starts from `base`, not from the repo's current `HEAD`. (b) New `base(scope) -> BTreeMap<RepoId, String>`: the `HEAD` of each repo, read once per group by the engine when no candidate of the group has a `run_step_commit` row yet — and re-derived from those rows' `before_hash` when one has (M2 D16). (c) New `diff(trees, commits) -> Option<DiffBlock>` (`htui_core::prompt::DiffBlock`, `prompt/mod.rs:151-165`): `None` when no repo committed; one repo → its range, stat and patch; several → ranges joined `, ` as `<repo name>:<before>..<after>`, stats and patches concatenated in scope order under a `# repo <name>` line. (d) `reconcile` gains `siblings: &'a [StepId]`, the other candidates of the winner's group (empty for `fan_out = 1`). (e) The `worktree` mode's `git worktree add` is serialised per repository, slot or not (D70). `local` never sees a slot (`fan_out > 1` is refused at snapshot, `graph.rs:425-429`). | (a) ANA-2 `:1006-1007` ("each reset to `before_hash` first") and fairness: a comparison between candidates that started from different commits compares nothing. (b) The base cannot come from candidate 0's `prepare` because `shared_serialized`'s sibling 1 cannot prepare until sibling 0 captures (M3 blueprint A-1), so "prepare 0, then the rest" deadlocks under a concurrent drive (D58). (c) Two consumers: the judge's candidate block (`:828`) and the loop's `previous_diff` (`:731`, D67); `DiffBlock` is the type `assemble()` already takes. (d) D56 needs to know the checkout is where *a sibling* left it before it resets the user's branch. |
| D55 | **`diff` shells out to `git diff` (OQ-2): one method, `Cli::diff(repo, before, after, stat: bool)`, spawning `git diff --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/ [--stat] <before> <after> --` in the repo directory, with M3's environment and 120 s budget (`isolate/git.rs:296-310`, `:38`). The capture is **head-capped, not tail-capped**: `Cli::run`'s `TailBuffer` keeps the *last* 64 KiB (`git.rs:346`, `:59`), which would cut a long patch's first `diff --git`/`---`/`+++` headers and start the text mid-hunk, so `diff` uses a head buffer of the same size and appends `\n[diff truncated at 64 KiB]` when it overflows. The trailing `--` keeps a revision string from ever being read as a path.** Run in the tree's own repository for `copy` (its objects live in the copy until reconcile, M3 OQ-5) and in the checkout for the other modes (a worktree shares the checkout's odb). Not retried (reads are not, M3 D39). A missing `git` makes `diff` answer `Ok(None)` and the caller writes a trim note, never a refusal. | M3 D47's rule — do not re-implement a `git` verb from `gix` primitives — applied to the next verb the tree needs. `--no-ext-diff`/`--no-textconv` keep `diff.external` and textconv drivers out of an orchestrator path; explicit prefixes defeat `diff.noprefix`/`diff.mnemonicPrefix`. The judge's diff is advisory input, so its absence degrades the prompt rather than failing the selection. |
| D56 | **`shared_serialized` fan-out: siblings run one after another in the checkout, each reset to the group base first; the winner is reconciled by resetting the checked-out branch to the winner's label.** `prepare` with a slot takes M3's guard (`isolate/real.rs:297-317`), then: `HEAD == base` → proceed; clean and `HEAD != base` → `git reset --hard <base>` (M3 D47's verb); dirty → `Refused("dirty_tree_not_reset: <path>")`, which fails that candidate (D48). `reconcile(winner, …, siblings)`: `HEAD == label(winner)` → identity (M3's case); clean and `HEAD` equal to `base_ref` or to `label(s)` for some sibling `s` → `git reset --hard <label(winner)>` and the winner's `after_hash` is that label; else `primary_moved`. | ANA-2 `:916` and `:1005-1007`. M3 D29 recorded a dirty `shared_serialized` tree and forwarded `:916`'s refusal to "milestone 4's sibling reset" (`mod-4-orch-tree.plan.md:174`). The rule is `:916`'s own "Refused when" cell ("the tree is dirty at step start and the user has not accepted the reset"); no acceptance exists, so it refuses. A reset of a dirty tree would destroy either the user's work or a sibling's uncommitted work — risk 2 (`:2065`) is the analogous "never destroy uncommitted work" precedent, as M3 D27 used it. M3 D26 committed on the checked-out branch and labels `htui/<step_id>` at capture instead of switching branches, so "the winner's branch is checked out" (`:916`) becomes "the checked-out branch is moved to the winner's label"; the sibling check is what makes that move safe to take. The guard is already `RepoId`-ordered (`real.rs:295-299`), so multi-repo siblings cannot deadlock. |
| D57 | **`copy` fan-out measures once per candidate against `copy_max_total_bytes / width`: `copy::measure_within_cap(src, excludes, cap, copies)` refuses when `size × copies > cap` and names both figures.** Each candidate gets its own `<root>/<run>/<step>/<name>` copy, reset to the group base. `worktree` fan-out's candidates each run `git worktree add` against the same repository concurrently; that call is serialised per repository by D70. | ANA-2 `:930-932` ("refuses with a named size when N copies would exceed"); M3 D35 left the multiplier at one "this milestone" (`mod-4-orch-tree.plan.md:180`). |
| D58 | **The candidates of one group run concurrently inside one engine call, with `futures::future::join_all`; the prompt is assembled once per group.** `htui-orch` gains the workspace's `futures = "0.3"` (`Cargo.toml:36`). Each candidate has its own driver, recorder and verify request; the `verify` semaphore (M3 D30) and M3's guard serialise what must be serialised. The one `AssembledPrompt` is recorded on every candidate (`record_prompt` + `set_step_prompt`), so `prompt_digest` is identical across siblings. | `R-ORCH-7`: "runs them in parallel on the same prompt" (`:770-771`); ANA-5: identical prompts across siblings are "required" (`:1870`). Assembling per candidate would read `resolve_inputs` while a sibling is writing its own output document. `tokio::task::JoinSet` needs `'static` futures and the engine holds `&'a` borrows of its nine service parts (`engine.rs:170-212`), so `join_all` is the shape that compiles. In the fake harness it is also deterministic: `FakeDriver::next_event` never suspends (`crates/htui-agent/src/fake.rs:393-408`) and `MemStore` does its work in synchronous closures (`mem.rs:1-7`), so candidates complete in `fanout_index` order. |
| D59 | **The cursor becomes group-aware for `fan_out > 1` phases and is unchanged for `fan_out = 1`.** A group is the steps at `(position, latest attempt)` with `fanout_index >= 0`; the judge is the `-1` row of the same slot. New `Cursor` arms: `Fan { position, attempt }` — fewer than `fan_out` candidates exist, or one is `pending` (drive the group; missing indices are created first); `Select { position, attempt }` — every candidate settled, none `selected`. A `selected` `done` candidate completes the position; every candidate `superseded \| cancelled` yields `Create { attempt + 1 }`; a `running` candidate or judge rests. `latest_at`/`next_attempt` keep reading `fanout_index 0` (`status.rs:108-122`), which every group has. | M2 plan D16: all of this is re-derived from `run_steps` on every pass. Today's cursor would read a loser at index 0 as `superseded` and create a new attempt (`status.rs:153-157`), and would never see a winner at index 2. A `Select` arm (rather than selecting inside `Fan`) is what lets a crash between the last candidate and the judge resume at selection, and lets a parked run whose judge `failed` re-park rather than re-judge. |
| D60 | **`select.rs` is `R-AGT-8`'s walk, pure over rows: `walk(SelectInput) -> Walk { eligible, skipped }`, first match per candidate: (1) no `agent` row → `NoAgentRow`; (2) `quota::available(box.quota, spent, cap)` skips → `Quota(SkipReason)` (ANA-4's four rules, `quota.rs:423-456`); (3) `gate_effective != Never` and neither `permission_requests` nor `edit_proposals` in `registry::caps_for` → `InlineApproval`; (4) `agent.enabled = false`, `agent_box.enabled = false`, or a probe that parses with a status other than `ready` → `NotReady(<why>)`; (5) `cap − spent < min_budget_for_new_attempt` with both known → `Budget { remaining, min }` (OQ-6).** An absent `agent_box` row or an absent/unparseable probe is *unknown* and does not skip. `spent` is `Σ run_step.usage->cost_micros` over the run — a key inside the `usage` JSON (`UsageTotals.cost_micros`, `model/usage.rs:37`), not a typed `RunStep` field (`model/run.rs:213-260`); `cap` is `snapshot.settings.per_token_cap_run`; batch is MOD-12's. The engine hands `eligible` to the existing `AgentSelector` (`engine.rs:82-106`), so `FirstCandidate` is "the first in priority order" and every candidate of a group is that one (OQ-7). A selection that skipped a higher-priority candidate writes an `item_note` naming the skipped row, the reason and the chosen one. | ANA-2 §7 (`:1603-1615`) and `R-AGT-8` (`REQUIREMENTS.md:164-165`). "Unknown is available" is §7's own rule for quota (`:1605-1607`) extended to rows never probed: the demo fixture seeds no `agent_box` row at all (`mem.rs:109`, `:213`), and treating "never probed" as "missing" would refuse every shipped conformance case. (A *probed* `cli` row is not the reason: its probe reports `status: ready` with no handshake, `crates/htui-agent/src/probe.rs:1444-1459`, so rule (4) never skips it.) The note is `R-ORCH-10`'s "never substitute silently" applied to agent choice (`:1355-1357`). The walk reads `agent_box` through a new `GraphSource::agent_boxes(box)` (D62), not a map in `EngineParts`, because the recorder latches quota mid-run (`agent.rs:79-85`) and a map built at engine construction would be stale by the next step. |
| D61 | **`allowed_warning` is selectable (PRD D3).** `quota.rs`'s one permissive string becomes a two-string set, `["allowed", "allowed_warning"]`; `exhausted`, `utilization >= 1.0`, every other status and the cap rule are untouched. The pinning test is inverted in the same commit. | PRD D3 (`:343-349`), which reserved the call for MOD-4 "knowingly rather than by accident" as the doc itself asks (`quota.rs:409-414`). HANDOFF's MOD-4 entry records the box's live `claude-cli` row at `allowed_warning` (`HANDOFF.md:178-180`). |
| D62 | **The empty-candidate chain is completed: rung 3 in `graph::resolve`, rung 4 as `no_candidate_agent`.** `GraphSource` gains `agent_boxes(box) -> Vec<AgentBox>` and `resolve` gains `box_id: BoxId`; rung 3 is "exactly one `agent` with `enabled` whose `agent_box` on this box is `enabled`", modelled on `agent.default_model` else `agents.models[0]`. A rung-4 `ResolveError::NoCandidate` at `StartRun` moves the item `open → blocked` and writes `item_note("no_candidate_agent: phase `<p>`")` before the error is returned (a `failed` item has no `blocked` edge, `item.rs:56`: note only). A stage-1 walk that leaves nothing eligible refuses as `refuse_capability` does today (`engine.rs:771-808`, blueprint H-16's order: item `in_progress → blocked`, note, `finish_run(Failed)`), with `RunFailure::MissingCapability` when every skip was `InlineApproval` (the shipped sentence and case) and `RunFailure::NoCandidateAgent { phase, detail }` → `no_candidate_agent: phase `<p>`; <agent> (<reason>), …` otherwise. | ANA-2 `:1617-1624` and PRD scope `:238-240`. M2 deferred rung 3 into `GraphSource::phase_agents` implementations (`graph.rs:494-501`) because `GraphSource` had no box listing — and none exists in production yet; only the fakes stand in (`fake.rs:534-543`). The stage-1 walk needs that listing anyway (D60), so rung 3 moves where §4.1 puts it, in `candidates` (`graph.rs:502-556`) behind `resolve`'s new `box_id` (`graph.rs:233`). On the demo fixture rung 3 still yields nothing (no `agent_box` rows, three enabled agents), so `FakeGraphSource`'s stand-in candidates remain necessary. `ResolveError::NoCandidate` today writes nothing (`graph.rs:122-128`), which leaves a refused item invisible — the failure invariant 7 exists to prevent. |
| D63 | **The two fan-out caps are refused at `StartRun`, before a run row exists, naming the cap and the figure: `ResolveError::FanOutCap { phase, fan_out, max }` when any phase exceeds `snapshot.settings.max_fan_out`, and `ResolveError::AgentCap { planned, max }` when `Σ fan_out + judged phases` exceeds `max_agents_per_run`, a *judged phase* being `fan_out > 1 && judge.is_some()` (OQ-1). Retry attempts are not counted — a recorded deviation from `:871-872` (OQ-1 gives the three reasons).** | ANA-2 `:868-876` ("refused at admission … never silently truncated"). Both caps are already resolved into `SnapshotSettings` (`graph.rs:296-298`, `model/run.rs:566-569`) and read by nothing. |
| D64 | **`fan_out > 1` on the `review` phase is refused at snapshot time (`ResolveError::ReviewFanOut`) (OQ-5).** | See OQ-5. Seeded graphs are unaffected (`seed.rs:221`). |
| D65 | **`Command::SelectFanout { run, position, attempt, winner }` lands**, guarded by `command::select_enabled(run, group, winner)`: the run is `awaiting_approval`; the slot holds more than one candidate; none is `selected`; `winner` is a candidate of that slot at `done \| awaiting_approval`. The engine writes `select_fanout(…, Some("selected by a human"))`, unparks (M2's `unpark`, `engine.rs:1507-1519`), reconciles the winner with its siblings (`reconcile_done_step`, M3 D25), and walks on; `CommandOutcome::Selected { rest }`. **`RetryStep` on any member of a parked fan-out slot retries the group**: retire every member (D66), then stage 1 admits `attempt + 1` for the whole group under `may_attempt`. | ANA-2 §6.2 (`:1566`) and criterion 10 (`:2113-2114`). `command.rs:28-30` reserves `SelectFanout` for milestones 4–6. A single-candidate retry inside a group would create `(position, attempt + 1, i)` beside `(position, attempt, j)` — two attempts in one comparison. |
| D66 | **The review loop over a fanned-out implement retires the whole slot and compares winners.** `gate::retire` supersedes or cancels every step at `(position, latest attempt)`, judge included, by M2 D5's status split (`gate.rs:854-890`); `no_progress`'s `at()` (`gate.rs:842-851`) becomes `status::winner_at`, the `selected` step of the slot, else its `fanout_index 0` when nothing in the slot carries `selected`. **R-8 is closed with no predicate change**: the comparison reads the winner's post-reconcile `after_hash`, and two consecutive implement attempts can only agree on it when both are `NULL` — attempt `a + 1` branches from a primary `HEAD` that already contains attempt `a`'s merge (`:948-951`), so any commit it makes is a descendant of both of attempt `a`'s hashes and differs from each, pre- or post-merge. | ANA-2 `:754-758` ("the previous attempt's winner and losers are all `superseded`. The judge runs again"). R-8 (`HANDOFF.md:274-275`, M3 blueprint H-19) asked whether the pre-merge hash is needed; the ancestry argument shows it would change no answer, and reconcile keeps a `NULL` `after_hash` `NULL` (`isolate/real.rs:797-799`). A real-git case pins criterion 7 in `worktree` mode (T8). |
| D67 | **The loop's `verify_failure` and `previous_diff` sections are wired (M3 D32's forward).** For a `Phase`-role step at `attempt > 1`: `previous = winner_at(position, attempt − 1)`; `verify_failure = Some(VerifyFailure { exit_code, output })` when `previous.verify_outcome = fail`, from its last `command_run` row; `previous_diff = Isolator::diff(step_trees(previous), step_commits(previous))`. The `engine.rs:1376-1380` comment is replaced. | ANA-2 `:730-731`, ANA-5 `:1275-1280`, M3 D32 (`mod-4-orch-tree.plan.md:177`). The diff seam lands in this milestone for the judge anyway (D54), and `assemble_prompt` is edited here; after a reconcile, `before..merge` has the same patch as `before..after` because reconcile refuses a moved primary (`real.rs:808-815`). |
| D68 | **`DriverFor` and `SessionSink::after_done` gain a `SessionKey { phase: &str, attempt: i32, fanout_index: i32, call: u32 }`.** `DriverFor` becomes `Fn(&SnapshotCandidate, &SessionKey<'_>)`, `after_done(item, step, phase, key, done)`. `FakeOrchestrator` scripts by `(phase, attempt, fanout_index, call)` with fallback to `(phase, attempt)` (every shipped script keeps working). | Three candidates share `(phase, attempt)` and the judge's two calls share `(phase, attempt, -1)`; criteria 8–10 need each to play a different script and write a different document, and `FakeDriver` refuses a second `start` (`htui-agent/src/fake.rs:178-180`). Today's key is `(phase, attempt)` (`engine.rs:160`, `fake.rs:922-935`). |
| D69 | **`MemStore::set_project_settings(project, Value)`, a tests-only inherent writer beside `set_app_setting` (`mem.rs:434-443`).** | The judge resolves from `project.settings.judge_agent_id` (`graph.rs:483-490`) and no seam writer reaches that column (`update_project` never touches it, M2 blueprint F-F; `graph.rs:835-837`); the conformance cases build on `FakeOrchestrator::demo()` and cannot rebuild the store. Same precedent, same doc sentence. |
| D70 | **`git worktree add` is serialised per repository inside `GixIsolator`: a second lock table, `adds: Mutex<BTreeMap<RepoId, Arc<tokio::sync::Mutex<()>>>>`, and every `add_worktree`/`add_worktree_on_branch` call in `worktree_at` (`isolate/real.rs:492-528`) runs under that repository's add lock, held for the one `git` child and nothing else.** No retry classification is added for the race. The lock is keyed by repository, independent of D43's `(box, repo)` step guard (which `worktree` mode never takes), and is never held across another lock or an agent session, so it cannot take part in a deadlock. Candidates still prepare concurrently; only the `add` children queue, each for well under a second on this box. | **The fact-check's probe on git 2.43.0** (`LC_ALL=C`, three backgrounded `git worktree add --lock --reason … -b htui/<x> <path> <base>` per round): 720 adds, 2 failures, both `fatal: failed to read .git/worktrees/<id>/commondir: Success` (exit 128, ≈ 0.4 % per add, ≈ 1 % per 3-way group), **no ref-lock collision at all**. M3's classifier matches only the `.lock': File exists` and `Unable to write index` wordings (`isolate/git.rs:844-851`), so `with_retry` would not retry it; and the failed add had **already created `htui/<step>`** with no worktree, so a blind `-b` retry would fail with "a branch named … already exists" (M3 D23). Two ways out: serialise, or classify the `commondir` wording and retry through `add_worktree_on_branch` (`git.rs:492`) after checking the stranded branch points at the base. Serialising removes the race rather than recovering from it, needs no stderr wording that git may change, and costs sub-second queueing on a path that then runs an agent for minutes. **Residual:** another process's own `git worktree add` on the same repository can still race ours; that is the one case left, and it fails the candidate readably (D48). |
| D71 | **`AgentSelector::select` gains `fanout_index: i32` and is called once per candidate (`-1` never: the judge's agent is the project judge, D51).** `select(phase, eligible, fanout_index)`; `FirstCandidate` ignores the index, so every candidate of a group still runs on the first eligible agent (OQ-7's default behaviour). Each candidate's `run_step.agent_id`/`model`, driver and session key come from its own selection, never from a group-level variable; D60's substitution note is written per candidate whose choice skipped a higher-priority row. A `fan_out = 1` phase calls it with `0`. | Maintainer's OQ-7 answer (2026-09-23): candidates must eventually spread across agents by per-model weight (MOD-36, weights from ANA-21). Threading the choice per candidate now costs one argument and keeps the engine from baking in "one agent per group"; MOD-36 then adds a weighted selector behind the unchanged seam. The prompt stays assembled once per group (D58); mixed-family budgeting is MOD-36's open point, recorded on its HANDOFF line. |
| D72 | **(blueprint A-1)** With a slot, a `shared_serialized` `prepare` checks dirtiness **first**: a dirty checkout is `Refused(dirty_tree_not_reset: <path>)` for every sibling, including sibling 0 at `HEAD == base`; otherwise `HEAD == base` proceeds and `HEAD != base` resets to base. A no-slot `prepare` keeps M3's `dirty = true` recording. Tightens D56. | D56's order let sibling 0 run on the user's uncommitted work while siblings 1..n were refused; ANA-2 `:916` refuses a dirty tree the user has not accepted resetting. Maintainer-accepted 2026-09-23. |
| D73 | **(blueprint A-2)** D70's per-repository lock also wraps every `git worktree remove` (capture-time removal, `worktree_at`'s remake, cleanup); named `admin` in code. | `remove` mutates `.git/worktrees/` too and its race with a sibling's `add` was never probed; same lock, one child held, same deadlock argument. Maintainer-accepted 2026-09-23. |
| D74 | **(blueprint A-3)** `Isolator::diff` chooses, per row, the repository holding `after`: `copy` tries the copy then falls back to the checkout (new `gix` read `git::has_commit`, not a verb); other modes use the checkout. | After reconcile a `copy` winner's `after_hash` is the merge commit, present only in the primary; D67's `previous_diff` would fail with `bad revision`. Maintainer-accepted 2026-09-23. |
| D75 | **(blueprint A-4)** `Cli::diff` sets `COLUMNS=80`; argv unchanged from D55. | `git diff --stat` honours an inherited `COLUMNS` even when piped (confirmed on this box), which would make the judge prompt and its digest depend on the launching terminal. Maintainer-accepted 2026-09-23. |
| D76 | **(blueprint A-5)** A missing required input found while assembling a group's prompt (before any candidate is live) moves every pending candidate `pending → running → failed` with an `item_note`, then `finish_run(Failed, MissingInput(kind))` and `cleanup_run`. | Same run outcome as `fan_out = 1`; M2's `fail_before_a_token` needs a running step, which a group does not yet have. Maintainer-accepted 2026-09-23. |
| D77 | **(blueprint A-6)** The auto-win (D49(4)) and a human pick (D65) also write one `item_note` carrying the reason, `via_step_id = winner`; the reason is still passed to `select_fanout`. | `select_fanout` writes `reason` only onto a live judge row; with no judge, or a failed one, the reason would be dropped silently (invariant 7). Maintainer-accepted 2026-09-23. |
| D78 | **(blueprint A-7)** A candidate that fails after `prepare` and before `capture` calls `isolator.capture` best-effort on its failure path (commits recorded if `Ok`, `Err` logged); T8 gains `a_failed_shared_sibling_releases_the_checkout_for_the_next`. | `shared_serialized` guards release only at `capture`, a `prepare` error or `cleanup(run)`; D48 removed the `fail_hard → cleanup_run` path, so a failed sibling would deadlock the rest of the group inside `join_all`. Maintainer-accepted 2026-09-23. |

## What this milestone touches from the carried list

| Carried | Disposition |
|---|---|
| **R-3** (`run.failure` NULL on a parked run) | **Touched, not resolved.** D50's two new parks follow the same rule; the reason is the `item_note` (and the judge's `gate_note`). Still milestone 5's. |
| **R-4** (a `blocked` item cannot resume) | **Avoided, not resolved.** D50 parks a judge failure at `awaiting_approval` for exactly this reason (OQ-3). D62's refusals still write `blocked`, which only milestone 6's `Unblock` clears. |
| **R-5** (composite park writer; `gate_outcome = skipped`) | **Touched, not resolved.** D50's park is two compare-and-sets plus a note, in M2 H-10's order. Every *selection* write is `select_fanout`, one transaction. Candidates carry no `gate_outcome` (H-9 unchanged). |
| **R-6** (`phase_agent` copy, `is_override`) | **Untouched.** |
| **R-7** (reconcile-refusal park has no resume verb) | **Unchanged; milestone 5 or 6.** `SelectFanout` resumes a *selection* park only; a winner whose reconcile refuses after `SelectFanout` lands in `park_run` and is R-7 again. Stated in `select_enabled`'s doc. |
| **R-8** (`after_hash` is the merge commit) | **Closed** by D66's argument; pinned by T8's real-git criterion-7 case. |
| **Quota `allowed_warning`** | **Loosened here** (D61, PRD D3). |
| **MOD-2 F-104** (`excerpt::select`'s `vetted()`) | **Out, by PRD D4** (`:350-354`): nothing registers a provider (`crates/htui-core/src/prompt/excerpt.rs:1327-1328`), and the engine passes an empty `ExcerptSet` (`engine.rs:1363-1374`). The note stays on MOD-4's line. |
| **M3's guard ordering and lifetime** | **Relied on, unchanged** (D56, D58): siblings serialise prepare-to-capture (M3 blueprint A-1), guards are taken in `RepoId` order (`real.rs:295-299`), and `cleanup(run)` still releases every guard of the run (`real.rs:1003-1006`). |
| **M3 D32** (`verify_failure`/`previous_diff`) | **Done here** (D67). |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A running step failed with a readable reason in `gate_note` | `transition_step(running → awaiting_approval)` + `answer_gate(Rejected, Some(note))` | `crates/htui-orch/src/gate.rs:403-423` |
| Park with the step left alone, reason on the item | `park_run`: run, item, note | `crates/htui-orch/src/engine.rs:1188-1221` |
| Stage-1 refusal in blueprint H-16's order | `refuse_capability` | `crates/htui-orch/src/engine.rs:771-808` |
| One-way typed failure vocabulary | `RunFailure` + byte-exact `Display` test | `crates/htui-orch/src/status.rs:38-74`, `:195-230` |
| Pure predicate returning a verdict, not a `bool` | `Availability` / `SkipReason` | `crates/htui-core/src/model/quota.rs:352-378` |
| Enabling guard as a free function over rows | `answer_gate_enabled`, `retry_enabled`, `cancel_enabled` | `crates/htui-orch/src/command.rs:281-365` |
| Retiring a chain status by status | `gate::retire` | `crates/htui-orch/src/gate.rs:854-890` |
| A `git` verb through the wrapper | `Cli::reset_hard` | `crates/htui-orch/src/isolate/git.rs:564` |
| `gix` read under the blocking pool | `branch_target`, `head` via `blocking` | `crates/htui-orch/src/isolate/git.rs:1049`, `:1188`, `:762` |
| Git-backed test that skips without `git` | `let Some(git) = skip_without_git!() else { return; };` over `testkit::usable_git`; `SKIP_GIT` is the sentence | `crates/htui-orch/src/isolate/git.rs:1463`, `:1441`, `:1426` |
| Tests-only store writer | `MemStore::set_app_setting` | `crates/htui-core/src/store/mem.rs:434-443` |
| Conformance case + two count pins | `CASES` + `cases_are_unique_and_eighteen` + `cases_len_is_eighteen` | `crates/htui-orch/src/conformance.rs:153`, `:1964-1976`; `crates/htui-orch/tests/fake_conformance.rs:14-17` |
| Editing a live graph in a case | `repoint` (clone via `create_phase`) | `crates/htui-orch/src/conformance.rs:334` |
| Real-git end-to-end over `EngineParts` | `tests/gix_isolator.rs`'s `dispatch` + `CommittingSink` | `crates/htui-orch/tests/gix_isolator.rs:136-176`, `:358-380` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/quota.rs` | edit | T1 | D61 |
| `crates/htui-orch/src/select.rs` | create | T2 | D60's walk, `SkipCause`, `Walk`, `run_spend` |
| `crates/htui-orch/src/fanout.rs` | create | T2 | D49's `prefilter`/`route`, D52's `parse_judge_verdict`/`JudgeFailure`, D53's `judge_phase`/`judge_inputs` |
| `crates/htui-orch/src/lib.rs` | edit | T2, T6, T7 | `pub mod select; pub mod fanout;` and the crate doc's "not created" sentence (T2); re-exports (T6, T7) |
| `crates/htui-orch/src/isolate.rs` | edit | T3 | D54: `FanoutSlot`, `prepare` slot, `base`, `diff`, `reconcile` siblings; the reconcile doc's "Fan-out is milestone 4's" sentence (`:138`) |
| `crates/htui-orch/src/isolate/real.rs` | edit | T3 | D54, D56, D57, D70 per mode; ~45 test call sites gain the new arguments |
| `crates/htui-orch/src/isolate/git.rs` | edit | T3 | D55: `Cli::diff(repo, before, after, stat)` with a head-capped capture; the "five verbs" / "never parsed" docs (`:11-15`, `:33`, `:156`, `:789-790`) |
| `crates/htui-orch/src/isolate/copy.rs` | edit | T3 | D57: `measure_within_cap(…, copies)` |
| `crates/htui-orch/src/fake.rs` | edit | T3, T6, T7 | `FakeIsolator` slot/base/diff (T3); `GraphSource::agent_boxes` impls, `SessionKey` scripting (T6); group helpers if any (T7) |
| `crates/htui-orch/src/engine.rs` | edit | T3, T6, T7 | the two isolator call sites' arity only, `:871` and `:1157` (T3); stage-1 walk, D62 refusals, D67, D68 (T6); D48–D53, D58, D59, D65 (T7) |
| `crates/htui-core/src/model/run.rs` | edit | T4 | D53: `SnapshotJudge.template` |
| `crates/htui-core/src/store/mem.rs` | edit | T4 | D69 |
| `crates/htui-orch/src/graph.rs` | edit | T4, T6 | D53 pin, D63, D64 (T4); D62's `agent_boxes` + rung 3 + `resolve(box_id)` (T6) |
| `crates/htui-orch/src/status.rs` | edit | T5, T7 | `RunFailure::{NoCandidateAgent, NoSurvivingCandidate}`, `winner_at`, `group_at`, `judge_at` (T5); `Cursor::{Fan, Select}` and the group-aware `cursor` (T7) |
| `crates/htui-orch/src/gate.rs` | edit | T5 | D66 |
| `crates/htui-orch/src/command.rs` | edit | T7 | D65 |
| `crates/htui-orch/Cargo.toml` | edit | T7 | D58: `futures = { workspace = true }` |
| `Cargo.lock` | regenerate | T7 | the new `futures` edge of `htui-orch` |
| `crates/htui-orch/src/conformance.rs` | edit | T6, T7 | 5 cases (T6), 12 cases (T7); `CASES` 18 → 23 → 35 and its in-file pin |
| `crates/htui-orch/tests/fake_conformance.rs` | edit | T6, T7 | the out-of-crate pin |
| `crates/htui-orch/tests/gix_isolator.rs` | edit | T6, T8 | D68's signatures (T6); real-git fan-out, D70's race and R-8 (T8) |
| `crates/htui-orch/tests/fixtures.rs` | edit | T6 | `graph::resolve` gains `box_id` at its two call sites (`:15`, `:107`) |
| `crates/htui-orch/tests/fixtures/feature*.snapshot.json` | regenerate *if needed* | T6 | only if rung 3 changes the demo candidates; it should not (D62: the demo has no `agent_box` rows), and T6 asserts the two files are byte-unchanged |

**Not touched, on purpose:** every `WriteStore`/`ReadStore` method (no seam change; `select_fanout`
already does §4.5's bookkeeping, `traits.rs:803-832`); every migration and `.sqlx` file; every
`.snap`; `crates/htui-store/**`; `crates/htui-agent/**` (the probe and caps are read, not changed);
`crates/htui/**` (milestone 6 wires `SelectFanout` into the Runs tab); the prompt assembler
(`crates/htui-core/src/prompt/**`, so its judge snapshots are untouched); `overlap.rs`,
`recover.rs` and `queue.rs` are not created (they do not exist yet and are milestones 5 and
MOD-12's); `docs/ANA-2.md` (OQ-2's `:1777` word and OQ-3's `:861` deviation are the main thread's
to record).

## Tasks

**Wave A: T1 ∥ T2 ∥ T3 ∥ T4 ∥ T5. Then T6, then T7, then T8.** Independence is decided by
intersecting the file sets, not by prose:

| Task | Files (complete list) |
|---|---|
| T1 | `crates/htui-core/src/model/quota.rs` |
| T2 | `crates/htui-orch/src/select.rs`, `crates/htui-orch/src/fanout.rs`, `crates/htui-orch/src/lib.rs` |
| T3 | `crates/htui-orch/src/isolate.rs`, `crates/htui-orch/src/isolate/real.rs`, `crates/htui-orch/src/isolate/git.rs`, `crates/htui-orch/src/isolate/copy.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/engine.rs` |
| T4 | `crates/htui-core/src/model/run.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-orch/src/graph.rs` |
| T5 | `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/gate.rs` |
| T6 | `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/graph.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/lib.rs`, `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs`, `crates/htui-orch/tests/gix_isolator.rs`, `crates/htui-orch/tests/fixtures.rs` (and, only if they move, `crates/htui-orch/tests/fixtures/feature*.snapshot.json`) |
| T7 | `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/command.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/lib.rs`, `crates/htui-orch/Cargo.toml`, `Cargo.lock`, `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs` |
| T8 | `crates/htui-orch/tests/gix_isolator.rs` |

The five Wave A sets are pairwise disjoint (re-checked after the fact-check amendments: D70
lands in `real.rs`, T3's; the `git.rs` doc edits are T3's; `tests/fixtures.rs` is T6's; nothing
moved into a second Wave A set). **Hidden coupling, checked:** no task in Wave A changes a query
(no `.sqlx`), a seed, an insta snapshot (grep of every `.snap` for `max_agents_per_run`,
`topology` or `judge` finds unrelated `topology` prose in three `crates/htui` snapshots and six
`htui-core` judge prompt-render snapshots, `prompt_render__section_judge_*` and
`prompt_golden__prompt_judge_three_candidates`, which no task touches because the assembler is
unchanged), a `CASES` list or pin (only T6 and T7 do, serially), a `mod.rs`/`lib.rs` registration
other than T2's, `tests/fixtures/*.snapshot.json` (only T6, and only if rung 3 moves the demo
candidates), or `Cargo.lock` (only T7). **Compile coupling, checked:** every Wave A commit must leave the
workspace compiling, because the five tasks share one tree — so T5 adds `RunFailure` variants
(constructed, never matched outside `Display`) and new helpers but **no `Cursor` variant** (that
would break `engine.rs`'s matches at `:626-656` and `:1534-1541`, which T3 is editing); T4 changes
`graph::resolve`'s *body* but not its signature (callers: `engine.rs:322`, `:681`, `:3658`;
`fake.rs:1120`; `tests/fixtures.rs:15`, `:107`; `graph.rs`'s tests); T3 changes
the `Isolator` signatures and updates both implementors, `engine.rs`'s two call sites (`:871`
`prepare`, `:1157` `reconcile`), `fake.rs`'s three test call sites (`:1165`, `:1190`, `:1205`) and
`real.rs`'s ~45 test call sites itself — no caller exists in `tests/` or another crate;
T2's `fanout.rs` builds `SnapshotPhase` by struct update (`..phase.clone()`) and takes the judge
template as a parameter, so T4's new `SnapshotJudge` field cannot break it; `quota::available`'s
signature is unchanged by T1. T6 → T7 → T8 are serial because each shares `engine.rs` or
`gix_isolator.rs` with the next and compiles against the previous one's types.

Every implementer prompt carries: PRD D1–D8 win over this plan where they disagree; graphify-first
for `htui-core` questions, **direct reads for `htui-orch`** (the graph predates the crate), every
graph-derived fact re-verified against the tree; no new migration, no seam method, no `.sqlx`
change; nothing sets `updated_at` by hand; the only `git` subprocesses in `src/` are
`isolate/git.rs`'s `Cli`, now six verbs (OQ-2); **commit incrementally** — uncommitted subagent
work does not survive the session, and Wave A shares one tree (no stash); verify your gate on the
real tree with `--test-threads=1`.

### Task 1: `htui-core` — `allowed_warning` is selectable (D61)
- **Files**: `crates/htui-core/src/model/quota.rs`.
- **Test first**: invert `available_skips_an_allowed_warning_status` (`quota.rs:1022-1040`) into
  `available_selects_an_allowed_warning_status` (same document, `Availability::Available`); add
  `available_still_skips_every_other_non_allowed_status` (`rejected`, `allowed_critical`, `""` →
  `Skip(Status(..))`) and `an_allowed_warning_with_a_full_window_still_skips` (rule 2 answers
  first). The first fails for the stated reason.
- **Action**: `ALLOWED` (`:237-239`) becomes `const SELECTABLE: [&str; 2] = ["allowed",
  "allowed_warning"]`, rule 3 tests membership (`:439-443`); rewrite the `# allowed_warning` and
  `# No caller yet` doc sections (`:409-421`) to name PRD D3 and `select.rs`; `SkipReason::Status`'s
  doc (`:369`).
- **Validate**: `cargo test -p htui-core --all-features model::quota`; `cargo test -p htui
  --all-features --test settings -- --test-threads=1` (its `quota::available` assertion at
  `crates/htui/tests/settings.rs:763` uses `"allowed"` and must stay green); `cargo clippy -p
  htui-core --all-targets --all-features -- -D warnings`.

### Task 2: `htui-orch` — `select.rs` and `fanout.rs`, pure (D49, D52, D53, D60)
- **Files**: `crates/htui-orch/src/select.rs` (new), `crates/htui-orch/src/fanout.rs` (new),
  `crates/htui-orch/src/lib.rs`.
- **Test first** (unit tests in each file, no store, no engine):
  `select.rs` — `the_first_unskipped_candidate_is_eligible_first`,
  `an_exhausted_quota_skips_and_names_the_rule`, `a_missing_agent_box_row_is_unknown_and_selected`,
  `a_probe_that_is_not_ready_skips`, `a_disabled_agent_or_agent_box_skips`,
  `a_cli_row_is_skipped_only_at_a_gated_phase`, `the_per_run_cap_is_the_callers_spend`,
  `min_budget_skips_only_when_cap_and_spend_are_known`,
  `every_skip_being_inline_approval_is_reported_as_such`, `run_spend_sums_cost_micros`.
  `fanout.rs` — `parse_judge_verdict_reads_the_last_fenced_json_block`,
  `a_prose_only_body_is_unparseable`, `a_non_integer_winner_is_unparseable`,
  `judge_failure_display_is_byte_exact` (every variant of D52),
  `prefilter_eliminates_fail_only_when_two_pass`, `one_passing_candidate_routes_to_auto_win`,
  `zero_passing_is_a_failed_group`, `route_follows_ana2_s_table` (all rows of `:807-812` × gate ×
  judge presence, D49's order), `judge_inputs_reverse_only_the_order`,
  `judge_phase_is_named_phase_colon_judge_and_outputs_judge`.
- **Action**: `select.rs` — `SelectInput<'a> { candidates: &'a [SnapshotCandidate], agents:
  &'a BTreeMap<AgentId, Agent>, boxes: &'a BTreeMap<AgentId, AgentBox>, gate_effective: Gate,
  spent_micros: Option<i64>, cap_micros: Option<i64>, min_budget_micros: i64 }`, `enum SkipCause
  { NoAgentRow, Quota(SkipReason), InlineApproval, NotReady(String), Budget { remaining: i64, min:
  i64 } }` with `Display`, `Skipped { agent_id, agent_name, cause }`, `Walk { eligible, skipped }`
  with `only_inline_approval()` and `summary()`, `pub fn walk`, `pub fn run_spend(&[RunStep]) ->
  Option<i64>` — reading `usage->cost_micros` out of each step's `usage: Option<Value>`
  (`Value::get("cost_micros")`; `RunStep` carries no typed cost field, `model/run.rs:247`),
  `None` when no step reports one. Probe via `htui_agent::probe::ProbeSnapshot::from_row` (`probe.rs:1123`), caps via
  `htui_agent::registry::caps_for` (`registry.rs:152`), quota via `quota::available`
  (`quota.rs:423`). `fanout.rs` — `JUDGE_KIND = "judge"`, `judge_phase_name(&str)`,
  `JudgeVerdict`, **`parse_judge_verdict`** (not `parse_verdict`: `gate::parse_verdict` and
  `gate::Verdict` are already crate-root re-exports, `lib.rs:36-38`, and T6/T7 re-export this
  one), `JudgeFailure` (D52), `CandidateView`, `Prefilter { pool, passing }`, `prefilter`,
  `Route { Human(HumanReason), AutoWin(StepId), Judge(Vec<StepId>), GroupFailed }` with
  `enum HumanReason { Gated, NoJudge, NoSurvivingCandidate, JudgeFailed(JudgeFailure),
  CandidateDropped(i32) }` — D50's reason list, each with a `Display` used in the park note —
  `route(gate, has_judge, &Prefilter)`, `judge_phase(phase, template: SnapshotTemplate, judge:
  &SnapshotCandidate) -> SnapshotPhase` (struct update over the judged phase: `gate`/
  `gate_effective` `Never`, `fan_out` 1, `output_kind` `judge`, `input_kinds` empty,
  `verify_command` `None`, `candidates` = `[judge]`), and `judge_candidate(judge: &SnapshotJudge,
  agent: &Agent) -> Option<SnapshotCandidate>` — `SnapshotJudge.model` is `Option<String>` while
  `SnapshotCandidate.model` is `String` (`model/run.rs:540`, `:545-553`), so the model resolves
  `judge.model` → `agent.default_model` → `agent.models[0]`, `None` (a `judge_unavailable`) when
  none; `judge_inputs(task, Vec<JudgeCandidate>, reverse) -> JudgeInputs`.
  `lib.rs`: the two `pub mod` lines and the doc paragraph (`lib.rs:8-14`) now naming milestone 4's
  two modules; no re-exports yet (T6, T7).
- **Mirror**: `quota.rs:352-456` for the verdict shape; `status.rs:61-74` for `Display`.
- **Validate**: `cargo test -p htui-orch --all-features --lib -- select:: fanout::` (libtest takes
  several filters after `--`; cargo takes one positional `TESTNAME`); `cargo clippy -p htui-orch
  --all-targets --all-features -- -D warnings`.

### Task 3: `htui-orch` — the `Isolator` seam for fan-out (D54, D55, D56, D57, D70)
- **Files**: `crates/htui-orch/src/isolate.rs`, `crates/htui-orch/src/isolate/real.rs`,
  `crates/htui-orch/src/isolate/git.rs`, `crates/htui-orch/src/isolate/copy.rs`,
  `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/engine.rs`.
- **Test first** (git-backed ones open with `let Some(git) = skip_without_git!() else { return; };`,
  the macro at `isolate/git.rs:1463` over `testkit::usable_git`, `:1441`):
  `git.rs` — `diff_and_diff_stat_of_a_range_match_git_s_own_output`,
  `diff_ignores_an_external_diff_driver_and_noprefix` (the test sets `diff.external` and
  `diff.noprefix` in the repo config first), `diff_of_an_empty_range_is_empty`,
  `a_diff_over_the_cap_keeps_its_head_and_says_it_was_truncated` (D55).
  `copy.rs` — `measure_refuses_when_size_times_copies_exceeds_the_cap` (names both figures).
  `real.rs` — `base_reads_each_repo_s_head`,
  `worktree_candidates_all_start_from_the_slot_base_even_after_the_primary_moved`,
  `copy_candidates_reset_to_the_slot_base`,
  `shared_serialized_siblings_reset_a_clean_checkout_to_base`,
  `shared_serialized_refuses_a_dirty_checkout_with_a_slot` (sentence
  `dirty_tree_not_reset: <path>`), `shared_serialized_reconcile_moves_the_branch_to_the_winner`
  (HEAD at the last sibling's label → reset to the winner's label; `after_hash` is that label),
  `shared_serialized_reconcile_refuses_a_head_no_sibling_left` (`primary_moved`),
  `diff_spans_every_repo_under_a_header_line`, `diff_is_none_without_git`,
  `concurrent_worktree_prepares_on_one_repo_never_overlap_their_adds` (D70: 24 slotted `prepare`s
  of distinct steps on one repository under a multi-thread runtime all succeed, and an
  in-flight counter the add lock wraps, exposed under `#[cfg(test)]`, never reads above 1),
  `adds_on_two_repositories_do_not_wait_for_each_other` (the lock is per repository),
  `a_no_slot_prepare_is_unchanged` (every M3 test keeps passing with `None`/`&[]`).
  `fake.rs` — `the_fake_honours_the_slot_base`, `the_fake_diff_is_scripted_and_none_by_default`.
- **Action**: `isolate.rs` — `FanoutSlot<'a>`, the new `prepare` parameter, `base` and `diff`
  verbs, `reconcile`'s `siblings`, doc sentences (idempotence stands; `:138`'s "Fan-out is
  milestone 4's" rewritten). `git.rs` — `Cli::diff(repo, before, after, stat: bool) ->
  Result<String, IsolateError>` with D55's argv and a head-capped capture (a head buffer beside
  `TailBuffer`); doc cites the `git diff --help` flags; **and the module docs that become false**:
  `git.rs:11-15`, `:33` and `:156` ("the five verbs" → six, and `diff` is the one verb whose
  stdout *is* the product) and `Exited::stdout`'s "Never parsed" (`:789-790`), narrowed to "never
  parsed except by `diff`". `copy.rs` — `copies` argument. `real.rs` — D70's `adds` lock table
  and its use around both add calls in `worktree_at`; per mode: `worktree_at`/`copy_at` take the base
  (`real.rs:482-631`), `prepare_in_place` takes the slot (D56), `reconcile_in_place` takes the
  siblings (`:838-860`), `base` = `head_of` per checkout, `diff` per D55 (copy → `tree.path`,
  else `checkout.local_path`). `fake.rs` — `FakeIsolator` answers `base` with a stable synthetic
  hash per repo, uses the slot's base as `before_hash`, adds `script_diff` (FIFO, unscripted
  `None`), ignores `siblings`; its three test call sites (`:1165`, `:1190`, `:1205`) and
  `real.rs`'s ~45 gain the new arguments. `engine.rs` — **only** the two call sites (`:871`
  `prepare(…, None)`, `:1157` `reconcile(…, &[])`); no behaviour change.
- **Mirror**: `git.rs:564` (`reset_hard`) for the verb; `real.rs:1172-1300` for per-mode tests.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; `cargo clippy -p
  htui-orch --all-targets --all-features -- -D warnings`; `grep -rn 'Command::new' crates/htui-orch/src/`
  hits `git.rs` and `verify.rs` only.

### Task 4: snapshot, caps, judge template pin, the tests-only settings writer (D53, D63, D64, D69)
- **Files**: `crates/htui-core/src/model/run.rs`, `crates/htui-core/src/store/mem.rs`,
  `crates/htui-orch/src/graph.rs`.
- **Test first**: `graph.rs` — `the_judge_template_is_pinned_beside_the_judged_phase`,
  `a_fan_out_above_max_fan_out_is_refused_naming_both_figures`,
  `planned_agents_above_the_cap_are_refused_at_resolution` (feature graph, `implement` `fan_out 3`,
  judge configured, default cap → `AgentCap { planned: 7, max: 6 }`),
  `the_analysis_graph_fans_out_three_under_the_default_cap` (5 ≤ 6),
  `a_review_phase_cannot_fan_out`, and `feature_snapshot_topology_is_pinned` **unchanged**
  (`graph.rs:724`). `run.rs` — `a_snapshot_judge_without_a_template_still_decodes`. `mem.rs` —
  `set_project_settings_is_read_back_by_project`.
- **Action**: `SnapshotJudge.template: Option<SnapshotTemplate>` with `#[serde(default)]` and doc
  (`model/run.rs:545-553`); `MemStore::set_project_settings` with `set_app_setting`'s doc sentence;
  `graph.rs` — resolve the `judge` template at `snapshot_phase` (`:483-490`), the three new
  `ResolveError` variants (`:108-160`) and their checks in `resolve` after the phases are built.
- **Mirror**: `graph.rs:137-145` (`LocalFanOut`) for the refusal shape; `mem.rs:434-443`.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-orch --all-features
  graph::`; clippy on both crates.

### Task 5: `status.rs` + `gate.rs` — groups in the loop (D66; helpers for D59)
- **Files**: `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/gate.rs`.
- **Test first**: `status.rs` — `run_failure_display_is_ana2s_bytes` extended with
  `no_candidate_agent: phase \`implement\`; claude (quota: status rejected)` and
  `no_surviving_candidate: implement`; `winner_at_prefers_the_selected_candidate`,
  `winner_at_falls_back_to_index_zero_when_nothing_is_selected`, `group_at_excludes_the_judge`.
  `gate.rs` (over `MemStore::demo()` rows — `gate.rs`'s current tests use no store, `:936+`, so
  these are the first; the fixture's `RUN_3` supplies only **two** `research` candidates at
  `(0, 1)`, indices 0 and 1, and **no judge row** (`fixtures.rs:1474-1490`), so each test inserts
  the `fanout_index = -1` judge and any third or `failed` candidate itself through `create_step`
  and `transition_step`) —
  `retire_supersedes_every_candidate_and_the_judge_of_the_slot`,
  `retire_cancels_a_failed_candidate_and_a_failed_judge`,
  `no_progress_compares_the_two_winners_not_index_zero`. Every existing `gate.rs` test and all 18
  `CASES` stay green.
- **Action**: `RunFailure::NoCandidateAgent { phase, detail }`, `RunFailure::NoSurvivingCandidate
  { phase }`; `pub fn group_at`, `judge_at`, `winner_at`; **no `Cursor` change** (T7). `gate.rs` —
  `retire` iterates `(position, latest attempt)`'s every row; `at()` becomes `winner_at`; the
  `commits_are_identical` doc gains D66's R-8 paragraph.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy.

### Task 6: stage 1 is `R-AGT-8`'s walk; the fallback; the forwarded set; session keys (D60, D62, D67, D68, D71)
- **Files**: `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/graph.rs`,
  `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/lib.rs`,
  `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/fake_conformance.rs`,
  `crates/htui-orch/tests/gix_isolator.rs`.
- **Test first**, five `CASES` (18 → 23), each failing for its stated reason:
  `allowed_warning_candidate_is_selected` (an `agent_box` row upserted with an `allowed_warning`
  quota through `upsert_agent_box` + `set_agent_box_quota`, `traits.rs:270`, `:300`; the step runs
  on it); `a_skipped_candidate_falls_through_to_the_next` (first candidate `rejected`, second
  chosen, one `item_note` naming both); `no_candidate_agent_blocks_the_item` (rung 4 at `StartRun`:
  no run row, item `blocked`, note `no_candidate_agent: phase \`prd\``);
  `every_candidate_skipped_refuses_the_run` (stage 1: run `failed` with `run.failure` starting
  `no_candidate_agent`, item `blocked`); `a_second_attempt_carries_verify_failure_and_previous_diff`
  (a `never` phase whose attempt 1 verify fails: attempt 2's `prompt` event has
  `verify_failure` and `previous_diff` sections; the diff is `FakeIsolator::script_diff`'s). Plus
  `graph.rs` unit tests `rung_three_is_the_single_enabled_agent_on_this_box` and
  `two_enabled_agents_are_not_a_rung_three`. `cli_agent_is_refused_at_a_gated_phase` keeps its
  sentence.
- **Action**: `GraphSource::agent_boxes(box)` (`graph.rs:53-91`) and both `fake.rs` impls
  (`MemStore`'s inherent `agent_boxes`, `mem.rs:521`) plus `graph.rs`'s `TestSource`; `resolve`
  (`graph.rs:233`) gains `box_id`, and rung 3 goes into `candidates` (`graph.rs:502-556`, whose
  doc `:495-501` is rewritten); every `resolve` caller gains the argument — `engine.rs:322`,
  `:681`, `:3658`, `fake.rs:1120`, `graph.rs`'s tests and `tests/fixtures.rs:15`, `:107` — and
  `tests/fixtures.rs` asserts `tests/fixtures/feature.snapshot.json` and
  `feature-with-verify.snapshot.json` are byte-unchanged (rung 3 finds nothing on the demo); `engine.rs` — `admit` (`:726-768`) builds `SelectInput`
  from `graphs.agent`, `graphs.agent_boxes`, `run_steps` and the snapshot, calls `select::walk`,
  writes D60's substitution note, generalises `refuse_capability` to D62's two sentences;
  `start_run` (`:315-367`) handles `ResolveError::NoCandidate` per D62; `assemble_prompt`
  (`:1275-1393`) fills D67; `DriverFor`/`SessionSink`/`dispatch_fake`/`resume_fake`/`fake_parts`
  per D68; `AgentSelector::select` gains `fanout_index` and `admit` passes `0` (D71; T7 calls it per candidate), with a unit test that a selector returning a different agent per index is honoured for index 0; `lib.rs` re-exports `SessionKey`, `select::{walk, Walk, SkipCause}`; `fake.rs` scripts
  by `SessionKey` with the `(phase, attempt)` fallback; `tests/gix_isolator.rs`'s `dispatch`
  closure (`:143`) and `CommittingSink::after_done` (`:358-362`) take the key. Both count pins move
  in the same commit: `tests/fake_conformance.rs:14-17` (`cases_len_is_eighteen` → renamed
  `cases_len_is_twenty_three`, then `…_thirty_five` in T7) and `conformance.rs:1964-1976`, whose
  **test name** `cases_are_unique_and_eighteen` and whose assertion message (it enumerates the
  eighteen) are both rewritten, not just the number.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy.

### Task 7: the fan-out drive, the judge, selection, `SelectFanout` (D48–D53, D58, D59, D65, D71)
- **Files**: `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/status.rs`,
  `crates/htui-orch/src/command.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/lib.rs`,
  `crates/htui-orch/Cargo.toml`, `Cargo.lock`, `crates/htui-orch/src/conformance.rs`,
  `crates/htui-orch/tests/fake_conformance.rs`.
- **Test first**: `status.rs` — `cursor_drives_an_incomplete_group`,
  `cursor_selects_a_settled_group`, `cursor_passes_a_selected_group`,
  `cursor_creates_the_next_attempt_after_a_retired_group`, and M2's
  `cursor_reads_the_zeroth_fanout_index_only` rewritten for a `fan_out = 1` phase (unchanged
  meaning). `command.rs` — `select_enabled` refusals (run not parked, single candidate, already
  selected, winner not a candidate, winner `failed`). Twelve `CASES` (23 → 35) — the criteria by
  number: `fan_out_three_with_a_judge_selects_one_winner` (**criterion 8**: `HTUI_ANA_2` on the
  `analysis` graph via `repoint` with `research` `fan_out 3`, judge via `set_project_settings`;
  three candidates at 0–2 and a judge at −1; winner `selected = true`/`done`; losers `false`/
  `superseded`; three `research` documents survive; `verdict`'s resolved inputs hold only the
  winner's; the judge's `prompt_digest` set and a `follow_up` at turn 1);
  `a_failing_verify_is_eliminated_before_the_judge` (**9a**: the judge prompt holds
  `judge_candidate:0` and `:2` and not `:1`); `one_passing_candidate_wins_without_a_judge` (**9b**:
  no `fanout_index = -1` row); `judge_orderings_that_disagree_park_for_selection` (**10a**: every
  candidate `done`, `selected` NULL, judge `failed` with `gate_note` =
  `judge_disagreement: forward 0, reversed 2`, run and item `awaiting_approval`);
  `an_unparseable_verdict_parks_and_select_fanout_completes` (**10b**: then `SelectFanout` → winner
  `done`/`selected`, judge still `failed` with its note, run walks to `done`);
  `a_gated_fan_out_parks_for_human_selection`; `no_judge_means_human_selection`;
  `a_failed_candidate_does_not_fail_its_siblings`; `a_group_with_no_survivor_retries_then_fails`
  (`never`, `no_surviving_candidate`); `the_review_loop_reruns_a_fanned_out_implement` (whole slot
  retired, new group at attempt 2, judge again); `fan_out_above_max_fan_out_is_refused_at_start`;
  `max_agents_per_run_is_refused_at_start`.
- **Action**: `Cargo.toml` `futures = { workspace = true }` (and the lock); `status.rs`'s
  `Cursor::{Fan, Select}` and group-aware `cursor` (D59), `engine.rs`'s two matches updated;
  `engine.rs` — `admit` creates the whole group, calling `AgentSelector::select` once per `fanout_index` and taking each candidate's agent/model from its own answer (D71; a `fanout_index`-keyed test selector pins that two indices may differ); `drive_group` (base via rows or
  `isolator.base`, one assembly, `join_all` over `run_candidate`); `run_candidate` (stages 2–5 with
  D48's settle, candidate-level failure); `select_stage` (D49's route, auto-win, `run_judge`,
  `park_selection`); `run_judge` (D51–D53: create, walk, `prepare` Local/empty, two calls, verdicts,
  `select_fanout` or the fail-and-park); `reconcile_done_step` passes `siblings`; `select_fanout`
  command handler (D65); `retry_step` retries a group (D65); `resting` maps the new arms.
  `command.rs` — `Command::SelectFanout`, `select_enabled`, `CommandOutcome::Selected`,
  `EngineError::{NotAFanout, AlreadySelected, NotACandidate}`, the `:28-30` reservation sentence.
  `lib.rs` re-exports `fanout::{JudgeFailure, Route}`, `FanoutSlot`, the new command types.
- **Mirror**: every row of Patterns to Mirror.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy; then the
  workspace gate.

### Task 8: real git — fan-out end to end and criterion 7 over worktrees (D54–D57, D66, D70)
- **Files**: `crates/htui-orch/tests/gix_isolator.rs`.
- **Test first** (each opens with `let Some(git) = skip_without_git!() else { return; };`, as the
  file's existing cases do, `:392`, `:552`, `:642`):
  `a_three_way_worktree_fan_out_is_deterministic_over_many_rounds` (D70: 50 consecutive 3-way
  groups on fresh repositories, every `prepare` succeeds — at the probe's ≈ 1 % per group an
  unserialised add would fail this ~40 % of the time; it must not rely on any retry);
  `a_three_way_worktree_fan_out_merges_only_the_winner`
  (three `htui/<step>` branches from one base, `git worktree list --porcelain` shows three trees
  plus the primary, the primary gains one two-parent merge whose second parent is the winner's
  label, the losers' branches remain; `CancelRun` then removes every tree and leaves the branches);
  `shared_serialized_siblings_run_in_turn_and_the_branch_ends_on_the_winner` (the checkout is reset
  to base before sibling 1; after reconcile `HEAD` is the winner's label);
  `a_dirty_shared_checkout_fails_every_sibling_and_parks` (no reset happened, the dirt survives);
  `copy_refuses_n_copies_above_the_cap`; `two_no_commit_implement_attempts_stop_the_loop_in_worktree_mode`
  (criterion 7 over real git, R-8).
- **Action**: the cases, with `CommittingSink` writing a different file per `fanout_index` and
  `FakeOrchestrator` scripts for the judge document.
- **Validate**: `cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1`,
  then the workspace gate.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
cargo doc --workspace --no-deps                  # baseline: the two errors below, no new one
cargo doc -p htui-orch --no-deps --all-features  # this milestone's own doc check: must exit 0
```

**`cargo doc` baseline at `01feaff`: two pre-existing errors**, not CLEAN-3's six (CLEAN-3 is done,
`DECISIONS.md:9`): `htui-core`'s unresolved link to `crate::store::MIRRORED_TABLES`, and
`htui-store`'s public doc of `record_command_run` linking the private `step_exists`. Cargo stops at
the first failing crate, so the workspace run never reaches `htui-orch` — hence the crate-scoped
line, which is the gate for every doc this milestone writes.

`--test-threads=1` is not optional (the keyring fake is process-wide; M2 plan `:207-208`).
**`cargo sqlx prepare --workspace --check` is not part of this milestone's gate**: no query, no
`.sqlx` file and no migration changes (every task's file list above), so there is nothing for it to
regenerate; the reviewer confirms with `git diff --stat -- crates/htui-store/.sqlx
crates/htui-store/migrations` being empty. If a task finds it must change a query, it stops and
reports — that would contradict this plan. Git-backed cases skip with M3's sentence on a box
without `git` ≥ 2.33.0; this box has 2.43.0 (M3 fact ledger), so the gate here runs them.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **Observed on git 2.43.0 by the fact-check:** concurrent `git worktree add` on one repository races on another add's half-written `.git/worktrees/<id>/commondir` — `fatal: failed to read .git/worktrees/<id>/commondir: Success`, exit 128, ≈ 0.4 % per add (2 of 720), ≈ 1 % per 3-way group. No ref-lock collision was seen. M3's classifier does not retry this wording (`isolate/git.rs:844-851`), and the failed add leaves `htui/<step>` created with no worktree, so a blind `-b` retry would fail with "already exists" | Certain without mitigation | **D70**: every `git worktree add` is serialised per repository inside `GixIsolator`, so our own adds cannot race; T3 pins non-overlap with an in-flight counter, T8 runs 50 real 3-way groups without relying on any retry. Residual: another *process's* concurrent add on the same repository; that candidate fails readably (D48), never the run |
| D48 deviates from ANA-2 `:1318-1319`'s "only the failed index is re-attempted": milestone 4 selects over the survivors and re-attempts nothing per candidate | Medium | Recorded in D48 with D65's `UNIQUE` argument; a human's `RetryStep` retries the whole group; the per-index re-attempt belongs to milestone 5's recovery sweep, whose section the passage is in |
| Until MOD-11, no production agent writes a `judge` document, so every production judge fails and every fan-out goes to a human (OQ-4) | Certain | Recorded; the failure is readable (`judge_missing_document: call 0`) and the human path is criterion 10's; the harness proves the judge path |
| ~~`max_agents_per_run = 6` refuses ordinary judged fan-outs of the seeded `feature` graph (OQ-1)~~ **Resolved at the review gate**: the default is 8 (`0004`, OQ-1); a judged fan-out wider than the default still refuses naming both figures | Low | The refusal names both figures; no code outside `graph.rs` depends on the reading |
| A `shared_serialized` fan-out left undecided (judge failed, human away) leaves the maintainer's checked-out branch at the *last* sibling's commit, not the base | Medium | The park note says so and names the base and every sibling label; D56's reconcile restores the winner; milestone 6 renders it; the alternative (reset to base after every capture) would need isolator state per step |
| `git reset --hard` on the maintainer's own checkout between siblings (D56) | Medium | Only when the tree is clean (else refused and nothing is touched), only to a commit that is already on the branch's history, and every sibling's commits stay reachable through its `htui/<step>` label |
| D49's reading of "zero passing" as a failed group disagrees with SWE-agent's "filter disabled, judge everything" | Low | Criterion 9 does not cover zero passing; the choice is §4.2's settle rule (`:439`) applied to the group and is named in D49 |
| D48's candidate settle ignores `verify_outcome`, so a verify-failed candidate is `done` — a reader of a lone `run_step` row could mistake it for a passing step | Low | `verify_outcome = fail` is on the same row, the prefilter reads it, and a candidate is never a winner by default; the gate cell for the group still fails on it (D49) |
| The judge's `gate_outcome = rejected` (D51) is read by a later milestone as "a human rejected it" | Low | D51 records it; milestone 6's renderer reads `fanout_index = -1` first; the alternative writes no `gate_note` at all, which ANA-2 `:860` forbids |
| Two sessions under one `Recorder` (D52) hit a recorder assumption no test has exercised (a second `pump` after a `done`) | Medium | T7's criterion-8 case asserts the judge's event log: one `prompt` at seq 0, one `follow_up` at turn 1, both sessions' `done`s; the fallback is a fresh `Recorder` continuing `seq`, which would need `htui-agent` — stop and report rather than widen the file set |
| `join_all` completes candidates in index order only because the fakes never suspend (D58); a future fake that yields reorders FIFO scripts | Low | Scripts are keyed by `SessionKey` (D68), not FIFO, for everything a fan-out case scripts except `FakeIsolator`'s `after_hash`/`diff` queues; those cases script one value per candidate and assert by `fanout_index` |
| The literal "identical `after_hash`" predicate (kept by D66) only ever fires on two no-commit attempts, so an agent that re-submits the same diff on a new base is not stopped by it | Medium | ANA-2's letter; the review-body half does **not** fire today — `reviews_are_identical` reads latest-only `documents_of_kinds`, so `NoProgressReview` has been unreachable since M2 (carried as **R-9**, blueprint F-B); a patch-identity predicate over D54's `diff` is a cheap later strengthening and is recorded here, not built |
| A §4.5/§4.3 disagreement is transcribed literally at milestone 5 or 6: `blocked` on judge failure (`:861`, `:576`) versus D50's `awaiting_approval` | Medium | OQ-3 and D50 state the deviation and the reason (`item.rs:55`); criterion 10 fails loudly if `blocked` is reintroduced |

## Acceptance

- [ ] ANA-2 §12 criteria 8, 9 (both halves) and 10 (both failure kinds, then `SelectFanout`) pass
      as `htui-orch` conformance cases over `FakeDriver` + `FakeIsolator` + `MemStore`; criterion 7
      passes over real git in `worktree` mode (R-8 closed).
- [ ] A judge step round-trips at `fanout_index = -1` with its own `prompt_digest`, a `follow_up`
      at turn 1 whose candidate order is the reverse of seq 0's (ANA-5 criterion 17's engine half),
      and — on success — `done` with the winner's reason in `gate_note`, written by `select_fanout`.
- [ ] `R-AGT-8`: a skipped candidate falls through with a note; `allowed_warning` is selectable
      and nothing else loosened; rung 3 resolves the single enabled agent on the box; rung 4 and an
      all-skipped stage 1 write `no_candidate_agent` with the item `blocked` and a note;
      `missing_capability: inline_approval` is unchanged.
- [ ] `max_fan_out` and `max_agents_per_run` are refused at `StartRun` naming both figures.
- [ ] Real-git fan-out: three worktrees from one base, one merge, losers' branches kept;
      `shared_serialized` siblings serialised and reset, the branch left on the winner; a dirty
      shared checkout is never reset; `copy` refuses N copies above the cap.
- [ ] `htui-orch`'s `CASES` is 35 and both pins agree; `htui-core`'s store conformance `CASES`
      (49, pinned at `crates/htui-core/tests/mem_store.rs:36`) and `htui-store`'s `EXPECTED_CASES`
      (49, `crates/htui-store/tests/pg_conformance.rs:19`) are unchanged.
- [ ] D70: 24 concurrent slotted `prepare`s on one repository never overlap their `git worktree
      add`, and 50 consecutive real 3-way worktree groups all prepare.
- [ ] No migration, no `.sqlx`, no `.snap`, no `WriteStore`/`ReadStore` change; `grep -rn
      'Command::new' crates/htui-orch/src/` hits `isolate/git.rs` and `verify.rs` only, and
      `git.rs`'s verbs are the five of M3 plus `diff`.
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --all-features --
      -D warnings` clean; `cargo test --workspace --all-features -- --test-threads=1` green with
      Postgres up; `cargo doc --workspace --no-deps` shows only the two baseline errors and
      `cargo doc -p htui-orch --no-deps --all-features` exits 0.

## Fact ledger (every tree claim above, read at `01feaff` on 2026-09-22)

| Claim | Where |
|---|---|
| Tree plan's highest decision is D47 | `.claude/plans/mod-4-orch-tree.plan.md:192`; `grep -ohE '\bD[0-9]{2}\b'` over it and its blueprint |
| Graph built from `3e34610`, which has no `crates/htui-orch` | `graphify-out/GRAPH_REPORT.md:12`; `git ls-tree -d 3e34610 crates/` |
| Milestone 4 row; success metric; PRD D3, D4, D6, D8 | PRD `:308`, `:188`, `:343-349`, `:350-354`, `:359-362`, `:366-370` |
| HANDOFF: F-104 and `allowed_warning` handed to MOD-4; M3 carried R-3..R-8 and the guard notes | `HANDOFF.md:173-180`, `:262-275` |
| `available()` skips any present status other than `"allowed"`; the pin; "no production caller" | `crates/htui-core/src/model/quota.rs:237-239`, `:439-443`, `:1022-1040`, `:416-421` |
| No other crate calls `quota::available` except a settings test with `"allowed"` | `crates/htui/tests/settings.rs:747-763`; grep `Availability::`/`SkipReason` |
| `AgentSelector`/`FirstCandidate` seam; stage-1 interlock and refusal | `crates/htui-orch/src/engine.rs:82-106`, `:726-808` |
| `DriverFor` keyed `(candidate, phase, attempt)`; `SessionSink::after_done(item, step, phase, done)` | `engine.rs:160`, `:122-134` |
| `WALKED_FANOUT_INDEX = 0` in three files; cursor reads index 0 only | `engine.rs:49-50`; `status.rs:12-17`, `:108-113`, `:135-162`; `gate.rs:25-26` |
| `walk_step → fail_hard` fails the run on any live-step error | `engine.rs:821-854`, `:1102-1127` |
| `reconcile_done_step` / `park_run` shapes; unpark order | `engine.rs:1151-1221`, `:1507-1519` |
| `assemble_prompt` leaves `verify_failure`/`previous_diff`/`judge` `None` | `engine.rs:1376-1381` |
| Engine call sites of `prepare`, `reconcile`, `graph::resolve` | `engine.rs:868-872`, `:1157`, `:322`, `:681` |
| `EngineParts` borrows its nine service parts `&'a` (store, graphs, isolator, verifier, clock, selector, sink, driver, scrubber); `app`, `box_profile`, `box_id`, `owner`, `user` are owned | `engine.rs:180-200`, `:201-211` |
| `gate::apply`'s automatic-rejection two-write pattern | `gate.rs:403-423` |
| `retire`, `at()`, `commits_are_identical`, `no_progress` | `gate.rs:854-890`, `:842-851`, `:771-798`, `:748-765` |
| `settle` fails on `verify_outcome = Some(Fail)` | `gate.rs:240`, `:261-263` |
| `RunFailure` and its `Display` test | `status.rs:38-74`, `:195-230` |
| `may_attempt` prospective | `status.rs:27-29` |
| `Isolator` trait: `prepare`, `capture`, `reconcile`, `cleanup`; `IsolatorFuture`; `Prepared` | `crates/htui-orch/src/isolate.rs:106-153`, `:38`, `:81-99` |
| `GixIsolator`: guard in `RepoId` order; `prepare_in_place` empty-scope `cwd`; worktree/copy makers; `reconcile_in_place`/`_isolated`; `NULL` after stays `NULL`; cleanup releases all guards | `isolate/real.rs:291-317`, `:392-403`, `:418-631`, `:777-860`, `:797-799`, `:1003-1006` |
| `measure_within_cap(src, excludes, cap)`; its single call | `isolate/copy.rs:157`; `real.rs:607-609` |
| `Cli::run`, `reset_hard`, `branch_target`, `blocking`, `usable_git` | `isolate/git.rs:325`, `:564`, `:1188`, `:762`, `:1441` |
| Only `git.rs` and `verify.rs` spawn processes in `src/` | `git.rs:196`, `:296`; M3 acceptance |
| `FakeIsolator` FIFO queues; `reconcile` echoes `capture` | `crates/htui-orch/src/fake.rs:53-62`, `:274-310` |
| `FakeOrchestrator` scripts by `(phase, attempt)`; `driver_for`; `after_done` writes `phase.output_kind` | `fake.rs:817`, `:859-864`, `:922-957`, `:959-993` |
| `FakeDriver` refuses a second `start`; `next_event` pops without suspending | `crates/htui-agent/src/fake.rs:178-180`, `:393-408` |
| `MemStore` does no `.await` under its lock | `crates/htui-core/src/store/mem.rs:1-7` |
| `GraphSource` has four methods; rung 3 deferred to the impl; rungs 1, 2, 4; judge from project settings only; caps resolved into the snapshot, unread | `crates/htui-orch/src/graph.rs:53-91`, `:494-556`, `:481-490`, `:296-298` |
| `LocalFanOut` refusal at snapshot time; `ResolveError` variants | `graph.rs:137-145`, `:425-429`, `:108-160` |
| `FEATURE_TOPOLOGY` pin | `graph.rs:590-591`, `:724` |
| `SnapshotJudge { agent_id, agent_name, model }`; `SnapshotPhase.judge`; `SnapshotSettings` caps; `GraphSnapshot::V = 1` | `crates/htui-core/src/model/run.rs:545-553`, `:520`, `:557-570`, `:465` |
| `SnapshotJudge` is used only by `htui-core` and `htui-orch` | grep over `crates/` (`graph.rs`, `model/run.rs`, `model/mod.rs`) |
| `StepGraphPhase` has no judge fields; `0003` added the columns | `model/kind.rs:177-211`; `crates/htui-store/migrations/0003_orchestration.sql:17-22` |
| `ProjectSettings.default_agent_id`/`judge_agent_id` | `model/kind.rs:272-274` |
| Seeded phases: `fan_out 1`, `gate Always`, `retry_limit 1`; the `analysis` graph is `research → verdict` | `crates/htui-core/src/seed.rs:209-239`, `:57-67` |
| `HTUI_ANA_2` is open on the `analysis` kind | `crates/htui-core/src/fixtures.rs:216`, `:853-865` |
| The demo fixture seeds ten templates per project, `judge` among them | `fixtures.rs:692`, `:734`; `seed.rs:372` |
| `select_fanout`: winner `awaiting_approval \| done`, losers superseded or kept, judge settled only from `pending \| running \| awaiting_approval` | `crates/htui-core/src/store/traits.rs:803-832`; `mem.rs:3518-3588` |
| `answer_gate(Rejected)` → `failed` with `gate_note` | `traits.rs:788-801` |
| `resolve_inputs` excludes `selected = false` on both stores | `mem.rs:3077`; `crates/htui-store/src/pg/read.rs:809` |
| `step_events` on `ReadStore`; the prompt event's payload `text` | `traits.rs:80`; `crates/htui-agent/src/record.rs:572-600` |
| `Recorder::record_follow_up` opens the next turn; `pump`; `finish` | `record.rs:623-640`, `:1684-1690`, `:928` |
| `StepStatus`/`RunStatus`/`Status` tables used by D50, D51, D62 | `model/run.rs:61-68`, `:111-128`; `model/item.rs:46-58` |
| `upsert_agent_box`, `set_agent_box_quota` on `WriteStore`; `MemStore::agent_boxes`; the demo seeds no `agent_box` row | `traits.rs:270`, `:300`; `mem.rs:521-532`, `:109`, `:213` |
| `AgentBox.enabled/quota/probe`; `Agent.enabled/default_model/models` | `crates/htui-core/src/model/agent.rs:65-102`, `:31-58` |
| `ProbeSnapshot::from_row`; a `cli` row's probe is `Ready` with no handshake; `ProbeStatus` values | `crates/htui-agent/src/probe.rs:1123`, `:1444-1459`, `:286-297` |
| `caps_for` from the agent row | `crates/htui-agent/src/registry.rs:152` |
| `UsageTotals.cost_micros` is what `run_step.usage` sums | `crates/htui-core/src/model/usage.rs:26-37` |
| `PromptSpec.judge/verify_failure/previous_diff`; `JudgeInputs { task, candidates, reverse }`; `JudgeCandidate`; reversal in the assembler | `crates/htui-core/src/prompt/mod.rs:104-110`, `:168-194`, `:573-580` |
| The seeded `judge` body's verdict block and "nothing after it" | `crates/htui-core/src/prompt/defaults.rs:174-197` |
| `MemStore::set_app_setting` is a tests-only writer | `mem.rs:434-443` |
| `htui-orch` `CASES` 18 with two pins; `htui-core` store `CASES` 49 and `htui-store` `EXPECTED_CASES` 49 | `crates/htui-orch/src/conformance.rs:153-203`, `:1964-1976`; `crates/htui-orch/tests/fake_conformance.rs:14-17`; `crates/htui-store/tests/pg_conformance.rs:19`; `crates/htui-core/tests/mem_store.rs:36` |
| `repoint` edits a live graph through `create_phase` | `conformance.rs:322-360` |
| `tests/gix_isolator.rs` builds `EngineParts` with its own driver closure and implements `SessionSink` | `crates/htui-orch/tests/gix_isolator.rs:136-176`, `:358-380` |
| Workspace `futures = "0.3"`; `htui-orch` has no `futures` edge | `Cargo.toml:36`; `crates/htui-orch/Cargo.toml:17-45` |
| `gix` features: `sha1`, `max-performance-safe`, `parallel`, `index`, `status` | `Cargo.toml:103-110` |
| `gix` diff primitives exist, no porcelain renderer | `gix-0.87.1/src/repository/diff.rs:50`, `src/repository/mod.rs:30`, `src/object/tree/diff/mod.rs:160`, `src/object/blob.rs:133`; `gix-diff-0.67.1/src/blob/unified_diff/impls.rs:11` |
| `vetted()` has nothing to vet: no provider registered | `crates/htui-core/src/prompt/excerpt.rs:1327-1328` |
| `Command` reserves `SelectFanout` for milestones 4–6 | `crates/htui-orch/src/command.rs:28-30` |
| No crate outside `htui-orch` matches on `htui_orch` types (only a tracing include list) | `crates/htui/src/main.rs:18`; grep over `crates/htui*` |
| Reviewer | `.claude/workflow-config.json:2` |

## Claims this plan could not verify

1. **`git diff`'s exact output under D55's flags** on git 2.43.0 — the flags are from `git diff
   --help` knowledge, not a run in this session; T3's first test is the check.
2. ~~That three concurrent `git worktree add -b` on one repository fail only with wordings M3 D39
   classifies~~ — **falsified by the fact-check** (a `commondir` race, not a ref lock); designed
   for by D70.
3. **That one `Recorder` survives a second `pump` after a `done` and records call 1 at turn 1**
   (D52) — read from `record.rs:623-640` and `:1684-1690`, not exercised; named under Risks with a
   stop-and-report instruction.
4. **That `join_all` over the fakes completes candidates in index order** (D58) — inferred from
   `FakeDriver::next_event` and `MemStore`'s synchronous closures, not run.
5. **The maintainer's intent for `max_agents_per_run`** (OQ-1) and whether the judge verdict should
   be readable before MOD-11 (OQ-4) — design questions, not tree facts.
6. **`crates/htui-store/src/pg/read.rs:809`** as the Postgres half of the loser exclusion was read
   as a `selected IS NOT FALSE` line in a `grep`, not traced through `resolve_inputs`' full query.
7. **The `analysis` graph's seeded `input_kinds` for `verdict`** (`["research"]`, `seed.rs:67`) was
   read from a `grep` listing of `seed.rs`; T7's criterion-8 case depends on it and asserts it.

## Verified claims (independent fact-check, 2026-09-22)

One checker pass over this plan against the tree at `01feaff`, the pinned toolchain (`cargo
1.98.1`), `git` 2.43.0 on this box, and the vendored `gix` 0.87.1 / `gix-diff` 0.67.1 sources.
**318 claims: 284 confirmed, 28 partial, 4 falsified, 2 unverifiable.** Every non-confirmed finding
was re-checked against the tree before it was applied; **none was rejected**. Where a row says
"amended", the plan text above carries the correction.

| Claim | Verdict | Evidence, and how it was resolved |
|---|---|---|
| The engine borrows every part `&'a` (D58, ledger) | partial | Nine service parts are `&'a` (`engine.rs:180-200`); `app`, `box_profile`, `box_id`, `owner`, `user` are owned (`:201-211`). D58's conclusion stands (nine borrows are enough to rule out `JoinSet`). **Amended** D58 and the ledger row. |
| `settle` fails on `verify = fail` at `gate.rs:256` | partial | `:256` is the deadline check; the verify check is `gate.rs:261-263` (re-read). **Amended** D48 and the ledger. |
| ANA-2 `:1777` needs "one more word" for `diff` | partial | `:1777` already omits the shipped `merge --abort` (`isolate/git.rs:691-697`). **Amended** OQ-2: the main thread adds two verbs. |
| OQ-6: at `min = 0` the budget rule "coincides" with `available()`'s rule 4 | partial | Rule 4 skips at `spent >= cap` (`quota.rs:446-455`); `cap − spent < 0` misses equality. **Amended** to "subsumed by (strictly weaker than)". |
| "17 new conformance cases" | unverifiable | Nothing in the tree to check a planned count against. **Resolved internally**: T6 lists 5 and T7 lists 12; 18 + 5 + 12 = 35, as Acceptance states. |
| `run_spend(&[RunStep])` sums `cost_micros` | partial | `RunStep` has no typed cost field; `cost_micros` is a key in `usage: Option<Value>` (`model/run.rs:247`, `model/usage.rs:37`). **Amended** T2 and D60. |
| `judge_phase` takes `judge: &SnapshotCandidate` | partial | The snapshot carries `SnapshotJudge` with `model: Option<String>` (`model/run.rs:545-553`) against `SnapshotCandidate.model: String` (`:540`). **Amended** T2: new `judge_candidate(&SnapshotJudge, &Agent)` resolves the model (judge → agent default → first model, else `judge_unavailable`). |
| `Route::Human(HumanReason)` | unverifiable | The type was named and never defined. **Amended** T2: `HumanReason { Gated, NoJudge, NoSurvivingCandidate, JudgeFailed(JudgeFailure), CandidateDropped(i32) }`, D50's list. |
| `fanout::parse_verdict` | partial | `gate::parse_verdict` and `gate::Verdict` are crate-root re-exports (`lib.rs:36-38`). **Amended**: renamed `parse_judge_verdict` (T2, Files). |
| T2's validate line `cargo test … select:: fanout::` | **falsified** | cargo takes one positional `TESTNAME` (reproduced by the checker). **Amended** to `cargo test -p htui-orch --all-features --lib -- select:: fanout::`; T1's and T4's single-filter lines were already valid. |
| T3 updates "three" engine call sites | partial (×2) | Two engine call sites change (`engine.rs:871`, `:1157`), plus `fake.rs:1165`, `:1190`, `:1205` and ~45 in `real.rs`'s tests (grep re-run). **Amended** Files table, the compile-coupling paragraph and T3. |
| `git.rs` gains `Cli::diff` and `Cli::diff_stat` vs one `Cli::diff(…, stat)` | partial | The plan contradicted itself. **Amended** to one method with a `stat: bool` everywhere. |
| `git.rs`'s doc edits are only the flag citations | partial | `git.rs:11-15`, `:33`, `:156` say "five verbs" and "nothing is ever parsed from a verb's stdout"; `Exited::stdout` says "Never parsed" (`:789-790`) — all re-read. **Amended** T3 and the Files table. |
| No Wave A task changes a `.snap`; the grep finds only three `crates/htui` snapshots | partial (×2) | Six `htui-core` judge prompt-render snapshots also match `judge`; the assembler is untouched, so the conclusion holds. **Amended** the hidden-coupling paragraph and the Not-touched list. |
| `RUN_3` is a usable fan-out seed for `gate.rs` tests | partial | Two candidates at `(0, 1)`, no judge row (`fixtures.rs:1474-1490`); `gate.rs`'s tests use no store today. **Amended** T5: each test inserts the judge and extra rows itself. |
| T6's pins move "18 → 23" | partial | The in-file pin is a test **named** `cases_are_unique_and_eighteen` with an enumerating message (`conformance.rs:1964-1976`). **Amended** T6: rename and rewrite, then again in T7. |
| `resolve` gains `box_id` and rung 3 at `graph.rs:502-556` | partial | `:502-556` is `candidates`, `resolve` is `:233`, and `tests/fixtures.rs:15`, `:107` call `resolve` (re-read). **Amended** D62, T6 and T6's file list (adds `tests/fixtures.rs`, conditionally `tests/fixtures/feature*.snapshot.json`). |
| T6's file list is complete | partial | Same finding, same resolution; the snapshot JSON is expected byte-unchanged because rung 3 finds nothing on the demo fixture, and T6 asserts that. |
| T8's cases open with `SKIP_GIT` | partial | The file's cases open with `skip_without_git!()` (`git.rs:1463`; `gix_isolator.rs:392`, `:552`, `:642`); `SKIP_GIT` is the sentence (`:1426`). **Amended** T3, T8 and Patterns to Mirror. |
| `cargo doc` adds no error over CLEAN-3's six | **falsified** | CLEAN-3 is done (`DECISIONS.md:9`); HEAD has two errors (`MIRRORED_TABLES` link in `htui-core`, `record_command_run` → private `step_exists` in `htui-store`), and cargo stops before `htui-orch`. **Amended** Validation and Acceptance: the two-error baseline plus `cargo doc -p htui-orch --no-deps --all-features` as this milestone's gate. |
| `htui-core`'s `CASES` (49) and `EXPECTED_CASES` (49) | partial | `EXPECTED_CASES` is `htui-store`'s (`pg_conformance.rs:19`). **Amended** Acceptance and the ledger. |
| `diff` output "tail-capped at 64 KiB like D30" | partial | `Cli::run`'s `TailBuffer` keeps the last 64 KiB (`git.rs:346`, `:59`), which beheads a long patch; argv had no trailing `--`. **Amended** D55: a head-capped capture with a truncation marker, and `--` after the revisions. |
| Concurrent worktree adds contend on ref locks, and M3's retry covers them | **falsified** | 720 probed adds: 2 failures, both `fatal: failed to read .git/worktrees/<id>/commondir: Success` (exit 128), zero ref-lock collisions; `lock_signature` does not match it (`git.rs:844-851`, re-read) and the failed add leaves `htui/<step>` behind. **Designed for**: new **D70** (per-repository serialisation of every `git worktree add` in `GixIsolator`), D54(e), D57, T3's non-overlap test, T8's 50-round determinism test, and the rewritten Risks row. |
| D63's example of 7 planned agents | partial | `graph.rs:483-490` puts the project judge on *every* phase, so "judged phase = has a judge" gives 10. **Amended** OQ-1 and D63: a judged phase is `fan_out > 1 && judge.is_some()`, which yields 7 and the analysis graph's 5. |
| `overlap.rs`, `recover.rs`, `queue.rs` are "left untouched" | **falsified** | None of the three exists (`ls crates/htui-orch/src/`). **Amended** Not-touched: named as not created. |
| D48 relies on `:1317-1319` without its re-attempt clause | partial | `:1318-1319` also says "only the failed index is re-attempted". **Amended** D48 with a recorded deviation (D65's `UNIQUE` argument; §4.9 is milestone 5's) and a new Risks row. |
| D51: "no writer sets `gate_note` on a running step" | partial | `select_fanout` does, on the way to `done` (`mem.rs:3577-3583`). **Amended** D51's premise to "no writer moves a running step to `failed` with a `gate_note`". |
| D53's trimmer citation and a dropped candidate | partial | The cap is `Trimmer::trim_candidates` (`trim.rs:810-847`), which **drops** an over-share candidate (`:841-845`, "MOD-4 reads a dropped candidate as escalate to human selection"; re-read). **Amended** D53 and D49: a dropped candidate routes to D50's park as `judge_candidate_dropped: <i>`, and `HumanReason::CandidateDropped` carries it. |
| D56 cites risk 2 (`:2065`) for refusing a dirty reset | partial | Risk 2 is about lock contention and a destructive cleanup; the rule is `:916`'s "Refused when" cell. **Amended** D56 to cite `:916` as the rule and `:2065` as the precedent. |
| D60: a `cli` probe's missing handshake motivates "unknown is available" | partial | A probed `cli` row reports `status: ready` (`probe.rs:1444-1459`), so rule (4) passes it anyway; the real reason is the demo's absent `agent_box` rows (`mem.rs:109`, `:213`). **Amended** D60. |
| D62: rung 3 was deferred into "production `phase_agents`" | partial | No production `GraphSource` exists yet; only the fakes (`fake.rs:506`, `:610`) and a test source (`graph.rs:634`). **Amended** D62, including that rung 3 still finds nothing on the demo fixture. |
| D63: caps are read by nothing; `:868-876` | partial | Citations right, but `:871-872` counts planned retries and D63 did not. **Amended** OQ-1 and D63: retries excluded as a recorded deviation, with three reasons. |

**Confirmed, by the section of the plan the checker cited** (line numbers are the pre-amendment
plan's):

- **Header, routing and open questions** — 35 confirmed: every PRD, HANDOFF, ANA-2, ANA-5 and
  REQUIREMENTS citation; D47 as the tree plan's last decision; the stale graph (`3e34610` has no
  `htui-orch`); the quota, `judge`-column, `review`-fan-out and `min_budget` facts behind OQ-1…OQ-8.
- **Summary and decisions D48–D69** — 73 confirmed: the settle and gate-table code paths, the
  `select_fanout` contract on both stores, the status tables, `answer_gate`'s rejection path, the
  `Isolator` and `GixIsolator` shapes and guard ordering, `copy`'s single measurement, the recorder
  and `step_events` APIs, `ProjectSettings`, the `gix` diff primitives, and the `DriverFor` /
  `SessionSink` keys.
- **Carried items, Patterns to Mirror, Files to Change** — 14 confirmed: R-3…R-8 dispositions
  against the M2/M3 blueprints, F-104's `vetted()` with no provider, every mirror target.
- **Tasks T1–T8** — 70 confirmed: the file-set disjointness of Wave A, every named test seam,
  `upsert_agent_box`/`set_agent_box_quota`, `repoint`, the `analysis` graph's
  `research → verdict` with `verdict.input_kinds = ["research"]`, the fixture item `HTUI_ANA_2`,
  and the `quota`/`settings` test interplay.
- **Validation, Risks, Acceptance** — 64 confirmed: the gate commands and their thread
  requirement, no `.sqlx`/migration/seed coupling, the `Command::new` grep, the risk mechanics other
  than the worktree race.
- **Fact ledger** — 28 confirmed as written.
