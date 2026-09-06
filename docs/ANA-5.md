# ANA-5 - Prompt assembly: template contract, section model, upstream walk, trim order, file excerpts

> **Scope note:** Design authority for how `htui` turns a resolved step into the one self-contained
> initial prompt an agent receives: the prompt template placeholder contract and its validation, the
> ordered section model and its rendering, the upstream-summary walk within the workspace bound, the
> token budget and the fixed trim order, deterministic file excerpt selection with no external tool,
> the three named templates ANA-2 requires (`judge`, the review-loop forwarded set, the promotion
> handoff prompt), and the byte-exact serialisation that makes `run_step.prompt_digest` reproducible.
> Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md`, `docs/REQUIREMENTS.md`,
> `docs/ANA-9.md` (§4.3, §5.4, §5.5, §5.6, §5.8, §5.9, §5.10, §6.1, §7.3), `docs/ANA-2.md`
> (§4.1, §4.2, §4.4, §4.5, §4.7, §4.8, §5.1) and `docs/ANA-4.md` (§4.1, §8, §9).
>
> **Requirements addressed:** `R-PRM-1..4`, `R-ID-5`.
> Touched at their seams only, and settled elsewhere: `R-ID-4` and `R-ID-6` (excerpt selection reads
> and never writes, and is deterministic code with no model in the loop), `R-SKL-1..3` (MOD-9 owns
> the library and the editor; this document owns the resolution and the render), `R-ORCH-3` (ANA-2
> owns the review loop; this document renders its forwarded set), `R-ORCH-7` (ANA-2 owns fan-out and
> the judge protocol; this document owns the judge prompt and the identical-prompt guarantee),
> `R-ORCH-5` (ANA-2 owns promotion; this document owns the handoff prompt), `R-HIS-1` (ANA-4 owns
> the recorder; this document owns what the `prompt` row contains), `R-SEC-3` (ANA-7 owns the
> scrubber; this document owns where it runs relative to the digest), `R-LATER-7` (ANA-3 supplies
> providers; this document owns the seam they plug into).
>
> **Status (2026-09-06): concluded.** Implementation tracked as MOD-2 (prompt builder) in
> `HANDOFF.md`, with template editing in MOD-9 and seeded templates in MOD-15. No new migration
> file: §9 folds this document's schema-side needs into MOD-2's `0002_agent_probe.sql`.

---

## 1. Context and problem statement

`docs/REQUIREMENTS.md` §8 fixes the contract in four sentences and hands every mechanism to this
document. `R-PRM-1` (`docs/REQUIREMENTS.md:202-206`) enumerates seven prompt contents and one
prohibition; `R-PRM-2` (`:207`) gives out-of-bound upstream items a one-line stub; `R-PRM-3`
(`:208-210`) names a per-step budget and a five-entry priority order; `R-PRM-4` (`:211-212`) makes
templates versioned Postgres rows with "a documented placeholder contract, editable in the TUI".
`docs/ANA-9.md:544` points the database itself at this document, in a column comment:

```sql
    body        TEXT NOT NULL,                   -- placeholder contract per ANA-5
