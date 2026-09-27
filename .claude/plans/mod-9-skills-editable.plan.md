# Plan: MOD-9 milestone 3 — skills are editable and bindable

**Status: DRAFTED and FACT-CHECKED 2026-09-27; CONFIRMED by the maintainer the same day, with two overrides.**
Branch `mod-9-m3` off `68c058f`. Three verifiers ran against the tree, the lock and the Postgres 16
docs; their verdicts are in the "Verified claims" table at the end, and every falsified claim amended
the plan before the gate. D70's cost and semantics claims, D73's preview claim, D75's pin description,
D78's inference caveats, D82's `Table` claim and the T3/T4 parallel marking were all wrong in the first
draft and are corrected in place. **The maintainer's answers at the gate:** OQ-14 **overridden** — no
`globset`; the matcher is hand-written (D70 rewritten, the crate survey kept below as the reason).
OQ-15 **overridden** — the previous attempt's changed paths are back in scope, and because
`run_step_commit` stores only hashes the derivation lands on the `Isolator` seam rather than a store
reader (D87, D89). OQ-16 through OQ-20 stand as drafted.
restart at **T1**. The PRD's gate decisions are cited as **PRD D1…PRD D6**.

**Source**: `.claude/prds/mod-9-skill-library-templates.prd.md`, milestone 3 (Delivery Milestones
table, row 3): "Skill writers, `Skills` view with library, editor, diff, bindings matrix by project
and phase, token estimate — in the shape ANA-22 decides; plus the language map, glob matcher and
clone gap (storage moved to milestone 2)." Requirements `R-SKL-3` (create, edit, version diff, bind,
unbind), `R-SKL-1` (the library is `name` + `description` + versioned body), `R-SKL-2` as amended
(activation lives on the attachment), `R-TUI-7` (library, editor, version diff, bindings matrix by
project and phase), `R-PRM-4`'s editor sibling (`R-NF-3`), `R-ID-5` (the match is recorded, the
injected bytes are digested), `R-STO-5` (forward-only).

**Design source**: `docs/ANA-22.md` §6 items 1–9, §7.1–§7.3, §8 (milestone 3's list), §9 (the open
points this plan closes), as amended by §10's 2026-09-26 line, which moved the migration, the model,
the three-level resolution and `select` into milestone 2.

**Complexity**: High. Four new `WriteStore` methods on both stores, four new `StoreRequest`/`StoreReply`
variants, a new glob matcher (one new dependency), the F2 file set made reachable from the assembler,
two new `ChoiceReason` variants and a new `SkillChoice` key, one forward migration that only restates
a comment, a new `WriteStore`-visible field on `NewStepGraph`, and two new UI modules. **No new
`ReadStore` method, no new table, no new column.**

**Routing**: continues `/handoff-run MOD-9` (PRD path, criteria C2 and C4; the PRD is already
APPROVED, so `plan-prd` is satisfied and this is the `plan` step). Ultracode was recommended for
`implement` and `review` and the maintainer accepted the route. Staffing: session model for every
step, `rust-reviewer` (`.claude/workflow-config.json`) as the review gate.

**Gortex note**: `graphify-out/` does not exist. Every tree fact in this plan was read with Gortex
against `68c058f` and carries a `file:line`; the "Verified claims" table is filled by the fact-check
step, not by this draft.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts, so implementation is not blocked.

