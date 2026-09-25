# ANA-22 - How a skill is stored and when it activates

> **Scope note:** "The maintainer wants the frontmatter of an imported SKILL.md file to become real
> fields — when the skill activates, for which language — and the prompt builder to inject the
> necessary skills." (`HANDOFF.md`, ANA-22; opened at MOD-9's PRD gate, 2026-09-25,
> `.claude/prds/mod-9-skill-library-templates.prd.md` D3.)
>
> **Requirements addressed:** `R-SKL-1`, `R-SKL-2`, `R-SKL-4`, `R-PRM-1`, `R-PRM-3`, `R-ID-5`.
>
> **Status (2026-09-25): concluded.** Verdict: a skill keeps its explicit bindings as the **scope**
> (which project and phase may see it), and gains an **activation rule stored on each version** as
> the **filter** inside that scope: `always` (today's behaviour) or `glob` (injected only when a file
> the step is about matches). Language is authored as a convenience and compiled to globs at save.
> Everything else in a frontmatter block is kept verbatim in a `source` JSON column and never read by
> the prompt builder. Model-decided activation (a catalog in the prompt, body fetched on demand) is
> deferred: it breaks `R-ID-5`'s inlining and needs MOD-11's MCP server. Import reads the Agent
> Skills / Claude Code `SKILL.md` format and the common rules formats through one hand-written
> frontmatter reader.

Code citations are against HEAD `6592d78`.

---

## 1. Context and problem statement

`htui` stores a skill as a name, a description and a versioned markdown body, and injects it only
where a binding says so. The maintainer's ask has two halves:

1. **Storage.** An imported SKILL.md file carries frontmatter — at minimum `name` and
   `description`, often activation hints (paths, globs, "always apply") and tool-specific keys.
   Which of these become columns, which are kept, which are dropped?
2. **Activation.** "The prompt builder automatically injects the necessary skills." Today it injects
   exactly the bound ones. Should it choose, and from what?

The answer must keep what `htui` already promises: a step's prompt is reproducible from recorded
inputs (`R-ID-5`: skills are inlined so behaviour is identical on every box), skills are never
trimmed and an oversize skill set refuses the step rather than dropping a skill (ANA-5 §4.2,
invariant 4), and the prompt digest covers what was injected.

## 2. Current state in htui

- **Tables** (`crates/htui-store/migrations/0001_init.sql:406-441`): `skill (id, name UNIQUE,
  description, created_by, created_at, updated_at)` — the library is global, scoping is by binding;
  `skill_version (skill_id, version ≥ 1, body, created_by, created_at)`, append-only, no
  `updated_at`; `skill_binding (id, skill_id, project_id, phase_id NULL = project level,
  pinned_version NULL = follow latest, position, updated_at)` with
  `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`. No column comments. Next free
  migration number is `0006` today (`HANDOFF.md:32`), subject to MOD-7 milestone 2.
- **Resolution** (`crates/htui-core/src/model/skill.rs`): `SkillBinding::version_in_force`
  (`:80-86`) takes the pin or the highest version, and drops a binding whose pin names a missing
  version. `BoundSkill::collapse(project, phase)` (`:121-137`) lets a phase binding override the
  project binding of the same skill, dedups by `skill_id`, and sorts by `(position, name bytes)`.
- **Readers**: `bound_skills(project, Option<PhaseId>)` on `PgStore` (`pg/read.rs:1150`), `MemStore`
  (`store/mem.rs:389`) and `Backend` (`backend.rs:363`); inherent, not on the traits, because skill
  tables are not cache-mirrored (`store/traits.rs:25-26`).
- **Rendering** (`prompt/render.rs:436-459`): `<skill name="…" version="N">\nbody\n</skill>` per
  skill, LF-joined; no section when empty. `{{skills}}` is allowed in the phase role only
  (`template.rs:187-213`); all eight phase defaults use it.
- **Cap**: `max_skill_tokens` (default 20 000, `settings.rs:40`) refuses the step with
  `SkillsExceedCap` (`trim.rs:464-478`, `mod.rs:372-378`) before the budget check.
