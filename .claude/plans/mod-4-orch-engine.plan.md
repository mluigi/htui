# Plan: MOD-4 milestone 2 — a graph walks

**Source**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`, milestone 2 (`:306`). Design
authority: the PRD's D1–D8 (cited as **PRD Dn**; this plan's own decisions are plain **Dn**, and
milestone 1's are **M1 Dn**), `docs/ANA-2.md` §4.1/§4.2/§4.3/§4.4/§4.9/§5.1/§5.4/§6.1/§6.2/§9,
`docs/ANA-5.md` §12 criteria 3 and 16, and the two seam questions milestone 1's review gate deferred
here (`.claude/plans/mod-4-orch-seam.blueprint.md:1364-1365`, R-1 and R-2).

**Requirements**: `R-ORCH-1`, `R-ORCH-2`, `R-ORCH-3`, `R-ORCH-11`, `R-ENT-6`, `R-ENT-8`, `R-ENT-12`,
`R-PRM-1` (by construction: the loop forwards three things and no transcript); `R-NF-3` by
construction (the crate links no UI and holds no store handle of its own).

**Complexity**: Large (a new workspace crate with seven modules, the six-stage walk, three status
machines already law but now driven, the review loop and its no-progress predicate, one composite
seam writer with its conformance leg, and six of ANA-2's validation criteria).

**Routing**: routed as **PRD** by `/handoff-run MOD-4` (criteria C2, C3, C4 fired); the PRD and its
milestone table already exist, so this milestone enters the chain at `plan`. Ultracode recommended
for the implement phase and **the maintainer scoped it to implement only**, as in milestone 1.
Reviewer: `rust-reviewer` (`.claude/workflow-config.json:2`).

**Status**: **in progress**. Blueprint: `.claude/plans/mod-4-orch-engine.blueprint.md`, whose §0
records sixteen flags against this plan; the eight `A-` rows amended the decisions below and are
marked in them.

## Summary

Milestone 1 taught the seam what a run is; this milestone makes one walk. A fifth workspace crate,
`crates/htui-orch`, depends on `htui-core` and `htui-agent` and never on `htui-store`
(ANA-2 invariant 10, `docs/ANA-2.md:143-146`), so the engine stays generic over `S: WriteStore` and
headless for `R-ORCH-12`. `graph.rs` resolves a live `step_graph` into the typed `GraphSnapshot`
milestone 1 already models (`crates/htui-core/src/model/run.rs:448`), computes the `topology` hash
that row's doc comment reserves for this milestone (`:453`), and owns the override clone that copies
`step_graph_phase` and `phase_agent` but never `skill_binding`. `engine.rs` walks ANA-2 §4.2's six
stages in their fixed order; `gate.rs` computes the three-valued settle outcome, applies the gate
table, and runs `R-ORCH-3`'s review loop with the no-progress predicate that stops an agent
confidently reproducing the same wrong answer. Both run against `FakeDriver`, which already exists
and is already deterministic (`crates/htui-agent/src/fake.rs:99`), and against a `FakeIsolator`,
which does not exist in any form and is this milestone's first implementation of a new `Isolator`
trait. No git, no Postgres, no real agent, no UI, and no second migration.

One seam addition crosses back into `htui-core`/`htui-store`: milestone 1's deferred **R-1**, a
composite `finish_run` that moves the run and mirrors the item in one transaction. It is answered
here because the blueprint says to decide it "before the first orchestrator writer lands"
(`blueprint.md:1364`), and this milestone is where the first orchestrator writer lands. **R-2 is not
answered here** — its only non-forbidden option is milestone 5's, and the engine's job is to not
foreclose it (D6).

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **`htui-orch` depends on `htui-core` and `htui-agent`, never on `htui-store`; the engine is generic over `S: WriteStore + ReadStore`.** Modules this milestone: `lib.rs`, `graph.rs`, `status.rs`, `command.rs`, `isolate.rs` (the `Isolator` trait only), `engine.rs`, `gate.rs`, plus `fake.rs` and `conformance.rs` behind `test-support`. `fanout.rs`, `select.rs`, `verify.rs`, `overlap.rs`, `recover.rs` and `queue.rs` are named by ANA-2 and **not created**, not even empty. The manifest carries `[lints] workspace = true` in the crate's **first** commit, and `#![warn(missing_docs)]` in `lib.rs`. | ANA-2 invariant 10 (`:143-146`) and `docs/ANA-10.md:2560` conclude the dependency direction; `docs/ANA-5.md:1475` records the `htui-agent` edge. The module list is `docs/ANA-2.md:1659-1690`, restated at PRD `:223-232` — a plan that re-derives it contradicts a concluded ANA. The lint block is inheritance, not propagation: omit it and `cargo clippy --workspace -- -D warnings` still passes while the crate silently loses `unsafe_code = "forbid"` (PRD `:145`, `:294`, `:398`). |
| D19 | **`graph.rs` resolves through a `GraphSource` trait that `htui-orch` defines itself, because `WriteStore + ReadStore` cannot build a snapshot.** Four methods — `phase_agents(phase)`, `prompt_template(project, name, version)`, `agent(id)` and `resolve_graph(item)` — with `fake.rs` implementing it over `&MemStore`. **`app_setting` is not a fifth method (blueprint A-2)**: those reads are inherent too, so `graph::resolve` takes `app: &BTreeMap<String, Value>` as a parameter, the shape `prompt::settings::resolve_budget` already uses; the harness passes `MemStore::app_settings()`, milestone 6 passes `Backend`'s (whose inherent copies `htui-orch` can call, since `MemStore` lives in `htui-core`) and **`htui` implementing it for `Backend` at milestone 6**, where the wiring already belongs. Nothing is added to `htui-core`'s store seam. | **Falsified by the fact-check (F21b), by compile probe, not by reading.** A scratch crate depending only on `htui-core` and generic over `S: ReadStore + WriteStore` fails with `E0599` on `resolve_graph`, `phase_agents`, `step_graph`, `prompt_template` and `agents`: milestone 1 put those eleven reads *inherent* on `MemStore` (`mem.rs:459-644`) and on `Backend` (`backend.rs:440-593`), on no trait, because the tables are not mirrored (M1 D1). Three `GraphSnapshot` fields have no generic source at all — `SnapshotPhase.candidates`, `SnapshotTemplate.version` and the denormalised `agent_name`. Defining the trait in `htui-orch` rather than `htui-core` keeps invariant 10 intact in both directions: the engine never names a concrete store, and `htui-store` never learns about the engine. |
| D20 | **The fake `GraphSource` carries an explicit per-phase candidates map, defaulting to `(AGENT_CLAUDE, "sonnet")`.** Rungs 1 and 3 are computed honestly and are both empty on the demo fixture, so the map is the fake's documented stand-in for rungs 2 and 3; rung 2 stays real in `graph.rs` and is covered by a unit test that writes `default_agent_id` into a `Project.settings` value. **Amended by the blueprint (A-1):** the first draft leaned on rung 3, which is also empty — all three demo agents are `enabled`, so there is no *single* enabled agent — and on `set_setting` for rung 2, which cannot reach `default_agent_id` because `SettingKey` is a closed enum. | `MemStore` holds no `phase_agent` table at all: `phase_agents` returns `Vec::new()` unconditionally (`mem.rs:463-472`), `State::resolve_graph` hard-codes `agents: Vec::new()` (`mem.rs:3856-3862`), and the demo fixture sets no `project.settings.default_agent_id` (`fixtures.rs:660-664`). So rung 1 is structurally empty in this milestone's only harness and rung 2 is empty in its only fixture — a `FakeOrchestrator` that did not say where candidates come from would resolve every phase to "no candidate" and refuse every run, and the criteria would fail for a reason that has nothing to do with the walk. |
| D2 | **Settle `ok` means the output document is present and `verify_outcome` is anything but `fail`.** ANA-2 §4.2's table says `verify_outcome IN ('pass','unavailable')` (`:438`), which excludes `NULL`; §4.3's step table says the same guard as `verify_outcome != 'fail'` (`:637`). The §4.3 form is adopted and the §4.2 form is recorded as a defect in this plan's Risks. | `NULL` is the *normal* case — it is what "no `verify_command` on the phase" writes (`:514`), and every seeded phase has `verify_command: None` (`:319`). Read literally, §4.2 makes every seeded phase settle `failed`, which would make criterion 1 unreachable. One reading has to win and only one of them ships a working walk. |
| D3 | **The retry admission predicate is prospective, in one helper, at every site.** `may_attempt(next_attempt, retry_limit) -> bool` is `next_attempt <= retry_limit + 1`, where `next_attempt` is the attempt about to be *created*, not the one that exists. | ANA-2 writes the predicate as `attempt <= retry_limit + 1` at `:487`, `:579`, `:646` and `:1562`, and as `attempt(p_impl) + 1 <= retry_limit(p_impl) + 1` at `:716`. If the bare form reads the existing step's attempt, the shipped default `retry_limit = 1` would permit creating attempt 3 — three attempts, contradicting `:487`'s own "permits two attempts" in the same sentence. Only the prospective reading yields two. One helper means the four sites cannot drift again. |
| D4 | **The review loop has two entry points and one routine.** `gate::review_loop(...)` is entered automatically from stage 6 when `gate_effective = never` and the settle outcome is `rejected` or `failed`, and from `command::AnswerGate { Rejected }` when a human answers a gated review step. The routine is identical; only the caller differs. | ANA-2 §4.4 states the trigger unconditioned (`:710`) while §4.2's gate table routes `rejected` under `always` and `on_failure` to `awaiting_approval` (`:450`) — under a gate the loop is entered by the human's answer (`:465`, `:613`). The seeded `review` phase's gate is `always` (`:317`), so **in manual mode the automatic path is the unreached one**; a plan that implemented only the §4.4 sentence would ship a loop this milestone's own criteria never execute. |
| D5 | **The loop retires the whole chain from `p_impl` to `p_review`, status by status, and each re-run position takes its own `attempt + 1`.** `p_impl` is the greatest `position < p_review` whose `phase_name` is `implement`, else the immediately preceding position. Retiring is **not** uniformly "supersede": a step in `pending`, `awaiting_approval` or `done` is superseded; a step still `running` is **cancelled first** (`running → cancelled` is legal, `running → superseded` is not); a step already `failed` is **also cancelled** — *corrected by T4 during implementation*: this plan first said "left alone", which makes the walk unreachable, because `cursor` reads a `failed` latest attempt as a rest and the rejecting review at `p_review` would stop the walk forever. `failed → cancelled` is legal and is the same legal retirement this decision already prescribes for a `running` step; `transition_step` moves `status` alone, so the review's `gate_outcome = rejected` and its `gate_note` survive, which is the half of ANA-2 §4.4 step 3 that carries meaning. Each position is then re-inserted at *its own* prior attempt plus one. A `review` at `position = 0`, or with no predecessor, is **terminal**: the step fails, the run parks, and `RunFailure::NoLoopTarget` renders `no_loop_target`. | ANA-2 §4.4 step 5 says intermediate positions "re-run, at the same `attempt` value" (`:722`) without saying the same as *which*, and step 3 supersedes only the reviewed implement step and the rejecting review step (`:719`) — so an intermediate position would be left `done` with a live step at the attempt the walk is about to re-insert, violating `UNIQUE (run_id, position, attempt, fanout_index)`. Per-position attempts are the only UNIQUE-safe reading. **The status split is the fact-check's (F14b, F15, F15b), and this plan's first draft had it wrong:** `StepStatus::can_move_to` admits `superseded` only from `pending`, `awaiting_approval` and `done` (`model/run.rs:113`, `:118-125`, `:127`), and `supersede_step` implements exactly that set on both stores — a blanket supersede of the chain would have earned `StoreError::Constraint` on any live or failed step. `select_fanout` already encodes the same rule for a live loser ("the live loser is the orchestrator's to cancel", `traits.rs:813-816`), so the engine is inheriting a precedent, not inventing one. |
| D6 | **Stage 1 and stage 2 get seams now, implementations later; `overlap.rs` stays milestone 5's single home.** Stage 1 is three things with three different owners: the capability check is **this milestone's** and reads its caps from `htui_agent::registry::caps_for(agent)`, never from a built driver (`permission_requests || edit_proposals`, refusing with `missing_capability: inline_approval`); overlap admission is **delegated whole to `WriteStore::claim_run`**, which milestone 1 already shipped; agent selection goes behind `trait AgentSelector`, whose only implementation here picks the first snapshot candidate that survives the capability check. Stage 2 goes behind `trait Isolator` in `isolate.rs`, whose only implementation here is `FakeIsolator`. **R-2 is not answered, widened, or worked around** — the engine adds no overlap reasoning of its own. | `overlap.rs` is build step 7 and `select.rs` is step 6 (`docs/ANA-2.md:1799-1810`), so ANA-2 never says what stage 1 executes at step 4. Seams cut now are what let steps 6 and 7 land without re-cutting `engine.rs`. R-2's option (a) needs per-repo `isolated`/`local`/`paths` columns, i.e. a `0004`, which PRD `:272` forbids outright ("If a decision seems to need `0004`, it is the wrong decision"), and its option (b) is milestone 5's admission work by the PRD's own milestone table (`:309`). The honest move is to leave the predicate exactly as milestone 1 shipped it and keep the single home the blueprint asks for. The caps source is the fact-check's (F9): caps are computed centrally by `registry::caps_for` from the agent row's transport (`registry.rs:149-189`) precisely "so the chat tab's capability banner and the orchestrator's gate check read one profile per row", and stage 1 must refuse *before* a driver is built, when there is no `driver.caps()` to ask. |
| D7 | **R-1 is answered with the blueprint's option (b): a composite `finish_run(run, to, at)`, the nineteenth writer.** Its signature is `finish_run(&self, run: RunId, to: RunStatus, failure: Option<&str>, at: DateTime<Utc>)` — **the `failure` argument is the blueprint's (A-3)**: a terminal `failed` must write `run.failure`, as `fail_run` does, and a three-argument writer would leave it NULL and force the second write R-1 exists to avoid; a `failure` that disagrees with `to` is a `Constraint`. It moves the `run` to a terminal status and derives the item's status from ANA-2 §4.3's mapping over the item's *remaining* non-terminal runs, in one transaction, on both stores. `transition_run` and `fail_run` are **unchanged** and stay the general-purpose compare-and-set the chat path and the conformance law already use. A conformance case pins item and run moving together. **The `traits.rs:669-675` banner is corrected while T1 is in it**: it names five transactions, but `promote_step` is a sixth on both backends (`pg/write.rs:3303-3335`, `mem.rs:3680-3720`), so `finish_run` makes seven — five → seven, not five → six. | The blueprint states the decision is due "before the first orchestrator writer lands" (`blueprint.md:1364`) and reads (b) as "the smaller change and the one that keeps D6 honest". Option (a) would make both existing writers count the item's other live runs under the item lock — a read they do not do today and a rule `MemStore` would have to duplicate — and would change behaviour for MOD-2's chat path, which M1 D16 already records as deliberately outside §4.3. |
| D8 | **One clock seam, `trait Clock`, and two deadlines with different owners.** `SystemClock` and a `TestClock` in `fake.rs`; every instant the engine hands a writer is `trunc_subsecs(TIMESTAMPTZ_DIGITS)`-truncated. The **step deadline** (§4.2's settle `failed`, `:439`) is this milestone's and is evaluated against the clock; the **verify deadline** (`verify_outcome = 'unavailable'`, `:515`) belongs to `verify.rs` in milestone 3 and is not implemented here. | Every seam writer takes its instant from the caller (blueprint F-S), so the engine owns the clock and must be able to hand tests a fake one; `Harness::settle()` snapshots must stay sleep-free (`docs/ANA-2.md:1766-1767`). `TIMESTAMPTZ_DIGITS` exists because Postgres and `MemStore` otherwise disagree about a column neither changed (`model/run.rs:272`). ANA-2 uses one phrase, "deadline elapsed", for two different clocks in two different stages; naming them apart now stops milestone 3 inheriting the ambiguity. |
| D9 | **`topology` is `"sha256:"` + `sha256_hex` over `serde_json::to_string(&phases)` applied to the typed `Vec<SnapshotPhase>` directly — never through `serde_json::Value`.** The envelope (`v`, `graph`, `mode`, `settings`) is excluded; field order is `SnapshotPhase`'s declaration order; nulls are emitted (no field carries `skip_serializing_if`), so adding a phase field changes every digest and must come with a `GraphSnapshot::V` bump. A test vector pins the exact digest of the seeded `feature` graph. | ANA-2 defines the hash as "over the canonical serialisation of `phases[]`" (`:1479`) and never defines "canonical". **The `Value` prohibition is the fact-check's, from a probe, and it is not a style preference (F2d):** `serde_json`'s `preserve_order` feature is *enabled in a whole-workspace build* — `schemars` ← `agent-client-protocol-schema` ← `htui-agent` turns it on, and Cargo feature unification then hands it to `htui-core` too — while `cargo test -p htui-core` builds without it. The same `GraphSnapshot` round-tripped through `Value` therefore serialises in declaration order in one build and alphabetical order in the other, yielding **two different digests for one graph**, and criterion 3 asserts on exactly that value (`:2090`). Serialising the struct directly is immune. `prompt::digest::sha256_hex` (`prompt/digest.rs:73`) is the workspace's one hasher and is reused. |
| D10 | **The front-matter verdict parser lives in `gate.rs`, and only an exact `request-changes` rejects.** Vocabulary: `approve` and `request-changes`. A review document whose front matter is absent, unparseable, or carries any other value is **not** a rejection; the unexpected value is recorded verbatim so the operator sees it — **as an `item_note` carrying `via_step_id`, not in `gate_note`**, corrected by T4: no shipped writer sets `gate_note` on a `running` step, and none sets it without also answering the gate (`answer_gate` is `awaiting_approval`-only, `select_fanout` is fan-out's). The same applies to every other settle reason (`deadline elapsed`, `stop_reason: refusal`, `cap breached`). The column stays NULL until a human answers, which keeps invariant 7; the gap is carried as R-5. | ANA-5 §12 criterion 16's parser is MOD-4's by `HANDOFF.md:208`; ANA-2 defines settle `rejected` off "front matter" (`:440`) and names no parser, no site and no second legal value. Treating silence as rejection would burn `R-ORCH-3`'s retry budget on a parse bug — the expensive direction of a coin flip the document does not call. The gate is where the settle outcome is computed, so the parser has no other honest home. |
| D11 | **The no-progress predicate compares per-repo `after_hash` (both-`None` counts as identical) or the `sha256_hex(canonical(body))` of the review document.** Both computed from rows the step already wrote. | ANA-2 states the predicate (`:735-739`) and defines neither the hash normalisation nor where the review body's boundaries are. `prompt::digest::canonical` (`digest.rs:36`) is the workspace's settled normalisation — BOM strip, CRLF fold, blank-run collapse, single trailing newline — so a review re-emitted with different line endings does not read as progress. Reusing it also means the predicate cannot disagree with the prompt digest about what two identical texts are. |
| D12 | **Failure strings are a typed `RunFailure` enum whose `Display` renders ANA-2's exact wording.** Variants: `MissingInput(kind)` → `missing input document: <kind>`, `MissingOutput` → `missing_output`, `MissingCapability` → `missing_capability: inline_approval`, `ReviewLoopExhausted(n)` → `review loop exhausted after N attempts`, `NoLoopTarget` → `no_loop_target`. | ANA-2 mixes prose strings and identifier-shaped codes across `:414`, `:430`, `:482`, `:746` with no rule, and two of them are asserted verbatim by validation criteria 6 and 14 (`:2095`, `:2101`). A typed enum with one `Display` is how the exact bytes stay exact while the engine reasons over variants. This mirrors the refusal-sentence vocabulary M1 put in `traits.rs:1053-1130` for the same reason. |
| D13 | **`fake.rs`'s scripted turn may carry an output document, written through `WriteStore::write_document` after the driver's `Done`.** That is the test-side stand-in for MOD-11's `document_write`; a turn that carries none drives the `missing_output` path. The engine itself writes no output document, ever. | Stage 5 requires a document and criterion 1 asserts four of them (`:2086`), but `FakeDriver` cannot write one and `document_write` is MOD-11's, which is blocked on MOD-4 — the circularity is PRD D8's and ANA-2 risk 4's, and is by design. Putting the stand-in in the harness rather than the engine keeps the production gap honest: nothing in `engine.rs` gains a document producer that would have to be removed when MOD-11 lands. |
| D14 | **`graph.rs` never resolves an item with a primary repo to an empty `repo_scope`.** An empty scope is legal only for an item that has no repo at all, and is refused with a named constraint otherwise. | Blueprint H-10 (`:1382`): empty `repo_scope` overlaps nothing, because `'{}' && x` is false in Postgres, while ANA-2 §4.7 intends "empty `touched_paths` overlaps the whole primary repo". The resolution from paths to scope is this milestone's, so this milestone is where the hole is closed — an item queued with an empty scope would silently defeat `claim_run`'s admission predicate. |
| D15 | **The engine reads `SnapshotPhase` and never introduces a second `ResolvedPhase`.** ANA-2 §4.1's design-notation `ResolvedPhase` is already shipped as `SnapshotPhase` (`model/run.rs:482`), and `htui-core` already has an unrelated `ResolvedPhase` (`model/kind.rs:309`, a `StepGraphPhase` plus its `PhaseAgent` rows) which `graph.rs` consumes at *resolution* time only. | Two types with one name across two crates is how a later milestone imports the wrong one. The distinction is also the invariant: resolution reads `step_graph_phase`, the walk reads the snapshot and nothing else (PRD `:288`, invariant 2 at `:109-113`). |
| D16 | **The engine holds no state across a call and re-derives position, attempt and completion from the store.** Every walk decision is a function of `run_steps`, `step_trees`, `step_commits` and `resolve_inputs`; the step-finished test is ANA-2's ("`after_hash` present for every repo in scope **and** a document of `output_kind` produced by this step exists", `:1297`). | Invariant 9 (`:139-142`) is enforced "by the orchestrator holding no cross-restart state", and milestone 5's recovery sweep is only correct if this milestone never introduced any. It is much cheaper to not add the state than to remove it later. |
| D17 | **The engine never assumes a `run` row passed through `can_move_to`.** Any status read from a row is matched exhaustively, and an unexpected status parks rather than panicking. | M1 D16: MOD-2's chat path inserts `run`/`run_step` at `'running'` (`pg/write.rs:721`, `:736`) and the offline upload path at `'done'` (`cache/pending.rs:518`, `:554`), all deliberately outside §4.3. Milestone 1 recorded this "so milestone 2's engine does not assume every `run` row in the database passed through `can_move_to`" — this is that decision being honoured. |
| D21 | **A phase's input kind is required unless a *later* position produces it; a back-edge kind is optional and its absence is recorded, not fatal.** Added by the blueprint (H-8). | The seeded `implement` phase lists `review` in `input_kinds` (`seed.rs:87`) — ANA-2 §4.1's own amendment, so the loop can feed a review back — while §4.2 makes a missing input a hard failure before a token is spent (`:412-414`). On attempt 1 no review exists, so a literal reading fails validation criterion 1 at position 2, and the seed and the contract cannot both be right as written. Requiring only kinds that no later position outputs is the one reading under which both hold: a forward input is still mandatory, and the loop's back-edge is absent exactly when it has not run yet. The hard path keeps a case of its own (`missing_input_fails_before_a_token`) driven by a kind nothing in the graph produces. |
| D18 | **`htui-orch`'s conformance suite mirrors `htui-agent`'s `CaseHarness` shape, not a fixture dump.** `pub const CASES: &[&str]`, `run_case<H: CaseHarness, S: WriteStore>`, `run_all`, a dispatcher whose `match` panics on an unknown name, and a unit test that runs every `CASES` name so the list and the dispatcher cannot drift. The harness trait names no concrete driver, isolator or store. | It is the shape both existing suites already use (`htui-agent/src/conformance.rs:146-238`, `htui-core/src/store/conformance.rs:36-239`) and the reason milestone 3's real `Isolator` and milestone 4's fan-out can be pinned against the same cases they will have to satisfy. `tests/fixtures/*` (ANA-2 `:1690`) holds the recorded graphs the cases walk, not the assertions. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| Crate manifest | inherited `edition`/`rust-version`/`license`/`publish` in that order; `[lints] workspace = true` last | `crates/htui-agent/Cargo.toml:75`, `crates/htui-core/Cargo.toml:33` |
| Feature gating | `default = []`; `test-support` forwarding to the dependency crates' own | `crates/htui-agent/Cargo.toml:15` |
| Dyn-compatible seam | hand-written `Pin<Box<dyn Future … + Send + 'a>>` alias, no `async_trait` | `crates/htui-agent/src/driver.rs:37`, `:363` |
| Generic-over-store seam | plain `async fn` in trait with the targeted `#[allow(async_fn_in_trait)]` and its justifying comment | `crates/htui-core/src/store/traits.rs:62`, `:193` |
| Deterministic fake | scripted turns, `epoch() + n ms` envelope stamps, session id derived from the step id | `crates/htui-agent/src/fake.rs:99-286` |
| Conformance suite | `CASES` + `run_case` + `run_all` + drift-proof dispatcher test | `crates/htui-agent/src/conformance.rs:146-238`, `:3069` |
| Refusal wording | one named free function per refusal so two stores cannot word it differently | `crates/htui-core/src/store/traits.rs:1053-1130` |
| Transition contract | lookup → `NotFound`; legality → `Constraint`; stale `from` → `Ok(false)`; else `Ok(true)` | `crates/htui-core/src/store/traits.rs:976-977`, M1 D14/D15 |
| Prompt construction | build a `PromptSpec` literal and call `assemble(&spec, &MinimalScrubber::new([]))` | `crates/htui/src/preview.rs:229-262` |
| Lock discipline | non-async closure over the guard, so `.await` under a lock is structurally impossible | `crates/htui-core/src/store/mem.rs:1-7` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/store/traits.rs` | edit | T1 | `finish_run` on `WriteStore` (D7); the `RunFailure`-facing refusal sentence if one is needed |
| `crates/htui-core/src/store/mem.rs` | edit | T1 | `MemStore`'s `finish_run`, one `write` closure |
| `crates/htui-core/src/store/conformance.rs` | edit | T1 | the case pinning item and run moving together; `CASES` 47 → 48 |
| `crates/htui-core/tests/mem_store.rs` | edit | T1 | the `CASES` 47 → 48 pin, which lives inside `demo_store_loads_the_fixture` (`:27-49`) next to an unrelated item count, not in a test of its own; its assertion message is a running ledger and gains this milestone's clause |
| `crates/htui-store/src/pg/write.rs` | edit | T1 | `PgStore`'s `finish_run` transaction |
| `crates/htui-store/src/writer.rs` | edit | T1 | **both** halves: `BufferedWriter`'s refusal (its `impl WriteStore` is here, at `:271`, M1 D5, no new constant) and `Writer`'s delegation (`:976`) |
| `crates/htui-store/tests/writer_buffered.rs` | edit | T1 | the one-sentence refusal assertion for the new writer — this path is the *test* file; there is no `src/writer_buffered.rs` |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T1 | `EXPECTED_CASES` 47 → 48 |
| `crates/htui-store/tests/pg_criteria.rs` | edit | T1 | the Postgres twin of the new case |
| `Cargo.toml` | edit | T2 | `members` gains `crates/htui-orch`; `[workspace.dependencies]` gains the internal entry |
| `crates/htui-orch/Cargo.toml` | create | T2 | manifest with `[lints] workspace = true` in the first commit (D1) |
| `crates/htui-orch/src/lib.rs` | create | T2 | `#![warn(missing_docs)]`, module tree, re-exports |
| `crates/htui-orch/src/graph.rs` | create | T2 | the `GraphSource` trait (D19), resolution, field chains, `topology` (D9), override clone, scope resolution (D14) |
| `crates/htui-orch/src/status.rs` | create | T2 | the walk's status helpers over the three shipped `can_move_to` tables; `may_attempt` (D3); `RunFailure` (D12) |
| `crates/htui-orch/src/command.rs` | create | T3 | `StartRun`, `AnswerGate`, `RetryStep` and their enabling guards (ANA-2 §6.2, `:1560-1562`) |
| `crates/htui-orch/src/isolate.rs` | create | T3 | the `Isolator` trait only (D6); milestone 3 fills the file |
| `crates/htui-orch/src/fake.rs` | create | T3 | `FakeIsolator`, `TestClock`, `FakeOrchestrator`, the scripted output document (D13) |
| `crates/htui-orch/src/conformance.rs` | create | T3 | `CASES` + `run_case` + `run_all` + the drift test (D18) |
| `crates/htui-orch/src/engine.rs` | create | T4 | the six-stage walk, `AgentSelector`, the capability check, `Clock` (D8), no cross-call state (D16) |
| `crates/htui-orch/src/gate.rs` | create | T4 | settle outcome (D2), the gate table, the verdict parser (D10), the review loop (D4, D5) and the no-progress predicate (D11) |
| `crates/htui-orch/tests/fixtures/` | create | T5 | the recorded graphs the cases walk |
| `crates/htui-orch/tests/*.rs` | create | T5 | criteria 1, 2, 3, 5, 6, 7 bound through the suite |
| `crates/htui-core/src/model/run.rs` | edit | T5 | the `// derived from row 652 + row 654's exception` marker on `STEP_SANCTIONED` (blueprint C-4) |

**Not touched, on purpose:** no migration of any kind (PRD `:272`) — this milestone adds no column.
**It does add three `sqlx::query!` sites** (`PgStore::finish_run`), corrected by the blueprint (F-E):
T1 regenerates `crates/htui-store/.sqlx/` and commits it with the query, and `cargo sqlx prepare
--check` stays green because of that commit, not because nothing changed. Also untouched: `crates/htui/**`,
which gains its orchestrator surface at milestone 6; `fanout.rs`, `select.rs`, `verify.rs`,
`overlap.rs`, `recover.rs`, `queue.rs`, which are later milestones' and are not created empty;
`claim_run`'s predicate (R-2, D6); `transition_run`/`fail_run` (D7); MOD-2's chat write path;
every `.snap` file in the workspace — this milestone renders nothing and **re-records no snapshot**.

## Tasks

**T1 ∥ T2, then T3, then T4, then T5.** T1 (the seam addition) and T2 (the crate skeleton) touch
disjoint file sets in different crates and are genuinely parallel; everything after is serial
because it compiles against what the previous task defined. That is **two parallel tasks at the
widest point**, which is the honest scale of the fan-out here — milestone 1's plan predicted the
ultracode recommendation would pay off at "milestones 3 to 6" (`mod-4-orch-seam.plan.md:137`) and
this milestone is only marginally wider than that prediction. The orchestration is the maintainer's
call at the CONFIRM gate, not this plan's to assume.

TDD per task: the test that fails for the stated reason comes first.

Every implementer prompt carries: PRD D1–D8 win over this plan where they disagree; graphify-first
for codebase questions, with every graph-derived fact re-verified against the tree; `.sqlx`
regenerated and committed with any query change; nothing sets `updated_at` by hand; no new refusal
constant; **no second migration**; commit incrementally, because uncommitted work does not survive
the session.

### Task 1: `htui-core` + `htui-store` — `finish_run`, the nineteenth writer
- **Action**: write the conformance case first (a graph run reaching `done` moves its item to `done`
  in the same transaction, and a second live run on the item holds the item at `in_progress`), then
  `MemStore`, then `PgStore`, then the `Writer`/`BufferedWriter` pair, then the `pg_criteria` twin
  and the two count pins. `transition_run` and `fail_run` are not edited.
- **Mirror**: M1 D6's transaction shape (one `write` closure on `MemStore`), M1 D5's refusal, M1
  D14's `NotFound`-before-`Constraint` ordering, `traits.rs:976-977`'s contract order.
- **Validate**: `cargo test -p htui-core --all-features` then `cargo test -p htui-store
  --all-features`, then `cargo clippy -p htui-core -p htui-store --all-targets --all-features --
  -D warnings`.

### Task 2: `crates/htui-orch` — the crate, `graph.rs`, `status.rs`
- **Action**: manifest and `lib.rs` in the **first** commit with `[lints] workspace = true`; then the
  `GraphSource` trait (D19), the topology test vector, the field-chain tests and the
  override-clone binding-count test, then the code. The override clone copies `step_graph_phase`
  and `phase_agent` and **not** `skill_binding` (PRD `:399`). `graph.rs` refuses an empty scope for
  an item with a primary repo (D14). The field chains are checked against the *real* seed table,
  which lives in `crates/htui-core/src/seed.rs` (`KINDS` at `:54-165`, `phase_row` at `:209-239`) —
  **not** in `fixtures.rs`, which only calls it (`fixtures.rs:722-731`).
- **Mirror**: `htui-agent`'s manifest shape; `model/kind.rs`'s `ResolvedGraph`/`ResolvedPhase` as the
  *input* to resolution (D15); `prompt::digest::sha256_hex` for the hash.
- **Validate**: `cargo test -p htui-orch --all-features` then `cargo clippy -p htui-orch
  --all-targets --all-features -- -D warnings`.

### Task 3: `htui-orch` — `command.rs`, `isolate.rs`, `fake.rs`, `conformance.rs`
- **Action**: the `Isolator` trait (dyn-compatible, `DriverFuture`-style alias per D1's mirror), then
  `FakeIsolator` returning synthetic, test-controllable hashes and creating no directory tree that
  outlives the test; `TestClock`; `FakeOrchestrator` over `MemStore` + `FakeDriver` + `FakeIsolator`
  with the scripted output document (D13); the `GraphSource` implementation over `&MemStore`,
  supplying candidates per D20 because `MemStore::phase_agents` is unconditionally empty; the three
  commands and their enabling guards; the suite skeleton with its drift test.
- **Mirror**: `htui-agent/src/fake.rs`'s determinism rules and `conformance.rs`'s `CaseHarness`.
- **Validate**: as T2.

### Task 4: `htui-orch` — `engine.rs`, `gate.rs`
- **Action**: the six stages in ANA-2's fixed order, `AgentSelector` and the capability refusal at
  stage 1, `resolve_inputs` at stage 3 with the hard failure before a token is spent, the settle
  outcome (D2), the gate table, the verdict parser (D10), the review loop (D4, D5) and the
  no-progress predicate (D11). Every instant comes from the `Clock` (D8) and is microsecond-truncated.
- **Mirror**: `preview.rs:229-262` for the `PromptSpec` literal; `traits.rs`'s contract order for
  every CAS the engine performs; D17's exhaustive matching on statuses read from rows.
- **Validate**: as T2.

### Task 5: `htui-orch/tests` — the six criteria, and blueprint C-4
- **Action**: the recorded graphs under `tests/fixtures/`, then criteria 1, 2, 3, 5, 6 and 7 as
  suite cases; the `STEP_SANCTIONED` doc marker (blueprint C-4). Blueprint C-5 stays carried — it
  needs a fixture `repo`, which this milestone does not add.
- **Mirror**: `htui-agent/tests/fake_conformance.rs:59` as the binding shape.
- **Validate**: as T2, then the full workspace gate below.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
cargo doc --workspace --no-deps
cd crates/htui-store && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features
```

`--test-threads=1` is not optional: the keyring fake is process-wide, so a green run at the default
thread count proves nothing (`blueprint.md:92`). `cargo doc` exits 101 at HEAD and did before MOD-4
— six pre-existing `htui-store` intra-doc errors, now CLEAN-3 (`HANDOFF.md:448-458`) — so the
acceptance bullet below is qualified, and this milestone must add **zero** new doc errors. Two
provenance notes from the fact-check: `README.md:469-473` carries only the fmt, clippy and test
lines (with clippy's flags in the other order, same effect) and **no `cargo doc` line anywhere** —
the doc gate is plan convention, inherited from `mod-4-orch-seam.plan.md:188`, not a README rule;
and the validator at close-out is not executable (mode `100644`), so it is invoked as
`bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`, which exits 0 at HEAD today.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `htui-orch` is added without `[lints] workspace = true` and silently loses `unsafe_code = "forbid"` and the clippy set | Low | Named in PRD `:398`; the crate's first commit carries the manifest and T2's gate runs clippy on it |
| ANA-2 §4.2's settle-`ok` definition (`:438`) is transcribed literally by an implementer, making every seeded phase settle `failed` | Medium | D2 states the adopted form and the reason; the criterion-1 case fails loudly if the wrong one ships |
| ANA-2 §4.2's gate table (`:450`) is read literally at milestone 5: its `never` × settle-`failed` cell ends an exhausted step at `failed` and the **run** at `awaiting_approval` (escalation), while §4.3's own two tables end the run `failed` for the same event (`:575` "step failed, budget exhausted, `gate_effective = never`, phase not loopable → `failed`"; `:608` "a step reached `failed` with no retry budget and no gate" → `failed`) | Medium | This is the **second** §4.2/§4.3 disagreement, after the settle-`ok` one D2 settles — both are defects in `:438-450` rather than decisions left open, and this one is written down here so milestone 5 does not re-read `:450` and "fix" the shipped behaviour. The §4.3 form is what milestone 2 ships: `gate::retry_or_fail` calls `finish_run(Failed, …)` when `may_attempt` says no, and `missing_output_settles_failed` asserts the run `failed`, the item `failed` and `run.failure = missing_output`. Escalation stays what §4.4 defines it to be — the review loop's exit (`:741-753`), which does park the run at `awaiting_approval` — so `:450`'s cell is not describing a third state, it is describing §4.4's with the wrong trigger |
| The review loop is implemented from §4.4's sentence alone, so only the unreachable automatic path exists and no criterion executes it | Medium | D4 names both entry points; criterion 6 drives the human path and is a required case |
| Intermediate positions between `p_impl` and `p_review` collide on `UNIQUE (run_id, position, attempt, fanout_index)` | Medium | D5 fixes per-position attempts; a fixture graph with an intermediate position is one of T5's recorded graphs |
| The `topology` hash is computed over a different canonicalisation than a later milestone assumes, and criterion 3 passes for the wrong reason | Medium | D9 fixes the serialisation and pins a test vector on the seeded `feature` graph |
| `finish_run` derives the item from the run alone rather than from the item's remaining non-terminal runs, so a second live run is lost | Medium | The conformance case in T1 asserts exactly that shape, on both stores |
| The engine grows overlap reasoning of its own, foreclosing milestone 5's answer to R-2 | Low | D6; `overlap.rs` is not created, and the engine calls `claim_run` and nothing else for admission |
| The engine accretes cross-call state for speed, breaking milestone 5's sweep before it is written | Low | D16; every walk decision re-derives from store reads, which the fake harness makes cheap |
| The topology digest is computed through `serde_json::Value` and silently differs between `cargo test -p htui-orch` and the workspace build, because `preserve_order` is feature-unified on in one and not the other | Medium | D9 forbids the `Value` round trip; the test vector is asserted in a crate-scoped run **and** in the workspace run, which is what makes the divergence visible if it ever returns |
| `GraphSource` is implemented for `Backend` inside `htui-store`, quietly inverting the dependency ANA-2 invariant 10 protects | Low | D19 sites the production impl in `htui` at milestone 6; `htui-store`'s manifest gains nothing this milestone, which is checkable in one diff |

## Acceptance

- [ ] `crates/htui-orch` exists with seven modules, `[lints] workspace = true` in its first commit,
      and depends on `htui-core` and `htui-agent` only.
- [ ] ANA-2 validation criteria 1, 2, 3, 5, 6 and 7 pass as suite cases against `FakeDriver` +
      `FakeIsolator` + `MemStore`, with no git, no Postgres and no real agent.
- [ ] The review loop terminates on both entry points, escalates with the exact
      `review loop exhausted after N attempts` wording, and stops early on two identical `after_hash`
      values.
- [ ] `finish_run` moves run and item in one transaction on both stores, pinned by a conformance
      case; `CASES` is 48 and the two count pins agree.
- [ ] `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets --all-features --
      -D warnings` clean.
- [ ] `cargo test --workspace --all-features -- --test-threads=1` green with Postgres up.
- [ ] `cargo sqlx prepare --check` clean, with the three `finish_run` query files regenerated from
      inside `crates/htui-store` and committed alongside the query (F-E).
- [ ] `cargo doc --workspace --no-deps` adds **no new** errors. It does not exit 0: six `htui-store`
      intra-doc errors pre-date MOD-4 and are CLEAN-3's, not this milestone's.
- [ ] No migration, no `.snap` re-recorded, no second producer of `document` rows in `engine.rs`.

## Verified claims (fact-check, 2026-09-18)

Run at `35dc1ff` over a clean tree, as nine parallel verifiers: **50 claims, 35 confirmed, 12
partial, 3 falsified.** Every falsified or partial claim amended this plan before the maintainer
saw it; the amendments are named in the rows. Two of the three falsifications were found by compile
probe rather than by reading, and both changed a decision rather than a sentence.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| F1 | `crates/htui-orch` does not exist; `members` is the four crates | **CONFIRMED** | `Cargo.toml:2`; no directory, no manifest, every `htui-orch` hit is prose |
| F2a–c | `GraphSnapshot`, `SnapshotPhase` (17 fields), `SnapshotSettings`, `SnapshotCandidate`, `SnapshotJudge`, `SnapshotTemplate` exist as milestone 1 shipped them | **CONFIRMED** | `model/run.rs:447-566`; `GraphSnapshot::V = 1` at `:463-466` |
| F2d | Compact `serde_json` output of the snapshot structs is order-deterministic | **PARTIAL** | True of the typed structs; **false through `serde_json::Value`** — `preserve_order` is on in a workspace build via `schemars` ← `agent-client-protocol-schema` ← `htui-agent` and off under `cargo test -p htui-core`, so a `Value` round trip yields declaration order in one build and alphabetical in the other. Probe in `/tmp/snapprobe`. **Amended D9** to forbid the `Value` path |
| F3 | `prompt::digest::{canonical, sha256_hex}` are public | **CONFIRMED** | `prompt/digest.rs:36`, `:73` |
| F4 | `resolve_inputs` signature; `ResolvedInput { kind, document }` | **CONFIRMED** | `store/traits.rs:176-181`; `model/document.rs:97` |
| F5a | `WriteStore` and `ReadStore` method counts | **CONFIRMED** | 60 and 16 as milestone 1 recorded |
| F5b | `transition_run` and `fail_run` write only the `run` row, on both stores | **CONFIRMED** | `pg/write.rs` single `UPDATE run`; `mem.rs` closure touches no item — this is what makes R-1 real and D7 necessary |
| F5c | No `finish_run` exists anywhere yet | **CONFIRMED** | workspace-wide grep |
| F-extra | M1 D6's "five transactions on every backend" banner | **PARTIAL** | `promote_step` is a sixth (`pg/write.rs:3303-3335`, `mem.rs:3680-3720`), so `finish_run` makes seven. **Amended D7**: the arithmetic is five → seven, and T1 corrects the banner at `traits.rs:669-675` |
| F6a–e | `CASES` = 47, `READ_CASES` = 9, `EXPECTED_CASES` = 47, drift-proof dispatchers in both suites, `CaseHarness` shape | **CONFIRMED** | `store/conformance.rs:36`, `:198`; `pg_conformance.rs:19`; `htui-agent/src/conformance.rs:146-238` — so the plan's 47 → 48 is right in both files |
| F6c | The count pin is a test of its own | **PARTIAL** | The literals live inside `demo_store_loads_the_fixture` (`mem_store.rs:27-49`) beside an unrelated `item_count() == 13`, behind `feature = "test-support"`; its message is a running ledger. **Amended the Files table** |
| F-extra-2 | `BufferedWriter`'s impl is in `src/writer_buffered.rs` | **FALSIFIED** | It is `crates/htui-store/src/writer.rs:271`; `writer_buffered.rs` exists only as `crates/htui-store/tests/writer_buffered.rs`. **Amended the Files table** |
| F7 | `FakeDriver` exists, `test-support`, deterministic, full caps | **CONFIRMED** | `htui-agent/src/fake.rs:99-286`; `FAKE_AGENT_NAME` at `:84` |
| F8 | Nothing named `Isolator` / `FakeIsolator` / `isolate.rs` exists in code | **CONFIRMED** | Every hit is prose in `docs/` or `.claude/` — this milestone writes the first one |
| F9 | `DriverCaps` fields and the CLI profile | **PARTIAL** | Nine fields, `Default` derived all-false (`driver.rs:322-352`); but the CLI profile is **not** declared in `cli/mod.rs` — caps are computed centrally by `registry::caps_for` (`registry.rs:149-189`) so the banner and the gate check read one profile. **Amended D6**: stage 1 calls `caps_for`, never `driver.caps()` |
| F9b–c | `AgentDriver`/`AgentSession` signatures, `DriverFuture`, `StopReason` | **CONFIRMED** | `driver.rs:37`, `:363`, `:402`; `event.rs:96` |
| F14a | The three `can_move_to` tables | **CONFIRMED** | `model/run.rs:60-70`, `:111-130`; `model/item.rs:46-60` |
| F14b | Which `→ Superseded` moves are legal | **CONFIRMED, and load-bearing** | Legal from `pending` (`:113`), `awaiting_approval` (`:118-125`), `done` (`:127`); **illegal from `running` and from `failed`**. Cross-checked against `STEP_SANCTIONED` (`:726-748`) and `cargo test -p htui-core --lib model::run::tests` (5 passed) |
| F15 | `supersede_step`'s accepted source statuses | **CONFIRMED** | `{pending, awaiting_approval, done}` on both stores |
| F15b | Some shipped path moves a step `running → superseded` | **FALSIFIED** | None does: `answer_gate` CASes on `awaiting_approval` only; `select_fanout` leaves a live loser `running` on purpose ("the live loser is the orchestrator's to cancel", `traits.rs:813-816`). **Amended D5**, whose first draft superseded the whole chain blindly and would have earned `Constraint` on any live or failed step |
| F16 | ANA-2 §4.2:438 and §4.3:637 disagree about `verify_outcome = NULL` | **CONFIRMED** | Under `:438` no seeded phase can ever settle `ok`; under `:637` all of them can. **D2 adopts `:637`** and records `:438` as the defect |
| F17 | The retry predicate is written two incompatible ways | **PARTIAL** | Worse than claimed: `:487`, `:579`, `:646` and `:1562` all carry the bare form, and `:646`/`:1562` force the *current-step* reading by spawning `attempt + 1`, which permits three attempts against `retry_limit = 1` — contradicting `:487`'s own "permits two attempts". Only `:716` is prospective. **D3 stands**, and now names all five sites |
| F18 | The seeded phase table lives in `fixtures.rs` | **PARTIAL** | It lives in `crates/htui-core/src/seed.rs` (`KINDS` `:54-165`, `phase_row` `:209-239`); `fixtures.rs:722-731` only calls it. Sub-claims confirmed: every seeded phase has `verify_command: None` (`seed.rs:233`), `review`'s gate is `Always` (`:222`), `review` is in `implement.input_kinds` (`:89`), `gate_hard` is seeded true on exactly three phases. **Amended T2** |
| F18b | ANA-2's no-progress predicate and loop steps 1–5 | **CONFIRMED** | `docs/ANA-2.md:710-722`, `:735-739` |
| F20 | `MemStore` holds `phase_agent` rows, so candidates resolve in a `MemStore` harness | **FALSIFIED** | `phase_agents` returns `Vec::new()` unconditionally (`mem.rs:463-472`), `State::resolve_graph` hard-codes `agents: Vec::new()` (`:3856-3862`), and the demo fixture sets no `default_agent_id` (`fixtures.rs:660-664`). **Added D20** |
| F20b | `MemStore` can answer `resolve_graph` equivalently to `PgStore` | **PARTIAL** | Graph row and ordered phases agree; the candidate half does not — precisely the field `SnapshotPhase.candidates` needs |
| F20c | Item → graph resolution is `item.step_graph_id` else `item_kind.default_graph_id` | **CONFIRMED** | as ANA-2 §4.1 states |
| F21 | The eleven orchestration reads are inherent on `Backend`, not trait methods | **PARTIAL** | True that they are inherent; **incomplete** — `MemStore` carries its own inherent copy of all eleven inside `htui-core` (`mem.rs:459-644`), so they are reachable on the concrete type but never through a trait |
| F21b | A crate generic over `S: ReadStore + WriteStore` with no `htui-store` dependency can build a `GraphSnapshot` | **FALSIFIED** | Compile probe (`/tmp/orchprobe`, `htui-core` only): `E0599` on `resolve_graph`, `phase_agents`, `step_graph`, `prompt_template`, `agents`. Three snapshot fields have no generic source: `candidates`, `template.version`, `agent_name`. **Added D19**, which is the plan's largest amendment |
| F12 | `async_trait` appears nowhere as a dependency | **PARTIAL** | Not declared anywhere and deliberately unused (`driver.rs:7`), but present transitively in `Cargo.lock:267` via `keyring` → `zbus`. The design point stands |
| F13, F19 | `TIMESTAMPTZ_DIGITS = 6`; `deadline_seconds: Option<u32>`, `retry_limit`/`fan_out` `i32` | **CONFIRMED** | `model/run.rs:272`, `:508-510`, `:488`, `:496` |
| F23a | Toolchain pinned to exactly 1.98.1 | **CONFIRMED** | `rust-toolchain.toml`; `cargo 1.98.1`, `rustc 1.98.1` — though in this shell the pin is satisfied via `RUSTUP_TOOLCHAIN`, which agrees |
| F24, F25 | `write_document` version allocation; `close_out` refusals | **CONFIRMED** | `traits.rs:865`, `:903-908` |
| F26, F27, F27b | PRD forbids a second migration; blueprint C-4, C-5, H-10; M1's ultracode sentence | **CONFIRMED** | PRD `:272`; `blueprint.md:1362`, `:1363`, `:1382`; `mod-4-orch-seam.plan.md:135-138` |
| F29 | The repo's gate commands are the README's | **PARTIAL** | fmt/clippy/test are (`README.md:469-473`, clippy's flags in the other order); **`cargo doc` appears nowhere in the README** — it is plan convention. `HTUI_TEST_DATABASE_URL` unset prints `skipped:` and passes (`:477-480`) |
| F30 | The close-out validator exists and is runnable | **PARTIAL** | Exists and **exits 0 at HEAD** (`workflow-docs: 0 errors, 0 warnings`), but is mode `100644` — direct execution fails with exit 126, so it must be run as `bash …/validate-workflow-docs.sh` |
| F-ind | **Task independence**: T1 and T2 touch disjoint file sets | **CONFIRMED** | T1 ⊂ `crates/htui-core/src/store/**`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/**`; T2 ⊂ `Cargo.toml` (root), `crates/htui-orch/**`. Intersection is empty. T3, T4 and T5 each intersect their predecessor inside `crates/htui-orch/src/` and are **serial** — no fan-out marking |
