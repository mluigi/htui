# MOD-9 - Skill library and templates (done, 2026-09-30)

**Requirements:** `R-SKL-1..4`, `R-PRM-4`, `R-TUI-7`; by construction `R-ID-5`, `R-SEC-3`,
`R-NF-3`. `R-SKL-2` was amended at milestone 2 (an attachment carries the activation `always`,
`glob` or `off`).
**Design authority:** `docs/ANA-5.md` §4.1, §4.5, §4.6, §5.4 (template save validation through
`htui_core::prompt::template::parse`, the reserved `judge`/`handoff` names, the placeholder tables
as inline help, the excerpt walk) and ANA-22 (`docs/ANA-22.md`, write-up
`docs/decisions/ana/ana-22.md`, concluded 2026-09-25): skills live in global storage and are
attached at global, project or phase level with the activation on the attachment; §5.4 F2 is the
file set a glob matches; §7 is the schema and import mapping; §8 the phasing. ANA-22's verdict is
unchanged; its §10 records three amendments made during this item (2026-09-26 twice, 2026-09-30).
**Artifacts:** PRD [`.claude/prds/mod-9-skill-library-templates.prd.md`](../../../.claude/prds/mod-9-skill-library-templates.prd.md)
(2026-09-25; everything in the Skills tab, templates and skills views, no Settings section; in-app
`TextArea` plus `$EDITOR`; bound skills wired into engine and preview). Plans and `code-architect`
blueprints, one pair per milestone:

| # | Plan | Blueprint |
|---|---|---|
| 1 | [`.claude/plans/mod-9-templates-editable.plan.md`](../../../.claude/plans/mod-9-templates-editable.plan.md) | [`.claude/plans/mod-9-templates-editable.blueprint.md`](../../../.claude/plans/mod-9-templates-editable.blueprint.md) |
| 2 | [`.claude/plans/mod-9-skills-reach-the-run.plan.md`](../../../.claude/plans/mod-9-skills-reach-the-run.plan.md) | [`.claude/plans/mod-9-skills-reach-the-run.blueprint.md`](../../../.claude/plans/mod-9-skills-reach-the-run.blueprint.md) |
| 3 | [`.claude/plans/mod-9-skills-editable.plan.md`](../../../.claude/plans/mod-9-skills-editable.plan.md) | [`.claude/plans/mod-9-skills-editable.blueprint.md`](../../../.claude/plans/mod-9-skills-editable.blueprint.md) |
| 4 | [`.claude/plans/mod-9-skill-import.plan.md`](../../../.claude/plans/mod-9-skill-import.plan.md) | [`.claude/plans/mod-9-skill-import.blueprint.md`](../../../.claude/plans/mod-9-skill-import.blueprint.md) |
| 5 | [`.claude/plans/mod-9-glob-attachments-fire.plan.md`](../../../.claude/plans/mod-9-glob-attachments-fire.plan.md) | [`.claude/plans/mod-9-glob-attachments-fire.blueprint.md`](../../../.claude/plans/mod-9-glob-attachments-fire.blueprint.md) |

Decision numbering runs across the item's own plans (the ported import plan re-used D87..D105 in
PR #10's namespace and is cited as "import plan D*n*"); milestone 5 used D108-D137, R-44-R-56 and
OQ-27-OQ-29. MOD-7's plans use overlapping numbers in their own namespace, so milestone 5's code
comments cite "MOD-9 D1xx".
**Commits:** five milestones, `e971418`..`d35098b`. Each milestone below names its own range.
**Spawned:** MOD-55 (agent help while editing a template or skill, PRD gate), MOD-57 (the external
editor inside the TUI pane, at the merge of MOD-7 milestone 2), MOD-59 (a write's reply names
itself, milestone 3 review finding 3). ANA-23 (pure-Rust embedder) was raised by the maintainer
during milestone 3 but is not MOD-9 work.

## What shipped

