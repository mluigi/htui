# ANA-22 - How a skill is stored and when it activates (concluded, 2026-09-25)

Opened at MOD-9's PRD gate (`.claude/prds/mod-9-skill-library-templates.prd.md` D3) when the
maintainer asked that an imported SKILL.md file's frontmatter become real fields — when a skill
activates, for which language — and that the prompt builder inject the necessary skills.
Analysis: `docs/ANA-22.md`.

**Verdict.** Bindings stay the **scope** (which project and phase may see a skill, `R-SKL-2`
unchanged) and each **skill version** gains an **activation rule** as the filter inside it:
`always` (today's behaviour, the default, so every existing row is unchanged) or `glob`. Storing the
rule on the version means a pin pins behaviour as well as text. Language is authoring sugar
compiled to globs at save through a data map; the matcher reads only the stored `globs`. Globs match
the files the excerpt walk already enumerates under the step's resolved repo roots, narrowed to the
item's `touched_paths` prefixes when it has any, plus the previous attempt's changed paths; with no
root a glob skill is not activated and `no_path` is recorded. Each step records every candidate's
choice and reason beside the trim record's `skills` section, which keeps `R-ID-5` auditable; the
`max_skill_tokens` cap still refuses. Remaining frontmatter is kept verbatim in a `source` JSONB
column the prompt builder never reads. Model-decided activation (catalog plus on-demand fetch) is
deferred: it breaks inlining and needs MOD-11; import maps description-only skills to `always` and
keeps the hint. Import uses a hand-written frontmatter reader mapping `paths`/`globs`/`applyTo`/
`fileMatchPattern` across the Agent Skills, Claude Code, Cursor, Copilot, Windsurf, Continue and Kiro
formats.

**Survey.** Thirteen formats; four activation mechanisms cover all of them (always, path glob,
model by description, manual); none has a language or version field — language is always globs.
Cursor, Windsurf and Kiro docs were unreachable and are marked unverified in §4.

**Schema.** One migration (next free, `0006` at conclusion) adds `activation`, `globs`, `languages`
and `source` to `skill_version`, with a check that `glob` has globs; the SQL was applied to a
scratch Postgres 16 on top of `0001`..`0005` at authoring.

**Spawned.** Nothing new: the work lands in MOD-9 milestones 3 (storage, editor, selection,
matcher, the `graph.rs` phase-binding clone gap found in passing) and 4 (import), which this
verdict unblocks. The `globset`-versus-hand-written matcher choice is left to the milestone 3 plan.

Commits: analysis and close-out in the commit that added this file.
