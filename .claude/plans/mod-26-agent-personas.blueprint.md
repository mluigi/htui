# Blueprint: MOD-26 — Declarative agent personas, milestone 1, T0–T5

**Status**: proposed (2026-10-02, code-architect). Implements
`.claude/plans/mod-26-agent-personas.plan.md` (CONFIRMED 2026-10-02, OQ-1 to OQ-6 as recommended,
fact-checked) under `.claude/prds/mod-26-agent-personas.prd.md`. The plan's D1-D13, I-1..I-7, task
order `T0 → {T1 ∥ T2 ∥ T3} → T4 → T5`, file sets and "Verified claims" are binding. Where this
blueprint had to choose, the choice is a **B-n**, driven by a finding **F-n**. Anything that would
move a confirmed decision is an **E-n** (§0b).

**Verified at**: `e72b6588` (`hr/MOD-26`, sandbox). `git diff e89b38b6 e72b6588` touches only the
plan and the PRD, so the plan's line numbers hold except where §0 says otherwise; every number below
was re-read at HEAD. Paths are relative to `crates/` unless they start with `docs/`, `.claude/` or
name a root file. Gortex answers symbol reads on this checkout despite its "INACTIVE" banner.
**The three wave lanes (T1, T2, T3) run in their own git worktrees, which Gortex does not index and
which its edit tools would not reach (they write the primary checkout): lanes read and edit with
native tools only.**

**House style (carried from MOD-41/MOD-42)**: `unsafe_code = "forbid"`; `missing_docs`,
`missing_debug_implementations`, `unused_qualifications` warn and clippy runs `-D warnings`; every
new `pub` item is documented and `Debug`; **no default body on a store trait**; `max_width = 100`;
edition 2024 (`if let … && …` chains are used, `traits.rs:1919-1923`); every commit compiles; red
first, then green, committed incrementally (uncommitted work dies with the session; **never
stash**). **E0034 hygiene**: no module `use`s `RecorderStore`, `RelayStore`, `WorkerStore` or
`WorkerHost`; the three new methods live on `WriteStore` alone, so plain method calls are
unambiguous, but a forwarding body on a type that implements two families stays UFCS if the
compiler asks. `every_cross_referenced_test_name_exists` (`htui-core/src/store/conformance.rs:14513`):
a backticked snake_case name with ≥ 4 underscores in that file's docs must be a fn there or in
`mem.rs`; a test elsewhere is spelled `pg_criteria.rs::name` **and must already exist**. **Never the
word `zeta`** in an identifier, fixture or seed (`htui-agent/tests/extensibility.rs` greps the
workspace). No test walks production parts against the real home. Every test command is
`--all-features` (without `testkit`, `htui-store`/`htui` integration tests compile to empty
binaries and print `ok. 0 passed`) and `--test-threads=1` (the keyring fake is process-wide).
`serde_json::Value` is never used to hash anything (`graph.rs:259-264`, the `preserve_order` trap).

**Layout**: §0 findings · §0a decisions · §0b escalations · §1 build order and lanes · §2 shared
shapes (2.1-2.14) · §3 T0 · §4 T1 · §5 T2 · §6 T3 · §7 wave merge · §8 T4 · §9 T5 · §10 lane rules ·
§11 pins · §12 gate reference.

---

## 0. Findings