Templates and skills are edited, versioned and diffed in the Skills tab, and skills reach the run.
A template save runs `parse`, puts the cursor on the byte offset of the mistake, warns on a missing
`{{item}}`, and appends a version as a compare-and-set. A skill is a global row with append-only
versions; it is attached at global, project or phase level with `always`, `glob` or `off`, the most
specific attachment wins, and a missing winning pin never falls back. SKILL.md files and rules
directories import into the library. Phase steps, judges and the Backlog preview resolve the
phase's candidates, and every candidate's outcome is recorded in `run_step.trim_record.skill_choices`
(`always`, `matched` with its `<repo>:<path>`, `no_match`, `off`, `no_path`, `missing_version`,
`not_placed`). A `glob` attachment fires over the step's F2 file set: the excerpt walk's listing
under the step's roots, narrowed to `touched_paths`, plus the previous attempt's changed paths.

**Migrations:** `0007_skill_attachments` (milestone 2) and `0008_trim_record_v3` (milestone 5,
comment only). The next migration is `0009` (cache: `0005`).

---

## The five milestones

### 1 - Templates are editable (`e971418`..`caacc96`, 2026-09-25)

- The `append_prompt_template` compare-and-set writer on every store (append-only, the head
  version as the token).
- `ui::TextArea`; the `$EDITOR` handoff with the terminal suspended and SIGINT/SIGQUIT held off
  htui.
- `Templates`/`SaveTemplate` on the store worker, and the Skills tab's `Templates` view: parse-gated
  save with the cursor on the error, a wrapped and scrollable diff between any two versions or
  against the built-in default.

### 2 - Skills reach the run (`7be0794`..`fd5161e`, 2026-09-26)

Widened by the maintainer to ANA-22's storage and activation read side (ANA-22 §10, first
2026-09-26 amendment).

- Migration `0007_skill_attachments` (ANA-22 §7.1): the global level, `activation`/`globs`/
  `languages` on the attachment, `skill_version.source`.
- One pure `model::skill::resolve` (most specific attachment wins, a missing winning pin never
  falls back), shared by both stores; `collapse` takes one list carrying `level` (plan D39).
- `select` in the assembler: `always` active; `off`, `glob` → `no_path`, `missing_version`,
  `not_placed` inactive; every choice in `trim_record.skill_choices`, record `v: 2` (D42).
- `GraphSource::bound_skills`, so phase steps and judges get the phase's candidates (a judge body
  may place `{{skills}}`, D48, though the default judge does not: its replayed `{{task}}` already
  carries them) and handoffs none. The preview resolves the phase that uses the chosen template;
  the Prompt sub-tab lists the choices.
- Review: `rust-reviewer` APPROVE WITH FIXES, findings applied.

### 3 - Skills are editable and attachable (`df91c82`..`e5db119`, 2026-09-26)

Split by the maintainer: glob *firing* became the PRD's row 5 (ANA-22 §10, second 2026-09-26
amendment).

- Skill names checked by the Agent Skills rule; `globset` (new dependency) behind
  `model::skill_glob` (`<repo>:` qualifiers, brace-aware lists, `canonical_globs`); a seed language
  map (`model::skill_language`, fourteen languages).
- Seven `WriteStore` methods on every store (`skills`, `skill_versions`, `skill_bindings`,
  `create_skill` with v1 in one transaction, `update_skill`, `add_skill_version`,
  `set_skill_binding`), each writer a compare-and-set with one refusal chain (`check_attachment`);
  store `CASES` 82.
- The clone gap closed: `NewStepGraph.is_override` is written; `override_graph` checks every copy
  before any write and carries the source phases' attachments (blueprint F-H: an override clone is
  checked first, then refused). The engine's clone-gap note is gone.
- `crate::skills` on the store worker; the Skills view: library, versions, diff, `TextArea` and
  `$EDITOR`, token estimate, rename; the attachments pane with a global row, winner stars, a form
  showing the effective globs and a repo picker.
- No migration; `.sqlx` 280.
- Review: `rust-reviewer` APPROVE WITH FIXES. Findings 1, 2, 4, 5, 7 and the acceptance gap (a
  phase `off` over a global `always`, end to end) applied (`22822ca`..`e5db119`); finding 3 opened
  as MOD-59; finding 6 (no editor on a skill with no version) fixed in milestone 5 (OQ-28);
  finding 8 accepted (documented residue in `crates/htui/src/skills.rs`).

### 4 - Existing skills come in (`09fd007`..`6af3f53`, 2026-09-29)

Ported from PR #10 (`4abb49a`..`bbac75c`) onto milestone 3 above; #10's own milestone 3 was not
taken. The plan and blueprint are each headed by the port's departures.

