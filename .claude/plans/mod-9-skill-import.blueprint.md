# Blueprint: MOD-9 milestone 4, "a `SKILL.md` import"

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

**Status**: written inline on the session model 2026-09-28, after the `code-architect` subagent
failed twice on an OpenRouter 402. See §0.2 — the review gate is degraded for this milestone and the
done-report must say so.

**Plan**: `.claude/plans/mod-9-skill-import.plan.md` (confirmed 2026-09-28). Its D90–D105, OQ-21
through OQ-26, R-36 through R-43 and T1–T3 are authoritative **except where §0 amends them**.
**PRD**: `.claude/prds/mod-9-skill-library-templates.prd.md:186` (milestone 4). **ANA**:
`docs/ANA-22.md` §5.6, §5.7, §6 items 11–12, §7.3, §8, §9. **Milestone 3's blueprint**:
`.claude/plans/mod-9-skills-editable.blueprint.md`.

**Verified at**: HEAD `4abb49a` on branch `htui-mod-9-m4` (worktree `/media/projects/htui-mod-9-m4`).
The tree carries milestone 3. `crates/htui-store/.sqlx/` holds **281** files;
`crates/htui/tests/snapshots/` holds **97**, five `skills__*.snap` and six `templates__*.snap`.
**Line numbers are pre-edit.**

**Gortex**: the daemon's index is pinned to `/home/mluigi/projects/htui`, checked out at `main`
(`68c058f`) — **one milestone behind this worktree**, and its `edit` path writes to that checkout.
Every fact below was read directly from this worktree. Gortex is a cross-reference here, never a
mutation path.

**Scope**:

- **Order**: T1 → T2 → T3, all serial. T2 and T3 are coupled by one constant: T3's
  `IMPORT_NAME: &str = REQUEST_NAMES[4]` cannot compile before T2 adds the fifth name (the same
  shape as milestone 3's `mod matrix;` problem, R-30 there).
- **No migration.** `0009` stays free. `skill_version.source` arrived with `0007`, and both writers
  and every reader already carry it (D90, D101).
- **No new `WriteStore` method**, so no conformance case, no spy arm, no `.sqlx` change. `WriteStore`
  stays **83**, `CASES` **81**, `READ_CASES` **14**, `EXPECTED_CASES` **81**, `TABLES` **39**,
  commented columns **34**, `MIRRORED_TABLES` **21**.
- **One new `StoreRequest` variant** (73 → 74), **one new `StoreReply` variant** (42 → 43),
  `REQUEST_NAMES` 4 → 5, **two new pure modules** in `htui-core`, **one new worker module**, two new
  modes in the Skills view, one prefill seam in the matrix.
- **No dependency.** `git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml` is a gate in
  every task (OQ-21).

**House style** (carried from milestones 1–3): `unsafe_code = "forbid"`; `missing_docs` on lib roots;
`missing_debug_implementations` and `unused_qualifications` warn; clippy `-D warnings`; rustdoc
denies broken/private intra-doc links; `rustfmt.toml` `max_width = 100`. Every new `pub` item has a
doc comment and a `Debug`. A body crossing the store boundary goes in a newtype whose `Debug` prints
its length. Commit the red tests first, then green; **every commit compiles** (red commits use
`todo!()`). No test is loosened; a moved pin names its reason in the assertion message.

---

## 0. What the plan left, and where this blueprint departs

### 0.1 Settled answers the plan left to the architect

| # | Question | Answer | Why |
|---|---|---|---|
| A-1 | Where does the reader's byte offset point? | At the **key**, i.e. the first byte of the key token, not of the value and not of the line's indentation. | `TemplateError::at` is the byte of the opening `{{` — the token the editor's cursor should sit on. A key is the same kind of thing. `Split` also carries `body_at`, so a caller that wants the body has it. |
| A-2 | Does `Entry` keep its source line? | Yes: `Entry { key, value, at, line }`, where `line` is 1-based. | §9's mitigation asks for "per key error with **line number**", and the report is read by a human, not by a cursor. Computing it during the single forward scan is free. |
| A-3 | Is `Split`'s body byte-exact after the fence? | Yes, modulo three normalisations: the newline immediately after the closing `---` is dropped, `\r\n` is normalised to `\n` (`editor::normalise_newlines` is the precedent), and exactly one trailing `\n` is added. Everything between is untouched, leading blank lines included. | The plan's D95 says so. A skill body is markdown, and leading blank lines are load-bearing for some renderers. |
| A-4 | What is `Raw` exactly? | The **verbatim source text** of the value, from the first byte after `key:` to the last byte of the construct, with the key's own indentation stripped per line and nothing else normalised. | §5.6's row is "keep the raw frontmatter … lossless". A `Raw` that has been re-joined or trimmed is not that. |
| A-5 | How does the reader tell a `Raw` nested map from a `Raw` refusal? | A `Value::Raw` produced by a **recognised** construct carries no issue; one produced by a **refusal** carries exactly one `Issue`. Same variant, different `issues` entry, so nothing is lost either way. | One variant instead of two, and §5.7's "reported per key, never silently" is satisfied by the issue list. |
| A-6 | The `source` JSON key set. | `{"format", "path", "imported_at", "frontmatter", "issues", "skipped"}` — the plan's D95, with `frontmatter` a `serde_json::Map` of `String → serde_json::Value` built from the entries (`Scalar` → string, `List` → array of strings, `Raw` → the raw text as a string), `issues` an array of `{key, at, line, message}`, `skipped` an array of strings. | This is a **contract between T1 and T3**, not an internal detail: T3's prefill re-derives `activation` / `globs` / `languages` from it. The T1 unit test pins the key set so T3 cannot drift. |
| A-7 | The worker's module boundary. | `crates/htui/src/skill_import.rs` holds the walk, the read and the per-file orchestration; `skills.rs` gains one `serve` arm that calls it. `skill_import.rs` does **not** call `snapshot()` — the arm does, after the batch. | Keeps `skills.rs` (979 lines) the request/reply seam and the new file the work, so each stays readable. |
| A-8 | The reply's `report` ordering. | Input order: the paths the maintainer typed, in the order typed; within a directory, the walk's byte order. | "Reported per key" only means something if it is the order the maintainer can check against what they typed. |
| A-9 | `ImportOutcome`'s spelling. | `Imported { name, path, version }` / `Updated { name, path, version }` / `Unchanged { name, path }` / `Refused { path, message }` / `Skipped { path, reason }` — the plan's D97. `version` is `i32` on the two that wrote one. | — |
| A-10 | Does the report carry the skip list separately? | No: skips are `Skipped` rows in the same `report`, so "everything that happened, in one list" is one `Vec` and the view has no second shape to render. | D102's report is one list; a second list would double the render code for no reading. |
| A-11 | Where does `Mode::Report` sit in the `Mode` enum? | After `Naming`, before `Editing`: `Browse`, `Naming`, `ImportPath`, `Report`, `Editing`. | `captures_input` is `!matches!(self.mode, Mode::Browse)`, so both new modes capture input for free — which is correct, since both are text input or a list that owns the keys. |
| A-12 | Does the report need its own scroll state? | Yes, reusing the view's existing `self.scroll` (the same `Scroll` the browse list and the editor use), reset on open exactly as `open_naming` resets. | A second scroll state is a second thing to keep in sync for no gain. |

### 0.2 How this milestone was staffed, and what that costs

The plan's implementer fan-out and its review gate were to be run by subagents. **Every `Agent`
route returned `API Error: 402 Insufficient credits` (OpenRouter) from 10:10 CEST onward** — Opus 5.5,
then Sonnet 5, and the maintainer directed that neither be used; the `Agent` tool's model enum
(`sonnet | opus | haiku | fable`) cannot express the session model
(`stealth/space-bunny-alpha`), so no subagent could be spawned at all. The maintainer chose
**"go inline"** at 10:14 CEST.

**Consequence, stated once and repeated in the done-report:** the blueprint, T1, T2, T3 and the
review were all done by the same agent on the same model. **The review gate is not independent.**
§8 is a self-review with the same blind spots a reviewer would have. Treat it as a second pass over
the diff, not as the gate `handoff-run` requires.

### 0.3 Departures from the plan

None. Every question the plan left open is answered in §0.1, and nothing the plan decided is
re-decided here.

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (min., each compiles) | Gate |
|---|---|---|---|
| T1 reader + mapping | htui-core | 3 (§2) | core: fmt, clippy, test, doc; `git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml` |
| T2 worker path | htui | 3 (§3) | htui (Postgres, `--test-threads=1`), `--test skills`; `cargo build --workspace --all-features --all-targets` |
| T3 import surface | htui | 3 (§4) | htui (Postgres, `--test-threads=1`), `--test skills` and `--test skills_matrix`; `cargo insta review` |
| merge | — | in task order | the workspace gate (§7) |

Environment for every gate (this box):

```bash
# The Postgres DSN is not written here. Take it from the same source milestone 3's blueprint
# uses (`.claude/plans/mod-9-skills-editable.blueprint.md`, §1); it is a live credential and a
# plan document is the wrong place for it.
export PG="$HTUI_PG_DSN"          # postgres://…@localhost:5432
export PGT="USERNAME=htui-ci HTUI_TEST_DATABASE_URL=$PG/postgres"
export ORT_LIB_LOCATION=/home/user/ort/package/bin/napi-v3/linux/x64   # R-34: parcel.pyke.io 403
export LD_LIBRARY_PATH=$ORT_LIB_LOCATION
export CARGO_TARGET_DIR=/home/user/htui/target                          # one target dir (R-33)
# `pg_isready -h localhost -p 5432` must say "accepting connections"
df -h /                                                            # R-33 before every task
```

The keyring fake is process-wide, so the suite's green is scheduling-dependent: the workspace runs
with `--test-threads=2`, anything Postgres-backed with `--test-threads=1`. None of this milestone's
new tests touch the keyring.

---

## 2. T1: the reader and the mapping (D91–D95, D104, D105, A-1…A-6)

**Files.** `crates/htui-core/src/model/frontmatter.rs` (new), `crates/htui-core/src/model/
skill_import.rs` (new), `crates/htui-core/src/model/mod.rs` (two `pub mod` lines, alphabetical:
`frontmatter` between `event` and `hierarchy`, `skill_import` after `skill`), and
`crates/htui-core/tests/skill_import.rs` (new). Nothing else. `htui-core` names no `std::fs`
(`prompt/excerpt.rs:553-554`), so both modules are text-in/text-out.

### 2.1 `model/frontmatter.rs` — the reader

```rust
/// One frontmatter entry, with the byte and the line its key starts on.
pub struct Entry { pub key: String, pub value: Value, pub at: usize, pub line: usize }
/// A parsed value. `Raw` is the lossless landing spot, recognised or refused (A-5).
pub enum Value { Scalar(String), List(Vec<String>), Raw(String) }
/// A per-key problem. Never fatal; the body imports regardless (D94).
pub struct Issue { pub key: String, pub at: usize, pub line: usize, pub message: String }
/// A file split into its frontmatter and its body.
pub struct Split { pub frontmatter: Vec<Entry>, pub body: String, pub body_at: usize, pub issues: Vec<Issue> }
/// Why a file is not a skill file at all.
pub enum FrontmatterError { NoFence, Unterminated { at: usize } }

pub fn split(text: &str) -> Result<Split, FrontmatterError>;
impl Split {
    pub fn get(&self, key: &str) -> Option<&Value>;
    pub fn scalar(&self, key: &str) -> Option<&str>;      // Scalar only, trimmed
    pub fn list(&self, key: &str) -> Option<Vec<String>>; // List as is; Scalar split on ',' only
    pub fn flag(&self, key: &str) -> Option<bool>;         // Scalar "true"/"false" only (D104)
    pub fn has(&self, key: &str) -> bool;                  // any value, including Raw
}
```

`impl std::fmt::Display for FrontmatterError` and `for Issue`, both `thiserror::Error` for the enum
(`template.rs:377-412` is the shape). `Debug` is derived on every type.

**The scan.** One forward pass over lines, tracking the byte offset of each line's start, so `at`
and `line` are free. Rules, in the order a line is examined:

1. **Fence.** Line 1 (after a BOM) must be exactly `---` (trailing whitespace allowed — the
   precedent is `gate.rs`'s own "exactly `---`" rule, relaxed for CRLF, which is
   `normalise_newlines`' business). Otherwise `NoFence` (D105). Scan forward to the next line that
   is exactly `---`; if none within `MAX_FRONTMATTER_LINES = 200`, `Unterminated { at }` (D92 —
   defensive; no real file on this machine needs it, 0 of 12,987).
2. **Blank line** inside the block: skipped, no issue.
3. **A line whose first non-space character is `-`** and we are inside a list: a list item
   (D93). Indent width is **not** fixed — the corpus has 2-space and 4-space block lists.
4. **A line with no colon** and we are not inside a nested construct: an issue
   (`"a frontmatter line is neither a key nor a list item"`), the line skipped. This is what keeps a
   second-level nested map from being silently misread as garbage.
5. **`key:` with an empty value** and the next non-blank line is more-indented: a nested map, block
   form. The whole indented block is captured as `Raw` (A-4), indentation stripped per line. Its
   inner `key: value` lines are **not** entries.
6. **`key:` with an empty value** and the next non-blank line is a `- ` item: a block list.
7. **`key:` with a value** that starts with `|`, `|-`, `|+`, `>`, `>-` or `>+`: a block scalar. The
   body is the following more-indented lines; `|` keeps a trailing newline, `|-` drops it, `>` folds
   lines with spaces, `>-` folds and drops (OQ-26, confirmed).
8. **`key:` with a value** starting with `[` and ending with `]`: an inline flow list, split on
   commas, each item trimmed, quotes stripped by the same rule as a quoted scalar.
9. **`key:` with a value** starting with `"` or `'`: a quoted scalar; the closing quote must exist on
   that line or the value is `Raw` with an issue. `\\` and `\"`/`\'` are the only escapes.
10. **Anything else**: a plain scalar, trimmed, split at the **first** `": "` — wait, the split is at
    the first `:` **followed by a space or end of line**; a value containing `: ` afterwards is
    untouched. (D93; the corpus's `Triggers on: "build an agent"` is the fixture.)

**The colon rule, precisely.** `key` is everything before the first `:` that is followed by a space
or ends the line. `description:When building` (no space) is therefore a key `description` with the
value `When building` — which is what every real file means, and a stricter rule would refuse it.

**Fold rules for `>` / `>-`** (the case OQ-26 covers): consecutive non-blank indented lines join
with a single space; a blank line becomes one `\n`; more-indented lines keep their own newlines
(so a literal block inside a folded one survives). `|` and `|-`/`|+` keep the raw newlines.

**Tests**, in-module `#[cfg(test)]` plus the integration file:

- the five real-file shapes (the plan's T1 list), each with the source file named in a comment;
- `a_block_list_is_read_at_either_indent_width`;
- `a_nested_map_is_kept_verbatim` (the flow-map `metadata:` round-trips byte for byte);
- `the_reader_never_coerces_a_yaml_one_one_word`;
- `a_description_holding_colon_space_is_one_value` (scalar fixture);
- `a_rejected_key_is_reported_with_its_byte_and_its_line_and_the_body_still_imports`;
- `an_unterminated_fence_is_refused` (synthetic);
- `the_opening_fence_must_be_the_first_line` (BOM, leading blank line, a `---` rule in a body);
- `a_body_keeps_its_leading_blank_lines`.

### 2.2 `model/skill_import.rs` — §7.3's mapping

```rust
/// What §7.3 says the attachment form should be seeded with. Re-derived in T3 from `source`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportPrefill {
    pub activation: Option<Activation>,
    pub globs: Vec<String>,
    pub languages: Vec<String>,
    pub hint: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSkill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: serde_json::Value,
    pub prefill: ImportPrefill,
    pub issues: Vec<Issue>,
}

pub fn parse(path: &str, file_name: &str, text: &str, now: DateTime<Utc>) -> Result<ParsedSkill, String>;
```

- **`name`**: `name` → parent directory name when the file is `SKILL.md` → file stem. Then
  `Skill::name_is_valid`; a failure returns `Err(invalid_skill_name(name))` — the writer's own
  sentence (`traits.rs:1598-1605`), so the view's notice and the store's `Constraint` are the same
  string (D100's rule, D-OQ-22's default: refuse that file, slugify nothing).
- **`description`**: `description`, plus `when_to_use` appended after a blank line when both exist.
  Trimmed; a folded value arrives already re-joined (D93).
- **`body`**: A-3.
- **`source`**: A-6. `format` is `skill-md` / `mdc` / `markdown` by file name and extension
  (OQ-25). `path` is the string the caller passed. `imported_at` is `now.to_rfc3339()`.
- **`prefill`**: §7.3 row by row. The glob sources are `paths`, `globs`, `applyTo`, `fileMatchPattern`;
  a comma string is split, a list taken as is. The always-sources are `alwaysApply: true`,
  `trigger: always_on`, `inclusion: always`, and `applyTo: "**"` — `applyTo` is read
  **value-aware** because §7.3 gives it both meanings. `languages` is htui's own key. The
  `always`-with-hint row is: description only, or `trigger: model_decision`, or
  `inclusion: manual`, or `disable-model-invocation`; the hint names which.
  **Precedence, which the plan does not state and this blueprint does:** an always-source beats a
  glob-source, because `alwaysApply: true` beside a `globs` list is a Cursor file that means "these
  globs, and also always" and the narrower reading is the safe one to prefill. Stated in the module
  doc so T3 does not re-decide it.
- **`issues`**: the reader's, carried through.

**`prefill` is public and pure** so T3 can call `parse`-independent logic. T3 does **not** re-run
`parse` — it reads the stored `source` — but `ImportPrefill`'s *derivation* is needed there, so it
lives as `pub fn prefill_from_source(source: &serde_json::Value) -> ImportPrefill` in this module,
and `parse` calls it. One definition, two callers (D96).

**Tests** (`crates/htui-core/tests/skill_import.rs`): the §7.3 table row by row, including
`applyTo: "**"` meaning `always` while `applyTo: "src/**"` means `glob`; `the_source_keeps_the_whole
_frontmatter_verbatim` (pins the key set, A-6); `a_refused_name_carries_the_writers_own_sentence`;
`the_name_falls_back_to_the_directory_then_the_stem` (the `.mdc` half real, the `SKILL.md` half
synthesised); `prefill_from_source_is_the_same_derivation_the_import_used` — the T1↔T3 contract.

### 2.3 T1 hazards

| # | Hazard | Mitigation |
|---|---|---|
| H-1 | The reader is a parser written by the same agent that writes its tests, so a wrong rule is a wrong test. | The table is **quoted from real files**, not invented, and each entry names its source. Six of the entries have a real path. |
| H-2 | Indent-width handling is where a block list silently loses items. | One rule: "more indented than the key", never "exactly two spaces". Pinned by `a_block_list_is_read_at_either_indent_width`. |
| H-3 | A `Raw` that has been normalised is not lossless, and §5.6 promises lossless. | A-4 defines `Raw` as verbatim source text, indentation-stripped and nothing else. The flow-map test round-trips byte for byte. |
| H-4 | `pub` items in `htui-core` need docs and `Debug` (`missing_docs`, `missing_debug_implementations`). | Every type and every method gets a doc comment; `Debug` derived or hand-written. |
| H-5 | The new `pub mod` lines break the alphabetical list's shape. | Two lines, both alphabetical; `cargo fmt --check` catches the rest. |

**Commits** (each compiles): (1) the reader's types and `split` with the red table, using `todo!()`
for the unimplemented branches; (2) the reader green; (3) the mapping, its tests, and the two
`pub mod` lines.

---

## 3. T2: the worker path (D97–D100, A-7…A-10)

**Files.** `crates/htui/src/skill_import.rs` (new), `crates/htui/src/lib.rs` (`pub mod
skill_import;`, alphabetical, beside `pub mod skills;` at `:28`), `crates/htui/src/skills.rs`
(`REQUEST_NAMES`, one `serve` arm, the redaction test at `:471`), `crates/htui/src/store_worker.rs`
(the variant, the `name()` arm, the or-ed arm). No `.sqlx`, no store file.

### 3.1 `store_worker.rs` — the seam

```rust
ImportSkills {
    scope: Scope,
    /// One or more paths, in the order the maintainer typed them. A path may name a file or a
    /// directory (D99).
    paths: Vec<String>,
},

SkillImports(Box<SkillImports>),
```

- `name()` gains `Self::ImportSkills { .. } => "import_skills"`, at `:718-721`'s block, so the
  `request_names_match_the_name_arms` test still holds.
- `try_serve`: `StoreRequest::ImportSkills { .. }` is **or-ed into the existing skills arm** at
  `:1205-1208`. The match is wildcard-free (verified), so this is the only way in (D97).
- `SkillImports { snapshot: SkillsSnapshot, report: Vec<ImportOutcome> }` lives in
  `skill_import.rs` and is re-exported the way `SkillsSnapshot` is (`skills.rs`'s types are
  imported at `store_worker.rs:46`).

### 3.2 `skills.rs` — the request family

`REQUEST_NAMES: [&str; 5] = ["skills", "save_skill", "set_skill_binding", "remove_skill_binding",
"import_skills"]`. `READ_NAME` is still `REQUEST_NAMES[0]`. The new arm:

```rust
StoreRequest::ImportSkills { scope, paths } => {
    let report = skill_import::import(backend, &paths).await?;
    Ok(StoreReply::SkillImports(Box::new(SkillImports {
        snapshot: snapshot(backend, scope).await?,
        report,
    })))
}
```

`import` takes `&Backend` because it needs `backend.writer()` and `backend.this_user()` exactly as
`serve`'s other arms do (D100: `created_by` once per batch, R-NF-3 ownership). The re-read is
`snapshot()`, so the reply carries the post-import library and the view needs no second request.

### 3.3 `skill_import.rs` — the walk and the writes

```rust
pub const MAX_DEPTH: usize = 4;
pub const MAX_BYTES: u64 = 256 * 1024;
pub const MAX_FILES: usize = 64;
/// Bundled directories ANA-22 §6 item 12 names, plus the two every walk prunes.
pub const SKIP_DIRS: [&str; 5] = ["scripts", "references", "assets", ".git", "node_modules"];

pub async fn import(backend: &Backend, paths: &[String]) -> Result<Vec<ImportOutcome>, StoreError>;
fn collect(root: &Path, out: &mut Vec<PathBuf>, skipped: &mut Vec<(PathBuf, String)>, depth: usize) -> Result<(), String>;
fn parse_one(backend: &Backend, path: &Path) -> impl Future<Output = ImportOutcome>;
```

- **The walk** is hand-written over `std::fs::read_dir`, sorting every level's names on
  `as_encoded_bytes()` before descending — `FsRepoReader::walk`'s rule
  (`htui-agent/src/excerpt.rs:249-255`), and the reason two machines agree. `std::fs::read` inside
  the `async fn` is the `editor.rs:222` precedent; the walk is capped at `MAX_FILES`, so it cannot
  hold the worker long (R-42).
- **Selection** is D99: a file imports whatever it is; a directory contributes `SKILL.md` at depth
  ≤ `MAX_DEPTH`, and `*.md` / `*.mdc` at depth 1 only. `SKIP_DIRS` prunes whole subtrees and each
  prune is a `Skipped` row (ANA-22: "skipped **and listed**"). Symlinked directories are not
  descended; a symlinked file named directly is read. Over `MAX_BYTES` or non-UTF-8 is a
  `Skipped` row, never a silent stop.
- **The per-file write** (D100), with **both tokens from one read taken immediately before the
  writes** — the order milestone 2's review finding 5 settled for bindings and versions:

  ```
  read the skill library once per batch (one query's worth)
  for each parsed file:
      existing = library.named(name)
      match existing {
          None => upsert_skill(NewSkill { id: SkillId::new(), name, description, created_by }, None)?
                  add_skill_version(NewSkillVersion { skill_id, body, source, created_by }, None)?
                  -> Imported { name, path, version }
          Some(skill) =>
              upsert_skill(NewSkill { …, name, description, created_by }, Some(skill.updated_at))?
              head = versions.last().version
              if head.body == body { -> Unchanged { name, path } }
              else { add_skill_version(…, Some(head))? -> Updated { name, path, version: head + 1 } }
      }
      a `Stale` at either step -> Refused { path, message: the writer's own sentence }
  ```

  `Stale` never aborts the batch; the next file proceeds. A single `this_user()` before the loop.
- **Refusals** reuse `invalid_skill_name` and `skill_refusal` (`traits.rs:1598`, `:1644`) so the
  message the maintainer reads is the store's own (D100).

### 3.4 T2 tests

- `skill_import.rs` unit tests over a temp tree built in the test (`std::env::temp_dir()` +
  `tempfile::TempDir`, which is already a regular dependency of `htui`, `Cargo.toml:54`): the two
  selection shapes, the skip list, the byte cap, a non-UTF-8 file, a symlinked directory, the file
  cap, and **the walk's order is byte order and is the same on two runs**.
- `skills.rs`: `request_names_match_the_name_arms` moves to five names; the redaction test at
  `:471` is extended so no **file content** reaches a `Debug` sink (`path` and `message` may).
- `import` end to end over `MemStore::demo()`: a first import creates at v1 with `source` non-empty;
  a second import of a changed body appends v2 and moves the description; an identical body reports
  `Unchanged` and the head does not move; a refused name writes nothing; a directory import skips
  and lists.

### 3.5 T2 hazards

| # | Hazard | Mitigation |
|---|---|---|
| H-6 | The library re-read per batch is a second read of what `snapshot()` will read again. | One read at the start of the batch (the tokens), one at the end (the reply). Two reads, not per file. |
| H-7 | The new reply is mistaken for a save's answer by `land_save`, which matches on plain `StoreReply::Skills`. | `SkillImports` is a **different variant**, so `land_save`'s `StoreReply::Skills` arm cannot see it. The `busy` gate names the request, so `library.rs` matches on `busy == Some(IMPORT_NAME)`. |
| H-8 | A `Stale` mid-batch leaves a description updated with no version appended (R-39). | The report says which half landed, and the view's notice says so in the same sentence. Documented, not hidden — the same residue `skills.rs:10-12` already records for the save path. |
| H-9 | `std::fs` in an async fn on the worker. | The precedent exists (`editor.rs:222`); the walk is capped. Noted in the module doc. |

**Commits**: (1) the request/reply seam and the red name/consistency tests with `todo!()` bodies;
(2) the walk and the collection tests; (3) the writes and the end-to-end tests.

---

## 4. T3: the import surface (D96, D102, A-11, A-12)

**Files.** `crates/htui/src/ui/tabs/skills/library.rs`, `crates/htui/src/ui/tabs/skills/matrix.rs`,
`crates/htui/tests/skills.rs`, `crates/htui/tests/skills_matrix.rs`,
`crates/htui/tests/snapshots/skills__import_*.snap` (new). The six `templates__*.snap` and the five
existing `skills__*.snap` must not move, and neither may the strip text
` 1 Backlog  2 Skills  3 Settings  4 Chat` nor the switch line `" Skills │ Templates "`.

### 4.1 `library.rs` — two modes, one key

- `Mode` gains `ImportPath { field: TextField, busy_note: Option<String> }` and
  `Report { outcomes: Vec<ImportOutcome>, scroll_at: usize }`, in the A-11 order.
  `captures_input` (`:345`) is `!matches!(self.mode, Mode::Browse)` and needs no change.
- **Browse**: `KeyCode::Char('i')` in `on_browse_key`'s `Char(c @ …)` arm (`:525-534`) or its own
  arm before it, opening `Mode::ImportPath` with an empty `TextField` — the `templates::Mode::Naming`
  shape (`templates.rs:189-196`). `i` is free: zero occurrences of `'i'` in `library.rs`, and it is
  not in `Keymap::default_global`. The guard test's claimed array at `:1311-1314` gains `"i"`.
- **`BROWSE_HINT`** (`:92-93`) gains `  i import` at the end. It is a `const` rendered into a
  78-column budget; the milestone-3 note at `matrix.rs:98-99` says so, so the new clause is the
  shortest one that names the key.
- **`on_import_path_key`**: `Enter` → trim the field; empty is refused with a notice; otherwise
  `ctx.request(StoreRequest::ImportSkills { scope: ctx.scope.clone(), paths: vec![path] })`,
  `self.busy = Some(IMPORT_NAME)`, notice `IMPORTING`, mode back to `Browse` (so the list is
  visible while the worker walks). `Esc` → `Mode::Browse`, notice cleared. Everything else goes to
  the `TextField`, `Pass` when the field does not take it.
- **`on_reply`**: a new arm before the `Skills` arm, matching
  `StoreReply::SkillImports(imports)`. It must be **scope-guarded** the way `Skills` is
  (`in_scope(&imports.snapshot, ctx)`), then: store the snapshot, clear `unavailable`, and if
  `self.busy == Some(IMPORT_NAME)` clear `busy` and open `Mode::Report` **when any outcome is
  `Refused` or `Skipped`** (D102), else a two-line notice
  `format!("imported {i}, updated {u}, unchanged {c}")`. `Mode::Report` renders one row per outcome
  — `Imported`/`Updated` as `+ {name} v{n}`, `Unchanged` as `= {name}`, `Refused` as
  `! {path} — {message}`, `Skipped` as `· {path} — {reason}` — with `j`/`k` (and the existing `J/K`,
  `PageUp/PageDown`) scrolling, `Esc` returning to `Browse`, and `r` reloading the library.
- **`Mode::Report` and the editor**: `Report` captures input, so `Esc` in the view-level
  `capturing()` path reaches the view before the tab's `h`/`l` switch — which is what `mod.rs:143-148`
  already guarantees for every capturing view. Nothing to change in `mod.rs`.

### 4.2 `matrix.rs` — the prefill seam

`open_form` (`:723-772`) keeps seeding from the stored row, and gains one thing: **when the cell has
no row** (token `None`, a new attachment), it asks
`skill_import::prefill_from_source(&source_of_the_version_in_force)` and uses the answer for
`activation`, `globs` and `languages` **only where the stored row had nothing to say**. An existing
row is never re-seeded (D96): its stored values are the truth.

The version in force for a skill is the pin's version when the cell has a pin, the head otherwise —
the same rule `SkillBinding::version_in_force` (`model/skill.rs`) already implements, and
`matrix.rs` already has a `versions(view, skill_id)` helper for the pin refusal. The prefill reads
`SkillVersion.source`, which every reader already selects (D101), so **no new query and no new
reader**. `Form::effective()` remains the single source of the globs that get written (A-12): the
prefill fills `form.globs` and `form.languages`, and the save sends `form.effective()` exactly as
before.

A skill with no `source` (every skill typed in the TUI, whose `source` is `{}`) prefills nothing,
so the form looks exactly as it does today.

### 4.3 T3 tests

In `crates/htui/tests/skills.rs`, over `Harness::over(MemStore::demo())`, opening the tab with `2`:
`i_opens_a_path_form_and_enter_sends_one_request` (asserted on the store, as the existing
"nothing was sent" assertions are), `esc_leaves_the_path_form_and_sends_nothing`,
`an_import_of_one_file_creates_a_skill_at_version_one`, `an_import_of_a_second_file_creates_a_second_skill`,
`a_skill_the_library_already_holds_gets_a_new_version_only_when_the_body_differs`,
`an_import_refuses_one_file_and_imports_the_rest`,
`a_directory_import_skips_the_bundled_scripts_references_and_assets`,
`a_refused_name_writes_nothing`, `a_missing_path_is_refused_in_one_sentence`,
`the_import_report_lists_every_outcome` (snapshot),
`an_all_clean_import_leaves_a_notice_and_no_report`, and the two guard tests
(`every_browse_key_misses_the_global_table` with `"i"` claimed, `the_strip_text_is_unchanged`).

In `crates/htui/tests/skills_matrix.rs`:
`a_new_attachment_prefills_activation_and_globs_from_the_imported_source` (snapshot),
`an_existing_attachment_is_never_reseeded_from_a_source`,
`a_skill_with_no_source_prefills_nothing`, and
`the_prefilled_globs_are_the_ones_the_save_writes` (the stored row's `globs` equals
`form.effective()` after the prefill).

### 4.4 T3 hazards

| # | Hazard | Mitigation |
|---|---|---|
| H-10 | A new browse key collides with the global table or with a Templates key. | `i` is free in both, verified; the guard test is extended rather than trusted. |
| H-11 | The report's rows make a snapshot that later drifts from the byte-for-byte ones. | New snapshots only; the five `skills__` and six `templates__` files are asserted unchanged by the existing tests, which is the check. |
| H-12 | The prefill seeds a glob the matcher cannot compile, and the save is refused with a message the maintainer did not expect. | R-40: the prefill is shown in the form's own effective-globs pane before the save, and the refusal is the writer's own sentence. A glob `glob::compile` refuses is reported as an **import-time issue** (T1) so the maintainer sees it in the report, not at save time. |
| H-13 | A directory import pointed at this repo's root imports its own three `.claude/skills/*/SKILL.md`. | D99 states it as a consequence of the rule, and the report names every file touched. Not a leak; the maintainer chose the path. |

**Commits**: (1) the path form and its tests; (2) the reply arm, the notice, the report and its
snapshot; (3) the matrix prefill and its three tests.

---

## 5. Cross-task contracts

1. **`source`'s key set** is written by T1 and re-read by T3. T1's
   `the_source_keeps_the_whole_frontmatter_verbatim` pins it; T3 calls
   `prefill_from_source`, never a hand-rolled `get("globs")`.
2. **`ImportPrefill`'s derivation** has exactly one definition
   (`skill_import::prefill_from_source`), called by both T1's `parse` and T3's `open_form`.
3. **`REQUEST_NAMES[4]`** is T2's; T3's `IMPORT_NAME` indexes it, which is why T2 lands first.
4. **`ImportOutcome`** is defined once in T2 and rendered by T3; the render never re-derives a
   message the worker produced.

## 6. Count pins

**Do not move**: `WriteStore` 83, `CASES` 81, `READ_CASES` 14, `EXPECTED_CASES` 81, `TABLES` 39,
commented columns 34, `MIRRORED_TABLES` 21, `.sqlx` 281, `Cargo.lock` 736 packages, the next free
migration `0009`, the six `templates__*.snap`, the five existing `skills__*.snap`.

**Move**: `StoreRequest` 73 → 74, `StoreReply` 42 → 43, `REQUEST_NAMES` 4 → 5,
`crates/htui/tests/snapshots` 97 → 97+N.

## 7. Merge order and the workspace gate

After each task, that task's crate gate on the real tree. At the end:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci RUST_BACKTRACE=0 cargo test --workspace --all-features --no-fail-fast -- --test-threads=2
USERNAME=htui-ci RUST_BACKTRACE=0 cargo test -p htui-store -p htui --all-features -- --test-threads=1
cargo doc --workspace --no-deps --keep-going     # six baseline errors, unchanged
git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml
git diff --exit-code crates/htui-store/migrations crates/htui-store/.sqlx
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## 8. Review

`rust-reviewer` could not run: no subagent route is funded (§0.2). The review that stands in its
place is §9, and it is a **self-review**. Two rules make it worth something: the reviewer pass reads
the diff as a stranger would, with the plan's hazard list (R-36…R-43, H-1…H-13) as the checklist,
and every finding it produces is either fixed or written into the done-report as deferred, with its
reason. The done-report says plainly that this milestone shipped without an independent review gate.

## 9. Self-review checklist (the stand-in for the gate)

Run against the finished diff:

- [ ] Every new `pub` item has a doc comment and a `Debug`; rustdoc has no broken link.
- [ ] No unwrap/expect/panic on a path reachable from a real file; every refusal is a sentence.
- [ ] No file content reaches a `Debug`, a log, or an error message that a `StoreRequest` carries.
- [ ] The reader is total: every input either parses or produces an `Issue`/`FrontmatterError`.
- [ ] `Raw` round-trips byte for byte in the flow-map and nested-map cases.
- [ ] The two CAS tokens are read once, immediately before the writes, and a `Stale` never aborts
      the batch.
- [ ] The same-name rule: identical body writes nothing; different body appends and moves the
      description.
- [ ] The report lists every file, including the skips, and the counts notice agrees with it.
- [ ] The prefill never reseeds an existing attachment, and `form.effective()` is still the only
      source of the written globs.
- [ ] No test is skipped, no pin moved without naming its reason.
- [ ] The six `templates__*.snap`, the five pre-existing `skills__*.snap`, the strip text and the
      switch line are byte-identical.
- [ ] `git diff --exit-code` on `Cargo.lock`, the manifests, `migrations/` and `.sqlx` is clean.
