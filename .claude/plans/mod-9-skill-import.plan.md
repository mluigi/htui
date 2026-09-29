# Plan: MOD-9 milestone 4 — `SKILL.md` import

> **Ported onto main's milestone 3 (2026-09-29, branch `mod-9-m4-port`).** This document was
> written on PR #10 (`origin/mod-9`, `4abb49a`..`bbac75c`) against **that branch's** milestone 3
> (`upsert_skill`, `remove_skill_binding`, `prompt/glob.rs`, `model/language.rs`, the attachments
> `matrix.rs`, migration `0008_skill_match`). The maintainer kept main's milestone 3 (PR #9) and
> took only milestone 4 from #10, so it is kept here as the record of milestone 4's decisions; its
> `file:line` references and count pins describe #10's tree, not main's. The code cites it as
> "import plan D*n*" so its numbers are not read as milestone 3's plan's. Where the port differs:
>
> - **Writers (D100, D101).** A new name is `create_skill` (the row and v1 together, main's D75);
>   a known one is `update_skill` under `skill.updated_at`, only when the description moved, then
>   `add_skill_version` over the head (`0` for none, main's D89) when the body did. Still no new
>   `WriteStore` method, SQL, `.sqlx` file, migration or dependency. A `Constraint` is reported in
>   the writer's own sentence.
> - **Names (D95, D97, D102).** Main's rule is `skill::validate_name`; `skills::REQUEST_NAMES` goes
>   5 → 6 (`import_skills`), so `IMPORT_NAME` is `REQUEST_NAMES[5]`. The key is **`I`**, not `i`:
>   main's `i` opens the rename form. `I import` takes the Browse hint room `h/l view` had.
> - **Prefill (D96).** Main has no matrix: the attachments pane's `open_form` prefills a row with no
>   attachment from the skill's head (a new row's pin is `latest`), comma-joining globs and
>   languages into main's single-line fields, and returns §7.3's hint as the notice. A stored row
>   is never re-seeded.
> - **Directory rule (D99), review finding.** A hidden tool root (`.cursor`, `.github`, `.kiro`)
>   was a rules directory, so its own depth-0 markdown was collected and `.cursor/rules/*.mdc` was
>   not. `RootShape` now says: a rules-named root gives its own `*.md` / `*.mdc`; a tool root gives
>   its rules-named child's, never its own; anything else gives `SKILL.md` only.
> - **Stem fallback (D95, OQ-22), review finding.** `<name>.instructions.md` names the rule
>   `<name>`, and `_` in a stem becomes `-`. A declared name is still taken as written; a stem with
>   a space or a capital is still refused.
> - **One batch, one name (D100), review finding.** A name met twice in one import is refused the
>   second time, naming the first file, instead of answering a spurious "changed while this import
>   was running".
> - **Report (D102).** It opens only over Browse; if an editor or form was opened while the walk was
>   in flight, the counts go to the notice so no draft is replaced.
> - **Symlinked root (R-41).** The typed path is read through a symlink (`metadata`, not
>   `symlink_metadata`), as R-41 says; symlinked directories under it are still not descended.
>
> The close-out is not #10's: main's MOD-9 keeps PRD row 5 ("Glob attachments fire") open, so
> MOD-9 is not closed by this milestone.

**Status: DRAFTED and FACT-CHECKED 2026-09-28; CONFIRMED by the maintainer the same day, with
OQ-21 through OQ-25 standing as drafted and OQ-26 taken as drafted (the reader accepts block scalars,
which widens ANA-22 §5.7's stated subset — recorded below and in the OQ).**
Branch `htui-mod-9-m4` (the worktree checkout of `mod-9`) at `f24c519`. Three verifiers ran against
the tree, the pins, and the written sources plus the real-file corpus on this machine; their verdicts
are in the "Verified claims" table at the end. **Falsified as first drafted and corrected in place:**
D92 (`Unterminated` has no real-file evidence), D95 (one `SKILL.md` without a `name`, not five), D99
(this repo holds three `SKILL.md` files, so a root import reaches them), the `allowed-tools`
mixed-comma rationale (refuted), the `gate.rs:64` reuse question (answered: not reusable), the
`omits_item` and `tempfile` line numbers, and the `R-NF-3` attribution. **One row raised a conflict
rather than a fix:** D93 accepts block scalars, which ANA-22 §5.7's *Against* column lists as
exotic YAML the reader rejects — that is **OQ-26**, for the maintainer.

**Source**: `HANDOFF.md:378-379` (the MOD-9 entry's closing line) and the PRD's milestone-4 row
(`.claude/prds/mod-9-skill-library-templates.prd.md:186`, "Existing skills come in | `SKILL.md`
import from a file or directory, frontmatter mapped per ANA-22"), plus the PRD's scope bullet at
`:128-129` ("a typed path names a file or a directory"). Requirement `R-SKL-4`
(`docs/REQUIREMENTS.md:269`, "Import from existing `SKILL.md` files as a convenience") is the whole
contract. `R-ID-6` (`docs/REQUIREMENTS.md:55-56`, "No LLM agent is ever in a sync, cache, import or
bookkeeping path") and `R-NF-3` (`:347-348`, "All long operations … run off the UI thread; the TUI
never blocks on network or subprocess I/O") constrain how it is built; the store-worker ownership rule
the plan leans on is the PRD's own Constraints block ("`R-NF-3` is enforced by ownership. Reads and
writes go through `StoreRequest` on the store worker", `prd:163-175`), not the requirement's words.

**Design source**: `docs/ANA-22.md` §5.6 (keep the frontmatter whole in `source`), §5.7 (a
hand-written reader, no YAML crate), §6 items 11 and 12 (what `source` carries; the skip list; the
same-name rule), §7.3 (the mapping table), §8's milestone-4 bullet, and §9's row "Frontmatter reader
rejects a real file" (Medium/Low, mitigated by "Per-key error with line number; the body still imports
if the maintainer accepts `always`").

**Complexity**: Medium. Two new pure modules in `htui-core` (a frontmatter reader and the §7.3
mapping), one new worker module that walks a directory and reads files, one new `StoreRequest`
variant and one new `StoreReply` variant, one new mode in the Skills view, and one prefill seam in the
attachments matrix. **No new `WriteStore` method, no new SQL, no `.sqlx` change, no migration, no new
dependency** — every one of those is a decision below with its evidence.

**Routing**: continues `/handoff-run MOD-9` (plan path; C2 fired, C1/C3/C4 did not; the maintainer
confirmed at the step-2 gate). Ultracode was offered and not taken; the implementer fan-out is
hand-driven. Staffing: session model for every step, `rust-reviewer`
(`.claude/workflow-config.json`) as the review gate.

**Gortex note**: the daemon's index is pinned to `/home/mluigi/projects/htui`, which is checked out
at `main` (`68c058f`) — **one milestone behind this worktree**. Every tree fact below was read
directly from `/media/projects/htui-mod-9-m4`. A `git_ref` view on `refs/heads/mod-9` does serve the
milestone-3 tree and is usable read-only, but the Gortex MCP `edit` path writes to the primary
checkout: **no implementer may mutate through Gortex in this worktree.** Edit the worktree files
directly.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts, so implementation is not blocked.

- [ ] **OQ-21 — The directory walk: hand-written, or `walkdir`?** `walkdir 2.5.0` is already in
  `Cargo.lock` and already compiled for this workspace, but only `htui-orch` depends on it
  (`Cargo.toml:110`, `crates/htui-orch/Cargo.toml:43`); adding it to `crates/htui/Cargo.toml` is a new
  dependency edge and moves `git diff --exit-code crates/*/Cargo.toml`, which milestone 3 made a hard
  gate. **Default (D99): hand-written**, ~70 lines over `std::fs::read_dir`, sorted on file-name bytes
  at every level, depth-capped and entry-capped, mirroring `FsRepoReader::walk`
  (`crates/htui-agent/src/excerpt.rs:249-255`) — the walk this workspace already treats as canonical
  for determinism. **Alternative:** `walkdir` with `follow_links(false)` and a `filter_entry` prune,
  the shape `crates/htui-orch/src/isolate/copy.rs:188-209` already uses; less code, one more manifest
  line.
- [ ] **OQ-22 — A name that fails the Agent Skills rule.** §7.3's fallback chain is `name` → parent
  directory name (for `SKILL.md`) → file stem, and the rule (`[a-z0-9-]`, 1–64, no edge or double
  hyphen) is checked by the writer. A Cursor rules file `workers.mdc` yields the stem `workers` and
  imports; a file named `My Skill.md` yields a name the writer refuses. **Default (D95): that one
  file is refused, with `invalid_skill_name`'s own sentence, and the refusal is listed in the report;
  the rest of the batch continues.** Nothing is slugified, because a slugified name is a name the
  maintainer did not write and cannot find again by search.
  **Alternative:** derive a slug (lowercase, non-`[a-z0-9-]` runs to `-`, trim) and import under it,
  recording both in `source.frontmatter`.
- [ ] **OQ-23 — Confirm before the first write.** Import is a write to the library, and the directory
  rule can sweep files the maintainer did not mean to import. **Default (D102): no confirm.** The
  report is shown afterwards and, per §9, names every skip and every refusal; the maintainer deletes
  what it should not have taken, and a skill is one `Ctrl+S` from a fix.
  **Alternative:** a warn-once gate before the first write of a run, the `omits_item` pattern
  (`templates.rs:702-708`, inside `fn save` at `:687-723`).
- [ ] **OQ-24 — One path per submission, or several?** The form is a single line. **Default: one
  path per submission** — the whole trimmed line is the path, so a path with spaces is one path and
  no quoting is invented. **Alternative:** split on whitespace into several paths, which breaks every
  path containing a space.
- [ ] **OQ-25 — What `source.format` records.** §6 item 11 names `format` beside `path` and
  `imported_at` without defining it, and the six ecosystems §4 surveyed are told apart by frontmatter
  keys that overlap (`globs` is Cursor's, Windsurf's and Continue's). **Default (D95): the file's
  shape, not its ecosystem** — `skill-md` when the file is named `SKILL.md`, `mdc` for a `.mdc`
  extension, `markdown` otherwise. It is falsifiable from the path, and the ecosystem is not guessed
  from keys that do not identify it; the keys stay verbatim in `source.frontmatter` either way.
  **Alternative:** a `str_enum!` of the six ecosystems with a first-match-wins key table, and `unknown`
  when nothing matches.
- [ ] **OQ-26 — Block scalars: accept them, or hold ANA-22's line?** **This is a conflict with a
  written constraint, not an extension of one.** §5.7's *Against* column lists "multi-line folded
  scalars" among the exotic YAML the hand-written reader **rejects**; D93 accepts `>`, `>-`, `|+` and
  their literal forms. The fact-check found all three attested in real files on this machine — 38 `>`,
  40 `>-` and 19 `|` in the `agent-toolkit-for-aws` tree alone (for example
  `…/aws-iam/SKILL.md`, `…/aws-observability/SKILL.md`, `…/aws-sdk-swift-usage/SKILL.md`) — so refusing
  them means the reader rejects a third of a real corpus and every one of those files imports with an
  `Issue` and no description. **Default (D93): accept them**, and record here that this widens §5.7's
  stated subset. **Alternative:** hold §5.7's line, treat a block scalar as a per-key `Raw` with an
  issue, and let the maintainer fix the description by hand — faithful to the analysis, lossy in
  practice.

---

## Summary

A skill is `name` + `description` + a versioned markdown body, and the four milestone-3 writers
already exist on both stores. What is missing is the other direction: nothing reads a `SKILL.md` off
the disk, and nothing has ever written a non-empty `skill_version.source` — the only writer passes
`serde_json::json!({})` (`crates/htui/src/skills.rs:209`).

Milestone 4 adds a hand-written frontmatter reader and ANA-22 §7.3's mapping (T1), a worker path that
collects files, reads them, and writes each one through `upsert_skill` + `add_skill_version` (T2), and
the Skills-tab surface: a path form, a per-file report, and the attachment form's prefill seam (T3).

**No new SQL, no migration, no dependency.** `skill_version.source` has existed since `0007`, both
writers already take it, and every reader already selects it, so `SkillsSnapshot` already carries an
imported version's provenance to the UI with no new read.

---

## Design decisions (settled here, not in code review)

| # | Decision | Why / evidence |
|---|---|---|
| D90 | **No migration. `0009` stays free.** Nothing in §7.3 needs a column: `0007_skill_attachments.sql:16-17` added `skill_version.source JSONB NOT NULL DEFAULT '{}'`, and `PgStore::add_skill_version` (fn at `pg/write.rs:2593`, statement at `:2612-2632`) already binds it as `$3`. | §7.1's schema shipped with milestone 2. The next free number stays `0009` and this milestone does not take it, so `tests/migrations.rs` moves for nothing. The separate `crates/htui-store/cache_migrations/` series has its own numbering and does not compete for `0009`. |
| D91 | **Two new pure modules in `htui-core`, both under `model/`: `model/frontmatter.rs` (the reader) and `model/skill_import.rs` (the §7.3 mapping),** registered in `model/mod.rs`'s flat alphabetical list (`:80-98`) — `frontmatter` between `event` and `hierarchy`, `skill_import` after `skill`. `htui-core` names no `std::fs` (`prompt/excerpt.rs:553-554`), so both are text-in/text-out; the file read lives in the `htui` crate (D98). | `model/language.rs` (191 lines, one `pub const` table) is the precedent for a data-shaped sibling module, and `prompt/template.rs`'s `is_token_shaped` (`:289`) is the precedent for hand-written scanning inside this crate. Splitting reader from mapping keeps the reader testable against a table of real-file shapes with no `Skill` type in sight, and keeps the mapping testable with no parser in it. |
| D92 | **The reader's surface: `pub fn split(text: &str) -> Result<Split, FrontmatterError>` with `Split { frontmatter: Vec<Entry>, body: String, body_at: usize }`, `Entry { key: String, value: Value, at: usize }`, and `enum Value { Scalar(String), List(Vec<String>), Raw(String) }`.** `at` is a **byte offset into the whole file**, exactly as `TemplateError`'s `at` is (`template.rs:377-412`), because the editor's cursor wants a byte and a line number is only a report nicety. `FrontmatterError` has two variants: `NoFence` and `Unterminated { at }` — the second kept **defensively**: the fact-check found zero unterminated frontmatters in 4,875 `SKILL.md` and zero in 12,987 markdown frontmatter files on this machine, so it is a total-function requirement, not an observed case, and its test is synthetic. | The byte offset is the established shape and the consumer already exists: `templates.rs:290-297`'s `error_at` plus `:698-699`'s `editor.area.set_cursor(at)`. A per-key problem is *not* a `Split` error (D94), so the enum stays two variants wide. |
| D93 | **The forms the reader accepts, and the ones it refuses.** Accepted: `key: scalar` (value trimmed, split on the **first** `": "` only, so a description containing `Triggers on: "x"` survives); `"…"` and `'…'` quoted scalars with `\\` and `\"` escapes; inline flow lists `[a, b]`; block lists (`- a` lines under the key, at **any** indent width — 2 and 4 both occur in the corpus); block scalars `|`, `|-`, `>`, `>-` per YAML's rules (folded re-joins with spaces; `|` keeps a trailing newline, `|-` drops it), and `|+` / `>+` accepted though unattested; a nested map under a key, block or single-line flow, kept as **`Raw` text** verbatim. Refused with a per-key issue: an unterminated quote, a block-scalar indicator that is not one of those, a list item that is not a scalar, and any other structure. `Raw` is the landing spot for a refusal, so nothing is ever lost. | §5.7's subset, widened only where real files on this machine demand it. Three widenings, each measured: block scalars, where all three attested forms occur (OQ-26 puts the conflict with §5.7's *Against* column to the maintainer); variable block-list indent (4-space in `discover-resources/SKILL.md`, 2-space in the firecrawl files), where keying on exactly two spaces would silently drop items; and `metadata:` as a **single-line flow map holding a nested JSON document**, which occurs at 63 `SKILL.md` paths (`…/vikingbot/workspace/skills/tmux/SKILL.md`) and which a reader handling only block nesting would truncate at the first `}`. |
| D94 | **A per-key problem is recorded, never fatal and never silent.** Each refusal becomes `Issue { key: String, at: usize, message: String }` on the `Split`, the entry's value becomes `Raw`, the import of that file **continues**, and the issues are carried into `source.issues` and shown in the report. The body imports regardless, which is §9's stated mitigation ("the body still imports if the maintainer accepts `always`"). Only a missing or unterminated **fence** fails the file. | §5.7's "reported per key, never silently" and §9's mitigation are mandates, not options. Refusing the file would make the reader's strictness the importer's data loss. |
| D95 | **The mapping, `model::skill_import::parse(path: &str, text: &str, now: DateTime<Utc>) -> ParsedSkill`,** with `ParsedSkill { name: String, description: String, body: String, source: serde_json::Value, prefill: ImportPrefill, issues: Vec<Issue> }` and `ImportPrefill { activation: Option<Activation>, globs: Vec<String>, languages: Vec<String>, hint: Option<&'static str> }`. `name` is `name` → parent directory name (for a `SKILL.md`) → file stem; `description` is `description` with `when_to_use` appended after a blank line; `body` is everything after the closing fence with the leading newline removed and exactly one trailing newline; `source` is `{"format","path","imported_at","frontmatter","issues","skipped"}` with `frontmatter` the **verbatim** key/value map. Prefill per §7.3: `paths` / `globs` / `applyTo` / `fileMatchPattern` → `activation = glob` plus those globs (a comma string is split, a list is taken as is); `alwaysApply: true` / `trigger: always_on` / `inclusion: always` / `applyTo: "**"` → `always`; `languages` → languages; description-only, `trigger: model_decision`, `inclusion: manual`, `disable-model-invocation` → `always` with the hint. `applyTo` is read **value-aware**, since §7.3 gives it both meanings. | §7.3's table verbatim, row by row. `when_to_use` is the one key the table appends rather than maps, so it is stated here. `format` is OQ-25. The name chain is load-bearing, not cosmetic: the fact-check counted **one** `SKILL.md` on this machine with no `name` key (one file at two mirrored paths, `~/.gemini/{antigravity-cli/plugins,extensions}/spec-flow/.claude/skills/x-announcement/SKILL.md`) plus the sampled Cursor `workers.mdc`, which has none at all. |
| D96 | **The prefill is applied by the matrix, from the stored `source`, not by the import reply.** `MatrixView::open_form` keeps seeding from the stored row, and for a **new** attachment (no row, token `None`) it then seeds `activation`, `globs` and `languages` from `ImportPrefill`, which it re-derives from the `source` of the version in force for that skill. An existing row is never re-seeded: its stored values are the truth. | §6 item 11 says the attachment form "reads it once, to prefill activation, globs and languages **from the version being attached**" — the stored version, not the import's return value. That also makes the prefill work for a skill imported a week ago and attached today, which an import-time prefill would not. `open_form` has no prefill parameter today and no `Option<…>` content argument anywhere in `matrix.rs` or `templates.rs`; the seam is new. |
| D97 | **One new request and one new reply.** `StoreRequest::ImportSkills { scope: Scope, paths: Vec<String> }` (`StoreRequest` 73 → 74) and `StoreReply::SkillImports(Box<SkillImports>)` with `SkillImports { snapshot: SkillsSnapshot, report: Vec<ImportOutcome> }` and `ImportOutcome { Imported { name, path, version }, Updated { name, path, version }, Unchanged { name, path }, Refused { path, message }, Skipped { path, reason } }` (`StoreReply` 42 → 43). `REQUEST_NAMES` 4 → 5, `["skills","save_skill","set_skill_binding","remove_skill_binding","import_skills"]`, with a matching `StoreRequest::name()` arm and the request **or-ed into the existing skills arm** at `store_worker.rs:1205-1208`, not given an arm of its own. | The match there is wildcard-free, so a new variant is a compile error until it joins that arm; or-ed is why the arm exists (the F-12 comment above it). One reply rather than two, because the report is not a CAS outcome: an import that hits a stale token on one file still reports every other file, so there is no `SkillsStale` shape to answer. |
| D98 | **The file read happens on the store worker**, in a new `crates/htui/src/skill_import.rs` (`pub mod skill_import;` at `crates/htui/src/lib.rs`), called from `skills::serve`. The UI types a path and nothing else. | `R-NF-3` puts store-touching work on the worker, and a directory walk is unbounded work that must not sit on the render task. `std::fs` in the worker's `async fn` is the existing precedent (`editor.rs:222`), and the walk is capped (D99) so it cannot hold the worker for long. |
| D99 | **Path collection.** A **file** is imported whatever it is named. A **directory** contributes `SKILL.md` at any depth up to `MAX_DEPTH = 4` (the `*/SKILL.md` shape) and `*.md` / `*.mdc` at depth 1 only (the rules-directory shape, `.cursor/rules/*.mdc`). Entries are sorted on file-name bytes at every level. **Skipped and listed** (ANA-22 §6 item 12): the bundled `scripts/`, `references/` and `assets/` directories, plus `.git`, `node_modules`, any file over `MAX_BYTES = 256 KiB`, and any non-UTF-8 file. At most `MAX_FILES = 64` candidates; the cap is a reported `Skipped` row, not a silent stop. Symlinked directories are not descended. **Stated consequence, not an accident:** a directory import pointed at *this* repository's root reaches its own three `.claude/skills/*/SKILL.md` at depth 3 and imports them. That is the rule working, not a leak, and the report names every file it touched. | §8's bullet names the two shapes and §6 item 12 names the three bundled directories. The sort and the cap are `FsRepoReader::walk`'s rules (`htui-agent/src/excerpt.rs:249-255`, `:291-301`), and no-follow-symlinks is `copy.rs:188-209`'s. The bundled-directory skip is also what keeps one skill's `references/` from importing as four more skills. |
| D100 | **The write path, per file, from one read taken immediately before the writes.** If no skill holds the name: `upsert_skill(NewSkill { id: SkillId::new(), name, description, created_by }, None)` then `add_skill_version(NewSkillVersion { skill_id, body, source, created_by }, None)`. If one does: `upsert_skill(…, Some(skill.updated_at))` — which updates the description, per §6 item 12 — and then `add_skill_version(…, Some(head))` **only when the body differs from the head's**; an identical body is `Unchanged` and writes nothing. A `Stale` at either step is `Refused { message }` for that file and the batch continues. `created_by` is `backend.this_user()` once per batch. | The same-name rule is §6 item 12 verbatim ("appends a version only when the body differs, and updates `skill.description`"). Both tokens come from one read, which is the order milestone 2's review finding 5 settled for bindings and versions: read the mutable row, then the append-only one, and let the tokens be the truth rather than a guess. An import writes no attachment, so `set_skill_binding` is not called at all (D101, §7.3). |
| D101 | **No new `WriteStore` method, no conformance case, no `.sqlx` file, no new table, no new column.** `WriteStore` stays **83**, `CASES` **81**, `READ_CASES` **14**, `EXPECTED_CASES` **81**, `TABLES` **39**, commented columns **34**, `MIRRORED_TABLES` **21**, `.sqlx` **281** — all unchanged. | Import calls the two writers milestone 3 already added and already covers in four conformance cases, with SQL text that already exists. A fifth writer would be a fifth conformance case, a fifth spy arm, and a fifth `.sqlx` file for a call sequence that is already the tested one. |
| D102 | **The UI is two modes in `library.rs` and one seam in `matrix.rs`.** `i` in Browse opens `Mode::ImportPath { field: TextField }` (the `templates::Mode::Naming` shape), Enter sends `ImportSkills` with the trimmed line, `Esc` cancels. The reply opens `Mode::Report` **whenever any file was refused or skipped** and otherwise leaves a two-line notice (`imported N, updated M, unchanged K`), the shape `SAVING`/`NO_CHANGES` already use. `Mode::Report` lists every `ImportOutcome` and the skip list, `j`/`k` scroll, `Esc` returns to Browse, and the snapshot re-reads behind it. `BROWSE_HINT` gains ` i import`; the guard test `every_browse_key_misses_the_global_table` gains `"i"` in its claimed array. A `IMPORT_NAME: &str = REQUEST_NAMES[4]` and the same `busy` gate `SAVE_NAME` has. | `i` is free in the library browse map and in `Keymap::default_global` (the matrix already took `m` on the same reasoning, `mod.rs:42`). A report is not a decoration: §9's mitigation is that a rejection is *reported*, and a two-line notice cannot carry a per-file list. The busy gate is what makes a second submission impossible while a walk is in flight. |
| D103 | **Scope fence.** Milestone 4 does not do: export, delete of a skill or a version, a template import (a template is not a skill, and PRD D3 makes the database the only source of truth in both directions), model-decided activation (ANA-22 §6 item 10, needs MOD-11), agent help while editing (MOD-55), a Settings section (PRD D1), a workspace level (ANA-22 §6 item 2), a `SkillChoice` or trim-record change, or an import that attaches anything. | Each is named in the PRD, ANA-22 or an open HANDOFF item, and taking one would pull its dependency with it. |
| D104 | **Booleans are read as `true` / `false` only.** `alwaysApply: false`, `user-invocable: false` and `disable-model-invocation` are bare YAML booleans and are compared as strings; `no`, `on`, `off`, `y` and `n` are **never** coerced. | §5.7 names the hazard exactly ("YAML 1.1/1.2 edge cases (`no` → `false`) in names"), and a name is the library key: coercing one would rename a skill. |
| D105 | **The opening fence must be the first line.** A leading BOM is skipped; a leading blank line is not, and a `---` that is a horizontal rule rather than a fence is `NoFence`. The body split is on the **second** `---` line. | Sampled files exist whose `---` is a rule, not a fence — they show up as false positives in any key-frequency count over `SKILL.md`, which is how the format survey found them. A reader that splits on the first rule would import half a document as frontmatter. |

---

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A hand-written scanner over text with a byte offset | `prompt::template::parse` | `crates/htui-core/src/prompt/template.rs:304-359`, error enum `:377-412` |
| The consumer that puts a byte offset on the cursor | `error_at` + `set_cursor` | `crates/htui/src/ui/tabs/skills/templates.rs:288-297`, `:695-700` |
| A pure data-shaped sibling module | `model/language.rs` | `crates/htui-core/src/model/language.rs`, `model/mod.rs:88` |
| A worker module serving a family of requests | `crates/htui/src/skills.rs` whole | `REQUEST_NAMES:149`, `serve:166`, `stale:290` |
| The or-ed dispatch arm | `store_worker.rs:1203-1208` | with the F-12 comment |
| A `busy` gate and its two replies | `SAVE_NAME` + `SkillsStale` | `library.rs:816-837`, `:388-410` |
| A one-field form that submits on Enter | `templates::Mode::Naming` | `templates.rs:189-196`, `:654-703` |
| A deterministic, capped, sorted directory walk | `FsRepoReader::walk` | `crates/htui-agent/src/excerpt.rs:235-255`, cap `:291-301` |
| No-follow-symlinks recursion | `copy.rs::walk` | `crates/htui-orch/src/isolate/copy.rs:188-209` |
| Reading a file and refusing in one sentence each | `editor::run` | `crates/htui/src/editor.rs:222-240` |
| A refuse-before-send gate reusing the writer's sentence | `matrix::refusal` | `matrix.rs:1319-1348` |
| A key that must not collide with the global table | `every_browse_key_misses_the_global_table` | `library.rs:1309-1334` |

---

## Tasks

**T1 → T2 → T3, serial.** The file sets are disjoint (see "Intersections"), so the order is a cost
decision, not a coupling one: a parallel wave needs a second worktree and a second target directory
(~10 G each, R-33), and three tasks do not repay that.

| Task | Files (complete list) | Order |
|---|---|---|
| T1 | `crates/htui-core/src/model/frontmatter.rs` (new), `crates/htui-core/src/model/skill_import.rs` (new), `crates/htui-core/src/model/mod.rs` (the two `pub mod` lines), `crates/htui-core/tests/skill_import.rs` (new) | first |
| T2 | `crates/htui/src/skill_import.rs` (new), `crates/htui/src/lib.rs` (`pub mod skill_import;`), `crates/htui/src/skills.rs` (`REQUEST_NAMES`, the `serve` arm, `stale` untouched), `crates/htui/src/store_worker.rs` (the variant, the `name()` arm, the or-ed arm) | after T1 |
| T3 | `crates/htui/src/ui/tabs/skills/library.rs`, `crates/htui/src/ui/tabs/skills/matrix.rs`, `crates/htui/tests/skills.rs`, `crates/htui/tests/skills_matrix.rs`, `crates/htui/tests/snapshots/skills__import_*.snap` (new) | after T2 |

**Intersections, checked by hand.** T1 ∩ T2 = ∅: T1 is `htui-core` text-in/text-out, T2 never calls it
except through the `pub` API, and `model/mod.rs` is T1's alone. T2 ∩ T3 = ∅ as file content, but they
share **one sequencing dependency**: T3's `IMPORT_NAME: &str = REQUEST_NAMES[4]` cannot compile before
T2 adds the fifth name, the same shape as milestone 3's `mod matrix;` problem, so the two are serial
for the same reason and the same reason is recorded (R-30 there). T1 ∩ T3 = ∅.

**No new `WriteStore` method**, so no `mem.rs`, `pg/write.rs`, `writer.rs`, `conformance.rs`,
`htui-agent/src/conformance.rs` or `htui-agent/tests/recorder.rs` edit, and no `.sqlx` regeneration.

Every implementer prompt carries these rules:

- ANA-22 §5.6, §5.7, §6 items 11–12, §7.3 and §8, this plan's D90–D105 and the maintainer's OQ
  answers win over prose.
- The Gortex index is one milestone stale and its edit path writes to the primary checkout: **read and
  write files in `/media/projects/htui-mod-9-m4` directly**; use Gortex only as a cross-reference, and
  never for an edit.
- `R-ID-6`: the import path is deterministic code. No agent, no network, no `htui` MCP call.
- No test is skipped or loosened to get green; a moved pin names its reason in the assertion message.
- Every new `pub` item has a doc comment and a `Debug`; a body crossing the store boundary goes in a
  newtype whose `Debug` prints its length, never its text (a skill body is a whole file).
- Commit the red tests first, then green, on `htui-mod-9-m4`.
- Gate command: `RUST_BACKTRACE=0 cargo test -p <crate> --all-features -- --test-threads=1` for
  anything Postgres-backed, `--test-threads=2` otherwise.

### Task 1: the reader and the mapping (D91–D95, D104, D105)

- **Tests first** (`crates/htui-core/tests/skill_import.rs`, plus unit tests beside each module).
  - `the_reader_accepts_the_shapes_real_files_use` — a table over five real files, quoted into the
    test as literals, each naming its source file in a comment: the two-key `handoff-run` form; the
    `allowed-tools: Read` + `metadata:` / `version: "2"` form; the `description: >` folded form with
    `allowed-tools: Read Grep Glob Bash` and a four-key `metadata:`; the `alwaysApply: false` +
    block-list `globs` `.mdc` form with **no `name`**; and the single-line flow
    `metadata: {"vikingbot":{"emoji":…,"os":[…],"requires":{"bins":[…]}}}`, from
    `…/vikingbot/workspace/skills/tmux/SKILL.md` — 63 `SKILL.md` paths on this machine use that shape,
    and it is the case a block-only reader would truncate at the first `}`.
  - `the_reader_never_coerces_a_yaml_one_one_word` — a description reading `no`, a name reading `on`,
    both survive verbatim.
  - `a_description_holding_colon_space_is_one_value` — a **scalar** description reading
    `Triggers on: "build an agent"` stays one value; the folded-block form of the same text is
    covered by the third row of the table above and is a different parse path.
  - `a_nested_map_is_kept_verbatim` — the flow-map `metadata:` round-trips byte for byte, braces and
    inner `]`s included.
  - `a_block_list_is_read_at_either_indent_width` — the same list at 2 and at 4 spaces yields the same
    items; `discover-resources/SKILL.md` uses 4, the firecrawl files use 2.
  - `a_rejected_key_is_reported_with_its_byte_and_its_line_and_the_body_still_imports` — an
    unterminated quote in `description`; the body is intact, the issue names key, byte and line, and
    `source.issues` carries it.
  - `an_unterminated_fence_is_refused` — **synthetic**: the fact-check found zero unterminated
    frontmatters in 4,875 `SKILL.md` and 12,987 markdown frontmatter files, so D92's second error
    variant is total-function insurance and its test is written, not observed.
  - `the_opening_fence_must_be_the_first_line` — a leading BOM is skipped; a leading blank line is
    `NoFence`; a `---` horizontal rule in a body does not split the body, and a file whose first
    `---` is a rule is `NoFence` (five such files exist on this machine, all under `~/.gemini`'s
    spec-flow trees).
  - `a_file_with_no_name_falls_back_to_its_directory_then_its_stem` — the `.mdc` half has a real
    fixture (`workers.mdc`); the `SKILL.md`-with-no-`name` half is **synthesised**, because the one
    real file that lacks the key is not reachable in a test.
  - `the_mapping_writes_exactly_what_the_table_says` — every row of §7.3, one case each, including
    `applyTo: "**"` meaning `always` while `applyTo: "src/**"` means `glob`.
  - `the_source_keeps_the_whole_frontmatter_verbatim`.
- **Action.** D91–D95, D104, D105.
- **Validate.** The `htui-core` gate, `cargo doc -p htui-core --no-deps`, and `cargo clippy
  --workspace --all-features --all-targets -- -D warnings`.

### Task 2: the worker path (D97–D100)

- **Tests first.**
  - `crates/htui/src/skill_import.rs` unit tests over a temp directory built in the test: a file, a
    directory holding two `*/SKILL.md` and one `.mdc` at depth 1, a `scripts/` and an `assets/`
    directory that must be skipped and listed, a file over the byte cap, a non-UTF-8 file, a symlinked
    directory that must not be descended, and a tree past `MAX_FILES`; plus "the walk's order is the
    same on two runs and is byte order", which is the `FsRepoReader::walk` rule.
  - `crates/htui/src/skills.rs`: `request_names_match_the_name_arms` moves to five names, and
    `serve` refuses a foreign request by name and answers `import_skills` with `SkillImports`. The
    existing redaction test at `skills.rs:471` (`format!("{request:?}")` asserted to carry `len:` and
    not the text) is extended to the new variant: the request carries paths, not a body, so the guard
    is that no **file content** reaches a `Debug` sink — the report's `path` and `message` are fine,
    an imported body is not.
  - End-to-end over `MemStore::demo()` in `crates/htui/tests/skills.rs` is T3's; T2's end-to-end is the
    worker's own `import` over the backend, asserting the outcomes and the rows.
- **Action.** D97–D100.
- **Validate.** The `htui` gate (Postgres), `cargo build --workspace --all-features --all-targets` so
  the wildcard-free `try_serve` match and every `StoreReply` match in the UI are forced to grow an arm,
  and `git diff --exit-code Cargo.lock crates/*/Cargo.toml` (nothing moved).

### Task 3: the import surface (D96, D102)

- **Tests first** (`crates/htui/tests/skills.rs`, over `Harness::over(MemStore::demo())`, opening the
  tab with `2` and settling):
  - `i_opens_a_path_form_and_enter_sends_one_request` (asserted on the store, as the existing
    "nothing was sent" assertions are).
  - `esc_leaves_the_path_form_and_sends_nothing`.
  - `an_import_of_one_file_creates_a_skill_at_version_one`.
  - `an_import_of_a_second_file_creates_a_second_skill`.
  - `a_skill_the_library_already_holds_gets_a_new_version_only_when_the_body_differs` — the same body
    reports `Unchanged` and the head does not move; a changed body appends v2 and updates the
    description.
  - `an_import_refuses_one_file_and_imports_the_rest` (the report names the refusal, and the batch's
    other file landed).
  - `a_directory_import_skips_the_bundled_scripts_references_and_assets` (the report lists them).
  - `a_refused_name_sends_no_writer_call` — the `invalid_skill_name` sentence, byte for byte the
    store's, D100.
  - `the_import_report_lists_every_outcome` (snapshot).
  - `a_missing_path_is_refused_in_one_sentence` (the `editor.rs:222` shape).
  - the prefill cases, in `crates/htui/tests/skills_matrix.rs`: `a_new_attachment_prefills_activation_and_globs_from_the_imported_source`
    (snapshot of the form with `glob` and the expanded globs), `an_existing_attachment_is_never_reseeded_from_a_source`,
    and `a_skill_with_no_source_prefills_nothing`.
  - the two guard tests keep passing: `every_browse_key_misses_the_global_table` (with `"i"` claimed) and
    `the_strip_text_is_unchanged`.
- **Action.** D96, D102.
- **Validate.** The `htui` gate (Postgres, `--test-threads=1`) and `cargo insta review` over every new
  snapshot, each named in its commit message. The strip text
  ` 1 Backlog  2 Skills  3 Settings  4 Chat`, the switch line `" Skills │ Templates "`, and the six
  `templates__*.snap` files must not move.

---

## Test plan

**Unit.** `model/frontmatter.rs` for the reader table (the real-file shapes, the refusals, the offsets);
`model/skill_import.rs` for §7.3's mapping row by row; `crates/htui/src/skill_import.rs` for the walk
(sorting, depth, caps, the skip list, symlinks).

**Conformance.** None new, and that is the point (D101): the two writers import calls are the ones
milestone 3 already runs on both stores, in four cases plus the Postgres CAS race.

**Integration.** The TUI over `testkit::Harness`, with `insta` snapshots for the form and the report.

**Gates.** `cargo fmt --all -- --check`; `cargo clippy --workspace --all-features --all-targets
-- -D warnings`; the workspace run with `--test-threads=2` and a single-threaded pass for the
Postgres-backed targets; `cargo sqlx prepare --check` (expected to be a no-op — nothing moved).

## Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-36 | The hand-written reader rejects or misreads a real file, and the import is lossy where a YAML crate would not be. | Medium | Medium | The reader's accepted-form table is the specification and is pinned against five real files (T1). A refused key is `Raw` and reported with a byte and a line, never dropped (D94), and §7.3's lossless row is honoured because `source.frontmatter` is the verbatim text. |
| R-37 | A directory import sweeps files the maintainer did not mean to import — a repository root's every `README.md`. | Medium | Medium | The selection rule is narrow by construction: `SKILL.md` at depth ≤ 4, and `*.md` / `*.mdc` at depth 1 only (D99). A project root's `docs/*.md` is at depth 2, so it is not swept. The report lists every file it touched and every one it skipped. |
| R-38 | A same-name import overwrites a version the maintainer edited in the TUI. | Low | High | The append is a compare-and-set on the head version read immediately before the write (D100), so a concurrent edit answers `Stale` and that file is refused with the token's own message. The description update is a compare-and-set on `skill.updated_at` for the same reason. |
| R-39 | A half-finished import leaves a skill row whose description changed and whose version did not append. | Low | Low | The same shape the milestone-3 worker already documents at `skills.rs:10-12`: the two writes are separate CAS operations and a stale second one leaves the first. The report says which half landed, and a `Ctrl+S` in the editor completes the other. |
| R-40 | The prefill seeds a form with globs the matcher cannot compile, and the writer refuses the save with a message the maintainer did not expect. | Medium | Low | The prefill is shown before the save in the form's own effective-globs pane (D96 reuses `Form::effective()`), and the refusal is the writer's own sentence via the existing `refusal` gate (`matrix.rs:1319-1348`). A glob the import read that `glob::compile` refuses is reported at import time as an issue rather than at save time. |
| R-41 | The walk reads a file outside what the maintainer named, through a symlink. | Low | Medium | Symlinked directories are not descended and the root itself is never pruned (D99), the shape `copy.rs` already uses. A symlinked *file* named explicitly is read, which is what naming it asks for. |
| R-42 | The worker's walk holds the store worker long enough to stall other requests. | Low | Medium | `MAX_FILES = 64` and `MAX_BYTES = 256 KiB` bound the work, and the reply is one variant rather than a stream, so a large import is a single bounded pause rather than an unbounded one. Measured and recorded in the T2 commit if it shows up. |
| R-43 | The parallel-worktree cost of a three-task milestone exceeds the time saved. | Certain | Low | The tasks run serially (T1→T2→T3); the file sets are disjoint but the `REQUEST_NAMES[4]` coupling is real (R-30's shape, recorded again here). |

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- `USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features
  --no-fail-fast -- --test-threads=2`, and `--test-threads=1` for the Postgres-backed targets
- `cargo sqlx prepare --check`
- `git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml` — **no dependency change**
- `git diff --exit-code crates/htui-store/migrations crates/htui-store/.sqlx` — **no migration and no
  query change**
- Validator: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`
- Pins afterwards:
  - unchanged: `WriteStore` 83, `CASES` 81, `READ_CASES` 14, `EXPECTED_CASES` 81, `TABLES` 39,
    commented columns 34, `MIRRORED_TABLES` 21, `.sqlx` 281, the next free migration `0009`
  - moved: `StoreRequest` 73 → 74, `StoreReply` 42 → 43, `REQUEST_NAMES` 4 → 5,
    `crates/htui/tests/snapshots` 97 → 97+N

## Acceptance

- A maintainer types a path to a `SKILL.md` in the Skills view, and the skill is in the library at
  version 1 with its description, its body, and its frontmatter kept verbatim in `skill_version.source`.
- A maintainer types a directory, and every `*/SKILL.md` and depth-1 `*.md` / `*.mdc` in it is
  imported, with the bundled `scripts/`, `references/` and `assets/` skipped and **listed**, and the
  report naming every file it touched, left alone, or refused.
- A file whose frontmatter the reader cannot fully parse still imports, and the report names the key,
  the byte and the line it could not read.
- Importing a file whose name the library already holds appends a version when the body differs,
  updates the description, and reports `Unchanged` when it does not.
- A new attachment in the matrix opens prefilled with the activation, globs and languages the imported
  frontmatter asked for, and an existing attachment is never re-seeded.
- Nothing is attached by import: the library grows, the matrix does not.
- `R-SKL-4` is satisfied by one sentence in the test names, and `R-ID-6` holds — the whole path is
  deterministic code with no agent in it.

## Where the PRD, ANA or tree disagree

- **ANA-22 §6 item 12** says a same-name import "updates `skill.description`". §7.3's table does not
  say that. D100 follows item 12, and reads the row above it (`name` → `skill.name`, validated) as not
  covering the description's update.
- **ANA-22 §5.7** lists "comma strings" among the forms the subset uses. D93 accepts a comma string
  as a scalar and splits it **only** for the keys §7.3 names as glob sources; a scalar is otherwise
  never split, because the fact-check counted **six** incompatible encodings of `allowed-tools` across
  12,987 frontmatter files on this machine — bare scalar (34 paths), comma-space (127), space-separated
  (22), inline flow list (166), quoted comma string (1) and block list (202) — and splitting on
  anything but a comma mangles the space-separated form into one bogus token. The block list is the
  second most common shape, not an edge case. The same scan found **no** value mixing a comma and a
  space inside one token, so the plan's earlier "commas *and* spaces inside one token" rationale is
  **dropped as unverified**; the reader still keeps such a value as one `Scalar`, which is correct
  whether or not such a file exists.
- **ANA-22 §5.6** offers "keep the raw frontmatter and the import provenance in one `source` JSONB
  column" and lists "Not validated" against it. D95 does not validate it either: it is written by the
  importer and read by the prefill, never by a query.
- **The PRD** names no key, no view and no error behaviour for the import; D102 chooses the Skills
  view and `i`, and §5.6's "not read by the prompt builder" is why the prefill reads it and the
  assembler does not.
- **The PRD's Evidence** (`prd:61-63`, and ANA-22 `:80-81`) says "the one frontmatter reader is
  hand-written for the review verdict (`crates/htui-orch/src/gate.rs:64`)". **Verified, and not
  reusable:** `gate.rs:64-78` is a fifteen-line function that splits on `\n`, requires `---` / one line
  / `---`, and returns that single middle line as a review verdict. It shares the fence-delimiting
  idea with D92 and nothing else — no keys, no lists, no per-key errors, no offsets. The PRD's phrase
  is prior-art naming, not a pointer to a reusable implementation, and this plan writes a new reader.
- **`docs/ANA-5.md`** says nothing about importing a skill file; its only "import" is `R-ID-6` quoted
  at `:995`. Nothing there constrains this milestone. (ANA-5's own cross-reference cites `R-ID-6` at
  `:39-40`; it lives at `docs/REQUIREMENTS.md:55-56`. A pre-existing doc error, left alone.)
- **`R-NF-3` is not the store-worker rule.** Its text (`docs/REQUIREMENTS.md:347-348`) is "All long
  operations … run off the UI thread; the TUI never blocks on network or subprocess I/O". The
  "`StoreRequest` on the store worker" ownership rule this plan leans on is the PRD's Constraints
  block (`prd:163-175`). Both point the same way; only the attribution is corrected above.

## Verified claims

*Checked on 2026-09-28 against `/media/projects/htui-mod-9-m4` at `f24c519`, by three verifiers: one
over the tree's `file:line` references, one over every pin and count, one over the written sources
and the real-file format evidence. A row marked **false as first drafted** is a claim this plan got
wrong and then corrected; the correction is named in the row.*

### Tree and line references

| Claim | Verdict | Evidence |
|---|---|---|
| `NewSkill` is at `skill.rs:424-434` with `id, name, description, created_by` | ✓ true | struct 425, fields 426/429/431/433 |
| `NewSkillVersion` at `:439-449` with `skill_id, body, source, created_by` and no `version`/`created_at` | ✓ true | struct 440, fields 442/444/446/448; doc 436-438 "the store assigns the version number … and the instant" |
| `NewSkillBinding` at `:457-478` has exactly 9 fields ending in `languages`, and no `updated_at` | ✓ true | struct 458, `languages` 477; the token is the parameter |
| `Skill::name_is_valid` at `:93-102` is the six-clause Agent Skills rule | ✓ true | verbatim |
| `WriteStore` spans `traits.rs:269-1433`; the four skill writers at 777/797/825/839; `set_setting` next at 859 | ✓ true | brace-matched trait body |
| `remove_skill_binding` takes a non-`Option` token; `add_skill_version` takes `Option<i32>` | ✓ true | 842, 800 |
| The seven refusal helpers are at 1598/1608/1619/1628/1644/1676/1714 | ✓ true | every cited range lands on the `fn` or the closing brace |
| `add_skill_version` binds `source` as `$3` with the head CAS in the SELECT's WHERE | ✓ true, **line-amended** | fn at `write.rs:2593`, statement `:2612-2632`; `COALESCE($4::int, 0) + 1` at 2616 |
| `upsert_skill` is `ON CONFLICT (name) DO UPDATE SET description … WHERE skill.updated_at = $5` | ✓ true, **line-amended** | fn at `:2528`, statement `:2553-2574` |
| `impl WriteStore for Writer` at `writer.rs:351`, four arms 717-759, `Writer` has two variants | ✓ true | so a new writer method grows two arms, not three |
| `REQUEST_NAMES` is `[&str; 4]` at `skills.rs:149-154`; `SkillSummary.versions` at 49; `SkillBody` at 56; `serve` 166-286; the save arm 174-224 with `json!({})` at 209; `stale` 290-294; the residue doc at 10-12; the name test at 843 | ✓ true | all nine confirmed |
| The four skill `StoreRequest` variants sit at 557-**612** and their `name()` arms at **718**-721 | ✓ **line-amended** (was 557-610 / 719-721) | `RemoveSkillBinding` closes at 612; line 718 is `Self::Skills(..) => "skills"` |
| The or-ed skills `try_serve` arm is at `store_worker.rs:1205-1208` with the F-12 comment at 1203-1204 | ✓ true | — |
| `pg::skill_library` (1692-1739) selects `source` at 1715; `skill_head` 1544-1564; `skill_versions_of` 1578-1597; `skill_attachments` (1752-1787) reads no version; `MemStore::skill_library` (463-483) clones whole rows | ✓ true | so no new reader is needed for provenance |
| `library.rs`: `captures_input` 345, `on_key` 368, `on_reply` 377, `on_external_edit` 427, `render` 459, `render_browse` 994, `render_editor` 1169 | ✓ true | all seven |
| `BROWSE_HINT` is exactly the claimed literal, two spaces between clauses | ✓ true | `library.rs:92-93` |
| `save` (816-837) sends both tokens; `SkillsStale` (388-410) keeps the draft and moves both tokens; `land_save` 937-989; `hand_off` 841-857; `Editor::new` 265-285 | ✓ true | `expected: editor.updated_at` 834, `expected_version: editor.token` 835 |
| The row format is `  {name:<NAME_WIDTH$} v{head:<3} {activation:<5} {tokens}` | ✓ true, **precision amended** | `NAME_WIDTH = 26` at `library.rs:58`; the source uses the constant, not a literal `26` |
| `every_browse_key_misses_the_global_table` at 1309-1334, 18 claimed keys, and **`i` is unbound** | ✓ true | zero hits for `'i'` anywhere in `library.rs` |
| `matrix::open_form` (723-772) seeds only from the stored row and takes no prefill parameter | ✓ true | fields seeded 755-766 from `cell(...)` at 731 |
| No `prefill` code parameter exists anywhere under `crates/` | ✓ true | 25 hits, all prose or one test-fn name |
| `matrix::save` (815-851) runs `refusal` at 826 before the request at 850; `refusal` 1319-1348 reuses both writer helpers | ✓ true | — |
| `Form::effective()` at 377-382, drawn at 1119-1125 | ✓ true | — |
| `template::parse` at 304; `TemplateError` 377-412 with `at: usize` on three variants; `error_at` 290-297; `set_cursor` 698-699 | ✓ true | — |
| The `omits_item` warn-once gate is at `templates.rs:702-708` | ✓ **line-amended** (plan said 687-723, which is the enclosing `fn save`) | OQ-23 updated |
| `excerpt.rs:249-255` sorts on `as_encoded_bytes()`; the cap is charged at 297 before the `symlink_metadata` at 303 | ✓ true | — |
| `copy.rs:188-209` uses `follow_links(false)` + `filter_entry`, root exempt at 200-202 | ✓ true | — |
| `editor.rs:222` is a `std::fs::read` inside an `async fn`, with both refusal sentences at 226 and 231 | ✓ true | no `tokio::fs`, no `spawn_blocking` |
| `prompt/excerpt.rs:553-554` states "this crate names no `std::fs`" | ✓ true | quoted verbatim |
| `walkdir = "2"` at `Cargo.toml:110`, consumed only by `htui-orch`; one lock entry, 2.5.0 | ✓ true | `Cargo.lock:7095-7103` |
| `globset`, `glob` and `ignore` are in no manifest and no lock entry | ✓ true | the only "glob" hit in a manifest is a comment, `htui-agent/Cargo.toml:37` |
| `tempfile` is a direct **regular** dependency of `htui` | ✓ line-amended (was `:53`) | `crates/htui/Cargo.toml:54`; the plan did not rely on it |
| `model/mod.rs:80-98` is a flat alphabetical list with `language` at 88 | ✓ true | `frontmatter` slots at 84, `skill_import` after `skill` at 96 |
| `htui/src/lib.rs:12-32` is a flat alphabetical `pub mod` block, `skills` at 28, `templates` at 30 | ✓ true | — |
| `0007_skill_attachments.sql:16-17` adds `source`; its COMMENT is at 30-31; `0008` is comment-only | ✓ true | — |
| **The PRD's "one frontmatter reader" (`gate.rs:64`) is not reusable** | ✓ **true, and the plan's guess was right for the wrong reason** | `gate.rs:64-78` is fifteen lines that split on `\n`, require `---`/line/`---`, and return that one line as a review verdict. No keys, no lists, no offsets. It shares the fence idea and nothing else. |

### Pins and counts

| Claim | Verdict | Evidence |
|---|---|---|
| `WriteStore` declares 83 `async fn` | ✓ true | counted in the brace-matched trait body only; 83, first at 271, last at 1427 |
| `CASES` 81, `READ_CASES` 14, `EXPECTED_CASES` 81 | ✓ true | `conformance.rs:44-126` (81), `:320-335` (14), `pg_conformance.rs:19` (a `usize` const) |
| `StoreRequest` 73, `StoreReply` 42 | ✓ true | both enums enumerated variant by variant; `StoreRequest` 97-647, `StoreReply` 742-933 |
| `REQUEST_NAMES` has 4 entries | ✓ true | typed array `[&str; 4]`, so the compiler enforces it |
| 97 `.snap` files, 5 `skills__`, 6 `templates__` | ✓ true | full prefix breakdown counted |
| `.sqlx` holds 281 files | ✓ true | flat, 281 regular files |
| `MIRRORED_TABLES` 21 | ✓ true | `cache/mod.rs:44-66`, typed `[&str; 21]` |
| `TABLES` 39 | ✓ true | `migrations.rs:19-61`; a naive non-blank count says 41 (two comment lines) |
| commented columns 34, derived as 25 + 4 + 5 | ✓ true | asserted at `migrations.rs:497-500`; there is no `34` literal — the chain is the pin |
| migrations are `0001`..`0008`, next free `0009` | ✓ true | asserted at `migrations.rs:78-87`; the separate `cache_migrations` series has its own numbering and does not compete |
| `try_serve` is wildcard-free, so a new `StoreRequest` variant is a compile error | ✓ true, by reading | no `_ =>` in the brace-matched match; **no compile probe was run** |
| `StoreRequest` derives `Debug` and reaches a `Debug` sink | ✓ true | `store_worker.rs:96`; `skills.rs:471` asserts the body is redacted to a length, and `RequestEnvelope` (`:1072`) is `Debug` |

### Written sources and real-file format

| Claim | Verdict | Evidence |
|---|---|---|
| `R-SKL-4` reads "Import from existing `SKILL.md` files as a convenience" | ✓ true | `docs/REQUIREMENTS.md:269`, word for word |
| ANA-22 §5.7, §5.6, §7.3's eight rows, §6 items 11-12, §8's milestone-4 bullet, §9's risk row | ✓ true | all quoted verbatim; §5.7 at `:195`, §5.6 at `:185-189`, §7.3 at `:294-303`, §6 at `:239-245`, §8 at `:316-318`, §9 at `:335` |
| The PRD's milestone-4 row and its scope bullet | ✓ true | `prd:186` and `prd:128-129` |
| `R-ID-6` and `R-NF-3` | ✓ true, **attribution amended** | `R-ID-6` at `REQUIREMENTS.md:55-56`; `R-NF-3` at `:347-348` says "off the UI thread", not "through `StoreRequest`" — that rule is the PRD's Constraints block (`prd:163-175`) |
| `docs/ANA-5.md` mentions import once, quoting `R-ID-6` at `:995` | ✓ true | — |
| `HANDOFF.md:378-379` is the MOD-9 entry's closing line; the item is open; `docs/decisions/mod/mod-9.md` does not exist | ✓ true | checklist line `HANDOFF.md:310` |
| The five real-file frontmatter shapes in T1's table | ✓ true, **one path corrected** | four confirmed verbatim. The fifth was mis-attributed: `…/copilot-sdk…/discover-resources/SKILL.md` is well-formed, has a `name`, and has no `metadata:`; the flow-map shape is the vikingbot/moltbot family (63 paths). The same file supplies the 4-space block-list fixture. |
| Block scalars occur in all three attested forms (`>`, `>-`, `|`) | ✓ true | 38 / 40 / 19 paths in the `agent-toolkit-for-aws` tree; `>+` and `|+` occur nowhere, and D93 accepts them as harmless |
| A single-line flow `metadata:` holding nested JSON occurs widely | ✓ true | 63 `SKILL.md` paths, e.g. `…/vikingbot/workspace/skills/tmux/SKILL.md` |
| Files exist whose `---` is a horizontal rule, not a fence | ✓ true | 10 paths, 5 unique files, all under `~/.gemini`'s spec-flow trees |
| **"Five `SKILL.md` files carry no `name` key"** | ✓ **false as first drafted — D95 amended** | full scan of 4,875 `SKILL.md`: 4,522 have a `name`, **2 do not**, and those two are byte-identical copies of one file. The `.mdc` half of the claim is true. The `SKILL.md`-half of T1's fallback test is therefore **synthesised**. |
| **`Unterminated` has real-file evidence** | ✓ **false as first drafted — D92 amended** | 0 unterminated frontmatters in 4,875 `SKILL.md` and 0 in 12,987 markdown frontmatter files. The variant stays as total-function insurance with a synthetic test. |
| Six incompatible `allowed-tools` encodings exist | ✓ true | full scan of 12,987 files: bare 34, comma-space 127, space-separated 22, inline list 166, quoted comma string 1, block list 202. The block list is the second most common shape, so it is not an edge case. |
| Some `allowed-tools` value mixes a comma and a space inside one token | ✓ **refuted — rationale dropped** | 0 matches for a comma not followed by a space. The reader keeps such a value as one `Scalar`, which is correct either way. |
| The repo contains no `SKILL.md` | ✓ **false as first drafted — D99 amended** | it holds three (`.claude/skills/{graphify,handoff-add,handoff-run}/SKILL.md`) and no `*.mdc`. A root-level directory import reaches them at depth 3; D99 now states that as a consequence. |