- **Recording**: the trim record carries one `skills` section with token counts
  (`trim.rs:181-205`); **which skills and versions were injected is recorded nowhere structured** —
  only in the rendered bytes the digest covers (`digest.rs:36,73`).
- **Supply**: the engine passes `skills: Vec::new()` for phase and judge steps
  (`crates/htui-orch/src/engine.rs:4310`, `:4927`); the preview passes project bindings only
  (`crates/htui/src/preview.rs:227`). MOD-9 milestone 2 wires both.
- **Signals available at assembly time** (`engine.rs:4830-4948`): item kind, title, body, phase
  name, `output_kind`, attempt, box profile; in scope but not passed: `item.touched_paths`
  (`"<repo>:<glob>"`, bare glob = primary repo, `0003_orchestration.sql:109-111`),
  `item.required_tags`, `run.repo_scope`. **No repo language exists anywhere**: `repo` has no
  language column (`0001_init.sql:185-196`); `box.probed_tags` (`rust`, `cmake`, …) describe the
  box's toolchains, not the repo, and are excluded from `BoxProfile` (`model/box_.rs:187-188`).
- **File walking already exists for excerpts** (ANA-5 §4.5, `prompt/excerpt.rs`): the ranker
  enumerates files under the repo roots of `run.repo_scope`, with `.git/`, the secret denylist,
  `.gitignore`, binary, size and lockfile skips, and weights files under `touched_paths` prefixes
  (`PathPrefix`, `excerpt.rs:54`). Its root is `run_step_tree.path`, then `repo_box_path`, then
  `no_path` — and `repo_box_path` gets its automatic writer in MOD-7 milestone 4.
- **No glob matcher and no YAML parser** are in `Cargo.lock`; `touched_paths` is only ever cut to a
  prefix (`excerpt.rs:61`, `crates/htui-orch/src/overlap.rs`). The one frontmatter reader is
  hand-written for the review verdict (`crates/htui-orch/src/gate.rs:64`).
- **Demo skills** (`crates/htui-core/src/fixtures.rs:472-565`): `rust-style` (v1, v2) and `tests`,
  bound at project level and `rust-style` pinned to v1 on the implement phase.
- **Found in passing**: a graph override clone copies `step_graph_phase` deeply but never
  `skill_binding` (`crates/htui-orch/src/graph.rs:350-352`, `:1414-1415`), so phase bindings do not
  follow a cloned phase. Recorded for MOD-9 milestone 3.

## 3. Constraints

- **`R-ID-5`** — the injected text is fully determined by recorded inputs and identical on every box.
  Anything activation reads must be recorded or be part of the digest's inputs.
- **Never trim, refuse instead** (ANA-5 §4.2). Activation may only *select*; the cap still refuses.
- **The database is the only source of truth** (MOD-9 PRD D3). Import is a one-shot copy; no file
  is read at run time.
- **Prompts are assembled up front.** Every surveyed tool matches its path rules against files the
  agent reads *during* a session; `htui` has no such moment. It must pick the file set before the
  agent starts.
- **Forward-only migrations** (`R-STO-5`); new `query!` macros need `cargo sqlx prepare`.

## 4. Prior art

Surveyed 2026-09-25. Cursor, Windsurf and Kiro vendor docs were unreachable from this environment
and are marked *unverified* (search snippets and secondary sources only); every other row was read
from the primary docs or the tool's own source on GitHub.

