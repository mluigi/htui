# ANA-5 - Prompt assembly: template contract, section model, upstream walk, trim order, file excerpts (done, 2026-09-06)

## Summary

Concluded how `htui` turns a resolved step into the one self-contained initial prompt an agent
receives: the prompt template placeholder contract and its save-time validation, the ordered
section model and its rendering, the upstream-summary walk within the workspace bound, the token
budget and the fixed trim order with its persisted trim record, deterministic file excerpt
selection with no external tool, the three named templates ANA-2 requires (`judge`, the review-loop
forwarded set, the promotion handoff prompt), and the byte-exact serialisation that makes
`run_step.prompt_digest` reproducible. Design in [`docs/ANA-5.md`](../../ANA-5.md). Requirements
addressed: `R-PRM-1..4`, `R-ID-5`; touched at their seams: `R-ID-4`, `R-ID-6`, `R-SKL-1..3`,
`R-ORCH-3`, `R-ORCH-5`, `R-ORCH-7`, `R-HIS-1`, `R-SEC-3`, `R-LATER-7`.

Method: four parallel Opus readers (requirements and prior ANAs, codebase seams, a web survey of
templating engines, token budgeting, trimming and reproducible serialisation, a web survey of
file-excerpt selection and upstream-context prior art), one Opus writer producing the document from
the four dossiers with its own repo reads, and one Opus verifier that re-checked 48 load-bearing
claims against primary sources and patched the document in place (17 corrected, 36 upheld, 5
collected as "Unverified - MOD-2 must confirm" in §12 criterion 21). Six agents total, per the
standing cap. The first launch was killed by the session usage limit after three readers finished;
the resume replayed them from the workflow cache and ran only the remaining three agents.

## What was decided

1. **Template placeholder contract (§4.1).** Plain `{{name}}` substitution over a closed, typed,
   per-role placeholder set (Phase, Judge, Handoff: 20 placeholders), hand-scanned in
   `htui-core::prompt::template` with no templating crate; `{{{{` is the only escape; no
   conditionals and no loops, because every section is pre-rendered in Rust and renders empty when
   its data is absent. Unknown or wrong-role tokens are rejected at save with a byte offset, and are
   a hard stage-3 step failure if a row reaches the assembler anyway. Version pinning is unchanged
   from ANA-2 (`judge` pins in the run snapshot, `handoff` resolves latest at promotion time).
   minijinja, tera, handlebars and liquid were surveyed and rejected on placement and dependency
   grounds, not on capability.
2. **Section model (§4.2).** `R-PRM-1`'s literal section order is preserved with the template as
   the frame rather than a leading section, so default bodies put the instruction last. One uniform
   `<section name="...">` wrapper with attributes; a closed vocabulary of ten step-prompt sections
   (`template`, `item`, `documents:<kind>`, `upstream`, `box`, `skills`, `excerpts`,
   `verify_failure`, `previous_diff`, `command_queue`) plus five judge and handoff names;
   `sections[] { name, tokens_after, trimmed }` in the parsed template's span order, with a
   zero-token entry for a dropped section. The box profile projection (hostname, os, arch, cpu, ram,
   gpu, `htui` version, up to 24 `box_tool` pairs, quirks; no tags, no settings, no tool paths) and
   the skill resolution order (phase binding over project binding, pinned or latest) are fixed. Raw
   transcripts of other items are blocked structurally: `PromptSpec` has no event field.
3. **Upstream-summary walk (§4.3).** ANA-9 §7.3's query is amended at query level, not DDL:
   `MIN(depth)` dedup (the shipped CTE emitted an item twice when reachable at two depths), an
   `in_scope` projection column (the shipped query could not distinguish "out of workspace" from
   "in workspace, no summary yet"), and the LATERAL summary lookup no longer gated by scope; final
   ordering redone in Rust by byte order. Hops default to 2 through `project.settings.upstream_hops`
   then `app_setting.prompt_upstream_hops`, clamped `1..=2`. Three render states: Summary block,
   in-scope Pending one-liner (`- no summary yet`), out-of-scope Stub one-liner (`R-PRM-2`). One new
   `ReadStore::upstream_summaries(id, hops, scope)` because every table involved is mirrored. The
   "current project when no workspace" bound cannot be expressed by the shipped `Scope` type; that
   is an "Unverified - MOD-2 must confirm" item.