```

`docs/ANA-9.md:11-13` repeats the delegation: "ANA-2, ANA-4, **ANA-5** and ANA-7 amend this schema
by forward-only migration where their verdicts need columns this document only reserves."

`docs/ANA-4.md:1257-1260` states the seam from the other side, verbatim:

> "**What ANA-5 must provide** (it gates step 1's `prompt` row): the assembled prompt text, the
> `sections[] { name, tokens, trimmed }` array, the token budget accounting that produces
> `run_step.trim_record`, and a stable serialization order so `prompt_digest` is reproducible for
> the same inputs. The driver computes the digest; ANA-5 owns what is digested."

`docs/ANA-2.md:1842` adds three templates and a stable serialisation to the same list, and
`docs/ANA-2.md:385` puts the work at stage 3 of the six-stage step lifecycle:

```
3. prompt     input_kinds resolution -> ANA-5 assembly -> prompt_digest
```

This document settles the eight questions the `HANDOFF.md` ANA-5 item and its two upstream ANAs
name, in this order:

1. the template placeholder contract (`R-PRM-4`): syntax, the closed set, escaping, conditionals,
   version pinning per phase, the default body per seeded phase name, validation on save, and what
   happens when a template names a placeholder the phase cannot supply,
2. the section model (`R-PRM-1`): the ordered sections, their rendering, the `sections[]` array, and
   the rule that raw transcripts of other items never appear,
3. the upstream-summary walk (`R-PRM-1`, `R-PRM-2`): edge kinds, direction, hops, the workspace
   bound, stubs, ordering, dedup, the summary document, and the store-trait methods,
4. the token budget and the trim order (`R-PRM-3`): counting, budget resolution, the reading of
   "first", per-section strategies, the `trim_record` shape and its writer, and the TUI surface,
5. file excerpt selection without an external tool (`R-PRM-1`, `R-ID-4`, `R-ID-6`, `R-LATER-7`),
6. the three templates ANA-2 names: `judge`, the review-loop forwarded set, the handoff prompt,
7. the byte-exact serialisation that makes `prompt_digest` reproducible,
8. crate and module placement for the prompt builder.

Six premises that were current when the item was written are now false or stale, and each of them
changes an answer. They are recorded here so the next reader does not re-derive them.

| Stale premise | State on 2026-09-06 |
|---|---|
| ANA-5 needs a migration for `run_step.trim_record` | False. The column ships today: `docs/ANA-9.md:765`, `crates/htui-store/migrations/0001_init.sql:488`, mirrored at `cache_migrations/0001_mirror.sql:117`, modelled at `crates/htui-core/src/model/run.rs:156-158`. Only its JSON shape is missing. §9 therefore adds no DDL. |
| ANA-5's migration is the next free number | Stale. ANA-4 took `0002_agent_probe.sql` (`docs/ANA-4.md:1267-1273`) and ANA-2 took `0003_orchestration.sql` (`docs/ANA-2.md:1848-1851`); neither exists on disk. MOD-2 lands `0002` before MOD-4 writes `0003`, so anything ANA-5 needs at MOD-2 time rides inside `0002`. §9 argues the three options. |
| `app_setting.token_budget` is the last link of a working chain | False. `docs/ANA-2.md:284` terminates the chain there, but the key is absent from ANA-2 §5.4's reserved table (`:1509-1522`), from `0003`'s seed INSERT (`:1986-1999`), from ANA-9 §5.9's key list and from `SEEDED_SETTINGS`, which ships two cache keys. §5.3 reserves, defaults and seeds it. |
| ANA-9 §7.3's SQL can express `R-PRM-2` | False. `CASE WHEN s.project_id IS NOT NULL THEN d.body END` (`docs/ANA-9.md:933`) yields NULL both for an out-of-scope item and for an in-scope item with no `summary` document, and the caller cannot tell them apart. The recursive term also dedups `(item_id, depth)` rather than `item_id`, so a diamond renders one summary twice. §4.3 amends both. |
| The template placeholder syntax is open | Mostly. One artefact exists and it is demo data: `crates/htui-core/src/fixtures.rs:637` writes a body whose rendered text contains the literal `{{item}}`. No templating crate is in `Cargo.lock` (`minijinja`, `tera`, `handlebars`, `liquid`, `askama` all absent). §4.1 adopts the shipped shape rather than inventing a second one. |
| A `summary` document is usually available for an upstream item | False, and this is the common case rather than the edge case. `document.kind = 'summary'` is written only by close-out, in the transaction that moves the item to `closed` (`docs/ANA-2.md:1380-1391`); `done` and `closed` are "genuinely different" (`:592-594`); none of the five seeded graphs has a `summary` phase (`crates/htui-core/src/fixtures.rs:508-544`) and §5.10 seeds no `summary` template. §4.3 gives that case its own render. |

Three facts shape every answer and are stated once here rather than repeated.

**The prompt is one `String` and nothing downstream re-parses it.** `AgentDriver::start` takes
`prompt: String` (`docs/ANA-4.md:238-243`); the recorder hashes it once and writes it as
`payload.text` of the `prompt` event at `seq = 0, turn = 0` (`docs/ANA-4.md:338-340`,
`docs/ANA-9.md:233`). Every delimiter this document chooses is therefore for the model's benefit and
for a human reading the Runs tab, never for a machine that must parse the prompt back. The one
exception is the judge's `task` section, which replays a stored prompt verbatim (§4.6).

**Nothing behavioural exists and almost nothing readable exists.** `ReadStore` is seven methods and
`WriteStore` is three plus a comment naming this gap (`crates/htui-core/src/store/traits.rs:50`).
There is no reader that returns a `PromptTemplate`, no reader that returns a `Document` body, no
`Skill`, `SkillVersion` or `SkillBinding` Rust type anywhere in the tree, no reader for
`repo_box_path`, and no code that walks a working tree: `htui-core` contains no `std::fs` at all,
and every filesystem call in the workspace is either under `dirs::config_dir()/htui`
(`htui-store/src/cache/*`, `htui-store/src/identity.rs`) or is the log file at
`crates/htui/src/lib.rs:120`. `MemStore` holds templates behind
`#[expect(dead_code, reason = "loaded now, read by MOD-2 / MOD-4 / MOD-15")]` and holds no skills at
all. §8 names every method and every type MOD-2 must add.

**Two of the three prompts this document defines depend on data that does not exist until after
MOD-2 ships.** The judge's `candidates` section needs `run_step.verify_outcome`,
`verify_exit_code` and `run_step_tree`, all of which arrive in ANA-2's `0003_orchestration.sql`
authored by MOD-4 (`docs/ANA-2.md:1926-1948`); the review loop's `previous_diff` needs the same
tables plus a git library that enters the workspace at MOD-4 build step 5 (`docs/ANA-2.md:1805-1807`).
This is not a defect: §9 splits the build order so MOD-2 ships the assembler, the step prompt and the
seam, and MOD-4 fills the two orchestration-only sections through the seam MOD-2 defines.

---

## 2. Invariants

Restated from `CONCEPTS.md`, `docs/REQUIREMENTS.md`, `docs/ANA-2.md` and `docs/ANA-4.md`; each has a
mechanical enforcement point in this design.

1. **No raw transcript of another item ever enters a prompt.** `R-PRM-1`'s last clause. Enforced by
   `PromptSpec` (§8) having no field that can carry a `SessionEvent`, and by the one legitimate
   transcript-derived section, the handoff `step_summary`, being built by deterministic code from
   the step's **own** stored events (§4.6) and rendered as a summary rather than a replay.
2. **The prompt is a pure function of its inputs.** Same template version, same item version, same
   resolved documents, same upstream rows, same base commit, same budget: same bytes, on any box,
   under any scheduling. Enforced by §4.7's ordering rules, by the ban on wall-clock values and
   absolute paths inside the digested text, and by the token estimator being a pure function.
3. **Fan-out siblings receive byte-identical prompts.** `R-ORCH-7` says N candidates run "on the
   same prompt", so their `prompt_digest` values must be equal. Enforced by the assembler running
   **once per (position, attempt)** rather than once per candidate - which is what makes the rule
   hold even for `shared_serialized`, whose siblings run one after another in one tree
   (`docs/ANA-2.md:916`) - by excerpts being read once at stage 3 before any sibling starts, and by
   every rendered path being repo-relative (§4.5).
4. **Skills and the phase template are never trimmed.** `R-ID-5` requires that inlining skill text
   makes "behavior identical on every box"; a skill that shrinks under budget pressure makes
   behaviour a function of how much other context happened to be present. Enforced by §4.4's
   protected set and by a step that cannot fit the protected set failing before a token is spent.
5. **The scrubber runs before the digest, not after.** `R-HIS-1` stores the prompt "after
   scrubbing" and `R-SEC-3` fails closed. Enforced by §4.7: every section's content passes the
   ANA-7 scrubber during assembly, so the bytes sent, the bytes digested and the bytes persisted are
   one string; the recorder's own scrub pass over the `prompt` payload must find nothing.
6. **`htui` writes nothing into a managed repository while building a prompt.** `R-ID-4`. Enforced
   by the excerpt path being read-only by type: `RepoReader` (§8) exposes `list` and `read` and no
   write operation, and no excerpt provider is given a write surface.
7. **No model chooses what enters a prompt.** `R-ID-6`. Enforced by the built-in excerpt ranker
   being a fixed-weight deterministic function and by `ExcerptProvider` being documented as
   non-LLM; a provider that calls a model is out of contract, which is what makes ANA-3's providers
   (Serena's LSP index, Graphify's graph) the right shape and an embedding reranker the wrong one.
8. **A missing required input fails loudly; a missing optional section renders empty.** ANA-2
   already fixed the first half at stage 3: a missing `input_kinds` document fails the step with
   `missing input document: <kind>` because "a phase silently running without its plan produces an
   artefact that looks valid and is not" (`docs/ANA-2.md:413-416`). Enforced by §4.1 leaving that
   rule untouched and by an absent optional section (no upstream links, no excerpts, attempt 1)
   rendering as the empty string with no `sections[]` entry.
9. **An unknown placeholder is never silently literal.** `R-PRM-4` requires a documented contract,
   which is worthless if an unrecognised token passes through as text. Enforced by §4.1's validator
   rejecting it at save in MOD-9's editor, and by the assembler failing the step at stage 3 with
   `unknown prompt placeholder: {{...}}` when a row reaches it that the editor did not write.
10. **The template version in force is the one pinned in the snapshot.** ANA-2 invariant 2 already
    makes a mid-run template edit invisible to a live run (`docs/ANA-2.md:109-113`, `:321-323`,
    `:1461`). Enforced by the assembler taking `TemplateRef { name, version }` from
    `graph_snapshot.phases[].template` and never resolving `latest` at assembly time for a graph
    run.

---

## 3. Surface as read

Read from the working tree on 2026-09-06, HEAD `f989da5`.

**`prompt_template`, eight columns** (`crates/htui-store/migrations/0001_init.sql:266-276` =
`docs/ANA-9.md:539-549`): `id`, `project_id`, `name` ("phase name; defaults are copied into each new
project"), `version` (`CHECK (version >= 1)`), `body` ("placeholder contract per ANA-5"),
`created_by`, `created_at`, `updated_at`, `UNIQUE (project_id, name, version)`. There is no
`is_builtin` column, no `kind` column, and no `CHECK` on `name`. `PromptTemplate` exists as a Rust
type (`crates/htui-core/src/model/kind.rs:139-157`) and no store method returns one.

**`step_graph_phase` carries the three fields this document resolves against**
(`0001_init.sql:242-244`): `template_name TEXT NOT NULL` ("prompt_template.name, defaults to phase
name"), `template_version INTEGER` ("NULL = follow latest"), `token_budget INTEGER` ("NULL =
project.settings.token_budget"). ANA-2 §4.1 fixes the resolution chains (`docs/ANA-2.md:284-285`)
and snapshots the results into `ResolvedPhase` (`:325-348`) with `template: TemplateRef` rendered as
`"template": { "name": "plan", "version": 3 }` (`:1461`) and covered by the topology hash
(`:1478-1479`).

**Skills exist in SQL and nowhere else.** `skill`, `skill_version` and `skill_binding` ship
(`0001_init.sql:406-441` = `docs/ANA-9.md:660-689`), `skill_binding` carries
`pinned_version INTEGER` ("NULL = follow latest"), `position INTEGER NOT NULL DEFAULT 0` and
`UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`, and `docs/ANA-9.md:691-692` fixes
resolution: "project-level bindings of the item's project, overridden per `skill_id` by a binding
whose `phase_id` is the current phase (`R-SKL-2`)". A workspace grep finds no `Skill`,
`SkillVersion` or `SkillBinding` Rust type; the only `skill` symbol is `SkillsTab`, a stub
(`crates/htui/src/ui/tabs/skills.rs`). `MemStore::State` has no skill field.

**`document` is append-only with an open kind set** (`0001_init.sql:389-400`): `kind` is "open set:
phase output kinds plus 'summary'", `UNIQUE (item_id, kind, version)`, `produced_by_step_id`
nullable, `created_at` only. `ReadStore::documents` returns `DocumentHead`, and its body says
"`body` is **deliberately not** in the `SELECT` list" (`crates/htui-store/src/pg/read.rs:230-231`).
There is no document-body reader anywhere in the workspace.

**`item_link` is directed and tombstoned** (`0001_init.sql:357-368`): header comment
"`from --kind--> to; 'blocked_by': from is blocked by to`", `PRIMARY KEY (from_item_id, to_item_id,
kind)`, `kind IN ('blocked_by','origin','relates','supersedes')`, `deleted_at` tombstone,
`idx_item_link_to ... WHERE deleted_at IS NULL`. The shipped `ReadStore::links(id, hops)` is
**undirected, un-kind-filtered and unscoped** in all three backends
(`crates/htui-store/src/pg/read.rs:163-169`, `crates/htui-core/src/store/mem.rs:344-397`,
`crates/htui-store/src/cache/read.rs:312`), and a conformance case pins that behaviour: "the link
traversal of §7.3: live edges only, **both directions**, across projects"
(`crates/htui-core/src/store/conformance.rs:552`). It cannot be narrowed.

**`run_step` already has both columns this document writes** (`0001_init.sql:487-488`):

```sql
    prompt_digest  TEXT,                         -- sha256 of session_event seq 0
    trim_record    JSONB,                        -- R-PRM-3: what was trimmed and by how much
```

Both are mirrored (`cache_migrations/0001_mirror.sql:117`) and modelled
(`crates/htui-core/src/model/run.rs:154-158`). `RunStepSummary` (`run.rs:184-209`) carries neither,
so the Runs tab cannot report a trim today. `trim_record` is `None` on every fixture step;
`prompt_digest` is `Some(PROMPT_DIGEST)` on the `plan` step (`fixtures.rs:1157`) and `None` on the
others (`:1179-1180`, `:1213-1214`).

**The `prompt` payload shape is already committed.** `crates/htui-core/src/fixtures.rs:1237-1249`
ships `{"text", "digest", "sections":[{"name","tokens","trimmed"}]}` with two entries named `prd`
and `skills`, mixing a document kind with a section name. `docs/ANA-9.md:229` says the payload keys
"are the minimum; the driver may add keys, never rename these". `PROMPT_DIGEST`
(`fixtures.rs:1223-1226`) is "a fixed literal, because a real sha256 of a fixture body would move
whenever the body is reworded".

**The budget chain's rows exist and its last link does not.** `step_graph_phase.token_budget` and
`project.settings.token_budget` both exist, the demo project sets `"token_budget": 120_000`
(`fixtures.rs:489`), `app_setting(key TEXT PRIMARY KEY, value JSONB)` exists
(`0001_init.sql:557-561`), and `SEEDED_SETTINGS` is
`[("cache_refresh_seconds", 30), ("cache_overlap_seconds", 300)]`
(`crates/htui-store/src/pg/mod.rs:44-48`). No reader returns a `Project` with its `settings`:
`PgStore::projects` returns `ProjectRef`, which has no settings field
(`crates/htui-core/src/model/hierarchy.rs:111-122`).

**Nothing supports excerpt selection.** `item.touched_paths TEXT[]` exists (`0001_init.sql:323`) and
every demo item writes `Vec::new()` (`fixtures.rs:924`), so ANA-2 §4.7's "backward compatible with
every existing value" is vacuous rather than wrong. `repo_box_path` has zero readers and zero
writers: a workspace grep for `repo_box_path|RepoBoxPath|local_path` returns the type definition,
its re-export, the demo loader's "no such field" note and a table-existence test. Neither
`repo_box_path` nor `workspace_box_path` is mirrored. `globset`, `ignore`, `walkdir`, `git2`, `gix`,
`tree-sitter`, `tiktoken-rs` and every templating crate are absent from `Cargo.lock`;
`similar 2.7.0` and `regex 1.13.1` are present transitively only. ANA-2 §8 refuses a glob crate:
"No glob crate: §4.7's prefix rule is deliberately matcher-free" (`docs/ANA-2.md:1783`).
Two crates a later revision might want **are** already in the lock transitively and would cost a new
*direct* edge rather than a new build-graph node: `unicode-normalization 0.1.25` (§4.7's deferred NFC
rule) and `insta 1.48` (§8's golden prompts).

**`serde_json 1.0.151` is built without `preserve_order`**, so `serde_json::Map` is a `BTreeMap` and
object keys serialise in sorted order. That is free determinism for `trim_record` and for the
`prompt` payload, and §4.7 relies on it.

**Workspace shape.** `Cargo.toml:3` `resolver = "3"`, `:7` `rust-version = "1.85"` (ANA-4 moves it
to 1.88 inside MOD-2), `rust-toolchain.toml` pins 1.98.1, `unsafe_code = "forbid"`,
`clippy::pedantic` deliberately off. `sha2 0.10` is already a workspace dependency. `insta 1.48` is
a dev-dependency of `crates/htui` only; `htui-core`'s only dev-dependency is `tokio`.

---
## 4. Settled questions

### 4.1 Template placeholder contract (`R-PRM-4`, `R-ID-5`)

**The constraint.** `R-PRM-4` (`docs/REQUIREMENTS.md:211-212`), verbatim:

> "Prompt templates per phase are versioned rows in Postgres with a documented placeholder
> contract, editable in the TUI."

`R-ID-5` (`:36-38`) constrains what a template may leave out:

> "Agents run with their own system skills disabled. `htui` owns every prompt and every skill text
> and inlines them into the initial prompt, so behavior is identical on every box and agents never
> spend turns reading instruction files."

**Options for the syntax and the engine.**

| Option | Verdict | Reason |
|---|---|---|
| `minijinja` with `UndefinedBehavior::SemiStrict` | Rejected | Technically the best runtime engine surveyed: four undefined tiers, `add_template_owned` for a body loaded out of Postgres, `AutoEscape::None` by default for extension-less names, fuel and recursion caps, one required dependency (https://docs.rs/minijinja/latest/minijinja/enum.UndefinedBehavior.html). It loses on placement, not on quality. The validator must live where MOD-9 can call it, MOD-9 is not blocked on this document (`HANDOFF.md:79-82`), and MOD-9's editor lives in `crates/htui`, which depends on `htui-core` and `htui-store` and on neither `htui-agent` nor `htui-orch` (`crates/htui/Cargo.toml`). `htui-core` is therefore the only crate both MOD-2 and MOD-9 can reach without a new edge, and putting a template engine behind it is exactly the dependency-weight test ANA-4 §8 applied to the JSON-RPC SDK (`docs/ANA-4.md:1156-1161`). Its strict mode also does not name the offending variable (https://github.com/mitsuhiko/minijinja/issues/871), so `htui` would own the naming pass anyway. |
| `tera` | Rejected | Strict-only with no lenient tier and no way to test an unsupplied name in `{% if %}` (https://github.com/Keats/tera/issues/120); slower on both compile and render; a divergent Jinja dialect. |
| `handlebars-rust` | Rejected | Escapes `&"<>'`=_` by default (https://docs.rs/handlebars/latest/handlebars/), which mangles every markdown fence and every snake_case identifier in a prompt. `register_escape_fn(no_escape)` fixes it, but it is opt-out: forgetting it corrupts every prompt silently and the corruption still digests cleanly. |
| `liquid` | Rejected | An allow-list parser by construction, which is attractive, but no undefined-variable contract is documented on the Rust side (`strict_variables` is a Ruby gem option). `R-PRM-4` demands a documented contract, so an undocumented one is disqualifying. |
| `askama` / `rinja` | Rejected | Compile-time only. Template bodies are Postgres rows edited at runtime, so this is structurally impossible. |
| A general mustache/Jinja-shaped hand parser with `{% if %}` and `{% for %}` | Rejected | Loops and conditionals in a user-editable body mean the section layout, and therefore the trim accounting and the digest, are decided by data the trimmer must reason about. It also invites the Roo Code failure: its `.roo/system-prompt-*` full override was documented under the name "Footgun Prompting" and then removed, because "This feature bypassed safeguards" (https://github.com/RooCodeInc/Roo-Code/pull/11387). |
| **`{{name}}` substitution over a closed, typed, per-role placeholder set, hand-scanned in `htui-core`; every section pre-rendered in Rust; no conditionals and no loops** | **Adopted** | It is the shipped shape: `crates/htui-core/src/fixtures.rs:637` already seeds `{{item}}` into all eight templates of all three demo projects (`TEMPLATE_NAMES`, `PROJECT_SPECS`), so any other syntax makes the seeded fixture bodies invalid on the day the validator lands. No snapshot renders a template body today - `MemStore::State.templates` is `#[expect(dead_code)]` - so the cost is the fixture alone, which is exactly why it is cheap to keep rather than to change. It adds zero dependencies, so it can live in `htui-core` where MOD-9 reaches it. Conditionality is not lost: it is moved into Rust, where a section whose data is absent renders the empty string. That is the same shape Cline ships in production, a skeleton of `{{TOOL_USE_SECTION}}`-style markers with per-model component overrides (https://github.com/cline/cline/tree/main/src/core/prompts/system-prompt), and the same shape Claude Code's `$ARGUMENTS` uses with a fully documented degenerate case (https://code.claude.com/docs/en/skills). |

**Options for the unknown-placeholder policy.**

| Option | Verdict | Reason |
|---|---|---|
| Unknown token passes through as literal text | Rejected | It makes the "documented contract" unenforceable and ships a prompt containing `{{itme}}` to a model. |
| Unknown token renders empty | Rejected | Silent omission is exactly what ANA-2 refused for a missing input document: "a phase silently running without its plan produces an artefact that looks valid and is not" (`docs/ANA-2.md:415-416`). |
| **Rejected at save with the token and its byte offset; a hard step failure at stage 3 if a row reaches the assembler anyway** | **Adopted** | Two gates, because there are two ways a body arrives: MOD-9's editor, and a row written by a future `htui` or by hand. The stage-3 failure mirrors ANA-2's missing-input rule verbatim in shape, so the two failure modes read the same in the Runs tab. |

**Verdict: `{{name}}`, a closed per-role set, pre-rendered sections, no control flow.**

*Syntax.* The scanner walks the body left to right over `char_indices`:

- `{{{{` emits a literal `{{` and advances four bytes. This is the only escape.
- `{{` followed by `[a-z][a-z0-9_]*` followed by `}}` is a placeholder.
- `{{` in any other position is a **validation error**, not literal text.
- `}}` outside a placeholder is literal text.
- Placeholder names are lowercase ASCII with underscores. There is no whitespace tolerance inside
  the braces: `{{ item }}` is an error, so one body has exactly one spelling and the digest cannot
  drift on whitespace inside a marker.

*The closed placeholder set.* Three roles, three sets. A `phase` template may use the phase set; a
`judge` template the judge set; a `handoff` template the handoff set. `TemplateRole` is derived from
the row's `name`: `judge` and `handoff` are reserved names (§4.6), everything else is a phase
template.

Phase set, in the order §4.2 renders them:

| Placeholder | Type | Source | Renders empty when |
|---|---|---|---|
| `{{item_key}}` | scalar | `project.slug` + `:` + `item.key` | never |
| `{{item_title}}` | scalar | `item.title`, newlines collapsed to spaces | never |
| `{{item_kind}}` | scalar | `item_kind.name` of the item's kind | never |
| `{{phase}}` | scalar | `ResolvedPhase.name` | never |
| `{{output_kind}}` | scalar | `ResolvedPhase.output_kind` | the phase declares none |
| `{{attempt}}` | scalar | `run_step.attempt`, 1-based per `docs/ANA-2.md:1939-1941` | never |
| `{{item}}` | section | `item.body` | never; an empty body still emits the section with an empty content block, because "this item has no body" is information |
| `{{documents}}` | section | the `input_kinds` resolution of `docs/ANA-2.md:395-406`, one block per kind in `input_kinds` order | `input_kinds` is empty |
| `{{upstream}}` | section | §4.3's walk | the walk returns no rows |
| `{{box}}` | section | §4.2's box profile projection | never |
| `{{skills}}` | section | `R-SKL-2` resolution, ordered by `skill_binding.position` | no binding resolves |
| `{{excerpts}}` | section | §4.5 | no excerpt survives selection or the budget |
| `{{verify_failure}}` | section | the previous attempt's `verify_exit_code` and stored `command_run.output` (`docs/ANA-2.md:730`) | `attempt == 1`, or the previous attempt had no verification output |
| `{{previous_diff}}` | section | the previous attempt's `before_hash..after_hash` (`docs/ANA-2.md:731`) | `attempt == 1`, or the previous attempt produced no commit |
| `{{command_queue}}` | section | the `R-MCP-4` note telling the agent to route heavy commands through `command_run` | the resolved `command_queue` is `off`, or is `fan_out_only` and neither `fan_out > 1` nor the item's `heavy_build` tag applies (`docs/ANA-2.md:496-500`) |

Judge set: `{{item_key}}`, `{{phase}}`, `{{task}}`, `{{candidates}}`.
Handoff set: `{{item_key}}`, `{{phase}}`, `{{attempt}}`, `{{step_summary}}`, `{{documents}}`,
`{{diff_so_far}}`, `{{failure_reason}}`.

*Why no conditionals.* Three cases genuinely need conditionality and all three are handled by the
empty-section rule rather than by syntax: a position-0 phase has `input_kinds = []`
(`crates/htui-core/src/fixtures.rs:611-614`), a root item has no upstream links, and `command_run`
exposure is conditional per `R-MCP-4`. In each case the placeholder resolves to the empty string,
the surrounding blank lines collapse (§4.7), and no `sections[]` entry is emitted. Headings live in
the renderer, not in the template, precisely so an absent section takes its heading with it.

*Type sketch.* Design notation, not code.

```rust
// crates/htui-core/src/prompt/template.rs

/// Which closed set a body's placeholders are checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateRole { Phase, Judge, Handoff }

/// Every placeholder htui understands. Adding a variant is a contract change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Placeholder {
    ItemKey, ItemTitle, ItemKind, Phase, OutputKind, Attempt,        // scalars
    Item, Documents, Upstream, Box, Skills, Excerpts,                // phase sections
    VerifyFailure, PreviousDiff, CommandQueue,                       // review-loop / MCP sections
    Task, Candidates,                                                // judge only
    StepSummary, DiffSoFar, FailureReason,                           // handoff only
}

impl Placeholder {
    pub const fn token(self) -> &'static str;                        // "item", "verify_failure", ...
    pub const fn allowed_in(self, role: TemplateRole) -> bool;
    pub const fn is_section(self) -> bool;
}

/// A parsed body: literal spans and placeholder spans, in source order.
#[derive(Debug, Clone)]
pub struct ParsedTemplate { pub spans: Vec<Span>, pub used: Vec<Placeholder> }

#[derive(Debug, Clone)]
pub enum Span { Literal(String), Slot(Placeholder) }

/// The save-time gate MOD-9 calls. Byte offsets so the editor can put the cursor on the error.
pub fn parse(role: TemplateRole, body: &str) -> Result<ParsedTemplate, TemplateError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("unknown prompt placeholder `{{{{{token}}}}}` at byte {at}")]
    UnknownPlaceholder { token: String, at: usize },
    #[error("placeholder `{{{{{token}}}}}` is not available to a {role:?} template, at byte {at}")]
    WrongRole { token: String, role: TemplateRole, at: usize },
    #[error("unterminated `{{{{` at byte {at}")]
    Unterminated { at: usize },
    #[error("a {role:?} template must use {token}")]
    MissingRequired { role: TemplateRole, token: &'static str },
}
```

*Validation on save (MOD-9).* `parse(role, body)` runs on every save, before the row is written.
Four checks, in order: syntax, closed-set membership, role membership, and required placeholders.
A `judge` body must contain `{{candidates}}`; a `handoff` body must contain `{{step_summary}}` and
`{{failure_reason}}`; a phase body has no required placeholder, because a phase whose prompt is pure
instruction is legitimate. The editor also **warns**, without refusing, when a phase body omits
`{{item}}`, because that is almost always a mistake and never illegal. Duplicate placeholders are
legal and each occurrence substitutes; §4.4 counts a duplicated section's tokens once per occurrence
and §4.7's ordering is source order, so nothing about duplication is undefined.

*Version pinning per phase.* Unchanged from ANA-2 §4.1 and restated so MOD-2 does not re-derive it:
`template_name` defaults to the phase name, `template_version` is the phase's pin and NULL means the
latest version in the project, and both are resolved **at snapshot time** into
`graph_snapshot.phases[].template` (`docs/ANA-2.md:285`, `:321-323`, `:1461`). The assembler for a
graph run takes `TemplateRef` off the snapshot and never resolves `latest`. This document adds the
rule for the two name-keyed templates, which have no phase to hang a pin on: `judge` pins alongside
the judged phase in the same snapshot entry as `judge_template: TemplateRef`, resolved by the same
chain at snapshot time; `handoff` resolves to the latest version **at promotion time**, because a
handoff is a human action taken after the run started and the maintainer expects the edit they just
made to take effect. That asymmetry is deliberate and is recorded in `trim_record.template`.

*Free-standing chat prompts.* `R-STO-4` starts no graph run offline, and `prompt_template`,
`skill*` and `step_graph*` are not mirrored (`docs/ANA-9.md:309-311`), so a graph step never needs a
template offline. The one exception is MOD-2's free-standing chat, whose prompt is the user's typed
text with no template, no sections, no trim record and no digest obligation beyond ANA-4's own. It
is out of this document's scope and is named here so the gap is not read as an oversight.

*What happens when a template names a placeholder the phase cannot supply.* Three distinct cases,
deliberately given three different answers:

| Case | Answer |
|---|---|
| A section placeholder whose data is legitimately absent (`{{documents}}` on a position-0 phase, `{{upstream}}` on a root item, `{{previous_diff}}` on attempt 1) | Renders the empty string. No section, no `sections[]` entry, no error. This is the normal path and is why no conditional syntax is needed. |
| A **required input document** named in `input_kinds` that does not resolve | Unchanged from ANA-2: the step fails at stage 3, before a token is spent, with `run.failure = "missing input document: <kind>"`, the run parks and the item goes to `blocked` (`docs/ANA-2.md:413-416`). This is not a template question and this document does not touch it. |
| A placeholder outside the closed set, or outside the role's set, in a body the assembler is asked to render | Hard failure at stage 3, symmetrically: `run.failure = "unknown prompt placeholder: {{foo}}"`, step `failed`, item `blocked`. It is reachable only when a row bypassed MOD-9's validator, which is a bug, and a bug that silently produces a degraded prompt is worse than one that stops. |

---

### 4.2 Section model of the assembled prompt (`R-PRM-1`, `R-ID-5`)

**The constraint.** `R-PRM-1` (`docs/REQUIREMENTS.md:202-206`), verbatim:

> "Each step receives one self-contained initial prompt built by `htui`: phase template, item body,
> input documents from prior steps, summaries of upstream items reached by `blocked_by` and
> `origin` edges within one to two hops and inside the active workspace (or current project when no
> workspace), box profile, bound skill text, and any `htui`-selected file excerpts. Never raw
> transcripts of other items."

**Options for the render order.**

| Option | Verdict | Reason |
|---|---|---|
| Bulk first, instruction last, ignoring `R-PRM-1`'s order | Rejected as a whole-prompt rule | The guidance is real and quantified: "Place your long documents and inputs near the top of your prompt, above your query, instructions, and examples", and "Queries at the end can improve response quality by up to 30 percent in tests" (https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices). But `docs/REQUIREMENTS.md` is the contract and its list is an order. Overriding it wholesale is a requirement change. |
| `R-PRM-1`'s literal order, template first, with the instruction inside the template at the top | Rejected | It obeys the requirement and throws away the measured gain for nothing. |
| **`R-PRM-1`'s literal order for the sections, with the template as the frame rather than a leading section, so the default bodies of §5.4 open with one line of role and close with the instruction block** | **Adopted** | The template is not a section that sits before the others; it is the document the others are substituted into, so `R-PRM-1`'s ordering of *sections* is preserved exactly while the instruction lands last, where the evidence wants it. It also gives the byte-stable prefix prompt caching rewards: the frame's opening line, the skills and the box profile are the sections that change least, and "The cache key is derived from the exact bytes of the rendered prompt" (https://github.com/anthropics/skills/blob/main/skills/claude-api/shared/prompt-caching.md). |

**Options for the section delimiter.**

| Option | Verdict | Reason |
|---|---|---|
| Markdown headings (`## Item body`) | Rejected | Item bodies, documents and excerpts all contain markdown headings, so the boundary is not distinguishable from content. |
| Triple-backtick fences as the section boundary | Rejected | Excerpts and diffs contain fences. |
| DSPy-style `[[ ## name ## ]]` field markers | Rejected as the section boundary, adopted as the reasoning | The rationale is the right one: "chosen for low collision and clean regex. The brackets-plus-hashes shape is unlikely to appear in real text or code" (https://dspy.ai/diving-deeper/adapters/). But it has no closing form, so a section's end is implicit. |
| **`<section name="...">` ... `</section>`, one uniform tag, name and metadata as attributes** | **Adopted** | Anthropic's own guidance is explicit: "When using multiple documents, wrap each document in `<document>` tags with `<document_content>` and `<source>` (and other metadata) subtags for clarity" and "XML tags help Claude parse complex prompts unambiguously". One tag name keeps the rendered text uniform, the closing tag makes every boundary explicit, and `htui` never parses the prompt back so a body containing a literal `<section>` line is inert. |

**Verdict: ten named step-prompt sections plus five role-specific ones, one wrapper, one closed name
vocabulary.**

*The ordered section table.* The table lists the sections in `R-PRM-1`'s canonical order, which is
the order §5.4's default bodies place their placeholders in, except that the two review-loop sections
sit where the `implement` and `fix` bodies put them. The *rendered* order of any one prompt is the
order the placeholders appear in that template body (§4.7 rule 1), because the template is the frame;
`sections[]` follows the same order. Trim rank 0 means protected: the section never loses a token.
Ranks count upward in the order sections are trimmed, so rank 1 is trimmed first (§4.4 argues the
direction).

| Section name | Source | Rendering | Trim strategy | Trim rank |
|---|---|---|---|---|
| `template` | the literal spans of the parsed body | inline; no wrapper. It is the frame. | none | 0 (protected) |
| `item` | `item.body` | `<section name="item" key="htui:MOD-2" title="...">` then the body verbatim | head+tail | 4 |
| `documents:<kind>` | one per resolved `input_kinds` entry, in `input_kinds` order | `<section name="documents:plan" kind="plan" version="3">` then the body verbatim | drop whole documents, oldest kind position last, then head+tail the survivors | 3 |
| `upstream` | §4.3 | `<section name="upstream">` then one block or one stub line per item | the depth ladder of §4.3 | 2 |
| `box` | `box` row plus `box_tool` rows | `<section name="box">` then the fixed field list below | none | 0 (protected) |
| `skills` | `R-SKL-2` resolution | `<section name="skills">` then one `<skill name=".." version="N">` block per binding | none; an aggregate cap refuses the step instead | 0 (protected) |
| `excerpts` | §4.5 | `<section name="excerpts">` then one `<file>` block per excerpt | budget-derived, not trimmed: drop whole files lowest rank first | 1 |
| `verify_failure` | previous attempt's verification output | `<section name="verify_failure" exit_code="1">` then a fence | tail-cut, keeping the tail | 3 |
| `previous_diff` | previous attempt's `before_hash..after_hash` | `<section name="previous_diff" range="abc1234..def5678">` then a diff stat, then a fenced unified diff | full diff, then diff stat only, then dropped | 2 |
| `command_queue` | `R-MCP-4` exposure | `<section name="command_queue">` then two sentences | none | 0 (protected) |
| `judge_task` | the judged step's stored `prompt` text | `<section name="judge_task">` then the text verbatim | head+tail | 2 (judge prompt only) |
| `judge_candidate:<i>` | one per surviving candidate | `<section name="judge_candidate:0" fanout_index="0" verify="pass" exit_code="0">` then document body, diff stat, diff, verification tail | per-candidate isolate cap, then diff-stat-only | 1 (judge prompt only) |
| `step_summary` | the step's own stored events, deterministically summarised | `<section name="step_summary" turns="3">` | head+tail | 2 (handoff only) |
| `diff_so_far` | this step's `before_hash..HEAD` | as `previous_diff` | full, then stat, then dropped | 1 (handoff only) |
| `failure_reason` | `run.failure` or the gate note | `<section name="failure_reason">` one paragraph | none | 0 (handoff only) |

*Content rendering rules, uniform across sections.*

1. The opening tag is on its own line; the closing tag is on its own line; there is exactly one LF
   between the opening tag and the first content byte and exactly one between the last content byte
   and the closing tag.
2. Content that is prose or markdown is emitted verbatim, with no re-indentation, no trailing
   whitespace stripping and no reflow. Modifying content bytes would change meaning in
   whitespace-sensitive languages and would make the digest a function of a cosmetic rule.
3. Content that is code, a diff or command output is wrapped in a fence whose length is one more
   backtick than the longest backtick run in the content, minimum three. This is Aider's
   collision-avoidance rule (`aider/coders/base_coder.py:609-633`) and it is what stops an excerpt
   containing a fence from ending its own section early.
4. Attribute values are attribute-escaped for `"` and `&` only, and any newline in an attribute
   value is collapsed to a space. An item title is truncated to 120 bytes on a character boundary
   with a trailing `...` when longer.
5. Absolute filesystem paths never appear inside any section. Paths are `<repo_slug>:<repo-relative
   path>`. This is what makes invariant 3 hold across fan-out siblings whose trees differ only by
   `<config_dir>/trees/<run_id>/<step_id>/`.

*The box profile projection.* `R-PRM-1` says only "box profile" and the term is undefined anywhere
else; `box.settings` is orchestrator policy (`max_concurrent_items`, `command_limits`) with no place
in a prompt, and `probed_tags` / `declared_tags` are `R-ORCH-10` matching vocabulary rather than a
machine description. The projection is fixed here, closed, and must agree with the `box_profile`
read tool `R-MCP-2` gives agents (MOD-11 consumes this list):

```
<section name="box">
hostname: dev-win-01
os: windows 10.0.26200 (x86_64)
cpu: AMD Ryzen 9 7950X, 32 threads
ram: 65536 MB
gpu: nvidia
htui: 0.4.1
tools: cargo 1.98.1, git 2.47.0, node 22.12.0, python 3.13.1, rustc 1.98.1
quirks: MSVC toolchain only; no WSL; long paths enabled.
</section>
```

Rules: `tools` is the `box_tool` rows sorted by `name` byte order, rendered `name version`, capped
at 24 entries with a trailing `, +N more`. `box_tool.version` is `TEXT NOT NULL`
(`0001_init.sql:84`), so there is no null case; an empty string renders the name bare. `path` is
never rendered, because it is an absolute path (rule 5) and because the agent invokes tools by name.
`ram` is omitted when `box.ram_mb` is NULL, and `gpu` is omitted when `gpu_present` is false or
`gpu_vendor` is NULL, since both are nullable (`crates/htui-core/src/model/box_.rs:41-46`).
`quirks` is omitted when empty and is otherwise
emitted verbatim with newlines collapsed to `; `. Tags and settings are deliberately excluded.
**Open for the maintainer 5** carries the field list.

*The skills section and its cap.* Bindings resolve per `docs/ANA-9.md:691-692`: project-level
bindings of the item's project, overridden per `skill_id` by a binding whose `phase_id` is the
current phase. Within the resolved set, order is `skill_binding.position` ascending, then
`skill.name` byte ascending as the tie-break, and each skill appears exactly once even when both a
project and a phase binding exist, which is the dedup OpenHands had to add after a reviewer asked
"whether the microagent prompt would be included twice"
(https://github.com/OpenHands/OpenHands/pull/7516). Version is `pinned_version`, or the latest
`skill_version` when NULL. Skills are never trimmed (invariant 4), but they are not unbounded:
`app_setting.max_skill_tokens` (default 20 000) caps the aggregate, and exceeding it **refuses the
step** at stage 3 with `skills exceed max_skill_tokens (N > M)` rather than dropping a binding.
No surveyed system caps inlined instruction text, and Gemini CLI documents the resulting failure -
"The dominant failure mode is context bloat" (https://geminicli.com/docs/cli/gemini-md/) - so this
is one place where `htui` deliberately exceeds prior art, and it refuses rather than degrades
because a silently dropped skill breaks `R-ID-5`'s identical-behaviour promise.

*The `sections[]` array.* `docs/ANA-9.md:233` fixes three keys per entry and permits more. This
document fixes the vocabulary and the semantics:

```rust
// serialised into the `prompt` event payload, and into trim_record in a wider form
pub struct SectionEntry {
    pub name: String,   // the closed vocabulary above; documents and judge candidates are suffixed
    pub tokens: i64,    // estimated tokens AFTER trimming, by the estimator named in trim_record
    pub trimmed: bool,  // true when the section lost content, including when it was dropped whole
}
```

Rules that make it machine-readable: an entry exists for every section that contributed **at least
one byte** to the prompt, plus an entry with `tokens: 0, trimmed: true` for every section that was
selected and then dropped entirely by the trimmer, because "this was dropped" is the fact a reader
needs. A section that never had data emits no entry at all. Entries are in **render order, which is
the order the placeholders appear in the rendered template body**, not the order of the table above:
the table is `R-PRM-1`'s canonical listing and a body may legitimately depart from it, as
`implement` and `fix` do by putting `verify_failure` and `previous_diff` directly after
`{{documents}}`. The array therefore reads top to bottom like the prompt it describes. `template` is
always the first entry, by convention, because its literal spans surround every other section.
The shipped fixture's bare `prd` (`fixtures.rs:1244-1247`) predates this contract
and becomes `documents:prd`; `skills` is already canonical. MOD-2 updates the fixture.

*Raw transcripts.* `R-PRM-1`'s prohibition is about **other items**. Three places in this design
touch transcript-derived text and each is accounted for:

| Where | Status |
|---|---|
| the review loop's `verify_failure` | Not a transcript. It is `command_run.output` of the previous attempt, which is command output, and ANA-2 assigns it to this document explicitly (`docs/ANA-2.md:730`). |
| the judge's `judge_task` | Not a transcript. It replays the candidate step's own `prompt` event, which is `htui`-authored text, not agent output. ANA-2 is explicit that the judge "does not receive any candidate's session transcript" (`docs/ANA-2.md:831`), and the `candidates` section carries documents and diffs only. |
| the handoff's `step_summary` | The same item and the same step, so `R-PRM-1`'s "other items" does not reach it, and ANA-2 asks for it by name: "ANA-5 builds a handoff prompt from the step's own stored transcript" (`docs/ANA-2.md:1203`). It is nonetheless rendered as a deterministic summary rather than a replay (§4.6), so `R-PRM-1`'s spirit survives even where its letter does not bind. |

Enforced by type: `PromptSpec` (§8) has no `Vec<SessionEvent>` field. The handoff builder takes a
`StepSummary` value that the handoff path constructs from the step's own events, and the assembler
cannot reach an event stream at all.

---
### 4.3 The upstream-summary walk (`R-PRM-1`, `R-PRM-2`)

**The constraint.** `R-PRM-1` (`docs/REQUIREMENTS.md:203-205`) requires "summaries of upstream items
reached by `blocked_by` and `origin` edges within one to two hops and inside the active workspace
(or current project when no workspace)". `R-PRM-2` (`:207`), verbatim:

> "Upstream items outside the active workspace appear as one-line status stubs."

**Options for the traversal.**

| Option | Verdict | Reason |
|---|---|---|
| Reuse `ReadStore::links(id, hops)` | Rejected | It is undirected, un-kind-filtered and unscoped in all three backends, and a conformance case pins that behaviour as "live edges only, both directions, across projects" (`crates/htui-core/src/store/conformance.rs:552`). The Graph tab (MOD-14) depends on exactly that shape, so narrowing it would break a shipped contract to save one method. |
| Walk the graph in Rust over repeated `links` calls and filter | Rejected | N round trips, and the filtering would still have to re-derive scope and the latest-summary lookup that `docs/ANA-9.md` §7.3 already expresses in one query. |
| **One new store method implementing an amended §7.3, on `ReadStore`** | **Adopted** | Every table §7.3 touches is mirrored: `item_link`, `item`, `project`, `document` with its body (`cache_migrations/0001_mirror.sql:97-101`) and `workspace_project` (`:52-55`). So the mirror can answer it, and ANA-2 §8's split rule (`docs/ANA-2.md:1699-1705`) puts it on `ReadStore` rather than making it a `PgStore` inherent read. |

**Options for the hop count.**

| Option | Verdict | Reason |
|---|---|---|
| Fixed 1 | Rejected as the default | The strongest published evidence is Sweep's: expanding a file/entity graph "by one degree" gives "nearly perfect recall for planning, as every function that might be remotely relevant will be retrieved, but the precision is really bad" (https://github.com/sweepai/sweep/blob/main/docs/pages/blogs/ai-code-planning.mdx). That argues for restraint, but it is about code entities, not a hand-curated dependency graph of at most a few edges per item. |
| Fixed 2, not configurable | Rejected | `R-PRM-1` says "one to two hops", so one must remain expressible, and a project whose graph is dense wants one. |
| A new `step_graph_phase` column | Rejected | It is a per-project taste, not a per-phase mechanism, and it would cost a DDL change this document otherwise does not need. |
| **Default 2, resolved through `project.settings.upstream_hops`, then `app_setting.prompt_upstream_hops` (default 2)** | **Adopted** | Two matches both superseded predecessors, whose CTEs hardcode `WHERE ug.depth < 2` (`docs/ANA-8.md:237`, stated in prose at `:208`; `docs/ANA-1.md:371`, stated at `:344`), and `R-PRM-1` permits it. Precision is protected structurally rather than by truncating the walk: §4.4 trims upstream before item body and documents, and the trim ladder degrades depth-2 entries to stubs before it touches depth 1, so a dense graph self-corrects under budget pressure instead of losing the near neighbours. A setting rather than a column matches how ANA-2 handled every other tunable (`docs/ANA-2.md:1509-1522`). |

**Options for the "in scope, no summary yet" case.**

| Option | Verdict | Reason |
|---|---|---|
| Reuse the `R-PRM-2` stub | Rejected | `docs/ANA-9.md:933` does exactly this today and it conflates two different facts. "This item is outside your workspace, here is its state" and "this item is in your workspace and nobody has written its summary" call for different agent behaviour: the second is a thing the agent can go and read documents about. |
| Fall back to the item body | Rejected | An item body is a specification, not a result, and can be as long as the summary it stands in for. It would also make the upstream section the largest in the prompt for a graph of open items, which is exactly backwards. |
| Fall back to the latest document of any kind | Rejected | Non-deterministic in a way the digest cannot tolerate: the "latest" kind changes as the upstream run progresses, so the same prompt inputs give different bytes on different days for reasons unrelated to this item. |
| **A third render state: the stub line plus an explicit `no summary yet` marker** | **Adopted** | It is one word of cost and it names the fact. It is also the common case, not the edge case, because `document.kind = 'summary'` is written only at close-out (`docs/ANA-2.md:1380-1391`) and `done` is not `closed`. |

**Verdict: an amended §7.3, one `ReadStore` method, three render states, deduped and re-ordered in
Rust.**

*The amended query.* Three changes to `docs/ANA-9.md:916-942`, none of which is DDL:

1. project `i.id` and `up.depth`, and add `s.project_id IS NOT NULL AS in_scope`, so the caller can
   distinguish scope from summary presence.
2. Move the scope test off the `LEFT JOIN LATERAL`'s `ON` clause, so an in-scope item with no
   summary and an out-of-scope item with a summary are separable facts rather than one NULL.
3. Dedup to one row per item at the minimum depth. The shipped `UNION` dedups whole rows, that is
   `(item_id, depth)` pairs, so an item reachable at depth 1 and depth 2 through a diamond yields
   two rows with the same `qualified_key` and `ORDER BY up.depth, qualified_key` emits both. That
   duplicates a summary in the prompt and makes the digest a function of edge insertion order.

```sql
-- ANA-5 amendment to docs/ANA-9.md 7.3. Query change only; no DDL.
WITH RECURSIVE up AS (
    SELECT l.to_item_id AS item_id, 1 AS depth
      FROM item_link l
     WHERE l.from_item_id = $item AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL
    UNION
    SELECT l.to_item_id, up.depth + 1
      FROM item_link l JOIN up ON l.from_item_id = up.item_id
     WHERE up.depth < $hops AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL
),
best AS (                                          -- ANA-5: one row per item, nearest hop wins
    SELECT item_id, MIN(depth) AS depth FROM up GROUP BY item_id
),
scope AS (                                         -- active workspace, or the current project
    SELECT project_id FROM workspace_project WHERE workspace_id = $workspace
    UNION SELECT $project WHERE $workspace IS NULL
)
SELECT i.id                          AS item_id,
       p.slug || ':' || i.key        AS qualified_key,
       i.title,
       i.status,
       best.depth,
       (s.project_id IS NOT NULL)    AS in_scope,  -- ANA-5: separable from `summary IS NULL`
       CASE WHEN s.project_id IS NOT NULL THEN d.body END AS summary
  FROM best
  JOIN item i    ON i.id = best.item_id
  JOIN project p ON p.id = i.project_id
  LEFT JOIN scope s ON s.project_id = i.project_id
  LEFT JOIN LATERAL (
       SELECT body FROM document WHERE item_id = i.id AND kind = 'summary'
        ORDER BY version DESC LIMIT 1) d ON TRUE   -- ANA-5: scope no longer gates the lookup
 ORDER BY best.depth, qualified_key, item_id;
```

The `ORDER BY` stays in SQL for readability, and the caller **re-sorts in Rust by byte order**
before rendering. Postgres orders text by the database collation while the SQLite mirror orders by
byte value. ANA-9 makes the "identical text" promise explicitly only for §7.5's replay query
(`docs/ANA-9.md:969`); this document extends the same expectation to §7.3, because without the Rust
re-sort the same item on the same box would digest differently online and offline. The
tombstone filter is a no-op on the mirror, which deletes tombstoned rows rather than mirroring
`deleted_at` (`cache_migrations/0001_mirror.sql:16-18`), so the two backends agree by construction.

*The algorithm, as numbered steps.*

1. Resolve the bound. `Scope::from_workspace` when a workspace is active, otherwise the item's own
   `project_id` alone (`docs/REQUIREMENTS.md:68-70`). **The shipped `Scope` cannot express the
   second case**: `Scope { workspace_id: WorkspaceId, project_ids: Vec<ProjectId> }` documents
   itself as "always one workspace, never a bare project list" and `from_workspace` is its only
   constructor (`crates/htui-core/src/model/scope.rs:8-27`), while `R-ENT-2` says "there are no
   implicit workspace rows". MOD-2 therefore adds either a `Scope::single_project(ProjectId)`
   constructor or a `PromptScope { workspace: Option<WorkspaceId>, project: ProjectId }` argument;
   the SQL already takes `$workspace` and `$project` separately and needs no change either way.
   **Unverified - MOD-2 must confirm** which of the two MOD-1's `Scope` consumers tolerate.
2. Resolve `hops`: `project.settings.upstream_hops`, then `app_setting.prompt_upstream_hops`,
   default 2. Clamp to `1..=2`; a stored value outside that range is clamped and noted in
   `trim_record.notes`, not an error.
3. Run the amended query with `$item`, `$hops`, `$workspace`, `$project`.
4. Sort the rows by `(depth, qualified_key.as_bytes(), item_id)`.
5. Classify each row: `in_scope && summary.is_some()` is **Summary**; `in_scope && summary.is_none()`
   is **Pending**; `!in_scope` is **Stub**.
6. Render in that sorted order, Summary rows as blocks and Pending/Stub rows as single lines, with
   all Summary blocks first and the single lines gathered at the end of the section under one
   `also upstream:` line. Gathering the one-liners is what keeps the section readable when a graph
   has one summarised parent and nine stubs, and it costs nothing in determinism because the two
   groups are each internally ordered.
7. If the section is empty, render nothing and emit no `sections[]` entry.

*The exact renders.*

```
<section name="upstream">
### htui:ANA-9 - Postgres schema (closed, 1 hop)
<summary body, verbatim>

### workspace:MOD-11 - Concept docs rule (closed, 2 hops)
<summary body, verbatim>

also upstream:
- htui:MOD-7 - Box registry and capabilities (in_progress) - no summary yet
- auth-service:MOD-1 - Token rotation (done)
</section>
```

The two one-line forms, byte-exact:

```
- {qualified_key} - {title} ({status}) - no summary yet
- {qualified_key} - {title} ({status})
```

`{qualified_key}` is `project.slug` + `:` + `item.key`, which the query already builds.
`{title}` has every `\r` and `\n` collapsed to a single space, runs of spaces collapsed to one, and
is truncated to 100 bytes on a character boundary with a trailing `...`. `{status}` is the raw
`item.status` value, one of the eight `R-ENT-8` strings, so a reader can map it to the Backlog
without a legend. The `- no summary yet` suffix appears only on the Pending form. The shape follows
the only convention the field offers, Linear's relation pill of identifier, title and state
(https://linear.app/docs/issue-relations), and the superseded ANA-8's own
`[External Prerequisite: auth-service:MOD-1 (status: done)]` (`docs/ANA-8.md:211-212`), simplified
because the bracketed label repeats what the section heading already says.

Summary blocks carry `### {qualified_key} - {title} ({status}, {depth} hop)` with `hop` pluralised
to `hops` at 2, then a blank line, then the summary body verbatim. Title treatment is the same. The
body is untrusted text from another item's close-out and may contain anything including a
`</section>` line; that is inert, because nothing parses the prompt back, and it is why the
delimiter choice of §4.2 does not need to be collision-proof in the way a fence does.

*The store-trait method.* One method, one round trip, on `ReadStore`:

```rust
// crates/htui-core/src/store/traits.rs, ReadStore
/// Upstream items over `blocked_by` and `origin` edges, up to `hops` hops, classified against
/// `scope` (R-PRM-1, R-PRM-2). Implements ANA-9 7.3 as amended by ANA-5 4.3.
async fn upstream_summaries(
    &self,
    id: ItemId,
    hops: u8,
    scope: &Scope,
) -> Result<Vec<UpstreamEntry>>;

// crates/htui-core/src/model/link.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamEntry {
    pub item_id: ItemId,
    pub qualified_key: String,   // "<project.slug>:<item.key>"
    pub title: String,
    pub status: Status,
    pub depth: u8,               // 1 or 2, the minimum over all paths
    pub in_scope: bool,          // false => the R-PRM-2 stub
    pub summary: Option<String>, // Some only when in_scope and a summary document exists
}
```

`MemStore` implements it over its existing in-memory `item_link` and `document` vectors with the
same dedup and the same classification, and it gets a conformance case (§8). The three backends must
agree on a diamond, on a self-referencing chain (impossible by `CHECK (from_item_id <> to_item_id)`
but reachable as a two-edge cycle), and on the tombstone filter.

*Cycles.* `item_link` forbids a self-edge but permits `A blocked_by B` and `B blocked_by A`. The
recursive term is bounded by `up.depth < $hops` with `$hops <= 2`, so a cycle terminates; `best`
then collapses the repeat visits. No `CYCLE` clause is needed and none is added.

*Risk 10's homeless outputs.* `docs/ANA-2.md:2073` records that close-out's "live coordinates" have
no database home and that "`R-PRM-1`'s upstream-summary walk is the only channel that would surface
them". This document does not solve that: the walk renders `document.kind = 'summary'` bodies
verbatim, so a close-out that writes its live coordinates **into the summary body** surfaces them
for free, and one that does not, does not. That is the whole of the answer available without a new
column, and it is stated so a later ANA can find it.

---

### 4.4 Token budget, the trim order and the trim record (`R-PRM-3`)

**The constraint.** `R-PRM-3` (`docs/REQUIREMENTS.md:208-210`), verbatim:

> "Per-step token budget; oversize inputs are trimmed by a fixed priority order (template and skills
> first, then item body, then prior documents, then upstream summaries, then excerpts) and the trim
> is recorded on the step."

**Options for counting tokens.**

| Option | Verdict | Reason |
|---|---|---|
| Anthropic's `POST /v1/messages/count_tokens` | Rejected | Authoritative and free, but it is a network call needing an API key on the executing box, which sits against `R-NF-2` ("No dependency on any external daemon other than Postgres and the agents"), and it is per-model-id so assembly would call once per family per step. Its own documentation calls the result "an estimate" and notes counts "may include tokens added automatically by Anthropic for system optimizations" (https://platform.claude.com/docs/en/build-with-claude/token-counting). |
| `tiktoken-rs` | Rejected | OpenAI BPE only, 3.79 MB of embedded tables, and explicitly out of scope for non-OpenAI models (https://github.com/zurawiki/tiktoken-rs). Using `cl100k_base` as a Claude proxy is a documented hack, not a measurement. |
| HuggingFace `tokenizers` with a local `tokenizer.json` | Rejected | Purely local and exact, but Anthropic publishes no `tokenizer.json`, so it would cover neither of the two seeded agents' primary models. |
| A community-reconstructed Claude tokenizer | Rejected | Unofficial, unversioned against the models it claims to reconstruct, and it would put a correctness-critical third-party table behind `htui-core`. |
| **A versioned deterministic heuristic estimator, per agent family, with fixed constants in v1** | **Adopted** | No transport gives an exact pre-flight count and the primary transport gives no post-hoc ground truth either: ACP's `usage_update` is "context-window occupancy and cumulative session cost, not per-turn token deltas", the agent only MAY send it, and ACP sessions persist all four token fields as null (`docs/ANA-4.md:161-166`, `:1094-1098`). An exact tokenizer would therefore buy precision against a number nothing in the primary path validates. The largest framework in the space recommends the same on the hot path: `count_tokens_approximately` "is recommended for using trim_messages on the hot path, where exact token counting is not necessary" (https://reference.langchain.com/python/langchain-core/messages/utils/trim_messages). Aider ships estimates and states plainly "The token counts that aider reports are estimates" (https://aider.chat/docs/troubleshooting/token-limits.html), and its own repo-map sizing samples every hundredth line above 200 characters rather than counting exactly (`aider/repomap.py:90-101`). |

**Options for reading "first" in `R-PRM-3`.**

| Option | Verdict | Reason |
|---|---|---|
| Trimmed first: the template and skills lose tokens before anything else | Rejected | It is the literal English and it is incoherent as a design: it would delete the phase instructions before deleting a file excerpt. It also contradicts `R-ID-5`'s stated rationale directly, because a skill that shrinks under pressure makes "behavior identical on every box" false. |
| **Kept first: the list is a keep-priority, template and skills are protected, and trimming proceeds from the tail of the list backwards** | **Adopted** | Three independent supports. Internal: `R-PRM-3`'s list is in the same order as `R-PRM-1`'s section list, which is a contents ordering running most-essential to least. Requirement-level: `R-ID-5` (`docs/REQUIREMENTS.md:36-38`) exists to make behaviour box-independent, and a budget-dependent skill text defeats it. External, and unanimous: LangChain's `trim_messages` carries an `include_system` flag whose only purpose is to keep the index-0 system message out of the trim (it defaults to `false`, so preserving is opt-in, but no comparable flag exists for any other message); Continue "always preserve the system message ... remove older messages first" (https://deepwiki.com/continuedev/continue/4.4-message-compilation-and-streaming); Claude Code's compaction "only touches your conversation history" while rules and memory are re-injected afterward (https://claudefa.st/blog/guide/mechanics/context-buffer-management); Codex's compaction discards tool results and file contents and keeps the summary (https://codex.danielvaughan.com/2026/04/14/context-compaction-deep-dive-codex-cli-claude-code-opencode/). |

**Verdict: kept first, an explicit protected set, `box profile` inserted where the requirement left
a gap, and a linear greedy pass rather than a priority search.**

*The protected set, and why each member is in it.* Protected sections never lose a token, and a step
whose protected set alone exceeds the target fails at stage 3 before a session is started.

| Protected section | Why |
|---|---|
| `template` | It is the instruction. A trimmed instruction produces an artefact that looks valid and is not, which is the failure ANA-2 already refused (`docs/ANA-2.md:415-416`). It is also bounded by construction: a template body is a hand-written row, not resolved content. |
| `skills` | `R-ID-5`, invariant 4. Bounded instead by `max_skill_tokens`, which refuses rather than degrades. |
| `box` | `R-PRM-3` omits it entirely, which is a gap this document fills rather than a permission to drop it. It is protected because it is small (the §4.2 projection is a dozen lines and a capped tool list), because it is what makes a command correct on this box, and because dropping it would silently change what the agent believes about its environment. |
| `command_queue` | Two sentences, and dropping them would make the agent run a heavy build directly against `R-MCP-4`'s intent. `R-MCP-4` (`docs/REQUIREMENTS.md:242-243`) says "**skill text** instructs agents to use `command_run`"; this document renders it as its own section instead, because the exposure is per phase (`docs/ANA-2.md:496-500`) while a skill body is a fixed row, so a skill cannot say "when it is exposed" truthfully. A skill may still repeat the instruction; the section is what makes it conditional. |
| `failure_reason` | Handoff only; one paragraph, and it is the reason the prompt exists. |

*The exact trim order.* Sections lose tokens in this order, first to lose listed first. The order is
`R-PRM-3`'s list read backwards, with the two review-loop sections and `excerpts` placed by the same
logic:

1. **`excerpts`** - `R-PRM-3` puts them last in the keep list. They are also budget-derived rather
   than budget-trimmed (§4.5), so in practice they simply never grow into the space that is not
   there.
2. **`upstream`** and **`previous_diff`** - `R-PRM-3` puts upstream summaries fourth. `previous_diff`
   joins them because ANA-2 assigns its render to this document "subject to its own trim order"
   (`docs/ANA-2.md:731`) and because a diff is the most compressible content in the prompt: its
   stat carries most of the signal at a fraction of the cost.
3. **`documents:*`** and **`verify_failure`** - `R-PRM-3` puts prior documents third.
   `verify_failure` joins them because it is command output whose tail carries the signal.
4. **`item`** - `R-PRM-3` puts the item body second. It is trimmed last of all the trimmable
   sections, because a step that cannot see what it was asked to do is a wasted step.
5. **the protected set** - never.

The two role-specific prompts reuse the same machinery with their own two-entry orders, which is why
the §4.2 table gives them their own ranks. **Judge:** `judge_candidate:<i>` first (per-candidate
isolate cap, then diff-stat-only, §4.6a), then `judge_task`; `template` stays protected. **Handoff:**
`diff_so_far` first, then `step_summary`, then `documents:*`; `template` and `failure_reason` stay
protected.

*The trim algorithm, as numbered steps.*

1. Resolve the budget: `ResolvedPhase.token_budget`, then `project.settings.token_budget`, then
   `app_setting.token_budget` (`docs/ANA-2.md:284`). Default 120 000 (§5.3).
2. Compute the target: `target = floor(budget * (1 - reserve))` where `reserve` is
   `app_setting.prompt_reserve_fraction`, default `0.10`. The reserve is the space the response and
   any tool schemas need; Priompt's `<empty>` component exists for exactly this
   (https://github.com/anysphere/priompt/blob/main/README.md) and Continue's config-load warnings
   are what happens without it ("This leaves only -24576 tokens for input context and will likely
   result in your inputs being truncated", https://github.com/continuedev/continue/issues/5166).
3. Render every section at full size. Estimate each one and the template's literal spans.
4. If the protected set alone exceeds `target`, fail the step: `run.failure = "prompt budget too
   small: protected sections need N tokens, target is M"`. No session starts. This is the one budget
   condition that is an error rather than a trim.
5. `deficit = total - target`. If `deficit <= 0`, assemble and stop.
6. Walk the trim order. For each section in turn, apply its strategy to reclaim at most `deficit`
   tokens, recompute `deficit`, and stop as soon as it reaches zero. A section is only ever trimmed
   after every section ahead of it in the order has been reduced to its floor.
7. Each strategy has a **floor** and a **drop**. Reaching the floor without clearing the deficit
   moves to the drop, which removes the section entirely and emits a
   `{ tokens: 0, trimmed: true }` entry. Moving to the next section happens only after the drop.
8. Re-estimate after every step. The estimator is cheap and re-estimating removes a whole class of
   accounting bug where a marker's own tokens are forgotten; Codex had to patch exactly that, its
   truncation fix "accounts for 3-line marker overhead when calculating head/tail line budgets"
   (https://github.com/openai/codex/pull/6476).
9. Assemble, normalise (§4.7), scrub (§4.7), digest, and write `trim_record` and `prompt_digest` in
   one call before the session starts.

*Per-section strategies, with their floor and their marker.*

| Section | Strategy | Floor | Drop |
|---|---|---|---|
| `excerpts` | drop whole files, lowest rank first | zero files | the section |
| `upstream` | the depth ladder: depth-2 Summary rows degrade to the Pending one-liner, then depth-2 rows are dropped, then depth-1 Summary rows degrade | all rows as one-liners | the section |
| `previous_diff` / `diff_so_far` | full diff, then the diff stat alone | the stat | the section |
| `documents:*` | drop whole documents from the end of `input_kinds` order, then head+tail the survivors | 25% of each survivor, minimum 40 lines | never dropped whole; a missing required document is §4.1's stage-3 failure, not a trim |
| `verify_failure` | tail-cut, keeping the tail | 200 lines | the section |
| `item` | head+tail | 50% of the body, minimum 80 lines | never dropped |
| `judge_candidate:<i>` | per-candidate isolate cap of `(target - task - template) / N`, then diff-stat-only | the document body plus the stat | the candidate block, which escalates to human selection rather than judging a candidate the judge cannot see |

Head+tail rather than head-only is settled by the clearest published before-and-after in the field.
Codex's own PR describes the old behaviour: "line-based truncation only. First 128 lines kept
entirely, regardless of size ... TAIL IS COMPLETELY LOST - error messages invisible to model"
(https://github.com/openai/codex/pull/6476). mini-SWE-agent ships 5 000 head plus 5 000 tail with an
`<elided_chars>` marker, SWE-agent caps observations at `max_observation_length: 100_000` characters,
and a Claude Code proposal asks for `preserveStart` / `preserveEnd` with a marker reporting bytes and
lines (https://github.com/anthropics/claude-code/issues/17611). All four report the elided quantity
in the marker, which is what `htui` copies:

```
[... htui elided 412 lines / 18 903 bytes ...]
```

One space-free ASCII form, on its own line, with the two numbers the reader can act on. The same two
numbers appear in `trim_record`, so the prompt and the record never disagree.

*Why a linear greedy pass and not a priority search.* Priompt is the closest prior art for
priority-driven prompt packing and it validates the mechanism parts this design borrows: `<isolate>`
for a per-candidate cap, `<first>` for a fallback ladder, `<empty>` for reserved response space. Its
authors also disown the generality: "We've discovered that adding priorities to everything is sort of
an anti-pattern. It is possible that priorities are the wrong abstraction", and "If you overuse
priorities, it is easy to make hard-to-cache prompts". Its binary search over cutoffs is
additionally "not always guaranteed to produce the perfect p_opt-cutoff"
(https://github.com/anysphere/priompt/blob/main/README.md). A linear pass over `R-PRM-3`'s five-entry
fixed order has none of that: it is exact, it is explainable in one table, and it is auditable
because the order is a requirement rather than a per-node number.

*How tokens are counted.* `TokenEstimator` is a pure function with a stable id recorded in
`trim_record.estimator`:

```rust
// crates/htui-core/src/prompt/estimate.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenEstimator { pub id: &'static str, pub prose_cpt: u16, pub code_cpt: u16 }

impl TokenEstimator {
    /// Splits `s` at fenced-code boundaries and sums ceil(chars / cpt) per span.
    /// Deliberately char-based, not byte-based: a multi-byte character is one unit, not four.
    pub fn estimate(self, s: &str) -> i64;
    /// Per agent family, from `agent.name`; unknown families get the conservative default.
    pub fn for_agent(name: &str) -> Self;
}
```

v1 ships three rows and nothing else: `("chars-v1", prose 35, code 30)` scaled by ten so the
constants are integers (3.5 and 3.0 characters per token) for the Claude family;
`("chars-v1", 40, 33)` (4.0 and 3.3) for the GPT and Gemini families; and the Claude row as the
default for an unknown agent, because it is the more conservative of the two and over-estimating
trims early rather than overflowing. The published ranges these come from put a Claude heuristic
within 10 to 20% of the real BPE tokenizer, and note that any pre-2026 ratio is roughly 30% low for
current Claude because "Claude 4.7 and later models ... The same input text produces approximately 30
percent more tokens than on earlier models"
(https://platform.claude.com/docs/en/build-with-claude/token-counting). The estimate is a budget
guard rail and never a billing statement, which is exactly the framing ANA-4 already adopted for cost
caps (`docs/ANA-4.md:1150`). Calibrating the constants from observed `run_step.usage` is the obvious
next move and is deliberately **not** in v1: it would make the trim decision, and therefore the
prompt bytes, a function of history, and it needs the pin-into-the-snapshot machinery to stay
reproducible. **Open for the maintainer 3** carries it.

*The `trim_record` writer.* `run_step.trim_record` already exists, so no migration is needed; what
does not exist is a way for MOD-2 to write it. ANA-2 assigns it to `finish_step(step, StepOutcome)`
(`docs/ANA-2.md:1745`), a MOD-4 method, while ANA-4's MOD-2 additions are `append_events`,
`set_step_usage`, `upsert_agent` and `upsert_agent_box` (`docs/ANA-4.md:351-358`), none of which
carries it. MOD-2 lands strictly before MOD-4 (`HANDOFF.md:53-72`), so `R-PRM-3`'s "the trim is
recorded on the step" would have no writer for a whole item. This document adds one:

```rust
// crates/htui-core/src/store/traits.rs, WriteStore
/// Records the assembled prompt's audit fields. Called at stage 3, before the session starts,
/// so a step that dies mid-session still says which prompt produced it (R-PRM-3, R-ORCH-11).
async fn set_step_prompt(&self, step: StepId, digest: &str, trim: &Value) -> Result<()>;
```

It is written **before** the session rather than at `finish_step` because a crashed step with no
digest is an audit hole, and because the digest is known at stage 3 by construction. ANA-4's
`set_step_usage(step, usage, prompt_digest)` keeps its signature; MOD-2 passes `None` for
`prompt_digest` because `set_step_prompt` already wrote it, and a later cleanup may drop the
parameter. That is the one ANA-4 detail this document supersedes, and §9 records it.

*What is reported in the TUI.* `R-PRM-3` says "recorded on the step", not "shown", and `R-TUI-4`'s
step-list columns are agent, model, gate state, usage and duration. MOD-4's `RunStepSummary`
additions (`docs/ANA-2.md:1573-1580`) omit both prompt fields. This document names the minimum
projection addition rather than leaving `trim_record` write-only:

- `RunStepSummary` gains `prompt_tokens: Option<i32>` and `trimmed: bool`, both derived from
  `trim_record` by the projection query, so the step list can render a `~34k` figure and a `!`
  marker in the existing columns without a new column.
- The step detail view renders the full record as a table of section, before, after, strategy, plus
  the header line of budget, target, estimator and totals. That is a MOD-2 view because MOD-2 owns
  the builder, and it is the only place the excerpt audit of §4.5 is visible.
- The displayed budget is always the **resolved effective** budget, never a default. Claude Code
  ships the opposite and it is a filed bug: `/context` "always displays the default autocompact
  buffer size ... even when `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` is set", so "users relying on
  `/context` to judge remaining capacity will overestimate how much space they have"
  (https://github.com/anthropics/claude-code/issues/27189).

---
### 4.5 File excerpt selection without an external tool (`R-PRM-1`, `R-ID-4`, `R-ID-6`, `R-LATER-7`)

**The constraint.** `R-PRM-1`'s last content clause (`docs/REQUIREMENTS.md:205-206`): "and any
`htui`-selected file excerpts. Never raw transcripts of other items." `R-ID-4` (`:33-35`): "`htui`
writes no files into managed repositories." `R-ID-6` (`:39-40`): "No LLM agent is ever in a sync,
cache, import or bookkeeping path. Those are deterministic code." `R-LATER-7` (`:283-284`):
"External context tools (Headroom, Serena, Graphify) as excerpt providers for `R-PRM-1`, **fail-open
when absent**."

**Options for the selector.**

| Option | Verdict | Reason |
|---|---|---|
| Aider's repo map: tree-sitter tags, a weighted file/identifier graph, personalized PageRank, binary-search sizing | Rejected for v1 | The best fully deterministic offline repo map in the field and the right long-term target (https://aider.chat/2023/10/22/repomap.html, `aider/repomap.py`). It costs a `cc` build dependency per grammar crate, an MSRV move from 1.85 to 1.90, roughly 300 to 400 KB per language, and it makes the grammar version a `prompt_digest` input. `htui` also does not need the graph half: Aider's own structure shows the personalization vector, which is mentioned filenames and identifiers, does most of the work (`aider/repomap.py:424-445`). |
| Embeddings, local or hosted | Rejected | Hosted means code egress and a model in a bookkeeping path, which `R-ID-6` forbids outright. Local means an index to build and keep fresh. Three production systems have publicly retreated: Sourcegraph replaced embeddings with "an adapted form of the BM25 ranking function" partly because "they needed to send source code to an OpenAI API for embedding" (https://sourcegraph.com/blog/how-cody-understands-your-codebase), and Sweep walked back from vectors to TF-IDF on latency, privacy and correctness (https://blog.sweep.dev/posts/autocomplete-context). |
| An LLM reranker in the style of Continue's `useReranking` | Rejected | It "will use an LLM to select the top nFinal results" (https://docs.continue.dev/reference/deprecated-codebase), which is `R-ID-6` violated by name. |
| No excerpts at all; let the agent grep | Rejected as the default, respected as the baseline | It is a real position with the strongest possible endorsement: Claude Code's lead states "In our testing we found that agentic search out-performed RAG for the kinds of things people use Code for" (https://news.ycombinator.com/item?id=43163011#43164253). It is rejected because `htui` holds two signals the agent cannot cheaply reach: the item's **declared** `touched_paths`, and the previous attempt's diff. It is respected in the sizing: excerpts are a head start, never a closed world. |
| **A five-tier deterministic signal union with lexical tie-breaking, budget-derived, no new crate** | **Adopted** | Every tier reuses a decision already taken. It adds no dependency: no glob crate (ANA-2 §4.7's matcher-free `PathPrefix`), no walker crate (`std::fs`), no git crate at MOD-2, no tokenizer. It is auditable, which is what makes the excerpt record of this section possible at all, and nothing in the surveyed field persists one. |

**Options for where the bytes are read from.**

| Option | Verdict | Reason |
|---|---|---|
| `repo_box_path` | Rejected as the primary | It is the wrong tree for two of four isolation modes: `worktree` and `copy` put the agent under `<config_dir>/trees/<run_id>/<step_id>/<repo_slug>/` (`docs/ANA-2.md:905`), so an excerpt read from the managed repo could disagree with the file the agent opens. It is also unreadable today: `repo_box_path` has no reader, no writer and no mirror, and MOD-7 writes those rows and has not landed. |
| The git object store at `base_ref` | Rejected for v1 | It is the cleanest answer for `R-ORCH-7` identity and for reproducibility from a commit alone, and it is what a later revision should adopt. It needs `gix`, which enters the workspace at MOD-4 build step 5, after MOD-2 (`docs/ANA-2.md:1805-1807`). |
| **`run_step_tree.path` for the repo, falling back to `repo_box_path`, falling back to no excerpts** | **Adopted** | The step's own tree is the tree the agent will work in, so an excerpt is what the agent will see. Identity across fan-out siblings is carried by invariant 3 - the assembler runs once per (position, attempt), not once per candidate - and the tree choice does not undo it. For `worktree` and `copy`, stage 2 has just created every candidate tree as a fresh checkout of the same `base_ref` (`docs/ANA-2.md:914-915`, `:959`), so at stage 3 all N trees are byte-identical and no agent has run. `local` is refused when `fan_out > 1` (`docs/ANA-2.md:917`). `shared_serialized` **does** fan out, one sibling after another in a single tree (`:916`), which is exactly why the single assembly per (position, attempt) is load-bearing rather than an optimisation: excerpts are read once, before the first sibling starts, and the later siblings receive those bytes rather than a re-read of a tree an earlier sibling has edited. |

**Verdict: five tiers, hard caps, budget-derived, read-only, provider-extensible, fail-open three
times over.**

*The signals and their weights.* Every candidate path gets the **maximum** tier weight it qualifies
for, not a sum, so one file cannot outrank another by accumulating weak evidence. Ties break on
repo-relative path byte order.

| Tier | Weight | Signal | Where it comes from |
|---|---|---|---|
| 1 | 100 | a path under a `PathPrefix` derived from `item.touched_paths` | `docs/ANA-2.md:1074-1077`: the glob truncated at its first `*?[{` and then at the last `/`, so `src/**/*.rs` becomes `src/` and a full path stays whole. Repo-qualified per `docs/ANA-2.md:1025`, bare glob meaning the primary repo. |
| 2 | 90 | a path changed by the previous attempt | the `before_hash..after_hash` range of that attempt's `run_step_tree` rows; the same range the review loop and the judge already use (`docs/ANA-2.md:731`, `:828`). MOD-4 supplies it. |
| 3 | 70 | a literal path-like token in the item body or a resolved input document | a token matching `[A-Za-z0-9_./-]+` that contains `/` and ends in a known source extension, or matches an existing repo-relative path exactly. Aider's `mentioned_fnames` (`aider/repomap.py:436`). |
| 4 | 50 | a file whose **path components** match an identifier mentioned in the item body | Aider's cheapest identifier signal, which needs no parser: "Add personalization once if any path component matches a mentioned ident" (`aider/repomap.py:437-445`). Identifiers are tokens of at least 8 characters that are snake, kebab or camel case, Aider's own filter (`repomap.py:493-494`), split on case per Sweep so `ChatGPT`, `chat_gpt` and `chatGPT` all yield `chat` and `gpt`. |
| 5 | 10 + a 0..9 lexical score | remaining files, scored by term overlap between the item's identifier set and the path plus the file's first 4 KB | a hand-rolled TF-IDF over the candidate set, roughly 150 lines and no crate. Cody's production ranker is "an adapted form of the BM25 ranking function"; Repoformer found "neither [UniXCoder nor CodeBLEU] outperformed the much more efficient Jaccard similarity" (https://arxiv.org/html/2403.10059v1), which removes the argument for anything learned. |

Tier 5 runs only when tiers 1 to 4 leave budget unspent, and it never contributes more than a third
of `max_files`.

*The algorithm, as numbered steps.*

1. Resolve the repo set: the run's `repo_scope` when a run exists, otherwise the project's repos.
   For each repo, resolve a readable root: `run_step_tree.path`, else `repo_box_path.local_path`,
   else skip the repo and record `no_path` in the excerpt audit. Skipping every repo yields no
   excerpt section, which is a fail-open, not an error: MOD-7 writes `repo_box_path` rows and has
   not landed, so a MOD-2 run may legitimately find none.
2. List candidate files under each root with a `std::fs` walk, depth-first, entries sorted by file
   name byte order at every level so the traversal is deterministic. Apply the skip rules below. Cap
   the walk at `app_setting.excerpt_max_scan_files`, default 20 000, and record `scan_truncated`
   when the cap bites.
3. Build the identifier and path-token sets from the item body and the resolved input documents.
4. Score every candidate by the tier table. Discard weight-zero candidates.
5. Sort by `(weight desc, repo_slug asc, path asc)` on raw bytes. Float scores from tier 5 are
   quantised to integers before sorting, so ordering never depends on `f64` comparison; Aider's own
   fix for the same hazard is the explicit composite key `key=lambda x: (x[1], x[0])`
   (`aider/repomap.py:548-550`).
6. Call every registered `ExcerptProvider` with the same `ExcerptRequest`, merge their candidates by
   `(repo, path)` keeping the higher weight, and re-sort. A provider that is absent, errors or
   misses its deadline is dropped and recorded (see the seam below).
7. Take candidates in order until a cap binds: `max_files` (default 12), the per-file caps, or the
   residual budget from §4.4 step 6.
8. Window each file: whole file when it has at most `file_line_cap` lines (default 400), otherwise
   the first `head_lines` (default 200) with the elision marker of §4.4. A file larger than
   `max_file_bytes` (default 512 KB) is skipped rather than windowed, because a 512 KB file that
   is not in tier 1 or 2 is almost always generated.
9. Render, in **path order** rather than rank order: `(repo_slug asc, path asc)`. Rank decides what
   survives; render order must be stable and scannable, and Aider likewise sorts its tree output by
   filename after ranking (`aider/repomap.py:759`).
10. Write the excerpt audit into `trim_record.excerpts`.

*Skip rules, evaluated in this order.* Each is deterministic and needs no crate.

| Rule | Reason |
|---|---|
| the path is inside `.git/` | never source |
| the path matches the **secret denylist**: `.env`, `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa*`, `id_ed25519*`, `*.keystore`, `.netrc`, `.npmrc`, `.pypirc`, `credentials`, `credentials.json`, `*.kdbx` | see below |
| the path matches a `.gitignore` entry, using a prefix-and-suffix subset matcher, not a glob crate | a file git ignores is build output or local state, not source. Cursor and Continue both reuse gitignore semantics for the same reason (https://cursor.com/docs/reference/ignore-file) |
| the file's first 8 KB contains a NUL byte | binary |
| the file is larger than `max_file_bytes` | see step 8 |
| the extension is a lockfile or a minified asset (`*.lock`, `*.min.js`, `*.min.css`, `*.map`) | high token cost, near-zero signal |

**The denylist is a selection rule, not a scrubbing rule, and it exists because scrubbing alone is
too late.** Excerpts are the only prompt section sourced from arbitrary working-tree bytes.
`R-SEC-3` is fail-closed: an unmasked pattern "marks the step failed and blocks persistence"
(`docs/REQUIREMENTS.md:227-228`). §4.7 puts the scrubber before the digest, so an excerpt carrying a
project secret would be masked before it is sent, which is correct; but a secret the scrubber does
not know a mask for, such as a hardcoded key in a `.env` a developer left in the tree, would reach
the agent and only then fail the step at persist time. Excluding the file class at selection is one
constant list and it removes the class. **Open for the maintainer 8** carries whether an excerpt
that trips the scrubber should also refuse the run.

*The render, byte-exact.*

```
<section name="excerpts">
Read-only context, selected by htui. These files may be truncated and may be out of date; the
working tree is authoritative. Do not treat an excerpt as the whole file, and open the file before
editing it.