| Format | Location | Metadata | Activation |
|---|---|---|---|
| **Agent Skills spec** (agentskills.io, `docs/specification.mdx`) | `<name>/SKILL.md` + optional `scripts/`, `references/`, `assets/` | `name` (req., 1-64, `[a-z0-9-]`, no edge or double hyphen, = directory), `description` (req., ≤1024, "what and when"), `license`, `compatibility` (≤500), `metadata` (string map; `version` by convention), `allowed-tools` (experimental) | Model picks by `description`. Progressive disclosure: ~100-token metadata always loaded, body (<5000 tokens advised) on activation, bundled files on demand |
| **Claude Code skills** (code.claude.com/docs/en/skills) | `~/.claude/skills`, `.claude/skills`, nested, plugins | Spec fields + `when_to_use`, `argument-hint`, `arguments`, `disable-model-invocation`, `user-invocable`, `allowed-tools`, `disallowed-tools`, `model`, `effort`, `context`, `agent`, `background`, `hooks`, `shell`, **`paths`** | Model by description; `paths` globs gate auto-load; `disable-model-invocation` = manual `/name` only. Unknown keys ignored |
| **Claude Code rules** (code.claude.com/docs/en/memory) | `.claude/rules/**/*.md` | `paths` only (list or comma string, brace expansion) | No `paths` = always; `paths` = when a matching file is read |
| **Cursor** *(unverified)* | `.cursor/rules/*.mdc` | `description`, `globs` (comma string), `alwaysApply` | Always / auto-attached by glob / agent-requested by description / manual `@rule` |
| **GitHub Copilot** (docs.github.com; microsoft/vscode-docs) | `.github/copilot-instructions.md`; `.github/instructions/*.instructions.md` | `applyTo` (comma globs), `excludeAgent`; VS Code adds `name`, `description` | Repo file always; `applyTo` by glob; description = on demand (VS Code). Also reads Agent Skills from `.github/skills`, `.claude/skills`, `.agents/skills` |
| **Windsurf** *(unverified)* | `.windsurf/rules/*.md` | `trigger` (`always_on`, `manual`, `model_decision`, `glob`), `globs`, `description` | One-to-one with `trigger` |
| **Continue** (continuedev/continue docs) | `.continue/rules/*.md`, lexicographic order | `name`, `globs`, **`regex`** (on file content), `description`, `alwaysApply` (tri-state) | Always / glob / regex / description |
| **OpenAI Codex** (openai/codex source) | `AGENTS.md` root→cwd concatenated, 32 KiB cap; skills in `.agents/skills` | Skills: `name`, `description`, `metadata.short-description`; sidecar `agents/openai.yaml` (`interface`, `dependencies`, `policy`) | AGENTS.md always; skills explicit `$name` or model-implicit |
| **Cline** (cline/cline docs) | `.clinerules/` | `paths` | No frontmatter = always; `paths` = when a matching file is open, mentioned or edited |
| **Roo Code** | `.roo/rules/`, `.roo/rules-{mode}/` | none | By agent mode (directory name) |
| **Gemini CLI** (google-gemini/gemini-cli docs) | `GEMINI.md` hierarchy; Agent Skills in `.gemini/skills`, `.agents/skills` | Spec fields | GEMINI.md always; skills by model via `activate_skill`, with consent |
| **Kiro** *(unverified)* | `.kiro/steering/*.md` | `inclusion` (`always`, `fileMatch`, `manual`), `fileMatchPattern` | One-to-one with `inclusion` |
| **Aider** | `CONVENTIONS.md` via `--read` | none | Always while loaded |

**What the survey says:**

1. **Four activation mechanisms cover every format**: *always*, *path glob*, *model decides from the
   description*, *manual*. Continue's content `regex` is the only other, and it is rare.
2. **No format has a language field.** Language is always expressed as globs (Continue's
   "TypeScript-specific rules" example is `globs: ["**/*.ts","**/*.tsx"]`; Copilot and Claude both
   advise per-language files with `applyTo`/`paths`).
3. **No format has a version field.** The Agent Skills spec puts `version` in `metadata` by
   convention; `htui`'s row versioning is stronger than anything in the survey.
4. **The common, portable fields** are `name`, `description`, an activation mode and a glob list.
   Everything else (`allowed-tools`, `model`, `hooks`, `context`, `excludeAgent`, …) is specific to
   the tool that reads it.
5. **Model-decided skills rely on progressive disclosure**: a catalog of names and descriptions in
   context, the body fetched when the model decides. That needs a way for the agent to fetch.
6. **Path rules match files the agent touches during a session.** None of the tools assembles a
   prompt up front the way `htui` does.

## 5. Options

### 5.1 Where activation sits relative to bindings

