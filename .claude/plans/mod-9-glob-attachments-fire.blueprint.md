# Blueprint: MOD-9 milestone 5, "glob attachments fire"

**Status**: PROPOSED 2026-09-29 by the code-architect, from the plan confirmed the same day.
Departures B-1..B-3 and fill-ins F-1..F-7 (§0) are proposed here and numbered **D130–D137** (§13).
**Blocker** means the plan read literally does not compile or cannot be built as written; **Major**
means a named test, pin or file list is wrong, or a consequence the plan did not spell out;
**Minor** is a citation, wording or placement. No finding needs the maintainer: each has a smallest
fix that keeps D108–D129 and the OQ answers intact.

**Plan**: `.claude/plans/mod-9-glob-attachments-fire.plan.md` (CONFIRMED; OQ-27 `0008` comment
migration, OQ-28 finding 6 as T7, OQ-29 changed paths feed excerpt tier 2). Its D108–D129, R-44..R-56
and T1–T8 are binding; where §0 amends a mechanism, a file list or a test, this file wins.
**PRD**: `.claude/prds/mod-9-skill-library-templates.prd.md` row 5. **ANA**: `docs/ANA-22.md` §5.4,
§6 items 6–8, §7.2, §9, §10; `docs/ANA-5.md` §4.5.

**Verified at**: HEAD `0e73705` (branch `hr/MOD-9`, TOOL-7 sandbox, `HR_SANDBOX=1`).
`git diff --stat b5e481f HEAD -- crates Cargo.toml Cargo.lock` is empty, so the plan's citations
(read at `b5e481f`) and every line number below (**pre-edit**) hold. `.sqlx` = **288** files,
`crates/htui/tests/snapshots` = **107**, migrations `0001`..`0007`.

**Tools**: Gortex MCP first (`explore task`, `search symbols/text`, `read symbols`, `relations
callers/implementations`); `sed`/`grep -n` only for line-exact anchors, which Gortex's windowed
reads do not number. No probe was needed beyond the plan's fact-check; every claim below is a read.

**Scope at a glance**
- **Order**: T1 → T2 → T3 → T4 → T5 → T6 → T7 → T8, serial, one tree (the plan's default; ~10 G
  `target/` per extra worktree). T3 ∥ T4 and T7 ∥ T3 are safe only in separate worktrees (plan).
- **One migration** (`0008_trim_record_v3.sql`, comment only). **No** `WriteStore` method,
  `StoreRequest`/`StoreReply` variant, `.sqlx` file, dependency or mirror schema change.
- **One trait method** (`Isolator::changed_paths`, three implementors). **One `PromptSpec` field**
  (`step_files`, 7 literal sites). **One `SkillChoice` field** (`path`, 5 literal sites).