<file path="htui:crates/htui-core/src/prompt/mod.rs" lines="1-180" reason="touched_path">
   1 | //! Prompt assembly (ANA-5).
   2 |
...
</file>
<file path="htui:crates/htui-core/src/store/traits.rs" lines="1-200" reason="mentioned" truncated="true">
...
[... htui elided 412 lines / 18 903 bytes ...]
</file>
</section>
```

The framing paragraph is modelled on the only battle-tested wording in the field, Aider's
"Do not propose changes to these files, treat them as *read-only*" (`aider/coders/base_prompts.py:45-48`),
and it exists because Aider also documents the failure it prevents: weaker models "sometimes
mistakenly try to edit the code in the repo map" (https://aider.chat/docs/faq.html). Line numbers
are on by default in `N | ` form, matching the `cat -n` shape Claude Code's `Read` returns and which
its edit tools address; they cost roughly 4 to 6% of the excerpt budget. **Open for the maintainer 9**
carries them. `reason` is a closed enum, not prose: `touched_path`, `prev_diff`, `mentioned`,
`identifier`, `lexical`, `provider:<name>`. `path` is `repo_slug:repo-relative`, never absolute
(§4.2 rule 5, invariant 3). Content is not fenced inside `<file>`, because the line-number prefix
already prevents a content fence from being read as the block's own delimiter, and because a fence
around numbered lines is noise.

*The excerpt audit shape.* Under `trim_record.excerpts`:

```json
{
  "provider_set": ["builtin@1"],
  "roots": [ { "repo": "htui", "source": "run_step_tree", "scan_truncated": false } ],
  "considered": 143,
  "selected": 6,
  "caps": { "max_files": 12, "file_line_cap": 400, "head_lines": 200, "max_file_bytes": 524288 },
  "files": [
    { "repo": "htui", "path": "crates/htui-core/src/prompt/mod.rs", "lines": "1-180",
      "rank": 1, "weight": 100, "reason": "touched_path", "truncated": false,
      "bytes": 6142, "sha256": "5f0a..." }
  ]
}
```

`sha256` is over the excerpt's rendered content bytes, not the whole file, so a reader can prove
which bytes the model saw without storing them. No surveyed tool persists a record of this kind;
Priompt's sourcemaps and Continue's pruning report are the nearest analogues and both are in-memory
debug aids.

*The `ExcerptProvider` seam for `R-LATER-7`.*

```rust
// crates/htui-core/src/prompt/excerpt.rs - the trait and the request live in htui-core so that
// ANA-3's providers, MOD-2's built-in and MOD-4's callers all see one definition.