| # | Severity | Plan says | Tree / evidence | Fix |
|---|---|---|---|---|
| **F-1** | Minor (T0) | D8: "Hand-rolled: the workspace has no YAML dependency" — read as a new parser. | A hand-written, dependency-free frontmatter reader already exists: `htui-core/src/model/frontmatter.rs` (ANA-22 §5.7), used by `skill_import.rs:22`. `split` (`:224-249`) answers `Split { frontmatter: Vec<Entry>, body, body_at, issues }`; a missing or unterminated fence is `FrontmatterError::{NoFence, Unterminated}` (`:92-104`); every other problem is a non-fatal `Issue`. It accepts more than D8's grammar: flow lists, block lists, block scalars, nested maps (`read_value` `:334-357`). | **B-1** |
| **F-2** | Minor (T0, T1) | D3: "pure helpers in `store/traits.rs`, worded once for both stores". T0 Action: "`model/persona.rs` types (D2) **with the D3 pure validators** and the D8 parser"; T0's file set has no `traits.rs`. | The parser (T0) must run D3 (I-2: a seed is validated when parsed); `traits.rs` is T1's. `has_nul` (`traits.rs:1899-1901`) and `validate_name` (`model/skill.rs:161-170`) are reusable from `model`. | **B-2** |
| **F-3** | Minor (T0, T1) | D7: "`fixtures` gains the two rows". `fixtures.rs` is in T0's set only (`:1345`, `:1379`). | `DemoData` (`fixtures.rs:344-406`) is read field by field by `MemStore::from_demo` (`mem.rs:292-367`) and `PgStore::load_demo`, never destructured, so a new field compiles unread. The two other literals spread `..demo_data()` (`htui/src/store_worker.rs:2862`, `:2923`). The fixture registry is derived from the seed (`agents()`, `fixtures.rs:503-516`). | **B-3** |
| **F-4** | Major (T4) | D12: the lookup lives "in stage 3 (`phase_spec` / `assemble_prompt`, `engine.rs:5276-5505`)". | `phase_spec` (`:5376`) answers `Result<Result<PromptSpec, String>, EngineError>`, the inner `String` being a missing input kind. Callers: `assemble_prompt` (`:5285`), promotion's `opening` (`:1367`, `strict = false`) and eight `engine.rs` tests (`:13319`, `:13505`, `:13558`, `:14017`, `:14204`, `:14313`, `:14403`, `:14523`); `assemble_prompt` has two production (`:3424`, `:3772`) and seven test callers (`:9594`, `:13287`, …, `:13743`). Every test unwraps the inner `Result` with `.expect(…)`, which needs only `Debug`. | **B-4** |
| **F-5** | Major (T4) | D12: "absent → the I-4 refusal through `refuse_prompt`/`fail_before_a_token` (`:5214`, `:5183`)". | `fail_before_a_token` always writes `RunFailure::MissingInput` ("missing input document: …", `status.rs:107`), the wrong sentence. `refuse_prompt` takes `&AssembleError` (`:5214-5220`), moves the step `running → failed`, `block_and_fail`s (item `blocked`, note, `finish_run(Failed)`, cleanup) and answers `Ok(Rest { run: Failed, … })` (`:5221-5241`): the walk settles and returns, it never raises. `drive_group`'s twin is `fail_group_before_a_token(…, block: true)` (`:3790-3800`). | **B-5** |
| **F-6** | Major (T4) | D12: `drive_once` "callers `:4078` candidate, `:4974` judge with `None`, `:5680` session". | `:5680` is inside `session()` (`:5662`), whose only caller is `walk_live_step` (`:3472`); `:4078` is inside `run_candidate(stage: CandidateStage)` (`:3942`), and `CandidateStage` (`:6169-6180`) carries no snapshot. `drive_once` is `#[allow(clippy::too_many_arguments)]` already (`:5756-5760`). | **B-6** |
| **F-7** | Minor (T3) | D13: "`Trimmer::source_of`'s exhaustive match (`trim.rs:503-507`) gains a fixed-source arm". | `trim_diff` (`trim.rs:795`) holds a second exhaustive `SectionName` match (`:801-822`, "Enumerated rather than `_ =>`", L-3). No other exhaustive `SectionName`/`Placeholder` match exists outside `prompt/` (searched `SectionName::CommandQueue`, `Placeholder::FailureReason`, `Placeholder::Attempt`). | **B-7** |
| **F-8** | Minor (T2) | I-1: "effective `allow` ⊆ base (an empty base means 'everything')". | Two non-empty, disjoint `allow` lists intersect to `[]`, which `ToolExposure` reads as "no allow-list: everything" (`driver.rs:207-208`) — a widening. Latent in M1 (the engine's base is always `ToolExposure::default()`, `engine.rs:5797`) but `narrow` is a pure function with its own I-1 tests. | **B-8** |
| **F-9** | Note (M2) | D3's refusal list. | A rule matcher's `tool_kind` is compared as a string (`permission.rs:124-128`), so a misspelt kind never matches and a persona reject rule would silently do nothing. M1 has no author of rules (D8: not expressible in frontmatter; no TUI, no `StoreRequest`), only tests write one. | Not acted on (it would extend D3's confirmed list). Recorded for M2's rule form, which should offer `TOOL_KINDS` as a closed list. |
| **F-10** | Minor (T0, T1) | D3: "no NUL". | Postgres `JSONB` refuses `\u0000` inside a string (`22P05`), so a NUL in `tools`/`permission` would be a raw `Backend` error on Pg and a stored row on MemStore. | **B-9** |
| **F-11** | Minor (T0) | D9: `GraphSnapshot.personas` `#[serde(default)]`. | `create_run` serialises the typed snapshot into `run.graph_snapshot` (`pg/write.rs:3636`); an always-written `"personas": []` would change every new run's stored JSON. `SnapshotPhase` is also built from `serde_json::from_value(json!{…})` with no persona key (`htui/src/run_worker.rs:1188-1191`, `htui-orch/src/fanout.rs:374-395`), so the field must default. | **B-10** |
| **F-12** | Note (T0) | D9: "the doc at `:251-270` is rewritten". | `topology`'s doc says "nulls are emitted because no field carries `skip_serializing_if`" (`graph.rs:254-257`) — false from the commit that adds `SnapshotPhase.persona`, which is T0's; `graph.rs` is in T0's set. | **B-11** |
| **F-13** | Note (T2) | D11: block `cli/mod.rs:138-159`, `extra_args` `:161`. | At HEAD: `argv` `:112`, the scoped block `:137-158` (closure `:138`), `args.extend(cli.extra_args.iter().cloned())` `:160`. `claude::tool_kind` is `cli/claude.rs:465-474` as the plan says. | Line numbers only |
| **F-14** | Minor (T2) | D11: the ACP refusal comes "before the write arm's `EditProposal{accepted: Some(true)}` emit (`:1508-1525`)". | No test drives `fs/read_text_file`/`fs/write_text_file` through a live session today. `acp_driver.rs` already hosts raw JSON-RPC agents over a `DuplexStream` (`refuse_first_request` `:208`, `refuse_session_new` `:252`). `on_inbound` (`acp/mod.rs:1453-1550`) gets `filesystem` but no spec; `session_main` (`:1060`, `spec` `:1062`) calls it at `:1265`. `wire_enum!` gives `ToolKind` `ALL`/`as_str`/`Display` but **no `FromStr`** (`lib.rs:55-97`). | **B-12**, **B-19** |
| **F-15** | Minor (T4) | T4 test "the relay rejects a denied kind end to end (`ScriptedStep::parks` + `set_policy`)". | `ScriptedStep::parks` always emits an `execute` call (`htui-orch/src/fake.rs:1097-1110`); both seeds keep `execute` (OQ-4). | **B-13** |
| **F-16** | Minor (T1) | T1 tests first: "`resolve_graph` returns the bound persona" in store conformance. | Store conformance is generic over `S: WriteStore` (`conformance.rs:179`); `resolve_graph` is inherent on both stores (`mem.rs:662`, `pg/read.rs:1726`) and a `WorkerHost` method (`store/worker.rs:387`), never on `WriteStore`. Pg/Mem agreement on it is pinned by `pg_criteria.rs::inherent_orchestration_reads_answer_the_fixture` (`:3734-3760`). | **B-14** |
| **F-17** | Minor (T1) | D4: "PgStore (`pg/write.rs`; a `persona_insert_refused` mapper…)". | `query_as!` cannot decode a `JSONB` column into `PersonaTools` (no `sqlx::Type`); MOD-42 decodes `Json<T>` into a private record (`pg/relay.rs:35-91`). Skill readers live in `pg/read.rs` (`skill_rows` `:2413`, `skill_row` `:2437`), row structs in `pg/rows.rs` (`SkillBindingRow` `:260`). A private fn of `write.rs` (`cas_miss` `:87`) is not visible to `pg/mod.rs`. | **B-15** |
| **F-18** | Minor (T1) | T1 Pins: "a new `MOD26_COLUMN_COMMENTS` list chained into both chains of the column-comment check (`:455-544`)". | There are **three** chains (`migrations.rs:460-465`, `:516-521`, `:531-536`), two count texts ("thirty-five … thirty-sixth" `:505-506`, `:542`) and one message naming the lists (`:482`). `step_graph_phase` is already a checked table (`0002:59`, `0003:24-33`), so `persona_id`'s comment must be listed, with all eight `persona` columns (D1: every new column): 35 + 9 = 44. | §11 |
| **F-19** | Note (T1) | D7: "the agent precedent, `demo.rs:30-35`". | `demo.rs:30-35` is the skill paragraph of the doc; the agent delete-by-name is `:148-163`, the agent loop `:165-185`, graphs `:187`, phases `:203-231`. | Line numbers only |
| **F-20** | Minor (T0) | D8: "then the body (one leading newline trimmed)". | `split` already drops the fence's own newline, keeps the body's leading blank lines, normalises CRLF and ends a non-empty body with exactly one LF (`frontmatter.rs:250-263`). | **B-16** |
| **F-21** | Note (T3) | R-6: "T3 decides, test pins". | The frame estimate is `est.estimate(&masked.literals.concat())` (`prompt/mod.rs:525`); section tokens are `estimate(wrap(section)) * weight` (`trim.rs:453`). | **B-17** |
| **F-22** | Minor (T4) | D9: the freeze is "in `snapshot_phase`/`graph::resolve` (`graph.rs:298-382`, `:636-721`)". | `resolve` (`:298-383`) owns the phase loop (`:331-347`) and the `GraphSnapshot` literal (`:364-376`); `snapshot_phase` (`:636-721`, literal `:675`) sees only `&StepGraphPhase`. A `persona_id` with `ResolvedPhase.persona == None` is unreachable on Pg (FK `RESTRICT`, no delete in M1) and on MemStore (checked on every write), but `resolve` must not freeze such a phase un-narrowed. | **B-18** |
| **F-23** | Note (T4) | R-4. | `every_case_name_dispatches` (`htui-orch/src/conformance.rs:6829`) sits near the 2 MiB debug stack (auto-memory `htui-orch-test-stack-headroom`); T4 adds five `run_case` arms. | §8.5: box each new case future |
| **F-24** | Note (T1) | — | `seed_if_empty_as` lives in `pg/mod.rs:437-515` and imports `htui_core::model::agent::seed_rows` by name (`pg/mod.rs:17`); a second `seed_rows` must be path-qualified. | §2.8 |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B-1** (F-1): `model::persona::parse_file` **wraps** `frontmatter::split` and narrows it to D8's
  grammar (§2.2): any `Issue` is a refusal (fail closed); every value must be a `Value::Scalar`
  with no newline (a flow list, a block list, a block scalar or a nested map is refused by key);
  list values are split on commas by the persona parser. A quoted scalar (`"…"`, `'…'`) is
  accepted: it is still one `key: value` line. No second reader is written.
- **B-2** (F-2): the D3 rules and their sentences are pure functions in `model/persona.rs` (T0),
  called by the parser. T1 re-exports them from `store/traits.rs` (`pub use
  crate::model::persona::{…}`) and adds them to `store/mod.rs`'s `traits::{…}` list, so D3's
  "pure helpers in `store/traits.rs`, worded once" holds by path. NUL sentences reuse
  `crate::store::has_nul`.
- **B-3** (F-3): T0 adds `DemoData.personas: Vec<Persona>`, `class::PERSONA = 20`,
  `ids::{PERSONA_REVIEWER, PERSONA_ARCHITECT}` and `fixtures::personas()` (the seed re-stamped with
  fixture ids, `agents()`'s length-checked shape). T1 loads it in `from_demo` and `load_demo`.
- **B-4** (F-4): `phase_spec`'s inner error becomes `StageThree`, which gains `NoPersona(String)`.
  The persona lookup is the first statement of `phase_spec`, through the pure
  `GraphSnapshot::persona_for` (T0): `strict` refuses with `NoPersona`, non-strict (promotion)
  proceeds with `None` (OQ-5). `assemble_prompt` passes the inner error through unchanged. No test
  call site changes.
- **B-5** (F-5): `refuse_prompt(run, step, phase, reason: String)`; I-4 is
  `RunFailure::PromptRefused { phase, reason: persona_not_in_snapshot(name) }`, rendered "prompt
  refused at \`<phase>\`: persona \`<name>\` is not in the run's snapshot, so the step does not run
  un-narrowed (MOD-26 I-4)". **Verified: it fails the step and settles the run (`Ok(Rest)`), it
  never returns `Err` and never aborts the walk.** No new `RunFailure` variant; `status.rs` is
  untouched.
- **B-6** (F-6): `drive_once(…, phase, persona: Option<&SnapshotPersona>, key, …)`,
  `session(…, phase, persona, prompt, …)`, `CandidateStage.persona: Option<&'s SnapshotPersona>`.
  `walk_live_step` and `drive_group` re-read the persona after stage 3 with
  `snapshot.persona_for(phase)`; its `Err` (unreachable once stage 3 passed) is
  `EngineError::Snapshot`, never a silent `None`. The judge passes `None` (OQ-2).
- **B-7** (F-7): `SectionName::Persona` joins **both** exhaustive matches in `trim.rs`:
  `Source::Fixed` in `source_of`, `None` in `trim_diff`.
- **B-8** (F-8): `narrow`'s allow rule: base empty → persona's; persona empty → base's; both
  non-empty → `base ∩ persona` in base order, **and every base name outside persona's list is
  appended to `deny`**; if the intersection is empty, `allow` stays the base list (every entry of
  which is now denied). The CLI then gets `--tools=a,b --disallowedTools=a,b`: no tool, never all.
- **B-9** (F-10): D3's NUL rule covers every string of both blobs: tool names (the tool-name rule
  forbids NUL), `deny_kinds` (closed list), every rule matcher string and `reason`
  (`has_nul("persona.permission.rules")`).
- **B-10** (F-11): `SnapshotPhase.persona`: `#[serde(default, skip_serializing_if =
  "Option::is_none")]` (D9 as written); `GraphSnapshot.personas`: `#[serde(default,
  skip_serializing_if = "Vec::is_empty")]`, so a persona-less run's stored snapshot is byte-identical
  (I-7). Neither is hashed by `topology` except `SnapshotPhase.persona` when bound (D9).
- **B-11** (F-12): T0 rewrites `topology`'s doc paragraph (`graph.rs:251-257`) in the same commit
  that adds the field.
- **B-12** (F-14): ACP: a new `pub const TOOL_KIND_DENIED: &str = "tool_kind_denied"` beside
  `PATH_OUTSIDE_SESSION` (`acp/mod.rs:76`). The deny check is the **first** thing each fs arm does,
  before `filesystem.guard`: a denied kind is refused whatever the path, the file is never read or
  written, no `EditProposal` is emitted. `session_main` clones `spec.tools.deny_kinds` beside
  `filesystem` and passes `&[ToolKind]` to `on_inbound`. Tests use a raw JSON-RPC `fs_agent` in
  `acp_driver.rs` (§5.4).
- **B-13** (F-15): T4's end-to-end relay case creates its own persona `no-shell` (`deny_kinds:
  ["execute"]`) through `create_persona`; `fake.rs` is untouched.
- **B-14** (F-16): D6 is pinned by a `mem.rs` unit test and `pg_criteria.rs`
  `resolve_graph_carries_the_bound_persona_as_mem_store_does`; store `CASES` gains the five
  `WriteStore` cases of §4.3 (119 → 124). `pg_criteria.rs` joins T1's files.
- **B-15** (F-17): `PersonaRow` (with `Json<PersonaTools>`, `Json<PersonaPermission>`) and its
  `From` in `pg/rows.rs`; readers `persona_rows`, `persona_row`, `persona_rows_by_id` in
  `pg/read.rs`; the three writers and the three mappers in `pg/write.rs`; the seed insert in
  `pg/mod.rs` serialises its own JSON. `error.rs` is untouched. `pg/rows.rs` joins T1's files.
- **B-16** (F-20): a persona body is `split.body` with **at most one** leading `\n` removed; CRLF
  is LF and the body ends with exactly one LF (the reader's rules). The prompt renderer trims
  trailing LFs anyway (`render.rs:156-159`).
- **B-17** (F-21): the `"\n\n"` separator (`PERSONA_SEPARATOR`) is counted **in the frame**: when a
  persona section renders, `template_tokens = est.estimate(&(literals.concat() +
  PERSONA_SEPARATOR))`; persona-less, the expression is today's, byte for byte (I-7). Pinned by
  `the_separator_is_counted_in_the_frame` (§6.3).
- **B-18** (F-22): `snapshot_phase` keeps its signature (T0 adds `persona: None` to its literal).
  `resolve` sets `phase.persona` after each `snapshot_phase` call through a private
  `frozen_persona(row, &mut personas)`; one `SnapshotPersona` per name, sorted by name bytes; a
  `persona_id` with no `persona` is `ResolveError::Store(Constraint(references_no_row(
  "step_graph_phase.persona_id", id, "persona")))` at `StartRun`.
- **B-19** (F-14): `htui_agent::persona::tool_kind_of(&str) -> Option<ToolKind>` over
  `ToolKind::ALL`. In `narrow`, a `deny_kinds` string that does not parse (impossible after D3) is
  `ToolKind::Other`: denied, never dropped.
- **B-20**: the persona section renders `<section name="persona" persona="<name>">`, the body as
  content (`render::content_of`), attribute escaped by `render::attr`.
- **B-21**: a persona rule with an empty `reason` records `persona <name>`; a `deny_kinds` rule
  records `persona <name> denies <kind>` (D10).
- **B-22**: `StepGraphPhase.persona_id` and `ResolvedPhase.persona` are `#[serde(default)]`;
  `persona_id` is declared just before `updated_at`. `PhasePatch.persona` is `#[serde(default)]`.
- **B-23**: `SnapshotPersona::freeze` returns `Result<_, serde_json::Error>` (`topology`'s
  no-panic-on-a-production-path rule, `graph.rs:265-270`); `resolve` maps it to
  `ResolveError::Store(Constraint(…))`.
- **B-24**: the persona frontmatter error is a typed `PersonaFileError` (§2.2) whose `Display` is
  the refusal sentence, so M2's import can name the line and key; the store never sees it.
- **B-25**: the wave lanes run in three worktrees (maintainer direction for this run), each from
  T0's last commit, merged `--no-ff` in the order T1, T2, T3 (§7, §10).

### 0b. Escalations for the maintainer

None. F-9 is recorded and deliberately not acted on, because acting would extend D3's confirmed
refusal list; B-2 keeps D3's "helpers in `store/traits.rs`" by re-export; B-4/B-5/B-6 choose
between shapes D12 leaves open and keep its outcome (stage-3 lookup, named refusal, step failed, walk
settled); B-8 and B-9 are I-1 and D3 applied to cases the plan did not spell out; every file a lane
adds (`pg/rows.rs`, `pg_criteria.rs`, `store/mod.rs` for T1) is inside that lane's crate files, so
the plan's file-set intersections stay empty.

---

## 1. Build order

| Task | Crates | Commits (each compiles) | Lane | Gate |
|---|---|---|---|---|
| T0 | core, agent, orch, store (test literal), htui (one literal) | (1) red: ids, `model/persona.rs` shapes with red bodies, seeds, snapshot fields, `PromptSpec.persona`, `ToolExposure.deny_kinds`, `judge_phase`/`handoff_spec` `None`, every literal, all T0 tests; (2) green: parser, validators, `seed_rows`, `freeze`, `persona_for`, fixture personas, `topology` doc | primary `hr/MOD-26`, alone | G-T0 |
| T1 | core (`model/kind.rs`, `seed.rs`, `store/*`), store, agent (two spies), htui (one literal) | (1) red: binding fields + literals, `0012`, four phase statements, `WriteStore` methods with red bodies in MemStore/PgStore, forwards, all T1 tests and pins, `.sqlx`; (2) green MemStore; (3) green Postgres, `.sqlx` | worktree `hr/MOD-26-t1` | G-T1 |
| T2 | agent | (1) red: `persona.rs` with an identity `narrow`, `tool_names` empty, ACP threading with no refusal, the constant, all T2 tests; (2) green | worktree `hr/MOD-26-t2` | G-T2 |
| T3 | core (`prompt/*`) | (1) red: `SectionName::Persona`, `Placeholder::Persona`, `render::persona`, match arms, `PERSONA_SEPARATOR`, `tests/prompt_persona.rs`; `assemble` ignores the persona; (2) green: render, scrub, protect, substitute, separator | worktree `hr/MOD-26-t3` | G-T3 |
| — | — | merge `t1`, `t2`, `t3` (`--no-ff`, that order), then G-W1 with no lane running | primary | G-W1 |
| T4 | orch | (1) red: signatures (`phase_spec`/`StageThree`, `refuse_prompt`, `session`, `drive_once`, `CandidateStage`), all T4 tests and the `CASES` pin; persona never looked up, never frozen; (2) green: freeze, lookup, frame, narrowing | primary | G-T4 |
| T5 | docs | (1) docs and PRD/plan status | primary | G-Final |

**Why three worktrees work.** The three lanes' file sets are disjoint (plan "Task independence",
re-checked with B-3/B-14/B-15's additions), each lane only needs T0's shapes, and each lane's gate
compiles its own crates against T0 plus itself. T1 is the only lane that touches Postgres and the
only one that writes `.sqlx`. Each worktree builds its own `target/` (≈ 10 GB; 131 GB free at HEAD).

---

## 2. Shared code shapes

Everything in this section crosses a task boundary and is settled here. A lane that believes a
shape here is wrong stops and reports to the main thread; it does not edit another lane's files.

### 2.1 Ids and model wiring (T0)

`htui-core/src/model/ids.rs`: one entry appended to the `id_newtype!` list after `RelaySessionId`
(`:125`):

```rust
    /// `persona.id` (MOD-26 plan D1): one agent persona of the global registry.
    PersonaId,
```

`htui-core/src/model/mod.rs`: `pub mod persona;` between `pub mod overlap;` (`:91`) and `pub mod
quota;`; `PersonaId` added to the `pub use ids::{…}` list (`:116-120`, alphabetical); and

```rust
pub use persona::{
    NewPersona, Persona, PersonaAnswer, PersonaDefault, PersonaFile, PersonaFileError, PersonaMatch,
    PersonaPatch, PersonaPermission, PersonaRule, PersonaTools, SnapshotPersona,
};
```

### 2.2 `htui-core/src/model/persona.rs` (new, T0)

Module doc: MOD-26 milestone 1 (plan D2, D3, D8, D9; PRD Q-model, Q-seeds): the registry row, its
save-time rules, the frontmatter reader of a persona file and the two seeds. Imports:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::frontmatter::{self, FrontmatterError, Value};
use crate::model::ids::PersonaId;
use crate::model::skill::validate_name;
use crate::store::has_nul;
```

**Constants.**

```rust
/// The ten ACP tool kinds, spelled and ordered as `htui_agent::event::ToolKind::ALL` spells them.
/// Core cannot name the agent crate (`htui-agent → htui-core`); `htui_agent::persona`'s
/// `core_tool_kinds_spell_tool_kind_all` pins the two together (plan D2, D10).
pub const TOOL_KINDS: [&str; 10] = [
    "read", "edit", "delete", "move", "search", "execute", "think", "fetch", "switch_mode", "other",
];

/// The kinds a persona may deny (plan D3). `think`, `switch_mode` and `other` are not narrowable.
pub const NARROWABLE_KINDS: [&str; 7] =
    ["read", "edit", "delete", "move", "search", "execute", "fetch"];

/// The frontmatter keys a persona file may carry (plan D8), in the order a refusal lists them.
pub const FRONTMATTER_KEYS: [&str; 7] = [
    "name", "description", "tools", "disallowed-tools", "deny-kinds", "command-run",
    "permission-default",
];

/// The prefix of an MCP tool's name, which `allow` refuses (plan D3: `--tools` filters built-ins).
pub const MCP_PREFIX: &str = "mcp__";
```

**Types.** Every shape is `deny_unknown_fields` (I-2).

```rust
/// A row of `persona` (MOD-26 plan D1, D2): one named posture of the global registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Persona {
    /// `persona.id`.
    pub id: PersonaId,
    /// `persona.name`: unique, [`validate_name`]'s alphabet.
    pub name: String,
    /// `persona.description`: the picker's one-liner; never rendered into a prompt.
    pub description: String,
    /// `persona.body`: the role text stage 3 renders ahead of the template (plan D13).
    pub body: String,
    /// `persona.tools`.
    pub tools: PersonaTools,
    /// `persona.permission`.
    pub permission: PersonaPermission,
    /// `persona.created_at`.
    pub created_at: DateTime<Utc>,
    /// `persona.updated_at`: the compare-and-set token of `update_persona`.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of `WriteStore::create_persona` (plan D4): the row minus the store's two stamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPersona {
    /// `persona.id`, minted client-side as a UUIDv7.
    pub id: PersonaId,
    /// `persona.name`.
    pub name: String,
    /// `persona.description`; may be empty.
    pub description: String,
    /// `persona.body`; refused when blank.
    pub body: String,
    /// `persona.tools`.
    pub tools: PersonaTools,
    /// `persona.permission`.
    pub permission: PersonaPermission,
}

/// Edit passed to `WriteStore::update_persona` (plan D4); `None` leaves the column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaPatch {
    /// `persona.name`; a rename is allowed.
    pub name: Option<String>,
    /// `persona.description`.
    pub description: Option<String>,
    /// `persona.body`.
    pub body: Option<String>,
    /// `persona.tools`, replaced whole.
    pub tools: Option<PersonaTools>,
    /// `persona.permission`, replaced whole.
    pub permission: Option<PersonaPermission>,
}

/// `persona.tools` (plan D2, D10): what a persona takes away from the agent row's exposure.
/// `{}` decodes to [`PersonaTools::default`], which narrows nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonaTools {
    /// Built-in tool names to keep; empty keeps every tool the base keeps (`--tools`).
    pub allow: Vec<String>,
    /// Tool names to remove (`--disallowedTools`); MCP tools are named here.
    pub deny: Vec<String>,
    /// ACP tool kinds to deny, each one of [`NARROWABLE_KINDS`].
    pub deny_kinds: Vec<String>,
    /// `false` withdraws `htui`'s `command_run` exposure and the `command_queue` section; `true`
    /// keeps the base (plan D13).
    pub command_run: bool,
}

impl Default for PersonaTools {
    fn default() -> Self {
        Self { allow: Vec::new(), deny: Vec::new(), deny_kinds: Vec::new(), command_run: true }
    }
}

/// `persona.permission` (plan D2, D10): deny-only additions to the agent row's policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonaPermission {
    /// The persona's floor for an unmatched request; `None` keeps the base's.
    pub default: Option<PersonaDefault>,
    /// Reject rules, evaluated before the agent row's rules and remembered choices.
    pub rules: Vec<PersonaRule>,
}

/// A persona's `permission.default` (plan D2): never `allow`, by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaDefault {
    /// Park and ask.
    Ask,
    /// Answer with the first reject option.
    Deny,
}

/// One persona rule (plan D2): a matcher and a reject-only answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaRule {
    /// The predicate; named `match` on the wire, as `PermissionRule`'s is.
    #[serde(rename = "match")]
    pub matcher: PersonaMatch,
    /// The reject kind to answer with.
    pub answer: PersonaAnswer,
    /// Why the rule exists; empty records `persona <name>` (B-21).
    #[serde(default)]
    pub reason: String,
}

/// `PermissionMatch`'s four fields (plan D2), core-side.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonaMatch {
    /// Matches `tool_call.tool_kind`, spelled as [`TOOL_KINDS`] spells it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    /// Matches the call's title (ACP carries no tool name, `permission.rs:110-113`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Matches the first path argument by prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    /// Matches the first command argument by prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_prefix: Option<String>,
}

/// A persona rule's answer (plan D2): reject-only, by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaAnswer {
    /// `reject_once`.
    RejectOnce,
    /// `reject_always`.
    RejectAlways,
}