| Option | Description | For | Against |
|---|---|---|---|
| **A1 Filter inside bindings** | A binding decides *where* a skill may apply (project, phase); the skill's activation rule decides *whether* it applies to this step | Keeps `R-SKL-2` and `collapse` as they are; a project never receives a skill nobody bound; one project-level binding plus a glob rule is already "automatic" per step | A skill must still be bound once per project |
| A2 Selection of its own | Every skill in the global library is a candidate for every step; activation alone decides | Zero binding work | A global library leaks every project's conventions into every other; `R-SKL-2`'s bindings become redundant; one bad glob affects every project |
| A3 Both | Bindings, plus a per-skill "global" flag that makes it a candidate everywhere | Covers house-wide conventions | Two scoping mechanisms to explain and test; the same effect is one project binding per project away |

### 5.2 Where the activation fields live

| Option | For | Against |
|---|---|---|
| On `skill` | One place, simple editor | Changing a glob changes what a **pinned** binding does, so a pin no longer pins behaviour — `R-ID-5` in spirit |
| **On `skill_version`** | A pin pins the body *and* the rule; import of a new SKILL.md with new `paths` is a new version, as it should be; history shows when a rule changed | The editor edits body and rule together (it already saves a new version for either) |

### 5.3 How language is expressed

| Option | For | Against |
|---|---|---|
| L1 Globs only | What every format does | The maintainer asked for language explicitly; `**/*.rs, **/Cargo.toml` is tedious to type |
| L2 Language as its own trigger, matched against a repo language column | Reads well | No repo language exists; it would need detection, storage and a refresh rule, and still reduces to file extensions |
| **L3 Language as authoring sugar compiled to globs at save** | Reads well in the editor; one matcher; the effective globs are stored on the version, so a later change to the language map never changes a saved version | Two fields shown in the editor (languages as typed, globs as stored) |

### 5.4 What file set a glob is matched against

The prompt is assembled before the agent runs, so `htui` chooses the files.

| Option | Files | For | Against |
|---|---|---|---|
| F1 `touched_paths` globs themselves | Glob-vs-glob intersection | No filesystem | Glob intersection is undecidable in general; `touched_paths` is often empty |
| **F2 Files the excerpt walk already enumerates**, restricted to `touched_paths` prefixes when the item has any, else the whole `run.repo_scope` checkout; plus the previous attempt's changed paths | Concrete paths from a walk that already exists and already honours `.gitignore` and the secret denylist; "does this repo contain Rust" falls out for items with no `touched_paths` | Needs a resolved root — `repo_box_path` (MOD-7 milestone 4) or the step's worktree; a box with no path row activates no glob skill |
| F3 The worktree's full file list, always | Simple | Loses the item's own focus: a Rust skill fires on a docs-only item in a mixed repo |

### 5.5 Model-decided activation