#[derive(Debug, Clone)]
pub struct ExcerptRequest<'a> {
    pub item_key: &'a str,
    pub item_body: &'a str,
    pub phase: &'a str,
    pub touched_prefixes: &'a [PathPrefix],   // ANA-2 4.7, already repo-qualified
    pub changed_paths: &'a [RepoPath],        // previous attempt's diff, empty on attempt 1
    pub roots: &'a [RepoRoot],                // repo slug + readable absolute root
    pub budget_tokens: i64,                   // the residual from 4.4 step 6
    pub deadline: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptCandidate {
    pub repo: String,                         // repo slug, never a path
    pub path: String,                         // repo-relative
    pub lines: Option<(u32, u32)>,            // None = the provider has no opinion on the window
    pub weight: u16,                          // 0..=100, htui re-normalises against its own tiers
    pub reason: String,                       // rendered as reason="provider:<name>"
}

/// A source of excerpt candidates. Providers PROPOSE; htui ranks, windows, caps, renders and
/// digests. R-ID-6: a provider must be deterministic code, never a model call. R-ID-4: read only.
pub trait ExcerptProvider: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn propose(&self, req: &ExcerptRequest<'_>) -> Result<Vec<ExcerptCandidate>, ProviderError>;
}
```

Five rules, each with its reason:

1. **Providers propose candidates; `htui` renders.** Otherwise the prompt bytes, and therefore the
   digest, depend on a third party's formatting. Serena's surface is already candidate-shaped
   (`find_symbol`, `get_symbols_overview`, `find_referencing_symbols`,
   https://github.com/oraios/serena), so this costs the named providers nothing.
2. **Fail-open with a deadline.** Absent, erroring, panicking or slow means drop the provider,
   record it in `provider_set` with its status, and continue. `R-LATER-7` says "fail-open when
   absent" and `R-NF-2` forbids depending on an external daemon. The precedent is Aider disabling
   its own map rather than failing the request (`aider/repomap.py:143-145`); the counter-example is
   Codex, where a missing ripgrep degrades search silently and "can be misdiagnosed as model quality
   degradation" (https://github.com/openai/codex/issues/26905).
3. **A provider's participation is recorded and is allowed to change the digest.** A provider that
   changes the excerpt set genuinely changes the prompt, so pretending otherwise would make the
   digest a lie. `trim_record.excerpts.provider_set` carries `name@version` for every provider that
   contributed, so a digest mismatch between two boxes is explainable in one field rather than
   mysterious.
4. **Read-only, `R-ID-4`.** The trait has no write method and `RepoRoot` carries no writable handle.
   Serena in particular writes a `.serena/` directory into the project by default, visible as
   untracked in this repository today; ANA-3 must configure that outside every `repo_box_path` or
   the provider itself breaks `R-ID-4`.
5. **Non-LLM, `R-ID-6`.** Stated in the trait doc, unenforceable by the type system, and therefore
   an ANA-3 review criterion rather than a compile error.

The built-in ranker is itself an `ExcerptProvider` named `builtin`, always registered first and
never removable, so the "no providers configured" path and the "every provider failed" path are the
same code path and are exercised by the same tests.

*Deliberately not built.* No symbol index, no cross-file reachability, no call graph, no test-file
inference. A file that defines a symbol the item needs, but is neither declared in `touched_paths`
nor mentioned in the body, is not excerpted. That is acceptable and should be stated plainly:
Agentless, an LLM-driven localiser, is right at file level only 69.7% of the time and at function
level 52.0% (https://arxiv.org/abs/2407.01489), so a deterministic selector promising completeness
would be promising something nobody has. `R-PRM-3` puts excerpts last in the keep order for the same
reason.

---

### 4.6 The three templates ANA-2 names (`R-ORCH-7`, `R-ORCH-3`, `R-ORCH-5`, `R-PRM-4`)

**The constraint.** `docs/ANA-2.md:1842` lists them: "three templates this document names: `judge`
(§4.5), the review-loop forwarded set as prompt sections (§4.4), and the promotion handoff prompt
(§4.8)". `docs/ANA-2.md:822-823` is the only one called a template: "The prompt is assembled by
ANA-5 from a template named `judge`, with these sections and no others". `docs/ANA-2.md:1203` names
the third without calling it one: "ANA-5 builds a **handoff prompt** from the step's own stored
transcript (a summary, the input documents, the diff so far, the failure reason)".

**Options for rows versus built-in constants.**

| Option | Verdict | Reason |
|---|---|---|
| Both `judge` and `handoff` as built-in Rust constants | Rejected | `R-PRM-4` requires templates to be editable in the TUI, and while it says "per phase", the two prompts a maintainer most wants to tune are exactly these: the judge decides which code ships, and the handoff is a fresh model context ANA-2 describes as "a fresh model context, not a fake one" (`docs/ANA-2.md:1205-1209`). Built-ins also give MOD-9 two code paths where one would do, and give `prompt_digest` a body with no version to record. |
| Built-in with a row of the same name overriding it | Rejected | It avoids the seed change and keeps MOD-2 unblocked, which is real. It loses because "which body ran" then depends on a row's existence rather than on a version number, so `trim_record.template` cannot name it and the audit degrades. |
| **Both as seeded `prompt_template` rows with reserved names** | **Adopted** | One code path, one resolver, one audit field. It costs two seed rows per project and one validation rule. Every mature prompt-management system treats every prompt as a versioned registry object, first-party ones included: Langfuse pairs an integer version with labels and a commit message, Braintrust pins an opaque hex version id in code (https://www.braintrust.dev/docs/evaluate/write-prompts). |
| The review-loop forwarded set as a fourth template row | Rejected | ANA-2 describes it as sections, not a prompt: two of the three items are "injected by ANA-5 as a prompt section" and the third rides ordinary `input_kinds` (`docs/ANA-2.md:727-733`). A rival template for one step's prompt is how SWE-agent's pre-1.1.0 config merge went wrong, where "specifying `agent` in the second config would completely overwrite all agent settings of the first" (https://swe-agent.com/latest/config/config/). |

**Verdict: two reserved rows, one section pair, and two schema obligations this document owns.**

*Reserved names.* `judge` and `handoff` are reserved `prompt_template.name` values. `prompt_template`
has no `CHECK` on `name` and `UNIQUE (project_id, name, version)` means a project cannot hold both a
`judge` phase template and a `judge` judge template. The fix is one validation rule rather than a
column: **MOD-15's kind and graph editor refuses a phase named `judge` or `handoff`**, and MOD-9's
template editor derives `TemplateRole` from the name. That is cheaper than an `is_builtin` column,
needs no migration, and it is recorded in §9 as a MOD-15 obligation.

*Seed amendment to `docs/ANA-9.md` §5.10.* The sentence "one `prompt_template` version 1 per phase
name" (`docs/ANA-9.md:817-818`) becomes "one `prompt_template` version 1 per phase name, plus one
each for the two reserved names `judge` and `handoff`". Ten rows per project rather than eight, from
the eight `TEMPLATE_NAMES` in `crates/htui-core/src/fixtures.rs:546-556`. MOD-15 owns the seed;
MOD-2 owns the fixture update, since `fixtures.rs` is what MOD-2's golden prompts run against.

*(a) The `judge` template.* Sections `task`, `candidates`, `instruction`, per
`docs/ANA-2.md:822-830`, reconciled with §4.2's section model as follows: `task` and `candidates`
are placeholders, and **`instruction` is the template body's own literal tail**. That is the
reconciliation this document owes, and it is the better shape: ANA-2 wants the instruction to be
"return the winning `fanout_index` and a one-line reason per candidate", and making it the editable
literal text is exactly what a `prompt_template` row is for. `sections[]` therefore carries
`template`, `judge_task` and one `judge_candidate:<i>` per survivor, and no `instruction` entry.

`{{task}}` is **replayed, not re-assembled**: it is the `payload.text` of the `prompt` event at
`seq = 0` of the surviving candidate with the lowest `fanout_index`. Three reasons. It is what
ANA-2 asks for, "the judged phase's own assembled prompt, verbatim, so the judge knows what was
asked". A re-assembly at judge time could differ, because excerpts would be re-read against a tree
the candidates have since edited and upstream summaries may have moved, which would make the task
section a description of a question nobody was asked. And the lowest `fanout_index` is a
deterministic choice among N stored texts that `R-ORCH-7` guarantees are identical anyway. The
stored text is post-scrub, which is correct: the judge must not see secrets either.

The judge's own budget has no dedicated column and `docs/ANA-2.md:281-288`'s chain covers phases,
not the judge step at `fanout_index = -1`. The chain is extended here without a column: the judge
step resolves `token_budget` from **the judged phase**, then project, then app_setting. Within it,
`task` and `template` are taken first, and the remainder is divided equally among the N survivors as
each candidate's isolate cap, which is Priompt's `<isolate>` "with its own token limit" and Cody's
"N being a function of the length of each snippet". A candidate whose diff exceeds its share renders
as a diff stat only, which is precisely what ANA-2 already fixed: "ANA-5's trim order applied per
candidate block, with a candidate whose diff exceeds its share rendered as a diff stat only"
(`docs/ANA-2.md:832-835`).

*(b) The review-loop forwarded set.* Not a template. Two placeholders in the `implement` and `fix`
phase templates, `{{verify_failure}}` and `{{previous_diff}}`, both empty on attempt 1. The third
forwarded item, the `review` document, arrives through ordinary `input_kinds` because ANA-2's §4.1
seed amendment puts `review` in the implement phase's `input_kinds` (`docs/ANA-2.md:310-313`), so it
renders as `documents:review` with no special case. `docs/ANA-2.md:733` closes the set: "Nothing
else crosses. The previous attempt's session transcript does not, by `R-PRM-1`."

One consequence is worth stating because it looks like a bug and is not. ANA-2's input resolver
prefers this run's own output (`ORDER BY (s.run_id = $run) DESC NULLS LAST, d.version DESC`,
`docs/ANA-2.md:403-404`), and a superseded `fan_out = 1` implement step has `selected = NULL`, which
passes `selected IS NOT FALSE`. So on attempt 2 the previous attempt's own output document is a
resolved input **and** its diff is a section. That is intended: the document is what the agent
wrote, the diff is what it changed, and the review is why it must change again. `trim_record` shows
both, so a reader can see the overlap rather than wonder about it.

*(c) The `handoff` template.* `docs/ANA-2.md:1197-1209` reaches it when `DriverCaps.follow_up_in_session`
and `DriverCaps.resume` are both false, and it "starts a fresh agent session against the same
`run_step`, same `isolation_path`, next `turn`". Two seam decisions this document owes:

1. **The handoff prompt is persisted as a `follow_up` event at the next turn, not a second `prompt`
   row.** `EventKind::Prompt` is documented "always `seq = 0`, `turn = 0`"
   (`crates/htui-core/src/model/event.rs:12-14`, `docs/ANA-9.md:233`), and the step already has one.
   The `follow_up` payload carries `text` plus, for audit, `digest` and `sections[]`, which
   `docs/ANA-9.md:229` permits ("the driver may add keys, never rename these").
   `run_step.prompt_digest` keeps the seq-0 digest, because `R-ORCH-11` asks which prompt produced
   which commits and the step's identity is its first prompt.
2. **`{{step_summary}}` is deterministic, not model-written.** ANA-2 says "a summary" of the step's
   own transcript. `R-ID-6` keeps a model out of a bookkeeping path, and re-running a model to
   summarise a step in order to restart that step is both circular and non-reproducible. The
   summary is rendered by code from the step's own `session_event` rows:

```
<section name="step_summary" turns="3" events="217">
Tool calls, in order: read(4), edit(9), bash(6), read(2).
Files edited: crates/htui-core/src/prompt/mod.rs, crates/htui-core/src/prompt/trim.rs.
Errors: 1 - `cargo test` exited 101 at turn 2.
Last assistant message (turn 3, tail):
<the tail of the last assistant_text row, head+tail windowed>
</section>
```

Counts come from `EventKind` frequencies, file lists from `edit_proposal` payload paths, errors from
`error` rows, and the tail from the last `assistant_text` row. No other item's rows are readable,
because the summariser takes the step id it is restarting.

*Two schema-adjacent obligations this document owns because it writes the default bodies.*

**The `review` document's front matter.** `docs/ANA-2.md:440` makes `rejected` mean "a `review`-phase
output document whose front matter says `verdict: request-changes`", and validation criterion 6
tests it (`:2098-2102`), yet nothing defines the format and nothing says who emits it. Since the
default `review` template body is this document's to write, this document owns making it happen.
The grammar, fixed here and parsed by MOD-4:

```
line 1:  ---
line 2:  verdict: <approve|request-changes>
line 3:  ---
```

Exactly three lines at the very start of the document body, `---` with no trailing whitespace, the
value lowercase and trimmed, no other keys. A document whose first line is not `---`, or whose
second line does not parse, is treated as `request-changes` by MOD-4 with a note, because a review
whose verdict cannot be read is not an approval. The `review` body of §5.4 states the format to the
agent verbatim.

**The `judge` document's verdict block.** `docs/ANA-2.md:837-846` fixes it and says "The
orchestrator parses that block, never prose":

```json
{ "winner": 2, "reasons": { "0": "...", "1": "...", "2": "..." } }
```

The `judge` body of §5.4 instructs the agent to emit exactly one fenced `json` block containing
exactly that object, as the last block of the document, with one `reasons` entry per candidate
index shown. The fence marker is emitted by the agent inside the document, not by `htui` around the
template, because it lives in the produced document rather than in the prompt; what `htui` owns is
the instruction, and the instruction is editable because it lives in a row.

---
### 4.7 Stable serialisation so `prompt_digest` is reproducible (`R-ORCH-11`, `R-HIS-1`, `R-SEC-3`)

**The constraint.** `R-ORCH-11` (`docs/REQUIREMENTS.md:185-186`) records a "prompt digest" per step.
`docs/ANA-4.md:338-340`, verbatim:

> "**`prompt_digest`.** The recorder computes `sha256` (workspace dep `sha2 0.10`) over the
> assembled prompt text once, writes it as the `digest` key of the `prompt` payload and as
> `run_step.prompt_digest`, and never recomputes it. **ANA-5 supplies the text and `sections[]`.**"

and `docs/ANA-4.md:1257-1260`: "The driver computes the digest; **ANA-5 owns what is digested**."

**Options for what is digested.**

| Option | Verdict | Reason |
|---|---|---|
| The whole `session_event` row at `seq = 0` | Rejected | `docs/ANA-9.md:764`'s column comment reads that way ("sha256 of session_event seq 0"), but it is not computable: the row's payload contains the `digest` key itself. ANA-4's phrasing is operative and this document says so explicitly; the ANA-9 comment is shorthand for "the digest of the seq-0 prompt", and §9 updates it to say so. |
| `text` plus `sections[]` | Rejected | `sections[]` is derived from the text plus the estimator, so digesting it makes the digest a function of the estimator version. The estimator is allowed to change; the audit answer "which prompt produced these commits" must not change with it. |
| **The assembled prompt text alone, canonicalised, as sent** | **Adopted** | It is the smallest thing that answers `R-ORCH-11`'s question, and ANA-2 already fixed what the field is for: "an audit field that says which prompt produced which commits", explicitly not a replay key (`docs/ANA-2.md:1309-1315`). |

**Options for the canonical form.**

| Option | Verdict | Reason |
|---|---|---|
| Hash the bytes as produced, no normalisation | Rejected | `R-NF-1` lists Windows first, and git's default is asymmetric: "the default is `eol=crlf` on Windows and `eol=lf` on all other platforms" (https://git-scm.com/docs/gitattributes). The same excerpt of the same commit would then digest differently on a Windows box and a Linux box, for no reason a reader could act on. |
| RFC 8785 canonical JSON over a structured prompt spec | Rejected for `prompt_digest`, recorded as an option | JCS is the right tool for a JSON manifest and the wrong one here, because ANA-4 hashes text. It matters only if a second `spec_digest` field is ever added; **Open for the maintainer 10** carries that. |
| Canonicalise for hashing but send the original bytes | Rejected | The digest would then describe a string that was never sent, which defeats the audit. |
| **LF normalisation, BOM strip, one trailing LF, applied to the assembled text, and that text is what is sent, digested and persisted** | **Adopted** | One string, three uses, no divergence. |
| Add Unicode NFC normalisation | Rejected for v1 | It is the correct second-order fix and it needs a new **direct** dependency on `unicode-normalization` behind `htui-core`, for a hazard that only bites when two sources of the same visual text disagree in composition. The crate is already in `Cargo.lock` at 0.1.25 transitively, so the cost is an edge on the domain crate rather than a new build-graph node - smaller than first stated, and still the wrong trade for v1. The load-bearing hazard is line endings, not composition. **Open for the maintainer 7** carries it, default off. |

**Verdict: nine byte-exact rules, one canonical string, scrubbed before it is hashed.**

*The assembly pipeline, in order.* Every step is deterministic and none reads a clock.

1. Render each section's content at full size (§4.2), against inputs ordered by the rules below.
2. Scrub every section's content with the ANA-7 scrubber (`R-SEC-3`). A section whose content still
   matches an unmasked pattern after scrubbing fails the step before the session starts, which is
   `R-SEC-3`'s fail-closed rule applied one stage earlier than ANA-4's recorder applies it.
3. Trim (§4.4), which only ever removes bytes and inserts elision markers.
4. Substitute the sections into the parsed template's spans in source order.
5. Collapse: any run of three or more consecutive LFs in the substituted result becomes exactly two.
   This is what makes an absent section take its surrounding blank lines with it, deterministically.
6. Normalise: `\r\n` and lone `\r` become `\n`; a leading U+FEFF is dropped; trailing whitespace at
   end of string is trimmed and exactly one `\n` is appended.
7. `sha256` over the UTF-8 bytes of the result, lowercase hex, all 64 characters stored.
8. Hand the same `String` to `AgentDriver::start` and the same bytes to the recorder.

Steps 2 and 7 in that order are this document's resolution of an ordering ANA-4 left open. ANA-4
requires "every event passes the ANA-7 scrubber before it reaches the store" (`docs/ANA-4.md:75-77`)
and `R-HIS-1` requires the prompt to be stored "after scrubbing", while `docs/ANA-4.md:338-340` says
the digest is computed once and never recomputed. Scrubbing at assembly makes all three true at
once: the text sent, the text digested and the text stored are the same bytes, so the recorder's own
scrub pass over the `prompt` payload is an assertion that must find nothing. A hit at that point is
a bug in the assembler, and it still fails closed.

*The nine byte-exact rules.*

1. **Section order is the parsed template's own span order**, which for the seeded bodies of §5.4 is
   `R-PRM-1`'s order as §4.2 tabulates it. It is a property of the pinned template version, so it is
   fixed for a given `TemplateRef`, and it never depends on a map iteration.
2. **Upstream items** are ordered by `(depth, qualified_key bytes, item_id)` after `MIN(depth)`
   dedup, re-sorted in Rust rather than trusted from SQL (§4.3).
3. **Input documents** are ordered by the phase's `input_kinds` array order, which is an ordered
   `TEXT[]`, and within a kind by the resolver's single winning row.
4. **Skills** are ordered by `(skill_binding.position, skill.name bytes)` after the `R-SKL-2`
   override collapse, and deduped by `skill_id`.
5. **Excerpts** are ordered by `(repo_slug bytes, path bytes)` for rendering, after ranking, with
   float ranks quantised before any comparison (§4.5 step 5).
6. **Judge candidates** are ordered by `fanout_index`, and reversed only for the second judge call,
   which is a different prompt with a different digest by design.
7. **No wall-clock value appears in the digested text.** No timestamp, no duration, no "modified 3
   days ago". Recency, if it ever becomes a signal, is keyed on a commit id. Anthropic's own
   silent-invalidator list names `datetime.now()` in a prefix first
   (https://github.com/anthropics/skills/blob/main/skills/claude-api/shared/prompt-caching.md), and
   ANA-4 refused an idle-time flush for the same reason: "A time-based trigger would make the
   persisted row set a function of scheduling" (`docs/ANA-4.md:329-330`).
8. **No absolute path, no run id and no step id appears in the digested text, and no `fanout_index`
   appears in a `Phase`-role prompt.** This is what makes invariant 3 true rather than aspirational:
   two fan-out siblings differ only in their tree path and their index, and neither is renderable in
   the prompt they share. The `Judge` role is the deliberate exception and the only one: ANA-2
   requires each candidate block to carry "the `fanout_index`" (`docs/ANA-2.md:828`) and the verdict
   block names a winner by that index, so `judge_candidate:<i>` renders it. A judge prompt is
   assembled once and has no sibling to match, so nothing is weakened.
9. **The template version is pinned** and taken from the snapshot, never resolved as `latest` during
   a graph run (invariant 10). `trim_record.template` records the `{name, version}` actually used,
   because a digest alone cannot tell an auditor which row produced it. Every mature prompt registry
   stores both a version pointer and, where reproducibility matters, a content hash.

*What varies legitimately, and what that means.* The box profile section carries `hostname`, `os`,
`cpu`, `ram` and the tool list, so the same logical prompt on two different boxes digests
differently. That is correct for an audit field whose question is "which prompt produced these
commits", and it is harmless for `R-ORCH-7`, whose N siblings all run on one executing box. It does
mean `prompt_digest` is not a cross-box cache key, which nothing in this design wants it to be.

*Prompt caching, as a free consequence rather than a coupling.* The stable-first ordering §4.2
adopts is also the cache-friendly ordering: "static content first, dynamic content last"
(https://claude.com/blog/lessons-from-building-claude-code-prompt-caching-is-everything), and
OpenAI's equivalent, "a single character difference in the first 1,024 tokens results in a cache
miss" (https://developers.openai.com/api/docs/guides/prompt-caching). `htui` takes the ordering,
which is free, and does **not** emit `cache_control` breakpoints, which would couple it to the
Anthropic Messages API rather than to the `claude` CLI and ACP transports ANA-4 chose.
**Open for the maintainer 11** carries that.

---

### 4.8 Crate and module placement for the prompt builder (`R-NF-4`)

**The constraint.** `HANDOFF.md:56` assigns "Prompt builder per ANA-5" to MOD-2 in prose, and
`docs/ANA-4.md:1257-1260` says this document gates step 1 of MOD-2's build order. Neither concluded
crate layout has a module for it: `htui-agent` is `driver, event, record, launch, probe, acp/, cli/,
fake, conformance` (`docs/ANA-4.md:1163-1189`), `htui-orch` is `graph, status, engine, gate, fanout,
isolate, overlap, verify, recover, select, queue, command, fake, conformance`
(`docs/ANA-2.md:1658-1685`).

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| `htui-orch::prompt` | Rejected | `htui-orch` does not exist until MOD-4, and MOD-4 is blocked on MOD-2 (`HANDOFF.md:53-72`). MOD-2 could not build the builder at all. |
| `htui-agent::prompt` for the whole thing | Rejected | The validator must be callable by MOD-9's editor in `crates/htui`, which depends on `htui-core` and `htui-store` and on neither `htui-agent` nor `htui-orch`, and MOD-9 is explicitly not blocked on this document. It would also give the transport crate template, skill and document semantics that have nothing to do with transporting. |
| A sixth crate, `htui-prompt` | Rejected | ANA-2 justified a fifth crate on dependency weight plus headlessness (`docs/ANA-2.md:1646-1656`). Neither applies: the assembler adds **zero** dependencies once §4.1 chooses a hand scanner and §4.4 chooses a heuristic estimator, and it is already linkable headlessly because `htui-core` is. A crate that exists only for tidiness is a build-graph node nobody needed. |
| Everything in `htui-core::prompt`, including the filesystem walk | Rejected | The pure half passes ANA-4 §8's test easily; the I/O half does not. `htui-core` contains **no `std::fs` at all** today - every filesystem call in the workspace is in `htui-store` (`cache/`, `identity.rs`, all under `dirs::config_dir()/htui`) or in `crates/htui/src/lib.rs:120`'s log file - and putting a repository walk, a gitignore subset matcher and a per-file read behind the crate every other crate depends on is the same objection ANA-4 raised against the JSON-RPC SDK. |
| **`htui-core::prompt` for everything pure, `htui-agent::excerpt` for the I/O half** | **Adopted** | The split falls exactly where the dependency test does. Every type the assembler consumes already lives only in `htui-core` (`PromptTemplate`, `Document`, `Item`, `BoxRow`, `StepGraphPhase`, `Scope`), and both MOD-2 and MOD-4 must reach one assembler: MOD-2 for the step prompt, MOD-4 for the judge and handoff prompts (`docs/ANA-2.md:822`, `:1203`). `htui-core::prompt` is the only placement where neither depends on the other's crate. The I/O half goes to `htui-agent` because MOD-2 creates that crate, because `htui-orch` already depends on `htui-agent` (`docs/ANA-2.md:1649`) so MOD-4 reaches it without a new edge, and because `htui-agent` already carries `tokio` and `process-wrap`. This is an amendment to ANA-4 §8's module list, stated as such in §9. |

**Verdict: `htui-core::prompt` plus one module in `htui-agent`.** §8 gives the full layout.

---

## 5. Shapes this document fixes

### 5.1 `run_step.trim_record` (JSONB)

The column exists (`crates/htui-store/migrations/0001_init.sql:488`); this is its shape. Version 1,
`v` first so a later reader can branch. `serde_json::Map` is a `BTreeMap` in this workspace, so
object keys serialise sorted and the JSON is byte-stable without any extra rule.

The example below is an `implement` step on attempt 2, so `sections[]` is in that body's span order
(§5.4): `item`, `documents:*`, `verify_failure` (absent here, the previous attempt produced no
verification output), `previous_diff`, `upstream`, `box`, `skills`, `excerpts`, `command_queue`
(absent here, `fan_out_only` and `fan_out = 1`).

```json
{
  "v": 1,
  "template": { "name": "implement", "version": 3, "role": "phase" },
  "budget": 120000,
  "budget_source": "project",
  "reserve": 0.10,
  "target": 108000,
  "estimator": "chars-v1",
  "estimated_before": 165000,
  "estimated_after": 108000,
  "sections": [
    { "name": "template",        "tokens_before": 210,   "tokens_after": 210,   "strategy": "none",       "trimmed": false },
    { "name": "item",            "tokens_before": 12400, "tokens_after": 12400, "strategy": "none",       "trimmed": false },
    { "name": "documents:plan",  "tokens_before": 71800, "tokens_after": 65090, "strategy": "head_tail",  "trimmed": true,
      "elided_lines": 512, "elided_bytes": 23485 },
    { "name": "documents:review","tokens_before": 9200,  "tokens_after": 9200,  "strategy": "none",       "trimmed": false },
    { "name": "previous_diff",   "tokens_before": 31600, "tokens_after": 1180,  "strategy": "stat_only",  "trimmed": true },
    { "name": "upstream",        "tokens_before": 12800, "tokens_after": 1300,  "strategy": "stub_ladder","trimmed": true,
      "stubbed": 4, "dropped": 1 },
    { "name": "box",             "tokens_before": 120,   "tokens_after": 120,   "strategy": "none",       "trimmed": false },
    { "name": "skills",          "tokens_before": 18500, "tokens_after": 18500, "strategy": "none",       "trimmed": false },
    { "name": "excerpts",        "tokens_before": 8370,  "tokens_after": 0,     "strategy": "dropped",    "trimmed": true }
  ],
  "excerpts": { "provider_set": ["builtin@1"],
                "roots": [ { "repo": "htui", "source": "run_step_tree", "scan_truncated": false } ],
                "considered": 143, "selected": 3,
                "caps": { "max_files": 12, "file_line_cap": 400, "head_lines": 200,
                          "max_file_bytes": 524288 },
                "files": [] },
  "notes": []
}
```

The arithmetic is the invariant criterion 9 asserts, and it is worth reading once. `tokens_before`
sums to `estimated_before` (165 000) and `tokens_after` sums to `estimated_after` (108 000), which
is exactly `target`. The 57 000-token deficit is cleared in §4.4's order and stops the moment it
reaches zero: `excerpts` drops (8 370), `upstream` reaches its one-liner floor (11 500),
`previous_diff` falls to its stat (30 420, an overshoot because that ladder is all-or-nothing), and
`documents:plan` then gives up exactly the residual 6 710. `documents:review` and `item` are never
reached, and the four protected sections carry `strategy: "none"` throughout.

`selected` is what §4.5's ranker chose; `files[]` records only what survived into the prompt, so it
is empty here because the trimmer dropped the whole section afterwards. That asymmetry is the point:
a reader can see that three files were paid for by the ranker and none reached the model.

| Field | Meaning |
|---|---|
| `template` | the row actually rendered, `{name, version, role}`; `role` distinguishes a phase template from `judge` and `handoff` |
| `budget_source` | `phase`, `project` or `app_setting`, so a surprising budget is traceable in one field |
| `estimator` | the `TokenEstimator.id`; every token figure in the record is by this estimator and no other |
| `strategy` | closed enum: `none`, `head_tail`, `tail_cut`, `stat_only`, `stub_ladder`, `dropped` |
| `elided_lines` / `elided_bytes` | the same two numbers the in-prompt marker states, so the record and the prompt never disagree |
| `stubbed` / `dropped` | upstream ladder counters |
| `excerpts` | §4.5's audit, verbatim |
| `notes` | free strings for conditions that are not errors: a clamped `hops`, a skipped repo, a dropped provider |

**`trim_record` is canonical and `sections[]` is its abridged projection.** The event payload's
array is `sections.map(|s| { name, tokens: s.tokens_after, trimmed: s.trimmed })`, in the same
order. Nothing derives the other way, so the two can never drift, and a reader who needs the whole
story reads the step rather than the event.

### 5.2 The `prompt` event payload

Unchanged from `docs/ANA-9.md:233` in its keys; fixed here in its vocabulary.

```json
{
  "text": "<the canonical assembled prompt, verbatim>",
  "digest": "<sha256 lowercase hex, 64 chars>",
  "sections": [
    { "name": "template", "tokens": 210, "trimmed": false },
    { "name": "documents:plan", "tokens": 65090, "trimmed": true }
  ]
}
```

`name` is drawn from the closed vocabulary of §4.2: the fixed names `template`, `item`, `upstream`,
`box`, `skills`, `excerpts`, `verify_failure`, `previous_diff`, `command_queue`; the suffixed forms
`documents:<kind>` and `judge_candidate:<index>`; and the role-specific `judge_task`,
`step_summary`, `diff_so_far`, `failure_reason`. The demo fixture's bare `prd`
(`crates/htui-core/src/fixtures.rs:1244-1247`) becomes `documents:prd`.

### 5.3 `app_setting` keys this document reserves

ANA-2 §5.4 reserves the orchestration keys; these are the prompt keys, and they are seeded by the
same idempotent INSERT (§9). `token_budget` is the missing last link of ANA-2's chain.

| Key | Type | Default | For |
|---|---|---|---|
| `token_budget` | integer | `120000` | `R-PRM-3`; the last link of `docs/ANA-2.md:284`'s chain. 120 000 is the only figure the shipped fixture asserts (`crates/htui-core/src/fixtures.rs:489`); the superseded ANA-8's 12 000 (`docs/ANA-8.md:548`) is an order of magnitude away and predates every current model. **Open for the maintainer 1**. |
| `prompt_reserve_fraction` | number | `0.10` | §4.4 step 2; the response and tool-schema headroom |
| `prompt_upstream_hops` | integer | `2` | §4.3; clamped to `1..=2` |
| `max_skill_tokens` | integer | `20000` | §4.2's aggregate skills cap; exceeding it refuses the step |
| `excerpt_max_files` | integer | `12` | §4.5 step 7 |
| `excerpt_file_line_cap` | integer | `400` | §4.5 step 8; whole-file threshold |
| `excerpt_head_lines` | integer | `200` | §4.5 step 8; head window |
| `excerpt_max_file_bytes` | integer | `524288` | §4.5 step 8; skip threshold |
| `excerpt_max_scan_files` | integer | `20000` | §4.5 step 2; walk cap |
| `excerpt_provider_deadline_ms` | integer | `1500` | §4.5's fail-open deadline |

`SEEDED_SETTINGS` in `crates/htui-store/src/pg/mod.rs:44-48` is typed `[(&str, i32); 2]`, so nine of
these ten fit without a type change; `prompt_reserve_fraction` is the exception and is the reason
§9's INSERT is SQL rather than a constant-array extension. `ProjectSettings` (ANA-2 §4.7,
`docs/ANA-2.md:1115-1147`) gains `upstream_hops: Option<u8>` beside the existing
`token_budget: Option<i32>`.

### 5.4 Default template bodies

One body per seeded phase name, plus the two reserved names. These are what MOD-15 seeds at version
1 and what MOD-2's golden prompts run against. They are prompt engineering rather than analysis, so
**Open for the maintainer 4** carries them; what is not negotiable is the two machine-read
obligations they encode, the `review` front matter and the `judge` verdict block.

Every body follows one frame: one line of role, the bulk placeholders in `R-PRM-1`'s order, then the
instruction last (§4.2). Blank lines around an empty placeholder collapse (§4.7 step 5).

**`prd`**

```
You are running the `prd` phase for {{item_key}} ({{item_kind}}).

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Write a `prd` document: the problem in the user's terms, who has it, what "done" looks like, and
the open questions a reader must answer before design can start. State what is explicitly out of
scope. Do not design a solution and do not name files. If a requirement id in docs/REQUIREMENTS.md
governs this work, cite it.
```

**`plan`**

```
You are running the `plan` phase for {{item_key}} ({{item_kind}}).

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Write a `plan` document: ordered steps, each naming the files it touches and the observable result
that proves it landed. Call out risks and the validation you will run. Cite the requirement ids the
work satisfies. Change no code in this phase.
```

**`implement`**

```
You are running the `implement` phase for {{item_key}}, attempt {{attempt}}.

