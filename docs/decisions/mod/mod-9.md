# MOD-9 - Skill library and templates (done, 2026-09-28)

A skill is a name, a description and a versioned markdown body in Postgres. This item built the
whole thing around that: an editor for templates, a read path that puts a skill into a prompt, the
writers that make a skill editable, and an import that brings a SKILL.md off the disk. Four
milestones, `e971418`..`8b259a4`; plans and blueprints in `.claude/plans/mod-9-*.{plan,blueprint}.md`.

**Requirements.** `R-SKL-1` (the library is name + description + versioned body), `R-SKL-2` as
amended by ANA-22 (attachments at global, project or phase level, the most specific wins, the
attachment carries the activation), `R-SKL-3` (create, edit, version diff, bind, unbind — each verb
is a test), `R-SKL-4` (import from existing SKILL.md files), `R-TUI-7` (the Skills tab), and
`R-PRM-4`'s editor sibling. `R-ID-5` and `R-ID-6` held throughout: the import path is deterministic
code, and the injected bytes are digested.

## What landed, in four milestones

**Milestone 1 — templates editable** (`e971418`..`caacc96`). `append_prompt_template` as a
compare-and-set writer on every store, append-only with the head version as the token; `ui::TextArea`;
the `$EDITOR` handoff with the terminal suspended and SIGINT held off htui; the Skills tab's
`Templates` view, with a parse-gated save that puts the cursor on the byte the error names and a
diff against any two versions or the built-in default.

**Milestone 2 — skills reach the run** (`7be0794`..`fd5161e`), widened by the maintainer to ANA-22's
storage and activation read side. Migration `0007_skill_attachments`: a global attachment level,
`activation`/`globs`/`languages` on the attachment, `skill_version.source`. One pure
`model::skill::resolve` shared by both stores; `select` in the assembler with every choice recorded
in `trim_record.skill_choices`; `GraphSource::bound_skills` so phase steps and judges see the
phase's candidates. `R-SKL-2` amended.

**Milestone 3 — skills editable and bindable** (`2d5a4fd`..`614ac2f`). Four compare-and-set writers
— `upsert_skill`, `add_skill_version`, `set_skill_binding`, `remove_skill_binding` — on both stores,
with four conformance cases and a Postgres CAS race; `WriteStore` 79 → 83. The glob matcher is
**hand-written** (`htui_core::prompt::glob`), chosen at the plan gate over `globset` so no dependency
was added; `Isolator::changed_paths` feeds it and makes `TIER2_PREV_DIFF` live in production; the
Skills view and the attachments matrix with a global row, the language → globs map and a repo
picker. Review found a CRITICAL the fix agent's own brief had overstated: the matcher's `**` rewind
resumed at the current path position instead of the one where the `**` began, a false negative on
ordinary patterns such as `**/*.rs` against `vendor/x.rs/y.rs`, fixed and pinned.

**Milestone 4 — SKILL.md import** (`4abb49a`..`8b259a4`). A hand-written frontmatter reader and
ANA-22 §7.3's mapping in `htui-core`; a worker path that reads a file or a directory and writes
through the two writers milestone 3 already had; the Skills view's `i` import form, its per-file
report, and the matrix form's prefill from the stored source.

## The decisions worth keeping

- **A skill depends on nothing.** Name, description, versioned body, and the import's raw frontmatter
  in `skill_version.source`. The **attachment** carries scope and activation; the most specific
  attachment wins. This is ANA-22 §6, amended by the maintainer after the first conclusion put
  activation on the version, because a shared skill should be added where it is used rather than
  bound to one project's repo names.
- **The matcher is hand-written.** `globset` is one package and was rejected; the deciding finding
  was that `*` matches across `/` in globset by default, so every pattern would need two builder
  flags configured, one of them platform-dependent. The price is stated once: we own this syntax, and
  a pattern we do not implement is **refused at save time** rather than silently matching nothing.
- **Import writes only `skill` and `skill_version`.** Nothing is attached by import; the frontmatter's
  activation keys are *prefill*, and the matrix reads them back out of the stored `source` when the
  skill is attached. That is why a skill imported last week and attached today is prefilled too.
- **A directory import collects SKILL.md at any depth and `*.md`/`*.mdc` only under a rules
  directory** — a directory named for what it holds, or a hidden tool root with a rules-named child.
  The first draft took the depth alone and a test caught it turning a project root into a skill
  sweep, where `README.md` is a skill named `readme`.
- **Block scalars are read.** ANA-22 §5.7 lists "multi-line folded scalars" among the exotic YAML
  the hand-written reader rejects, and all three attested forms (`>`, `>-`, `|`) occur in real files
  — 38, 40 and 19 paths in one marketplace tree alone. The maintainer took the widened reading at
  the gate; the widening is recorded in the plan's OQ-26.
- **No migration for milestone 4.** `skill_version.source` arrived with `0007`; both writers and every
  reader already carry it, so the next free number stayed `0009`, `WriteStore` stayed at 83, `CASES`
  at 81 and `.sqlx` at 281.

## What it cost, and what it left

- **A cap asked before the read.** The self-review caught `read_text` pulling a file into memory and
  *then* checking its size; a refusal that has already read the file is not a refusal.
- **`source` is a contract, not a detail.** The key set is pinned by a test on the writing side and
  re-read on the prefilling side, and the one derivation of §7.3's prefill rows has a single
  definition with two callers.
- **The review gate was not independent for milestone 4.** Every `Agent` route returned
  `API Error: 402 Insufficient credits` from 10:10 CEST, and the `Agent` tool's model enum cannot
  express the session model, so no subagent could be spawned. The maintainer chose to proceed inline.
  The blueprint, the three tasks and the review were done by one agent on one model; the review was a
  second pass over the diff, not the gate `handoff-run` requires.

## Follow-ups

`MOD-55` (agent help while editing) and `MOD-57` (the embedded editor) were spawned from this item
and no longer depend on it. Model-decided activation is deferred until `MOD-11` exists, and the
`regex` activation and activation-by-item-kind the analysis considered are not opened. `MOD-49` was
unblocked by `MOD-7`, not by this item.
