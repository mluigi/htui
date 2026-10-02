# Blueprint: MOD-26 — Declarative agent personas, milestone 2 (authoring in the TUI), T6–T10

**Status**: proposed (2026-10-02, code-architect). Implements
`.claude/plans/mod-26-m2-persona-authoring.plan.md` (CONFIRMED 2026-10-02, OQ-7 to OQ-12 as
recommended, fact-checked) under `.claude/prds/mod-26-agent-personas.prd.md`. The plan's D14–D23,
I-8–I-12, task order `{T6 → T7} ∥ T8, then T9, then T10`, file sets and "Verified claims" are
binding. Where this blueprint had to choose, the choice is a **B-n**, driven by a finding **F-n**.
Every place the plan is wrong or under-specified against the tree is listed in §0b as a
**Deviation** (with evidence and resolution). There is **no BLOCKER**: every confirmed OQ decision
is feasible as written.

**Verified at**: `cb69a011` (`hr/MOD-26`, sandbox). `git diff aeba8f63 cb69a011` touches only the
plan, so the plan's line numbers hold; every number below was re-read at HEAD. Paths are relative
to `crates/` unless they start with `docs/`, `.claude/` or name a root file. Gortex answers symbol
reads on this checkout despite its "INACTIVE" banner; its `text` search works for `crates/htui/src`.

**House style (carried from M1, MOD-41/MOD-42)**: `unsafe_code = "forbid"`; `missing_docs`,
`missing_debug_implementations`, `unused_qualifications` warn and clippy runs `-D warnings`; every
new `pub` item is documented and `Debug`; **no default body on a store trait**; `max_width = 100`;
edition 2024 (`if let … && …` chains); every commit compiles **and is green** (tests are written
first and seen to fail locally, then the implementation lands in the same commit); commits are
incremental (uncommitted work dies with the session; **never stash**), subject
`feat(mod-26): T<n> …` / `test(mod-26): …` / `docs(mod-26): …`, trailer
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. **E0034 hygiene**: no module `use`s
`RecorderStore`, `RelayStore`, `WorkerStore` or `WorkerHost`. `every_cross_referenced_test_name_exists`
(`htui-core/src/store/conformance.rs`): a backticked snake_case name with ≥ 4 underscores in that
file's docs must be a fn there or in `mem.rs`; a test elsewhere is spelled `pg_criteria.rs::name`
and must already exist. **Never the word `zeta`** in an identifier, fixture or seed. No test reads
the real home (`~/.claude/agents`): the real files' frontmatter is **copied** into test constants.
Every test command is `--all-features` (without `testkit`, `htui-store`/`htui` integration tests
compile to empty binaries and print `ok. 0 passed`) and `--test-threads=1` (the keyring fake is
process-wide). `serde_json::Value` is never used to hash anything.

**Layout**: §0 findings · §0a decisions · §0b deviations and blockers · §1 build order · §2 shared
shapes (2.1–2.13) · §3 T6 · §4 T7 · §5 T8 · §6 T9 · §7 T10 · §8 lanes · §9 pins · §10 gate reference.

---

## 0. Findings

| # | Severity | Plan says | Tree / evidence | Fix |
|---|---|---|---|---|
| **F-1** | Minor (T6) | D18: kind check "after its empty-match and NUL checks". | `permission_refusal` (`model/persona.rs:423-442`) loops rule by rule: empty match, then NUL, **per rule**. A per-rule kind check would let rule 1's bad kind beat rule 2's empty match, changing the sentence of an input M1 already refused. | **B-1** |
| **F-2** | Minor (T6) | D17: escapes `\" \\ \n \t \r`. | Both stores accept every non-NUL control char in matcher values and reasons (`permission_refusal` checks NUL only). `TextArea` splits lines on `\n` only (`ui/text_area.rs`) and stores other controls verbatim, drawing them as one-cell stand-ins. | **B-2** (five escapes suffice; no `\u{…}`) |
| **F-3** | Minor (T6, T9) | D22: the form's `tools` etc. are "comma list"s. | The file grammar's list splitter `list_of` is private (`model/persona.rs:637-645`). A second splitter in the form would be a second grammar (I-8). | **B-3** |
| **F-4** | Minor (T6) | D16: "`#[error]` … the `PersonaFileError` precedent". | No `#[error]` in the workspace passes `.field.method()` today; thiserror 2 (`Cargo.toml:58`) supports it (`#[error("{}", .persona.escape_debug())]`). | Used as written |
| **F-5** | Minor (T6) | D16 line list. | Two doc mentions of the `String` payload besides the plan's: `engine.rs:5360` (`assemble_prompt` doc) and `:5458` (`phase_spec` doc, "`Err(NoPersona(reason))`"). `run.rs:1156-1176` (`persona_for_finds_refuses_and_skips`) imports the removed fn. All inside T6's files. | §3.1 |
| **F-6** | Minor (T7) | D14: `persona_is_bound(name, &phases)`; "sorted and deduped on that triple". | Sorting the joined `slug/graph/phase` strings is **not** sorting the triple (`"web-app/x" < "web/x"` because `-` < `/`, but `("web", …) < ("web-app", …)`). | **B-4**: the helper takes the triples |
| **F-7** | Minor (T7) | D14 "and on zero rows a query for the holders". | A phase whose graph or project row were missing would vanish from an inner `JOIN` (Pg) or a `filter_map` (Mem) and could let a "bound" refusal name zero phases. Unreachable on Pg (NOT NULL FKs), unreachable on Mem (phases go with their graph). | **B-5** |
| **F-8** | Minor (T7) | T7 Validate omits `htui-agent`. | `UsageSpy` (`htui-agent/src/conformance.rs:743`) and `SpyStore` (`htui-agent/tests/recorder.rs:431`) are `WriteStore` impls with no default bodies; `-p htui-store` does not build `htui-agent`'s tests. | §4.4 adds `nopg htui-agent` |
| **F-9** | Minor (T8) | D23: `patch_changed` must compare persona "otherwise a persona-only edit falls into the budget-only branch". | `build_phase_edit` (`kinds.rs:948-1005`) takes the budget-only branch only when `budget_changed && !patch_changed`. A persona-only edit (`budget_changed == false`) already goes out as `UpdatePhase`; the loss is a **persona + budget** edit with no other column moved: one `SetPhaseBudget`, persona dropped. | Deviation D-4; tests pin both shapes |
| **F-10** | Minor (T8) | "the other six `kinds__*` unchanged". | There are **eight** `kinds__*` snapshots (`delete_ask demo editor_phase no_workspace offline phase_detail prefix_warn stale`). | Deviation D-5 (seven unchanged) |
| **F-11** | Minor (T8) | D23 says nothing of the editor's stored text. | Every phase-edit field goes out whole (B-5 of MOD-15 M4, `kinds.rs:854-857`), except `token_budget`, whose comparison uses `Editor.stored_budget`, which a `CatalogueStale` deliberately does **not** refresh (`kinds.rs:1119-1132`): "only a user who typed in the column writes it". `persona` must be compared the same way, or an untouched field would rewrite another writer's binding. | **B-8** |
| **F-12** | Minor (T9) | D20: `import(backend, path: &str) -> Vec<PersonaOutcome>`. | The import must refuse offline before any read (`DATABASE_UNREACHABLE`) and propagates the one `personas()` read's error, exactly as `skill_import::import` returns `Result<Vec<ImportOutcome>>` (`skill_import.rs:129`). | Deviation D-2 |
| **F-13** | Minor (T9) | D21: `PersonaWrite::{Created(PersonaId), Updated, Deleted, Stale(PersonaId), Gone}`. | `AgentWrite` (`agent_settings.rs:517-575`) names the row in **every** variant (`Edited { id, name }`, `Gone { id }`) so a reply closes only its own row's editor (MOD-23 F-14): an editor can be closed with `Esc` while its write is in flight and another opened. | Deviation D-3 / **B-10** |
| **F-14** | Major (T9) | D21: refusals are "the store's sentence". | `failed()` renders `StoreError` through `Display` (`store_worker.rs:1881-1886`); `Constraint` prints `constraint violated: <sentence>` (`store/error.rs:21`). The form would show a prefix the local check never shows (I-8). `agent_settings::refused` (`:712-717`) is the precedent for sending the sentence bare. | **B-11** |
| **F-15** | Minor (T9) | D22: `Browse` keys `n e b r d I`. | The global keymap binds `q`, `Tab`, `BackTab`, digits, `?`, `ctrl-c` (`keymap.rs:205-254`) and `register_all` binds `w`; the Settings tab consumes `h l [ ] ←→` while the section does not capture (`settings/mod.rs:336-351`). `r` is Kinds' reload; here it is the rules editor (D22), so Personas has **no reload key**: a refused read is retried by re-activating the tab, as Agents does. | B-12 |
| **F-16** | Minor (T9) | D22: `TextArea` "Tab/BackTab pass". | `TextArea::on_key` passes them (`text_area.rs:191-247`); the Boxes section then **swallows** every non-`CONTROL` pass (`boxes.rs:533-534`), so `Tab` inside an editor does not switch tabs. Same for `TextField` passes in the import path. | §2.13 key tables |
| **F-17** | Note (T9) | D22 / I-12. | Every snapshot that draws the Settings strip builds its own section list (`tests/kinds.rs:763-776`, `hierarchy`, `prompt_settings`), never `register_all`; the product's own registration is exercised only by `tests/connection.rs::the_product_registers_connection_after_prompt` (four `l` still reach Connection). Appending `Personas` moves no snapshot. | Confirms the plan |
| **F-18** | Note (T9) | D22 "the report opens when anything was refused, skipped or had entries dropped". | A directory sweep of `.claude/agents/` drops `mcp__` entries from every real file, so a real import **always** opens the report. Intended (R-13: every drop is named). | — |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B-1** (F-1): the D18 kind check is a **second pass** in `permission_refusal`, after the
  existing per-rule loop has passed every rule: any rule set M1 refused is refused with the same
  sentence; a new sentence appears only for a rule set M1 accepted. NUL in a kind is therefore
  still `has_nul("persona.permission.rules")`.