{{item}}
{{documents}}
{{verify_failure}}
{{previous_diff}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Make the change the plan describes, in this working tree. Keep the diff to what the plan asks for.
When a prior attempt is shown above, read its diff and its verification output first and fix the
cause rather than the symptom. Write a `{{output_kind}}` document describing what changed and how
you verified it, one section per file group.
```

**`review`**

```
You are running the `review` phase for {{item_key}}.

{{item}}
{{documents}}
{{previous_diff}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}

Review the change against the plan, the item body and the requirement ids it cites.

Your `review` document must begin with exactly these three lines and nothing before them:

---
verdict: request-changes
---

Set `verdict` to `approve` or `request-changes`, lowercase, nothing else on the line. Everything
after the closing `---` is your review: one section per finding, each naming the file and the
requirement, invariant or plan step it breaks, and what to do instead. `request-changes` sends the
item back to `implement` with this document attached, so every finding must be actionable. Do not
edit any file in this phase.
```

**`research`**

```
You are running the `research` phase for {{item_key}} ({{item_kind}}).

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Write a `research` document: what is already true in this repository, what prior art exists, what
options are open and what each costs. Cite file paths with line numbers for repository claims and
URLs for external ones. Where two sources disagree, say so and say which you believe. Reach no
verdict; the `verdict` phase does that.
```

**`verdict`**

```
You are running the `verdict` phase for {{item_key}}.

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}

