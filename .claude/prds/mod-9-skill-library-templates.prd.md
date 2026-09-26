# MOD-9 — Skill library and templates

> Routed as **PRD** by `/handoff-run MOD-9` (criteria C2 and C4 fired, C3 borderline). Ultracode
> recommended for the implement phase; the maintainer accepted. The contract is `R-SKL-1..4`,
> `R-PRM-4` and `R-TUI-7`, with the template half already designed in `docs/ANA-5.md` §4.1, §4.6,
> §5.4 and §6.2 and the read-only skill model shipped by MOD-2 (D105). The PRD gate opened two new
> items: **ANA-22** (how a skill is stored and when it activates) and **MOD-55** (asking an agent for
> help while editing). Milestones 3 and 4 wait on ANA-22.

## Problem

Nothing in `htui` can change a prompt template or a skill. The ten templates are seeded at version 1
when a project is created and never move; a maintainer who wants a different `implement` prompt has
to write SQL. The skill tables exist, the resolution rules (`R-SKL-2`) are implemented and tested,
and the assembler renders a skills section — but no code writes a skill, no UI shows one, and the
engine passes an empty skill list to every step, so even a skill inserted by hand never reaches a
run. The Skills tab is a one-line stub.

## Evidence

Facts gathered from the tree on 2026-09-25 while routing; each is re-checked by the plan fact-check.

- **The save-time validator exists and reports a byte offset.** `htui_core::prompt::template::parse(role,
  body)` (`crates/htui-core/src/prompt/template.rs:303`) documents itself as "the save-time gate MOD-9
  calls". `TemplateError` carries `at: usize` (byte offset of the opening `{{`) on
  `UnknownPlaceholder`, `WrongRole` and `Unterminated`; `MissingRequired` has no offset (`:376-411`).
  First error wins. `ParsedTemplate::omits_item()` (`:267`) is "MOD-9's warning, not an error".
- **The placeholder tables are public data.** `Placeholder::ALL`, `token()`, `allowed_in(role)`,
  `is_section()`, `required_by(role)` are `pub` (`template.rs:128-233`); there is no runtime
  description string per placeholder, only rustdoc and ANA-5 §4.1's table.
- **`template_text` is kept for this editor.** `render::template_text` (`render.rs:260`) has only
  test callers; its doc names MOD-9's editor (MOD-2 F-83, `fd9e752`).
- **Role comes from the name.** `TemplateRole::of_name` (`template.rs:52`): exact `judge` and
  `handoff` are reserved, everything else is a phase. MOD-15's graph editor already refuses phase
  names `judge` and `handoff`.
- **Templates are versioned rows with no writer.** `prompt_template` has
  `UNIQUE (project_id, name, version)` (`0001_init.sql:266-276`); only `create_project`'s seed
  (`pg/write.rs:4317-4338`, `mem.rs` around `:1993`) and the demo loader insert. Reads are inherent on
  `PgStore`/`MemStore`/`Backend` (`prompt_templates`, `prompt_template`), not on the traits, because
  the tables are not cache-mirrored (`traits.rs:25-27`). A phase's `template_version` NULL means
  latest (ANA-5 §4.1).
- **Skills have a model and a reader, no writer.** `Skill`, `SkillVersion`, `SkillBinding`,
  `BoundSkill::collapse` (`crates/htui-core/src/model/skill.rs`); the module doc names
  `upsert_skill`, `add_skill_version`, `set_skill_binding` as MOD-9's. `skill.name` is globally
  `UNIQUE`; `skill_version` is append-only (no `updated_at`); `skill_binding` is
  `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`.
- **Skills never reach a run.** The engine sets `skills: Vec::new()` for phase and judge steps
  (`crates/htui-orch/src/engine.rs:4310`, `:4925-4927`, whose comment defers it to "the Runs tab at
  milestone 6"). The preview resolves project bindings only (`crates/htui/src/preview.rs:227`,
  `bound_skills(project, None)`). `GraphSource` (`crates/htui-orch/src/graph.rs`) has no skills read.