- A hand-written frontmatter reader (`model::frontmatter`, no YAML dependency, per-key issues with
  byte and line, block scalars accepted) and ANA-22 §7.3's mapping (`model::skill_import`,
  `prefill_from_source`).
- `StoreRequest::ImportSkills` / `StoreReply::SkillImports` (`skills::REQUEST_NAMES` 5 → 6),
  walking a file or directory on the store worker (`crate::skill_import`: any SKILL.md to depth 4,
  a rules directory's `*.md`/`*.mdc`, a hidden tool root's rules child only, bundled
  `scripts/`/`references/`/`assets/` skipped and listed, 64-file and 256 KiB caps asked before the
  read), writing through `create_skill`, or through `update_skill` for a moved description and
  `add_skill_version` for a changed body when the name exists; never an attachment.
- The Skills view's `I` path form and per-file report; the attachments pane prefills a new
  attachment from the head's `source`.
- Three review findings fixed in the port: a hidden tool root collected its own markdown and
  skipped its rules files; a `<name>.instructions.md` or snake-case stem was refused; a name
  repeated in one import answered a spurious stale refusal.
- No migration, no `WriteStore` method, no `.sqlx` file, no dependency.

### 5 - Glob attachments fire (`f57fca4`..`d35098b`, 2026-09-30)

Plan `7fa06e7`, fact-check `f355ca0`, maintainer confirmation `0e73705` (OQ-27..OQ-29 at their
defaults, 2026-09-29): two independent verifiers re-checked every claim; the ones falsified as first
drafted were corrected in place (OQ-27's precedent and cost, D119's capture mechanism, the
three snapshots D124/D128 expected to move, T5's file list, T2's `promote.rs`, D116's wording,
three citations). Blueprint `f3729ca` (departures B-1..B-3, fill-ins F-1..F-7, numbered
D130-D137). Seven implementation tasks, serial on one tree, red tests committed before green:

| Task | Commits | What |
|---|---|---|
| T1 | `f57fca4` `b077477` | Model: reasons, `StepFiles`, `select` (D109-D113) |
| T2 | `8d2024a` `ace607e` `19ccae5` | Listing split, the F2 builder, its scrub filter, record `v 3` (D114-D118) |
| T3 | `2a812c0` `59ca213` | Migration `0008` (D118, OQ-27) |
| T4 | `9a4f3a0` `aed72ea` | `Isolator::changed_paths` (D119, D130, D135) |
| T5 | `e16b67d` `52a0c49` `ada1b2e` `b38cbaa` `bb71d5d` `c6c0ca5` | The shared pass, engine and preview (D120-D124, D132, D133) |
| T6 | `cac3719` `2cdede5` `6760d20` | Prompt sub-tab and attachments pane (D125, D126, D131) |
| T7 | `42ce3b4` `0399367` | Milestone 3 finding 6 (D127, D134, OQ-28) |

- **Reasons and the record (D109, D118).** `ChoiceReason` gains `matched` and `no_match`;
  `SkillChoice` gains `path`, `Some("<repo>:<path>")` iff `matched` and skipped when `None`, so
  every earlier choice's JSON is byte-identical. `RECORD_VERSION` 2 → 3. `0008_trim_record_v3.sql`
  is one `COMMENT ON COLUMN run_step.trim_record` restating the contract for `v 3` (OQ-27); it is
  the first comment-only migration, and it raises `schema_version()` to 8, so every box's SQLite
  mirror is deleted and refilled once on its first start after upgrade (R-56, accepted). The demo
  record `IMPL_TRIM_RECORD` stays `v 2`: it is a record written before `0008`.
- **`StepFiles` and `select` (D110-D113).** A per-repo reach map (`BTreeMap<String,
  BTreeSet<String>>`) rather than ANA-22 §7.2's two-armed `Resolved | NoPath` enum, because a
  step's scope can be half-resolved. `select(candidates, placed, &files)` compiles each glob
  candidate's `SkillGlobs` once per step; the first hit in `(repo bytes, path bytes)` order is
  `matched` (`R-ID-5`: never walk order); no hit is `no_match` when a reached repo is one the globs
  can match in (`SkillGlobs::reaches`), else `no_path`; a glob that no longer compiles is
  `no_match` (R-53). `needs_files` lets the pass skip the walk when no placed `glob` winner exists.
