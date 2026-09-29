# Plan: MOD-9 milestone 5 — glob attachments fire

**Status: DRAFTED, FACT-CHECKED and CONFIRMED by the maintainer 2026-09-29, OQ-27..OQ-29 at their
defaults** (OQ-27: `0008` comment migration, mirror refill accepted; OQ-28: finding 6 fixed here as
T7; OQ-29: changed paths feed excerpt tier 2). Two independent verifiers (one over `htui-core`/`htui-store`, one over
`htui-orch`/`htui-agent`/`htui` plus task independence) re-checked every claim against `7fa06e7`; their
verdicts are in "Verified claims — fact-check" at the end. **Falsified as first drafted and corrected
in place:** OQ-27's precedent (no earlier migration is comment-only) and its cost (any new migration
wipes and refills every box's SQLite mirror), D119's capture mechanism (`run_capturing` returns a
lossily decoded `String` with a truncation marker, so neither non-UTF-8 names nor truncation are
detectable from it), D124/D128's three moving snapshots (none move), T5's file list (missing
`crates/htui-agent/src/lib.rs`), T2's `promote.rs` (needs no edit), D116's "convention" wording, and
three line citations.

**Source**: `.claude/prds/mod-9-skill-library-templates.prd.md`, Delivery Milestones row 5 ("Glob
attachments fire": the F2 file set — the excerpt walk's listing under the step's roots, narrowed to
`touched_paths`, plus the previous attempt's changed paths — roots for fan-out groups, `select` with
`matched`/`no_match`, the preview's roots). `HANDOFF.md` MOD-9 entry, closing paragraph: "a `glob`
attachment still records `no_path`, and an imported file's globs prefill the form but match nothing
until row 5", plus milestone 3's review finding 6 (`library.rs` `on_skill_key`). Milestone 3's plan
D86 (`.claude/plans/mod-9-skills-editable.plan.md`), which defined row 5 and added the attachments
pane's "repos with no path row on this box" warning to it.

**Design source (binding)**: `docs/ANA-22.md` §5.4 (F2), §6 items 6, 7 and 8, §7.2 (`select(bound,
&StepFiles)`), §9 (risks: "a glob attachment silently never fires", "the walk for activation slows
assembly — it is the excerpt walk, done once per step and shared", "a same-file skill fires on an
unrelated item"), §10 (amendments). ANA-5 §4.5 (the walk, its skip rules, tier 2 "previous attempt's
changed paths") where §5.4 leans on it.

**Requirements**: `R-SKL-2` (as amended: an attachment carries the activation `always`, `glob`,
`off`), `R-SKL-3` (bindings editable — finding 6), `R-ID-5` (htui owns every skill text; the
recorded choice is what is audited, ANA-22 §6 item 8), `R-SEC-3` (a recorded path is scrubbed,
fail-closed), `R-NF-3` (the walk runs off the async runtime).

**Complexity**: Medium. Pure model and assembler work in `htui-core`, one new `Isolator` method, the
shared pass in `htui-agent` reshaped to return a file set beside the excerpts, engine and preview
wiring, two small UI changes. **No `WriteStore` method, no `StoreRequest`, no `.sqlx` file, no new
dependency.** One comment-only migration (`0008`), OQ-27.

**Routing**: continues `/handoff-run MOD-9`; this milestone routed as **plan** (C1–C3 ✗, C4
borderline), ultracode not needed, maintainer accepted 2026-09-29. Review gate `rust-reviewer`
(`.claude/workflow-config.json`). Sandbox run: branch `hr/MOD-9` (TOOL-7, `HR_SANDBOX=1`, no
docker, no push), collected on the host with `scripts/hr collect MOD-9`.

**Numbering**: continues MOD-9's own series. The highest used so far are **D107**
(`mod-9-skills-editable.blueprint.md` §12; the ported import plan re-used D87..D105 in PR #10's
namespace and is cited as "import plan D*n*"), **R-43** (`mod-9-skill-import.plan.md`) and
**OQ-26** (the same). This plan uses decisions **D108…D129**, risks **R-44…R-55**, open questions
**OQ-27…OQ-29**. Tasks restart at **T1**. MOD-7's plans also use D104..D125 and R-44.. in *their*
namespace, and `engine.rs` already cites "D108", "D122", "D125" meaning MOD-7's; **every new code
comment cites these as "MOD-9 D1xx"**.

**Tree**: every fact below was read at `b5e481f` (`hr/MOD-9`) through the Gortex MCP index
(`search`, `read` source, `relations`) and, where Gortex text search returned nothing or a file was
too large for one read, with `grep`/`sed`. The "Verified claims" table at the end carries the
evidence.

---

## Open questions for the maintainer (read these first)

- [x] **OQ-27 — Record version and a comment migration.** Adding two reasons and a `path` key makes
      the column comment on `run_step.trim_record` false: `0007_skill_attachments.sql:43-50` states
      "`v 2` … reason always, off, no_path, missing_version or not_placed", and
      `crates/htui-store/tests/migrations.rs:214-223` pins that text verbatim as the contract.
      Migrations are forward-only, so the only way to restate it is a new migration.
      **Default (D118): bump the record to `v: 3` and add `0008_trim_record_v3.sql`, a single
      `COMMENT ON COLUMN run_step.trim_record`** copying `0007` §2's statement (`0007:39-50`, text
      decided by milestone 2's D42). It would be the **first comment-only migration**: `0007` also
      carries two `ALTER TABLE`s and five skill-column comments, and none of `0001`..`0007` is
      comment-only. No DDL, no cache migration (`run_step`'s comment is Postgres-only), no `.sqlx`
      change. Next migration becomes `0009`. **Cost (fact-check):** any new migration raises
      `PgStore::schema_version()` 7 → 8 (`htui-store/src/pg/mod.rs:625-628`), and `CacheStore::open`
      then deletes and rebuilds every box's mirror file on first start
      (`htui-store/src/cache/mod.rs:116-121`, `:141-153`; `cache_migrations/0004_requirements.sql:13`
      says so in writing) — a one-time, self-healing refill per box, for a comment (R-56).
      **Alternative:** bump the record to `v: 3` in code but add no migration; the pinned column
      comment stays at `v 2`'s text until the next real migration restates it (T3 dropped, the
      migration pins unchanged, no mirror refill).
- [x] **OQ-28 — Milestone 3's review finding 6 (the Skills view cannot open an editor on a skill
      with no version).** MOD-9 closes with this row, so the finding either lands here or becomes a
      new item. The fix is UI-only: `WriteStore::add_skill_version` already takes `expected = 0` as
      "no version yet" (`crates/htui-core/src/store/traits.rs:852`), and only a hand-written row can
      reach the state (`create_skill` writes v1 with the row). **Default (D127): take it, as the
      independent task T7** (`library.rs` plus one Postgres test). **Alternative:** leave it and
      open a small MOD item at close-out with `/handoff-add`.
- [x] **OQ-29 — Feed the same changed paths to excerpt tier 2.** The list row 5 builds for skills is
      exactly ANA-5 §4.5's tier-2 input, which MOD-7 milestone 4 left empty only because no list
      existed (MOD-7 plan D122; `crates/htui-agent/src/excerpt.rs:1044-1045`, "the previous
      attempt's diff carries no repo-qualified path list"). **Default (D121): yes**, `PassInput`
      carries `changed_paths` into the excerpt request too, so a retry's excerpts rank the previous
      attempt's files at tier 2 (weight 90). Retry prompts' excerpt sets and digests move (R-52).
      **Alternative:** skills only; `ExcerptRequest.changed_paths` stays `Vec::new()`.

Not questions (ANA-22 already decided them): the narrowing to `touched_paths` prefixes when the item
has any (§5.4 F2), a bare glob matching in any repo of the step's scope (§6 item 6), `no_path` when no
root resolves and the step proceeding (§6 item 7), recording every candidate (§6 item 8).

---

## Summary

**Model (T1, pure, `htui-core`).** `ChoiceReason` gains `Matched` and `NoMatch`; `SkillChoice`
gains `path: Option<String>` (`<repo>:<path>`, present only for `matched`). A new `StepFiles` (repo
slug → byte-ordered set of repo-relative paths, plus which repos were *reached*) is `select`'s third
argument. A `glob` winner compiles its globs once (`SkillGlobs::compile`), takes the **first**
matching path in `(repo bytes, path bytes)` order, and records `matched`; with no match it records
`no_match` when some repo its globs can reach was listed, else `no_path`.

**Assembler and the walk (T2, `htui-core`).** `PromptSpec` gains `step_files: StepFiles` (default:
nothing reached, i.e. today's `no_path`). `excerpt::select` is split into `list` (steps 1-2, the
walk) and `select_listed` (steps 3-10), so the listing exists once and can be shared; `step_files`
builds the F2 set from a `Listing`, the item's touched prefixes and the changed paths. The matched
path is masked like a skill name. The record goes to `v: 3`.

**Migration (T3).** `0008_trim_record_v3.sql` restates the `trim_record` comment (OQ-27).

**Changed paths (T4, `htui-orch`).** `Isolator::changed_paths` — `git diff --name-only -z
--no-renames` over the same ranges `diff` already uses — on the real, fake and test isolators.

**The shared pass (T5, `htui-agent`, engine, preview).** `excerpts_for` becomes `step_pass`, which
walks once (for `{{excerpts}}`, or for a placed `glob` winner even when the body has no
`{{excerpts}}`), builds the file set, computes the residual with the file set in place, and runs the
excerpt selection over the same listing. The engine feeds it the previous attempt's changed paths;
fan-out groups get it over MOD-7's `repo_box_path` roots; judges record `no_path`; the preview gets
the same pass.

**UI (T6, T7, `htui`).** The Prompt sub-tab shows `matched <repo:path>` / `no_match`; the
attachments pane drops "fires from milestone 5" for a "no path here" warning; finding 6 is fixed
(OQ-28).

**Close-out (T8).** MOD-9 is done: write-up, index line, HANDOFF line removed, PRD row 5 complete,
ANA-22 §10 amendment.

---

## Design decisions (settled here, not in code review)

| # | Decision | Why / evidence |
|---|---|---|
| D108 | **Scope.** Row 5 is the last open part of MOD-9 and closes it. It adds **no** `WriteStore` method, `StoreRequest`/`StoreReply` variant, `.sqlx` file, table, column, dependency or SQLite-mirror change; `GraphSource::bound_skills` is unchanged (it returns candidates; selection stays in the assembler). The one migration is OQ-27's comment; it moves `schema_version()` and so refills the SQLite mirror once per box (R-56), with no mirror schema change. | `globset` is already an `htui-core` dependency (milestone 3 D72). The file set is built from data the pass already has (roots, touched prefixes) plus one isolator read. Nothing new is persisted except inside `run_step.trim_record`, which is JSON. |
| D109 | **Reasons and the record row.** `ChoiceReason` gains `Matched` and `NoMatch` (serde snake_case `matched`, `no_match`; `as_str` the same), stays `Copy + Hash`; no existing variant is renamed (the promise in `skill.rs:378-380`). `SkillChoice` gains `pub path: Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`: `Some("<repo>:<path>")` **iff** `reason == Matched`, `None` otherwise. `active` is true iff the reason is `Always` or `Matched`. | ANA-22 §6 item 8's `matched: <path>`, spelled as a separate key so the reason stays a closed, `Copy` vocabulary. `<repo>:<path>` is the qualified-glob and `touched_paths` syntax, so the recorded path reads in the language the glob was written in and is unambiguous across repos. Skipping `None` keeps every existing choice's JSON byte-identical. |
| D110 | **`StepFiles`** in `htui_core::model::skill` (pure data, no `prompt` import): `#[derive(Debug, Clone, Default, PartialEq, Eq)] pub struct StepFiles { repos: BTreeMap<String, BTreeSet<String>> }` keyed by repo slug. API: `reach(&mut self, repo: &str)` (a repo that was listed, even to zero files), `insert(&mut self, repo: &str, path: &str)` (reaches the repo too), `is_reached(&self, repo: &str) -> bool`, `repos(&self) -> impl Iterator<Item = (&str, &BTreeSet<String>)>` in repo byte order, `retain(&mut self, f: impl FnMut(&str, &str) -> bool)` (the scrub filter), `is_empty`. `Default` reaches nothing. | ANA-22 §7.2 sketches `Resolved(paths by repo) \| NoPath`; a step's scope can be half-resolved (one repo with a tree, one without), so reach is per repo. `BTreeMap<String, _>`/`BTreeSet<String>` order by bytes (`str`'s `Ord`), which D113 needs, and dedup for free. |
| D111 | **`select(candidates: Vec<BoundSkill>, placed: bool, files: &StepFiles) -> (Vec<BoundSkill>, Vec<SkillChoice>)`.** Rules, first match wins, as today for the first three: `!placed` → `not_placed`; `version: None` → `missing_version`; `Off` → `off`; `Always` → `always`; **`Glob`**: `SkillGlobs::compile(&skill.globs)` once for this candidate on this step; on `Err` → `no_match` (D126's R-53); else for each `(repo, paths)` of `files.repos()` take `globs.first_match(repo, paths.iter().map(String::as_str))`; the first hit is `matched` with `path = format!("{repo}:{path}")`; with none, `no_match` if `files.repos()` holds a reached repo for which `globs.reaches(repo)` (D112), else `no_path`. Also `pub fn needs_files(candidates: &[BoundSkill]) -> bool` = `BoundSkill::collapse(candidates.to_vec())` holds a `Glob` winner with `version.is_some()`. | `SkillGlobs` and `first_match` were built for exactly this call (`skill_glob.rs:178-215`, "Tested now, called there"). Compiling per candidate per step is "compiled once per step"; a step has a handful of candidates. `placed` stays because a body without `{{skills}}` must still record `not_placed` (milestone 2 D40). `needs_files` is what lets the pass skip the walk when nothing could use it. |
| D112 | **`SkillGlobs::reaches(&self, repo: &str) -> bool`**: any glob unqualified, or qualified with `repo`. | "No path" must mean "no repo this attachment can match in was listed": `web:**/*.ts` on a step that listed only `htui` has no path for `web`, which is `no_path`, not `no_match`. |
| D113 | **Deterministic match (`R-ID-5`).** When many files match, the recorded path is the first in **`(repo bytes, path bytes)`** order, never walk order. The walk sorts per directory level and descends as it meets a directory (`htui-agent/src/excerpt.rs:255`, `:323`), so it yields `a/x.rs` before `a.rs` while byte order of whole paths puts `a.rs` first (`.` 0x2E < `/` 0x2F); `StepFiles`' sets impose byte order regardless of how the listing arrived. | Two boxes with the same tree record the same path, and a changed walk order can never move a record. |
| D114 | **The walk is split out of `excerpt::select`, once per step.** In `htui-core/src/prompt/excerpt.rs`: `pub struct Listing { pub roots: Vec<RootRecord>, pub files: Vec<RepoPath>, pub listed: Vec<String>, pub notes: Vec<String> }` (`listed`: repos whose `RepoReader::list` succeeded, byte order); `pub fn list(reader: &dyn RepoReader, req: &ExcerptRequest<'_>) -> Listing` is today's steps 1-2 verbatim (roots sorted by repo bytes, `NoPath` and unlistable roots noted, `skip_by_path` with the declared-denied notes and per-rule counts, non-repo-relative paths dropped); `pub fn select_listed(reader, req, listing: &Listing, merged, providers, est) -> ExcerptSet` is steps 3-10, starting its notes with the F-101 caps note and then `listing.notes`, exactly the old order; `select` keeps its signature and becomes `select_listed(reader, req, &list(reader, req), …)`. | ANA-22 §9: "It is the excerpt walk, done once per step and shared." Today `select` builds `listing` as a local and returns only the `ExcerptSet` (`excerpt.rs:1081-1168`), so a second consumer would have to walk again. Keeping `select`'s signature keeps every existing `htui-core` excerpt test as the regression net for the split. |
| D115 | **The F2 set**, `pub fn step_files(listing: &Listing, touched: &[PathPrefix], changed: &[RepoPath]) -> StepFiles` in `prompt/excerpt.rs`: every repo in `listing.listed` is reached; a listed file is inserted when `touched` is empty **or** some `PathPrefix::matches(repo, path)` (global narrowing: with touched paths only in `htui`, a listed `docs` repo contributes no file but is still reached); then every changed path is inserted when `is_repo_relative` and `skip_by_path` is `None` (it reaches its repo even with no root). | §5.4 F2 verbatim: "restricted to `touched_paths` prefixes when the item has any, else the whole `run.repo_scope` checkout; plus the previous attempt's changed paths". The same skip rules as the walk, so a denied path is match evidence nowhere. `PathPrefix` and `touched_prefixes` are MOD-7's (`excerpt.rs:54-97`, `htui-agent/src/excerpt.rs:934-943`), bare touched globs going to the primary repo. |
| D116 | **Scrub filter for the file set**: `pub fn drop_unmaskable_files(files: &mut StepFiles, scrubber: &dyn Scrubber) -> Option<String>` in `prompt/mod.rs` beside `drop_unmaskable_excerpts` (`:916`), removing every `(repo, path)` for which `refused_rule` (`:991`) fires on the repo or the path; one note, count only: ``skills: {n} path(s) withheld from glob matching; the scrubber refused them``. Called by `step_pass` before it returns. | A matched path lands in `trim_record`, which `to_value` scrubs fail-closed (`trim.rs:272-276`): an unmaskable path would abort the run (the cost spelled out at `trim.rs:245-262`). Filtering first means a refused path can never be the recorded match. The note never names the path — stricter than `drop_unmaskable_excerpts`, which names a `repo:path` the scrubber leaves unchanged (`prompt/mod.rs:902-911`): count only, never a name. |
| D117 | **`PromptSpec.step_files: StepFiles`** (`prompt/mod.rs:71-123`). `scrubbed_inputs` calls `select(BoundSkill::collapse(spec.skills.clone()), placed, &spec.step_files)` (`:861`), then masks each `choice.path` with `mask(scrubber, path, &SectionName::Skills.render())?` as it masks names (`:778-780`). The digest is untouched by the path (it is not rendered); a `matched` skill renders exactly the bytes an `always` one would. `excerpt_residual` inherits `step_files` through `..spec.clone()` (`:533-536`). | Default `StepFiles` reproduces today's behaviour for every fixture, handoff and judge. `trim.rs:241-243`'s list of fields that "arrive already masked" gains `skill_choices[].path`. |
| D118 | **Record `v: 3` and `0008_trim_record_v3.sql` (OQ-27).** `RECORD_VERSION = 3` (`trim.rs:56-58`, doc updated). The migration is one statement: `COMMENT ON COLUMN run_step.trim_record IS 'ANA-5 5.1 as amended by MOD-9 D42 and D118: {v, template, budget, budget_source, reserve, target, estimator, estimated_before, estimated_after, sections[], skill_choices[], excerpts, notes}, v 3. skill_choices[] is every candidate skill, ordered by position then name, each {skill, name, version, level, activation, active, reason} with reason always, matched, no_match, off, no_path, missing_version or not_placed (ANA-22 6 item 8); a matched choice adds path, <repo>:<path>, the first matching file in repo then path byte order. A v 2 record, written before 0008, has no matched or no_match; a v 1 record, written before 0007, has no skill_choices. Canonical; the prompt payload sections[] array is its abridged projection. Written at stage 3 by set_step_prompt, before the session starts.'` The demo's stored `IMPL_TRIM_RECORD` (`crates/htui-core/src/fixtures.rs:1640`, the demo module, not `prompt/fixtures.rs`) stays `"v": 2`: it is a record written before `0008`. | `0007` §2 restated the comment for `v 2` (milestone 2 D42's text) inside a migration that also changed the schema; `0008` would be the first comment-only one. The comment is the column's contract and is pinned verbatim (`migrations.rs:214-224`, compared byte for byte at `:429-450`). |
| D119 | **`Isolator::changed_paths`** (`htui-orch/src/isolate.rs`, beside `diff` at `:186`): `fn changed_paths<'a>(&'a self, trees: &'a [RunStepTree], commits: &'a [RunStepCommit]) -> IsolatorFuture<'a, ChangedPaths>` with `pub struct ChangedPaths { pub paths: Vec<(RepoId, String)>, pub truncated: bool }`, sorted by `(repo id, path bytes)`, deduplicated. No default body: every implementor must answer. **Real** (`isolate/real.rs`): the range selection of `diff_of` (`:1287-…`: copy tree vs checkout, D141's reconcile-merge first parent, D146) is factored into one private `range_of` both use; per committed row, `Cli::name_only(repo, before, after)` runs `git diff --name-only -z --no-renames --no-ext-diff --no-textconv <before> <after> --` with the same argument discipline as `Cli::diff` (`git.rs:815-845`) but through a **new byte capture**: `Capture::HeadBytes` (beside `Capture::Head`, `git.rs:424-431`) keeps the first 64 KiB as raw bytes and reports `overflowed: bool`, and `run_capturing` returns them without decoding. Fact-check: today's path cannot serve this — `Exited.stdout` is a `String` decoded lossily (`git.rs:937-948`) and `HeadBuffer::into_string` (`:1116-1122`) appends `"\n[diff truncated at 64 KiB]"` on overflow, so a non-UTF-8 name becomes an undetectable U+FFFD string and truncation is text, not a flag. With bytes: NUL-split; an entry that is not valid UTF-8 is dropped; when `overflowed`, the last (partial) entry is dropped and `truncated` is set. No usable `git` is `Ok(ChangedPaths::default())`, as `diff` answers `Ok(None)`. **Fake** (`fake.rs`): `script_changed_paths`/`fail_changed_paths` queues, default empty. **`StallAfterReconcile`** (`tests/gix_isolator.rs:1987`): delegates. | Nothing records an attempt's changed paths as a list: `DiffBlock` is `{range, stat, diff}` text (`prompt/mod.rs:159-166`) and `Isolator::diff` is the only reader (`isolate.rs:176-190`). Parsing the unified patch would be a second diff parser; asking git is exact. The session event log's `EditProposalEvent.path` (`htui-agent/src/event.rs:338-345`) is agent-claimed and per proposal, not the committed range, so it is not a substitute. `--no-renames` lists a rename as its old and new path, so a moved `.rs` file counts under both names; `-z` removes quoting. Three implementors exist (`real.rs:1387`, `fake.rs:409`, `tests/gix_isolator.rs:1987`). |
| D120 | **The shared pass, `htui_agent::excerpt::step_pass`** replaces `excerpts_for` (`htui-agent/src/excerpt.rs:1000-1085`): `pub async fn step_pass(spec: &PromptSpec, input: PassInput, app: &BTreeMap<String, Value>, scrubber: &dyn Scrubber) -> StepPass` with `pub struct StepPass { pub excerpts: ExcerptSet, pub files: StepFiles }`. `PassInput` (`:947-955`) gains `pub changed_paths: Vec<RepoPath>`. New blocking helpers: `pub fn walk_pass(req: &OwnedExcerptRequest) -> Listing` (`list` over `FsRepoReader::new(req.caps)`) and `excerpt_pass(req, listing: &Listing, est) -> ExcerptSet` (providers, then `select_listed`). Flow: (1) parse the body: `places_excerpts`, `places_skills`; (2) `wants_files = places_skills && needs_files(&spec.skills)`; (3) neither → today's `unscanned` set and the unchanged note, `files = step_files(&Listing::default(), &touched, &changed)`; (4) `readable` as today; walk under `spawn_blocking` when readable, else `list` inline (no I/O, notes the `no_path` roots); (5) `files = step_files(…)`, then D116's filter; (6) `!places_excerpts` → the excerpt set is empty with `audit.roots = listing.roots`, `considered 0`, and notes = caller's, then ``excerpt: template `…` places no {{excerpts}}; the walk listed files for glob skills only``, then `listing.notes`; (7) otherwise the residual is `excerpt_residual` over the spec **with `step_files` set**, then `excerpt_pass` under `spawn_blocking` over the same listing, then today's scrub filters. A `JoinError` in either hop is today's note and an unscanned set; the file set then holds only changed paths. | One walk per step (§9). Computing the residual after the file set is known means a newly active skill's tokens are paid before excerpts are chosen, not trimmed off afterwards. Running the walk for a placed glob winner even without `{{excerpts}}` is what makes the default `verdict` body (`defaults.rs:132`, `{{skills}}` only) and custom bodies fire; the walk costs what the excerpt walk costs and runs only when a glob winner is placed. |
| D121 | **Tier 2 (OQ-29).** `step_pass` puts `input.changed_paths` into `OwnedExcerptRequest.changed_paths`, replacing MOD-7 D122's `Vec::new()` and its comment. | The same list, already computed; ANA-5 §4.5's tier 2 and its declared-denied notes (`excerpt.rs:1131-1145`) finally see it. |
| D122 | **Engine** (`htui-orch/src/engine.rs` `with_excerpts`, `:4974-5018`): for a `TemplateRole::Phase` spec with `step.attempt > 1`, a new private `changed_paths(run, step, &scope, &mut notes)` reads `run_steps`, `winner_at(&steps, step.position, step.attempt - 1)` (`status.rs:243`), that winner's `step_trees` and `step_commits`, calls `isolator.changed_paths`, and maps each `RepoId` to its scope name (an id outside the scope is dropped). An isolator error is the note ``changed paths unavailable: {err}`` (the `previous_diff unavailable` shape, `:5239-5244`) and an empty list; `truncated` adds ``changed paths: the list was cut at 64 KiB``. No winner → empty, no note (`forwarded` already notes it, `:5205-5211`). Then `step_pass(spec, input, …)` sets `spec.excerpts` and `spec.step_files`. **Fan-out groups** need nothing more: `drive_group` assembles on the first pending step before any candidate tree exists (`:3478-3481`), and MOD-7 D108 already resolves those roots from this box's `repo_box_path` rows — the managed checkout every candidate tree is cut from (`run_worker.rs:1130-1150` builds the isolator's repo map from the same rows) — so the group's file set is that checkout's listing plus, from attempt 2, the previous winner's changed paths. | `forwarded` (`:5198-5247`) reads the same winner for `previous_diff`; three more store reads on a retry are cheaper than threading a second value out of `phase_spec`, whose other caller (`opening`) must not do this work (MOD-7 D125). |
| D123 | **Judges and handoffs.** The judge's spec (`engine.rs:4418-4450`) sets `step_files: StepFiles::default()`, with a comment: a judge runs in no tree (`prepare(…, &[], Isolation::Local, None)`, `:4581`) and gets no excerpt pass (MOD-7 D109), so a placed `glob` winner records `no_path` truthfully; the default `judge` body places no `{{skills}}` (milestone 2 D49 withdrawn, `engine.rs:13291`), so it records `not_placed`. The handoff empties `skills` (`promote.rs:99`); `..phase` carries `step_files` unused. | §6 item 7: no resolvable root → `no_path`, the step proceeds. The judge's `{{task}}` already replays the judged step's prompt, skills included. |
| D124 | **Preview** (`crates/htui/src/preview.rs:257-340`): `step_pass` replaces `excerpts_for` with `changed_paths: Vec::new()` (attempt 1), and sets `spec.step_files`. `SKILLS_NOTE` (`:84-88`) becomes: ``preview: phase-level skills come from the first phase of the item's graph that uses this template; glob attachments match this box's repo_box_path listing, narrowed to touched_paths, with no previous attempt``. `STAND_INS` stays 8. **No snapshot moves** (fact-check): the three snapshots that print the note (`prompt_preview__preview_feat_1`, `prompt_preview__preview_ana_2`, `backlog__detail_prompt`) clip it at the pane edge before "uses this template; ", where old and new text are still identical, and the demo holds no `glob` attachment. A move under `cargo insta review` is a regression. | PRD row 5's "the preview's roots": the preview already runs the engine's pass (MOD-7 P-1), so it fires globs over the same roots and the note stops claiming `no_path`. |
| D125 | **Prompt sub-tab** (`ui/tabs/backlog/detail/prompt.rs` `choice_line`, `:214-229`): the outcome is `active` for `always`, `matched <repo:path>` for `matched` (CR/LF → space, as names), and the reason's `as_str` otherwise (`no_match`, `no_path`, …). | ANA-22 §9's first mitigation: "`no_path` recorded … and shown in the preview". A matched skill should say which file woke it. |
| D126 | **Attachments pane** (`ui/tabs/skills/attach.rs`): `FIRES_LATER` (`:56-57`, pushed at `:753-755`) and the module doc's milestone-5 sentence (`:12-13`) go. `crate::skills::ProjectSkills` (`skills.rs:57-68`) gains `pub unrooted: Vec<String>`: the project's repo names with no `repo_box_path` row for this box, byte order (every repo when the box is unregistered). `snapshot` (`skills.rs:146`) gains `this_box: Option<BoxId>`, read in `serve` with `backend.box_info()` (the `hierarchy::serve` shape, `hierarchy.rs:221-225`), and reads this box's rows once with `backend.repo_paths(box_id)` (`backend.rs:526`, the preview's read) rather than `WriteStore::repo_box_paths(repo)` per repo. On a project or phase `glob` row, when `SkillGlobs::compile(&globs)` reaches (D112) an unrooted repo, the summary appends ``no path here: <a>, <b>``; a global row adds nothing (it spans every project). | ANA-22 §9's second mitigation ("the editor warns when the scoped project has repos with no path row") and milestone 3's D86. No `StoreRequest` changes: the snapshot reply gains a field. |
| D127 | **Finding 6 (OQ-28).** `LibraryView::on_skill_key` (`library.rs:705-717`) returns early when the skill has no head. `e` and `E` on such a skill open the editor on an empty body with head token `0`; save sends `SaveSkillVersion { expected: 0, … }`, which `add_skill_version` writes as v1 (`traits.rs:852`'s order). Version stepping, base and diff keys stay inert with no version. | The writer already supports it; only the view's guard is missing. |
| D128 | **Pins.** Unchanged: store `CASES` 96, `READ_CASES` 14, `htui-orch` `CASES` 73, `GraphSource` 7 methods, `StoreRequest` 85, `StoreReply` 47, `skills::REQUEST_NAMES` 6, `.sqlx` 288, `TABLES` 39, commented columns 34 (restated, not added), `MIRRORED_TABLES` 21, 107 `crates/htui/tests/snapshots` (none changes content, D124). Moved: `RECORD_VERSION` 2 → 3; migrations `0001`..`0008`, next **`0009`** (cache `0005`), `Pending(7)` → `Pending(8)`; `Isolator` gains one method. | Counted at `b5e481f` (HANDOFF live coordinates `:36-44`; `ls crates/htui-store/.sqlx \| wc -l` = 288; `ls crates/htui/tests/snapshots \| wc -l` = 107). |
| D129 | **Scope fence.** Not in row 5: model-decided activation (ANA-22 §6 item 10, needs MOD-11), content-regex activation, a language-map overlay, a Settings surface, re-listing per fan-out candidate, a per-step record of the file set itself (only the choice and its path are recorded — ANA-22 §9 "one short row per bound skill"), changes to `touched_paths` semantics or to the attachments form. | Each is named elsewhere or explicitly deferred. |

## Patterns to mirror

| Concern | Mirror | Where |
|---|---|---|
| A pure selection over a caller-supplied set | `select` (milestone 2), `rank` | `model/skill.rs:440-470`; `prompt/excerpt.rs:789` |
| Compiled attachment globs, qualifier rule | `SkillGlobs::compile`, `first_match` | `model/skill_glob.rs:178-215` |
| Touched-path prefixes, repo-qualified | `PathPrefix::parse`/`matches`, `touched_prefixes` | `prompt/excerpt.rs:54-97`; `htui-agent/src/excerpt.rs:934-943` |
| Count-only scrub filter with a note | `drop_unmaskable_excerpts`, `refused_rule` | `prompt/mod.rs:916`, `:991` |
| Blocking walk off the runtime, fail-open on `JoinError` | `excerpts_for` | `htui-agent/src/excerpt.rs:1054-1076` |
| An advisory isolator read that degrades to a note | `forwarded` / `Isolator::diff` | `engine.rs:5198-5247`; `isolate/real.rs:1447-1470` |
| A git read with fixed flags and a head-capped capture | `Cli::diff` | `isolate/git.rs:815-845` |
| Comment-only migration restating `trim_record` | `0007`'s section 2 | `migrations/0007_skill_attachments.sql:39-50` |
| Snapshot read with this box's identity | `hierarchy::serve` | `crates/htui/src/hierarchy.rs:221-230` |
| Engine test over a real temp tree | `excerpt_prologue`, `a_step_without_trees_reads_repo_box_path_with_a_note` | `engine.rs:12525-12540`, `:12745` |

## Files to change

| File | Create/edit | Task |
|---|---|---|
| `crates/htui-core/src/model/skill.rs` | edit | T1 |
| `crates/htui-core/src/model/skill_glob.rs` | edit | T1 |
| `crates/htui-core/src/model/mod.rs` | edit (re-export `StepFiles`, `needs_files`) | T1 |
| `crates/htui-core/src/prompt/mod.rs` | edit | T1 (call site), T2 |
| `crates/htui-core/src/prompt/excerpt.rs` | edit | T2 |
| `crates/htui-core/src/prompt/trim.rs` | edit | T2 |
| `crates/htui-core/src/prompt/fixtures.rs` | edit (four literals) | T2 |
| `crates/htui-core/tests/prompt_skills.rs` | edit | T1 (literal), T2 |
| `crates/htui-core/tests/prompt_digest.rs` | edit (`v` 3) | T2 |
| `crates/htui-store/tests/skill_attachments.rs` | edit (`select` call) | T1 |
| `crates/htui-store/migrations/0008_trim_record_v3.sql` | create | T3 |
| `crates/htui-store/tests/migrations.rs` | edit | T3 |
| `crates/htui-store/tests/connect.rs` | edit | T3 |
| `crates/htui-orch/src/isolate.rs` | edit | T4 |
| `crates/htui-orch/src/isolate/git.rs` | edit (`Capture::HeadBytes`, `name_only`) | T4 |
| `crates/htui-orch/src/isolate/real.rs` | edit | T4 |
| `crates/htui-orch/src/fake.rs` | edit | T4 |
| `crates/htui-orch/src/lib.rs` | edit only if `ChangedPaths` is re-exported beside the isolator types | T4 |
| `crates/htui-orch/tests/gix_isolator.rs` | edit | T4 |
| `crates/htui-orch/src/engine.rs` | edit | T2 (two literals), T5 |
| `crates/htui-agent/src/excerpt.rs` | edit | T5 |
| `crates/htui-agent/src/lib.rs` | edit (re-exports `:148-151`: `step_pass`, `StepPass`, `walk_pass` replace `excerpts_for`) | T5 |
| `crates/htui-agent/tests/excerpt.rs` | edit | T5 |
| `crates/htui/src/preview.rs` | edit | T2 (literal), T5 |
| `crates/htui/tests/prompt_preview.rs` | edit | T1 (literal), T5 |
| `crates/htui/src/ui/tabs/backlog/detail/prompt.rs` | edit | T1 (test literal), T6 |
| `crates/htui/src/skills.rs` | edit | T6 |
| `crates/htui/src/ui/tabs/skills/attach.rs` | edit | T6 |
| `crates/htui/tests/skills.rs` | edit | T6 |
| `crates/htui/src/ui/tabs/skills/library.rs` | edit | T7 |
| `crates/htui/tests/skills_pg.rs` | edit | T7 |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-9.md` (create), `.claude/prds/mod-9-skill-library-templates.prd.md`, `docs/ANA-22.md`, this plan's Status line | edit/create | T8 |

## Tasks

**Order: T1 → T2 → T3 → T4 → T5 → T6 → T7 → T8, serial.** Why serial: T1, T2 and T5 each change a
type every later crate compiles against (`select`'s signature, `SkillChoice`, `PromptSpec`,
`excerpts_for`), so a parallel task in `htui-orch` or `htui` would build against a moving
`htui-core`; T2, T4 and T5 all edit `engine.rs`; T5, T6 and T7 all live in the `htui` crate, whose
tests share the snapshot directory and the one Postgres. **Genuinely parallel-safe, if a second
worktree is wanted:** T3 (only `htui-store/migrations/` and two `htui-store` test files; its text is
fixed by D118) may run ∥ T4; T7 (two `htui` files no other task touches) may run ∥ T3, and ∥ T4 **only in a
separate worktree** (`htui` depends on `htui-orch`, which does not build while T4's trait method is
half-implemented), but not ∥ T5/T6 (same crate, same test binary set; `library.rs` tests call
`skills::serve`, which T6 changes). T4 → T5 is a semantic dependency though their files are disjoint
(T5 calls T4's method). In this sandbox the default is one tree, serial (memory:
~10G `target/` per worktree; `/` fills first).

Every implementer prompt carries: D108–D129 and the OQ answers win over prose; tree reads through
Gortex first; red tests committed before green; no test skipped or loosened; a moved pin names its
reason; every new `pub` item has a doc comment; new code comments cite "MOD-9 D1xx"; explicit-path
`git add`, never `-A`, never stash or amend; commit after each task.

### T1 — model: reasons, `StepFiles`, `select` (D109-D113)
- **Files (complete)**: `crates/htui-core/src/model/skill.rs`, `crates/htui-core/src/model/skill_glob.rs`,
  `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/prompt/mod.rs` (the one `select` call at
  `:861` passes `&StepFiles::default()` until T2), `crates/htui-core/tests/prompt_skills.rs`
  (`SkillChoice` literal `:85` gains `path: None`), `crates/htui-store/tests/skill_attachments.rs`
  (`:300`), `crates/htui/src/ui/tabs/backlog/detail/prompt.rs` (test literal `:480`),
  `crates/htui/tests/prompt_preview.rs` (helper literal `:357`). No snapshot, seed, `.sqlx` or
  migration.
- **Tests first** (unit, `skill.rs` and `skill_glob.rs`):
  `select_matches_a_glob_against_the_step_files` (`**/*.rs` over `htui:{src/lib.rs}` → `matched`,
  `path = "htui:src/lib.rs"`, active, the skill in the active list);
  `the_first_match_is_the_first_in_repo_then_path_byte_order` (inserted as `htui:a/x.rs`,
  `htui:a.rs`, `api:z.rs` → `api:z.rs`; without `api`, `htui:a.rs`);
  `a_glob_over_a_reached_repo_with_no_match_records_no_match` (a reached repo with zero files too);
  `a_glob_reaching_no_listed_repo_records_no_path` (default `StepFiles`; and `web:**` over only
  `htui` reached); `a_glob_that_does_not_compile_records_no_match`;
  `select_applies_its_rules_in_order` extended (a matching file does not beat `not_placed`,
  `missing_version` or `off`); `choices_serialize_their_documented_keys` extended (no `path` key
  unless matched; `"path"` present for matched; the `as_str` loop covers seven reasons);
  `needs_files_sees_only_a_glob_winner_with_a_version` (a project `glob` under a phase `off` → false;
  a `glob` with `version: None` → false); `reaches_follows_the_qualifier` (`skill_glob.rs`).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo build --workspace
  --all-targets --all-features` (the literal fixes in other crates).

### T2 — assembler, listing split, F2 builder, record v 3 (D114-D118)
- **Files (complete)**: `crates/htui-core/src/prompt/mod.rs`, `crates/htui-core/src/prompt/excerpt.rs`,
  `crates/htui-core/src/prompt/trim.rs`, `crates/htui-core/src/prompt/fixtures.rs` (literals `:161`,
  `:259`, `:361`, `:499` gain `step_files: StepFiles::default()`), `crates/htui-core/tests/prompt_skills.rs`,
  `crates/htui-core/tests/prompt_digest.rs` (`:1007-1011` → `json!(3)`, "MOD-9 D118 bumped the
  record"), `crates/htui-orch/src/engine.rs` (literals `:4418` judge and `:5111` phase gain
  `step_files: StepFiles::default()`; T5 replaces the phase one's source), `crates/htui/src/preview.rs` (literal `:276`, default until T5). No
  snapshot moves (the default reproduces today's output); no `.sqlx`. `promote.rs` needs no edit:
  both its literals (`:105` `..phase`, `:429` `..phase.clone()`) inherit the new field. Refresh the
  stale doc prose at `prompt/mod.rs:25`.
- **Tests first**:
  - `prompt_skills.rs`: `a_glob_skill_records_no_path_before_milestone_3` becomes
    `a_glob_skill_with_no_step_files_records_no_path`; new `a_matched_glob_renders_like_always_and_records_the_path`
    (same digest and text as the `always` variant, `trim.skill_choices[1]` differs only by
    `activation`, `reason`, `path`); `a_glob_with_no_match_renders_nothing`;
    `a_matched_path_is_masked_in_the_record` (repo or path holding a session secret →
    `[REDACTED]` in `trim`, absent from `to_value`); `the_skills_cap_counts_a_matched_glob`;
    `the_digest_moves_only_when_the_active_set_moves` keeps its assertions with its "before
    milestone 3" message reworded.
  - `prompt/excerpt.rs` unit: `select_is_list_then_select_listed` (every existing in-memory reader
    fixture yields an identical `ExcerptSet` both ways, notes in the same order);
    `select_listed_never_lists` (a counting reader: `list` once per root in `list`, never in
    `select_listed`); `step_files_narrows_to_touched_prefixes`;
    `step_files_keeps_the_whole_listing_without_touched_paths`;
    `step_files_adds_changed_paths_after_the_narrowing`;
    `step_files_drops_a_denied_or_non_relative_changed_path` (`.env`, `/abs`, `../x`);
    `a_repo_that_could_not_be_listed_is_not_reached`.
  - `prompt/mod.rs` unit: `drop_unmaskable_files_withholds_and_counts_without_naming`.
  - `prompt_digest.rs`: `v` is 3; keys still thirteen.
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo build --workspace
  --all-targets --all-features`.

### T3 — migration `0008` (D118, OQ-27)
- **Files (complete)**: `crates/htui-store/migrations/0008_trim_record_v3.sql` (new; header comment
  in `0007`'s style, "Forward-only (R-STO-5)", one `COMMENT ON COLUMN`, D118's text verbatim),
  `crates/htui-store/tests/migrations.rs` (applied list `:86-93` → `1..=8` with `0008` named;
  the doc comment `:181-183` "the text 0007 restates" → `0008`;
  the `trim_record` comment row `:214-223` → D118's text; `Pending(7)`/"seven embedded migrations"
  `:884-885`, `:983`, `:988`, `:1007` → 8 / "eight"), `crates/htui-store/tests/connect.rs`
  (`:141-159`, `:243-245`: `Pending(8)` and the message naming `0008`). No `.sqlx` change, no
  cache migration, `TABLES` 39 and the commented-column count 34 unchanged.
- **Tests first**: the pins above flipped red, then the migration. **Dropped entirely if OQ-27 takes
  the alternative** (no migration; `RECORD_VERSION` still goes to 3 in T2).
- **Validate**: `cargo test -p htui-store --all-features -- --test-threads=1` (Postgres);
  `cargo sqlx prepare --check` from `crates/htui-store` against a scratch DB migrated to `0008`
  (`docs/hr-sandbox.md` "Changing SQL queries in a run").

### T4 — `Isolator::changed_paths` (D119)
- **Files (complete)**: `crates/htui-orch/src/isolate.rs`, `crates/htui-orch/src/isolate/git.rs`,
  `crates/htui-orch/src/isolate/real.rs`, `crates/htui-orch/src/fake.rs`,
  `crates/htui-orch/src/lib.rs` (only if `ChangedPaths` joins the isolator re-exports),
  `crates/htui-orch/tests/gix_isolator.rs`. No store, snapshot or `.sqlx` change.
- **Tests first**: `git.rs` `name_only_lists_every_changed_path_with_renames_split` (real `git`:
  an add, a modify, a rename, a delete, a name with a space and one with a newline → exact list);
  `name_only_drops_a_non_utf8_name` (a raw `\xff\xfe.rs`); `head_bytes_capture_reports_overflow`;
  `name_only_of_an_empty_range_is_empty`; `name_only_drops_a_partial_last_entry_when_capped`;
  `gix_isolator.rs` `changed_paths_of_a_worktree_step_names_its_committed_paths` and
  `changed_paths_of_a_reconcile_merge_uses_the_first_parent` (D141's case, mirroring the existing
  `diff` tests); `fake.rs` scripted answer round-trip.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`.

### T5 — the shared pass, engine and preview (D120-D124)
- **Files (complete)**: `crates/htui-agent/src/excerpt.rs` (incl. the stale intra-doc link `:24`),
  `crates/htui-agent/src/lib.rs` (re-exports `:148-151`), `crates/htui-agent/tests/excerpt.rs`
  (`excerpt_pass` calls `:1114`, `:1144` take a listing; `excerpts_for` calls `:1248`, `:1280`,
  `:1310`, `:1338`, `:1384` become `step_pass(…).excerpts`; `PassInput` literals gain
  `changed_paths`), `crates/htui-orch/src/engine.rs` (`with_excerpts`, new `changed_paths`, judge
  comment, unit tests), `crates/htui/src/preview.rs`, `crates/htui/tests/prompt_preview.rs`
  (`SKILLS_NOTE` `:343-345` and the verbatim list `:329-330`). **No snapshot may move** (D124): the
  three that print the note clip it before the changed text.
- **Tests first**:
  - `htui-agent/tests/excerpt.rs`: `step_pass_lists_for_a_glob_skill_when_the_body_places_no_excerpts`
    (a `{{skills}}`-only body, a project `glob` winner → `files` holds the tree, `excerpts.files`
    empty, the "listed files for glob skills only" note, `audit.roots` from the listing);
    `step_pass_skips_the_walk_without_excerpts_or_a_glob_winner` (today's note, unchanged);
    `step_pass_narrows_files_to_touched_prefixes`; `step_pass_feeds_changed_paths_to_tier_2`
    (a changed path ranks `reason = previous_diff`, OQ-29);
    `step_pass_withholds_an_unmaskable_path_from_files`.
  - `engine.rs` (over `excerpt_prologue`'s temp tree): `a_glob_skill_fires_on_a_file_in_the_step_tree`
    (project attachment `glob` `**/*.rs` written with `set_skill_binding`; rendered, choice
    `matched`, `path = "htui:src/lib.rs"`); `a_glob_skill_with_no_matching_file_records_no_match`
    (`**/*.py`); `a_retry_matches_the_previous_attempts_changed_paths` (`FakeIsolator` scripts
    `docs/notes.md` for the attempt-1 winner; `**/*.md` fires on attempt 2 with no `.md` in the
    tree); `changed_paths_unavailable_is_a_note_not_a_failure`;
    `a_fan_out_group_matches_under_repo_box_path` (the `a_step_without_trees_reads_repo_box_path_with_a_note`
    set-up with a glob attachment); `a_judge_records_no_path_for_a_placed_glob_winner` (a judge body
    placing `{{skills}}`); `a_template_without_excerpts_runs_no_pass` unchanged.
  - `crates/htui/tests/prompt_preview.rs`: `the_preview_fires_a_glob_skill_over_repo_box_path` (a
    temp checkout as this box's `repo_box_path` row, a project `glob` attachment); the note pins.
- **Validate**: `cargo test -p htui-agent -p htui-orch -p htui --all-features -- --test-threads=1`.

### T6 — Prompt sub-tab and attachments pane (D125, D126)
- **Files (complete)**: `crates/htui/src/ui/tabs/backlog/detail/prompt.rs`, `crates/htui/src/skills.rs`,
  `crates/htui/src/ui/tabs/skills/attach.rs`, `crates/htui/tests/skills.rs`. Expected snapshot
  movement: none (`skills__attachments.snap` and its siblings hold no `glob` row, verified by grep;
  if one moves, the commit says why).
- **Tests first**: `detail/prompt.rs` unit `a_matched_choice_names_its_path` and
  `a_no_match_choice_is_dim_and_says_so`; `skills.rs` unit
  `the_snapshot_lists_repos_with_no_path_on_this_box`; `tests/skills.rs`
  `a_glob_row_says_it_fires_from_milestone_5` becomes `a_glob_row_names_repos_with_no_path_here`,
  plus `a_glob_row_with_every_repo_rooted_adds_nothing` (a `repo_box_path` written with
  `upsert_repo_box_path` first) and `a_qualified_glob_warns_only_for_its_own_repo`.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`.

### T7 — finding 6: edit a skill with no version (D127, OQ-28)
- **Files (complete)**: `crates/htui/src/ui/tabs/skills/library.rs` (guard and its unit test in the
  module's `#[cfg(test)]` at `:1716`, over a hand-built `SkillsSnapshot` whose entry has no
  versions), `crates/htui/tests/skills_pg.rs` (a raw `INSERT INTO skill` with no `skill_version`
  row; `e`, type, `ctrl-s` → v1 exists and the view shows `v1`). `MemStore` has no seam for a
  version-less skill (`create_skill` always writes v1), so the end-to-end case is Postgres only.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`.

### T8 — close-out (MOD-9 done)
Per `.claude/skills/handoff-run/references/lifecycle.md` P2 and `.claude/rules/workflow-docs.md`,
in this order, after the `rust-reviewer` gate and its fixes:
1. `docs/decisions/mod/mod-9.md` (new): `# MOD-9 - Skill library and templates (done, <date>)`;
   all five milestones (commit ranges `e971418`..`caacc96`, `7be0794`..`fd5161e`,
   `df91c82`..`e5db119`, `09fd007`..`6af3f53`, and this row's), each plan and blueprint path,
   ANA-22's verdict and amendments, the review outcomes, the carried items (MOD-55, MOD-57, MOD-59;
   finding 6 fixed or opened per OQ-28), and the moved pins (D128).
2. `DECISIONS.md`: prepend
   `- **[MOD-9](docs/decisions/mod/mod-9.md)** - Skill library and templates (done, <date>)` at the
   top of the index.
3. `HANDOFF.md`: delete the MOD-9 checklist entry (`:335-420`). Keep live coordinates in the
   status block (`:36-44`): migrations now end at `0008_trim_record_v3`, **next `0009`** (cache
   `0005`), `trim_record` `v 3`, and the unchanged pins of D128.
4. Summary table (`:809`): MOD-N 35 → 34, "MOD-9 skills" removed from the list.
5. Status line: date, lead with "MOD-9 done", cap the recap at ~2-3 completions (drop the oldest).
6. Cross-links: MOD-55 (`:736-742`), MOD-57 (`:744`) and MOD-59 (`:302`) say "from MOD-9" — point
   them at `docs/decisions/mod/mod-9.md`; `:85` ("MOD-9, MOD-11 … can start now") drops MOD-9;
   `:179` and `:610` cite MOD-9 milestones as history — point them at the write-up.
7. PRD row 5 → `complete (<first>..<last>, <date>)` with `[plan](../plans/mod-9-glob-attachments-fire.plan.md)`.
8. `docs/ANA-22.md` §10: one amendment line dated at close-out — row 5 shipped F2 as §6 item 7
   states, with `StepFiles` per repo rather than §7.2's two-armed enum, the record's `path` key and
   `v 3` (D109, D110, D118), judges recording `no_path` (D123); **the verdict is unchanged**.
9. This plan's Status line → IMPLEMENTED with the commit range and the review outcome.
10. `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` green.

## Test plan

TDD per task: each task's red tests are committed before its green code. The pure rules (reasons,
first-match order, reach) are unit tests over hand-built `StepFiles`; the F2 builder and the
listing split are unit tests over the in-memory `RepoReader` double, with every pre-existing
`excerpt::select` test as the regression net; the assembler's digest and masking behaviour are
`htui-core` integration tests; git's name-only listing is tested against real `git` in a temp
repository; the pass is tested over real temp trees (`htui-agent`), then end to end in the engine
over `excerpt_prologue`'s tree and a scripted `FakeIsolator`, and in the preview over a real
`repo_box_path` root; the UI through unit tests and the testkit harness; the migration through the
Postgres suite. Run the whole workspace with `--test-threads=1` before review (memory: suite
green is scheduling-dependent).

## Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-44 | A root over `excerpt_max_scan_files` (20 000, `settings.rs:45`) lists a prefix only, so a glob can record `no_match` though a later file matches. | Low | Medium | `audit.roots[].scan_truncated` and the walk's note are already in the record; the cap is an `app_setting`. |
| R-45 | Another sandbox run also takes migration `0008`. | Medium | Low | Checked at `scripts/hr collect MOD-9`: if main gained a `0008`, renumber this file and its pins before merge. |
| R-46 | A glob winner under a body with no `{{excerpts}}` (e.g. `verdict`) now walks the tree. | Certain there | Low | Only when a `glob` winner is placed (D120 step 2); the walk is the excerpt walk, off the runtime. |
| R-47 | The scrub filter scans every listed path. | Low | Low | `MinimalScrubber`'s rules are prefix tests; bounded by the scan cap. |
| R-48 | The walk's skip rules (lockfiles, minified, binary, over `max_file_bytes`, `.gitignore`d) also hide files from globs: `**/Cargo.lock` or `**/*.min.js` never matches. | Medium | Low | Stated in the write-up and the ANA-22 amendment line; typed source globs are unaffected. |
| R-49 | Every step with a matching `glob` winner renders a skill it did not before; digests move. | Certain | Low | Intended; digests are per step and no test pins a value. |
| R-50 | A fan-out group matches the managed checkout, not the candidates' fresh trees. | Medium | Low | The same accepted gap as MOD-7's R-48 (D108's note is recorded). |
| R-51 | A bare `touched_paths` glob in a project with no primary repo narrows the listing to nothing (`touched_prefixes`' empty-slug rule), so globs record `no_match`. | Low | Medium | Same rule the excerpt tier 1 and the overlap check use; the choice says `no_match`, not silence. |
| R-52 | Tier 2 (OQ-29) changes which excerpts a retry sees. | Certain on retries | Low | ANA-5 §4.5's intended ranking; the audit names `previous_diff` as the reason. |
| R-53 | A hand-written row whose glob no longer compiles records `no_match` with no note. | Low | Low | Writers refuse such globs (milestone 3 D78); the attachments pane's form shows the compile error when the row is opened. |
| R-54 | The matched path is clipped in the 43-column detail pane. | Certain for long paths | Low | The record keeps it whole; the pane is a summary. |
| R-55 | `rust-reviewer` or `opus`-pinned agents die on a 402 (memory). | Medium | Low | Run the reviewer unpinned. |
| R-56 | (OQ-27 default) `0008` raises `schema_version()` to 8, so every box's SQLite mirror is deleted and refilled on its first start after upgrade. | Certain | Low | Self-healing by design (`cache/mod.rs:141-153`), as for `0006`/`0007`; the alternative in OQ-27 avoids it. |

## Validation

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features -- --test-threads=1` (`HTUI_TEST_DATABASE_URL` is set in
  this sandbox; Postgres on `localhost:5439`, trust auth)
- `cargo sqlx prepare --check` from `crates/htui-store`, against a scratch database migrated to
  `0008` (recipe: `docs/hr-sandbox.md`, "Changing SQL queries in a run"); T3 adds a migration, no
  query, so this must pass unchanged at 288 files
- `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`
- Pins afterwards (D128): `ls crates/htui-store/.sqlx | wc -l` = 288; `ls crates/htui/tests/snapshots | wc -l`
  = 107, and `git diff --stat <base> -- crates/htui/tests/snapshots` empty; migrations `0001`..`0008`
  (or `0001`..`0007` under OQ-27's alternative).

## Acceptance

- A project attachment `glob` with `languages: rust` renders its skill into a phase step whose tree
  holds a `.rs` file under the item's touched paths, and the record says `matched` with
  `htui:<path>`, the first such file in byte order.
- The same attachment on an item whose touched paths hold only Markdown records `no_match` and does
  not render; on a box with no path for the repo and no tree, `no_path`, and the step proceeds.
- On attempt 2, a file the attempt-1 winner changed fires a glob even if the walk would not list it.
- A fan-out group fires over this box's `repo_box_path` checkout; a judge records `no_path` or
  `not_placed`; a handoff has no candidates.
- The Prompt sub-tab shows `matched <repo:path>` / `no_match`; the attachments pane names the repos
  that have no path on this box instead of "fires from milestone 5".
- The walk runs once per step, and not at all when nothing places `{{excerpts}}` and no `glob`
  winner is placed.
- `trim_record.v` is 3 and the column comment says so; nothing else in the schema moved.
- (OQ-28) A skill with no version opens in the editor and saves as v1.
- MOD-9 is archived: write-up, index line, HANDOFF entry gone, PRD row 5 complete, validator green.

## Where the PRD, ANA and tree disagree

1. **PRD row 5 "roots for fan-out groups"** is already delivered by MOD-7 milestone 4's D108 (a
   group assembles before its trees exist and reads this box's `repo_box_path` roots,
   `engine.rs:4966-5003`). Row 5 adds only the file set on those roots (D122).
2. **ANA-22 §7.2**'s `StepFiles = Resolved(paths by repo) | NoPath` becomes a per-repo reach map
   (D110): a scope can be half-resolved. §7.2's `select(bound, &StepFiles)` keeps milestone 2's
   `placed` argument (D111).
3. **ANA-22 §6 item 8**'s `matched: <path>` and `no match` are recorded as `reason: "matched"` with
   a separate `path: "<repo>:<path>"`, and `no_match` (D109); milestone 2's plan D40 already fixed
   the snake_case spelling.
4. **ANA-22 §8**'s dependency note ("until MOD-7 milestone 4 … only in steps that run in a worktree")
   is superseded: MOD-7 milestone 4 landed and both rungs resolve.
5. **ANA-22 §5.4** F2's "else the whole `run.repo_scope` checkout" is bounded by the walk's scan cap
   and skip rules (R-44, R-48); §5.4 cites only `.gitignore` and the secret denylist.
6. **MOD-7 plan D122** (tier 2 stays empty) is superseded by D121 if OQ-29 stands.
7. **The ported import plan** (`mod-9-skill-import.plan.md:1-8`, `…blueprint.md:6`) names a
   `0008_skill_match` migration from PR #10's tree; main never had it. `0008` here is new.
8. **`HANDOFF.md` MOD-9** says "a `glob` attachment still records `no_path`"; true at `b5e481f`,
   false after T2/T5.
9. **The walk order is not byte order of whole paths** (D113), although several docs describe the
   listing as "in byte order" (`RepoReader::list` doc, `excerpt.rs:556-557`: "byte order at every
   level" — accurate, but per level). The recorded match sorts explicitly.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `select(candidates, placed)` maps `Glob` → `NoPath`; `ChoiceReason` has five variants, `Copy + Hash`, snake_case | true | `crates/htui-core/src/model/skill.rs:383-400`, `:440-470` |
| `SkillChoice` has seven keys and no path | true | `skill.rs:415-431`; test `choices_serialize_their_documented_keys` |
| `SkillGlobs::compile`/`first_match` exist, unused outside tests, "called there" at milestone 5 | true | `model/skill_glob.rs:178-215`; no production caller in a workspace grep |
| `first_match` returns the first path in the caller's iteration order | true | `skill_glob.rs:200-214` |
| The only production `select` caller is `scrubbed_inputs` | true | `prompt/mod.rs:861`; the other caller is `htui-store/tests/skill_attachments.rs:300` |
| Skill names are masked before `select` | true | `prompt/mod.rs:778-780` |
| `excerpt::select` builds the listing as a local and returns only `ExcerptSet` | true | `prompt/excerpt.rs:1050-1299` (listing `:1085-1168`), `ExcerptSet` `:1463-1472` |
| `ExcerptRequest.changed_paths` exists and is always empty in production | true | `excerpt.rs:403`; `htui-agent/src/excerpt.rs:1044-1045` ("D122") |
| The pass is skipped when the body places no `{{excerpts}}`, with a pinned note | true | `htui-agent/src/excerpt.rs:1013-1021`; `engine.rs:12624` |
| The walk sorts per level, charges every entry to the cap, skips symlinks, `.git`, secret denylist, gitignore subset, binary, oversize, lockfile/minified | true | `htui-agent/src/excerpt.rs:234-380`; `prompt/excerpt.rs:135-155` |
| Default scan cap is 20 000 | true | `prompt/settings.rs:45` |
| Default phase bodies place both `{{skills}}` and `{{excerpts}}` except `verdict` (skills only) | true | `prompt/defaults.rs:35-36 … :132, :146-147, :165-166` |
| Engine: roots from `run_step_tree`, else `repo_box_path`, else `no_path`; group note D108 | true | `engine.rs:4974-5018`; `htui-agent/src/excerpt.rs:902-928` |
| A live step prepares trees before stage 3; a fan-out group assembles before candidate trees | true | `engine.rs:3119-3144`; `:3442-3481` |
| The judge runs in no tree and gets `no_excerpts`; the default judge places no `{{skills}}` | true | `engine.rs:4581`, `:4439-4441`; `:13291` |
| The handoff spec empties `skills` and inherits the rest with `..phase` | true | `crates/htui-orch/src/promote.rs:81-106` |
| No attempt's changed paths are recorded as a list; `DiffBlock` is text | true | `prompt/mod.rs:159-166`; `isolate.rs:176-190`; `isolate/git.rs:815-845` |
| `forwarded` reads the previous winner's trees and commits for `previous_diff` | true | `engine.rs:5198-5247`; `winner_at` `status.rs:243` |
| Three `Isolator` implementors | true | `isolate/real.rs:1387`, `fake.rs:409`, `tests/gix_isolator.rs:1987` |
| `Cli::diff` captures head-capped at 64 KiB | true | `git.rs:69`, `:426-431`, `:815-845` |
| The isolator's repo map is built from this box's `repo_box_path` rows | true | `crates/htui/src/run_worker.rs:1130-1150` |
| `trim_record` is `v 2` via `RECORD_VERSION`; the top-level key set and `v` are pinned | true | `prompt/trim.rs:56-58`, `:1092`; `tests/prompt_digest.rs:982-1011` |
| The `trim_record` column comment lists five reasons and is pinned verbatim | true | `migrations/0007_skill_attachments.sql:43-50`; `tests/migrations.rs:214-223` |
| Migrations are `0001`..`0007`; `Pending(7)` pinned in two test files | true | `ls crates/htui-store/migrations`; `migrations.rs:78-87`, `:884`, `:983`, `:988`, `:1007`; `connect.rs:141`, `:243` |
| The demo's stored trim record says `"v": 2` and no test ties it to `RECORD_VERSION` | true | `fixtures.rs:1640-1641`; grep for `RECORD_VERSION` finds `trim.rs` only |
| No snapshot holds a trim record's JSON | true | grep of `*.snap` for `skill_choices` finds none |
| `a_glob_skill_records_no_path_before_milestone_3` pins `no_path` | true | `crates/htui-core/tests/prompt_skills.rs:100-114` |
| The preview's skills note claims `no_path` and is pinned verbatim; three snapshots print it | true | `preview.rs:84-88`; `tests/prompt_preview.rs:329-330`, `:343-345`; grep of `crates/htui/tests/snapshots` |
| The preview runs the engine's pass over `repo_box_path` roots | true | `preview.rs:257-275`, `:309` |
| The attachments pane labels a glob row "fires from milestone 5", pinned by one test and no snapshot | true | `attach.rs:56-57`, `:753-755`; `tests/skills.rs:642-667`; grep of snapshots |
| `ProjectSkills` has one literal site; `skills::serve` has `backend` for `box_info` | true | `crates/htui/src/skills.rs:57-68`, `:165`, `:220-228` |
| `on_skill_key` returns early with no head | true | `crates/htui/src/ui/tabs/skills/library.rs:705-717` |
| `add_skill_version` treats `expected` 0 as "no version" | true | `crates/htui-core/src/store/traits.rs:852` (D89's order) |
| `MemStore` has no seam for a version-less skill | true | `store/mem.rs` public skill fns: only `bound_skills` (`:464`) |
| `PromptSpec` has ~10 literal sites | true | `fixtures.rs:161, 259, 361, 499`; `engine.rs:4418, 5111`; `promote.rs:89 (..phase), 416`; `preview.rs:276`; `prompt/mod.rs:533, 1207 (..spec)` |
| `SkillChoice` literal sites outside `skill.rs` | true | `tests/prompt_skills.rs:85`; `detail/prompt.rs:480`; `tests/prompt_preview.rs:357` |
| Pins: `CASES` 96, `READ_CASES` 14, orch `CASES` 73, `StoreRequest` 85, `StoreReply` 47, `.sqlx` 288, snapshots 107 | true | `HANDOFF.md:41-44`; `ls crates/htui-store/.sqlx \| wc -l` = 288; `ls crates/htui/tests/snapshots \| wc -l` = 107 |
| Highest MOD-9 numbers so far: D107, R-43, OQ-26 | true | grep of `.claude/plans/mod-9-*` (editable blueprint §12 D87–D107; import plan R-43, OQ-26) |
| Review gate is `rust-reviewer` | true | `.claude/workflow-config.json` |
| MOD-55, MOD-57, MOD-59 cite MOD-9 as origin | true | `HANDOFF.md:302`, `:736`, `:744` |

## Verified claims — fact-check (2026-09-29, independent verifiers at `7fa06e7`)

Two read-only verifiers, split by crate; probes ran in `/tmp` (a scratch crate on rustc 1.98.1 with
`globset` 0.4.20, `serde` 1.0.229, `serde_json` 1.0.151 `preserve_order`; a scratch git repo).

| Claim | Verdict | Evidence / amendment |
|---|---|---|
| OQ-27: `0008` has milestone 2's shape (a comment-only migration) | **falsified → amended** | `0007` has two `ALTER TABLE`s (`:16-30`), five column comments (`:32-38`), then §2 (`:39-50`); no migration is comment-only. OQ-27, D118 reworded |
| OQ-27/D108: the migration costs nothing beyond itself | **falsified → amended** | `pg/mod.rs:625-628` `schema_version()` = highest embedded; `cache/mod.rs:116-121`, `:141-153`, `:265-280` wipe the mirror on a version change. OQ-27 cost, R-56 |
| D119: `Cli::name_only` can reuse `run_capturing(…, Capture::Head)` and detect non-UTF-8 and truncation | **falsified → amended** | `git.rs:937-948` lossy `String`; `:1116-1122` appends a marker. D119 now adds `Capture::HeadBytes` |
| D124/D128: three snapshots change content | **falsified → amended** | Each clips the note before "uses this template; " (`feat_1:32` widest). None moves; T5 lists no snapshot |
| T5 file list complete | **falsified → amended** | `htui-agent/src/lib.rs:148-151` re-exports `excerpts_for`/`excerpt_pass`; added |
| T2: `promote.rs:416` literal needs `step_files` | **falsified → amended** | `promote.rs:105` `..phase`, `:429` `..phase.clone()`; removed from T2 |
| D116 follows the `drop_unmaskable_excerpts` convention | **falsified → amended** | That one names `repo:path` when unchanged (`prompt/mod.rs:902-911`); D116 is stricter |
| Citations `skill.rs:380-382`, `excerpt.rs:1085-1168`, `migrations.rs:78-87` | **falsified → amended** | `:378-380`, `:1081-1168`, `:86-93` |
| T7 ∥ T4 safe | **qualified → amended** | only in a separate worktree (`crates/htui/Cargo.toml` depends on `htui-orch`) |
| `SkillChoice {…}` / `PromptSpec {…}` construction sites; `select` callers; exhaustive `ChoiceReason` matches | verified complete | `skill.rs:456`, `:884`; `prompt_skills.rs:85`; `detail/prompt.rs:480`; `prompt_preview.rs:357`; `PromptSpec` 8 literals; only `as_str` matches exhaustively (`skill.rs:397-406`) |
| D109: `Copy + Hash` survive; `skip_serializing_if` keeps old JSON byte-identical; a v2 choice reads back with `path: None` | verified (probe) | `identical=true {"a":1,"reason":"no_path"}` |
| `SkillGlobs::first_match(repo, impl IntoIterator<Item=&str>)`; `**/*.rs` matches root `lib.rs` | verified | `skill_glob.rs:177-215`, `:265`; probe |
| D113 walk order vs byte order | verified (probe) | `htui-agent/src/excerpt.rs:255`, `:323`; `["a.rs", "a/x.rs"]` |
| D114 split keeps note order | verified feasible | note order `excerpt.rs:1073-1079`, `:1081-1168`, `:1172` |
| D128 pins | verified | `CASES` 96, `READ_CASES` 14, orch 73, `GraphSource` 7, `StoreRequest` 85, `StoreReply` 47, `REQUEST_NAMES` 6, `.sqlx` 288, snapshots 107, `TABLES` 39, 34 commented columns, `MIRRORED_TABLES` 21, cache next `0005` |
| Migration-count pins complete in T3 | verified | `migrations.rs:88`, `:884-885`, `:983`, `:988`, `:1007`; `connect.rs:141-143`, `:156-159`, `:243-245` |
| Three `Isolator` implementors only | verified | text search of every crate |
| `--no-renames -z` lists a rename as both names; newline names unquoted; raw non-UTF-8 bytes | verified (probe) | scratch repo |
| D122 engine facts (`with_excerpts`, `forwarded`, `winner_at`, `drive_group` first pending step, `run_worker.rs:1130-1150`) | verified | as cited |
| D123 judge/handoff; D125 `choice_line`; D126 `FIRES_LATER`, `ProjectSkills` one literal, `box_info`; D127 `on_skill_key` | verified | as cited; D126 now reads `backend.repo_paths(box_id)` once |
| OQ-28: `expected = 0` means "no version" | verified | `traits.rs:851-855`; `pg/write.rs:2876-2883` |
| Task-file intersections | verified | T1∩T2 `prompt/mod.rs`, `prompt_skills.rs`; T1∩T5 `prompt_preview.rs`; T1∩T6 `detail/prompt.rs`; T2∩T5 `engine.rs`, `preview.rs`; all other pairs ∅ by file, but T5/T6/T7 share the `htui` crate — serial stands |
