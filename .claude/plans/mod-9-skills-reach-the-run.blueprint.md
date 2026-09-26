# Blueprint: MOD-9 milestone 2, "skills reach the run"

**Status**: CONFIRMED by the maintainer 2026-09-26, with the amendment below (D49 withdrawn).
Findings F-A to F-Q (§0) and decisions D50–D69 (§11) are proposed here. A finding marked **Blocker** means the plan, read literally, does not compile, fails
its own named test or the workspace gate, or cannot be validated. **Major** means a named test or a
named pin is wrong, or a design consequence the maintainer has not seen. **Minor** is a citation, a
wording or a placement. The Fix column is what the implementer builds; a Fix that amends one of
D37–D49 is flagged **(amends Dnn)** and goes to the maintainer through the main thread.

> **Amendment 2026-09-26 (maintainer, on F-G):** judges see the judged phase's skills already, inside
> their `{{task}}` (the candidate's replayed prompt, `engine.rs:4212-4231`). The maintainer chose
> "allow, not in the default": **D48 stands** (the judge role admits `{{skills}}`, and the engine
> resolves the judged phase's attachments for the judge, D44/D62), **D49 is withdrawn** — the
> default `judge` body is unchanged, `0007` carries no judge upgrade, and there is no
> `JUDGE_SEED_V1`. D56, D57, D58, F-B, F-O and R-20 are moot. A judge on the default body records
> every candidate `not_placed` (D40), which is the honest record. Where this file still names D49's
> artefacts, this note wins.

**Plan**: `.claude/plans/mod-9-skills-reach-the-run.plan.md`, confirmed 2026-09-26 (OQ-8
overturned; OQ-9..OQ-13 defaults). Its D37–D49, T1–T4, file sets and Verified-claims table are
authoritative except where §0 amends them. **PRD**: `.claude/prds/mod-9-skill-library-templates.prd.md`
milestone 2. **ANA**: `docs/ANA-22.md` §6, §7.

**Verified at**: HEAD `90ad465`, branch `claude/amazing-rubin-uipggb`.
`git diff --stat 16560c2 HEAD -- crates Cargo.toml Cargo.lock` is empty, so the plan's `file:line`
citations still hold (the only drift is noted in F-P). **Line numbers are pre-edit.**
`crates/htui-store/.sqlx/` holds **264** files. `df -h /` shows 13 GB free (R-18 applies). Two
probes ran on this box's Postgres 16.13 at `localhost:5432` (scratch databases, both dropped): the
full `0007` text of §2.1 applied after `0001`..`0006` with four planted projects (§2.1 table), and
the four refusals of §2.9 with their constraint names. `pg_attribute.attnotnull` for
`skill_binding.project_id` reads `f` after `0007`.

**Graphify / Gortex**: `graphify-out/` does not exist and Gortex is not reachable. Every fact here
was read with `grep`/`sed`/`Read`.

**Scope**:
- **Order**: T1 → T2 → (T3 ∥ T4), as the plan says. T3 and T4 in their own worktrees, one shared
  `CARGO_TARGET_DIR`, `--test-threads=2` at most.
- **One migration**, `0007_skill_attachments.sql` (§2.1). `TABLES` stays 39; applied list 1..=7;
  `Pending(7)`; commented columns 29 → 34; next migration `0008`.
- **No new `WriteStore`/`ReadStore` method, no new `StoreRequest`**: `CASES` 71, `READ_CASES` 14,
  `StoreRequest` 66, `StoreReply` 37 unchanged. **One new `GraphSource` method** (T3), four
  implementors.
- **`.sqlx`**: 264 → 263 (T1: three files out, two in).
- **Dependencies**: none. `git diff --exit-code Cargo.lock` holds for every task.

**House style (carried from milestone 1)**: `unsafe_code = "forbid"`; `missing_docs` on lib roots;
`missing_debug_implementations` and `unused_qualifications` warn; clippy `-D warnings`; rustdoc
denies broken/private intra-doc links (so no `[`X`]` link to a private item). `rustfmt.toml`:
`max_width = 100`. Every new `pub` item has a doc comment and a `Debug`. Commit the red tests
first, then green; every commit compiles (red commits use `todo!()`). No test is loosened; a moved
pin names its reason in the assertion message. Implementers stage their own paths only.

---

## 0. Findings the plan fact-check missed

| # | Severity | Plan says | Tree at `90ad465` | Fix |
|---|---|---|---|---|
| **F-A** | **Blocker** (T1 does not compile) | T1's file list (plan:160); D41: "The demo loader's `INSERT INTO skill_binding` (`pg/demo.rs:302-315`) … keep their text". | `crates/htui-store/src/pg/demo.rs:306` binds `ProjectId::as_uuid(row.project_id)`. With `SkillBinding.project_id: Option<ProjectId>` that is a type error. `pg/demo.rs` is in no task's file list. | D59: T1 adds `crates/htui-store/src/pg/demo.rs`; the bind becomes `row.project_id.map(ProjectId::as_uuid)`. The query **text** is unchanged, so its `.sqlx` file (`query-b742e011…json`) stays and the 263 count holds. |
| **F-B** | **Blocker** (T1 fails clippy; D49's test cannot compile) | D49: "kept as `pub(crate) const JUDGE_SEED_V1`"; T1 test list puts `the_migration_bodies_equal_the_rust_constants` and the D49 upgrade test in `crates/htui-store/tests/migrations.rs` (plan:200). | Nothing outside tests reads the constant, so a `pub(crate)` item is `dead_code` in every non-test build, and `clippy -D warnings` fails. A `pub(crate)` item of `htui-core` is also invisible to an `htui-store` integration test. | D57 **(amends D49)**: `pub const JUDGE_SEED_V1` in `htui_core::prompt::defaults` (not re-exported from `prompt`), doc'd. Both D49 tests stay in `migrations.rs` (D58). |
| **F-C** | **Major** (existing test goes red) | D44: "`handoff_spec` sets `skills: Vec::new()` before `..phase`". | `promote.rs:402-428` `the_handoff_spec_keeps_the_phase_s_item_and_budget` builds `expected` with `..phase.clone()` from `phase_implement_attempt2()`, which carries one skill (`prompt/fixtures.rs:221-227`), and asserts `spec == expected`. The handoff doc (`promote.rs:74-77`) says "Every other field is the phase's". | D63: `expected` gains `skills: Vec::new()` with the message "D44: a handoff carries no skills"; the doc names the exception. |
| **F-D** | **Major** (named test cannot observe what it asserts) | T3 test `the_handoff_carries_no_skills`: "the handoff's trim record has an empty `skill_choices`". | The only engine path that assembles a handoff, `open_chat`'s `OpeningKind::Handoff` arm (`engine.rs:1199-1260`), keeps `assembled.text` and `assembled.digest` and drops the record; no row or event stores it. | D63: the test lives in `promote.rs`'s `mod tests`, over `handoff_spec(phase_implement_attempt2(), …)` then `assemble`: `spec.skills` is empty and `trim.skill_choices` is `[]`. |
| **F-E** | **Major** (T1's gate leaves a red suite for T2–T4) | T1 Validate: `htui-core`, `htui-store`, `cargo build --workspace` (plan:202-204). T1 files: `templates__*.snap` "only if a judge help listing moves". | It moves. `templates__edit_help.snap` (from `crates/htui/tests/templates.rs:154-177`) changes twice: the help pane lists `allowed_in(Judge)` (`ui/tabs/skills/templates.rs:1043-1046`) and gains `{{skills}} section` between `{{phase}} scalar` and `{{task}} section`; the editor pane shows the demo `judge` v1, which `MemStore::demo()` seeds from `DEFAULT_TEMPLATES` (`fixtures.rs:775-776`), so it gains the line `{{skills}}` after `{{task}}`. Also `template.rs:476-480` (`the_three_role_sets_are_closed`) pins the judge set to four tokens. | D68: T1 edits `template.rs:476-480` to `["item_key", "phase", "skills", "task", "candidates"]`, adds `"{{skills}}"` to the judge token loop at `crates/htui/tests/templates.rs:160`, and its gate runs `cargo test -p htui --all-features --test templates` with the `edit_help` snapshot reviewed and accepted in T1. No other `templates__*` snapshot shows a judge body or the judge help (`browse` and `diff_two_versions` show `implement`). |
| **F-F** | **Major** (named test fails as written) | T2 test `an_off_skill_never_trips_the_cap` "(the `phase_skills_over_cap` fixture with one skill switched `Off` assembles)". | `phase_skills_over_cap` (`prompt/fixtures.rs:694-716`) sets `max_skill_tokens = 100` and gives **each** skill `prose_block(_, 30)`: 30 lines of ~80 chars, ≈600 tokens apiece (`prose_block`, `:548-556`). One `Always` skill alone is still over the cap, so the spec still refuses. | §3.5: the test measures one skill's `skills` tokens first and sets the cap to exactly that, so both-on refuses and one-`Off` assembles. |
| **F-G** | **Major** (design consequence of D48/D49 the maintainer should see; no code change proposed) | D48: "A judge weighing candidates against the project's conventions needs the same skills the candidates were written under." | The judge's `{{task}}` is the lowest-index survivor's recorded seq-0 `prompt` text **verbatim** (`engine.rs:4213-4226`), and after T3 that text already contains the judged phase's `<section name="skills">`. The default judge then carries each skill body twice: once inside `judge_task` (trimmable, judge rank 2, `trim.rs:271`) and once as the protected `skills` section, in both the forward and the reversed call. The protected copy counts against the judged phase's budget (`engine.rs:4329-4333`), so `BudgetTooSmall` arrives earlier for a judge than for its candidates. | Recorded as **R-21**. The main thread asks the maintainer to confirm (the protected copy is what survives a trimmed task) or to amend. Nothing in T1–T4 changes for it. |
| **F-H** | **Major** (reviewer would expect moves that do not happen, and miss one that does) | T4: "The `preview_feat_1` snapshot moves deliberately: the note, the skills section bytes, and the digest." Tasks table names `prompt_preview__*.snap` and `backlog__detail_prompt.snap`. | FEAT-1's preview defaults to `prd` (`preview.rs:307-327`; pinned by `prompt_preview.rs:235-259`). FEAT-1's graph `feature` has a `prd` phase, but no phase-level binding points at it (only `implement`, `fixtures.rs:583-589`), so the skills are the same two project rows as today: **the skills bytes and the digest `b9fb821f…` do not move.** What moves is the `SKILLS_NOTE` row and D46's new block. `preview_ana_2` moves too, for a reason the plan does not give: htui `ANA-2` runs the `analysis` graph (`research`, `verdict`, `seed.rs:54-71`), which has no `prd` phase, so D45's second note is added. | §5.5 lists the three moved snapshots and exactly what moves in each; the digest line must be identical before and after. |
| **F-I** | Minor | T1: "the existing three collapse tests, ported to the one-list signature with their assertions unchanged". | `model/skill.rs` has **two** collapse tests (`phase_overrides_project_once` `:187`, `equal_positions_break_on_name_bytes` `:216`) and one `version_in_force` test (`pinned_version_wins_over_latest` `:235`). Their `bound()`/`binding()` helpers gain fields, so the expected values change spelling (a level argument, `Some(n)`), not meaning. | §2.2.6. |
| **F-J** | Minor | T1: `a_preview_style_bound_skills_read_collapses_overrides` (`mem.rs:5967`) "still passing unchanged"; "`pg_criteria.rs:1806`'s equality test keeps passing with the new fields". | Both compare tuples `(name, skill.version, position)` with `("tests", 1, 0)` (`mem.rs:5977-5993`, `pg_criteria.rs:1847-1866`). `version` becomes `Option<i32>`, so the literals must read `Some(1)`, `Some(2)`. The whole-`Vec` equality (`pg_criteria.rs:1876-1881`) now also compares `level`, `activation` and `globs`, which holds only because the demo literals equal the column defaults (D59). | T1 edits both tuple lists; messages unchanged. |
| **F-K** | Minor | D43: "`places` is whatever `ParsedTemplate` already exposes … or adds a one-line `pub fn places`"; T2 lists `template.rs` "only if `places` is added". | `ParsedTemplate.used` is `pub` (`template.rs:259-260`) and `omits_item` already reads it (`:267-269`). | D54: `placed = parsed.used.contains(&Placeholder::Skills)`. `template.rs` leaves T2's list. |
| **F-L** | Minor | D45: `resolve_graph` goes "after the template reads (so an offline preview still refuses with `PROMPT_ON_SERVER_ONLY` first, `tests/prompt_preview.rs:343-356`)". | That test never reaches `preview::build`: `AgentRuntime::preview` answers `Backend::Offline` inline before spawning (`agent_worker.rs:1233-1238`), and the test asserts `background_len() == 0` (`prompt_preview.rs:325-362`). The ordering still matters for `build`'s own error on an offline arm (`prompt_templates` refuses with `PROMPT_ON_SERVER_ONLY`, `resolve_graph` with `DATABASE_UNREACHABLE`, `backend.rs:349-354`, `:488-494`). | Placement kept (D64); the reason is the one in this row. |
| **F-M** | Minor | D46: "active lines in the normal style and inactive lines dim". | The pane holds `lines: Vec<String>` (`prompt.rs:51`) and draws `Line::raw` (`:333`); there is no per-row style. Text snapshots (`Harness::render` = `buffer_text`) cannot show a style either. | D65: a `Row { text, dim }` model and a unit test on `render_lines` (§5.4). |
| **F-N** | Minor | D41: `MemStore` "pairs each with `skills[&skill_id].name`". | `State.skills` is a `HashMap` (`mem.rs:115`); indexing panics on a binding whose skill row is missing, where today's `State::bind` drops it (`mem.rs:1101-1103`). | D60: `filter_map` over `skills.get`. `State::bind` is deleted (its only caller is `bound_skills`, `mem.rs:436`). |
| **F-O** | Minor | D49 doc for `JUDGE_SEED_V1`: "the body `0001`..`0006` seeded". | No migration seeds a template: `create_project` inserts `DEFAULT_TEMPLATES` (`pg/write.rs:5057-5078`), and `load_demo` inserts the fixture's (`pg/demo.rs:235`). | The doc reads "the `judge` body `create_project` seeded before MOD-9 milestone 2 (`0007` upgrades a head still equal to it)". |
| **F-P** | Minor | `closed_rows_backfill_to_done`: "the plain `run` applies 0006" (`migrations.rs:734-736`, `:772`). | After T1, `MIGRATOR.run` applies `0006` **and** `0007`. The case still passes; its label and `expect("apply 0006")` become false. Citation drift elsewhere: `prompt_digest.rs:949` (not `:950`); `prompt_preview.rs:325-362` (not `:343-356`). | D69: `:772` becomes `MIGRATOR.run_to(6, &db.pool)`. |
| **F-Q** | Minor (record) | D44 asks for no change to the offline behaviour of `BackendGraphs`. | `Backend::bound_skills` refuses offline with `PROMPT_ON_SERVER_ONLY` (`backend.rs:372`) where the other five `GraphSource` reads say `DATABASE_UNREACHABLE`. Unreachable in practice: runs are online-only. | Nothing to change; `BackendGraphs::bound_skills` delegates like the others. |

### 0a. Settled answers to the brief's questions

| Question | Answer | Where |
|---|---|---|
| How does an empty `{{skills}}` render in the judge? | `substitute` pushes nothing for a placeholder with no live section (`prompt/mod.rs:540-546`), and `digest::canonical` collapses any run of ≥3 LFs to 2 (`digest.rs:36-64`). With `{{task}}\n{{skills}}\n\n{{candidates}}`, no skills gives `task` + `\n` + `` + `\n\n` + `candidates` = three LFs → two: **byte-identical to today's `{{task}}\n\n{{candidates}}`**. With skills: `task`, one LF, `skills`, a blank line, `candidates` — the phase bodies' single-LF stacking (`{{box}}\n{{skills}}\n{{excerpts}}`, `defaults.rs:34-36`) for the context pair, and the judge's own blank line before the candidates. `prompt_golden__prompt_judge_three_candidates.snap` (skills empty) is therefore unchanged and is D56's pin. The template-literal estimate grows by one LF (`estimate.rs:114-143`); no test pins a judge `template` token count. | D56, §2.5 |
| Every reader of `BoundSkill.version` | `render.rs:447` (renders `version="N"`: D53 skips `None`); `mem.rs:5977`, `:5990` and `pg_criteria.rs:1847`, `:1862` (test tuples: `Some(n)`, F-J). New readers: `select` (`None` → `missing_version`), `SkillChoice.version` (serialised `null`), the pane's `v<N|?>`. Nothing in `htui-orch` or `htui` reads it today (grep `skill.version`, `.skills[`). | §2.2, §3, §5.4 |
| Does `htui-store` enable `htui-core`'s `sqlx` feature? | Yes: `crates/htui-store/Cargo.toml:25` `htui-core = { …, features = ["sqlx"] }`, so `str_enum!`'s `sqlx::Type` (`model/mod.rs:34-35`) exists for `Activation` and `"activation: Activation"` decodes as `"isolation: Isolation"` does (`pg/read.rs:2214`). | §2.4 |
| `project_id` nullability after `DROP NOT NULL`; `TEXT[]`; `JSONB` | sqlx 0.9 reads `pg_attribute.attnotnull` for a table column (probed `f`), and an inner join adds no nullability, so `b.project_id` infers `Option<_>`; D61 spells it `"project_id?: ProjectId"` so the intent is in the text. `TEXT[] NOT NULL` → `Vec<String>`. `JSONB NOT NULL` → `serde_json::Value` (workspace sqlx has `json`, `Cargo.toml:43-45`; precedent `project_settings`, `pg/read.rs:1364-1372`). `serde_json::Value: Eq` (serde_json 1.0.151 `value/mod.rs:115`), so `SkillVersion` keeps `derive(Eq)`. | §2.4 |
| How can a test run `0001..0006`, plant, then `0007`? | `common::bare_db()` (`testkit.rs:69`) then `MIGRATOR.run_to(6, &db.pool)`, plant with runtime `sqlx::query`, then `MIGRATOR.run_to(7, &db.pool)`: the pattern `the_0004_bump_moves_only_an_untouched_six` (`migrations.rs:604-648`) and `closed_rows_backfill_to_done` (`:737-809`) already use. `bare_db` has no `app_user`, so the test inserts one (`:750-755`). | §2.9 |
| Would widening `allowed_in(Judge)` move a `templates__*` snapshot? | Yes, `templates__edit_help` only (F-E). | D68 |
| Engine/conformance tests at risk from demo phases now rendering skills | None change outcome. Evidence: every section assertion is `any`/`contains` (`htui-orch/src/conformance.rs:2702-2729`, `:2977-2989`, `:3236-3241`, `:4284-4294`; `engine.rs:6778-6783`; `tests/gix_isolator.rs:953-962`); notes assertions are `contains` (`engine.rs:6744-6765`); the demo `token_budget` is 120 000 (`fixtures.rs:702`) against ≈90 skill tokens (`backlog__detail_prompt.snap`: `skills 88`); no orch test pins a digest (plan Verified claims). `CASES` 70 (`tests/fake_conformance.rs:16`) is a count of cases, unchanged. The judge's demo template (`MemStore::demo`, v1 = new body) now renders a skills section in `ANA-2`'s judged `research` (project `htui`), again only `contains`-asserted. | — |
| Does the demo `FEAT-1`'s `implement` phase have `template_name == "implement"`? | Yes: `seed::phase_row` sets `template_name: phase.name` (`seed.rs:234`); FEAT-1 resolves to `feature` with four phases (`fake.rs:2107-2111`), and `PHASE_HTUI_IMPLEMENT` is that graph's `implement` (`fixtures.rs:301-307`). | §5 |
| `STAND_INS`, `SKILLS_NOTE` | Eight entries unchanged in number (`preview.rs:61-70`); `SKILLS_NOTE` (`:83-84`) re-worded (D45). | §5.1 |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (min., each compiles) | Gate |
|---|---|---|---|
| T1 storage + resolution + judge contract | htui-core, htui-store, htui (1 test + 1 snapshot) | 3 (§2.11) | core; store (Postgres); `cargo sqlx prepare --check`; `.sqlx` = 263; `cargo test -p htui --all-features --test templates`; `cargo build --workspace --all-features --all-targets`; clippy |
| T2 select + record | htui-core | 2 (§3.7) | core; then `htui-orch` and `htui` suites (golden and trim readers live there) |
| T3 engine | htui-orch, htui (`run_worker.rs`) | 2 (§4.8) | orch; htui lib (`run_worker`) |
| T4 preview + Prompt sub-tab | htui | 2 (§5.7) | htui (Postgres), snapshots reviewed |
| merge | — | T3, then T4 | after each, the touched crates' gates on the real tree; then the workspace gate (§8) |

Environment for every gate (this box):

```bash
export PG=postgres://postgres:htui@localhost:5432
export PGT="USERNAME=htui-ci HTUI_TEST_DATABASE_URL=$PG/postgres"
export ORT_LIB_LOCATION=/home/user/ort/package/bin/napi-v3/linux/x64   # R-19: parcel.pyke.io 403
export LD_LIBRARY_PATH=$ORT_LIB_LOCATION
export CARGO_TARGET_DIR=/home/user/htui/target                          # shared by T3/T4 worktrees
# `pg_isready -h localhost -p 5432` must say "accepting connections"
```

---

## 2. T1: storage and resolution (D38, D39, D41, D48, D49; D50–D61, D68, D69)

**Files** (plan's list **plus** F-A, F-E): `crates/htui-store/migrations/0007_skill_attachments.sql`
(new); `crates/htui-core/src/model/skill.rs`; `crates/htui-core/src/model/mod.rs`;
`crates/htui-core/src/fixtures.rs` (demo literals + one test); `crates/htui-core/src/store/mem.rs`;
`crates/htui-core/src/prompt/mod.rs` (the `collapse` call and the `skills` doc only);
`crates/htui-core/src/prompt/fixtures.rs`; `crates/htui-core/src/prompt/render.rs`;
`crates/htui-core/src/prompt/template.rs`;
`crates/htui-store/src/pg/read.rs`; `crates/htui-store/src/pg/rows.rs`;
**`crates/htui-store/src/pg/demo.rs`**; `crates/htui-store/.sqlx/` (−3, +2);
`crates/htui-store/tests/migrations.rs`; `crates/htui-store/tests/pg_criteria.rs`;
`crates/htui-store/tests/skill_attachments.rs` (new); **`crates/htui/tests/templates.rs`**;
`crates/htui/tests/snapshots/templates__edit_help.snap`.

**First failing test**: `model::skill::tests::phase_beats_project_beats_global`
(`cargo test -p htui-core --all-features --lib model::skill`).

### 2.1 `crates/htui-store/migrations/0007_skill_attachments.sql` (new), complete

Probed (2026-09-26, Postgres 16.13): applied after `0001`..`0006`; the `trim_record` comment read
back equal to the Rust literal of §2.8. **Amended 2026-09-26 (D49 withdrawn):** the judge-upgrade
section that the first draft carried is gone; the file is sections 1 and 2 only.

````sql
-- 0007_skill_attachments.sql - MOD-9 milestone 2 (docs/ANA-22.md 7.1; plan D38, D42).
-- Forward-only (R-STO-5).
--
-- 1. ANA-22 7.1 verbatim. skill_version keeps its import provenance, and skill_binding becomes the
--    attachment at three levels: project_id NULL is global (every project), phase_id NULL is the
--    project level, both set is one phase. The existing UNIQUE NULLS NOT DISTINCT (skill_id,
--    project_id, phase_id) already allows exactly one global row per skill, and project_id keeps
--    ON DELETE CASCADE, which never fires for a NULL key. Existing rows read activation 'always',
--    globs '{}' and languages '{}': every binding keeps today's behaviour.
-- 2. run_step.trim_record's contract is restated for record v 2 (plan D42): skill_choices[].

-- --------------------------------------------------------------------------------------------
-- 1. ANA-22 7.1: skill attachments
-- --------------------------------------------------------------------------------------------

ALTER TABLE skill_version
    ADD COLUMN source JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE skill_binding
    ALTER COLUMN project_id DROP NOT NULL,
    ADD COLUMN activation TEXT   NOT NULL DEFAULT 'always'
        CHECK (activation IN ('always', 'glob', 'off')),
    ADD COLUMN globs      TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN languages  TEXT[] NOT NULL DEFAULT '{}',
    ADD CONSTRAINT skill_binding_phase_needs_project
        CHECK (phase_id IS NULL OR project_id IS NOT NULL),
    ADD CONSTRAINT skill_binding_glob_needs_globs
        CHECK (activation <> 'glob' OR cardinality(globs) > 0);

COMMENT ON COLUMN skill_version.source IS
    'import provenance and raw frontmatter; prefills an attachment, never read by the prompt builder';
COMMENT ON COLUMN skill_binding.project_id IS 'NULL = global attachment (every project)';
COMMENT ON COLUMN skill_binding.activation IS
    'always | glob | off; the most specific attachment of a skill wins (ANA-22)';
COMMENT ON COLUMN skill_binding.globs IS
    'effective globs: typed plus languages expanded at save; <repo>:<glob> only on project or phase rows';
COMMENT ON COLUMN skill_binding.languages IS 'languages as authored; display only';

-- --------------------------------------------------------------------------------------------
-- 2. MOD-9 D42: the trim record's v 2
-- --------------------------------------------------------------------------------------------

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42: {v, template, budget, budget_source, reserve, target, '
  'estimator, estimated_before, estimated_after, sections[], skill_choices[], excerpts, notes}, '
  'v 2. skill_choices[] is every candidate skill, ordered by position then name, each {skill, '
  'name, version, level, activation, active, reason} with reason always, off, no_path, '
  'missing_version or not_placed (ANA-22 6 item 8). A v 1 record, written before 0007, has no '
  'skill_choices. Canonical; the prompt payload sections[] array is its abridged projection. '
  'Written at stage 3 by set_step_prompt, before the session starts.';

````

Notes for the implementer: the file is LF-only. The skill tables are not mirrored
(`cache/mod.rs:44`), so there is no cache migration.

### 2.2 `crates/htui-core/src/model/skill.rs`

Module doc (`:1-7`): replace "Read-only in this milestone (plan D105) …" with: "Read-only still:
the writers are MOD-9 milestone 3's. Since MOD-9 milestone 2 (ANA-22 §6-§7) a skill attaches
globally, to a project or to one phase; [`resolve`] picks the most specific attachment per skill,
and [`select`] decides, per step, which winners render and records why."

Imports: `use serde_json::Value;` is not needed at the top (write `serde_json::Value` in the field);
add `SkillLevel`/`Activation` definitions in this file.

#### 2.2.1 `Activation` (D39, via `str_enum!`, `model/mod.rs:25-70`)

```rust
str_enum!(
    /// `skill_binding.activation` (ANA-22 §6 item 4): whether the winning attachment puts its
    /// skill into a step's prompt.
    Activation {
        /// Always rendered. The column default, so every binding written before `0007` is unchanged.
        Always => "always",
        /// Rendered when the step's file set matches `globs` (MOD-9 milestone 3). Until the matcher
        /// lands, a `glob` winner is inactive and records `no_path` (plan D40, OQ-12).
        Glob => "glob",
        /// Attached more broadly but not here: a narrower `off` hides a broader attachment.
        Off => "off",
    }
);
```

#### 2.2.2 `SkillLevel` (D50) — plain enum, not a column

```rust
/// Which level an attachment sits at (ANA-22 §6 item 2). **Declaration order is specificity**:
/// `Global < Project < Phase`, so the attachment that wins is the `max` (`R-SKL-2` as amended).
///
/// Not a column — it is derived from the two nullable keys by [`SkillBinding::level`] — so it is a
/// plain enum rather than a `str_enum!`, and its serde spelling is its [`as_str`](Self::as_str).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLevel {
    /// `project_id IS NULL`: every project.
    Global,
    /// `project_id` set, `phase_id IS NULL`.
    Project,
    /// Both set: one phase of one project.
    Phase,
}

impl SkillLevel {
    /// `global`, `project` or `phase`: the record's and the Prompt sub-tab's spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Phase => "phase",
        }
    }
}
```

#### 2.2.3 Row types

`SkillVersion` gains one field, after `body`:

```rust
    /// `skill_version.source` (ANA-22 §6 item 11): import provenance and the raw frontmatter.
    /// Never read by the prompt builder; `{}` for a version that was not imported.
    pub source: serde_json::Value,
```

`SkillBinding` (doc `:48-52` becomes "A row of `skill_binding` (§5.6 as amended by ANA-22 §7.1):
one attachment of a skill, globally, to a project, or to one phase of it. `UNIQUE NULLS NOT DISTINCT
(skill_id, project_id, phase_id)` allows one attachment per skill per level, which is why [`resolve`]
has to pick one."). Fields, in this order:

```rust
    /// `skill_binding.id`.
    pub id: SkillBindingId,
    /// `skill_binding.skill_id`.
    pub skill_id: SkillId,
    /// `skill_binding.project_id`; `None` is a global attachment (every project).
    pub project_id: Option<ProjectId>,
    /// `skill_binding.phase_id`; `None` is the project (or global) level. `Some` requires
    /// `project_id` (`skill_binding_phase_needs_project`).
    pub phase_id: Option<PhaseId>,
    /// `skill_binding.pinned_version`; `None` follows the latest version.
    pub pinned_version: Option<i32>,
    /// `skill_binding.position`: ascending render order, `skill.name` bytes breaking the tie.
    pub position: i32,
    /// `skill_binding.activation`.
    pub activation: Activation,
    /// `skill_binding.globs`: the effective globs, non-empty when `activation` is `Glob`
    /// (`skill_binding_glob_needs_globs`). Nothing matches them before MOD-9 milestone 3.
    pub globs: Vec<String>,
    /// `skill_binding.languages`: as authored, display only; the matcher reads `globs`.
    pub languages: Vec<String>,
    /// `skill_binding.updated_at`.
    pub updated_at: DateTime<Utc>,
```

and, in `impl SkillBinding` beside `version_in_force` (unchanged):

```rust
    /// The level this attachment sits at, from its two nullable keys. A `(None, Some(_))` row is
    /// refused by `skill_binding_phase_needs_project`; were one ever read, it is `Global`, the
    /// same answer `PgStore`'s `WHERE b.project_id IS NULL` gives it.
    #[must_use]
    pub fn level(&self) -> SkillLevel {
        match (self.project_id, self.phase_id) {
            (None, _) => SkillLevel::Global,
            (Some(_), None) => SkillLevel::Project,
            (Some(_), Some(_)) => SkillLevel::Phase,
        }
    }
```

`BoundSkill` (doc keeps its two paragraphs; the first sentence becomes "One step's candidate skill:
the winning attachment of one skill, resolved to a version and a body."). Fields:

```rust
    /// `skill.id`, the key the most-specific-wins collapse is resolved on.
    pub skill_id: SkillId,
    /// `skill.name`, rendered as the `<skill name="..">` attribute and the order's tie-break.
    pub name: String,
    /// `skill_version.version` in force, rendered as `version="N"`. `None` when the winning
    /// attachment's pin names no version, or the skill has none: the skill then renders nothing
    /// and records `missing_version`, with no fallback to a broader attachment (plan D39, OQ-9).
    pub version: Option<i32>,
    /// `skill_binding.position` of the attachment that won.
    pub position: i32,
    /// `skill_version.body`, inlined verbatim; empty when `version` is `None`.
    pub body: String,
    /// The level of the attachment that won.
    pub level: SkillLevel,
    /// The winning attachment's activation, which [`select`] reads.
    pub activation: Activation,
    /// The winning attachment's globs, for MOD-9 milestone 3's matcher. Recorded nowhere yet.
    pub globs: Vec<String>,
```

#### 2.2.4 `collapse` (one list, D39, D52) and `resolve` (D39)

```rust
impl BoundSkill {
    /// `R-SKL-2` as amended (ANA-22 §6 item 3): per `skill_id`, the candidate at the **most
    /// specific** level wins (phase over project over global) with its own pin, position and
    /// activation; the result is ordered by `(position, name bytes)`; and **each skill appears
    /// exactly once**.
    ///
    /// (OpenHands paragraph and "Pure, and the single definition of the order" paragraph kept.)
    ///
    /// Two candidates of one skill at one level cannot come from a store — `UNIQUE NULLS NOT
    /// DISTINCT (skill_id, project_id, phase_id)` and one project and at most one phase per read —
    /// but the assembler re-collapses a caller's list, so the tie is defined: the first in input
    /// order wins.
    #[must_use]
    pub fn collapse(mut skills: Vec<Self>) -> Vec<Self> {
        // Most specific first. `sort_by` is stable, so equal levels keep their input order.
        skills.sort_by(|a, b| b.level.cmp(&a.level));
        let mut resolved: Vec<Self> = Vec::with_capacity(skills.len());
        for skill in skills {
            if !resolved.iter().any(|kept| kept.skill_id == skill.skill_id) {
                resolved.push(skill);
            }
        }
        // (the existing `as_bytes` comment)
        resolved.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
        });
        resolved
    }
}

/// One step's candidates from its attachment rows (plan D39): each row, paired with its joined
/// `skill.name`, becomes a [`BoundSkill`] at its own level with the version its own pin puts in
/// force, and [`BoundSkill::collapse`] keeps the most specific per skill.
///
/// `rows` are every attachment that applies to the step — the global ones, the project's, and the
/// phase's — and `versions` may hold any skill's rows, as [`SkillBinding::version_in_force`]
/// allows. **The winner is picked by level before its version is looked at**: a winning pin that
/// names no version yields `version: None` rather than the broader attachment's body (OQ-9). Both
/// stores call this, so "which attachment wins" has one definition.
#[must_use]
pub fn resolve(rows: Vec<(SkillBinding, String)>, versions: &[SkillVersion]) -> Vec<BoundSkill> {
    let candidates = rows
        .into_iter()
        .map(|(binding, name)| {
            let in_force = binding.version_in_force(versions);
            BoundSkill {
                skill_id: binding.skill_id,
                name,
                version: in_force.map(|version| version.version),
                position: binding.position,
                body: in_force.map(|version| version.body.clone()).unwrap_or_default(),
                level: binding.level(),
                activation: binding.activation,
                globs: binding.globs,
            }
        })
        .collect();
    BoundSkill::collapse(candidates)
}
```

(`level` is evaluated before `globs` moves out of `binding`: struct-literal fields evaluate in
source order.)

#### 2.2.5 `SkillChoice`, `ChoiceReason`, `select` (D40, D51 — here, not in `prompt/`)

```rust
/// Why a candidate did or did not render (plan D40, ANA-22 §6 item 8). Serialised snake_case into
/// `trim_record.skill_choices[].reason`. MOD-9 milestone 3 adds `matched` (with the path) and
/// `no_match`; no variant here is renamed then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceReason {
    /// `activation = always`: rendered. The only active reason.
    Always,
    /// `activation = off` on the winning attachment.
    Off,
    /// `activation = glob` and no repo root resolves for the step (every step, until milestone 3).
    NoPath,
    /// The winning attachment's pin names no version, or the skill has none.
    MissingVersion,
    /// The template body places no `{{skills}}`, so nothing could render.
    NotPlaced,
}

impl ChoiceReason {
    /// The serde spelling: `always`, `off`, `no_path`, `missing_version`, `not_placed`.
    #[must_use]
    pub const fn as_str(self) -> &'static str { /* one arm per variant */ }
}

/// One candidate's line in `trim_record.skill_choices` (plan D42): what was attached, at which
/// level, and whether it rendered. `name` is the masked name — the assembler scrubs every
/// candidate before it selects (plan D43).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillChoice {
    /// `skill.id`.
    pub skill: SkillId,
    /// `skill.name`, masked.
    pub name: String,
    /// The version in force; `null` in the record when none is.
    pub version: Option<i32>,
    /// The winning attachment's level.
    pub level: SkillLevel,
    /// The winning attachment's activation.
    pub activation: Activation,
    /// Whether the skill rendered.
    pub active: bool,
    /// Why.
    pub reason: ChoiceReason,
}

/// Plan D40: decides every candidate, in order, for one step. Pure.
///
/// The rules, first match wins: a body that does not place `{{skills}}` (`placed == false`) makes
/// every candidate `not_placed`; `version: None` is `missing_version`; `Off` is `off`; `Glob` is
/// `no_path` (no step resolves a root before milestone 3, OQ-12); `Always` is `always` and the only
/// active outcome. Returns the active candidates in input order — which is collapse order, the
/// render order — and one [`SkillChoice`] per candidate in the same order.
#[must_use]
pub fn select(candidates: Vec<BoundSkill>, placed: bool) -> (Vec<BoundSkill>, Vec<SkillChoice>) {
    let mut active = Vec::with_capacity(candidates.len());
    let mut choices = Vec::with_capacity(candidates.len());
    for skill in candidates {
        let reason = if !placed {
            ChoiceReason::NotPlaced
        } else if skill.version.is_none() {
            ChoiceReason::MissingVersion
        } else {
            match skill.activation {
                Activation::Off => ChoiceReason::Off,
                Activation::Glob => ChoiceReason::NoPath,
                Activation::Always => ChoiceReason::Always,
            }
        };
        let is_active = reason == ChoiceReason::Always;
        choices.push(SkillChoice {
            skill: skill.skill_id,
            name: skill.name.clone(),
            version: skill.version,
            level: skill.level,
            activation: skill.activation,
            active: is_active,
            reason,
        });
        if is_active {
            active.push(skill);
        }
    }
    (active, choices)
}
```

`model/mod.rs:143` becomes `pub use skill::{Activation, BoundSkill, ChoiceReason, Skill,
SkillBinding, SkillChoice, SkillLevel, SkillVersion};` (`resolve`/`select` stay path-qualified,
`model::skill::{resolve, select}`, D51).

#### 2.2.6 Tests (`model/skill.rs` `mod tests`; one in `model/mod.rs`)

Helpers change: `bound(skill_id, name, version: i32, position, level: SkillLevel)` builds
`version: Some(version)`, `activation: Activation::Always`, `globs: Vec::new()`;
`binding(skill_id, pinned)` gains `project_id: Some(ProjectId::new())`, `activation:
Activation::Always`, `globs`/`languages: Vec::new()`; a new `attach(skill_id, project:
Option<ProjectId>, phase: Option<PhaseId>, pinned, position, activation) -> (SkillBinding,
String)`; `version(..)` gains `source: serde_json::json!({})`.

| Test | Outline |
|---|---|
| `phase_overrides_project_once` (ported, F-I) | `collapse([project…, phase…].concat())`, project items at `SkillLevel::Project`, phase at `Phase`; the two assertions unchanged in meaning (expected values gain the level argument). |
| `equal_positions_break_on_name_bytes` (ported) | `collapse(list)` of three `Project` items; names `["Zebra", "apple", "Ångström"]`. |
| `pinned_version_wins_over_latest` (kept) | Unchanged but for the helper fields. |
| `phase_beats_project_beats_global` | One skill, versions 1..=3; rows global (pin 1, pos 7), project (pin 2, pos 5), phase (pin 3, pos 2). `resolve` → one `BoundSkill`, `level: Phase`, `version: Some(3)`, `position: 2`, body `"v3"`. Drop the phase row → `Project`, `Some(2)`, 5. Drop that → `Global`, `Some(1)`, 7. |
| `off_at_a_narrower_level_wins_and_keeps_the_skill_out_of_broader_levels` | global `Always` + project `Off` → one candidate, `Off`, `Project`; `select(.., true)` → no active, one choice `{active: false, reason: Off, level: Project}`. |
| `a_missing_pin_on_the_winner_does_not_fall_back` | project row pinned 1 (exists), phase row pinned 7 (missing) → `level: Phase`, `version: None`, `body: ""`, `position` = the phase row's; **not** v1. |
| `global_rows_order_by_position_then_name_bytes` | three global rows at position 0 (`apple`, `Ångström`, `Zebra`) and one at −1 → `[-1 row, Zebra, apple, Ångström]`. |
| `level_of_a_binding_follows_its_nullable_keys` | `(None, None)` Global, `(Some, None)` Project, `(Some, Some)` Phase, `(None, Some)` Global. |
| `select_applies_its_rules_in_order` | candidates `[Always Some(1), Off Some(1), Glob Some(1), Always None]`: `placed = true` → reasons `[always, off, no_path, missing_version]`, active = the first only, choices in input order; `placed = false` → four `not_placed`, none active. |
| `choices_serialize_their_documented_keys` | `serde_json::to_value(choice)` has exactly `{skill, name, version, level, activation, active, reason}`; `ChoiceReason::MissingVersion` → `"missing_version"`, `SkillLevel::Phase` → `"phase"`, `version: None` → `null`. |
| `model/mod.rs`: `activation_matches_check_list` (the plan's `activation_round_trips_its_db_text`, D51: beside the other `check_enum` tests, `model/mod.rs:178-260`) | `check_enum(Activation::ALL, &["always", "glob", "off"])`. |

### 2.3 `crates/htui-core/src/store/mem.rs` (D41, D60)

- `:118` doc: "`skill_binding`, resolved through `model::skill::resolve`."
- `:409-444` `bound_skills`: doc becomes "The skill candidates of one step (ANA-22 §6 items 2-3): the
  global attachments, the project's, and — with `phase` — that phase's, resolved most-specific-wins
  by [`resolve`](crate::model::skill::resolve). Inactive winners are included; the assembler's
  `select` decides and records them. A binding whose `skill` row is missing is dropped. # Errors
  Never; …". Body:

```rust
        Ok(self.read(|state| {
            let rows = state
                .skill_bindings
                .iter()
                .filter(|binding| match binding.project_id {
                    None => true,
                    Some(owner) => {
                        owner == project
                            && (binding.phase_id.is_none() || binding.phase_id == phase)
                    }
                })
                .filter_map(|binding| {
                    state
                        .skills
                        .get(&binding.skill_id)
                        .map(|skill| (binding.clone(), skill.name.clone()))
                })
                .collect();
            crate::model::skill::resolve(rows, &state.skill_versions)
        }))
```

  (`phase: None` with a `Some` phase row: `binding.phase_id == phase` is false — the row is
  excluded, as `PgStore`'s `b.phase_id = NULL` is.)
- `:1097-1111` `State::bind`: **deleted** (F-N; its only caller was `:436`).
- `:3088-3093` `project_reach`: `.filter(|row| row.project_id == Some(id))`.
- `:3269` `delete_project`: `self.skill_bindings.retain(|row| row.project_id != Some(id));`
- `:6833-6839` test: `.all(|row| row.project_id != Some(gone))`.
- `:5966-6010` `a_preview_style_bound_skills_read_collapses_overrides`: the two tuple lists read
  `vec![("tests", Some(1), 0), ("rust-style", Some(2), 1)]` and
  `vec![("tests", Some(1), 0), ("rust-style", Some(1), 2)]` (F-J). Messages unchanged.
- New tests (in `mem.rs`'s `mod tests`, `#[tokio::test]`), over `MemStore::from_demo(data)` where
  `data = demo_data()` gains skill `house` (`SkillId::new()`, v1 body `"House rules."`,
  `source: json!({})`) and one global binding (`project_id: None`, `phase_id: None`, position 5,
  `Always`):
  - `a_global_attachment_reaches_every_project`: `bound_skills(PROJECT_AGY, None)` is exactly
    `[house, Global, Some(1), 5]`; `bound_skills(PROJECT_HTUI, Some(PHASE_HTUI_IMPLEMENT))` is
    `[tests Project, rust-style Phase v1, house Global]` in that order.
  - `delete_project_keeps_global_attachments`: `delete_project(PROJECT_HTUI)` answers
    `skill_bindings == 3` (the project's own), and `bound_skills(PROJECT_AGY, None)` still holds
    `house`.

### 2.4 `crates/htui-store/src/pg/read.rs:1433-1507` and `pg/rows.rs:249-300` (D41, D61)

`rows.rs` imports (`:28-32`): drop `BoundSkill` and `SkillVersion` (unused after `bind` goes;
`unused_imports` under `-D warnings`), add `Activation`. `SkillBindingRow` (doc: "One
`skill_binding` row with `skill.name` joined: one attachment `PgStore::bound_skills` hands to
`htui_core::model::skill::resolve`."):

```rust
pub(crate) struct SkillBindingRow {
    pub(crate) id: SkillBindingId,
    pub(crate) skill_id: SkillId,
    /// `skill_binding.project_id`; `None` is a global attachment.
    pub(crate) project_id: Option<ProjectId>,
    pub(crate) phase_id: Option<PhaseId>,
    pub(crate) pinned_version: Option<i32>,
    pub(crate) position: i32,
    pub(crate) activation: Activation,
    pub(crate) globs: Vec<String>,
    pub(crate) languages: Vec<String>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) name: String,
}

impl SkillBindingRow {
    /// The row as the model's [`SkillBinding`], paired with its joined `skill.name`: the shape
    /// `resolve` takes.
    pub(crate) fn into_binding(self) -> (SkillBinding, String) {
        (
            SkillBinding {
                id: self.id,
                skill_id: self.skill_id,
                project_id: self.project_id,
                phase_id: self.phase_id,
                pinned_version: self.pinned_version,
                position: self.position,
                activation: self.activation,
                globs: self.globs,
                languages: self.languages,
                updated_at: self.updated_at,
            },
            self.name,
        )
    }
}
```

`read.rs` imports: add `Activation`. `bound_skills` doc: "The skill candidates of one step … Two
`SELECT`s — every version of every candidate skill, and the candidate attachments (global rows, the
project's, and the phase's) — then `htui_core::model::skill::resolve` in Rust, exactly as
`MemStore` does. … A winning pin that names no version is a candidate with `version: None`
(plan D39)." Body:

```rust
        let phase = phase.map(PhaseId::as_uuid);
        let versions = sqlx::query_as!(
            SkillVersion,
            r#"
            SELECT v.skill_id   AS "skill_id: SkillId",
                   v.version,
                   v.body,
                   v.source,
                   v.created_by AS "created_by: htui_core::model::UserId",
                   v.created_at
              FROM skill_version v
             WHERE v.skill_id IN (
                   SELECT b.skill_id
                     FROM skill_binding b
                    WHERE b.project_id IS NULL
                       OR (b.project_id = $1 AND (b.phase_id IS NULL OR b.phase_id = $2)))
            "#,
            project.as_uuid(),
            phase,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let rows = sqlx::query_as!(
            SkillBindingRow,
            r#"
            SELECT b.id         AS "id: htui_core::model::SkillBindingId",
                   b.skill_id   AS "skill_id: SkillId",
                   b.project_id AS "project_id?: ProjectId",
                   b.phase_id   AS "phase_id: PhaseId",
                   b.pinned_version,
                   b.position,
                   b.activation AS "activation: Activation",
                   b.globs,
                   b.languages,
                   b.updated_at,
                   s.name
              FROM skill_binding b JOIN skill s ON s.id = b.skill_id
             WHERE b.project_id IS NULL
                OR (b.project_id = $1 AND (b.phase_id IS NULL OR b.phase_id = $2))
            "#,
            project.as_uuid(),
            phase,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(htui_core::model::skill::resolve(
            rows.into_iter().map(SkillBindingRow::into_binding).collect(),
            &versions,
        ))
```

`$2` is inferred `uuid` from `b.phase_id = $2`; `None` binds SQL `NULL`, so `b.phase_id = $2` is
NULL and only `phase_id IS NULL` rows of the project apply. `"phase_id: PhaseId"` already infers
`Option` (nullable column). `.sqlx`: `query-0576e113…`, `query-5a620677…`, `query-a1d3a8d1…` go;
two arrive: **263**.

### 2.5 `crates/htui-core/src/prompt/` (T1's share)

- `render.rs:436-460` `skills` (D53):

```rust
pub fn skills(skills: &[BoundSkill]) -> Option<Rendered> {
    let blocks: Vec<String> = skills
        .iter()
        .filter_map(|skill| {
            // MOD-9 D53: a candidate with no version in force renders nothing. The assembler never
            // passes one — `select` records it `missing_version` — so this is the pure function's
            // own guard, not a path.
            let version = skill.version?;
            let body = content_of(&skill.body);
            let open = format!("<skill name=\"{}\" version=\"{version}\">", attr(&skill.name));
            Some(if body.is_empty() {
                format!("{open}\n</skill>")
            } else {
                format!("{open}\n{body}\n</skill>")
            })
        })
        .collect();
    if blocks.is_empty() {
        return None;
    }
    Some(Rendered {
        name: SectionName::Skills,
        attrs: Vec::new(),
        content: blocks.join("\n"),
    })
}
```

  Doc: "`None` when no candidate has a version: §4.2's vocabulary has no empty `skills` section."
  Test `skills_blocks_carry_their_own_closing_tag` (`:1212-1240`): literals gain `version: Some(3)`
  / `Some(1)`, `level: SkillLevel::Project`, `activation: Activation::Always`, `globs: Vec::new()`;
  one new assertion: a list of one `version: None` skill renders `None`.
- `prompt/fixtures.rs` BoundSkill literals `:221-227`, `:678-684`, `:698-711`: `version: Some(2)`
  (resp. `Some(2)`, `Some(2)`, `Some(1)`), `level: SkillLevel::Project`, `activation:
  Activation::Always`, `globs: Vec::new()`. Nothing else moves: every fixture skill stays `Always`
  and versioned, so every rendered byte, golden and digest is unchanged by T1.
- `prompt/mod.rs:835`: `let skills = BoundSkill::collapse(spec.skills.clone());` (T2 replaces this
  line). `:97` doc of `PromptSpec.skills`: "The step's skill candidates, resolved by
  `model::skill::resolve`: global, project and phase attachments, most specific winning, inactive
  ones included. The assembler collapses and selects (MOD-9 D43)."
- `defaults.rs`: **unchanged** (D49 withdrawn 2026-09-26). The default `judge` body does not place
  `{{skills}}`; a maintainer adds it per project in the Templates view. `every_phase_body_is_wrong_role_for_…`
  (`:326-379`) still holds: no phase body's first judge-illegal token is `skills` (D48 verified:
  `item_kind`/`attempt`/`item` all precede it).
- `template.rs:196-201` `allowed_in`: `Self::ItemKey | Self::Phase | Self::Skills | Self::Task |
  Self::Candidates`. Doc (`:185`): "… (ANA-5 §4.1 `:319-341`, with `skills` added to the judge by
  MOD-9 D48)". Test pin `:476-480`: `vec!["item_key", "phase", "skills", "task", "candidates"]`,
  message `"ANA-5 §4.1 `:339`, plus skills (MOD-9 D48)"`. New tests:
  `skills_is_allowed_in_a_judge_body` (`parse(Judge, "{{task}}\n{{skills}}\n{{candidates}}")`
  succeeds and `used` contains `Skills`) and `skills_is_still_refused_in_a_handoff_body`
  (`parse(Handoff, "{{skills}}")` is `WrongRole { token: "skills", role: Handoff, at: 0 }`).

### 2.6 `crates/htui-core/src/fixtures.rs` (demo literals only, D59)

- `:556-562` `SkillVersion { …, source: json!({}), … }`.
- `:592-600` `SkillBinding { …, project_id: Some(ids::PROJECT_HTUI), …, activation:
  Activation::Always, globs: Vec::new(), languages: Vec::new(), … }`. No row is added (D47).
- Import `Activation` (`:26`).
- New test `demo_skill_rows_use_the_column_defaults`: every demo binding is `Always` with empty
  `globs`/`languages`, and every demo version's `source` is `{}` — "the demo loader
  (`pg/demo.rs:286-315`) writes none of the four `0007` columns, so Postgres reads their defaults;
  a fixture row that differed would make `inherent_prompt_reads_answer_the_fixture` compare two
  different stores".

### 2.7 `crates/htui-store/src/pg/demo.rs:306` (F-A, D59)

`ProjectId::as_uuid(row.project_id),` → `row.project_id.map(ProjectId::as_uuid),`. Text unchanged.

### 2.8 `crates/htui-store/tests/migrations.rs` (D38, D58, D69)

| Where | Change |
|---|---|
| `:79-86` | `vec![1, 2, 3, 4, 5, 6, 7]`; message appends "and MOD-9 milestone 2's 0007_skill_attachments.sql". |
| `:168-176` doc | "…the last nineteen are ANA-2 §9. `run_step.trim_record` is the text `0007_skill_attachments.sql` restates (MOD-9 D42), which replaces `0002`'s." |
| `:206-213` | the trim_record literal below. |
| after `:369` | `MOD9_COLUMN_COMMENTS` below. |
| `:386`, `:436`, `:448` | `.chain(MOD7_COLUMN_COMMENTS).chain(MOD9_COLUMN_COMMENTS)` |
| `:424` comment | "…exactly the thirty-four contracts the three ANAs, MOD-7 and ANA-22 wrote and no half-finished thirty-fifth." |
| `:454` | `"exactly the thirty-four commented columns, and no others"` |
| `:772` | `MIGRATOR.run_to(6, &db.pool).await.expect("apply 0006");` (F-P) |
| `:831-835` | `MigrationState::Pending(7)`, `"seven embedded migrations, none applied"` |
| `TABLES` `:17-60`, `:106-110` | unchanged (39). |

```rust
    (
        "run_step",
        "trim_record",
        "ANA-5 5.1 as amended by MOD-9 D42: {v, template, budget, budget_source, reserve, target, \
         estimator, estimated_before, estimated_after, sections[], skill_choices[], excerpts, \
         notes}, v 2. skill_choices[] is every candidate skill, ordered by position then name, \
         each {skill, name, version, level, activation, active, reason} with reason always, off, \
         no_path, missing_version or not_placed (ANA-22 6 item 8). A v 1 record, written before \
         0007, has no skill_choices. Canonical; the prompt payload sections[] array is its \
         abridged projection. Written at stage 3 by set_step_prompt, before the session starts.",
    ),
```

(Verified: with Rust's `\`-newline continuation this is byte-equal to §2.1's SQL literal.)

```rust
/// The five `COMMENT ON COLUMN` texts of `0007_skill_attachments.sql` (ANA-22 §7.1, MOD-9 plan
/// D38), verbatim, for [`ANA_COLUMN_COMMENTS`]'s reason.
const MOD9_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
    (
        "skill_version",
        "source",
        "import provenance and raw frontmatter; prefills an attachment, never read by the prompt \
         builder",
    ),
    ("skill_binding", "project_id", "NULL = global attachment (every project)"),
    (
        "skill_binding",
        "activation",
        "always | glob | off; the most specific attachment of a skill wins (ANA-22)",
    ),
    (
        "skill_binding",
        "globs",
        "effective globs: typed plus languages expanded at save; <repo>:<glob> only on project or \
         phase rows",
    ),
    ("skill_binding", "languages", "languages as authored; display only"),
];
```

No D49 tests (D58 withdrawn with D49).

### 2.9 `crates/htui-store/tests/skill_attachments.rs` (new)

`#![cfg(feature = "demo")]`, `use htui_store::testkit as common;`, runtime `sqlx::query` only (no
`.sqlx` file, the `prompt_template_cas.rs` rule). Helpers: `plant_skill(pool, name) -> SkillId`
(skill + v1 body `"{name} rules."`), `plant_binding(pool, skill, project: Option<ProjectId>, phase:
Option<PhaseId>, activation: &str, globs: &[&str], position: i32) -> Result<(), sqlx::Error>`, and
`mem_with(edit)` = `MemStore::from_demo` over `demo_data()` edited identically.

| Test | Outline |
|---|---|
| `pg_and_mem_agree_on_global_project_and_phase` | Plant on both: skill `house` global `always` pos 5; `tests` global `off` pos 3; `rust-style` global `always` pos 9; `tests` on `implement` `off` pos 0. For `(PROJECT_HTUI, None)`, `(PROJECT_HTUI, Some(PHASE_HTUI_IMPLEMENT))`, `(PROJECT_AGY, None)`: `db.store.bound_skills(..) == mem.bound_skills(..)`. Spell out the implement answer: `[tests Phase Off Some(1) 0, rust-style Phase Always Some(1) 2, house Global Always Some(1) 5]`. |
| `an_off_phase_attachment_hides_a_project_skill` | `tests` on `implement` `off`: the Postgres candidates carry `tests` at `Phase`/`Off`, and `select(candidates, true)` leaves only `rust-style` active with `tests` recorded `off`. |
| `a_global_row_survives_project_delete` | Plant `house` global; `delete_project(PROJECT_HTUI)` reports `skill_bindings == 3`; `count(*) … WHERE project_id IS NULL` is 1; `bound_skills(PROJECT_AGY, None)` holds `house`. |
| `the_checks_refuse_a_phase_row_without_a_project_and_glob_without_globs` | Each insert fails with the named constraint in the database error (probed): `(None, Some(phase))` → `skill_binding_phase_needs_project`; `glob` with `'{}'` → `skill_binding_glob_needs_globs`; `'sometimes'` → `skill_binding_activation_check`; a second global row of one skill → `skill_binding_skill_id_project_id_phase_id_key`. And the three demo rows read `always`, `{}`, `{}`. |

### 2.10 `crates/htui-store/tests/pg_criteria.rs:1847-1866` (F-J)

The two tuple lists read `Some(1)`/`Some(2)` as in §2.3. The `Vec` equality (`:1876-1881`) is
unchanged in text.

### 2.11 Gate and commits

```bash
# .sqlx against a scratch DB migrated from zero (drop and recreate before every prepare)
psql "$PG/postgres" -c 'DROP DATABASE IF EXISTS htui_prepare_mod9m2' -c 'CREATE DATABASE htui_prepare_mod9m2'
DATABASE_URL=$PG/htui_prepare_mod9m2 sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_mod9m2 cargo sqlx prepare -- --all-features --all-targets)
ls crates/htui-store/.sqlx | wc -l          # 263
cargo sqlx prepare --check                  # from crates/htui-store, same DATABASE_URL
RUST_BACKTRACE=0 cargo test -p htui-core --all-features -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui-store --all-features -- --test-threads=2
cargo test -p htui --all-features --test templates     # review + accept templates__edit_help
cargo build --workspace --all-features --all-targets
cargo clippy -p htui-core -p htui-store --all-features --all-targets -- -D warnings
```

Commits: (1) red — model types, `resolve`/`select`/`collapse` with `todo!()` bodies, the new tests;
(2) green — model, `mem.rs`, `prompt/{render,fixtures,mod}.rs`, `fixtures.rs`, `defaults.rs`,
`template.rs`; (3) Postgres — `0007`, `pg/{read,rows,demo}.rs`, `.sqlx`, `migrations.rs`,
`pg_criteria.rs`, `skill_attachments.rs`, `htui/tests/templates.rs` + the snapshot.

---

## 3. T2: selection and the record (D40, D42, D43; D54, D55)

**Files**: `crates/htui-core/src/prompt/mod.rs`, `crates/htui-core/src/prompt/trim.rs`,
`crates/htui-core/src/fixtures.rs` (`IMPL_TRIM_RECORD` only), `crates/htui-core/tests/prompt_digest.rs`,
`crates/htui-core/tests/prompt_skills.rs` (new). **Not** `template.rs` (F-K).

### 3.1 `prompt/mod.rs`

- Import `use crate::model::skill::{BoundSkill, SkillChoice, select};` (`:56`).
- `scrubbed_inputs` `:833-845`: after the masking loops (unchanged — every candidate's `name` and
  `body` is masked, `:755-759`):

```rust
    let mut upstream = spec.upstream.clone();
    UpstreamEntry::sort_canonical(&mut upstream);
    // MOD-9 D43: collapse the candidates, then decide them. Only the active ones render, are
    // estimated and meet the cap; every candidate is recorded.
    let placed = parsed.used.contains(&Placeholder::Skills);
    let (skills, skill_choices) = select(BoundSkill::collapse(spec.skills.clone()), placed);
    let candidates = judge_candidates(&spec);
    Ok(ScrubbedInputs { spec, upstream, skills, skill_choices, candidates, literals })
```

- `ScrubbedInputs` `:851-863`: `skills` doc "the **active** candidates: `spec.skills` collapsed and
  selected (MOD-9 D43)"; new field `skill_choices: Vec<SkillChoice>` ("every candidate's choice, in
  collapse order, for `trim_record.skill_choices`").
- `render_sections` (`:593`) and step 3 (`:457-466`) are unchanged: they already take
  `&masked.skills`, which is now the active list, so `render::skills`, the trimmer's
  `skills_tokens` (`trim.rs:438-443`, which reads the rendered `skills` entry) and the cap see only
  active skills. `skills_tokens` needs no edit.
- `trim::record` call `:502-509`: insert `masked.skill_choices.clone(),` after `sections,`.

### 3.2 `prompt/trim.rs`

- `:54-55`: `/// \`trim_record.v\`: version 2 since MOD-9 D42 added \`skill_choices\`; first, so a
  reader can branch (§5.1).` `const RECORD_VERSION: u8 = 2;`
- `:170` doc: "`run_step.trim_record`, version 2 (ANA-5 §5.1 as amended by MOD-9 D42)."
- `TrimRecord` `:181-206`: after `sections` (D55: declaration order, so `--workspace`'s
  `preserve_order` serialisation places it there):

```rust
    /// Every skill candidate and whether it rendered, in collapse order (MOD-9 D42, ANA-22 §6
    /// item 8). Always present; `[]` when the step had no candidate.
    pub skill_choices: Vec<SkillChoice>,
```

- `record` `:1016-1045`: new parameter `skill_choices: Vec<SkillChoice>` after `sections` (seven
  parameters, under clippy's default threshold of 7), assigned in the literal after `sections`.

### 3.3 `fixtures.rs:1631-1665` `IMPL_TRIM_RECORD`

`"v": 2,` and, after `"sections": [ … ],`:

```json
  "skill_choices": [
    { "skill": "00000000-0000-0000-0000-000000005111", "name": "rust-style", "version": 2,
      "level": "project", "activation": "always", "active": true, "reason": "always" }
  ],
```

(`phase_oversize` inherits `phase_implement_attempt2`'s one skill, `prompt/fixtures.rs:221-227`;
`SkillId` serialises transparent, `ids.rs:23`.) Every other number is unchanged —
`estimated_after` stays 35 988, so `backlog__detail_runs` and `replay__runs_step_selected` (`~36k`)
do not move. `step_impl_carries_the_golden_trim_record` (`fixtures.rs:2444-2456`) is the pin.

### 3.4 `tests/prompt_digest.rs:949-1000`

Key set gains `"skill_choices"`; message `"§5.1's twelve keys and MOD-9 D42's skill_choices:
thirteen, no more and no fewer"`; `:995` `assert_eq!(first["v"], serde_json::json!(2), "MOD-9 D42
bumped the record")`. New asserts: `first["skill_choices"]` is an array of one with
`"reason": "always"`. **Do not move** the opaque `{"v": 1}` literals handed to stores
(`pg_criteria.rs:1101`, `:1106`; `conformance.rs:1746`, `:1771`; `mem.rs:5915`, `:5934`;
`model/run.rs:968`): they are not the assembler's record.

### 3.5 `tests/prompt_skills.rs` (new, `#![cfg(feature = "test-support")]`)

`scrubber()` and `ok()` as `prompt_golden.rs:22-30`. Base spec: `fixtures::phase_skills_over_cap()`
with `max_skill_tokens = 20_000` unless stated (`rust-style` v2 pos 0, `command-queue` v1 pos 1).

| Test | Outline |
|---|---|
| `an_off_skill_is_not_rendered_and_is_recorded_off` | `skills[1].activation = Off` → text has `name="rust-style"`, lacks `name="command-queue"`; `trim.skill_choices[1] == {command-queue, Some(1), Project, Off, active: false, reason: Off}`. |
| `a_glob_skill_records_no_path_before_milestone_3` | `skills[1].activation = Glob`, `globs = ["**/*.rs"]` → not rendered, reason `NoPath`. |
| `a_missing_version_records_missing_version` | `skills[1].version = None`, `body = ""` → not rendered; reason `MissingVersion`; `to_value()["skill_choices"][1]["version"]` is `null`. |
| `a_template_without_the_placeholder_records_not_placed_and_renders_nothing` | `body = body.replace("{{skills}}\n", "")` → no `<section name="skills">`, no `skills` row in `trim.sections`, both choices `NotPlaced`/inactive. |
| `an_off_skill_never_trips_the_cap` (F-F) | Measure: a spec with only `skills[0]` and cap `i64::MAX` → `single` = the `skills` row's `tokens_before`. Then the two-skill spec with `max_skill_tokens = single`: assembling refuses `SkillsExceedCap`; with `skills[1].activation = Off` it assembles and the `skills` row's `tokens_before == single`. |
| `choices_follow_the_collapse_order_and_carry_masked_names` | Reverse `skills`, rename `skills[0]` to `"deploy-s3cr3t"`, scrubber `MinimalScrubber::new(["s3cr3t".to_owned()])` → choices ordered `(position, name)` not input order; the choice name contains `[REDACTED]`; `to_value().to_string()` never contains `s3cr3t`. |
| `the_digest_moves_only_when_the_active_set_moves` | `skills[1]` `Off` with globs `[]` vs `Off` with body changed → equal digests (records differ only in nothing rendered); `Off` → `Always` → digest differs. |

### 3.6 Gate

`cargo test -p htui-core --all-features`, then `cargo test -p htui-orch --all-features` and
`env $PGT cargo test -p htui --all-features` (the golden and every `trim_record` reader), clippy for
the three crates.

### 3.7 Commits

(1) red: `prompt_skills.rs`, the moved digest/golden pins, `record`'s new parameter as `todo!()`
upstream; (2) green.

---

## 4. T3: engine (D44; D62, D63, D66, D67)

**Files**: `crates/htui-orch/src/graph.rs`, `fake.rs`, `engine.rs`, `promote.rs`,
`crates/htui/src/run_worker.rs`.

### 4.1 `GraphSource::bound_skills` (`graph.rs:57-107`)

Imports (`:15-19`): add `BoundSkill`. Trait doc: "The eleven orchestration reads …" stays; "this
trait is that source" paragraph gains nothing; the method, after `agent_boxes`:

```rust
    /// One step's skill candidates (MOD-9 D44): the global attachments, `project`'s, and — with
    /// `phase` — that phase's, resolved most-specific-wins by `htui_core::model::skill::resolve`
    /// and ordered `(position, name bytes)`. Inactive winners (`off`, `glob`, a pin with no
    /// version) are **included**: the assembler's `select` decides and records them.
    ///
    /// Inherent on both stores and on `Backend` (`pg/read.rs:1433`, `mem.rs:425`,
    /// `backend.rs:364`) for `prompt_template`'s reason: `skill*` is not mirrored.
    ///
    /// # Errors
    /// The backend's own failures; `Backend` offline refuses with `PROMPT_ON_SERVER_ONLY`.
    async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> Result<Vec<BoundSkill>>;
```

Implementors (each a one-line delegation):
- `graph.rs:781-819` `TestSource`: `self.store.bound_skills(project, phase).await`.
- `fake.rs:843-875` `impl GraphSource for MemStore`: `self.bound_skills(project, phase).await`
  (inherent wins, as its doc says). Doc `:829`, `:842`: "five" → "six".
- `fake.rs:954-986` `FakeGraphSource`: `GraphSource::bound_skills(self.store, project,
  phase).await`.
- `run_worker.rs:2365-2394` `BackendGraphs`: `self.0.bound_skills(project, phase).await`; import
  `BoundSkill`.

### 4.2 `engine.rs`: the shared helper and constant (D62)

Beside `JUDGE_TEMPLATE`/`HANDOFF_TEMPLATE`:

```rust
/// MOD-9 D44: an override graph's phase ids are minted by the clone, which copies no
/// `skill_binding` (`graph.rs:350-355`), so its steps see global and project attachments only.
const OVERRIDE_SKILLS_NOTE: &str =
    "skills: an override graph's phases carry no phase-level attachments until MOD-9 milestone 3";
```

In `impl Engine`, beside `phase_spec`:

```rust
    /// MOD-9 D44: the skill candidates of `phase` in `snapshot`'s graph, for a phase step and for
    /// the judge of that phase alike.
    ///
    /// `SnapshotPhase` carries no `PhaseId` (`model/run.rs:488-527`) and resolution re-densifies
    /// positions (`graph.rs:286-300`), so the live row is found by `(graph id, name)`, which
    /// `UNIQUE (graph_id, name)` makes exact (`0001_init.sql:247`). A phase renamed or deleted
    /// since the snapshot, and every phase of an override graph, get a note rather than a silent
    /// loss of their phase-level attachments (plan R-15).
    async fn phase_skills(
        &self,
        project: ProjectId,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
        notes: &mut Vec<String>,
    ) -> Result<Vec<BoundSkill>, EngineError> {
        let phase_id = self
            .parts
            .store
            .phases(snapshot.graph.id)
            .await?
            .into_iter()
            .find(|row| row.name == phase.name)
            .map(|row| row.id);
        if phase_id.is_none() {
            notes.push(format!(
                "skills: phase `{}` is no longer in graph `{}`; phase-level attachments were not \
                 applied",
                phase.name, snapshot.graph.name
            ));
        }
        if snapshot.graph.is_override {
            notes.push(OVERRIDE_SKILLS_NOTE.to_owned());
        }
        let skills = self.parts.graphs.bound_skills(project, phase_id).await?;
        Ok(skills)
    }
```

### 4.3 `phase_spec` (`engine.rs:4842-4961`)

After `let (caps, _scan, _deadline) = settings::resolve_excerpt_caps(&self.parts.app);` (`:4910`),
before `let role = …`:

```rust
        let skills = self.phase_skills(project.id, snapshot, phase, &mut notes).await?;
```

and `:4937-4939` becomes:

```rust
            // MOD-9 D44: the global, project and phase attachments of this phase, most specific
            // winning (`model::skill::resolve`); the assembler's `select` decides which render and
            // records every candidate in `trim_record.skill_choices`.
            skills,
```

Notes order in a phase record: input notes, the `hops` clamp, the skills notes, then `forwarded`'s.

### 4.4 `judge_prompts` (`engine.rs:4195`) and `run_judge` (`:4032`, `:4053`)

Signature: `async fn judge_prompts(&self, run: &Run, snapshot: &GraphSnapshot, phase:
&SnapshotPhase, attempt: i32, survivors: &[&RunStep])`. Caller `:4053`:
`self.judge_prompts(run, snapshot, phase, attempt, &survivors).await?` (`snapshot` is
`run_judge`'s parameter, `:4035`). Doc gains: "Skills are the judged phase's candidates, resolved
as the phase step resolves them (MOD-9 D44, D48); a pinned judge body without `{{skills}}` records
them `not_placed`." After `let kind = self.item_kind_name(&row).await?;` (`:4301`), before the
closure:

```rust
        let skills = self.phase_skills(project.id, snapshot, phase, &mut notes).await?;
```

and in the closure (`:4322`): `skills: skills.clone(),` with the comment "MOD-9 D44, D48: the judged
phase's candidates, the same list for both orders." The closure still clones `notes`, so both
records carry the skills notes. The `Unavailable` return for a missing template (`:4295-4299`)
stays before the read.

### 4.5 `promote.rs:73-103` (D63)

Doc: "…and `verify_failure`/`previous_diff`/`judge` cleared, and `skills` emptied (MOD-9 D44: a
handoff body cannot place `{{skills}}`, so a candidate would only be scrubbed — failing the handoff
on a skill body it never shows — and recorded `not_placed`). Every other field is the phase's."
Literal: `skills: Vec::new(),` before `..phase`.

### 4.6 Tests

Engine tests use the existing harness (`engine.rs:7580-7610` `started`/`snapshot_of`, `:7811-7823`
`harness_engine!`) and call `phase_spec`/`judge_prompts` directly (private items are reachable from
the in-file `mod tests`), so no gate has to be walked (D67). Common prologue: `let (run, step) =
started(&harness).await;` (FEAT-3 parked at `prd`), `row = harness.orch.run(run)`, `snapshot =
snapshot_of(..)`, the `prd` step row, `item = row.item_id`, `harness_engine!(harness.orch,
engine)`, `implement = snapshot.phases.iter().find(|p| p.name == "implement")`.

| Test (`engine.rs` `mod tests`) | Outline |
|---|---|
| `a_phase_step_renders_its_phase_and_project_skills` | `engine.phase_spec(&row, &snapshot, &prd_step, implement, item, false)` → `spec.skills` = `[tests Project Some(1) 0, rust-style Phase Some(1) 2]` (`pg_criteria.rs:1806`'s phase answer); `assemble(&spec, &MinimalScrubber::new([]))` text contains `<skill name="rust-style" version="1">` and not `version="2"`; `trim.skill_choices` two entries, both active. |
| `a_non_implement_phase_renders_project_skills_only` | Same over `snapshot.phases[0]` (`prd`) → `[tests Project 1, rust-style Project 2]`; no skills note. |
| `a_renamed_phase_falls_back_to_project_skills_with_a_note` | `update_phase(PHASE_HTUI_IMPLEMENT, <its updated_at>, PhasePatch { name: Some("build".into()), ..Default::default() })` after `started`; `phase_spec` for the snapshot's `implement` → `rust-style` at `Project`/`Some(2)`, and `spec.notes` contains ``"skills: phase `implement` is no longer in graph `<snapshot.graph.name>`; phase-level attachments were not applied"``. |
| `an_override_graph_notes_the_clone_gap` | `let mut snapshot = …; snapshot.graph.is_override = true;` (no writer can set it, `graph.rs:357-360`) → `spec.notes` contains `OVERRIDE_SKILLS_NOTE`. |
| `a_judge_renders_the_judged_phases_skills_in_both_orders` | `engine.judge_prompts(&row, &snapshot, implement, 1, &[&prd_step])` (the `prd` step recorded a seq-0 prompt) → `Ok(JudgePrompts { forward, reversed, .. })`; both texts contain the same `<section name="skills">` block with `rust-style` v1; `forward.trim.skill_choices == reversed.trim.skill_choices`, two entries active. |
| `promote.rs`: `the_handoff_carries_no_skills` (F-D) | `handoff_spec(phase_implement_attempt2(), &handoff_template(), &handoff_events(), &[root()], None, "x")` → `spec.skills.is_empty()`; `assemble(..).trim.skill_choices` is `[]`. |
| `promote.rs:402-428` (F-C) | `expected` gains `skills: Vec::new(),` before `..phase.clone()`. |
| `fake.rs:2100-2143` `the_store_answers_the_source_without_recursing` (D66) | Doc "five" → "six"; add `GraphSource::bound_skills(&store, ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))` equals `store.bound_skills(..)` (inherent) and has two entries. |

### 4.7 Build coupling

`run_worker.rs` must implement the new method in the same commit as the trait (T3 owns it). T4
never names `GraphSource`.

### 4.8 Gate and commits

`cargo test -p htui-orch --all-features -- --test-threads=2`; `env $PGT cargo test -p htui
--all-features --lib run_worker -- --test-threads=2`; clippy `-p htui-orch -p htui`. Commits: (1)
trait + four delegations + red tests; (2) engine and promote green.

---

## 5. T4: preview and Prompt sub-tab (D45, D46; D64, D65)

**Files**: `crates/htui/src/preview.rs`, `crates/htui/src/ui/tabs/backlog/detail/prompt.rs`,
`crates/htui/tests/prompt_preview.rs`, `crates/htui/tests/snapshots/prompt_preview__preview_feat_1.snap`,
`…/prompt_preview__preview_ana_2.snap`, `…/backlog__detail_prompt.snap`. `backlog.rs` does **not**
change: FEAT-1 already yields two choices.

### 5.1 `preview.rs`

- `:83-84`:

```rust
/// MOD-9 D45: which phase's attachments the preview shows, and why a glob one never renders here.
const SKILLS_NOTE: &str = "preview: phase-level skills come from the first phase of the item's \
                           graph that uses this template; a glob attachment records no_path \
                           because no root resolves";
```

- New:

```rust
/// MOD-9 D45's second note: no phase of the item's graph uses the chosen template, or the item
/// resolves to no graph, so only global and project attachments apply.
fn no_phase_note(template: &str) -> String {
    format!(
        "preview: no phase of this item's graph uses template `{template}`; global and project \
         skills only"
    )
}
```

- `build` (`:141-264`): after `let pinned = TemplateRef { … };` (`:170-173`) — i.e. after
  `prompt_templates` (F-L):

```rust
    // MOD-9 D45: the phase whose attachments this template's step would get — the first phase, in
    // position order, of the item's graph whose `template_name` is the chosen one.
    let phase = backend
        .resolve_graph(item)
        .await?
        .and_then(|graph| {
            graph
                .phases
                .into_iter()
                .filter(|row| row.phase.template_name == chosen.name)
                .min_by_key(|row| row.phase.position)
        })
        .map(|row| row.phase.id);
```

  After `let mut notes: Vec<String> = STAND_INS…collect();` (`:179`), before `resolve_hops`:

```rust
    if phase.is_none() {
        notes.push(no_phase_note(&chosen.name));
    }
```

  `:227`: `let skills = backend.bound_skills(row.project_id, phase).await?;`. Module doc `:1`
  "eight store reads" → "nine"; `build`'s doc table gains "the item's graph (for the phase id)".
  `STAND_INS` stays eight.

### 5.2 Notes order in a preview record

The eight stand-ins, then the no-phase note (when any), then `resolve_hops`'s clamp note (when
any), then the assembler's.

### 5.3 `tests/prompt_preview.rs`

| Test | Change |
|---|---|
| `the_preview_declares_its_stand_ins` (`:158-192`) | `:181` becomes D45's `SKILLS_NOTE` text verbatim (the "edit in two files" rule of its comment). |
| `the_preview_carries_the_phase_skills_of_the_matching_phase` (new) | `build(&Backend::memory(MemStore::demo()), FEAT-1, Some("implement"), &scope)` → text contains `<skill name="tests" version="1">` then `<skill name="rust-style" version="1">`; `trim.skill_choices` = `[tests Project, rust-style Phase Some(1)]`; notes contain `SKILLS_NOTE` and no `"no phase of this item's graph"` note. |
| `a_template_no_phase_uses_is_noted_and_shows_project_skills` (new) | `Some("research")` on FEAT-1 (the `feature` graph has no `research` phase) → notes contain ``"preview: no phase of this item's graph uses template `research`; global and project skills only"``; choices `[tests Project Some(1), rust-style Project Some(2)]`. |
| `the_preview_refuses_offline_with_one_sentence` (`:325-362`) | Unchanged (F-L). |

### 5.4 `ui/tabs/backlog/detail/prompt.rs` (D46, D65)

Imports: `use htui_core::model::SkillChoice;`.

```rust
/// One rendered row of the pane: its text, and whether it is drawn in `Theme::dim` (MOD-9 D46).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Row {
    text: String,
    dim: bool,
}

impl Row {
    /// A row in the normal style.
    fn plain(text: String) -> Self {
        Self { text, dim: false }
    }

    /// This row as the rows it renders to: one per `\n`-separated piece, a trailing `\r` dropped,
    /// each keeping the style. For every row `render_lines` builds this equals the old
    /// `rows.join("\n").lines()` (review finding M5): only the last row could differ, and it is a
    /// line of `assembled.text.lines()` or an `AssembleError` sentence, neither of which ends in
    /// `\n`.
    fn split(self) -> impl Iterator<Item = Self> {
        let dim = self.dim;
        self.text
            .split('\n')
            .map(|piece| piece.strip_suffix('\r').unwrap_or(piece).to_owned())
            .map(move |text| Self { text, dim })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// One skill choice as the pane lists it: `<name> v<N|?> · <level> · <activation> → <outcome>`,
/// where the outcome is `active` or the reason. CR and LF in a name become spaces, so one choice
/// is one row.
fn choice_line(choice: &SkillChoice) -> String {
    let version = choice
        .version
        .map_or_else(|| "?".to_owned(), |version| version.to_string());
    let outcome = if choice.active {
        "active"
    } else {
        choice.reason.as_str()
    };
    format!(
        "{} v{version} \u{b7} {} \u{b7} {} \u{2192} {outcome}",
        choice.name.replace(['\n', '\r'], " "),
        choice.level.as_str(),
        choice.activation.as_str(),
    )
}
```

- `lines: Vec<String>` (`:51`) → `lines: Vec<Row>`; field doc unchanged in substance.
- `rebuild` (`:94-99`): `self.lines = rows.into_iter().flat_map(Row::split).collect();`.
- `render_lines` (`:104-161`) returns `Vec<Row>`; every existing push wraps with `Row::plain`.
  After `lines.extend(section_lines(..))` and its blank row, before the notes loop:

```rust
    for (index, choice) in record.skill_choices.iter().enumerate() {
        let label = if index == 0 { "skills" } else { "" };
        lines.push(Row {
            text: labelled(label, &choice_line(choice)),
            dim: !choice.active,
        });
    }
    if !record.skill_choices.is_empty() {
        lines.push(Row::default());
    }
```

- `render` (`:318-334`): `.map(|row| if row.dim { Line::styled(row.text.as_str(), ctx.theme.dim)
  } else { Line::raw(row.text.as_str()) })`.
- Tests: `the_window_renders_the_cells_the_scrolled_whole_used_to` unchanged (it builds its own
  `String`s); `one_element_of_lines_is_one_rendered_row` builds `Row::plain` values and
  `flat_map(Row::split)`, expecting the same five texts; new `an_inactive_choice_is_a_dim_row`:
  a `PromptPreview` over `assemble(&fixtures::phase_implement_attempt2(), …)` with one pushed
  choice `{active: false, reason: Off}` → `render_lines` holds a row `labelled("", "… → off")` with
  `dim: true`, and the active choice's row has `dim: false`.

### 5.5 Snapshots that move (F-H), and only these

| Snapshot | What moves | What must not |
|---|---|---|
| `prompt_preview__preview_feat_1` | the `SKILLS_NOTE` row; two new rows `skills    tests v1 · project · always → active` / `rust-style v2 · project · always → active` and a blank row before `notes`; every row below shifts down three | `digest    b9fb821faa4926cf5bfad8cd773e6ba769baccc3a37067c7f48354045f9cb915`, `tokens 1067 → 1067`, the section table |
| `prompt_preview__preview_ana_2` | the same block and note, plus a ninth note (``no phase of this item's graph uses template `prd` …``) | `digest d68f3503…`, `tokens 354 → 354` |
| `backlog__detail_prompt` | as `preview_feat_1`, clipped at 43 columns; three notes scroll off the 30-row frame | the digest prefix `b9fb821faa4926cf5bfad8cd773e6ba76` |

`templates__edit_help` moved in T1. No other snapshot in `crates/*/tests/snapshots/` contains a
prompt pane or a trim record (grep `digest`, `estimated_after`).

### 5.6 Gate

`env $PGT RUST_BACKTRACE=0 cargo test -p htui --all-features -- --test-threads=2`;
`cargo insta review` of the three; clippy `-p htui`.

### 5.7 Commits

(1) red: the two new preview tests, the updated stand-in string, the pane's unit test; (2) green
with the three snapshots, each named in the message.

---

## 6. Cross-task contracts

| Producer | Contract | Consumer |
|---|---|---|
| T1 | `BoundSkill { version: Option<i32>, level, activation, globs }`, `SkillChoice`, `ChoiceReason`, `SkillLevel`, `Activation`, `model::skill::{resolve, select}`; `JUDGE` places `{{skills}}`; `allowed_in(Judge)` has `Skills` | T2, T3, T4 |
| T1 | `MemStore::bound_skills` / `PgStore::bound_skills` / `Backend::bound_skills` return **candidates**, inactive winners included | T3 (`GraphSource`), T4 (preview) |
| T2 | `TrimRecord.skill_choices`, `v: 2`; `assemble` renders only `select`'s active list | T3 tests, T4 pane |
| T3 | `GraphSource::bound_skills` | `run_worker.rs` (same task) |

T3 ∩ T4 = ∅ (checked: T3 is `htui-orch/src/*` + `run_worker.rs`; T4 is `preview.rs`,
`detail/prompt.rs`, `htui/tests/prompt_preview.rs` and three snapshots).

---

## 7. Count pins

| Pin | Where | Before → after |
|---|---|---|
| applied migrations | `migrations.rs:79-86` | `1..=6` → `1..=7` |
| `Pending` | `migrations.rs:831-835` | 6 → **7** |
| commented columns | `migrations.rs:424`, `:454` | 29 → **34** |
| `TABLES` | `migrations.rs:106-110` | 39 (unchanged) |
| `.sqlx` files | `ls crates/htui-store/.sqlx \| wc -l` | 264 → **263** |
| store `CASES` | `mem_store.rs:37`, `pg_conformance.rs:19` | 71 (unchanged) |
| `READ_CASES` | `mem_store.rs:51` | 14 (unchanged) |
| orch `CASES` | `htui-orch/tests/fake_conformance.rs:16` | 70 (unchanged) |
| `StoreRequest` / `StoreReply` | `store_worker.rs` enums (no test pins them, milestone 1 F-Q) | 66 / 37 (unchanged) |
| `MIRRORED_TABLES` | `htui-store/tests/cache.rs:1392-1393` | 21 (unchanged) |
| trim record keys, `v` | `prompt_digest.rs:976-995` | 12 → **13**, 1 → **2** |
| judge closed set | `template.rs:476-480` | 4 → **5** tokens |
| `STAND_INS` | `preview.rs:61` | 8 (unchanged) |
| `DeleteReach.skill_bindings` | `conformance.rs:2421` | 3 (unchanged) |

---

## 8. Merge order and the workspace gate

T1 merged, then T2, then T3 and T4 from the same base; merge T3 then T4, re-running the touched
crates' gates after each. Then:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
env $PGT RUST_BACKTRACE=0 cargo test --workspace --all-features --no-fail-fast -- --test-threads=2
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_mod9m2 cargo sqlx prepare --check)
git diff --exit-code Cargo.lock
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

---

## 9. Risks (continuing from the plan's R-20)

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-21 | The default judge carries each skill twice — inside `judge_task` (the candidate's replayed prompt) and as the protected `skills` section (F-G) — so a judge's protected set grows by the phase's skill tokens and `BudgetTooSmall` fires for a judge before it would for its candidates. | Certain once skills are bound | Low (demo: ≈90 tokens) to Medium (large skill sets) | Maintainer decision on F-G. The cap bounds the protected copy (`max_skill_tokens`, default 20 000). |
| R-22 | A run resolved before `0007` pins judge v1 (`graph.rs:580`, `SnapshotJudge.template`), so its judges record every skill `not_placed`. | Certain for in-flight runs | Low | Honest by design (D40's `placed`); the record says why. |
| R-23 | The demo loader writes none of `0007`'s four columns, so a future fixture row with `Off`/`glob`/`source` would load differently into Postgres than into `MemStore`. | Low | Medium (silent store disagreement in tests) | D59's `demo_skill_rows_use_the_column_defaults` fails first. |
| R-24 | A `.sqlx` regenerated against a database that was not migrated from zero keeps a stale `project_id` nullability. | Low | Low | D61's explicit `?`, and §2.11's drop-and-recreate before every `prepare`. |

---

## 10. Where this blueprint departs from the plan

F-A (`pg/demo.rs` joins T1), F-B (`JUDGE_SEED_V1` is `pub`, **amends D49**), F-C/F-D (handoff test
placement and pin), F-E (T1 moves `templates__edit_help` and `the_three_role_sets_are_closed`),
F-F (the cap test's arithmetic), F-H (which snapshots move and why), F-K (`template.rs` leaves T2).
F-G is a question for the maintainer, not a change.

---

## 11. Decisions (D50 onward)

Milestone 3's plan continues at **D70**, **R-25**.

| # | Decision |
|---|---|
| D50 | `SkillLevel { Global, Project, Phase }` and `ChoiceReason` are plain enums with `as_str` and `#[serde(rename_all = "snake_case")]`; `SkillLevel` derives `PartialOrd, Ord` by declaration order, so `Phase` is the max. Only `Activation`, a column, uses `str_enum!`. |
| D51 | `resolve`, `select`, `SkillChoice`, `ChoiceReason` live in `model/skill.rs` beside `collapse` (pure, no `prompt` dependency). `model` re-exports the types; the two functions stay path-qualified. The plan's `activation_round_trips_its_db_text` is `model/mod.rs`'s `activation_matches_check_list`, beside `check_enum`. |
| D52 | One-list `collapse`: stable sort by level descending, keep the first per `skill_id`, sort `(position, name bytes)`. `resolve` maps every row (each with its own pin's version) and then collapses, so the winner is chosen before its version is examined (OQ-9). Equal-level ties: first in input order. |
| D53 | `render::skills` skips a `version: None` candidate and answers `None` when nothing remains; `select` never passes one. |
| D54 | `placed = parsed.used.contains(&Placeholder::Skills)`; no new `ParsedTemplate` accessor (F-K). |
| D55 | `TrimRecord.skill_choices` is declared after `sections` and always serialised; `SkillChoice`'s keys are `skill, name, version, level, activation, active, reason`, and `version: None` is `null`. |
| D56 | The new `judge` body is today's with `{{skills}}\n` inserted after `{{task}}\n` (§2.5). With no active skill it renders byte-identical to today (§0a); `prompt_golden__prompt_judge_three_candidates.snap` is the pin. |
| D57 | **(amends D49)** `pub const JUDGE_SEED_V1` in `htui_core::prompt::defaults`, not re-exported from `prompt` (F-B). |
| D58 | Both D49 tests are in `migrations.rs`: the literal check reads the file with `include_str!` and needs no server; the upgrade test stages `run_to(6)` → plant → `run_to(7)` over four projects (§2.1's table). `$old$`/`$new$` each occur exactly twice in `0007`. |
| D59 | `pg/demo.rs` joins T1 for the `Option` bind only; its query text (and `.sqlx` file) is unchanged, and the demo literals equal the column defaults, pinned by `demo_skill_rows_use_the_column_defaults` (F-A, R-23). |
| D60 | `MemStore::bound_skills` drops a binding whose `skill` row is missing (`filter_map`), as `State::bind` did; `State::bind` is deleted (F-N). |
| D61 | Postgres reads spell `b.project_id AS "project_id?: ProjectId"`; both statements share one `WHERE`, with `$2 = phase.map(PhaseId::as_uuid)`. |
| D62 | One engine helper, `phase_skills(project, snapshot, phase, &mut notes)`, serves `phase_spec` and `judge_prompts`; the two notes are the exact strings of §4.2, pushed after the `hops` note and before `forwarded`'s. |
| D63 | `handoff_spec` empties `skills`; `the_handoff_spec_keeps_the_phase_s_item_and_budget` expects it; `the_handoff_carries_no_skills` lives in `promote.rs` (F-C, F-D). |
| D64 | The preview resolves the phase right after the template reads (F-L), picks `min_by_key(position)` among phases whose `template_name` is the chosen name, and pushes the no-phase note right after the stand-ins. |
| D65 | The Prompt sub-tab stores `Row { text, dim }`; the choices block sits between the section table and the notes, one row per choice, `skills` label on the first, a blank row after; CR/LF in a name become spaces (F-M). |
| D66 | `fake.rs`'s delegation test is extended with the sixth read rather than a second test, so its "calls all N through the trait" doc stays true. |
| D67 | T3's engine tests call `phase_spec` / `judge_prompts` directly under `harness_engine!`; the override case mutates the decoded snapshot's `is_override`, which no writer can set. |
| D68 | T1 owns the judge-contract fallout: `template.rs:476-480`, `crates/htui/tests/templates.rs:160` (`"{{skills}}"` joins the judge token list), and `templates__edit_help.snap`, with `cargo test -p htui --all-features --test templates` in T1's gate (F-E). |
| D69 | `closed_rows_backfill_to_done` stages `run_to(6)` instead of `run` (F-P). |