- **The Skills tab exists as a stub.** `crates/htui/src/ui/tabs/skills.rs` (61 lines) renders one dim
  line attributing the tab to MOD-12; it is registered second (`app/mod.rs:48`) and the strip text
  ` 1 Backlog  2 Skills  3 Settings  4 Chat` is pinned by `tests/integration.rs:58` and seven
  snapshots — keeping the title keeps them.
- **No multi-line editor exists.** `TextField` is single-line by design (`ui/text_field.rs:1-7`),
  the chat composer is single-line, there is no `$EDITOR` handling and bracketed paste is off.
  MOD-7 D3 plans a small multi-line widget for quirks; it is not built.
- **A diff library is already in the workspace.** `similar = "3.2.0"` (`Cargo.toml:75`), used by
  `htui-agent`; `crates/htui` does not declare it. The chat transcript has the one unified-diff
  renderer (`chat/transcript.rs:552-570`, `diff_style` `:628-639`).
- **No YAML parser in the tree.** The one frontmatter reader is hand-written for the review verdict
  (`crates/htui-orch/src/gate.rs:64`). `SKILL.md` import has no format decision anywhere; `R-SKL-4`
  is one line.
- **A compare-and-set writer has a known shape.** `update_item_kind` (MOD-15): trait method →
  `MemStore` → `PgStore` (`UPDATE … WHERE updated_at = $n`, re-read to classify) → `Writer` dispatch →
  spy stores in `htui-agent` → `StoreRequest` variant, name, routing arm, handler → conformance case.
  Offline query data lives in `crates/htui-store/.sqlx/` (`SQLX_OFFLINE=true`).

## Users

- **Primary**: the maintainer tuning how agents are prompted. The need fires when a phase's output is
  consistently off and the fix is in the prompt, and when a convention (a coding standard, a review
  checklist) should reach every run of a project or phase without being pasted into item bodies.
- **Also served**: MOD-12 (auto mode runs whatever templates and skills are bound), MOD-26 (personas
  will be built on the same editor), MOD-55 (agent help plugs into this editor).
- **Not for**: editing skills as files on disk — the database is the only source of truth; import is
  a one-shot copy (D3). Agents never receive skills as files; the prompt builder injects them.

## Hypothesis

We believe **a Skills tab that edits templates and skills as versioned rows, validates on save with
the error on the cursor, diffs any two versions, binds skills to projects and phases, and feeds the
bound skills into every step's prompt** will **make prompt tuning a TUI task instead of a SQL task**
for **the maintainer**.

We'll know we're right when **editing the `implement` template in the Skills tab, saving it with an
unknown placeholder, lands the cursor on the placeholder; fixing and saving creates version 2 that
the next `implement` step uses; and a skill bound to the `implement` phase appears in that step's
prompt and the preview** — without leaving the TUI.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Save gate | Every save runs `parse`; a refused save writes no row; the cursor lands on `at` | Unit tests over each `TemplateError` variant; TextArea cursor test |
| Missing-item warning | A phase body without `{{item}}` saves after a confirm, never silently | Section test |
| Append-only versions | Save inserts version N+1; a concurrent save of the same N is reported as changed elsewhere, never overwrites | Conformance case over `MemStore` and `PgStore` |
| Diff | Any two versions of a template (and, after ANA-22, a skill) render as a line diff | Snapshot tests |
| `$EDITOR` round trip | `E` suspends the TUI, edits a temp file, returns, validates; the terminal is restored on every exit path | Integration test with a fake editor command |
| Skills reach runs | Phase and judge steps and the preview carry `bound_skills(project, Some(phase))` | `htui-orch` test; preview test |
| UI never blocks | Every read and write runs on the store worker (`R-NF-3`) | Existing worker pattern, no store handle on the render side |

## Scope

**MVP** — the Skills tab with two views, a reusable multi-line editor plus an `$EDITOR` handoff,
versioned template editing with the ANA-5 save gate, version diff, and bound skills wired into every
step. Skill writing, bindings and import follow once ANA-22 settles how a skill is stored.

Concretely in scope:

- **Skills tab, two views (D1).** The stub becomes a tab with a `Skills | Templates` view switch.
  Templates view: the scoped project's templates by name, versions, body, diff. Skills view
  (milestone 3): library, editor, diff, bindings matrix by project and phase (`R-TUI-7`). The tab
  title stays `Skills`.
- **Multi-line editor (D2).** A small reusable `TextArea` widget in `crates/htui/src/ui/` (MOD-7's
  quirks editor reuses it), and `E` to hand the body to `$VISUAL`/`$EDITOR` with the TUI suspended.
  A refused save puts the cursor on the error's byte offset; `MissingRequired` puts it at the end.
- **Template writer (D5).** Save inserts version N+1 of `(project, name)`; the version the editor
  opened is the compare-and-set token. New template names may be created (version 1, role from the
  name); nothing is deleted. Inline help lists `Placeholder::ALL` filtered by the role.
- **Version diff.** `similar` line diff between any two versions, rendered with the chat
  transcript's `diff_style`.
- **Bound skills in every step (D4).** The engine resolves `bound_skills(project, Some(phase))` for
  phase and judge steps through a `GraphSource` seam; the preview passes the phase too.
- **Skill writers, Skills view, bindings (milestone 3, after ANA-22).** `upsert_skill`,
  `add_skill_version`, `set_skill_binding` (bind, unbind, pin, follow latest, position) in whatever
  shape ANA-22 settles; per-skill token estimate at save (ANA-5 risk 2).
- **`SKILL.md` import (milestone 4, after ANA-22).** Frontmatter fields become skill fields as
  ANA-22 decides; a typed path names a file or a directory.
- **Record corrections.** The Skills stub's MOD-12 attribution, the engine comment at
  `engine.rs:4925`, and MOD-23's HANDOFF text (MOD-9 adds no Settings section).

**Out of scope**

- **Agent help while editing** — **MOD-55**, opened at this PRD gate.
- **How a skill is stored and activated** (frontmatter fields, language/glob/trigger conditions,
  automatic selection by the prompt builder) — **ANA-22**, opened at this PRD gate.
- **Exporting skills to files, materialising them for agents** — declined (D3): the database is the
  only source of truth and the prompt builder injects skills.
- **Deleting templates or skills** — versions are referenced by pinned phases and trim records.
- **Syntax highlighting, Markdown rendering, bracketed paste** in the editor.
- **Personas** — MOD-26.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-09-25, before planning.

- **D1 — Everything lives in the Skills tab.** Two views, Skills and Templates. No Settings section;
  MOD-23's text saying otherwise is corrected.
- **D2 — Both editors.** An in-app `TextArea` and an `$EDITOR` handoff. Asking an agent for help is a
  separate item, **MOD-55**.
- **D3 — The database is the source of truth.** Skills live in Postgres; import is a one-shot copy;
  nothing is exported or materialised as files — the prompt builder injects the necessary skills.
  **How a skill is saved** — which frontmatter fields become columns (e.g. when it activates, for
  which language) — is an analysis, **ANA-22**, run before milestone 3.
- **D4 — Engine wiring is in scope.** Phase and judge steps and the preview carry the phase-level
  bound skills.
- **D5 — Proposed at the gate, confirmed by the plan:** template saves are append-only (new version
  row, the opened version as the compare-and-set token); new names allowed; no delete.
- **D6 — Sequencing.** ANA-22 runs first, in the same session; milestones 1 and 2 do not wait on it,
  milestones 3 and 4 do.

## Constraints (fixed before planning)

- **Migrations are forward-only (`R-STO-5`).** Milestone 1 needs none (templates already have
  version rows); ANA-22 may add one for milestone 3. `tests/migrations.rs` pins the applied list,
  table count and commented-column count.
- **`R-NF-3` is enforced by ownership.** Reads and writes go through `StoreRequest` on the store
  worker; the `$EDITOR` process runs with the render loop suspended, not beside it.
- **Every writer is a compare-and-set**, keyed on a token only its own writes change.
- **`parse` is the only template validator**; the editor adds no grammar of its own.
- **The strip text ` 1 Backlog  2 Skills  3 Settings  4 Chat` stays** unless a snapshot update is
  deliberate.
