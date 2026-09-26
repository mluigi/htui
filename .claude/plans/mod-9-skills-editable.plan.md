# Plan: MOD-9 milestone 3 — skills are editable and attachable

**Status: CONFIRMED by the maintainer 2026-09-26, OQ-16..OQ-20 at their defaults.** Two questions were answered before drafting
(2026-09-26): milestone 3 is **split** (OQ-14) and the glob matcher is the **`globset` crate**
(OQ-15).

**Source**: `.claude/prds/mod-9-skill-library-templates.prd.md`, milestone 3 (Delivery Milestones
table, row 3): "Skill writers, `Skills` view with library, editor, diff, bindings matrix by project
and phase, token estimate — in the shape ANA-22 decides; plus the language map, glob matcher and
clone gap." PRD scope bullet "Skill writers, Skills view, bindings (milestone 3, after ANA-22)";
`docs/ANA-22.md` §6 items 1-7 and 12 and §8's milestone 3 paragraph; the milestone 2 plan's carry
list (`.claude/plans/mod-9-skills-reach-the-run.plan.md` D47; review findings 2 and 5 in `HANDOFF.md`
MOD-9).

**Requirements**: `R-SKL-1` (versioned skills), `R-SKL-2` as amended by D37 (global, project and
phase attachments with activation), `R-SKL-3` (bindings editable), `R-TUI-7` (the Skills tab),
`R-NF-3` (every read and write on the store worker).

**Complexity**: Large. Five `WriteStore` writers and three readers across five implementors, a pure
glob and language module in `htui-core` with one new dependency (`globset`), five new
`StoreRequest`s, and the Skills view with its attachments pane. **No migration**: `0007` already
holds every column this milestone writes.

**Routing**: continues `/handoff-run MOD-9` (PRD path, ultracode recommended for implement and
accepted with the item). Selected with MOD-7 milestone 3 running on another box; see "Concurrent
work" below. The review gate is `rust-reviewer` (`.claude/workflow-config.json`); there is no
`.claude/agents/` in this repo, so it runs as a general-purpose agent under that brief, as in
milestones 1 and 2.

**Numbering**: continues from milestone 2's blueprint §11. Decisions **D70…**, risks **R-25…**, open
questions **OQ-14…**. Tasks restart at **T1**.

