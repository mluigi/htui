# Plan: MOD-2 prompt assembler + preview (milestone 9, last)

**Source PRD**: `.claude/prds/mod-2-agent-driver-chat.prd.md`
**Selected Milestone**: 9 (Prompt assembler + preview), the last of nine. Milestones 1–2 landed under
`.claude/plans/mod-2-driver-seam-registry.plan.md` (`5c717d0`..`34d5f04`), milestone 3 under
`.claude/plans/mod-2-live-acp-chat.plan.md` (`a142fbf`..`682a423`), milestone 4 under
`.claude/plans/mod-2-durable-history-replay.plan.md` (`81d247b`), milestone 5 under
`.claude/plans/mod-2-probe-autodiscovery.plan.md` (`fb626a8`), milestone 6 under
`.claude/plans/mod-2-agy-acp.plan.md` (`acf16f7`) with its `T34` closed inside milestone 7,
milestone 7 under `.claude/plans/mod-2-quota-caps.plan.md` (`142beb1`..`acec1b8`), and milestone 8
under `.claude/plans/mod-2-cli-transport.plan.md` (`a3cbfff`..`42294d8`). This plan continues their
decision (`D95`+) and task (`T59`+) numbering. It is also **MOD-2's close-out**: the item archives as
one write-up covering all nine phases.