- **B-2** (F-2): the rule-line formatter escapes exactly D17's five (`\"`, `\\`, `\n`, `\t`, `\r`);
  any other control char is written **raw inside quotes** (a bare value never holds one), and the
  parser takes every char inside quotes literally except `"` and `\`. A stored value with an `ESC`
  in it therefore round-trips and still prints on one line. No `\u{…}` escape.
- **B-3** (F-3): `persona::list_of` becomes `pub` (doc: "the one-line list grammar of a persona
  file and of the Settings › Personas form"); the form splits with it and prints with
  `.join(", ")`, which round-trips because the tool-name rule forbids commas and whitespace.
- **B-4** (F-6): `persona_is_bound(name: &str, holders: &[(String, String, String)]) -> String`
  sorts the triples (`Ord` on `(String, String, String)` is component-wise byte order), dedups,
  names at most five, then `and n more`.
- **B-5** (F-7): MemStore refuses on `phases.iter().any(|p| p.persona_id == Some(id))` and names a
  holder whose graph or project is missing as `?`; Pg's guard is the `NOT EXISTS` in the `DELETE`
  (raw rows), and the holders query is informational. The refusal can never be skipped by a
  missing join.
- **B-6**: three new store `CASES` (130 → 133): `delete_persona_removes_an_unbound_row_once`,
  `a_bound_persona_is_not_deleted_and_names_its_phases`,
  `a_persona_bound_to_many_phases_names_five_and_counts_the_rest`. D18's store-level shape is an
  **18th** shape of the existing `persona_writers_refuse_every_widening_shape` (17 → 18), not a
  case.
- **B-7**: migration `0013` changes only the migration-count pins; the `TABLES` messages that say
  "MOD-26's 0012_persona.sql adds persona" stay true and stay (Deviation D-6).
- **B-8** (F-11): the Kinds phase editor gains `stored_persona: Option<String>` (the field's text
  at open, never refreshed by `CatalogueStale`); `PhasePatch.persona` is `None` while the field
  still reads that text.
- **B-9**: T8 runs in its own worktree `hr/MOD-26-t8` (§8) because T7 changes the `WriteStore`
  trait under the same `target/`; it merges after T7.
- **B-10** (F-13): `PersonaWrite` carries ids (and names where the notice needs one), §2.10.
- **B-11** (F-14): `persona_settings::serve` turns a `StoreError::Constraint(sentence)` from a
  write into `Ok(StoreReply::Failed { request, message: sentence })` — the sentence byte for byte —
  and leaves every other error an `Err` (so `Unreachable` still drives `go_offline`).
- **B-12** (F-15): Personas `Browse` binds `j k ↓ ↑ n e b r d I`; everything else passes. No
  reload key.
- **B-13**: the section mints the new persona's id (`PersonaId::new()` in the form, as
  `NewPersona.id` is "minted client-side", `model/persona.rs:84`); the worker passes `NewPersona`
  through untouched.
- **B-14**: a directly named file is read whatever its extension (the skill import's rule); a
  directory contributes only its depth-0 regular `*.md` files (extension exactly `md`), following
  symlinks to files (`std::fs::metadata`), sorted by file-name bytes; subdirectories and non-`.md`
  files are **left alone and not reported** (a sweep that reported every non-`.md` file would open
  the report for every directory). A fence-less file is `Skipped` whether named or swept.
- **B-15**: the T6 engine tests assert the persisted reason as the **literal** M1 sentence, not
  through the new type's `Display`, so byte-identity is pinned independently of the type.

### 0b. Deviations and blockers

**BLOCKERs: none.** OQ-7 … OQ-12 are implementable as confirmed.

| # | Plan text | Evidence | Resolution |
|---|---|---|---|
| **D-1** | D18 "after its empty-match and NUL checks" | F-1 | Second pass (B-1); strictly preserves every M1 sentence |
| **D-2** | D20 `import(…) -> Vec<PersonaOutcome>` | F-12 | `-> htui_core::store::Result<Vec<PersonaOutcome>>` |
| **D-3** | D21 `PersonaWrite` variant payloads | F-13 | `Created{id,name}`, `Updated{id,name}`, `Deleted{id}`, `Stale{id}`, `Gone{id}` |
| **D-4** | D23 "otherwise a persona-only edit falls into the budget-only branch" | F-9 | The broken shape is persona + budget; both shapes tested |
| **D-5** | T8 "the other six `kinds__*` unchanged" | F-10 | Seven unchanged |
| **D-6** | D15 "the 'twelve'/`0012_persona.sql` prose (`:101-102`, `:122-124`, `:130-132`, …)" | `migrations.rs:118-133` count **tables**, which 0013 does not move | Only migration-count prose moves (§9) |
| **D-7** | D17 escape list | F-2 | Five escapes kept; other controls raw inside quotes (B-2) — a clarification, not a widening |
| **D-8** | T7 Validate | F-8 | Adds `htui-agent` tests to G-T7 |
| **D-9** | D21 refusals "the store's sentence" | F-14 | `Constraint` → bare `Failed` (B-11) |
| **D-10** | §3.3(b) "the sentence for ``"e`x"`` is one line with the backtick escaped" | §2.2 and D18 use `escape_debug` (as `kind_not_narrowable`), which never escapes a backtick | The backtick stays raw; only `escape_debug`'s escapes apply, so the sentence stays on one line; the test pins the literal sentence (commit 8884a86d) |
| **D-11** | §2.7 "in full": one guarded `DELETE`, then an unconditional holders read and refusal | F-7/B-5 judged a zero-phase "bound" sentence unreachable on Pg, but the guard and the holders read are two READ COMMITTED snapshots: an unbind, phase delete or graph delete committing between them empties the holders read | The Pg guard runs in a loop: an empty holders read re-runs the guarded `DELETE` (holders only shrink under the persona's `FOR UPDATE`, so it ends); the same three statements, so the `.sqlx` count stays 321; pinned by `pg_criteria.rs::an_unbind_racing_a_persona_delete_lets_it_delete` (commit f4f9e9bc) |

---

## 1. Build order

| Task | Crates | Commit groups (each compiles and is green) | Where | Gate |
|---|---|---|---|---|
| T6 | core, orch | 6.1 N4 typed refusal · 6.2 closed rule kinds · 6.3 rule lines · 6.4 import mode | primary `hr/MOD-26` | G-T6 |
| T7 | core (store), store, agent | 7.1 `delete_persona` everywhere + `.sqlx` + Pg race tests · 7.2 migration `0013` + pins | primary, after T6 | G-T7 |
| T8 | htui (catalogue, kinds) | 8.1 catalogue persona list · 8.2 Kinds `persona` field | worktree `hr/MOD-26-t8` from `cb69a011`, merged `--no-ff` after T7 (B-9) | G-T8, then G-M (§10) |
| T9 | htui | 9.1 `persona_import` · 9.2 requests + `persona_settings` · 9.3 section: browse, form, body · 9.4 section: rules, delete, import, report · 9.5 registration, strip pin, offline | primary, after the T8 merge | G-T9 |
| T10 | docs | 10.1 docs, pins, PRD row, plan status | primary | G-Final |

T8 may also simply run after T7 on the primary tree if no worktree is wanted; it shares no file
with T6, T7 or T9 (plan "Verified claims", re-checked: `catalogue.rs`, `kinds.rs`,
`tests/kinds.rs`, two snapshots).

---

## 2. Shared code shapes

Everything here crosses a task boundary and is settled. An implementer who believes a shape is
wrong stops and reports; nobody edits another task's files.

### 2.1 `PersonaNotInSnapshot` (T6, D16) — `htui-core/src/model/persona.rs`

Replaces `persona_not_in_snapshot` (`:328-336`), same position:

```rust
/// I-4 (plan D12; MOD-26 milestone 2 D16, review N4): a phase names a persona its run's snapshot
/// does not carry. `Display` is the sentence `RunFailure::PromptRefused.reason` stores, byte for
/// byte as milestone 1 wrote it; the name is `escape_debug`'d so a stored reason is one line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "persona `{}` is not in the run's snapshot, so the step does not run un-narrowed \
     (MOD-26 I-4)",
    .persona.escape_debug()
)]
pub struct PersonaNotInSnapshot {
    /// The name `SnapshotPhase.persona` carries, unescaped.
    pub persona: String,
}
```

`htui-core/src/model/run.rs:462-481`:

```rust
    /// MOD-26 D12, I-4: the frozen persona `phase` names. `Ok(None)` for a persona-less phase;
    /// `Err` is [`PersonaNotInSnapshot`](crate::model::persona::PersonaNotInSnapshot) when the
    /// snapshot does not carry the name.
    ///
    /// # Errors
    /// As above.
    pub fn persona_for(
        &self,
        phase: &SnapshotPhase,
    ) -> Result<
        Option<&crate::model::persona::SnapshotPersona>,
        crate::model::persona::PersonaNotInSnapshot,
    > {
        let Some(name) = phase.persona.as_deref() else {
            return Ok(None);
        };
        self.personas
            .iter()
            .find(|persona| persona.name == name)
            .map(Some)
            .ok_or_else(|| crate::model::persona::PersonaNotInSnapshot {
                persona: name.to_owned(),
            })
    }
```

Re-exports: `model/mod.rs:139-142` `pub use persona::{…}` gains `PersonaNotInSnapshot` and
`RuleLineError` (§2.3). `store/traits.rs:1947-1951` and `store/mod.rs:16-41`: replace
`persona_not_in_snapshot` with `PersonaNotInSnapshot` and add `rule_kind_unknown` (§2.2) — the T7
re-export of `persona_is_bound` is added beside `item_kind_is_held` later (the two lines are
distinct; T7 rebases cleanly).

Engine (`htui-orch/src/engine.rs`), every site:

| Line | Today | After |
|---|---|---|
| `:6278-6286` | `NoPersona(String)` + doc "carries `persona_not_in_snapshot`'s sentence" | `NoPersona(htui_core::model::persona::PersonaNotInSnapshot)`; doc "carries the typed refusal; its `Display` is the stored sentence" |
| `:5478-5482` | `Err(reason) => return Ok(Err(StageThree::NoPersona(reason)))` | `Err(refusal) => return Ok(Err(StageThree::NoPersona(refusal)))` (the `Err(_) if !strict => None` arm unchanged) |
| `:3474-3477` | `refuse_prompt(run, step, phase, reason)` | `refuse_prompt(run, step, phase, refusal.to_string())` |
| `:3480-3486` | `.map_err(\|reason\| EngineError::Snapshot { run: run.id, reason })` | `.map_err(\|refusal\| EngineError::Snapshot { run: run.id, reason: refusal.to_string() })` |
| `:3856-3866` | `RunFailure::PromptRefused { phase, reason }` | `reason: refusal.to_string()` |
| `:3869-3875` | as `:3480` | as `:3480` |
| `:1387-1392` | `format!("the handoff prompt was refused: {reason}")` | binding renamed `refusal`; the `format!` reads `{refusal}` (same text) |
| docs `:5297-5299`, `:5360`, `:5458` | name the fn / `NoPersona(reason)` | name [`PersonaNotInSnapshot`] / `NoPersona(refusal)` |
| tests `:11729-11747` | `matches!(… NoPersona(reason)) if *reason == expected` | `matches!(&refused, Err(super::StageThree::NoPersona(refusal)) if refusal.persona == "reviewer")` and `refusal.to_string()` equals the literal (B-15) |
| tests `:11826`, `:11933` | `reason: persona_not_in_snapshot("reviewer")` | `reason: M1_SENTENCE.to_owned()` with `const M1_SENTENCE: &str = "persona \`reviewer\` is not in the run's snapshot, so the step does not run un-narrowed (MOD-26 I-4)";` in the test module |

`StageThree` stays `#[derive(Debug)]`; no future grows (R-11).

### 2.2 Closed rule kinds (T6, D18) — `htui-core/src/model/persona.rs`

New helper beside `kind_not_narrowable` (`:318-326`):

```rust
/// MOD-26 milestone 2 D18: a rule's `tool_kind` outside [`TOOL_KINDS`]. A misspelt kind matches
/// nothing (`permission.rs` compares the text), so the rule would silently do nothing.
#[must_use]
pub fn rule_kind_unknown(kind: &str) -> String {
    format!(
        "persona.permission.rules entry kind `{}` is not one of read, edit, delete, move, search, \
         execute, think, fetch, switch_mode, other",
        kind.escape_debug()
    )
}
```

Rendered: ``persona.permission.rules entry kind `exec` is not one of read, edit, delete, move,
search, execute, think, fetch, switch_mode, other`` (one line).

`permission_refusal` (`:421-442`) keeps its loop and its doc gains "then (MOD-26 M2 D18, B-1) a
`tool_kind` outside [`TOOL_KINDS`], checked once every rule has passed the two checks above, so a
rule set milestone 1 refused keeps its sentence"; the tail becomes:

```rust
    permission
        .rules
        .iter()
        .filter_map(|rule| rule.matcher.tool_kind.as_deref())
        .find(|kind| !TOOL_KINDS.contains(kind))
        .map(rule_kind_unknown)
```

Both stores call `persona_refusal`/`persona_patch_refusal`, so both refuse it from T6 on; the
conformance shape lands in T7 (owner of `conformance.rs`).

### 2.3 Rule lines (T6, D17, OQ-10) — `htui-core/src/model/persona.rs`

```rust
/// The four matcher keys of a rule line (MOD-26 M2 D17), in the order [`format_rules`] writes
/// them: `kind` → `tool_kind`, `name` → `tool_name`, `path` → `path_prefix`, `command` →
/// `command_prefix`.
pub const RULE_KEYS: [&str; 4] = ["kind", "name", "path", "command"];

/// Why [`parse_rules`] refused a rules text (MOD-26 M2 D17): the 1-based line and one sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("rules line {line}: {message}")]
pub struct RuleLineError {
    /// The 1-based line of the text.
    pub line: usize,
    /// One sentence; for a kind outside [`TOOL_KINDS`], [`rule_kind_unknown`]'s — the store's.
    pub message: String,
}

/// Reads the Settings › Personas rules editor (MOD-26 M2 D17, OQ-10): one rule per line, blank
/// lines and `#` lines ignored. Pure. The result still goes through the store's rules (I-8):
/// an empty match or a NUL is refused there, not here.
///
/// # Errors
/// The first [`RuleLineError`], in line order.
pub fn parse_rules(text: &str) -> Result<Vec<PersonaRule>, RuleLineError>;

/// Writes `rules` one per line, `\n`-separated, no trailing newline, each in the shortest form
/// [`parse_rules`] reads back to the same rule. Pure.
#[must_use]
pub fn format_rules(rules: &[PersonaRule]) -> String;
```

**Grammar (exact EBNF).** Applied to each line of `text.split('\n')`, after removing **one**
trailing `\r` (a pasted CRLF). `ws` is any char with `char::is_whitespace`; `control` any char with
`char::is_control`; `any` any char.

```ebnf
line        = blank | comment | rule ;
blank       = { ws } ;
comment     = { ws } , "#" , { any } ;
rule        = { ws } , answer , { ws1 , pair } , [ { ws } , reason ] , { ws } ;
answer      = "reject_once" | "reject_always" ;
pair        = key , "=" , value ;
key         = "kind" | "name" | "path" | "command" ;          (* each at most once per line *)
value       = bare | quoted ;
bare        = bare-char , { bare-char } ;                       (* never empty *)
bare-char   = any - ( ws | '"' | "#" | "=" | "\" | control ) ;
quoted      = '"' , { q-char | escape } , '"' ;
q-char      = any - ( '"' | "\" ) ;                             (* raw controls allowed, B-2 *)
escape      = "\" , ( '"' | "\" | "n" | "t" | "r" ) ;
reason      = "#" , { ws } , ( quoted , { ws } | text ) ;       (* quoted iff next char is '"' *)
text        = { any } ;                                         (* to end of line, then trimmed *)
ws1         = ws , { ws } ;
```

Tokenising rules that make the EBNF deterministic:

1. A line whose first non-`ws` char is `#` is a comment. A blank line is skipped. Line numbers
   count every line, 1-based.
2. The **answer word** is the maximal run of chars that are neither `ws` nor `#`.
3. After the answer and after each pair: skip `ws`; end of line → done; `#` → reason; otherwise a
   pair, which **requires** that at least one `ws` was skipped (else E-9).
4. A **key** is the maximal run of chars not in `ws # = "`; it must be followed by `=`.
5. A **bare value** is the maximal run of chars that are neither `ws` nor `#` (so `#` ends it and
   opens the reason); it is then refused if empty (E-5) or if it holds `"`, `=`, `\` or a control
   char (E-6).
6. After a **quoted value** the next char must be `ws`, `#` or end of line (E-9).
7. A **quoted reason** must be followed by `ws` only (E-10); a plain reason is the rest of the
   line `.trim()`ed (it may contain `#`, `"` after its first char, `=`).
8. Absent key → `None`; `key=""` → `Some("")`. Values and the reason are unescaped.
9. After a rule is read, its `kind` (when `Some`) must be in `TOOL_KINDS` (E-11).

**Escape table** (both directions; nothing else is an escape):

| In text | Char | Formatter writes it escaped |
|---|---|---|
| `\"` | `"` | always, inside quotes |
| `\\` | `\` | always, inside quotes |
| `\n` | LF | always (a value must stay on one line) |
| `\t` | TAB | always |
| `\r` | CR | always |
| any other `\x` | — | parse error E-8 |

Any other control char (e.g. `ESC`, `DEL`, `U+0085`) is written raw inside quotes (B-2).

**Errors** (`RuleLineError { line, message }`; `Display` = `rules line {line}: {message}`). Exact
rendered `message`s; every interpolated user string is `escape_debug`'d:

| # | When | `message` |
|---|---|---|
| E-1 | answer word is not one of the two | ``a rule starts with reject_once or reject_always, not `{word}` `` |
| E-2 | unknown key | ``` `{key}` is not a rule key; a rule takes kind, name, path and command ``` |
| E-3 | key twice | ``rule key `{key}` appears more than once`` |
| E-4 | no `=` after the key run (or the run is empty) | ``` `{token}` is not `key=value` ``` — `token` = the maximal non-`ws` run at the pair position |
| E-5 | `key=` then `ws`/`#`/end | ``` `{key}=` needs a value; write `{key}=""` for an empty one ``` |
| E-6 | bare value holds `"`, `=`, `\` or a control | ``the value of `{key}` must be quoted: it holds `"`, `=`, `\` or a control character`` |
| E-7 | end of line inside quotes | `a quoted string is not closed` |
| E-8 | `\` + other char inside quotes | ``` `\{c}` is not an escape; use \", \\, \n, \t or \r ``` |
| E-9 | after a value, neither `ws`, `#` nor end; or a pair with no `ws` before it | ``expected a space, `#` or the end of the line after `{key}`'s value`` (after the answer: ``…after `{answer}` ``) |
| E-10 | non-`ws` after a quoted reason | `nothing may follow a quoted reason` |
| E-11 | `kind` not in `TOOL_KINDS` | `rule_kind_unknown(kind)` — the store's sentence |

**Formatter — shortest form.** Per rule, `answer` (`reject_once` / `reject_always`), then for each
`Some` matcher field in `RULE_KEYS` order ` key=value`, then the reason; rules joined with `\n`.

- A value is **bare** iff it is non-empty and holds no `ws`, `"`, `#`, `=`, `\` or control char;
  otherwise it is **quoted**, escaping per the table.
- A reason is omitted when empty. It is written **plain** (` # ` + reason) iff
  `reason == reason.trim()`, it does not start with `"`, and it holds no control char; otherwise
  ` # ` + quoted(reason).
- Layout is fixed (one space between tokens, ` # ` before a reason); "shortest" governs quoting and
  escaping only. `format_rules(&[])` is `""`.

**Fixed round-trip table** (`rule_lines_round_trip_the_fixed_table`; each row:
`format_rules(&[rule]) == line` **and** `parse_rules(line) == Ok(vec![rule])`; Rust escapes in the
"rule" column, the "line" column is the text):

| # | Rule (answer · matcher · reason) | Line |
|---|---|---|
| 1 | once · kind=`execute`, command=`rm -rf` · `never wipe` | `reject_once kind=execute command="rm -rf" # never wipe` |
| 2 | always · path=`/etc` · `` | `reject_always path=/etc` |
| 3 | once · name=`` · `` | `reject_once name=""` |
| 4 | once · command=`say "hi"` · `` | `reject_once command="say \"hi\""` |
| 5 | once · path=`C:\tmp` · `` | `reject_once path="C:\\tmp"` |
| 6 | once · command=`a#b` · `` | `reject_once command="a#b"` |
| 7 | once · command=`x=1` · `` | `reject_once command="x=1"` |
| 8 | once · command=`"l1\nl2\tx\r"` · `` | `reject_once command="l1\nl2\tx\r"` |
| 9 | once · name=`café→` · `` | `reject_once name=café→` |
| 10 | once · kind=`read` · `"  padded  "` | `reject_once kind=read # "  padded  "` |
| 11 | once · kind=`read` · `"\"quoted\" first"` | `reject_once kind=read # "\"quoted\" first"` |
| 12 | once · kind=`read` · `"two\nlines"` | `reject_once kind=read # "two\nlines"` |
| 13 | once · kind=`read` · `has # inside` | `reject_once kind=read # has # inside` |
| 14 | always · kind=`fetch`, name=`WebFetch`, path=`/`, command=`curl` · `no network` | `reject_always kind=fetch name=WebFetch path=/ command=curl # no network` |
| 15 | once · command=`"\u{1b}[0m"` · `` | `reject_once command="␛[0m"` (raw ESC inside quotes, B-2) |
| 16 | once · name=`"a\u{a0}b"` (NBSP) · `` | `reject_once name="a b"` (raw NBSP inside quotes) |
| 17 | once · kind=`switch_mode` · `#tag` | `reject_once kind=switch_mode # #tag` |

Two-rule row: rules 1 and 2 together format to line 1 + `\n` + line 2.

**Parse-only table** (`rule_lines_parse_spacing_comments_and_blank_lines`): `"\n  # header\n\n"`
→ `[]`; `"  reject_once   kind=read   "` → kind `read`, reason `""`; `"reject_once command=\"rm\"#why"`
→ reason `why`; `"reject_once kind=read #   "` → reason `""`; `"reject_once kind=read\r\nreject_always path=/x\r"`
→ two rules; `"reject_once path=/a#b"` → path `/a`, reason `b`; `"reject_once"` → one rule with
`PersonaMatch::default()` (refused later by the store, not here); `"reject_once command=x kind=read"`
→ both keys (order free).

**Generated cases** (`rule_lines_round_trip_generated_values`, deterministic, no new
dependency): fragments `["a", "Z9", " ", "\"", "\\", "#", "=", "\n", "\t", "\r", "é", "→",
"\u{1b}", "\u{a0}", "/"]`; values = every concatenation of 1 to 3 fragments (3 615) plus `""`. For
value `v` at index `i`: rule A `{name: v}` once, reason `""`; rule B `{kind: TOOL_KINDS[i % 10],
path: v}` always, reason `v`; rule C `{command: v}` once, reason `"x"`. Assert
`parse_rules(&format_rules(&[r])) == Ok(vec![r])` and `format_rules(&[r]).lines().count() == 1`
for each, and once for the whole list together.

### 2.4 Import mode of the parser (T6, D19, OQ-7) — `htui-core/src/model/persona.rs`

`parse_file` (`:513-617`) is split with no behaviour change:

```rust
/// The reader half of [`parse_file`]: every D8 key rule, no D3 save rule.
fn read_file(text: &str) -> Result<PersonaFile, PersonaFileError>;   // today's :517-607
/// The save-rule half: `persona_refusal` as `PersonaFileError::Refused`.
fn checked(file: PersonaFile) -> Result<PersonaFile, PersonaFileError>; // today's :609-616

pub fn parse_file(text: &str) -> Result<PersonaFile, PersonaFileError> {
    read_file(text).and_then(checked)
}
```

New variant, last in `PersonaFileError`:

```rust
    /// MOD-26 M2 OQ-7's exception, import only: a non-empty `tools` whose every entry is an MCP
    /// tool would import as an empty `allow`, which keeps every built-in tool.
    #[error(
        "every `tools` entry is an MCP tool; htui's `allow` keeps built-in tools only, so this \
         file would keep all of them \u{2014} write `disallowed-tools` or `deny-kinds` instead"
    )]
    OnlyMcpTools,
```

New type and function:

```rust
/// A persona file as the import reads it (MOD-26 M2 D19, OQ-7): the file, which has passed the
/// save rules, and the `tools` entries dropped because they name MCP tools, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The file, `allow` without its `mcp__` entries.
    pub file: PersonaFile,
    /// The dropped entries, as written.
    pub dropped: Vec<String>,
}

/// Reads one file for the Settings › Personas import (MOD-26 M2 D19): [`parse_file`]'s reader,
/// then every `tools` entry starting with [`MCP_PREFIX`] is moved to `dropped` (`--tools` never
/// filtered MCP tools, so the file meant nothing htui can enforce through it), then a `tools`
/// that was non-empty and is now empty is [`PersonaFileError::OnlyMcpTools`], then the save
/// rules. The seed path keeps [`parse_file`], which still refuses `mcp__` in `tools`.
///
/// # Errors
/// Every [`PersonaFileError`] variant.
pub fn parse_import(text: &str) -> Result<Imported, PersonaFileError>;
```

Order inside `parse_import`: `read_file` → partition `allow` (stable, `starts_with(MCP_PREFIX)`)
→ `OnlyMcpTools` iff the original `allow` was non-empty and the kept one is empty → `checked`.

`list_of` (`:637-645`) becomes `pub` (B-3), doc: "A one-line list value (plan D8), the grammar of a
persona file **and** of the Settings › Personas form (MOD-26 M2 B-3): comma-separated, each item
trimmed, empty items dropped."

### 2.5 `delete_persona` on the seam (T7, D14) — `htui-core/src/store/traits.rs`

The comment `:1003-1009` becomes:

```rust
    // persona (MOD-26 milestone 1, plan D4; milestone 2, D14)
    //
    // A global registry like `skill`, read and written here for the skill block's reason
    // (above). `step_graph_phase.persona_id` is `ON DELETE RESTRICT`, and `delete_persona` refuses a
    // bound persona first with [`persona_is_bound`]'s sentence, so the constraint is never what the
    // caller reads. The refusal sentences are `model::persona`'s pure helpers, re-exported below
    // (plan D3), so both stores word them once.
```

Method, after `update_persona` (`:1032-1038`):

```rust
    /// Deletes a persona no phase binds (MOD-26 milestone 2 D14, I-9). No compare-and-set token,
    /// as [`delete_item_kind`](WriteStore::delete_item_kind). Order: `NotFound { entity:
    /// "persona" }` for an unknown id; then [`persona_is_bound`]'s sentence while any
    /// `step_graph_phase` names it, override graphs included; then the delete. A started run never
    /// needs the row: it reads its snapshot (M1 I-3).
    ///
    /// # Errors
    /// As above. A refusal writes nothing.
    async fn delete_persona(&self, id: PersonaId) -> Result<()>;
```

Helper, after `item_kind_is_held` (`:2160-2167`):

```rust
/// How many phases [`persona_is_bound`] names before it counts the rest.
const BOUND_PHASES_NAMED: usize = 5;

/// MOD-26 milestone 2 D14 (I-9): `delete_persona`'s refusal while phases bind the persona. Each
/// holder is `(project slug, graph name, phase name)`, named `` `<slug>/<graph>/<phase>` ``: a
/// persona is global and a graph name is unique only per project (`0001_init.sql:221`). Sorted by
/// the triple's bytes and deduplicated **here**, so both stores word it the same whatever order
/// they read in; at most five, then "and n more".
#[must_use]
pub fn persona_is_bound(name: &str, holders: &[(String, String, String)]) -> String {
    let mut holders: Vec<&(String, String, String)> = holders.iter().collect();
    holders.sort_unstable();
    holders.dedup();
    let named: Vec<String> = holders
        .iter()
        .take(BOUND_PHASES_NAMED)
        .map(|(project, graph, phase)| {
            format!("`{}`", format!("{project}/{graph}/{phase}").escape_debug())
        })
        .collect();
    let more = holders.len().saturating_sub(BOUND_PHASES_NAMED);
    let more = if more == 0 { String::new() } else { format!(" and {more} more") };
    let (noun, them) = if holders.len() == 1 { ("phase", "it") } else { ("phases", "them") };
    format!(
        "persona `{}` is bound to {} {noun} ({}{more}); clear {them} in Settings \u{203a} Kinds \
         first",
        name.escape_debug(),
        holders.len(),
        named.join(", ")
    )
}
```

Exact renderings (pinned by tests):

- 1: ``persona `reviewer` is bound to 1 phase (`htui/feature/review`); clear it in Settings › Kinds first``
- 3: ``persona `reviewer` is bound to 3 phases (`agy/feature/review`, `htui/HTUI-3-override/review`, `htui/feature/review`); clear them in Settings › Kinds first``
- 7: ``persona `architect` is bound to 7 phases (`htui/analysis/research`, `htui/analysis/verdict`, `htui/bug/reproduce`, `htui/feature/implement`, `htui/feature/plan` and 2 more); clear them in Settings › Kinds first``

Re-export: `store/mod.rs` `traits::{…}` gains `persona_is_bound` (beside `item_kind_is_held`);
`mem.rs:~60` and `pg/write.rs:47-60` import it.

Implementors (five; no default bodies):

| Impl | Body |
|---|---|
| `MemStore` (`mem.rs:6494` block, beside `delete_item_kind` `:6743`) | `self.write(\|state\| state.delete_persona(id))` |
| `PgStore` (`pg/write.rs`, after `update_persona` `:3441-3496`) | §2.7 |
| `Writer` (`htui-store/src/writer.rs`, after `update_persona` `:809-820`) | `match self { Self::Memory(store) => store.delete_persona(id).await, Self::Online(pg) => pg.delete_persona(id).await }` |
| `UsageSpy` (`htui-agent/src/conformance.rs`, after `:1044-1051`) | `async fn delete_persona(&self, id: htui_core::model::PersonaId) -> StoreResult<()> { self.inner.delete_persona(id).await }` |
| `SpyStore` (`htui-agent/tests/recorder.rs`, after `:771-778`) | same |

### 2.6 Migration `htui-store/migrations/0013_persona_phase_index.sql` (T7, D15), in full

```sql
-- 0013_persona_phase_index.sql - MOD-26 milestone 2 (plan D14, D15; milestone 1 review N5).
-- Forward-only (R-STO-5).
--
-- step_graph_phase.persona_id gains an index. delete_persona refuses a persona still bound to a
-- phase by finding its phases by persona_id, and fk_step_graph_phase_persona's ON DELETE RESTRICT
-- check makes the same lookup; without the index both scan every phase. Index only: no table,
-- column or constraint moves, and the SQLite mirror's schema is untouched. schema_version still
-- becomes 13, so each box rebuilds its mirror once on first start (0008's R-56). A headless
-- worker never migrates: migrate from a TUI first.

CREATE INDEX idx_step_graph_phase_persona ON step_graph_phase(persona_id);   -- FK check on persona delete
```

`0012_persona.sql` is not edited (applied checksum). No cache migration, no column comment.

### 2.7 PgStore `delete_persona` (T7) — `htui-store/src/pg/write.rs`

```rust
    /// Locks the persona, then deletes it under a guard the lock makes authoritative (MOD-26 M2
    /// D14), `delete_item_kind`'s shape (review L2's reasoning, above).
    ///
    /// A bind (`create_phase`/`update_phase` naming the persona) takes `FOR KEY SHARE` on the
    /// persona row through `fk_step_graph_phase_persona`, which `FOR UPDATE` conflicts with: a bind
    /// in flight is waited for and then **seen** by the guarded `DELETE`'s snapshot (zero rows, the
    /// sentence); a bind that arrives after the lock waits behind it and gets `23503`, which
    /// [`phase_persona_refused`] words as `references_no_row`. Without the lock the deleter itself
    /// would see a raw `23503` (probed, plan "Verified claims").
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id; [`StoreError::Constraint`] with
    /// [`persona_is_bound`]'s sentence while a phase binds it.
    async fn delete_persona(&self, id: PersonaId) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let Some(name) = sqlx::query_scalar!(
            "SELECT name FROM persona WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        else {
            return Err(StoreError::NotFound {
                entity: "persona",
                id: id.to_string(),
            });
        };

        let removed = sqlx::query_scalar!(
            r#"
            DELETE FROM persona
             WHERE id = $1
               AND NOT EXISTS (SELECT 1 FROM step_graph_phase WHERE persona_id = $1)
            RETURNING id
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        if removed.is_none() {
            // The row is there - it was just locked - so zero rows means the guard fired; the
            // holders are read only to name them in the sentence.
            let holders = sqlx::query!(
                r#"
                SELECT pr.slug AS "project!", g.name AS "graph!", p.name AS "phase!"
                  FROM step_graph_phase p
                  JOIN step_graph g ON g.id = p.graph_id
                  JOIN project pr ON pr.id = g.project_id
                 WHERE p.persona_id = $1
                "#,
                id.as_uuid(),
            )
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .into_iter()
            .map(|row| (row.project, row.graph, row.phase))
            .collect::<Vec<_>>();
            return Err(StoreError::Constraint(persona_is_bound(&name, &holders)));
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(())
    }
```

As built, the guarded `DELETE` and the holders read sit in a loop that re-runs the guard when the
holders read comes back empty (D-11).

Three new statements → three new `.sqlx` files (318 → **321**; restate from
`ls crates/htui-store/.sqlx | wc -l`). No existing query text changes (auto-memory
`sqlx-offline-hash-is-literal-query`).

### 2.8 MemStore (T7) — `htui-core/src/store/mem.rs`

`State::delete_persona`, after `State::update_persona` (`:3270-3318`):

```rust
    /// MOD-26 M2 D14: `NotFound("persona")` → [`persona_is_bound`] naming every phase that binds
    /// it (graph → project; a missing parent is named `?`, unreachable, B-5) → remove.
    fn delete_persona(&mut self, id: PersonaId) -> Result<()> {
        let persona = self.personas.get(&id).ok_or_else(|| StoreError::NotFound {
            entity: "persona",
            id: id.to_string(),
        })?;
        if self.phases.iter().any(|phase| phase.persona_id == Some(id)) {
            let holders: Vec<(String, String, String)> = self
                .phases
                .iter()
                .filter(|phase| phase.persona_id == Some(id))
                .map(|phase| {
                    let graph = self.graphs.get(&phase.graph_id);
                    let project = graph.and_then(|graph| self.projects.get(&graph.project_id));
                    (
                        project.map_or_else(|| "?".to_owned(), |row| row.slug.clone()),
                        graph.map_or_else(|| "?".to_owned(), |row| row.name.clone()),
                        phase.name.clone(),
                    )
                })
                .collect();
            return Err(StoreError::Constraint(persona_is_bound(&persona.name, &holders)));
        }
        self.personas.remove(&id);
        Ok(())
    }
```

### 2.9 Catalogue persona list and the Kinds field (T8, D23)

`htui/src/catalogue.rs`:

```rust
/// One persona as `Settings > Kinds` names it (MOD-26 M2 D23): id and name, nothing the phase
/// editor does not print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaSummary {
    /// `persona.id`.
    pub id: PersonaId,
    /// `persona.name`.
    pub name: String,
}

pub struct CatalogueSnapshot {
    /// The scope's projects, in scope order.
    pub projects: Vec<ProjectCatalogue>,
    /// The global persona registry by name (MOD-26 M2 D23): the phase editor's `persona` field
    /// resolves against it and a bound phase's line names its persona from it.
    pub personas: Vec<PersonaSummary>,
}
```

`snapshot()` (`:83-108`), after the projects loop:

```rust
    let personas = store
        .personas()
        .await?
        .into_iter()
        .map(|row| PersonaSummary { id: row.id, name: row.name })
        .collect();
    Ok(CatalogueSnapshot { projects, personas })
```

`htui/src/ui/tabs/settings/kinds.rs` (exact changes):

| Item | Change |
|---|---|
| imports | `PersonaId` from `htui_core::model`; `PersonaSummary` from `crate::catalogue` |
| `Stored` (`:169-175`) | `+ persona: Option<String>` — the phase editor's `persona` text at open |
| `Editor` (`:191-206`) | `+ stored_persona: Option<String>` ("never refreshed: B-8, `stored_budget`'s rule"); `Debug` unchanged (labels only) |
| `open_with` (`:500-521`) | destructures and stores `persona` |
| `:578-584` (kind edit) | `Stored { prefix, budget: None, persona: None }` |
| `:598-609` (phase edit) | `let persona = persona_text(phase.persona_id, self.personas());` `edit_phase_fields(phase, self.personas())`, `Stored { prefix: None, budget: Some(budget), persona: Some(persona) }` |
| `edit_phase_fields` (`:1682-1691`) | `fn edit_phase_fields(phase: &StepGraphPhase, personas: &[PersonaSummary]) -> Vec<Field>`; seventh field `Field::optional("persona", &persona_text(phase.persona_id, personas))`; doc "PRD D2's six editable columns plus MOD-26 D23's persona, in editor order" |
| `build_phase_edit` (`:948-1005`) | after `let budget = budget(&editor.text(5))?;`: `let persona = persona_patch(&editor.text(6), editor.stored_persona.as_deref().unwrap_or_default(), self.personas())?;`; `patch_changed` gains `\|\| persona.is_some()`; `PhasePatch { …, persona }` |
| `phase_line` (`:1842-1856`) | `fn phase_line(phase: &StepGraphPhase, personas: &[PersonaSummary]) -> String` — today's text, then `if let Some(id) = phase.persona_id { line.push_str(" \u{b7} persona "); line.push_str(&persona_label(id, personas)); }` |
| `:1202` | `phase_line(phase, self.personas())` |
| new `fn personas(&self) -> &[PersonaSummary]` | `self.snapshot.as_ref().map_or(&[], \|snapshot\| snapshot.personas.as_slice())` |
| test literal `:1902-1910` | `+ stored_persona: None` |

New private helpers:

```rust
/// What the `persona` field says when the registry is empty (MOD-26 M2 D23).
const NO_PERSONAS: &str = "no personas exist; add one in Settings \u{203a} Personas";

/// A bound persona's name, or its id when the loaded list does not know it (a persona created
/// after the catalogue was read; D23).
fn persona_label(id: PersonaId, personas: &[PersonaSummary]) -> String {
    personas
        .iter()
        .find(|row| row.id == id)
        .map_or_else(|| id.to_string(), |row| row.name.clone())
}

/// The `persona` field's text at open: blank for none (D23).
fn persona_text(bound: Option<PersonaId>, personas: &[PersonaSummary]) -> String {
    bound.map_or_else(String::new, |id| persona_label(id, personas))
}

/// The `persona` field as `PhasePatch.persona` (MOD-26 M2 D23, OQ-11; B-8): `None` while the
/// trimmed text is still the trimmed text the editor opened with — an id the list does not know
/// included, so it is left alone unless edited; `Some(None)` for a blank field; `Some(Some(id))`
/// for a name the list holds.
///
/// # Errors
/// [`no_persona_named`] for any other text.
fn persona_patch(
    text: &str,
    opened: &str,
    personas: &[PersonaSummary],
) -> Result<Option<Option<PersonaId>>, String> {
    let text = text.trim();
    if text == opened.trim() {
        return Ok(None);
    }
    if text.is_empty() {
        return Ok(Some(None));
    }
    personas
        .iter()
        .find(|row| row.name == text)
        .map(|row| Some(Some(row.id)))
        .ok_or_else(|| no_persona_named(text, personas))
}

/// The refusal of a name the loaded list does not hold, before anything is sent (D23).
fn no_persona_named(name: &str, personas: &[PersonaSummary]) -> String {
    if personas.is_empty() {
        return NO_PERSONAS.to_owned();
    }
    let known: Vec<&str> = personas.iter().map(|row| row.name.as_str()).collect();
    format!("no persona named `{}`; known: {}", name.escape_debug(), known.join(", "))
}
```

Rendered: ``no persona named `ghost`; known: architect, reviewer``. The `persona` label is shorter
than `gate_hard (y/n)` (15), so no label column moves. `CreatePhase` is unchanged.

### 2.10 Requests and replies (T9, D21) — `htui/src/store_worker.rs`

Imports: `use htui_core::model::{NewPersona, Persona, PersonaId, PersonaPatch};` (merged into the
existing `htui_core::model` list), `use crate::persona_import::PersonaImports;`,
`use crate::persona_settings::{self, PersonaWrite};`.

`StoreRequest`, five variants inserted **after `SetToolPaths`** (`:315-…`), next to the agent
registry block (R-8: an additive hunk away from the enum's tail, where MOD-13/69/72 append):

```rust
    /// The persona registry by name (MOD-26 M2 D21). Served by [`persona_settings::serve`]
    /// through the writer: personas are not mirrored, so offline it is refused with
    /// `DATABASE_UNREACHABLE`. Answered with [`StoreReply::Personas`].
    Personas,
    /// Create one persona from the Settings form (D21); the section mints the id (B-13).
    /// Answered with [`StoreReply::PersonaWritten`] (`Created`), or with [`StoreReply::Failed`]
    /// carrying the store's sentence byte for byte (I-8, B-11).
    CreatePersona {
        /// The row to insert; checked again by the store.
        new: NewPersona,
    },
    /// Edit one persona under compare-and-set on `updated_at` (D21): only the fields the editor
    /// changed are `Some`. Answered with [`StoreReply::PersonaWritten`] (`Updated`, `Stale` or
    /// `Gone`), or `Failed` with the store's sentence.
    UpdatePersona {
        /// The row.
        id: PersonaId,
        /// `updated_at` as a registry reply answered it, never a built one (MOD-40 F-17).
        expected: DateTime<Utc>,
        /// The changed fields.
        patch: PersonaPatch,
    },
    /// Delete one persona no phase binds (D14, D21). Answered with
    /// [`StoreReply::PersonaWritten`] (`Deleted` or `Gone`), or `Failed` carrying
    /// `persona_is_bound`'s sentence.
    DeletePersona {
        /// The row.
        id: PersonaId,
    },
    /// Import one frontmatter `.md` file or the depth-0 `*.md` of a directory (D20, OQ-9). The
    /// **worker** reads the filesystem (`R-NF-3`, I-11). Answered with
    /// [`StoreReply::PersonaImports`].
    ImportPersonas {
        /// The path exactly as typed (one line, never split).
        path: String,
    },
```

`name()` (`:931`), after the `SetToolPaths` arm:

```rust
            // The five of `persona_settings::REQUEST_NAMES`, in that order (MOD-26 M2 D21).
            Self::Personas => "personas",
            Self::CreatePersona { .. } => "create_persona",
            Self::UpdatePersona { .. } => "update_persona",
            Self::DeletePersona { .. } => "delete_persona",
            Self::ImportPersonas { .. } => "import_personas",
```

`StoreReply`, three variants after `AgentWritten` (`:1297-1302`):

```rust
    /// The persona registry by name: the answer to [`StoreRequest::Personas`] (MOD-26 M2 D21). A
    /// read answer only: it never closes an editor or moves its token.
    Personas(Vec<Persona>),
    /// The answer to every persona write (D21; self-naming, MOD-59): the registry re-read after
    /// the write, and what the write did.
    PersonaWritten {
        /// The registry as it is now, by name, whatever the outcome.
        personas: Vec<Persona>,
        /// What the write did.
        outcome: PersonaWrite,
    },
    /// The registry after an import, and what happened to every file (D20). Boxed: the report
    /// can be long.
    PersonaImports(Box<PersonaImports>),
```

`try_serve` (`:1571`), after the agent registry arm (`:1687-1692`):

```rust
        // The five persona requests, or-ed for the reason the arms above are: a guard does not
        // count towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12, MOD-26 M2
        // D21). Served here, in the loop: the import's file reads included (`R-NF-3`, I-11).
        StoreRequest::Personas
        | StoreRequest::CreatePersona { .. }
        | StoreRequest::UpdatePersona { .. }
        | StoreRequest::DeletePersona { .. }
        | StoreRequest::ImportPersonas { .. } => persona_settings::serve(backend, request).await?,
```

Counts: `StoreRequest` 99 → 104, `StoreReply` 58 → 61 (re-count at T10).

### 2.11 `htui/src/persona_settings.rs` (new, T9)

Module doc: "The persona registry editor of `Settings > Personas` (MOD-26 milestone 2, D20-D22):
[`serve`], the five requests served in the store loop, and the write outcome. Nothing here reads a
clock or mints an id: the store stamps every instant and the section mints a new row's id (B-13).
Every refusal is the store's own sentence (I-8), sent bare (B-11)."

```rust
use htui_core::model::PersonaId;
use htui_core::store::{CasOutcome, Result as StoreResult, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE};

use crate::persona_import::{self, PersonaImports};
use crate::store_worker::{StoreReply, StoreRequest};

/// The five request names, in [`StoreRequest`] order (MOD-26 M2 D21). The section's `Failed`
/// match reads from here; `StoreRequest::name`'s arms spell the same five and
/// `request_names_match_the_name_arms` pins them together.
pub const REQUEST_NAMES: [&str; 5] = [
    "personas",
    "create_persona",
    "update_persona",
    "delete_persona",
    "import_personas",
];

/// The read's name: a refused read leaves the section with no list.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// The import's name: what `busy` holds while the worker walks.
pub const IMPORT_NAME: &str = REQUEST_NAMES[4];

/// What one persona write did (MOD-26 M2 D21, B-10), carried by `StoreReply::PersonaWritten`
/// beside the re-read. Ids and names only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonaWrite {
    /// A new row landed.
    Created {
        /// The row.
        id: PersonaId,
        /// Its name, as stored.
        name: String,
    },
    /// The edit applied.
    Updated {
        /// The row.
        id: PersonaId,
        /// Its name after the edit (a rename is allowed).
        name: String,
    },
    /// The row is gone; no phase held it.
    Deleted {
        /// The row the request named.
        id: PersonaId,
    },
    /// The token was spent: nothing was written; the re-read holds the row as it is now.
    Stale {
        /// The row.
        id: PersonaId,
    },
    /// The row was already gone (`NotFound` on an update or a delete). Nothing was written.
    Gone {
        /// The row the request named.
        id: PersonaId,
    },
}

/// Serves the five persona requests (MOD-26 M2 D21) in the store loop.
///
/// Offline there is no writer, and all five are `Err(Unreachable(DATABASE_UNREACHABLE))` before
/// any read (`R-STO-4`; the import before any file is read). A store `Constraint` from a write is
/// `Ok(Failed)` carrying its sentence byte for byte (B-11); every other error propagates, so an
/// `Unreachable` still drops an `Online` backend onto the mirror. Every write that reached the
/// store answers `PersonaWritten` with the registry re-read.
///
/// # Errors
/// Whatever the store reports, `Unreachable` offline, and `Backend` for a request that is not one
/// of the five.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> StoreResult<StoreReply>;
```

Arms (exact semantics):

| Request | Store call | Outcome |
|---|---|---|
| `Personas` | `writer.personas()` | `Ok(StoreReply::Personas(rows))` |
| `CreatePersona { new }` | `writer.create_persona(new.clone())` | `Ok(row)` → `Created { id: row.id, name: row.name }`; `Err(Constraint(s))` → `refused(request, s)`; other `Err` → `Err` |
| `UpdatePersona { id, expected, patch }` | `writer.update_persona(*id, *expected, patch.clone())` | `Applied(row)` → `Updated { id, name: row.name }`; `Stale(_)` → `Stale { id }`; `Err(NotFound { entity: "persona", .. })` → `Gone { id }`; `Constraint(s)` → `refused`; other → `Err` |
| `DeletePersona { id }` | `writer.delete_persona(*id)` | `Ok(())` → `Deleted { id }`; `NotFound("persona")` → `Gone { id }`; `Constraint(s)` → `refused` (the bound sentence); other → `Err` |
| `ImportPersonas { path }` | `persona_import::import(backend, path)` then `writer.personas()` | `Ok(StoreReply::PersonaImports(Box::new(PersonaImports { personas, report })))` |
| any other | — | `Err(StoreError::Backend(format!("not a persona request: {}", other.name())))` |

After a write outcome: `let personas = writer.personas().await?; Ok(StoreReply::PersonaWritten {
personas, outcome })` (agent_settings' residue: a failed re-read after an applied write answers
`Failed`).

```rust
/// A store refusal as the section shows it (I-8, B-11): `Failed` carrying the sentence itself,
/// never `constraint violated: …`.
fn refused(request: &StoreRequest, sentence: String) -> StoreReply {
    StoreReply::Failed {
        request: request.name(),
        message: sentence,
    }
}
```

### 2.12 `htui/src/persona_import.rs` (new, T9, D20, OQ-8, OQ-9)

Module doc: why the worker reads the files (`R-NF-3`, I-11), the row is the truth and no path is
stored (I-10, `R-ID-3`), no LLM (`R-ID-6`), the sweep is new logic (not the skill walk: depth 0,
`*.md` only, fence-less files skipped — the skill import's R-37 refuses exactly that sweep for
skills; for personas a directory **is** `.claude/agents/`), known names are refused (OQ-8), and the
sentence helpers are copied from `skill_import.rs` rather than widened (it stays untouched).

```rust
/// The largest file read as a persona. A file over this is reported as skipped, never truncated.
pub const MAX_BYTES: u64 = 256 * 1024;

/// The most `*.md` files one directory contributes; past it the rest are one reported row.
pub const MAX_FILES: usize = 64;

/// Why a fence-less file is left alone (OQ-9): a `README.md` beside the agents.
pub const NO_FRONTMATTER: &str = "no frontmatter: the file does not open with a `---` fence";

/// What happened to one file. No variant carries file content: a name, a path, a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonaOutcome {
    /// A new persona row.
    Imported {
        /// `persona.name`, as stored.
        name: String,
        /// The file it came from.
        path: String,
        /// The `tools` entries dropped as MCP tools (OQ-7), in file order; usually empty.
        dropped: Vec<String>,
    },
    /// Not imported, and the one sentence why: the reader's, the store's, OQ-8's or the batch's.
    Refused {
        /// The file, or the path the maintainer typed.
        path: String,
        /// One sentence.
        message: String,
    },
    /// Left alone: no frontmatter, over the byte cap, not UTF-8, past the file cap.
    Skipped {
        /// The file, or the directory.
        path: String,
        /// One sentence.
        reason: String,
    },
}

/// The import's answer: the registry after the batch, and one row per file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaImports {
    /// A fresh `personas()` read, by name.
    pub personas: Vec<Persona>,
    /// One row per file, in the order the walk met them.
    pub report: Vec<PersonaOutcome>,
}

/// Imports the file or directory at `path` (D20) and reports on every file.
///
/// # Errors
/// [`StoreError::Unreachable`] with [`DATABASE_UNREACHABLE`] offline (before any file is read),
/// or the one `personas()` read's error. A path that does not exist is a reported `Refused`.
pub async fn import(backend: &Backend, path: &str) -> Result<Vec<PersonaOutcome>>;
```

Batch state: `writer: &Writer`, `known: Vec<String>` (the names of **one** `personas()` read,
before any write), `written: Vec<(String, String)>` (`(name, path)` of every row this batch
created).

Walk (`one_path`):

1. `std::fs::metadata(path)` fails → `[Refused { path, message: format!("could not read
   `{path}` ({error})") }]`.
2. A file → `[write_one(path)]`, whatever its extension (B-14).
3. Neither file nor directory → ``Refused { "`{path}` is neither a file nor a directory" }``.
4. A directory → `std::fs::read_dir`, entries whose `Path::extension() == Some("md")` and whose
   `std::fs::metadata` (follows links) `is_file()`; sorted by `file_name().as_encoded_bytes()`;
   every other entry (subdirectory, non-`.md`, broken link) left alone and **not reported**
   (B-14). More than `MAX_FILES` → the first 64, then `Skipped { path: <dir>, reason:
   format!("{n} more file(s) past the {MAX_FILES}-file cap") }` **after** the 64 rows.

`write_one(path, label)`, first match wins:

| Step | Outcome |
|---|---|
| `read_text` (copied from `skill_import.rs:499-514`): size > `MAX_BYTES` | `Skipped` ``"the file is over the 262144-byte cap; nothing was imported"`` |
| unreadable / not UTF-8 | `Skipped` ``"could not read this file ({error}); nothing was imported"`` / ``"the file is not UTF-8; nothing was imported"`` |
| `parse_import` → `Err(PersonaFileError::Fence(FrontmatterError::NoFence))` | `Skipped { reason: NO_FRONTMATTER }` |
| `parse_import` → any other `Err(e)` | `Refused { message: e.to_string() }` (`Unterminated`, `Model`, `OnlyMcpTools`, `Refused(sentence)`, …) |
| name in `written` | `Refused` ``"`{name}` was already imported from `{first}` by this import; this file was not written"`` |
| name in `known` (OQ-8) | `Refused` ``"persona `{name}` exists; edit it in Settings › Personas, or delete it and import again"`` |
| `create_persona(file.into_new(PersonaId::new()))` `Ok(row)` | `Imported { name: row.name, path, dropped }`; push to `written` |
| `Err(Constraint(s))` | `Refused { message: s }` |
| `Err(other)` | `Refused { message: other.to_string() }` |

The `{name}` in the two refusals is `escape_debug`'d.

### 2.13 `PersonasSection` (T9, D22) — `htui/src/ui/tabs/settings/personas.rs` (new)

Identity: `pub const ID: SectionId = SectionId("personas")`, `title()` `"Personas"` (10 strip
columns; 61 → 71 of 100).

```rust
/// The persona registry, with the keys that edit it (MOD-26 M2 D22).
#[derive(Debug, Default)]
pub struct PersonasSection {
    /// The registry by name, as the last `Personas`/`PersonaWritten`/`PersonaImports` answered.
    personas: Vec<Persona>,
    /// `Some(message)` after `Failed { request: "personas" }`.
    unavailable: Option<String>,
    /// Index into `personas`; no wrap (a held `j` must not aim `d` at a row nobody looked at).
    cursor: usize,
    /// What the section is doing.
    mode: Mode,
    /// The write in flight, by request name; one at a time.
    busy: Option<&'static str>,
    /// The last outcome, drawn in `theme.error` when `Notice::Error`.
    notice: Option<Notice>,
}

#[derive(Debug, Default)]
enum Mode {
    /// The list and the cursor; captures nothing.
    #[default]
    Browse,
    /// `n` or `e`: the seven one-line fields.
    Editing(Editor),
    /// `b`, or `n`'s second step: the body `TextArea`.
    Body(BodyEditor),
    /// `r`: the rules `TextArea`, one rule per line (D17).
    Rules(RulesEditor),
    /// `d`: the y/n question, then the delete in flight.
    Deleting { id: PersonaId, name: String, stage: DeleteStage },   // DeleteStage::{Asking, InFlight}
    /// `I`: the path field.
    ImportPath { field: TextField },
    /// An import's per-file report, scrolled by line.
    Report { report: Vec<PersonaOutcome>, top: usize },
}

/// The fields form. `Debug` hand-written: labels and focus only, never typed text (H-7).
struct Editor { target: Target, fields: Vec<Field>, focus: usize }
enum Target {
    /// `n`: a new row; `body` keeps the body editor's text across `Esc` back to the fields.
    Create { body: String },
    /// `e`: one row under CAS; `opened` is the row the form prefilled from (rebased on `Stale`).
    Edit { id: PersonaId, expected: DateTime<Utc>, opened: Persona },
}
/// `TextArea` redacts its own `Debug`.
#[derive(Debug)]
struct BodyEditor {
    target: BodyTarget,            // Create { form: Editor } | Edit { id, name, expected }
    area: TextArea,
    original: String,              // the text it opened on (the "unchanged" and warn-once baseline)
    esc_armed: bool,
    sent: Option<String>,          // the body in flight, for the skills rule on `Updated`
}
#[derive(Debug)]
struct RulesEditor {
    id: PersonaId, name: String, expected: DateTime<Utc>,
    default: Option<PersonaDefault>, // carried into the permission patch; retaken on `Stale`
    area: TextArea, original: String, esc_armed: bool, sent: Option<String>,
}
enum Notice { Info(String), Error(String) }   // kinds.rs' shape, private copy
```

`FIELD_LABELS: [&str; 7] = ["name", "description", "tools", "disallowed-tools", "deny-kinds",
"command-run (y/n)", "permission-default"]` (the frontmatter spellings, D22).

Prefill from a row (`prefill(&Persona) -> [String; 7]`): `name`; `description`;
`allow.join(", ")`; `deny.join(", ")`; `deny_kinds.join(", ")`; `"y"`/`"n"`; `""`/`"ask"`/`"deny"`.
Create starts from `["", "", "", "", "", "y", ""]`.

**Form parse → draft** (`fn draft(&self) -> Result<Draft, Refusal>`, `Refusal { field: &'static
str, reason: String }`), in field order, first refusal wins and **focuses its field**:

1. `name` = text `.trim()`; `description` = text as typed; `tools`/`disallowed-tools`/`deny-kinds`
   = `persona::list_of(text)` (B-3).
2. `command-run (y/n)` = `super::yes_or_no(text)`, else ``"`command-run (y/n)` is y or n"``.
3. `permission-default` = trimmed, lowercased: `""` → `None`, `ask`, `deny`, else
   ``"`permission-default` is blank, ask or deny"``.
4. The store's rules, field by field, through `persona_patch_refusal` on a **one-field** patch
   (I-8: the store's own sentences): `name` → `PersonaPatch { name }`; `description` →
   `{ description }`; `tools` → `{ tools: PersonaTools { allow, ..default } }`;
   `disallowed-tools` → `{ tools: { deny, ..default } }`; `deny-kinds` → `{ tools: { deny_kinds,
   ..default } }`.

**Edit → patch** (only changed fields, D22): `name` iff `!= opened.name`; `description` iff `!=`;
`tools = Some(PersonaTools { allow, deny, deny_kinds, command_run })` iff that value `!=
opened.tools`; `permission = Some(PersonaPermission { default, rules:
opened.permission.rules.clone() })` iff `default != opened.permission.default`. All `None` → close,
notice `Info(UNCHANGED)`, nothing sent.

**Keys per mode** (`CONTROL` chords always pass, so `ctrl-c` quits):

| Mode | Key | Effect |
|---|---|---|
| Browse | `j` `↓` / `k` `↑` | cursor ±1, stops at the ends |
| | `n` | (refusals below) → `Editing(Create)`, focus 0, notice cleared |
| | `e` | selected row → `Editing(Edit)` prefilled |
| | `b` | selected row → `Body(Edit)` over its body, cursor at the end |
| | `r` | selected row → `Rules` over `format_rules(&row.permission.rules)` |
| | `d` | selected row → `Deleting { stage: Asking }` |
| | `I` | `ImportPath { field: TextField::new() }` |
| | any other | `Handled::Pass` (the shell's `q`, `Tab`, digits, `?`, `w`) |
| | refusals | `unavailable.is_some()` → `n e b r d I` do nothing (no keys but navigation); `busy` → `Error(in_flight(busy))`; `e b r d` with no row → `Error(NO_ROW)` |
| Editing | printable, `Backspace`, `←→`, `Home`/`End` | the focused `TextField` |
| | `Tab` `↓` / `BackTab` `↑` | next / previous field, wrapping |
| | `Enter` | Create: draft ok → `Body(Create { form })` over `target.body`; Edit: patch → `UpdatePersona` (stays open until the reply) / unchanged → close; refusal → focus + `Error` |
| | `Esc` | `Browse`, nothing sent |
| | other | swallowed |
| Body / Rules | text keys, `Enter` (newline), arrows, `PgUp`/`PgDn` | the `TextArea` (`on_key(key, EDITOR_PAGE)`); an edit disarms `esc_armed` and clears an `Info` notice |
| | `Ctrl+S` | save (below) |
| | `Esc` | busy → `Error(in_flight)`; `Body(Create)` → back to `Editing(form)` with `form.target.body = text` (nothing lost); text == `original` or `esc_armed` → `Browse`; else arm + `Info(UNSAVED)` |
| | `Tab`/`BackTab`/other passes | swallowed |
| Deleting (Asking) | `y` | stage `InFlight`, send `DeletePersona { id }`, notice cleared |
| | `n` / `Esc` | `Browse` |
| | other | swallowed |
| Deleting (InFlight) | any non-`CONTROL` | swallowed |
| ImportPath | text keys | the field |
| | `Enter` | trimmed empty → `Error(ENTER_A_PATH)`; busy → `Error(in_flight)`; else `busy = Some(IMPORT_NAME)`, send `ImportPersonas { path }`, `Browse`, `Info(IMPORTING)` |
| | `Esc` | `Browse` |
| | other | swallowed |
| Report | `j` `↓` / `k` `↑` | scroll one line |
| | `Esc` / `Enter` | `Browse` |
| | other | swallowed |

**Saves.** Body(Create) `Ctrl+S`: build `NewPersona { id: PersonaId::new(), name, description,
body: text, tools, permission: PersonaPermission { default, rules: vec![] } }` from the kept form;
`new_persona_refusal(&new)` → `Error(sentence)` and stay; else `busy`, send `CreatePersona`.
Body(Edit) `Ctrl+S`: text == `original` → `Browse` + `Info(UNCHANGED)`; `persona_patch_refusal(&
PersonaPatch { body: Some(text), ..default })` → `Error`; else `sent = Some(text)`, send
`UpdatePersona { patch: body only }`. Rules `Ctrl+S`: `parse_rules(text)` → `Err(e)` →
`Error(e.to_string())`; `Ok(rules)` → `rules == parse_rules(original)` → `Browse` +
`Info(UNCHANGED)`; `persona_patch_refusal(&PersonaPatch { permission: Some(PersonaPermission {
default, rules }) })` → `Error`; else send `UpdatePersona { patch: permission only }`.

**Replies.**

| Reply | Effect |
|---|---|
| `Personas(rows)` | `personas = rows`, `unavailable = None`, clamp; never closes an editor |
| `PersonaWritten { personas, outcome }` | `personas` replaced, `unavailable = None`, `busy = None`, clamp, then `on_written` |
| `PersonaImports(imports)` | `personas` replaced, clamp; iff `busy == Some(IMPORT_NAME)`: `busy = None`, `land_import` |
| `Failed { request: "personas", message }` | `personas.clear()`, clamp, `unavailable = Some(message)` |
| `Failed { request, message }` with `request ∈ REQUEST_NAMES[1..]` and `busy == Some(request)` | `busy = None`; `Deleting` → `Browse`; every editor stays open over its text; `notice = Error(message)` (the store's sentence, B-11) |

`on_written`:

- `Created { id, name }`: `Body(Create)`/`Editing(Create)` → `Browse`; cursor → `id`'s row;
  ``Info("created persona `{name}`")``.
- `Updated { id, name }`: an `Editing(Edit)` of `id` → `Browse`; a `Body`/`Rules` editor of `id`:
  text == `sent` → `Browse`, else stays with `expected` = the row's new `updated_at`,
  `original = sent` (the skills rule, `library.rs:1291-1300`); ``Info("saved persona `{name}`")``.
- `Deleted { id }`: `Deleting { id, name }` → `Browse`, ``Info("deleted persona `{name}`")``;
  without that mode `Info("deleted persona")`.
- `Stale { id }`: an editor of `id` open and the row present → form: `rebase` (agents' rule:
  untouched fields take the row's text, changed ones keep theirs, `opened`/`expected` = the row's),
  notice `CHANGED_ELSEWHERE` or `clash_notice(labels)`; body/rules: text == `original` → replaced by
  the row's, `original` = the row's; `expected` (and `RulesEditor.default`) retaken; notice
  `CHANGED_ELSEWHERE_SAVE`. Row absent → `Browse` + `DELETED_ELSEWHERE`. No editor of `id` →
  `CHANGED_ELSEWHERE_CLOSED`.
- `Gone { id }`: an editor of `id` or `Deleting { id }` → `Browse` + `DELETED_ELSEWHERE`; else
  `GONE_CLOSED`.

`land_import(report)`: counts `imported`, `refused`, `skipped`, `dropped` (Imported rows with
non-empty `dropped`). Report empty → `Info(NOTHING_IMPORTED)`. `refused + skipped + dropped == 0`
→ ``Info(format!("imported {}", names.join(", ")))``. Otherwise, over `Browse` →
`Report { report, top: 0 }` and notice cleared; over any other mode (keys typed while the walk ran
opened an editor) → ``Error(format!("imported {i} · refused {r} · skipped {s}"))``.

**Section trait.** `wants_requests` → `vec![StoreRequest::Personas]` (global, unscoped);
`on_scope_change` does nothing (personas are global); `captures_input` →
`!matches!(self.mode, Mode::Browse)` (R-10: `h`/`l` are letters in every editor); `takes_paste`
default; `on_paste` → the focused `TextField` (`Editing`, `ImportPath`) or `area.on_paste(text)`
(`Body`, `Rules`), `Pass` otherwise.

**Constants** (exact text; `·` is `\u{b7}`, `—` `\u{2014}`, `›` `\u{203a}`, `…` `\u{2026}`):

| Const | Text |
|---|---|
| `HINT_BROWSE` | `j/k select · n new · e edit · b body · r rules · d delete · I import` |
| `HINT_EMPTY` (no rows, readable) | `n new · I import` |
| `HINT_UNAVAILABLE` | `` (empty: no keys but navigation) |
| `HINT_FORM_NEW` | `Tab/Shift+Tab field · Enter body · Esc cancel` |
| `HINT_FORM_EDIT` | `Tab/Shift+Tab field · Enter save · Esc cancel` |
| `HINT_BODY_NEW` | `Ctrl+S create · Esc back to the fields` |
| `HINT_EDITOR` (body edit, rules) | `Ctrl+S save · Esc cancel · Enter breaks the line` |
| `HINT_DELETING` | `y delete · n/Esc stop` |
| `HINT_IMPORT` | `Enter import · Esc cancel` |
| `HINT_REPORT` | `j/k scroll · Esc close` |
| `NOT_READ` | `personas not read yet` |
| `UNAVAILABLE` | `personas unavailable` (drawn `{UNAVAILABLE}: {message}`, `theme.error`, wrapped) |
| `NO_PERSONAS` | `no personas yet` |
| `NO_ROW` | `no persona is selected` |
| `UNCHANGED` | `nothing changed; nothing was written` |
| `UNSAVED` | `unsaved changes — Esc again discards` |
| `COMMAND_RUN_IS_Y_OR_N` | `` `command-run (y/n)` is y or n `` |
| `DEFAULT_IS_ASK_OR_DENY` | `` `permission-default` is blank, ask or deny `` |
| `ENTER_A_PATH` | `` type a path to a persona `.md` file or a directory of them `` |
| `IMPORTING` | `importing…` |
| `NOTHING_IMPORTED` | `nothing to import at that path` |
| `CHANGED_ELSEWHERE_SAVE` | `changed elsewhere since you opened it — reloaded; Ctrl+S retries against the current row` |
| `CHANGED_ON_BOTH_SIDES` (copied, agents' text) | `changed elsewhere — reloaded; Enter retries · also changed elsewhere: ` |
| `GONE_CLOSED` | `deleted elsewhere; nothing was written` |
| `RULES_HELP` | `one rule per line: reject_once\|reject_always kind=… name=… path=… command=… # reason · quote a value holding a space, ", #, = or \ · kind: read edit delete move search execute think fetch switch_mode other` (wrapped to the pane) |
| `fn delete_question(name)` | ``delete persona `{name}`? a persona bound to a phase is refused.`` |
| `fn in_flight(busy)` | `` `{busy}` is still in flight `` |

Reused from `settings/mod.rs`: `CHANGED_ELSEWHERE`, `CHANGED_ELSEWHERE_CLOSED`,
`DELETED_ELSEWHERE`, `wrapped`, `message`, `yes_or_no`. `clash_notice` is a private copy of
`agents.rs:2221-2246` (`agents.rs` stays untouched).

**Render** (the area is the section's; 100x30 in `SectionBench`): `Layout::vertical([Min(3),
Length(notice lines), Length(1)])` = body, notice (wrapped, 0 when none; `theme.error` for
`Notice::Error` and for `is_error` sentences), hint.

- Browse, readable, rows: one line per persona, the cursor row `theme.accent`, others
  `theme.base`: `{name} · {description} · deny {deny_kinds joined "," or "none"} · allow
  {allow.len()} · rules {rules.len()}`; the description is cut to fit the width with `…` (the
  name and the tail are never cut). Under the cursor row, dimmed and wrapped at width − 4 with a
  4-space indent: `allow {allow joined ", " or "all"} · disallowed {deny joined ", " or "none"} ·
  command-run {y|n} · default {ask|deny|inherit}`.
- Browse, no rows: `NO_PERSONAS` (dim). Unavailable: `{UNAVAILABLE}: {message}` in error,
  wrapped. Not read: `NOT_READ`.
- Editing: line 1 `new persona` / ``edit persona `{name}` ``, a blank line, then the seven
  `label: field` lines, labels padded to 18 (`command-run (y/n)` is 17 + 1), the focused label
  `theme.accent`, the focused field carrying the cursor (agents' `Editor::lines`).
- Body: line 1 ``body of `{name}` `` (create: ``body of new persona `{name}` ``), then the
  `TextArea` (`area.lines(width, height − 1, true, theme)`).
- Rules: line 1 ``rules of `{name}` ``, the wrapped `RULES_HELP` (dim), then the `TextArea`.
- Deleting: the Browse body; notice = `Info(delete_question(name))`; hint `HINT_DELETING`.
- ImportPath: the Browse body; the notice row is `path: ` + the field's line; hint `HINT_IMPORT`.
- Report: line 1 ``imported {i} · refused {r} · skipped {s}``, then one entry per outcome,
  wrapped with a 2-space continuation indent, scrolled from `top`:
  `+ {name} ({path})` and, when `dropped` is non-empty, `` — dropped from `tools`: `a`, `b` —
  `allow` keeps built-in tools only `` (base style); `! {path} — {message}` (`theme.error`);
  `· {path} — {reason}` (dim).

**The seven snapshots** (`htui/tests/snapshots/personas__*.snap`, 100x30):

| Snapshot | Built by | Shows |
|---|---|---|
| `personas__browse` | `SectionBench` + `Personas(demo rows)` | `architect` (cursor) and `reviewer` lines with cut descriptions and `deny edit,delete,move · allow 0 · rules 0`; the detail line `allow all · disallowed none · command-run y · default inherit` under `architect`; hint `HINT_BROWSE` |
| `personas__form` | `e` on `architect` | ``edit persona `architect` ``, seven prefilled fields (`deny-kinds: edit, delete, move`, `command-run (y/n): y`, empty `permission-default`), focus on `name`; hint `HINT_FORM_EDIT` |
| `personas__body` | `b` on `architect` | ``body of `architect` `` and the seed body's first lines (`You are the architect for this step. …`); hint `HINT_EDITOR` |
| `personas__rules` | `Personas` with `architect` carrying two rules (§2.3 rows 1 and 2), then `r` | ``rules of `architect` ``, the wrapped help, the two lines `reject_once kind=execute command="rm -rf" # never wipe` / `reject_always path=/etc` |
| `personas__delete_ask` | `d` on `reviewer` (after `j`) | the list; ``delete persona `reviewer`? a persona bound to a phase is refused.``; hint `HINT_DELETING` |
| `personas__import_report` | `I`, path, `Enter`, then a hand-built `PersonaImports` | header `imported 1 · refused 2 · skipped 1`; `+ code-architect (/srv/agents/code-architect.md) — dropped from \`tools\`: \`mcp__gortex__search\`, \`mcp__gortex__read\` — \`allow\` keeps built-in tools only`; `! /srv/agents/gortex-search.md — every \`tools\` entry is an MCP tool; …`; ``! /srv/agents/reviewer.md — persona `reviewer` exists; edit it in Settings › Personas, or delete it and import again``; `· /srv/agents/README.md — no frontmatter: …`; hint `HINT_REPORT` (fixed fake paths: no tempdir in a snapshot) |
| `personas__offline` | `Harness::over_backend(Backend::Offline { cache, since: Some(now) })` + `SettingsTab::with_sections(vec![PersonasSection])`, `with_store_state("offline · 0s", None)`, keyring mock | the whole frame: strip ` Personas `, `personas unavailable: store unreachable: this box browses its read-only cache and starts no run` in error, empty hint |

Registration (`htui/src/app/mod.rs:62-73`), after `BoxesSection`:

```rust
        // Last (MOD-26 milestone 2, D22): appending moves no existing section's line.
        Box::new(PersonasSection::new()),
```

An empty `description` drops its ` · {description}` segment from the Browse line.

---

## 3. T6 — core: N4, closed kinds, rule lines, import mode (serial, first; no SQL)

### 3.1 Files

| File | Change |
|---|---|
| `htui-core/src/model/persona.rs` | §2.1 type (fn removed); §2.2 `rule_kind_unknown` + second pass; §2.3 `RULE_KEYS`, `RuleLineError`, `parse_rules`, `format_rules`; §2.4 `read_file`/`checked`, `OnlyMcpTools`, `Imported`, `parse_import`, `pub fn list_of`; tests |
| `htui-core/src/model/run.rs` | `persona_for` (`:462-481`) and its test (`:1156-1176`) |
| `htui-core/src/model/mod.rs` | `pub use persona::{…}` (`:139-142`) gains `PersonaNotInSnapshot`, `RuleLineError` |
| `htui-core/src/store/traits.rs` | re-export `:1947-1951` only: `persona_not_in_snapshot` → `PersonaNotInSnapshot`, `+ rule_kind_unknown` |
| `htui-core/src/store/mod.rs` | the same two names in `traits::{…}` (`:16-41`) |
| `htui-orch/src/engine.rs` | every row of the §2.1 engine table |

### 3.2 Commit groups

1. **6.1** `feat(mod-26): T6 typed PersonaNotInSnapshot (N4)` — §2.1 in all six files, with the
   ported/new tests of §3.3 (a). Workspace compiles (no other crate names the fn, plan "Verified
   claims").
2. **6.2** `feat(mod-26): T6 closed rule kinds in the save rules (D18)` — §2.2, tests (b).
3. **6.3** `feat(mod-26): T6 rule-line grammar (D17)` — §2.3, tests (c).
4. **6.4** `feat(mod-26): T6 import mode of the persona parser (D19, OQ-7)` — §2.4, tests (d).

### 3.3 Tests (written first; red reason in brackets)

(a) N4 — `model/persona.rs` `mod tests`, `model/run.rs`, `htui-orch/src/engine.rs`:

| Test | Asserts [red reason] |
|---|---|
| `the_snapshot_refusal_escapes_the_persona_name` (ported, `:799-813`) | `PersonaNotInSnapshot { persona: "rev\niewer".into() }.to_string()` starts ``persona `rev\niewer` is not in the run's snapshot`` and has no `\n`; `"reviewer"` renders exactly the M1 sentence [type absent] |
| `persona_for_finds_refuses_and_skips` (`run.rs`, updated) | persona-less `Ok(None)`; carried `Ok(Some(_))`; missing `Err(PersonaNotInSnapshot { persona: "reviewer".into() })` [returns `String`] |
| `a_snapshot_without_its_persona_refuses_at_stage_three` (engine, updated) | `Err(StageThree::NoPersona(refusal))` with `refusal.persona == "reviewer"` and `refusal.to_string() == M1_SENTENCE` [payload is `String`] |
| the two I-4 end-to-end engine tests at `:11826`, `:11933` | `RunFailure::PromptRefused { phase: "prd", reason: M1_SENTENCE.to_owned() }` — the persisted reason byte-identical (B-15) [the fn they imported is gone] |

(b) D18 — `model/persona.rs`:

| Test | Asserts |
|---|---|
| `every_widening_shape_is_refused` (extended) | one more shape: a rule with `tool_kind: Some("exec")` → `Some(rule_kind_unknown("exec"))` [accepted today] |
| `a_misspelt_rule_kind_is_refused_after_the_older_sentences` | `[kind "exec"]` → `rule_kind_unknown("exec")`; `[kind "exec", PersonaMatch::default()]` → `RULE_MATCHES_EVERYTHING` (B-1); `[kind "ex\0ec"]` → `has_nul("persona.permission.rules")`; `kind ""` → `rule_kind_unknown("")`; each of the ten `TOOL_KINDS` (incl. `think`, `switch_mode`, `other`) accepted; the sentence for ``"e`x"`` is one line with the backtick raw (D-10) [no kind check] |
| `a_patch_is_checked_field_by_field` (extended) | `PersonaPatch { permission: Some(… kind "exec" …) }` → the same sentence |

(c) D17 — `model/persona.rs`:

| Test | Asserts |
|---|---|
| `rule_lines_round_trip_the_fixed_table` | §2.3's 17 rows both ways, and the two-rule row [stub returns `Ok(vec![])`/`""`] |
| `rule_lines_round_trip_generated_values` | §2.3's generated property, every value, one line each |
| `rule_lines_parse_spacing_comments_and_blank_lines` | §2.3's parse-only table |
| `rule_line_errors_name_their_line` | one input per E-1…E-10, each on line 2 after a valid line 1, `Err(RuleLineError { line: 2, message })` with the exact message, and `to_string()` == `rules line 2: {message}`; inputs: `reject kind=read`, `reject_once colour=red`, `reject_once kind=read kind=edit`, `reject_once kind`, `reject_once kind= name=x`, `reject_once path=a=b`, `reject_once path="open`, `reject_once path="a\qb"`, `reject_once path="a"b`, `reject_once kind=read # "r" x`; and `reject_oncekind=read` → E-1 with word `reject_oncekind=read` |
| `a_rule_line_kind_outside_the_list_is_the_store_sentence` | `reject_once kind=exec` → `RuleLineError { line: 1, message: rule_kind_unknown("exec") }`; `kind=""` → `rule_kind_unknown("")`; `kind="read"` (quoted) accepted |
| `an_empty_value_is_quoted_and_an_absent_key_is_none` | `name=""` → `Some("")`, formats back as `name=""`; an absent key is `None` and is not written |
| `format_rules_writes_one_line_per_rule` | `format_rules(&[])` is `""`; three rules → three lines, no trailing `\n` |

(d) D19 — `model/persona.rs`. Fixtures are `const`s in the test module: the **frontmatter lines
verbatim** of `.claude/agents/code-architect.md`, `.claude/agents/rust-reviewer.md` and
`~/.claude/agents/gortex-{impact,search}.md` (copied, §0 house style), each followed by `---\n\n`
and a one-line body (`You design.\n` etc.):

| Test | Asserts |
|---|---|
| `parse_import_drops_mcp_tools_and_names_them` | code-architect → `allow == ["Read","Grep","Glob","Bash"]`, `dropped` = the nine `mcp__gortex__*` names in file order, `name == "code-architect"`, description verbatim [stub refuses] |
| `parse_import_accepts_both_repo_agent_files` | rust-reviewer likewise; both `PersonaFile`s pass `persona_refusal` |
| `parse_import_refuses_an_all_mcp_tools_file` | gortex-impact and gortex-search → `Err(PersonaFileError::OnlyMcpTools)`; its `to_string()` is exactly D19's sentence |
| `parse_file_still_refuses_every_mcp_tools_file` | all four through `parse_file` → `Refused(allow_names_an_mcp_tool(<first mcp entry>))` (the seed path is unchanged) |
| `parse_import_keeps_parse_files_other_refusals` | `model: opus` → `Model`; no fence → `Fence(NoFence)`; `tools:` empty → `allow` empty, `dropped` empty, accepted; `tools: Read, Bad Name` → `Refused(not_a_tool_name("allow", "Bad Name"))` |
| `list_of_is_the_forms_list_grammar` | `list_of(" a, ,b ,")` == `["a","b"]`; `list_of("")` empty |

### 3.4 Gate

G-T6 (§10). Also `grep -rn "persona_not_in_snapshot" crates` prints nothing.

---

## 4. T7 — store: `delete_persona` and `0013` (after T6)

### 4.1 Files

| File | Change |
|---|---|
| `htui-core/src/store/traits.rs` | method + comment `:1003-1009` + `BOUND_PHASES_NAMED` + `persona_is_bound` (§2.5) |
| `htui-core/src/store/mod.rs` | `persona_is_bound` in `traits::{…}` |
| `htui-core/src/store/mem.rs` | import (`:~60`); `State::delete_persona` (§2.8); `WriteStore` forward; unit test |
| `htui-core/src/store/conformance.rs` | three cases + `CASES` (after `:182`) + `run_case` arms (before `:460`); the 18th shape in `persona_writers_refuse_every_widening_shape` (`shapes.len()` 17 → 18, `:15634`); imports |
| `htui-core/tests/mem_store.rs` | `130 → 133` (`:37`), sentence gains "…and MOD-26 milestone 2's three persona delete cases (plan D14)" |
| `htui-store/src/pg/write.rs` | import (`:47-60`); `delete_persona` (§2.7) |
| `htui-store/src/writer.rs` | forward |
| `htui-store/migrations/0013_persona_phase_index.sql` (new) | §2.6 verbatim |
| `htui-store/.sqlx/*` | three new files (regen) |
| `htui-store/tests/pg_conformance.rs` | `EXPECTED_CASES` `130 → 133` (`:26`), doc `:25` ("…and MOD-26 milestone 2's three delete cases (plan D14) make it 133"), message `:33-34` |
| `htui-store/tests/pg_criteria.rs` | two race tests (§4.3) |
| `htui-store/tests/migrations.rs` | pins (§9) + `the_persona_phase_index_exists` |
| `htui-store/tests/connect.rs` | `12 → 13` at `:140`, `:156`, `:242` and their sentences |
| `htui-agent/src/conformance.rs` | `UsageSpy` forward only |
| `htui-agent/tests/recorder.rs` | `SpyStore` forward only |

### 4.2 Commit groups

1. **7.1** `feat(mod-26): T7 delete_persona, refused while bound (D14, I-9)` — every row of §4.1
   except `0013`, `migrations.rs` and `connect.rs`; `.sqlx` regenerated **before** the commit
   (the commit builds with `SQLX_OFFLINE=true`), count restated in the message (318 → 321).
2. **7.2** `feat(mod-26): T7 index step_graph_phase.persona_id (0013, N5)` — `0013`,
   `migrations.rs`, `connect.rs`. No query changes, no `.sqlx` change; `regen` re-run only to
   confirm `check` passes against a 13-migration scratch database.

### 4.3 Tests (written first)

Store conformance (generic `<S: WriteStore>`; MemStore via `mem_store.rs`, PgStore via
`pg_conformance.rs`, one fresh demo store per case). Fixture: personas `architect`
(`ids::PERSONA_ARCHITECT`) and `reviewer` (`ids::PERSONA_REVIEWER`); graphs `GRAPH_HTUI_FEAT`
(`feature`: prd, plan, implement, review), `GRAPH_HTUI_ANA` (`analysis`: research, verdict),
`GRAPH_HTUI_FIX` (`bug`: reproduce, fix, review), `GRAPH_AGY_FEAT`; slugs `htui`, `agy`.

| Case | Asserts [red reason: the method does not exist / answers wrong] |
|---|---|
| `delete_persona_removes_an_unbound_row_once` | `create_persona(scout)`; `delete_persona(scout.id)` → `Ok`; names `[architect, reviewer]`; again → `NotFound { entity: "persona", .. }`; `delete_persona(PersonaId::new())` → `NotFound`; `delete_persona(architect)` (unbound in the fixture) → `Ok`; names `[reviewer]` |
| `a_bound_persona_is_not_deleted_and_names_its_phases` | bind `reviewer` to `htui/feature/review` and `agy/feature/review` (`update_phase`, `Some(Some(id))`); `create_step_graph(NewStepGraph { project_id: PROJECT_HTUI, name: "HTUI-3-override", is_override: true, .. })` + `create_phase(StepGraphPhase { persona_id: Some(reviewer), ..new_phase(g, 0, "review") })`; `delete_persona(reviewer)` → `Constraint` **exactly** the 3-phase sentence of §2.5; `personas()` still lists `reviewer`; clear the three (`Some(None)`) → `delete_persona` `Ok`; the three phases remain with `persona_id == None` |
| `a_persona_bound_to_many_phases_names_five_and_counts_the_rest` | bind `architect` to htui `feature/{prd,plan,implement,review}`, `analysis/{research,verdict}`, `bug/reproduce` (7) → exactly the 7-phase sentence of §2.5 |
| `persona_writers_refuse_every_widening_shape` (extended) | 18th shape: `rules(vec![rule(PersonaMatch { tool_kind: Some("exec".into()), .. })])` → `rule_kind_unknown("exec")` on create and patch, both stores [T6 made it refuse; the count pin moves] |

`mem.rs` unit test: `persona_is_bound_sorts_by_the_triple_and_counts_the_rest` — one holder →
"1 phase … clear it"; `[("web-app","x","p"), ("web","x","p"), ("web","x","p")]` → 2 phases,
`web/x/p` first (triple order, not string order, F-6), duplicate counted once; seven holders →
five named, "and 2 more".

`pg_criteria.rs` (`common::demo_db()`, `db.drop_db().await` at the end; the
`a_mint_racing_a_kind_delete_still_names_what_holds_it` shape, `:3326`):

| Test | Asserts |
|---|---|
| `a_bind_racing_a_persona_delete_still_names_the_phase` | racer: `BEGIN`; `UPDATE step_graph_phase SET persona_id = $1 WHERE id = $2` (reviewer, `htui/feature/review`); spawn `store.delete_persona(reviewer)`; sleep 500 ms; `COMMIT` → `Constraint` exactly the 1-phase sentence; `SELECT count(*) FROM persona WHERE id = $1` is 1 |
| `a_bind_behind_a_persona_delete_gets_references_no_row` | racer: `BEGIN`; `DELETE FROM persona WHERE id = $1` (architect, unbound); spawn `store.update_phase(plan.id, plan.updated_at, PhasePatch { persona: Some(Some(architect)), .. })`; sleep 500 ms; `COMMIT` → `Constraint(references_no_row("step_graph_phase.persona_id", architect, "persona"))`, never a raw `23503` |

`migrations.rs`: `the_persona_phase_index_exists` — on `common::fresh_db()`, `SELECT indexdef FROM
pg_indexes WHERE schemaname = 'public' AND indexname = 'idx_step_graph_phase_persona'` →
`CREATE INDEX idx_step_graph_phase_persona ON public.step_graph_phase USING btree (persona_id)`
[no index before `0013`].

### 4.4 Gate

G-T7 (§10), every Postgres command under the lock.

---

## 5. T8 — Kinds `persona` field (worktree `hr/MOD-26-t8`, merged after T7)

### 5.1 Files

| File | Change |
|---|---|
| `htui/src/catalogue.rs` | `PersonaSummary`, `CatalogueSnapshot.personas`, `snapshot()` fill, the `:107` literal, imports (`PersonaId`) |
| `htui/src/ui/tabs/settings/kinds.rs` | §2.9 table and helpers; module doc gains a sentence on D23 |
| `htui/tests/kinds.rs` | tests below; rename `:1554` |
| `htui/tests/snapshots/kinds__editor_phase.snap` | updated (seven fields) |
| `htui/tests/snapshots/kinds__phase_persona.snap` | new |

### 5.2 Commit groups

1. **8.1** `feat(mod-26): T8 the catalogue carries the persona registry (D23)` — `catalogue.rs`
   and `the_catalogue_carries_the_personas_by_name`.
2. **8.2** `feat(mod-26): T8 persona field on the phase editor (D23, OQ-11)` — `kinds.rs`, the
   remaining tests, both snapshots.

### 5.3 Tests (written first; `htui/tests/kinds.rs`)

Helper: `bench_with_bound(persona: Option<PersonaId>)` — `MemStore::demo()`, `update_phase` binding
vulkan `analysis/research`, then the catalogue through `serve(&Backend::memory(store),
&Catalogue(vulkan_scope()))` delivered to a `SectionBench`; `research(&snapshot)` as today.

| Test | Asserts [red reason] |
|---|---|
| `the_catalogue_carries_the_personas_by_name` (8.1) | `demo_catalogue(&demo()).personas == [PersonaSummary { id: PERSONA_ARCHITECT, name: "architect" }, { PERSONA_REVIEWER, "reviewer" }]` [field absent] |
| `e_on_a_phase_opens_seven_fields_and_enter_sends_update_phase` (renamed from `…_six_…`, doc updated) | today's assertions plus label `persona` present and last; the patch carries `persona: None` (untouched); snapshot `editor_phase` [six fields] |
| `a_typed_persona_name_binds_the_phase` | six `tab`, type `reviewer`, `enter` → exactly one `UpdatePhase` with `patch.persona == Some(Some(PERSONA_REVIEWER))` [field absent] |
| `a_persona_and_budget_change_sends_the_patch_then_the_budget` | type `reviewer` in `persona` and `9000` in `token_budget` → first request `UpdatePhase` with `persona == Some(Some(reviewer))` (the follow-up `SetPhaseBudget` is owed) — D-4 [today: one `SetPhaseBudget`, persona lost] |
| `blanking_a_bound_persona_sends_some_none` | `bench_with_bound(Some(reviewer))`; field prefilled `reviewer`; clear it; `enter` → `persona == Some(None)` |
| `an_untouched_bound_persona_sends_none` | bound; edit `name` only → `persona == None` |
| `an_unknown_persona_name_is_refused_before_sending` | type `ghost` → no `Action::Store`; notice ``no persona named `ghost`; known: architect, reviewer`` in `theme.error` (`error_text`) |
| `with_no_personas_the_refusal_says_add_one` | the catalogue delivered with `personas` cleared; type `ghost` → `no personas exist; add one in Settings › Personas` |
| `an_id_the_list_does_not_know_prints_as_the_id_and_is_left_alone` | catalogue whose research binds a `PersonaId` absent from `personas` → the line ends `· persona <uuid>`, the field prefills the uuid, a `name` edit sends `persona == None` |
| `a_bound_phase_line_names_its_persona` | bound `reviewer`: the research line ends ` · budget inherit · persona reviewer`; the verdict line has no `persona`; snapshot `phase_persona` |

The other seven `kinds__*` snapshots must stay unchanged (D-5): `cargo insta test --check`.

### 5.4 Gate

G-T8 (§10) in the worktree; after the merge, G-M on the primary tree.

---

## 6. T9 — Settings › Personas and the import (after the T8 merge)

### 6.1 Files

| File | Change |
|---|---|
| `htui/src/persona_import.rs` (new) | §2.12 + unit tests |
| `htui/src/persona_settings.rs` (new) | §2.11 + unit tests |
| `htui/src/lib.rs` | `pub mod persona_import;` `pub mod persona_settings;` (after `keymap`, before `preview`) |
| `htui/src/store_worker.rs` | §2.10 |
| `htui/src/ui/tabs/settings/personas.rs` (new) | §2.13 |
| `htui/src/ui/tabs/settings/mod.rs` | `pub mod personas;`, `pub use personas::PersonasSection;`, module doc names MOD-26's section |
| `htui/src/app/mod.rs` | import + registration (§2.13) |
| `htui/tests/personas.rs` (new) | `#![cfg(feature = "testkit")]`, section tests |
| `htui/tests/settings.rs` | strip pin: eighth section, doc "eight … 71 of the 100 columns", `assert_eq!(sections.len(), 8)` |
| `htui/tests/snapshots/personas__*.snap` | seven new |

### 6.2 Commit groups

1. **9.1** `feat(mod-26): T9 persona import walk on the worker (D20, OQ-8, OQ-9)` —
   `persona_import.rs`, `lib.rs` (one line), its tests.
2. **9.2** `feat(mod-26): T9 persona requests and their worker module (D21)` —
   `persona_settings.rs`, `lib.rs`, `store_worker.rs`, tests.
3. **9.3** `feat(mod-26): T9 Settings › Personas: list, form, body (D22)` — `personas.rs` with
   Browse, Editing, Body (create and edit), the reply handling; `settings/mod.rs`; `tests/personas.rs`
   (the 9.3 rows); snapshots `browse`, `form`, `body`. Not registered yet (a `pub` type: no
   dead-code warning).
4. **9.4** `feat(mod-26): T9 Settings › Personas: rules, delete, import (D17, D14, D20)` — Rules,
   Deleting, ImportPath, Report; tests; snapshots `rules`, `delete_ask`, `import_report`.
5. **9.5** `feat(mod-26): T9 register Settings › Personas (D22)` — `app/mod.rs`,
   `tests/settings.rs`, the offline and registration tests, snapshot `offline`.

### 6.3 Tests (written first)

`persona_import.rs` `mod tests` (`Backend::memory(MemStore::demo())`, `tempfile::tempdir()`;
fixture texts are consts: the code-architect frontmatter of §3.3(d) with a body, a two-key file
`---\nname: scout\ndeny-kinds: execute\n---\n\nYou scout.\n`, a gortex-search copy, a `README.md`
without a fence):

| Test | Asserts |
|---|---|
| `a_file_imports_one_persona_and_names_its_dropped_mcp_tools` | `[Imported { name: "code-architect", path, dropped: <9 names> }]`; the row's `tools.allow == [Read, Grep, Glob, Bash]` |
| `a_directory_imports_its_depth_zero_md_files_in_byte_order` | `b.md` (scout) and `a.md` (code-architect) → rows in `a`, `b` order; both rows exist |
| `a_readme_without_frontmatter_is_skipped` | `Skipped { path: …/README.md, reason: NO_FRONTMATTER }`; no row |
| `a_non_md_file_and_a_subdirectory_are_left_alone` | `notes.txt` (with valid frontmatter) and `sub/x.md` → absent from the report, no row (B-14) |
| `an_all_mcp_tools_file_is_refused` | `Refused { message: OnlyMcpTools's sentence }` |
| `a_known_name_is_refused_and_the_row_is_untouched` | a file named `reviewer` → OQ-8's sentence; `reviewer`'s body unchanged |
| `a_name_met_twice_in_one_import_is_refused_naming_the_first_file` | `a.md` and `b.md` both `scout` → second `Refused` naming `a.md`; one row |
| `a_directory_past_the_file_cap_imports_the_cap_and_reports_the_rest` | 65 files `p00.md`…`p64.md` (names `p-00`…) → 64 `Imported`, last row `Skipped { "1 more file(s) past the 64-file cap" }` |
| `a_file_over_the_byte_cap_is_skipped` | 256 KiB + 1 → `Skipped` with the cap sentence |
| `a_missing_path_is_refused` | ``Refused { "could not read `…` (…)" }`` |
| `a_refused_file_writes_nothing` | `model: opus` → `Refused { MODEL_REFUSED }`; registry unchanged |
| `the_report_prints_no_file_content` | the report's `Debug` holds no body text |
| `offline_the_import_is_unreachable` | `Backend::Offline` (`CacheStore::open` in a tempdir) → `Err(Unreachable(DATABASE_UNREACHABLE))` |

`persona_settings.rs` `mod tests` (the `skills.rs` helpers: `demo()`, `offline()`):

| Test | Asserts |
|---|---|
| `the_read_answers_the_registry_by_name` | `Personas` → `StoreReply::Personas` with `[architect, reviewer]` |
| `a_create_answers_created_with_the_reread` | `PersonaWritten { outcome: Created { id, name: "scout" }, personas }` holds scout |
| `a_refused_create_answers_the_stores_sentence_bare` | name `Bad` → `Failed { request: "create_persona", message: invalid_persona_name("Bad") }` (no `constraint violated:`); a taken name → `already_exists("persona", "reviewer")` (B-11) |
| `an_update_applies_only_the_patch_it_carries` | description-only patch → `Updated`; other fields unchanged in the re-read |
| `a_spent_token_answers_stale_and_writes_nothing` | `Stale { id }`; the row unchanged |
| `an_update_of_a_gone_row_answers_gone` | unknown id → `Gone { id }` |
| `a_delete_answers_deleted_and_the_row_is_gone` | `Deleted { id }`; re-read lacks it |
| `a_bound_delete_answers_the_bound_sentence` | bind reviewer to htui `feature/review` on the memory store first → `Failed { request: "delete_persona", message: <1-phase sentence> }` |
| `a_delete_of_a_gone_row_answers_gone` | `Gone { id }` |
| `offline_every_request_is_refused_with_the_unreachable_sentence` | all five through `serve` → `Err(Unreachable(DATABASE_UNREACHABLE))`, and through `store_worker::serve` → `Failed { request: <its name>, message ⊇ DATABASE_UNREACHABLE }` |
| `request_names_match_the_name_arms` | one sample of each, in order, `.name()` == `REQUEST_NAMES`; `READ_NAME == REQUEST_NAMES[0]`, `IMPORT_NAME == REQUEST_NAMES[4]` |
| `a_foreign_request_is_refused_by_name` | `Workspaces` → `Err(Backend("not a persona request: workspaces"))` |
| `an_import_is_routed_here_and_answers_its_report_beside_the_registry` | through `store_worker::serve`: `PersonaImports` whose `personas` includes the new row |

`htui/tests/personas.rs` (`SectionBench`; replies built from `MemStore::demo().personas()`):

| Test (commit) | Asserts |
|---|---|
| `the_registry_lists_one_line_per_persona` (9.3) | two lines, `deny edit,delete,move · allow 0 · rules 0`, the detail line; snapshot `browse` |
| `j_and_k_move_the_cursor_and_stop_at_the_ends` (9.3) | `k` at 0 stays; `j j j` stops at 1 (the detail line moves) |
| `wants_requests_is_the_unscoped_read` (9.3) | `vec![StoreRequest::Personas]` |
| `n_opens_the_fields_and_enter_moves_to_the_body` (9.3) | `n`, type `scout`, `enter` → body editor (``body of new persona `scout` ``), nothing sent |
| `ctrl_s_in_the_new_body_sends_create_persona` (9.3) | `n`, name `scout`, `deny-kinds` `execute`, `enter`, type `You scout.`, `ctrl-s` → one `CreatePersona` whose `new` has those fields, `command_run: true`, `default: None`, no rules |
| `esc_in_the_new_body_returns_to_the_fields_and_keeps_the_body` (9.3) | type body, `esc` → form; `enter` → body editor shows the same text |
| `a_blank_new_body_is_refused_with_the_stores_sentence` (9.3) | `ctrl-s` on an empty body → notice `BLANK_PERSONA_BODY`, nothing sent |
| `a_created_reply_closes_the_editor_and_selects_the_row` (9.3) | `PersonaWritten { Created }` with the re-read → Browse, cursor on `scout`, ``created persona `scout` `` |
| `e_opens_the_prefilled_form` (9.3) | labels and prefill; snapshot `form` |
| `e_then_enter_sends_only_the_changed_fields` (9.3) | description changed → `UpdatePersona { patch: PersonaPatch { description: Some(_), ..default } }` and `expected` == the row's `updated_at` |
| `an_unchanged_form_sends_nothing` (9.3) | `UNCHANGED`, Browse |
| `a_permission_default_edit_keeps_the_rules` (9.3) | row with one rule; `deny` typed → `patch.permission == Some({ default: Some(Deny), rules: <the row's> })`, `tools == None` |
| `a_form_refusal_is_the_stores_sentence_and_focuses_its_field` (9.3) | `tools` = `mcp__x__y` → notice `allow_names_an_mcp_tool("mcp__x__y")`; the next typed char lands in `tools` (render shows it) |
| `command_run_must_be_y_or_n` (9.3) | `maybe` → `COMMAND_RUN_IS_Y_OR_N`, focus there |
| `b_opens_the_body_and_ctrl_s_sends_the_body_only` (9.3) | snapshot `body`; edit → `UpdatePersona { patch: { body: Some(_) } }` |
| `h_and_l_type_into_the_body` (9.3) | after `b`, `captures_input()` is true; `h` `l` appear in the text (R-10) |
| `esc_on_an_edited_body_warns_once` (9.3) | first `esc` → `UNSAVED`, still open; second → Browse |
| `a_stale_form_rebases_untouched_fields` (9.3) | `e`, change name; reply `Stale` with a re-read whose description moved → description field shows the new text, name keeps the typed one, notice `CHANGED_ELSEWHERE`; the next `enter` carries the new `expected` |
| `a_stale_form_names_a_field_changed_on_both_sides` (9.3) | both changed description → `CHANGED_ON_BOTH_SIDES` + `description` |
| `gone_closes_the_editor` (9.3) | `Gone { id }` → Browse, `DELETED_ELSEWHERE` |
| `a_write_key_while_a_write_is_in_flight_is_refused` (9.3) | after a send, `n` → ``Error("`update_persona` is still in flight")`` |
| `a_refused_write_keeps_the_editor_over_its_text` (9.3) | `Failed { request: "update_persona", message }` → editor open, notice the message in error, `busy` cleared (a second `enter` sends) |
| `a_section_debug_prints_no_typed_text` (9.3) | `format!("{section:?}")` with `secret` typed in a field holds no `secret` |
| `r_opens_the_rules_editor_one_line_per_rule` (9.4) | snapshot `rules` |
| `rules_save_sends_the_permission_with_the_default_kept` (9.4) | add `reject_once kind=read`, `ctrl-s` → `patch.permission == Some({ default: <row's>, rules: [3 rules] })`, other fields `None` |
| `a_rules_line_error_names_the_line_and_sends_nothing` (9.4) | `reject_once colour=red` on line 3 → ``rules line 3: `colour` is not a rule key; …``, nothing sent |
| `a_misspelt_rule_kind_is_refused_before_sending` (9.4) | `kind=exec` → `rules line 1: <rule_kind_unknown("exec")>` |
| `an_empty_match_is_refused_with_the_stores_sentence` (9.4) | a bare `reject_once` → `RULE_MATCHES_EVERYTHING` |
| `d_asks_and_y_sends_delete_persona` (9.4) | `j`, `d` → snapshot `delete_ask`; `y` → one `DeletePersona { id: reviewer }` |
| `n_or_esc_at_the_question_sends_nothing` (9.4) | Browse, nothing drained |
| `a_bound_refusal_closes_the_question_and_shows_the_sentence` (9.4) | `Failed { request: "delete_persona", message: <bound sentence> }` → Browse; the sentence in `theme.error` |
| `a_deleted_reply_says_which` (9.4) | `PersonaWritten { Deleted }` → ``deleted persona `reviewer` `` |
| `upper_i_opens_the_path_and_enter_sends_import_personas` (9.4) | `I`, type `/srv/agents`, `enter` → `ImportPersonas { path: "/srv/agents" }`, Browse, `IMPORTING` |
| `an_empty_path_is_refused` (9.4) | `ENTER_A_PATH`, nothing sent |
| `a_clean_import_says_so_in_the_notice` (9.4) | report of two clean `Imported` (`code-architect`, `scout`, no drops) → notice `imported code-architect, scout`, no report opens |
| `an_import_with_a_refusal_or_a_drop_opens_the_report` (9.4) | snapshot `import_report`; `j` scrolls; `esc` → Browse |
| `an_import_landing_over_an_editor_says_the_counts` (9.4) | `e` open while busy → `Error("imported 1 · refused 2 · skipped 1")`, the editor kept |
| `offline_the_section_says_unavailable_and_offers_no_keys` (9.5) | Harness offline (§2.13 table): the sentence, empty hint; `n`, `e`, `d`, `I` change nothing on screen; snapshot `offline` |
| `the_product_registers_personas_last` (9.5) | `Harness::demo()` + `register_all`, `4`, seven `l` → ` Personas ` accented and the two seed rows on screen |
| `the_section_strip_fits_the_frame` (`tests/settings.rs`, 9.5) | eight sections, ≤ 100 (71) |

### 6.4 Gate

G-T9 (§10).

---

## 7. T10 — docs and pins (serial, last)

- `docs/personas.md`: replace "Binding a persona to a phase in milestone 1" (`:71-105`) with
  "Binding a persona to a phase" (the Kinds `persona` field: blank = none, a name from the
  registry, the refusal sentences, a bound phase's line, store API and SQL kept as the scripted
  path); a new "Settings › Personas" section (keys, the seven fields in frontmatter spelling, the
  body and rules editors, only-changed-fields saves, stale/gone behaviour, offline); "Rule lines"
  (§2.3's grammar, escape table, examples, errors); "Deleting a persona" (I-9, the sentence, the
  index); "Importing persona files" (file or directory, depth 0, `*.md`, fence-less skipped,
  known names refused, twice-in-batch, caps, the `mcp__` drop and the all-`mcp__` refusal, the
  report; the row is the truth, the file is never re-read); `:184-187` now says a misspelt kind
  is refused (D18 closed list of ten); "What milestone 1 does not do yet" (`:346-359`) becomes
  "What MOD-26 does not do": engine steps only (chat, promoted, judge, preview), no per-candidate
  binding, no export, no `$EDITOR` for the body (OQ-12; with or after MOD-13 milestone 4).
- `README.md`: `:172` Settings row adds "personas"; the sections table `:301-311` gains
  `Personas`.
- `HANDOFF.md` (`:40-52`): **re-counted, never incremented** — `StoreRequest` 104, `StoreReply` 61,
  snapshots 142, `.sqlx` 321, `CASES` 133, `READ_CASES` 14, migrations 13 (`0013_persona_phase_index`
  in the list), next `0014` (cache next `0005`), `persona_settings::REQUEST_NAMES` 5 beside
  `skills::REQUEST_NAMES` 6, eight Settings sections (71 of the 100 strip columns), the pins'
  re-count date.
- `.claude/prds/mod-26-agent-personas.prd.md`: milestone 2 row → done, with the moved counts.
- `.claude/plans/mod-26-m2-persona-authoring.plan.md`: status, acceptance boxes, close-out counts.
- Commit **10.1** `docs(mod-26): T10 milestone 2 docs, pins and status`.
- Gate: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`, then G-Final.

---

## 8. Lanes and rules

| Wave | Tasks | Where | Postgres |
|---|---|---|---|
| W1 | T6 → T7 | primary `hr/MOD-26` | T7 only |
| W1' | T8 (parallel with W1) | worktree `hr/MOD-26-t8` | none |
| merge | `git merge --no-ff hr/MOD-26-t8` after T7's last commit; G-M | primary, no lane running | yes |
| W2 | T9 | primary | none in its gate |
| W3 | T10, G-Final | primary | full |

1. **Branch first** (auto-memory `workflow-worktree-implementers`): `git worktree add -b
   hr/MOD-26-t8 /home/mluigi/projects/htui-wt/mod-26-t8 cb69a011`; `df -h .` first (≈ 10 GB per
   `target/`). Remove the worktree before `git branch -d`.
2. **Native tools in the worktree**: Gortex does not index it and its edit tools write the primary
   checkout.
3. **File ownership**: a task edits only its §n.1 table.
4. **One Postgres lane; `.sqlx` is T7's alone.** Every command with `HTUI_TEST_DATABASE_URL` set
   and every `cargo sqlx` runs under `flock /tmp/mod26-pg.lock`. The prepare database is a
   scratch `htui_sqlx_mod26m2` (`docs/hr-sandbox.md:194-210`), never the test database (auto-memory
   `sqlx-prepare-needs-migrated-scratch-db`); migrate it again after writing `0013`. Commits build
   offline.
5. `--all-features` and `--test-threads=1` on every test command; htui-orch with
   `--no-fail-fast` and a `SIGABRT` grep (auto-memory `htui-orch-test-stack-headroom`).
6. **Commit incrementally**; never stash.
7. After each gate: `pgrep -af 'target/debug/deps'` and `pgrep -af 'htui worker'` print nothing;
   `ls ~/.config/htui` shows no test debris; `df -h .` before a Postgres-heavy gate.

---

## 9. Pins

| Pin | Now | After | Where it moves |
|---|---|---|---|
| Store conformance `CASES` | 130 | 133 | T7: `htui-core/tests/mem_store.rs:37` + its sentence; `htui-store/tests/pg_conformance.rs:25-26`, `:33-34` |
| `persona_writers_refuse_every_widening_shape` shapes | 17 | 18 | T7: `conformance.rs:15634` |
| `READ_CASES` | 14 | 14 | — |
| Migrations | 12 | 13 | T7: `migrations.rs:96` (vector + `13`) and `:97-102` ("…MOD-26's 0012_persona.sql and MOD-26 milestone 2's 0013_persona_phase_index.sql, in ordinal order"); `:989-990` `Pending(13)`, "thirteen embedded migrations, none applied (through MOD-26 milestone 2's 0013_persona_phase_index.sql)"; `:1088`, `:1093`, `:1112` (13); `:1262-1264` `13`, "the thirteen embedded migrations (through …0013_persona_phase_index.sql)"; `connect.rs:140-142`, `:156-158`, `:242-244` (13, "thirteen", "0013_persona_phase_index.sql") |
| `TABLES` / its messages | 42 | 42 | — (D-6) |
| Commented columns | 44 | 44 | — |
| `.sqlx` files | 318 | 321 | T7 commit 7.1 (restated from `ls … \| wc -l`) |
| `StoreRequest` / `StoreReply` | 99 / 58 | 104 / 61 | T9 (HANDOFF says 96/55: re-count at T10) |
| `crates/htui/tests/snapshots` | 134 | 142 | T8 +1, T9 +7 |
| Settings sections / strip columns | 7 / 61 | 8 / 71 | T9 |
| `persona_settings::REQUEST_NAMES` | — | 5 | T9 |
| `WriteStore` methods | — | +1 | T7 |
| `htui-orch` `CASES` | 92 | 92 | — |
| Unchanged | | | `GraphSnapshot::V`, `RECORD_VERSION`, `FEATURE_TOPOLOGY`, `MIRRORED_TABLES`, `PolicyFor`, workspace members, every snapshot but `kinds__editor_phase` (I-12) |

---

## 10. Gate reference

```bash
LOCK="flock /tmp/mod26-pg.lock"
NOPG="env -u HTUI_TEST_DATABASE_URL -u DATABASE_URL"
pg()   { $LOCK cargo test -p "$1" --all-features --no-fail-fast -- --test-threads=1; }
nopg() { $NOPG cargo test -p "$1" --all-features --no-fail-fast -- --test-threads=1; }
lint() { cargo clippy -p "$1" --all-targets --all-features -- -D warnings; }
offline() { $NOPG SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features; }
# T7's prepare database, created once:
#   psql -h localhost -p 5439 -U postgres -c 'CREATE DATABASE htui_sqlx_mod26m2;'
SQLX_DB=postgres://postgres@localhost:5439/htui_sqlx_mod26m2
regen() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB sh -c \
  'cargo sqlx migrate run --source migrations && cargo sqlx prepare -- --all-targets --all-features'); }
check() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB \
  cargo sqlx prepare --check -- --all-targets --all-features); }
orch() { $NOPG cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 \
  | tee /tmp/mod26-m2-orch.log; ! grep -n SIGABRT /tmp/mod26-m2-orch.log; }
```

| Gate | Commands |
|---|---|
| G-T6 | `nopg htui-core`; `orch` (no `SIGABRT`, no `FAILED`); `offline`; `lint htui-core`; `lint htui-orch`; `cargo fmt --all -- --check`; `grep -rn persona_not_in_snapshot crates` empty |
| G-T7 | `regen` then `check`; `pg htui-core`; `pg htui-store` (the log shows `case delete_persona_removes_an_unbound_row_once` etc., not `0 passed`); `nopg htui-agent`; `offline`; `lint htui-core`; `lint htui-store`; `lint htui-agent`; `cargo fmt --all -- --check`; `ls crates/htui-store/.sqlx \| wc -l` = 321 |
| G-T8 (worktree) | `$NOPG cargo test -p htui --all-features --test kinds -- --test-threads=1`; `$NOPG cargo insta test -p htui --all-features --test kinds --check`; `lint htui`; `cargo fmt -p htui -- --check` |
| G-M (after the merge) | `offline`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `nopg htui`; `$NOPG cargo insta test -p htui --all-features --check`; `check` |
| G-T9 | `nopg htui`; `$NOPG cargo insta test -p htui --all-features --check` (nothing pending, `kinds__*` and every other old snapshot unchanged); `lint htui`; `offline`; `cargo fmt --all -- --check` |
| G-Final | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `offline`; `check`; `$LOCK cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 \| tee /tmp/mod26-m2-final.log; ! grep -n SIGABRT /tmp/mod26-m2-final.log`; `$NOPG cargo insta test --workspace --all-features --check`; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; §8 rule 7 |
