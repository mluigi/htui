# Plan: MOD-9 milestone 2 — bound skills reach the run

**Status: CONFIRMED by the maintainer 2026-09-26** with OQ-9..OQ-13 defaults as written and **OQ-8 overturned**: judge prompts carry skills (D44, D48, D49), and untouched seeded `judge` templates are migrated to the new default (D49). **Amended 2026-09-26 at the blueprint gate (F-G):** a judge's `{{task}}` already replays the candidate's prompt, skills included, so the maintainer chose "allow, not in the default": D48 stands, **D49 is withdrawn** (the default `judge` body is unchanged, `0007` has no judge upgrade, R-20 is moot).

**Source**: `.claude/prds/mod-9-skill-library-templates.prd.md`, milestone 2 (Delivery Milestones
table, row 2): "Engine phase and judge steps and the preview resolve `bound_skills(project,
Some(phase))`." Scope bullet "Bound skills in every step (D4)"; success-metric row "Skills reach
runs". **Widened by maintainer decision on 2026-09-26, before this plan was written.** Milestone 2
also takes ANA-22's **storage and activation read side** (`docs/ANA-22.md` §7.1, §7.2 and §6
items 2–4, 8 and 9): migration `0007`, the global level, the three-level most-specific-wins
resolution, and a per-step `select` that resolves `always` and `off` and records every candidate's
choice. **Milestone 3 keeps** the writers, the Skills view and attachments matrix, the
language→globs map, the glob matcher over ANA-22 §5.4's F2 file set, and the `graph.rs`
phase-binding clone gap. Also decided on 2026-09-26: `R-SKL-2` is amended to name the global level
and the activation (D37), and blueprint finding F-U is opened as **MOD-52** instead of being fixed
here.

**Requirements**: `R-SKL-2` (as amended by D37), `R-SKL-1` (the `source` column), `R-PRM-1` and
`R-PRM-3` (the skills section and its cap), `R-ID-5` (skills inlined and the choice recorded),
`R-NF-3` (the preview still reads on the store worker).

**Complexity**: Medium. One forward migration (`0007_skill_attachments`), one pure resolution and
selection module in `htui-core`, one new `GraphSource` method across four implementors, and the
engine, preview and Prompt sub-tab wired to them. **No new `WriteStore` method, no new
`StoreRequest`, no new package in `Cargo.lock`.**

**Routing**: continues `/handoff-run MOD-9` (PRD path). Staffing matches milestone 1: the session
model for every step, and `rust-reviewer` (`.claude/workflow-config.json:2`) as the review gate.
There is no `.claude/agents/` in this repo, so the reviewer runs as a general-purpose agent under
that brief.

**Numbering**: continues from milestone 1's blueprint (`mod-9-templates-editable.blueprint.md` §12).
Decisions **D37…**, risks **R-14…**, open questions **OQ-8…**. Tasks restart at **T1** within this
plan, and the PRD's gate decisions are cited as **PRD D1…PRD D6**.

**Graphify / Gortex note**: `graphify-out/` does not exist and Gortex is not reachable in this
session. Every tree fact was read with `grep`/`sed` at `16560c2` (PR #8's head after the main merge)
and carries a `file:line`.

---

## Open questions for the maintainer (read these first)

Each has a default that this plan adopts, so implementation is not blocked.

- [x] **OQ-8 — Judge steps and skills. Answered 2026-09-26: judges carry skills.** The PRD's
      D4 says "phase and judge steps", but `Placeholder::allowed_in` refused `{{skills}}` in a
      judge body (`prompt/template.rs:187-213`). The maintainer wants judges to see the project's
      skills, so the contract is widened for the judge role (D48), the default `judge` body places
      `{{skills}}`, and `0007` appends the new body to every project whose `judge` head is the
      untouched seed (D49). The judge resolves the judged phase's attachments, the same resolution
      as the phase step (D44). **Handoff steps still carry none:** `promote::handoff_spec` stops
      inheriting the phase's skills (`..phase`, `promote.rs:79-103`), which would otherwise be
      scrubbed for nothing and could fail a handoff on a skill body.
- [ ] **OQ-9 — A winning attachment whose pin cannot be honoured.** Today a phase binding pinned
      to a missing version is dropped *before* the collapse, so the project binding silently takes
      over (`mem.rs:425-444`, `pg/read.rs:1506`). ANA-22 §6 item 3 says the most specific
      attachment wins with its own pin. **Default (D39):** the winner is picked first, by level. If
      its pin names no existing version, the skill renders nothing and records `missing_version`,
      with no fallback to a broader attachment. That matches `version_in_force`'s own doc
      ("renders nothing rather than quietly falling back", `model/skill.rs:75-79`). It only happens
      with hand-written SQL, because versions are append-only and never deleted.
      **Alternative:** keep today's fallback.
- [ ] **OQ-10 — Where the choice is recorded** (ANA-22 §9's open point). **Default (D42):** a new
      top-level `skill_choices` array in the trim record, which moves the record to `v: 2`. The
      column comment is re-issued in `0007`, and the twelve-key pin (`prompt_digest.rs:950`) and
      the comment pin (`migrations.rs:206-213`) both move. **Alternative:** extra fields on the
      `sections[]` row named `skills`. `Section` is generic across every section, so every other
      row would carry them as absent.
- [ ] **OQ-11 — Which phase the preview uses.** The preview picks a template by name and never
      reads the item's graph (`preview.rs:141-264`, `:307-327`). **Default (D45):** it reads
      `Backend::resolve_graph(item)` and uses the first phase, in position order, whose
      `template_name` is the chosen template. With no such phase it shows global and project
      skills only and says so in a note. **Alternative:** replace the template picker with a
      phase picker. That is a larger UI change, and the template picker is what milestone 1's
      snapshots pin.
- [ ] **OQ-12 — Glob attachments before milestone 3.** No step resolves a repo root today: the
      engine passes `no_excerpts` (`engine.rs:4940-4943`) and the preview records `no_path`
      (`preview.rs:95-97`). **Default (D40):** a winning `glob` attachment is inactive and records
      `no_path`. That is ANA-22 §6 item 7's own rule for a step with no resolvable root, and it
      stays true until milestone 3 brings the file set. Nothing can write a `glob` row before
      milestone 3 except SQL. **Alternative:** treat `glob` as `always` until the matcher lands.
      That would inject skills a maintainer scoped narrower.
- [ ] **OQ-13 — Showing the choice.** **Default (D46):** the Prompt sub-tab lists one line per
      candidate under the sections, for example `rust-style v1 · phase · always → active` and
      `tests v1 · global · off → off`. That is ANA-22 §9's mitigation "shown in the preview".
      **Alternative:** record it only, with no UI change.

---

## Summary

Skills are pure library rows today. Bindings exist at project and phase level only
(`0001_init.sql:432-441`, `project_id NOT NULL`), and nothing supplies them to a step. The engine
passes `skills: Vec::new()` to phase and judge specs (`engine.rs:4939`, `:4322`), and the preview
reads project bindings only (`preview.rs:227`).

**Storage (T1).** Migration `0007_skill_attachments.sql` is ANA-22 §7.1 verbatim: `skill_version.
source`; `skill_binding.project_id` nullable, where NULL means global; `activation`, `globs` and
`languages`; two checks and five column comments. It also re-issues the `run_step.trim_record`
comment for D42. `SkillBinding` gains `project_id: Option<ProjectId>`, `activation`, `globs` and
`languages`. `SkillVersion` gains `source`. `BoundSkill` gains `level`, `activation` and `globs`.
One pure `resolve` in `model/skill.rs` turns a project's candidate attachment rows into
most-specific-wins `BoundSkill`s, and both `MemStore::bound_skills` and `PgStore::bound_skills`
call it. The Postgres reader becomes two queries that include the global rows.

**Selection and record (T2).** One pure `select` decides each candidate: `always` is active, and
`off`, `glob` (`no_path`), `missing_version` and `not_placed` are inactive. The assembler runs it
after the scrub and the collapse, renders and caps only the active skills, and writes every choice
into the trim record's new `skill_choices` key (`v: 2`).

**Engine (T3).** `GraphSource` gains `bound_skills(project, phase)`. A phase step finds its
`PhaseId` by `(snapshot.graph.id, phase.name)` through `WriteStore::phases`, because
`SnapshotPhase` has no id (`model/run.rs:488-527`). It then passes the resolved candidates. The
judge resolves the judged phase's attachments the same way (its role now allows `{{skills}}`, D48,
and the default body places it, D49), and the handoff spec stops inheriting skills.

**Preview and Prompt sub-tab (T4).** The preview finds the phase as in OQ-11 and passes
`Some(phase)`. `SKILLS_NOTE` gets new text, `STAND_INS` stays at eight entries, and the sub-tab
lists the choices.

## Design decisions (settled here, not in code review)

| # | Decision | Why / evidence |
|---|---|---|
| D37 | **`R-SKL-2` is amended (maintainer, 2026-09-26)** to: "Skills attach at global, project or phase level; the most specific attachment of a skill wins (phase over project over global). An attachment pins a version or follows latest, and carries the activation (`always`, `glob`, `off`)." `docs/REQUIREMENTS.md`'s amendment header gains a 2026-09-26 line citing ANA-22. `CONCEPTS.md:38-39` ("`project` owns … skill bindings") is corrected to say that a skill attaches globally or to a project or phase. | ANA-22 §9 left the wording to the maintainer, and the maintainer chose "amend now". `REQUIREMENTS.md` is edited only by maintainer decision, never at close-out (`.claude/rules/workflow-docs.md`). |
| D38 | **Migration `0007_skill_attachments.sql` is ANA-22 §7.1 unchanged**, plus `COMMENT ON COLUMN run_step.trim_record` restated with `skill_choices` and `v 2` (D42), plus D49's judge upgrade. It adds no cache migration: the skill tables are not mirrored (`htui-store/src/cache/mod.rs:44`, `cache_migrations/0001_mirror.sql:24`). `tests/migrations.rs` moves as follows: the applied list becomes `1..=7` (`:79-86`); `Pending(7)` / "seven embedded migrations" (`:831-835`); a new `MOD9_COLUMN_COMMENTS` of five rows (`skill_version.source`, `skill_binding.{project_id, activation, globs, languages}`) takes the commented-column total from twenty-nine to **thirty-four** (`:452-455`); and the `trim_record` text is updated (`:206-213`). `TABLES` stays at 39. The next migration is **`0008`**. | Probed on Postgres 16 at `localhost:5432`: `0001`..`0006` then this SQL applies cleanly. One global row of a skill is accepted and a second is refused by `UNIQUE NULLS NOT DISTINCT`. A phase row with no project is refused by `skill_binding_phase_needs_project`, `glob` with empty `globs` by `skill_binding_glob_needs_globs`, and `activation = 'sometimes'` by the column check. Existing rows read `always`, `{}` and `{}`. |
| D39 | **Model.** In `model/skill.rs`: `str_enum! Activation { Always => "always", Glob => "glob", Off => "off" }` (the text-enum macro, `model/mod.rs:25-70`, which derives `sqlx::Type` under the `sqlx` feature). `SkillLevel { Global, Project, Phase }` is ordered so that `Phase` is the most specific. `SkillVersion.source: serde_json::Value`; `htui-core` already depends on `serde_json` (`Cargo.toml:20`). `SkillBinding.project_id: Option<ProjectId>`, plus `activation`, `globs: Vec<String>` and `languages: Vec<String>`, and `fn level(&self) -> SkillLevel`. `BoundSkill` gains `level`, `activation` and `globs`. **`pub fn resolve(rows: Vec<(SkillBinding, String)>, versions: &[SkillVersion]) -> Vec<BoundSkill>`** keeps, per `skill_id`, the row of the highest level. It then resolves that row's version with `version_in_force`; a miss becomes a `BoundSkill` whose `version` is `None` (OQ-9, never a fallback). It ends with the existing `(position, name bytes)` sort. **`BoundSkill::collapse(Vec<Self>) -> Vec<Self>`** becomes one list with most-specific-wins by `level`; its two-list callers are `mem.rs:436`, `pg/read.rs:1506` and `prompt/mod.rs:835`. `BoundSkill.version` becomes `Option<i32>` and `body` stays a `String` (empty when `version` is `None`). | This is one definition of "which attachment wins" for both stores and the assembler, the same reason `collapse` is pure today (`model/skill.rs:108-121`). **Deviation from ANA-22 §7.2**, recorded under "Where the PRD, ANA or tree disagree": §7.2 writes `collapse(global, project, phase)` with three lists. Once `level` is a field, the assembler's re-collapse of `spec.skills` (one list, `prompt/mod.rs:835`) needs the one-list form anyway, and three positional lists are one more way to pass them in the wrong order. |
| D40 | **`select`.** `pub fn select(candidates: Vec<BoundSkill>, placed: bool) -> (Vec<BoundSkill>, Vec<SkillChoice>)`. Choices are in collapse order. `SkillChoice { skill: SkillId, name: String, version: Option<i32>, level: SkillLevel, activation: Activation, active: bool, reason: ChoiceReason }`, and `ChoiceReason` serializes snake_case as `always`, `off`, `no_path`, `missing_version` or `not_placed`. The rules are applied in order: `!placed` makes every candidate `not_placed`; `version: None` gives `missing_version`; `Off` gives `off`; `Glob` gives `no_path` (OQ-12); `Always` gives `always`, which is active. Milestone 3 adds `matched` (with the path) and `no_match`, and a file-set argument; no reason is renamed. | ANA-22 §6 item 8's reasons, plus two that exist only because this milestone has no matcher and no writer guard. `placed` keeps the record honest for a phase body a maintainer edited to drop `{{skills}}`, now possible since milestone 1: nothing renders, so nothing is recorded as active. |
| D41 | **Readers.** `MemStore::bound_skills(project, phase)` (`mem.rs:425-444`) collects rows where `project_id.is_none()`, or `project_id == Some(project)` with `phase_id` `None` or `== phase`, pairs each with `skills[&skill_id].name`, and calls `resolve`. `PgStore::bound_skills` (`pg/read.rs:1433-1507`) runs **two** `query_as!` instead of three. The first is the versions of every skill in that candidate set; the second is the candidate bindings `WHERE b.project_id IS NULL OR (b.project_id = $1 AND (b.phase_id IS NULL OR b.phase_id = $2))`, with `$2` NULL when `phase` is `None`, plus `activation`, `globs` and `languages`. `SkillBindingRow` (`pg/rows.rs:256-300`) gets `project_id: Option<ProjectId>` and the three columns, and `bind` becomes `into_binding`. Three `.sqlx` files go and two arrive: **264 → 263**, confirmed by `cargo sqlx prepare`. The demo loader's `INSERT INTO skill_binding` (`pg/demo.rs:302-315`) and `project_reach` (`pg/write.rs:342`) keep their text, so their `.sqlx` files stay. `Backend::bound_skills` is unchanged, still refusing offline (`backend.rs:372`). | This keeps the reads inherent, for the same reason as today (`pg/read.rs:1374-1382`, `traits.rs:24-27`). A global row survives `delete_project` on both stores: Postgres by `ON DELETE CASCADE` on a NULL key; `MemStore` by changing `retain(|row| row.project_id != id)` to `!= Some(id)` (`mem.rs:3269`), with its reach count likewise (`mem.rs:3088-3093`). `DeleteReach.skill_bindings` therefore still counts only the project's own rows, and the conformance case's `3` (`conformance.rs:2421`) holds. |
| D42 | **The record (OQ-10).** `TrimRecord` (`prompt/trim.rs:181-206`) gains `skill_choices: Vec<SkillChoice>`, always serialized (an empty array when there are no candidates), and `v` becomes `2`. `prompt_digest.rs:950`'s key set becomes thirteen with a matching message, and the golden `IMPL_TRIM_RECORD` (`fixtures.rs:1629-1665`) gains the key. The record is `Serialize` only and never read back (`trim.rs:181`), so no reader breaks; the only field readers are `run.rs:776` (`estimated_after`, `trimmed`) and the Prompt sub-tab. | ANA-22 §6 item 8: "record every candidate … beside the trim record's `skills` section". A bump of `v` is what the field exists for. The digest does not move for the record (`digest.rs` hashes the rendered bytes), only for a change in which skills render. |
| D43 | **The assembler.** `scrubbed_inputs` still masks every candidate's `name` and `body` (`prompt/mod.rs:755-759`), so recorded names are masked names. Then `collapse` runs, then `select(collapsed, parsed.places(Placeholder::Skills))`, where `places` is whatever `ParsedTemplate` already exposes for its spans (T2 picks the existing accessor or adds a one-line `pub fn places`). Only the active list reaches `render::skills`, `skills_tokens` and the cap, so an `off` skill can never cause `SkillsExceedCap`. The choices go to `trim::record`. `PromptSpec.skills`'s doc becomes "candidates, resolved by `model::skill::resolve`; the assembler collapses and selects". | One place decides, for the engine and the preview alike. `select` is pure, which keeps `assemble` pure (ANA-5 §4.4). |
| D44 | **Engine (OQ-8).** `GraphSource` (`graph.rs:57-107`) gains `async fn bound_skills(&self, project: ProjectId, phase: Option<PhaseId>) -> Result<Vec<BoundSkill>>`, implemented in `TestSource` (`graph.rs:758-819`), `impl GraphSource for MemStore` and `FakeGraphSource` (`fake.rs:843-875`, `:896-986`, delegating as their `prompt_template` does), and `BackendGraphs` (`run_worker.rs:2362-2394`). `phase_spec` (`engine.rs:4842-4961`) calls `self.parts.store.phases(snapshot.graph.id)` (`traits.rs:685`) and takes the row whose `name == phase.name` (`UNIQUE (graph_id, name)`, `0001_init.sql:247`). `Some(id)` passes it through; `None` (the phase was renamed or deleted after the snapshot) passes `None` and adds the note `skills: phase \`<name>\` is no longer in graph \`<graph>\`; phase-level attachments were not applied`. When `snapshot.graph.is_override` it adds `skills: an override graph's phases carry no phase-level attachments until MOD-9 milestone 3` (the clone gap, `graph.rs:350-355`). The comment at `engine.rs:4937-4938` ("milestone 6") is rewritten. **The judge** (`judge_prompts`, `:4195`, which gains the graph id from its caller `run_judge`, `:4032`/`:4053`, where `snapshot` is in scope) resolves `bound_skills(project, judged phase id)` the same way and passes the candidates to both its forward and reverse specs (`:4303-4337`). `handoff_spec` sets `skills: Vec::new()` before `..phase`. | `SnapshotPhase` carries no `PhaseId` and its positions are re-densified (`graph.rs:286-300`), so `(graph id, name)` is the only exact key. Adding an id to the snapshot would change every stored `run.graph_snapshot` and the orch fixture JSON (`tests/fixtures/feature.snapshot.json`) for a key the table already guarantees. The read is one per phase step, and the fan-out group assembles once (`engine.rs:3377`). |
| D48 | **The placeholder contract widens for the judge role.** `Placeholder::allowed_in(TemplateRole::Judge)` (`template.rs:196-201`) gains `Skills`. Nothing else moves for the judge: `SectionName::Skills` is already protected in every role (`prompt/mod.rs:291-297`), so the judge's trim order (`trim.rs:271`, candidates then task) never touches it, and the cap applies unchanged. `JUDGE_REQUIRED` is untouched, so `{{skills}}` stays optional in a judge body. `defaults.rs`'s `every_phase_body_is_wrong_role_for_judge_and_handoff_and_vice_versa` (`:326-350`) still holds: no phase body's first trip token was `skills`. The Templates view's inline help (`ui/tabs/skills/templates.rs`, via `allowed_in`) lists `skills` for a judge from then on. | The maintainer's OQ-8 answer. A judge weighing candidates against the project's conventions needs the same skills the candidates were written under. |
| D49 | **WITHDRAWN 2026-09-26 (blueprint F-G; judges already see skills in their replayed `{{task}}`).** ~~The default `judge` body places `{{skills}}`, and `0007` upgrades untouched seeds.~~ `defaults.rs`'s `JUDGE` (`:177-199`) becomes the current body with `{{skills}}` on its own line after `{{task}}`. The fenced `json` block and everything after it are byte-identical, which keeps `the_judge_body_states_the_verdict_block_verbatim` (`:405`) green. The current text is kept as `pub(crate) const JUDGE_SEED_V1` (doc: "the body `0001`..`0006` seeded; `0007` upgrades rows still equal to it"). `0007` then runs `INSERT INTO prompt_template (project_id, name, version, body, created_by) SELECT h.project_id, 'judge', h.version + 1, $new$…$new$, h.created_by FROM (the head `judge` row per project) h WHERE h.body = $old$…$old$`, with both bodies dollar-quoted verbatim. A project whose head `judge` was edited, or that has none, is left alone. A migration test plants a project on `0001`..`0006` with `JUDGE_SEED_V1` as its judge v1 (and a second one with an edited body), applies `0007`, and asserts: head v2 equals `body_of("judge")` for the first; the second is unchanged; both literals in the SQL equal the Rust constants byte for byte. | The maintainer's answer to "Judge seed", mirroring `0004`'s "only the exact seeded value moves" (`0004_max_agents_per_run_default.sql:10-12`). Append-only (PRD D5): a new version, never an edit of v1, so pinned judges (`phase.judge.template`) keep their version. |
| D45 | **Preview (OQ-11).** After the template reads (so an offline preview still refuses with `PROMPT_ON_SERVER_ONLY` first, `tests/prompt_preview.rs:343-356`), `build` calls `backend.resolve_graph(item)` (`backend.rs:488`) and takes the first `ResolvedPhase` whose `phase.template_name == chosen.name` (`model/kind.rs:204-205`, `:300-314`). It passes its id to `bound_skills`. `SKILLS_NOTE` becomes "preview: phase-level skills come from the first phase of the item's graph that uses this template; a glob attachment records no_path because no root resolves". With no graph or no matching phase, it adds a second note: "preview: no phase of this item's graph uses template `<name>`; global and project skills only". `STAND_INS` stays at eight. | `ResolvedGraph` already carries phases with ids. A template shared by two phases is rare (the seed gives each phase its own name), and the note names the choice. |
| D46 | **Prompt sub-tab (OQ-13).** `ui/tabs/backlog/detail/prompt.rs` (`:156` renders `record.notes`) renders a `skills` block before the notes when `skill_choices` is non-empty: one line per choice, `<name> v<N|?> · <level> · <activation> → <active|reason>`, active lines in the normal style and inactive lines dim. | ANA-22 §9 risk row 1's mitigation. The pane already renders the record, and the data is the record's. |
| D47 | **What milestone 2 leaves alone, by name.** No writers (`upsert_skill`, `add_skill_version`, `set_skill_binding`), no Skills view content, no language map, no glob matcher, no file walk, no `SKILL.md` import, and no change to `graph.rs`'s override clone: all of these are milestone 3 or 4. No new `StoreRequest` (the preview request already exists), no `WriteStore` or `ReadStore` change (so `CASES` stays at 71 and `READ_CASES` at 14), no demo fixture rows (so every demo count and digest that does not render a skill stays put), and no MOD-52 fix. | Scope as the maintainer set it on 2026-09-26. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| Pure resolution shared by both stores | `BoundSkill::collapse` + `version_in_force` | `model/skill.rs:81-138` |
| Text enum with DB spelling | `str_enum!` | `model/mod.rs:25-70` |
| Inherent read behind a seam | `GraphSource::prompt_template` in four implementors | `graph.rs:81-86`, `:781`; `fake.rs:843`, `:954`; `run_worker.rs:2365` |
| Column comments pinned by test | `MOD7_COLUMN_COMMENTS` | `tests/migrations.rs:339-369` |
| Custom rows in a MemStore test | edit `DemoData`, `MemStore::from_demo` | `graph.rs:836-840` (`store_with`) |
| Engine note on a spec | `settings::resolve_hops(.., &mut notes)` | `engine.rs:4899-4905` |

