# Plan: MOD-26 — Declarative agent personas, milestone 2 (authoring in the TUI)

**Status: DRAFTED 2026-10-02 — awaiting fact-check (handoff-run step 3.5) and the maintainer's
CONFIRM.**

**Source PRD**: `.claude/prds/mod-26-agent-personas.prd.md`, Delivery Milestones row 2 ("Settings >
Personas lists, edits, validates and saves personas; frontmatter `.md` import; a phase's persona is
picked in the step-graph editor") and the two items the milestone 1 review deferred to it, **N4**
(typed `PersonaNotInSnapshot`) and **N5** (index on `step_graph_phase.persona_id`, shipped with the
delete). PRD scope bullet 8 ("Authoring: a Settings > Personas tab (form fields + body editor),
plus a one-shot import of a frontmatter Markdown file (the `.claude/agents/*.md` shape) into a row.
The row is always the truth; the file is never re-read"). Milestone 1 plan:
`.claude/plans/mod-26-agent-personas.plan.md` (cited as **M1 D*n***, **M1 I-*n***).

**Requirements**: `R-ID-3` (row is the truth), `R-ID-6` (no LLM in an import path), `R-NF-3` (every
store read and write and the file read on the store worker), `R-STO-5` (forward-only migration),
`R-TUI-*` (Settings tab).

**Complexity**: Large. One new `WriteStore` method across five implementors, one index-only
migration (`0013`), one typed error replacing a `String` through the engine, one parser mode, four
new `StoreRequest`s with their worker module, a new Settings section with form, body and rule
editors, delete and import, and a persona field in the Settings › Kinds phase editor.

**Routing**: `/handoff-run MOD-26` (sandbox `hr/MOD-26`), plan path on the existing PRD (C4 only),
ultracode accepted for the implement phase. Reviewer: `rust-reviewer`
(`.claude/workflow-config.json`).

**Numbering** continues milestone 1: decisions **D14…**, invariants **I-8…** (M1's I-1…I-7 still
hold), risks **R-7…**, open questions **OQ-7…**, tasks **T6…** (so commit subjects never collide
with milestone 1's T0-T5).

**Tree reading**: HEAD `aeba8f63` (branch `hr/MOD-26`, sandbox; same commit as the host's `main`).
Three readers (Settings/Skills/`StoreRequest`; Kinds phase editor/`persona_for`; store
delete/migration/parser/import), Gortex for symbols, `grep`/`sed` where its text search returned
nothing. Paths are relative to `crates/` for code, the repo root for docs.

---

## Open questions for the maintainer (read these first)

Each has a recommended default; CONFIRM without comment takes all of them.

- **OQ-7 — `mcp__` entries in an imported `tools:` line.** Every real agent file on this machine
  (`.claude/agents/{code-architect,rust-reviewer}.md`, `~/.claude/agents/*.md`) carries only
  `name, description, tools`, and every one lists `mcp__gortex__*` names in `tools`. The M1 parser
  refuses them all: `allow_names_an_mcp_tool` (`htui-core/src/model/persona.rs`, rule M1 D3,
  "`--tools` filters built-in tools only"). As it stands the import accepts no real file.
  **Recommended:** the **import** (not the save rule, not the seed path) drops `mcp__` entries from
  `tools` and names them in the import report ("dropped from `tools`: `mcp__gortex__search`, …
  — `allow` keeps built-in tools only"). Dropping an `allow` entry for an MCP tool narrows nothing
  away: `--tools` never filtered MCP tools, so the file meant nothing htui can enforce through it.
  The save rule stays as it is. **Alternatives:** keep refusing (import works only on hand-written
  files); move them to `disallowed-tools` (inverts the author's intent — refused).
- **OQ-8 — Import onto an existing name.** A skill import updates a known name, but skills are
  versioned and a persona is not, so an update would lose the operator's edits. **Recommended:**
  a known name is **refused** for that file ("persona `reviewer` exists; edit it in Settings ›
  Personas, or delete it and import again"); new names are created. **Alternative:** overwrite
  under compare-and-set (the skill precedent).
- **OQ-9 — Import reach.** **Recommended:** the typed path names a `.md` file **or** a directory,
  whose depth-0 `*.md` files are each one persona (so `.claude/agents/` imports in one go); no
  recursion, caps as the skill import (`MAX_FILES`, `MAX_BYTES`, `htui/src/skill_import.rs:39-46`).
  A per-file report opens when anything was refused, skipped or dropped. **Alternative:** a single
  file only.
- **OQ-10 — Editing permission rules.** A rule is `{match: {tool_kind, tool_name, path_prefix,
  command_prefix}, answer, reason}`; the TUI has no list-of-structs widget. **Recommended:** a rules
  editor (a `TextArea`, `r` from Browse) holding **one rule per line in a small, round-tripping
  syntax** parsed and printed by pure functions in `htui-core` (D17):
  `reject_once kind=execute command="rm -rf" # never wipe`. Quoted values carry spaces; `#` starts
  the reason. **Alternative:** one JSON object per line (exact, no new grammar, harder to type).
- **OQ-11 — Picking a phase's persona in Settings › Kinds.** The phase editor is all one-line text
  fields and picks related rows by **typing their name** (`graph_named`,
  `htui/src/ui/tabs/settings/kinds.rs:1733`). **Recommended:** a seventh field, `persona`, typed
  like the others: blank = none, a name resolves against the loaded persona list, an unknown name
  is refused with the list of known names. On the edit form only; a new phase binds through its
  edit form after creation (`CreatePhase` stays as it is). **Alternative:** an inline `‹ ›` picker
  (`htui/src/ui/tabs/backlog/item_form.rs:600-652`, `:799-830`) — better discovery, but the
  Kinds `Field` becomes an enum and every field path in the editor changes.
- **OQ-12 — `$EDITOR` for the body.** `SettingsSection` has no `on_external_edit` and
  `SettingsTab` does not forward `Tab::on_external_edit` (`htui/src/ui/tabs/registry.rs:77`; the
  Skills tab's override at `skills/mod.rs:132`). MOD-13 milestone 4 (`$EDITOR` round-trip) is being
  planned in another run. **Recommended:** milestone 2 ships the embedded `TextArea` only (the
  Boxes section's precedent inside Settings, `settings/boxes.rs:161-190`); `$EDITOR` comes with or
  after MOD-13 milestone 4.

## Summary

A new Settings section, **Personas**, appended after the existing seven, lists the persona rows and
opens a form (one-line fields), a body editor and a rules editor; saves go through new
`StoreRequest`s served by a `persona_settings` worker module under compare-and-set, as Settings ›
Agents does. `d` deletes after a `y/n` question; the store refuses a persona still bound to a phase
with a sentence naming the phases, and migration `0013` indexes `step_graph_phase.persona_id` so that
check (and the `ON DELETE RESTRICT` FK) stops scanning phases. `I` imports a frontmatter file or a
directory of them through the M1 parser, with the import's own `mcp__` handling. Settings › Kinds
gains a `persona` field on the phase edit form. `GraphSnapshot::persona_for` returns a typed
`PersonaNotInSnapshot` that the engine's I-4 arms carry as a type.

## Invariants (M1 I-1…I-7 still hold; these are added)

- **I-8 One rule set.** Every refusal the form shows is the store's own sentence (or a pure
  `htui-core` helper the store also calls). The form never accepts what the store would refuse,
  and never refuses what the store would accept, except for the closed rule-kind list (D18), which
  both sides apply.
- **I-9 Delete never strands a binding.** A persona bound to any phase cannot be deleted, on either
  store, and the refusal names what holds it. A started run never needs the row (M1 I-3: it reads
  its snapshot), so deleting an unbound persona never breaks a run.
- **I-10 Import writes rows, never reads them back from files** (`R-ID-3`): a persona is read from
  its file once, at import; no path is stored.
- **I-11 Off the UI thread** (`R-NF-3`): the persona list, every write and the import's file reads
  run on the store worker.
- **I-12 Persona-less screens unchanged.** No existing snapshot changes except the Kinds phase
  editor's (the new field) and the strip pin's prose; a phase line shows `· persona <name>` only
  when the phase has one.

## Design decisions

### Store

- **D14 — `WriteStore::delete_persona(id: PersonaId) -> Result<()>`.** Beside the persona block
  (`htui-core/src/store/traits.rs:1003-1038`, whose comment says M2 adds it). Order: `NotFound` →
  bound → delete. No compare-and-set token, as `delete_item_kind` (`traits.rs:821-828`), the only
  "refused while referenced" delete. Bound → `StoreError::Constraint(persona_is_bound(name,
  &phases))`, a new pure helper beside `item_kind_is_held` (`traits.rs:2160-2167`):
  "persona \`reviewer\` is bound to 2 phases (`default/review`, `hotfix/review`); clear them in
  Settings › Kinds first". Phases are named `<graph>/<phase>`, sorted, at most five then "and *n*
  more". MemStore: `State::delete_persona` mirrors `State::delete_item_kind`
  (`htui-core/src/store/mem.rs:2771-2785`). PgStore mirrors `delete_item_kind`
  (`htui-store/src/pg/write.rs:2615-2663`): one transaction, `SELECT name … FOR UPDATE`, `DELETE …
  WHERE id = $1 AND NOT EXISTS (SELECT 1 FROM step_graph_phase WHERE persona_id = $1)`, and on zero
  rows a query for the holding phases' names, so a racing bind still gets the sentence, not the raw
  `23503`. Implementors (five, no default methods): MemStore (`mem.rs:6494`), PgStore
  (`pg/write.rs:777`), `Writer` (`htui-store/src/writer.rs:313`), `UsageSpy`
  (`htui-agent/src/conformance.rs:743`), `SpyStore` (`htui-agent/tests/recorder.rs:431`).
- **D15 — Migration `0013_persona_phase_index.sql` (N5).** `CREATE INDEX
  idx_step_graph_phase_persona ON step_graph_phase(persona_id); -- FK check on persona delete`,
  the `0006_requirements.sql:88` shape. Header per `0012`/`0010` (index only: no table, column or
  constraint moves, the mirror's schema is untouched, `schema_version` still becomes 13 so each box
  rebuilds its mirror once). No cache migration (`0010`, `0012` precedent), no column comment.
  Pins move 12 → 13 (`htui-store/tests/migrations.rs`, `tests/connect.rs`); `TABLES` stays 42.
- **D16 — N4: typed `PersonaNotInSnapshot`.** In `htui-core/src/model/persona.rs`, a
  `#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)] pub struct PersonaNotInSnapshot { pub
  persona: String }` whose `#[error]` is today's sentence, byte-identical (the `PersonaFileError`
  precedent in the same module). `GraphSnapshot::persona_for`
  (`htui-core/src/model/run.rs:469`) returns `Result<Option<&SnapshotPersona>,
  PersonaNotInSnapshot>`. `persona_not_in_snapshot` is removed and its re-exports
  (`store/traits.rs:1950`, `store/mod.rs:28`) export the type instead. Engine:
  `StageThree::NoPersona(PersonaNotInSnapshot)` (`htui-orch/src/engine.rs:6278-6286`);
  `.to_string()` happens only where a `String` is stored — `refuse_prompt`'s
  `RunFailure::PromptRefused { reason }` (`:3475`, `:3857`), `EngineError::Snapshot { reason }`
  (`:3481-3486`, `:3870-3875`) and the handoff wrap (`:1387-1392`). Persisted reasons are therefore
  byte-identical; tests match `NoPersona(e) if e.persona == "reviewer"`.

### Parser and import

- **D17 — Rule lines.** `persona::parse_rules(&str) -> Result<Vec<PersonaRule>, RuleLineError>` and
  `persona::format_rules(&[PersonaRule]) -> String`, pure, round-tripping
  (`parse_rules(format_rules(r)) == r` property-style test over the conformance fixtures). Grammar
  per line: `<answer> <key>=<value>… [# <reason>]`; `answer` ∈ {`reject_once`, `reject_always`};
  keys `kind`, `name`, `path`, `command` (each once); a value is a bare word or a `"`-quoted string
  with `\"` and `\\` escapes; blank lines and lines starting with `#` are ignored. Errors name the
  line number. The result then goes through the store's `persona_refusal` (I-8).
- **D18 — Closed rule-kind list.** A rule's `kind` must be one of `TOOL_KINDS` (all ten, unlike
  `deny_kinds`' seven), checked by `parse_rules` **and** added to `permission_refusal`
  (`model/persona.rs:423`), so both stores refuse a misspelt kind ("persona.permission.rules entry
  kind \`exec\` is not one of read, edit, …"). This closes the gap `docs/personas.md` names ("a
  misspelt kind matches nothing; milestone 2's rule form is meant to offer the closed list"). Seeds
  carry no rules, so no stored row changes meaning.
- **D19 — Import mode of the parser (OQ-7).** `persona::parse_import(text) -> Result<Imported,
  PersonaFileError>` where `Imported { file: PersonaFile, dropped: Vec<String> }`: it runs
  `parse_file`'s key handling but moves `mcp__` entries of `tools` into `dropped` before the
  refusals run. `parse_file` (the seed path) is unchanged and still refuses them.
- **D20 — Import on the store worker (OQ-8, OQ-9).** New `htui/src/persona_import.rs`, the
  `skill_import.rs` shape: `import(backend, path) -> PersonaImports { personas, report }`, writer
  required (offline → `Unreachable(DATABASE_UNREACHABLE)`), one `personas()` read for the names,
  `create_persona(file.into_new(PersonaId::new()))` per new name, a known name refused (OQ-8), a
  name met twice in one batch refused naming the first file, a directory read at depth 0 for
  `*.md`, the same caps. Report rows: `Imported{name, path, dropped}`, `Refused{path, message}`,
  `Skipped{path, reason}`. Writer errors through `skill_import::sentence`'s pattern.

### TUI

- **D21 — Requests and replies** (in `htui/src/store_worker.rs`): `StoreRequest::{Personas,
  CreatePersona { new: NewPersona }, UpdatePersona { id, expected, patch: PersonaPatch },
  DeletePersona { id }, ImportPersonas { path: String }}`; `StoreReply::{Personas(Vec<Persona>),
  PersonaWritten { personas: Vec<Persona>, outcome: PersonaWrite }, PersonaImports(Box<…>)}` with
  `PersonaWrite::{Created(PersonaId), Updated, Deleted, Stale(Persona), Gone}`. `name()` arms
  (string literals, `const fn`), `try_serve` routing to `persona_settings::serve`, and
  `persona_settings::REQUEST_NAMES` in request order with the `request_names_match_the_name_arms`
  test (`htui/src/skills.rs:1090-1100`). The settings reply mirrors `AgentWritten`
  (`agent_settings.rs:517`): every write answers with the re-read list.
- **D22 — `PersonasSection`** (`htui/src/ui/tabs/settings/personas.rs`), the `agents.rs` shape
  (`agents.rs:349`, `:404-476`): `Mode::{Browse, Editing(Editor), Body(BodyEditor), Rules(RulesEditor),
  Deleting{..}, ImportPath{field}, Report{..}}`, `busy`, `notice`. Browse shows one line per
  persona: `name · description · deny <kinds> · allow <n> · rules <n>`.
  - `n` new / `e` edit: fields `name`, `description`, `tools` (comma list), `disallowed-tools`
    (comma list), `deny-kinds` (comma list), `command-run (y/n)`, `permission-default` (blank,
    `ask`, `deny`) — the frontmatter key spellings, so the docs describe one vocabulary. A new
    persona's body is required (`BLANK_PERSONA_BODY`), so `n` opens the fields and Enter moves on
    to the body editor; Ctrl+S in the body creates the row.
  - `b` edits the body, `r` the rules (D17), each a `TextArea` (`htui/src/ui/text_area.rs:53`) with
    Ctrl+S to save and Esc warning once on unsaved text (`skills/library.rs:1028`).
  - Every save is one `UpdatePersona` with only the changed fields set (`PersonaPatch` is
    all-`Option`); `Stale` rebases untouched fields and says `CHANGED_ELSEWHERE`
    (`agents.rs:1916`), `Gone` says `DELETED_ELSEWHERE`, a `Failed` keeps the editor open with the
    store's sentence.
  - `d` asks `y/n`, naming the refusal up front ("a persona bound to a phase is refused"), the
    `kinds.rs` `Deleting` shape (`:250-313`, `:657`, `:682`, `:1809`).
  - `I` opens a typed path (`skills/library.rs:249`, `:710`, `:861`) and sends `ImportPersonas`;
    the report opens when anything was refused, skipped or had entries dropped.
  - Offline (`backend.writer()` is `None`): the section says personas need the database and offers
    no keys but navigation.
  Registered last in `app/mod.rs:62-73` (`register_all`; "appending moves no existing section's
  line").
- **D23 — Kinds `persona` field (OQ-11).** `CatalogueSnapshot` (`htui/src/catalogue.rs:35`) gains
  `personas: Vec<PersonaSummary { id, name }>`, filled by `snapshot()` (`:83`) from `personas()` when
  a writer exists, empty otherwise. `edit_phase_fields` (`kinds.rs:1682`) gains `persona` (prefilled
  with the bound name, blank when none); `build_phase_edit` (`:948-1005`) sets `PhasePatch.persona`
  only when the field changed: blank → `Some(None)`, a known name → `Some(Some(id))`, an unknown name
  refused before sending with "no persona named \`x\`; known: architect, reviewer" (and "no personas
  exist; add one in Settings › Personas" when the list is empty). An id the list does not know (a
  persona created after the snapshot) prints as the id and is left untouched unless edited.
  `phase_line` (`:1842`) appends `· persona <name>` only when bound (I-12). Settings › Personas'
  writes do not refresh Kinds' snapshot; Kinds re-reads on its own scope change, as today.

### Scope guards

- No `$EDITOR` hand-off (OQ-12), no `‹ ›` picker in Kinds (OQ-11), no persona on `CreatePhase`.
- No export to a file, no per-candidate binding, no chat or promoted application (PRD out of scope).
- No change to `narrow`, the prompt frame, the snapshot shape or the transports (milestone 1).
- No change to the M1 save rules except D18's closed kind list.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Delete refused while referenced | `traits.rs:821-828`, `:2160-2167`; `mem.rs:2771-2785`; `pg/write.rs:2615-2663`; conformance `item_kind_delete_refused_while_referenced` (`htui-core/src/store/conformance.rs:3687`); race `htui-store/tests/pg_criteria.rs:3317` | No CAS; `Constraint(sentence naming the holders)`; Pg `DELETE … NOT EXISTS` then count, not FK mapping |
| Index migration | `migrations/0006_requirements.sql:88`, `0010_*.sql:1-8` (index-free header), `0012_persona.sql:1-12` | `idx_<table>_<col>` with trailing reason comment; header names what moves and what does not |
| Typed error | `PersonaFileError` (`model/persona.rs:456`), `AnswerRefusal` (`model/relay.rs:149`) | `thiserror`, `Display` is the sentence |
| Registry CRUD section | `htui/src/ui/tabs/settings/agents.rs` (`:349`, `:404-476`, `:1495`, `:1589`, `:1625`, `:1916`); `htui/src/agent_settings.rs` (`:483`, `:517`, `:579`, `:640-654`) | Form of `TextField`s, `submit` → `request()`, re-run rules on the worker, CAS rebase |
| Delete y/n | `settings/kinds.rs:250-313`, `:657`, `:682`, `:1493-1503`, `:1809` | `Mode::Deleting{stage}`, refusal as `Notice::Error` |
| Body editor in Settings | `settings/boxes.rs:161-190` (`Mode::Quirks(Editor<TextArea,…>)`) | Embedded `TextArea`, Ctrl+S |
| Import | `htui/src/skill_import.rs` (`:39-46`, `:66-117`, `:129-154`, `:266-345`, `:363`); `skills.rs:402-413`; `skills/library.rs:249`, `:571`, `:710`, `:861`, `:1246` | Typed path → request → worker walk → per-file report |
| Requests | `htui/src/store_worker.rs` (`:124`, `:928-1051` `name()`, `:1055`, `:1571` `try_serve`, `:1660-1665`), `skills.rs:1090-1100` | Module-owned `REQUEST_NAMES` pinned against `name()` |
| Phase field by name | `kinds.rs:1682`, `:948-1005`, `:1733`, `:1842` | Typed name, resolver refusal |
| Tests (TUI) | `htui/src/testkit.rs:116-580` (`Harness`), `:625-725` (`SectionBench`); `tests/kinds.rs:1554`, `:1309`; insta 100x30 in `crates/htui/tests/snapshots/` | `#![cfg(feature = "testkit")]`, `<file>__<name>.snap` |
| Tests (store) | conformance `CASES` (`htui-core/src/store/conformance.rs:52`, persona section `:15265`), pins `htui-core/tests/mem_store.rs:36`, `htui-store/tests/pg_conformance.rs:27` (130) | Name in `CASES`, `run_case` arm, generic `async fn` |

## Tasks

Order: **T6 → {T7 ∥ T8} → T9 → T10.** Independence is decided by the file-set intersections under
"Verified claims", not by this prose. TDD throughout: each task's tests are written first and fail
for the stated reason. Each implementer commits incrementally (`feat(mod-26): T<n> …`).

### T6: Core — N4, rule lines, closed kinds, import parse (serial, first)
- **Action**: D16 (type, `persona_for`, re-exports, engine arms and their tests), D17, D18 (parser
  side and `permission_refusal`), D19.
- **Tests first**: `PersonaNotInSnapshot`'s `Display` equals the old sentence byte-for-byte; the
  engine I-4 tests match the type and their persisted `PromptRefused` reason is unchanged;
  `parse_rules`/`format_rules` round-trip, quoting, escapes, comments, each error with its line;
  `kind=exec` refused by `parse_rules` and by `permission_refusal`; `parse_import` drops `mcp__`
  entries from `tools` and reports them, accepts both real `.claude/agents` files' frontmatter
  verbatim (fixtures copied into the test), and `parse_file` still refuses them.
- **Files**: `htui-core/src/model/persona.rs`, `htui-core/src/model/run.rs`,
  `htui-core/src/store/traits.rs` (re-export line only), `htui-core/src/store/mod.rs` (re-export),
  `htui-orch/src/engine.rs`, plus any other `persona_not_in_snapshot` user the fact-check finds.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-orch --all-features --
  --test-threads=1 --no-fail-fast` (grep `SIGABRT`); `SQLX_OFFLINE=true cargo check --workspace
  --all-targets --all-features`.

### T7: Store — `delete_persona` and `0013` (parallel with T8)
- **Action**: D14, D15; `persona_is_bound` helper; MemStore, PgStore, `Writer`, `UsageSpy`,
  `SpyStore`; `.sqlx` regenerated against a migrated scratch database (`docs/hr-sandbox.md`);
  every migration pin; `CASES` pins.
- **Tests first** (store conformance, both stores): delete an unbound persona (gone from
  `personas()`); delete an unknown id → `NotFound`; delete a bound persona → the sentence naming
  `<graph>/<phase>` sorted, and the row survives; more than five phases → "and *n* more"; after the
  phase is cleared (`Some(None)`) the delete succeeds. Pg only: a bind racing the delete still gets
  the sentence (the `pg_criteria.rs:3317` shape); the index exists (`pg_indexes`).
- **Files**: `htui-core/src/store/traits.rs`, `htui-core/src/store/mem.rs`,
  `htui-core/src/store/conformance.rs`, `htui-core/tests/mem_store.rs`,
  `htui-store/src/pg/write.rs`, `htui-store/src/writer.rs`,
  `htui-store/migrations/0013_persona_phase_index.sql` (new), `htui-store/tests/migrations.rs`,
  `htui-store/tests/connect.rs`, `htui-store/tests/pg_conformance.rs`,
  `htui-store/tests/pg_criteria.rs`, `htui-store/.sqlx/*`, `htui-agent/src/conformance.rs`,
  `htui-agent/tests/recorder.rs`.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-store --all-features
  -- --test-threads=1`; `cargo sqlx prepare --check -- --all-targets --all-features` (in
  `crates/htui-store`).

### T8: Kinds `persona` field (parallel with T7)
- **Action**: D23.
- **Tests first** (`htui/tests/kinds.rs`): `e` on a phase opens seven fields with `persona` last,
  prefilled; a typed known name sends `UpdatePhase` with `persona: Some(Some(id))`; blanking a bound
  one sends `Some(None)`; an untouched field sends `None`; an unknown name is refused before
  sending, listing the known names; a bound phase's line shows `· persona reviewer` and an unbound
  one's is unchanged; the worker's `snapshot` carries the personas online and none offline.
  Snapshots: `kinds__editor_phase` updated (seven fields), one new `kinds__phase_persona`.
- **Files**: `htui/src/ui/tabs/settings/kinds.rs`, `htui/src/catalogue.rs`, `htui/tests/kinds.rs`,
  `htui/tests/snapshots/kinds__editor_phase.snap`, `kinds__phase_persona.snap` (new), plus any
  `CatalogueSnapshot` literal the fact-check finds.
- **Validate**: `cargo test -p htui --all-features --test kinds`; `cargo insta test -p htui
  --all-features --test kinds` (no pending snapshots).

### T9: Settings › Personas and the import (serial, after T6 and T7)
- **Action**: D20, D21, D22.
- **Tests first**: worker (`serve` over a memory backend): list, create, update CAS (applied,
  stale, gone), delete (applied, bound refusal sentence), offline refusals, import (file, directory,
  dropped `mcp__`, known name refused, twice-in-batch refused, caps, non-`.md` skipped);
  `REQUEST_NAMES` against `name()`; section (`SectionBench`): browse list, `n` → fields → body →
  Ctrl+S sends `CreatePersona`, `e` sends only changed fields, `b`/`r` editors, a form refusal
  focuses the field, stale rebase, `d` y/n and the refusal notice, `I` path → report; the strip pin
  (`htui/tests/settings.rs` `the_section_strip_fits_the_frame`) with eight sections.
  Snapshots: `personas__browse`, `personas__form`, `personas__body`, `personas__rules`,
  `personas__delete_ask`, `personas__import_report`, `personas__offline`.
- **Files**: `htui/src/persona_settings.rs` (new), `htui/src/persona_import.rs` (new),
  `htui/src/lib.rs`, `htui/src/store_worker.rs`, `htui/src/ui/tabs/settings/personas.rs` (new),
  `htui/src/ui/tabs/settings/mod.rs`, `htui/src/app/mod.rs`, `htui/tests/personas.rs` (new),
  `htui/tests/settings.rs`, `htui/tests/snapshots/personas__*.snap` (new), and
  `htui/tests/connection.rs` only if its section-walk tests move (fact-check).
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui
  --all-features` (no pending snapshots).

### T10: Docs and pins (serial, last)
- **Action**: `docs/personas.md` (replace "Binding a persona to a phase in milestone 1" with the
  Kinds field, add the Personas section, the import and its `mcp__` rule, rule-line syntax, delete,
  D18's closed list; shrink "What milestone 1 does not do yet" to the remaining out-of-scope list);
  `README.md` (Settings summary and the sections table); `HANDOFF.md` pins re-counted, not
  incremented (`StoreRequest`/`StoreReply` are already stale at 96/55 against 99/58 in the tree;
  eight Settings sections and their strip columns; snapshots; `CASES`; migrations 13, next `0014`);
  PRD milestone row; this plan's status.
- **Files**: `docs/personas.md`, `README.md`, `HANDOFF.md`, `.claude/prds/mod-26-agent-personas.prd.md`,
  this plan.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Test plan

| PRD outcome | Pinned by |
|---|---|
| Settings › Personas lists, edits, validates, saves | T9 section and worker cases; I-8 by the form refusing with the store's sentences |
| Frontmatter `.md` import | T6 `parse_import` on real `.claude/agents` frontmatter; T9 import worker cases |
| A phase's persona is picked in the step-graph editor | T8 |
| Delete refused while bound (I-9), N5 index | T7 conformance (both stores), Pg race and `pg_indexes` |
| N4 | T6 engine tests matching the type; persisted reasons unchanged |
| Persona applied end to end (1 real run, PRD metric deferred from M1) | Manual, on the host after collect: bind `reviewer` in Kinds, run one step, read the step's prompt `sections[]` and its argv/relay record |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| R-7 another sandbox run also takes migration `0013` | Low | Host tree checked at plan time (latest `0012`); renumber at merge if a sibling lands first — the file is index-only, so a rename plus the pins is the whole conflict |
| R-8 `store_worker.rs` is a hot file (MOD-13, MOD-69, MOD-72 add requests) | High | T9 owns it alone; merge conflicts are additive enum arms; pins re-counted at T10, never incremented |
| R-9 `.sqlx`, migration pins and conformance pins couple T7 with anything else touching SQL | High | T7 owns every SQL, `.sqlx` and store pin file; T6 and T8 touch none (auto-memory `parallel-fanout-hidden-file-coupling`) |
| R-10 `TextArea` inside a Settings section and the strip's key capture (`captures_input`) | Medium | Boxes' `Quirks` precedent; a section test that `h`/`l` type into the body instead of cycling |
| R-11 htui-orch stack headroom when `StageThree` changes | Low | No new future; `--no-fail-fast`, grep `SIGABRT` (auto-memory `htui-orch-test-stack-headroom`) |
| R-12 the rule-line grammar fails on a real matcher value | Medium | Quoted values with escapes; round-trip test over every conformance rule fixture |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # HTUI_TEST_DATABASE_URL set (sandbox)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Acceptance

- [ ] I-8…I-12 hold, each with a test naming it; M1's I-1…I-7 tests still green
- [ ] PRD milestone 2 outcome pinned per the test plan; N4 and N5 closed
- [ ] Validation passes; reviewer gate (`rust-reviewer`) findings applied or deferred with the
      maintainer
- [ ] Close-out restates moved counts from a fresh count: store `CASES` (130 → N), `READ_CASES`
      (14), migrations (12 → 13, next `0014`), `.sqlx` (318 → N), snapshots, Settings sections
      (7 → 8), `StoreRequest`/`StoreReply` (re-counted), `persona_settings::REQUEST_NAMES`

## Verified claims

*(filled by the step 3.5 fact-check)*