4. **Token budget, trim order, trim record (§4.4, §5.1).** `R-PRM-3`'s list is read as
   **kept-first**: `template`, `skills`, `box` and `command_queue` (and handoff `failure_reason`)
   are protected, and a step that cannot fit them fails before a token is spent; sections lose
   tokens in the order excerpts, then upstream and `previous_diff`, then documents and
   `verify_failure`, then item body, each with a per-section floor and strategy (whole-unit drop,
   head+tail, diff-stat-only). Tokens are counted by a versioned deterministic heuristic
   (`chars-v1`, per agent family, fixed constants in v1), never an exact tokenizer or a network
   call. Budget chain phase, then `project.settings.token_budget`, then a new `app_setting`
   `token_budget` (120 000) with a 10% reserve. `run_step.trim_record` already ships in
   `0001_init.sql`, so no migration; its JSON shape is fixed, and `sections[]` is derived from it by
   one `map`. `R-MCP-4`'s "skill text instructs agents to use `command_run`" is rendered as its own
   per-phase section instead, with the reason stated.
5. **File excerpt selection (§4.5).** A five-tier deterministic signal union: `touched_paths`
   prefix (100), prior-attempt diff paths (90), literal path mentions in item and documents (70),
   path-component identifier match (50), hand-rolled TF-IDF tail (10 + 0..9); max weight, not sum;
   ties on path byte order. No glob, walker, git or tree-sitter crate; bytes read from
   `run_step_tree.path`, falling back to `repo_box_path`, falling back to no excerpts (fail-open).
   Caps as `app_setting` keys: `max_files` 12, 400-line whole-file threshold, 200-line head, 512 KB
   skip, 20 000-file scan. A secret-path denylist is applied at selection because `R-SEC-3`
   fail-closed is too late; an excerpt that still trips the scrubber refuses the run. Rendered in
   path order with repo-relative paths, line numbers and a read-only framing. The `R-LATER-7` seam
   is `ExcerptProvider` in `htui-core::prompt::excerpt`: propose-only, fail-open with a deadline,
   read-only, non-LLM. Fan-out identity (`R-ORCH-7`) is carried by the assembler running once per
   `(position, attempt)`; `local` is refused at `fan_out > 1`, `shared_serialized` is exactly the
   case the single assembly exists for.
6. **The three ANA-2 templates (§4.6, §5.4).** `judge` and `handoff` are seeded `prompt_template`
   rows with reserved names (MOD-15's phase editor refuses those two names; ANA-9 §5.10's seed
   becomes ten rows per project). The review-loop forwarded set is not a fourth row: it is
   `{{verify_failure}}` and `{{previous_diff}}` in the `implement` and `fix` bodies, with the review
   document arriving through ordinary `input_kinds`. The judge's `instruction` section is the
   template body's own literal tail; `{{task}}` is replayed from the lowest-`fanout_index`
   candidate's stored seq-0 prompt rather than re-assembled; the judge budget resolves from the
   judged phase with an equal per-candidate isolate cap and a diff-stat-only fallback. The handoff
   prompt persists as a `follow_up` at the next turn. Full default bodies for all ten names are in
   §5.4; the `review` front matter and the `judge` verdict block are machine-read by MOD-4.
7. **Stable serialisation for `prompt_digest` (§4.7).** sha256 lowercase hex over the assembled
   prompt **text** alone (not the payload, not `sections[]`), canonicalised as LF-normalised,
   BOM-stripped, blank-run-collapsed, exactly one trailing LF; that same string is what is sent,
   digested and persisted. The ANA-7 scrubber runs at assembly, before the digest, which resolves
   the ordering ANA-4 left open. Nine byte-exact ordering rules; no wall clock, no absolute path, no
   run or step id, and no `fanout_index` in a Phase-role prompt (the Judge role is the one named
   exception). Template version pinned and recorded in `trim_record.template`. NFC normalisation
   deferred. ANA-9 §5.8's "sha256 of session_event seq 0" column comment is superseded by ANA-4's
   reading.