Write a `verdict` document: for each question the research raised, the option adopted, the options
rejected and the reason each lost. Name what only the maintainer can decide and pick a default for
each. End with the work items the verdict implies. Cite the requirement ids involved.
```

**`reproduce`**

```
You are running the `reproduce` phase for {{item_key}}.

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Reproduce the reported failure in this working tree and write a `reproduce` document: the exact
command, the exact output, the smallest input that still fails, and the first place in the code
where behaviour diverges from expectation. Add a failing test if the project has a test suite.
Do not fix the bug in this phase.
```

**`fix`**

```
You are running the `fix` phase for {{item_key}}, attempt {{attempt}}.

{{item}}
{{documents}}
{{verify_failure}}
{{previous_diff}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Fix the cause the reproduction identified, not the symptom. Keep the diff minimal and leave the
failing test passing. Write a `{{output_kind}}` document describing the cause, the fix and the
evidence it is fixed.
```

**`judge`** (reserved name, role `judge`)

````
You are judging {{phase}} candidates for {{item_key}}. You did not write any of them.

{{task}}

{{candidates}}

Pick the candidate that best satisfies the task above. Weigh, in order: correctness against the
task, evidence from verification, the smallest change that does the job, and fit with the
surrounding code. Ignore candidate order, prose confidence and length; a longer answer is not a
better one.

Write one paragraph per candidate saying why it wins or loses, then end the document with exactly
one fenced json block, and nothing after it:

```json
{ "winner": 0, "reasons": { "0": "one line", "1": "one line" } }
```

`winner` must be one of the `fanout_index` values shown above, and `reasons` must have one entry
per candidate shown, keyed by its index as a string.
````

**`handoff`** (reserved name, role `handoff`)

```
You are continuing work already under way on {{item_key}}, phase {{phase}}, attempt {{attempt}}.
A previous session did the work below and could not be resumed, so this is a fresh context. The
working tree is exactly as that session left it.

{{step_summary}}
{{documents}}
{{diff_so_far}}
{{failure_reason}}

Read the diff before changing anything: the work already done is real and must not be redone or
reverted. Continue from where it stopped, address the failure reason, and finish the phase.
```

---
## 6. Requirement map and TUI surface

### 6.1 Where each requirement is met

| Requirement | Met by |
|---|---|
| `R-PRM-1` contents | §4.2's section table; §4.3's upstream walk; §4.5's excerpts; §4.2's box projection and skills resolution |
| `R-PRM-1` "never raw transcripts of other items" | invariant 1; `PromptSpec` has no event field; §4.2's three-case table |
| `R-PRM-2` stubs | §4.3's `in_scope` amendment and the two one-line forms |
| `R-PRM-3` budget | §4.4's chain, §5.3's `token_budget` key and default |
| `R-PRM-3` fixed priority order | §4.4's protected set and five-step trim order, read as keep-first |
| `R-PRM-3` "recorded on the step" | §5.1's `trim_record` shape and `WriteStore::set_step_prompt` |
| `R-PRM-4` versioned rows | unchanged `prompt_template`; §4.1's pinning rules; §4.6's two reserved names |
| `R-PRM-4` documented placeholder contract | §4.1's closed per-role tables and `TemplateError` |
| `R-PRM-4` editable in the TUI | MOD-9's editor calling `htui_core::prompt::template::parse` |
| `R-ID-5` skills inlined, behaviour box-independent | §4.2's skills resolution; invariant 4's protected set; §4.2's refuse-rather-than-drop cap |
| `R-ID-4` no writes into a managed repo | invariant 6; `RepoReader` has no write method |
| `R-ID-6` no model in a bookkeeping path | invariant 7; §4.5's fixed-weight ranker; §4.6's deterministic `step_summary` |
| `R-SKL-2` phase overrides project | §4.2's resolution, per `docs/ANA-9.md:691-692` |
| `R-ORCH-3` review loop | §4.6(b)'s two sections; `review` in `input_kinds` per ANA-2 |
| `R-ORCH-7` "the same prompt" | invariant 3; §4.7 rules 7 and 8; §4.5's stage-3 read |
| `R-ORCH-5` promotion | §4.6(c)'s handoff template and its `follow_up` persistence rule |
| `R-ORCH-11` prompt digest | §4.7's canonical form; `set_step_prompt` writing it at stage 3 |
| `R-HIS-1` prompt stored after scrubbing | §4.7 steps 2 and 7 in that order |
| `R-SEC-3` fail closed | §4.7 step 2; §4.5's selection denylist as the earlier gate |
| `R-LATER-7` fail-open providers | §4.5's `ExcerptProvider` and its five rules |
| `R-NF-4` every item cites its IDs | §9's HANDOFF amendments give MOD-2 `R-PRM-1..3` |

### 6.2 What the TUI shows, and which item owns it

| Surface | Content | Owner |
|---|---|---|
| Runs tab step list | `~34k` prompt tokens and a `!` when anything was trimmed, from the `RunStepSummary` additions of §4.4 | MOD-4 renders the list, MOD-2 adds the fields |
| Runs tab step detail | the full `trim_record` as a table of section, before, after, strategy, plus budget, target, estimator and totals; the excerpt audit as a file list with `reason` and `lines` | MOD-2 |
| Runs tab step detail | the assembled prompt itself, from `session_event` `seq = 0`, with its digest | MOD-2 (`StoreRequest::StepEvents` already reserved, `crates/htui/src/store_worker.rs:42-43`) |
| Skills tab, template editor | save-time validation errors from `TemplateError`, with the byte offset placed on the cursor; the closed placeholder list as inline help; the role derived from the name | MOD-9 |
| Settings tab | `token_budget`, `prompt_reserve_fraction`, `prompt_upstream_hops`, the excerpt caps | MOD-15 (`R-TUI-8` sections) |

---

## 7. The three prompt kinds, side by side

One assembler, three roles. This table exists so a reader does not have to diff §4.2, §4.6 and §5.4
to see what differs.

| | step prompt | judge prompt | handoff prompt |
|---|---|---|---|
| template role | `Phase` | `Judge` | `Handoff` |
| template name | `ResolvedPhase.template_name` | `judge` | `handoff` |
| version resolution | snapshot pin (`docs/ANA-2.md:285`) | snapshot pin beside the judged phase | latest at promotion time |
| `run_step` it belongs to | the step itself, `fanout_index` 0..N-1 | the judge step, `fanout_index = -1` (`docs/ANA-2.md:814-820`) | the same step, next `turn` |
| persisted as | `prompt` at `seq = 0, turn = 0` | `prompt` at `seq = 0, turn = 0` of the judge step | `follow_up` at the next turn (§4.6c) |
| writes `run_step.prompt_digest` | yes | yes, on the judge step | no; the seq-0 digest stands |
| budget chain | phase, project, app_setting | the judged phase, project, app_setting | the same as the step's |
| sections | `template`, `item`, `documents:*`, `upstream`, `box`, `skills`, `excerpts`, `command_queue`, plus `verify_failure` and `previous_diff` on attempt > 1 | `template`, `judge_task`, `judge_candidate:<i>` | `template`, `step_summary`, `documents:*`, `diff_so_far`, `failure_reason` |
| trimmable | per §4.4 | per-candidate isolate cap, then diff-stat-only | per §4.4, `failure_reason` protected |
| built by | MOD-2 | MOD-4 through the same assembler | MOD-4 (promotion is `R-ORCH-5`, ANA-2 §4.8) |
| identical across siblings | required (`R-ORCH-7`) | n/a, one judge | n/a |

---

## 8. Crate and module layout for MOD-2

§4.8 fixed the placement; this is the layout, mirroring `docs/ANA-4.md` §8's shape.

**No new crate.** The assembler adds zero workspace dependencies: §4.1 chose a hand scanner over a
template engine, §4.4 chose a heuristic estimator over a tokenizer, §4.5 chose ANA-2's matcher-free
`PathPrefix` over a glob crate and `std::fs` over a walker crate, and `sha2 0.10` is already a
workspace dependency used by `crates/htui-store/src/identity.rs`. That is the whole argument: ANA-4
§8's rule is that `htui-core` is "deliberately dependency-light (no `sqlx`, no `tokio` process, no
child-process code) and is the crate every other one depends on" (`docs/ANA-4.md:1156-1161`), and a
module that adds nothing to `Cargo.toml` cannot violate it.

```
crates/htui-core/src/prompt/
  mod.rs         #![warn(missing_docs)] surface: assemble(), PromptSpec, AssembledPrompt,
                 Section, SectionName, TemplateRef, TemplateRole
  template.rs    the {{name}} scanner, Placeholder, ParsedTemplate, parse(), TemplateError
                 -- the module MOD-9 calls at save time
  render.rs      the <section> wrapper, the fence-collision rule, the box projection,
                 the skills render, the upstream render incl. the two stub forms
  estimate.rs    TokenEstimator, the per-family constants, the fenced-span split
  trim.rs        the protected set, the trim order, per-section strategies, TrimRecord
  excerpt.rs     ExcerptProvider, ExcerptRequest, ExcerptCandidate, RepoReader, RepoRoot,
                 the built-in ranker as a pure function over a supplied listing
  digest.rs      the canonical form of 4.7 and the sha256
  fixtures.rs    golden PromptSpec inputs, feature `test-support`
crates/htui-core/src/model/link.rs      + UpstreamEntry
crates/htui-core/src/model/skill.rs     + Skill, SkillVersion, SkillBinding, BoundSkill  (new file)
crates/htui-core/src/model/box_.rs      + BoxProfile (the 4.2 projection)
crates/htui-core/src/store/traits.rs    + the reads and the one write below
crates/htui-core/src/store/mem.rs       + skills, bindings, projects-with-settings state
crates/htui-core/src/store/conformance.rs + the cases below
crates/htui-agent/src/excerpt.rs        FsRepoReader: the std::fs walk, the gitignore subset
                                        matcher, the skip rules, the per-file read.
                                        -- an addition to ANA-4 8's module list (9)
crates/htui/src/ui/tabs/backlog/detail/runs.rs   the trim-record view (MOD-2's half)
```

**Public types, as design notation.**

```rust
// crates/htui-core/src/prompt/mod.rs

/// Everything the assembler needs. No store handle, no I/O, no clock: assemble() is pure.
#[derive(Debug, Clone)]
pub struct PromptSpec {
    pub role: TemplateRole,
    pub template: TemplateRef,          // name + version, pinned
    pub body: String,                   // prompt_template.body of that version
    pub item_key: String,               // "<project.slug>:<item.key>"
    pub item_title: String,
    pub item_kind: String,
    pub item_body: String,
    pub phase: String,
    pub output_kind: Option<String>,
    pub attempt: i32,
    pub documents: Vec<InputDocument>,  // input_kinds order, already resolved by ANA-2 4.2
    pub upstream: Vec<UpstreamEntry>,   // 4.3 order, already deduped
    pub box_profile: BoxProfile,
    pub skills: Vec<BoundSkill>,        // R-SKL-2 order, already collapsed
    pub excerpts: Vec<Excerpt>,         // 4.5 candidates already read and windowed
    pub command_queue: bool,            // R-MCP-4 exposure, already resolved
    pub verify_failure: Option<VerifyFailure>,
    pub previous_diff: Option<DiffBlock>,
    pub judge: Option<JudgeInputs>,     // Some only for role == Judge
    pub handoff: Option<HandoffInputs>, // Some only for role == Handoff
    pub budget: Budget,                 // resolved number + source + reserve
    pub estimator: TokenEstimator,
}

/// The result. `text` is canonical (4.7) and is what is sent, digested and persisted.
#[derive(Debug, Clone)]
pub struct AssembledPrompt {
    pub text: String,
    pub digest: String,                 // sha256 lowercase hex; digest.rs computes it
    pub sections: Vec<Section>,         // full form; SectionEntry is the abridged projection
    pub trim: TrimRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: SectionName,
    pub tokens_before: i64,
    pub tokens_after: i64,
    pub strategy: TrimStrategy,         // None | HeadTail | TailCut | StatOnly | StubLadder | Dropped
    pub elided_lines: Option<u32>,
    pub elided_bytes: Option<u64>,
}

/// The closed vocabulary of 4.2 as a type, so a section name cannot be invented at a call site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SectionName {
    Template, Item, Documents(String), Upstream, Box, Skills, Excerpts,
    VerifyFailure, PreviousDiff, CommandQueue,
    JudgeTask, JudgeCandidate(i32),
    StepSummary, DiffSoFar, FailureReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRef { pub name: String, pub version: i32 }

/// The one entry point. Pure: same PromptSpec, same bytes, on any box.
pub fn assemble(spec: &PromptSpec) -> Result<AssembledPrompt, AssembleError>;
```

**Store-trait additions, by method name and signature.** Reads go on `ReadStore` only when the
SQLite mirror can answer them, which is ANA-2 §8's rule (`docs/ANA-2.md:1699-1705`); the rest are
inherent on `PgStore` and dispatched by `Backend`, following the `workspaces` / `box_info` /
`active_runs` / `projects` precedent (`crates/htui-store/src/pg/read.rs:378-382`).

*Reads, `ReadStore` (mirrorable).*

| Method | Signature | Mirrorable because | Claimed by |
|---|---|---|---|
| `document` | `async fn document(&self, id: DocumentId) -> Result<Option<Document>>` | `document` is mirrored with its body (`cache_migrations/0001_mirror.sql:97-101`) | already named at `docs/ANA-2.md:1730` |
| `documents_of_kinds` | `async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>>` | same table | already named at `docs/ANA-2.md:1731` |
| `upstream_summaries` | `async fn upstream_summaries(&self, id: ItemId, hops: u8, scope: &Scope) -> Result<Vec<UpstreamEntry>>` | `item_link`, `item`, `project`, `document` and `workspace_project` are all mirrored | **ANA-5, new**; the `scope` argument needs the no-workspace case §4.3 step 1 flags, which the shipped `Scope` cannot express |
| `project` | `async fn project(&self, id: ProjectId) -> Result<Option<Project>>` | `project.settings` is mirrored as TEXT (`0001_mirror.sql:57-61`) | **ANA-5, new**; the shipped `projects()` returns `ProjectRef`, which has no `settings`, so `docs/ANA-2.md:284`'s chain is unreadable without it |

*Reads, `PgStore` inherent (online only; `prompt_template`, `skill*`, `repo_box_path`, `app_setting`
and `box_tool` are all absent from the mirrored list, `docs/ANA-9.md:306-311`).*

| Method | Signature | Claimed by |
|---|---|---|
| `prompt_template` | `async fn prompt_template(&self, project: ProjectId, name: &str, version: Option<i32>) -> Result<Option<PromptTemplate>>` | already named at `docs/ANA-2.md:1715` |
| `bound_skills` | `async fn bound_skills(&self, project: ProjectId, phase: Option<PhaseId>) -> Result<Vec<BoundSkill>>` | **ANA-5, new**; ANA-2 §8 has no skill reader at all |
| `box_profile` | `async fn box_profile(&self, id: BoxId) -> Result<Option<BoxProfile>>` | **ANA-5, new**; it is `box_row` joined to `box_tool` ordered by name, which is the §4.2 projection in one round trip. `BoxTool` has zero readers today. `box` **is** mirrored ("own row only", `docs/ANA-9.md:306`) but `box_tool` is not, so the join cannot be answered offline and the method is inherent rather than on `ReadStore` |
| `app_settings` | `async fn app_settings(&self) -> Result<BTreeMap<String, Value>>` | already named at `docs/ANA-2.md:1720` |
| `repo_paths` | `async fn repo_paths(&self, box_id: BoxId) -> Result<Vec<RepoBoxPath>>` | already named at `docs/ANA-2.md:1719` |

*Writes, `WriteStore`.*

| Method | Signature | For |
|---|---|---|
| `set_step_prompt` | `async fn set_step_prompt(&self, step: StepId, digest: &str, trim: &Value) -> Result<()>` | **ANA-5, new**; §4.4's writer gap. Called at stage 3, before the session |

MOD-9 additionally needs `upsert_skill`, `add_skill_version` and `set_skill_binding`, and MOD-9's own
item owns them; this document names only what the prompt builder reads. The `Skill`, `SkillVersion`,
`SkillBinding` and `BoundSkill` **types** are a shared prerequisite of MOD-2 and MOD-9 and neither is
blocked on the other, so whichever lands first owns them, mirroring the MOD-15 / MOD-4 seed-amendment
rule of `docs/ANA-2.md:1845`.

**Cost this prices, deliberately.** Six new store methods, each obliging a `MemStore` implementation
and a `conformance::CASES` entry. The suite is "written against `WriteStore` alone"
(`crates/htui-core/src/store/conformance.rs:1-7`) and `WriteStore` has exactly two implementors,
`MemStore` and `PgStore` (`mem.rs:727`, `pg/write.rs:27`); `CacheStore` implements `ReadStore` only
(`cache/read.rs:208`), so **`CASES` cannot exercise the mirror**. The four `ReadStore` additions
therefore need a second, `ReadStore`-shaped harness for the Postgres-versus-mirror comparison of test
4 and criterion 19; MOD-2 either generalises `run_case` over `ReadStore` or writes a small parallel
suite. **Unverified - MOD-2 must confirm** which, since generalising touches a shipped signature.
`CASES.len()` is asserted in two places (`crates/htui-store/tests/pg_conformance.rs:19` and
`crates/htui-core/tests/mem_store.rs:24`, both against `15`), so a case cannot be added without a
Postgres run. `MemStore::State` also gains skill and binding vectors and a projects-with-settings
map, which it does not have today.

**Fakes and test doubles.**

| Piece | What it is |
|---|---|
| `MemStore` extensions | skills, bindings, project settings, and `upstream_summaries` implemented over the existing `item_link` and `document` vectors with the same dedup, classification and byte ordering as the SQL |
| `FakeRepoReader` | an in-memory `BTreeMap<(repo, path), String>` behind `RepoReader`, so every excerpt test runs with no filesystem, no git and no temp directory |
| `FakeExcerptProvider` | four scripted behaviours: returns candidates, returns an error, panics, and sleeps past the deadline. All four must leave the prompt assembled |
| `htui_core::prompt::fixtures` | golden `PromptSpec` values behind feature `test-support`, one per template role plus an "everything empty" and an "everything oversize" case |

**Test plan.**

1. **Golden prompts.** `insta::assert_snapshot!("prompt_implement_attempt2", assembled.text)` per
   role and per interesting emptiness pattern, using the existing explicit-name convention
   (`crates/htui/tests/backlog.rs:77`). This requires adding `insta` as an `htui-core`
   dev-dependency; today `htui-core`'s only dev-dependency is `tokio`. The alternative,
   `include_str!` plus `assert_eq!`, was considered and rejected: `insta` gives review and accept
   workflow for a body of text that will be reworded often, and the dependency is already in the
   workspace lock.
2. **Digest stability, same input twice.** `assemble(&spec).digest == assemble(&spec).digest`, for
   every fixture, asserting there is no map iteration and no clock in the path.
3. **Digest stability, CRLF versus LF.** The same fixture with every excerpt and document body's
   line endings switched to CRLF produces the identical digest. This is the Windows hazard of §4.7
   and it is the one test that cannot be skipped on either platform.
4. **Digest stability, Postgres versus mirror.** The same item's upstream walk read through
   `PgStore` and through `CacheStore` produces the identical rendered section, which is what the
   Rust re-sort of §4.3 exists for.
5. **Fan-out identity.** Assembling for `fanout_index` 0, 1 and 2 of one phase attempt yields three
   identical digests, and the text contains no absolute path, no run id and no step id. A grep
   assertion over the rendered text, not just an equality check, so a future section that leaks a
   path fails loudly.
6. **Diamond dedup.** An item with `A blocked_by B`, `A origin C`, `B blocked_by D`, `C blocked_by D`
   renders `D` exactly once, at depth 2.
7. **Trim determinism.** A fixture 1.5x over budget produces a `trim_record` whose
   `estimated_after <= target`, whose per-section `tokens_after` sum plus the template matches
   `estimated_after`, and whose `sections[]` projection equals the payload array element for element.
8. **Protected-set refusal.** A fixture whose skills alone exceed `max_skill_tokens` and one whose
   protected set exceeds `target` both fail before any session is started, with the exact message.
9. **Template validation.** A table-driven case per `TemplateError` variant, including
   `{{ item }}` with spaces, `{{itme}}`, `{{candidates}}` in a phase body, `{{{{item}}}}` rendering
   the literal `{{item}}`, and an unterminated `{{`.
10. **Conformance.** One `CASES` entry per new store method, run over the two `WriteStore` targets
    `MemStore` and `PgStore`; `upstream_summaries` gets three cases (diamond, out-of-scope,
    in-scope-no-summary) because it is the method with three render states. `CacheStore` is not a
    `CASES` target (it implements `ReadStore` only), so the mirror half of the four `ReadStore`
    additions is asserted by test 4's separate Postgres-versus-mirror comparison.
11. **Excerpt provider fail-open.** Each `FakeExcerptProvider` behaviour leaves a valid prompt, and
    the failing ones appear in `trim_record.excerpts.provider_set` with a status.

`crates/htui-core/src/fixtures.rs:1223-1226`'s `PROMPT_DIGEST` stays a hand-written literal. It is
demo data for MOD-1's renders, not an assembler output, and computing it would make every fixture
rewording churn a constant. The digest-stability net is tests 2 to 5 against
`prompt::fixtures`, which is a separate, purpose-built body of inputs. MOD-2 does update the demo
`sections[]` array to the §5.2 vocabulary.

**What each downstream item consumes.**

| Item | Consumes |
|---|---|
| **MOD-2** | the whole of `htui_core::prompt`; `htui-agent::excerpt`; the six store methods; `set_step_prompt`; the step prompt and its `prompt` event; the trim-record view |
| **MOD-4** | `assemble()` for the judge prompt (§4.6a) and the handoff prompt (§4.6c); `verify_failure` and `previous_diff` for the review loop (§4.6b); `finish_step` keeps `trim_record` in `StepOutcome` for a later re-write but the pre-flight write is MOD-2's; the `RunStepSummary` fields of §4.4 |
| **MOD-9** | `htui_core::prompt::template::parse` at save; `TemplateRole` derived from the row name; the closed placeholder tables of §4.1 as editor help; the `judge` and `handoff` reserved names |
| **MOD-15** | §4.6's seed amendment, ten templates per project rather than eight; §5.4's bodies; the reserved-name refusal in the phase editor; §5.3's `app_setting` keys in the Settings tab |
| **MOD-13** | nothing new, but `touched_paths` quality is tier 1 of §4.5, so its editor is what makes excerpts good; the `repo:glob` validation it already owes (`HANDOFF.md:107-108`) is what §4.5's `PathPrefix` consumes |
| **MOD-7** | nothing new, but `repo_box_path` rows and a real box probe are what turn §4.5's fallback root and §4.2's box projection from degraded into complete |
| **MOD-11** | the `box_profile` read tool of `R-MCP-2` must return §4.2's projection, so tool and section agree |
| **ANA-3** | `ExcerptProvider`, `ExcerptRequest`, `ExcerptCandidate` and the five seam rules of §4.5 |

---

## 9. Phasing and downstream impact

**MOD-2 build order, prompt-builder half.** ANA-4 §9's MOD-2 order stands; this is the work this
document adds, and steps 1 to 5 need nothing from ANA-4's transports.

1. `htui-core::prompt::template` and `estimate`: the scanner, `Placeholder`, `parse()`,
   `TemplateError`, the estimator. Table-driven tests only. This is the commit MOD-9 unblocks on,
   so it lands first even though MOD-9 is not formally blocked.
2. `htui-core::prompt::render` and `digest`: the `<section>` wrapper, the fence-collision rule, the
   box projection, the skills render, the upstream render, the canonical form. Golden prompts
   against `prompt::fixtures` with every section present and every section absent.
3. `htui-core::prompt::trim`: the protected set, the order, the strategies, `TrimRecord`, and
   `assemble()` wiring 1 to 3 together. Tests 2, 3, 7 and 8 of §8.
4. `htui-core::model::skill` and `link::UpstreamEntry`, the `MemStore` state additions, the six store
   methods and their conformance cases. `MemStore` implements them all; nothing touches Postgres yet.
5. `PgStore` implementations plus the `Backend` dispatch arms, the amended §7.3 query, and
   `cargo sqlx prepare --check` from `crates/htui-store` because `SQLX_OFFLINE=true` is set.
   Migration `0002_agent_probe.sql` gains its ANA-5 sections here.
6. `htui-core::prompt::excerpt`'s pure ranker plus `htui-agent::excerpt`'s `FsRepoReader`, the skip
   rules, the denylist and the gitignore subset matcher. `FakeRepoReader` and the four
   `FakeExcerptProvider` behaviours.
7. The step prompt end to end: stage-3 assembly, `set_step_prompt`, the `prompt` event at `seq = 0`,
   and the fixture `sections[]` update. Test 5's fan-out identity assertions.
8. The Runs-tab trim-record view and the `RunStepSummary` field additions.

MOD-4 then fills the two orchestration-only sections and the two extra roles: `verify_failure` and
`previous_diff` need `run_step.verify_outcome`, `verify_exit_code` and `run_step_tree` from
`0003_orchestration.sql` plus the git library that arrives at MOD-4 build step 5, and the judge and
handoff prompts are MOD-4 callers of the same `assemble()`.

**Forward-only migration amendments.** This document needs **no DDL**. `run_step.trim_record` and
`run_step.prompt_digest` both ship in `0001_init.sql:487-488`, are mirrored at
`cache_migrations/0001_mirror.sql:117` and are modelled at `crates/htui-core/src/model/run.rs:154-158`;
`prompt_template.body` needs a documented contract, not a column; the two reserved template names
need a seed, not a column, and a `prompt_template` seed is per project and therefore belongs in
MOD-15's project-creation path rather than in a migration. What remains is one idempotent
`INSERT` of §5.3's ten `app_setting` keys and five `COMMENT ON COLUMN` statements.

*Which number it takes.* Three options, and the constraint that decides between them is landing
order, not tidiness.

| Option | Verdict | Reason |
|---|---|---|
| A new `0004_prompt.sql` | Rejected | `sqlx::migrate!` applies pending files in ordinal order and `PgStore::connect` refuses an applied version the binary does not know ("schema is newer than this htui", `docs/decisions/mod/mod-6.md` §2). MOD-2 needs these rows before MOD-4 has even written `0003`, so an applied `0004` with `0003` missing is exactly the refusal path. |
| Take `0003` and renumber ANA-2 to `0004` | Rejected | Correct by landing order and expensive: it supersedes a concluded ANA's fully written SQL, its `-- Depends on 0002_agent_probe.sql` header (`docs/ANA-2.md:1862-1864`), four `HANDOFF.md` references and the ANA-2 close-out. The cost is paid to rename ten INSERT rows. |
| **Fold into MOD-2's `0002_agent_probe.sql` as numbered sections, keeping the file name** | **Adopted** | MOD-2 authors `0002` and MOD-2 is this document's consumer, so there is zero sequencing risk and no concluded document is edited. The file name stops being a complete description of its contents, which is the whole cost, and a header comment naming both ANAs fixes the discoverability half of it. `0003` already sets the precedent that one migration file carries one item's whole schema change rather than one ANA's. |

*The ANA-5 sections of `0002_agent_probe.sql`.* Appended after ANA-4's `ALTER TABLE agent_box ADD
COLUMN probe JSONB;` and its comment.

```sql
-- --------------------------------------------------------------------------------------------
-- ANA-5 (prompt assembly). No DDL: run_step.prompt_digest and run_step.trim_record already
-- exist in 0001_init.sql. These are the documented contracts and the app_setting defaults.
-- --------------------------------------------------------------------------------------------

COMMENT ON COLUMN prompt_template.body IS
  'ANA-5 4.1: {{name}} placeholders over a closed per-role set; {{{{ escapes a literal {{; no '
  'conditionals and no loops, because a section whose data is absent renders empty. Role is '
  'derived from name: judge and handoff are reserved, everything else is a phase template.';

COMMENT ON COLUMN prompt_template.name IS
  'phase name, or one of the reserved names judge and handoff (ANA-5 4.6); defaults are copied '
  'into each new project by the ANA-9 5.10 seed as amended by ANA-5';

COMMENT ON COLUMN run_step.prompt_digest IS
  'ANA-5 4.7: sha256, lowercase hex, over the canonical assembled prompt TEXT as sent - LF '
  'normalised, BOM stripped, one trailing LF, scrubbed before hashing. Not over the payload and '
  'not over sections[]. An audit field, never a replay key (ANA-2 4.9).';

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1: {v, template, budget, budget_source, reserve, target, estimator, estimated_before, '
  'estimated_after, sections[], excerpts, notes}. Canonical; the prompt payload sections[] array '
  'is its abridged projection. Written at stage 3 by set_step_prompt, before the session starts.';

COMMENT ON COLUMN step_graph_phase.token_budget IS
  'ANA-5 4.4: phase, then project.settings.token_budget, then app_setting.token_budget; the '
  'assembler targets budget * (1 - app_setting.prompt_reserve_fraction)';

-- ANA-5 5.3 defaults. Idempotent, so a re-run and the MOD-6 seed agree.
INSERT INTO app_setting (key, value) VALUES
  ('token_budget',                 '120000'::jsonb),
  ('prompt_reserve_fraction',      '0.10'::jsonb),
  ('prompt_upstream_hops',         '2'::jsonb),
  ('max_skill_tokens',             '20000'::jsonb),
  ('excerpt_max_files',            '12'::jsonb),
  ('excerpt_file_line_cap',        '400'::jsonb),
  ('excerpt_head_lines',           '200'::jsonb),
  ('excerpt_max_file_bytes',       '524288'::jsonb),
  ('excerpt_max_scan_files',       '20000'::jsonb),
  ('excerpt_provider_deadline_ms', '1500'::jsonb)
ON CONFLICT (key) DO NOTHING;
```

**No cache-mirror migration.** `document` with its body, `item_link`, `item`, `project.settings`,
`run_step.prompt_digest` and `run_step.trim_record` are all already mirrored, and
`upstream_summaries` reads nothing else. `cache_migrations/0002_orchestration.sql` stays ANA-2's.

**Material this document supersedes.**

| Superseded | By |
|---|---|
| `docs/ANA-9.md:764`'s column comment read as "hash of the seq-0 row" | §4.7: ANA-4's "over the assembled prompt text" is operative, and the `0002` `COMMENT` above replaces the shorthand in the database itself |
| `docs/ANA-9.md:817-818`'s "one `prompt_template` version 1 per phase name" | §4.6: plus one each for the reserved names `judge` and `handoff`, ten per project |
| `docs/ANA-9.md:916-942`'s §7.3 query | §4.3's amended query: `MIN(depth)` dedup, an `in_scope` column, the LATERAL join no longer gated by scope, and final ordering redone in Rust |
| `docs/ANA-4.md:355`'s `set_step_usage(step, usage, prompt_digest)` third parameter | §4.4: `set_step_prompt` writes the digest at stage 3; MOD-2 passes `None`, and dropping the parameter is a later cleanup |
| `docs/ANA-4.md:1163-1189`'s `htui-agent` module list | §8: one addition, `src/excerpt.rs` |
| `docs/ANA-4.md:1267-1277`'s "One migration, `0002_agent_probe.sql`" and its "Nothing else changes" | §9: the file keeps its name and gains the five `COMMENT ON COLUMN` statements and the one `app_setting` INSERT above, after ANA-4's `ALTER TABLE agent_box ADD COLUMN probe JSONB;` |
| `crates/htui-core/src/fixtures.rs:1244-1247`'s bare `prd` section name | §5.2's vocabulary: `documents:prd` |

**`HANDOFF.md` changes this verdict implies, at close-out.**

| Item | Change |
|---|---|
| **ANA-5** | the open checklist line is deleted; `docs/decisions/ana/ana-5.md` is written; `DECISIONS.md` gains its index line; the summary table's ANA count drops from 3 to 2; the top status line gains an ANA-5 recap and drops its oldest mention |
| **MOD-2** | gains `R-PRM-1..3` in its requirement list, which no open item currently cites and `R-NF-4` requires; gains "prompt builder in `htui-core::prompt` per `docs/ANA-5.md` §8, `htui-agent::excerpt` for the filesystem half, six store methods plus `set_step_prompt`, and the ANA-5 sections of `0002_agent_probe.sql`"; **the "Blocked on ANA-5" clause is removed**, so MOD-2 becomes startable and MOD-4 becomes blocked on MOD-2 only, as its line already says |
| **MOD-9** | gains "template save validation calls `htui_core::prompt::template::parse` per `docs/ANA-5.md` §4.1; `judge` and `handoff` are reserved names whose role is derived from the row name" |
| **MOD-15** | gains "seed ten `prompt_template` rows per project, the eight phase names plus the reserved `judge` and `handoff`, bodies per `docs/ANA-5.md` §5.4 (amends `docs/ANA-9.md` §5.10); the phase editor refuses the two reserved names; the Settings tab exposes `docs/ANA-5.md` §5.3's keys" |
| **MOD-13** | gains a pointer: "`touched_paths` is tier 1 of `docs/ANA-5.md` §4.5's excerpt ranking, so the `repo:glob` validation it owes is what makes excerpts useful" |
| **MOD-7** | gains a pointer: "`repo_box_path` rows and the box probe are what turn `docs/ANA-5.md` §4.5's fallback root and §4.2's box projection from degraded into complete" |
| **ANA-3** | gains "the seam is `ExcerptProvider` in `htui-core::prompt::excerpt` per `docs/ANA-5.md` §4.5, propose-only, fail-open with a deadline, read-only and non-LLM" |
| **MOD-11** | gains "the `R-MCP-2` `box_profile` tool returns `docs/ANA-5.md` §4.2's projection so the tool and the prompt section agree" |

---
## 10. Open for the maintainer

Each has a default this document adopts, so MOD-2 is not blocked on any of them; each is a choice a
verdict could reasonably have gone the other way on.

| # | Question | Default adopted |
|---|---|---|
| 1 | `token_budget`. The two figures in the tree are an order of magnitude apart: `120_000` in the demo `project.settings` (`crates/htui-core/src/fixtures.rs:489`) and `12000` in the superseded ANA-8 workspace descriptor (`docs/ANA-8.md:548`). This is a cost and quality trade, not an analysis result, which is why ANA-2 said "ANA-5 owns the number" and this document hands it back with a default. | **120 000**, the only figure the shipped fixture asserts, with a 10% reserve so the effective target is 108 000. |
| 2 | Upstream hops, and whether it is configurable at all. `R-PRM-1` permits one or two; the two superseded predecessors both hardcode two; Sweep's one-degree evidence argues for restraint. | **2**, resolved through `project.settings.upstream_hops` then `app_setting.prompt_upstream_hops`, clamped to `1..=2`. The trim ladder degrades depth 2 first, so a dense graph self-corrects. |
| 3 | Calibrating the token estimator from observed `run_step.usage`. It is the obvious accuracy win and it makes the prompt bytes a function of history unless the constants are pinned into the run snapshot. | **Not in v1.** Fixed per-family constants, `chars-v1`. A later item adds calibration together with the pin. |
| 4 | The ten default template bodies of §5.4. These determine agent behaviour on every run and are prompt engineering rather than analysis. | **As written in §5.4.** The two machine-read parts, the `review` front matter and the `judge` verdict block, are not negotiable because MOD-4 parses them; the prose around them is. |
| 5 | How much of the box profile enters the prompt. `box.settings` is orchestrator policy, the tag arrays are `R-ORCH-10` matching vocabulary, and the surviving prior art renders hostname, OS, compilers, tools, shell and quirks (`docs/ANA-1.md:491-508`). | **§4.2's projection**: hostname, os, arch, cpu, ram, gpu, htui version, up to 24 `box_tool` name-and-version pairs, quirks. No tags, no settings, no tool paths. |
| 6 | Excerpt caps: `max_files` 12, whole-file threshold 400 lines, head window 200 lines, 512 KB skip. They interpolate between Agentless `--top_n 3`, Continue's `nFinal 5`, Sweep's 1 500-character chunk, Aider's 1 024-token map and Claude Code's 2 000-line `Read`. | **As in §5.3.** They are starting defaults to tune against real runs, not derived constants, and they are `app_setting` keys precisely so tuning costs no code. |
| 7 | Unicode NFC normalisation before hashing. It would need a direct `unicode-normalization` edge on `htui-core` (the crate is already in `Cargo.lock` transitively, so no new build-graph node), and one published position argues against normalising before hashing at all. | **Off in v1.** LF normalisation is the load-bearing rule and it is applied; composition differences do not arise between two renders of the same database rows. |
| 8 | Whether an excerpt that trips the scrubber should refuse the run, or be excluded silently. §4.5's denylist removes the file class; this is about the residue. | **Refuse.** `R-SEC-3` is fail-closed and §4.7 puts the scrub before the send, so a residue means the denylist has a hole worth seeing. The alternative, dropping the file quietly, hides the hole. |
| 9 | Line numbers in excerpts. They cost roughly 4 to 6% of the excerpt budget; Claude Code's `Read` returns `cat -n` form and its edit tools address lines, while Aider deliberately omits them because its edit format is search and replace. | **On.** `htui` drives both agent families and the more demanding convention is the safe default. If the two families ever want different answers, the rendered text becomes agent-dependent and so does `prompt_digest`, which is why this is a maintainer question and not an implementation detail. |
| 10 | A second `spec_digest` over an RFC 8785 canonical JSON of the assembler's inputs, beside the text digest. | **Not added.** `prompt_digest` answers `R-ORCH-11`'s question. A second field would answer "were the inputs the same", which nothing currently asks. |
| 11 | Whether `htui` emits provider `cache_control` breakpoints to earn prompt-cache hits. | **No.** The stable-first ordering is taken because it is free; explicit breakpoints would couple `htui` to the Anthropic Messages API rather than to the `claude` CLI and ACP transports ANA-4 chose. |
| 12 | The migration number (§9). Folding into `0002_agent_probe.sql` makes that file carry two ANAs. | **Fold.** The alternative edits a concluded ANA's written SQL and four `HANDOFF.md` references to rename ten INSERT rows. |

---

## 11. Risks

| # | Risk | Mitigation |
|---|---|---|
| 1 | **The estimator is wrong by 20% and a step overflows the model's real context window.** `htui` has no exact counter and no model-window table, so it cannot clamp a configured budget to a real limit. | The 10% reserve plus the conservative per-family default absorb the common case; an overflow surfaces as an agent-side error the driver records rather than as silent truncation, which is the same posture Aider takes ("aider never enforces token limits, it only reports token limit errors from the API provider"). `trim_record.estimator` makes a systematic error diagnosable from stored data rather than from a rerun. |
| 2 | **A skill grows and starts refusing steps.** `max_skill_tokens` refuses rather than degrades, so a maintainer who pastes a long document into a skill breaks every bound phase at once. | The refusal names the figure and the cap, the Skills tab shows a per-skill token estimate at save, and the cap is an `app_setting` so raising it costs no release. Refusing is still right: the alternative silently changes behaviour per box, which is what `R-ID-5` exists to prevent. |
| 3 | **Excerpts help less than they cost.** A published study finds LLM-generated repository context files "actually degrade task success rates while inflating inference costs by over 20%" by inducing over-exploration. | Excerpts are last in the keep order, budget-derived rather than budget-funded, capped at 12 files, and framed read-only in the words Aider proved. If the maintainer concludes they are net negative, `excerpt_max_files = 0` turns them off with no code change. |
| 4 | **`touched_paths` is empty everywhere today**, so tier 1, the highest-precision signal, contributes nothing on a real repository until MOD-13's editor lands and people use it. | Tiers 3 and 4 need no declaration at all, and tier 2 arrives with MOD-4. The excerpt audit records `considered` and `selected` per run, so the value of each tier is measurable rather than argued. |
| 5 | **`repo_box_path` has no writer**, so at MOD-2 time the fallback root may not exist on a box where isolation has not created a tree either. | Fail-open by design: no readable root means no excerpt section and a `no_path` note, never an error. §12 criterion 12 tests exactly that. |
| 6 | **The judge's `task` section replays a stored prompt, so the judge prompt contains a whole other prompt** plus N candidate blocks, against a budget resolved from the judged phase. | The per-candidate isolate cap divides the residual equally and the diff-stat-only fallback is already ANA-2's rule. A judge prompt that still does not fit fails the judge step, and ANA-2 already makes a judge failure park for human selection rather than fail the run (`docs/ANA-2.md:848-866`). |
| 7 | **The `review` front-matter grammar is new and nothing enforces it agent-side.** A model that writes prose before the `---` breaks MOD-4's loop. | Unparseable is read as `request-changes`, so the failure mode is an extra loop iteration rather than a false approval, and ANA-2's no-progress predicate stops an unproductive loop anyway (`docs/ANA-2.md:735-739`). The template states the format verbatim and MOD-4's parse error is surfaced in the gate note. |
| 8 | **Six new store methods oblige `MemStore` and the conformance suite**, and `CASES.len()` is asserted in two files, so a case cannot be added without a Postgres run. | Every method lands with its case in the same commit, as ANA-2 already priced for its sixteen. `upstream_summaries` gets three cases because it has three render states. |
| 9 | **A future section leaks an absolute path or a timestamp** and silently breaks fan-out identity, which no equality test between two assemblies of the same spec would catch. | §12 criterion 5 greps the rendered text for the tree root, the run id and the step id rather than only comparing digests, so a leak fails a test rather than a run. |
| 10 | **`insta` as an `htui-core` dev-dependency** widens the domain crate's test surface and makes every template rewording a snapshot review. | That is the intended cost: golden prompts are the only way `prompt_digest` means anything, and localising the churn to `.snap` files is better than hand-edited hex strings. `insta` is already in the workspace lock through `crates/htui`. |
| 11 | **MOD-9 can ship a template editor before MOD-2 lands the validator**, since MOD-9 is not blocked on this document. | The validator is MOD-2 build step 1 precisely so it exists first, and it lives in `htui-core`, which MOD-9 already depends on. If MOD-9 does land first, its editor saves unvalidated bodies and §4.1's stage-3 failure catches them, loudly. |
| 12 | **Two prompt-shaped fields, `trim_record` and `sections[]`, can drift.** | One is derived from the other by a `map`, in one function, tested element for element (§12 criterion 8). Nothing constructs `sections[]` independently. |

---

## 12. Validation criteria (for MOD-2's prompt builder)

1. `parse(Phase, body)` accepts every body §5.4 seeds for a phase name, `parse(Judge, ..)` accepts
   the `judge` body and `parse(Handoff, ..)` the `handoff` body; each rejects the other two roles'
   bodies with `WrongRole` naming the offending token.
2. `{{ item }}`, `{{itme}}`, `{{Item}}` and a bare `{{` each produce the documented
   `TemplateError` variant with a byte offset that indexes the offending token; `{{{{item}}}}`
   renders the literal `{{item}}` and produces no section.
3. A body naming a placeholder outside the closed set, written directly to `prompt_template` and
   then assembled, fails the step at stage 3 with `unknown prompt placeholder: {{foo}}`, sets the
   item to `blocked`, and starts no session.
4. A position-0 phase with `input_kinds = []` on a root item with no upstream links, no excerpts and
   attempt 1 assembles successfully; the rendered text contains no `<section name="documents:`, no
   `<section name="upstream">` and no `<section name="excerpts">`; `sections[]` has entries only for
   the sections that contributed bytes; and no run of three or more consecutive newlines survives.
5. Assembling the same `PromptSpec` for `fanout_index` 0, 1 and 2 yields three identical digests,
   and the rendered text contains none of: the tree root path, the run id, the step id, the
   `fanout_index`, or any string matching an absolute Windows or POSIX path.
6. The same fixture with every document body and excerpt converted to CRLF yields the identical
   digest as the LF version, and the text handed to `AgentDriver::start` contains no `\r`.
7. An item whose upstream graph is a diamond (`A blocked_by B`, `A origin C`, `B blocked_by D`,
   `C blocked_by D`) renders `D` exactly once at depth 2; the same graph read through `PgStore` and
   through `CacheStore` renders byte-identically.
8. `trim_record.sections` mapped to `{name, tokens_after, trimmed}` equals the `prompt` payload's
   `sections[]` element for element and in order, for every fixture including the oversize one.
9. A fixture 1.5x over budget produces `estimated_after <= target`; the trimmed sections are exactly
   those at or before the deficit-clearing point in §4.4's order; `template`, `skills`, `box` and
   `command_queue` all have `strategy: "none"` and `trimmed: false`; and every elision marker's two
   numbers equal that section's `elided_lines` and `elided_bytes`.
10. A fixture whose protected set alone exceeds `target` fails before any session starts with
    `prompt budget too small`; a fixture whose skills exceed `max_skill_tokens` fails with
    `skills exceed max_skill_tokens`, and neither writes a `prompt` event.
11. `run_step.prompt_digest` and `run_step.trim_record` are both non-NULL after stage 3 and before
    the session's first event, and `prompt_digest` equals the `digest` key of the `prompt` payload
    at `seq = 0`, which is ANA-4 validation criterion 3 read from this side.
12. With no `run_step_tree` row and no `repo_box_path` row for any repo, the prompt assembles, the
    excerpt section is absent, and `trim_record.excerpts.roots` records `no_path` per repo.
13. Each of the four `FakeExcerptProvider` behaviours (candidates, error, panic, past deadline)
    leaves a valid assembled prompt; the three failing ones appear in
    `trim_record.excerpts.provider_set` with a non-`ok` status; and the built-in ranker's own output
    is unaffected.
14. A file matching the secret denylist (`.env`, `id_rsa`, `*.pem`) is never selected even when it
    is the only path under a `touched_paths` prefix, and its absence is recorded rather than silent.
15. A skill bound at both project and phase level appears exactly once, at the phase binding's
    version; two skills with equal `position` render in `skill.name` byte order; and the rendered
    order is unchanged by the order the store returned them in.
16. The `review` body seeded by MOD-15 produces, for a fixture agent that follows it, a document
    whose first three lines parse as §4.6's front matter; MOD-4's parser reads `request-changes`
    from it and reads an unparseable document as `request-changes` too.
17. A judge prompt built for three candidates contains `judge_task` once and `judge_candidate:0`,
    `:1`, `:2` in `fanout_index` order; the second call's text differs from the first only in
    candidate order; and a candidate whose diff exceeds its isolate share renders with a diff stat
    and no unified diff.
18. A handoff prompt is persisted as a `follow_up` event at the next `turn`, not as a second
    `prompt` row; `run_step.prompt_digest` is unchanged by it; and `step_summary` contains no
    verbatim `assistant_text` beyond the windowed tail of the final message.
19. Every new store method has a `conformance::CASES` entry and `MemStore` and `PgStore` return
    equal results for all of them. Separately, outside `CASES` because `CacheStore` implements
    `ReadStore` only, `CacheStore` returns equal results to `PgStore` for the four `ReadStore`
    additions.
20. `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo fmt --all --
    --check`, `cargo doc --workspace --no-deps` with zero warnings, and
    `cargo sqlx prepare --check -- --all-targets --all-features` from `crates/htui-store` all pass
    with the prompt builder in place, and `cargo tree` shows no new workspace dependency.
21. Every item below is closed by a build or a live run before MOD-2 is marked done. These are the
    "**Unverified - MOD-2 must confirm**" lines from the body, collected in the shape ANA-4 §11.14
    uses:
    - how the "no workspace, current project" bound is expressed in Rust, given that the shipped
      `Scope` mandates a `workspace_id` and `from_workspace` is its only constructor: a
      `Scope::single_project` constructor or a separate `PromptScope` argument (§4.3 step 1, §8);
    - whether `conformance::run_case` can be generalised over `ReadStore` so the four `ReadStore`
      additions run against `CacheStore`, or whether the mirror comparison needs its own harness
      (§8, criterion 19);
    - the real per-family characters-per-token constants for `chars-v1`, measured against
      `run_step.usage` from the first real runs; §4.4's 3.5/3.0 and 4.0/3.3 are read off published
      ranges and nothing in the tree validates them (§4.4, Open for the maintainer 3);
    - whether `set_step_usage`'s third parameter can be dropped once `set_step_prompt` exists, or
      whether ANA-4's recorder still needs it for the chat path (§4.4, §9);
    - whether the `agy` and `claude` families want different excerpt renderings (line numbers on or
      off), which would make `prompt_digest` agent-dependent (Open for the maintainer 9).

---

## 13. Sources

**Local repository** (branch `main`, HEAD `f989da5`, read 2026-09-06):

`docs/REQUIREMENTS.md` (read in full; §1 `R-ID-3..7` lines 31-42, §3 `R-ENT-5` 75-77, `R-ENT-9`
94-97, `R-ENT-12` 103-105, `R-ENT-13` withdrawn 106-107, §5 `R-AGT-1` 129-131 and `R-AGT-5` 145-146,
§6 `R-ORCH-1..3` 154-162, `R-ORCH-7` 172-175, `R-ORCH-11` 185-186, §7 `R-HIS-1..3` 194-198, §8
`R-PRM-1..4` and `R-SKL-1..4` 202-217, §9 `R-SEC-2..3` 224-228, §10 `R-MCP-2` 236-237 and `R-MCP-4`
242-243, §11 `R-TUI-4` 254-256, `R-TUI-7` 262-263, `R-TUI-9` 266-267, §12 `R-LATER-7` 283-284, §13
`R-NF-1..4` 288-292, §15 superseded material 301-308);
`CONCEPTS.md` (read in full; "Prompt economy" 62-72, "Single source of truth" 18-34);
`docs/ANA-9.md` (§4.3 event rows and the `prompt` payload contract at 233, §4.4 mirror contents
306-311, §5.0 forward-only migration rule 347-350, §5.4 `step_graph_phase` 509-529 and
`prompt_template` 539-549 with the ANA-5 delegation at 544, §5.5 `item` 578-602 with `touched_paths`
at 591, `item_link` 617-628 and `document` 641-656, §5.6 skills 658-692 with the `R-SKL-2`
resolution at 691-692, §5.8 `run_step` 749-772 with `prompt_digest` at 764 and `trim_record` at 765,
§5.9 `app_setting` 801-812, §5.10 seed 814-818, §6.1 store traits 824-847, §7.3 upstream summaries
916-942 quoted and amended in §4.3, §7.4 ready items 944-960, §8 what this replaces 973-986 incl.
the CTE row at 986, §9 phasing 990-1016 incl. the MOD-9 ownership clause at 1012 and the stale
migration line at 1010);
`docs/ANA-2.md` (scope note and the `R-PRM` delegation 12-14, §2 invariants 98-146 incl. invariant 2
at 109-113 and invariant 8 at 135-138, §4.1 field-resolution chains 281-288 incl. `token_budget` at
284 and `template_version` at 285, `ResolvedPhase` 325-348, seed amendments 307-319, §4.2 the six
stages 382-389, the input-document SQL 395-406, the loser rule 408-411 and the missing-kind hard
failure 413-416, §4.3 `done` versus `closed` 592-594, §4.4 the forwarded-set table 724-733 incl. the
two ANA-5 injections at 730-731 and the no-progress predicate at 735-739, §4.5 the judge in full
766-884 incl. what the judge receives 822-835, the verdict block 837-846 and judge-failure
bookkeeping 848-866, §4.6 isolation 885-1008 incl. the scratch root at 905 and commit capture at
955-973, §4.7 `touched_paths` qualification 1024-1029 and `PathPrefix` 1074-1077, typed settings
1115-1147, §4.8 promotion 1160-1241 incl. the handoff prompt at 1203, §4.9 the no-prompt-digest-replay
verdict 1309-1315, §4.10 close-out 1380-1397, §5.1 the template pin at 1461 and the topology rule
1477-1485, §5.4 `app_setting` keys 1507-1525, §6.2 `RunStepSummary` additions 1573-1580, §8 crate
layout 1639-1783 incl. the store split rule 1699-1705, the read and write tables 1707-1756 with
`finish_step` at 1745, and "No glob crate" at 1783, §9 phasing 1787-2038 incl. the ANA-5 row at 1842,
the MOD-13 and MOD-15 rows at 1845-1846, migration numbering 1848-1857 and `0003` in full 1859-2000,
§10 open for the maintainer 2042-2057, §11 risks incl. risk 10 at 2073);
`docs/ANA-4.md` (scope note incl. "ANA-5 supplies the prompt this document transports" at 10-11, §2
invariants 56-82 incl. scrub-before-persist at 75-77 and determinism at 81-82, §4.1 in full 181-372
incl. `SessionSpec` 204-217, `DriverCaps` 219-230, `AgentDriver::start` 233-243, coalescing 317-330,
`prompt_digest` 338-340, the scrub-then-persist order 346-349, the store seam 351-358 and the
trimming boundary 368-372, §6.1 the ACP event table incl. the `prompt` row at 1012, §7 usage
reconciliation 1089-1103 and the cap-as-guard-rail framing at 1143-1150, §8 crate layout 1154-1236
incl. the dependency test 1156-1161, the module list 1163-1189, the dependency table with `sha2` at
1209 and the test strategy 1214-1236, §9 phasing 1240-1283 incl. "What ANA-5 must provide" 1257-1260
and `0002_agent_probe.sql` 1267-1277, §11 validation incl. the digest criterion 1345-1347);
`docs/ANA-1.md` and `docs/ANA-8.md` (superseded per `docs/REQUIREMENTS.md` §15; read only where they
touch prompts: ANA-1's `<HOST_ENVIRONMENT>` block 491-508 and its recursive CTE 342-406, ANA-8's
workspace-bounded CTE 201-292 with the stub render at 211-212 and the two-hop bound at 208, and
`prompt_budget_tokens` 12000 at 548);
`HANDOFF.md` (read in full; the ANA-5 item 37-43, ANA-3 47-49, MOD-2 53-59, MOD-4 60-72, MOD-7
73-78, MOD-9 79-82, MOD-11 87-92, MOD-13 101-108, MOD-15 113-120, the summary table 142-149);
`.claude/rules/workflow-docs.md` (document law: file roles, ID minting, the index line format and
its parse regex, the lifecycle and the close-out bookkeeping this document triggers, the
live-coordinates rule at 123-125);
`docs/decisions/ana/ana-2.md`, `docs/decisions/ana/ana-4.md`, `docs/decisions/ana/ana-9.md`,
`docs/decisions/mod/mod-1.md`, `docs/decisions/mod/mod-6.md` (the migration prompt and the "schema
is newer than this htui" refusal, the seed, the hermetic Postgres test protocol and the validation
command list).

`crates/htui-store/migrations/0001_init.sql` (`step_graph_phase` 228-248 with `template_name`,
`template_version` and `token_budget` at 242-244, `prompt_template` 266-276 with the ANA-5 comment at
271, `item` 310-338 with `touched_paths` at 323, `item_link` 357-368, `document` 389-400,
`skill`/`skill_version`/`skill_binding` 406-441, `run_step` 472-499 with `prompt_digest` at 487 and
`trim_record` at 488, `session_event` 513-535, `app_setting` 557-561, the `updated_at` trigger loop
and its exclusions 563-580; zero INSERT statements in the file);
`crates/htui-store/cache_migrations/0001_mirror.sql` (the not-mirrored list 24-25, `item` with
`touched_paths` at 80, `item_link` without `deleted_at` 86-90, `document` with its body 97-101,
`run_step` with `prompt_digest` and `trim_record` at 117; no `repo_box_path`);
`crates/htui-core/src/model/{kind,document,item,link,run,box_,hierarchy,scope,event,agent,ids,mod}.rs`
(`StepGraphPhase` 88-124 and `PromptTemplate` 139-157, `Document` 10-30 and `DocumentHead` 33-51,
`Item` 39-78 with `touched_paths` at 64-65, `LinkKind` 10-23 with the direction doc at 11-12,
`RunStep` 126-169 with `prompt_digest` 154-155 and `trim_record` 156-158, `RunStepSummary` 184-209,
`BoxRow` 25-65 and `BoxTool` 67-81 and `BoxInfo` 83-92, `Project` 28-51 and `ProjectRef` 111-122,
`Scope` 9-42, `EventKind::Prompt` 12-14 and `SessionEvent` 56-78, `Agent` 29-53, the ID newtypes
100-112, and the `pub use` surface 94-117 confirming no skill type exists);
`crates/htui-core/src/store/{traits,error,mem,conformance}.rs` (`ReadStore` 18-33 with
`links(id, hops)` at 24, `WriteStore` 38-51 and the "runs, steps, events, ..." comment at 50,
`StoreError`, `MemStore::State` 39-85 with the dead-coded templates at 55-66 and no skill field,
`State::link_graph` 344-397, the conformance `CASES` 19-37 and the "both directions" case doc at 552);
`crates/htui-core/src/fixtures.rs` (the determinism doctrine 1-10, `TEMPLATE_NAMES` 546-556, the
seeded phase construction 601-626, the template body with the literal `{{item}}` at 637,
`project.settings.token_budget` 120 000 at 489, empty `touched_paths` at 924, the run-step fixtures
1141-1220, `PROMPT_DIGEST` and its rationale 1223-1226, and the only shipped `sections[]` example
1237-1249);
`crates/htui-store/src/pg/{mod,read,demo}.rs` (`SEEDED_SETTINGS` 44-48, `schema_version` 341-345 and
the `MIGRATOR.version_exists` refusal, the `links` CTE 155-228 esp. 163-169, the body-less
`documents` 230-251, the inherent-read rationale 378-382, the demo loader's untouched-table list
21-36);
`crates/htui-store/src/{backend,identity}.rs`, `crates/htui-store/src/cache/read.rs`,
`crates/htui/src/{store_worker,testkit}.rs` (the reserved `StepEvents` request at 42-43, the inline
`settle()` at 1-10), `crates/htui/src/ui/tabs/skills.rs` (the stub);
`Cargo.toml` (workspace dependency list, `resolver = "3"`, `rust-version = "1.85"`, the lint table),
`Cargo.lock` (absence probe: `minijinja`, `tera`, `handlebars`, `liquid`, `askama`, `tiktoken-rs`,
`tokenizers`, `tree-sitter`, `globset`, `ignore`, `walkdir`, `git2`, `gix` all absent; `similar
2.7.0` and `regex 1.13.1` transitive only; `serde_json 1.0.151` whose dependency list is
`itoa, memchr, serde, serde_core, zmij` with no `indexmap`, so `preserve_order` is off;
`unicode-normalization 0.1.25` present transitively),
`crates/htui-core/Cargo.toml` (dev-dependencies: `tokio` only; optional `sqlx` feature) and
`crates/htui/Cargo.toml` (dependencies `htui-core` and `htui-store`; dev-dependencies `insta`,
`tokio`, `tempfile`), `crates/htui-store/src/pg/write.rs:27` and
`crates/htui-core/src/store/mem.rs:727` (the only two `impl WriteStore`),
`crates/htui-store/src/cache/read.rs:208` (`impl ReadStore for CacheStore`, no `WriteStore`),
`rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`.

**Template engines and the placeholder contract (§4.1).**

- https://docs.rs/minijinja/latest/minijinja/enum.UndefinedBehavior.html (the four undefined tiers)
- https://docs.rs/minijinja/latest/minijinja/struct.Environment.html (`add_template_owned`, the
  auto-escape callback, `set_fuel`, the recursion limit)
- https://docs.rs/minijinja/latest/minijinja/struct.Error.html (`kind`, `detail`, `line`, `range`)
- https://github.com/mitsuhiko/minijinja/issues/871 (strict mode does not name the undefined variable)
- https://keats.github.io/tera/ and https://github.com/Keats/tera/issues/120 (strict-only, no toggle)
- https://docs.rs/handlebars/latest/handlebars/ and
  https://docs.rs/handlebars/latest/handlebars/fn.no_escape.html (the default escape set)
- https://docs.rs/liquid/latest/liquid/ (allow-list parser; no documented Rust strict-variables)
- https://github.com/askama-rs/template-benchmark (compile-time engines cannot reload a body)
- https://github.com/advisories/GHSA-cpwx-vrp4-4pq7 and
  https://reference.langchain.com/python/langchain-core/prompts/prompt/PromptTemplate (why an
  untrusted-template surface is a real hazard in the Python prior art)
- https://github.com/cline/cline/tree/main/src/core/prompts/system-prompt (a production
  `{{SECTION}}` skeleton with per-model component overrides)
- https://code.claude.com/docs/en/skills (`$ARGUMENTS` and its documented degenerate cases)
- https://swe-agent.com/latest/reference/template_config/ and
  https://swe-agent.com/latest/config/config/ (a closed, per-role named template set; the pre-1.1.0
  config-merge bug)
- https://mini-swe-agent.com/latest/advanced/yaml_configuration/ (truncation policy inside the
  user-editable template)
- https://github.com/RooCodeInc/Roo-Code/pull/11387 and
  https://docs.roocode.com/features/footgun-prompting (full system-prompt override, documented as a
  footgun and then removed)
- https://dspy.ai/diving-deeper/adapters/ (the `[[ ## field ## ]]` delimiter rationale)

**Section model, budgeting and trimming (§4.2, §4.4).**

- https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices
  (longform data at the top, queries at the end, XML document tags)
- https://platform.claude.com/docs/en/build-with-claude/token-counting (the endpoint, its
  "estimate" caveat, the rate-limit tiers, and the ~30% tokenizer shift at Claude 4.7 and later)