**Design authority**: `docs/ANA-5.md` in full — §4.1 (the `{{name}}` contract, the closed per-role
placeholder sets, the unknown-placeholder policy), §4.2 (the section model, the `<section>` wrapper,
the box projection, the skills resolution and its cap), §4.3 (the amended ANA-9 §7.3 upstream walk
and its three render states), §4.4 (kept-first trim order, the protected set, the strategies, the
estimator, the `set_step_prompt` writer gap), §4.5 (the five-tier excerpt ranker, the skip rules,
the denylist, the `ExcerptProvider` seam), §4.6 (the two reserved template names, the `review` front
matter, the `judge` verdict block, the deterministic `step_summary`), §4.7 (the canonical form and
the nine byte-exact rules), §4.8 and §8 (module layout, store-trait additions, the test plan), §9
(build order and the material this document supersedes), §12 (validation criteria 1–21).
`docs/ANA-4.md` §4.1 (the `prompt` event), §9 (`set_step_usage`'s third parameter), §11 criterion 3.
`docs/ANA-2.md` §4.1 (the budget and template-version chains), §4.7 (`PathPrefix` semantics), §8
(the `ReadStore`-versus-inherent split rule).
`docs/REQUIREMENTS.md`: `R-PRM-1`, `R-PRM-2`, `R-PRM-3`, `R-PRM-4`, `R-ID-4`, `R-ID-5`, `R-ID-6`,
`R-SKL-2`, `R-ORCH-11`, `R-HIS-1`, `R-SEC-3`, `R-NF-3`, `R-NF-4`, `R-TUI-6`, `R-LATER-7`.

**Requirements**: `R-PRM-1` (**must** — one self-contained prompt per step, the section list and the
"never raw transcripts of other items" prohibition), `R-PRM-2` (**must** — out-of-scope upstream
items as one-line stubs), `R-PRM-3` (**must** — the per-step token budget, the fixed priority order,
and the trim recorded on the step), `R-PRM-4` (**must** — the documented placeholder contract the
MOD-9 editor validates against), `R-ID-5` (skills inlined and never trimmed), `R-ID-4`/`R-ID-6` (the
excerpt path is read-only and model-free), `R-SEC-3` (the scrub runs before the digest and fails
closed), `R-ORCH-11`/`R-HIS-1` (the digest and the stored prompt), `R-NF-3` (no assembly on the UI
task).

**Complexity**: Large

**Routing**: PRD path, continued. Routed as **plan** on 2026-09-11; three criteria fired — C2 (a new
public module tree in `htui-core`, five new store methods, three new model type families), C3 (the
PRD's five open questions, all of them ANA-5 §12 criterion 21 items), C4 (~30 files). The PRD already
exists, so `plan-prd` is skipped. **Ultracode recommended for the implement and review phases and
accepted at routing**: the task list below decomposes into independent fronts, and the review gate's
findings are fanned out one verifier per finding.

**CONFIRMED 2026-09-11.** The maintainer accepted **D95**–**D107** as written, over a fact-checked
plan: the five decisions that close ANA-5 §12 criterion 21 (D95 `PromptScope`, D96 `READ_CASES`,
D97 `set_step_usage` keeps its third parameter, D98 one excerpt rendering, D99 the measured
deferral), and the eight that follow from the tree rather than from the ANA (D100 the `Value::String`
scrub wrap, D101 compiled-in `app_setting` defaults, D102 the preview as a deferred sub-tab that
writes nothing, D103 declared stand-ins, D104 the bodies as a constant, D105 read-only skill types,
D106 the two projection fields, D107 three roles built with criterion 18's persistence half
re-deferred to MOD-4). Two facts were flagged at CONFIRM and accepted: T70 closes MOD-2 in full
rather than writing a ninth phase note, and the skills section renders empty on every real prompt
until MOD-9 ships a writer. Work runs on `main`, no branch and no worktree, per the milestone-8
precedent.

---

## Why this milestone looks different from the other eight

Milestones 1–8 built a transport. This one builds the *other* half of the PRD's problem statement —
"an agent session is only as good as the single self-contained prompt it receives" (`prd:17-19`) —
and it has one structural difference the plan must design around rather than discover:

**The assembler's production caller does not exist.** `assemble()` is called at ANA-2 stage 3 of a
graph run, and step graphs are MOD-4's. The PRD already priced this and named the answer: a
**read-only preview** — "pick an item and a phase; see the assembled prompt, its section list, its
trim record and its digest. Without it the assembler ships correct but never executed by the running
binary … Preview renders; it does not send" (`prd:107-110`), against the risk row "the prompt
assembler ships with no production caller and rots before MOD-4" (`prd:206`).

Three consequences run through every task below. The preview is the only binary-level exercise of
the assembler, so it must drive the **real** path rather than a demo shortcut (D102). The inputs a
graph run supplies and a preview cannot — `ResolvedPhase`, `run_step_tree`, the previous attempt's
diff — need declared stand-ins rather than silent zeros (D103). And the two role prompts whose
callers are MOD-4's, `judge` and `handoff`, are built and tested here as pure assembler roles while
their *persistence* rules stay MOD-4's (D107).

---

## Summary

After this milestone `htui` can state, for a real item on a real box, the exact bytes a step would
receive: the template frame with its sections substituted in `R-PRM-1`'s order, every section
scrubbed before it is hashed, a budget resolved through the phase/project/app_setting chain, a trim
that removes content in `R-PRM-3`'s order and records what it removed, and a sha256 that two boxes
agree on whenever their inputs agree.

What already exists and is load-bearing:

- **The migration is done.** `0002_agent_probe.sql` already carries every ANA-5 §9 section — the five
  `COMMENT ON COLUMN` statements (`:40-61`) and the ten `app_setting` defaults (`:68-79`), pinned by
  `crates/htui-store/tests/migrations.rs:195-201,294-322`. ANA-5 §9's build step 5 has no DDL left in
  it, and **MOD-4's `0003_orchestration.sql` stays unheld**.
- **The columns are done.** `run_step.prompt_digest` and `run_step.trim_record` ship in
  `0001_init.sql:487-488`, are mirrored at `cache_migrations/0001_mirror.sql:117` (as TEXT), and are
  modelled on the full row `RunStep` (`model/run.rs:155,158`).
- **The digest and the scrubber exist.** `record.rs:589` already computes `sha256` over a scrubbed
  prompt text for the chat path, and `Scrubber::scrub(&mut Value)` (`scrub.rs:49-58`) is fail-closed
  with `MinimalScrubber` behind it.
- **The preview has a seam and needs no new one.** `DetailRegistry::register`
  (`backlog/detail/mod.rs:102`) exists precisely so a sixth sub-tab is "one `register` line, never a
  `match` arm" (`detail/mod.rs:4-7`).
- **`run_case` is already generic** — `pub async fn run_case<S: WriteStore>(name: &str, store: &S)`
  (`store/conformance.rs:58`).

What does not exist: the whole of `htui_core::prompt` (`lib.rs:11-16` declares four modules and none
of them is it), `WriteStore::set_step_prompt`, a document-**body** reader anywhere on the store seam,
the `Skill`/`SkillVersion`/`SkillBinding`/`BoundSkill` types, `UpstreamEntry`, `BoxProfile`,
`PathPrefix`, a generic `app_setting` reader, and any UI that contains the string `preview`.

---

## Probe findings — what T69 measured (2026-09-11, before `estimate.rs` was written)

D99 scheduled the estimator differential first so the constants would be measured rather than
assumed. It was run before any of `prompt/` existed, against `claude` **2.1.267** on this box,
model `claude-opus-5[1m]`.

*Method.* Total input for a turn is `input_tokens + cache_creation_input_tokens +
cache_read_input_tokens` off the terminal `result` envelope — all three, because the CLI's own
system prompt lands in the cache fields and reading `input_tokens` alone reports **2** (which is
why the milestone-8 fixtures could not calibrate anything). A baseline turn establishes the system
prompt's cost; a second turn appends a corpus of known character count; the delta over the character
count is characters per token. Corpora are deterministic and in-tree: `docs/ANA-5.md` with its fenced
blocks stripped for prose, `crates/htui-core/src/store/conformance.rs` for code. The reply is pinned
to one word so output tokens cannot move the input figure.

| Run | chars | total input tokens | delta vs baseline | chars/token |
|---|---|---|---|---|
| baseline (`Reply with exactly the word: ok`) | — | 17 150 | — | — |
| prose | 40 000 | 33 108 | 15 958 | **2.507** |
| prose, half size | 20 000 | 25 168 | 8 018 | **2.494** |
| code | 40 000 | 33 543 | 16 393 | **2.440** |

**F-16 — §4.4's Claude-family constants are low by more than the reserve can absorb, and low in the
direction that overflows.** ANA-5 §4.4 ships `("chars-v1", prose 3.5, code 3.0)` for the Claude
family. Measured: **2.51 prose, 2.44 code**. Prose is **28.4%** away, outside D99's ±25% threshold;
code is 18.7%, inside it. But the threshold is not the interesting number — the *direction* is.
Assuming 3.5 chars per token where reality is 2.51 under-counts every prose section by **40%**: a
prompt the assembler believes is 108 000 tokens is 151 000, and §4.4's 10% reserve exists to absorb
a rounding error, not a 40% one. That is ANA-5 risk 1 ("the estimator is wrong by 20% and a step
overflows the model's real context window") arriving at twice its assumed size, and it would have
shipped silently: every trim test in the plan asserts *internal* arithmetic — that `tokens_after`
sums to `estimated_after` and that `estimated_after <= target` — and all of them pass just as well
against a wrong constant.

**The two halves agree, which is what makes this a measurement rather than a sample.** The 20 000-
and 40 000-character prose runs give 2.494 and 2.507, 0.5% apart, so the relation is linear and the
baseline subtraction is sound. ANA-5 predicted the sign itself and did not apply it: §4.4 cites
Anthropic's note that "Claude 4.7 and later models … produce approximately 30 percent more tokens
than on earlier models" and then adopts constants read off pre-2026 published ranges anyway.
Measured 2.51 against a published 3.5 is exactly that 30%.

**F-17 — the GPT/Gemini row is not measurable on this box and stays as ANA-5 wrote it.** The second
constants row (4.0 prose / 3.3 code) would be measured against `agy`, and milestone 7 established
that `agy_acp_server` emits **no `usage_update` whatsoever** (`HANDOFF.md`, phase 7 note), so there
is no per-turn token figure to difference. The row keeps ANA-5's values and keeps its
"Unverified" status, with the reason now recorded rather than assumed.

**F-18 — the conservative-default argument survives and is strengthened.** §4.4 makes the Claude row
the default for an unknown agent "because it is the more conservative of the two and over-estimating
trims early rather than overflowing". At 3.5 that was false for any agent whose real ratio is lower;
at 2.44 it is true against both published rows, because a lower chars-per-token yields a *higher*
token estimate for the same text.

---

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Module family in `htui-core` | `crates/htui-core/src/model/`, `store/` | one `mod.rs` holding the public surface and re-exports, one file per concern, `#![warn(missing_docs)]` inherited from the crate |
| Pure closed-vocabulary enum | `crates/htui-core/src/model/quota.rs`, `usage.rs` (`str_enum!`) | a closed enum with `as_str`/`FromStr` and a test that the two round-trip; a new variant is a contract change stated in the doc comment |
| Store-trait addition | `traits.rs:95-100` (`set_step_usage`) + its six impls | trait method, doc naming the requirement, one `MemStore` impl over the in-memory vectors, one `PgStore` impl, two `writer.rs` arms, two test doubles |
| Store conformance | `store/conformance.rs:24-46,58-87` | a `&[&str]` of case names sorted-unique, one `match` arm per name, `run_case` panicking on an unknown name; the two `CASES.len()` literals (`tests/mem_store.rs:25`, `htui-store/tests/pg_conformance.rs:24`) move together |
| Golden text | `crates/htui-agent/tests/cli_map.rs` + `insta` | explicit snapshot names, `.snap` files reviewed with `cargo insta`; the convention is `assert_snapshot!("name", value)` |
| Detail sub-tab | `backlog/detail/runs.rs` | `pub const ID: DetailId`, `impl DetailTab`, no store handle (`R-NF-3`, `runs.rs:5`); state dropped on `on_item_change`, populated from `on_reply` |
| Deferred store work | `AgentRuntime::serve` → `Served::Deferred` (milestone 5's `ProbeAgents`) | work that reads the store and touches the filesystem runs on an owned handle off the UI task, never in the worker's `select!` arm |
| Read-only filesystem walk | `crates/htui-agent/src/probe.rs`'s hand-rolled `*`-per-segment glob walker | `std::fs` plus a hand matcher, no new crate; every cap and every skip rule named in a doc comment |
| Fail-open subsystem | `probe.rs`'s tier-2 decline; `quota::normalize`'s unknown source | absent input yields an absent section plus a recorded note, never an error |

---

## Decisions

| # | Decision | Why |
|---|---|---|
| **D95** | **`PromptScope { workspace: Option<WorkspaceId>, project: ProjectId }`, a new type in `model/scope.rs`, is the `upstream_summaries` argument. The shipped `Scope` is not touched.** `PromptScope::from_scope(&Scope, ProjectId)` and `PromptScope::project_only(ProjectId)` are its
constructors. (Corrected during T61: this row first wrote the former as `PromptScope::from`, which
cannot exist — `From::from` takes one argument. The blueprint's B.11 had it right.) **A third ANA-5
amendment falls out of this and is recorded here rather than in the ANA** (maintainer-only, the
milestone-5 precedent): `docs/ANA-5.md:753-758`'s store-trait snippet still declares
`upstream_summaries(.., scope: &Scope)`, contradicting §4.3 step 1's own finding that the shipped
`Scope` cannot express the no-workspace case. The snippet is stale; `&PromptScope` is the signature,
and T62 takes it from the blueprint rather than from the ANA. **This closes ANA-5 §12 criterion 21's first item.** | The alternative, `Scope::single_project`, would have to invent a `workspace_id` for the no-workspace case, and `R-ENT-2` says "there are no implicit workspace rows" while `scope.rs:10-14` documents itself as "always one workspace, never a bare project list". `Scope` has **20+ construction and consumption sites** across five crates (`app/update.rs:91`, `store_worker.rs:940,1249`, `agent_worker.rs:3254`, `chat/mod.rs:638`, `detail/runs.rs:289`, `writer.rs:547`, `conformance.rs:112`, `mem.rs:1183,1222`, `state.rs:195`, plus seven test sites), every one of which means "the active workspace" — widening that meaning to serve one new method is how a shipped invariant erodes. The SQL takes `$workspace` and `$project` separately (`ANA-5.md:655-657`), so the separate type costs nothing at the query. |
| **D96** | **`run_case<S: WriteStore>` keeps its signature. A second, additive harness — `READ_CASES` plus `run_read_case<S: ReadStore>` and `run_all_reads` — lands beside it in the same module, and `CacheStore` binds to that one.** **This closes criterion 21's second item.** | The question ANA-5 asked ("can `run_case` be generalised over `ReadStore`") has a sharper answer than it expected: `run_case` is *already* generic (`conformance.rs:58`), and the bound cannot be relaxed to `ReadStore` because eleven of the twenty-two existing cases call write methods. Relaxing it would mean splitting every existing case, which is the shipped-signature cost ANA-5 flagged. A parallel `READ_CASES` list is additive, costs no existing case a byte, and is what makes criterion 19's second clause ("`CacheStore` returns equal results to `PgStore` for the four `ReadStore` additions") a test rather than a claim. It follows milestone 8's D80 reasoning exactly: do not widen a shipped private API for one assertion. |
| **D97** | **`set_step_usage` keeps its third parameter. It is not dead and this is not a deferral.** **This closes criterion 21's fourth item.** | ANA-5 §9 listed it as superseded material, expecting `set_step_prompt` to make it redundant. The tree says otherwise: `record.rs:1072` is its only production caller and it is the **free-standing chat** path, whose prompt has no template, no sections and no assembler (`ANA-5.md:418-422` puts it out of scope by name) — its digest is computed in the recorder (`record.rs:589`) and has nowhere else to be written. Dropping the parameter would delete the chat path's only digest writer to tidy a signature. `traits.rs:86-91` already anticipated this ("the digest parameter survives to milestone 9, plan D15(b)"); this decision confirms it rather than reversing it. The assembler path passes `None` and writes through `set_step_prompt`, which is the split ANA-5 wanted. |
| **D98** | **Excerpt rendering is uniform across agent families: line numbers on, `N \| ` form, for `claude` and `agy` alike. `prompt_digest` stays agent-independent.** **This closes criterion 21's fifth item.** | ANA-5 Open-for-the-maintainer 9 states the cost of the other answer precisely: "If the two families ever want different answers, the rendered text becomes agent-dependent and so does `prompt_digest`". Nothing measured in milestones 3–8 argues for divergence — both families' edit tools address content the same way — and an agent-dependent digest would break invariant 3's fan-out identity the moment MOD-4 ever mixed families across a fan-out. One rendering is also one golden snapshot per fixture instead of two. |
| **D99** | **`chars-v1` keeps its constants, and the deferral is confirmed *by measurement* rather than by repetition.** **T69** runs a two-point differential against the live `claude` CLI before `estimate.rs` is finalised: one turn with a minimal prompt, one with a prompt of known character count, the delta in `result.usage.input_tokens + cache_creation_input_tokens` giving the characters-per-token of the prompt text alone. The measured figure is recorded in this plan and in the decision doc. The constants change **only** if the measurement is outside ±25%. **This closes criterion 21's third item.** | The item says the constants "are read off published ranges and nothing in the tree validates them". The committed milestone-8 fixtures cannot validate them either — their `input_tokens` are 0 and 2 (`tests/fixtures/claude_stream_json_*.jsonl`), because the CLI's system prompt dominates the count and cache reads carry the rest. A differential is the only honest measurement available, and it is two cheap turns. Confirming a deferral with a number is worth more than confirming it with a sentence; changing the constants on a single-box, single-family sample would not be. |
| **D100** | **The assembler scrubs by wrapping each section's rendered content in `Value::String`, calling the existing `Scrubber`, and unwrapping. No trait change, no second scrubber.** A residue fails assembly with `AssembleError::Unmasked`, before any session and before the digest. | `Scrubber::scrub` takes `&mut Value` (`scrub.rs:49-58`) because the recorder scrubs event payloads, and `MinimalScrubber::mask_value` handles a `Value::String` leaf directly (`scrub.rs:150`). Wrapping is two lines and keeps `R-SEC-3`'s single fail-closed implementation, which is what makes MOD-10's replacement "behind an unchanged trait" (`prd:100-104`) still true. Adding a `scrub_text` method would give MOD-10 two surfaces to replace and two places to be inconsistent. Scrubbing per section rather than over the assembled string is deliberate: `Unmasked.path` then names the section, which is the fact a maintainer can act on. |
| **D101** | **`assemble()` takes a fully resolved `Budget`; nothing inside it reads a setting. Resolution lives in `prompt::settings`, which takes the phase/project/app_setting values the caller read and falls back to `DEFAULTS`, a compiled-in table.** A test asserts `DEFAULTS` equals migration `0002`'s ten `INSERT` values verbatim. | `assemble()` is pure by contract (invariant 2, `ANA-5.md:1975-1976`) and a store read inside it would end that. The compiled-in table is not a duplicate for its own sake: **`app_setting` has no cache mirror** (`cache_migrations/0001_mirror.sql` has no such table) and **no generic reader** (`connect.rs:299-302` reads exactly two hard-coded cache keys), so an offline box has no other way to learn `token_budget`, and `SEEDED_SETTINGS` is typed `[(&str, i32); 2]` (`pg/mod.rs:46-50`) and cannot express `prompt_reserve_fraction = 0.10` at all. The test against the migration is what stops the two drifting, and `migrations.rs:294-322` already reads those rows, so the assertion has a home. |
| **D102** | **The preview is a sixth Backlog detail sub-tab, `Prompt`, answered by a new `StoreRequest::PromptPreview { item, template_name }` served as deferred work on an owned handle. It writes nothing — no `set_step_prompt`, no `prompt` event, no run.** | The registry exists for this (`detail/mod.rs:4-7,102`) and a sub-tab holds no store handle by rule (`runs.rs:5`, `R-NF-3`), so the assembly — which reads five tables and walks a filesystem — has to be deferred work off the UI task, which is the shape milestone 5's `ProbeAgents` already established. Writing nothing is not caution, it is correctness: `set_step_prompt` writes **a step's** audit row and a preview has no step. The write path is exercised by its conformance case and by an integration test instead (criterion 11). |
| **D103** | **The preview's stand-ins for the inputs MOD-4 owns are declared in `trim_record.notes`, never silently defaulted.** Template: chosen by name from the item's project rows, latest version, `TemplateRole` derived from the name. Attempt: 1, so `verify_failure` and `previous_diff` are absent by the normal empty-section rule. Documents: latest version per kind in kind byte order, with the note `preview: documents are latest-per-kind; ANA-2 input_kinds resolution arrives with MOD-4`. Excerpt roots: `run_step_tree` does not exist and `repo_box_path` has no writer, so the preview resolves no root, renders no excerpt section, and records `no_path` per repo. | The alternative — inventing an `input_kinds` order, or pointing the excerpt walk at the process's working directory — would make the preview's bytes a thing no real run would ever produce, which defeats the only purpose it has. `notes` is exactly where ANA-5 puts "conditions that are not errors" (`ANA-5.md:1549`). The excerpt case is not a degradation either: it is ANA-5 §12 **criterion 12** ("with no `run_step_tree` row and no `repo_box_path` row … the prompt assembles, the excerpt section is absent, and `trim_record.excerpts.roots` records `no_path` per repo"), so the preview proves a criterion rather than dodging one. |
| **D104** | **§5.4's ten default bodies ship as a compiled-in table, `htui_core::prompt::defaults::DEFAULT_TEMPLATES: [(&str, TemplateRole, &str); 10]`, and `fixtures.rs` seeds from it. MOD-15 keeps ownership of the per-project database seed.** `TEMPLATE_NAMES` goes 8 → 10, gaining `judge` and `handoff`. | The PRD's milestone outcome is "confirm the ten default template bodies before MOD-4 depends on them" (`prd:158`) — confirmation needs the bodies in the tree, parsed by the validator and pinned by golden prompts, which is MOD-2's. Seeding them into every project's `prompt_template` rows is a different act, needs a project-creation path, and ANA-5 §4.6 assigns it to MOD-15 by name. A constant satisfies the first without pre-empting the second, and it gives MOD-15 something to seed *from* rather than a document to retype. Today's fixture bodies are `"You are running the `{name}` phase.\n\n{{item}}\n"` (`fixtures.rs:639`) — a single placeholder, which is why no fixture exercises a section today. |
| **D105** | **The `Skill`, `SkillVersion`, `SkillBinding` and `BoundSkill` types land here, in `model/skill.rs`, with the `PgStore::bound_skills` reader and `MemStore` state, but no editor and no writer.** | ANA-5 §8 makes them "a shared prerequisite of MOD-2 and MOD-9 … whichever lands first owns them", and MOD-2 lands first. The skills section is **protected** (invariant 4) and its cap **refuses the step** (`max_skill_tokens`), so criteria 10 and 15 cannot be tested without the types, the `R-SKL-2` override collapse and the binding order. `upsert_skill` / `add_skill_version` / `set_skill_binding` are MOD-9's and are deliberately absent: the tables ship in `0001_init.sql:406-441` and nothing writes them yet, so a read-only model layer is the whole of what this milestone can honestly own. |
| **D106** | **`RunStepSummary` gains `prompt_tokens: Option<i32>` and `trimmed: bool`, derived from `trim_record` by each backend's projection, and the Runs pane renders `~34k` and `!` inside its existing columns. The full trim-record table is rendered by the `Prompt` sub-tab, not by a new pane.** | ANA-5 §4.4 asks for exactly these two fields and for the detail view, and §6.2 assigns the step-list rendering to MOD-4 with the fields to MOD-2. Two fields on a projection that three backends compute (`mem.rs`, `pg/read.rs`, `cache/read.rs`) is the cost; a third pane would be a fourth place the same record is formatted. Putting the table in the `Prompt` sub-tab also means one view renders a *preview's* record and a *stored step's* record through one code path, which is what stops the two drifting. |
| **D108** | **The estimator ships as `chars-v2`, Claude family prose **25** / code **24** (the ×10 integers), measured. The GPT/Gemini row keeps ANA-5's 40 / 33 and keeps its "Unverified" status, with F-17's reason recorded. The Claude row stays the default for an unknown agent.** Taken at CONFIRM+1 on 2026-09-11, after T69 reported. **This amends `docs/ANA-5.md` §4.4's constants and §5.1's example `estimator` value, and is recorded here rather than in the ANA** (maintainer-only, the milestone-5 precedent). | D99 fired its own rule: prose measured 2.507 against a published 3.5, 28.4% away and outside the ±25% threshold. The threshold is not the argument — the direction is. Under-counting prose by 40% means a prompt the assembler believes is 108 000 tokens is 151 000, and §4.4's 10% reserve is sized for a rounding error. It would also have shipped silently, because every trim assertion in this plan checks *internal* arithmetic that a wrong constant satisfies perfectly. A **new id** rather than new constants under the old one, because `trim_record.estimator` is what makes a stored record interpretable: "every token figure in the record is by this estimator and no other" (`ANA-5.md:1544`), which is false the moment one id has two arithmetics. Code moves with prose at 2.44 even though 18.7% is inside the threshold, because leaving it at 3.0 would under-count code by 23% while prose was corrected — one estimator with one row measured and one row guessed is worse than either. |
| **D107** | **All three template roles — `Phase`, `Judge`, `Handoff` — are implemented and tested in the pure assembler. Their callers stay MOD-4's, and criterion 18's persistence half is re-deferred to MOD-4 by name.** | §7's table shows one assembler and three roles, and the role-specific trim orders are inside `trim.rs` either way, so building only `Phase` would leave `SectionName::{JudgeTask, JudgeCandidate, StepSummary, DiffSoFar, FailureReason}` as unreachable variants MOD-4 would have to re-derive. What MOD-2 genuinely cannot do is persist them: criterion 18 requires a handoff prompt to be written as a `follow_up` event at the next turn, which needs a promotion, which is `R-ORCH-5` and MOD-4's. The assembler half is tested here by fixture; the persistence half is named as MOD-4's in the close-out rather than left looking closed. |

---

## Files to Change

Per-task file sets are listed under each task and are what the fact-check intersects; this is the
union.

| File | Action | Why |
|---|---|---|
| `crates/htui-core/Cargo.toml` | UPDATE | `sha2` (workspace, already in the lock) + dev-dep `insta`; no new workspace dependency |
| `crates/htui-core/src/lib.rs` | UPDATE | `pub mod prompt;` beside the four at `:11-16` |
| `crates/htui-core/src/prompt/mod.rs` | CREATE | `assemble()`, `PromptSpec`, `AssembledPrompt`, `Section`, `SectionName`, `TemplateRef`, `AssembleError` |
| `crates/htui-core/src/prompt/template.rs` | CREATE | §4.1 scanner, `Placeholder`, `ParsedTemplate`, `parse()`, `TemplateError` — the module MOD-9 calls |
| `crates/htui-core/src/prompt/estimate.rs` | CREATE | `TokenEstimator`, the per-family constants, the fenced-span split |
| `crates/htui-core/src/prompt/render.rs` | CREATE | the `<section>` wrapper, the fence-collision rule, box projection, skills render, upstream render |
| `crates/htui-core/src/prompt/trim.rs` | CREATE | protected set, trim order, per-section strategies, `TrimRecord`, `TrimStrategy` |
| `crates/htui-core/src/prompt/digest.rs` | CREATE | §4.7's canonical form and the sha256 |
| `crates/htui-core/src/prompt/excerpt.rs` | CREATE | `ExcerptProvider`, `ExcerptRequest`, `ExcerptCandidate`, `RepoReader`, `RepoRoot`, `PathPrefix`, the pure ranker |
| `crates/htui-core/src/prompt/settings.rs` | CREATE | D101's `DEFAULTS` and the budget/hops resolution |
| `crates/htui-core/src/prompt/defaults.rs` | CREATE | D104's ten bodies |
| `crates/htui-core/src/prompt/fixtures.rs` | CREATE | golden `PromptSpec` values behind `test-support` |
| `crates/htui-core/src/model/skill.rs` | CREATE | D105's four types |
| `crates/htui-core/src/model/link.rs` | UPDATE | `UpstreamEntry` |
| `crates/htui-core/src/model/box_.rs` | UPDATE | `BoxProfile`, the §4.2 projection over `BoxRow` + `BoxTool` |
| `crates/htui-core/src/model/scope.rs` | UPDATE | D95's `PromptScope` |
| `crates/htui-core/src/model/run.rs` | UPDATE | D106's two `RunStepSummary` fields |
| `crates/htui-core/src/model/mod.rs` | UPDATE | re-exports |
| `crates/htui-core/src/store/traits.rs` | UPDATE | four `ReadStore` reads + `set_step_prompt` |
| `crates/htui-core/src/store/mem.rs` | UPDATE | the five impls, skills/bindings/settings state, the projection fields |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | the `set_step_prompt` case; D96's `READ_CASES` + `run_read_case` + `run_all_reads` |
| `crates/htui-core/src/fixtures.rs` | UPDATE | D104's ten templates; skills/bindings rows; `sections[]` → `documents:prd` |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | the `CASES.len()` literal; the `READ_CASES` binding |
| `crates/htui-core/tests/prompt_golden.rs` | CREATE | `insta` golden prompts per role and per emptiness pattern |
| `crates/htui-core/tests/prompt_digest.rs` | CREATE | criteria 5, 6, 9, 10 — stability, fan-out identity, CRLF, refusals |
| `crates/htui-store/src/pg/read.rs` | UPDATE | the four reads + the amended §7.3 query + the inherent reads |
| `crates/htui-store/src/pg/write.rs` | UPDATE | `set_step_prompt` |
| `crates/htui-store/src/cache/read.rs` | UPDATE | the four `ReadStore` reads over the mirror |
| `crates/htui-store/src/backend.rs` | UPDATE | dispatch arms for the inherent reads |
| `crates/htui-store/src/writer.rs` | UPDATE | `set_step_prompt` on `BufferedWriter` and the `Writer` enum |
| `crates/htui-store/.sqlx/*` | UPDATE | `cargo sqlx prepare` for the new queries |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE | the `CASES.len()` literal; the `READ_CASES` binding |
| `crates/htui-store/tests/cache.rs` | UPDATE | D96's Postgres-versus-mirror parity binding (criteria 7, 19) |
| `crates/htui-agent/src/excerpt.rs` | CREATE | `FsRepoReader`: the `std::fs` walk, the gitignore subset matcher, the skip rules, the per-file read |
| `crates/htui-agent/src/lib.rs` | UPDATE | `pub mod excerpt;` |
| `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | UPDATE | `set_step_prompt` on the two test doubles |
| `crates/htui-agent/tests/excerpt.rs` | CREATE | `FakeRepoReader`, the four `FakeExcerptProvider` behaviours, the denylist case |
| `crates/htui-agent/tests/estimator_live.rs` | CREATE | T69's differential measurement |
| `crates/htui/src/store_worker.rs` | UPDATE | `StoreRequest::PromptPreview`, `StoreReply::PromptPreview`, the deferred serve |
| `crates/htui/src/ui/tabs/backlog/detail/prompt.rs` | CREATE | the sixth sub-tab: assembled text, sections, trim record, digest |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | `pub mod prompt;` + the re-export |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | one `register` line |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | D106's `~34k` and `!` |
| `crates/htui/tests/prompt_preview.rs` | CREATE | the preview end to end against the demo store |
| `crates/htui/tests/snapshots/*` | UPDATE | the sub-tab strip gains a sixth entry; the Runs pane gains two indicators |
| `README.md` | UPDATE | what the preview is and what it deliberately does not do |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-2.md` | UPDATE/CREATE | MOD-2's full close-out, all nine phases |
| `.claude/prds/mod-2-agent-driver-chat.prd.md` | UPDATE | milestone 9 row `complete`; the five open questions closed |

---

## Tasks

Build order follows ANA-5 §9 steps 1–8. **T59, T61 and T69 are the three independent fronts**;
everything else is serial on them in the chain the "Depends" line names.

### T59: `template.rs` + `estimate.rs` — the scanner and the estimator (ANA-5 step 1) — independent

- **Action**: the `char_indices` scanner of §4.1 (`{{{{` → literal `{{`, `[a-z][a-z0-9_]*` names, no
  whitespace tolerance, `}}` outside a placeholder literal); `Placeholder` with `token()`,
  `allowed_in(role)`, `is_section()`; `ParsedTemplate { spans, used }`; `parse(role, body)` running
  syntax → closed set → role → required-placeholder checks in that order; `TemplateError`'s four
  variants with byte offsets. `TokenEstimator { id, prose_cpt, code_cpt }` with `estimate()`
  splitting at fenced-code boundaries and summing `ceil(chars / cpt)` **per character, not per byte**,
  and `for_agent(name)` returning the three rows — **`chars-v2` per D108: Claude prose 25 / code 24
  (measured, F-16), GPT-Gemini 40 / 33 (ANA-5's, unmeasurable here per F-17), Claude as the unknown
  default**. A test pins each constant to the finding that produced it, so a later change has to
  argue with a number.
- **Mirror**: `model/quota.rs`'s closed-enum-plus-round-trip-test style; `scrub.rs`'s doc density for
  a module whose doc comments are part of a contract.
- **Files**: `crates/htui-core/src/prompt/{mod,template,estimate}.rs`,
  `crates/htui-core/src/lib.rs`, `crates/htui-core/Cargo.toml`.
- **Validate**: `cargo test -p htui-core prompt::template`; criteria 1 and 2 as table-driven cases
  (`{{ item }}`, `{{itme}}`, `{{Item}}`, bare `{{`, `{{{{item}}}}`, `{{candidates}}` in a phase body).
- **Note**: this is the commit MOD-9 unblocks on and it lands first, per ANA-5 §9.

### T60: D104's ten default bodies and the fixture corpus (ANA-5 step 1, cont.)

- **Action**: `prompt/defaults.rs` with the ten §5.4 bodies verbatim, each tagged with its role; a
  test that `parse(role, body)` accepts every one and that each rejects the other two roles' bodies
  with `WrongRole` (criterion 1). `fixtures.rs`: `TEMPLATE_NAMES` 8 → 10, bodies sourced from
  `DEFAULT_TEMPLATES` rather than `format!`, and the demo `prompt` payload's `"prd"` section renamed
  `documents:prd` (§5.2).
- **Depends**: T59.
- **Files**: `crates/htui-core/src/prompt/defaults.rs`, `crates/htui-core/src/fixtures.rs`, the demo
  snapshots the longer bodies move.
- **Validate**: `cargo test -p htui-core --features test-support`; `cargo test -p htui --features
  demo,test-support`.

### T61: model types — `skill`, `UpstreamEntry`, `BoxProfile`, `PromptScope`, `PathPrefix` — independent

- **Action**: `model/skill.rs` (D105's four types, `BoundSkill` carrying the resolved
  `(skill, version, position)` triple); `link::UpstreamEntry` per §4.3; `box_::BoxProfile` as the
  §4.2 projection over `BoxRow` + `Vec<BoxTool>` with the omission rules (`ram` when NULL, `gpu` when
  absent, `quirks` when empty, tools capped at 24 with `, +N more`); `scope::PromptScope` (D95);
  `PathPrefix` in `prompt/excerpt.rs` implementing ANA-2 §4.7's truncation (cut at the first of
  `*?[{`, then at the last `/`; repo-qualified, bare means the primary repo).
- **Mirror**: `model/box_.rs`'s existing field docs; `model/link.rs`'s `LinkNode` for the shape of a
  read-side projection type.
- **Files**: `crates/htui-core/src/model/{skill,link,box_,scope,mod}.rs`,
  `crates/htui-core/src/prompt/excerpt.rs` (the `PathPrefix` half only).
- **Validate**: `cargo test -p htui-core model`; a table-driven `PathPrefix` case per ANA-2 §4.7
  example (`src/**/*.rs` → `src/`, a full path stays whole).
- **Note**: `PathPrefix` does **not** exist in the tree today despite ANA-5 §4.5 citing it as
  shipped — see Verified claims.

### T62: the store seam and `MemStore` (ANA-5 step 4)

- **Action**: `ReadStore` gains `document`, `documents_of_kinds`, `upstream_summaries`, `project`;
  `WriteStore` gains `set_step_prompt`. `MemStore` implements all five over its existing vectors,
  with the upstream walk's `MIN(depth)` dedup, three-state classification and
  `(depth, qualified_key bytes, item_id)` ordering done in Rust exactly as the SQL will be.
  `MemStore::State` gains skills, bindings and projects-with-settings. Conformance: one `CASES` entry
  for `set_step_prompt`; D96's `READ_CASES` with six entries — `document_body_round_trip`,
  `documents_of_kinds_ordered`, `upstream_diamond_dedup`, `upstream_out_of_scope_stub`,
  `upstream_in_scope_no_summary`, `project_settings_readable` — plus `run_read_case` and
  `run_all_reads`. Both `CASES.len()` literals move (`tests/mem_store.rs:25`,
  `htui-store/tests/pg_conformance.rs:24`), from **22 to 23**.
- **Depends**: T61.
- **Files**: `crates/htui-core/src/store/{traits,mem,conformance}.rs`,
  `crates/htui-core/src/fixtures.rs` (skill and binding rows),
  `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/tests/pg_conformance.rs`,
  `crates/htui-agent/src/conformance.rs` + `crates/htui-agent/tests/recorder.rs` (the two test
  doubles gain `set_step_prompt`), `crates/htui-store/src/writer.rs` (the two `Writer` arms).
- **Validate**: `cargo test -p htui-core --features test-support`; `cargo build --workspace` (the
  trait addition must compile against all six implementors).
- **Note**: serial with T60 on `fixtures.rs`.

### T63: `PgStore`, `CacheStore` and the amended §7.3 query (ANA-5 step 5)

- **Action**: the four `ReadStore` reads on `PgStore` and on `CacheStore`; the amended §7.3 recursive
  CTE (`MIN(depth)` dedup in `best`, the `scope` CTE over `workspace_project` or the bare project,
  `in_scope` as its own column, the LATERAL summary lookup no longer gated by scope) with the final
  order **re-sorted in Rust** in both backends; the inherent `PgStore` reads `prompt_template`,
  `bound_skills`, `box_profile`, `app_settings`, `repo_paths` plus their `Backend` dispatch arms;
  `set_step_prompt` on `PgStore` and `BufferedWriter`. `cargo sqlx prepare` because `SQLX_OFFLINE=true`.
  **No migration** — `0002_agent_probe.sql` already carries every ANA-5 section.
- **Depends**: T62.
- **Files**: `crates/htui-store/src/pg/{read,write}.rs`, `crates/htui-store/src/cache/read.rs`,
  `crates/htui-store/src/backend.rs`, `crates/htui-store/src/writer.rs`,
  `crates/htui-store/.sqlx/*`, `crates/htui-store/tests/{pg_conformance,cache}.rs`.
- **Validate**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features`;
  `cargo sqlx prepare --check -- --all-targets --all-features` from `crates/htui-store`; criterion 7's
  second clause (the same diamond read through `PgStore` and `CacheStore` renders byte-identically).

### T64: `render.rs` + `digest.rs` (ANA-5 step 2)

- **Action**: the `<section name="...">` wrapper with §4.2's five content rules (tag on its own line,
  content verbatim, Aider's fence-length rule, attribute escaping with the 120-byte title truncation,
  **no absolute path ever**); the box projection render; the skills render with its `<skill>` blocks;
  the upstream render with Summary blocks first and the one-liners gathered under `also upstream:`;
  the excerpt section's framing paragraph and `<file>` blocks (D98's line numbers). `digest.rs`: the
  eight-step pipeline of §4.7 — render, **scrub per section (D100)**, trim, substitute in span order,
  collapse 3+ LFs to 2, normalise (CRLF → LF, BOM strip, one trailing LF), sha256, hand out one
  `String`.
- **Depends**: T59 (spans), T61 (`BoxProfile`, `UpstreamEntry`, `BoundSkill`).
- **Files**: `crates/htui-core/src/prompt/{render,digest,mod}.rs`,
  `crates/htui-core/src/prompt/fixtures.rs`, `crates/htui-core/tests/prompt_golden.rs`.
- **Validate**: `cargo insta test -p htui-core --test prompt_golden`; criterion 4 (the all-empty
  fixture: no `documents:`/`upstream`/`excerpts` section, no run of three LFs).

### T65: `trim.rs`, `settings.rs` and `assemble()` (ANA-5 step 3)

- **Action**: the protected set (`template`, `skills`, `box`, `command_queue`, `failure_reason`); the
  five-step trim order and the two role-specific orders; the per-section strategies with their floors
  and drops and the one elision marker form; `TrimRecord` serialising exactly §5.1's shape with
  `sections[]` as its `map`-derived projection (invariant against drift: one function, tested element
  for element); re-estimation after every step; the two refusals (`prompt budget too small`,
  `skills exceed max_skill_tokens`) as `AssembleError` variants that happen **before** any session;
  `settings.rs`'s `DEFAULTS` (D101) and the budget/reserve/hops resolution with `budget_source`;
  `assemble()` wiring T59, T64 and this together.
- **Depends**: T64.
- **Files**: `crates/htui-core/src/prompt/{trim,settings,mod}.rs`,
  `crates/htui-core/tests/prompt_digest.rs`.
- **Validate**: criteria 5, 6, 8, 9, 10 — fan-out identity by **grep over the rendered text** for the
  tree root, run id, step id and any absolute path (not only a digest equality); the CRLF fixture;
  the 1.5×-over-budget fixture's arithmetic; both refusals; `DEFAULTS` against migration `0002`.

### T66: excerpts — the pure ranker and `htui-agent::excerpt` (ANA-5 step 6)

- **Action**: `prompt/excerpt.rs`'s five-tier ranker as a pure function over a supplied listing
  (max-weight-not-sum, quantised tier-5 scores, `(weight desc, repo asc, path asc)`, tier 5 capped at
  a third of `max_files`), the `ExcerptProvider` seam with its five rules, the built-in registered
  first as `builtin@1`, windowing per §4.5 step 8, render in path order, and the
  `trim_record.excerpts` audit with per-file `sha256` over the **rendered** bytes.
  `htui-agent/src/excerpt.rs`: `FsRepoReader` — the depth-first `std::fs` walk sorted by name at every
  level, the six skip rules **in order** (`.git/`, the secret denylist, the gitignore subset matcher,
  the NUL-in-first-8-KB binary test, `max_file_bytes`, lockfiles/minified), the scan cap and
  `scan_truncated`.
- **Depends**: T61 (`PathPrefix`), T65 (the residual budget).
- **Files**: `crates/htui-core/src/prompt/excerpt.rs`, `crates/htui-agent/src/{excerpt,lib}.rs`,
  `crates/htui-agent/tests/excerpt.rs`.
- **Validate**: criteria 13 and 14 — the four `FakeExcerptProvider` behaviours (candidates, error,
  panic, past deadline) each leave a valid prompt with a non-`ok` `provider_set` status; a `.env`
  under a `touched_paths` prefix is never selected **and its exclusion is recorded**.

### T67: the preview (D102, D103) — ANA-5 step 7, and the milestone's reason to exist

- **Action**: `StoreRequest::PromptPreview { item, template_name }` served as deferred work on an
  owned handle (never in the worker's `select!` arm, never on the UI task — `R-NF-3`), assembling from
  real store reads with D103's declared stand-ins; `StoreReply::PromptPreview` carrying the text, the
  sections, the trim record and the digest; the sixth detail sub-tab rendering all four with `J`/`K`
  scrolling and a template picker; one `register` line in `backlog/mod.rs`.
- **Depends**: T63, T65, T66.
- **Files**: `crates/htui/src/store_worker.rs`,
  `crates/htui/src/ui/tabs/backlog/detail/{prompt,mod}.rs`,
  `crates/htui/src/ui/tabs/backlog/mod.rs`, `crates/htui/tests/prompt_preview.rs`,
  `crates/htui/tests/snapshots/*`.
- **Validate**: `cargo test -p htui --features demo,test-support`; the preview of a demo item renders
  a digest, a section list and a `no_path` excerpt note (criterion 12 from the running binary).

### T68: the Runs indicators and `RunStepSummary` (D106) — ANA-5 step 8

- **Action**: two fields on `RunStepSummary`, derived from `trim_record` in each of the three `runs()`
  projections; `~34k` and `!` in the Runs pane's existing step row; a fixture step carrying a real
  `trim_record` so the indicators have something to render.
- **Depends**: T63, T65.
- **Files**: `crates/htui-core/src/model/run.rs`, `crates/htui-core/src/store/mem.rs`,
  `crates/htui-store/src/{pg/read.rs,cache/read.rs}`, `crates/htui-core/src/fixtures.rs`,
  `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `crates/htui/tests/snapshots/*`.
- **Validate**: `cargo test -p htui --features demo,test-support runs`; the Runs snapshot shows the
  marker only on the trimmed step.

### T69: the estimator differential (D99) — **measured 2026-09-11, see Probe findings**

- **Status**: the measurement is **done** and produced F-16, F-17, F-18 and **D108**. What remains is
  making it repeatable rather than anecdotal.
- **Action**: commit the probe as `crates/htui-agent/tests/estimator_live.rs`, `#[ignore]`d like every
  other live test: the four-run shape (baseline, prose 40 000, prose 20 000, code 40 000) over
  in-tree deterministic corpora, reading **all three** input fields
  (`input_tokens + cache_creation_input_tokens + cache_read_input_tokens` — reading the first alone
  reports 2, which is F-16's method note), asserting the measured ratios within a stated tolerance
  and asserting the two prose sizes agree within 2% so a broken baseline subtraction fails loudly
  rather than silently re-deriving a constant.
- **Files**: `crates/htui-agent/tests/estimator_live.rs`.
- **Validate**: `cargo test -p htui-agent --features test-support --test estimator_live -- --ignored
  --nocapture`.
- **Note**: burns a small number of model tokens against the maintainer's credential (~$1 for the
  four runs, measured). Independent of every other task; its result is already consumed by T59.

### T70: criteria sweep and MOD-2 close-out (serial, last)

- **Action**: the §12 criteria 1–20 table with a test name per row, and criterion 21's five items
  marked closed with their decision ids (D95–D99); criterion 18's persistence half **re-deferred to
  MOD-4 by name** (D107). `README.md` gains the preview. Then MOD-2's full close-out per
  `.claude/rules/workflow-docs.md` P2: the `HANDOFF.md` checklist line is **deleted**,
  `docs/decisions/mod/mod-2.md` is written covering all nine phases, `DECISIONS.md` gains one index
  line, the summary table's MOD count drops, and the top status line gains MOD-2 and drops its oldest
  mention. The PRD's milestone 9 row goes `complete` and its five open questions are closed with
  their answers. The live coordinates milestone 8 recorded stay in the `HANDOFF.md` recap, and
  **D108's ANA-5 amendment is recorded in the phase note** (§4.4's constants and §5.1's `estimator`
  value, per the milestone-5 precedent that an ANA edit is maintainer-only), together with F-16's
  measured figures — they are the number a later calibration argues with.
- **Files**: `README.md`, `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-2.md`,
  `.claude/prds/mod-2-agent-driver-chat.prd.md`, this plan.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` — **0 errors, or
  no done-report**.

---

## Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps
cargo tree                                    # criterion 20: no new workspace dependency
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features --no-fail-fast
( cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features )

# T69, explicitly (spawns the real CLI; burns model tokens)
cargo test -p htui-agent --features test-support --test estimator_live -- --ignored --nocapture
```

`--no-fail-fast` is not optional here: milestone 8 recorded that a run stopping at the first failing
binary is how two width-broken suites were briefly called green.

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The preview's stand-ins (D103) drift from what MOD-4's stage 3 actually assembles, so the preview shows bytes no run produces | Medium | High | Every stand-in is recorded in `trim_record.notes` and named in the decision, and the preview calls the **same** `assemble()` with the same `PromptSpec` type MOD-4 will build; what differs is who fills three fields. MOD-4's own criterion 11 then compares a real step's digest against the preview's for the same inputs |
| `assemble()` ships correct and MOD-4 later finds the `PromptSpec` shape wrong | Medium | Medium | The three roles are all built (D107), so the shape is exercised by the two prompts MOD-4 will build as well as the one MOD-2 previews. `PromptSpec` is plain data with no store handle, so a missing field is an additive change rather than a re-architecture |
| Six new store methods across six implementors is where a milestone stalls | Medium | Medium | ANA-5 §8 priced it and ANA-2 already paid it once for sixteen methods. Each method lands with its conformance case in the same commit, and D96's `READ_CASES` is additive so no existing case is touched |
| `app_setting` has no cache mirror, so an offline preview silently uses different numbers from an online one | Medium | Medium | D101's `DEFAULTS` are the migration's values, asserted equal by test, and `trim_record.budget_source` names which rung answered — so an offline preview *says* `app_setting_default` rather than looking identical to a project-configured one |
| The skills section is dead code: no writer exists, so every real prompt renders it empty | High | Low | Stated, not hidden. The types, the `R-SKL-2` collapse and the cap are what criteria 10 and 15 test, over fixture rows; MOD-9 supplies the editor. The alternative — deferring the types — would leave MOD-9 blocked on MOD-2 for a model layer, which ANA-5 §8 explicitly set out to avoid |
| The estimator is wrong by more than the 10% reserve absorbs and a step overflows a real context window | Medium | Medium | T69 measures it rather than assuming it, and `trim_record.estimator` makes a systematic error diagnosable from stored rows. An overflow surfaces as an agent-side error the driver already records, which is ANA-5 risk 1's stated posture |
| Golden `insta` snapshots turn every template rewording into a review | Certain | Low | ANA-5 risk 10 accepts this by name: golden prompts are what make `prompt_digest` mean anything, and localising churn to `.snap` files beats hand-edited hex |
| MOD-2's close-out (T70) is a nine-phase write-up and the validator gates the done-report | — | — | The eight phase notes already in `HANDOFF.md` are the source material; the write-up is an edit of existing prose, not a reconstruction. Validator run is a task step, not a post-hoc check |
| Windows: the excerpt walk is the first `htui-agent` filesystem code that traverses arbitrary repositories, and path handling differs | Medium | Medium | Paths are repo-relative by contract (§4.2 rule 5) and the digest normalises line endings (criterion 6), which is the Windows hazard that matters. The runtime verification is **MOD-16's** and will be stated as such, not claimed — and the Windows lint target still cannot build on this box (TOOL-3) |

---

## Verified claims

Checked against the tree at `a601931` on 2026-09-11, before CONFIRM. Sources are file:line in the
working tree, or a documented migration/test assertion.

| Claim | Verdict | Evidence |
|---|---|---|
| `htui_core::prompt` does not exist in any form | true | `lib.rs:11-16` declares `model`, `scrub`, `store`, `fixtures` and nothing else; zero hits for `PromptSpec`, `TokenEstimator`, `TrimRecord`, `ExcerptProvider`, `assemble(` |
| ANA-5 §9's migration work is **already landed** | true | `0002_agent_probe.sql:40-61` carries all five ANA-5 `COMMENT ON COLUMN` statements verbatim and `:68-79` all ten `app_setting` keys with ANA-5's defaults; pinned by `migrations.rs:195-201,294-322`. **Build step 5 has no DDL left** and MOD-4's `0003` stays unheld |
| `run_step.prompt_digest` / `trim_record` exist and are mirrored | true | `0001_init.sql:487-488`; `cache_migrations/0001_mirror.sql:117` (mirror types them TEXT, written through `opt_json_text`, `cache/refresh.rs:1196`) |
| ANA-5 §8's "`CASES` is 15, asserted in two places" | **false — now 22** | `store/conformance.rs:24-46` lists 22 names; asserted at `tests/mem_store.rs:25` and `htui-store/tests/pg_conformance.rs:24`. ANA-5 read the tree at `f989da5`; milestones 2–7 added seven cases. T62 moves both literals 22 → 23 |
| `conformance::run_case` can be generalised over `ReadStore` | **false — and already generic** | it is `run_case<S: WriteStore>(name, store)` (`conformance.rs:58`); eleven of the 22 cases call write methods, so the bound cannot be relaxed without splitting every case. D96 adds `READ_CASES` beside it instead |
| The shipped `Scope` cannot express "current project, no workspace" | true | `scope.rs:10-27`: `{ workspace_id, project_ids }`, `from_workspace` the only constructor, doc "always one workspace, never a bare project list" |
| Narrowing `Scope` would be cheap | **false** | 20+ sites across five crates construct or consume it (`app/update.rs:91`, `store_worker.rs:940,1249`, `agent_worker.rs:3254`, `chat/mod.rs:638`, `detail/runs.rs:289`, `writer.rs:547`, `conformance.rs:112`, `mem.rs:1183,1222`, `state.rs:195`, `backlog/detail/runs.rs:289`, seven test sites). Hence D95's separate type |
| There is **no document-body reader** on the store seam | true | `ReadStore::documents` returns `Vec<DocumentHead>` (`traits.rs:41`) and `DocumentHead` has no `body` (`model/document.rs:33-51`); `Document.body` exists (`:11-30`) with nothing returning it. `ReadStore::document` and `documents_of_kinds` are therefore genuinely new, as ANA-5 §8 says |
| `project.settings` is already readable | true | inherent `project_settings(ProjectId) -> Result<Option<Value>>` on all four backends (`mem.rs:242`, `pg/read.rs:620`, `cache/read.rs:777`, dispatched `backend.rs:312-316`), consumed at `agent_worker.rs:1130`. The budget chain's middle rung needs no new method; ANA-5 §8's `ReadStore::project` is still added for the mirrorable typed row |
| `app_setting` has a generic reader | **false** | `connect.rs:292-302` reads exactly `cache_refresh_seconds` and `cache_overlap_seconds`, hard-coded, and skips any non-positive or non-numeric value. `SEEDED_SETTINGS` is `[(&str, i32); 2]` (`pg/mod.rs:46-50`) and cannot hold `0.10` |
| `app_setting` is mirrored for offline reads | **false** | `cache_migrations/0001_mirror.sql` declares no such table — the sharpest gap, and the reason for D101's compiled-in `DEFAULTS` |
| `set_step_usage`'s third parameter is dead once `set_step_prompt` exists | **false** | its only production caller is `record.rs:1072`, the free-standing chat path, whose digest is computed at `record.rs:589` and which ANA-5 §4.1 puts outside the assembler's scope. `traits.rs:86-91` already says the parameter survives to this milestone. Hence D97 keeps it |
| `Scrubber` can scrub a `&str` | **false — but wrappable** | the trait is `scrub(&self, value: &mut Value)` (`scrub.rs:49-58`); `MinimalScrubber::mask_value` masks a `Value::String` leaf directly (`:150`). D100 wraps and unwraps; no trait change |
| `PathPrefix` is shipped and repo-qualified, per ANA-5 §4.5 | **false — does not exist** | grep over `crates/` returns nothing. ANA-2 §4.7 *specifies* it; nothing implements it. T61 defines it, and MOD-4 inherits it |
| `sha2` is already an `htui-core` dependency | **false at the crate, true at the workspace** | root `Cargo.toml:37` declares it; direct consumers are `htui-store` (`:26`) and `htui-agent` (`:23`). `htui-core/Cargo.toml:16-23` has none. Adding it is a new *edge*, not a new node — criterion 20's `cargo tree` clause survives |
| `insta` is available to `htui-core` | **false** | workspace `Cargo.toml:29`; dev-dep of `crates/htui` (`:50`) and `crates/htui-agent` (`:73`) only. `htui-core`'s only dev-dep is `tokio` (`Cargo.toml:25-26`). Already in the lock, so same reasoning as `sha2` |
| `serde_json::Map` is a `BTreeMap` here, so object keys serialise sorted | true | no `preserve_order` feature anywhere in `Cargo.toml`, the crate manifests or `Cargo.lock`. §4.7's byte-stable `trim_record` relies on it |
| `Skill`/`SkillVersion`/`SkillBinding` Rust types exist | **false** | tables ship (`0001_init.sql:406-441`); the only Rust trace is the `SkillId` newtype (`model/ids.rs:108-109`). `crates/htui/src/ui/tabs/skills.rs` is a stub |
| `UpstreamEntry` / `BoxProfile` exist | **false** | `model/link.rs` has `LinkKind`, `ItemLink`, `LinkNode`, `LinkEdge`, `LinkGraph` and no `UpstreamEntry`; `model/box_.rs` has `BoxRow`, `BoxTool`, `BoxInfo`, `OsFamily` and no `BoxProfile`. `LinkNode` carries no body text, so the upstream section cannot be built from `links()` |
| `RunStepSummary` carries prompt fields | **false** | `run.rs:262-287` — eleven fields, none of them prompt-related. The full `RunStep` does (`:155,158,161`) but `ReadStore::runs` does not return it. Hence D106's two projection fields across three backends |
| `WriteStore` has ten methods and six implementors | true | `traits.rs:62-201`; impls at `mem.rs:1057`, `pg/write.rs:39`, `writer.rs:186`, `writer.rs:419`, `htui-agent/src/conformance.rs:616` (`UsageSpy`), `htui-agent/tests/recorder.rs:274` (`SpyStore`). `set_step_prompt` is the eleventh and touches all six |
| `CacheStore` implements `ReadStore` only, so `CASES` cannot exercise the mirror | true | `cache/read.rs:205`; `backend.rs:6` — "There is no `impl WriteStore for Backend`". This is what criterion 19's second clause and D96 exist for |
| The detail pane takes a sixth sub-tab without a `match` arm | true | `detail/mod.rs:102` `register`, `:4-7` states the design intent; five registrations at `backlog/mod.rs:65-69` |
| A detail sub-tab holds no store handle | true | `detail/mod.rs:52-54`, `runs.rs:5` — which is why D102's assembly is deferred work, not a sub-tab call |
| No UI anywhere contains "preview" | true | case-insensitive grep over `*.rs`, `*.snap`, `*.toml`, `*.sql` outside `target/`: zero hits |
| Today's fixture template bodies exercise one placeholder | true | `fixtures.rs:639` — `"You are running the \`{name}\` phase.\n\n{{item}}\n"`, eight names (`:548-558`), neither reserved name seeded |
| The demo `prompt` payload already uses the `{name, tokens, trimmed}` shape | true | `fixtures.rs:1239-1251`, with sections `prd` and `skills`; §5.2 renames the first `documents:prd` (T60) |
| Milestone-8 fixtures can calibrate the estimator | **false** | their `input_tokens` are 0 and 2 — the CLI's system prompt and cache reads dominate. Hence D99's differential design |
| Highest decision id in use is D94; highest task id is T58 | true | grep over `.claude/plans/mod-2-*.plan.md` |
| **Task independence** (file-set intersection, skill step 3.5) | verified | **T59** (`prompt/{mod,template,estimate}.rs`, `lib.rs`, `Cargo.toml`) ∩ **T61** (`model/{skill,link,box_,scope,mod}.rs`, `prompt/excerpt.rs`) = ∅; **T69** (`htui-agent/tests/estimator_live.rs`) ∩ both = ∅. So the parallel front is exactly **{T59, T61, T69}**. Everything after is serial: T60 ∩ T62 ≠ ∅ (`fixtures.rs`), T62 ∩ T63 ≠ ∅ (`writer.rs`, `pg_conformance.rs`), T64 ∩ T65 ≠ ∅ (`prompt/mod.rs`), T65 ∩ T66 ≠ ∅ (`prompt/mod.rs`, the residual budget), T67 ∩ T68 ≠ ∅ (`htui/tests/snapshots/*`), T62 ∩ T68 ≠ ∅ (`mem.rs`, `fixtures.rs`). **T59 and T64 also intersect** on `prompt/mod.rs` — T64 extends the surface T59 creates, so it is serial on it by construction, not merely by file |

---

## Acceptance

- [ ] All tasks complete — T59, T60, T61, T62, T63, T64, T65, T66, T67, T68, T69, T70
- [ ] ANA-5 §12 criteria **1–20** pass, each with a named test in the sweep table
- [ ] ANA-5 §12 criterion **21**: all five items closed by decision and evidence — D95 (`PromptScope`),
      D96 (`READ_CASES`, not a relaxed bound), D97 (`set_step_usage` keeps its parameter, with the
      reason), D98 (one excerpt rendering for both families), D99 (the constants measured, not
      assumed)
- [ ] `assemble()` is pure: same `PromptSpec`, same bytes, and the fan-out assertion greps the text
      for absolute paths, run ids and step ids rather than only comparing digests
- [ ] The preview renders a real item's prompt, sections, trim record and digest from the running
      binary, and **writes nothing**
- [ ] Store conformance: `CASES` 22 → 23 with both literals moved, `READ_CASES` green over `MemStore`,
      `PgStore` and `CacheStore`
- [ ] `cargo tree` shows no new workspace dependency (criterion 20); `htui-core` gains `sha2` and a
      dev-dep `insta`, both already in the lock
- [ ] No migration added — `0002_agent_probe.sql` already carries every ANA-5 section, and MOD-4's
      `0003_orchestration.sql` is still unheld
- [ ] `rust-reviewer` gate run over the full change set, findings applied or deferred with the
      maintainer (ultracode: one adversarial verifier per finding before applying)
- [ ] Validator green (0 errors); **MOD-2 closed out in full** — checklist line deleted,
      `docs/decisions/mod/mod-2.md` written across all nine phases, `DECISIONS.md` index line
      prepended, summary table and top status line updated; PRD milestone 9 row `complete` and its
      five open questions answered

## Status

**Drafted 2026-09-11, fact-checked, awaiting CONFIRM.**