| Option | For | Against |
|---|---|---|
| M1 Catalog in the prompt, body fetched on demand | What the Agent Skills spec and Claude Code do; cheapest in tokens | The fetched body is not in the prompt digest — `R-ID-5` breaks unless every fetch is recorded; needs a fetch tool (MOD-11's MCP server); ACP and CLI agents differ in whether they can call it |
| M2 Catalog plus all bodies inlined | Reproducible | Same tokens as `always`; nothing gained |
| **M3 Defer; import maps description-only skills to `always`** | Import works today; a bound skill behaves as today; the original intent is kept in `source` for when M1 lands | No model-decided skills yet |

### 5.6 What happens to the rest of the frontmatter

| Option | For | Against |
|---|---|---|
| Drop unknown keys | Simple | Import is lossy; a later feature (MOD-11 tool scoping from `allowed-tools`) cannot recover them |
| Column per key | Queryable | Twenty tool-specific columns nobody reads |
| **Keep the raw frontmatter and the import provenance in one `source` JSONB column** | Lossless, cheap, never read by the prompt builder, so it cannot change a digest | Not validated |

### 5.7 Import parser

| Option | For | Against |
|---|---|---|
| **Hand-written frontmatter reader** | The maintainer's choice at the PRD gate; no dependency; the subset real files use is small: `key: scalar`, quoted scalars, inline `[a, b]`, block `- a` lists, comma strings, one level of nested map (`metadata:`) kept as raw text | Rejects exotic YAML (anchors, multi-line folded scalars) — reported per key, never silently |
| A YAML crate | Complete | New dependency for a convenience path; YAML 1.1/1.2 edge cases (`no` → `false`) in names |

## 6. Verdict

1. **A1 — bindings stay the scope, activation is a filter.** `collapse` is unchanged; after it, each
   bound skill's version-in-force is kept or dropped by its activation rule. "The prompt builder
   injects the necessary skills" is realised by binding a skill once at project level with a glob
   rule: from then on each step gets it only when it is about matching files. A2 and A3 are
   rejected: a global candidate set leaks conventions across projects.
2. **Activation lives on `skill_version`** (5.2). A pin pins behaviour.
3. **Two activation modes now: `always` and `glob`.** `always` is today's behaviour and the default,
   so every existing row and binding behaves exactly as before the migration. `glob` requires a
   non-empty glob list.
4. **Language is authoring sugar (L3).** `languages text[]` is kept as typed; at save the editor and
   the importer expand it through a language→globs map (data, not code — the same pattern as MOD-7's
   probe spec), union it with the typed globs, and store the result in `globs`. The matcher reads
   `globs` only. A saved version never changes when the map does.
5. **Glob matching reads F2**: the files the excerpt walk enumerates under the step's resolved repo
   roots, restricted to `touched_paths` prefixes when the item has any, plus the previous attempt's
   changed paths. Glob syntax: `*`, `**`, `?`, `{a,b}`, `[...]`, matched against repo-relative paths
   with `/` separators, optionally repo-qualified as `<repo>:<glob>` exactly like `touched_paths`.
   With no resolvable root, a glob skill is **not activated** and the reason is recorded; the step
   proceeds (an unmatched rule is not an error).
6. **Record the selection.** The trim record's `skills` section gains the list of candidates with
   `(skill, version, activation, active, reason)` — `always`, `matched: <path>`, `no match`,
   `no_path`. The digest already covers the injected bytes; the record makes the *choice* auditable,
   which `R-ID-5` needs once selection depends on the tree.
7. **The cap still refuses.** Activation only narrows the set; `SkillsExceedCap` is unchanged.
8. **Model-decided activation is deferred (M3).** Import maps a description-only skill (the Agent
   Skills default) to `always` and records `"activation_hint": "model"` in `source`. A catalog with
   on-demand fetch becomes possible once MOD-11's MCP server exists; it must record every fetch in
   the step's history to keep `R-ID-5`. Opened as an idea in §8, not an item yet.
9. **Frontmatter is kept whole in `source`** (5.6), alongside `format`, `path` and `imported_at`.
   The prompt builder never reads it.
10. **Import uses a hand-written reader** (5.7) and maps formats as in §7. Names follow the Agent
    Skills rule (`[a-z0-9-]`, 1-64, no edge or double hyphen) for new skills, checked by the writer,
    not by a constraint, so existing rows are untouched. Bundled `scripts/`, `references/` and
    `assets/` are not imported; the importer lists what it skipped. A same-name import appends a
    version only when body or activation differs, and updates `skill.description`.

## 7. Schema and mapping

### 7.1 Migration (next free number, `0006` today)

```sql
ALTER TABLE skill_version
    ADD COLUMN activation TEXT   NOT NULL DEFAULT 'always'
        CHECK (activation IN ('always', 'glob')),
    ADD COLUMN globs      TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN languages  TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN source     JSONB  NOT NULL DEFAULT '{}'::jsonb,
    ADD CONSTRAINT skill_version_glob_needs_globs
        CHECK (activation <> 'glob' OR cardinality(globs) > 0);

COMMENT ON COLUMN skill_version.activation IS
    'always = injected wherever bound; glob = injected where bound and a step file matches globs (ANA-22)';
COMMENT ON COLUMN skill_version.globs IS
    'effective globs, typed globs plus languages expanded at save; the only field the matcher reads';
COMMENT ON COLUMN skill_version.languages IS 'languages as authored; display only';
COMMENT ON COLUMN skill_version.source IS
    'import provenance and raw frontmatter; never read by the prompt builder';
```

`skill.description` stays on `skill` (it is the library's display line and, once model activation
exists, the catalog line). `tests/migrations.rs`' pinned applied list, table count (unchanged) and
commented-column count (+4) move with it.

### 7.2 Model

`SkillVersion` gains `activation: Activation { Always, Glob }`, `globs: Vec<String>`,
`languages: Vec<String>`, `source: serde_json::Value`. `BoundSkill` gains `activation` and `globs`
so the assembler can filter; a new pure function
`select(bound: Vec<BoundSkill>, files: &StepFiles) -> (Vec<BoundSkill>, Vec<SkillChoice>)` runs
after `collapse`, where `StepFiles` is `Resolved(paths) | NoPath`. `collapse` itself is unchanged.

### 7.3 Import mapping

| Source key | → `htui` field |
|---|---|
| `name` (or directory name, or file stem) | `skill.name` (validated) |
| `description` (+ `when_to_use` appended after a blank line) | `skill.description` |
| body after the closing `---` | `skill_version.body` |
| `paths` (Claude), `globs` (Cursor, Windsurf, Continue), `applyTo` (Copilot), `fileMatchPattern` (Kiro) | `globs` (comma strings split, lists taken as is); non-empty → `activation = glob` |
| `alwaysApply: true`, `trigger: always_on`, `inclusion: always`, `applyTo: "**"` | `activation = always` |
| `languages` (htui's own key, for round-tripping) | `languages` |
| description only, `trigger: model_decision`, `inclusion: manual`, `disable-model-invocation`, `trigger: manual` | `activation = always`; the hint kept in `source.activation_hint` |
| everything else (`license`, `compatibility`, `metadata`, `allowed-tools`, `model`, `hooks`, …) | `source.frontmatter`, verbatim |

## 8. Phasing

All of it lands in **MOD-9 milestones 3 and 4**, which this verdict unblocks. No new item is needed.

- **Milestone 3 (skills editable and bindable)** adds: the migration of §7.1; `Activation`,
  `globs`, `languages`, `source` on the model and both stores; `add_skill_version` taking the
  activation fields; the language→globs map as data; the editor's activation, languages and globs
  fields with the expanded globs shown before save; `select` and the trim-record choice list; the
  glob matcher over F2's file set. The phase-binding clone gap in `graph.rs` (§2) is fixed there.
- **Milestone 4 (import)** adds the hand-written frontmatter reader and §7.3's mapping, file or
  directory input (`*/SKILL.md`, and `*.md`/`*.mdc` under a rules directory), skip list for bundled
  files, and the same-name version rule.
- **Dependency**: glob activation needs a resolved repo root. Until MOD-7 milestone 4 writes
  `repo_box_path` rows, a glob skill activates only in steps that run in a worktree
  (`run_step_tree.path`); elsewhere it records `no_path`. MOD-9 milestone 3 does not wait on MOD-7.
- **Glob matcher**: the plan decides between `globset` (the matcher behind ripgrep; a new
  dependency, justified in the plan per MOD-7's rule for new crates) and a small hand-written
  matcher over the §6.5 syntax. Either is tested against the same table.
- **Later, not opened**: model-decided activation (M1) once MOD-11 exists; content `regex`
  activation if a real need appears; activation by item kind (a `kinds` filter) if phase bindings
  prove too coarse.

## 9. Risks and open questions

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A glob skill silently never fires because no root resolves | High until MOD-7 M4 | Medium | `no_path` recorded in the trim record and shown in the preview; the editor warns when the scoped project has repos with no path row |
| The walk for activation slows assembly | Low | Low | It is the excerpt walk, done once per step and shared |
| A same-file skill fires on an unrelated item in a mixed repo | Medium | Low | F2 restricts to `touched_paths` prefixes when the item declares them |
| Frontmatter reader rejects a real file | Medium | Low | Per-key error with line number; the body still imports if the maintainer accepts `always` |
| The language map misses a language | Medium | Low | Map is data; typed globs always work |
| Recording choices bloats the trim record | Low | Low | One short row per bound skill |

**Open for the MOD-9 milestone 3 plan:** `globset` vs hand-written matcher; the exact language map
seed (at least `rust`, `c`, `cpp`, `python`, `typescript`, `javascript`, `go`, `java`, `csharp`,
`shell`, `sql`, `markdown`, `toml`, `yaml`); whether `SkillChoice` rides the trim record's existing
`skills` section or a sibling field.

## 10. Amendment record

- 2026-09-25 — concluded at authoring.
