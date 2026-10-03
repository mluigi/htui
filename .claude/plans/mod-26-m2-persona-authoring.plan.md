# Plan: MOD-26 — Declarative agent personas, milestone 2 (authoring in the TUI)

**Status: IMPLEMENTED 2026-10-03 (milestone 2, T6-T10, `cd3de2ff`..`3b7b6297`; MOD-26 closed,
`docs/decisions/mod/mod-26.md`). Review: `rust-reviewer` approve with fixes, 0 critical/high; R1
fixed M-1, L-1-L-4 and N-2, plus ADV-1 from the R1 verifier; N-1 not done (maintainer). CONFIRMED
by the maintainer 2026-10-02, OQ-7 to OQ-12 as recommended. DRAFTED and FACT-CHECKED 2026-10-02.** Three
verifiers (core/engine with a scratch parser probe over the six real agent files; store/migrations
with a migrated scratch database and two-session lock probes; TUI/worker) checked every claim;
falsified and partly-true claims are amended in place — see "Verified claims".

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
  The save rule stays as it is. **One exception (fact-check):** a file whose `tools` was non-empty
  and holds **only** `mcp__` entries (`~/.claude/agents/gortex-{impact,search}.md`) would import
  with an empty `allow`, which keeps every built-in tool — a widening of the author's intent. That
  file is **refused**: "every `tools` entry is an MCP tool; htui's `allow` keeps built-in tools
  only, so this file would keep all of them — write `disallowed-tools` or `deny-kinds` instead".
  **Alternatives:** keep refusing (import works only on hand-written files); move them to
  `disallowed-tools` (inverts the author's intent — refused).
- **OQ-8 — Import onto an existing name.** A skill import updates a known name, but skills are
  versioned and a persona is not, so an update would lose the operator's edits. **Recommended:**
  a known name is **refused** for that file ("persona `reviewer` exists; edit it in Settings ›
  Personas, or delete it and import again"); new names are created. **Alternative:** overwrite
  under compare-and-set (the skill precedent).
- **OQ-9 — Import reach.** **Recommended:** the typed path names a `.md` file **or** a directory,
  whose depth-0 `*.md` files are each one persona (so `.claude/agents/` imports in one go); no
  recursion, caps as the skill import (`MAX_FILES`, `MAX_BYTES`, `htui/src/skill_import.rs:38-45`).
  This sweep is **new logic**, not a mirror: the skill import deliberately refuses "every `*.md` of
  whatever directory" (`skill_import.rs:52-57`, R-37, a README becoming a skill). Here a `.md`
  that does not open with a `---` fence (a `README.md`) is **skipped** ("no frontmatter"), not
  refused. A per-file report opens when anything was refused, skipped or dropped.
  **Alternative:** a single file only.
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
  "persona \`reviewer\` is bound to 2 phases (`web/default/review`, `web/FEAT-12-override/review`);
  clear them in Settings › Kinds first". A persona is global and graph names are unique only per
  project (`0001_init.sql:221`), so phases are named `<project-slug>/<graph>/<phase>`, sorted and
  deduped on that triple, at most five then "and *n* more". Override graphs copy `persona_id`
  (`htui-orch/src/graph.rs:540-544`), so they appear in the list. Re-exported beside
  `item_kind_is_held` from `store/mod.rs:25` (imported by `mem.rs:60`, `pg/write.rs:55`).
  MemStore: `State::delete_persona` mirrors `State::delete_item_kind`
  (`htui-core/src/store/mem.rs:2771-2785`), joining `phases` → `graphs` → `projects`. PgStore
  mirrors `delete_item_kind` (`htui-store/src/pg/write.rs:2615-2663`): one transaction, `SELECT
  name … FOR UPDATE`, `DELETE … WHERE id = $1 AND NOT EXISTS (SELECT 1 FROM step_graph_phase WHERE
  persona_id = $1)`, and on zero rows a query for the holders (`JOIN step_graph g ON g.id =
  p.graph_id JOIN project pr ON pr.id = g.project_id`). The lock is required (probed): a bind
  that commits first makes the delete return 0 rows and the sentence; a bind that waits behind the
  lock gets `23503`, which `update_phase`/`create_phase` already turn into `references_no_row`
  (`pg/write.rs:2817`, `:2871-2873`); without `FOR UPDATE` the deleter would see a raw `23503`.
  The "M2 adds the delete" comment (`traits.rs:1005-1009`) is rewritten; `0012_persona.sql`'s
  header is **not** edited (an applied migration's checksum). Implementors (five, no default methods): MemStore (`mem.rs:6494`), PgStore
  (`pg/write.rs:777`), `Writer` (`htui-store/src/writer.rs:313`), `UsageSpy`
  (`htui-agent/src/conformance.rs:743`), `SpyStore` (`htui-agent/tests/recorder.rs:431`).
- **D15 — Migration `0013_persona_phase_index.sql` (N5).** `CREATE INDEX
  idx_step_graph_phase_persona ON step_graph_phase(persona_id); -- FK check on persona delete`,
  the `0006_requirements.sql:88` shape. Header per `0012`/`0010` (index only: no table, column or
  constraint moves, the mirror's schema is untouched, `schema_version` still becomes 13 so each box
  rebuilds its mirror once). No cache migration (`0010`, `0012` precedent), no column comment; no
  index on `persona_id` exists today (probed on a migrated scratch database). Pins move 12 → 13:
  `htui-store/tests/migrations.rs` `:96` (`1..=12`), `:989`, `:1088`, `:1093`, `:1112`, `:1262`
  and the "twelve"/`0012_persona.sql` prose (`:101-102`, `:122-124`, `:130-132`, …);
  `tests/connect.rs` `:140`, `:156`, `:242` and prose. `TABLES` stays 42; the column-comment test
  (`migrations.rs:450`) is unaffected. A new `pg_indexes` assertion goes in `migrations.rs` (no
  precedent).
- **D16 — N4: typed `PersonaNotInSnapshot`.** In `htui-core/src/model/persona.rs`, a
  `#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)] pub struct PersonaNotInSnapshot { pub
  persona: String }` whose `#[error]` is today's sentence, byte-identical, **including the
  `escape_debug` of the name** (`#[error("persona `{}` is not in …", .persona.escape_debug())]`,
  thiserror 2; the `PersonaFileError` precedent in the same module). `GraphSnapshot::persona_for`
  (`htui-core/src/model/run.rs:469`) returns `Result<Option<&SnapshotPersona>,
  PersonaNotInSnapshot>`; its intra-doc link (`:464`) is repointed (`broken_intra_doc_links` is
  denied). `persona_not_in_snapshot` is removed and its re-exports (`store/traits.rs:1950`,
  `store/mod.rs:28`) export the type instead; `model/mod.rs` exports it beside `PersonaFileError`.
  Engine: `StageThree::NoPersona(PersonaNotInSnapshot)` (`htui-orch/src/engine.rs:6278-6286`);
  `.to_string()` happens only where a `String` is stored — `refuse_prompt`'s `reason`
  (`:3475` → `:5305`), the inline `RunFailure::PromptRefused` handed to
  `fail_group_before_a_token` (`:3857`), `EngineError::Snapshot { reason }` (`:3481-3486`,
  `:3870-3875`, `command.rs:477-482`) and the handoff wrap (`:1387-1392`). Persisted reasons
  (`htui-orch/src/status.rs:96-100`) are therefore byte-identical; tests match `NoPersona(e) if
  e.persona == "reviewer"`, and `the_snapshot_refusal_escapes_the_persona_name`
  (`model/persona.rs:801`) is ported to the type.

### Parser and import

- **D17 — Rule lines.** `persona::parse_rules(&str) -> Result<Vec<PersonaRule>, RuleLineError>` and
  `persona::format_rules(&[PersonaRule]) -> String`, pure, round-tripping. Grammar per line:
  `<answer> <key>=<value>… [# <reason>]`; `answer` ∈ {`reject_once`, `reject_always`}; keys `kind`,
  `name`, `path`, `command` (each once; an absent key is `None`, `key=""` is `Some("")`); a value
  is a bare word or a `"`-quoted string. Quoted strings escape `\"`, `\\`, `\n`, `\t` and `\r`
  (the stores accept newlines and tabs in matcher values and reasons, which must still print on
  one line). The reason runs from the first unquoted `#` to the end of the line, trimmed; a reason
  that needs leading or trailing spaces or a control character is written `#"…"` (quoted, same
  escapes). `format_rules` emits the shortest form that parses back. Blank lines and lines
  starting with `#` are ignored. Errors name the line number. The result then goes through the
  store's `persona_refusal` (I-8). The round-trip test is a **fixed table plus generated cases**
  (spaces, `"`, `\`, `#`, `=`, newline, tab, Unicode, empty values) — the repo holds no valid rule
  fixture with such values (fact-check), and the empty match and NUL are excluded (refused).
- **D18 — Closed rule-kind list.** A rule's `kind` must be one of `TOOL_KINDS` (all ten, unlike
  `deny_kinds`' seven, `model/persona.rs:21-35`), checked by `parse_rules` **and** added to the
  private `permission_refusal` (`model/persona.rs:423`) **after** its empty-match and NUL checks,
  so existing sentences keep their precedence; the kind is escaped with `escape_debug` as
  `kind_not_narrowable` does ("persona.permission.rules entry kind \`exec\` is not one of read,
  edit, …"). Both stores then refuse a misspelt kind, which closes the gap `docs/personas.md:186-187`
  names. No stored or fixture rule uses a kind outside the list (seeds carry no rules; store rule
  fixtures use only `execute`). The store-level conformance case (in
  `persona_writers_refuse_every_widening_shape`, `conformance.rs:15510`) is T7's (it owns
  `conformance.rs`).
- **D19 — Import mode of the parser (OQ-7).** `parse_file` is split into a private unvalidated
  reader (today's `:517-608`) and the final `persona_refusal` (`:609-616`). `persona::parse_import
  (text) -> Result<Imported, PersonaFileError>` where `Imported { file: PersonaFile, dropped:
  Vec<String> }` runs the reader, moves `mcp__` entries of `tools` into `dropped`, refuses a file
  whose non-empty `tools` became empty (OQ-7's exception, a new `PersonaFileError` variant), then
  runs `persona_refusal`. `parse_file` (the seed path) keeps its behavior and still refuses `mcp__`
  in `tools`. Probed: all six real agent files on this machine pass the reader once `mcp__` is
  dropped (quoted descriptions with `'` included); the two gortex files hit the exception.
- **D20 — Import on the store worker (OQ-8, OQ-9).** New `htui/src/persona_import.rs`, the
  `skill_import.rs` shape where it applies: `import(backend, path: &str) -> Vec<PersonaOutcome>`
  (one path, unlike the skill import's `&[String]`), assembled into `PersonaImports { personas,
  report }` by the worker as `skills.rs:402-413` does; writer required (offline →
  `Unreachable(DATABASE_UNREACHABLE)`, `htui-store/src/writer.rs:106`), one `personas()` read for
  the names, `create_persona(file.into_new(PersonaId::new()))` per new name, a known name refused
  (OQ-8), a name met twice in one batch refused naming the first file (`Batch.written`,
  `skill_import.rs:167`, `:233-241`), a directory read at depth 0 for `*.md` (new logic, OQ-9),
  a fence-less file skipped, the same caps. Report rows: `Imported{name, path, dropped}`,
  `Refused{path, message}`, `Skipped{path, reason}`. `skill_import::sentence` (`:363`) and
  `read_text` (`:499`) are private; the persona import copies the few-line pattern rather than
  widening their visibility, so `skill_import.rs` stays untouched.

### TUI

- **D21 — Requests and replies** (in `htui/src/store_worker.rs`): `StoreRequest::{Personas,
  CreatePersona { new: NewPersona }, UpdatePersona { id, expected, patch: PersonaPatch },
  DeletePersona { id }, ImportPersonas { path: String }}`; `StoreReply::{Personas(Vec<Persona>),
  PersonaWritten { personas: Vec<Persona>, outcome: PersonaWrite }, PersonaImports(Box<…>)}` with
  `PersonaWrite::{Created(PersonaId), Updated, Deleted, Stale(PersonaId), Gone}` (five requests,
  three replies). The two wildcard-free matches are the only exhaustive ones (fact-check):
  `name()` (`store_worker.rs:931`, `const fn`, string-literal arms) and `try_serve` (`:1571`), where
  the five requests are one or-ed arm routed to `persona_settings::serve` (the
  `agent_settings::serve` arm, `:1689-1692`); `persona_settings::REQUEST_NAMES` (5) in request
  order with the `request_names_match_the_name_arms` test (`htui/src/skills.rs:1088-1100`). The
  settings reply mirrors `StoreReply::AgentWritten` (`store_worker.rs:1297`) and `AgentWrite`
  (`agent_settings.rs:517`, whose `Stale` carries an id, not a row): every write answers with the
  re-read list.
- **D22 — `PersonasSection`** (`htui/src/ui/tabs/settings/personas.rs`), the `agents.rs` shape
  (`agents.rs:349`, `:404-476`): `Mode::{Browse, Editing(Editor), Body(BodyEditor), Rules(RulesEditor),
  Deleting{..}, ImportPath{field}, Report{..}}`, `busy`, `notice`. Browse shows one line per
  persona: `name · description · deny <kinds> · allow <n> · rules <n>`.
  - `n` new / `e` edit: fields `name`, `description`, `tools` (comma list), `disallowed-tools`
    (comma list), `deny-kinds` (comma list), `command-run (y/n)`, `permission-default` (blank,
    `ask`, `deny`) — the frontmatter key spellings, so the docs describe one vocabulary. A new
    persona's body is required (`BLANK_PERSONA_BODY`), so `n` opens the fields and Enter moves on
    to the body editor; Ctrl+S in the body creates the row.
  - `b` edits the body, `r` the rules (D17), each a `TextArea` (`htui/src/ui/text_area.rs:53`;
    Ctrl+S submits, Esc cancels at once, Enter is a newline, Tab/BackTab pass, `:191-247`). The
    Esc warn-once on unsaved text is the caller's (`skills/library.rs:1051-1062`) and is mirrored.
  - `captures_input()` is true in every mode but Browse (`boxes.rs:667-669`), so `h`/`l` type into
    a field instead of cycling sections (`settings/mod.rs:146-151`, `:336-344`).
  - Every save is one `UpdatePersona` with only the changed fields set (`PersonaPatch` is
    all-`Option`); `Stale` rebases untouched fields and says `CHANGED_ELSEWHERE`
    (`agents.rs:1916`), `Gone` says `DELETED_ELSEWHERE`, a `Failed` keeps the editor open with the
    store's sentence.
  - `d` asks `y/n`, naming the refusal up front ("a persona bound to a phase is refused"), the
    `kinds.rs` `Deleting` shape (`:250-313`, `:657`, `:682`, `:1809`).
  - `I` opens a typed path (`skills/library.rs:249`, `:710`, `:861`) and sends `ImportPersonas`;
    the report opens when anything was refused, skipped or had entries dropped.
  - Offline (`backend.writer()` is `None`): `wants_requests` still issues `Personas` when Settings
    opens, which answers `Failed` with `DATABASE_UNREACHABLE`; the section shows that sentence and
    offers no keys but navigation (the `kinds__offline` precedent).
  Registered last in `app/mod.rs:62-73` (`register_all`; "appending moves no existing section's
  line").
- **D23 — Kinds `persona` field (OQ-11).** `CatalogueSnapshot` (`htui/src/catalogue.rs:36`, one
  literal at `:107`, none in tests) gains `personas: Vec<PersonaSummary { id, name }>`, filled by
  `snapshot()` (`:83`, bound `ReadStore + WriteStore`) from `personas()`. `snapshot` is only ever
  reached with a writer (`:180`, `:298`); offline the whole Catalogue request is refused (`:125-127`)
  and `kinds__offline` is unchanged. `edit_phase_fields` (`kinds.rs:1682`) gains `persona`
  (prefilled with the bound name, blank when none); `build_phase_edit` (`:948-1005`) sets
  `PhasePatch.persona` only when the field changed, and `patch_changed` (`:969-973`) compares it
  (otherwise a persona-only edit falls into the budget-only branch): blank → `Some(None)`, a known
  name → `Some(Some(id))`, an unknown name
  refused before sending with "no persona named \`x\`; known: architect, reviewer" (and "no personas
  exist; add one in Settings › Personas" when the list is empty). An id the list does not know (a
  persona created after the snapshot) prints as the id and is left untouched unless edited.
  `phase_line` (`:1842`, today `phase_line(phase)`) takes the persona list as a new parameter and
  appends `· persona <name>` only when bound (I-12; no demo or fixture phase is bound,
  `seed.rs:237`, `pg/demo.rs:265`). Settings › Personas'
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

Order: **{T6 → T7} ∥ T8, then T9, then T10.** T6 and T7 share `store/traits.rs` and
`store/mod.rs` (serial); T8 shares no file with T6, T7 or T9 and depends on neither (verified), so
it runs alongside the T6 → T7 lane. T9 shares no file with T6, T7 or T8 but needs T6's
`parse_import`/`parse_rules`/`format_rules` and T7's `delete_persona` to compile, so it starts after
T7. Independence is decided by the file-set intersections under "Verified claims", not by this
prose. TDD throughout: each task's tests are written first and fail
for the stated reason. Each implementer commits incrementally (`feat(mod-26): T<n> …`).

### T6: Core — N4, rule lines, closed kinds, import parse (serial, first)
- **Action**: D16 (type, `persona_for`, re-exports, engine arms and their tests), D17, D18 (parser
  side and `permission_refusal`), D19.
- **Tests first**: `PersonaNotInSnapshot`'s `Display` equals the old sentence byte-for-byte; the
  engine I-4 tests match the type and their persisted `PromptRefused` reason is unchanged;
  `parse_rules`/`format_rules` round-trip (fixed table + generated cases, D17), quoting, escapes,
  `key=""`, quoted reasons, comments, each error with its line; `kind=exec` refused by
  `parse_rules` and by `permission_refusal` (after the empty-match and NUL refusals);
  `parse_import` drops `mcp__` entries from `tools` and reports them, accepts the two repo
  `.claude/agents` files' frontmatter verbatim, refuses the two all-`mcp__` gortex files (fixtures
  copied into the test), and `parse_file` still refuses them all.
- **Files** (verified complete): `htui-core/src/model/persona.rs`, `htui-core/src/model/run.rs`,
  `htui-core/src/model/mod.rs` (export), `htui-core/src/store/traits.rs` (re-export `:1950` only),
  `htui-core/src/store/mod.rs` (re-export `:28`), `htui-orch/src/engine.rs` (`:1387`, `:3475`,
  `:3482`, `:3857`, `:3871`, `:5478-5481`, docs `:5299`, `:6284`, enum `:6285`, tests
  `:11729-11744`, `:11826`, `:11933`). No other crate uses the function or the type.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-orch --all-features
  --no-fail-fast -- --test-threads=1` (grep `SIGABRT`); `SQLX_OFFLINE=true cargo check --workspace
  --all-targets --all-features`.

### T7: Store — `delete_persona` and `0013` (parallel with T8)
- **Action**: D14, D15; `persona_is_bound` helper; MemStore, PgStore, `Writer`, `UsageSpy`,
  `SpyStore`; `.sqlx` regenerated against a migrated scratch database (`docs/hr-sandbox.md`);
  every migration pin; `CASES` pins.
- **Tests first** (store conformance, both stores): delete an unbound persona (gone from
  `personas()`); delete an unknown id → `NotFound`; delete a bound persona → the sentence naming
  `<project>/<graph>/<phase>` sorted, and the row survives; an override graph's copy is listed;
  more than five phases → "and *n* more"; after the phase is cleared (`Some(None)`) the delete
  succeeds; D18's misspelt rule kind refused on create and update (added to
  `persona_writers_refuse_every_widening_shape`). Pg only: a bind racing the delete still gets the
  sentence (the `pg_criteria.rs:3326` shape) and a bind that loses gets `references_no_row`; the
  index exists (`pg_indexes`).
- **Files** (verified complete): `htui-core/src/store/traits.rs` (method, `:1005-1009` comment,
  `persona_is_bound`), `htui-core/src/store/mod.rs` (re-export beside `:25`; rebases on T6's `:28`
  edit), `htui-core/src/store/mem.rs` (import `:60`),
  `htui-core/src/store/conformance.rs`, `htui-core/tests/mem_store.rs`,
  `htui-store/src/pg/write.rs` (import `:55`), `htui-store/src/writer.rs`,
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
  prefilled (rename `e_on_a_phase_opens_six_fields_and_enter_sends_update_phase`, `:1554`, and its
  doc); a typed known name sends `UpdatePhase` with `persona: Some(Some(id))`; a persona-only change
  is a patch, not a budget write; blanking a bound one sends `Some(None)`; an untouched field sends
  `None`; an unknown name is refused before sending, listing the known names; a bound phase's line
  shows `· persona reviewer` and an unbound one's is unchanged; the worker's `snapshot` carries the
  fixture's two personas. Snapshots: `kinds__editor_phase` updated (seven fields), one new
  `kinds__phase_persona`; the other six `kinds__*` unchanged.
- **Files** (verified complete): `htui/src/ui/tabs/settings/kinds.rs`, `htui/src/catalogue.rs`
  (field, `PersonaSummary`, the `:107` literal, `snapshot`), `htui/tests/kinds.rs`,
  `htui/tests/snapshots/kinds__editor_phase.snap`, `kinds__phase_persona.snap` (new).
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
  `htui/tests/settings.rs` (strip pin: eight sections, 71 of 100 columns),
  `htui/tests/snapshots/personas__*.snap` (seven new). `tests/connection.rs` does not move
  (verified: its section walk reaches Connection before the appended section).
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui
  --all-features` (no pending snapshots).

### T10: Docs and pins (serial, last)
- **Action**: `docs/personas.md` (replace "Binding a persona to a phase in milestone 1" with the
  Kinds field, add the Personas section, the import and its `mcp__` rule, rule-line syntax, delete,
  D18's closed list; shrink "What milestone 1 does not do yet" to the remaining out-of-scope list);
  `README.md` (Settings summary `:172` and the sections table `:303-311`); `HANDOFF.md` pins
  re-counted, not incremented (`:47` `StoreRequest`/`StoreReply` are already stale at 96/55 against
  99/58 in the tree — expected 104/61 after T9; `:48` snapshots 134 → expected 142, `.sqlx` 318 →
  N, `persona_settings::REQUEST_NAMES` 5 beside `skills::REQUEST_NAMES` 6; `:50` eight Settings
  sections, 71 of 100 strip columns; `CASES`; migrations 13, next `0014`);
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
| R-12 the rule-line grammar fails on a real matcher value | Medium | Quoted values with `\" \\ \n \t \r` escapes, quoted reasons; round-trip over a fixed table plus generated cases (D17) |
| R-13 an imported file keeps more tools than its author meant | Medium | OQ-7's exception refuses an all-`mcp__` `tools`; every drop is named in the report |

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

- [x] I-8…I-12 hold. Tests that carry the label: I-9 (conformance
      `a_bound_persona_is_not_deleted_and_names_its_phases` and the two `pg_criteria.rs` races) and
      I-12 (`tests/kinds.rs`). I-8, I-10 and I-11 are named in the module docs
      (`persona_settings.rs`, `persona_import.rs`) and pinned by tests that do not carry the label:
      `a_form_refusal_is_the_stores_sentence_and_focuses_its_field`,
      `a_misspelt_rule_kind_is_refused_before_sending`,
      `a_refused_create_answers_the_stores_sentence_bare` (I-8);
      `the_report_prints_no_file_content` and the import walk cases (I-10);
      `offline_the_import_is_unreachable` and the worker's `serve` cases (I-11)
- [x] PRD milestone 2 outcome pinned per the test plan; N4 (`cd3de2ff`) and N5 (`7c7e78c2`) closed;
      the "persona applied end to end" metric stays a manual check on the host after collect
- [x] Validation passes; reviewer gate (`rust-reviewer`) findings applied or deferred with the
      maintainer. Approve with fixes, 0 critical/high. R1 fixed M-1 `9fca4b56`, L-1 `d5348fa2`, L-2 `10df35e0`, L-3
      `0f175aa2`, L-4 `2aedd54c` and N-2 `a3620ce3` (persona import only). The R1 verifier's ADV-1
      was fixed in `3b7b6297`. N-1 (long functions) was not done, by maintainer decision. T10
      changed docs only and re-ran only the workflow-docs validator. The code gates are the run's
      R1 verification.
- [x] Close-out restates moved counts from a fresh count: see "Close-out counts" below

### Close-out counts

Fresh count at T10 (2026-10-03, on `3b7b6297`), each read at its pin site:

- Store conformance `CASES`: 130 → **133** (`htui-core/src/store/conformance.rs`; pinned at
  `htui-core/tests/mem_store.rs` and `htui-store/tests/pg_conformance.rs` `EXPECTED_CASES`).
  `persona_writers_refuse_every_widening_shape` shapes: 17 → **18** (D18).
- `READ_CASES`: **14**, unchanged.
- `htui-orch` `CASES`: **92**, unchanged.
- Migrations: 12 → **13** (`0013_persona_phase_index.sql`, index only; next `0014`, cache next
  `0005`).
- Postgres `TABLES`: **42**, unchanged; commented columns: **44**, unchanged.
- `.sqlx` files: 318 → **321** (`ls crates/htui-store/.sqlx | wc -l`).
- Snapshots: 134 → **142** under `crates/htui/tests/snapshots` (+1 `kinds__phase_persona`, +7
  `personas__*`); `kinds__editor_phase` updated, every other existing snapshot unchanged.
- `StoreRequest` / `StoreReply`: 99 / 58 → **104 / 61** (`htui/src/store_worker.rs`; HANDOFF said
  96 / 55, already stale before this milestone).
- `persona_settings::REQUEST_NAMES`: **5** (new); `skills::REQUEST_NAMES` **6**, unchanged.
- Settings sections: 7 → **8**, 61 → **71** of the 100 strip columns
  (`htui/tests/settings.rs` `the_section_strip_fits_the_frame`).

## Verified claims

Fact-check 2026-10-02 at `ac8d0c3a`. Probes ran under `/tmp` and in a scratch database
(`mod26_v2_probe`), all removed.

| Claim | Verdict | Evidence / amendment |
|---|---|---|
| `persona_for` returns `Result<Option<&SnapshotPersona>, String>` (`run.rs:469`) | CONFIRMED | `run.rs:469-481`; intra-doc link `:464` must be repointed (D16) |
| Sentence fn and re-exports (`persona.rs:330`, `traits.rs:1950`, `store/mod.rs:28`) | CONFIRMED | Uses `escape_debug` → kept in `#[error]` (D16); `model/mod.rs` export added |
| All users of `persona_not_in_snapshot`/`persona_for` are in htui-core and engine.rs | CONFIRMED | No other crate; full line list in T6 |
| `:3475` and `:3857` both go through `refuse_prompt` | PARTLY | `:3857` builds `PromptRefused` inline for `fail_group_before_a_token` (D16 amended) |
| Persisted reasons stay byte-identical | CONFIRMED | `String` in `status.rs:96-100`, `command.rs:477-482`; `.to_string()` at the four sites |
| `TOOL_KINDS` has ten entries; `permission_refusal` ignores kinds | CONFIRMED | `persona.rs:21-35`, `:423` |
| D18 refuses no existing data | CONFIRMED | Seeds carry no rules; store rule fixtures use only `execute` |
| D18 placement | AMENDED | After empty-match and NUL checks; `escape_debug`; store case moved to T7 |
| D19 import mode without duplicating the parser | CONFIRMED | Split `parse_file` (`:517-608` reader, `:609-616` refusal) |
| Real agent files carry only `name, description, tools`, all with `mcp__` | CONFIRMED (probe) | Six files; all pass the reader once `mcp__` is dropped |
| OQ-7 "dropping narrows nothing away" | PARTLY | Two gortex files become an empty `allow` (= all built-ins) → refused (OQ-7 exception, R-13) |
| D17 round-trip over conformance rule fixtures | FALSIFIED | Three fixtures, all refusals → fixed table + generated cases; grammar gains `\n \t \r`, `key=""`, quoted reasons |
| T6 htui-orch validate command | FALSIFIED | `--no-fail-fast` is a cargo flag → before `--` |
| Five `WriteStore` implementors, no default methods | CONFIRMED | `mem.rs:6494`, `pg/write.rs:777`, `writer.rs:313`, `htui-agent/src/conformance.rs:743`, `tests/recorder.rs:431` |
| Delete precedent (`delete_item_kind`, `item_kind_is_held`, Mem, Pg, conformance, race test) | CONFIRMED | Anchors as cited; race fn at `pg_criteria.rs:3326`; helper re-exported at `store/mod.rs:25` |
| `<graph>/<phase>` identifies a phase | PARTLY | Graph names unique per project only (`0001_init.sql:221`) → `<project>/<graph>/<phase>`; override graphs copy `persona_id` (`graph.rs:540-544`) (D14 amended) |
| Pg race: a bind cannot slip in after `FOR UPDATE` | CONFIRMED (probe) | Binder-first → `DELETE 0` + sentence; deleter-first → binder `23503` → `references_no_row`; without the lock → raw `23503` |
| No index on `persona_id`; FK creates none | CONFIRMED (probe) | `\d step_graph_phase` on a 12-migration scratch DB |
| No cache migration needed | CONFIRMED | `cache_migrations/0001-0004` never mention the table |
| Migration pins | CONFIRMED, list completed | `migrations.rs` `:96 :989 :1088 :1093 :1112 :1262` + prose; `connect.rs` `:140 :156 :242` + prose |
| `CASES` 130, `READ_CASES` 14, pins | CONFIRMED | `mem_store.rs:36-37`, `:63-64`; `pg_conformance.rs:26` |
| `.sqlx` 318, pinned by a test | PARTLY | 318 files; pinned only in `HANDOFF.md:48` prose (T10) |
| `0013` free | CONFIRMED | No `0013*` on any branch; host tree latest `0012` |
| One `CatalogueSnapshot` literal; `snapshot()` can call `personas()` | CONFIRMED | `catalogue.rs:107`; bound `:87` |
| `snapshot()` empty offline | FALSIFIED | Only reached with a writer; offline Catalogue is refused (D23 amended) |
| `phase_line`/`build_phase_edit` anchors | CONFIRMED, amended | `phase_line(phase)` needs the persona list; `patch_changed` `:969-973` must compare persona |
| No bound demo phase → only `kinds__editor_phase` changes | CONFIRMED | `seed.rs:237`, `pg/demo.rs:265` |
| `StoreRequest` 99 / `StoreReply` 58 vs HANDOFF 96/55 | CONFIRMED | `store_worker.rs:125-926`, `:1056-1343`; `HANDOFF.md:47` |
| Only `name()` and `try_serve` match exhaustively | CONFIRMED | `:931` (`const fn`), `:1571`; worker loop and testkit fall through |
| `captures_input`, Boxes' `TextArea`, `TextArea` keys | CONFIRMED | `settings/mod.rs:146-151`, `boxes.rs:667-669`, `text_area.rs:191-247`; warn-once is `library.rs:1051-1062` |
| Agents/kinds/app anchors | CONFIRMED | `AgentWritten` is `store_worker.rs:1297`; `AgentWrite::Stale` carries an id (D21 amended) |
| Eighth section fits the strip | CONFIRMED | 61 + 10 = 71 of 100 |
| `tests/connection.rs` moves | FALSIFIED | Neither test moves (T9 amended) |
| Skill import walks depth 0 of a directory | FALSIFIED | Depth ≤ 4, `SKILL.md` and rules dirs only, refuses "every `*.md`" (R-37) → D20/OQ-9 call the sweep new; fence-less files skipped |
| `skill_import::sentence` reusable | FALSIFIED | Private (`:363`, `read_text` `:499`) → copied, `skill_import.rs` untouched |
| HANDOFF/README anchors | CONFIRMED | `HANDOFF.md:47`, `:48`, `:50`; `README.md:172`, `:303-311` |

**File-set intersections** (decide parallelism):

| Pair | Intersection | Verdict |
|---|---|---|
| T6 ∩ T7 | `htui-core/src/store/traits.rs`, `htui-core/src/store/mod.rs` | serial (T6 → T7) |
| T6 ∩ T8 | ∅ | parallel |
| T7 ∩ T8 | ∅ | parallel |
| T6 ∩ T9, T7 ∩ T9, T8 ∩ T9 | ∅ | file-disjoint, but T9 needs T6 and T7 to compile → after T7 |