- **One walk per step (D114, D115, D120, D121).** `excerpt::select` split into `list` and
  `select_listed` with the note order pinned literally (D136). `step_files` builds F2: every listed
  repo is reached; a listed file enters when `touched_paths` is empty or a prefix matches; every
  repo-relative, non-skipped changed path enters too. `htui_agent::excerpt::step_pass` replaces
  `excerpts_for` and returns the excerpts and the file set; it walks under `spawn_blocking` once
  when `{{excerpts}}` or a glob winner is placed, and not at all otherwise. The residual is measured
  with `step_files` set, so a newly active skill's tokens are paid before excerpts are chosen. The
  changed paths also feed excerpt tier 2 (OQ-29), replacing MOD-7 D122's empty list.
- **Scrubbing (D116, D132).** `drop_unmaskable_files` removes every path the scrubber refuses
  before it can be recorded, with a count-only note; a repo stays reached when all its paths go, so
  `no_match` and `no_path` stay truthful. `withhold_unmaskable_notes` also covers the "listed for
  glob skills only" branch.
- **Changed paths (D119, D122, D130, D135).** `Isolator::changed_paths` (three implementors: real,
  fake, the `StallAfterReconcile` test double) runs `git diff --name-only -z --no-renames` over the
  same range selection as `diff`, through a new undecoded `Capture::HeadBytes` (B-1: a private
  `run_captured` returns the raw sink; `Exited` and the other verbs are untouched). Non-UTF-8 names
  are dropped; a 64 KiB overflow drops the partial entry and sets `truncated`. The engine reads the
  previous attempt's winner's trees and commits on `attempt > 1`; an isolator error or a cut list is
  a note.
- **Judges, handoffs, fan-out (D122, D123).** A judge runs in no tree and gets
  `StepFiles::default()`, so a placed glob winner records `no_path` (the default `judge` body places
  no `{{skills}}`, so `not_placed`); a handoff has no candidates. A fan-out group's file set is this
  box's `repo_box_path` checkout listing (MOD-7 D108), plus the previous winner's changed paths from
  attempt 2.
- **Preview and UI (D124-D126, D131).** The preview runs `step_pass` over this box's roots with no
  previous attempt, and its skills note says so; no snapshot moved. The Prompt sub-tab shows
  `matched <repo:path>`, `no_match`, `no_path`. The attachments pane drops "fires from milestone 5"
  and, on a project or phase `glob` row reaching a repo with no `repo_box_path` row on this box,
  appends `no path here: <a>, <b>` (`ProjectSkills.unrooted`; B-2: `snapshot` takes this box's rows
  as an argument, read once per request).
- **Finding 6 (D127, D134).** `e` and `E` on a skill with no version open the editor on an empty
  body and save v1 through `add_skill_version` with `expected = 0`; `i` (rename) and `a` (attach)
  no longer go dead on such a skill either.
- **Review** (`rust-reviewer`): APPROVE WITH FIXES, no CRITICAL or HIGH; all five findings applied.
  - 1, MEDIUM (`09aec1f`): `step_pass` builds the listing's file set only for a glob winner;
    `step_files` runs inside the walk's `spawn_blocking`, and the repo slug is scrubbed once.
    Deviation: `drop_unmaskable_files` stays on the runtime thread because the scrubber is
    borrowed, but it only runs when a glob winner is placed.
  - 2, LOW (`d8e15c2`): `drop_unmaskable_files` checks the joined `repo:path` too.
  - 3, LOW (`3a30a67`): two tests pinned, the glob-only branch's withheld note and the 64 KiB cut
    changed-path note. The suggested denied-path scenario cannot produce a listing note, so the
    test uses a repo slug that is a session secret.
  - 4, LOW (`ff18505`): `step_pass` withholds an unmaskable caller note; plus the
    maintainer-requested residue fix `d35098b`: the `previous_diff unavailable` note is withheld in
    `forwarded`, so it cannot refuse the record.
  - 5, LOW (`ff18505`): citations.
- **Scope fence (D129).** Not here: model-decided activation (ANA-22 §6 item 10, needs MOD-11),
  content-regex activation, a language-map overlay, a Settings surface, re-listing per fan-out
  candidate, a recorded copy of the file set itself.