- [x] **OQ-14 — Which glob matcher. Answered by the maintainer 2026-09-27: hand-written, no crate.**
  `crates/htui-core/src/prompt/glob.rs` implements the dialect D71 writes down (D70). The fact-check had
  already surveyed the field, and that survey is why the answer is comfortable rather than a shrug:
  **`globset 0.4.20` is one new package** (all four of its non-optional dependencies are already in the
  736-package lock, so nothing else is dragged in; MSRV 1.88 against the workspace's pinned 1.98), and
  **`*` matches across `/` in globset by default** — every pattern would need
  `GlobBuilder::new(p).literal_separator(true).backslash_escape(false)`, and `backslash_escape`'s default
  is platform-dependent (`!is_separator('\')`, true on Unix and false on Windows). `gix-glob 0.27.1` is
  the only zero-package option and has no `{a,b}` alternation anywhere in its parser. The price is
  stated in D70: we own the syntax, and anything we do not implement is refused at save time rather
  than silently matching nothing.
- [x] **OQ-15 — The previous attempt's changed paths. Answered by the maintainer 2026-09-27: in scope.**
  This reverses the plan's first default. It does **not** become a `ReadStore` method, because
  `run_step_commit` stores hashes and no paths (`model/run.rs:585-594`); the derivation lands on
  `Isolator::changed_paths` (D89), which is cheaper than parsing the `DiffBlock` the engine already
  holds and more correct about renames. The union feeds both the matcher's file set and
  `ExcerptRequest.changed_paths`, so `TIER2_PREV_DIFF` stops being dead in production.
- [ ] **OQ-16 — How the override clone sets `is_override`.** `NewStepGraph` has no such field and
  both stores hard-code `false` (`mem.rs:2539`, the Postgres `INSERT` selects the column and never
  supplies it), so `Engine::phase_skills`'s override note cannot fire in production.
  **Default (D85): add `is_override: bool` to `NewStepGraph`**, defaulting every existing
  construction site to `false` and `override_graph` to `true`. That is the honest fix — the column
  exists, is read into every `StepGraph`, and is documented in `kind.rs:165` as "not here". The cost
  is a compile error at each construction site, which is the point, and the fact-check counted
  those sites: **9**, below the ~15 threshold at which the alternative gets cheaper.
  **Alternative:** a dedicated `WriteStore::mark_graph_override(id)` after the clone, which is a
  fifth writer and a window in which the graph exists un-flagged.
- [ ] **OQ-17 — What the override note says afterwards.** Today `OVERRIDE_SKILLS_NOTE` says an
  override's phases "carry no phase-level attachments". Once the clone copies them, the true
  statement is that they were **copied at creation and do not follow later changes to the original
  graph**. **Default (D85): new text naming the source graph.** **Alternative:** delete the note
  and the constant, on the grounds that the record already shows the attachments that applied.
- [ ] **OQ-18 — What a skill save writes.** `upsert_skill` can update `description`; the name is
  the library key and never moves. **Default (D78): one `SaveSkill` request carrying name,
  description and body, which calls `upsert_skill` then `add_skill_version`;** a save whose body is
  byte-identical to the head's still appends, exactly as the Templates view appends unconditionally
  (PRD D5's append-only rule), and the diff pane makes the no-op visible before the save.
  **Alternative:** refuse a save that changes nothing.
- [ ] **OQ-19 — Unbind.** `R-SKL-3` says "bind and unbind", and ANA-22 §8 lists "detach" as part of
  `set_skill_binding`. **Default (D76): a fourth writer, `remove_skill_binding(id, expected)`,**
  CAS on `updated_at`, so unbind is as safe as every other write and the row really goes. Writing
  `activation = off` instead would keep a row that reads as an attachment and is neither.
  **Alternative:** fold detach into `set_skill_binding` behind an enum, which makes one method do
  two things and halves the conformance surface.
- [ ] **OQ-20 — Skill name rule.** ANA-22 §6 item 12 adopts the Agent Skills rule: `[a-z0-9-]`,
  1–64, no leading/trailing hyphen, no double hyphen, checked by the writer. **Default (D77): the
  writer refuses**, in the same sentence on both stores, beside `invalid_template_name`
  (`traits.rs:1464`), as `Skill::name_is_valid` mirroring `PromptTemplate::name_is_valid`
  (`kind.rs:359`) and `ItemKind::prefix_is_valid` (`kind.rs:80`). **Alternative:** accept any
  trimmed non-empty single-line name, and validate only on import.

---

## Summary

Today a skill is a name, a description and a versioned body that nothing can write, show or bind.
The three tables exist (`skill`, `skill_version`, `skill_binding`), the read side is
milestone 2's (three-level most-specific-wins resolution, `select`, `trim_record.skill_choices`),
and the UI is a one-line stub. Milestone 3 adds the write side, the matcher that makes
`activation = glob` real, and the Skills view.

**Writers (T1).** Four `WriteStore` methods — `upsert_skill`, `add_skill_version`,
`set_skill_binding`, `remove_skill_binding` — implemented on `MemStore` (`State` methods over
`skills` / `skill_versions` / `skill_bindings`) and on `PgStore` (one-statement
`INSERT … SELECT … WHERE … IS NOT DISTINCT FROM $n ON CONFLICT … DO NOTHING RETURNING …`, then
`cas_miss`), dispatched by `Writer`, spied by `UsageSpy` and `SpyStore`, and pinned by four
conformance cases. Then the worker seam: a new `crates/htui/src/skills.rs`, four `StoreRequest`
variants, two `StoreReply` variants, one or-ed `try_serve` arm.

**Matching (T2).** The excerpt walk's **enumerated file set** becomes reachable
(`ExcerptSet.listed`), a new `htui_core::prompt::glob` module turns `globs` plus that set into one
matched path per candidate, the engine fills `PromptSpec.skill_matches` in `with_excerpts`, and
`model::skill::select` decides `matched` / `no_match` / `no_path` instead of always `no_path`. Two
new `ChoiceReason` variants, one new `SkillChoice` key, and migration `0008`, which changes no
column and only re-issues the `run_step.trim_record` comment with the new reason vocabulary.

**Skills view (T3, then T4 — the fact-check demoted the parallel wave; see "Intersections").** `ui/tabs/skills/library.rs` is the library list, the version browser, the
line diff, the `TextArea` + `$EDITOR` editor and the parse-free save; `ui/tabs/skills/matrix.rs` is
the attachments matrix (a global row above the projects and the phases), the activation form, the
language→globs expansion shown before save, the repo picker for a qualified glob, and the per-skill
token estimate. The tab's `Skills │ Templates` switch keeps its keys and the strip text is untouched.

**Clone gap (T5).** `NewStepGraph` gains `is_override`; `override_graph` sets it and copies each
phase-level attachment onto the cloned phase with the new writer; the engine's override note says
what is actually true.

---

## Design decisions (settled here, not in code review)

| # | Decision | Why / evidence |
|---|---|---|
| D70 | **The matcher is hand-written, in `crates/htui-core/src/prompt/glob.rs`. No new dependency: `Cargo.lock` stays at 736 packages and no `Cargo.toml` in the workspace is edited.** `pub fn compile(pattern: &str) -> Result<Pattern, GlobError>`, `Pattern::matches(&self, path: &str) -> bool`, and the two entry points D71 names. | **The maintainer's OQ-14 answer, 2026-09-27: no `globset`.** The fact-check had already measured the alternative — `globset 0.4.20` is one new package, MSRV 1.88 against our 1.98 — and found the trap that decided it: `*` matches across `/` in globset by default, so every pattern would have to go through `GlobBuilder::new(p).literal_separator(true).backslash_escape(false)`, and `backslash_escape`'s default is platform-dependent (`!is_separator('\\')`). We would be configuring away two defaults to get the dialect ANA-22 §6 item 7 already describes. The surface we need is five metacharacters over `/`-separated paths, the test table for it is written either way (T2), and `gix-glob` — the only zero-package option — has no `{a,b}` alternation at all. **The price we accept, stated once:** we own this syntax forever, and a pattern we do not implement is **refused at save time** (D78) rather than silently matching nothing. |
| D71 | **The matcher lives in `crates/htui-core/src/prompt/glob.rs`**, not in `model::skill`. `pub fn compile(pattern: &str) -> Result<Pattern, GlobError>` and `pub fn matched_skills(skills: &[BoundSkill], files: &[RepoPath]) -> BTreeMap<SkillId, String>`, where the map's value is the matched path rendered `<repo>:<path>`. A `<repo>:` qualifier matches only that repo; a bare glob matches in any repo. A pattern `compile` rejects is **not** an error at match time: `matched_skills` skips it and the writer refuses it at save time (D78), so the two can never disagree about the same string. | `RepoPath { repo, path }` (`excerpt.rs:102`) is the type the walk already produces and it lives under `prompt`, so a matcher over it belongs there too. Keeping the matcher out of `model::skill` keeps that module pure (D51's rule). `matched_skills` is one call from two places (engine and preview), so the qualified-glob rule is written once. **The dialect, normatively ours** (over repo-relative `/`-separated paths): `?` is one non-`/` character; `*` is any run of non-`/` characters and **never crosses `/`**, so `*.rs` matches `main.rs` and not `crates/main.rs`; `**` matches zero or more whole components and is legal only as a whole component (`**/`, `/**`, `/**/`) — anywhere else `compile` refuses; `{a,b}` alternates and nesting is refused; `[abc]`, `[!abc]`, `[a-z]` are character classes; `\` escapes the next metacharacter; a leading `!` is **not** negation and is refused, as is a trailing `/`; matching is case-sensitive everywhere. |
| D72 | **`ExcerptSet` gains `pub listed: Vec<RepoPath>`,** filled by `excerpt::select` from the `listing` it already builds (`excerpt.rs:1090-1170`) after the `skip_by_path` and `is_repo_relative` filters. `no_excerpts` (`engine.rs:5822-5835`) and every literal `ExcerptSet` in tests set it empty. | ANA-22 §5.4 F2's file set is exactly the walk's listing, and the walk already applies `.git`, the secret denylist, `.gitignore`, binary, size and lockfile skips in both the ranker and the reader. Re-walking (option (a) of the research) would double the filesystem cost per step for no new data. `listed` is a superset of the selected excerpts, which is the point: a skill can activate on a file that was too big to excerpt. |
| D73 | **`PromptSpec` gains `pub skill_files: Vec<RepoPath>` (the walk's listing unioned with the previous attempt's changed paths, D89) and `pub skill_matches: BTreeMap<SkillId, String>`,** filled in `Engine::with_excerpts` (`engine.rs:4936-4980`) from `spec.skill_files` and `spec.skills`, **and in `preview::build`, which already runs the same walk** (`preview.rs:308`, `spec.excerpts = excerpts_for(…)` immediately before `assemble`). `model::skill::select` takes `matches: Option<&BTreeMap<SkillId, String>>` — `None` means "no file set resolved", which is what `no_path` means. | **Amended by the fact-check:** this plan's first draft assumed the preview ran no walk and recorded `no_path`; it does run one, so the preview records `matched` / `no_match` like a real step, and `SKILLS_NOTE` flips from "a glob attachment records no_path until glob activation lands" to the sentence that names what the preview does. `with_excerpts` and `preview::build` are the two places that have roots, a walk and the candidates at the same time. The judge and the handoff keep `no_excerpts` (`engine.rs:5822-5835`) and therefore record `no_path`, which is the truth: neither reads a file. `Option` rather than an empty map, because "the walk ran and matched nothing" and "there was nothing to walk" are different records. |
| D74 | **`ChoiceReason` gains `Matched` and `NoMatch`; nothing is renamed.** `SkillChoice` gains `pub matched: Option<String>`, so its pinned key set goes from seven to eight (`skill, name, version, level, activation, active, reason, matched`) and `choices_serialize_their_documented_keys` (`skill.rs:772-810`) moves deliberately. `select`'s active test widens from `reason == Always` to `matches!(reason, Always \| Matched)`. `select_applies_its_rules_in_order` (`skill.rs:726-770`) gains a `Glob` case. | `model/skill.rs:250-254` promises exactly this: "MOD-9 milestone 3 adds `matched` (with the path) and `no_match`; no variant here is renamed then." `no_path` stays, because it is still reachable in the judge, the handoff and any step whose roots do not resolve. |
| D75 | **Migration `0008_skill_match.sql` changes no column and no constraint.** It re-issues `COMMENT ON COLUMN run_step.trim_record` with the new reason vocabulary (`always`, `matched`, `no_match`, `off`, `no_path`, `missing_version`, `not_placed`) and nothing else. `tests/migrations.rs` moves, and the moves are exactly these: the `vec![1, 2, 3, 4, 5, 6, 7]` literal at `:81` gains an `8` and its message names `0008`; `Pending(7)` / "seven embedded migrations" at `:877-878` become `8` / "eight" (also pinned twice in `tests/connect.rs:102` and `:204`); and **the pinned comment text at `migrations.rs:208-216` moves to the new wording byte for byte** — that literal is the guard `the_ana_column_comments_are_present_and_verbatim` (`:417`) exists to enforce, and `0007:40-46` is where the old one is written. `TABLES` stays 39 (`:101-105`) and the commented-column total stays **34** (`:495-498`), which is derived as `ANA_COLUMN_COMMENTS` 25 + `MOD7_COLUMN_COMMENTS` 4 + `MOD9_COLUMN_COMMENTS` 5 and is *not* affected by a comment-only migration — the test's own message forbids "a half-finished thirty-fifth". | `PgStore::connect` refuses a database that applied an older text of an existing migration: `schema_state` at `pg/mod.rs:584-627` compares checksums and returns `checksum_drift(version)` — `"migration 7 was applied with a different checksum"` (`error.rs:107-109`) — which is why editing `0007` in place is not available. `0001_init.sql:9-11` states the rule and `migrations.rs:913-929` pins the refusal. The precedent for a comment-only migration is `0007` itself, which re-issued this same comment for D42. |
| D76 | **Four writers, on `WriteStore` (79 → 83 methods):**<br>`async fn upsert_skill(&self, new: NewSkill, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<Skill>>`<br>`async fn add_skill_version(&self, new: NewSkillVersion, expected: Option<i32>) -> Result<CasOutcome<SkillVersion>>`<br>`async fn set_skill_binding(&self, new: NewSkillBinding, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<SkillBinding>>`<br>`async fn remove_skill_binding(&self, id: SkillBindingId, expected: DateTime<Utc>) -> Result<CasOutcome<SkillBinding>>` | Writers go on the trait because they must be reachable through `Writer` (`writer.rs:705-714`), unlike the reads, which stay inherent on `MemStore`/`PgStore` and are dispatched by `Backend` because the skill tables are not mirrored (`backend.rs:336`). `expected` is the row's `updated_at` for the two mutable tables and the head version for the append-only one, exactly as `update_item_kind` and `append_prompt_template` already do. `remove_skill_binding` is a fourth rather than a mode of the third, so unbind is its own conformance case and its own reply. |
| D77 | **Refusal helpers beside the template ones in `traits.rs` (`:1464-1492`): `invalid_skill_name(name)` in the Agent Skills rule (`[a-z0-9-]`, 1–64, no edge or double hyphen), `skill_key(name)`, `skill_binding_key(skill, project, phase)`, and `skill_refusal(name, description) -> Option<String>` (name first, then a NUL in either, since Postgres `text` cannot hold `U+0000`).** `NewSkill`, `NewSkillVersion`, `NewSkillBinding` are newtype-free structs beside `NewPromptTemplate`. | `prompt_template_refusal` is the shape to copy, including its NUL rule, which exists because `22021` is a Postgres error a `MemStore` would never produce and the two stores must agree. ANA-22 §6 item 12 puts the name rule "checked by the writer, not a constraint". |
| D78 | **`set_skill_binding` is an upsert on `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`, and the CHECKs are mirrored in Rust before the statement runs.** Refused in Rust, in this order: the token (row read, `Stale` or `NotFound`); a `phase_id` without a `project_id`; `activation = glob` with empty `globs`; a repo-qualified glob on a **global** row (ANA-22 §6 item 6: a global attachment cannot name a repo); **a glob `glob::compile` rejects** (a `**` that is not a whole component, a nested `{…}`, an empty alternate, a leading `!`, a trailing `/` — the writer is the only place a bad pattern is refused, and it is where the maintainer sees the message, with the position); a `pinned_version` the skill does not have. Postgres mirrors it with `ON CONFLICT (skill_id, project_id, phase_id) DO UPDATE SET … WHERE skill_binding.updated_at = $n RETURNING …`, then one `cas_miss` read. | The CHECKs are `23514` on Postgres and absent on `MemStore`, so a Rust refusal is what makes the two stores agree. The qualified-glob rule has no CHECK and no column — it is purely the writer's, and therefore purely Rust, on both backends. **Inference, confirmed against the Postgres 16 docs by the fact-check:** a bare column list may be used to infer an arbiter index over a `UNIQUE NULLS NOT DISTINCT` index — the inference rule is stated in terms of columns and expressions only, and "without regard to order"; the set must match exactly, and a *partial* index would need an explicit `index_predicate`. `skill_binding`'s key is a non-partial table constraint (`0001_init.sql:440`), so the bare list suffices, and the 16 release note that disallows `NULLS NOT DISTINCT` **primary keys** does not apply to a `UNIQUE` constraint. A duplicate under this key is `23505` (unique violation), not `23514`; `tests/skill_attachments.rs:414` already asserts the constraint name `skill_binding_skill_id_project_id_phase_id_key`. |
| D79 | **`State` gains four methods; the three collections keep their shapes** — `skills: HashMap<SkillId, Skill>` (single-column key), `skill_versions: Vec<SkillVersion>` and `skill_bindings: Vec<SkillBinding>` (composite/nullable keys, so filtered, never indexed). `project_reach` and `delete_project` are unchanged: a global attachment survives a project delete, and `DeleteReach.skill_bindings` still counts only the project's own rows. | The map/vec split is the module's own discipline (`mem.rs:114-122`). `delete_project`'s `retain(|row| row.project_id != Some(id))` (`mem.rs:3317`) already keeps globals, and conformance case `project_delete_takes_everything_and_says_so` pins the count `3`. |
| D80 | **Four conformance cases (77 → 81), registered by hand in both `CASES` and `run_case`, with `EXPECTED_CASES` in `crates/htui-store/tests/pg_conformance.rs` moved to 81.** `skill_upsert_creates_then_edits_under_the_updated_at_token`, `skill_version_append_is_a_cas_on_the_head`, `skill_binding_upsert_replaces_its_own_row_and_a_spent_token_is_stale`, `skill_binding_refuses_what_its_checks_refuse`. Each uses the existing `applied` / `stale` helpers, never `.unwrap()`. | There is no case macro: `CASES` is a `&[&str]` and `run_case` is a hand-written `match` with a `panic!` wildcard, so a one-sided edit fails loudly. Case 3 for templates is the model for case 4 here, including the "a save at the head after the refusals lands, so none of them wrote" tail. |
| D81 | **The worker seam is `crates/htui/src/skills.rs`, mirroring `crates/htui/src/templates.rs`.** `REQUEST_NAMES: [&str; 4] = ["skills", "save_skill", "set_skill_binding", "remove_skill_binding"]` with `READ_NAME`, `SkillsSnapshot { skills: Vec<SkillSummary>, attachments: Vec<SkillBindingRow> }`, a `SkillBody(String)` newtype whose `Debug` prints its length, `snapshot(backend, scope)` and `serve(backend, request)`. `StoreRequest` 69 → 73, `StoreReply` 40 → 42 (`Skills`, `SkillsStale`). One new **or-ed** `try_serve` arm. | `templates.rs` is the whole shape, including the refusal arm that names the request that got here by mistake, the re-read-rather-than-patch rule, and `request_names_match_the_name_arms` — which the new module must also ship. The two reply variants exist because each re-reads a whole snapshot, so `Applied` and `Stale` are two variants and not a carried enum. A guard arm does not count towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12), so the arm is or-ed. |
| D82 | **Two new UI modules under the existing tab directory**: `ui/tabs/skills/library.rs` (the Skills view: library list, version browser, line diff, editor, save, token estimate) and `ui/tabs/skills/matrix.rs` (the attachments matrix, the activation form, the language expansion, the repo picker). `skills/mod.rs` keeps `h`/`l`/`[`/`]`/`Left`/`Right` for the switch and gains a per-view `captures_input()` guard (today's is at `:86`, `self.view == View::Templates && …`), so `l` is a letter inside either editor. The tab title and the strip text are unchanged. | **Amended by the fact-check:** three files use `ratatui::widgets::Table` — `settings/agents.rs:50`, `backlog/detail/documents.rs:7` and `backlog/detail/graph.rs:11` — not one as this plan's first draft said. All three have a **row** cursor and none has a selected *cell*, so D82's point stands on the narrower and correct claim: the attachments matrix is a `format!`-packed row per skill in a bordered `Block` (the Templates view's list pattern, `templates.rs:876-949`) with the existing right-hand `pane()` carrying the selected row's detail. `diff::unified` / `diff::lines` are reused for skill versions exactly as for templates. |
| D83 | **The language→globs map is data**: `crates/htui-core/src/model/language.rs` with `pub const LANGUAGE_GLOBS: &[(&str, &[&str])]` seeded with `rust`, `c`, `cpp`, `python`, `typescript`, `javascript`, `go`, `java`, `csharp`, `shell`, `sql`, `markdown`, `toml`, `yaml` (ANA-22 §9's list). The activation form shows the **effective globs** — typed globs unioned with every named language's expansion — before the save, and the save writes that union into `skill_binding.globs`, with `languages` keeping what was typed. | ANA-22 §6 item 5: the map is data expanded at save, so a later map change never changes a saved attachment, and the matcher reads only `globs`. The precedent for "a spec as data in this repo" is MOD-7's `htui_agent::box_probe` spec. |
| D84 | **The per-skill token estimate is `TokenEstimator::DEFAULT.estimate(&version.body)`, computed in `render`.** The row shows the number next to the version, with the estimator id in the pane title. | `TokenEstimator` is `Copy`, `estimate` is a pure function of one `&str`, and no call site estimates a bare skill today (every caller estimates an assembled section). A 30-row list is 30 short scans of strings already in the snapshot, and it does not touch the store, so `R-NF-3` holds. The estimator's own rule is "one id never carries two arithmetics", which is safe here because the bodies are the same bytes the assembler would measure. |
| D85 | **The clone gap, closed.** `NewStepGraph` gains `pub is_override: bool`; `MemStore::State::create_step_graph` and `PgStore::create_step_graph` insert it; `override_graph` sets it `true` and, after each `create_phase`, copies that phase's level-`Phase` attachments with `set_skill_binding` (new ids, same skill, `pinned_version`, `position`, `activation`, `globs`, `languages`; `languages` are display-only so they copy verbatim). `OVERRIDE_SKILLS_NOTE` gets new text: the override's phase attachments were copied from `<graph>` when it was made and do not follow later changes there. The test `override_clone_leaves_bindings_alone` (`graph.rs:1459`) is replaced by `override_clone_copies_phase_attachments`, and `an_override_graph_notes_the_clone_gap` (`engine.rs:12924`) keeps its hand-patched snapshot but expects the new sentence. | The current test's rationale — "a phase binding keyed `UNIQUE NULLS NOT DISTINCT` would double every project binding the item's phases inherit" — is about *project* rows, which the clone still does not copy; a phase row copied onto the cloned phase is the point of an override. Today the engine's note is unreachable in production because both stores hard-code `false` (`mem.rs:2539`, and the Postgres `INSERT` at `.sqlx/query-dc01b34…`), which is the defect HANDOFF names. The copy is done with the new writer rather than a new bulk path, so it is one code path and one conformance case. |
| D86 | **No new `ReadStore` method.** The library and attachment reads are inherent on `MemStore` / `PgStore` and dispatched by `Backend` (`skill_library()`, `skill_attachments(project, phase)`), so the cache mirror is untouched and `READ_CASES` stays 14. | The skill tables are absent from `MIRRORED_TABLES` (`cache/mod.rs`, `cache_migrations/0001_mirror.sql:24`), which is the stated reason `bound_skills` is inherent. A trait read would promise an offline answer the cache cannot give. |
| D87 | **The previous attempt's changed paths are back in scope** — the maintainer's OQ-15 answer, 2026-09-27, reversing this plan's first default. The matcher's file set is `ExcerptSet.listed ∪ changed_paths`, and `ExcerptRequest.changed_paths` is filled for the first time in production, which makes the excerpt ranker's `TIER2_PREV_DIFF` live rather than dead. | ANA-22 §6 item 7 names this half of the file set, and the excerpt walk was built with the input for it — the field exists (`excerpt.rs:403`) and only the producer was missing. The cost is one extra `git diff --name-only` per step (D89), against a walk that is already running for the same step. |
| D89 | **The changed paths come from the `Isolator` seam, not from a new `ReadStore` method.** `Isolator::changed_paths(&self, trees: &[RunStepTree], commits: &[RunStepCommit]) -> IsolatorFuture<Vec<RepoPath>>` alongside the existing `Isolator::diff` (`isolate.rs:192`), implemented in `GixIsolator` with the same plumbing `diff_of` uses (`isolate/real.rs:1287-1451`) and scripted in `FakeIsolator` (`fake.rs:576`). `Engine::with_excerpts` calls it once, unions the result into `spec.skill_files`, and hands the same vector to `ExcerptRequest.changed_paths`. | **This is the maintainer's answer, reshaped by the tree.** `run_step_commit` stores `before_hash` / `after_hash` and no paths (`model/run.rs:585-594`), so a store reader would have to re-derive paths from a rendered `DiffBlock` — and `DiffBlock`'s own doc (`prompt/mod.rs:157-159`) says re-deriving a stat from the unified text "would be a parser in the assembler". Asking git is both cheaper (no parsing, no second source of truth) and more correct: git knows renames and binary files, a diff-text parser guesses. `READ_CASES` stays 14 and no store table changes. |
| D88 | **Scope fence.** Milestone 3 does not do: `SKILL.md` import (milestone 4, and ANA-22 §5.7's hand-written frontmatter reader), model-decided activation (ANA-22 §6 item 10, needs MOD-11), agent help while editing (MOD-55), export, delete of a skill or a version, syntax highlighting, a Settings section (PRD D1: the Skills tab is the only home), or a workspace-level attachment (ANA-22 §6 item 2 rejected it). | Each is named in the PRD, ANA-22 or an open HANDOFF item, and taking one now would pull its dependencies with it. |

---

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A compare-and-set writer, end to end | `append_prompt_template` | `traits.rs:722-745`, `mem.rs:2714-2768` / `:5537-5544`, `pg/write.rs:2439-2502`, `writer.rs:705-714`, `conformance.rs:5727-5978` |
| A CAS on a mutable row's `updated_at` | `update_item_kind` | `pg/write.rs:2031-2110` |
| A refusal in the same sentence on both stores | `prompt_template_refusal` + `invalid_template_name` | `traits.rs:1464-1492` |
| Re-reading to classify a miss | `cas_miss` | `pg/write.rs:75-89` |
| A pure resolution shared by both stores | `resolve` / `collapse` / `select` | `model/skill.rs:217-360` |
| A text enum with the DB spelling | `str_enum!` | `model/mod.rs:25-70` |
| A store worker module | `crates/htui/src/templates.rs` whole | `snapshot`, `serve`, `REQUEST_NAMES`, `TemplateBody` |
| An exhaustive `WriteStore` wrapper | `UsageSpy`, `SpyStore` | `htui-agent/src/conformance.rs:936-942`, `htui-agent/tests/recorder.rs:631-637` |
| A list + detail pane in the TUI | `TemplatesView::render_browse` | `ui/tabs/skills/templates.rs:876-949` |
| A warn-once gate before a write | the `omits_item` confirm | `templates.rs:687-723` |
| A version diff | `diff::unified` / `diff::lines` | `ui/diff.rs:20-97` |
| Test-only SQL that adds nothing to `.sqlx` | `sqlx::query`, unchecked | `htui-store/tests/skill_attachments.rs:38-88` |

---

## Tasks

**T1 → T2 → T3 → T4 → T5**, all serial. T1 adds trait methods every other store impl must satisfy,
so it lands whole. T2 changes types the assembler and the engine compile against, so it lands
whole. **T3 and T4 were drafted as parallel and the fact-check demoted them** (see "Intersections"):
both need a `mod` line in `ui/tabs/skills/mod.rs`, and the file cannot hold `mod matrix;` before
`matrix.rs` exists, so one of the two must land first. T5 lands last because it rewrites a test
whose reasoning T1's writer changes.

| Task | Files (complete list) | Order |
|---|---|---|
| T1 | `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/src/model/kind.rs` (the `NewSkill*` structs, beside `NewPromptTemplate` at `:359`'s neighbour `name_is_valid`), `crates/htui-core/src/model/skill.rs` (the `name_is_valid` helper and doc amends only), `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/pg/read.rs` (the per-row re-reads), `crates/htui-store/src/writer.rs`, `crates/htui-store/src/backend.rs`, `crates/htui-store/.sqlx/` (+N), `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/skill_attachments.rs` (new cases), `crates/htui-store/tests/skill_binding_cas.rs` (new, the concurrent-CAS proof, modelled on `prompt_template_cas.rs`), `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`, `crates/htui/src/skills.rs` (new, `pub mod skills;` at `crates/htui/src/lib.rs:28`), `crates/htui/src/store_worker.rs` | first |
| T2 | `crates/htui-core/src/prompt/glob.rs` (new), `crates/htui-core/src/prompt/mod.rs` (the module, `PromptSpec` at `:71-123`, `scrubbed_inputs`), `crates/htui-core/src/prompt/excerpt.rs` (`ExcerptSet` at `:1463`, the `listing` local at `:1085`), `crates/htui-core/src/model/skill.rs` (`select` at `:314`, `ChoiceReason` at `:258`, `SkillChoice` at `:289`), `crates/htui-core/tests/prompt_skills.rs`, `crates/htui-core/tests/prompt_digest.rs` (the eight keys), `crates/htui-store/migrations/0008_skill_match.sql` (new), `crates/htui-store/tests/migrations.rs`, `crates/htui-store/src/pg/demo.rs` (only if a `.sqlx` query text moves), `crates/htui-orch/src/engine.rs` (`with_excerpts` at `:4936`), `crates/htui-orch/src/isolate.rs` (the trait, `:192`), `crates/htui-orch/src/isolate/real.rs` (`GixIsolator::changed_paths`, beside `diff_of` at `:1287`), `crates/htui-orch/src/fake.rs` (`FakeIsolator`, `:576`), `crates/htui/src/preview.rs` (`build` at `:155`, the walk at `:308`), `crates/htui/tests/prompt_preview.rs` | after T1 |
| T3 | `crates/htui/src/ui/tabs/skills/library.rs` (new, `mod library;` at `skills/mod.rs:8`), `crates/htui/src/ui/tabs/skills/mod.rs` (the `mod` line, the per-view guard at `:86`, `wants_requests` at `:75`), `crates/htui/tests/skills.rs` (new), `crates/htui/tests/snapshots/skills__*.snap` (new) | after T2 |
| T4 | `crates/htui/src/ui/tabs/skills/matrix.rs` (new, `mod matrix;` added to `skills/mod.rs` by T4 itself), `crates/htui-core/src/model/language.rs` (new, `pub mod language;` at `model/mod.rs:88`), `crates/htui-core/src/model/mod.rs`, `crates/htui/tests/skills_matrix.rs` (new), `crates/htui/tests/snapshots/skills_matrix__*.snap` (new) | after T3 |
| T5 | `crates/htui-orch/src/graph.rs` (`NewStepGraph` literal at `:410`, `override_graph` at `:399`, the clone test at `:1459`), `crates/htui-core/src/store/mem.rs` (`create_step_graph`, `is_override: false` at `:2539`), `crates/htui-store/src/pg/write.rs` (`create_step_graph`), `crates/htui-store/.sqlx/` (one file replaced), `crates/htui-orch/src/engine.rs` (`OVERRIDE_SKILLS_NOTE` at `:88`) | last |

**Intersections, checked by hand, and re-checked by the fact-check.** T1 ∩ T2 = {`model/skill.rs`}:
T1 amends its module doc and adds `Skill::name_is_valid`; T2 rewrites `select` and the two choice
types. T1 lands first, T2 second. **T3 ∩ T4 = ∅ as file *content*, but they share
`crates/htui/src/ui/tabs/skills/mod.rs` as a *sequencing* dependency**: T3 adds `mod library;` and
T4 later adds `mod matrix;` to the same file, and a `mod matrix;` committed before `matrix.rs`
exists does not compile. The two therefore run **serially**, T3 then T4, not in parallel worktrees.
(If the maintainer prefers the parallel wave, the price is one stub file: T3 would commit an empty
`matrix.rs` with a `//` comment and T4 would fill it — this plan does not propose that, because a
placeholder commit is a worse thing to read than a slower wave.) T4's `model/mod.rs` edit is
`pub mod language;` plus one `pub use` arm, and no other task in the wave touches it. T5 ∩
everything = ∅ except `graph.rs` and `create_step_graph`, which T1 and T2 do not touch.

**Build coupling.** T2 changes `select`'s signature, so the assembler moves in the same commit as
the change — `model::skill::select` has one non-test caller in-tree (`prompt/mod.rs:861`) plus
`crates/htui-store/tests/skill_attachments.rs:19`; the engine and the preview only fill the new
field. T5 changes `NewStepGraph`, and the fact-check counted **9** construction sites
(`graph.rs:410`, `catalogue.rs:190`, `conformance.rs:3254` and `:3264`, `htui-orch/src/conformance.rs:824`
and `:884`, `engine.rs:6152`, `tests/fixtures.rs:54`, `tests/gix_isolator.rs:338`) — below the
threshold at which OQ-16's alternative would be cheaper, so D85's field stands. The five
`WriteStore` implementors forward the struct by value and do not break.

Every implementer prompt carries these rules:

- The PRD's gate decisions, ANA-22's verdict, this plan's D70–D88 and the maintainer's OQ answers
  win over prose.
- Explore the code with Gortex (`read`, `search`, `relations`, `trace`), not with `grep`/`Read`.
- No test is skipped or loosened to get green; a moved pin names its reason in the assertion message.
- Every new `pub` item has a doc comment and a `Debug`; a body crossing the store boundary goes in
  a newtype whose `Debug` prints its length, never its text.
- Commit the red tests first, then green.
- Gate command: `RUST_BACKTRACE=0 cargo test -p <crate> --all-features -- --test-threads=2`, plus
  `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=…` for `htui-store` and `htui`.

### Task 1: the writers and the worker seam (D76–D81, D84's estimate is T3's)

- **Tests first.**
  - `conformance.rs`, four cases (D80), each with its `CASES` entry and its `run_case` arm:
    `skill_upsert_creates_then_edits_under_the_updated_at_token` (a save with `None` creates at
    `now`; a second save with the first row's `updated_at` moves the description; a third with the
    stale timestamp is `Stale` carrying the current row),
    `skill_version_append_is_a_cas_on_the_head` (v2 at the head, `Stale` from a spent token, `None`
    on a new skill starts at 1, and `Some(0)` on a skill with no versions is `NotFound` — the F-B
    regression the template case pins),
    `skill_binding_upsert_replaces_its_own_row_and_a_spent_token_is_stale` (the same
    `(skill, None, None)` twice is one row, then a project row, then a phase row; the unique key's
    NULL semantics are the whole test),
    `skill_binding_refuses_what_its_checks_refuse` (phase without project; `glob` with empty globs;
    a qualified glob on a global row; **a glob `GlobBuilder` rejects** — `**` in an illegal position,
    a nested `{a,{b,c}}`, an empty alternate `foo{,.txt}`; a pin the skill has no version for; a NUL
    name; and after every refusal a good save at the same token still applies, so none of them wrote).
  - `traits.rs` unit tests: `a_skill_name_follows_the_agent_skills_rule` over `[a-z0-9-]`, length
    1 and 64, leading/trailing/double hyphen; `skill_refusal` prefers the name over the NUL.
  - `mem.rs`: `a_global_attachment_survives_project_delete` (the count stays 3), and the three
    `State` collections keep their shapes after a write.
  - `tests/skill_binding_cas.rs` (Postgres, new): two bindings written at one `updated_at` produce
    one row, the second answered `Stale`, using a transaction held open by hand exactly as
    `prompt_template_cas.rs` does. Unchecked `sqlx::query`, so it adds nothing to `.sqlx`.
  - `tests/skill_attachments.rs`: the new cases over planted rows, including that a `glob` row with
    empty globs is refused by the writer and not only by the CHECK.
  - `crates/htui/src/skills.rs`: `request_names_match_the_name_arms` (the `templates.rs` test, same
    shape), and `serve` refuses a foreign request by name.
- **Action.** D76–D81, D86. Regenerate `.sqlx` against a **scratch database migrated from zero to
  `0007`** — the compose `htui` database is empty, and the prepare DSN is not the test DSN.
- **Validate.** Gates for `htui-core` and `htui-store` (Postgres); `cargo sqlx prepare --check`;
  `ls crates/htui-store/.sqlx | wc -l` recorded in the commit; `cargo build --workspace
  --all-features --all-targets` so every exhaustive `WriteStore` impl is forced to grow its arms.
- **Pins that move here:** `WriteStore` 79 → 83, `CASES` 77 → 81, `EXPECTED_CASES` 81,
  `StoreRequest` 69 → 73, `StoreReply` 40 → 42, `.sqlx` 268 → 268+N. `READ_CASES` stays 14.

### Task 2: the matcher, the record and the two callers (D71–D75, D87)

- **Tests first.**
  - `prompt/glob.rs` unit tests over the dialect D71 writes down — this table is now the *specification*,
    not a cross-implementation check:
    `**/*.rs` matches `crates/x.rs`; `*.rs` does not match `crates/x.rs` and `src/*.rs` does not
    match `crates/x.rs`; `{a,b}/x.rs` matches `a/x.rs`; `[abc].rs` matches `b.rs` and not `d.rs`;
    `htui:**/*.rs` matches only repo `htui`; a bare glob matches every repo; `src/` does not match
    `src2/x.rs`; an unparsable glob (`**/[`) matches nothing rather than panicking; an empty `globs`
    list matches nothing.
  - `model/skill.rs`: `select_applies_its_rules_in_order` gains the `Glob` case; a new
    `a_glob_skill_records_matched_with_its_path`; a new
    `a_glob_skill_records_no_match_when_the_file_set_ran_and_missed`;
    `choices_serialize_their_documented_keys` moves to eight keys and says so in the message.
  - `prompt_skills.rs` (integration, over `prompt::fixtures`): a skill on `**/*.rs` assembles with
    the excerpt set and records `matched` with the path; the same skill with an excerpt set that
    misses records `no_match` and renders nothing; `no_excerpts` still records `no_path`; an
    `off` skill never trips the cap; `the_digest_moves_only_when_the_active_set_moves` gains a
    glob case (toggling a matched skill's globs so it stops matching moves the digest; toggling one
    that never matched does not).
  - `excerpt.rs`: `the_listed_set_is_the_walk_after_the_skip_rules` — a `.git` file, a `.pem`, a
    lockfile and a binary never appear in `ExcerptSet.listed`, and a file too large to excerpt does.
  - `migrations.rs`: the D75 pins, and `the_migration_names_every_reason_the_record_can_carry`
    (reads `0008` with `include_str!` and asserts each of the seven reason words appears).
  - `engine.rs`: a phase step whose skills are `glob` and whose walk resolves a `.rs` file renders
    the skill; a judge step still records `no_path`; the existing `a_glob_skill_records_no_path`
    case (T2's predecessor in milestone 2) is replaced, not kept alongside.
  - `isolate.rs` / `real.rs` / `fake.rs`: `changed_paths_are_the_files_the_previous_attempt_touched` —
    `FakeIsolator` scripts them, `GixIsolator` reads them from `git diff --name-only`, and an attempt
    of 1 asks for none; `with_excerpts` records a note when the call fails and continues with the
    walk's listing alone.
  - `excerpt.rs` / the excerpt tests: `TIER2_PREV_DIFF` now fires in production for the first time —
    a step whose previous attempt touched a file that the walk did not rank still carries it.
  - `preview.rs`: the preview runs the same walk, so a `glob` skill over a `.rs` file records
    `matched` in the preview's trim record and a skill over a glob nothing matches records
    `no_match`; `SKILLS_NOTE` flips its sentence and `STAND_INS` keeps its eight entries.
- **Action.** D71–D75, D87.
- **Validate.** Gates for `htui-core`, then `htui-orch` and `htui` (the `IMPL_TRIM_RECORD` golden and
  every reader of `trim_record` keys live downstream), plus `cargo sqlx prepare --check` if
  `demo.rs` moved.

### Task 3: the Skills view (D82, D84)

- **Tests first** (`tests/skills.rs`, over `Harness::over(MemStore::demo())`, opening the tab with
  `2` and settling):
  - `the_skills_view_lists_the_library_with_versions_and_token_estimates` (snapshot)
  - `any_two_skill_versions_diff` (snapshot; `diff::unified` with `v1 → v2` labels)
  - `a_save_appends_a_version_and_moves_the_head`
  - `a_save_over_a_moved_head_keeps_the_draft` — the `TemplatesStale` shape, with the skill's
    `updated_at` as the token
  - `an_editor_keeps_the_draft_when_the_scope_changes`
  - `the_editor_hands_off_to_a_fake_editor_and_parses_on_return` (the `ExternalEditOutcome::Edited`
    path, no `parse` here: a skill body is markdown, so there is no byte-offset error to point at —
    the view shows the name and description as its own fields and the body is free text)
  - `a_new_skill_starts_at_version_one`
  - `the_six_template_snapshots_do_not_move` (an assertion over the existing files' bytes if the
    switch line or the hint strings are touched — the coupling HANDOFF warns about)
- **Action.** D82, D84.
- **Validate.** The `htui` gate, and `cargo insta` review of every new snapshot, each named in the
  commit message. The strip text ` 1 Backlog  2 Skills  3 Settings  4 Chat` and the six
  `templates__*.snap` files must not move.

### Task 4: the attachments matrix and the activation form (D82, D83, D78's UI half)

- **Tests first** (`tests/skills_matrix.rs`):
  - `the_matrix_shows_a_global_row_above_the_projects_and_the_phases` (snapshot)
  - `attaching_at_a_level_writes_one_row_and_the_matrix_re_reads`
  - `a_pin_follows_latest_until_it_is_set_and_cleared`
  - `activating_glob_without_globs_is_refused_before_it_is_sent`
  - `a_qualified_glob_is_refused_on_a_global_row`
  - `a_glob_the_matcher_cannot_compile_is_refused_before_it_is_sent` (D78) — `src/**x/*.rs`,
    `a/{b,{c,d}}/x.rs` and `x{,.txt}` are all `Constraint` on both stores, in the same sentence
  - `the_language_map_expands_into_the_effective_globs_shown_before_the_save` (snapshot; `rust` →
    its two patterns, unioned with a typed glob, in the order the union is defined)
  - `unbinding_removes_the_row_and_the_matrix_shows_it_gone`
  - `a_repo_picker_writes_the_qualifier_from_the_project_s_repos`
  - `a_spent_token_leaves_the_row_as_it_is_and_says_so`
- **Action.** D82, D83.
- **Validate.** The `htui` gate and the same `insta` review, run after T3's commit so the two
  views are never half-declared.

### Task 5: the clone gap (D85, OQ-16, OQ-17)

- **Tests first.**
  - `graph.rs`: `override_clone_copies_phase_attachments` — the cloned `implement` phase resolves
    the same attachments the original's did, the cloned phases carry no project-level rows (the
    original test's own rationale, kept as its own assertion), and the original graph's rows are
    untouched.
  - `graph.rs`: `an_override_graph_is_marked_as_one` — `store.step_graphs(project)` shows
    `is_override` on the clone and not on the seeded graphs.
  - `engine.rs`: `an_override_graph_notes_that_its_attachments_were_copied` — the hand-patched
    snapshot keeps working and expects the new sentence, naming the source graph.
  - `mem.rs`: `create_step_graph_honours_its_new_is_override_field`.
- **Action.** D85.
- **Validate.** Gates for `htui-core`, `htui-store` (Postgres) and `htui-orch`; `cargo sqlx prepare
  --check` (the `create_step_graph` `INSERT` gains a column, so its `.sqlx` file is replaced, not
  added).

---

## Test plan

**Unit.** `model/skill.rs` for the four writers' refusals and the selection rules; `prompt/glob.rs`
for the matcher table, which is the same table whichever implementation D70 lands on;
`prompt/excerpt.rs` for the listed set; `model/language.rs` for the map (every entry's patterns are
parsable, and no two entries are identical).

**Conformance.** The four new cases run on `MemStore` and, through `pg_conformance.rs`, on
`PgStore`. The Postgres-only cases add what one store cannot show: a real CHECK violation the
writer refused first, and the concurrent CAS with a transaction held open.

**Integration.** The assembler over `prompt::fixtures` (render, cap, record, digest); the engine
over the fake graph source (a matched glob renders, a judge records `no_path`); the preview (the
`no_path` note); the TUI over `testkit::Harness` (list, diff, save, stale, matrix, expand, unbind)
with `insta` snapshots named per case.

**Gates.** `cargo fmt --all -- --check`; `cargo clippy --workspace --all-features --all-targets
-- -D warnings`; the workspace test run with `--test-threads=2` and a single-threaded pass for
anything that touches the keyring fake; `cargo sqlx prepare --check`.

## Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-25 | The hand-written matcher gets a semantic detail wrong — `*` crossing `/`, a `**` in an illegal position, a class that admits a `/` — and a maintainer types a glob that quietly matches nothing. | Medium | Medium | One dialect, written down in D71 and enforced by the table in T2, which every pattern form is tested against. `compile` is total: it either yields a `Pattern` or a `GlobError` with a position, and the writer (D78) refuses the second before it is stored, so a stored glob is always a compiling one. A future syntax request is a change to one file and one test table. |
| R-35 | The changed-paths call is a second `git diff` per step, and on a repository with a large previous diff it is not free. | Certain | Low | It runs once per step, beside a walk that is already reading the same tree, and only when a `glob` attachment is in play — the engine skips the call when no candidate is `Activation::Glob`, which is the common case. Measured and recorded in the T2 commit if it shows up. |
| R-26 | `ExcerptSet.listed` is a second, larger list carried on every prompt, and `PromptSpec` grows with it — a run's memory and the golden fixtures move. | Certain | Low | `listed` holds `RepoPath` pairs the walk already built and already dropped; the cost is the vector, not a second walk. The golden record `IMPL_TRIM_RECORD` moves only if a test's skills change, and the digest does not move at all unless the active set does. |
| R-27 | A `glob` skill fires on a mixed repo for a docs-only item, because the walk saw a `.rs` file somewhere. | Medium | Medium | F2 narrows to `touched_paths` prefixes when the item declares them, and the recorded `matched` path is in `trim_record.skill_choices` and in the Prompt sub-tab, so the firing is auditable per step. |
| R-28 | `NewStepGraph.is_override` touches every construction site in the tree, and one of them is a fixture or a seed whose `false` is what a test asserts. | Certain | Low | It is a compile error at each site, so nothing is missed; the fact-check counts the sites before T5 is confirmed and the blueprint records the list. |
| R-29 | The cloned override's copied attachments go stale when the original graph's change, and nothing says so. | Medium | Low | The note says it in the record (D85), and the copied row keeps its own `id`, so a later edit of the original is visible as a difference between the two graphs in the matrix. |
| R-30 | T3 and T4 touch the same tab widget: T4's `matrix.rs` is unreachable until a `mod` line lands in T3's `mod.rs`, so the two cannot both be in flight. | Certain (found by the fact-check) | Low | They run serially, T3 then T4. T4's only shared file is one `mod` line plus one `pub use` arm, and the six `templates__*.snap` files stay byte-identical because neither task changes the switch line, the hint strings or the list widths. |
| R-31 | The Postgres demo loader writes **none** of `0007`'s four columns (its two `INSERT`s name six and seven columns respectively), while the Rust `MemStore` fixture writes **all** of them — so a fixture row added by T1 can load differently into the two stores. | Medium | Medium | The asymmetry is deliberate and already pinned: `demo_skill_rows_use_the_column_defaults` (`fixtures.rs:2037`) and `tests/skill_attachments.rs:437-446` ("the demo loader writes none of the new columns, so its three rows read the defaults"). T1's new fixture rows set every column explicitly on **both** sides, and any demo row it adds sets the demo literals equal to the column defaults. |
| R-32 | `remove_skill_binding` on a row another writer just changed deletes the winner's row. | Low | High | The token is `updated_at` and the delete is `DELETE … WHERE id = $1 AND updated_at = $2 RETURNING …`, so a spent token is `Stale` and the row survives. The Postgres race case covers it. |
| R-33 | The parallel worktrees contend on the shared target dir and on disk (the `htui` suite is large; ~10 G per worktree was measured before). | Medium | Medium | One `CARGO_TARGET_DIR`; `--test-threads=2`; delete `target/debug/incremental` on ENOSPC. |
| R-34 | `ORT_LIB_LOCATION` points at an npm copy of ONNX Runtime 1.18, because `parcel.pyke.io` is still denied. | Certain here | Low | The gate env is recorded in the PR test plan, as in milestone 2 (R-19 there). CI has no workflows in this repo. |

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- `USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features
  --no-fail-fast -- --test-threads=2`
- `cargo sqlx prepare --check` against a scratch database migrated from zero to `0007`
- `git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml` — **no dependency change at all in this milestone** (D70)
- Validator: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`
- Pins afterwards:
  - unchanged: `READ_CASES` 14, `MIRRORED_TABLES` 21, `TABLES` 39, commented columns 34
  - moved: `WriteStore` 79 → 83, `CASES` 77 → 81, `StoreRequest` 69 → 73, `StoreReply` 40 → 42,
    the applied-migration `vec![1, 2, 3, 4, 5, 6, 7]` literal at `migrations.rs:81` and the pinned
    `trim_record` comment at `:208-216`, `.sqlx` 268 → 268+N

## Acceptance

- A skill can be created, edited and version-diffed in the Skills tab, and every save appends a
  version under a compare-and-set that a second editor cannot spend.
- A skill can be attached at global, project or phase level, pinned or left following latest,
  positioned, detached, and activated `always`, `glob` or `off`; the matrix shows the global row
  above the projects and the phases, and the effective globs before a save.
- A `glob` attachment fires on a real phase step whose file set matches, records `matched` with the
  path beside the trim record, and does not fire — recording `no_match` — when the set misses.
- A `glob` attachment records `no_path` where no file set resolves (the judge, the handoff, the
  preview), and the step proceeds.
- A language named in the activation form becomes globs in the stored row, and changing the map
  later does not change a saved attachment.
- An override graph's phases carry the original's phase-level attachments, the graph says it is an
  override, and the record says the attachments were copied at creation.
- `R-SKL-3`'s five verbs — create, edit, view version diff, bind, unbind — are each a test.

## Where the PRD, ANA or tree disagree

- **ANA-22 §8** puts the migration, the model, the three-level resolution and `select` in milestone
  3. The maintainer moved them to milestone 2 on 2026-09-26 (recorded in ANA-22 §10 and in
  milestone 2's plan). This plan assumes they are landed, not that they are to be done.
- **ANA-22 §7.2** writes `collapse(global, project, phase)`; milestone 2's D39 uses one list with
  `level`. This plan inherits the one-list form.
- **ANA-22 §6 item 7** names "the previous attempt's changed paths" as part of the file set. There
  is no reader for it and this plan leaves it out (D87, OQ-15) — a narrowing, recorded here.
- **ANA-22 §9**'s open point "whether `SkillChoice` rides the trim record's existing `skills`
  section or a sibling field" was answered in milestone 2 (D42: a top-level `skill_choices`, record
  `v: 2`). This plan adds a **key**, not a field, to each entry.
- **ANA-22 §8** says the language map's seed is at least the fourteen languages named in §9; D83
  takes exactly that list.
- **`docs/ANA-5.md` §4.1's role table** (`:319-341`) and PRD D4 are dated records; D48 widened the
  judge role in milestone 2 and neither file is edited.
- **The PRD's Evidence** cites `skills.rs` as a 61-line single file and `engine.rs:4310`/`:4925`;
  the tab is a directory module and the engine's lines are `:4322`/`:4937-4939`. Left as written:
  the PRD is a dated record.

## Verified claims

*Every row below was checked on 2026-09-27 against branch `mod-9-m3` at `68c058f`, by three verifiers: one
over the tree, one over `Cargo.lock` plus the crates.io and docs.rs documentation, one over the migration
files plus the Postgres 16 documentation. A row marked **false as first drafted** is a claim this plan
got wrong and then corrected; the correction is named in the row.*

| Claim | Verdict | Evidence |
|---|---|---|
| `WriteStore` has 79 methods today | ✓ true | 79 `async fn` in the trait body, `traits.rs:259-1333`; `append_prompt_template` at `:739` |
| `CASES` is 77, `READ_CASES` is 14, `EXPECTED_CASES` is 77 | ✓ true | `conformance.rs:43-121` (77 entries), `:303-318` (14), `pg_conformance.rs:19` (**not** `:20`) |
| `StoreRequest` has 69 variants, `StoreReply` 40 | ✓ true | both enumerated from `store_worker.rs:95-584` and `:674-859` |
| The or-ed `try_serve` arm for the two template requests is at `:1126` | ✓ true | `store_worker.rs:1126` |
| `pub mod templates;` is at `crates/htui/src/lib.rs:29`, so the new worker module goes at `:28` | ✓ true | `lib.rs:29` |
| `.sqlx` holds 268 files; the skill tables are touched only by `bound_skills`, `project_reach` and the demo inserts | ✓ true | file query → 268; demo inserts `pg/demo.rs:272/288/303` |
| `globset` is absent from `Cargo.lock`; `gix-glob 0.27.1` is present | ✓ true | zero occurrences of `globset`; `Cargo.lock:2240-2248` |
| The excerpt walk's listing is a local; nothing public returns the enumerated file set | ✓ true | `let mut listing: Vec<Listed>` at `excerpt.rs:1085`, first push `:1157`, inside `select` (`:1050`); `ExcerptSet` at `:1463-1472` has three fields |
| `RepoPath` is `{ repo: String, path: String }` | ✓ true | `excerpt.rs:102-107` |
| Root order is `run_step_tree.path` → `repo_box_path` → `NoPath` | ✓ true | `htui-agent/src/excerpt.rs:837-863`; test `tests/excerpt.rs:1092` |
| No reader exists for the previous attempt's changed paths; tier 2 is dead in production | ✓ true | `traits.rs:155/161/167`; `htui-agent/src/excerpt.rs:975-977` |
| `spec.skills`, `step.attempt`, the roots and the touched prefixes are all in scope in `with_excerpts` | ✓ true | `engine.rs:4936-4980`; `assemble_prompt` calls it at `:4919` |
| `is_override` is hard-coded `false` in both stores' insert paths | ✓ true | `mem.rs:2539`; the `INSERT INTO step_graph (id, project_id, name, description)` in `.sqlx/query-dc01b34…` |
| `NewStepGraph` has four fields and no `is_override` | ✓ true | `model/kind.rs:152-161`, note at `:165`; `graph.rs:388` says the same |
| **`NewStepGraph` is constructed at 9 sites — under the ~15 threshold, so D85's field stands** | ✓ **gate passed** | `graph.rs:410`, `catalogue.rs:190`, `conformance.rs:3254`/`:3264`, `htui-orch/src/conformance.rs:824`/`:884`, `engine.rs:6152`, `tests/fixtures.rs:54`, `tests/gix_isolator.rs:338`. The five `WriteStore` implementors forward the struct by value and do not break. |
| `override_graph` is at `graph.rs:399` and the clone test at `:1459` | ✓ true | both confirmed; the test name is `override_clone_leaves_bindings_alone` |
| `select_applies_its_rules_in_order` asserts `[Always, Off, NoPath, MissingVersion]` | ✓ true, **line amended** | `skill.rs:711-753` (**not** `:726-770`); the `select(candidates, false)` half asserts every choice is `NotPlaced` and inactive |
| `choices_serialize_their_documented_keys` pins a seven-key set on `SkillChoice` | ✓ true, **line amended** | `skill.rs:757-815` (**not** `:772-810`); keys `skill, name, version, level, activation, active, reason` at `:776-783` |
| `ChoiceReason`'s doc promises `matched` and `no_match` for milestone 3, renaming nothing | ✓ true, **line amended** | `skill.rs:253-255` (**not** `:250-254`); `pub enum ChoiceReason` at `:258` |
| `select` is at `skill.rs:314`, `SkillChoice` `:289`, `BoundSkill` `:161`, `SkillLevel` `:35`, `resolve` `:231`, `collapse` `:201`; `Activation` is a `str_enum!` block at `:13-16` | ✓ true | all confirmed; there is no `pub enum Activation` text to cite |
| `model/skill.rs` has no name validator today; the model to mirror is `PromptTemplate::name_is_valid` | ✓ true | `kind.rs:359` (and `ItemKind::prefix_is_valid` at `kind.rs:80`); 17 `is_valid(` hits, none in `skill.rs` |
| `PromptSpec` spans `prompt/mod.rs:71-123` with 24 fields, and the result of `select` lands in `ScrubbedInputs` | ✓ true | `ScrubbedInputs` at `:874-888`, built at `:863` |
| The one non-test caller of `select` is `prompt/mod.rs:861` | ✓ **line amended** | `prompt/mod.rs:861` (**not** `:858`); the other importer is `htui-store/tests/skill_attachments.rs:19` |
| `no_excerpts` is at `engine.rs:5822-5835` and builds a literal `ExcerptSet`, so it is a third construction site for D72 | ✓ true | `engine.rs:5822-5835` |
| `OVERRIDE_SKILLS_NOTE` is declared at `engine.rs:88` with its text at `:89`; the engine test is at `:12924` | ✓ true | all three confirmed |
| **The preview already runs the excerpt walk** | ✓ **false as first drafted — D73 amended** | `preview.rs:155` `build`, `bound_skills` at `:256`, `spec.excerpts = excerpts_for(…)` at `:308` immediately before `assemble`. The plan's first draft said the preview records `no_path`; it does not, and `SKILLS_NOTE` (`preview.rs:85-87`, in `STAND_INS` at `:64`) flips its sentence. |
| The Skills tab is `mod.rs` (133) + `templates.rs` (1310); there is no `skills.rs` file | ✓ true | confirmed; the input-capture guard is `mod.rs:86`, `wants_requests` `:75-77` |
| The Templates view sends exactly two requests (`Templates` at `:479`, `SaveTemplate` at `:714`) and matches `StoreReply` at `:331-376` | ✓ true | confirmed |
| `ui/diff.rs` exposes `unified` `:20`, `diff_style` `:32`, `lines` `:47` | ✓ true | confirmed |
| `TokenEstimator::DEFAULT` is `{ id: "chars-v2", prose_cpt: 25, code_cpt: 24 }` at `estimate.rs:59-63` and `estimate` is `pub fn(self, s: &str) -> i64` at `:114` | ✓ true | confirmed pure: no I/O, no clock, no global state |
| 21 `.estimate(` call sites, and none estimates a bare skill | ✓ true, **amended** | 21 hits in **four** files (**not** three): `prompt/estimate.rs` (11, all `#[cfg(test)]`), `prompt/trim.rs` (7), `prompt/excerpt.rs` (2), `prompt/mod.rs` (1) |
| **`settings/agents.rs` is the only `Table` user** | ✓ **false — D82 amended** | three files use `ratatui::widgets::Table`: `settings/agents.rs:50`, `backlog/detail/documents.rs:7`, `backlog/detail/graph.rs:11`. All three have a **row** cursor; none has a selected **cell**, so D82's narrower claim holds. `render_table` is at `agents.rs:981`. |
| Six `templates__*.snap` files exist and are 34 lines each | ✓ true | 90 snapshots in the directory; only `prompt_preview__preview_feat_1.snap` differs in length (64) |
| T3 and T4 file sets are disjoint | ✓ **true as content, amended as sequencing** | the intersection is empty, but both need a `mod` line in `skills/mod.rs`, so they run **serially** (T3 then T4). T3 owns `mod library;` at `:8`; T4 adds `mod matrix;` itself once `matrix.rs` exists. `pub mod language;` goes at `model/mod.rs:88`. |
| `0007`'s two CHECKs and the NULLS-NOT-DISTINCT unique key are the only `skill_binding` guards besides the FKs | ✓ true | `migrations/0007_skill_attachments.sql`, `0001_init.sql:406-441` |
| `skill` and `skill_binding` carry the `updated_at` trigger; `skill_version` is absent from it | ✓ true | `0001_init.sql` §5.1 DO block and the comment naming `skill_version` as deliberately absent |
| `0007` cannot be edited in place; the checksum guard refuses | ✓ true | `PgStore::connect`'s checksum check, named in `0001_init.sql`'s header |
| The next free migration number is `0008` | ✓ true | milestone 2's plan (D38): "The next migration is `0008`" |
| `globset` adds **one** package and zero transitive dependencies, not zero packages | ✓ **false as first drafted; then the maintainer took the hand-written option (OQ-14)** — the measurement is kept as the reason D70 gives | mirrored-manifest `cargo fetch` probe: `NEW: {globset 0.4.20}`, `REMOVED: []`, `VERSION CHANGED: {}` against the 736-package lock. All four non-optional deps already locked: `aho-corasick 1.1.5` (`Cargo.lock:228`), `bstr 1.13.1` (`:672`), `regex-automata 0.4.18` (`:5183`), `regex-syntax 0.8.11` (`:5200`) |
| `globset 0.4.20` is the latest and its MSRV fits (recorded for the rejected option) | ✓ true | crates.io API: `max_version 0.4.20`, `rust_version 1.88`; workspace `rust-version = "1.98"` (`Cargo.toml:8`), `rust-toolchain.toml` pins `1.98.1` |
| `default-features = false` removes only `log`; there is no `path` feature and `serde` is not a default (recorded for the rejected option) | ✓ true | globset 0.4.20 manifest `[features]`: `default = ["log"]`, `arbitrary`, `simd-accel`, `serde1` (off by default) |
| `*` matches across `/` in globset by default — the finding that decided OQ-14 | ✓ **true, and the opposite of the plan's assumption** | `GlobBuilder::literal_separator` doc: "By default this is false: `*` and `?` will match `/`"; the crate docs' own example asserts `Glob::new("*.rs").is_match("foo/bar.rs")` |
| `backslash_escape` defaults platform-dependently (recorded for the rejected option) | ✓ true | `!is_separator('\\')` — true on Unix, false on Windows; D70 sets it `false` explicitly |
| `**` is legal only as leading `**/`, trailing `/**` or `/**/`; nested `{…}` and `foo{,.txt}` are errors — the cases D71 refuses | ✓ true | globset docs, Syntax section; `build()` returns `Result` |
| `gix-glob 0.27.1` adds zero packages and has no `{a,b}` alternation — the reason it was rejected on dialect, not cost | ✓ true | second probe `NEW: {}`; its parser has no brace handling; it does support `!` negation (`parse.rs:17-20`) and trailing-`/` directory-only (`parse.rs:35-38`) |
| `0007` cannot be edited: `schema_state` compares checksums and refuses | ✓ true | `pg/mod.rs:584-627`, refusal at `:614-616`, sentence `"migration {n} was applied with a different checksum"` from `error.rs:107-109`; documented `0001_init.sql:9-11`, pinned `migrations.rs:913-929` |
| The `updated_at` trigger list is 20 tables including `skill` and `skill_binding`, and excluding `skill_version` | ✓ true | `0001_init.sql:574-580`, the exclusion comment at `:569-571`, `BEFORE UPDATE` only (`:566-568`), and `demo.rs:311` binds `updated_at` on an insert |
| `skill_binding`'s key is a non-partial `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`, and `project_id` keeps `ON DELETE CASCADE` | ✓ true | `0001_init.sql:440`; `0007:20` drops only the `NOT NULL`; `0007:6-8` says so |
| Postgres 16 accepts `ON CONFLICT (skill_id, project_id, phase_id)` against that index | ✓ true | docs.postgresql.org 16 `sql-insert.html`: inference is over columns/expressions "without regard to order"; a partial index would need `index_predicate`; a duplicate raises `23505` |
| `migrations.rs`'s applied list is a `vec![1..7]` literal, not a range | ✓ **amended — D75 rewritten** | `migrations.rs:81` (and a self-checking assertion at `:78`); `Pending(7)` at `:877`; "seven embedded migrations" at `:878`, also in `tests/connect.rs:102/119/204` |
| `TABLES` is 39 and the commented-column total is 34, the latter derived as 25 + 4 + 5 | ✓ true | `migrations.rs:101-105`; `migrations.rs:495-498` built from the three lists at `:177-334`, `:343-371`, `:377-405` |
| The pinned `run_step.trim_record` comment lives at `migrations.rs:208-216` and must move with `0008` | ✓ true | `the_ana_column_comments_are_present_and_verbatim` (`:417`) is the byte-for-byte guard; the text is written at `0007:40-46` |
| `crates/htui-core/tests/prompt_digest.rs` pins **13 top-level** trim-record keys, unaffected by a new `SkillChoice` key; `SkillChoice`'s 7 keys are pinned in the unit test | ✓ **amended** | `prompt_digest.rs:979-994` (message at `:994`), `v == 2` at `:996-999`; `skill.rs:777-786` |
| The Postgres demo loader writes none of `0007`'s columns; the Rust fixture writes all of them | ✓ true | `demo.rs:288-289` and `:303-304`; `fixtures.rs:561`, `:601-603`; pinned by `fixtures.rs:2037` and `tests/skill_attachments.rs:437-446` |
| `tests/skill_attachments.rs` (448 lines, 4 tests) and `tests/prompt_template_cas.rs` (130 lines, 1 test) use unchecked `sqlx::query` and add nothing to `.sqlx` | ✓ true | `skill_attachments.rs:10-11`, `:41`, `:47-48`, `:70-71`; `prompt_template_cas.rs:11-12`, `:50-51`, with the held-open transaction at `:44-47` and commit at `:88` |
| **No dependency is added anywhere in this milestone** | ✓ true, by decision | the maintainer's OQ-14 answer; `git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml` is a gate |
| `run_step_commit` carries hashes and no paths, so a store reader cannot answer the changed-paths question | ✓ true | `model/run.rs:585-594`; `DiffBlock`'s doc at `prompt/mod.rs:157-159` warns against re-deriving from the unified text — hence D89's `Isolator` seam |
| `Isolator::diff` is the seam a sibling `changed_paths` belongs on | ✓ true | `isolate.rs:192`, `real.rs:1451` (`diff_of` at `:1287`), `fake.rs:576` |