/// One persona as a run froze it at `StartRun` (plan D9, I-3): content only, no id, no stamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPersona {
    /// `persona.name`, what `SnapshotPhase.persona` names.
    pub name: String,
    /// `"sha256:"` + hex over the canonical JSON of `{name, body, tools, permission}`.
    pub digest: String,
    /// `persona.body`.
    pub body: String,
    /// `persona.tools`.
    pub tools: PersonaTools,
    /// `persona.permission`.
    pub permission: PersonaPermission,
}

/// A persona file read by [`parse_file`]: a [`NewPersona`] without its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaFile {
    /// `name`.
    pub name: String,
    /// `description`, `""` when absent.
    pub description: String,
    /// The body (B-16).
    pub body: String,
    /// `tools`, `disallowed-tools`, `deny-kinds`, `command-run`.
    pub tools: PersonaTools,
    /// `permission-default`; rules are not expressible in a file (plan D8).
    pub permission: PersonaPermission,
}

impl PersonaFile {
    /// The insert this file describes, under `id`.
    #[must_use]
    pub fn into_new(self, id: PersonaId) -> NewPersona { /* field moves */ }
}
```

`SnapshotPersona::freeze` and the digest (B-23):

```rust
impl SnapshotPersona {
    /// Freezes `persona` for a run's snapshot (plan D9).
    ///
    /// # Errors
    /// The serialiser's own, which these types cannot produce (no map keys, no floats).
    pub fn freeze(persona: &Persona) -> Result<Self, serde_json::Error> { … }
}

/// The digest's input, serialised **typed** in this field order — never through `Value`.
#[derive(Serialize)]
struct Canonical<'a> {
    name: &'a str,
    body: &'a str,
    tools: &'a PersonaTools,
    permission: &'a PersonaPermission,
}
// digest = format!("sha256:{}", crate::prompt::digest::sha256_hex(&serde_json::to_string(&c)?))
```

**Refusal sentences (exact wording).**

```rust
/// PRD Q-model, plan D8 (I-5).
pub const MODEL_REFUSED: &str =
    "a persona does not set the model; the phase candidate's model is used (MOD-26)";

/// Plan D3: a blank body would render an empty frame that costs tokens and says nothing.
pub const BLANK_PERSONA_BODY: &str = "a persona needs a prompt body";

/// Plan D3: the all-`None` matcher, which would reject every request.
pub const RULE_MATCHES_EVERYTHING: &str =
    "a persona rule with an empty match would deny every request; use `default: deny` instead";

/// Plan D3: a name [`validate_name`] refuses.
#[must_use]
pub fn invalid_persona_name(name: &str) -> String {
    format!(
        "persona.name `{}` must be 1-64 of a-z, 0-9 and single inner hyphens",
        name.escape_debug()
    )
}

/// Plan D3: `list` is `allow` or `deny`; each entry becomes part of one argv value (D11).
#[must_use]
pub fn not_a_tool_name(list: &str, name: &str) -> String {
    format!(
        "persona.tools.{list} entry `{}` is not a tool name: one or more characters, no \
         whitespace, comma or NUL",
        name.escape_debug()
    )
}

/// Plan D3: `--tools` filters built-in tools only.
#[must_use]
pub fn allow_names_an_mcp_tool(name: &str) -> String {
    format!(
        "persona.tools.allow entry `{}` is an MCP tool; `allow` keeps built-in tools only, so \
         deny an MCP tool by name instead",
        name.escape_debug()
    )
}

/// Plan D3: a `deny_kinds` entry outside [`NARROWABLE_KINDS`].
#[must_use]
pub fn kind_not_narrowable(kind: &str) -> String {
    format!(
        "persona.tools.deny_kinds entry `{}` is not one of read, edit, delete, move, search, \
         execute, fetch",
        kind.escape_debug()
    )
}

/// I-4 (plan D12): a phase names a persona its run's snapshot does not carry.
#[must_use]
pub fn persona_not_in_snapshot(persona: &str) -> String {
    format!(
        "persona `{persona}` is not in the run's snapshot, so the step does not run un-narrowed \
         (MOD-26 I-4)"
    )
}
```

NUL sentences are `has_nul("persona.description")`, `has_nul("persona.body")`,
`has_nul("persona.permission.rules")` ("`<column>` must not contain a NUL character").

**The D3 rules** (order is the refusal order; first failure wins):

```rust
/// Plan D3 over a whole row: name; description NUL; body blank, body NUL; tools; permission.
#[must_use]
pub fn persona_refusal(
    name: &str,
    description: &str,
    body: &str,
    tools: &PersonaTools,
    permission: &PersonaPermission,
) -> Option<String> {
    if !validate_name(name) {
        return Some(invalid_persona_name(name));
    }
    if description.contains('\0') {
        return Some(has_nul("persona.description"));
    }
    body_refusal(body).or_else(|| tools_refusal(tools)).or_else(|| permission_refusal(permission))
}

/// [`persona_refusal`] over a [`NewPersona`] (`create_persona`).
#[must_use]
pub fn new_persona_refusal(new: &NewPersona) -> Option<String> { … }

/// Plan D3 over a [`PersonaPatch`]: each `Some` field through the same rule, in the same order.
#[must_use]
pub fn persona_patch_refusal(patch: &PersonaPatch) -> Option<String> { … }

fn body_refusal(body: &str) -> Option<String> {
    if body.trim().is_empty() {
        return Some(BLANK_PERSONA_BODY.to_owned());
    }
    body.contains('\0').then(|| has_nul("persona.body"))
}

/// allow entries (tool name, then MCP prefix), deny entries (tool name), deny_kinds (closed list).
fn tools_refusal(tools: &PersonaTools) -> Option<String> { … }

/// One rule at a time: the all-`None` matcher, then a NUL in any matcher string or the reason.
fn permission_refusal(permission: &PersonaPermission) -> Option<String> { … }

/// Non-empty, no `char::is_whitespace`, no `,`, no `\0`.
fn is_tool_name(name: &str) -> bool { … }
```

`default` ∈ {ask, deny} and reject-only answers hold by type and need no rule.

**Frontmatter grammar (plan D8, exact; B-1, B-16).**

```
file      = fence LF *( entry-line LF ) fence [ LF body ]   ; a leading U+FEFF is skipped
fence     = "---" *SP                                          ; the reader's `is_fence`
entry-line= blank / key ":" *SP value
key       = "name" / "description" / "tools" / "disallowed-tools" / "deny-kinds"
          / "command-run" / "permission-default"
value     = one-line scalar: plain, "double-quoted" or 'single-quoted'; no LF after unquoting
list      = item *( "," item )        ; tools, disallowed-tools, deny-kinds: items trimmed,
                                      ; empty items dropped (`tools:` alone is the empty list)
command-run        = "true" / "false"
permission-default = "ask" / "deny"
body      = the rest of the file; CRLF → LF; at most one leading LF removed; exactly one trailing LF
```

Refusals, checked in file order (first failure wins): `model` → `MODEL_REFUSED`; any other key
outside `FRONTMATTER_KEYS` → by name; a key twice; a value that is not a one-line scalar; after the
loop, no `name`; finally `persona_refusal` on the result. Permission rules are not expressible in
a file (D8).

```rust
/// Why a persona file was refused (B-24). `Display` is the sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PersonaFileError {
    /// No opening fence, or no closing fence within `MAX_FRONTMATTER_LINES`.
    #[error(transparent)]
    Fence(#[from] FrontmatterError),
    /// The reader could not read a line (any `frontmatter::Issue`; fail closed).
    #[error("persona frontmatter line {line}: {message}")]
    Unreadable { line: usize, message: String },
    /// `model:` (PRD Q-model).
    #[error("a persona does not set the model; the phase candidate's model is used (MOD-26)")]
    Model,
    /// Any other key D8 does not name.
    #[error(
        "`{key}` is not a persona key; a persona file takes name, description, tools, \
         disallowed-tools, deny-kinds, command-run and permission-default"
    )]
    UnknownKey { key: String },
    /// A key written twice.
    #[error("persona key `{key}` appears more than once")]
    Duplicate { key: String },
    /// A list, block or map value.
    #[error("persona key `{key}` takes one `key: value` line; write a list as `a, b, c`")]
    NotOneLine { key: String },
    /// No `name:` line.
    #[error("a persona file needs a `name`")]
    MissingName,
    /// `command-run` other than `true`/`false`.
    #[error("`command-run` is `true` or `false`, not `{value}`")]
    CommandRun { value: String },
    /// `permission-default` other than `ask`/`deny`.
    #[error("`permission-default` is `ask` or `deny`, not `{value}`")]
    PermissionDefault { value: String },
    /// A D3 refusal of the parsed persona.
    #[error("{0}")]
    Refused(String),
}

/// Reads one persona file (plan D8): the seeds now, M2's import later.
///
/// # Errors
/// Every [`PersonaFileError`] variant; nothing is half-read.
pub fn parse_file(text: &str) -> Result<PersonaFile, PersonaFileError> { … }
```

Body of `parse_file` (shape binding, layout not): `let split = frontmatter::split(text)?;` →
`split.issues.first()` → `Unreadable { line: issue.line, message: issue.message.clone() }` → loop
over `split.frontmatter` with a `seen: Vec<&str>`, the `model` check **before** the
`FRONTMATTER_KEYS` check, `let Value::Scalar(value) = &entry.value else { NotOneLine }` and
`value.contains('\n') → NotOneLine`, then the per-key `match` → `MissingName` if no `name` →
`body = split.body.strip_prefix('\n').map_or_else(|| split.body.clone(), str::to_owned)` →
`persona_refusal(…).map_or(Ok(file), |r| Err(Refused(r)))`. The `Model` text is the literal of `MODEL_REFUSED` (thiserror cannot format a const; `the_model_key_is_refused_with_its_sentence` asserts the two equal). Fields of `PersonaFileError` carry
doc comments (missing_docs).

**Seeds.**

```rust
/// The two seed personas (plan D7, OQ-4), parsed from the compiled-in files with fresh ids.
///
/// # Panics
/// Never in a shipped build: the files are compile-time constants and
/// `seed_rows_are_the_two_seed_files` parses both.
#[must_use]
pub fn seed_rows(now: DateTime<Utc>) -> Vec<Persona> {
    [
        include_str!("../../seeds/persona_reviewer.md"),
        include_str!("../../seeds/persona_architect.md"),
    ]
    .into_iter()
    .map(|text| {
        let file = parse_file(text).expect("a compiled-in persona seed parses");
        Persona {
            id: PersonaId::new(),
            name: file.name,
            description: file.description,
            body: file.body,
            tools: file.tools,
            permission: file.permission,
            created_at: now,
            updated_at: now,
        }
    })
    .collect()
}
```

### 2.3 The two seed files, in full (T0)

`htui-core/seeds/persona_reviewer.md`:

```markdown
---
name: reviewer
description: Reviews the step's inputs and code for correctness, risk and missing tests, and reports findings without editing files.
deny-kinds: edit, delete, move
---

You are the reviewer for this step. Judge the work the item describes against its stated intent,
using the item, the input documents and the code they name.

- Correctness first: logic errors, unhandled cases, broken invariants, races.
- Then risk: security, data loss, behaviour a caller would not expect.
- Then tests: every claim of the change that no test pins.
- You may read files and run read-only commands, such as the test suite. Do not edit, create,
  delete or move files: your output is the review document, not a fix.
- Report each finding with its location, why it matters and the smallest change that resolves
  it, most severe first. Say plainly when nothing blocks.
```

`htui-core/seeds/persona_architect.md`:

```markdown
---
name: architect
description: Designs the change before it is built, studying the existing code and writing a blueprint, without editing files.
deny-kinds: edit, delete, move
---

You are the architect for this step. Design the change the item asks for, so that an implementer
can build it without re-deriving the design.

- Study the code first: the modules, patterns, naming and tests the change has to fit.
- Choose the simplest design that meets the item; add no abstraction the codebase does not
  already use.
- Name every file to create or change, the types and signatures that cross a boundary, the data
  flow, and the build order, with the test that proves each step.
- Record every choice you had to make and why; flag anything that would change a decision
  already taken instead of changing it.
- Do not edit, create, delete or move files: your output is the design document.
```

No `tools:` (allow) list in either (OQ-4), no `model`, no `command-run` (defaults `true`).
Descriptions contain no `:` and no non-ASCII.

### 2.4 Snapshot fields (T0) — `htui-core/src/model/run.rs`

`SnapshotPhase` (`:473-512`), appended after `judge`:

```rust
    /// MOD-26 D9: the name of the persona this phase runs under, frozen with its content in
    /// [`GraphSnapshot::personas`]. Hashed by `topology` when bound; absent from the JSON, and
    /// so from the digest, when `None` (I-7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
```

`GraphSnapshot` (`:433-452`), after `scope`:

```rust
    /// MOD-26 D9: every persona a phase names, frozen at `StartRun`, one per name, ordered by
    /// name bytes. **Not** hashed by `topology` (editing a persona's body never parks a run,
    /// I-3) and [`GraphSnapshot::V`] is not bumped; absent from the JSON when empty (B-10).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub personas: Vec<crate::model::persona::SnapshotPersona>,
```

In `impl GraphSnapshot` (`:454`):

```rust
    /// MOD-26 D12, I-4: the frozen persona `phase` names. `Ok(None)` for a persona-less phase;
    /// `Err` is [`persona_not_in_snapshot`]'s sentence when the snapshot does not carry the name.
    ///
    /// # Errors
    /// As above.
    pub fn persona_for(
        &self,
        phase: &SnapshotPhase,
    ) -> Result<Option<&crate::model::persona::SnapshotPersona>, String> {
        let Some(name) = phase.persona.as_deref() else {
            return Ok(None);
        };
        self.personas
            .iter()
            .find(|persona| persona.name == name)
            .map(Some)
            .ok_or_else(|| crate::model::persona::persona_not_in_snapshot(name))
    }
```

### 2.5 Phase binding (T1) — `htui-core/src/model/kind.rs`

`StepGraphPhase` (`:178-215`), just before `updated_at`:

```rust
    /// `step_graph_phase.persona_id` (MOD-26 D5): the persona this phase runs under; `None` for
    /// none. Frozen by name and content into a run's snapshot at `StartRun` (D9).
    #[serde(default)]
    pub persona_id: Option<PersonaId>,
```

`PhasePatch` (`:216-235`), last field, and its type doc gains one sentence ("MOD-26 D5 adds the
persona binding"):

```rust
    /// `step_graph_phase.persona_id` (MOD-26 D5): `None` leaves the binding, `Some(None)` clears
    /// it, `Some(Some(id))` binds `id`, which must name a `persona` row (`references_no_row`).
    #[serde(default)]
    pub persona: Option<Option<PersonaId>>,
```

`ResolvedPhase` (`:310-317`):

```rust
    /// The `persona` row `phase.persona_id` names (MOD-26 D6); `None` when it names none.
    #[serde(default)]
    pub persona: Option<Persona>,
```

Literals T1 fixes: `seed.rs:216` (`persona_id: None`), `store/conformance.rs:2277` (`new_phase`,
`persona_id: None`) and `:3680` (`persona: None`), `htui/src/ui/tabs/settings/kinds.rs:976`
(`persona: None`: the editor leaves the binding alone), `mem.rs:5164` and `pg/read.rs:1750`
(`ResolvedPhase`, filled per §2.8/§2.9). Every other `StepGraphPhase`/`PhasePatch` literal spreads
(`graph.rs:507`, `engine.rs:6736`, `:7822`, `:11407`, `:11537`, `:14575`, `conformance.rs:968`,
`:1036`, `:1050`, `:1098`, `tests/fixtures.rs:70`, `:77`, `tests/gix_isolator.rs:356`,
`htui/tests/kinds.rs:594`, `:1747`, `:1819`; checked at HEAD).

### 2.6 `WriteStore` additions (T1) — `htui-core/src/store/traits.rs`

After `set_skill_binding` (`:974-985`), before `// settings (D7, D8)` (`:987`):

