# ANA-22 - How a skill is stored and when it activates (concluded, 2026-09-25)

Opened at MOD-9's PRD gate (`.claude/prds/mod-9-skill-library-templates.prd.md` D3) when the
maintainer asked that an imported SKILL.md file's frontmatter become real fields — when a skill
activates, for which language — and that the prompt builder inject the necessary skills.
Analysis: `docs/ANA-22.md`.

**Verdict (as amended the same day, `docs/ANA-22.md` §10).** A **skill is pure library content** —
name, description, versioned body, and the raw import frontmatter in `skill_version.source` — with
no dependency on any project, repo, language or activation rule. It is **attached** at one of three
levels, **global** (`skill_binding.project_id` NULL), **project** or **phase**, and the
**attachment carries the activation**: `always` (the default, so every existing binding is
unchanged), `glob` or `off`. The most specific attachment of a skill wins (phase over project over
global), extending `R-SKL-2` by one level. Language is authoring sugar compiled to globs at save
through a data map; per-repo activation is a repo-qualified glob (`<repo>:<glob>`, the
`touched_paths` syntax) on a project or phase attachment, refused on a global one. Globs match the
files the excerpt walk already enumerates under the step's resolved roots, narrowed to the item's
`touched_paths` prefixes, plus the previous attempt's changed paths; with no root a glob attachment
records `no_path`. Each step records every candidate's choice and reason, which keeps `R-ID-5`
auditable over mutable attachment rows; the `max_skill_tokens` cap still refuses. Model-decided
activation is deferred (breaks inlining, needs MOD-11). Import writes only the skill and its
version; frontmatter activation keys prefill the attachment form. A workspace level was rejected:
`workspace_project` is many-to-many.

**Amendment.** The first conclusion stored activation on `skill_version` with repo-qualified globs;
the maintainer pointed out that a shared skill should depend on nothing and be added, with the
activation wanted, where it is used — which also removes the coupling of a global skill to one
project's repo names.

**Survey.** Thirteen formats; four activation mechanisms cover all of them (always, path glob,
model by description, manual); none has a language or version field — language is always globs.
Cursor, Windsurf and Kiro docs were unreachable and are marked unverified in §4.

**Schema.** One migration (next free, `0006` at conclusion) adds `source` to `skill_version`, makes
`skill_binding.project_id` nullable, and adds `activation`, `globs` and `languages` to
`skill_binding` with checks that a phase row has a project and a `glob` row has globs; the amended
SQL was applied to a scratch Postgres 16 on top of `0001`..`0005`.

**Spawned.** Nothing new: the work lands in MOD-9 milestones 3 (storage, attachments matrix with a
global row, selection, matcher, the `graph.rs` phase-binding clone gap found in passing) and 4 (import), which this
verdict unblocks. The `globset`-versus-hand-written matcher choice is left to the milestone 3 plan.

Commits: analysis and close-out in the commit that added this file.
