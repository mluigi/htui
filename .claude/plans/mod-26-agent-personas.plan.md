# Plan: MOD-26 — Declarative agent personas, milestone 1 (registry rows applied by the engine)

**Status: CONFIRMED by the maintainer 2026-10-02, OQ-1 to OQ-6 as recommended. Fact-checked 2026-10-02 (handoff-run step 3.5: three independent verifiers —
store, orchestrator/prompt, agent/transports with an offline `claude` 2.1.286 probe; falsified and
partly-true claims amended in place, see "Verified claims").**

**Source PRD**: `.claude/prds/mod-26-agent-personas.prd.md`, milestone 1, with its gate decisions
(maintainer, 2026-10-02; cited as **PRD Q-storage**, **Q-model**, **Q-binding** (revised to the
phase), **Q-authoring**, **Q-seeds**, **Q-enforcement**). Design origin: `docs/ANA-13.md` §3.1,
§4; `docs/ANA-16.md` §6.2, §8; `docs/ANA-27.md` §5.1 T9.

**Requirements**: `R-AGT-4`, `R-AGT-8`, `R-ID-3`, `R-ID-5`, `R-PRM-1`, `R-PRM-3`, `R-HIS-1`,
`R-STO-5` (forward-only migration).

**Complexity**: Large. One migration (`0012`), one new table and one new column, three new
`WriteStore` methods, a frontmatter parser and two seeds, a snapshot field pair, a protected prompt
frame, a narrowing function in `htui-agent`, two transport enforcement points, and the engine wiring.

**Routing**: `/handoff-run MOD-26`, PRD path (C2, C3, C4). Ultracode accepted for the implement
phase. Reviewer: `rust-reviewer` (`.claude/workflow-config.json`).

**Numbering**: decisions **D1…**, tasks **T0…**, risks **R-1…**, open questions **OQ-1…**,
invariants **I-1…**.

**Tree reading**: HEAD `e89b38b6` (branch `hr/MOD-26`, sandbox). Three Gortex readers (store,
orchestrator, agent transports), direct reads of the seams, then the fact-check. Paths are
relative to `crates/` for code, the repo root for docs.

---

## Open questions for the maintainer (read these first)

Each has a recommended default; CONFIRM without comment takes all of them.