## Tasks

**T1 → T2 → (T3 ∥ T4)**. T1 changes model types that `htui-store` and the assembler compile
against, so it must land whole. T3 and T4 both need T2's `select` in the assembler. Parallel T3 and
T4 each run in their own git worktree against **one shared `CARGO_TARGET_DIR`**, with
`--test-threads=2` at most.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T1 | `crates/htui-store/migrations/0007_skill_attachments.sql` (new), `crates/htui-core/src/model/skill.rs`, `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/fixtures.rs` (demo literals only), `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/prompt/mod.rs` (the `collapse` call only), `crates/htui-core/src/prompt/fixtures.rs`, `crates/htui-core/src/prompt/render.rs` (`version` becomes `Option`: the attribute reads the active skill's `Some`, and the test literals), `crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/pg/rows.rs`, `crates/htui-store/.sqlx/` (−3, +2), `crates/htui-store/tests/migrations.rs`, `crates/htui-store/tests/pg_criteria.rs`, `crates/htui-store/tests/skill_attachments.rs` (new), `crates/htui-core/src/prompt/defaults.rs`, `crates/htui-core/src/prompt/template.rs` (`allowed_in`), `crates/htui/tests/snapshots/templates__*.snap` (only if a judge help listing moves) | first |
| T2 | `crates/htui-core/src/prompt/mod.rs`, `crates/htui-core/src/prompt/trim.rs`, `crates/htui-core/src/prompt/template.rs` (only if `places` is added; after T1's `allowed_in` edit), `crates/htui-core/src/fixtures.rs` (`IMPL_TRIM_RECORD` only), `crates/htui-core/tests/prompt_digest.rs`, `crates/htui-core/tests/prompt_skills.rs` (new) | after T1 |
| T3 | `crates/htui-orch/src/graph.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/promote.rs`, `crates/htui/src/run_worker.rs` | ∥ T4, after T2 |
| T4 | `crates/htui/src/preview.rs`, `crates/htui/src/ui/tabs/backlog/detail/prompt.rs`, `crates/htui/tests/prompt_preview.rs`, `crates/htui/tests/snapshots/prompt_preview__*.snap`, `crates/htui/tests/snapshots/backlog__detail_prompt.snap`, `crates/htui/tests/backlog.rs` (only if the detail snapshot's fixture needs a choice) | ∥ T3, after T2 |

**Intersections, checked.** T1 ∩ T2 = {`prompt/mod.rs`, `fixtures.rs`}, so they run serially (T1
touches one line of each: the `collapse` call and the demo literals). T3 ∩ T4 = ∅: T3 is
`htui-orch` plus `run_worker.rs`, and T4 is `preview.rs`, the detail pane and `crates/htui/tests`.
**Build coupling:** T3 adds a trait method that `run_worker.rs` must implement, which is why T3
owns that file. T4 builds against T3's absence, because `preview.rs` calls `Backend` directly,
never `GraphSource`. Snapshots move only in T2 (golden record) and T4.

Every implementer prompt carries these rules:
- The PRD's gate decisions, this plan's D37–D49 and the maintainer's OQ answers win over prose.
- Read the tree with grep and read (no `graphify-out/`, no Gortex).
- No test is skipped or loosened to get green; a moved pin names its reason in the assertion message.
- Every new `pub` item has a doc comment and a `Debug`.
- Commit the red tests first, then green.
- Gate command: `RUST_BACKTRACE=0 cargo test -p <crate> --all-features -- --test-threads=2`, plus `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=…` for `htui-store` and `htui`.

### Task 1: storage and resolution (D38, D39, D41)
- **Tests first.**
  - `model/skill.rs` unit tests:
    - `phase_beats_project_beats_global`
    - `off_at_a_narrower_level_wins_and_keeps_the_skill_out_of_broader_levels` (the resolve half: the winner is `Off`)
    - `a_missing_pin_on_the_winner_does_not_fall_back` (OQ-9)
    - `global_rows_order_by_position_then_name_bytes`
    - `level_of_a_binding_follows_its_nullable_keys`
    - `activation_round_trips_its_db_text`
    - the existing three collapse tests, ported to the one-list signature with their assertions unchanged
  - `mem.rs`: `a_global_attachment_reaches_every_project`, `delete_project_keeps_global_attachments`, and the existing `a_preview_style_bound_skills_read_collapses_overrides` (`:5967`) still passing unchanged.
  - `tests/migrations.rs`: the moved pins of D38.
  - `tests/skill_attachments.rs` (Postgres, `testkit::demo_db`, rows planted with runtime `sqlx::query`):
    - `pg_and_mem_agree_on_global_project_and_phase` (the same rows planted in both; equal `Vec<BoundSkill>`)
    - `an_off_phase_attachment_hides_a_project_skill`
    - `a_global_row_survives_project_delete`
    - `the_checks_refuse_a_phase_row_without_a_project_and_glob_without_globs`
  - `pg_criteria.rs:1806`'s equality test keeps passing with the new fields.
  - `template.rs`: `skills_is_allowed_in_a_judge_body` and `skills_is_still_refused_in_a_handoff_body`.
  - `defaults.rs`: `the_judge_body_places_skills_and_keeps_its_verdict_block`.
  - `tests/migrations.rs`: `an_untouched_judge_seed_gets_the_skills_body_and_an_edited_one_does_not` (D49), and `the_migration_bodies_equal_the_rust_constants`.
- **Action**: D38, D39, D41, D48, D49. Regenerate `.sqlx` against a scratch database migrated to `0007`.
- **Validate**: gates for `htui-core` and `htui-store` (Postgres); `cargo sqlx prepare --check`;
  `ls crates/htui-store/.sqlx | wc -l` = 263; `cargo build --workspace --all-features --all-targets`
  (downstream crates compile against the new fields).

### Task 2: selection and the record (D40, D42, D43)
- **Tests first** (`tests/prompt_skills.rs`, over `prompt::fixtures`):
  - `an_off_skill_is_not_rendered_and_is_recorded_off`
  - `a_glob_skill_records_no_path_before_milestone_3`
  - `a_missing_version_records_missing_version`
  - `a_template_without_the_placeholder_records_not_placed_and_renders_nothing`
  - `an_off_skill_never_trips_the_cap` (the `phase_skills_over_cap` fixture with one skill switched `Off` assembles)
  - `choices_follow_the_collapse_order_and_carry_masked_names`
  - `the_digest_moves_only_when_the_active_set_moves` (toggling an already-inactive skill's `globs` leaves the digest equal)
  - `prompt_digest.rs:950` moves to thirteen keys and `v == 2`.
  - `IMPL_TRIM_RECORD` gains `"skill_choices": [...]`.
- **Action**: D40, D42, D43.
- **Validate**: gates for `htui-core`, then the `htui-orch` and `htui` gates, because the
  `IMPL_TRIM_RECORD` golden and any test reading `trim_record` keys live downstream too.

### Task 3: engine (D44)
- **Tests first** (`engine.rs` tests over `FakeGraphSource` + `MemStore::demo()`):
  - `a_phase_step_renders_its_phase_and_project_skills`: demo `implement` gives `tests` v1 then `rust-style` v1 pinned, which is `pg_criteria.rs:1806`'s phase answer.
  - `a_non_implement_phase_renders_project_skills_only`
  - `a_renamed_phase_falls_back_to_project_skills_with_a_note`
  - `an_override_graph_notes_the_clone_gap`
  - `a_judge_renders_the_judged_phases_skills_in_both_orders`: the forward and reverse prompts carry the same skills section, and the judge's trim record lists the choices.
  - `the_handoff_carries_no_skills`: the handoff's trim record has an empty `skill_choices`.
  - `fake.rs`: a delegation test beside `the_store_answers_the_source_without_recursing` (`:2106`).
- **Action**: D44.
- **Validate**: gates for `htui-orch` and `htui` (`run_worker.rs` compiles, and its tests pass).

### Task 4: preview and Prompt sub-tab (D45, D46)
- **Tests first**:
  - `tests/prompt_preview.rs`: `the_preview_carries_the_phase_skills_of_the_matching_phase`, where demo `FEAT-1` with `implement` shows `rust-style` v1 at position 2.
  - `a_template_no_phase_uses_is_noted_and_shows_project_skills` (`templates` offered under a new name).
  - `the_preview_declares_its_stand_ins` updated to the new `SKILLS_NOTE` text (`:181`).
  - `the_offline_preview_still_refuses_with_the_prompt_sentence` (the existing `:343-356`, unchanged).
  - The `preview_feat_1` snapshot moves deliberately: the note, the skills section bytes, and the digest.
  - `backlog__detail_prompt` gains the skills block.
- **Action**: D45, D46.
- **Validate**: the `htui` gate (Postgres), and `cargo insta` review of every moved snapshot, each
  named in the commit message.

## Test plan

Unit tests cover resolution and selection (`model/skill.rs`). Assembler integration tests cover
rendering, the cap and the record (`prompt_skills.rs`). MemStore and Postgres agreement goes on a
real Postgres 16 (`skill_attachments.rs`). Engine behaviour is tested over the fake source, and
preview behaviour through `preview::build` on `Backend::memory` and `demo_db`. Snapshots move only
where the rendered skills or the recorded choices change, and each moved snapshot is named.

## Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-14 | Wiring skills moves every demo prompt that renders `{{skills}}`: the digest, `estimated_*`, and possibly a trim decision in a budget-tight fixture. | Certain | Low | This is the PRD's own risk row 3. Snapshot digests move only in T4, named. Any engine test whose budget tips into a trim is re-read, not re-pinned blindly. |
| R-15 | A phase renamed after the snapshot, or an override graph, silently loses its phase-level skills. | Low | Medium | D44's two notes in `trim_record.notes`. The clone gap is milestone 3's by ANA-22 §8. |
| R-16 | `project_id` becoming nullable ripples into a `query!` that selected it as non-null. | Low | Low | Only `pg/read.rs:1457`/`:1479` select it (the survey, confirmed by grep). `cargo sqlx prepare --check` fails loudly otherwise. |
| R-17 | OQ-9's no-fallback changes behaviour for a winning pin to a missing version. | Very low | Low | It is reachable only by SQL (versions are append-only), and it is recorded as `missing_version`, not silent. |
| R-18 | Parallel T3 and T4 builds contend on the shared target dir and on disk (28 GB seen last session). | Medium | Medium | One `CARGO_TARGET_DIR`; `--test-threads=2`; delete `target/debug/incremental` on ENOSPC. |
| R-20 | D49's SQL literal drifts from the Rust constant, so an untouched seed is not upgraded, or a new body diverges from `body_of("judge")`. | Low | Medium | `the_migration_bodies_equal_the_rust_constants` reads the migration file and compares both dollar-quoted literals with `JUDGE_SEED_V1` and `body_of("judge")`. |
| R-19 | `ORT_LIB_LOCATION` points at an npm copy of ONNX Runtime 1.18, because `parcel.pyke.io` is still denied (403). | Certain here | Low | Gate env recorded in the PR test plan. CI has no workflows in this repo. |

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- `USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features --no-fail-fast -- --test-threads=2`
- `cargo sqlx prepare --check` against a database migrated fresh to `0007`; `.sqlx` count 263
- Validator: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`
- Pins afterwards:
  - unchanged: `CASES` 71, `READ_CASES` 14, `StoreRequest` 66, `StoreReply` 37, `MIRRORED_TABLES` 21, `TABLES` 39
  - moved: `.sqlx` 263, commented columns 34, next migration `0008`

## Acceptance

- A skill attached globally, at project level or at phase level reaches a real phase step's prompt,
  with the most specific attachment winning.
- An `off` attachment keeps a skill out of the prompt.
- Every candidate's choice is in `run_step.trim_record.skill_choices`.
- The preview shows the same skills as the phase the chosen template belongs to, and the Prompt
  sub-tab lists the choices.
- A judge prompt renders the judged phase's active skills when its template places `{{skills}}` (the new default does); handoff prompts are unchanged.
- The PRD's hypothesis clause "a skill bound to the `implement` phase appears in that step's prompt
  and the preview" holds on the demo data (`rust-style` pinned v1 on `implement`).

## Where the PRD, ANA or tree disagree

- **ANA-22 §8** assigns the migration, model, `select` and recording to milestone 3. The maintainer
  moved them to milestone 2 on 2026-09-26 (this plan's source). ANA-22 §10 gets an amendment
  record line at close-out; the verdict is unchanged.
- **ANA-22 §7.1** says "next free number, `0006` today". `0006` is MOD-38's `0006_requirements.sql`,
  so this is `0007`.
- **ANA-22 §7.2** writes `collapse(global, project, phase)`. D39 uses one list with `level` (reason
  in D39).
- **ANA-22 §2** cites stale lines (`pg/read.rs:1150`, `mem.rs:389`, `backend.rs:363`,
  `engine.rs:4310`/`:4927`, `fixtures.rs:472-565`). The current lines are `:1433`, `:425`, `:364`,
  `:4322`/`:4939` and `:513-603`. Left as written: an analysis is a dated record.
- **PRD D4** says judge steps carry skills, which the placeholder contract refused. It is widened by D48 on the maintainer's OQ-8 answer; ANA-5 §4.1's role table (`:319-341`) is a dated record and is not edited, and the change is recorded in MOD-9's write-up.
- **PRD Evidence** cites `engine.rs:4310` and `:4925-4927`. The current lines are `:4322` and
  `:4937-4939`.
- **`.claude/plans/mod-4-orch-engine.blueprint.md:740`, `:915`** say "milestone 6 fills them". This
  is historical and not edited; the engine comment is what gets corrected (D44).

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Engine passes empty skills for judge and phase | true | `engine.rs:4322`, `:4939` (grep `skills: Vec::new()`) |
| Preview reads project bindings only | true | `preview.rs:227` `bound_skills(row.project_id, None)` |
| `{{skills}}` is phase-only | true | `template.rs:187-213` `allowed_in` |
| Handoff spec inherits `skills` | true | `promote.rs:79-103` ends `..phase` |
| `SnapshotPhase` has no `PhaseId` | true | `model/run.rs:488-527` field list |
| `UNIQUE (graph_id, name)` on phases | true | `0001_init.sql:246-247` |
| `WriteStore::phases(graph)` exists | true | `traits.rs:685`; `writer.rs:679` |
| Override clone mints fresh phase ids, copies no bindings | true | `graph.rs:389-397`; test `graph.rs:1418` |
| `bound_skills` is not on any trait | true | grep: definitions only at `pg/read.rs:1433`, `mem.rs:425`, `backend.rs:364` |
| Skill tables are not mirrored | true | `cache/mod.rs:44` (21 entries, no skill tables); `cache_migrations/0001_mirror.sql:24` |
| PgStore reader uses 3 `query_as!` filtering `project_id = $1` | true | `pg/read.rs:1440-1499` |
| `SkillBindingRow.project_id` is non-optional | true | `pg/rows.rs:256-300` |
| Trim record has 12 pinned keys | true | `prompt_digest.rs:976-993` |
| Trim record column comment is pinned | true | `migrations.rs:206-213`; set in `0002_agent_probe.sql:54` |
| Commented-column total is 29 | true | `migrations.rs:452-455` (25 + 4) |
| Applied list is 1..6, `Pending(6)` | true | `migrations.rs:79-86`, `:831-835` |
| `.sqlx` holds 264 files | true | `ls crates/htui-store/.sqlx \| wc -l` after the main merge |
| `str_enum!` derives `sqlx::Type` under a feature | true | `model/mod.rs:25-45` |
| `htui-core` depends on `serde_json` | true | `crates/htui-core/Cargo.toml:20` |
| §7.1 SQL applies on 0001..0006 and its checks refuse the three bad rows | true | Postgres 16 probe, 2026-09-26 (D38) |
| No engine test pins a prompt digest hash | true | grep `digest` asserts in `htui-orch`: only `is_some`/`is_none`/`assert_ne!` (`conformance.rs:1338`, `:2892`) |
| T3 ∩ T4 file sets are disjoint | true | Tasks table, intersected by hand |
| The demo `implement` phase answer is `tests` v1 then `rust-style` v1 | true | `pg_criteria.rs:1806-1878` |
| `ResolvedGraph` carries phases with `template_name` | true | `model/kind.rs:204-205`, `:300-314` |
| The offline preview refusal is tested | true | `tests/prompt_preview.rs:343-356` |
| `Skills` is protected for every role | true | `prompt/mod.rs:291-297` |
| The judge trim order touches only candidates and task | true | `trim.rs:271` |
| The judge body's first trip token for other roles is `task` | true | `defaults.rs:340` |
| `0004` moves only an exact seeded value | true | `0004_max_agents_per_run_default.sql:10-15` |