- **`unsafe_code = "forbid"`, MSRV and the workspace lint set unchanged; TDD per repo convention.**
- **New `query!` macros need `cargo sqlx prepare`** against a migrated scratch database.

## Delivery Milestones

<!-- Status: pending | in-progress | complete | blocked -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Templates are editable | Skills tab `Templates` view: list, body, diff between versions; `TextArea` and `$EDITOR` editing; save runs `parse`, puts the cursor on the error, warns on a missing `{{item}}`, and appends a version as a compare-and-set. | complete (`e971418`..`caacc96`, 2026-09-25) | [plan](../plans/mod-9-templates-editable.plan.md), [blueprint](../plans/mod-9-templates-editable.blueprint.md) |
| 2 | Bound skills reach the run | Engine phase and judge steps and the preview resolve `bound_skills(project, Some(phase))`. Widened 2026-09-26 by the maintainer to ANA-22's storage and activation read side (migration `0007`, global level, `always`/`off` selection recorded per step). | complete (`7be0794`..`fd5161e`, 2026-09-26) | [plan](../plans/mod-9-skills-reach-the-run.plan.md), [blueprint](../plans/mod-9-skills-reach-the-run.blueprint.md) |
| 3 | Skills are editable and bindable | Skill writers, `Skills` view with library, editor, diff, attachments pane (global, project, phase) and token estimate; the language map and glob matcher at save; the clone gap. Split 2026-09-26 by the maintainer: glob *firing* is row 5. | complete (`df91c82`..`e5db119`, 2026-09-26) | [plan](../plans/mod-9-skills-editable.plan.md), [blueprint](../plans/mod-9-skills-editable.blueprint.md) |
| 4 | Existing skills come in | `SKILL.md` import from a file or directory, frontmatter mapped per ANA-22. | pending (ANA-22 concluded) | — |
| 5 | Glob attachments fire | The F2 file set (the excerpt walk's listing under the step's roots, narrowed to `touched_paths`, plus the previous attempt's changed paths), roots for fan-out groups, `select` with `matched`/`no_match`, the preview's roots. Opened 2026-09-26 by the milestone 3 split; better after MOD-7 milestone 4 writes `repo_box_path`. | pending | — |

Milestones 1 and 2 are independent of each other and of ANA-22. Milestone 4 needs milestone 3's
writers; milestone 5 needs milestone 3's matcher and step roots (MOD-7 milestone 4 helps).

## Open Questions

- [x] Where templates are edited — Skills tab (D1).
- [x] Editor — both `TextArea` and `$EDITOR`; agent help is MOD-55 (D2).
- [x] Skills as files — no; database only (D3).
- [x] Engine wiring — in scope (D4).
- [x] How a skill is stored and when it activates — ANA-22 concluded (`docs/decisions/ana/ana-22.md`): a skill is library content attached at global, project or phase level; the attachment carries activation (`always`, `glob`, `off`); §7 schema and import mapping.
- [ ] How `$EDITOR` is chosen and invoked on Windows (`notepad` fallback?) — plan's call.
- [ ] Whether the Templates view follows the scope selector's project or has its own picker — plan's call.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `$EDITOR` leaves the terminal in raw/alternate mode after a crash or signal | Medium | High | Restore on every exit path through a guard; test with a fake editor that exits non-zero |
| A saved template breaks the `review` front matter or `judge` json wire contract | Medium | High | `parse` enforces required placeholders; the view shows the default body's diff; wire contracts named in inline help |
| Wiring skills changes every production prompt digest | High | Low | Expected; empty binding lists render no section, so digests move only where skills are bound |
| ANA-22 changes the skill tables and invalidates milestone 3 assumptions | High | Low | Milestone 3 is not planned until ANA-22 concludes |
| The multi-line widget grows into a general editor | Medium | Medium | Scope capped: insert, delete, newline, arrows, home/end, page; no undo stack beyond cancel |

---
*Status: APPROVED at the PRD gate — milestones 1–2 ready for /plan; 3–4 follow ANA-22 (concluded).*