- https://reference.langchain.com/python/langchain-core/messages/utils/trim_messages (`include_system`,
  the approximate counter recommended on the hot path, the keep-the-separator splitter invariant)
- https://deepwiki.com/continuedev/continue/4.4-message-compilation-and-streaming and
  https://github.com/continuedev/continue/issues/9231 and
  https://github.com/continuedev/continue/issues/5166 (preserve the system message, the structured
  pruning report, the `actualTokenCounts` proposal, the negative-context-budget warning)
- https://aider.chat/docs/troubleshooting/token-limits.html (estimates, never enforced locally)
- https://github.com/anysphere/priompt/blob/main/README.md (`<isolate>`, `<first>`, `<empty>`,
  sourcemaps, and the authors' own "priorities are the wrong abstraction" retrospective)
- https://github.com/openai/codex/pull/6476 (head-only truncation loses the tail; the marker's
  own overhead must be budgeted)
- https://github.com/anthropics/claude-code/issues/17611 (per-tool `maxBytes` / `preserveStart` /
  `preserveEnd` and a marker reporting bytes and lines)
- https://github.com/anthropics/claude-code/issues/27189 (`/context` showing a default rather than
  the resolved budget)
- https://codex.danielvaughan.com/2026/04/14/context-compaction-deep-dive-codex-cli-claude-code-opencode/
  and https://claudefa.st/blog/guide/mechanics/context-buffer-management (what compaction keeps and
  what it discards)