```rust
    // persona (MOD-26 milestone 1, plan D4)
    //
    // A global registry like `skill`, read and written here for the skill block's reason
    // (`:908-913`). No delete in milestone 1: `step_graph_phase.persona_id` is `ON DELETE
    // RESTRICT`, and M2 adds the delete with its "bound to phases" refusal. The refusal sentences
    // are `model::persona`'s pure helpers, re-exported below (plan D3), so both stores word them
    // once.

    /// Every persona, ordered by `name` bytes (`COLLATE "C"`).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn personas(&self) -> Result<Vec<Persona>>;

    /// Inserts one persona; both stamps are the store's clock (plan D4). Order:
    /// [`new_persona_refusal`]'s sentences, then a taken id (`already_exists("persona", id)`),
    /// then a taken name (`already_exists("persona", name)`).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) as above. Nothing is
    /// written.
    async fn create_persona(&self, new: NewPersona) -> Result<Persona>;

    /// Edits a persona under CAS on `persona.updated_at` (plan D4), `update_skill`'s order:
    /// `NotFound { entity: "persona" }` for an unknown id, then `Stale(current)` for a spent token
    /// (even with bad input), then `Constraint` for [`persona_patch_refusal`]'s sentences or a
    /// taken name (`already_exists("persona", name)`). An all-`None` patch still stamps
    /// `updated_at`. A started run never sees the edit: it reads its snapshot (I-3).
    ///
    /// # Errors
    /// As above.
    async fn update_persona(
        &self,
        id: PersonaId,
        expected: DateTime<Utc>,
        patch: PersonaPatch,
    ) -> Result<CasOutcome<Persona>>;
```

`create_phase` (`:856-863`) and `update_phase` (`:865-877`) docs each gain: "…, or a
`persona_id` that names no row (`references_no_row("step_graph_phase.persona_id", id,
"persona")`), checked after the position and name clashes (MOD-26 D5)."

Re-export (B-2), next to the skill helpers (`:1860` banner):

```rust
// ---- MOD-26: the persona writers' refusals (plan D3) live in `model::persona`, where the
// persona-file reader needs them too; re-exported so the store's refusal vocabulary is one list.
pub use crate::model::persona::{
    BLANK_PERSONA_BODY, MODEL_REFUSED, RULE_MATCHES_EVERYTHING, allow_names_an_mcp_tool,
    invalid_persona_name, kind_not_narrowable, new_persona_refusal, not_a_tool_name,
    persona_not_in_snapshot, persona_patch_refusal, persona_refusal,
};
```

and the same eleven names added to `store/mod.rs`'s `pub use traits::{…}` list (`:16-32`).
Implementors (verified complete): MemStore (`mem.rs:6217`), PgStore (`pg/write.rs:719`), `Writer`
(`writer.rs:312`, `match self { Self::Memory(store) => …, Self::Online(pg) => … }`), `UsageSpy`
(`htui-agent/src/conformance.rs:743`, `self.inner.…`), `SpyStore` (`htui-agent/tests/recorder.rs:430`,
`self.inner.…`), each placed after its `set_skill_binding`.

### 2.7 Migration `htui-store/migrations/0012_persona.sql` (T1), in full

```sql
-- 0012_persona.sql - MOD-26 milestone 1 (plan D1, D5; PRD Q-storage, Q-binding).
-- Forward-only (R-STO-5).
--
-- persona holds the agent persona registry: one global row per name, like skill, carrying the
-- role text stage 3 renders ahead of the phase template (body), a narrowing of the agent row's
-- tool exposure (tools) and deny-only permission rules (permission). A persona never carries or
-- influences a model (R-AGT-8). step_graph_phase.persona_id binds at most one persona to a
-- phase; ON DELETE RESTRICT keeps a bound persona from vanishing under a graph (milestone 1 has
-- no delete). A started run reads personas only from its graph_snapshot, so editing a row never
-- changes a started run's steps. Neither is mirrored, but schema_version becomes 12, so each box
-- rebuilds its mirror once on first start. A headless worker never migrates: migrate from a TUI
-- first.

CREATE TABLE persona (
    id          UUID        PRIMARY KEY,
    name        TEXT        NOT NULL CONSTRAINT uq_persona_name UNIQUE,
    description TEXT        NOT NULL DEFAULT '',
    body        TEXT        NOT NULL,
    tools       JSONB       NOT NULL DEFAULT '{}',
    permission  JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 0001's set_updated_at, BEFORE UPDATE only (0006_requirements.sql:135-142's shape).
CREATE TRIGGER trg_persona_updated_at BEFORE UPDATE ON persona
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

ALTER TABLE step_graph_phase
    ADD COLUMN persona_id UUID NULL
        CONSTRAINT fk_step_graph_phase_persona REFERENCES persona(id) ON DELETE RESTRICT;

COMMENT ON COLUMN persona.id IS
    'MOD-26 D1: client-minted UUIDv7; what step_graph_phase.persona_id references.';
COMMENT ON COLUMN persona.name IS
    'MOD-26 D1, D3: the registry name, unique; 1-64 of a-z, 0-9 and single inner hyphens, '
    'checked by the writers. A run snapshot freezes a persona under this name.';
COMMENT ON COLUMN persona.description IS
    'MOD-26 D1: the one-line summary a picker shows; never rendered into a prompt.';
COMMENT ON COLUMN persona.body IS
    'MOD-26 D1, D13: the role text stage 3 renders as the protected persona section ahead of '
    'the phase template. Never blank.';
COMMENT ON COLUMN persona.tools IS
    'MOD-26 D2, D10: {allow, deny, deny_kinds, command_run}, narrow-only against the agent row. '
    'allow keeps built-in tool names (empty keeps all), deny removes tool names, deny_kinds denies '
    'ACP tool kinds (read, edit, delete, move, search, execute, fetch), command_run false '
    'withdraws the command queue. Unknown keys are refused.';
COMMENT ON COLUMN persona.permission IS
    'MOD-26 D2, D10: {default, rules[]}, deny-only. default is null, ask or deny; every rule '
    'answers reject_once or reject_always and is evaluated before the agent row rules and '
    'remembered choices. Unknown keys are refused.';
COMMENT ON COLUMN persona.created_at IS
    'MOD-26 D1: when the row was inserted.';
COMMENT ON COLUMN persona.updated_at IS
    'MOD-26 D1: the update_persona compare-and-set token, stamped by trg_persona_updated_at.';
COMMENT ON COLUMN step_graph_phase.persona_id IS
    'MOD-26 D5: the persona this phase runs under; NULL for none. StartRun freezes it by name '
    'and content into run.graph_snapshot (phases[].persona, personas[]); ON DELETE RESTRICT.';
```

Adjacent string constants separated by a newline concatenate; every split keeps its space on the
first fragment, so the stored text is the fragments joined with nothing between them.
`persona_pkey` is Postgres's default name for the primary key.

`MOD26_COLUMN_COMMENTS` in `htui-store/tests/migrations.rs` (after `MOD33_COLUMN_COMMENTS`,
`:443`) carries the nine `(table, column, text)` triples **in the order above**, each text the exact
concatenation (Rust `\`-continuations with the space before the backslash, as `:409-411` does).

### 2.8 PgStore SQL (T1; B-15)

**`pg/rows.rs`**:

```rust
/// One `persona` row with its two `JSONB` columns still wrapped (MOD-26 B-15).
#[derive(Debug)]
pub(crate) struct PersonaRow {
    pub(crate) id: PersonaId,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) body: String,
    pub(crate) tools: Json<PersonaTools>,
    pub(crate) permission: Json<PersonaPermission>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}
impl From<PersonaRow> for Persona { /* field moves; tools.0, permission.0 */ }
```

A hand-written row whose blob does not decode (an unknown key, I-2) is the driver's decode
error through `map_sqlx` → `Backend`: `StartRun` fails rather than run with half a persona.

**`pg/read.rs`** (beside `skill_rows`, `:2413`). One select list, three predicates:

```sql
SELECT id          AS "id: PersonaId",
       name,
       description,
       body,
       tools       AS "tools: Json<PersonaTools>",
       permission  AS "permission: Json<PersonaPermission>",
       created_at,
       updated_at
  FROM persona
 [ (none)             -- persona_rows()
 | WHERE id = $1      -- persona_row(id) -> Option, fetch_optional
 | WHERE id = ANY($1) -- persona_rows_by_id(&[Uuid]) ]
 ORDER BY name COLLATE "C"
```

`phase_row` (`:2226`) and `phase_rows` (`:2371`) add `persona_id AS "persona_id: PersonaId",` to
their select lists (before `updated_at`). `resolve_graph` (`:1726-1753`):

```rust
        let mut phases = Vec::new();
        for phase in self.phase_rows(graph_id).await? {
            let agents = self.phase_agents(phase.id).await?;
            phases.push(ResolvedPhase { phase, agents, persona: None });
        }
        // MOD-26 D6: one read over the distinct bound personas.
        let mut ids: Vec<Uuid> = phases
            .iter()
            .filter_map(|row| row.phase.persona_id)
            .map(PersonaId::as_uuid)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        if !ids.is_empty() {
            let personas = self.persona_rows_by_id(&ids).await?;
            for row in &mut phases {
                row.persona = row
                    .phase
                    .persona_id
                    .and_then(|id| personas.iter().find(|persona| persona.id == id).cloned());
            }
        }
        Ok(Some(ResolvedGraph { graph, phases }))
```

**`pg/write.rs`**, the mappers (beside `skill_insert_refused`, `:649-666`):

```rust
/// MOD-26 D4: `uq_persona_name`'s `23505` in `MemStore`'s sentence; anything else through
/// [`map_sqlx`].
fn persona_name_taken(err: sqlx::Error, name: &str) -> StoreError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("uq_persona_name") => {
            StoreError::Constraint(already_exists("persona", name))
        }
        _ => map_sqlx(err),
    }
}

/// `create_persona`'s refusals in `MemStore`'s order: `persona_pkey` is the id, then the name.
fn persona_insert_refused(err: sqlx::Error, new: &NewPersona) -> StoreError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("persona_pkey") => {
            StoreError::Constraint(already_exists("persona", new.id))
        }
        _ => persona_name_taken(err, &new.name),
    }
}

/// MOD-26 D5: `fk_step_graph_phase_persona`'s `23503` in `MemStore`'s sentence (today every
/// `23xxx` is raw, `error.rs:42-46`); anything else through [`map_sqlx`].
fn phase_persona_refused(err: sqlx::Error, persona: Option<PersonaId>) -> StoreError {
    match (&err, persona) {
        (sqlx::Error::Database(db), Some(persona))
            if db.constraint() == Some("fk_step_graph_phase_persona") =>
        {
            StoreError::Constraint(references_no_row(
                "step_graph_phase.persona_id",
                persona,
                "persona",
            ))
        }
        _ => map_sqlx(err),
    }
}

/// A persona blob as `JSONB`, `create_run`'s shape (`:3636-3638`).
fn persona_json<T: serde::Serialize>(column: &str, value: &T) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|error| StoreError::Constraint(format!("{column} does not serialise: {error}")))
}
```

The three methods (after `set_skill_binding`, `:3116`):

```rust
    async fn personas(&self) -> Result<Vec<Persona>> { self.persona_rows().await }

    async fn create_persona(&self, new: NewPersona) -> Result<Persona> {
        if let Some(refusal) = new_persona_refusal(&new) {
            return Err(StoreError::Constraint(refusal));
        }
        let tools = persona_json("persona.tools", &new.tools)?;
        let permission = persona_json("persona.permission", &new.permission)?;
        sqlx::query_as!(
            PersonaRow,
            r#"
            INSERT INTO persona (id, name, description, body, tools, permission)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id          AS "id: PersonaId",
                      name,
                      description,
                      body,
                      tools       AS "tools: Json<PersonaTools>",
                      permission  AS "permission: Json<PersonaPermission>",
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(), new.name, new.description, new.body, tools, permission,
        )
        .fetch_one(&self.pool)
        .await
        .map(Persona::from)
        .map_err(|err| persona_insert_refused(err, &new))
    }

    async fn update_persona(&self, id: PersonaId, expected: DateTime<Utc>, patch: PersonaPatch)
        -> Result<CasOutcome<Persona>>
    {
        // `update_skill`'s shape (`:3017-3065`): bad input pays one read for NotFound → Stale →
        // Constraint; a spent token matches no row and never reaches the unique index.
        if let Some(refusal) = persona_patch_refusal(&patch) {
            return match self.persona_row(id).await? {
                None => Err(StoreError::NotFound { entity: "persona", id: id.to_string() }),
                Some(row) if row.updated_at != expected => Ok(CasOutcome::Stale(row)),
                Some(_) => Err(StoreError::Constraint(refusal)),
            };
        }
        let tools = patch.tools.as_ref().map(|t| persona_json("persona.tools", t)).transpose()?;
        let permission = patch.permission.as_ref()
            .map(|p| persona_json("persona.permission", p)).transpose()?;
        let updated = sqlx::query_as!(
            PersonaRow,
            r#"
            UPDATE persona SET
                name        = COALESCE($3, name),
                description = COALESCE($4, description),
                body        = COALESCE($5, body),
                tools       = COALESCE($6, tools),
                permission  = COALESCE($7, permission)
             WHERE id = $1 AND updated_at = $2
            RETURNING <the select list above>
            "#,
            id.as_uuid(), expected, patch.name.as_deref(), patch.description.as_deref(),
            patch.body.as_deref(), tools, permission,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| match &patch.name {
            Some(name) => persona_name_taken(err, name),
            None => map_sqlx(err),
        })?;
        match updated {
            Some(row) => Ok(CasOutcome::Applied(row.into())),
            None => cas_miss(self.persona_row(id).await?, "persona", id),
        }
    }
```

`create_phase` (`:2704`): the insert gains `persona_id` as `$17` (`phase.persona_id.map(PersonaId::as_uuid)`),
the `RETURNING` list gains `persona_id AS "persona_id: PersonaId",`, and `.map_err(map_sqlx)`
becomes `.map_err(|err| phase_persona_refused(err, phase.persona_id))`. `update_phase` (`:2770`):

```sql
            UPDATE step_graph_phase SET
                name          = COALESCE($3, name),
                position      = COALESCE($4, position),
                template_name = COALESCE($5, template_name),
                gate_hard     = COALESCE($6, gate_hard),
                input_kinds   = COALESCE($7, input_kinds),
                persona_id    = CASE WHEN $8::bool THEN $9::uuid ELSE persona_id END
             WHERE id = $1 AND updated_at = $2
            RETURNING … persona_id AS "persona_id: PersonaId", updated_at