- **OQ-1 — Tool vocabulary (PRD open question "agent-neutral or agent-scoped").** ACP carries no
  tool *name*: `ToolCallEvent` is `{tool_call_id, title, tool_kind, input, locations}`
  (`htui-agent/src/event.rs:303-316`) and a rule's `tool_name` matches the prose `title`
  (`htui-agent/src/permission.rs:126-145`). The only vocabulary every transport shares is
  `ToolKind` (`event.rs:44-65`). **Recommended:** a persona narrows on two axes — `deny_kinds`
  (agent-neutral, enforced on every transport) and tool **names** (`allow` = built-in tools to keep,
  `deny` = tools to remove; agent-native, enforced only on `claude-cli`'s argv). Names on an ACP
  agent are kept on the row but cannot be enforced; the persona docs say so.
- **OQ-2 — Judges.** `fanout::judge_phase` builds the judge's phase with `..phase.clone()`
  (`htui-orch/src/fanout.rs:285-305`, spread at `:303`). **Recommended:** the judge never runs
  under the phase's persona (`persona: None` set explicitly). Its `{{task}}` still replays the
  candidate's recorded prompt (`engine.rs:4640-4660`), persona block included, as the task the
  candidates were given.
- **OQ-3 — Prompt placement.** A template places sections through placeholders
  (`htui-core/src/prompt/mod.rs:490-498`), and no existing `prompt_template` row has a persona
  placeholder. **Recommended:** the persona block is an htui frame rendered **before** the template
  body whenever the step has a persona — an internal placeholder a template cannot name, protected
  from trimming. `template` stays `sections[0]` (P-9, `mod.rs:246`); `persona` is `sections[1]`.
  Persona-less prompts are byte-identical (goldens unchanged).
- **OQ-4 — Seed posture.** **Recommended:** both seeds deny kinds `edit`, `delete`, `move`, keep
  `execute` so a reviewer can run tests, and carry no `allow` list (the `.claude/agents` lists
  name Gortex MCP tools htui does not provide). The docs name the shell as the residual write path.
- **OQ-5 — Chat and promotion.** Two more `SessionSpec` builders exist outside the engine
  (`htui/src/agent_worker.rs:1037` in `bind_promoted`, `:2069` in chat `start`), and the
  promotion handoff prompt reuses `phase_spec` (`engine.rs:1367`, `promote.rs:81-106`).
  **Recommended:** M1 applies personas to engine steps only; `promote::handoff_spec` sets
  `persona: None`. A promoted step is a human at the keyboard with the agent's own posture.
- **OQ-6 — Enforcement record.** The PRD asked that "the step records which layers were in
  force". The layers are a fixed function of the transport (D11). **Recommended:** document the
  matrix; per-step provenance is the `persona` entry in the step's prompt `sections[]`, the prompt
  digest (which covers the block) and the snapshot's `{name, digest}` — no new record field, no
  `TrimRecord` version bump (a bump needs a migration restating the `run_step.trim_record`
  comment and four pins, see Verified claims).

## Summary

A persona is a global registry row (`persona`), like a skill: a name, a description, a prompt
body, a tool narrowing and deny-only permission rules. A `step_graph_phase` may name one through a
nullable `persona_id`. At `StartRun` the resolver freezes every referenced persona into the run's
`GraphSnapshot`; at each step the engine inlines the frozen body as a protected frame ahead of the
template and narrows the step's `ToolExposure` and `PermissionPolicy` through one pure function.
`claude-cli` receives the narrowing as `--tools=`/`--disallowedTools=`; the permission relay rejects
denied tool kinds on every transport that asks; htui's own ACP file handlers refuse denied reads
and writes. A Markdown/frontmatter parser in `htui-core` reads the two seed personas (`reviewer`,
`architect`) and is the M2 import's parser.

## Invariants (every task keeps these)

- **I-1 Narrow-only.** No persona makes a step's exposure or policy looser than the agent row's:
  effective `allow` ⊆ base (an empty base means "everything"), `deny` ⊇ base, `deny_kinds` ⊇ base,
  `command_run` = base ∧ persona, persona rules only reject and run before the base rules and the
  remembered choices, `default` = the stricter of the two (Allow < Ask < Deny). **No transport
  receives a flag that approves or adds a tool** (`--allowedTools` is never emitted: it
  auto-approves, verified).
- **I-2 Unknown keys refused.** Every persona shape is `deny_unknown_fields`; the frontmatter parser
  refuses an unknown key by name; `model` is refused with its own sentence (PRD Q-model).
- **I-3 Frozen per run.** A started run only ever reads personas from its snapshot. Editing a
  persona row never changes a started run's steps.
- **I-4 Fail closed.** A phase whose snapshot names a persona the snapshot does not carry fails
  **that step** with a named refusal (stage 3), never runs un-narrowed and never aborts the walk.
- **I-5 No model.** Personas never carry or influence a model (`R-AGT-8`).
- **I-6 Row is the truth (`R-ID-3`).** No file is read at run time; the seed `.md` files are
  `include_str!`'d and only seed rows.
- **I-7 Persona-less steps unchanged.** Topology digests, prompt text and goldens of steps without a
  persona are byte-identical to today.

## Design decisions

### Model and store

- **D1 — Migration `0012_persona.sql`.** `persona (id UUID PK, name TEXT NOT NULL CONSTRAINT
  uq_persona_name UNIQUE, description TEXT NOT NULL DEFAULT '', body TEXT NOT NULL, tools JSONB
  NOT NULL DEFAULT '{}', permission JSONB NOT NULL DEFAULT '{}', created_at, updated_at)` with an
  explicit `set_updated_at` trigger (`0006_requirements.sql:135-142`), and `ALTER TABLE
  step_graph_phase ADD COLUMN persona_id UUID NULL CONSTRAINT fk_step_graph_phase_persona
  REFERENCES persona(id) ON DELETE RESTRICT`. Header per `0009`; `COMMENT ON COLUMN` for every new
  column. Not mirrored (no cache migration; `schema_version` still bumps, so each box rebuilds its
  mirror once, as `0011` states).
- **D2 — Types (`htui-core/src/model/persona.rs`, new).** `PersonaId` (`ids.rs` `id_newtype!`),
  `Persona {id, name, description, body, tools: PersonaTools, permission: PersonaPermission,
  created_at, updated_at}`, `NewPersona`, `PersonaPatch` (all-`Option`),
  `PersonaTools {allow: Vec<String>, deny: Vec<String>, deny_kinds: Vec<String>, command_run: bool
  /* default true = keep base */}`, `PersonaPermission {default: Option<PersonaDefault /* ask |
  deny */>, rules: Vec<PersonaRule>}`, `PersonaRule {matcher: PersonaMatch {tool_kind, tool_name,
  path_prefix, command_prefix}, answer: PersonaAnswer /* reject_once | reject_always */, reason}`,
  `SnapshotPersona {name, digest, body, tools, permission}`. All `deny_unknown_fields`. Core
  cannot name `htui-agent` types (`htui-agent → htui-core`, verified), so kinds are strings
  validated against `TOOL_KINDS` in core, pinned to `htui_agent::ToolKind::ALL`'s spelling by a
  test in `htui-agent` (D10).
- **D3 — Save-time validation** (pure helpers in `store/traits.rs`, worded once for both stores):
  a new `invalid_persona_name` (reusing `validate_name`; `invalid_skill_name` hardcodes "skill.name",
  `traits.rs:1886`); no NUL; `deny_kinds` ⊆ {read, edit, delete, move, search, execute, fetch}
  (think, switch_mode, other are not narrowable); tool names non-empty, no whitespace, no comma
  (each list becomes one argv value, D11); `allow` names must not start with `mcp__` (`--tools`
  filters built-ins only — "deny an MCP tool by name instead"); `default` ∈ {ask, deny} and rule
  answers reject-only, by type; a rule with an all-`None` matcher refused ("use `default: deny`").
- **D4 — Store surface.** On `WriteStore`, beside the skill methods (`traits.rs:908-985`, rationale
  `:908-913`): `personas() -> Vec<Persona>` (by name, `COLLATE "C"`), `create_persona(NewPersona)
  -> Persona`, `update_persona(id, expected, PersonaPatch) -> CasOutcome<Persona>`. No delete in
  M1 (the FK is `RESTRICT`; skills have no delete either; M2 adds one with a "bound to phases"
  refusal). Implementors (verified complete): MemStore (`State.personas`, the `skills` precedent),
  PgStore (`pg/write.rs`; a `persona_insert_refused` mapper on `uq_persona_name`, the
  `skill_insert_refused` pattern `pg/write.rs:658-666`), `Writer`, `UsageSpy`
  (`htui-agent/src/conformance.rs:743`), `SpyStore` (`htui-agent/tests/recorder.rs:430`). No
  `StoreRequest` arms in M1 (the TUI surface is M2).
- **D5 — Phase binding.** `StepGraphPhase.persona_id: Option<PersonaId>`; `PhasePatch.persona:
  Option<Option<PersonaId>>` (outer `None` untouched, `Some(None)` clears; `COALESCE` cannot clear,
  `pg/write.rs:2794-2800`, so Pg uses `CASE WHEN $set THEN $val ELSE persona_id END`).
  `create_phase`/`update_phase` refuse a `persona_id` that references no row with
  `references_no_row` (`traits.rs:2112`): MemStore checks `State.personas` **after** the
  position/name clashes in `check_phase` (`mem.rs:2867-2897`, matching Pg's order: unique fires at
  insert, the FK after); Pg gets a **new** mapper keyed on `fk_step_graph_phase_persona` (today
  every `23xxx` is a raw `Constraint`, `htui-store/src/error.rs:42-46`). `override_graph`
  (`graph.rs:507`) copies the column through its existing spread. `seed_project`'s insert omits it
  (defaults to `NULL`); `seed::phase_row` (`htui-core/src/seed.rs:216`) sets `None`.
- **D6 — Resolution.** `ResolvedPhase` gains `persona: Option<Persona>`, filled by `resolve_graph`
  on both stores (Pg `pg/read.rs:1726-1753`: one query over the distinct `persona_id`s; MemStore
  `mem.rs:5146-5164`: from `State.personas`). No new `GraphSource` method, so `HostGraphs`,
  `WorkerHost`, `Backend` are untouched.
- **D7 — Seeds.** `htui-core/seeds/persona_reviewer.md` and `persona_architect.md` in the D8
  format, `include_str!`'d by `persona::seed_rows(now)`, inserted by `PgStore::seed_if_empty_as`
  with `ON CONFLICT (name) DO NOTHING` (`pg/mod.rs:480-501`: operator edits survive, a later seed
  is added). MemStore does not seed. `fixtures` gains the two rows; `pg/demo.rs` loads them after
  deleting the seeded personas by name (the agent precedent, `demo.rs:30-35`), so the two stores
  agree in conformance.

### Parser

- **D8 — Frontmatter format** (in `model/persona.rs`): `---`, flat `key: value` lines, `---`, then
  the body (one leading newline trimmed). Keys: `name`, `description`, `tools` (allow),
  `disallowed-tools` (deny), `deny-kinds`, `command-run` (`true|false`), `permission-default`
  (`ask|deny`). List values are comma-separated (`tools: Read, Grep, Glob`, the `.claude/agents`
  shape). `model` is refused with "a persona does not set the model; the phase candidate's model
  is used (MOD-26)"; any other unknown key (e.g. `color`) is refused by name. Permission rules are
  not expressible in frontmatter (M2's form edits them). Hand-rolled: the workspace has no YAML
  dependency (only `toml` and `serde_json`).

### Engine, prompt, transports

- **D9 — Snapshot.** `SnapshotPhase.persona: Option<String>` (the persona **name**) with
  `#[serde(default, skip_serializing_if = "Option::is_none")]` — hashed by `topology` when bound,
  absent otherwise (probe: byte-identical JSON when `None`; `FEATURE_TOPOLOGY` `graph.rs:879-880`
  stays green; the doc at `:251-270` is rewritten). `GraphSnapshot.personas: Vec<SnapshotPersona>`
  (`#[serde(default)]`, **not** hashed, like `scope` `run.rs:446-451`) carries the frozen content
  with `digest` = sha256 of the canonical JSON. Rebinding a phase parks a resumed run
  (`resume_window` re-resolves and compares topology, `engine.rs:2961-3035` — a graph change,
  correct); editing a persona's body does not (I-3). `GraphSnapshot::V` stays 1. `judge_phase`
  sets `persona: None` (OQ-2).
- **D10 — `narrow` (`htui-agent/src/persona.rs`, new).** `pub fn narrow(base: &ToolExposure,
  policy: &PermissionPolicy, persona: &SnapshotPersona) -> (ToolExposure, PermissionPolicy)`
  implementing I-1. `ToolExposure` gains `deny_kinds: Vec<ToolKind>` (the struct is already
  `#[serde(default)]`; no literal exists anywhere). Persona rules map to `PermissionRule`s with
  `RejectOnce`/`RejectAlways` (`event.rs:115-124`); each `deny_kinds` entry becomes `{match:
  {tool_kind}, answer: reject_once, reason: "persona <name> denies <kind>"}`; order = persona
  rules, kind rules, base rules; `remembered` kept (evaluated after rules,
  `permission.rs:57-92`). A test pins core's `TOOL_KINDS` to `ToolKind::ALL`.
- **D11 — Enforcement matrix** (documented in `docs/personas.md`, new). Seeds: `claude` and `agy`
  are ACP; only `claude-cli` is the CLI transport (`seeds/agent_claude_cli.json`).

  | Transport | names | `deny_kinds` | Residual |
  |---|---|---|---|
  | CLI (`claude-cli`) | `allow` → `--tools=<a,b>` (restricts the built-in set; omitted when empty — `""` disables all tools); `deny` → `--disallowedTools=<x,y>` | inverted from `cli/claude.rs:465-474` and appended to the deny list: read → `Read,NotebookRead`; edit → `Edit,Write,MultiEdit,NotebookEdit`; execute → `Bash,BashOutput,KillShell`; search → `Glob,Grep`; fetch → `WebFetch,WebSearch` (absent names are harmless, probed) | persona `rules`/`default` have no effect (no permission channel, `cli/mod.rs:554-562`); `delete`/`move` have no tool; the shell; an operator `extra_args` flag comes last |
  | ACP (`claude`, `agy`) | not enforceable (no name on the wire) | relay rejects a permission request whose call kind is denied; `fs/read_text_file`/`fs/write_text_file` refuse when `read`/`edit` is denied | a call run without asking and outside htui's fs handlers; a request with no prior `tool_call` of the same id has `call = None` and falls through to remembered/default (`record/relay.rs:277-321`); the shell |

  The two CLI flags are pushed directly after the scoped pair-push block (`cli/mod.rs:138-159`)
  and before `args.extend(cli.extra_args…)` (`:161`), each as one `=`-joined argument (the closure
  pushes pairs; the `=` form parses and the comma list splits, probed). ACP: `on_inbound`
  (`acp/mod.rs:1453-1550`) has no spec, so `session_main` (`:1060-1062`, call `:1265`) threads
  `&spec.tools.deny_kinds` in; the refusal comes **before** the write arm's
  `EditProposal{accepted: Some(true)}` emit (`:1508-1525`), answers `invalid_params`, and emits
  `DriverEvent::Error` with a new code beside `PATH_OUTSIDE_SESSION` (`acp/mod.rs:76`).
- **D12 — Stage 3 and `drive_once`.** The persona lookup lives in stage 3 (`phase_spec` /
  `assemble_prompt`, `engine.rs:5276-5505`): `phase.persona` resolved in `snapshot.personas`;
  absent → the I-4 refusal through `refuse_prompt`/`fail_before_a_token` (`engine.rs:5214`,
  `:5183`), failing the step, not the walk. The resolved `SnapshotPersona` is passed to
  `drive_once` (`:5761-5831`; callers `:4078` candidate, `:4974` judge with `None`, `:5680`
  session), which, after `let policy = …` (`:5782`), computes `(tools, policy) =
  narrow(&ToolExposure::default(), &policy, persona)`; the spec's `tools` (`:5797`) and
  `permission` (`:5799`) and the `Relay` (`:5812`) take the narrowed values. `drive_once` is the
  only `SessionSpec` site in `htui-orch`.
- **D13 — Prompt frame.** `SectionName::Persona` (spelled `persona`), an internal
  `Placeholder::Persona` kept **out** of `Placeholder::ALL` (pinned at 20, `template.rs:429`, and
  listed to users at `htui/src/ui/tabs/skills/templates.rs:1091`) so `from_token` never yields it;
  `token`, `allowed_in`, `is_section`, `render_sections` (`mod.rs:694`) gain arms. `assemble`
  pushes the rendered persona before the loop at `mod.rs:486`, so the scrub scan (`:501`) and the
  trimmer cover it; `is_protected` (`:306-312`) includes it; `Trimmer::source_of`'s exhaustive
  match (`trim.rs:503-507`) gains a fixed-source arm. `substitute` (`:621`) writes the block plus
  `"\n\n"` before the spans, so both the digest form and the sent form carry it. `PromptSpec.persona:
  Option<PersonaBlock {name, body}>`; `phase_spec` (`engine.rs:5457`) fills it and sets
  `command_queue: phase.command_queue != Off && persona.command_run` (`:5488`) — the one place
  `command_run` has an effect before MOD-11. Judge prompt (`engine.rs:4734`), Backlog preview
  (`htui/src/preview.rs:281`) and `promote::handoff_spec` (`promote.rs:89`) pass `None`.

### Scope guards

- No TUI surface, no `StoreRequest` arms, no import action, no delete (M2).
- No per-candidate binding (PRD out of scope; HANDOFF R-6).
- No chat/promoted application (OQ-5).
- No change to `PolicyFor`, `policy_lookup` or the worker runtime.
- No `TrimRecord` change, no `RECORD_VERSION` bump (OQ-6).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Global registry CRUD | `htui-core/src/store/traits.rs:908-985` (skills); `mem.rs:3078-3111`; `htui-store/src/pg/write.rs:2923-3011` (+ `update_skill` `:3013`) | Reads on `WriteStore` beside writers; CAS on `updated_at`; refusal sentences shared |
| Refusals | `traits.rs:1885-1937`, `:2112`, `:2118`; `pg/write.rs:658-666` | `StoreError::Constraint(sentence)` from a pure helper; Pg maps by constraint name; order token → `NotFound` → `Constraint` |
| Seeds | `htui-core/src/model/agent.rs:141-188`; `htui-store/src/pg/mod.rs:437-515`; `pg/demo.rs:30-35` | `include_str!`, private seed struct, `ON CONFLICT (name) DO NOTHING`; demo deletes seeded rows by name |
| Migration | `migrations/0009_agent_box_user_off.sql`, `0011_permission_relay.sql`, `0006_requirements.sql:135-142` | Header, forward-only, named constraints, explicit trigger, `COMMENT ON COLUMN` |
| Snapshot field outside the hash | `htui-core/src/model/run.rs:446-451` (`scope`) | `#[serde(default)]`, no `V` bump |
| Protected inlined text | `engine.rs:5515-5539` (`phase_skills`), `prompt/mod.rs:306-312` | Protected section, scrubbed |
| CLI argv | `htui-agent/src/cli/mod.rs:112-162`; `tests/cli_driver.rs:84-121` | Ordered pushes; exact-`Vec` and `windows(2)` asserts |
| Relay policy tests | `htui-agent/tests/relay.rs:637-683` | `drive(…, Some(&relay), …)`, assert `answers()` and `by: policy`; `call()` (`:209-222`) needs a kind parameter |
| Engine spec capture | `htui-orch/src/engine.rs:11886`, `:11928-11956` (`SpecSpy`) | Custom `driver` closure; extend the spy to keep the prompt |
| Store conformance | `htui-core/src/store/conformance.rs` (`CASES` `:48`, `run_case` `:179`) | Pins `htui-core/tests/mem_store.rs:36-37` and `htui-store/tests/pg_conformance.rs:23` (119) |
| Orch conformance | `htui-orch/src/conformance.rs` (`CASES` `:385`, `repoint` `:948-992`, `prompt_sections` `:2699`) | Pins `:6788` and `htui-orch/tests/fake_conformance.rs:15` (86) |
| Tests (Postgres) | `htui-store/src/testkit.rs:151-169` | `--features testkit`/`--all-features`, scratch DB per test; `apply_migrations` seeds, so `demo_db` holds seeded personas |

## Tasks

Order: **T0 → {T1 ∥ T2 ∥ T3} → T4 → T5**. Independence is decided by the file-set intersections
under "Verified claims", not by this prose. TDD throughout: each task's tests are written first and
fail for the stated reason. Each implementer commits incrementally.

### T0: Contracts (serial, first) — no SQL
- **Action**: `PersonaId`; `model/persona.rs` types (D2) with the D3 pure validators and the D8
  parser; the two seed `.md` files and `seed_rows`; `SnapshotPhase.persona`,
  `GraphSnapshot.personas`, `SnapshotPersona`; `PromptSpec.persona` + `PersonaBlock` (field only,
  `assemble` ignores it until T3); `ToolExposure.deny_kinds`; `judge_phase` and
  `promote::handoff_spec` set `persona: None`. Fix every non-spread literal these break.
  **Phase-binding fields (`StepGraphPhase.persona_id`, `PhasePatch.persona`,
  `ResolvedPhase.persona`) are T1's**: `query_as!` builds `StepGraphPhase` literals, so they
  cannot land without SQL and `.sqlx`.
- **Tests first**: parser — a `.claude/agents`-shaped file round-trips; `model` refused with its
  sentence; `color` refused by name; missing closing `---` refused; validators — every D3 refusal;
  serde — unknown key refused on every persona shape; snapshot — a persona-less phase serialises
  byte-identically (topology pin unchanged) and a bound one carries `"persona":"<name>"`;
  `judge_phase` and `handoff_spec` drop the persona.
- **Files**: `htui-core/src/model/ids.rs`, `model/mod.rs`, `model/persona.rs` (new), `model/run.rs`,
  `htui-core/seeds/persona_reviewer.md` (new), `seeds/persona_architect.md` (new),
  `htui-core/src/fixtures.rs` (`:1345`, `:1379`), `htui-core/src/store/conformance.rs` (`:4281`),
  `htui-core/src/store/mem.rs` (`:9308` only), `htui-core/src/prompt/mod.rs` (`PromptSpec` field +
  its own literals), `htui-core/src/prompt/fixtures.rs`, `htui-core/tests/prompt_digest.rs`,
  `htui-core/tests/prompt_hostname.rs`, `htui-core/tests/prompt_skills.rs` (non-spread
  `PromptSpec` literals only), `htui-agent/src/driver.rs`, `htui-agent/src/excerpt.rs`,
  `htui-agent/tests/excerpt.rs` (`PromptSpec` literals only), `htui-agent/tests/relay.rs`
  (`GraphSnapshot` `:81` only), `htui-store/tests/pg_criteria.rs` (`:1088`),
  `htui-orch/src/graph.rs` (`:364`, `:675` literals only), `htui-orch/src/engine.rs` (literals
  `:4734`, `:5457`, `:6844`, `:7643` only), `htui-orch/src/fanout.rs`, `htui-orch/src/promote.rs`,
  `htui/src/preview.rs`.
- **Validate**: `cargo test -p htui-core --all-features`; `SQLX_OFFLINE=true cargo check
  --workspace --all-targets --all-features`.

### T1: Store — binding fields, MemStore, Postgres, migration (parallel with T2 and T3)
- **Action**: D1, D3 (store side), D4, D5, D6, D7. `StepGraphPhase.persona_id`,
  `PhasePatch.persona`, `ResolvedPhase.persona` and their literals; `0012_persona.sql`; MemStore
  `State.personas`, the three methods, the `check_phase`/`update_phase` reference check after the
  clashes, `resolve_graph` fill; PgStore methods, the four `query_as!(StepGraphPhase…)` statements
  (`read.rs:2226`, `:2371`, `write.rs:2704`, `:2770`), the FK and unique mappers, `resolve_graph`
  persona query, seed insert, `demo.rs` load; `Writer`, `UsageSpy`, `SpyStore` forwarding;
  `.sqlx` regenerated against a migrated scratch database (`docs/hr-sandbox.md`); every pin.
- **Tests first** (store conformance, both stores): create/list/update CAS (stale, not found, name
  clash); every D3 refusal through the store; a phase bound to a missing persona refused on create
  and update with the same sentence on both stores; `Some(None)` clears the binding;
  `resolve_graph` returns the bound persona; Pg only — the seed adds both personas once and keeps
  an operator edit (`tests/migrations.rs:1522` pattern); a bound persona cannot be deleted (FK).
- **Pins**: `tests/migrations.rs` — `TABLES` (`:28`, `len == 41` at `:116`, messages `:119`,
  `:126`) gains `persona`; the version vector (`:93`) and `Pending(11)`/count 11 (`:922`, `:1021`,
  `:1026`, `:1045`, `:1195`) become 12; a new `MOD26_COLUMN_COMMENTS` list chained into both chains
  of the column-comment check (`:455-544`) with its count text; `load_demo_round_trips_a_count_per_table`
  (`:1840`) gains a `persona` row. `tests/connect.rs` `Pending(11)` (`:140`, `:242`). Store
  `CASES` 119 → N (`mem_store.rs:37`, `pg_conformance.rs:23`).
- **Files**: `htui-store/migrations/0012_persona.sql` (new), `htui-core/src/model/kind.rs`,
  `htui-core/src/seed.rs`, `htui-core/src/store/traits.rs`, `store/mem.rs`, `store/conformance.rs`,
  `htui-core/tests/mem_store.rs`, `htui-store/src/error.rs` (if the mapper lives there),
  `htui-store/src/pg/write.rs`, `pg/read.rs`, `pg/mod.rs`, `pg/demo.rs`, `htui-store/src/writer.rs`,
  `htui-store/.sqlx/*`, `htui-store/tests/pg_conformance.rs`, `htui-store/tests/migrations.rs`,
  `htui-store/tests/connect.rs`, `htui-agent/src/conformance.rs` (`UsageSpy` forwarding only),
  `htui-agent/tests/recorder.rs` (`SpyStore` forwarding only), `htui/src/ui/tabs/settings/kinds.rs`
  (`PhasePatch` literal `:976` only).
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-store --all-features
  -- --test-threads=1`; `cd crates/htui-store && cargo sqlx prepare --check -- --all-targets
  --all-features`; `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`.

### T2: Agent — `narrow` and transport enforcement (parallel with T1 and T3)
- **Action**: D10, D11. `persona.rs` with `narrow`; CLI `--tools=`/`--disallowedTools=` and the
  kind→claude-name inversion; ACP fs refusal with `deny_kinds` threaded from `session_main`; the
  `TOOL_KINDS` pin test.
- **Tests first**: `narrow` — every I-1 clause, including a base `remembered` allow beaten by a
  persona reject and `default` strictness; argv — exact order with both flags, `--allowedTools`
  never present, `--tools` absent when `allow` is empty, `deny_kinds: [edit]` yields the four
  claude names; relay — a parked `edit` request is answered `reject_once` by policy (kind-
  parameterised `call` helper); ACP — a denied write is refused before any `EditProposal` and the
  file is untouched, a denied read is refused, both emit the new error code.
- **Files**: `htui-agent/src/persona.rs` (new), `htui-agent/src/lib.rs`, `htui-agent/src/cli/mod.rs`,
  `htui-agent/src/cli/claude.rs`, `htui-agent/src/acp/mod.rs`, `htui-agent/tests/cli_driver.rs`,
  `htui-agent/tests/relay.rs`, `htui-agent/tests/acp_driver.rs`.
- **Validate**: `cargo test -p htui-agent --all-features`.

### T3: Prompt frame (parallel with T1 and T2)
- **Action**: D13's `htui-core` half: `SectionName::Persona`, internal `Placeholder::Persona`,
  render-before-template in `assemble`/`substitute`, protected, scrubbed, trim source.
- **Tests first**: a spec with a persona renders the block first in the text, `sections[0]` is
  `template` and `sections[1]` is `persona`; the block survives a budget that trims everything
  trimmable; a secret in the body is masked; `{{persona}}` in a template body is refused as an
  unknown placeholder; `Placeholder::ALL` still has 20; a persona-less spec renders byte-identically
  (goldens untouched); the digest changes when the body changes.
- **Files**: `htui-core/src/prompt/mod.rs` (assembly only; T0 already added the field),
  `htui-core/src/prompt/template.rs`, `htui-core/src/prompt/render.rs`,
  `htui-core/src/prompt/trim.rs` (`source_of` arm only), `htui-core/tests/prompt_persona.rs` (new).
- **Validate**: `cargo test -p htui-core --all-features`.

### T4: Engine wiring (serial, after T1, T2, T3)
- **Action**: D9's freeze in `snapshot_phase`/`graph::resolve` (`graph.rs:298-382`, `:636-721`);
  D12; D13's `phase_spec` half.
- **Tests first** (orch conformance through `FakeOrchestrator`; spec assertions as `engine.rs` unit
  tests with `SpecSpy` extended to keep the prompt): a persona-bound phase's step prompt carries
  the `persona` section; the spec carries the narrowed exposure and policy; the relay rejects a
  denied kind end to end (`ScriptedStep::parks` + `set_policy`); editing the persona row after
  `StartRun` does not change the next step (I-3); rebinding parks a resumed run; a snapshot naming
  an absent persona fails that step with the I-4 refusal and the walk continues to settle; the
  judge step carries no persona; `command_run: false` turns the command-queue section off; a
  persona-less run is unchanged; the agent row's `updated_at` is unchanged.
- **Files**: `htui-orch/src/graph.rs`, `htui-orch/src/engine.rs`, `htui-orch/src/conformance.rs`,
  `htui-orch/tests/fake_conformance.rs`, `htui-orch/src/fake.rs` (persona seeding helper, if
  needed).
- **Validate**: `cargo test -p htui-orch --all-features -- --no-fail-fast` (grep `SIGABRT`,
  auto-memory `htui-orch-test-stack-headroom`).

### T5: Docs and pins (serial, last)
- **Action**: `docs/personas.md` (format, keys, narrow-only rule, precedence "agent row → persona
  narrows; model from the phase candidate", the D11 matrix and residuals); README pointer; PRD
  milestone row; the plan's status.
- **Files**: `docs/personas.md` (new), `README.md`, `.claude/prds/mod-26-agent-personas.prd.md`,
  this plan.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Test plan

| PRD metric | Pinned by |
|---|---|
| Narrow-only holds (0 ways to widen) | T0 validators + T1 store refusals + T2 `narrow` clauses + T2 "`--allowedTools` never emitted" |
| Out-of-list tool refused | T2 relay, argv and ACP fs cases; T4 end-to-end relay case |
| Agent row untouched | T4: agent row `updated_at` unchanged across a persona run |
| Replay stable | T4 "edit after `StartRun`" case |
| Persona applied end to end (1 real run) | Measured at M2, once the TUI can bind a persona; M1 proves it through conformance |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| R-1 a future `claude` changes `--tools` semantics or tool names | Low | Argv test pins the form; denying an absent name is harmless (probed on 2.1.286) |
| R-2 a missed non-spread literal breaks T0's workspace check | Medium | T0's list is from the fact-check's exhaustive literal survey; T0 compiles the whole workspace before the fan-out |
| R-3 `.sqlx`, seeds, demo loader and migration pins couple tasks | High | T1 owns every SQL, `.sqlx`, migration and pin file; T0 touches no SQL (auto-memory `parallel-fanout-hidden-file-coupling`) |
| R-4 htui-orch stack headroom (`every_case_name_dispatches`, `conformance.rs:6829`) | Medium | Box any new large future (`drive` already boxed, `engine.rs:5819`); `--no-fail-fast`, grep `SIGABRT` |
| R-5 ACP agents run tools without asking; `claude-agent-acp` fs routing unverified | Certain / Unknown | D11 residuals documented; `deny_kinds` blocks gated calls and htui's fs handlers |
| R-6 the persona frame's `"\n\n"` separator is not counted in `template_tokens` | Low | Count it in the persona section's tokens, or accept a two-byte estimate drift (T3 decides, test pins) |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features -- --test-threads=1   # HTUI_TEST_DATABASE_URL set (sandbox)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Acceptance

- [ ] I-1…I-7 hold, each with a test naming it
- [ ] PRD metrics pinned per the test plan
- [ ] Validation passes; reviewer gate (`rust-reviewer`) findings applied or deferred with the
      maintainer
- [ ] Close-out restates moved counts from a fresh count: store `CASES` (119 → N), `READ_CASES`
      (14), htui-orch `CASES` (86 → N), `.sqlx`, snapshots, migrations (11 → 12, next `0013`),
      `TABLES` (41 → 42)
- [ ] ANA-27's precedence sentence corrected where it is quoted (HANDOFF MOD-26 entry, DECISIONS)

## Verified claims

Fact-check 2026-10-02, three verifiers (store/migrations; orchestrator/prompt with a serde probe;
agent/transports with an offline `claude` 2.1.286 probe against a dead local endpoint, no API
calls). Probes ran in scratch directories under `/tmp` (removed). Amendments are applied above.

| Claim | Verdict | Evidence / amendment |
|---|---|---|
| Skill block `traits.rs:902-985`, rationale `:907-913` | CONFIRMED (±6 lines) | `:908-985`, `:908-913` |
| Persona names refused "as `invalid_skill_name`" | PARTLY | Hardcodes "skill.name" (`traits.rs:1886`) → new `invalid_persona_name` (D3) |
| Skill is global, name-unique, CRUD + CAS, no delete | CONFIRMED | `0001_init.sql:406-413`; no `delete_skill` |
| Pg maps FK `23503` to `references_no_row` today | FALSIFIED | Every `23xxx` is raw (`error.rs:42-46`) → new constraint-name mapper (D5) |
| Refusal order across stores | AMENDED | Unique fires before FK → MemStore checks persona after clashes (D5) |
| `PhasePatch` + `COALESCE` cannot clear | CONFIRMED | `pg/write.rs:2794-2800` |
| Four Pg phase statements + `seed_project` + `seed::phase_row` are the complete set | PARTLY | `seed_project` needs no change; `pg/demo.rs:205`, `pg/read.rs:1750` added (T1) |
| T0 can add phase-binding fields without SQL | FALSIFIED | `query_as!` builds `StepGraphPhase` literals → fields moved to T1 |
| T0 literal list | PARTLY | Added `graph.rs:364/:675`, `mem.rs:9308`, `pg_criteria.rs:1088`, `relay.rs:81`, `PromptSpec` sites; dropped spreads (`graph.rs:507`, `engine.rs:6736`, `overlap.rs`, `recover.rs`, `htui/tests/kinds.rs`, `store_worker.rs`) |
| `ResolvedPhase` is `{phase, agents}`; `resolve_graph` on both stores | CONFIRMED | `kind.rs:312-317`; `mem.rs:5146`, `pg/read.rs:1726` |
| Agent seed precedent | CONFIRMED | `agent.rs:141-188`; `pg/mod.rs:480-501` |
| Demo store gains personas via fixtures only | PARTLY | `pg/demo.rs` must load them and delete seeded ones; `migrations.rs:1840` count (T1) |
| `migrations.rs` pins | CONFIRMED, all move | Listed under T1 "Pins"; plus `connect.rs:140, :242` |
| No YAML dependency | CONFIRMED | `toml` and `serde_json` only |
| `WriteStore` implementors | CONFIRMED | MemStore, Writer, UsageSpy, SpyStore, PgStore (`pg/write.rs:719`) |
| Topology over typed `serde_json`; skip-if-none is byte-identical | CONFIRMED (probe) | `graph.rs:251-282`; pin `:879-880` |
| `scope` outside the hash | CONFIRMED | `run.rs:446-451` |
| `judge_phase` spreads `..phase.clone()` | CONFIRMED | `fanout.rs:303` |
| `drive_once` anchors | PARTLY | `permission:` at `:5799`; no snapshot parameter → persona passed in from stage 3 (D12) |
| I-4 refusal in `drive_once` | FALSIFIED as placed | An `Err` aborts the walk → stage 3 via `refuse_prompt`/`fail_before_a_token` (D12) |
| `phase_spec` sets `command_queue` from the phase | CONFIRMED | `engine.rs:5488` |
| Handoff prompt untouched by personas | FALSIFIED | `phase_spec` feeds promotion (`:1367`, `promote.rs:81-106`) → `handoff_spec` sets `None` (OQ-5) |
| Persona as first section | FALSIFIED | `template` is always `sections[0]` (P-9, `trim.rs:1104-1122`, pinned `prompt_digest.rs:215-219`) → persona is `sections[1]` |
| Render before the template without a placeholder | FEASIBLE | Internal placeholder outside `ALL`; push before `mod.rs:486`; `substitute` prefix (D13) |
| `TrimRecord` v5 is a trim.rs-only change | FALSIFIED | Needs a migration restating the column comment + pins (`migrations.rs:429-432`, `prompt_digest.rs:988-1012`, `fixtures.rs:2513-2540`) → dropped (OQ-6) |
| `resume_window` parks on topology mismatch | CONFIRMED | `engine.rs:2965`, `:2996`, `:3017-3024` |
| `SpecSpy` captures the prompt | PARTLY | Drops it (`:11951`) → extend (T4) |
| Orch `CASES`, pins, `repoint`, `prompt_sections` | CONFIRMED | 86 at `:6788` and `fake_conformance.rs:15` |
| `htui-agent → htui-core` only | CONFIRMED | `htui-agent/Cargo.toml:18` |
| `ToolExposure` struct-level `serde(default)`, no literals | CONFIRMED | `driver.rs:206-213` |
| Permission shapes, `reject_once`/`reject_always` | CONFIRMED | `driver.rs:133-196`, `event.rs:115-124` |
| `evaluate` order | CONFIRMED | Rules `:57-79`, remembered `:81-92`, default `:94-101`; unoffered matched answer asks |
| Kind rule matches `ToolKind` spelling | CONFIRMED, caveat | Needs a prior `tool_call` of the same id (`relay.rs:277-321`) → D11 residual |
| CLI argv push block and order | PARTLY | The closure pushes pairs → `=` flags pushed after `:159`, before `:161` |
| Claude inversion table | CONFIRMED | Full inversion kept; 2.1.286 lacks some names, denying them is harmless |
| `--allowedTools=` narrows | **FALSIFIED** | Auto-approves (probe kept `Bash`) → `--tools=` restricts the built-in set; `--allowedTools` never emitted (I-1) |
| `=` form parses; comma list splits | CONFIRMED (probe) | `unknown option` control test; `--disallowedTools=Edit,Bash` left `['Read']` |
| Deny vs `acceptEdits` | CONFIRMED (probe) | Denied tools are removed from the init tool list |
| `claude` seed is the CLI transport | FALSIFIED | `claude` is ACP (`claude-agent-acp`); only `claude-cli` is CLI → D11 rows |
| ACP fs handlers can read the spec | PARTLY | `on_inbound` has no spec → thread from `session_main` (D11) |
| `SessionSpec` builders in `agent_worker.rs` | CONFIRMED | `:1037`, `:2069` |

**Task independence (file-set intersection).** T1 ∩ T2 = ∅, T1 ∩ T3 = ∅, T2 ∩ T3 = ∅ over the
lists above (same crates, disjoint files: T1's `htui-agent` files are `src/conformance.rs` and
`tests/recorder.rs`, T2's are `persona.rs`, `lib.rs`, `cli/*`, `acp/mod.rs` and three other tests;
T1's `htui-core` files are `model/kind.rs`, `seed.rs`, `store/*`, T3's are `prompt/*` and a new
test). T0 precedes all three and owns every shared literal. Parallel marking stands.