- https://geminicli.com/docs/cli/gemini-md/ (concatenated instruction hierarchy; "context bloat" as
  the dominant failure mode)
- https://github.com/OpenHands/OpenHands/pull/7516 (duplicate-inclusion of a triggered instruction
  block, the reason skills dedup by id)

**Excerpt selection (§4.5).**

- https://aider.chat/2023/10/22/repomap.html and https://aider.chat/docs/repomap.html and
  https://github.com/Aider-AI/aider/blob/main/aider/repomap.py (tree-sitter tags, personalized
  PageRank, the identifier filters at 493-494, the path-component personalization at 437-445, the
  composite sort key at 548-550, the sampled token estimate at 90-101, the `RecursionError`
  fail-open at 143-145, the filename-ordered render at 759)
- https://github.com/Aider-AI/aider/blob/main/aider/coders/base_prompts.py (the read-only framing)
- https://github.com/Aider-AI/aider/blob/main/aider/coders/base_coder.py (content-adaptive fence
  selection at 609-633)
- https://aider.chat/docs/faq.html (weaker models try to edit the repo map)
- https://news.ycombinator.com/item?id=43163011#43164253 (Claude Code uses agentic search, not RAG)
- https://sourcegraph.com/blog/how-cody-understands-your-codebase (embeddings removed, BM25 adopted,
  N snippets as a function of snippet length) and
  https://sourcegraph.com/changelog/improved-context-fetching (more than one snippet per file)
- https://blog.sweep.dev/posts/autocomplete-context and
  https://github.com/sweepai/sweep/blob/main/docs/pages/blogs/chunking-2m-files.mdx and
  https://github.com/sweepai/sweep/blob/main/docs/pages/blogs/ai-code-planning.mdx (vectors to
  TF-IDF, ~1:5 chars-to-token for code, identifier case-splitting, one-degree expansion recall and
  precision)
- https://arxiv.org/html/2403.10059v1 (Jaccard matches learned retrievers for code)
- https://arxiv.org/abs/2407.01489 (Agentless localisation accuracy: 69.7% file, 52.0% function,
  35.3% line)
- https://arxiviq.substack.com/p/evaluating-agentsmd-are-repository (repository context files can
  degrade success and inflate cost)
- https://docs.continue.dev/reference/deprecated-codebase (`nRetrieve`, `nFinal`, `useReranking`,
  the LLM in the retrieval path this design forbids)
- https://cursor.com/docs/reference/ignore-file (gitignore-derived exclusion semantics)
- https://github.com/openai/codex/issues/26905 and https://github.com/openai/codex/issues/1205 (a
  missing external binary degrading search silently)
- https://github.com/oraios/serena (the LSP-backed candidate surface an `R-LATER-7` provider exposes)
- https://developers.openai.com/codex/cli (ripgrep-first agentic search, no index)

**Serialisation, caching and provenance (§4.7).**

- https://github.com/anthropics/skills/blob/main/skills/claude-api/shared/prompt-caching.md (the
  prefix-match rule, the silent-invalidator table, static-first ordering, deterministic tool
  serialisation)
- https://claude.com/blog/lessons-from-building-claude-code-prompt-caching-is-everything
- https://developers.openai.com/api/docs/guides/prompt-caching (1 024-token prefix, dynamic content
  last)
- https://git-scm.com/docs/gitattributes (`eol=crlf` on Windows, `eol=lf` elsewhere; LF in the index)
- https://www.rfc-editor.org/rfc/rfc8785.html and https://www.nakedpnl.com/glossary/canonical-json
  (JCS, and the correct reading that it does not apply Unicode normalisation)
- https://www.w3.org/wiki/I18N/CanonicalNormalizationIssues (normalise the joined string once, not
  per fragment)
- https://langfuse.com/docs/prompt-management/features/prompt-version-control and
  https://www.braintrust.dev/docs/evaluate/write-prompts (a version pointer plus a content digest is
  the norm in prompt provenance tooling)
- https://linear.app/docs/issue-relations and
  https://github.blog/changelog/2025-08-21-dependencies-on-issues/ (the only field conventions for
  rendering a linked-issue stub; no surveyed harness walks the relation graph for prompt context)