```

with `let persona = patch.persona;` read **before** the macro (it is `Copy`), `$8 =
persona.is_some()`, `$9 = persona.flatten().map(PersonaId::as_uuid)`, and
`.map_err(|err| phase_persona_refused(err, persona.flatten()))`. The reserved-name pre-check
(`:2776-2789`) is unchanged; a stale token never reaches the FK (zero rows → `cas_miss`).

**Seed** (`pg/mod.rs`, after the agent loop `:481-503`, inside the same transaction; F-24):

```rust
        // MOD-26 D7: the persona seeds, the agent loop's name-keyed top-up: an operator's edit
        // survives, a later seed is added.
        for persona in htui_core::model::persona::seed_rows(Utc::now()) {
            let tools = serde_json::to_value(&persona.tools).map_err(|error| {
                StoreError::Constraint(format!("persona.tools does not serialise: {error}"))
            })?;
            let permission = serde_json::to_value(&persona.permission).map_err(|error| {
                StoreError::Constraint(format!("persona.permission does not serialise: {error}"))
            })?;
            sqlx::query!(
                "INSERT INTO persona (id, name, description, body, tools, permission, \
                                      created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (name) DO NOTHING",
                persona.id.as_uuid(), persona.name, persona.description, persona.body,
                tools, permission, persona.created_at, persona.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }
```

**Demo** (`pg/demo.rs`): after the agent loop (`:185`) and before graphs (`:187`):
`DELETE FROM persona WHERE name = ANY($1)` over `data.personas`' names, then one `INSERT INTO
persona (id, name, description, body, tools, permission, created_at, updated_at) VALUES ($1 … $8)`
per row (no `ON CONFLICT`, the module rule). The phase insert (`:203-231`) gains `persona_id`
(`$18`, `row.persona_id.map(PersonaId::as_uuid)`). The module doc's exception sentence (`:8-11`)
names `persona` beside `agent`.

### 2.9 MemStore reference semantics (T1) — `htui-core/src/store/mem.rs`

- `State.personas: HashMap<PersonaId, Persona>` after `skills` (`:167`), doc: "`persona` (MOD-26
  D1), read by [`WriteStore::personas`] and `resolve_graph`, written by `create_persona` and
  `update_persona`; global, so `delete_project` leaves it." `from_demo` (`:292`) loads
  `data.personas`.
- `State::persona_rows()` sorted by `name.as_bytes()`.
- `State::create_persona(new, now)`: `new_persona_refusal` → `already_exists("persona", id)` →
  `already_exists("persona", name)` → insert with both stamps `now`.
- `State::update_persona(id, expected, patch, now)`: `NotFound { entity: "persona" }` →
  `Stale(current)` → `persona_patch_refusal` → a name another row holds → apply every `Some`,
  stamp `now` (an all-`None` patch stamps).
- `State::require_persona(id) -> Result<()>`: `Constraint(references_no_row(
  "step_graph_phase.persona_id", id, "persona"))` when absent.
- `create_phase` (`:2901`): after `check_phase` (`:2867-2897`) and the id-exists check,
  `if let Some(persona) = phase.persona_id { self.require_persona(persona)?; }` — Pg's order: the
  unique indexes fire at insert, the FK after.
- `update_phase` (`:2921`): after the CAS read and `check_phase`, `if let Some(Some(persona)) =
  patch.persona { self.require_persona(persona)?; }`; apply `if let Some(persona) =
  patch.persona { row.persona_id = persona; }`.
- `resolve_graph` (`:5146-5170`): `persona: phase.persona_id.and_then(|id|
  self.personas.get(&id).cloned())`.
- `WriteStore` wrappers after `set_skill_binding` (`:6554`), the skill wrappers' shape
  (`let now = self.now(); self.write(|state| …)`).
- `delete_project_leaves_no_row_in_any_map` (`mem.rs:8739`): `!state.personas.is_empty()` joins
  the "not below a project" assertion (`:8954-8963`).

### 2.10 Prompt (T0 field, T3 mechanics) — `htui-core/src/prompt/`

**T0** (`prompt/mod.rs`): `PromptSpec` (`:73-135`) gains, after `skills`:

```rust
    /// MOD-26 D13: the phase's persona, rendered as the protected `persona` section ahead of
    /// the template body. `None` — every judge, handoff and preview spec and every persona-less
    /// phase — renders exactly as before (I-7).
    pub persona: Option<PersonaBlock>,
```

and, after `TemplateRef`:

```rust
/// MOD-26 D13: what the persona frame renders — a frozen persona's name and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaBlock {
    /// `SnapshotPersona.name`, the section's `persona` attribute.
    pub name: String,
    /// `SnapshotPersona.body`, the section's content.
    pub body: String,
}

impl From<&SnapshotPersona> for PersonaBlock {
    fn from(persona: &SnapshotPersona) -> Self {
        Self { name: persona.name.clone(), body: persona.body.clone() }
    }
}
```

`assemble` ignores the field until T3. Every non-spread `PromptSpec` literal gets `persona: None`:
`prompt/fixtures.rs:161`, `:261`, `:365`, `:505` (and any other the compiler names),
`tests/prompt_digest.rs`, `tests/prompt_hostname.rs`, `tests/prompt_skills.rs`,
`htui-agent/src/excerpt.rs:1147`, `htui-agent/tests/excerpt.rs:1095`, `:1434`,
`htui-orch/src/engine.rs:4734` (judge), `:5457` (phase, `None` until T4),
`htui-orch/src/promote.rs:89` (OQ-5) and `:416`, `htui/src/preview.rs:281`.

**T3, `template.rs`**: `Placeholder` (`:76-117`) gains a **last** variant (declaration order is
`Ord`; appending keeps every existing comparison):

```rust
    /// MOD-26 D13: the persona frame. **Internal**: not in [`Placeholder::ALL`], so
    /// [`Placeholder::from_token`] never yields it and no template can place it — a body that
    /// writes `{{persona}}` is refused as an unknown placeholder. `assemble` renders it ahead of
    /// the body whenever the spec carries a persona.
    Persona,
```

`token` → `"persona"`; `allowed_in` → `false` in every role (add `| Self::Persona` to the Phase
arm's `matches!`; the Judge and Handoff arms already exclude it); `is_section` → `true`
(unchanged expression). `ALL` stays twenty (`:428-432`), so the editor's list
(`htui/src/ui/tabs/skills/templates.rs:1091`) and `the_three_role_sets_are_closed` are unchanged.

**T3, `mod.rs`**: `SectionName::Persona` declared right after `Template` (doc: "The persona
frame (MOD-26 D13). Always `sections[1]` when present: `template` keeps `sections[0]`, P-9.");
`render` → `"persona"`; `is_protected` arm `Self::Template | Self::Persona | Self::Box |
Self::Skills | Self::CommandQueue => true` (`:308`). New constant:

```rust
/// MOD-26 D13, B-17: the bytes between the persona frame and the template body, counted in the
/// frame's estimate.
pub const PERSONA_SEPARATOR: &str = "\n\n";
```

`scrubbed_inputs` (after the skills loop, `:868-872`):

```rust
    if let Some(persona) = &mut spec.persona {
        let persona_name = SectionName::Persona.render();
        mask(scrubber, &mut persona.name, &persona_name)?;
        mask(scrubber, &mut persona.body, &persona_name)?;
    }
```

`render_sections` (`:694`): `Placeholder::Persona => spec.persona.iter().map(render::persona).collect(),`.

`assemble`, immediately before the loop at `:486`:

```rust
    // MOD-26 D13: the persona frame renders first and outside the body's spans, so it is scanned
    // with every section below, protected in the trimmer, and recorded as `sections[1]`.
    for section in render_sections(Placeholder::Persona, spec, upstream, skills, candidates) {
        rendered.push((Placeholder::Persona, section, 1));
    }
```

`:525`:

```rust
    let frame = masked.literals.concat();
    let template_tokens = if spec.persona.is_some() {
        est.estimate(&format!("{frame}{PERSONA_SEPARATOR}"))
    } else {
        est.estimate(&frame)
    };
```

`substitute` (`:621`), right after `let mut text = String::new();` (`:634`):

```rust
    if let Some(frame) = blocks.get(&Placeholder::Persona) {
        text.push_str(&frame.join("\n\n"));
        text.push_str(PERSONA_SEPARATOR);
    }