**Pins (D128).** Moved: `RECORD_VERSION` 2 → 3; migrations `0001`..`0008`, next `0009` (cache
`0005`); `Pending(7)` → `Pending(8)`; `Isolator` +1 method (`changed_paths`);
`crate::skills::ProjectSkills` gains `unrooted`. Unchanged: `.sqlx` 288, `crates/htui/tests/snapshots`
107 (none changed content), store `CASES` 96, `READ_CASES` 14, `htui-orch` `CASES` 73, `GraphSource`
7 methods, `StoreRequest` 85, `StoreReply` 47, `skills::REQUEST_NAMES` 6, `TABLES` 39, 34 commented
columns (restated, not added), `MIRRORED_TABLES` 21. Tests whose pins moved, each with its reason:
`fixtures::tests::step_impl_carries_the_golden_trim_record` (the demo record stays `v 2`, asserted
explicitly), `applying_migrations_raises_the_target_and_never_lowers_it` (row count 8),
`an_attachment_saved_in_the_form_lands_and_the_form_closes` (search needle).

**Every phase-prompt digest with a matching `glob` winner moves** (R-49), and a retry's excerpt set
and digest move with tier 2 (R-52). Intended; digests are per step and no test pins a value.

---

## Deviations from the PRD and ANA-22

- **`StepFiles` is a per-repo reach map, not §7.2's `Resolved | NoPath` enum (D110),** and
  `select` keeps milestone 2's `placed` argument (D111).
- **§6 item 8's `matched: <path>`** is recorded as `reason: "matched"` plus a separate `path:
  "<repo>:<path>"` (D109), so the reason stays a closed `Copy` vocabulary.
- **PRD row 5's "roots for fan-out groups"** was already delivered by MOD-7 milestone 4's D108;
  row 5 added only the file set on those roots (D122).
- **Milestone 2 was widened and milestone 3 split** by the maintainer (ANA-22 §10, 2026-09-26).
- **Milestone 4 was ported** from PR #10 rather than built from its own plan.

---

## Carried

| What | Owner | Detail |
|---|---|---|
| Agent help while editing a template or skill | **MOD-55** | PRD gate, 2026-09-25. |
| The external editor inside the TUI pane | **MOD-57** | Raised at the merge of MOD-7 milestone 2. |
| A write's reply names itself | **MOD-59** | Milestone 3 review finding 3: the Skills and Templates views land a save by finding it in the re-read snapshot. |
| Scan cap and skip rules hide files from globs | accepted (R-44, R-48) | A root over `excerpt_max_scan_files` lists a prefix only, so a glob can record `no_match` though a later file matches (`audit.roots[].scan_truncated` says so). The walk's skip rules (lockfiles, minified, binary, over `max_file_bytes`, `.gitignore`d) apply to globs too: `**/Cargo.lock` or `**/*.min.js` never matches. |
| A fan-out group matches the managed checkout | accepted (R-50) | Not the candidates' fresh trees; the same accepted gap as MOD-7's fan-out excerpts. |
| Migration `0008` may collide | check at merge (R-45) | Another sandbox run could also claim `0008`; check at `scripts/hr collect MOD-9` and renumber this file and its pins if main gained one. |
| Milestone 3 finding 8 | accepted | Documented residue in `crates/htui/src/skills.rs`. |
| `cargo doc -D warnings` fails on four private-item links | pre-existing, not an item | `store/traits.rs:1251`, `agent_worker.rs:729`, `ui/text_area.rs:18`, `ui/text_field.rs:5`; all outside this change. |

---

## Validation

Milestone 5's gate on the main thread, 2026-09-30, before the residue fix `d35098b`:
`cargo test --workspace --all-features -- --test-threads=1` 2640 passed, 0 failed, 26 ignored;
`cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`
clean. The residue commit added one engine test. Milestones 1-4 recorded their gates in their plans;
each was green at `--test-threads=1` when it landed.

## Live coordinates

Kept in `HANDOFF.md`'s "Live coordinates" line: migrations through `0008_trim_record_v3`, next
`0009` (cache `0005`), `run_step.trim_record` at `v: 3`, and the D128 pins above.