- **Snapshots**: none moves (D124, re-verified: the three snapshots that print the skills note clip
  it at "…skills come" / "…from the first phase", well before the changed text).

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs`; `missing_debug_implementations`
and `unused_qualifications` warn; clippy `-D warnings`; rustdoc denies broken/private intra-doc
links; `max_width = 100`; edition 2024. Every new `pub` item has a doc comment and `Debug`. Red
tests first (new bodies `todo!()`, old behaviour kept so only the new tests fail), then green; **every
commit compiles**; no test skipped or loosened; a moved pin names its reason in the assertion message;
explicit-path `git add`, never `-A`, never stash or amend; commit after each step (memory: uncommitted
subagent work dies with the session). **Every new code comment cites "MOD-9 D1xx"** (§11).

---

## 0. Blueprint departures, and fill-ins the plan left open

### 0.1 Departures (the plan as written cannot be built)

| # | Severity | Plan says | Tree at `0e73705` | Smallest fix |
|---|---|---|---|---|
| **B-1** | **Blocker** (T4) | D119: "`Capture::HeadBytes` … keeps the first 64 KiB as raw bytes and reports `overflowed: bool`, and **`run_capturing` returns them without decoding**." | `run_capturing` (`isolate/git.rs:360-421`) returns `Exited` (`:938-948`), whose `stdout` is a `String`; `into_string(out)` (`:410`, `:1173-1177`) decodes every sink. `Exited` is `pub`, `Clone + PartialEq + Eq`, and every verb reads `.stdout`/`.failure()` from it. `run_capturing` cannot return bytes without changing `Exited` for all six verbs. | **D130**: split the body. A new private `Cli::run_captured(…) -> Result<Captured, IsolateError>` is today's `run_capturing` body returning the **undecoded** `Sink`; `run_capturing` becomes a three-line wrapper that decodes into `Exited` exactly as today. `Capture::HeadBytes` builds `Sink::HeadBytes(HeadBuffer::new(DIFF_CAP))`; `HeadBuffer::into_bytes(self) -> (Vec<u8>, bool)` hands back `(kept, overflowed)`. `Cli::name_only` calls `run_captured` directly. `Exited` is untouched, so no other verb moves. §5.2. |
| **B-2** | **Blocker** (T6) | D126: "`snapshot` (`skills.rs:146`) gains `this_box: Option<BoxId>`, read in `serve` with `backend.box_info()` … and reads this box's rows once with `backend.repo_paths(box_id)`". | `snapshot(writer: &Writer, scope: &Scope)` (`crates/htui/src/skills.rs:146`) holds a `Writer`, not the `Backend`; `repo_paths(box_id)` exists only on `Backend` (`htui-store/src/backend.rs:526`), and `Writer`/`WriteStore` only has the per-repo `repo_box_paths(repo)` (`traits.rs:683`) the plan rejects. With `this_box` alone, `snapshot` could only do the N reads D126 forbids. Of its three callers, two (`answer` `:349-350`, serving the four write arms at `:250`, `:267`, `:291`, `:312`) hold only the writer too. | **D131**: `snapshot(writer, scope, box_paths: &[RepoBoxPath])`. A new private `async fn this_box_paths(backend: &Backend) -> Result<Vec<RepoBoxPath>>` = `box_info()` → `None` ⇒ `vec![]` (every repo unrooted), `Some(info)` ⇒ `repo_paths(info.box_id)`. The `Skills` read arm (`:222-229`) and the import arm (`:314-324`) call it after their writer check; `answer` gains a leading `backend: &Backend` parameter and calls it itself (its four callers are already past `write_access`). So the offline arms still refuse with their pinned sentences first (the `hierarchy::serve` order, `hierarchy.rs:221-225`). Same semantics as D126. §7.2. |
| **B-3** | Minor (T5 test) | T5: `step_pass_feeds_changed_paths_to_tier_2` "(a changed path ranks `reason = previous_diff`)". | The variant is `ExcerptReason::PrevDiff`, rendered and serialised `prev_diff` (`prompt/excerpt.rs:191-224`); tier 2 fires only for a path that is also **listed** and not under a touched prefix (`tier_of`, `:742-776`). | The test asserts `ExcerptReason::PrevDiff` on a listed file the touched prefix does not cover (§6.4). |

### 0.2 Fill-ins (the plan is silent; decided here)

| # | Where | Decision | Why |
|---|---|---|---|
| **F-1** | D120 step 6 | `withhold_unmaskable_notes` runs over `listing.notes` in the "listed for glob skills only" branch too, before they are appended. **D132**. | `list`'s notes name `repo:path` raw (declared-denied, not repo-relative). `TrimRecord::to_value` is fail-closed (`trim.rs:272-276`): a note holding an unmaskable string would abort the run. Step 7 already gets "today's scrub filters"; step 6 must not be the hole. |
| **F-2** | D116 | `drop_unmaskable_files` runs on **every** return branch of `step_pass`, and **before** the residual; its note is appended **last** to `StepPass.excerpts.notes` (after `drop_unmaskable_excerpts`' notes). `StepFiles::retain` keeps a repo **reached** even when every path under it is withheld. **D132**. | The residual (step 7) assembles the spec **with** `step_files`; an unmaskable matched path would make `scrubbed_inputs`' `mask(…)?` refuse and the residual fall to `unscanned`. Reach carries no string into the record, so keeping it is safe and keeps `no_match` vs `no_path` truthful. `excerpts.notes` is what `notes(spec)` copies into `trim_record.notes` (`prompt/mod.rs:1117-1121`). |
| **F-3** | D120 fail-open | A `JoinError` in the **walk** hop: excerpts = `unscanned(roots, caps, notes + today's sentence)`, `files = step_files(&Listing::default(), touched, changed)`. A `JoinError` in the **select** hop: the same excerpt set, but `files` keeps the listing's set (the walk succeeded). `excerpt_residual` `Err`: excerpts = `unscanned(roots, caps, notes)` exactly as today (listing notes dropped), files as computed. **D132**. | The plan's "the file set then holds only changed paths" is right for the walk hop only; after a good walk there is no reason to throw the listing away, and the residual `Err` path must stay byte-identical to today (`assemble` refuses the same way). |
| **F-4** | T5 commits | `excerpts_for` survives T5's first green agent commit as a thin wrapper, `step_pass(…).await.excerpts`, so the engine and the preview keep compiling while they are switched one commit at a time; T5's last commit deletes it and its re-export. **D133**. | `excerpts_for` has consumers in three crates (`htui-agent` lib + tests, `engine.rs:28,5015`, `preview.rs:29,308`). Removing it in the agent commit breaks the workspace build between commits. |
| **F-5** | D127 | The guard restructure also stops blocking `i` (rename) and `a` (attach) on a version-less skill; `,` `.` `b` `d` stay inert with no version. **D134**. | `on_skill_key` (`library.rs:705-717`) returns before the `match`, so today `i` and `a` are dead on such a skill too — R-SKL-3 ("bindings editable") is the same finding. Neither reads the head. |
| **F-6** | T4 fake | `FakeIsolator` records every `changed_paths` call's rows in `changed_path_requests: Mutex<Vec<DiffRequest>>` (the `diff_requests` shape, `fake.rs:92-94`, `:121-129`). **D135**. | `a_retry_matches_the_previous_attempts_changed_paths` must prove the engine read **attempt 1's winner's** rows, which only a recorded request can show. |
| **F-7** | D114 test | `select_is_list_then_select_listed` pins the **literal** note order over one fixture that produces every kind of step-1/2 note, rather than only comparing `select` with `list`+`select_listed` (tautological once `select` delegates). **D136**. | The split's one real risk is note order (F-101 first, then per-root, then declared-denied, then per-rule counts); only a literal list catches a reorder. |

Also decided (no plan text moves): `Cli::name_only` passes `--no-color` besides D119's flags, as
`Cli::diff` does (**D130**); the name-only NUL parse is a pure `parse_name_only(bytes, overflowed)`
so the partial-entry rule is unit-tested without 64 KiB of real file names (**D130**); `StepFiles`'s
`is_empty` means "reaches no repo" (**D137**).

---

## 0.3 Environment and gates (this sandbox)

```bash
# Postgres is up in the sandbox (localhost:5439, trust auth); the test DSN is already exported:
#   HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres
pg_isready -h localhost -p 5439            # "accepting connections" at blueprint time
df -h /                                    # memory: target/ fills the disk first; check before a long gate
# per crate (memory: suite green is scheduling-dependent; gate with one thread):
cargo test -p <crate> --all-features -- --test-threads=1
# workspace, before review:
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
```

`SQLX_OFFLINE = "true"` is set in `.cargo/config.toml`, so build/clippy/doc need no server.
`sqlx-cli 0.9.0` is installed. **`.sqlx` check against a scratch DB migrated to `0008`** (memory:
the compose `htui` database is empty; the test DSN and the prepare DSN are not the same database;
recipe `docs/hr-sandbox.md` "Changing SQL queries in a run"):

```bash
psql -h localhost -p 5439 -U postgres -c 'DROP DATABASE IF EXISTS htui_sqlx_mod9m5' \
                                      -c 'CREATE DATABASE htui_sqlx_mod9m5'
cd crates/htui-store
DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx_mod9m5 cargo sqlx migrate run --source migrations
DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx_mod9m5 cargo sqlx prepare --check -- --all-targets --all-features
ls .sqlx | wc -l      # 288, unchanged (T3 adds a migration, no query)
```

`warning: potentially unused queries found in .sqlx` from `--check` is expected.

---

## 1. Build order and validation, at a glance

| Task | Crates touched | Commits (each compiles) | Gate |
|---|---|---|---|
| T1 model | htui-core; literal fixes in htui-store tests, htui | 2 (red, green) | core; `cargo test -p htui-store --all-features --test skill_attachments -- --test-threads=1`; workspace build |
| T2 assembler, split, F2, v 3 | htui-core; literal fixes in htui-orch, htui | 3 (red, split, F2+v3) | core; workspace build |
| T3 `0008` | htui-store | 2 (red pins, migration) | store (Postgres); `.sqlx` check (§0.3) |
| T4 `changed_paths` | htui-orch | 2 (red, green) | orch; workspace build |
| T5 pass, engine, preview | htui-agent, htui-orch, htui | 5 (refactor, red, agent, engine, preview) | agent; orch; htui; workspace build; snapshot dir diff empty |
| T6 UI | htui | 3 (red, sub-tab, pane) | htui; snapshot dir diff empty |
| T7 finding 6 | htui | 2 (red, green) | htui (Postgres) |
| review | — | fixes as needed | §10's workspace gate; `rust-reviewer` **unpinned** (memory: opus 402) |
| T8 close-out | docs | 2 | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |

---

## 2. T1 — model: reasons, `StepFiles`, `select` (D109–D113)

### 2.1 Signatures

**`crates/htui-core/src/model/skill.rs`**

Imports (`:9-12`) gain `use std::collections::{BTreeMap, BTreeSet};` and
`use crate::model::skill_glob::SkillGlobs;` (no cycle: `skill_glob` imports only `skill_language`).

`ChoiceReason` (`:383-395`), two variants **appended** (no rename; `Copy + Hash` kept):

```rust
    /// MOD-9 D109: `activation = glob` and a file of the step's set matched; the choice carries
    /// the file as `path`. Active, like `Always`.
    Matched,
    /// MOD-9 D109/D112: `activation = glob`, a repo the globs can reach was listed, and nothing in
    /// it matched (or the stored globs no longer compile, R-53).
    NoMatch,
```

`as_str` (`:397-406`) gains `Self::Matched => "matched"`, `Self::NoMatch => "no_match"`; its doc
lists seven spellings. The `Always` doc ("The only active reason") becomes "Active; `Matched` is the
other active reason". The `NoPath` doc drops "(every step, until PRD milestone 5, D86)" for "no repo
the globs can reach was listed for the step (MOD-9 D112)". The enum doc (`:378-380`) drops the
milestone-5 promise sentence.

`SkillChoice` (`:415-431`), the field goes **last** (serde_json's `preserve_order` keeps struct order,
so the existing keys stay byte-identical):

```rust
    /// MOD-9 D109: `<repo>:<path>` of the file that woke a `matched` choice, masked like `name`;
    /// `None` for every other reason, and then absent from the JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
```

`StepFiles`, new, placed after `SkillChoice`:

```rust
/// MOD-9 D110: one step's file set for `glob` matching (ANA-22 §5.4 F2): per repo slug, the
/// repo-relative paths, and which repos were reached (listed, even to zero files). Byte order
/// throughout (`str`'s `Ord`), so the first match never depends on walk order (D113).
/// `Default` reaches nothing, which is `no_path` for every `glob` winner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepFiles {
    repos: BTreeMap<String, BTreeSet<String>>,
}

impl StepFiles {
    /// Marks `repo` as listed, with no file added.
    pub fn reach(&mut self, repo: &str);
    /// Adds `repo:path`, reaching `repo`.
    pub fn insert(&mut self, repo: &str, path: &str);
    /// Whether `repo` was reached.
    #[must_use] pub fn is_reached(&self, repo: &str) -> bool;
    /// Every reached repo and its paths, in repo byte order, paths in byte order.
    pub fn repos(&self) -> impl Iterator<Item = (&str, &BTreeSet<String>)>;
    /// Keeps the `(repo, path)` pairs `keep` answers `true` for; a repo stays reached even when
    /// every path under it goes (MOD-9 D132).
    pub fn retain(&mut self, mut keep: impl FnMut(&str, &str) -> bool);
    /// Whether no repo was reached (MOD-9 D137).
    #[must_use] pub fn is_empty(&self) -> bool;
}
```

`select` (`:432-470`), new signature and the `Glob` arm:

```rust
#[must_use]
pub fn select(
    candidates: Vec<BoundSkill>,
    placed: bool,
    files: &StepFiles,
) -> (Vec<BoundSkill>, Vec<SkillChoice>)
```

Rules, first match wins: `!placed` → `NotPlaced`; `version.is_none()` → `MissingVersion`; `Off` →
`Off`; `Always` → `Always`; `Glob` → `glob_reason(&skill.globs, files)`, a private
`fn glob_reason(globs: &[String], files: &StepFiles) -> (ChoiceReason, Option<String>)`:

```text
let Ok(compiled) = SkillGlobs::compile(globs) else { return (NoMatch, None) };   // R-53
for (repo, paths) in files.repos() {
    if let Some(path) = compiled.first_match(repo, paths.iter().map(String::as_str)) {
        return (Matched, Some(format!("{repo}:{path}")));
    }
}
if files.repos().any(|(repo, _)| compiled.reaches(repo)) { (NoMatch, None) } else { (NoPath, None) }
```

`active = matches!(reason, Always | Matched)`; an active candidate is pushed to the active list.
Every other choice has `path: None`. The doc (`:432-438`) states the seven rules.

`needs_files`, new, after `select`:

```rust
/// MOD-9 D111: whether a walk could change any choice — the collapsed winners hold a `Glob`
/// with a version. `BoundSkill::collapse` first, so a project `glob` under a phase `off` is not one.
#[must_use]
pub fn needs_files(candidates: &[BoundSkill]) -> bool
```

Stale prose: module doc `:4-7`, `Activation::Glob` doc `:21-22`, `SkillBinding.globs` `:116`,
`BoundSkill.globs` `:303` ("Recorded nowhere yet" → "matched by `select` against `StepFiles`,
MOD-9 D111").

**`crates/htui-core/src/model/skill_glob.rs`** — `SkillGlobs` (`:177-213`) gains:

```rust
    /// MOD-9 D112: whether any glob can match in `repo`: an unqualified one, or one qualified
    /// with `repo`.
    #[must_use]
    pub fn reaches(&self, repo: &str) -> bool
```

(`self.globs.iter().any(|(qualifier, _)| qualifier.as_deref().is_none_or(|own| own == repo))`.) The
type doc "(D86). Tested now, called there." becomes "`select`'s matcher (MOD-9 D111)".

**`crates/htui-core/src/model/mod.rs:150-153`** — `pub use skill::{…, StepFiles, …, needs_files}`.

**Literal and call sites that must move in the same commit** (compile coupling):

| Site | Change |
|---|---|
| `prompt/mod.rs:861` | `select(…, placed, &StepFiles::default())` — T2 replaces the argument with `&spec.step_files`; import `StepFiles` at `:57` |
| `skill.rs:456` (inside `select`) | `path` set per the rule above |
| `skill.rs:884` (test) | `path: None` |
| `htui-core/tests/prompt_skills.rs:85` | `path: None` |
| `htui-store/tests/skill_attachments.rs:300` | `select(candidates, true, &StepFiles::default())`; import at `:19-20` |
| `htui/src/ui/tabs/backlog/detail/prompt.rs:480` (unit test) | `path: None` |
| `htui/tests/prompt_preview.rs:357` (`active` helper) | `path: None` |

### 2.2 Tests (unit, in the modules' `#[cfg(test)]`)

`skill.rs` (helpers: the existing `bound(…)` `:482-499`; new `fn glob(name, globs: &[&str]) -> BoundSkill`
= `BoundSkill { activation: Glob, globs, ..bound(SkillId::new(), name, 1, 0, SkillLevel::Project) }`;
new `fn files(entries: &[(&str, &str)]) -> StepFiles` inserting each pair):

| Test | Asserts |
|---|---|
| `select_matches_a_glob_against_the_step_files` | `glob("g", ["**/*.rs"])` over `files([("htui","src/lib.rs")])` → one choice `{reason: Matched, active: true, path: Some("htui:src/lib.rs")}`, and the active list is `[that candidate]`. |
| `the_first_match_is_the_first_in_repo_then_path_byte_order` | Inserted in the order `htui:a/x.rs`, `htui:a.rs`, `api:z.rs` → `path == "api:z.rs"`; the same without `api` → `"htui:a.rs"` (`.` 0x2E < `/` 0x2F; D113). |
| `a_glob_over_a_reached_repo_with_no_match_records_no_match` | `["**/*.rs"]` over `files([("htui","README.md")])` → `NoMatch`, `path: None`, inactive; over a `StepFiles` with only `reach("htui")` → `NoMatch`. |
| `a_glob_reaching_no_listed_repo_records_no_path` | `StepFiles::default()` → `NoPath`; `["web:**/*.ts"]` over `files([("htui","a.ts")])` → `NoPath` (D112). |
| `a_glob_that_does_not_compile_records_no_match` | `globs: ["[a"]` over `StepFiles::default()` → `NoMatch` (the compile error is checked before reach; R-53). |
| `select_applies_its_rules_in_order` (extend `:836-879`) | Existing calls take `&StepFiles::default()` and keep `NoPath` for `c`. New: with `files([("htui","src/lib.rs")])`, a `Glob` `**/*.rs` with `version: None` → `MissingVersion`; `placed = false` → `NotPlaced`; an `Off` carrying globs → `Off`. |
| `choices_serialize_their_documented_keys` (extend `:882-941`) | The seven-key set is unchanged for `path: None`; a `Matched` choice with a path serialises `"path": "htui:src/lib.rs"` (eight keys); the `as_str` loop covers all seven reasons; `serde_json::from_value` of a v 2 choice (seven keys, no `path`) gives `path: None`. |
| `needs_files_sees_only_a_glob_winner_with_a_version` | `[Glob Project, Off Phase]` same `skill_id` → `false`; `[Glob version None]` → `false`; `[Always]` → `false`; `[Glob Some(1)]` → `true`. |

`skill_glob.rs`: `reaches_follows_the_qualifier` — `["**/*.rs"]` reaches `"anything"`;
`["web:**/*.ts"]` reaches `"web"`, not `"htui"`; `["web:a", "b"]` reaches `"htui"`; `[]` reaches
nothing.

### 2.3 Commits

1. `test(mod-9): glob selection over a step's files, red` — all §2.1 types and signatures; `select`
   takes `files` and ignores it (`let _ = files;`, `Glob` still `NoPath`); `StepFiles` methods,
   `needs_files` and `reaches` are `todo!()`; every literal site; the tests.
2. `feat(mod-9): a glob attachment matches the step's files (D109-D113)` — bodies, docs.

### 2.4 Gate

`cargo test -p htui-core --all-features -- --test-threads=1`;
`cargo test -p htui-store --all-features --test skill_attachments -- --test-threads=1` (Postgres);
`cargo build --workspace --all-targets --all-features`.

### 2.5 Hazards

- **Breaks until the same commit fixes it**: htui-store tests (`select` arity) and the `htui` crate
  (two `SkillChoice` literals). Build the workspace, not only `htui-core`.
- `path` **last** in the struct, or the record's key order moves for every existing choice.
- The first match iterates `files.repos()` (byte order), **never** the listing: `first_match` returns
  the first in the caller's iteration order (`skill_glob.rs:197-212`).
- `needs_files` must collapse first; the raw candidate list holds broader rows a narrower `off` hides.
- `ChoiceReason` is `Copy + Hash + Eq`: no data on the variants (the path is a field, D109).

---

## 3. T2 — assembler, listing split, F2 builder, record v 3 (D114–D118)

### 3.1 Signatures

**`crates/htui-core/src/prompt/excerpt.rs`** — new, placed just before `select` (`:1029`):

```rust
/// MOD-9 D114: §4.5 steps 1-2 on their own — every root resolved and listed once, path-only skip
/// rules applied — so the excerpt selection and the `glob` file set share one walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// One per repo in `req.roots`, repo byte order (becomes `audit.roots`).
    pub roots: Vec<RootRecord>,
    /// Every surviving listed file, root order then the reader's order.
    pub files: Vec<RepoPath>,
    /// The repos whose `RepoReader::list` succeeded, repo byte order.
    pub listed: Vec<String>,
    /// Steps 1-2's notes, in `select`'s old order (the F-101 note is not here).
    pub notes: Vec<String>,
}

#[must_use]
pub fn list(reader: &dyn RepoReader, req: &ExcerptRequest<'_>) -> Listing;

#[must_use]
pub fn select_listed(
    reader: &dyn RepoReader,
    req: &ExcerptRequest<'_>,
    listing: &Listing,
    merged: Vec<ExcerptCandidate>,
    providers: Vec<String>,
    est: crate::prompt::TokenEstimator,
) -> ExcerptSet;

/// MOD-9 D115: the F2 set.
#[must_use]
pub fn step_files(listing: &Listing, touched: &[PathPrefix], changed: &[RepoPath]) -> StepFiles;
```

`select` (`:1050-1056`) keeps its signature; its body becomes
`select_listed(reader, req, &list(reader, req), merged, providers, est)`.

- `list` = `:1081-1167` verbatim (roots sorted by repo bytes; `NoPath` record + note; list error
  record + note; `scan_truncated` + note; per path `skip_by_path` → per-rule count and, when
  declared by a touched prefix **or a changed path**, a declared-denied note (cap `DENIED_NOTE_CAP`);
  `!is_repo_relative` → note and drop), then `notes.extend(denied_notes)`, then the per-rule counts.
  A successful `reader.list` pushes `root.repo` to `listed`. Files are `RepoPath { repo, path }`.
- `select_listed` = `:1057-1079` (provider set, F-101 caps and its note), then
  `notes.extend(listing.notes.iter().cloned())`, then re-derives the sorted `roots: Vec<&RepoRoot>`
  from `req.roots` exactly as `:1082-1083` (used by `fill_lexical_heads` and the read loop, `:1175`,
  `:1216`), converts `listing.files` to `Vec<Listed>` with empty heads, `considered =
  listing.files.len()`, then `:1170-1298` unchanged with `audit.roots = listing.roots.clone()`.
- `step_files`: every repo of `listing.listed` is `reach`ed; each `listing.files` entry is inserted
  when `touched.is_empty() || touched.iter().any(|p| p.matches(&f.repo, &f.path))`; then each
  `changed` entry is inserted when `is_repo_relative(&c.path) && skip_by_path(&c.path).is_none()`
  (inserting reaches its repo even with no root; a dropped path reaches nothing).

`use crate::model::skill::StepFiles;` at the top of the file.

**`crates/htui-core/src/prompt/mod.rs`**

`PromptSpec` (`:71-123`) gains, after `excerpts` (`:103`):

```rust
    /// MOD-9 D110/D117: the F2 file set a `glob` attachment matches against — the excerpt walk's
    /// listing under the step's roots, narrowed to `touched_paths`, plus the previous attempt's
    /// changed paths, already scrub-filtered (D116). `StepFiles::default()` reaches no repo, so a
    /// `glob` winner records `no_path`: the judge's, the handoff's, every hand-built spec's.
    pub step_files: StepFiles,
```

`scrubbed_inputs` (`:858-861`): `select(BoundSkill::collapse(spec.skills.clone()), placed,
&spec.step_files)`, then, **after** `select` and before building `ScrubbedInputs`:

```rust
    // MOD-9 D117: the matched path is recorded, never rendered; masked as the names are.
    for choice in &mut skill_choices {
        if let Some(path) = &mut choice.path {
            mask(scrubber, path, &skills_name)?;
        }
    }
```

(`skills_name` is `:778`'s binding; `skill_choices` becomes `let (skills, mut skill_choices)`.)

New, beside `drop_unmaskable_excerpts` (`:916`):

```rust
/// MOD-9 D116: withholds from `glob` matching every file whose repo slug or path the scrubber
/// refuses (`refused_rule`), so a refused path can never be the recorded match (the record is
/// scrubbed fail-closed, `TrimRecord::to_value`). Count only — the note names no repo and no
/// path, stricter than `drop_unmaskable_excerpts`. `None` when nothing was withheld.
#[must_use]
pub fn drop_unmaskable_files(files: &mut StepFiles, scrubber: &dyn Scrubber) -> Option<String>
```

Note text, verbatim: ``skills: {n} path(s) withheld from glob matching; the scrubber refused them``.

Doc refresh: module doc `:16-26` (the `excerpts_for` sentence at `:25`; in T2 say "the shared pass
in `htui_agent::excerpt`", T5 names `step_pass`); `excerpt_residual` doc `:522-531` adds "the spec's
`step_files` included, so an active matched skill is paid for first (MOD-9 D120)".

**`crates/htui-core/src/prompt/trim.rs`** — `RECORD_VERSION: u8 = 3` (`:56-58`), doc:
"version 3 since MOD-9 D118 added `matched`, `no_match` and `skill_choices[].path`; 2 since MOD-9
D42 added `skill_choices`". The `to_value` doc list at `:241-243` gains `skill_choices[].path`.

**Literal sites** (`step_files: StepFiles::default()`), all in the red commit:
`prompt/fixtures.rs:161`, `:259`, `:361`, `:499` (import `crate::model::skill::StepFiles`);
`htui-orch/src/engine.rs:4418` (judge; T5 adds D123's comment) and `:5111` (phase; stays default —
`with_excerpts` sets it in T5; comment it like `excerpts` at `:5132-5136`); `htui/src/preview.rs:276`.
`..spec`/`..phase` sites need nothing (`prompt/mod.rs:533`, `:1207`; `promote.rs:89/105`, `:416/429`).

### 3.2 Tests

`crates/htui-core/tests/prompt_skills.rs` (import `htui_core::model::StepFiles`; `base()` is
`fixtures::phase_skills_over_cap()` with the cap at 20 000, `skills[1]` = `command-queue`):

| Test | Asserts |
|---|---|
| `a_glob_skill_with_no_step_files_records_no_path` (rename of `:100-114`) | Unchanged body; message "MOD-9 D117: default `StepFiles` reaches no repo". |
| `a_matched_glob_renders_like_always_and_records_the_path` | `glob` = base with `skills[1]` `Glob ["**/*.rs"]` and `step_files` = `{htui: src/lib.rs}`; `always` = base unchanged. Same `digest`, same `text`; `trim.sections` equal; `trim.skill_choices[1]` equals `always`'s except `activation: Glob`, `reason: Matched`, `path: Some("htui:src/lib.rs")`. |
| `a_glob_with_no_match_renders_nothing` | `step_files = {htui: docs/a.md}` → `command-queue` absent from `text`, `reason: NoMatch`, digest equals the `off` variant's. |
| `a_matched_path_is_masked_in_the_record` | Scrubber `MinimalScrubber::new(["s3cr3t".to_owned()])`; `step_files = {htui: src/s3cr3t.rs}` → `path == Some("htui:src/[REDACTED].rs")`; a second case with repo `s3cr3t-repo`, path `src/lib.rs` → `"[REDACTED]-repo:src/lib.rs"`; `trim.to_value(&scrubber)` string contains no `s3cr3t`. |
| `the_skills_cap_counts_a_matched_glob` | Measure one skill's `tokens_before` as `an_off_skill_never_trips_the_cap` does (`:158-190`); cap = that; `skills[1]` `Glob` + matching `step_files` → `Err(SkillsExceedCap { cap == single, tokens > single })`; the same with `StepFiles::default()` assembles and `tokens_before == single`. |
| `the_digest_moves_only_when_the_active_set_moves` (`:220-262`) | Assertions kept; the "before milestone 3" message becomes "`off` and an unmatched `glob` are both inactive". |

`crates/htui-core/tests/prompt_digest.rs:1007-1011` → `json!(3)`, message "MOD-9 D118 bumped the
record to v 3"; the thirteen-key assertion above it is unchanged (`path` lives inside
`skill_choices[]`, not at the top level).

`prompt/excerpt.rs` unit tests. `MapReader` (`:1671-1763`) gains a field
`unlistable: BTreeSet<String>` (init empty in `with`) and a builder `fn unlistable(mut self, repo:
&str) -> Self`; `list` returns `Err(ProviderError::new("map", "unlistable"))` for such a root. New
`CountingReader { inner: MapReader, lists: AtomicUsize }` implementing `RepoReader` by delegation,
counting `list` calls (`RepoReader` is `Send + Sync`; `AtomicUsize`, not `Cell`).

| Test | Fixture → asserts |
|---|---|
| `select_is_list_then_select_listed` (D136) | `request("nothing\n", &["**"], &[])`; roots `agy` (`NoPath`) and `htui` (`root("htui")`); `MapReader::with([("htui","src/a.rs","fn a() {}\n"), ("htui",".env","X=1\n"), ("htui","/abs.rs","fn b() {}\n")]).refusing_over(1_000)` with `truncated = true`. `list(..).notes` is **exactly**: ``excerpt: no readable root for repo `agy`; nothing was scanned``, ``excerpt: repo `htui` hit the scan cap of 20000 files; the listing is partial``, ``excerpt: listed path `htui:/abs.rs` is not repo-relative; dropped``, ``excerpt: `htui:.env` is declared but excluded by rule `secret_denylist`; never selected``, ``excerpt: 1 path(s) skipped by rule `secret_denylist` `` (listing order is the `BTreeMap`'s: `.env` < `/abs.rs` < `src/a.rs`). `listed == ["htui"]`; `files == [htui:src/a.rs]`. `select(..)` equals `select_listed(.., &list(..), ..)`, and its notes are the F-101 note (`1000` below `524288`) followed by the five above, then whatever selection adds. |
| `select_listed_never_lists` | `CountingReader` over two listable roots: `list` → 2 calls; `select_listed` over that listing → still 2; `select` on a fresh counter → 2. |
| `step_files_narrows_to_touched_prefixes` | Files `htui:{src/a.rs, docs/b.md}`, `docs:{x.md}`; touched `[PathPrefix::parse("src/", "htui")]` → `htui: {src/a.rs}`; `docs` reached with no file (global narrowing, D115). |
| `step_files_keeps_the_whole_listing_without_touched_paths` | Same listing, touched `[]` → every file present. |
| `step_files_adds_changed_paths_after_the_narrowing` | Touched `src/` plus changed `htui:docs/b.md` and `web:c.rs` → `docs/b.md` present; `web` reached with `c.rs` though it has no root. |
| `step_files_drops_a_denied_or_non_relative_changed_path` | Changed `htui:.env`, `htui:/abs`, `htui:../x`, `htui:` (empty) over an empty listing → `StepFiles::default()` (nothing reached). |
| `a_repo_that_could_not_be_listed_is_not_reached` | `MapReader::with(..).unlistable("broken")`, roots `htui`, `broken` → `listed == ["htui"]`; `step_files(..).is_reached("broken") == false`; the "could not be listed" note is in `listing.notes`. |

`prompt/mod.rs` unit: `drop_unmaskable_files_withholds_and_counts_without_naming` — files
`htui:{src/a.rs, src/sk-live.rs, docs/ghp_token.md}`, `sk-repo:{x.rs}`, scrubber
`MinimalScrubber::new([])` → note `Some("skills: 3 path(s) withheld from glob matching; the scrubber
refused them")`; kept `htui:{src/a.rs}`; `sk-repo` still reached with no file; the note contains
neither `sk-` nor `ghp_`. A second call on the result → `None`, set unchanged. (`sk-`/`ghp_` refuse
at a token start, including after `/`: `scrub.rs:20-29`, `:253-260`.)

### 3.3 Commits

1. `test(mod-9): one listing per step and record v 3, red` — `PromptSpec.step_files` + every
   literal site (three crates); `Listing`; `list`, `select_listed`, `step_files`,
   `drop_unmaskable_files` as `todo!()`; `select` body untouched; `scrubbed_inputs` still passes
   `&StepFiles::default()`; `MapReader`/`CountingReader`; all §3.2 tests; `prompt_digest` flipped.
2. `feat(mod-9): list once, select over the listing (D114)` — `list`, `select_listed`, `select`
   delegating. Every pre-existing `excerpt::select` test green is the regression net.
3. `feat(mod-9): the F2 file set, its scrub filter and record v 3 (D115-D118)` — `step_files`,
   `drop_unmaskable_files`, `scrubbed_inputs` wiring and masking, `RECORD_VERSION`, docs.

### 3.4 Gate

`cargo test -p htui-core --all-features -- --test-threads=1`;
`cargo build --workspace --all-targets --all-features`.

### 3.5 Hazards

- **Note order** is the split's one risk: F-101 note → `listing.notes` (per root in repo byte order,
  then declared-denied, then per-rule counts in rule-name byte order) → selection notes. D136's
  literal list is the pin.
- `select_listed` must re-sort `req.roots` itself; do **not** derive its roots from `listing.listed`
  (the read loop looks roots up by repo, `:1216`, and must behave exactly as before).
- `list` uses `req.changed_paths` for declared-denied notes — with OQ-29 these now fire on retries.
- Mask `choice.path` **after** `select`; the matcher must see the unmasked set (a `[REDACTED]` path
  never matches a glob). The digest must not move with the path: it is recorded, not rendered.
- `IMPL_TRIM_RECORD` in `crates/htui-core/src/fixtures.rs:1640` (the demo module) **stays `"v": 2`**:
  it is a record written before `0008`. No test ties it to `RECORD_VERSION`.
- The red commit adds a field to a struct built in `htui-orch` and `htui`: build the workspace.

---

## 4. T3 — migration `0008` (D118, OQ-27)

### 4.1 The file — `crates/htui-store/migrations/0008_trim_record_v3.sql` (new)

```sql
-- 0008_trim_record_v3.sql - MOD-9 milestone 5 (docs/ANA-22.md 6 item 8; plan D109, D118, OQ-27).
-- Forward-only (R-STO-5).
--
-- run_step.trim_record's contract is restated for record v 3: skill_choices[].reason gains
-- matched and no_match, and a matched choice carries path. Comment only: no table, column,
-- constraint or index moves, and the SQLite mirror's schema is untouched. schema_version still
-- becomes 8, so each box rebuilds its mirror once on first start (plan R-56).

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42 and D118: {v, template, budget, budget_source, reserve, '
  'target, estimator, estimated_before, estimated_after, sections[], skill_choices[], excerpts, '
  'notes}, v 3. skill_choices[] is every candidate skill, ordered by position then name, each '
  '{skill, name, version, level, activation, active, reason} with reason always, matched, '
  'no_match, off, no_path, missing_version or not_placed (ANA-22 6 item 8); a matched choice '
  'adds path, <repo>:<path>, the first matching file in repo then path byte order. A v 2 '
  'record, written before 0008, has no matched or no_match; a v 1 record, written before 0007, '
  'has no skill_choices. Canonical; the prompt payload sections[] array is its abridged '
  'projection. Written at stage 3 by set_step_prompt, before the session starts.';
```

The concatenated text is D118's, byte for byte (adjacent literals separated by a newline are one
string, as `0007:43-50`). Each break falls after a space inside the literal, so exactly one space
joins them. **Verify it once** on the scratch DB:
`psql -h localhost -p 5439 -U postgres -d htui_sqlx_mod9m5 -Atc "SELECT col_description('run_step'::regclass, (SELECT attnum FROM pg_attribute WHERE attrelid = 'run_step'::regclass AND attname = 'trim_record'))"`
must print D118's text on one line.

### 4.2 Pins (red first)

`crates/htui-store/tests/migrations.rs`:
- `:86-93` → `vec![1, 2, 3, 4, 5, 6, 7, 8]`, message appends "and MOD-9 milestone 5's
  0008_trim_record_v3.sql".
- `:181-183` doc: "`run_step.trim_record` is the text `0008_trim_record_v3.sql` restates (MOD-9
  D118), which replaces `0007`'s, which replaced `0002`'s."
- `:214-224` → D118's text as a Rust literal with `\` continuations (a continuation strips the next
  line's leading whitespace, so each line must end with the single space it needs, as today).
- `:884-885` → `Pending(8)`, "eight embedded migrations, none applied"; `:983`, `:988`, `:1007` → 8.

`crates/htui-store/tests/connect.rs`: `:141-143`, `:157-159`, `:243-245` → `Pending(8)` / `8`,
messages "eight embedded migrations since MOD-9 milestone 5's 0008_trim_record_v3.sql".

Unchanged and to be re-checked green: `TABLES` 39, the commented-column count 34 (a restated
comment, not a new one), `MIRRORED_TABLES` 21, no cache migration (next cache `0005`).

### 4.3 Commits

1. `test(mod-9): the migration pins name 0008, red`.
2. `feat(mod-9): 0008 restates the trim_record comment for record v 3 (D118, OQ-27)`.

### 4.4 Gate

`cargo test -p htui-store --all-features -- --test-threads=1` (Postgres); the §0.3 `.sqlx` check
(288, unchanged); `ls crates/htui-store/migrations` ends at `0008_trim_record_v3.sql`.

### 4.5 Hazards

- A byte-level mismatch between the SQL and the Rust literal (a double space at a line join) fails
  `migrations.rs:429-450` with a message that does not show where; compare with the `psql` read.
- `sqlx::migrate!` embeds files at compile time: after adding the file, a stale build can still see
  seven. `cargo clean -p htui-store` if `Pending(7)` persists.
- R-45: another run taking `0008` is checked at `scripts/hr collect MOD-9`.

---

## 5. T4 — `Isolator::changed_paths` (D119)

### 5.1 The trait — `crates/htui-orch/src/isolate.rs`

New type, before the trait (`:124`):

```rust
/// MOD-9 D119: the paths a step's committed ranges touched, for the next attempt's `glob` file
/// set and excerpt tier 2. `(repo, path)` repo-relative and `/`-separated, sorted by `(repo id,
/// path bytes)`, deduplicated; a rename appears under both names (`--no-renames`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangedPaths {
    /// The paths.
    pub paths: Vec<(RepoId, String)>,
    /// Some repo's list was cut at [`git::DIFF_CAP`]; its partial last entry was dropped.
    pub truncated: bool,
}
```

Trait method, directly after `diff` (`:186-190`), **no default body**:

```rust
    /// MOD-9 D119: the paths [`diff`](Isolator::diff)'s ranges changed, over the same rows and the
    /// same range choice. No usable `git` is `Ok(ChangedPaths::default())`, as `diff` is `Ok(None)`.
    fn changed_paths<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, ChangedPaths>;
```

Trait doc `:124-125` "plus milestone 4's two reads" → "three reads (MOD-9 D119 adds
`changed_paths`)". `lib.rs:60-63` re-exports `ChangedPaths` beside `ResetReport`.

### 5.2 git — `crates/htui-orch/src/isolate/git.rs` (D130)

```rust
enum Capture { Tail, Head, /** MOD-9 D130: the first DIFF_CAP bytes, never decoded. */ HeadBytes }
enum Sink { Tail(TailBuffer), Head(HeadBuffer), HeadBytes(HeadBuffer) }   // Sink::push/into_string gain the arm
                                                                          // (into_string: lossy, no marker)
impl HeadBuffer {
    /// MOD-9 D130: the kept bytes and whether anything was dropped after them.
    #[must_use] pub fn into_bytes(self) -> (Vec<u8>, bool);
}

/// MOD-9 D130: one verb's exit and its undecoded stdout sink.
struct Captured { code: Option<i32>, stdout: Sink, stderr: String }

impl Cli {
    async fn run_captured(&self, verb: &'static str, cwd: &Path, args: &[&OsStr],
                          extra_env: &[(&str, &str)], stdout: Capture) -> Result<Captured, IsolateError>;
    // run_capturing (:360) = run_captured(..).await.map(|c| Exited {
    //     code: c.code, stdout: c.stdout.into_string(), stderr: c.stderr })

    /// MOD-9 D119/D130: `git diff --no-color --name-only -z --no-renames --no-ext-diff
    /// --no-textconv <before> <after> --` in `repo`, stdout head-capped at DIFF_CAP as bytes.
    pub async fn name_only(&self, repo: &Path, before: &str, after: &str)
        -> Result<NameOnly, IsolateError>;
}

/// MOD-9 D119: one range's changed names, in git's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameOnly { pub paths: Vec<String>, pub truncated: bool }

/// MOD-9 D130: NUL-split; the final segment is always dropped (empty when git finished its last
/// entry, partial when the cap cut it); a non-UTF-8 or empty entry is dropped; `truncated =
/// overflowed`.
fn parse_name_only(bytes: &[u8], overflowed: bool) -> NameOnly;
```

`name_only` on a non-zero exit: `Err(Exited { code, stdout: String::new(), stderr }.failure("diff"))`
(the verb name `diff`, as `Cli::diff`). Not retried (a read, M3 D39). The `Captured` stderr is the
tail capture, decoded as today.

### 5.3 Real — `crates/htui-orch/src/isolate/real.rs`

Factor the range choice out of `diff_of` (`:1287-1351`) into a private

```rust
/// MOD-9 D119: the repository and range `diff` reads for one committed row — the copy tree or
/// the checkout (D74), the reconcile merge from its first parent (D141, D146) — or `None` when
/// the row committed nothing.
async fn range_of(&self, trees: &[RunStepTree], commit: &RunStepCommit)
    -> Result<Option<Range>, IsolateError>;
struct Range { repo: PathBuf, name: String, before: String, after: String }
```

`diff_of` becomes `range_of` + the two `git.diff` calls (behaviour identical; its tests are the net).
`impl Isolator for GixIsolator` (`:1387`) gains, after `diff` (`:1448-1480`):
`let Ok(git) = self.cli() else { return Ok(ChangedPaths::default()) };` then per commit
`range_of` → `git.name_only(&range.repo, &range.before, &range.after)` → push
`(commit.repo_id, path)` for each; `truncated |= part.truncated`; finally `paths.sort(); paths.dedup()`.

### 5.4 Fake — `crates/htui-orch/src/fake.rs` (D135)

Fields beside `diffs` (`:89-94`):
`changed: Mutex<VecDeque<std::result::Result<ChangedPaths, IsolateError>>>` and
`changed_path_requests: Mutex<Vec<DiffRequest>>`. Methods beside `script_diff`/`fail_diff`
(`:215-241`): `pub fn script_changed_paths(&self, paths: ChangedPaths)`,
`pub fn fail_changed_paths(&self, reason: &str)` (`IsolateError::Git`),
`#[must_use] pub fn changed_path_requests(&self) -> Vec<DiffRequest>`. The impl records the rows,
pops FIFO, **unscripted answers `Ok(ChangedPaths::default())`** (never `todo!()` after green: every
retry in every engine test calls it from T5 on).

`crates/htui-orch/tests/gix_isolator.rs` `StallAfterReconcile` (`:1987-2058`): delegate
`self.inner.changed_paths(trees, commits)` beside `diff` (`:2011-2017`).

### 5.5 Tests

`git.rs` `mod tests` (real `git` via `crate::skip_without_git!()`; commits made by a small local
helper `fn git(repo: &Path, args: &[&str])` spawning `git.binary()` with `SCRUBBED_ENV` removed and
`-c user.name=t -c user.email=t@t`, since `testkit::commit_file` writes root-level entries only and
cannot rename or delete):

| Test | Asserts |
|---|---|
| `name_only_lists_every_changed_path_with_renames_split` (`#[cfg(unix)]`, newline names) | Base: `f`, `old.rs`, `gone.rs`. After: modify `f`, `git mv old.rs moved.rs`, `git rm gone.rs`, add `a b.rs` and `new\nline.rs`. Sorted result == `["a b.rs", "f", "gone.rs", "moved.rs", "new\nline.rs", "old.rs"]`, `truncated == false`. |
| `name_only_drops_a_non_utf8_name` (`#[cfg(target_os = "linux")]`) | A file named by the bytes `\xff\xfe.rs` (`OsStr::from_bytes`) plus `ok.rs` → `["ok.rs"]`. |
| `name_only_of_an_empty_range_is_empty` | `head..head` → `NameOnly::default()`. |
| `head_bytes_capture_reports_overflow` | `HeadBuffer::new(8)` pushed 10 bytes → `into_bytes() == (first 8, true)`; pushed exactly 8 → `(8 bytes, false)`. |
| `name_only_drops_a_partial_last_entry_when_capped` (pure) | `parse_name_only(b"a.rs\0b.rs\0c.r", true)` → `["a.rs","b.rs"]`, truncated; `parse_name_only(b"a.rs\0b.rs\0", true)` → both kept, truncated (cut exactly on a NUL); `parse_name_only(b"a.rs\0b.rs\0", false)` → both, not truncated; `b""` → empty. |

`gix_isolator.rs` (fixture helpers `Fixture::new` `:115-129`, `committed_worktree_step`
`:2708-2736`, `head_of`):

| Test | Asserts |
|---|---|
| `changed_paths_of_a_worktree_step_names_its_committed_paths` | Prepare a worktree step, `commit_file` twice (`a.txt`, `b.txt`), `capture` → commits; `changed_paths(&rows, &commits)` == `ChangedPaths { paths: [(core.id,"a.txt"), (core.id,"b.txt")], truncated: false }`. |
| `changed_paths_of_a_reconcile_merge_uses_the_first_parent` | Mirror `a_merge_onto_a_moved_primary_diffs_only_its_own_paths` (`:2795-2831`): `changed_paths(&b_rows, &second)` == `[(core.id, "b.txt")]`, never `a.txt`. |

`fake.rs` tests (`:1772`): `scripted_changed_paths_round_trip` — unscripted → default; scripted →
returned once, then default; `fail_changed_paths("x")` → `Err(Git("x"))`; `changed_path_requests`
holds each call's step ids.

### 5.6 Commits

1. `test(mod-9): an attempt's changed paths from git, red` — `ChangedPaths`, the trait method, the
   three implementor stubs (`Box::pin(async move { todo!() })`), `Capture::HeadBytes`, `NameOnly`,
   `name_only`/`parse_name_only`/`into_bytes` stubs, all tests.
2. `feat(mod-9): Isolator::changed_paths names what a range changed (D119)`.

### 5.7 Gate

`cargo test -p htui-orch --all-features -- --test-threads=1`;
`cargo build --workspace --all-targets --all-features`.

### 5.8 Hazards

- **The partial-entry rule**: always drop the final NUL-split segment (empty or partial); never
  pop a second one when the output ends on a NUL and overflowed. `truncated = overflowed` even then.
- Do **not** decode then split: `String::from_utf8_lossy` turns a bad name into a plausible U+FFFD
  path that could match a glob (the fact-check's falsified mechanism).
- `range_of` must keep D141/D146 exactly (the five `diff_of` tests at `:2795-3090` are the net);
  `changed_paths` and `diff` must never disagree on a range.
- `htui-orch` does not build between adding the trait method and stubbing all three implementors:
  one commit.

---

## 6. T5 — the shared pass, engine and preview (D120–D124)

### 6.1 `crates/htui-agent/src/excerpt.rs`

```rust
pub struct PassInput {            // (:945-955) gains, last:
    /// MOD-9 D121: the previous attempt's changed paths, repo-qualified by name; empty on
    /// attempt 1 and in the preview. Excerpt tier 2 and the `glob` file set both read it.
    pub changed_paths: Vec<RepoPath>,
}

/// MOD-9 D120: one step's pass — the excerpt set and the `glob` file set, from one walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepPass {
    /// For `spec.excerpts`.
    pub excerpts: ExcerptSet,
    /// For `spec.step_files`, already scrub-filtered (D116).
    pub files: StepFiles,
}

/// MOD-9 D114: §4.5 steps 1-2 over the filesystem. **Blocking.**
#[must_use]
pub fn walk_pass(req: &OwnedExcerptRequest) -> Listing;
    // list(&FsRepoReader::new(req.caps), &req.as_request())

/// §4.5 steps 3-10 over a listing already walked. **Blocking** (reads files).
#[must_use]
pub fn excerpt_pass(req: &OwnedExcerptRequest, listing: &Listing, est: TokenEstimator) -> ExcerptSet;
    // providers, then select_listed(&FsRepoReader::new(req.caps), &request, listing, merged, provider_set, est)

/// MOD-9 D120: replaces `excerpts_for`.
pub async fn step_pass(
    spec: &PromptSpec,
    input: PassInput,
    app: &BTreeMap<String, serde_json::Value>,
    scrubber: &dyn Scrubber,
) -> StepPass;
```

`step_pass` flow (D120 as filled by D132):

1. `(caps, scan_cap, deadline) = resolve_excerpt_caps(app)`; destructure `input`.
2. `parsed = parse(spec.role, &spec.body)`; `places_excerpts`, `places_skills` from `parsed.used`
   (a parse error is neither).
3. `wants_files = places_skills && needs_files(&spec.skills)`.
4. **Neither** → excerpts = `unscanned(&roots, caps, notes + "excerpt: template \`{name}\` places no
   {{excerpts}}; nothing was read")` (unchanged text); `files = step_files(&Listing::default(),
   &touched, &changed)`; go to 9.
5. Build `request: OwnedExcerptRequest` as today but `changed_paths: changed.clone()` (D121; the
   `:1044-1045` comment goes) and `budget_tokens: 0` for now. `readable` as today.
6. Walk: readable → `spawn_blocking(move || { let l = walk_pass(&request); (request, l) })` (hand the
   request back out of the closure); `JoinError` → F-3 walk-hop branch, go to 9. Not readable →
   `list(&FsRepoReader::new(caps), &request.as_request())` inline (no I/O).
7. `files = step_files(&listing, &touched, &changed)`; `withheld = drop_unmaskable_files(&mut files,
   scrubber)` (**before** the residual).
8. `!places_excerpts` → `withhold_unmaskable_notes(&mut listing.notes, scrubber)` (D132); excerpts =
   `{ files: [], audit: { provider_set: [BUILTIN_ID], roots: listing.roots, considered: 0, selected:
   0, caps, files: [] }, notes: notes + "excerpt: template \`{name}\` places no {{excerpts}}; the walk
   listed files for glob skills only" + listing.notes }`.
   Otherwise: `budget = if readable { excerpt_residual(&PromptSpec { step_files: files.clone(),
   ..spec.clone() }, scrubber) } else { Ok(0) }`; `Err` → excerpts = `unscanned(&roots, caps, notes)`
   (today's output). `Ok(b)` → `request.budget_tokens = b`; readable → `spawn_blocking(move ||
   excerpt_pass(&request, &listing, est))` (F-3 select-hop `JoinError` → today's note + unscanned),
   not readable → `excerpt_pass` inline; then `withhold_unmaskable_notes(&mut set.notes, scrubber)`,
   `drop_unmaskable_excerpts(&mut set, scrubber)`, `set.notes = notes ++ set.notes`.
9. On branches that skipped 7 (step 4, walk-hop `JoinError`), `withheld = drop_unmaskable_files(&mut
   files, scrubber)` here. `if let Some(note) = withheld { excerpts.notes.push(note) }` (last).
   Return `StepPass`.

Imports: `htui_core::prompt::excerpt::{Listing, RepoPath, list, select_listed, step_files, …}`
(**drop** `select` if unused — `unused_imports` fails clippy `-D warnings`),
`htui_core::prompt::drop_unmaskable_files`, `htui_core::model::{StepFiles, needs_files}`.
Module doc `:24-26` and `excerpt_pass` doc `:957-959` name `step_pass`.

**`crates/htui-agent/src/lib.rs:148-151`**: `pub use excerpt::{FsRepoReader, GitignoreSubset,
PassInput, SkipRule, StepPass, excerpt_pass, excerpt_roots, run_providers, step_pass,
touched_prefixes, walk_pass};` (`excerpts_for` removed in commit 5, D133).

### 6.2 Engine — `crates/htui-orch/src/engine.rs`

Import `:28` → `use htui_agent::excerpt::{PassInput, excerpt_roots, step_pass, touched_prefixes};`;
`RepoPath` joins the `htui_core::prompt::excerpt` import at `:38`; `StepFiles` joins the model import.

New private method beside `forwarded` (`:5190`):

```rust
    /// MOD-9 D122: the previous attempt's winner's changed paths, by scope name, for excerpt
    /// tier 2 and the `glob` file set. An isolator error is a note and an empty list (the
    /// `previous_diff unavailable` shape); no winner is empty with no note (`forwarded` notes it).
    async fn changed_paths(
        &self,
        run: &Run,
        step: &RunStep,
        scope: &[(RepoId, String)],
        notes: &mut Vec<String>,
    ) -> Result<Vec<RepoPath>, EngineError>
```

Body: `run_steps(run.id)` → `winner_at(&steps, step.position, step.attempt - 1)` → `None` ⇒
`Ok(vec![])`; `step_trees(prev.id)`, `step_commits(prev.id)`, `isolator.changed_paths(..)`:
`Err(err)` ⇒ note ``changed paths unavailable: {err}``, `vec![]`; `Ok(c)` ⇒ `c.truncated` adds
``changed paths: the list was cut at 64 KiB``; each `(id, path)` maps to `RepoPath { repo: name,
path }` through `scope`; an id outside the scope is dropped.

`with_excerpts` (`:4974-5018`): after `scope` and before `PassInput`,
`let changed = if matches!(spec.role, TemplateRole::Phase) && step.attempt > 1 {
self.changed_paths(run, step, &scope, &mut notes).await? } else { Vec::new() };` then
`PassInput { roots, touched_prefixes, notes, changed_paths: changed }`, and
`let pass = step_pass(spec, input, &self.parts.app, self.parts.scrubber).await;
spec.excerpts = pass.excerpts; spec.step_files = pass.files;`. Doc `:4961-4971` names `step_pass`
and the changed paths.

Judge spec (`:4418-4455`), beside `excerpts`:

```rust
            // MOD-9 D123: a judge runs in no tree (`prepare(…, &[], Isolation::Local, None)`) and
            // gets no pass, so a placed `glob` winner records `no_path` truthfully; the default
            // judge body places no `{{skills}}` and records `not_placed`.
            step_files: StepFiles::default(),
```

### 6.3 Preview — `crates/htui/src/preview.rs`

`SKILLS_NOTE` (`:85-88`), verbatim:
``preview: phase-level skills come from the first phase of the item's graph that uses this template;
glob attachments match this box's repo_box_path listing, narrowed to touched_paths, with no previous
attempt`` (doc: "MOD-9 D45, D124"). `PassInput` (`:270-274`) gains `changed_paths: Vec::new()`
(attempt 1); `:308` becomes `let pass = step_pass(&spec, input, &app, &scrubber).await;
spec.excerpts = pass.excerpts; spec.step_files = pass.files;`. Import `:29`; module doc `:10-12`.
`STAND_INS` stays 8.

### 6.4 Tests

`crates/htui-agent/tests/excerpt.rs` (imports `:13-27` swap `excerpts_for` for `step_pass`,
`walk_pass`; add `htui_core::model::{Activation, BoundSkill, SkillId, SkillLevel}`, `RepoPath`,
`ExcerptReason`). New helper `fn glob_skill(globs: &[&str]) -> BoundSkill` (`name: "globbed"`,
`version: Some(1)`, `position: 0`, `body: "Mind the globs.\n"`, `level: Project`, `activation:
Glob`). `readable_input` (`:1101-1107`) and the literal at `:1378` gain `changed_paths: Vec::new()`.
`:1114`, `:1144`: `let req = base_request(..); excerpt_pass(&req, &walk_pass(&req),
TokenEstimator::DEFAULT)`.

| Test | Fixture → asserts |
|---|---|
| `step_pass_lists_for_a_glob_skill_when_the_body_places_no_excerpts` | `verdict` body (`body_of("verdict")`, template name `verdict`), `skills = [glob_skill(["**/*.rs"])]`, tree `src/lib.rs`, `readable_input`. `files.is_reached("htui")` and holds `src/lib.rs`; `excerpts.files` empty; `audit.considered == 0`; `audit.roots == [RootRecord { htui, RunStepTree, false }]`; notes start with the caller's note and contain ``excerpt: template `verdict` places no {{excerpts}}; the walk listed files for glob skills only``. |
| `step_pass_skips_the_walk_without_excerpts_or_a_glob_winner` | `verdict` body, no skills → the unchanged "…nothing was read" note, `files.is_empty()`; the same with `glob_skill` but `version: None` → still no walk (`files.is_empty()`). |
| `step_pass_narrows_files_to_touched_prefixes` | `verdict` body + glob skill, tree `src/lib.rs`, `docs/a.md`; touched `src/lib.rs` → `files` has `src/lib.rs` only, `htui` reached. |
| `step_pass_feeds_changed_paths_to_tier_2` (B-3) | `phase_spec()` (places `{{excerpts}}`), tree `src/lib.rs` and `src/other.rs`, touched `src/lib.rs`, `changed_paths: [htui:src/other.rs]` → `excerpts.files` has `src/other.rs` with `reason == ExcerptReason::PrevDiff`; `files` holds both. |
| `step_pass_withholds_an_unmaskable_path_from_files` | `verdict` body + glob skill, tree `src/lib.rs`, `src/sk-live.rs`, touched `[]` → `files` holds `src/lib.rs`, not `src/sk-live.rs`; notes contain ``skills: 1 path(s) withheld from glob matching; the scrubber refused them``; no note contains `sk-live`. |
| renamed, bodies kept (+ `.excerpts`) | `excerpts_for_reads_a_tree_it_can_render` → `step_pass_reads_a_tree_it_can_render`; `…_skips_a_template_without_the_placeholder`, `…_with_no_roots_is_the_empty_audit`, `…_drops_a_file_the_scrubber_refuses`, `…_never_persists_a_note_naming_a_masked_path` likewise (`excerpts_for_` → `step_pass_`). |

`engine.rs` tests (after `a_handoff_spec_carries_no_excerpts_and_runs_no_pass`, `:12889`). New helper
beside `skills_prologue` (`:12895`): `async fn attach_glob(harness: &Harness, name: &str, globs:
&[&str]) -> SkillId` = `create_skill(NewSkill { name, body: "Glob body.", .. })` +
`set_skill_binding(SkillBindingKey { skill, project: Some(ids::PROJECT_HTUI), phase: None }, None,
BindingChange::Attach(Attachment { activation: Glob, globs, pinned_version: None, position: 0,
languages: vec![] }))` asserting `Applied` (shape of `:13117-13171`).

| Test | Fixture → asserts |
|---|---|
| `a_glob_skill_fires_on_a_file_in_the_step_tree` | `excerpt_prologue` + `attach_glob("rust-glob", ["**/*.rs"])`; `assemble_prompt(.., &snapshot.phases[0], FEAT_3)` → `text` contains `<skill name="rust-glob"`; its choice: `reason Matched`, `active`, `path Some("htui:src/lib.rs")`. |
| `a_glob_skill_with_no_matching_file_records_no_match` | Same with `**/*.py` → `NoMatch`, not rendered. |
| `a_retry_matches_the_previous_attempts_changed_paths` | `a_forward_with_nothing_to_render_degrades_to_trim_notes`' set-up (`:6943-6960`: `free_feat_3`, `repoint` prd `Gate::Never`, `retry_limit 1`, `verify_command`, template `implement`; `FakeVerifier::fail(1)`), plus `add_primary_repo`, `root_trees_at(dir)`, `write_tree(dir, repo, "src/lib.rs", ..)`, `attach_glob("md-style", ["**/*.md"])`, and `script_changed_paths(ChangedPaths { paths: [(repo, "docs/notes.md"), (RepoId::new(), "stray.md")], truncated: false })`. `StartRun`. Attempt 1's `trim_record.skill_choices` records `md-style` `no_match`; attempt 2's `matched` with `path "htui:docs/notes.md"` (no `.md` in the tree; the out-of-scope id dropped). `changed_path_requests()` == one request whose ids are attempt 1's step. |
| `changed_paths_unavailable_is_a_note_not_a_failure` | Same set-up with `fail_changed_paths("index.lock held")` → attempt 2 `Done`; its notes hold a note starting ``changed paths unavailable: `` and containing `index.lock held`. |
| `a_fan_out_group_matches_under_repo_box_path` | `a_step_without_trees_reads_repo_box_path_with_a_note`' set-up (`:12745-12798`) + `attach_glob("rust-glob", ["**/*.rs"])` → after `with_excerpts(&row, &group, ..)`: `spec.step_files.is_reached("htui")`; `assemble(&spec, &MinimalScrubber::new([]))` records `matched` `htui:src/lib.rs`. |
| `a_judge_records_no_path_for_a_placed_glob_winner` | `a_judge_renders_the_judged_phases_skills_in_both_orders`' judge v2 body (`:13288-13313`) + `attach_glob("rust-glob", ["**/*.rs"])` → `forward.trim.skill_choices` has `("rust-glob", Some(1), false, NoPath)`; the same in `reversed`. **Passes at red**: it pins D123's existing behaviour. |
| `a_template_without_excerpts_runs_no_pass` (`:12590`) | Unchanged, must stay green (the demo holds no `glob` attachment, so `needs_files` is false). |

`crates/htui/tests/prompt_preview.rs`: `SKILLS_NOTE` (`:343-345`) and the verbatim list entry
(`:329-330`) → D124's text. New `the_preview_fires_a_glob_skill_over_repo_box_path`:
`a_project_with_a_checkout()` (`:182-228`) + a new skill with a project `glob` `**/*.rs` attachment on
`PROJECT_HTUI` written through `set_skill_binding` → `preview::build(.., FEAT-1, None, ..)` →
`text` contains that skill's `<skill name="…"`, its choice `matched` `htui:src/lib.rs`.

### 6.5 Commits (D133)

1. `feat(mod-9): walk_pass and excerpt_pass share one listing (D114, D121)` — agent: `walk_pass`,
   `excerpt_pass(req, &Listing, est)`; `excerpts_for` walks then selects inside its one blocking
   hop; `PassInput.changed_paths` feeds `request.changed_paths`; literal sites (`engine.rs:5010`,
   `preview.rs:270`, agent tests) get `Vec::new()`; `:1114`/`:1144` updated. All green.
2. `test(mod-9): step_pass, the engine and the preview fire glob skills, red` — `StepPass`,
   `step_pass` (`todo!()`), re-exports; every §6.4 test; the preview note pins flipped (the existing
   preview note tests go red with them — expected at red).
3. `feat(mod-9): step_pass walks once for excerpts and glob skills (D116, D120)` — agent green;
   `excerpts_for` becomes `step_pass(..).await.excerpts`, kept one more commit.
4. `feat(mod-9): stage 3 feeds changed paths and the file set to the prompt (D122, D123)` — engine.
5. `feat(mod-9): the preview fires glob skills over repo_box_path (D124)` — preview; delete
   `excerpts_for` and its re-export; rename the five tests; doc sweep (`htui-agent/src/excerpt.rs:24,958`,
   `engine.rs:4970`, `preview.rs:12`, `prompt/mod.rs:25`).

### 6.6 Gate

`cargo test -p htui-agent -p htui-orch -p htui --all-features -- --test-threads=1`;
`cargo build --workspace --all-targets --all-features`;
`ls crates/htui/tests/snapshots | wc -l` = 107 and `git diff --stat 0e73705 -- crates/htui/tests/snapshots`
empty. A snapshot that moves under `cargo insta review` is a regression (D124), never an accept.

### 6.7 Hazards

- **Residual after the file set** (D120 step 7): computed over `PromptSpec { step_files: files.clone(),
  ..spec.clone() }` — the plain `spec` would under-count an active matched skill.
- The request moves into the walk's `spawn_blocking`; return it from the closure (or clone before)
  or the select hop has nothing to run on. `roots` must be cloned before the move for `unscanned`.
- Two blocking hops now: a readable pass costs two `spawn_blocking`s. Both fail open.
- The "nothing was read" note text must stay byte-identical (pinned at `engine.rs:12624` and in the
  agent tests); the new "listed files for glob skills only" note is a **different** sentence.
- Caller notes (incl. ``changed paths unavailable``) go **first** in `excerpts.notes`.
- `a_template_without_excerpts_runs_no_pass` stays green only because the demo has no `glob`
  attachment; a demo change adding one would flip it.
- Numbering: `engine.rs` already cites D108, D122, D125 (MOD-7) and `htui-agent/src/excerpt.rs`
  D109, D118, D119, D122 (MOD-7). New comments say "MOD-9 D122", never bare "D122".
- `FakeIsolator::changed_paths` unscripted must answer `Ok(default)`: from commit 4 every retry test
  in `htui-orch` calls it.

---

## 7. T6 — Prompt sub-tab and attachments pane (D125, D126)

### 7.1 `crates/htui/src/ui/tabs/backlog/detail/prompt.rs`

`choice_line` (`:211-229`):

```rust
    let outcome = match (choice.reason, &choice.path) {
        (ChoiceReason::Always, _) => "active".to_owned(),
        (ChoiceReason::Matched, Some(path)) => format!("matched {}", path.replace(['\n', '\r'], " ")),
        (reason, _) => reason.as_str().to_owned(),
    };
```

(doc: "`active` for `always`, `matched <repo:path>` for `matched` (MOD-9 D125), the reason
otherwise"). Import `ChoiceReason`. Dimming stays `!choice.active` (`:191`), so `matched` is normal
style, `no_match` dim. The comment at `:244` is about excerpt roots — leave it.

### 7.2 `crates/htui/src/skills.rs` (D126 as built by D131)

`ProjectSkills` (`:57-68`) gains, last:

```rust
    /// MOD-9 D126: its repo names with no `repo_box_path` row for this box, byte order; every
    /// repo when the box is unregistered.
    pub unrooted: Vec<String>,
```

`snapshot(writer: &Writer, scope: &Scope, box_paths: &[RepoBoxPath]) -> Result<SkillsSnapshot>`
(`:146`): in the per-project loop keep the `Repo` rows, `repos` as today, `unrooted` = names whose
`id` is in no `box_paths` row, sorted by bytes. New private
`async fn this_box_paths(backend: &Backend) -> Result<Vec<RepoBoxPath>>` (box_info → repo_paths).
Callers: the `Skills` arm (`:222-229`) and the import arm (`:314-324`) call it after their writer
check; `answer` (`:349`) becomes `answer(backend: &Backend, writer, scope, stale)` and calls it
before `snapshot`; its four callers (`:250`, `:267`, `:291`, `:312`) pass `backend`. Doc `:135-145`
names the new read (one per request, never per repo).

### 7.3 `crates/htui/src/ui/tabs/skills/attach.rs`

Delete `FIRES_LATER` (`:56-57`) and its push (`:753-755`); module doc `:12-13` becomes "A `glob`
attachment fires over the step's file set (MOD-9 D111); a project or phase row whose globs reach a
repo with no path on this box says so." `summary` (`:729-757`) after the globs part:

```rust
        if stored.activation == Activation::Glob
            && let Some(entry) = stored.project_id.and_then(|p| snapshot.project(p))
            && let Ok(compiled) = SkillGlobs::compile(&stored.globs)
        {
            let missing: Vec<&str> = entry.unrooted.iter().map(String::as_str)
                .filter(|repo| compiled.reaches(repo)).collect();
            if !missing.is_empty() {
                parts.push(format!("no path here: {}", missing.join(", ")));   // MOD-9 D126
            }
        }
```

A global row (`project_id: None`) adds nothing; a glob that no longer compiles adds nothing (the
`?` mark and the form carry that). Doc `:729-731` updated. The line fits: the pane is 100 columns,
`LABEL_WIDTH` 28 (`:45`); `glob · latest · pos 0 · 2 globs · no path here: htui, web` is 57 of ~66.

### 7.4 Tests

| Test | Where | Asserts |
|---|---|---|
| `a_matched_choice_names_its_path` | `detail/prompt.rs` unit | `choice_line(&SkillChoice { name "rust-glob", version Some(1), level Project, activation Glob, active true, reason Matched, path Some("htui:src/lib\nx.rs") })` == `"rust-glob v1 · project · glob → matched htui:src/lib x.rs"`. |
| `a_no_match_choice_is_dim_and_says_so` | `detail/prompt.rs` unit | Push a `NoMatch` glob choice onto `phase_implement_attempt2`'s record (the `:471-538` shape) → its row text ends `glob → no_match` and `dim == true`. |
| `the_snapshot_lists_repos_with_no_path_on_this_box` | `skills.rs` unit | Demo + repos `htui` (primary) and `web` on `PROJECT_HTUI`, a `repo_box_path` for `htui` on `ids::BOX` → `read(&backend, &platform_scope(..))`'s `htui` entry has `repos == ["htui","web"]`, `unrooted == ["web"]`; `agy`'s `unrooted == []`. |
| `a_glob_row_names_repos_with_no_path_here` (replaces `:641-668`) | `tests/skills.rs` | `add_repo("htui", true)`, `add_repo("web", false)`, the `tests` row set to `Glob` + `languages: ["rust"]` → line ` htui ` contains `glob · latest · pos 0 · 2 globs · no path here: htui, web`. |
| `a_glob_row_with_every_repo_rooted_adds_nothing` | `tests/skills.rs` | As above plus `upsert_repo_box_path` for both on `ids::BOX` → the line ends `· 2 globs` and contains no `no path here`. |
| `a_qualified_glob_warns_only_for_its_own_repo` | `tests/skills.rs` | Repos `htui`, `web`, neither rooted; row `Glob`, `globs: ["web:**/*.ts"]` → `… · 1 globs · no path here: web` (never `htui`). |

`the_snapshot_carries_the_library_the_attachments_and_the_scope_s_graphs` (`skills.rs:493`): if it
compares whole `ProjectSkills` values, it gains `unrooted: vec!["core"]` for `htui` (repo `core` has
no path row) — a moved pin, message "MOD-9 D126: `core` has no path on this box".

### 7.5 Commits

1. `test(mod-9): the Prompt sub-tab and the attachments pane name matches and missing paths, red` —
   `ProjectSkills.unrooted` filled with `Vec::new()`, tests.
2. `feat(mod-9): the Prompt sub-tab shows matched <repo:path> (D125)`.
3. `feat(mod-9): the attachments pane names repos with no path here (D126)` — `snapshot` and
   `answer` signatures, `this_box_paths`, the summary, `FIRES_LATER` gone.

### 7.6 Gate

`cargo test -p htui --all-features -- --test-threads=1` (Postgres env set); snapshot dir diff empty
(`skills__attachments.snap` and its siblings hold no `glob` row).

### 7.7 Hazards

- Read `this_box_paths` after the writer check in every arm, or
  `an_offline_read_is_refused_with_the_server_only_sentence` answers the orchestration sentence.
- A global row must add nothing (it spans every project, D126).
- The line is `"{n} globs"` even for one (`1 globs`), as today (`:751`).

---

## 8. T7 — finding 6: edit a skill with no version (D127, D134, OQ-28)

### 8.1 `crates/htui/src/ui/tabs/skills/library.rs` — `on_skill_key` (`:703-779`)

Replace the early return (`:714-717`) with optional values:

```rust
        let head = snapshot.head(skill).map(|row| row.version);
        let shown = self.shown_row(snapshot, skill).map(|row| (row.version, row.body.clone()));
```

- `,` `.` `b` `d`: `let (Some(head), Some((shown_version, _))) = (head, &shown) else { return };`
  then today's bodies (inert with no version).
- `e` / `E`: `let (token, from, body) = match (head, shown) { (Some(h), Some((v, b))) => (h, Some(v),
  b), _ => (0, None, String::new()) };` → `Editor::new(target, token, from, &body)` (`:322`). A save
  sends `SaveSkillVersion { expected: 0, .. }`, which `add_skill_version` writes as v1
  (`traits.rs:851-855`, `pg/write.rs:2876-2883`). Comment "MOD-9 D127".
- `i`, `a`: unchanged bodies, no longer blocked (MOD-9 D134).

### 8.2 Tests

| Test | Where | Asserts |
|---|---|---|
| `a_skill_with_no_version_opens_an_empty_editor_on_token_0` | `library.rs` `mod tests` (`:1716`) | Serve a `Skills` read over `Backend::memory(MemStore::demo())`, clear one entry's `versions` in the reply, land it (`on_reply`); `e` → `Mode::Editing` with `token 0`, `from None`, empty text; type `Body.`, `ctrl-s` → one `SaveSkillVersion { expected: 0, .. }` sent; `,` `.` `b` `d` on that skill send nothing and change no mode; `i` opens the info form. |
| `a_skill_with_no_version_is_edited_into_v1_on_postgres` | `tests/skills_pg.rs` | Raw `INSERT INTO skill (name, description, created_by) VALUES ('bare-skill', '', $1)` with `ids::USER` on `stack.db.pool`; `2`, `r`, select `bare-skill`, `e`, type, `ctrl-s`, settle → `db.store.skill_versions(id)` is exactly `[v1]` with the typed body; the frame shows `v1`. |

### 8.3 Commits

1. `test(mod-9): a skill with no version opens in the editor, red`.
2. `feat(mod-9): e and E edit a skill with no version into v1 (D127, D134, OQ-28)`.

### 8.4 Gate

`cargo test -p htui --all-features -- --test-threads=1` (Postgres).

### 8.5 Hazards

- `MemStore` cannot hold a version-less skill (`create_skill` writes v1); only the unit test's
  doctored reply and Postgres reach the state.
- `a_blank_body_dispatches_no_request` (`:2018`) must stay green: an unchanged empty body sends
  nothing.

---

## 9. T8 — close-out (MOD-9 done)

As the plan's T8 (items 1–10), after the `rust-reviewer` gate (**unpinned**, memory) and its fixes.
Blueprint additions: the write-up lists D130–D137 beside D108–D129 and states that `excerpts_for`
is gone (replaced by `step_pass`), `Isolator` has nine methods, and `trim_record` is `v 3`.

Commits:
1. `docs(mod-9): MOD-9 write-up, index line and HANDOFF close-out` — `docs/decisions/mod/mod-9.md`,
   `DECISIONS.md`, `HANDOFF.md` (entry removed; live coordinates: migrations end at
   `0008_trim_record_v3`, next `0009`, cache next `0005`, `trim_record` v 3; summary MOD-N 35 → 34;
   status line; cross-links).
2. `docs(mod-9): PRD row 5 complete, ANA-22 amendment, plan implemented` — PRD row 5, `docs/ANA-22.md`
   §10 line, plan Status line.

Gate: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## 10. Cross-task coupling, pins and the workspace gate

**Compile coupling** (a commit that changes the left breaks the right until the same commit fixes it):

| Change | Breaks |
|---|---|
| T1 `select(.., &StepFiles)` | `htui-core` `prompt/mod.rs:861`; `htui-store/tests/skill_attachments.rs:300` |
| T1 `SkillChoice.path` | `htui-core` tests `:85`; `htui` `detail/prompt.rs:480`, `tests/prompt_preview.rs:357` |
| T2 `PromptSpec.step_files` | `htui-core` fixtures ×4; `htui-orch` `engine.rs:4418`, `:5111`; `htui` `preview.rs:276` |
| T4 `Isolator::changed_paths` | `real.rs:1387`, `fake.rs:409`, `tests/gix_isolator.rs:1987` |
| T5 `PassInput.changed_paths` | `engine.rs:5010`, `preview.rs:270`, agent tests `:1102`, `:1378` |
| T5 `excerpt_pass(.., &Listing, ..)` | agent tests `:1114`, `:1144` |
| T5 `excerpts_for` removed | `engine.rs:28,5015`, `preview.rs:29,308`, `lib.rs:149`, agent tests — hence D133's wrapper |
| T6 `ProjectSkills.unrooted`; `snapshot`/`answer` arity | `skills.rs:165`; `skills.rs:227`, `:250`, `:267`, `:291`, `:312`, `:321`, `:350` |

**Pins (D128, re-verified)**. Unchanged: store `CASES` 96, `READ_CASES` 14, orch `CASES` 73,
`GraphSource` 7, `StoreRequest` 85, `StoreReply` 47, `skills::REQUEST_NAMES` 6, `.sqlx` 288,
`TABLES` 39, commented columns 34, `MIRRORED_TABLES` 21, snapshots 107 (content unchanged).
Moved: `RECORD_VERSION` 2 → 3 (`prompt_digest.rs:1009`); migrations `0001`..`0008`, next `0009`,
`Pending(7)` → `Pending(8)` (`migrations.rs:884,983,988,1007`; `connect.rs:141,157,243`);
`Isolator` 8 → 9 methods; `ChoiceReason` 5 → 7 variants; `SkillChoice` 7 → 8 fields (the eighth
skipped when `None`).

**Workspace gate, before review**:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
# .sqlx check per §0.3 (288); snapshots 107 and no diff; migrations end at 0008
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

**Risks added here** (continuing the plan's R-56):

| # | Risk | Mitigation |
|---|---|---|
| R-57 | The newline and non-UTF-8 name tests do not run on macOS/Windows (`cfg`). | `parse_name_only`'s pure test covers the rules everywhere; the Linux gate runs the real ones. |
| R-58 | Two blocking hops per readable pass (walk, then select) instead of one. | Both off the runtime; the walk is the cost it was; the second hop reads only selected files. |
| R-59 | D133's wrapper outlives T5 if commit 5 is skipped. | Commit 5 is part of T5's done-definition; removing the re-export makes a leftover caller fail to build. |

---

## 11. Numbering note

- This milestone's decisions are **MOD-9 D108–D129** (plan) and **D130–D137** (this blueprint, §13);
  risks **R-44–R-56** (plan) and **R-57–R-59** (here); OQ-27–OQ-29. Tasks T1–T8.
- **Every new code comment and doc cites "MOD-9 D1xx"**, never a bare "D1xx". The bare numbers already
  mean something else in exactly the files this milestone edits: MOD-7's D108/D122/D125 in
  `engine.rs`; MOD-7's D109/D118/D119/D122/D129 in `htui-agent/src/excerpt.rs` and
  `prompt/mod.rs` (`drop_unmaskable_excerpts` is "MOD-7 … D119", `withhold_unmaskable_notes` "D129",
  `excerpt_residual` "D118"); MOD-4's D141/D146 in `isolate/real.rs`. A reviewer reading "D119" in
  `prompt/mod.rs` cannot tell `Isolator::changed_paths` from `drop_unmaskable_excerpts` without it.
- The SQL comment keeps D118's wording ("as amended by MOD-9 D42 and D118"), which already qualifies.

---

## 12. Where to look (pre-edit, `0e73705`)

| Symbol / anchor | File:line |
|---|---|
| `ChoiceReason`, `as_str` | `crates/htui-core/src/model/skill.rs:383-407` |
| `SkillChoice` | `…/model/skill.rs:415-431` |
| `select` (model) | `…/model/skill.rs:432-470` |
| `BoundSkill`, `collapse` | `…/model/skill.rs:286-305`, `:326-343` |
| skill tests: `bound`, `select_applies_its_rules_in_order`, `choices_serialize_…` | `…/model/skill.rs:482-499`, `:836-879`, `:882-941` |
| `SkillGlobs`, `compile`, `first_match` | `crates/htui-core/src/model/skill_glob.rs:177-213` |
| model re-exports | `crates/htui-core/src/model/mod.rs:150-154` |
| `PromptSpec` (`excerpts` field) | `crates/htui-core/src/prompt/mod.rs:71-123` (`:103`) |
| `excerpt_residual` | `…/prompt/mod.rs:522-539` |
| skill-name masking; `select` call | `…/prompt/mod.rs:778-782`; `:858-861` |
| `drop_unmaskable_excerpts`, `withhold_unmaskable_notes`, `refused_rule`, `scrubs_unchanged`, `mask`, `scrub_text` | `…/prompt/mod.rs:890-960`, `:981-987`, `:991-996`, `:1000-1005`, `:1008`, `:1040` |
| `notes(spec)` | `…/prompt/mod.rs:1117-1121` |
| `RECORD_VERSION`; `to_value` doc; `record` | `crates/htui-core/src/prompt/trim.rs:56-58`; `:230-276`; `:1079-1110` |
| `PathPrefix`, `RepoPath`, `skip_by_path` | `crates/htui-core/src/prompt/excerpt.rs:53-98`, `:101-107`, `:135-155` |
| `ExcerptReason` (`PrevDiff`) | `…/prompt/excerpt.rs:191-224` |
| `RootRecord`, `ExcerptAudit` | `…/prompt/excerpt.rs:274-283`, `:315-330` |
| `ExcerptRequest.changed_paths`, `OwnedExcerptRequest` | `…/prompt/excerpt.rs:391-414` (`:403`), `:422-483` |
| `RepoReader`, `Listed` | `…/prompt/excerpt.rs:555-593`, `:600-608` |
| `tier_of` | `…/prompt/excerpt.rs:742-776` |
| `is_repo_relative` | `…/prompt/excerpt.rs:952-961` |
| `select` (excerpt): caps `:1069-1079`, steps 1-2 `:1081-1168`, rest `:1170-1298` | `…/prompt/excerpt.rs:1050-1299` |
| `ExcerptSet`; test `request`, `MapReader`, `root` | `…/prompt/excerpt.rs:1462-1472`; `:1488`, `:1671-1763`, `:1765` |
| fixtures `PromptSpec` literals; `phase_skills_over_cap` | `crates/htui-core/src/prompt/fixtures.rs:161,259,361,499`; `:700` |
| demo `IMPL_TRIM_RECORD` (`"v": 2`, stays) | `crates/htui-core/src/fixtures.rs:1640-1641` |
| `prompt_skills.rs` literal, glob test, cap test, digest test | `crates/htui-core/tests/prompt_skills.rs:85`, `:100-114`, `:158-190`, `:220-262` |
| record `v` pin | `crates/htui-core/tests/prompt_digest.rs:1007-1011` |
| scrubber prefix rules | `crates/htui-core/src/scrub.rs:20-29`, `:253-260` |
| store `select` caller | `crates/htui-store/tests/skill_attachments.rs:19`, `:300` |
| `0007` §2 comment | `crates/htui-store/migrations/0007_skill_attachments.sql:39-50` |
| migration pins | `crates/htui-store/tests/migrations.rs:86-93`, `:181-183`, `:214-224`, `:429-450`, `:884-885`, `:983`, `:988`, `:1007`; `tests/connect.rs:139-144`, `:156-160`, `:241-246` |
| `Backend::repo_paths`, `box_info` | `crates/htui-store/src/backend.rs:520-532`, `:252-258` |
| `Isolator` trait, `diff` | `crates/htui-orch/src/isolate.rs:124-238`, `:178-190` |
| orch re-exports | `crates/htui-orch/src/lib.rs:60-63` |
| `DIFF_CAP`; `run`; `run_capturing`; `Capture` | `crates/htui-orch/src/isolate/git.rs:65-72`; `:346-355`; `:357-421`; `:424-431` |
| `Cli::diff`; `Exited`; `failure` | `…/isolate/git.rs:801-841`; `:934-948`; `:978` |
| `HeadBuffer`, `Sink`, `into_string` | `…/isolate/git.rs:1083-1123`, `:1125-1146`, `:1172-1177` |
| git testkit (`usable_git`, `repo_with_one_commit`, `commit_file`); diff tests | `…/isolate/git.rs:1904-2070`; `:3443-3600` |
| `GixIsolator::cli`, `diff_of`, impl, `diff` | `crates/htui-orch/src/isolate/real.rs:381`, `:1275-1351`, `:1387`, `:1448-1480` |
| `FakeIsolator` fields, `DiffRequest`, `script_diff`/`fail_diff`/`diff_requests`, impl `diff`, tests | `crates/htui-orch/src/fake.rs:68-119`, `:121-129`, `:215-241`, `:574-596`, `:1772` |
| gix fixture, `StallAfterReconcile`, `committed_worktree_step`, D141 test | `crates/htui-orch/tests/gix_isolator.rs:115-129`, `:1956-2058`, `:2708-2736`, `:2795-2831` |
| `winner_at` | `crates/htui-orch/src/status.rs:243` |
| engine imports; judge spec; judge `prepare` | `crates/htui-orch/src/engine.rs:28`, `:38`; `:4413-4455`; `:4578-4582` |
| `assemble_prompt`; `with_excerpts`; `phase_spec`; `forwarded` | `…/engine.rs:4940-4959`; `:4961-5018`; `:5030-5154`; `:5190-5247` |
| `Harness::add_primary_repo`; retry-degrade test | `…/engine.rs:6262-6277`; `:6943-7030` |
| excerpt tests: `touch_feat_3`, `write_tree`, `excerpt_prologue`, no-pass, `repo_box_path` group, handoff | `…/engine.rs:12496`, `:12516`, `:12525-12541`, `:12590-12628`, `:12745-12817`, `:12823-12889` |
| skills tests: `skills_prologue`, `choice_rows`, `implement_prompt`, `set_skill_binding` case, judge cases | `…/engine.rs:12895`, `:12957`, `:12974`, `:13117`, `:13283`, `:13370` |
| agent imports; `excerpt_roots`; `touched_prefixes`; `PassInput`; `excerpt_pass`; `excerpts_for` (D122 comment); `unscanned` | `crates/htui-agent/src/excerpt.rs:36-47`; `:902-928`; `:934-943`; `:945-955`; `:957-972`; `:974-1085` (`:1044-1045`); `:1087-1111` |
| `FsRepoReader::new` (no I/O) | `…/htui-agent/src/excerpt.rs:230-232` |
| agent re-exports | `crates/htui-agent/src/lib.rs:148-151` |
| agent tests: imports, `write`, `fs_root`, `phase_spec`, `readable_input`, pass tests | `crates/htui-agent/tests/excerpt.rs:13-27`, `:461`, `:479`, `:1094`, `:1101-1107`, `:1109-1159`, `:1241-1408` |
| preview: `SKILLS_NOTE`; `build` (input, spec, pass) | `crates/htui/src/preview.rs:85-88`; `:155-316` (`:270-274`, `:276-301`, `:308`) |
| preview tests: note pins, `active`, `a_project_with_a_checkout` | `crates/htui/tests/prompt_preview.rs:329-330`, `:343-345`, `:351-366`, `:182-228` |
| `choice_line`; its test | `crates/htui/src/ui/tabs/backlog/detail/prompt.rs:211-229`; `:471-538` |
| `ProjectSkills`; `snapshot`; `serve`; `answer` | `crates/htui/src/skills.rs:53-68`; `:135-182`; `:220-333`; `:347-358` |
| `hierarchy::serve` shape | `crates/htui/src/hierarchy.rs:221-225` |
| attach: `LABEL_WIDTH`, `FIRES_LATER`, `summary` | `crates/htui/src/ui/tabs/skills/attach.rs:45`, `:56-57`, `:729-757` |
| skills UI tests: `always`, `set_binding`, `add_repo`, fires-later test | `crates/htui/tests/skills.rs:156`, `:167`, `:184`, `:641-668` |
| `Editor::new`; `on_skill_key`; tests | `crates/htui/src/ui/tabs/skills/library.rs:322`; `:703-779`; `:1716` |
| Postgres UI stack | `crates/htui/tests/skills_pg.rs:28-90` |

---

## 13. Decisions proposed here (D130 onward)

| # | Decision | Why |
|---|---|---|
| D130 | `Cli::name_only` reads raw bytes through a private `Cli::run_captured` returning `Captured { code, stdout: Sink, stderr }`; `run_capturing` becomes its decoding wrapper; `Capture::HeadBytes` → `Sink::HeadBytes(HeadBuffer)` → `HeadBuffer::into_bytes`. `Exited` is unchanged. Flags add `--no-color` to D119's. The NUL parse is the pure `parse_name_only(bytes, overflowed)`: always drop the final segment; drop non-UTF-8 and empty entries; `truncated = overflowed`. | B-1: D119's "`run_capturing` returns them without decoding" cannot hold while it returns `Exited { stdout: String }`. |
| D131 | `skills::snapshot(writer, scope, box_paths: &[RepoBoxPath])`; `serve`'s read and import arms and `answer(backend, …)` read `box_info` then `repo_paths` once through `this_box_paths`, after the writer check. | B-2: `snapshot` and `answer` hold a `Writer`, not the `Backend`. |
| D132 | `step_pass`: `withhold_unmaskable_notes` over `listing.notes` in the glob-only branch; `drop_unmaskable_files` before the residual and on every branch, its note last in `excerpts.notes`; `StepFiles::retain` keeps reach; walk-hop `JoinError` keeps changed paths only, select-hop keeps the listing's set, residual `Err` stays today's output. | F-1, F-2, F-3. |
| D133 | `excerpts_for` stays one commit as `step_pass(..).await.excerpts`, deleted in T5's last commit. | F-4: every commit compiles. |
| D134 | Finding 6's guard restructure unblocks `i` and `a` too; `,` `.` `b` `d` stay inert with no version. | F-5; R-SKL-3. |
| D135 | `FakeIsolator` records `changed_path_requests` in the `DiffRequest` shape. | F-6. |
| D136 | `select_is_list_then_select_listed` pins the literal note order over one all-notes fixture. | F-7. |
| D137 | `StepFiles::is_empty` = reaches no repo. | Default is "nothing reached"; the useful emptiness test is reach, not path count. |