8. **Crate and module placement (§4.8, §8).** No new crate. `htui-core::prompt` holds everything
   pure (template scanner and validator, render, estimate, trim, digest, the `ExcerptProvider` and
   `RepoReader` traits, the pure ranker) because it adds zero dependencies and is the only place
   MOD-2, MOD-4 and MOD-9 can all reach; `htui-agent::excerpt` holds the filesystem half
   (`FsRepoReader`, the walk, the gitignore-subset matcher, the skip rules), an addition to ANA-4
   §8's module list. Six store methods (four `ReadStore`, `bound_skills` and `box_profile` as
   `PgStore` inherent) plus `WriteStore::set_step_prompt`, which is the pre-flight digest and
   trim-record writer MOD-2 owns (ANA-2's `finish_step` is not it). New `Skill`, `SkillVersion`,
   `SkillBinding`, `BoundSkill` model types. Tests: golden prompts through `insta` as a new
   `htui-core` dev-dependency, digest-stability and fan-out-identity cases, `FakeRepoReader` and
   `FakeExcerptProvider`; the two `WriteStore` conformance targets are `MemStore` and `PgStore`, and
   the mirror comparison for the four `ReadStore` additions needs a second harness (unverified).
9. **Schema (§9).** No new migration file and no DDL. ANA-5's schema-side needs ride inside
   MOD-2's `0002_agent_probe.sql` as appended numbered sections: five `COMMENT ON COLUMN`
   statements and one idempotent `INSERT` seeding ten `app_setting` keys (§5.3). Rejected: a
   `0004_prompt.sql` (sqlx ordinal order makes `0004` applied before `0003` the refusal path) and
   renumbering ANA-2's `0003`. No cache-mirror migration. ANA-4's "one migration, nothing else
   changes" wording is superseded by the fold.

## Open for the maintainer (defaults adopted, MOD-2 not blocked on any)

Listed in `docs/ANA-5.md` §10: `token_budget` 120 000 with 10% reserve; upstream hops 2,
configurable and clamped; estimator calibration from observed usage deferred; the ten default
template bodies as written; the box profile projection; the excerpt caps; NFC off in v1; refuse on
scrubber residue in an excerpt; line numbers in excerpts on; no second `spec_digest`; no provider
`cache_control` breakpoints; the migration fold into `0002`.

## Residual gaps (named, not solved)

- The "current project when no workspace" bound of `R-PRM-1` has no representation in the shipped
  `Scope` type (`docs/ANA-5.md` §4.3 step 1, §12 criterion 21).
- The per-family estimator constants are reasoned interpolations, not measurements; calibration is
  deferred (§4.4, §10 item 3).
- `touched_paths` is empty everywhere and `repo_box_path` has no writer until MOD-13 and MOD-7 land,
  so tiers 1 and the fallback root contribute nothing on a real repository at MOD-2 time (§11 risks
  4 and 5).
- Whether `set_step_usage`'s `prompt_digest` parameter from ANA-4 can be dropped once
  `set_step_prompt` exists is unsettled (§12 criterion 21).

## Downstream items

- **MOD-2** now not blocked: gains `R-PRM-1..3`, the prompt builder in `htui-core::prompt` and
  `htui-agent::excerpt`, six store methods plus `WriteStore::set_step_prompt`, and the ANA-5
  sections of `0002_agent_probe.sql`.
- **MOD-4** consumes `assemble()` for the judge and handoff prompts, supplies `verify_failure` and
  `previous_diff` for the review loop; `RunStepSummary` gains `prompt_tokens` and `trimmed`.
- **MOD-9**: template save validation calls `htui_core::prompt::template::parse`; `judge` and
  `handoff` are reserved names; the placeholder tables are the editor's inline help.
- **MOD-15**: seeds ten `prompt_template` rows per project with the §5.4 bodies (amending ANA-9
  §5.10); the phase editor refuses the reserved names; Settings exposes the ten `app_setting` keys.
- **MOD-13**: `touched_paths` is tier 1 of the excerpt ranking.
- **MOD-7**: `repo_box_path` rows and a real probe complete the fallback root and the box profile.
- **MOD-11**: the `box_profile` read tool returns the §4.2 projection.
- **ANA-3**: the seam is `ExcerptProvider`; Serena's `.serena/` directory must live outside every
  `repo_box_path`.

## Commits

- docs(ana-5): conclude prompt assembly analysis (this close-out).