**Tree**: facts read with `grep`/`sed` at `98e6d2f` (main, after PR #8). `graphify-out/` does not
exist and Gortex is not reachable in this session.

---

## Open questions for the maintainer (read these first)

- [x] **OQ-14 — Split milestone 3. Answered 2026-09-26: split.** Glob *firing* needs a per-step file
      set that does not exist: no step gets a real root (`no_excerpts`, `engine.rs:5691-5704`,
      used at `:4338` and `:4967`; the preview's `empty_excerpts`, `preview.rs:301-315`), the excerpt
      walk runs nowhere in production, a fan-out group's prompt is assembled before any candidate
      has a tree (`drive_group`, `engine.rs:3381-3383`), and no attempt records its changed paths
      as a list (`Isolator::diff` returns `--stat` text only, `isolate/git.rs:79`, `:815-845`). That
      work lives in `walk_live_step`, `drive_group`, `phase_spec`, `judge_prompts`, `forwarded`,
      `EngineParts` and the isolator, next to MOD-7 milestone 3. **This milestone** keeps the
      writers, the Skills view and attachments pane, the language map, the glob matcher (used to
      validate and canonicalise at save, and tested against its table), and the clone gap. **A new
      PRD milestone row 5, "Glob attachments fire"**, takes the F2 file set, changed paths, the
      fan-out timing, `ChoiceReason::{Matched, NoMatch}` and the preview's glob story (D86). A
      `glob` winner keeps recording `no_path` until then.
- [x] **OQ-15 — Glob matcher. Answered 2026-09-26: `globset`.** Added to `htui-core` (D72).
- [ ] **OQ-16 — Writer set.** ANA-22 §8 and the model doc name `upsert_skill`, `add_skill_version`,
      `set_skill_binding`. **Default (D75):** four writers — `create_skill` (the row **and** version
      1 in one transaction, so no skill ever exists without a body), `update_skill` (name and
      description, compare-and-set on `updated_at`), `add_skill_version` (compare-and-set on the
      head version) and `set_skill_binding` (attach, change or detach by natural key,
      compare-and-set on `updated_at`). **Alternative:** one `upsert_skill` that creates a
      body-less row, then `add_skill_version`; that leaves a window where a skill has no version.
- [ ] **OQ-17 — Renaming a skill.** The name is rendered (`<skill name="…">`, `render.rs:444-447`)
      and recorded (`skill_choices[].name`). **Default (D76):** allowed, through `update_skill`,
      validated as at create; the digest moves for the next step that renders it, which is the
      point. **Alternative:** the name is fixed at create.
- [ ] **OQ-18 — A repo qualifier naming no repo of the project.** **Default (D79):** the writer
      refuses it (`Constraint`), so a stored qualified glob always named a real repo when saved; a
      later repo rename leaves it stale, as for `touched_paths` (ANA-22 §9's row), and the
      attachments pane marks it. **Alternative:** accept and only warn.
- [ ] **OQ-19 — Language map overlay.** ANA-22 §6 item 5 calls the map "data, not code, like
      MOD-7's probe spec", which has an `app_setting` overlay. **Default (D73):** a seed JSON file in
      `htui-core`, no overlay in this milestone; an unknown language is refused at save and typed
      globs always work. **Alternative:** add an `app_setting.skill_languages` overlay now (a
      reader, a writer and a Settings surface — MOD-51's shape for the probe spec).
- [ ] **OQ-20 — A blank skill body.** **Default (D77):** refused at create and at append ("a skill
      needs text"), because it renders an empty `<skill>` block that costs tokens and says nothing.
      **Alternative:** allow it.

---

## Summary

**Model (T1, pure).** `htui_core::model::skill` gains name validation (the Agent Skills rule:
`[a-z0-9-]`, 1-64, no leading, trailing or doubled hyphen), and two new pure modules: `skill_glob`
(parse `<repo>:<glob>` or a bare glob, compile with `globset`, canonicalise a list) and
`skill_language` (a seed map from language to globs, `expand`). The attachment writer and, at
milestone 5, `select` both call them.

**Writers and readers (T2).** `WriteStore` gains `skills`, `skill_versions`, `skill_bindings`,
`create_skill`, `update_skill`, `add_skill_version`, `set_skill_binding`, on `MemStore`, `PgStore`,
`Writer` and both spy stores, with conformance cases for every writer. The attachment writer expands
languages and stores the effective globs.

**Clone gap (T3).** `NewStepGraph` gains `is_override`, which both `create_step_graph`s write.
`override_graph` sets it and copies every phase attachment of the source graph onto the cloned phase
ids through `set_skill_binding`. The engine's clone-gap note and its test go.

**Store worker (T4).** `crate::skills` serves `Skills(Scope)` (one snapshot: library, versions,
global and project attachments, each project's graphs, phases and repo names) and the four writes,
mirroring `crate::templates`.

**Skills view (T5).** The stub becomes a library list with body, version stepping, diff (to any
version), `TextArea` and `$EDITOR` editing, a token estimate, create and rename; and an attachments
pane per skill: a global row, then each scoped project and its phases, with a form for activation,
pin, position, languages and globs, the expanded globs shown before save, and a repo picker for
qualified globs.

## Design decisions (settled here, not in code review)

| # | Decision | Why / evidence |
|---|---|---|
| D70 | **Split (OQ-14).** Row 3 of the PRD's milestone table is reworded to what this plan ships, and a **row 5, "Glob attachments fire"**, is added (D86). ANA-22 §10 gets a 2026-09-26 amendment line at close-out; the verdict is unchanged. | Maintainer, 2026-09-26. Row 5 rather than "3b" so milestone 4's number stays put. |
| D71 | **Skill names** (`htui_core::model::skill::validate_name`): 1-64 bytes of `[a-z0-9-]`, no leading, trailing or doubled hyphen (ANA-22 §6 item 12, the Agent Skills spec). Checked by the writers, not by a constraint, so the demo rows and any hand-written row still load. `invalid_skill_name(name)` in `store/traits.rs` beside `invalid_template_name` (`:1442`) phrases the refusal. | The demo names `rust-style` and `tests` pass (`fixtures.rs:515-538`). Import (milestone 4) uses the same check. |
| D72 | **`globset` (OQ-15)** in `[workspace.dependencies]` and `htui-core`'s `[dependencies]` with `default-features = false` (drops `log`). Its own dependencies `aho-corasick`, `bstr`, `regex-automata`, `regex-syntax` are already in `Cargo.lock` (`:228`, `:672`, `:5183`, `:5200`), so the lock gains one package. MSRV 1.88 ≤ the pinned 1.98 (`Cargo.toml:8`). Globs compile with `GlobBuilder::literal_separator(true)` (`*` and `?` never cross `/`), `backslash_escape(true)`, `empty_alternates(false)`; `**` as a whole segment crosses directories. Paths are repo-relative with `/`. | ANA-22 §6 item 7's syntax is exactly `globset`'s (`*`, `**`, `?`, `{a,b}`, `[...]`). `htui-core` keeps its no-`std::fs` rule: `globset` is pure. |
| D73 | **Language map (OQ-19).** `crates/htui-core/src/model/skill_languages.json`, `include_str!`-ed and parsed once (`LazyLock`), `{ "rust": ["**/*.rs", "**/Cargo.toml"], … }` for at least ANA-22 §9's fourteen: `rust`, `c`, `cpp`, `python`, `typescript`, `javascript`, `go`, `java`, `csharp`, `shell`, `sql`, `markdown`, `toml`, `yaml`. `skill_language::expand(&[String]) -> Result<Vec<String>, UnknownLanguage>`; names are matched after trim and ASCII lowercase. A unit test compiles every glob in the file. No `app_setting` overlay. | Data, not code (ANA-22 §6 item 5). The map moving never changes a saved attachment: the writer stores the expanded globs. |
| D74 | **`skill_glob`.** `SkillGlob { repo: Option<String>, glob: String }` parsed from text: a `:` before the first `/` and any glob metacharacter splits off a non-empty repo name, as in `touched_paths` (`excerpt.rs:68-88`); a bare glob matches in any repo of the step's scope (ANA-22 §6 item 6, **not** `touched_paths`' "primary repo"). `canonical_globs(typed, languages) -> Result<Vec<String>, GlobError>`: trim, drop empties, parse and compile each, then typed order followed by expanded language globs, deduplicated keeping the first. `GlobError` names the glob and `globset`'s message. `SkillGlobs::compile(&[String]) -> Result<Self>` and `fn first_match<'a>(&self, repo: &str, paths: impl IntoIterator<Item = &'a str>) -> Option<&'a str>` exist now, tested against a table, and are what row 5's `select` calls. | One place turns typed text into stored globs, for the writer and the form's "effective:" preview alike. |
| D75 | **Writers (OQ-16)** on `WriteStore` (`store/traits.rs`), with readers beside them as MOD-15 did so conformance can read back (`traits.rs:472-473`; the not-mirrored rule of `:25-27` is about the *cache*, which these do not touch): `skills() -> Vec<Skill>` by name bytes; `skill_versions(SkillId) -> Vec<SkillVersion>` by version; `skill_bindings(Option<ProjectId>) -> Vec<SkillBinding>` (`None`: the global rows; `Some(p)`: `p`'s project and phase rows); `create_skill(NewSkill) -> (Skill, SkillVersion)`; `update_skill(SkillId, expected: DateTime<Utc>, SkillPatch) -> CasOutcome<Skill>`; `add_skill_version(SkillId, expected: i32, NewSkillVersion) -> CasOutcome<SkillVersion>` (token = head version; `Stale` carries the head); `set_skill_binding(SkillBindingKey, expected: Option<DateTime<Utc>>, BindingChange) -> CasOutcome<Option<SkillBinding>>` (`None` = "I expect no row"; `Stale` carries the row as it is, or `None`). `BindingChange::{Attach(Attachment), Detach}`; `Attachment { pinned_version, position, activation, globs (typed), languages }`. `CasOutcome` is the existing type (`traits.rs:1676-1681`). | Precedence as `append_prompt_template` (`traits.rs:715-723`): the token first (a spent token answers `Stale` even for bad input), then `NotFound` for the skill, project or phase, then `Constraint`. |
| D76 | **`update_skill` (OQ-17)** changes `name` and/or `description`; the name passes D71 and a taken name is `Constraint(already_exists("skill", name))`. `skill.updated_at` is the token (the `set_updated_at` trigger covers `skill`, `0001_init.sql:575-580`). | A rename is a deliberate prompt change. |
| D77 | **`create_skill` and `add_skill_version` (OQ-20).** Refuse a blank (whitespace-only) body. `source` is `{}` from the Skills view (milestone 4 fills it). `created_by` must be a user (`references_no_row`). `add_skill_version` writes `max + 1` with `INSERT … SELECT … WHERE (SELECT max(version) …) = $expected`, the `append_prompt_template` shape (`pg/write.rs:2419-2479`). A version equal to the head's body is still appended (the view prevents it; the store does not second-guess). | Append-only (`skill_version` has no trigger, `0001_init.sql:570`). |
| D78 | **`set_skill_binding` key and checks.** `SkillBindingKey { skill, project: Option<ProjectId>, phase: Option<PhaseId> }`. Refusals, in order after the token: skill, project, phase exist (`NotFound`); a phase key needs a project and the phase's graph belongs to it (`Constraint`, mirroring `graph_not_in_project`, `traits.rs:1419`; no DB constraint checks this, and both reach counts filter by `project_id`); `pinned_version` names an existing version; `position >= 0`; languages known (D73); every glob parses and compiles (D74); a global row carries no qualified glob; a qualifier names a repo of the project (D79); `Glob` needs non-empty effective globs (the DB's `skill_binding_glob_needs_globs` backs it). Stored `globs` = `canonical_globs(typed, languages)`; stored `languages` = trimmed, lowercased, deduplicated, as typed otherwise. `Detach` deletes the row. On Postgres: attach with `expected: None` is `INSERT … ON CONFLICT DO NOTHING` (0 rows → re-read → `Stale`); change is `UPDATE … WHERE id = … AND updated_at = $expected`; detach is `DELETE … WHERE … AND updated_at = $expected`. | ANA-22 §6 items 2-6. Review finding 5 of milestone 2 holds: versions are never deleted and a pin is checked against existing versions at write, so a reader that reads bindings before versions never sees a pin to a version that did not exist when the pin was written. |
| D79 | **Qualified globs (OQ-18)** on a project or phase row must name a repo of `project` (`WriteStore::repos`, `traits.rs:597`); otherwise `Constraint("glob `<g>` names repo `<r>`, which project `<slug>` does not have")`. | Save-time truth; renames are the pane's warning. |
| D80 | **Clone gap.** `NewStepGraph` (`model/kind.rs:152-161`) gains `is_override: bool`; its nine constructors set `false` except `override_graph`; `MemStore::create_step_graph` (`mem.rs:2492-2525`) stores it and `PgStore`'s `INSERT INTO step_graph` (`pg/write.rs:2180-2203`) writes the column (one `.sqlx` file replaced). `override_graph` (`graph.rs:386-441`) keeps an old→new `PhaseId` map while cloning, then for each phase-level row of `skill_bindings(Some(project))` whose phase is a source phase, calls `set_skill_binding(key with the new phase, None, Attach(copy))`. The copy passes the stored `globs` as typed with the stored `languages`; D74's union is idempotent on them. `OVERRIDE_SKILLS_NOTE` and its push (`engine.rs:85-88`, `:5017-5019`) and the test `an_override_graph_notes_the_clone_gap` (`:12059-12075`) are deleted; `override_clone_leaves_bindings_alone` (`graph.rs:1442`) becomes `override_clone_carries_phase_attachments_and_marks_itself`. | ANA-22 §2 "found in passing", milestone 2 review finding 2. `override_graph` has no production caller yet (`graph.rs:1460` test, `lib.rs:56` re-export), so this is correctness ahead of use. |
| D81 | **Store worker.** New module `crates/htui/src/skills.rs` (mirrors `crate::templates`, `templates.rs:115-180`): `StoreRequest::{Skills(Scope), CreateSkill{scope, name, description, body}, EditSkill{scope, skill, expected, patch}, SaveSkillVersion{scope, skill, expected, body}, SetSkillBinding{scope, key, expected, change}}`; replies `StoreReply::{Skills(Box<SkillsSnapshot>), SkillsStale{snapshot, what: StaleWhat}}` and the existing `Failed`. `SkillsSnapshot { skills: Vec<SkillEntry { skill, versions }>, global: Vec<SkillBinding>, projects: Vec<ProjectSkills { project, bindings, graphs: Vec<(StepGraph, Vec<StepGraphPhase>)> (override graphs excluded), repos: Vec<String> }> }` over the scope's projects. `REQUEST_NAMES`, `READ_NAME`, routing arms and the name test as in `templates.rs:121`, `:425`, `store_worker.rs:1110-1112`, `:2818`. `NotFound` of the skill maps to `SkillsStale` (as `box_settings.rs:82-121` does for a box). | One snapshot per reply keeps the view's staleness gate simple (`state.rs:296-321`). The library is small; N+1 version reads are acceptable and noted (R-27). |
| D82 | **Skills view, library.** `ui/tabs/skills/library.rs`, `LibraryView`, modelled on `TemplatesView` (`templates.rs:108-1103`): list left (`LIST_WIDTH`), body or diff right. Keys: `j`/`k`, `,`/`.` version, `b` diff base, `d` body/diff, `e` edit in `TextArea`, `E` `$EDITOR`, `n` new skill (name, then description, then the editor for v1), `i` rename/description form, `a` attachments pane, `r` reload. Save is `ctrl-s`; a stale save keeps the draft and moves the token (`TemplatesStale`'s rule, `templates.rs:342`). The header shows `v<N> · ~<T> tokens`, where `T = TokenEstimator::DEFAULT.estimate(render::skills(&[as_bound]).content)` (`estimate.rs:59`, `render.rs:436`), the bytes the assembler would count; the same estimate is repeated in the save notice. The diff reuses `ui::diff::unified`/`lines` (`ui/diff.rs:20-55`). | The PRD's "library, editor, diff, token estimate" (row 3), with milestone 1's proven editor. |
| D83 | **Skills view, attachments pane.** `ui/tabs/skills/attach.rs`, opened with `a` on a skill. Rows: `global`, then per scoped project (by position) the project row and, under it, `graph › phase` rows of its non-override graphs in graph name then phase position order (`settings/prompt.rs:93-113` and `settings/kinds.rs:101-134` are the row-enum precedents). Each row shows `—` or `<activation> · <v N | latest> · pos <p>[ · <n> globs]`, and a `*` on the row that wins for that project's phases (computed with `model::skill::resolve` over the snapshot, so the pane and the run agree). `Enter` opens the form; `x` detaches after `y`. The form (the `settings/kinds.rs:181-206` `Field` shape): activation (`space` cycles), pin (`latest` or a number), position, languages (comma list), globs (comma list); a read-only `effective:` line under it recomputes `canonical_globs` on every keystroke and shows its error inline; `ctrl-r` on a project or phase row opens a picker of the project's repo names and inserts `<repo>:` at the globs cursor; `ctrl-s` saves. A qualified glob naming no current repo is marked `?` on its row (D79's rename case). A glob attachment's row notes `fires from milestone 5` (the `no_path` truth, OQ-14). | ANA-22 §8's "attachments matrix with a global row above the projects and phases, the expanded globs shown before save, and a repo picker". One skill at a time: a skills × scopes grid does not fit 100 columns. |
| D84 | **Tab shell.** `SkillsTab` (`ui/tabs/skills/mod.rs:30-133`) holds `library: LibraryView` and routes keys to it on the Skills view; `wants_requests` returns `Templates(scope)` and `Skills(scope)`; `captures_input` covers both views; `SKILLS_LATER` goes (no test pins it). The strip text stays. | `Tab::wants_requests` already returns a `Vec`. |
| D85 | **Pins that move.** `htui-core` `CASES` 74 → **80** (six new cases, T2) in `store/conformance.rs:43-118`, `tests/mem_store.rs:35-49` and `htui-store/tests/pg_conformance.rs:19`; `READ_CASES` stays 14. `StoreRequest` 68 → 73 and `StoreReply` 39 → 41 (unpinned). `.sqlx`: the new statements, and one replaced (`create_step_graph`); the exact count is whatever `cargo sqlx prepare` leaves, recorded in the close-out. No migration: the applied list, `TABLES` (39) and commented columns (34) do not move. | Numbers from the survey at `98e6d2f`. |
| D86 | **PRD row 5, "Glob attachments fire"** (pending): the F2 file set (the excerpt walk's listing under the step's roots, narrowed to `touched_paths` prefixes, plus the previous attempt's changed paths from a name-only diff), roots for fan-out groups, `select` taking the file set with `ChoiceReason::{Matched, NoMatch}` and the matched path in the record, the preview's roots, and the pane's "repos with no path row on this box" warning. Better after MOD-7 milestone 4 writes `repo_box_path`. | OQ-14. |

## Concurrent work (MOD-7 milestone 3 on another box)

MOD-7 milestone 3 changes admission in `htui-orch` (`engine.rs` `enqueue`..`note_substitution`,
`:596-2953`; `select.rs`), and likely `htui-store` `pg/read.rs`, `backend.rs` and
`htui-core/src/store/mem.rs`. This plan touches `engine.rs` only at D80's three deletions plus one
constructor line (`:6021`, a test fixture), never `select.rs` or `backend.rs`, and adds new
functions (not edits) to `traits.rs`, `mem.rs`, `pg/write.rs`, `pg/read.rs` and `writer.rs`. Expect
textual merge conflicts at shared insertion points (trait method lists, `impl` blocks), not logical
ones. The branch merges `main` before T2 and again before review.

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| Head-version compare-and-set append | `append_prompt_template` | `traits.rs:724`; `mem.rs:2696-2749`; `pg/write.rs:2419-2479` |
| `updated_at` compare-and-set | MOD-15 hierarchy writers | `traits.rs:467-491` |
| Zero rows → `Stale` or `NotFound` | `cas_miss` | `pg/write.rs:78-89` |
| Delegation in `Writer` and the spies | `edit_box` | `writer.rs:444-453`; `htui-agent/src/conformance.rs:759-766`; `htui-agent/tests/recorder.rs:443-450` |
| Store worker module | `crate::templates` | `crates/htui/src/templates.rs:115-180`; `store_worker.rs:529-544`, `:1110-1112` |
| Versioned body editor, diff, `$EDITOR` | `TemplatesView` | `ui/tabs/skills/templates.rs` |
| Multi-field form | kinds editor | `ui/tabs/settings/kinds.rs:181-206`, `:751-812` |
| Global row above projects | prompt settings | `ui/tabs/settings/prompt.rs:93-113` |
| Two-session PG race | `prompt_template_cas.rs` | `crates/htui-store/tests/prompt_template_cas.rs` |

## Tasks

**T1 → T2 → (T3 ∥ T4) → T5**, with T5 ∥ T3 allowed. Parallel tasks run in their own worktrees
against one shared `CARGO_TARGET_DIR`, `--test-threads=2` at most.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T1 | `Cargo.toml` (workspace dep), `Cargo.lock`, `crates/htui-core/Cargo.toml`, `crates/htui-core/src/model/skill.rs` (name validation, doc), `crates/htui-core/src/model/skill_glob.rs` (new), `crates/htui-core/src/model/skill_language.rs` (new), `crates/htui-core/src/model/skill_languages.json` (new), `crates/htui-core/src/model/mod.rs` | first |
| T2 | `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/pg/rows.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/.sqlx/` (new files), `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/skill_writers.rs` (new), `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | after T1 |
| T3 | `crates/htui-core/src/model/kind.rs`, `crates/htui-core/src/store/mem.rs` (`create_step_graph` only), `crates/htui-core/src/store/conformance.rs` (constructor lines only), `crates/htui-store/src/pg/write.rs` (`create_step_graph` only), `crates/htui-store/.sqlx/` (one replaced), `crates/htui-orch/src/graph.rs`, `crates/htui-orch/src/engine.rs` (D80's deletions, `:6021`), `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/gix_isolator.rs`, `crates/htui-orch/tests/fixtures.rs`, `crates/htui/src/catalogue.rs` | ∥ T4, T5; after T2 |
| T4 | `crates/htui/src/skills.rs` (new), `crates/htui/src/store_worker.rs`, `crates/htui/src/lib.rs` | ∥ T3; after T2 |
| T5 | `crates/htui/src/ui/tabs/skills/mod.rs`, `crates/htui/src/ui/tabs/skills/library.rs` (new), `crates/htui/src/ui/tabs/skills/attach.rs` (new), `crates/htui/tests/skills.rs` (new), `crates/htui/tests/skills_pg.rs` (new), `crates/htui/tests/snapshots/skills__*.snap` (new) | ∥ T3; after T4 |

**Intersections, checked.** T1 ∩ T2 = ∅ but T2 calls T1's modules, so serial. T2 ∩ T3 = {`mem.rs`,
`store/conformance.rs`, `pg/write.rs`, `.sqlx/`}, so T3 runs after T2. T3 ∩ T4 = ∅. T3 ∩ T5 = ∅.
T4 ∩ T5 = ∅, but T5 consumes T4's types, so serial. **Build coupling:** T3 changes a struct every
crate constructs; it owns all nine constructor files.

Every implementer prompt carries: the PRD gate decisions, D70–D86 and the OQ answers win over prose;
read the tree with grep and read (no `graphify-out/`, no Gortex); no test is skipped or loosened; a
moved pin names its reason; every new `pub` item has a doc comment and `Debug`; commit red tests
first, then green; gate `. /root/ort/env.sh; USERNAME=htui-ci RUST_BACKTRACE=0
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p <crate>
--all-features -- --test-threads=2`.

### Task 1: names, globs, languages (D71-D74)
- **Tests first** (unit, in the new modules):
  - `validate_name`: accepts `rust-style`, `a`, 64 × `a`; refuses empty, 65 bytes, `Rust`, `-a`,
    `a-`, `a--b`, `a_b`, `a b`.
  - `skill_glob`: the D74 table — `*.rs` vs `src/a.rs` (no), `**/*.rs` vs `a.rs` and `src/a/b.rs`
    (yes), `src/*.{rs,toml}`, `[ab].md`, `?.md`, `htui:**/*.rs` (repo `htui`), `src/a:b.rs` stays a
    bare glob (its `:` follows a `/`), `:foo` (no repo), an unclosed `[` (error with
    the glob named).
  - `canonical_globs`: order, dedup, language expansion appended, idempotent on its own output.
  - `skill_language`: every seed glob compiles; `Rust` and ` rust ` expand; `klingon` is refused
    by name; the fourteen seed names are present.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo tree -p htui-core -e normal`
  shows `globset` with no `log`.

### Task 2: writers and readers (D75-D79, D85)
- **Tests first**:
  - Conformance (`store/conformance.rs`, six cases, run on `MemStore` and `PgStore`):
    `skill_create_reads_back_with_version_one`,
    `skill_update_is_a_compare_and_set_and_refuses_a_taken_name`,
    `skill_version_append_is_a_compare_and_set_on_the_head`,
    `skill_binding_attach_change_detach_are_compare_and_set`,
    `skill_binding_refuses_by_rule` (phase of another project, missing pin, unknown language, bad
    glob, qualified glob on global, unknown repo, glob without globs, negative position — each
    writes nothing),
    `skill_binding_stores_expanded_globs_and_languages_as_typed`.
  - `tests/skill_writers.rs` (Postgres): two sessions append on the same head, one wins, the other
    gets `Stale` with the winner's head; two sessions attach the same key with `expected: None`.
  - The `CASES` pins move to 80.
- **Action**: D75-D79. Regenerate `.sqlx` against a database migrated fresh to `0007`.
- **Validate**: `htui-core`, `htui-store` (Postgres) and `htui-agent` gates; `cargo sqlx prepare
  --check`; `cargo build --workspace --all-features --all-targets`.

### Task 3: the clone gap (D80)
- **Tests first**: `graph.rs` `override_clone_carries_phase_attachments_and_marks_itself` (the demo
  `implement` phase's pinned `rust-style` attachment appears on the clone's `implement` phase with
  the same pin, position and activation; the clone's `is_override` is true; the source rows are
  untouched; `bound_skills(project, Some(cloned implement))` equals the source phase's answer);
  conformance: `create_step_graph` round-trips `is_override`.
- **Action**: D80.
- **Validate**: `htui-core`, `htui-store`, `htui-orch` gates.

### Task 4: store worker (D81)
- **Tests first** (in `crates/htui/src/skills.rs`, over `Backend::memory`): the snapshot carries the
  demo library (two skills, three versions), the three demo attachments, and the `htui` project's
  graphs, phases and repo names; each write answers `Skills` on success and `SkillsStale` on a spent
  token; offline answers `Failed` with the server-only sentence; `REQUEST_NAMES` ↔ routing arms.
- **Validate**: the `htui` gate.

### Task 5: Skills view (D82-D84)
- **Tests first** (`tests/skills.rs`, testkit harness, `2` then Skills view):
  - snapshots `skills__library`, `skills__edit`, `skills__diff_two_versions`,
    `skills__attachments`, `skills__attach_form_effective_globs`, `skills__repo_picker`,
    `skills__changed_elsewhere`;
  - `a_new_skill_needs_a_valid_name_and_a_body`, `a_saved_version_moves_the_head_and_the_estimate`,
    `the_winning_row_is_starred_per_project`, `a_glob_row_says_it_fires_from_milestone_5`,
    `detach_asks_first`, `tab_and_digits_still_switch_tabs_with_a_draft_open`,
    `the_strip_text_is_unchanged`;
  - `tests/skills_pg.rs`: the same create → append → attach round trip on Postgres.
- **Validate**: the `htui` gate; `cargo insta` review of each new snapshot; `templates__*.snap`
  unchanged.

## Test plan

Pure rules (names, globs, languages) are unit-tested against tables in `htui-core`. Every writer is
a conformance case on both stores, plus a two-session race on Postgres. The clone gap is tested
where it lives (`graph.rs`) and read back through `bound_skills`. The worker is tested over the
memory backend; the view through the testkit harness with snapshots, and once on Postgres.

## Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-25 | MOD-7 milestone 3 merges first and conflicts at shared insertion points (`traits.rs`, `mem.rs`, `pg/*.rs`, `writer.rs`, `engine.rs:6021`). | High | Low | Merge `main` before T2 and before review; conflicts are additive (new methods side by side). |
| R-26 | A language-map edit silently changes what an old attachment would store on re-save. | Medium | Low | The form shows `effective:` before save; saved rows never change on their own (D73). |
| R-27 | The snapshot reads every version body of every skill on each reload. | Low | Low | The library is tens of rows; revisit with a lazy version read if it grows. |
| R-28 | Users attach `glob` skills expecting them to fire. | High until row 5 | Medium | The pane labels glob rows `fires from milestone 5`; the Prompt sub-tab already shows `no_path`. |
| R-29 | A rename collides with a name in flight from another session. | Low | Low | `UNIQUE (name)` and `already_exists`; the view shows the refusal and keeps the form. |
| R-30 | `globset`'s semantics differ from what users expect of `touched_paths` bare globs. | Medium | Low | D74 documents the any-repo rule in the form's help line. |
| R-19 | `ort-sys` needs `ORT_LIB_LOCATION` here (`parcel.pyke.io` 403). | Certain here | Low | `/root/ort/env.sh` (sandbox only, no repo change). The maintainer asked about a pure-Rust embedder; that is a separate item if opened. |

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- `. /root/ort/env.sh; USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test --workspace --all-features --no-fail-fast -- --test-threads=2`
- `cargo sqlx prepare --check` against a database migrated fresh to `0007`
- `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`
- Pins afterwards: `CASES` 80, `READ_CASES` 14, `TABLES` 39, commented columns 34, next migration
  `0008`, `MIRRORED_TABLES` 21.

## Acceptance

- In the Skills tab a maintainer creates a skill, edits it into version 2 in the `TextArea` or
  `$EDITOR`, diffs v1 and v2, and sees the token estimate move.
- They attach it globally `always`, switch it `off` for one phase, and the next step of that phase
  records it `off` while other phases render it (the milestone 2 read side, now reachable without
  SQL).
- A `glob` attachment with `languages: rust` stores `**/*.rs, **/Cargo.toml`, shows them before
  save, and refuses `nosuchrepo:**/*.rs`.
- An override clone carries its source phases' attachments and is marked `is_override`.
- Every write is a compare-and-set; a concurrent edit is reported, never overwritten.

## Where the PRD, ANA or tree disagree

- **ANA-22 §8** puts the glob matcher over F2 in milestone 3; split to row 5 by the maintainer
  (OQ-14). ANA-22 §10 records it at close-out.
- **ANA-22 §8 / model doc** name `upsert_skill`; D75 uses `create_skill` + `update_skill` (OQ-16).
- **ANA-22 §8** leaves `globset` vs hand-written open; `globset` (OQ-15).
- **`graph.rs:368-371`**'s rationale for not copying bindings ("would double every project
  binding") is superseded by ANA-22 §2 and D80: only phase rows are copied, onto new phase ids.
- **The milestone 2 plan D47** pins `CASES` at 71; it is 74 after MOD-7 milestone 2's merge.
- **`engine.rs:4964-4965`, `:5688`, `preview.rs:297`** say `repo_box_path` has no writer; MOD-15's
  `SetRepoPath` (`crates/htui/src/hierarchy.rs:303-322`) is one. Left for row 5, which rewrites
  those comments with the roots.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| No step gets a real excerpt root | true | `engine.rs:5691-5704` `no_excerpts`, used at `:4338`, `:4967`; `preview.rs:301-315` |
| Fan-out prompt assembled before candidate trees | true | `engine.rs:3381-3383` vs `candidate_live` `:3605-3640` |
| No per-attempt changed-path list | true | `Isolator::diff` → `DiffBlock` stat text, `isolate/git.rs:79`, `:815-845` |
| No glob matcher in `Cargo.lock`; `gix-glob` lacks braces | true | `Cargo.lock` grep; `gix-glob-0.27.1/src/wildmatch.rs` |
| `globset` 0.4.20, MSRV 1.88; its deps already locked | true | `cargo info globset`; `Cargo.lock:228`, `:672`, `:3724`, `:5183`, `:5200` |
| Workspace MSRV is 1.98 | true | `Cargo.toml:8` |
| `CasOutcome { Applied, Stale }` | true | `traits.rs:1676-1681` |
| `append_prompt_template` precedence: token first | true | `traits.rs:715-723` |
| `set_updated_at` covers `skill`, `skill_binding`, not `skill_version` | true | `0001_init.sql:570`, `:575-580` |
| No skill writer or list reader exists | true | survey; only `bound_skills` at `pg/read.rs:1432`, `mem.rs:421`, `backend.rs:366` |
| Five `WriteStore` implementors | true | `mem.rs:5216`, `pg/write.rs:587`, `writer.rs:350`, `htui-agent/src/conformance.rs:709`, `htui-agent/tests/recorder.rs:389` |
| `CASES` = 74, `READ_CASES` = 14 | true | `store/conformance.rs:43-118`, `:291-306`; `pg_conformance.rs:19` |
| `NewStepGraph` has no `is_override`; 9 constructors in 7 files | true | `model/kind.rs:152-161`; grep `NewStepGraph {` |
| `create_step_graph` writes no `is_override` | true | `mem.rs:2520`; `pg/write.rs:2180-2203` |
| `override_graph` has no production caller | true | `graph.rs:1460` (test), `lib.rs:56` (re-export) |
| Override note and its test exist | true | `engine.rs:87`, `:5018`, `:12071` |
| No constraint ties a binding's phase to its project | true | `0001_init.sql:432-441`, `0007_skill_attachments.sql:19-28` |
| `SKILLS_LATER` stub, not pinned by a test | true | `ui/tabs/skills/mod.rs:26`, `:127`; grep in `crates/htui/tests` |
| Skills render with the name in an attribute | true | `render.rs:444-447` |
| `TokenEstimator::DEFAULT` exists | true | `estimate.rs:59` |
| Repo names come from `WriteStore::repos` | true | `traits.rs:597`; `pg/write.rs:1898` |
| Demo skill names pass D71 | true | `fixtures.rs:515-538` (`rust-style`, `tests`) |
| Task file sets intersect only as stated | true | Tasks table, intersected by hand |