```

Both forms (digest and sent) go through `substitute`, so both carry the frame; the box swap
(`:545-560`) never touches it. `excerpt_residual` spreads the spec, so it pays the frame too.

**T3, `render.rs`** (after `command_queue`, `:627`):

```rust
/// MOD-26 D13: `<section name="persona" persona="…">` then the persona's body (B-20).
#[must_use]
pub fn persona(block: &PersonaBlock) -> Rendered {
    Rendered {
        name: SectionName::Persona,
        attrs: vec![("persona", attr(&block.name))],
        content: content_of(&block.body),
    }
}
```

**T3, `trim.rs`** (B-7): `| SectionName::Persona` joins the `Source::Fixed` arm of `source_of`
(`:503-507`) and the `None` arm of `trim_diff` (`:809-821`).

### 2.11 `ToolExposure.deny_kinds` (T0) and `narrow` (T2) — `htui-agent`

**T0, `driver.rs`** (`ToolExposure`, `:200-213`; the struct doc's "A plain record with no reader
in milestones 1–2" becomes "Filled by the engine from a persona (MOD-26 D10); read by `claude-cli`'s
argv (D11), the relay's policy and the ACP `fs/*` handlers (`deny_kinds`)."):

```rust
    /// ACP tool kinds the step may never use (MOD-26 D10): refused by the permission relay on
    /// every transport that asks and by `htui`'s own ACP `fs/*` handlers, and inverted to tool
    /// names on `claude-cli`'s argv (D11). Empty denies no kind.
    pub deny_kinds: Vec<ToolKind>,
```

(`use crate::event::ToolKind;` — `event.rs` already imports from `driver`, a legal module cycle.)
No `ToolExposure` literal exists anywhere; every builder uses `ToolExposure::default()`.

**T2, `htui-agent/src/persona.rs`** (new; `pub mod persona;` after `pub mod permission;`,
`lib.rs:117`):

```rust
//! MOD-26 D10: a persona narrows a step's tool exposure and permission policy, never widens them
//! (I-1). Pure: no store, no transport, no clock.

use htui_core::model::persona::{PersonaAnswer, PersonaDefault, PersonaMatch, SnapshotPersona};

use crate::driver::{
    PermissionDefault, PermissionMatch, PermissionPolicy, PermissionRule, ToolExposure,
};
use crate::event::{PermissionOptionKind, ToolKind};

/// The [`ToolKind`] spelled `text`, `None` outside the ten (`wire_enum!` has no `FromStr`).
#[must_use]
pub fn tool_kind_of(text: &str) -> Option<ToolKind> {
    ToolKind::ALL.iter().copied().find(|kind| kind.as_str() == text)
}

/// The step's exposure and policy under `persona`, from the agent row's (I-1):
///
/// - `allow`: base empty → persona's; persona empty → base's; both non-empty → `base ∩ persona`
///   in base order, every base name outside the persona's appended to `deny`, and the base list
///   kept when the intersection is empty (B-8);
/// - `deny`: base, then persona's, then B-8's additions, first occurrence kept;
/// - `deny_kinds`: base, then persona's (`tool_kind_of`, an unknown string → `Other`, B-19);
/// - `command_run`: `base && persona`;
/// - rules: persona rules (reject-only; empty reason → `persona <name>`, B-21), then one
///   `{match: {tool_kind}, answer: reject_once, reason: "persona <name> denies <kind>"}` per
///   persona `deny_kinds` entry, then the base rules;
/// - `remembered`: the base's, evaluated after every rule (`permission.rs:81-92`);
/// - `default`: the stricter of the two, `Allow < Ask < Deny`.
#[must_use]
pub fn narrow(
    base: &ToolExposure,
    policy: &PermissionPolicy,
    persona: &SnapshotPersona,
) -> (ToolExposure, PermissionPolicy) { … }
```

Private helpers (names binding, bodies not): `fn strictness(default: PermissionDefault) -> u8`
(`Allow` 0, `Ask` 1, `Deny` 2), `fn persona_default(default: PersonaDefault) -> PermissionDefault`,
`fn answer_kind(answer: PersonaAnswer) -> PermissionOptionKind`, `fn matcher(m: &PersonaMatch) ->
PermissionMatch` (four field clones), `fn push_unique(list: &mut Vec<String>, name: &str)`.

### 2.12 CLI argv (T2) — `htui-agent/src/cli/`

`claude.rs`, after `tool_kind` (`:465-474`):

```rust
/// MOD-26 D11: [`tool_kind`] inverted — the tool names one ACP kind covers in this dialect, for
/// `--disallowedTools`. `delete`, `move`, `think`, `switch_mode` and `other` name no tool. A
/// name a given CLI release lacks is harmless to deny (probed on 2.1.286, plan R-1).
#[must_use]
pub const fn tool_names(kind: ToolKind) -> &'static [&'static str] {
    match kind {
        ToolKind::Read => &["Read", "NotebookRead"],
        ToolKind::Edit => &["Edit", "Write", "MultiEdit", "NotebookEdit"],
        ToolKind::Execute => &["Bash", "BashOutput", "KillShell"],
        ToolKind::Search => &["Glob", "Grep"],
        ToolKind::Fetch => &["WebFetch", "WebSearch"],
        ToolKind::Delete
        | ToolKind::Move
        | ToolKind::Think
        | ToolKind::SwitchMode
        | ToolKind::Other => &[],
    }
}
```

`mod.rs` `argv` (`:112`): the doc's ordered list gains item 6½ ("the step's narrowing, D11"); the
push goes **between** the scoped block's closing brace (`:158`) and `args.extend(cli.extra_args…)`
(`:160`):

```rust
    // MOD-26 D11: the step's narrowing, each as one `=`-joined argument (the closure above pushes
    // pairs). `--tools` restricts the built-in set and is omitted when `allow` is empty —
    // `--tools=""` would disable every tool. `--allowedTools` is never emitted: it auto-approves
    // (I-1, probed).
    if !spec.tools.allow.is_empty() {
        args.push(format!("--tools={}", spec.tools.allow.join(",")));
    }
    let denied = disallowed(&spec.tools);
    if !denied.is_empty() {
        args.push(format!("--disallowedTools={}", denied.join(",")));
    }
```

```rust
/// `deny`, then every name `deny_kinds` inverts to ([`claude::tool_names`]), first occurrence
/// kept.
fn disallowed(tools: &ToolExposure) -> Vec<String> { … }
```

A default exposure adds no argument: every existing exact-`Vec` argv test is unchanged (I-7).

### 2.13 ACP fs refusal (T2) — `htui-agent/src/acp/mod.rs` (B-12)

Beside `PATH_OUTSIDE_SESSION` (`:76`):

```rust
/// `error.code` of an `fs/*` request the step's persona denies (MOD-26 D11):
/// `fs/read_text_file` under a denied `read`, `fs/write_text_file` under a denied `edit`.
pub const TOOL_KIND_DENIED: &str = "tool_kind_denied";
```

`session_main` (`:1060`), right after `let mut state = TaskState::new(…)` (`:1071`):
`let deny_kinds = spec.tools.deny_kinds.clone();`; the call at `:1265` becomes
`on_inbound(&mut state, &events, &filesystem, &deny_kinds, request)`. `on_inbound` gains
`deny_kinds: &[ToolKind]` before `request`, and two guarded arms **before** the existing ones:

```rust
        Inbound::ReadFile(request, responder) if deny_kinds.contains(&ToolKind::Read) => {
            let _ = responder.respond_with_error(agent_client_protocol::Error::invalid_params());
            denied(state, events, ToolKind::Read, "fs/read_text_file", &request.path).await
        }
        Inbound::WriteFile(request, responder) if deny_kinds.contains(&ToolKind::Edit) => {
            // Before `guard`, the read of the old text and the `EditProposal`: nothing touches the
            // file and no proposal is recorded for an edit that never happens.
            let _ = responder.respond_with_error(agent_client_protocol::Error::invalid_params());
            denied(state, events, ToolKind::Edit, "fs/write_text_file", &request.path).await
        }
```

```rust
/// Records a request the step's persona denies (MOD-26 D11), `refused`'s shape.
async fn denied(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    kind: ToolKind,
    method: &str,
    path: &Path,
) -> bool {
    tracing::warn!(%kind, method, "an fs request the step's persona denies");
    let event = DriverEvent::Error(ErrorEvent {
        code: TOOL_KIND_DENIED.to_owned(),
        message: format!(
            "the step's persona denies `{kind}`: {method} of `{}` refused",
            path.display()
        ),
    });
    emit(state, events, event, None).await
}
```

(`request.path`'s exact type is the SDK's; if it is not a `Path`, take `&impl AsRef<Path>`.)

### 2.14 Engine (T4) — `htui-orch`

**`StageThree`** (`engine.rs:6159`) gains:

```rust
    /// MOD-26 I-4: the phase names a persona the run's snapshot does not carry; carries
    /// `persona_not_in_snapshot`'s sentence.
    NoPersona(String),
```

**`phase_spec`** (`:5376`): returns `Result<Result<PromptSpec, StageThree>, EngineError>`; the
existing `return Ok(Err(input.kind))` becomes `return Ok(Err(StageThree::MissingInput(input.kind)))`;
its first statements are (B-4):

```rust
        // MOD-26 D12: stage 3's persona lookup, in the run's own snapshot (I-3). A promoted step's
        // handoff (not strict) opens without one (OQ-5); a phase step whose snapshot does not carry
        // its persona is refused (I-4) and never runs un-narrowed.
        let persona = match snapshot.persona_for(phase) {
            Ok(persona) => persona,
            Err(_) if !strict => None,
            Err(reason) => return Ok(Err(StageThree::NoPersona(reason))),
        };
```

and the literal (`:5457`) sets `persona: persona.map(PersonaBlock::from),` and

```rust
            // MOD-26 D13: the one place `command_run` acts before MOD-11.
            command_queue: phase.command_queue != htui_core::model::CommandQueue::Off
                && persona.is_none_or(|persona| persona.tools.command_run),
```

`assemble_prompt` (`:5276-5295`): `Err(stage) => return Ok(Err(stage)),`. `opening` (`:1367`):
`Err(StageThree::MissingInput(kind)) =>` keeps today's `EngineError::Snapshot` sentence, and
`Err(other) => return Err(EngineError::Snapshot { run: run.id, reason: format!("the handoff prompt
was refused: {other:?}") })` (unreachable with `strict = false`).

**`refuse_prompt`** (`:5214`) takes `reason: String` (B-5); `walk_live_step` (`:3435-3437`):

```rust
            Err(StageThree::Refused(err)) => {
                return self.refuse_prompt(run, step, phase, err.to_string()).await.map(Some);
            }
            // MOD-26 I-4: the step fails before a token, the item is blocked, the run settles.
            Err(StageThree::NoPersona(reason)) => {
                return self.refuse_prompt(run, step, phase, reason).await.map(Some);
            }
        };
        // MOD-26 D12: stage 3 passed, so the snapshot carries the persona; `Err` is unreachable
        // and refuses rather than run un-narrowed.
        let persona = snapshot
            .persona_for(phase)
            .map_err(|reason| EngineError::Snapshot { run: run.id, reason })?;
```

and the stage-4 call (`:3472`) becomes `.session(run, step, phase, persona, &prompt, …)`.
`drive_group` (`:3790-3800`) gains the twin arm
(`RunFailure::PromptRefused { phase: phase.name.clone(), reason }` →
`fail_group_before_a_token(run, phase, &pending, failure, true)`), the same post-stage-3
`persona_for`, and `CandidateStage { …, persona }`.

**`CandidateStage`** (`:6169`) gains `/// MOD-26 D12: the phase's frozen persona. pub persona:
Option<&'s SnapshotPersona>,` (still `Clone, Copy`). `run_candidate`'s call (`:4078`) passes
`stage.persona`.

**`session`** (`:5662`): `(…, phase: &SnapshotPhase, persona: Option<&SnapshotPersona>, prompt:
&AssembledPrompt, …)`, forwarded to `drive_once`.

**`drive_once`** (`:5761`), signature and narrowing:

```rust
    async fn drive_once(
        &self,
        run: &Run,
        step: &RunStep,
        phase: &SnapshotPhase,
        persona: Option<&SnapshotPersona>,
        key: &SessionKey<'_>,
        text: &str,
        cwd: PathBuf,
        extra_dirs: Vec<PathBuf>,
        recorder: &mut Recorder<'a, S>,
    ) -> Result<SessionResult, EngineError> {
        …
        let policy = (self.parts.policy)(candidate.agent_id);
        // MOD-26 D10, D12: the persona narrows the agent's exposure and policy, never widens them
        // (I-1); a persona-less step is exactly today's (I-7).
        let (tools, policy) = match persona {
            Some(persona) => {
                htui_agent::persona::narrow(&ToolExposure::default(), &policy, persona)
            }
            None => (ToolExposure::default(), policy),
        };
        …
            tools,                       // :5797
            permission: policy.clone(),  // :5799, unchanged expression
        …
        let relay = Relay { …, policy: &policy, … };  // :5806-5816, unchanged expression
```

The `#[allow(clippy::too_many_arguments)]` reason (`:5758-5759`) gains "and the persona stage 3
froze". The judge (`:4974`) passes `None`.

**Freeze** (`graph.rs`, B-18). `resolve`'s loop (`:331-347`) becomes:

```rust
    let mut personas: Vec<SnapshotPersona> = Vec::new();
    for (dense, row) in rows.iter().enumerate() {
        …
        let mut phase = snapshot_phase(source, &row.phase, &row.agents, position, project.id,
                                       &settings, app, box_id).await?;
        phase.persona = frozen_persona(row, &mut personas)?;
        phases.push(phase);
    }
    personas.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
```

and the literal (`:364-376`) carries `personas,`. Private helper:

```rust
/// MOD-26 D9: the name of the persona `row` binds, frozen once into `personas`. A `persona_id`
/// with no row is refused (unreachable under the FK and MemStore's write check) rather than
/// frozen un-narrowed.
fn frozen_persona(
    row: &ResolvedPhase,
    personas: &mut Vec<SnapshotPersona>,
) -> std::result::Result<Option<String>, ResolveError> {
    let Some(id) = row.phase.persona_id else {
        return Ok(None);
    };
    let Some(persona) = &row.persona else {
        return Err(ResolveError::Store(StoreError::Constraint(references_no_row(
            "step_graph_phase.persona_id",
            id,
            "persona",
        ))));
    };
    if !personas.iter().any(|frozen| frozen.name == persona.name) {
        personas.push(SnapshotPersona::freeze(persona).map_err(|err| {
            ResolveError::Store(StoreError::Constraint(format!(
                "persona `{}` does not serialise: {err}",
                persona.name
            )))
        })?);
    }
    Ok(Some(persona.name.clone()))
}
```

`override_graph` copies `persona_id` through its spread (`:507-511`); `judge_phase` sets
`persona: None` (T0, OQ-2), so the judge's prompt carries no frame while its `{{task}}` replays
the candidate's recorded prompt, frame included.

---

## 3. T0 — contracts (serial, first; no SQL)

### 3.1 Files

| File | Change |
|---|---|
| `htui-core/src/model/ids.rs` | `PersonaId` (§2.1) |
| `htui-core/src/model/mod.rs` | `pub mod persona;`, re-exports (§2.1) |
| `htui-core/src/model/persona.rs` (new) | §2.2 and its tests |
| `htui-core/seeds/persona_reviewer.md`, `persona_architect.md` (new) | §2.3, verbatim |
| `htui-core/src/model/run.rs` | §2.4 and its tests |
| `htui-core/src/fixtures.rs` | `:1345` `persona: None`; `:1379` `personas: Vec::new()`; B-3: `DemoData.personas` (doc "`persona` rows (MOD-26 D7): the two seeds, re-stamped"), `class::PERSONA: u8 = 20`, `ids::PERSONA_REVIEWER: PersonaId = (class::PERSONA, 0)`, `ids::PERSONA_ARCHITECT = (class::PERSONA, 1)`, `fn personas()` (`agents()`'s shape: `persona::seed_rows(epoch())`, length asserted against the id array, `Persona { id, ..seed }`), wired in `demo_data()` |
| `htui-core/src/store/conformance.rs` | `:4281` `personas: Vec::new()` (`:4854` spreads) |
| `htui-core/src/store/mem.rs` | `:9308` `personas: Vec::new()` only |
| `htui-core/src/prompt/mod.rs` | `PromptSpec.persona`, `PersonaBlock`, `From<&SnapshotPersona>` (§2.10, T0 half) |
| `htui-core/src/prompt/fixtures.rs`; `htui-core/tests/prompt_digest.rs`, `prompt_hostname.rs`, `prompt_skills.rs` | `persona: None` in non-spread `PromptSpec` literals |
| `htui-agent/src/driver.rs` | `ToolExposure.deny_kinds` and the struct doc (§2.11) |
| `htui-agent/src/excerpt.rs`, `htui-agent/tests/excerpt.rs` | `PromptSpec` literals only |
| `htui-agent/tests/relay.rs` | `:81` `personas: Vec::new()` only |
| `htui-store/tests/pg_criteria.rs` | `:1088` `personas: Vec::new()` only |
| `htui-orch/src/graph.rs` | `:364` `personas: Vec::new()`; `:675` `persona: None`; `topology`'s doc (B-11) |
| `htui-orch/src/engine.rs` | literals `:4734`, `:5457` (`persona: None`), `:6844`, `:7643` (`persona: None`) only |
| `htui-orch/src/fanout.rs` | `judge_phase` (`:290-304`) `persona: None` explicitly before the spread (OQ-2) + test |
| `htui-orch/src/promote.rs` | `handoff_spec` (`:89-103`) `persona: None` (OQ-5), `:416` + test |
| `htui/src/preview.rs` | `:281` `persona: None` (D13) |

`topology`'s new doc paragraph (replacing "nulls are emitted because no field carries
`skip_serializing_if`"): "…field order is [`SnapshotPhase`]'s declaration order and nulls are
emitted, **except `persona`** (MOD-26 D9), which is skipped when `None`, so a persona-less phase
serialises exactly as before and only a bound phase's digest names its persona. Any other field
added to `SnapshotPhase` changes every digest and is a `GraphSnapshot::V` question."

### 3.2 Commits

1. **red** — every shape of §2.1-§2.4, §2.10 (T0 half) and §2.11 (T0 half), every literal of
   §3.1, the seed files, `judge_phase`/`handoff_spec`/preview `None`, and every test of §3.3.
   Red bodies: `parse_file` → `Err(PersonaFileError::Refused("MOD-26 T0: red".to_owned()))`;
   `persona_refusal` → `None`; `freeze` → `Ok` with `digest: "sha256:red".to_owned()`;
   `persona_for` → `Ok(None)`; `seed_rows` → `Vec::new()`; `fixtures::demo_data()` sets
   `personas: Vec::new()`. Compiles workspace-wide; red: the parser, rule, seed, digest and
   `persona_for` tests fail at their first assertion.
2. **green** — the bodies, `fixtures::personas()` wired, the `topology` doc. `grep -rn "MOD-26 T0:
   red" crates` prints nothing.

### 3.3 Tests (written first)

`htui-core/src/model/persona.rs` `mod tests`:

| Test | Asserts |
|---|---|
| `a_claude_agents_shaped_file_parses` | `"---\nname: code-reviewer\ndescription: Reviews code\ntools: Read, Grep, Glob\n---\n\nYou review.\n"` → `name`, `description`, `allow == ["Read","Grep","Glob"]`, empty `deny`/`deny_kinds`, `command_run`, `default: None`, `body == "You review.\n"` |
| `the_model_key_is_refused_with_its_sentence` | a file with `model: opus` → `Model`; `Model.to_string() == MODEL_REFUSED`; `model` beats an unknown key that follows it |
| `an_unknown_key_is_refused_by_name` | `color: blue` → `UnknownKey { key: "color" }`, the sentence names it |
| `a_missing_closing_fence_is_refused` | no closing `---` → `Fence(Unterminated { at: 0 })`; no opening fence → `Fence(NoFence)` |
| `a_repeated_key_is_refused` | two `name:` lines → `Duplicate { key: "name" }` |
| `a_list_block_or_map_value_is_refused` | `tools: [Read]`, `tools:\n  - Read`, `description: \|\n  two\n  lines` → `NotOneLine` for that key; a quoted `description: "a, b: c"` parses |
| `command_run_and_permission_default_take_only_their_literals` | `command-run: false` → `false`; `command-run: no` → `CommandRun`; `permission-default: deny` → `Some(Deny)`; `allow` → `PermissionDefault` |
| `the_body_drops_one_leading_blank_line` | two blank lines after the fence keep one; CRLF input yields LF; `tools:` alone is the empty list |
| `a_parsed_file_meets_the_save_rules` | `deny-kinds: think` → `Refused(kind_not_narrowable("think"))`; `tools: mcp__gortex__search` → `Refused(allow_names_an_mcp_tool(…))`; `name: Reviewer` → `Refused(invalid_persona_name("Reviewer"))`; a fenced file with an empty body → `Refused(BLANK_PERSONA_BODY)` |
| `every_widening_shape_is_refused` | `persona_refusal` over a valid base varied one field at a time, each with its exact sentence: bad name; NUL description; blank body; NUL body; `allow` `""`, `"Read Grep"`, `"Read,Grep"`, `"Re\0ad"`, `"mcp__x__y"`; `deny` `"Bash Output"`; `deny_kinds` `think`, `switch_mode`, `other`, `Edit`; a rule with `PersonaMatch::default()`; NUL in a matcher's `path_prefix`; NUL in a rule `reason`. The valid base answers `None`. |
| `a_patch_is_checked_field_by_field` | each `Some` field of `PersonaPatch` meets the same sentence; the all-`None` patch answers `None` |
| `every_persona_shape_refuses_an_unknown_key` | `serde_json::from_value` with an extra `"color": 1` fails for `Persona`, `NewPersona`, `PersonaPatch`, `PersonaTools`, `PersonaPermission`, `PersonaRule`, `PersonaMatch`, `SnapshotPersona` (I-2) |
| `an_empty_blob_narrows_nothing` | `{}` decodes to `PersonaTools { command_run: true, .. }` and `PersonaPermission::default()` |
| `seed_rows_are_the_two_seed_files` | names `["reviewer", "architect"]`; both `deny_kinds == ["edit","delete","move"]`, `allow` empty, `deny` empty, `command_run`, `default: None`, no rules; bodies start `"You are the "`; both pass `new_persona_refusal`; stamps equal `now` |
| `seed_rows_mint_fresh_ids_per_call` | two calls, distinct ids, equal names |
| `freeze_digests_the_content_and_not_the_row` | same content under two ids and stamps → one digest; a body edit → another; `digest` is `"sha256:"` + 64 lowercase hex |
| `the_tool_kind_lists_are_closed` | `NARROWABLE_KINDS ⊂ TOOL_KINDS`, lengths 7 and 10, no duplicates |

`htui-core/src/model/run.rs` tests (add a `mod tests` if absent):

| Test | Asserts |
|---|---|
| `a_persona_less_phase_serialises_without_a_persona_key` | `serde_json::to_string(&phase)` has no `"persona"`; JSON without the key decodes to `persona: None` |
| `a_bound_phase_carries_its_persona_name` | the JSON ends `,"persona":"reviewer"}` |
| `a_snapshot_without_personas_omits_the_key` | no `"personas"` key when empty; old JSON decodes to `personas: []` |
| `persona_for_finds_refuses_and_skips` | persona-less → `Ok(None)`; carried → `Ok(Some(&…))`; absent → `Err(persona_not_in_snapshot("reviewer"))` |

`htui-core/src/fixtures.rs` tests: `the_fixture_personas_are_the_seeds_re_stamped` (names, fixture
ids, `epoch()` stamps). `htui-orch/src/fanout.rs`: `the_judge_phase_drops_the_persona`
(`SnapshotPhase { persona: Some("reviewer".into()), ..phase() }` → `judge_phase(…).persona ==
None`). `htui-orch/src/promote.rs`: `the_handoff_spec_drops_the_persona`. Both orch tests are green
on arrival (they pin T0's own literal edits); the `FEATURE_TOPOLOGY` pin (`graph.rs:879-880`)
must stay green unchanged.

### 3.4 Gate

G-T0 (§12). T0 runs alone on the primary tree; nothing in T0 touches Postgres.

---

## 4. T1 — store (worktree `hr/MOD-26-t1`; the wave's only Postgres lane)

### 4.1 Files

| File | Change |
|---|---|
| `htui-store/migrations/0012_persona.sql` (new) | §2.7, verbatim |
| `htui-core/src/model/kind.rs` | §2.5 |
| `htui-core/src/seed.rs` | `:216` `persona_id: None` |
| `htui-core/src/store/traits.rs` | §2.6 (methods, docs, re-export) |
| `htui-core/src/store/mod.rs` | the eleven re-exported names (B-2) |
| `htui-core/src/store/mem.rs` | §2.9, unit tests |
| `htui-core/src/store/conformance.rs` | five cases (§4.3), `new_phase` (`:2277`) and `:3680` literals |
| `htui-core/tests/mem_store.rs` | `119 → 124` (`:37`), the sentence gains "MOD-26 T1's five persona cases (plan D3-D5)" |
| `htui-store/src/pg/rows.rs` | `PersonaRow` (B-15) |
| `htui-store/src/pg/read.rs` | three readers, `phase_row`/`phase_rows`, `resolve_graph` (§2.8) |
| `htui-store/src/pg/write.rs` | three methods, three mappers, `persona_json`, `create_phase`/`update_phase` (§2.8) |
| `htui-store/src/pg/mod.rs` | the seed loop (§2.8) |
| `htui-store/src/pg/demo.rs` | persona delete + load, phase `persona_id`, doc (§2.8) |
| `htui-store/src/writer.rs` | three forwards |
| `htui-store/.sqlx/*` | regenerated (§12 `regen`) |
| `htui-store/tests/pg_conformance.rs` | `EXPECTED_CASES` `119 → 124` (`:23`), doc (`:17-22`), message (`:30-31`) |
| `htui-store/tests/pg_criteria.rs` | two tests (§4.3) |
| `htui-store/tests/migrations.rs` | pins (§11), `MOD26_COLUMN_COMMENTS`, seed test, demo count |
| `htui-store/tests/connect.rs` | `11 → 12` at `:140`, `:156`, `:242` and their sentences ("twelve … through MOD-26's 0012_persona.sql") |
| `htui-agent/src/conformance.rs` | `UsageSpy`: three `WriteStore` forwards only |
| `htui-agent/tests/recorder.rs` | `SpyStore`: three `WriteStore` forwards only |
| `htui/src/ui/tabs/settings/kinds.rs` | `:976` `persona: None` only |

### 4.2 Commits

1. **red** — `0012`; §2.5 and every literal; the four phase statements in their final form
   (§2.8, without `phase_persona_refused`: `map_sqlx` stays); `.sqlx` regenerated for them;
   §2.6 on the trait; MemStore and PgStore persona bodies `Err(StoreError::Backend("MOD-26 T1:
   red".into()))`; `Writer`/`UsageSpy`/`SpyStore` forwards; every test of §4.3 and every pin of
   §11 (T1 rows). Compiles offline workspace-wide. Red: the five cases fail at the placeholders
   (and the phase-refusal case on the raw `23503` text), the seed and demo-count tests find no
   persona.
2. **green MemStore** — §2.9. `nopg htui-core` green.
3. **green Postgres** — `PersonaRow`, readers, writers, mappers, `resolve_graph`, seed, demo,
   `.sqlx` regenerated and counted. `grep -rn "MOD-26 T1: red" crates` prints nothing.

### 4.3 Tests (written first)

Store conformance (generic `<S: WriteStore>`; run by `mem_store.rs` and `pg_conformance.rs`),
each appended to `CASES` after `adopt_runs_never_leases_a_chat_run` (`:167`), its arm before `other
=> panic!` (`:420`). Fixture: the demo store holds `architect` and `reviewer` (B-3); phases come
from the FEAT graph (`ids::GRAPH_HTUI_FEAT`) or `new_phase`.

| Case | Asserts |
|---|---|
| `personas_are_listed_by_name_and_created_once` | `personas()` is `[architect, reviewer]`; `create_persona(scout)` (`deny_kinds: ["execute"]`, body "You scout.") returns the input with `created_at == updated_at`; the list is `[architect, reviewer, scout]`; the same id under another name → `Constraint(already_exists("persona", id))`; a fresh id named `scout` → `Constraint(already_exists("persona", "scout"))`; the list is unchanged |
| `update_persona_is_a_compare_and_set` | unknown id → `NotFound { entity: "persona" }`; a spent token with a bad patch (`name: "Bad"`) → `Stale(current)`; a body patch → `Applied`, `updated_at` advanced, other fields kept; the old token → `Stale`; a rename to `reviewer` → `Constraint(already_exists("persona", "reviewer"))`; an all-`None` patch → `Applied` with a new `updated_at` |
| `persona_writers_refuse_every_widening_shape` | every shape of `every_widening_shape_is_refused` (§3.3) through `create_persona` and, as a patch, through `update_persona` on `reviewer` (fresh token): `Constraint` with the exact sentence on both stores; `personas()` is unchanged after all of them (nothing written) |
| `a_phase_naming_no_persona_is_refused` | `create_phase(new_phase(…, persona_id: Some(unknown)))` → `Constraint(references_no_row("step_graph_phase.persona_id", unknown, "persona"))`; the same with a taken position → a `Constraint` whose text is **not** that sentence (the clash wins, D5); `update_phase(…, Some(Some(unknown)))` → the persona sentence; `phases()` unchanged |
| `a_phase_persona_binding_sets_keeps_and_clears` | `Some(Some(reviewer))` → `Applied`, `persona_id == Some(reviewer)`; a name-only patch keeps it; `Some(None)` clears it; `phases()` reads each state back |

`mem.rs` unit tests: `resolve_graph_fills_the_bound_persona` (bind `reviewer` to FEAT's first
phase; `resolve_graph(ids::HTUI_FEAT_3)` → that phase's `persona` is the row, the others `None`)
and `persona_times_are_the_handles_clock` (`with_clock(TestClock::at(t))`: create and update stamp
`t`). `pg_criteria.rs` (`common::demo_db()`, `db.drop_db().await` at the end):
`resolve_graph_carries_the_bound_persona_as_mem_store_does` (the same binding on Pg and on
`MemStore::demo()`; the two `ResolvedPhase.persona` and `phase.persona_id` agree,
`inherent_orchestration_reads_answer_the_fixture`'s shape `:3734-3760`) and
`a_bound_persona_cannot_be_deleted` (raw `DELETE FROM persona WHERE id = $1` on a bound persona →
`23503` naming `fk_step_graph_phase_persona`; the row survives). `migrations.rs`:
`seeding_adds_both_personas_once_and_keeps_an_operator_edit` (`:1522`'s shape: `fresh_db` holds
`architect` and `reviewer`; SQL edits `reviewer.body` and deletes `architect`;
`seed_if_empty_as` again → `architect` is back, `reviewer.body` is still the edit, two rows).

### 4.4 Gate

G-T1 (§12), in the T1 worktree, every Postgres command under the lock.

---

## 5. T2 — `narrow` and enforcement (worktree `hr/MOD-26-t2`)

### 5.1 Files

`htui-agent/src/persona.rs` (new, §2.11), `htui-agent/src/lib.rs` (`pub mod persona;`),
`htui-agent/src/cli/mod.rs` and `cli/claude.rs` (§2.12), `htui-agent/src/acp/mod.rs` (§2.13),
`htui-agent/tests/cli_driver.rs`, `htui-agent/tests/relay.rs`, `htui-agent/tests/acp_driver.rs`.

### 5.2 Commits

1. **red** — `persona.rs` with `narrow` answering `(base.clone(), policy.clone())` and
   `tool_kind_of`; `claude::tool_names` answering `&[]`; the argv push block in place (it emits
   nothing while `tool_names` is empty and the test exposures name only kinds); `TOOL_KIND_DENIED`,
   `denied`, the `deny_kinds` threading with **no** guarded arms; every test of §5.3. Red: the
   I-1 clauses, the inversion round trip, the kind argv case, the relay case and the two ACP
   refusals fail.
2. **green** — the bodies and the two guarded arms.

### 5.3 Tests (written first)

`persona.rs` unit tests (each names the I-1 clause it pins in its doc):

| Test | Asserts |
|---|---|
| `core_tool_kinds_spell_tool_kind_all` | `TOOL_KINDS` equals `ToolKind::ALL.map(as_str)`, same order (D2, D10) |
| `allow_intersects_and_an_empty_base_means_everything` | base `[]` + persona `[Read, Grep]` → `[Read, Grep]`; base `[Read, Bash]` + persona `[]` → base; base `[Read, Bash, Grep]` + persona `[Grep, Read, Edit]` → `[Read, Grep]` and `Bash` joins `deny` |
| `disjoint_allow_lists_leave_no_tool` | base `[Bash]` + persona `[Read]` → `allow == [Bash]`, `deny ⊇ [Bash]` (B-8) |
| `deny_and_deny_kinds_only_grow` | base entries kept first, persona entries appended once each |
| `command_run_is_the_conjunction` | the four combinations |
| `persona_rules_run_first_then_kind_rules_then_base_rules` | order and reasons (`persona reviewer denies edit`; an empty rule reason → `persona reviewer`), answers `RejectOnce`/`RejectAlways` mapped |
| `a_remembered_allow_loses_to_a_persona_kind_reject` | base `remembered` holds an `allow_always` for kind `edit`; `permission::evaluate(&narrowed, Some(&edit_call), &options)` answers the reject option at `PolicyStage::Rule` |
| `default_is_the_stricter_of_the_two` | 3 × 3 table over base `Allow/Ask/Deny` × persona `None/Ask/Deny` |
| `an_unknown_kind_string_is_denied_as_other` | `deny_kinds: ["bogus"]` → `ToolKind::Other` denied (B-19) |
| `narrow_never_widens` | over a small grid of bases × personas: `allow` ⊆ base (or base empty), `deny ⊇ base.deny`, `deny_kinds ⊇ base.deny_kinds`, `!command_run || base.command_run`, `strictness(default) >= strictness(base.default)`, base rules a suffix of the result |

`cli/claude.rs` unit: `every_inverted_name_maps_back_to_its_kind` (for every kind and every name
in `tool_names(kind)`, `tool_kind(name) == kind`; `delete`/`move`/`think`/`switch_mode`/`other`
are empty). `tests/cli_driver.rs` (the exact-`Vec` and `windows(2)` style of `:84-121`):
`persona_flags_follow_the_pairs_and_precede_extra_args` (allow `[Read, Grep]`, deny
`[mcp__x__y]`, kinds `[Execute]`, `extra_args ["--foo"]` → `…, "--tools=Read,Grep",
"--disallowedTools=mcp__x__y,Bash,BashOutput,KillShell", "--foo"` right after the last pair),
`allowed_tools_is_never_emitted` (no argument starts with `--allowedTools` for any exposure of
the grid), `tools_is_absent_when_allow_is_empty`, `deny_kinds_edit_yields_the_four_claude_names`
(`--disallowedTools=Edit,Write,MultiEdit,NotebookEdit`), `a_default_exposure_adds_no_flag` (I-7).

`tests/relay.rs`: a `call_of(title, kind)` helper (`call` keeps its signature and delegates with
`ToolKind::Execute`, `:209-222`); `a_persona_denied_kind_is_rejected_by_policy`: an `edit` call
then `park(…)`, policy `narrow(&ToolExposure::default(), &PermissionPolicy::default(),
&reviewer).1` (the base is `Ask`), `drive(…, Some(&relay), …)` → the session got
`Selected(REJECT)`, no relay row, the recorded answer is `by: "policy"` with reason `persona
reviewer denies edit` (`a_policy_answer_writes_no_relay_row_and_records_by_policy`'s shape,
`:637-683`).

`tests/acp_driver.rs`: a raw newline-delimited JSON-RPC `fs_agent(stream, request: Value,
answered: oneshot::Sender<Value>)` in `refuse_session_new`'s style (`:252`): answers `initialize`
(`protocolVersion: 1`) and `session/new` (`sessionId`), and on `session/prompt` sends `request`
with id 100 (the `sessionId` filled in), forwards the client's response line to `answered`, then
answers the prompt `{"stopReason": "end_turn"}`. Driven through `open_session` with `spec(cwd)`
(`:93`) and the spec's `tools.deny_kinds` set; the client capabilities must advertise fs read and
write (`client::client_capabilities`, settings in `options`, `:112`). Cases:
`a_denied_write_is_refused_before_any_edit_proposal` (deny `[Edit]`, `fs/write_text_file` on a
file in `cwd` → an error response with code `-32602`, the file's bytes unchanged, an `Error {
code: "tool_kind_denied" }` event, **no** `EditProposal`), `a_denied_read_is_refused` (deny
`[Read]`, `fs/read_text_file` → `-32602`, the event), `an_undenied_write_still_lands` (deny
`[Read]` only: the write lands and its `EditProposal { accepted: Some(true) }` is emitted). Each
spawned task is joined or aborted at the end of its test.

### 5.4 Gate

G-T2 (§12), in the T2 worktree, with the DSN unset.

---

## 6. T3 — prompt frame (worktree `hr/MOD-26-t3`)

### 6.1 Files

`htui-core/src/prompt/mod.rs` (assembly only; T0 added the field), `prompt/template.rs`,
`prompt/render.rs`, `prompt/trim.rs` (the two arms, B-7), `htui-core/tests/prompt_persona.rs`
(new).

### 6.2 Commits

1. **red** — `SectionName::Persona` (render, protected), `Placeholder::Persona` (token,
   `allowed_in`), `render::persona`, the `render_sections` and two `trim.rs` arms,
   `PERSONA_SEPARATOR`, the scrub of `spec.persona`, and every test of §6.3; `assemble` still
   pushes nothing for a persona. Red: the frame, protection, separator and digest cases fail.
2. **green** — the push before `:486`, the frame estimate (`:525`), the `substitute` prefix.

### 6.3 Tests (written first)

`tests/prompt_persona.rs` (spec = `fixtures::phase_implement_attempt2()` with `persona:
Some(PersonaBlock { name: "reviewer", body: "You are the reviewer.\n" })`, scrubber
`MinimalScrubber::new([])` unless stated):

| Test | Asserts |
|---|---|
| `a_persona_renders_first_and_is_sections_one` | `text` starts `<section name="persona" persona="reviewer">\nYou are the reviewer.\n</section>\n\n`; `sections[0].name == Template`, `sections[1].name == Persona`; `payload_sections()[1].name == "persona"`; `digest_text` starts with the same frame |
| `the_persona_section_survives_any_trim` | at the protected floor (`at_target`'s method, `prompt_digest.rs:607`) every trimmable section is trimmed or dropped and the persona row is untouched with the frame still first; one token less → `BudgetTooSmall` whose `needed` includes the persona's tokens |
| `a_secret_in_the_persona_body_is_masked` | a session secret in the body: `[REDACTED]` in `text` and `digest_text`, the secret in neither |
| `an_unmaskable_persona_body_refuses_the_prompt` | a prefix-rule secret → `Unmasked { section: "persona", .. }` |
| `a_template_cannot_place_the_persona_placeholder` | a body with `{{persona}}` → `UnknownPlaceholder { token: "persona" }`; `Placeholder::from_token("persona") == None`; `Placeholder::ALL.len() == 20`; `Placeholder::Persona.allowed_in(role)` is `false` for all three roles |
| `the_digest_changes_with_the_persona_body` | two bodies, two digests; persona `None` vs `Some` differ too |
| `the_separator_is_counted_in_the_frame` | `sections[0].tokens_before == est.estimate(&(template_text + "\n\n"))` with a persona and `== est.estimate(&template_text)` without (B-17, R-6) |
| `a_persona_less_spec_is_unchanged` | the spec with `persona: None` assembles to the same `text`, `digest` and `trim` as before T3 — proven by the untouched goldens of `tests/prompt_digest.rs`; this test asserts no section is named `persona` and `text` has no `name="persona"` |

`mod.rs` unit tests: `section_names_are_one_vocabulary_with_two_uses` gains `Persona.render() ==
"persona"`; `the_protected_set_is_role_dependent_only_for_failure_reason` lists `Persona` among
the protected in every role. `render.rs` unit: `the_persona_section_escapes_its_name` (`a"b&c` →
`persona="a&quot;b&amp;c"`).

### 6.4 Gate

G-T3 (§12), in the T3 worktree, with the DSN unset. `git diff --stat <T0 sha> -- crates/htui-core/tests/`
lists only `prompt_persona.rs` (goldens untouched, I-7).

---

## 7. Wave merge (primary, no lane running)

After T1, T2 and T3 have each committed their last green commit and passed their gate:

```bash
cd /home/mluigi/projects/htui            # primary, on hr/MOD-26, clean
git merge --no-ff hr/MOD-26-t1 -m "Merge branch 'hr/MOD-26-t1'"
git merge --no-ff hr/MOD-26-t2 -m "Merge branch 'hr/MOD-26-t2'"
git merge --no-ff hr/MOD-26-t3 -m "Merge branch 'hr/MOD-26-t3'"
```

Expected conflicts: none (disjoint files). Then G-W1 on the merged tree. Hidden couplings G-W1
exists to catch: T1's fixture personas reach every `MemStore::demo()` harness (orch, worker,
htui); T1's `0012` makes every Postgres harness migrate one more file and seed two rows; T3's frame
code runs for every prompt (persona-less goldens must hold workspace-wide); T2's argv block runs in
every CLI test. Then `git worktree remove --force` each of the three worktrees and `git branch -d`
each branch, in that order.

---

## 8. T4 — engine wiring (serial, primary)

### 8.1 Files

`htui-orch/src/graph.rs` (freeze, §2.14), `htui-orch/src/engine.rs` (§2.14, unit tests),
`htui-orch/src/conformance.rs` (five cases, `bind_persona`), `htui-orch/tests/fake_conformance.rs`
(pin). `fake.rs` and `status.rs` are untouched (B-13, B-5).

### 8.2 Commits

1. **red** — every signature of §2.14 (`StageThree::NoPersona`, `phase_spec`'s inner error,
   `refuse_prompt(…, String)`, `session`/`drive_once` `persona`, `CandidateStage.persona`, the new
   match arms) with `persona_for` **not yet called** (stage 3 and stage 4 pass `None`, `phase_spec`
   sets `persona: None`) and `resolve` not yet freezing; every test of §8.3 and both `CASES` pins.
   Red: every persona case and test fails; every existing test stays green.
2. **green** — the freeze, the lookups, the frame, the narrowing, the `command_queue` rule.

### 8.3 Tests (written first)

Orch conformance (`CASES` `:385`, 86 → 91), generic over `O: Orchestrate`, through a helper:

```rust
/// MOD-26: binds `persona` (by name, read through `personas()`) to `item`'s phase `phase` with
/// `update_phase`, `repoint`'s shape (`:948-992`); answers the persona row.
async fn bind_persona<O: Orchestrate>(orch: &O, item: ItemId, phase: &str, persona: &str) -> Persona
```

| Case | Asserts |
|---|---|
| `a_persona_bound_phase_prompt_carries_the_persona_section` | `reviewer` bound to FEAT-3's first phase; after `StartRun` the step's `prompt_sections` (`:2699`) are `["template", "persona", …]` and its recorded prompt text starts with the frame |
| `a_persona_edit_after_start_run_does_not_reach_the_run` | bind to the second phase; `StartRun` (the run parks at the first gate); `update_persona(reviewer, body: "changed")`; answer the gate; the second step's prompt carries the **old** body and the snapshot's `personas[0].digest` is the pre-edit digest (I-3) |
| `the_judge_step_runs_without_a_persona` | a judged fan-out phase with `reviewer` bound: candidates' prompts carry the frame at `sections[1]`, the judge's prompt has no `persona` section while its `judge_task` text contains the candidates' frame (OQ-2) |
| `a_persona_without_command_run_drops_the_command_queue_section` | a created persona with `command_run: false` bound to a phase whose `command_queue` is on: no `command_queue` section; the same phase unbound keeps it |
| `a_persona_run_leaves_the_agent_row_untouched` | the candidate agent's row (`updated_at`, `settings`) is identical before and after a persona-bound run (PRD metric "agent row untouched") |

Engine unit tests (`engine.rs` `mod tests`; `SpecSpy` extended to keep `(SessionSpec, String)`,
the prompt no longer dropped at `:11951`; `a_tree_outside_the_cwd_reaches_the_session_as_an_extra_dir`
reads `.0`):

| Test | Asserts |
|---|---|
| `a_persona_narrows_the_session_spec` | `reviewer` bound: the spy's spec has `tools.deny_kinds == [Edit, Delete, Move]`, `tools.allow` empty, the first three `permission.rules` are the kind rejects, the agent's rules follow; the prompt starts with the frame |
| `a_persona_less_step_spec_is_unchanged` | no binding: `tools == ToolExposure::default()`, `permission == ` the agent's policy (I-7) |
| `the_relay_rejects_a_kind_the_persona_denies` | B-13: persona `no-shell` (`deny_kinds: ["execute"]`) bound; `set_policy(agent, Ask)`; `ScriptedStep::parks(…)`: the parked `execute` request is answered reject **by policy**, no relay row is written, the recorded `permission_answer` is `by: "policy"` |
| `a_snapshot_without_its_persona_refuses_at_stage_three` | `assemble_prompt` on a snapshot whose `phases[0].persona = Some("reviewer")` and `personas` empty (the `:13287` call shape) → `Ok(Err(StageThree::NoPersona(persona_not_in_snapshot("reviewer"))))` |
| `a_claimed_run_whose_snapshot_lost_its_persona_fails_the_step_and_settles` | I-4 end to end: `graph::resolve` a `reviewer`-bound FEAT-3, clear `snapshot.personas`, `create_run` with it (`enqueue`'s literal, `:688-700`), `engine.claim(run)` → `Started { rest }` with `rest.run == Failed` and `rest.failure == Some(PromptRefused { phase, reason: persona_not_in_snapshot("reviewer") })`; the step is `failed` with no `prompt_digest`; the item is `blocked`; no driver was started (the driver closure counts zero calls); `claim` returned `Ok`, not `Err` |
| `rebinding_a_phase_persona_parks_a_resumed_run` | the shape of the tests built on the `parked_run_with_a_moved_graph` helper (`:11518`): a run parked at a gate, the next phase rebound to `architect`, `resume` → parked on topology; a body edit of the bound persona instead does **not** park |

`graph.rs` unit tests: `start_run_freezes_each_bound_persona_once` (two phases bound to `reviewer`,
one to `architect`: `personas` is `[architect, reviewer]` with `freeze` digests, each phase names
its persona), `only_a_bound_phase_moves_the_topology` (the persona-less FEAT digest equals
`FEATURE_TOPOLOGY`; binding one phase changes it), `an_unbacked_persona_id_is_refused_at_resolution`
(a test `GraphSource` returning a `ResolvedPhase` with `persona_id: Some(id)` and `persona: None`
→ `ResolveError::Store(Constraint(references_no_row("step_graph_phase.persona_id", id,
"persona")))`, B-18).

### 8.4 `drive_once`'s three callers

`walk_live_step` → `session(run, step, phase, persona, &prompt, …)` (`:3472`) → `drive_once(…,
phase, persona, &SessionKey::of(step), …)` (`:5680`); `run_candidate` → `drive_once(…, phase,
stage.persona, &key, …)` (`:4078`); `judge_calls` → `drive_once(…, jp, None, &key, …)` (`:4974`).

### 8.5 Stack headroom (F-23)

Each new `run_case` arm awaits `Box::pin(case(orch)).await` if `every_case_name_dispatches`
(`conformance.rs:6829`) aborts; the gate runs `--no-fail-fast` and greps `SIGABRT`.

### 8.6 Gate

G-T4 (§12).

---

## 9. T5 — docs (serial, last)

- `docs/personas.md` (new): what a persona is (registry row, global, no model, I-5); the file
  format (§2.2's grammar, the seven keys, `model` refused, rules not expressible in a file); the
  narrow-only rule (I-1, every clause of `narrow`); precedence "agent row → persona narrows; model
  from the phase candidate"; freezing (I-3, rebinding parks, editing does not); the D11
  enforcement matrix with every residual (the shell; ACP calls run without asking; a request with
  no prior `tool_call`; `delete`/`move` have no CLI tool; persona `rules`/`default` inert on the
  CLI; an operator `extra_args` flag comes last); OQ-6's provenance (the `persona` entry of
  `sections[]`, the prompt digest, the snapshot's `{name, digest}`); M1's scope (no TUI, no
  delete, no chat/promoted application).
- `README.md`: one pointer line to `docs/personas.md`.
- `.claude/prds/mod-26-agent-personas.prd.md`: milestone 1 row → done, with the moved counts.
- `.claude/plans/mod-26-agent-personas.plan.md`: status line, acceptance boxes, the close-out
  counts (§11).
- Gate: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`, then G-Final.

---

## 10. Wave schedule and lane rules

| Wave | Tasks | Where | Postgres |
|---|---|---|---|
| W0 | T0 | primary `hr/MOD-26` | none |
| **W1** | **T1 ∥ T2 ∥ T3** | worktrees `hr/MOD-26-t1`, `-t2`, `-t3` | T1 only |
| W1-merge | merge, G-W1 | primary, no lane running | yes, under the lock |
| W2 | T4 | primary | none in its gate |
| W3 | T5, G-Final | primary | full gate |

1. **Branch first, from T0's last green commit** (`T0=$(git rev-parse HEAD)` after G-T0):
   `git worktree add -b hr/MOD-26-t1 /home/mluigi/projects/htui-wt/mod-26-t1 $T0`, likewise
   `-t2`, `-t3`. `df -h /` first: three worktree `target/`s are ≈ 30 GB (131 GB free at HEAD).
2. **File ownership.** A lane edits only the files of its §n.1 table. Shared shapes (§2) that look
   wrong are reported to the main thread, never fixed in the lane. No lane edits a T0 file except
   the ones its table names (T1: `store/conformance.rs`, `store/mem.rs`, `pg_criteria.rs`; T3:
   `prompt/mod.rs`) — and only the parts named.
3. **Native tools in worktrees.** Gortex does not index the worktrees and its edit tools write the
   primary checkout: lanes read, search and edit with native tools, by absolute worktree path.
4. **Crate-scoped commands only.** T1: `-p htui-core` and `-p htui-store` (and `cargo sqlx` inside
   its `crates/htui-store`), plus the offline workspace `check`; T2: `-p htui-agent`; T3: `-p
   htui-core`. No lane runs a workspace test, `cargo fmt --all` or a workspace clippy; `cargo fmt -p
   <crate> -- --check` instead.
5. **One Postgres lane; `.sqlx` is T1's alone.** Every command with `HTUI_TEST_DATABASE_URL` set
   and every `cargo sqlx` command runs under `flock /tmp/mod26-pg.lock`. T1's scratch database is
   `htui_sqlx_mod26` (recipe: `docs/hr-sandbox.md:194-210`; `psql -h localhost -p 5439 -U postgres
   -c 'CREATE DATABASE htui_sqlx_mod26;'`; migrate it again after writing `0012`). The **test**
   database is `HTUI_TEST_DATABASE_URL` (scratch DB per test, `testkit.rs:151-169`); the prepare
   database is not the test database (auto-memory `sqlx-prepare-needs-migrated-scratch-db`). While
   new queries have no `.sqlx` entry, T1 builds online with `DATABASE_URL=<scratch>`; every
   **commit** builds offline (`SQLX_OFFLINE=true`), so `regen` runs before each commit that adds or
   changes a query. T2 and T3 run with `env -u HTUI_TEST_DATABASE_URL -u DATABASE_URL`.
6. **`--all-features`, `--test-threads=1`** on every test command.
7. **Commit incrementally**, red then green, on the lane's branch, with the attribution trailer.
   **Never stash.** An uncommitted lane is lost with its session.
8. **After every lane and gate**: `pgrep -af 'htui worker'` prints nothing; no test process is
   left behind (`pgrep -af 'target/debug/deps'`); `ls ~/.config/htui` shows no `trees/<run>` a test
   made; `df -h /` before any Postgres-heavy gate (a crash loop is disk pressure first).

---

## 11. Pins

| Pin | Now | After | Where it moves |
|---|---|---|---|
| Store conformance `CASES` | 119 | 124 | T1 (`htui-core/tests/mem_store.rs:36-37` and its sentence; `htui-store/tests/pg_conformance.rs:23`, doc `:17-22`, message `:30-31`) |
| `READ_CASES` | 14 | 14 | — (`conformance.rs:456`) |
| `htui-orch` `CASES` | 86 | 91 | T4 (`htui-orch/src/conformance.rs:6795` in `cases_are_unique_and_counted` `:6788`; `htui-orch/tests/fake_conformance.rs:18` in `cases_len_is_pinned` `:15`, message `:19-20` "…and MOD-26's five persona cases (plan D9, D12, D13) 91") |
| Migrations | 11 | 12 | T1: `migrations.rs` version vector `:93` (+ `12`) and its sentence `:94-99` ("…and MOD-26's 0012_persona.sql"); `Pending(11)` `:922` + message `:923`; `:1021`; `MigrationsPending(11)` `:1026`, `:1045`; `_sqlx_migrations` count `11` `:1195` + message `:1196-1197`; `connect.rs:140` (+ `:141-142`), `:156` (+ `:157-158`), `:242` (+ `:243-244`) |
| Postgres tables | 41 | 42 | T1: `migrations.rs` `TABLES` (doc `:25-27`; a `// 0012_persona.sql (MOD-26)` group with `"persona"` after `:72`), `41,` `:117`, messages `:118-120`, `:126-127` |
| Column comments | 35 | 44 | T1: `MOD26_COLUMN_COMMENTS` (nine, §2.7) after `:443`, chained at `:460-465`, `:516-521`, `:531-536`; message `:482` ("…MOD-23, MOD-33 or MOD-26"); "thirty-five … thirty-sixth" `:505-506` → "forty-four … forty-fifth" (and "…MOD-33 and MOD-26 wrote"); `:542` "forty-four" |
| `load_demo` tables | 25 | 26 | T1: `migrations.rs:1848-1874` (`tables`) and `:1883-1912` (`expected`) gain `persona` (`[(&str, usize); 26]`); `:1932` `if *table == "agent" \|\| *table == "persona"` (replaced, not added) |
| `.sqlx` files | 307 | 307 − 4 replaced + new | T1: the four phase statements change hash; new: three readers, insert, update, seed insert, demo delete, demo insert, demo phase insert (changed). Counted with `ls crates/htui-store/.sqlx \| wc -l` and restated in commit 3's message |
| `Placeholder::ALL` | 20 | 20 | — (`template.rs:428-432`) |
| `WriteStore` methods | — | +3 | T1 |
| `GraphSnapshot::V` / `RECORD_VERSION` | 1 / 4 | 1 / 4 | — (D9, OQ-6) |
| `FEATURE_TOPOLOGY` | unchanged | unchanged | — (`graph.rs:879-880`, I-7) |
| `crates/htui/tests/snapshots`, prompt goldens | unchanged | unchanged | — (no TUI; persona-less byte-identical) |
| Unchanged | | | `StoreRequest`/`StoreReply`, `GraphSource`, `EngineParts`, `RunFailure`, `TrimRecord`, `MIRRORED_TABLES`, `PolicyFor`, workspace members |

Each moved pin's message names its reason ("MOD-26 T1's five persona cases", "MOD-26's
0012_persona.sql"). Implementers re-count at each gate; T5's close-out restates every row from a
fresh count.

---

## 12. Gate reference

```bash
LOCK="flock /tmp/mod26-pg.lock"                              # anything touching Postgres
NOPG="env -u HTUI_TEST_DATABASE_URL -u DATABASE_URL"         # Postgres suites skip
pg()   { $LOCK cargo test -p "$1" --all-features -- --test-threads=1; }
nopg() { $NOPG cargo test -p "$1" --all-features -- --test-threads=1; }
lint() { cargo clippy -p "$1" --all-targets --all-features -- -D warnings; }
fmtp() { cargo fmt -p "$1" -- --check; }
offline() { $NOPG SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features; }
# T1's prepare database (docs/hr-sandbox.md:194-210), created once:
#   psql -h localhost -p 5439 -U postgres -c 'CREATE DATABASE htui_sqlx_mod26;'
SQLX_DB=postgres://postgres@localhost:5439/htui_sqlx_mod26
regen() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB sh -c \
  'cargo sqlx migrate run --source migrations && cargo sqlx prepare -- --all-targets --all-features'); }
check() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB \
  cargo sqlx prepare --check -- --all-targets --all-features); }
```

| Gate | Commands |
|---|---|
| G-T0 | `nopg htui-core`; `$NOPG cargo test -p htui-orch --all-features --lib -- fanout promote graph --test-threads=1`; `$NOPG cargo test -p htui-agent --all-features --test relay -- --test-threads=1`; `offline` (**the whole workspace compiles with no SQL change**); `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo fmt --all -- --check`; `grep -rn "MOD-26 T0: red" crates` empty |
| G-T1 (worktree) | `regen` then `check`; `pg htui-core`; `pg htui-store`; `offline`; `lint htui-core`; `lint htui-store`; `fmtp htui-core`; `fmtp htui-store`; `grep -rn "MOD-26 T1: red" crates` empty; `ls crates/htui-store/.sqlx \| wc -l` restated |
| G-T2 (worktree) | `nopg htui-agent`; `lint htui-agent`; `fmtp htui-agent` |
| G-T3 (worktree) | `nopg htui-core`; `lint htui-core`; `fmtp htui-core`; `git diff --stat $T0 -- crates/htui-core/tests/` lists only `prompt_persona.rs` |
| G-W1 (merged, no lane running) | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `offline`; `check`; `$LOCK cargo test --workspace --all-features -- --test-threads=1` |
| G-T4 | `$NOPG cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 \| tee /tmp/mod26-t4.log; grep -n SIGABRT /tmp/mod26-t4.log` (empty); `nopg htui-worker`; `$NOPG cargo test -p htui --all-features run_worker -- --test-threads=1`; `lint htui-orch`; `cargo fmt --all -- --check` |
| G-Final | the plan's Validation list: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `offline`; `$LOCK cargo test --workspace --all-features -- --test-threads=1`; `check`; `$NOPG cargo insta test --workspace --all-features` with nothing pending; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; the §10 rule 8 checks |
