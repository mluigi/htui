# Blueprint: MOD-9 milestone 3, "skills are editable and bindable"

**Status**: proposed. Written against the plan the maintainer confirmed 2026-09-27.
**Plan**: `.claude/plans/mod-9-skills-editable.plan.md`. Its D70–D89, R-25–R35, T1–T5, OQ-14
(overridden: the matcher is hand-written) and OQ-15 (overridden: the previous attempt's changed
paths are in scope, via the `Isolator` seam) are authoritative **except where §0 amends them**.
**PRD**: `.claude/prds/mod-9-skill-library-templates.prd.md` milestone 3. **ANA**:
`docs/ANA-22.md` §6, §7, §8, §9. **Milestone 2's blueprint**:
`.claude/plans/mod-9-skills-reach-the-run.blueprint.md`.

**Verified at**: HEAD `df99f62` on branch `mod-9-m3` — the plan document on top of `68c058f`, so
the source tree is `68c058f`'s and the plan's `file:line` citations hold. `crates/htui-store/.sqlx/`
holds **268** files. `crates/htui/tests/snapshots/` holds 88 files, six of them `templates__*.snap`.
**Line numbers are pre-edit.**

**Gortex**: `graphify-out/` does not exist. Every fact below was read with the Gortex MCP tools
(`read`, `search`, `relations`), not with `grep`/`Read`. `Cargo.lock` is not indexed and no fact
here needed it.

**Scope**:

- **Order**: T1 → T2 → T3 → T4 → T5, all serial. T3 and T4 run serially for the reason the plan
  names (R-30): both need a `mod` line in `skills/mod.rs`, and a `mod matrix;` committed before
  `matrix.rs` exists does not compile.
- **One migration**, `0008_skill_match.sql`, which changes **no column and no constraint** and only
  re-issues the `run_step.trim_record` comment. `TABLES` stays 39, the commented-column total stays
  **34**, the applied list becomes 1..=8, `Pending(8)`.
- **Four new `WriteStore` methods** (79 → 83) and **three new inherent reads** on `MemStore` /
  `PgStore` dispatched by `Backend` (D92). **No new `ReadStore` method**: `READ_CASES` stays 14 and
  `MIRRORED_TABLES` stays 21.
- **Four new `StoreRequest` variants** (69 → 73), **two new `StoreReply` variants** (40 → 42), one
  new `GraphSource` method (T5) with four delegations, two new UI modules, one new hand-written
  glob module, and one new field on `NewStepGraph` (9 construction sites).
- **`.sqlx`**: 268 → 268+11 in T1 (D110), 268+11 in T2 and T4, 268+11 in T5 — the `create_step_graph`
  file is **replaced**, not added, so T5's net is 0.
- **Dependencies**: none. `git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml` is a gate
  in every task (D70).

**House style** (carried from milestones 1 and 2): `unsafe_code = "forbid"`; `missing_docs` on lib
roots; `missing_debug_implementations` and `unused_qualifications` warn; clippy `-D warnings`;
rustdoc denies broken/private intra-doc links; `rustfmt.toml` `max_width = 100`. Every new `pub`
item has a doc comment and a `Debug`. A body crossing the store boundary goes in a newtype whose
`Debug` prints its length. Commit the red tests first, then green; **every commit compiles** (red
commits use `todo!()`). No test is loosened; a moved pin names its reason in the assertion message.
Implementers stage their own paths only.

---

## 0. Findings the plan's fact-check missed

A finding marked **Blocker** means the plan, read literally, does not compile or fails its own named
test. **Major** means a named test or pin is wrong, or a design consequence the maintainer has not
seen. **Minor** is a citation, a spelling or a placement. The Fix column is what the implementer
builds; every Fix that amends a plan decision is flagged **(amends Dnn)** and is listed in §11.

| # | Severity | Plan says | Tree at `df99f62` | Fix |
|---|---|---|---|---|
| **F-1** | **Blocker** (T1 does not compile) | T2's file list owns `crates/htui-core/src/prompt/glob.rs`. T1's file list does not mention it. D78 requires `set_skill_binding` to refuse "a glob `glob::compile` rejects", and T1 lands before T2. | `glob.rs` does not exist; `crate::prompt` has no such module. `prompt/glob.rs` would depend only on `RepoPath` (`excerpt.rs:102`) and `BoundSkill` (`model/skill.rs:161`), both of which exist. | **D95**: `prompt/glob.rs` — the module, its `pub mod glob;` line in `prompt/mod.rs`, and its **full** dialect test table — lands as **commit 1 of T1**, not in T2. T2 keeps `matched_skills` and the wiring. §2.1 is the whole module. |
| **F-2** | **Major** (the matrix would draw one project) | D86: the attachment read is `skill_attachments(project, phase)`, inherent on the stores and dispatched by `Backend`. | `store_worker.rs`'s own `Catalogue(Scope)` doc says "One request per event, never one per project — the staleness index keeps only the newest of a variant, so N requests of one variant would leave all but one project undrawn." The matrix needs the global row **above the projects and the phases** of the whole scope. | **D92 (amends D86)**: two reads. `Backend::skill_attachments(&[ProjectId]) -> Vec<SkillAttachmentRow>` for the matrix, and `Backend::phase_attachments(project, phase) -> Vec<SkillBinding>` for the clone (F-7). One request, one `StoreRequest::Skills` variant, the whole scope in one reply. |
| **F-3** | **Major** (E0252 in two files) | D81's `SkillsSnapshot { skills, attachments: Vec<SkillBindingRow> }`. | `crates/htui-store/src/pg/rows.rs:257` already declares `pub(crate) struct SkillBindingRow`, imported by `pg/read.rs` (`:1432`) and `pg/write.rs`. A second public `SkillBindingRow` in `htui-core::model::skill` is an ambiguous import in both. | **D93 + D94 (amends D81's spelling)**: the model's row type is `SkillAttachmentRow`, in `model/skill.rs`; `pg/rows.rs`'s milestone-2 `SkillBindingRow` is untouched. |
| **F-4** | Minor | D77 and T1's list: `crates/htui-core/src/model/kind.rs` (the `NewSkill*` structs, beside `NewPromptTemplate`). | `model/kind.rs` is ANA-2's kind/graph/template module (`ItemKind`, `StepGraph`, `PromptTemplate`, `NewItemKind` :95, `NewStepGraph` :152, `NewPromptTemplate` :340). `model/skill.rs` holds `Skill`, `SkillVersion`, `SkillBinding`, `BoundSkill`, `ChoiceReason`, `SkillChoice`, `resolve`, `select`. `NewSkill` in `kind.rs` would need `model/skill.rs` to `use crate::model::kind::NewSkill` and would not be re-exported by `model::skill`. | **D90 (amends the file list, not D77's intent)**: the three structs live in `model/skill.rs`, re-exported from `model` beside `Skill`/`SkillVersion`/`SkillBinding`. D77's "newtype-free structs beside `NewPromptTemplate`" reads as a shape description, which is honoured. |
| **F-5** | **Major** (two records collapse into one) | D73: "`PromptSpec` gains `pub skill_files: Vec<RepoPath>` … and `pub skill_matches: BTreeMap<SkillId, String>`"; and `select` takes `matches: Option<&BTreeMap<SkillId, String>>` — "`None` means 'no file set resolved'". | `PromptSpec` derives `Debug, Clone, PartialEq` (`prompt/mod.rs:71-73`) and **no `Default`**, so every one of its literals is a hard error when it grows a field. A bare `BTreeMap` on the spec cannot express the `None` that `select` is given. | **D97 (amends D73's type)**: `pub skill_matches: Option<BTreeMap<SkillId, String>>`. `Some` at the two places that resolve a file set (`Engine::with_excerpts`, `preview::build`), `None` at the judge, the handoff and every fixture. The `no_path` / `no_match` split survives. §3.4 lists every literal that must grow both fields. |
| **F-6** | Minor | D71: "`{a,b}` alternates and nesting is refused". It does not say whether `{…}` may sit *inside* a component. | The plan's own test table only uses `{a,b}/x.rs` (whole component) and `x{,.txt}` (an empty alternate, refused either way). | **D96**: `{…}` is legal **only as a whole component**. The compiled form is then one level deep (`Segment::Alt(Vec<Vec<Segment>>)`) and the matcher is three functions. `pre{a,b}fix.rs` is refused, and refusing it is cheaper than owning a two-level alternation forever. |
| **F-7** | **Major** (T5's file list is short three files) | T5's list: `graph.rs`, `mem.rs`, `pg/write.rs`, `.sqlx`, `engine.rs`. D85 copies each source phase's attachments. | `override_graph<S: WriteStore, G: GraphSource>` (`graph.rs:399`) has **no read**; `ReadStore` is the *mirrored* trait and `skill*` is not mirrored (D86). `BoundSkill` carries no `languages`, so `G::bound_skills` cannot supply the copy verbatim (D85 requires `languages` copied). | **D98 (amends T5's file list)**: a new `GraphSource::phase_attachments(project, phase) -> Vec<SkillBinding>`, inherent on the stores and dispatched by `Backend` — **all landed in T1** (they are pure reads) — with the four delegations (`graph.rs::TestSource`, `fake.rs::MemStore`, `fake.rs::FakeGraphSource`, **`run_worker.rs::BackendGraphs`**) added in T5. `run_worker.rs` is in the `htui` crate and the plan's T5 list does not name it. |
| **F-8** | Minor | T1's Validate: "Regenerate `.sqlx` against a scratch database migrated from zero to `0007`". T5's Validate: `cargo sqlx prepare --check`, "the `create_step_graph` `INSERT` gains a column, so its `.sqlx` file is replaced". | T2's migration lands `0008` between the two, so the scratch database T5 checks against must be at `0008`, not `0007`. | **D99**: T1 prepares against a scratch DB migrated to `0007`. T2 adds `0008` (a comment only, so no query hash moves), migrates the same scratch DB forward and runs `sqlx prepare --check` without re-preparing. T5 re-prepares against the same DB, now at `0008`. |
| **F-9** | Minor (the two halves contradict) | D78: "the writer is the only place a bad pattern is refused, and it is where the maintainer sees the message". T4's test `a_glob_the_matcher_cannot_compile_is_refused_before_it_is_sent`: "`src/**x/*.rs`, `a/{b,{c,d}}/x.rs` and `x{,.txt}` are all `Constraint` on both stores". | The two hold together only if both sites render the **same** string. | **D100**: one `GlobError` type, one `Display`; the view's notice is `err.to_string()` and the writer's `StoreError::Constraint` carries that same string. T1's case 4 pins the writer; T4's test pins the view. |
| **F-10** | Minor | T4 test `the_language_map_expands_into_the_effective_globs_shown_before_the_save`: "`rust` → its two patterns". | ANA-22 §9's list names fourteen languages; a `rust` entry with two patterns has no evident second extension. | §5.2 gives `rust` one pattern (`**/*.rs`) and the test keeps its **shape** using `shell` (`.sh`, `.bash`, `.zsh`): a three-pattern language, unioned with a typed glob, in the union's order. |
| **F-11** | Minor (a silent store disagreement) | T1's four conformance cases write skills. | Every case runs against a store seeded by `fixtures::skill_bindings()` (`fixtures.rs:574-608`) with skills `tests` and `rust-style`. `skill.name` is unique across the table, so a case that upserts either name **moves the demo row**, and `a_preview_style_bound_skills_read_collapses_overrides` (`mem.rs:5966-6010`) plus `pg_criteria.rs:1806` compare tuples built from those rows. | H-2: every case uses a fresh name (`house`, `mod9-upsert`, `mod9-version`, `mod9-binding`, `mod9-refuse`). The seed's own names are never written. |
| **F-12** | Minor | D82 keeps `wants_requests` "at `:75`". | It currently returns one `StoreRequest::Templates`. T1 adds a second variant. | The tab returns **two** requests of two different variants, which the staleness index keeps separately. The catalogue's rule (F-2) is about N of *one* variant. §4.3 gives the code; `the_tab_asks_for_templates_and_skills` pins it. |
| **F-13** | Minor (record) | — | `keymap.rs::default_global` binds exactly `q`, `Tab`, `Shift+Tab`, `1`..`9`, `?`, and `Esc` for overlays. `w` is documented as T6's and is **not** bound. | Every browse key T3 and T4 claim is free, and `w` is avoided. The claim is restated in each view's `on_browse_key` doc, as `templates.rs:471` does, and a test in `library.rs` asserts the collision set. |

### 0a. Settled answers to the brief's questions

| Question | Answer | Where |
|---|---|---|
| Where do the three `New*` structs live, and do they carry ids? | `model/skill.rs` (D90), and **all three carry their id, minted client-side** (D91). That is the house rule of `NewItemKind` (`kind.rs:95-98`: "`item_kind.id`, minted client-side as a UUIDv7"), `NewRepo` (`hierarchy.rs:150`) and `NewPromptTemplate` (`kind.rs:342`). `NewSkillVersion` has no id — its key is `(skill_id, version)` and the store assigns the version, exactly as `append_prompt_template` does. | §2.2 |
| What exactly does `set_skill_binding` refuse, and in what order? | D78's five, in D78's order, all in Rust before the statement: a `phase_id` without a `project_id`; `activation = glob` with empty `globs`; a repo-qualified glob on a global row; a glob `glob::compile` rejects, with the position; a `pinned_version` the skill has no version for. The **token** is read first, so a spent token answers `Stale` before any of them (D18's order). | §2.3, §2.5 |
| What separates the *rule* refusals from the *reference* refusals? | The rules are `skill_binding_refusal` and `skill_pin_refusal` in `traits.rs`, called by **both** stores, so both answer in one sentence. The references (`skill_id`, `project_id`, `phase_id` naming no row) live only in `MemStore`, because on Postgres they are the FKs' `23503` and `map_sqlx` already turns them into `StoreError::Constraint` — the asymmetry `append_prompt_template` already has. | §2.3, §2.5 |
| Why is `{…}` restricted to a whole component? | D96/F-6: it makes the compiled form one level deep. Nothing in the plan's table needs intra-component alternation, and the two nested/empty cases the plan *does* name are refused either way. | §3.1 |
| How does `matched_skills` order its answer? | Files in the order given — the walk's `listed`, then any changed path not already in it (D89) — and the **first** match wins. The recorded value is `<repo>:<path>`. Determinism, not preference, is the requirement: the value lands in `trim_record.skill_choices[].matched`. | §3.1 |
| Where is `ExcerptSet.listed` captured? | Inside `select`, immediately after `let considered = …;` (`excerpt.rs:1166`) and **before** `vetted`, `fill_lexical_heads(&mut listing, …)` and `rank` — H-8. | §3.2 |
| Who answers `changed_paths`, and when? | The engine, once per step, in `with_excerpts`, and **only when some candidate is `Activation::Glob`** (R-35's mitigation). Attempt 1 asks for none. A failure records a note and continues with the walk's listing alone. | §3.5, §3.6 |
| What is the second CAS token in a skill save? | `skill.updated_at` guards the description and the head version guards the append; they are two different tables and two different surfaces, so `SaveSkill` carries both (D101). The worker short-circuits on the first `Stale`. | §2.9 |
| Does a demo digest move? | No. Every demo attachment is `Always` with empty `globs`, pinned by `demo_skill_rows_use_the_column_defaults` (`fixtures.rs:2024-2044`), so no demo skill is `Glob`, so no demo `PromptSpec` gets a non-`None` `skill_matches` and no rendered skill section changes. `prompt_digest.rs`'s 13 top-level keys and `v == 2` do not move. `IMPL_TRIM_RECORD` does: its one choice gains `"matched": null`. | §3.4, §8 |
| How many `.sqlx` files does T1 add? | **11**: four writers, three per-row re-reads (`skill`, `skill_binding`, `skill_head`), three library reads (the `skill` rows, the `skill_version` rows, the attachment join) and one `phase_attachments`. The implementer records the real number in the commit; a different decomposition is fine, a silent drift is not. | §2.11 |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (min., each compiles) | Gate |
|---|---|---|---|
| T1 storage + matcher + worker seam | htui-core, htui-store, htui-agent, htui | 5 (§2.12) | core; store (Postgres); `htui --lib templates skills`; `sqlx prepare --check`; `.sqlx` = 268+11; `cargo build --workspace --all-features --all-targets`; clippy; `git diff --exit-code Cargo.lock` |
| T2 matcher wiring + record + isolator | htui-core, htui-orch, htui, htui-store (migrations) | 4 (§3.10) | core; orch; htui (Postgres); `htui-store --test migrations`; `sqlx prepare --check` |
| T3 the Skills library view | htui | 2 (§4.7) | htui (Postgres), `--test skills` and `--test templates`; `cargo insta review` |
| T4 the attachments matrix | htui, htui-core | 2 (§5.7) | htui (Postgres), `--test skills_matrix`; `cargo insta review` |
| T5 the clone gap | htui-core, htui-store, htui-orch, htui | 3 (§6.6) | core; store (Postgres); orch; htui; `sqlx prepare --check`, `.sqlx` = 268+11 |
| merge | — | in task order | after each, the touched crates' gates on the real tree; then the workspace gate (§9) |

Environment for every gate (this box):

```bash
export PG=postgres://postgres:htui@localhost:5432
export PGT="USERNAME=htui-ci HTUI_TEST_DATABASE_URL=$PG/postgres"
export ORT_LIB_LOCATION=/home/user/ort/package/bin/napi-v3/linux/x64   # R-34: parcel.pyke.io 403
export LD_LIBRARY_PATH=$ORT_LIB_LOCATION
export CARGO_TARGET_DIR=/home/user/htui/target                          # one target dir (R-33)
export PREPARE=$PG/htui_prepare_m3      # the scratch database of §2.11
# `pg_isready -h localhost -p 5432` must say "accepting connections"
df -h /                                                            # R-33 before every wave
```

The keyring fake is process-wide, so the suite's green is scheduling-dependent: run the workspace
with `--test-threads=2` and any keyring-touching target single-threaded. None of T3/T4's new tests
touch the keyring.

---

## 2. T1: the four writers, the matcher module and the worker seam

**Files**. The plan's T1 list, **plus** F-1, F-2, F-3, F-4, F-7:

- plan's: `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`,
  `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/src/model/kind.rs` **(dropped,
  F-4/D90)**, `crates/htui-core/src/model/skill.rs`,
  `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`,
  `crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/writer.rs`,
  `crates/htui-store/src/backend.rs`, `crates/htui-store/.sqlx/` (+11),
  `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/skill_attachments.rs`,
  `crates/htui-store/tests/skill_binding_cas.rs` (new), `crates/htui-agent/src/conformance.rs`,
  `crates/htui-agent/tests/recorder.rs`, `crates/htui/src/skills.rs` (new),
  `crates/htui/src/store_worker.rs`.
- **added by F-1/D95**: `crates/htui-core/src/prompt/glob.rs` (new), `crates/htui-core/src/prompt/mod.rs`
  (the `pub mod glob;` line only).
- **added by F-7/D98**: `crates/htui-orch/src/isolate.rs` is **not** touched in T1 — only the
  inherent reads on the two stores and their `Backend` dispatches, which T1 already owns. T5 adds
  the trait method and the delegations.

`crates/htui-core/tests/mem_store.rs` and `crates/htui-agent/tests/recorder.rs` are named by the
plan; `htui-core/tests/mem_store.rs` holds `READ_CASES` 14 and the store `CASES` runner over
`MemStore`, and it needs no change beyond `EXPECTED_CASES` living in `pg_conformance.rs` — the plan
lists it, so the implementer checks it and leaves it alone if it needs nothing.

**First failing test**:
`cargo test -p htui-core --all-features --lib prompt::glob` (F-1's table), then
`cargo test -p htui-core --all-features --lib store::conformance::skill_`.

### 2.1 `crates/htui-core/src/prompt/glob.rs` (new — F-1, D95, D96, D103)

The whole matcher, landed with T1 because D78's writer needs `compile`. It depends on `RepoPath`
(`excerpt.rs:102-107`) and `BoundSkill` (`model/skill.rs:161-192`) and nothing else.

```rust
//! The glob dialect of `skill_binding.globs` (ANA-22 §6 item 7; plan D70, D71).
//!
//! **We own this syntax.** No crate was taken (D70: the maintainer's OQ-14 answer, 2026-09-27), and
//! the price is stated once: a pattern this module does not implement is **refused at save time**
//! (D78), never silently matched against nothing. `compile` is total — it answers a [`Pattern`] or
//! a [`GlobError`] carrying the byte of the mistake — and the writer refuses exactly the strings
//! `compile` does, so the two can never disagree about the same text (D100).
//!
//! The dialect, over repo-relative `/`-separated paths, case-sensitive everywhere:
//!
//! | Form | Meaning |
//! |---|---|
//! | `?` | one character, never `/` |
//! | `*` | zero or more characters, **never crossing `/`** — so `*.rs` matches `main.rs` and not `crates/main.rs` |
//! | `**` | zero or more **whole components**; legal only as a whole component (`**/`, `/**`, `/**/`) |
//! | `{a,b}` | alternation, **only as a whole component**; no nesting, no empty alternative (D96) |
//! | `[abc]`, `[!abc]`, `[a-z]` | a character class; `!` negates and never admits `/` |
//! | `\` | escapes the next metacharacter |
//! | `<repo>:` | a qualifier: the pattern matches in that repo only; a bare glob matches in every repo |
//!
//! A leading `!` is **not** negation and is refused. A trailing `/` and a leading `/` are refused.
//! Matching is a pure function of the compiled pattern and the path: no filesystem, no clock, no
//! reader, no `Default`.

use std::collections::BTreeMap;

use crate::model::ids::SkillId;
use crate::model::skill::BoundSkill;
use crate::prompt::excerpt::RepoPath;

/// One `/`-separated component of a compiled pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// A whole component compared by bytes.
    Literal(String),
    /// `?`, `*`, `[…]` and escaped literals, matched per character. `*` never crosses `/`.
    Parts(Vec<Tok>),
    /// `**`: zero or more whole components, matched only as a whole component of the pattern.
    AnyComponents,
    /// `{a,b}`: one of several whole segment sequences. Alternatives are non-empty and unnested.
    Alt(Vec<Vec<Segment>>),
}

/// One part of a component pattern. `*` is the only part that consumes more than one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    /// A literal character, after `\` unescaping.
    Char(char),
    /// `?`.
    Any,
    /// `*`.
    Star,
    /// `[abc]`, `[!abc]`, `[a-z]`. Membership never admits `/`.
    Class {
        /// `true` for `[!…]`.
        negated: bool,
        /// The ranges, inclusive, in the order written.
        ranges: Vec<(char, char)>,
    },
}

/// A compiled glob: the repo it is qualified to (or `None`) and its components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// The `<repo>:` qualifier, or `None` for a bare glob.
    repo: Option<String>,
    /// The components, in order.
    segments: Vec<Segment>,
}

/// Why `compile` refused a pattern. Every variant carries the byte offset of the mistake, as
/// [`TemplateError`](crate::prompt::TemplateError)'s `Display` does, so the activation form can put
/// the cursor on it and the writer can name the position in its `Constraint` (D100).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GlobError {
    /// The pattern was the empty string.
    #[error("a glob must not be empty")]
    Empty,
    /// A leading `!`, which is not negation here.
    #[error("`!` is not negation in a skill glob, at byte {at}")]
    Negation {
        /// The offset of the `!`.
        at: usize,
    },
    /// A trailing `/`, which would name a directory and never match a file.
    #[error("a glob must not end with `/`, at byte {at}")]
    TrailingSlash {
        /// The offset of the `/`.
        at: usize,
    },
    /// A leading `/`, which is not repo-relative.
    #[error("a glob must be repo-relative, so it must not start with `/`, at byte {at}")]
    Absolute {
        /// The offset of the `/`.
        at: usize,
    },
    /// A `**` that is not a whole component, e.g. `src/**x/*.rs`.
    #[error("`**` is only a whole component, at byte {at}")]
    MisplacedGlobstar {
        /// The offset of the first `*` of the run.
        at: usize,
    },
    /// A `{` inside an alternative, e.g. `a/{b,{c,d}}`.
    #[error("nested `{…}` is not supported, at byte {at}")]
    NestedAlternation {
        /// The offset of the inner `{`.
        at: usize,
    },
    /// An alternative with no text, e.g. `x{,.txt}`.
    #[error("an alternative must not be empty, at byte {at}")]
    EmptyAlternative {
        /// The offset of the `,` or the `}` that closed the empty alternative.
        at: usize,
    },
    /// A `{` that is not a whole component, e.g. `pre{a,b}fix.rs`.
    #[error("`{…}` is only a whole component, at byte {at}")]
    MisplacedBrace {
        /// The offset of the `{`.
        at: usize,
    },
    /// A `{` with no `}` before the end.
    #[error("unterminated `{{` at byte {at}")]
    UnterminatedBrace {
        /// The offset of the `{`.
        at: usize,
    },
    /// A `[` with no `]` before the end.
    #[error("unterminated `[` at byte {at}")]
    UnterminatedClass {
        /// The offset of the `[`.
        at: usize,
    },
    /// `[]` or `[^]`.
    #[error("an empty character class at byte {at}")]
    EmptyClass {
        /// The offset just past the `[`.
        at: usize,
    },
    /// A trailing `\`, or a `\` before an ordinary character.
    #[error("`\\` must escape a metacharacter, at byte {at}")]
    DanglingEscape {
        /// The offset of the `\`.
        at: usize,
    },
    /// A `U+0000`, which Postgres `text` cannot hold (`22021`).
    #[error("a glob must not contain NUL, at byte {at}")]
    Nul {
        /// The offset of the `U+0000`.
        at: usize,
    },
    /// A `<repo>:` qualifier that is empty or holds a `/`.
    #[error("a repo qualifier must be a non-empty name without `/`, at byte {at}")]
    BadQualifier {
        /// The offset of the `:`.
        at: usize,
    },
}

impl GlobError {
    /// Where the mistake is, for the cursor: the byte every variant but [`GlobError::Empty`] names.
    /// `None` is only `Empty`'s, whose mistake is the pattern as a whole (the cursor then goes to
    /// 0, as `templates.rs`'s `error_at` does for `MissingRequired`).
    #[must_use]
    pub const fn at(&self) -> Option<usize> {
        match self {
            Self::Empty => None,
            Self::Negation { at }
            | Self::TrailingSlash { at }
            | Self::Absolute { at }
            | Self::MisplacedGlobstar { at }
            | Self::NestedAlternation { at }
            | Self::EmptyAlternative { at }
            | Self::MisplacedBrace { at }
            | Self::UnterminatedBrace { at }
            | Self::UnterminatedClass { at }
            | Self::EmptyClass { at }
            | Self::DanglingEscape { at }
            | Self::Nul { at }
            | Self::BadQualifier { at } => Some(*at),
        }
    }
}

impl Pattern {
    /// The `<repo>:` qualifier, or `None` for a bare glob. The writer reads this to refuse a
    /// qualified glob on a **global** attachment (D78, ANA-22 §6 item 6), which no CHECK can hold.
    #[must_use]
    pub fn repo(&self) -> Option<&str> {
        self.repo.as_deref()
    }

    /// Whether `path`, repo-relative and `/`-separated, is matched. `None` of the repo is not read
    /// here: use [`Pattern::matches_path`] for a listed file, which is what the callers hold.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        let parts: Vec<&str> = path.split('/').collect();
        segments_match(&self.segments, &parts)
    }

    /// Whether `file`, the `(repo, path)` pair the walk produced, is matched. A qualified pattern
    /// matches only its own repo; a bare one matches in every repo.
    #[must_use]
    pub fn matches_path(&self, file: &RepoPath) -> bool {
        if let Some(repo) = &self.repo
            && *repo != file.repo
        {
            return false;
        }
        self.matches(&file.path)
    }
}

/// Compiles `pattern` under the dialect this module's doc writes down.
///
/// # Errors
///
/// [`GlobError`], always with a byte offset except `Empty`. Never a panic, never a silent
/// "matches nothing": this function is the only place the syntax is decided, and the writer
/// refuses exactly the strings it refuses (D78, D100).
pub fn compile(pattern: &str) -> Result<Pattern, GlobError> {
    // 1. NUL, before anything reads a byte: `text` cannot hold it.
    if let Some(at) = pattern.find('\0') {
        return Err(GlobError::Nul { at });
    }
    if pattern.is_empty() {
        return Err(GlobError::Empty);
    }
    // 2. The qualifier: everything before the first `/`, when that prefix holds a `:`. A `:` after
    //    a `/` is a literal character of the path, which `is_repo_relative` still admits.
    let (repo, body) = match pattern.find('/') {
        Some(slash) if pattern[..slash].contains(':') => {
            let (repo, _colon) = pattern[..slash].split_once(':').expect("the prefix holds a `:`");
            if repo.is_empty() {
                return Err(GlobError::BadQualifier { at: slash });
            }
            (Some(repo.to_owned()), &pattern[slash..])
        }
        _ => (None, pattern),
    };
    // 3. The shape rules on the whole body, before any component is parsed, so a leading `!` is
    //    `Negation` and not a stray character.
    if let Some(at) = body.strip_prefix('!').map(|_| 0) {
        return Err(GlobError::Negation { at });
    }
    if body.ends_with('/') {
        return Err(GlobError::TrailingSlash {
            at: pattern.len() - 1,
        });
    }
    if body.starts_with('/') {
        return Err(GlobError::Absolute { at: pattern.find('/').unwrap_or(0) });
    }
    // 4. The components.
    let mut segments = Vec::new();
    for (index, part) in body.split('/').enumerate() {
        let at = offset_of(body, index);
        segments.push(component(part, at)?);
    }
    Ok(Pattern { repo, segments })
}

/// The byte offset of the `index`-th `/`-separated component of `body`, as `offset_of`'s own test
/// pins. `body` has no leading `/` by the time this is called.
fn offset_of(body: &str, index: usize) -> usize {
    body.split('/')
        .take(index)
        .map(|part| part.len() + 1)
        .sum()
}

/// One `/`-separated component, at `at` bytes into the whole body.
fn component(text: &str, at: usize) -> Result<Segment, GlobError> {
    if text == "**" {
        return Ok(Segment::AnyComponents);
    }
    if text.contains("**") {
        return Err(GlobError::MisplacedGlobstar {
            at: at + text.find("**").expect("the component holds a `**`"),
        });
    }
    if let Some(open) = text.find('{') {
        if open != 0 || !text.ends_with('}') {
            return Err(GlobError::MisplacedBrace { at: at + open });
        }
        let inner = &text[1..text.len() - 1];
        let mut alternatives = Vec::new();
        for (n, alternative) in inner.split(',').enumerate() {
            if alternative.is_empty() {
                return Err(GlobError::EmptyAlternative {
                    at: at + 1 + inner.split(',').take(n).map(|s| s.len() + 1).sum::<usize>(),
                });
            }
            if alternative.contains('{') {
                return Err(GlobError::NestedAlternation {
                    at: at + 1 + inner.find('{').expect("the alternative holds a `{`"),
                });
            }
            // An alternative is itself a `/`-free segment sequence: one `Segment`, or the globstar.
            alternatives.push(vec![component(alternative, at + 1)?]);
        }
        if alternatives.len() < 2 {
            return Err(GlobError::MisplacedBrace { at: at + open });
        }
        return Ok(Segment::Alt(alternatives));
    }
    if text.contains('}') {
        return Err(GlobError::UnterminatedBrace {
            at: at + text.find('}').expect("the component holds a `}`"),
        });
    }
    let mut parts = Vec::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    while let Some((offset, c)) = chars.next() {
        match c {
            '*' => parts.push(Tok::Star),
            '?' => parts.push(Tok::Any),
            '[' => {
                let (class, used) = class(&text[offset..], at + offset)?;
                parts.push(class);
                for _ in 1..used {
                    chars.next();
                }
            }
            '\\' => match chars.next() {
                Some((_, escaped)) => parts.push(Tok::Char(escaped)),
                None => return Err(GlobError::DanglingEscape { at: at + offset }),
            },
            other => parts.push(Tok::Char(other)),
        }
    }
    // A component of literals is compared by bytes, which is both cheaper and the order the walk's
    // own listing is sorted in.
    if parts.iter().all(|part| matches!(part, Tok::Char(_))) {
        return Ok(Segment::Literal(
            parts
                .iter()
                .map(|part| match part {
                    Tok::Char(c) => *c,
                    _ => unreachable!("the segment is all `Tok::Char`"),
                })
                .collect(),
        ));
    }
    Ok(Segment::Parts(parts))
}

/// The class at the head of `text`, and how many bytes of it it used.
fn class(text: &str, at: usize) -> Result<(Tok, usize), GlobError> {
    let mut chars = text.char_indices().skip(1);
    let mut negated = false;
    let mut ranges: Vec<(char, char)> = Vec::new();
    let mut used = 1;
    let mut previous: Option<char> = None;
    loop {
        let Some((offset, c)) = chars.next() else {
            return Err(GlobError::UnterminatedClass { at: at + offset_of_class(text, 0) });
        };
        used = offset + c.len_utf8();
        match c {
            ']' if previous.is_none() => {
                // `[]` and `[^]`: an empty set, which matches nothing and is refused rather than
                // left to mean "no file ever".
                return Err(GlobError::EmptyClass { at: at + 1 });
            }
            ']' => return Ok((Tok::Class { negated, ranges }, used)),
            '!' if previous.is_none() => {
                negated = true;
                used = offset + 1;
            }
            '-' if let Some(low) = previous
                && let Some((_, high)) = chars.peek().copied()
                && high != ']'
            =>
            {
                ranges.push((low, high));
                used = high.len_utf8() + 1;
                chars.next();
                previous = None;
            }
            other => {
                ranges.push((other, other));
                previous = Some(other);
            }
        }
    }
}

/// The offset of the `index`-th component inside a class body; only the unterminated case uses it.
fn offset_of_class(_text: &str, _index: usize) -> usize {
    0
}

/// The components of a path against the pattern's. `**` is the only backtracking source and it is
/// a whole component, so the search is bounded by the number of components; the compiler's
/// exponential form is not reachable from a pattern a maintainer can type.
fn segments_match(segments: &[Segment], parts: &[&str]) -> bool {
    let Some((first, rest)) = segments.split_first() else {
        return parts.is_empty();
    };
    match first {
        Segment::AnyComponents => {
            // Zero components first, then one: `**/*.rs` prefers the shallowest match, and the
            // recorded path is the walk's first.
            segments_match(rest, parts) || (!parts.is_empty() && segments_match(segments, &parts[1..]))
        }
        Segment::Literal(literal) => {
            parts.first() == Some(&literal.as_str()) && segments_match(rest, &parts[1..])
        }
        Segment::Parts(tokens) => {
            parts.first().is_some_and(|part| parts_match(tokens, part))
                && segments_match(rest, &parts[1..])
        }
        Segment::Alt(alternatives) => alternatives
            .iter()
            .any(|alternative| segments_match(alternative, parts)),
    }
}

/// `tokens` against one component: the two-pointer matcher, with `*` the only backtracking token.
fn parts_match(tokens: &[Tok], part: &str) -> bool {
    let chars: Vec<char> = part.chars().collect();
    let (mut t, mut p) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while p < chars.len() {
        match tokens.get(t) {
            Some(Tok::Star) => {
                star = Some((t, p));
                t += 1;
            }
            Some(Tok::Any) if chars[p] != '/' => {
                t += 1;
                p += 1;
            }
            Some(Tok::Class { negated, ranges }) if chars[p] != '/' => {
                let inside = ranges.iter().any(|(low, high)| chars[p] >= *low && chars[p] <= *high);
                if inside != *negated {
                    t += 1;
                    p += 1;
                } else if let Some((star_at, mark)) = star {
                    t = star_at + 1;
                    p = mark + 1;
                    star = Some((star_at, mark + 1));
                } else {
                    return false;
                }
            }
            Some(Tok::Char(want)) if *want == chars[p] => {
                t += 1;
                p += 1;
            }
            _ => {
                let Some((star_at, mark)) = star else {
                    return false;
                };
                t = star_at + 1;
                p = mark + 1;
                star = Some((star_at, mark + 1));
            }
        }
    }
    tokens[t..].iter().all(|token| *token == Tok::Star)
}

/// One matched path per skill that fired, keyed by `skill_id`; the value is `<repo>:<path>` (D71).
///
/// `files` is read in the order given — [`crate::prompt::excerpt::ExcerptSet::listed`] first, then
/// the previous attempt's changed paths (D89) — and the **first** match wins, so the recorded path
/// is the walk's first. A glob `compile` refuses is **skipped**, never an error: the writer
/// refuses it at save time (D78), so a stored glob always compiles and the two can never disagree
/// about the same string (D71).
#[must_use]
pub fn matched_skills(skills: &[BoundSkill], files: &[RepoPath]) -> BTreeMap<SkillId, String> {
    let mut matched = BTreeMap::new();
    for skill in skills {
        if skill.activation != crate::model::skill::Activation::Glob {
            continue;
        }
        let Some(path) = skill.globs.iter().find_map(|glob| {
            let pattern = compile(glob).ok()?;
            files
                .iter()
                .find(|file| pattern.matches_path(file))
                .map(|file| format!("{}:{}", file.repo, file.path))
        }) else {
            continue;
        };
        matched.insert(skill.skill_id, path);
    }
    matched
}
```

`prompt/mod.rs` gains, among the `pub mod` lines:

```rust
pub mod glob;
```

#### 2.1.1 The dialect test table (the specification, not a cross-implementation check)

In `glob.rs`'s `mod tests`. Each row is one assertion pair; every `*` row is exercised against both
a match and a near-miss.

| Pattern | Matches | Does **not** match | Why |
|---|---|---|---|
| `**/*.rs` | `crates/x.rs`, `x.rs` | `crates/x.py` | `**` is zero or more whole components |
| `*.rs` | `main.rs` | `crates/main.rs` | `*` never crosses `/` |
| `src/*.rs` | `src/main.rs` | `crates/x.rs` | a literal component |
| `src/**` | `src/a.rs`, `src/a/b.rs`, `src` | `src2/x.rs` | `**` as a whole component |
| `**/src` | `src`, `a/src`, `a/b/src` | `a/src2` | ditto |
| `a/**/b` | `a/b`, `a/x/b`, `a/x/y/b` | `a/b/c` | zero components in the middle |
| `src/?.rs` | `src/a.rs` | `src/ab.rs`, `src/.rs` | `?` is exactly one character |
| `src/a*.rs` | `src/a.rs`, `src/abc.rs` | `src/b.rs` | `*` inside a component |
| `src/{a,b}/x.rs` | `src/a/x.rs`, `src/b/x.rs` | `src/c/x.rs` | alternation, whole component (D96) |
| `[abc].rs` | `a.rs`, `b.rs`, `c.rs` | `d.rs` | a class admits `/` never: `[/].rs` matches nothing |
| `[!abc].rs` | `d.rs` | `a.rs` | `!` inside a class negates |
| `[a-c].rs` | `a.rs`, `b.rs`, `c.rs` | `d.rs` | a range |
| `htui:**/*.rs` | `htui:crates/x.rs` | `agy:crates/x.rs` | the qualifier (via `matches_path`) |
| `**/*.rs` (bare) | any repo's `x.rs` | — | a bare glob is every repo |
| `crates/ax.rs` | — | `crates/abc.rs` | a literal is compared by bytes, not by prefix |
| `*.rs` and `**/*.rs` together | — | — | **case-sensitive**: `X.RS` matches neither |

Refusals — each `compile` is an `Err` whose `at` is the byte named, and `at()` returns it:

| Pattern | Variant | `at` | Why |
|---|---|---|---|
| `""` | `Empty` | `None` | the pattern is the empty string |
| `!*.rs` | `Negation` | 0 | a leading `!` is not negation |
| `src/` | `TrailingSlash` | 3 | a directory never matches a file |
| `/src/*.rs` | `Absolute` | 0 | not repo-relative |
| `src/**x/*.rs` | `MisplacedGlobstar` | 4 | a `**` that is not a whole component |
| `a**/x.rs` | `MisplacedGlobstar` | 1 | ditto |
| `**x` | `MisplacedGlobstar` | 0 | ditto |
| `a/{b,{c,d}}/x.rs` | `NestedAlternation` | 8 | nesting is refused |
| `x{,.txt}` | `EmptyAlternative` | 3 | an alternative must not be empty |
| `x{a,}` | `EmptyAlternative` | 5 | ditto |
| `{,a}` | `EmptyAlternative` | 2 | ditto |
| `pre{a,b}fix.rs` | `MisplacedBrace` | 3 | `{…}` is only a whole component (D96) |
| `{a,b` | `MisplacedBrace` | 0 | not a whole component (no closing `}`) |
| `a/{b` | `UnterminatedBrace` | 2 | the `{` is never closed |
| `a/b}` | `UnterminatedBrace` | 3 | a `}` with no `{` |
| `src/[a.rs` | `UnterminatedClass` | 4 | the `[` is never closed |
| `[]` | `EmptyClass` | 1 | an empty class matches nothing and is refused |
| `src/\` | `DanglingEscape` | 4 | a trailing `\` |
| `src/\x.rs` | `DanglingEscape` | 4 | `\` before an ordinary character is a refusal, not a literal `x` |
| `a\0b` | `Nul` | 1 | Postgres `text` cannot hold it |
| `:/x.rs` | `BadQualifier` | 0 | an empty qualifier |
| `htui/a:/x.rs` | `BadQualifier` | 9 | a `/` before the `:` makes the `:` a path character, and the leading `/` is then `Absolute` |

Plus, in the same `mod tests`:

- `escapes_bind_the_next_character`: `src/a\\.rs` matches `src/a.rs` and not `src/ab.rs`.
- `the_display_names_the_byte`: one assertion per `GlobError` variant, pinning the exact
  `to_string()` of §2.1 and its `at()`, as `model/skill.rs`'s `choices_serialize_their_documented_keys`
  pins its own vocabulary.
- `matched_skills_skips_a_glob_compile_refuses` (T2's test, landing with the function in T2).
- `no_panic_on_any_short_input`: a table of the 32 one- and two-byte strings, none of which may
  panic (R-25's "compile is total").

### 2.2 `crates/htui-core/src/model/skill.rs` — the argument structs, the name rule, the read rows (D90–D94)

Module doc: `model/skill.rs:1-6` — "Read-only still: the writers are MOD-9 milestone 3's" becomes
"**Read-only in milestone 2 only.** Since MOD-9 milestone 3 a skill is written by
[`crate::store::WriteStore::upsert_skill`], versioned by `add_skill_version` and attached by
`set_skill_binding` / `remove_skill_binding`; [`resolve`] still picks the most specific attachment
and [`select`] still decides, per step, which winners render and records why."

Placed after `SkillBinding` and before `BoundSkill`, so the file reads row, row, row, arguments,
candidate:

```rust
/// Arguments of [`crate::store::WriteStore::upsert_skill`]: the library row, whose key is `name`
/// and whose token is `updated_at`.
///
/// `id` is minted client-side, as [`NewPromptTemplate`](crate::model::NewPromptTemplate)'s is
/// (D91): a create carries the id the caller will later name, and an update ignores it, because the
/// key the editor holds is `name` and not the id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSkill {
    /// `skill.id`, minted client-side as a UUIDv7.
    pub id: SkillId,
    /// `skill.name`, the library key: `[a-z0-9-]`, 1–64, no leading, trailing or double hyphen
    /// (ANA-22 §6 item 12). Checked by the writer, not by a constraint.
    pub name: String,
    /// `skill.description`; the library list's one-liner, never rendered into a prompt.
    pub description: String,
    /// `skill.created_by`; the worker fills it from `Backend::this_user` (`R-NF-3`), so a view never
    /// holds a `UserId`.
    pub created_by: UserId,
}

/// Arguments of [`crate::store::WriteStore::add_skill_version`]: one immutable body, appended as
/// `expected + 1`.
///
/// No `id`: the key is `(skill_id, version)` and the store assigns the version, exactly as
/// `append_prompt_template` does with `(project_id, name, version)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSkillVersion {
    /// `skill_version.skill_id`, which must name a row.
    pub skill_id: SkillId,
    /// `skill_version.body`, inlined into the prompt's skills section verbatim. A NUL is refused
    /// before the statement: `parse` is not involved, but `text` cannot hold one.
    pub body: String,
    /// `skill_version.source` (ANA-22 §6 item 11): import provenance and the raw frontmatter.
    /// `{}` for a body authored in the Skills view, which is every body until milestone 4.
    pub source: serde_json::Value,
    /// `skill_version.created_by`; the worker fills it.
    pub created_by: UserId,
}

/// Arguments of [`crate::store::WriteStore::set_skill_binding`]: one attachment, upserted on
/// `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSkillBinding {
    /// `skill_binding.id`, minted client-side; the upsert keeps the existing row's id, so this is
    /// used only by a create.
    pub id: SkillBindingId,
    /// `skill_binding.skill_id`.
    pub skill_id: SkillId,
    /// `skill_binding.project_id`; `None` is a global attachment (every project).
    pub project_id: Option<ProjectId>,
    /// `skill_binding.phase_id`; `Some` requires `project_id` (D78).
    pub phase_id: Option<PhaseId>,
    /// `skill_binding.pinned_version`; `None` follows the latest version.
    pub pinned_version: Option<i32>,
    /// `skill_binding.position`: ascending render order, `skill.name` bytes breaking the tie.
    pub position: i32,
    /// `skill_binding.activation`.
    pub activation: Activation,
    /// `skill_binding.globs`: the **effective** globs — typed, plus every named language's
    /// expansion (D83). The matcher reads only this; `languages` is display only.
    pub globs: Vec<String>,
    /// `skill_binding.languages`: as authored, display only.
    pub languages: Vec<String>,
}

/// One `skill` row with every one of its `skill_version` rows: what
/// [`MemStore::skill_library`](crate::store::MemStore::skill_library) and its siblings answer, so
/// the view never reads a table twice for one list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    /// The `skill` row.
    pub skill: Skill,
    /// Every version, ascending; the last is the head.
    pub versions: Vec<SkillVersion>,
}

/// One `skill_binding` row with its `skill.name`, its project's slug and its phase's name joined:
/// what the **matrix** draws (D92). Deliberately *not* the name `SkillBindingRow`: `pg/rows.rs:257`
/// already owns that for `model::skill::resolve` (F-3, D94).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillAttachmentRow {
    /// `skill_binding.id`, the token `remove_skill_binding` names.
    pub id: SkillBindingId,
    /// `skill_binding.skill_id`.
    pub skill_id: SkillId,
    /// `skill.name`, the matrix's row label.
    pub name: String,
    /// `skill_binding.project_id`; `None` is the global row.
    pub project_id: Option<ProjectId>,
    /// The project's slug, for the project row's label; `None` on the global row.
    pub project_slug: Option<String>,
    /// `skill_binding.phase_id`; `None` is the project (or global) row.
    pub phase_id: Option<PhaseId>,
    /// The phase's name, for the phase row's label.
    pub phase_name: Option<String>,
    /// `skill_binding.pinned_version`.
    pub pinned_version: Option<i32>,
    /// `skill_binding.position`.
    pub position: i32,
    /// `skill_binding.activation`.
    pub activation: Activation,
    /// `skill_binding.globs`.
    pub globs: Vec<String>,
    /// `skill_binding.languages`.
    pub languages: Vec<String>,
    /// `skill_binding.updated_at`, the CAS token both writers take.
    pub updated_at: DateTime<Utc>,
}
```

In `impl Skill`, after `is_valid` has no neighbour today — place it right after the struct:

```rust
    /// ANA-22 §6 item 12's Agent Skills rule, checked by the writer and not by a constraint:
    /// `[a-z0-9-]`, 1–64, no leading or trailing hyphen and no double hyphen. One sentence on both
    /// stores, for [`PromptTemplate::name_is_valid`]'s reason and
    /// [`ItemKind::prefix_is_valid`](crate::model::ItemKind::prefix_is_valid)'s.
    ///
    /// The length test counts **bytes**, like the column `prefix_is_valid` mirrors, and every
    /// admitted character is one byte, so the two agree without a second rule.
    #[must_use]
    pub fn name_is_valid(name: &str) -> bool {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > 64 {
            return false;
        }
        if !bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        {
            return false;
        }
        !name.starts_with('-') && !name.ends_with('-') && !name.contains("--")
    }
```

`mod tests` gains, in the house style of `choices_serialize_their_documented_keys`:

- `a_skill_name_follows_the_agent_skills_rule` — over `[a-z0-9-]`: `a`, `a-b`, `0`, `-`… all the
  refusals: `""`, `A`, `a_b`, `a.b`, `"-a"`, `"a-"`, `"a--b"`, `"a b"`, the 65-`a` string, and a
  64-`a` string accepted. The message names ANA-22 §6 item 12.
- `the_argument_structs_reach_the_model` — `serde_json::to_value(NewSkill{…})` has exactly
  `id, name, description, created_by`; the other two likewise, so the record's shape is pinned.

`crates/htui-core/src/model/mod.rs`:

- `pub use skill::{` gains `NewSkill`, `NewSkillBinding`, `NewSkillVersion`, `SkillAttachmentRow`,
  `SkillEntry` in name order.
- `model/mod.rs` gains **no** `mod` line: `skill` already exists at `:95`.
- T4 adds `pub mod language;` between `kind` (:87) and `link` (:88) — that is T4's, not T1's.

### 2.3 `crates/htui-core/src/store/traits.rs` — the four methods and the refusals

Placed after `append_prompt_template` (`:743`) and before the `// settings (D7, D8)` comment at
`:745`, so the three prompt writers sit together. The module doc's "…Nothing is added to these two
traits in MOD-1" paragraph gains one sentence: "**MOD-9 milestone 3** adds four [`WriteStore`]
methods for the skill tables — `upsert_skill`, `add_skill_version`, `set_skill_binding`,
`remove_skill_binding` — and nothing on [`ReadStore`]: `skill*` is not mirrored, so the library and
the attachments are inherent on [`MemStore`](crate::store::MemStore) / `PgStore` and dispatched by
`Backend` (plan D86, blueprint D92)."

```rust
    /// A `skill` row created or edited, `name` being the key and `updated_at` the token.
    ///
    /// `expected` is the `updated_at` the editor opened on. `None` is a **token**, not "don't
    /// care": it means "I expect no row", so it is `Stale` once the name has one and `NotFound`
    /// is never returned for a spent token on a row that exists.
    ///
    /// The name never moves — it is the library key the editor, the render order and the unique
    /// index all name — so an edit writes `description` and nothing else.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) when `expected` is `Some` and
    /// the name has no row; [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a
    /// name [`Skill::name_is_valid`] refuses, a NUL in either field, an `id` that is taken, or a
    /// `created_by` that names no `app_user`. Nothing is written by any of them.
    async fn upsert_skill(
        &self,
        new: NewSkill,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Skill>>;

    /// Append version `expected + 1` of `new.skill_id` iff `expected` is that skill's head version;
    /// `None` is a skill with no version yet and starts at 1.
    ///
    /// `skill_version` carries **no** `updated_at` trigger (`0001_init.sql` §5.1 names it as
    /// deliberately absent), so this never moves `skill.updated_at` and the two tokens of a
    /// `SaveSkill` are independent surfaces (blueprint D101).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) when `expected` is `Some` and
    /// the skill has no version at all (`Some(0)` included — the F-B regression the template case
    /// pins); [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a body with a
    /// NUL, a `skill_id` or a `created_by` that names no row. Nothing is written.
    async fn add_skill_version(
        &self,
        new: NewSkillVersion,
        expected: Option<i32>,
    ) -> Result<CasOutcome<SkillVersion>>;

    /// The attachment of `(skill_id, project_id, phase_id)`, inserted or replaced, under the
    /// `updated_at` token.
    ///
    /// The upsert is the key's, not a blind insert: `UNIQUE NULLS NOT DISTINCT (skill_id,
    /// project_id, phase_id)` is one attachment per skill per level, and replacing it must not mint
    /// a second row. `expected` is `Some(row.updated_at)` for a replace and `None` for a create.
    ///
    /// Refused **in Rust, before the statement**, in this order (D78): a `phase_id` without a
    /// `project_id`; `activation = glob` with empty `globs`; a repo-qualified glob on a **global**
    /// row; a glob [`glob::compile`](crate::prompt::glob::compile) refuses, with its byte; a
    /// `pinned_version` the skill has no version for. The Postgres CHECKs are `23514` and absent
    /// on `MemStore`, so the Rust refusal is what makes the two stores agree; the last two have no
    /// CHECK at all and are purely the writer's (blueprint D100).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) when `expected` is `Some` and
    /// the key has no row; [`StoreError::Constraint`](crate::store::StoreError::Constraint) for
    /// any of the five refusals, or a `skill_id`, `project_id` or `phase_id` that names no row
    /// (Postgres: the FKs' `23503`).
    async fn set_skill_binding(
        &self,
        new: NewSkillBinding,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<SkillBinding>>;

    /// Detach one row, under its own `updated_at` token.
    ///
    /// A writer rather than `activation = off` (OQ-19): a spent token is `Stale` and the row
    /// survives, so unbind is as safe as every other write, and the row really goes rather than
    /// reading as an attachment that is neither.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an `id` no row has.
    async fn remove_skill_binding(
        &self,
        id: SkillBindingId,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<SkillBinding>>;
```

The refusal helpers, in the block beside `prompt_template_refusal` (`:1492`) — same order, same
doc shape, same `#[must_use]`:

```rust
/// MOD-9 D77: a skill name [`Skill::name_is_valid`] refuses, in the sentence both stores give it.
#[must_use]
pub fn invalid_skill_name(name: &str) -> String {
    format!(
        "skill.name `{}` must be 1-64 characters of [a-z0-9-] with no leading, trailing or double \
         hyphen",
        name.escape_debug()
    )
}

/// MOD-9 D77: the `NotFound` id of a `skill` row, so both stores spell it alike.
#[must_use]
pub fn skill_key(name: &str) -> String {
    name.to_owned()
}

/// MOD-9 D77: the `NotFound` id of a `(skill, version)` pair, so both stores spell it alike.
#[must_use]
pub fn skill_version_key(skill: SkillId, version: i32) -> String {
    format!("{skill}/v{version}")
}

/// MOD-9 D77: the `NotFound` id of a `skill_binding` key, so both stores spell it alike.
#[must_use]
pub fn skill_binding_key(
    skill: SkillId,
    project: Option<ProjectId>,
    phase: Option<PhaseId>,
) -> String {
    format!(
        "{skill}/{}/{}",
        project.map_or_else(|| "global".to_owned(), |id| id.to_string()),
        phase.map_or_else(|| "project".to_owned(), |id| id.to_string())
    )
}

/// MOD-9 D77 / D78: why a skill may not be written, or `None` when it may. The name first, then
/// the description's NUL, which `name_is_valid` cannot catch because a NUL is not a name character
/// and `text` cannot hold one (`22021`).
#[must_use]
pub fn skill_refusal(name: &str, description: &str) -> Option<String> {
    if !Skill::name_is_valid(name) {
        return Some(invalid_skill_name(name));
    }
    if description.contains('\0') {
        return Some("skill.description must not contain a NUL character".to_owned());
    }
    None
}

/// MOD-9 D78: the four rules of `set_skill_binding` that need no read — the fifth, the pin, is
/// [`skill_pin_refusal`] because it needs the skill's versions.
///
/// Split so `PgStore` pays for the version read only when a pin is actually set, exactly as
/// `append_prompt_template` pays for its head read only when there is a refusal to classify.
#[must_use]
pub fn skill_binding_refusal(new: &NewSkillBinding) -> Option<String> {
    if new.phase_id.is_some() && new.project_id.is_none() {
        return Some(
            "skill_binding.phase_id needs a project_id: a phase attachment is the project's \
             (ANA-22 §6 item 2)"
                .to_owned(),
        );
    }
    if new.activation == Activation::Glob && new.globs.is_empty() {
        return Some(
            "skill_binding.globs must name at least one glob when activation is `glob` \
             (skill_binding_glob_needs_globs)"
                .to_owned(),
        );
    }
    for glob in &new.globs {
        let pattern = match crate::prompt::glob::compile(glob) {
            Ok(pattern) => pattern,
            // D100: the writer's sentence is the `GlobError`'s own, so the activation form and the
            // store say the same thing about the same bytes.
            Err(err) => return Some(format!("skill_binding.globs `{glob}`: {err}")),
        };
        if pattern.repo().is_some() && new.project_id.is_none() {
            return Some(format!(
                "skill_binding.globs `{glob}` names a repo, and a global attachment applies to \
                 every project (ANA-22 §6 item 6)"
            ));
        }
    }
    None
}

/// MOD-9 D78: a `pinned_version` the skill has no version for, in one sentence for both stores.
#[must_use]
pub fn skill_pin_refusal(
    pinned: i32,
    skill: SkillId,
    versions: &[SkillVersion],
) -> Option<String> {
    if versions
        .iter()
        .any(|row| row.skill_id == skill && row.version == pinned)
    {
        return None;
    }
    Some(format!(
        "skill_binding.pinned_version {pinned} names no version of skill `{skill}`"
    ))
}
```

`traits.rs` unit tests (its own `mod tests`, beside the template ones):

- `a_skill_name_follows_the_agent_skills_rule` — the table of §2.2.
- `skill_refusal_prefers_the_name_over_the_nul`: `skill_refusal("Bad Name\0", "ok\0")` is
  `invalid_skill_name`'s sentence, and `skill_refusal("ok", "a\0b")` is the description's, with the
  reason in both messages.
- `skill_binding_refusal_is_d78_s_five_in_order`: the four read-free rules, one assertion each,
  with the position in the glob case (`"at byte 4"` in `skill_binding.globs \`src/**x/*.rs\``).

### 2.4 `crates/htui-core/src/store/mem.rs` — the `State` methods and the three reads

`State`'s three collections keep their shapes (D79, already true: `skills: HashMap<SkillId, Skill>`
at `:115`-ish, `skill_versions: Vec<SkillVersion>` at `:118`, `skill_bindings: Vec<SkillBinding>`).
`project_reach` and `delete_project` do not move (`:3317`'s `retain(|row| row.project_id != Some(id))`
already keeps the globals, and `DeleteReach.skill_bindings` still counts the project's own three).

The four `State` methods, in the module's own order — `append_prompt_template` is at `:2715`, so
they follow it:

```rust
    /// MOD-9 D76: `skill.name` is the key and `skill.updated_at` the token; token, then input, then
    /// keys, the order of `append_prompt_template` (D18), so a spent token answers `Stale` before
    /// the name is judged and `None` on a name that now has a row is `Stale` too.
    fn upsert_skill(
        &mut self,
        new: NewSkill,
        expected: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Skill>> {
        let current = self
            .skills
            .values()
            .find(|row| row.name == new.name)
            .cloned();
        match &current {
            // `expected != Some(row.updated_at)` covers both spent tokens and `None` on a row that
            // exists: `None` is "I expect no row", not "don't care".
            Some(row) if expected != Some(row.updated_at) => {
                return Ok(CasOutcome::Stale(row.clone()));
            }
            None if expected.is_some() => {
                return Err(StoreError::NotFound {
                    entity: "skill",
                    id: skill_key(&new.name),
                });
            }
            _ => {}
        }
        if let Some(refusal) = skill_refusal(&new.name, &new.description) {
            return Err(StoreError::Constraint(refusal));
        }
        self.require_user(new.created_by, "skill.created_by")?;
        if self.skills.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists("skill", new.id)));
        }
        let row = match current {
            Some(mut row) => {
                // The name is the key and never moves; only the description and the token do.
                row.description = new.description;
                row.updated_at = now;
                self.skills.insert(row.id, row.clone());
                row
            }
            None => {
                let row = Skill {
                    id: new.id,
                    name: new.name,
                    description: new.description,
                    created_by: new.created_by,
                    created_at: now,
                    updated_at: now,
                };
                self.skills.insert(row.id, row.clone());
                row
            }
        };
        Ok(CasOutcome::Applied(row))
    }

    /// MOD-9 D76: the head version is the token, the shape of `append_prompt_template` with
    /// `(skill_id, version)` in place of `(project_id, name, version)`.
    fn add_skill_version(
        &mut self,
        new: NewSkillVersion,
        expected: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<SkillVersion>> {
        let head = self
            .skill_versions
            .iter()
            .filter(|row| row.skill_id == new.skill_id)
            .max_by_key(|row| row.version)
            .cloned();
        match (&head, expected) {
            (Some(head), _) if Some(head.version) != expected => {
                return Ok(CasOutcome::Stale(head.clone()));
            }
            (None, Some(token)) => {
                return Err(StoreError::NotFound {
                    entity: "skill_version",
                    id: skill_version_key(new.skill_id, token),
                });
            }
            _ => {}
        }
        if new.body.contains('\0') {
            return Err(StoreError::Constraint(
                "skill_version.body must not contain a NUL character".to_owned(),
            ));
        }
        if !self.skills.contains_key(&new.skill_id) {
            return Err(StoreError::Constraint(references_no_row(
                "skill_version.skill_id",
                new.skill_id,
                "skill",
            )));
        }
        self.require_user(new.created_by, "skill_version.created_by")?;
        let row = SkillVersion {
            skill_id: new.skill_id,
            version: expected.unwrap_or(0) + 1,
            body: new.body,
            source: new.source,
            created_by: new.created_by,
            created_at: now,
        };
        self.skill_versions.push(row.clone());
        Ok(CasOutcome::Applied(row))
    }

    /// MOD-9 D76/D78: the token, then the five rules, then the references. The token comes first
    /// so a spent token answers `Stale` and never a refusal (D18).
    fn set_skill_binding(
        &mut self,
        new: NewSkillBinding,
        expected: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<SkillBinding>> {
        let current = self
            .skill_bindings
            .iter()
            .find(|row| {
                row.skill_id == new.skill_id
                    && row.project_id == new.project_id
                    && row.phase_id == new.phase_id
            })
            .cloned();
        match &current {
            Some(row) if expected != Some(row.updated_at) => {
                return Ok(CasOutcome::Stale(row.clone()));
            }
            None if expected.is_some() => {
                return Err(StoreError::NotFound {
                    entity: "skill_binding",
                    id: skill_binding_key(new.skill_id, new.project_id, new.phase_id),
                });
            }
            _ => {}
        }
        if !self.skills.contains_key(&new.skill_id) {
            return Err(StoreError::Constraint(references_no_row(
                "skill_binding.skill_id",
                new.skill_id,
                "skill",
            )));
        }
        if let Some(refusal) = skill_binding_refusal(&new) {
            return Err(StoreError::Constraint(refusal));
        }
        if let Some(pinned) = new.pinned_version
            && let Some(refusal) = skill_pin_refusal(pinned, new.skill_id, &self.skill_versions)
        {
            return Err(StoreError::Constraint(refusal));
        }
        // The FKs, which `PgStore` answers as `23503`. `phase_id`'s FK is to `step_graph_phase(id)`
        // alone, so "the phase belongs to this project" is **not** checked on either store: a row
        // may name another project's phase, and the engine's `(graph id, name)` lookup never reads
        // it. D78 mirrors the CHECKs; it does not extend them (H-30).
        if let Some(project_id) = new.project_id
            && !self.projects.contains_key(&project_id)
        {
            return Err(StoreError::Constraint(references_no_row(
                "skill_binding.project_id",
                project_id,
                "project",
            )));
        }
        if let Some(phase_id) = new.phase_id
            && !self.phases.contains_key(&phase_id)
        {
            return Err(StoreError::Constraint(references_no_row(
                "skill_binding.phase_id",
                phase_id,
                "step_graph_phase",
            )));
        }
        if current.is_none() && self.skill_bindings.iter().any(|row| row.id == new.id) {
            return Err(StoreError::Constraint(already_exists(
                "skill_binding",
                new.id,
            )));
        }
        let row = match current {
            Some(mut row) => {
                row.pinned_version = new.pinned_version;
                row.position = new.position;
                row.activation = new.activation;
                row.globs = new.globs;
                row.languages = new.languages;
                row.updated_at = now;
                self.skill_bindings[position_of(&self.skill_bindings, row.id)].clone_into(&mut row);
                row
            }
            None => {
                let row = SkillBinding {
                    id: new.id,
                    skill_id: new.skill_id,
                    project_id: new.project_id,
                    phase_id: new.phase_id,
                    pinned_version: new.pinned_version,
                    position: new.position,
                    activation: new.activation,
                    globs: new.globs,
                    languages: new.languages,
                    updated_at: now,
                };
                self.skill_bindings.push(row.clone());
                row
            }
        };
        Ok(CasOutcome::Applied(row))
    }

    /// MOD-9 D76: the token, then the removal. A spent token is `Stale` and the row survives
    /// (R-32), which is the whole reason this is a CAS and not a bare delete.
    fn remove_skill_binding(
        &mut self,
        id: SkillBindingId,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<SkillBinding>> {
        let index = self
            .skill_bindings
            .iter()
            .position(|row| row.id == id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "skill_binding",
                id: id.to_string(),
            })?;
        if self.skill_bindings[index].updated_at != expected {
            return Ok(CasOutcome::Stale(self.skill_bindings[index].clone()));
        }
        Ok(CasOutcome::Applied(self.skill_bindings.remove(index)))
    }
```

`position_of` is a three-line private helper beside them — `skill_bindings` is a `Vec` (D79), so a
replace needs the index, and this is it:

```rust
    /// The index of `id` in `rows`. Read a statement ago under the same lock, so it is `Some`.
    fn position_of(rows: &[SkillBinding], id: SkillBindingId) -> usize {
        rows.iter()
            .position(|row| row.id == id)
            .expect("the row was read a statement ago under the same lock")
    }
```

The four `impl WriteStore for MemStore` arms, after `append_prompt_template` (`:5544`):

```rust
    async fn upsert_skill(
        &self,
        new: NewSkill,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Skill>> {
        let now = Utc::now();
        self.write(|state| state.upsert_skill(new, expected, now))
    }

    async fn add_skill_version(
        &self,
        new: NewSkillVersion,
        expected: Option<i32>,
    ) -> Result<CasOutcome<SkillVersion>> {
        let now = Utc::now();
        self.write(|state| state.add_skill_version(new, expected, now))
    }

    async fn set_skill_binding(
        &self,
        new: NewSkillBinding,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<SkillBinding>> {
        let now = Utc::now();
        self.write(|state| state.set_skill_binding(new, expected, now))
    }

    async fn remove_skill_binding(
        &self,
        id: SkillBindingId,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<SkillBinding>> {
        self.write(|state| state.remove_skill_binding(id, expected))
    }
```

The three **inherent reads**, after `bound_skills` (`:449`), which is the method they mirror:

```rust
    /// MOD-9 D93: the whole library with every version, `skill.name` byte order and
    /// `skill_version.version` ascending. **Not** a scoped read: `skill` has no `project_id` and
    /// the library is global (D92), which is why the snapshot carries `Scope` and not a project.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn skill_library(&self) -> Result<Vec<SkillEntry>> {
        self.read(|state| {
            let mut skills: Vec<Skill> = state.skills.values().cloned().collect();
            skills.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
            skills
                .into_iter()
                .map(|skill| {
                    let mut versions: Vec<SkillVersion> = state
                        .skill_versions
                        .iter()
                        .filter(|row| row.skill_id == skill.id)
                        .cloned()
                        .collect();
                    versions.sort_by_key(|row| row.version);
                    SkillEntry { skill, versions }
                })
                .collect()
        })
    }

    /// MOD-9 D92/D93: every attachment of every listed project plus every global one, with the
    /// `skill.name`, the project's slug and the phase's name joined: the matrix's one read, and one
    /// request per event (never one per project — the staleness index keeps only the newest of a
    /// variant, F-2).
    ///
    /// The order is the matrix's: `project_id` NULLS FIRST, then `project_id`, then `phase_id`
    /// NULLS FIRST, then `phase_id`, then `skill.name` bytes.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s.
    pub async fn skill_attachments(
        &self,
        projects: &[ProjectId],
    ) -> Result<Vec<SkillAttachmentRow>> {
        self.read(|state| {
            let mut rows: Vec<SkillAttachmentRow> = state
                .skill_bindings
                .iter()
                .filter(|row| match row.project_id {
                    None => true,
                    Some(owner) => projects.contains(&owner),
                })
                .map(|row| {
                    let phase_name = row
                        .phase_id
                        .and_then(|id| state.phases.get(&id))
                        .map(|phase| phase.name.clone());
                    SkillAttachmentRow {
                        id: row.id,
                        skill_id: row.skill_id,
                        name: state
                            .skills
                            .get(&row.skill_id)
                            .map_or_else(String::new, |skill| skill.name.clone()),
                        project_id: row.project_id,
                        project_slug: row
                            .project_id
                            .and_then(|id| state.projects.get(&id))
                            .map(|project| project.slug.clone()),
                        phase_id: row.phase_id,
                        phase_name,
                        pinned_version: row.pinned_version,
                        position: row.position,
                        activation: row.activation,
                        globs: row.globs.clone(),
                        languages: row.languages.clone(),
                        updated_at: row.updated_at,
                    }
                })
                .collect();
            rows.sort_by(|a, b| {
                match (a.project_id, b.project_id) {
                    // NULLS FIRST, written out because `Option`'s own `Ord` is NULLS LAST and
                    // Postgres would order it the other way round.
                    (None, Some(_)) => core::cmp::Ordering::Less,
                    (Some(_), None) => core::cmp::Ordering::Greater,
                    (left, right) => left.cmp(&right),
                }
                .then_with(|| match (a.phase_id, b.phase_id) {
                    (None, Some(_)) => core::cmp::Ordering::Less,
                    (Some(_), None) => core::cmp::Ordering::Greater,
                    (left, right) => left.cmp(&right),
                })
                .then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
            });
            rows
        })
    }

    /// MOD-9 D98: the **raw** `skill_binding` rows of one phase, nothing resolved. This is the read
    /// `override_graph` copies from (D85): a [`BoundSkill`] would have lost `languages`, which the
    /// copy must carry verbatim.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s.
    pub async fn phase_attachments(
        &self,
        project: ProjectId,
        phase: PhaseId,
    ) -> Result<Vec<SkillBinding>> {
        self.read(|state| {
            let mut rows: Vec<SkillBinding> = state
                .skill_bindings
                .iter()
                .filter(|row| row.project_id == Some(project) && row.phase_id == Some(phase))
                .cloned()
                .collect();
            rows.sort_by(|a, b| a.name_of(&state.skills).cmp(&b.name_of(&state.skills)));
            rows
        })
    }
```

`SkillBinding::name_of` is **not** added: `phase_attachments` sorts by `(position, skill name)` in
Rust after the rows are gathered, which is the order `resolve` uses, and needs no method on the
model. The sort is therefore:

```rust
            rows.sort_by_key(|row| row.position);
```

`mem.rs`'s tests, in its `mod tests` (which already holds `demo_with_a_global_skill` at `:6122`):

- `a_global_attachment_survives_project_delete` — `delete_project(PROJECT_HTUI)` answers
  `skill_bindings == 3` and `bound_skills(PROJECT_AGY, None)` still holds `house`. **This must
  already pass**; T1's job is to keep it passing (H-31), and it is listed so a reader sees the pin
  is live.
- `the_three_skill_collections_keep_their_shapes` — after each of the four writes, `skills` is a
  map keyed by `SkillId` and the other two are `Vec`s whose `retain` by key is the only way to read
  one; the test writes and then finds the row, which is what a `HashMap` for either would not allow.
- `upsert_skill_creates_then_edits_and_answers_stale` — the `MemStore` half of conformance case 1,
  so a failure names the store. Not a second copy of the case: it asserts only the three `State`
  transitions the case cannot reach (`updated_at == created_at` on a create, a strictly later
  `updated_at` on an edit, the `NotFound` a `Some` token on a missing name gives).
- `a_refused_binding_wrote_nothing` — the five refusals, and `skill_bindings.len()` is unchanged
  after each.

### 2.5 `crates/htui-store/src/pg/write.rs` — the four statements

`PgStore`'s four arms follow `append_prompt_template` (`write.rs:2442-2502`, which ends at `:2502`,
before the `// settings (D7, D8)` comment). Each is the house shape: refuse in Rust first, one
statement, `RETURNING` with the alias list, then `cas_miss`.

```rust
    /// MOD-9 D76: `INSERT … SELECT … WHERE <the row's updated_at> IS NOT DISTINCT FROM $5 … ON
    /// CONFLICT (name) DO UPDATE … WHERE skill.updated_at = $5 RETURNING …`.
    ///
    /// `skill.created_at` and `updated_at` are both **supplied**, because the `updated_at` trigger
    /// is `BEFORE UPDATE` only (`0001_init.sql` §5.1) and an insert therefore reads whatever the
    /// statement wrote — the same reason `pg/demo.rs` binds `updated_at` on its `step_graph` insert.
    ///
    /// `ON CONFLICT (name)` infers the unique index over `skill.name` whatever its name is; the
    /// create path is `expected = NULL` (the `IS NOT DISTINCT FROM NULL` guard) and the edit path
    /// collides and updates, so a spent token reaches neither and falls through to `cas_miss`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for `Some` on a name with no row;
    /// [`StoreError::Constraint`] as the trait's. Nothing is written by either.
    async fn upsert_skill(
        &self,
        new: NewSkill,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Skill>> {
        let key = skill_key(&new.name);
        if let Some(refusal) = skill_refusal(&new.name, &new.description) {
            // No row can be named with a NUL, and binding one is `22021`, not an empty read.
            let current = if new.name.contains('\0') {
                None
            } else {
                self.skill(&new.name).await?
            };
            return match (current, expected) {
                (Some(current), _) if expected != Some(current.updated_at) => {
                    Ok(CasOutcome::Stale(current))
                }
                (None, Some(_)) => Err(StoreError::NotFound {
                    entity: "skill",
                    id: key,
                }),
                _ => Err(StoreError::Constraint(refusal)),
            };
        }

        let inserted = sqlx::query_as!(
            Skill,
            r#"
            INSERT INTO skill (id, name, description, created_by, created_at, updated_at)
            SELECT $1, $2, $3, $4, now(), now()
             WHERE (SELECT updated_at FROM skill WHERE name = $2)
                   IS NOT DISTINCT FROM $5::timestamptz
            ON CONFLICT (name) DO UPDATE SET description = EXCLUDED.description
             WHERE skill.updated_at = $5::timestamptz
            RETURNING id         AS "id: SkillId",
                      name,
                      description,
                      created_by AS "created_by: UserId",
                      created_at,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.name,
            new.description,
            new.created_by.as_uuid(),
            expected,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if let Some(row) = inserted {
            return Ok(CasOutcome::Applied(row));
        }
        cas_miss(self.skill(&new.name).await?, "skill", key)
    }

    /// MOD-9 D76: the `append_prompt_template` statement with `(skill_id, version)` for
    /// `(project_id, name, version)`; `source` is written as given, so the UI's `{}` reaches the
    /// column and not its default by accident.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for `Some` on a skill with no version; [`StoreError::Constraint`]
    /// for a NUL body or a `skill_id`/`created_by` that names no row (`23503`).
    async fn add_skill_version(
        &self,
        new: NewSkillVersion,
        expected: Option<i32>,
    ) -> Result<CasOutcome<SkillVersion>> {
        if new.body.contains('\0') {
            let head = self.skill_head(new.skill_id).await?;
            return match (head, expected) {
                (Some(head), _) if Some(head.version) != expected => Ok(CasOutcome::Stale(head)),
                (None, Some(token)) => Err(StoreError::NotFound {
                    entity: "skill_version",
                    id: skill_version_key(new.skill_id, token),
                }),
                _ => Err(StoreError::Constraint(
                    "skill_version.body must not contain a NUL character".to_owned(),
                )),
            };
        }

        let inserted = sqlx::query_as!(
            SkillVersion,
            r#"
            INSERT INTO skill_version (skill_id, version, body, source, created_by)
            SELECT $1, COALESCE($4::int, 0) + 1, $2, $3, $5
             WHERE (SELECT max(version) FROM skill_version WHERE skill_id = $1)
                   IS NOT DISTINCT FROM $4::int
            ON CONFLICT (skill_id, version) DO NOTHING
            RETURNING skill_id   AS "skill_id: SkillId",
                      version,
                      body,
                      source,
                      created_by AS "created_by: UserId",
                      created_at
            "#,
            new.skill_id.as_uuid(),
            new.body,
            new.source,
            expected,
            new.created_by.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if let Some(row) = inserted {
            return Ok(CasOutcome::Applied(row));
        }
        cas_miss(
            self.skill_head(new.skill_id).await?,
            "skill_version",
            skill_version_key(new.skill_id, expected.unwrap_or_default()),
        )
    }

    /// MOD-9 D76/D78: the five rules in Rust, then one statement. `ON CONFLICT (skill_id,
    /// project_id, phase_id)` infers the `UNIQUE NULLS NOT DISTINCT` table constraint by its column
    /// list — the inference is over columns and expressions only, and this constraint is not
    /// partial — which the plan's Verified-claims row checked against the Postgres 16
    /// `sql-insert.html` on 2026-09-27.
    ///
    /// The insert guard is the row's own `updated_at`, so a create (`None`) and a replace
    /// (`Some(t)`) both insert, both collide, and the `DO UPDATE`'s own `WHERE skill_binding.updated_at
    /// = $10` decides; a spent token inserts nothing at all and falls through to `cas_miss`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for `Some` on a key with no row; [`StoreError::Constraint`] for
    /// any of D78's five, or an FK that names no row (`23503`).
    async fn set_skill_binding(
        &self,
        new: NewSkillBinding,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<SkillBinding>> {
        let key = skill_binding_key(new.skill_id, new.project_id, new.phase_id);
        // The pin is the only rule that needs a read, so the read is only paid for a pin — the
        // split `skill_binding_refusal` / `skill_pin_refusal` exists for.
        let mut refusal = skill_binding_refusal(&new);
        if refusal.is_none()
            && let Some(pinned) = new.pinned_version
        {
            let versions = self.skill_versions(new.skill_id).await?;
            refusal = skill_pin_refusal(pinned, new.skill_id, &versions);
        }
        if let Some(refusal) = refusal {
            let current = self.attachment(new.skill_id, new.project_id, new.phase_id).await?;
            return match (current, expected) {
                (Some(current), _) if expected != Some(current.updated_at) => {
                    Ok(CasOutcome::Stale(current))
                }
                (None, Some(_)) => Err(StoreError::NotFound {
                    entity: "skill_binding",
                    id: key,
                }),
                _ => Err(StoreError::Constraint(refusal)),
            };
        }

        let inserted = sqlx::query_as!(
            SkillBinding,
            r#"
            INSERT INTO skill_binding (id, skill_id, project_id, phase_id, pinned_version,
                                       position, activation, globs, languages)
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9
             WHERE (SELECT updated_at FROM skill_binding
                     WHERE skill_id = $2 AND project_id IS NOT DISTINCT FROM $3::uuid
                       AND phase_id IS NOT DISTINCT FROM $4::uuid)
                   IS NOT DISTINCT FROM $10::timestamptz
            ON CONFLICT (skill_id, project_id, phase_id) DO UPDATE SET
                pinned_version = EXCLUDED.pinned_version,
                position       = EXCLUDED.position,
                activation     = EXCLUDED.activation,
                globs          = EXCLUDED.globs,
                languages      = EXCLUDED.languages
             WHERE skill_binding.updated_at = $10::timestamptz
            RETURNING id          AS "id: SkillBindingId",
                      skill_id    AS "skill_id: SkillId",
                      project_id  AS "project_id?: ProjectId",
                      phase_id    AS "phase_id: PhaseId",
                      pinned_version,
                      position,
                      activation  AS "activation: Activation",
                      globs,
                      languages,
                      updated_at
            "#,
            new.id.as_uuid(),
            new.skill_id.as_uuid(),
            new.project_id.map(ProjectId::as_uuid),
            new.phase_id.map(PhaseId::as_uuid),
            new.pinned_version,
            new.position,
            new.activation.as_str(),
            &new.globs[..],
            &new.languages[..],
            expected,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if let Some(row) = inserted {
            return Ok(CasOutcome::Applied(row));
        }
        cas_miss(
            self.attachment(new.skill_id, new.project_id, new.phase_id)
                .await?,
            "skill_binding",
            key,
        )
    }

    /// MOD-9 D76 / R-32: `DELETE … WHERE id = $1 AND updated_at = $2 RETURNING …`. A spent token
    /// deletes nothing and answers `Stale` with the winner's row, so a second unbind never removes
    /// the first one's result.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an `id` no row has.
    async fn remove_skill_binding(
        &self,
        id: SkillBindingId,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<SkillBinding>> {
        let deleted = sqlx::query_as!(
            SkillBinding,
            r#"
            DELETE FROM skill_binding
             WHERE id = $1 AND updated_at = $2
            RETURNING id          AS "id: SkillBindingId",
                      skill_id    AS "skill_id: SkillId",
                      project_id  AS "project_id?: ProjectId",
                      phase_id    AS "phase_id: PhaseId",
                      pinned_version,
                      position,
                      activation  AS "activation: Activation",
                      globs,
                      languages,
                      updated_at
            "#,
            id.as_uuid(),
            expected,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if let Some(row) = deleted {
            return Ok(CasOutcome::Applied(row));
        }
        cas_miss(self.skill_binding(id).await?, "skill_binding", id)
    }
```

`self.skill_versions(new.skill_id)` returns `Vec<SkillVersion>` — a **fourth** read helper. The plan
names three (`the per-row re-reads`); the pin check needs the whole version list, and reading only
the pinned number would answer a different question ("does *this* version exist") from the one the
refusal asks ("does the skill have that version"). `SELECT version FROM skill_version WHERE skill_id
= $1` is the small statement; the bodies are not needed and are not fetched. **H-11.**

`crates/htui-store/src/pg/read.rs` gains the six read helpers, after `bound_skills` (`:1432-1495`):

| Method | One statement | Answers |
|---|---|---|
| `skill(&self, name: &str) -> Result<Option<Skill>>` | `SELECT … FROM skill WHERE name = $1` | `upsert_skill`'s `cas_miss` |
| `skill_head(&self, skill: SkillId) -> Result<Option<SkillVersion>>` | `… ORDER BY version DESC LIMIT 1` | `add_skill_version`'s `cas_miss` |
| `skill_versions(&self, skill: SkillId) -> Result<Vec<SkillVersion>>` | `SELECT version … WHERE skill_id = $1` | the pin check (`skill_pin_refusal`) |
| `skill_binding(&self, id: SkillBindingId) -> Result<Option<SkillBinding>>` | `SELECT … WHERE id = $1` | `remove_skill_binding`'s `cas_miss` |
| `attachment(&self, skill, project, phase) -> Result<Option<SkillBinding>>` | `SELECT … WHERE skill_id = $1 AND project_id IS NOT DISTINCT FROM $2 AND phase_id IS NOT DISTINCT FROM $3` | `set_skill_binding`'s `cas_miss` |
| `skill_library(&self) -> Result<Vec<SkillEntry>>`, `skill_attachments(&self, projects: &[ProjectId]) -> Result<Vec<SkillAttachmentRow>>`, `phase_attachments(&self, project, phase) -> Result<Vec<SkillBinding>>` | §2.4's three | the snapshot, the matrix, the clone |

`attachment`'s `IS NOT DISTINCT FROM` is load-bearing and is the one place in the tree that gets it
wrong twice if it is copied: `project_id = $2` would make every **global** row compare to NULL and
never match, so a global attachment's `Stale` would answer `NotFound`. The re-read is a `SELECT`,
not the writer's guard, and both spell it the same way.

`crates/htui-store/src/backend.rs` gains three `pub async fn` after `bound_skills` (`:366-376`),
each dispatching `Memory` / `Online` and answering `Err(prompt_offline())` for `Offline` — the same
`PROMPT_ON_SERVER_ONLY` sentence, because `skill*` is not mirrored and D86 is the reason:

```rust
    pub async fn skill_library(&self) -> Result<Vec<SkillEntry>> { … }
    pub async fn skill_attachments(&self, projects: &[ProjectId]) -> Result<Vec<SkillAttachmentRow>> { … }
    pub async fn phase_attachments(&self, project: ProjectId, phase: PhaseId) -> Result<Vec<SkillBinding>> { … }
```

### 2.6 `Writer`, `UsageSpy`, `SpyStore` — the exhaustive arms

Each is a one-line delegation, placed after `append_prompt_template` in all three, exactly as
milestone 1 added `append_prompt_template` beside `set_agent_box_quota`:

```rust
    // crates/htui-store/src/writer.rs, after :714
    async fn upsert_skill(&self, new: NewSkill, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<Skill>> {
        match self {
            Self::Memory(store) => store.upsert_skill(new, expected).await,
            Self::Online(pg) => pg.upsert_skill(new, expected).await,
        }
    }
    // … add_skill_version, set_skill_binding, remove_skill_binding, same shape
```

```rust
    // crates/htui-agent/src/conformance.rs, after :942
    async fn upsert_skill(&self, new: NewSkill, expected: Option<DateTime<Utc>>) -> StoreResult<CasOutcome<Skill>> {
        self.inner.upsert_skill(new, expected).await
    }
    // … the other three
```

```rust
    // crates/htui-agent/tests/recorder.rs, after the template arm (:637)
    async fn upsert_skill(&self, new: NewSkill, expected: Option<DateTime<Utc>>) -> StoreResult<CasOutcome<Skill>> {
        self.inner.upsert_skill(new, expected).await
    }
    // … the other three
```

`Writer`'s doc says "the exhaustive `WriteStore` wrapper"; nothing else changes. Both spies forward
by value, so `NewStepGraph`'s new field (T5) never reaches them (F-7's evidence).

### 2.7 `crates/htui-core/src/store/conformance.rs` — the four cases

Registered by hand in **both** `CASES` and `run_case` (D80). Placed immediately after
`"prompt_template_refuses_what_parse_refuses"` in each, so the three prompt writers and the four
skill writers read as one block; the two `claim_run_*` cases and `infer_repo_box_path_…` follow. The
alternative — appending at the end — is also valid, since a case's identity is its name, and
`run_case_accepts_every_name_in_cases` (`conformance.rs:10722`) is what enforces the agreement.

**New helper constructors**, beside `new_template` (`:5717`):

```rust
/// A skill to write, with a fresh id, authored by the fixture user. H-2: the name must be one no
/// demo row holds, because `skill.name` is unique across the table and a case that moved
/// `tests` or `rust-style` would break `a_preview_style_bound_skills_read_collapses_overrides`
/// (`mem.rs:5966`) and `pg_criteria.rs:1806`.
fn new_skill(id: SkillId, name: &str, description: &str) -> NewSkill {
    NewSkill {
        id,
        name: name.to_owned(),
        description: description.to_owned(),
        created_by: ids::USER,
    }
}

/// One version to append, with a fresh body.
fn new_version(skill: SkillId, body: &str) -> NewSkillVersion {
    NewSkillVersion {
        skill_id: skill,
        body: body.to_owned(),
        source: serde_json::json!({}),
        created_by: ids::USER,
    }
}

/// One attachment to write, at a level.
fn new_binding(
    id: SkillBindingId,
    skill: SkillId,
    project: Option<ProjectId>,
    phase: Option<PhaseId>,
) -> NewSkillBinding {
    NewSkillBinding {
        id,
        skill_id: skill,
        project_id: project,
        phase_id: phase,
        pinned_version: None,
        position: 0,
        activation: Activation::Always,
        globs: Vec::new(),
        languages: Vec::new(),
    }
}
```

**Case 1 — `skill_upsert_creates_then_edits_under_the_updated_at_token`** (`:43-121` gets the
entry, `:132-269` the arm):

| Step | Call | Assert |
|---|---|---|
| 1 | `upsert_skill(new_skill(id, "house", "House rules."), None)` | `Applied`; `(name, description) == ("house", "House rules.")`; `created_by == ids::USER`; `updated_at == created_at` — the trigger is `BEFORE UPDATE` only, so an insert's two instants are the statement's `now()` |
| 2 | the same with `Some(row.updated_at)` and description `"House rules, revised."` | `Applied`; the description moved; `updated_at > row.updated_at` |
| 3 | the same with **step 1's** `updated_at` | `Stale` carrying the step-2 row, description `"House rules, revised."` |
| 4 | `upsert_skill(new_skill(SkillId::new(), "no-such", ""), Some(at()))` | `Err(NotFound { entity: "skill", .. })`, `id == "no-such"` |
| 5 | `upsert_skill(new_skill(new_id, "House", ""), None)` | `Constraint` naming `skill.name` — the Agent Skills rule |
| 6 | `upsert_skill(new_skill(new_id, "ok\0", ""), None)` and `("ok", "a\0b")` | `Constraint` naming `skill.name` and `skill.description` |
| 7 | `upsert_skill(new_skill(id_of_house_but_fresh, "other", ""), None)` — the demo `tests` id, a fresh name | `Constraint` from the `id` that is taken, i.e. `skill` `already exists` |
| 8 | `upsert_skill(new_skill(SkillId::new(), "house", "again"), Some(row.updated_at))` after step 2 | `Applied` — none of 4–7 wrote |

**Case 2 — `skill_version_append_is_a_cas_on_the_head`**: `house` from case 1's shape, then v2 at
`Some(1)`, a `Stale` from a spent `Some(1)`, v3 at `Some(2)`, `None` on a fresh skill starting at 1,
and `Some(0)`/`Some(7)` on a skill with no version both `NotFound` with
`id == "<skill>/v0"` / `"<skill>/v7"`. A NUL body is `Constraint` naming `skill_version.body`, and a
`created_by` that names no row is `Constraint` **under the head token**, so the head read precedes
it (the template case's "at the current head, so only the author can refuse it"). The tail: a good
append at the current head lands at v4.

**Case 3 — `skill_binding_upsert_replaces_its_own_row_and_a_spent_token_is_stale`**: the NULL
semantics are the whole test. `(house, None, None)` twice is **one** row with the **first** `id`; a
`Stale` from step 1's `updated_at` after a step-2 replace carries the replaced row; then
`(house, Some(PROJECT_HTUI), None)` and `(house, Some(PROJECT_HTUI), Some(PHASE_HTUI_IMPLEMENT))` are
two **more** rows, so three attachments of one skill coexist; `remove_skill_binding(project_row.id,
project_row.updated_at)` is `Applied` and the two others are untouched; a second remove with the same
token is `NotFound`. The tail: a `set_skill_binding` at the phase row's current `updated_at` lands.

**Case 4 — `skill_binding_refuses_what_its_checks_refuse`**, D78's order, each a
`StoreError::Constraint` whose message is asserted with `contains`:

| # | Input | Needle in the message |
|---|---|---|
| 1 | `phase_id: Some(…)`, `project_id: None` | `skill_binding.phase_id` |
| 2 | `activation: Glob`, `globs: []` | `skill_binding.globs` and `skill_binding_glob_needs_globs` |
| 3 | `activation: Glob`, `globs: ["htui:**/*.rs"]`, `project_id: None` | `skill_binding.globs` and `` `htui:**/*.rs` names a repo `` |
| 4 | `globs: ["src/**x/*.rs"]` | `**` and `` at byte 4 `` |
| 5 | `globs: ["a/{b,{c,d}}/x.rs"]` | `nested` and `at byte 8` |
| 6 | `globs: ["x{,.txt}"]` | `an alternative must not be empty` |
| 7 | `globs: ["pre{a,b}fix.rs"]` | `` `{…}` is only a whole component `` |
| 8 | `pinned_version: Some(9)` on a skill whose versions are 1..=1 | `skill_binding.pinned_version 9 names no version` |
| 9 | `skill_id: SkillId::new()` | `references no app_user`-shaped: `skill_binding.skill_id` |
| 10 | `project_id: ProjectId::new()` | `skill_binding.project_id` |
| 11 | after 1–4, a good `set_skill_binding` at the same token | `Applied` — **none of them wrote** |
| 12 | a spent token under a refused input | `Stale`, carrying the row — the token is read first (D18) |

The plan puts "a NUL name" in this case; it moves to **case 1**, because `NewSkillBinding` has no
name field and a `upsert_skill` refusal does not belong in a `set_skill_binding` case. **Minor
deviation, §11.**

`crates/htui-store/tests/pg_conformance.rs:19` becomes `const EXPECTED_CASES: usize = 81;`.

### 2.8 The Postgres-only cases

`crates/htui-store/tests/skill_binding_cas.rs` (new) is `prompt_template_cas.rs` in a new key: a
winner's transaction held open by hand inserts the second row, the loser's write blocks on the
unique index, `pg_stat_activity` shows a `Lock` wait, the commit releases it, and the loser answers
`Stale` with the winner's row. Two cases, both `#![cfg(feature = "demo")]` and both using unchecked
`sqlx::query`, so the file adds **nothing** to `.sqlx`:

- `two_bindings_at_one_key_write_one_row` — the `(skill, NULL, NULL)` upsert, the loser holding the
  stale `updated_at`.
- `an_unbind_under_a_spent_token_keeps_the_row` — R-32's race: a winner updates the row in a held
  transaction, the loser's `remove_skill_binding` with the old token is `Stale` and the row is
  there after the commit.

`crates/htui-store/tests/skill_attachments.rs` gains, over the planted rows the file already has
(`plant_skill`, `plant_binding`, `constraint`, `mem_with`, `plant`):

- `the_writer_refuses_a_glob_row_with_no_globs_before_the_check_does` — the same input through
  `store.set_skill_binding(..)` is `Constraint` with the writer's sentence, and through
  `plant_binding` is `23505`… no: through `plant_binding` it is `skill_binding_glob_needs_globs`.
  The point of the case is that the writer refused **first**, so the constraint name never appears
  in a writer-driven test.
- `the_writer_and_mem_agree_on_every_refusal` — the five D78 rules, each run on `db.store` and on
  `MemStore::from_demo` over the same edit, asserting the **same** message string from both.
- `the_check_names_stay_what_milestone_two_probed` — the four constraint names, unchanged, so a
  schema change that renames one fails here rather than silently in a writer test.
- `a_qualified_glob_is_accepted_on_a_project_row` — the positive half of D78's third rule.

### 2.9 `crates/htui/src/skills.rs` (new — the worker seam, D81, D101)

`crates/htui/src/lib.rs` gains, at **:28** (between `pub mod run_worker;` and
`pub mod store_worker;`):

```rust
pub mod skills;
```

The module, mirroring `crates/htui/src/templates.rs` whole:

```rust
//! The skill library and its attachments behind the Skills tab's Skills view (MOD-9 milestone 3,
//! D81): one read per scope, one save, three attachment writers, the [`crate::prompt_settings`]
//! shape.
//!
//! One read per event and never one per keystroke, one reply out, and every write through
//! [`WriteStore`]. The view renders from the snapshot and never patches a row into it. The
//! re-read-rather-than-patch rule, the refusal arm that names the request that got here by mistake,
//! and `request_names_match_the_name_arms` are `templates.rs`'s and are carried byte for byte.
//!
//! Known residue, the same one `templates.rs` and `crate::catalogue` carry: a re-read that fails
//! *after* an applied write answers `Failed`, so the view is told nothing happened when a version
//! has in fact been appended.
//!
//! The worker fills `created_by` from [`Backend::this_user`], mints the three ids and holds no
//! `UserId` (`R-NF-3`); nothing here reads the clock; the store stamps both instants.

use htui_core::model::{
    Activation, NewSkill, NewSkillBinding, NewSkillVersion, ProjectId, Scope, Skill,
    SkillAttachmentRow, SkillBindingId, SkillBinding, SkillVersion,
};
use htui_core::store::{CasOutcome, Result, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE};

use crate::store_worker::{StoreReply, StoreRequest};

/// The library and the scope's attachments, as the two views draw them.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillsSnapshot {
    /// Every skill in the library, `skill.name` byte order, each with every version ascending.
    pub skills: Vec<SkillSummary>,
    /// Every attachment that applies to the scope: the globals, the scope projects' and their
    /// phases', in the matrix's order (D92).
    pub attachments: Vec<SkillAttachmentRow>,
}

/// One library entry as the library list shows it. The `skill` row is flattened because the view
/// needs the name, the description and the token and never `created_by` or `created_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillSummary {
    /// `skill.id`, so the matrix and the form name a row without a second read.
    pub id: htui_core::model::SkillId,
    /// `skill.name`, the library key and the render order's tie-break.
    pub name: String,
    /// `skill.description`: the picker's one-liner, never rendered into a prompt.
    pub description: String,
    /// `skill.updated_at`: the `upsert_skill` token a save of this skill passes.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Every version, ascending; the last is the head.
    pub versions: Vec<SkillVersion>,
}

/// A body on its way to the store in [`StoreRequest::SaveSkill`]. `StoreRequest` derives `Debug`,
/// and a skill body is user text, so this prints its length only (the rule of
/// [`crate::editor::ExternalEdit`] and [`crate::ui::TextArea`]).
#[derive(Clone, PartialEq, Eq)]
pub struct SkillBody(String);

impl SkillBody {
    /// Wraps `text`.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for SkillBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SkillBody")
            .field("len", &self.0.len())
            .finish()
    }
}

impl SkillsSnapshot {
    /// The skill the library list shows under `name`.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&SkillSummary> {
        self.skills.iter().find(|entry| entry.name == name)
    }

    /// The head of `name`: the last version, or `None` for a skill with none.
    #[must_use]
    pub fn head(&self, name: &str) -> Option<&SkillVersion> {
        self.named(name).and_then(|entry| entry.versions.last())
    }

    /// One version of `name`.
    #[must_use]
    pub fn version(&self, name: &str, version: i32) -> Option<&SkillVersion> {
        self.named(name)?
            .versions
            .iter()
            .find(|row| row.version == version)
    }

    /// The attachment of `(skill_id, project, phase)`, if one is stored.
    #[must_use]
    pub fn attachment(
        &self,
        skill: htui_core::model::SkillId,
        project: Option<ProjectId>,
        phase: Option<PhaseId>,
    ) -> Option<&SkillAttachmentRow> {
        self.attachments.iter().find(|row| {
            row.skill_id == skill && row.project_id == project && row.phase_id == phase
        })
    }
}

/// One read of the scope: the whole library, and the scope's attachments in one request (D92 —
/// **never** one request per project, F-2).
///
/// # Errors
/// Whatever the backend reports; offline, [`StoreError::Unreachable`] with
/// [`htui_store::PROMPT_ON_SERVER_ONLY`] for both reads, because `skill*` is not mirrored.
pub async fn snapshot(backend: &Backend, scope: &Scope) -> Result<SkillsSnapshot> {
    let entries = backend.skill_library().await?;
    let skills = entries
        .into_iter()
        .map(|entry| SkillSummary {
            id: entry.skill.id,
            name: entry.skill.name,
            description: entry.skill.description,
            updated_at: entry.skill.updated_at,
            versions: entry.versions,
        })
        .collect();
    let attachments = backend.skill_attachments(&scope.project_ids).await?;
    Ok(SkillsSnapshot { skills, attachments })
}

/// The four request names, in [`StoreRequest`] order.
///
/// The views' `Failed` match reads from here. [`StoreRequest::name`]'s arms spell the same four as
/// literals, because it is a `const fn`, and `request_names_match_the_name_arms` pins them to this
/// list, so a name changed in one place and not the other fails there.
pub const REQUEST_NAMES: [&str; 4] = ["skills", "save_skill", "set_skill_binding", "remove_skill_binding"];

/// The **read**'s name: a refused read leaves the view with no tree, where a refused write leaves the
/// editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// Serves one skill request, off the UI task.
///
/// # Errors
/// Whatever the seam reports; offline, [`StoreError::Unreachable`] with `PROMPT_ON_SERVER_ONLY` for
/// the read and `DATABASE_UNREACHABLE` for the three writers; [`StoreError::Backend`] for a
/// request that is not one of this module's four.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        // The read goes through `Backend` rather than a `Writer`: the library and the attachments
        // are inherent on each store and refuse offline with their own sentence.
        StoreRequest::Skills(scope) => Ok(StoreReply::Skills(Box::new(snapshot(backend, scope).await?))),
        StoreRequest::SaveSkill {
            scope,
            name,
            description,
            body,
            expected,
            expected_version,
        } => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
            let created_by = backend.this_user().await?;
            // MOD-9 D101: the two tokens are two surfaces — `skill.updated_at` for the description
            // and the head version for the append — and the write is short-circuited so a stale
            // token can never leave a moved description behind an unsaved body.
            let saved = match writer
                .upsert_skill(
                    NewSkill {
                        id: htui_core::model::SkillId::new(),
                        name: name.clone(),
                        description: description.clone(),
                        created_by,
                    },
                    *expected,
                )
                .await?
            {
                CasOutcome::Applied(skill) => skill,
                CasOutcome::Stale(_) => return stale(backend, scope).await,
            };
            match writer
                .add_skill_version(
                    NewSkillVersion {
                        skill_id: saved.id,
                        body: body.as_str().to_owned(),
                        source: serde_json::json!({}),
                        created_by,
                    },
                    *expected_version,
                )
                .await?
            {
                CasOutcome::Applied(_) => {}
                CasOutcome::Stale(_) => return stale(backend, scope).await,
            }
            // The worker re-reads rather than handing the view the one row the outcome carries: the
            // view renders a list, and a row patched in locally would be a second source of truth.
            Ok(StoreReply::Skills(Box::new(snapshot(backend, scope).await?)))
        }
        StoreRequest::SetSkillBinding {
            scope,
            skill_id,
            project_id,
            phase_id,
            pinned_version,
            position,
            activation,
            globs,
            languages,
            expected,
        } => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
            let outcome = writer
                .set_skill_binding(
                    NewSkillBinding {
                        id: SkillBindingId::new(),
                        skill_id: *skill_id,
                        project_id: *project_id,
                        phase_id: *phase_id,
                        pinned_version: *pinned_version,
                        position: *position,
                        activation: *activation,
                        globs: globs.clone(),
                        languages: languages.clone(),
                    },
                    *expected,
                )
                .await?;
            match outcome {
                CasOutcome::Applied(_) => Ok(StoreReply::Skills(Box::new(snapshot(backend, scope).await?))),
                CasOutcome::Stale(_) => stale(backend, scope).await,
            }
        }
        StoreRequest::RemoveSkillBinding { scope, id, expected } => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
            let outcome = writer.remove_skill_binding(*id, *expected).await?;
            match outcome {
                CasOutcome::Applied(_) => Ok(StoreReply::Skills(Box::new(snapshot(backend, scope).await?))),
                CasOutcome::Stale(_) => stale(backend, scope).await,
            }
        }
        // `try_serve` routes exactly this module's four variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!("not a skill request: {}", other.name()))),
    }
}

/// The fresh snapshot a spent token answers with: the store as it is, unchanged, which is what the
/// editor reloads against (D101).
async fn stale(backend: &Backend, scope: &Scope) -> Result<StoreReply> {
    Ok(StoreReply::SkillsStale(Box::new(snapshot(backend, scope).await?)))
}
```

`skills.rs`'s own `mod tests`, the seven `templates.rs` tests ported plus three:

- `the_read_answers_the_whole_library_and_the_whole_scope` — `Backend::memory(MemStore::demo())`,
  the platform scope; the library holds the two demo skills with their versions ascending, and the
  attachments hold the demo's three rows **plus** the globals, in the matrix's order.
- `a_save_request_debug_prints_the_body_length_not_the_body` — the `templates.rs` test byte for
  byte, with `secret` as the body and `"len: 6"`.
- `a_save_at_the_head_appends_and_answers_skills` / `a_save_at_a_spent_token_answers_skills_stale_and_writes_nothing` — the two `templates.rs` cases.
- `a_refused_name_answers_failed_with_the_agent_skills_rule` — the writer's sentence reaches the view
  and nothing is written.
- `a_set_at_a_spent_token_answers_skills_stale_and_the_row_is_as_it_was`.
- `an_unbind_answers_skills_without_the_row`.
- `an_offline_save_is_refused_with_the_unreachable_sentence` and `an_offline_read_is_refused_with_the_server_only_sentence` — the two `templates.rs` tests.
- `request_names_match_the_name_arms` — four samples, `names == REQUEST_NAMES`.

### 2.10 `crates/htui/src/store_worker.rs`

- **`StoreRequest` gains four variants** (69 → 73), after `SaveTemplate { … }` and before
  `ConnectionInfo`, with the plan's group comment:

```rust
    // MOD-9 milestone 3 (D81): the four skill requests, served by [`crate::skills`]. Every write
    // carries the `Scope` because the reply re-reads *is* the scope, and all four are served
    // through one further or-ed arm of `try_serve`.
    /// The whole library and the scope's attachments.
    Skills(Scope),
    /// Creates or edits one skill and appends the body as the next version (D78, OQ-18).
    SaveSkill {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The library key; a create is a name no row holds. It never moves.
        name: String,
        /// `skill.description`, the library's one-liner.
        description: String,
        /// The whole new body. A skill body is markdown and is **not** parsed: its `Debug` is its
        /// length.
        body: SkillBody,
        /// The `skill.updated_at` the editor opened on: the `upsert_skill` token, `None` for a
        /// create. `None` is a token — "I expect no row" — not "don't care" (D101).
        expected: Option<DateTime<Utc>>,
        /// The head version the editor opened on: the `add_skill_version` token, `None` for a new
        /// name. Exactly `append_prompt_template`'s `expected`.
        expected_version: Option<i32>,
    },
    /// The attachment of one `(skill, project, phase)`, inserted or replaced (D78).
    SetSkillBinding {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The skill the attachment is for.
        skill_id: SkillId,
        /// The level: `None`/`None` is global, `Some(_)`/`None` the project, both `Some` a phase.
        project_id: Option<ProjectId>,
        /// The phase, which requires `project_id`.
        phase_id: Option<PhaseId>,
        /// Follows the latest version when `None`.
        pinned_version: Option<i32>,
        /// `skill_binding.position`.
        position: i32,
        /// `skill_binding.activation`.
        activation: Activation,
        /// The **effective** globs, typed plus every named language's expansion (D83).
        globs: Vec<String>,
        /// The languages as authored; display only.
        languages: Vec<String>,
        /// The `updated_at` the form opened on (`None`: no row yet). A spent token answers
        /// [`StoreReply::SkillsStale`].
        expected: Option<DateTime<Utc>>,
    },
    /// Detaches one row. The token is `updated_at`, so an unbind is as safe as every other write
    /// and a row another writer changed survives (OQ-19, R-32).
    RemoveSkillBinding {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The row to detach.
        id: SkillBindingId,
        /// The `updated_at` the matrix listed.
        expected: DateTime<Utc>,
    },
```

- **`StoreReply` gains two variants** (40 → 42), after `TemplatesStale`:

```rust
    /// The library and the scope's attachments, freshly read: the answer to
    /// [`StoreRequest::Skills`] and to every skill write that applied (MOD-9 D81).
    Skills(Box<SkillsSnapshot>),
    /// A skill write missed its token (PRD D8): the store as it is now, for the editor to reload
    /// against. The editor keeps its typed text and its draft.
    SkillsStale(Box<SkillsSnapshot>),
```

- **`StoreRequest::name` gains four arms**, string literals, after `SaveTemplate`:

```rust
            // The four of `skills::REQUEST_NAMES`, in that order (MOD-9 D81).
            Self::Skills(..) => "skills",
            Self::SaveSkill { .. } => "save_skill",
            Self::SetSkillBinding { .. } => "set_skill_binding",
            Self::RemoveSkillBinding { .. } => "remove_skill_binding",
```

- **`try_serve` gains one or-ed arm**, after the templates arm (`:1126`). Or-ed, not guarded: this
  `match` has no wildcard, and an arm with a guard does not count towards exhaustivity (MOD-15 M3
  plan F-12).

```rust
        // The four skill requests, or-ed for the reason the arms above are: a guard does not count
        // towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12).
        StoreRequest::Skills(..)
        | StoreRequest::SaveSkill { .. }
        | StoreRequest::SetSkillBinding { .. }
        | StoreRequest::RemoveSkillBinding { .. } => skills::serve(backend, request).await?,
```

### 2.11 T1 hazards, `.sqlx` and the gate

| # | Hazard | Evidence | Fix |
|---|---|---|---|
| **H-1** | D78's writer needs `glob::compile`, and the plan puts the module in T2, which lands after T1. | The plan's T2 file list owns `prompt/glob.rs`; T1's list does not mention it; `D78` lists "a glob `glob::compile` refuses" as a `set_skill_binding` refusal. | F-1/D95: the module, its `mod` line and its whole test table are commit 1 of T1. It depends only on `RepoPath` and `BoundSkill`. |
| **H-2** | A conformance case that upserts `tests` or `rust-style` moves the demo row. | `fixtures::skill_bindings()` (`:574`) binds both; `skill.name` is unique; `mem.rs:5966-6010` and `pg_criteria.rs:1806` compare tuples built from those rows. | F-11: `new_skill` takes a name no demo row holds (`house`, `mod9-*`). |
| **H-3** | `.sqlx` prepared against a database that was not migrated from zero keeps a stale nullability. | The plan's own Validate names the scratch DB; the workspace's compose `htui` database is empty. | D99: drop, recreate, `sqlx migrate run`, then `cargo sqlx prepare -- --all-features --all-targets`, and `cargo sqlx prepare --check` in the same gate. |
| **H-4** | `SaveSkill`'s two tokens can diverge, leaving a written description with no appended body. | `skill_version` is excluded from the `updated_at` trigger (`0001_init.sql` §5.1), so an append never moves `skill.updated_at`; only `SaveSkill` can write a version, and it upserts first — so the divergence is unreachable in production but constructible by a direct call. | D101: the worker short-circuits on the first `Stale`; case 2 pins a bare append; `SaveSkill`'s doc says why. |
| **H-5** | `StoreRequest` derives `Debug` and `SaveSkill` carries a body. | `templates.rs`'s `TemplateBody` and its test `a_save_request_debug_prints_the_body_length_not_the_body`. | `SkillBody` newtype with the same `Debug`; `skills.rs` ships the same test. |
| **H-6** | `pg/write.rs` binds `TEXT[]` wrongly and sqlx will not catch it. | `demo.rs` binds `&row.probed_tags[..]`; `append_prompt_template` binds a `String` for `text`. | `&new.globs[..]`, `&new.languages[..]`. |
| **H-7** | `ON CONFLICT (skill_id, project_id, phase_id)` may not infer the `UNIQUE NULLS NOT DISTINCT` constraint. | The plan's Verified-claims row checked the Postgres 16 `sql-insert.html` on 2026-09-27: inference is over columns and expressions only and the constraint is not partial. | As written; case 3 pins both answers on both stores, and a real server runs it. |
| **H-8** | `attachment`'s re-read spells `project_id = $2` and a global row compares to NULL. | The same three-way NULL comparison is the whole of `bound_skills`' `WHERE` (`read.rs:1432`), written once and read many times. | `IS NOT DISTINCT FROM` in the re-read **and** in the writer's guard, with a comment naming the global row. Case 3's global row is the test. |
| **H-9** | A duplicate `id` is `23505` on `skill_pkey` and prose on `MemStore`. | `already_exists` in `mem.rs`; `append_prompt_template` checks the id itself. | `MemStore` checks `contains_key`; case 1 step 7 pins it. |
| **H-10** | `set_skill_binding`'s create path inserts a **fresh** id while the replace path keeps the old one, so `id` is meaningful on one and ignored on the other. | `ON CONFLICT … DO UPDATE SET` does not list `id` (and must not). | Documented on `NewSkillBinding::id`; case 3 asserts the first `id` survives. |
| **H-11** | The pin refusal needs the skill's version **numbers**, so `PgStore` needs a fourth read, which the plan's "three re-reads" does not name. | D78's sixth item is "a `pinned_version` the skill does not have"; the writer-only rules were split for exactly this cost. | `skill_versions` reads `version` only; `skill_binding_refusal` / `skill_pin_refusal` are split so the read is paid only when a pin is set. |
| **H-12** | `wants_requests` and the tab's `on_reply` are not the only places that match `StoreReply`. | `app/update.rs`, `app/state.rs` and the staleness index key on `StoreRequest::name`. | `cargo build --workspace --all-features --all-targets` is in the gate; a non-exhaustive match fails there. |
| **H-13** | The `htui` suite is scheduling-dependent. | The keyring fake is process-wide. | `--test-threads=2` in the gate; nothing new here touches the keyring. |

`.sqlx`: **268 → 268+11**. Four writers, four re-reads (`skill`, `skill_head`, `skill_versions`,
`skill_binding`, `attachment` — five, so **+12** in the count as written), three library reads. The
implementer records the real `ls crates/htui-store/.sqlx | wc -l` in the commit message; a
decomposition that reads `skill_versions` and `skill_head` as one statement is fine, a silent drift
is not. **H-14.**

Gate (F-8's D99: the scratch DB is at `0007` for T1):

```bash
psql "$PG/postgres" -c 'DROP DATABASE IF EXISTS htui_prepare_m3' -c 'CREATE DATABASE htui_prepare_m3'
DATABASE_URL=$PG/htui_prepare_m3 sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_m3 cargo sqlx prepare -- --all-features --all-targets)
ls crates/htui-store/.sqlx | wc -l                       # record it
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_m3 cargo sqlx prepare --check)
RUST_BACKTRACE=0 cargo test -p htui-core --all-features -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui-store --all-features -- --test-threads=2
RUST_BACKTRACE=0 cargo test -p htui --all-features --lib -- templates skills --test-threads=2
cargo build --workspace --all-features --all-targets
cargo clippy --workspace --all-features --all-targets -- -D warnings
git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml     # D70: no dependency
```

**Commits** (each compiles): (1) red — `prompt/glob.rs` with its whole table, the `mod glob;` line,
`model/skill.rs`'s `name_is_valid` and the three `New*` structs with `todo!()` bodies for the
writers; (2) green core — `traits.rs`, `mem.rs`, `conformance.rs`, and **the five exhaustive impls'
arms** in the same commit (`Writer`, `MemStore`, `UsageSpy`, `SpyStore`, and the trait itself),
because adding a trait method breaks every implementor at once; (3) Postgres — `pg/write.rs`,
`pg/read.rs`, `backend.rs`, `.sqlx`, `pg_conformance.rs`, `skill_attachments.rs`,
`skill_binding_cas.rs`; (4) the worker seam — `skills.rs`, `store_worker.rs`, `lib.rs`; (5) the
`mem.rs` and `skills.rs` unit tests that need the green bodies.

---

## 3. T2: the matcher wiring, the record, the isolator seam and the two callers

**Files**: the plan's T2 list **minus** `prompt/glob.rs` (moved to T1, F-1/D95) **plus**
`crates/htui-agent/src/excerpt.rs` (`PassInput.changed_paths` and `excerpts_for`, D87/D89) and
`crates/htui-orch/src/isolate/git.rs` (`Cli::changed_paths`, D89). The plan's list already names
`isolate/real.rs` and `fake.rs`; it does not name `htui-agent/src/excerpt.rs`, and without it
`ExcerptRequest.changed_paths` stays dead (**H-15**).

**First failing test**:
`cargo test -p htui-core --all-features --lib model::skill` and
`cargo test -p htui-core --all-features --test prompt_skills`.

### 3.1 `model/skill.rs` — `ChoiceReason`, `SkillChoice`, `select` (D74, D97)

`ChoiceReason` gains two variants, placed **after `Always`** so the declaration order is the
decision order and the doc's list reads top to bottom. It is a plain enum with
`#[serde(rename_all = "snake_case")]` and **no** `ALL`, and `model/mod.rs`'s `check_enum` covers only
`Activation`, so no CHECK-list test moves (**H-16**):

```rust
/// Why a candidate did or did not render (plan D40, ANA-22 §6 item 8). Serialised snake_case into
/// `trim_record.skill_choices[].reason`, and the seven words are the vocabulary `0008` re-issues
/// (plan D75).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceReason {
    /// `activation = always`: rendered.
    Always,
    /// `activation = glob` and the step's file set matched one of the attachment's globs
    /// (MOD-9 D71, D73). Rendered; `SkillChoice::matched` carries the `<repo>:<path>` that fired.
    Matched,
    /// `activation = glob` and the file set resolved and matched nothing (D74).
    NoMatch,
    /// `activation = off` on the winning attachment.
    Off,
    /// `activation = glob` and **no** file set resolved: the judge, the handoff, and any step whose
    /// roots do not (D73's `Option`).
    NoPath,
    /// The winning attachment's pin names no version, or the skill has none.
    MissingVersion,
    /// The template body places no `{{skills}}`, so nothing could render.
    NotPlaced,
}

impl ChoiceReason {
    /// The serde spelling: `always`, `matched`, `no_match`, `off`, `no_path`, `missing_version`,
    /// `not_placed`. `0008_skill_match.sql` writes the same seven words into
    /// `run_step.trim_record`'s comment and `the_migration_names_every_reason_the_record_can_carry`
    /// pins the two against each other.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Matched => "matched",
            Self::NoMatch => "no_match",
            Self::Off => "off",
            Self::NoPath => "no_path",
            Self::MissingVersion => "missing_version",
            Self::NotPlaced => "not_placed",
        }
    }
}
```

`SkillChoice` gains one field, **declared last** so the eight keys are D74's order:

```rust
    /// Why.
    pub reason: ChoiceReason,
    /// The `<repo>:<path>` that fired, for a `matched` choice; `null` in the record otherwise
    /// (MOD-9 D74). D74's key set moves from seven to eight, so
    /// `choices_serialize_their_documented_keys` moves deliberately and its message says so.
    pub matched: Option<String>,
```

`select` takes the third argument D73 names and decides `matched` / `no_match` / `no_path`:

```rust
/// Plan D40/D51: decides every candidate, in order, for one step. Pure.
///
/// The rules, first match wins: a body that does not place `{{skills}}` (`placed == false`) makes
/// every candidate `not_placed`; `version: None` is `missing_version`; `Off` is `off`; a `Glob` is
/// `matched` with the path when `matches` is `Some` and names the skill, `no_match` when it is
/// `Some` and does not, and `no_path` when it is `None` (MOD-9 D73); `Always` is `always`. Returns
/// the active candidates in input order — which is collapse order, the render order — and one
/// [`SkillChoice`] per candidate in the same order.
///
/// `matches` is the engine's or the preview's answer from
/// [`glob::matched_skills`](crate::prompt::glob::matched_skills), keyed by `skill_id`. `None` means
/// **no file set resolved**, which is the truth for the judge, the handoff and a step whose roots
/// do not, and is not the same as "the set ran and matched nothing" (D97).
#[must_use]
pub fn select(
    candidates: Vec<BoundSkill>,
    placed: bool,
    matches: Option<&BTreeMap<SkillId, String>>,
) -> (Vec<BoundSkill>, Vec<SkillChoice>) {
    let mut active = Vec::with_capacity(candidates.len());
    let mut choices = Vec::with_capacity(candidates.len());
    for skill in candidates {
        let mut matched = None;
        let reason = if !placed {
            ChoiceReason::NotPlaced
        } else if skill.version.is_none() {
            ChoiceReason::MissingVersion
        } else {
            match skill.activation {
                Activation::Off => ChoiceReason::Off,
                Activation::Glob => match matches {
                    Some(matches) => match matches.get(&skill.skill_id) {
                        Some(path) => {
                            matched = Some(path.clone());
                            ChoiceReason::Matched
                        }
                        None => ChoiceReason::NoMatch,
                    },
                    // D73: no file set resolved, which is the judge's and the handoff's record.
                    None => ChoiceReason::NoPath,
                },
                Activation::Always => ChoiceReason::Always,
            }
        };
        // D74: `matched` and `always` are the two active outcomes; the test widens from
        // `reason == Always` to `matches!(reason, Always | Matched)`.
        let is_active = matches!(reason, ChoiceReason::Always | ChoiceReason::Matched);
        choices.push(SkillChoice {
            skill: skill.skill_id,
            name: skill.name.clone(),
            version: skill.version,
            level: skill.level,
            activation: skill.activation,
            active: is_active,
            reason,
            matched,
        });
        if is_active {
            active.push(skill);
        }
    }
    (active, choices)
}
```

`use std::collections::BTreeMap;` at the top of `model/skill.rs` (it has `chrono` and `serde`
imports only today).

**Tests** in `model/skill.rs`'s `mod tests` (the helpers `bound`, `attach`, `version` stay; `attach`
already gives a `Glob` candidate `vec!["**/*.rs"]`):

| Test | Change |
|---|---|
| `select_applies_its_rules_in_order` (`:711-753`) | candidates `[Always Some(1), Off Some(1), Glob Some(1), Always None]`. With `matches = None` the reasons are `[Always, Off, NoPath, MissingVersion]`; with `matches = Some({glob_skill: "htui:src/main.rs"})` they are `[Always, Off, Matched, MissingVersion]`, the `Glob` choice is active with `matched: Some("htui:src/main.rs")`, and with `matches = Some({})` it is `NoMatch` and inactive. `placed = false` still makes all four `NotPlaced` and inactive. |
| `choices_serialize_their_documented_keys` (`:757-815`) | the key set gains `"matched"`, the message becomes `"§5.1's eight keys and D74's matched: eight, no more and no fewer"`, and the loop over reasons covers the two new variants. |
| **new** `a_glob_skill_records_matched_with_its_path` | one `Glob` candidate, `Some({skill: "htui:crates/htui-core/src/lib.rs"})` → the choice is `{active: true, reason: Matched, matched: Some(…)}` and the candidate is in the active list. |
| **new** `a_glob_skill_records_no_match_when_the_file_set_ran_and_missed` | the same with `Some({})` → `{active: false, reason: NoMatch, matched: None}` and the active list is empty. |
| **new** `no_file_set_is_no_path_and_is_not_no_match` | `matches = None` → `NoPath`, distinct from `NoMatch`, with the reason in the message. |
| **new** `a_matched_choice_carries_the_path_into_the_record` | `serde_json::to_value(choice)` has `"matched": "htui:x.rs"` for a matched choice and `null` for every other reason, over all seven. |

### 3.2 `crates/htui-core/src/prompt/excerpt.rs` — `ExcerptSet.listed` (D72)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExcerptSet {
    /// The selected excerpts. Rendered in `(repo, path)` byte order (§4.7 rule 5), whatever order
    /// the ranker produced them in.
    pub files: Vec<Excerpt>,
    /// The audit half, filled by the ranker; `files` is completed by the assembler.
    pub audit: ExcerptAudit,
    /// Notes the pass produced that are not errors — a skipped repo, a dropped provider. Copied
    /// into `trim_record.notes`.
    pub notes: Vec<String>,
    /// MOD-9 D72: the walk's **enumerated** file set — every path the walk listed, after
    /// `skip_by_path` and `is_repo_relative`, whether or not it was selected. This is the set
    /// `skill_binding.globs` is matched against (D87), and it is deliberately a **superset** of
    /// `files`: a skill can activate on a file that was too big to excerpt, and on one the budget
    /// did not reach.
    ///
    /// Empty wherever no pass ran — `no_excerpts`, `unscanned`, and every fixture — which is
    /// exactly the `None` `select` is given (D97).
    pub listed: Vec<RepoPath>,
}
```

In `select` (`:1050-1299`), **immediately after** `let considered = …;` (`:1166`) and before
`vetted` / `fill_lexical_heads` / `rank` — H-17, `fill_lexical_heads` takes `&mut listing` and would
otherwise let a head-bearing entry be captured:

```rust
    let considered = u32::try_from(listing.len()).unwrap_or(u32::MAX);
    // MOD-9 D72: the enumerated file set, taken from the walk's own listing after the two filters
    // above — not a second walk (the research's option (a)), which would double the filesystem cost
    // of every step for no new data. Captured here because `fill_lexical_heads` takes the listing
    // by `&mut` and tiers 1–4 are pure over the path.
    let listed: Vec<RepoPath> = listing
        .iter()
        .map(|entry| RepoPath {
            repo: entry.repo.clone(),
            path: entry.path.clone(),
        })
        .collect();
```

and the returned literal at `:1295-1299` becomes:

```rust
    ExcerptSet {
        files,
        audit,
        notes,
        listed,
    }
```

`htui-agent/src/excerpt.rs`'s `unscanned` (the set a pass that read nothing records) sets
`listed: Vec::new()`; `engine.rs`'s `no_excerpts` (`:5822-5835`) sets `listed: Vec::new()`. Every
`ExcerptSet::default()` is already correct, because the struct derives `Default`.

**Tests**: `the_listed_set_is_the_walk_after_the_skip_rules` — a reader offering `.git/config`,
`secret.pem`, `Cargo.lock`, a NUL-bearing file, a 3 MB `.rs` and `src/main.rs`: the first five are
absent from `listed` (the first three by `skip_by_path`, the fourth by the reader, the fifth only by
`max_file_bytes` in the **reader**, which prunes it before the listing) and the last is present
whether or not the budget took it. Plus `an_unscanned_pass_lists_nothing` and
`no_excerpts_lists_nothing`.

### 3.3 `crates/htui-core/src/prompt/mod.rs` — `PromptSpec` and `scrubbed_inputs` (D73, D97)

`PromptSpec` gains two fields, declared after `skills` and before `excerpts`:

```rust
    /// MOD-9 D73/D89: the step's file set for glob activation — the walk's listing
    /// ([`ExcerptSet::listed`]) unioned with the previous attempt's changed paths, de-duplicated on
    /// `(repo, path)` and in that order. Filled by [`Engine::with_excerpts`](…) and by
    /// [`crate::preview::build`]; empty wherever no file set resolved.
    pub skill_files: Vec<RepoPath>,
    /// MOD-9 D73/D97: what [`matched_skills`](crate::prompt::glob::matched_skills) fired on, or
    /// `None` when **no** file set resolved — which is what `select` records as `no_path`, and is
    /// not the same as "the set ran and matched nothing". `Some({})` is that other answer.
    ///
    /// The `Option` is D73's, which gives `select` an `Option<&BTreeMap<…>>`; a bare map on the
    /// spec would collapse `no_path` and `no_match` into one record (F-5).
    pub skill_matches: Option<BTreeMap<SkillId, String>>,
```

`PromptSpec` derives no `Default`, so **every** literal must grow both fields (F-5, H-18):

| Literal | `skill_files` | `skill_matches` |
|---|---|---|
| `prompt/fixtures.rs` — every `PromptSpec` factory (`phase_implement_attempt2` :102, `phase_all_empty` :258, `phase_oversize` :590, `phase_skills_over_cap` :694) | `Vec::new()` | `None` |
| `engine.rs:5085` (`phase_spec`'s literal) | `Vec::new()` | `None` — `with_excerpts` fills both for a phase prompt |
| `engine.rs`'s judge prompt builder | `Vec::new()` | `None` — the judge's placeholder set cannot place `{{excerpts}}` |
| `promote.rs:81` (`handoff_spec`) | inherited through `..phase` | inherited |
| `preview.rs`'s `build` literal (:266) | filled after the walk, immediately before `assemble` | `Some(…)`, the same two statements |

`scrubbed_inputs` (`:851-871`), the one line the plan names at `:861`:

```rust
    let placed = parsed.used.contains(&Placeholder::Skills);
    // MOD-9 D73/D97: the spec's own match, `None` where no file set resolved. `skill_matches` is
    // cloned because the masked spec is moved into `ScrubbedInputs` immediately after and the
    // borrow would outlive the match on `spec.skill_matches`.
    let (skills, skill_choices) = select(
        BoundSkill::collapse(spec.skills.clone()),
        placed,
        spec.skill_matches.as_ref(),
    );
```

**Tests**: `crates/htui-core/tests/prompt_skills.rs` gains the plan's rows, and
`crates/htui-core/tests/prompt_digest.rs` moves **one** place: the golden's `skill_choices` entry
gains `"matched": null` (the demo skills are `Always`, H-19). The **13 top-level keys** at
`:979-994` and `v == 2` at `:996-999` do **not** move, and the message there is unchanged.

`crates/htui-core/src/fixtures.rs:1631-1665` `IMPL_TRIM_RECORD` becomes:

```json
  "skill_choices": [
    { "skill": "00000000-0000-0000-0000-000000005111", "name": "rust-style", "version": 2,
      "level": "project", "activation": "always", "active": true, "reason": "always",
      "matched": null }
  ],
```

`estimated_after` stays 35 988, so `backlog__detail_runs` and `replay__runs_step_selected` do not
move. `step_impl_carries_the_golden_trim_record` (`fixtures.rs:2444-2456`) is the pin.

### 3.4 `crates/htui-store/migrations/0008_skill_match.sql` (new) and its pins (D75)

The file, complete. LF only, no cache migration (`skill*` is not mirrored):

```sql
-- 0008_skill_match.sql - MOD-9 milestone 3 (docs/ANA-22.md 6 item 8; plan D75, D87, D89).
-- Forward-only (R-STO-5).
--
-- No column and no constraint changes. 0007 wrote run_step.trim_record's comment with the reason
-- vocabulary of milestone 2, which had no `matched` and no `no_match`; a comment cannot be edited
-- in place, because schema_state refuses a database that applied an older text of an existing
-- migration (pg/mod.rs:584-627, error.rs checksum_drift), so the contract is re-issued here. The
-- commented-column total does not move: this writes one comment over a column 0007 already
-- commented, so the thirty-four of migrations.rs is still thirty-four.

-- --------------------------------------------------------------------------------------------
-- 1. run_step.trim_record: MOD-9 D75's reason vocabulary
-- --------------------------------------------------------------------------------------------

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42: {v, template, budget, budget_source, reserve, target, '
  'estimator, estimated_before, estimated_after, sections[], skill_choices[], excerpts, notes}, '
  'v 2. skill_choices[] is every candidate skill, ordered by position then name, each {skill, '
  'name, version, level, activation, active, reason, matched} with reason always, matched, '
  'no_match, off, no_path, missing_version or not_placed (ANA-22 6 item 8); matched is the '
  'repo:path the glob fired on, or null. A v 1 record, written before 0007, has no '
  'skill_choices. Canonical; the prompt payload sections[] array is its abridged projection. '
  'Written at stage 3 by set_step_prompt, before the session starts.';
```

`crates/htui-store/tests/migrations.rs`:

| Where | Change |
|---|---|
| `:81` | `vec![1, 2, 3, 4, 5, 6, 7, 8]`, and the self-checking assertion at `:78` and the message naming `0007_skill_attachments.sql` gain `0008_skill_match.sql` |
| `:206-213` | the `trim_record` literal moves byte for byte to the text above, written with Rust `\` line continuations, so the `\`-joined value is byte-equal to the SQL literal. **This is the guard** `the_ana_column_comments_are_present_and_verbatim` (`:417`) exists to enforce. |
| `:168-176` (doc) | "…the text `0007_skill_attachments.sql` restates" becomes "the text `0008_skill_match.sql` re-issues" |
| `:877-878` | `MigrationState::Pending(8)` and `"eight embedded migrations, none applied"` |
| `:101-105` `TABLES` | **unchanged**, 39 |
| `:495-498` and `:454` | **unchanged**, 34 and the message that forbids "a half-finished thirty-fifth" |
| `MOD7_COLUMN_COMMENTS` (4) and `MOD9_COLUMN_COMMENTS` (5) | **unchanged** — `0008` writes no new column comment |
| **new** `the_migration_names_every_reason_the_record_can_carry` | reads `0008` with `include_str!("../migrations/0008_skill_match.sql")` and asserts each of the seven `ChoiceReason::as_str()` words appears, and that the eight `SkillChoice` keys all appear. No server needed, so it runs on every gate. |
| **new** `the_older_text_is_still_the_one_0007_wrote` | asserts `0007`'s text does **not** contain `no_match`, so a future edit to `0007` in place fails here as well as at `schema_state` (`:913-929`). |

`crates/htui-store/tests/connect.rs`: `:102` → `Pending(8)`, `:103` "seven embedded migrations
since MOD-9 milestone 2" → "eight embedded migrations since MOD-9 milestone 3", `:119` "all seven"
→ "all eight", `:204` → `Pending(8)`, `:205` "seven … since" → "eight … since".

### 3.5 `Isolator::changed_paths` (D89)

`crates/htui-orch/src/isolate.rs`, after `diff` (`:196`) and before `reconcile`:

```rust
    /// MOD-9 D89: the repo-relative paths the previous attempt's commits touched, one entry per
    /// `(repo, path)`, in `commits` order and then git's own order. Empty when no row committed
    /// anything, and when `git` is unusable — the file set is a hint, and its absence degrades a
    /// prompt rather than failing it.
    ///
    /// This is the input the excerpt ranker's tier 2 has always read
    /// ([`ExcerptRequest::changed_paths`]) and, since MOD-9 milestone 3, half of the file set a
    /// `glob` attachment is matched against (D87). It is a seam method rather than a
    /// [`ReadStore`](crate::store::ReadStore) one because `run_step_commit` stores `before_hash` and
    /// `after_hash` and **no paths** (`model/run.rs:585-594`): a store reader would have to
    /// re-derive them from the rendered `DiffBlock`, which that type's own doc
    /// (`prompt/mod.rs:157-159`) forbids. Asking git is cheaper and more correct — it knows
    /// renames and binary files, and a diff-text parser guesses.
    fn changed_paths<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, Vec<RepoPath>>;
```

`crates/htui-orch/src/isolate/real.rs`. `diff_of` (`:1287-1351`) is split so both verbs share the
range resolution — H-20, the refactor must not change `diff`'s bytes:

```rust
    /// MOD-9 D89: the checkout and the two revisions `diff_of` resolved, or `None` when the commit
    /// moved nothing or its repo has no checkout here. Shared verbatim: a `changed_paths` answer
    /// over a different range than `diff` would be a different question.
    async fn range_of(
        &self,
        trees: &[RunStepTree],
        commit: &RunStepCommit,
    ) -> Result<Option<(String, PathBuf, String, String)>, IsolateError> {
        let before = commit.before_hash.as_str();
        let Some(after) = commit
            .after_hash
            .as_deref()
            .filter(|after| *after != before)
        else {
            return Ok(None);
        };
        let checkout = self
            .config
            .repos
            .get(&commit.repo_id)
            .ok_or_else(|| IsolateError::Refused(no_checkout_for_repo()))?;
        let mut repo = checkout.local_path.clone();
        if let Some(tree) = trees
            .iter()
            .find(|tree| tree.repo_id == commit.repo_id && tree.mode == Isolation::Copy)
        {
            let copy = PathBuf::from(&tree.path);
            let (probe, hex) = (copy.clone(), after.to_owned());
            if blocking(move || git::has_commit(&probe, &hex))
                .await
                .unwrap_or(false)
            {
                repo = copy;
            }
        }
        let merges = trees.iter().any(|tree| {
            tree.repo_id == commit.repo_id
                && matches!(tree.mode, Isolation::Worktree | Isolation::Copy)
        });
        let mut merged_onto = None;
        if merges {
            let (probe, base, hex, step) = (
                checkout.local_path.clone(),
                before.to_owned(),
                after.to_owned(),
                commit.run_step_id,
            );
            merged_onto =
                blocking(move || git::reconcile_parent(&probe, &base, &hex, step)).await?;
        }
        let before = merged_onto.as_deref().unwrap_or(before);
        Ok(Some((
            checkout.name.clone(),
            repo,
            before.to_owned(),
            after.to_owned(),
        )))
    }

    /// MOD-9 D89: one `git diff --name-only -z` per committed row, over `range_of`'s revisions, in
    /// the order of `commits`. No usable `git` is `Ok(Vec::new())`.
    fn changed_paths<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, Vec<RepoPath>> {
        Box::pin(async move {
            let Ok(git) = self.cli() else {
                return Ok(Vec::new());
            };
            let git = git.clone();
            let mut out: Vec<RepoPath> = Vec::new();
            for commit in commits {
                let Some((name, repo, before, after)) = self.range_of(trees, commit).await? else {
                    continue;
                };
                let listed = git.changed_paths(&repo, &before, &after).await?;
                for path in listed.split('\0').filter(|path| !path.is_empty()) {
                    out.push(RepoPath {
                        repo: name.clone(),
                        path: path.to_owned(),
                    });
                }
            }
            Ok(out)
        })
    }
```

`diff_of` then becomes the tail of `range_of` plus its two `git.diff` calls, and its doc is kept
("D55: two `git diff`s per committed row, in the order of `commits`… No usable `git` is `Ok(None)`").
`diff` (`:1451`) is **not** changed: it keeps its own `git.clone()`, its `parts` accumulation and
its `None` for an empty list, so every `DiffBlock` byte is the same (**H-20**).

`crates/htui-orch/src/isolate/git.rs`, beside `Cli::diff` (`:815-841`):

```rust
    /// MOD-9 D89: the repo-relative paths `<before>..<after>` touches, NUL-delimited.
    ///
    /// `-z` so a path git would otherwise quote under `core.quotePath` arrives verbatim and needs
    /// no unescaping, and so a name containing a space is one field. The same flags as `diff` and
    /// the same `DIFF_COLUMNS` budget, so the call is bounded exactly as that one is.
    pub async fn changed_paths(
        &self,
        repo: &Path,
        before: &str,
        after: &str,
    ) -> Result<String, IsolateError> {
        let args = [
            OsStr::new("diff"),
            OsStr::new("--no-color"),
            OsStr::new("--no-ext-diff"),
            OsStr::new("--no-textconv"),
            OsStr::new("--name-only"),
            OsStr::new("-z"),
            OsStr::new(before),
            OsStr::new(after),
            OsStr::new("--"),
        ];
        let exited = self
            .run_capturing("diff", repo, &args, &[DIFF_COLUMNS], Capture::Head)
            .await?;
        if !exited.ok() {
            return Err(exited.failure("diff"));
        }
        Ok(exited.stdout)
    }
```

`crates/htui-orch/src/fake.rs`, beside `FakeIsolator::diff` (`:576-595`):

```rust
    /// MOD-9 D89: the next [`script_changed_paths`](FakeIsolator::script_changed_paths) or
    /// [`fail_changed_paths`](FakeIsolator::fail_changed_paths) answer, or `Vec::new()` unscripted;
    /// the rows are recorded for [`changed_path_requests`](FakeIsolator::changed_path_requests)
    /// either way, so a case can assert the call happened exactly once and on which step.
    fn changed_paths<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, Vec<RepoPath>> { … }
```

with `changed_paths: Mutex<VecDeque<Result<Vec<RepoPath>, IsolateError>>>` beside `diffs`,
`changed_path_requests: Mutex<Vec<DiffRequest>>` beside `diff_requests`, and the two scripting
methods beside `script_diff` (`:217`).

### 3.6 `Engine::with_excerpts` — the exact place (D73, D87, D89, R-35)

`crates/htui-orch/src/engine.rs:4936-4980`. The current tail is:

```rust
        let input = PassInput {
            roots,
            touched_prefixes: touched_prefixes(&row.touched_paths, &repos),
            notes,
        };
        let excerpts = excerpts_for(spec, input, &self.parts.app, self.parts.scrubber).await;
        spec.excerpts = excerpts;
        Ok(())
```

It becomes:

```rust
        // MOD-9 D89 / R-35: the previous attempt's changed paths, asked of the isolator — the only
        // holder of the checkout the commits were made in. Two guards, both about cost: an attempt
        // of 1 has no commits, and a step with no `glob` candidate cannot use the answer, which is
        // the common case. The second guard is why tier 2 of the ranker stays **inert** on a step
        // with no glob attachment: `TIER2_PREV_DIFF` is live only where a `glob` fired (H-22).
        let wants_changed = step.attempt > 1
            && spec
                .skills
                .iter()
                .any(|skill| skill.activation == htui_core::model::Activation::Glob);
        let mut changed: Vec<RepoPath> = Vec::new();
        if wants_changed {
            // The previous step's trees and commits, read again: `forwarded` read them for the
            // diff, and the two reads are two primary-key lookups beside a walk that is already
            // reading the same tree (H-23). The alternative — a field on `PromptSpec` — would carry
            // a commit list into a struct the assembler digests.
            let previous = self
                .parts
                .store
                .run_steps(run.id)
                .await?
                .into_iter()
                .filter(|row| row.position == step.position && row.attempt < step.attempt)
                .max_by_key(|row| row.attempt);
            if let Some(previous) = previous {
                let trees = self.parts.store.step_trees(previous.id).await?;
                let commits = self.parts.store.step_commits(previous.id).await?;
                match self.parts.isolator.changed_paths(&trees, &commits).await {
                    Ok(paths) => changed = paths,
                    // The diff is advisory input, so a failure degrades the prompt with a note
                    // rather than failing the step — `forwarded`'s rule, for the same input.
                    Err(err) => notes.push(format!("changed_paths unavailable: {err}")),
                }
            }
        }
        let input = PassInput {
            roots,
            touched_prefixes: touched_prefixes(&row.touched_paths, &repos),
            // MOD-9 D87: the ranker's tier 2 reads this and has never had a producer in
            // production. Now it does.
            changed_paths: changed.clone(),
            notes,
        };
        let excerpts = excerpts_for(spec, input, &self.parts.app, self.parts.scrubber).await;
        spec.excerpts = excerpts;
        // MOD-9 D73/D89: the matcher's file set is the walk's listing unioned with the changed
        // paths, de-duplicated on `(repo, path)` and in that order, so the recorded `matched` path
        // is the walk's first when both could match. The union is what `select` reads, and the
        // `Some` is what separates a `no_match` from a `no_path` (D97).
        spec.skill_files = {
            let mut files = spec.excerpts.listed.clone();
            for path in changed {
                if !files.contains(&path) {
                    files.push(path);
                }
            }
            files
        };
        spec.skill_matches = Some(htui_core::prompt::glob::matched_skills(
            &spec.skills,
            &spec.skill_files,
        ));
        Ok(())
```

`PassInput` (`htui-agent/src/excerpt.rs:882`) gains one field, after `touched_prefixes`:

```rust
    /// The previous attempt's changed paths (MOD-9 D87/D89), already repo-qualified. Empty on
    /// attempt 1 and wherever the isolator could not answer; the pass reads it for tier 2's
    /// `prev_diff` and the matcher reads the union of it and the listing.
    pub changed_paths: Vec<RepoPath>,
```

and `excerpts_for` (`:932`) replaces its `changed_paths: Vec::new()` and its `D122` comment with
`changed_paths: input.changed_paths,` — the destructuring at `:940-944` gains the field.

**Tests**:

- `changed_paths_are_the_files_the_previous_attempt_touched` — `FakeIsolator` scripts two
  `RepoPath`s; the engine's spec carries the union; `changed_path_requests` records exactly one
  call, on the previous step's `StepId`.
- `an_attempt_of_one_asks_for_no_changed_paths` — `script_changed_paths` is never popped and
  `changed_path_requests` is empty.
- `a_step_with_no_glob_candidate_asks_for_none` — the fixture's three attachments are all `Always`,
  so the call is skipped even at attempt 2 (**H-22**, the test that makes the guard visible).
- `a_failing_changed_paths_call_notes_and_continues` — `fail_changed_paths`; the note is
  `"changed_paths unavailable: …"` and `spec.skill_files` is the walk's listing alone.
- `a_phase_step_whose_glob_matches_renders_the_skill` and `a_judge_step_still_records_no_path` —
  over `harness_engine!`, as milestone 2's `a_phase_step_renders_its_phase_and_project_skills`
  (`engine.rs:12924`'s neighbours) already do.
- **replaced**, not kept alongside: `a_glob_skill_records_no_path` (milestone 2's) becomes
  `a_glob_skill_records_no_match_when_the_walk_finds_nothing`.
- `TIER2_PREV_DIFF` now fires: a step at attempt 2 whose previous attempt touched a file the
  ranker did not rank carries it, with reason `"prev_diff"` and the weight of
  `TIER2_PREV_DIFF`. This test **must** carry a `glob` candidate (H-22).

### 3.7 `preview.rs` — the second caller (D73)

`crates/htui/src/preview.rs`. The walk is at `:308`, immediately before `assemble`; the two fills go
in the same place, and the note is re-worded at the top of the file.

```rust
/// MOD-9 D73: which phase's attachments the preview shows, and that it runs the same walk a step
/// does, so a `glob` attachment here records `matched` or `no_match` exactly as it would there.
const SKILLS_NOTE: &str = "preview: phase-level skills come from the first phase of the item's \
                           graph that uses this template; a glob attachment is matched against \
                           this box's excerpt walk, as it is in a run";
```

`STAND_INS` (`:61-70`) keeps **eight** entries — one sentence is re-worded, none is added — and
`the_preview_declares_its_stand_ins` (`prompt_preview.rs:158-192`, its literal at `:181`) is edited
in the same commit, which its own comment demands ("edit in two files").

The spec literal (`:266`) gains `skill_files: Vec::new(),` and `skill_matches: None,` after
`skills,`, and the two fills land after the walk, before `assemble`:

```rust
    // The engine's own pass, one call (MOD-7 P-1): the preview's bytes and a run's cannot drift.
    spec.excerpts = excerpts_for(&spec, input, &app, &scrubber).await;
    // MOD-9 D73: the preview runs the same walk, so it fills the same two fields the engine does.
    // There is no previous attempt, so `skill_files` is the listing alone.
    spec.skill_files = spec.excerpts.listed.clone();
    spec.skill_matches = Some(htui_core::prompt::glob::matched_skills(
        &spec.skills,
        &spec.skill_files,
    ));
```

`build`'s doc table gains one row ("the file set a `glob` attachment is matched against") and the
module doc's "three fields filled by [`STAND_INS`] instead of by a `run_step`" stays true: the
preview has no `run_step`, so `changed_paths` is empty and the walk's listing is the whole set.

**Tests** in `crates/htui/tests/prompt_preview.rs`: a new
`the_preview_matches_a_glob_attachment_against_its_own_walk` — a `glob` attachment over `**/*.rs`
on FEAT-1's `implement` phase, an `htui` checkout holding a `.rs` file, `Some(chosen)` template —
`trim.skill_choices[0].reason == Matched` and `matched == "htui:<path>"`; and
`the_preview_records_no_match_when_the_walk_misses`, with an empty checkout, `reason == NoMatch`.
The two existing snapshot cases move **only** in the `SKILLS_NOTE` row; no digest moves, because
every demo attachment is `Always` with empty `globs` (H-19).

### 3.8 T2 hazards, the `.sqlx` state and the gate

| # | Hazard | Evidence | Fix |
|---|---|---|---|
| **H-15** | `ExcerptRequest.changed_paths` stays dead without `htui-agent/src/excerpt.rs`. | `excerpts_for` (`:932`) hard-codes `changed_paths: Vec::new()` under a `D122` comment; `PassInput` (`:882`) has no such field. | T2's file list gains `crates/htui-agent/src/excerpt.rs`; the `htui-agent` crate is in T2's gate. |
| **H-16** | A `CHECK`-list test moves with `ChoiceReason`. | `ChoiceReason` is a plain enum with no `ALL`; `model/mod.rs`'s `check_enum` covers only column enums and `activation_matches_check_list` covers `Activation`. | Nothing moves. `0008`'s new test is the vocabulary cross-check instead. |
| **H-17** | `listed` is captured after `fill_lexical_heads` took `&mut listing`. | `select` (`:1050-1299`): `fill_lexical_heads(reader, req, &roots, &mut listing, &mut notes)` precedes `rank(req, &listing)`. | Capture immediately after `let considered = …;` (`:1166`), as §3.2 spells. |
| **H-18** | `PromptSpec` has no `Default`, so a missed literal is a compile error — but there are more literals than §3.3 lists. | `PromptSpec` derives `Debug, Clone, PartialEq` (`prompt/mod.rs:71-73`). | `cargo build --workspace --all-features --all-targets` in the gate; the compiler names every one. |
| **H-19** | A demo digest or snapshot moves. | `demo_skill_rows_use_the_column_defaults` (`fixtures.rs:2024`) pins every demo attachment to `Always` with empty `globs`. | No demo skill is `Glob`, so no demo `skill_matches` is non-`None` and no rendered section changes. `IMPL_TRIM_RECORD` gains `"matched": null` and nothing else. `prompt_digest.rs`'s 13 keys and `v == 2` are untouched. |
| **H-20** | Splitting `diff_of` changes `DiffBlock`'s bytes. | `diff_of` (`:1287`) computes the checkout, the `Copy`-tree probe and the D141/D146 merge parent, then two `git.diff` calls. | `range_of` is the extraction; `diff` and `diff_of`'s tail are byte-identical, and the whole `htui-orch` suite is the gate. |
| **H-21** | A `**` path under a `git diff` is not repo-relative, and `is_repo_relative` would drop it. | `is_repo_relative` (`excerpt.rs:952`) is the walk's filter, not the matcher's. | The matcher does **not** re-filter, and it cannot: `**/*.rs` matches `../x.rs`, `a/../x.rs` and `/abs/x.rs` alike, because `**` matches zero components and `*.rs` then matches the last one. The defence is upstream and is now the engine's, where the union is built — `PromptSpec.skill_files` applies the walk's own two guards (`excerpt::denied_path`, MOD-9 milestone 3's fix pass) to every changed path before adding it, so a name that is not a file the walk would have listed never enters the matcher's input at all. `git diff --name-only` from a repo root is repo-relative by construction, so the filter is a second line rather than the only one; `changed_paths_are_the_files_the_previous_attempt_touched` scripts `../outside.rs` and `Cargo.lock` and pins that neither reaches the file set. |
| **H-22** | R-35's guard keeps `TIER2_PREV_DIFF` inert on a step with no `glob` candidate. | R-35's mitigation text; `tier_of` (`excerpt.rs:755-761`) reads `req.changed_paths`. | The guard is in `with_excerpts` and stated in its comment; the tier-2 test carries a `glob` candidate, and `a_step_with_no_glob_candidate_asks_for_none` names the trade. |
| **H-23** | `with_excerpts` re-reads the previous step's trees and commits, which `forwarded` already read. | `forwarded` (`:5163-5212`) reads `step_trees(previous.id)` and `step_commits(previous.id)`. | Two primary-key lookups on a retried step, beside a walk already reading the same tree. Stated in the code comment; recorded in the T2 commit if it shows up. |
| **H-24** | `run_steps(run.id)` in the new code re-reads every step to find the previous attempt, where `forwarded` already knows it. | `winner_at(&steps, step.position, step.attempt - 1)` is `forwarded`'s own lookup. | Accept the read (a `run_steps` read is one indexed query htui already makes several of per step) and say so; the alternative is a field on `PromptSpec`, which is worse (H-23's last clause). |
| **H-25** | The migration is comment-only but the pinned literal moves byte for byte. | `the_ana_column_comments_are_present_and_verbatim` (`:417`) reads `col_description`. | §3.4 gives the SQL and the Rust literal's new text; the `\` continuations must produce a byte-equal value. |
| **H-26** | `schema_state` refuses a database that applied an older text, so a database already at `0007` cannot take `0008` twice. | `pg/mod.rs:584-627`, `error.rs:107-109`, `0001_init.sql:9-11`. | Every gate's scratch database is dropped and recreated; the T2 gate migrates the T1 scratch DB forward exactly once. |

`.sqlx`: **unchanged** by T2. `0008` writes one comment and touches no column a query names, so
`cargo sqlx prepare --check` passes against the same files (D99).

```bash
DATABASE_URL=$PG/htui_prepare_m3 sqlx migrate run --source crates/htui-store/migrations   # to 0008
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_m3 cargo sqlx prepare --check)
RUST_BACKTRACE=0 cargo test -p htui-core --all-features -- --test-threads=2
RUST_BACKTRACE=0 cargo test -p htui-orch --all-features -- --test-threads=2
RUST_BACKTRACE=0 cargo test -p htui-agent --all-features -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui --all-features -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui-store --all-features --test migrations -- --test-threads=2
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

**Commits**: (1) red — the moved `select` signature, the new `ChoiceReason` variants, the
`ExcerptSet.listed` field, the `PromptSpec` fields and every literal, all bodies `todo!()`; the
`migrations.rs` pins; the `preview.rs` and `engine.rs` tests; (2) green core —
`excerpt.rs`, `skill.rs`, `prompt/mod.rs`, `fixtures.rs`; (3) the isolator — `isolate.rs`,
`real.rs`, `git.rs`, `fake.rs`, `htui-agent/src/excerpt.rs`; (4) the two callers — `engine.rs`,
`preview.rs`, `0008`, `migrations.rs`, `connect.rs`.

---

## 4. T3: the Skills view (D82, D84)

**Files**: `crates/htui/src/ui/tabs/skills/library.rs` (new, `mod library;` at `skills/mod.rs:8`),
`crates/htui/src/ui/tabs/skills/mod.rs` (the `mod` line, the per-view guard at `:86`,
`wants_requests` at `:75`), `crates/htui/tests/skills.rs` (new),
`crates/htui/tests/snapshots/skills__*.snap` (new).

### 4.1 The state struct

`library.rs` is `ui/tabs/skills/templates.rs` with the project half removed and the estimate added.
Thirteen fields, thirteen of the template view's, in the same order:

```rust
/// The Skills view (MOD-9 milestone 3, D82): the library, a skill's versions, the line diff, the
/// body editor and the save. Holds no store handle and no `UserId` (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct SkillsView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<SkillsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`: the pane says so instead of a stale list.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](SkillsView::rows).
    cursor: usize,
    /// The version shown for the selected skill; `None` is the head.
    shown: Option<i32>,
    /// What `d` diffs against; `None` is the shown version's predecessor.
    base: Option<DiffBase>,
    /// Which pane the Browse layout shows.
    pane: Pane,
    /// The pane's first drawn row. Back to the top whenever the pane shows something else.
    scroll: Scroll,
    /// The pane's rows at the last draw, what [`scroll`](SkillsView::scroll) clamps against.
    pane_rows: Cell<usize>,
    /// Browsing, naming a skill, or editing its body.
    mode: Mode,
    /// The write in flight, by `StoreRequest::name`. One at a time (`settings/prompt.rs`'s rule).
    busy: Option<&'static str>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
    /// What `E`/`Ctrl+E` asked `$EDITOR` for, until `on_external_edit`.
    external: Option<Pending>,
    /// The editor's last drawn height.
    page: Cell<u16>,
}

/// One row of the list, derived from the snapshot on demand. **One variant, not two**: the library
/// is global (`skill` has no `project_id`, D92), so there is no project header to draw — which is
/// the first and only structural deviation from the Templates view.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// One library entry, by name: the row's label and its identity.
    Skill(String),
}

/// Browsing, naming a skill, or editing its body.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// The list and the pane.
    Browse,
    /// `n` (empty) or `e` (filled): the name and the description. `Tab` moves between the two
    /// fields, so create and edit-description are one code path and one hint.
    Naming {
        /// The library key. Prefilled and immutable under `e`; free under `n`.
        name: TextField,
        /// `skill.description`, the list's one-liner.
        description: TextField,
        /// `true` when the name is fixed (`e`), so `Backspace` on it is ignored.
        fixed_name: bool,
    },
    /// `E`: the body editor.
    Editing(Editor),
}

/// An open body editor. Never `Debug`s a body.
struct Editor {
    /// The library key, and the subject of both CAS tokens.
    name: String,
    /// The head version when the editor opened: the `add_skill_version` token (`None`: a new name).
    token: Option<i32>,
    /// The skill's `updated_at` when the editor opened: the `upsert_skill` token (`None`: no row).
    /// **D101**: two tokens, two tables, and the request carries both.
    updated_at: Option<DateTime<Utc>>,
    /// The version the draft started from, for the pane title (`None`: a new name).
    from: Option<i32>,
    /// The draft.
    area: TextArea,
    /// The text the draft started from: `Esc` asks only when the draft differs.
    original: String,
    /// The description the save carries, as `e` last left it; empty means "as it is stored".
    description: String,
    /// `Esc` warned about unsaved changes; the next one discards.
    esc_armed: bool,
    /// The body the save in flight carries: what tells that save's version from another session's,
    /// and a draft typed on since from the one that was saved.
    sent: Option<String>,
}

/// Which pane the Browse layout shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Pane {
    /// The shown version's body.
    #[default]
    Body,
    /// The line diff against [`DiffBase`].
    Diff,
}

/// What `d` diffs against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DiffBase {
    /// The shown version's predecessor.
    #[default]
    Predecessor,
    /// One named version.
    Version(i32),
    /// The first version of the skill, which is what the Templates view's `D` means by "default"
    /// (`body_of(name)`, the compiled seed) — here there is no compiled seed, so the base is the
    /// oldest stored version.
    Oldest,
}

/// `Notice` and `Pending` are the Templates view's own types, copied verbatim (`templates.rs:281`
/// and `:272`): two variants each, and no third state is needed here.
```

`Mode::Naming` is the one place this blueprint departs from a pure copy (§11): the templates view's
`Naming { project, field }` has one field because a template has no description, and a skill's
name **and** description are both `R-SKL-1` ("the library is `name` + `description` + versioned
body").

### 4.2 Constants, layout, keymap, notices

Constants, in `templates.rs`'s order and style (`:33-97`):

```rust
/// The `StoreRequest::SaveSkill` name, read by the view's `Failed` match. It is a local copy of
/// `skills::REQUEST_NAMES[1]` because `name()` is a `const fn` and cannot read a slice.
const SAVE_NAME: &str = "save_skill";

/// The pane before the first reply.
const NOT_READ: &str = "skills not read yet";
/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "skills unavailable";
/// A key pressed with nothing under the cursor.
const SELECT_A_SKILL: &str = "select a skill";

/// A save went out.
const SAVING: &str = "saving\u{2026}";
/// `Esc` over a modified draft, the first time.
const UNSAVED: &str = "unsaved changes \u{2014} Esc again discards";
/// An `$EDITOR` return that came back with text.
const EDITED: &str = "edited in $EDITOR \u{2014} Ctrl+S saves";
/// An `$EDITOR` return that changed nothing.
const NO_CHANGES: &str = "no changes";
/// Appended to [`NO_CHANGES`] when the editor returned within `QUICK_EXIT`.
const WAIT_FLAG: &str = " \u{2014} a GUI editor needs its wait flag, e.g. `code --wait`";

/// The list's width. Wider than the templates list's 40: a skill row carries a version, an
/// activation and a token estimate where a template row carried a version and a role (H-27).
const LIST_WIDTH: u16 = 46;
/// The name column inside [`LIST_WIDTH`], cut with an ellipsis past it.
const NAME_WIDTH: usize = 26;
/// The editor's help pane.
const HELP_WIDTH: u16 = 38;
/// How many lines the notice row wraps to.
const NOTICE_LINES: usize = 36;
/// The pane's bottom border while its lines overflow it.
const SCROLL_HINT: &str = " J/K PgUp/PgDn scroll ";

/// The hint row in Browse. The Templates view's clauses in its order, with the skills verbs: it
/// has no `D default` (a skill has no compiled seed) and no `h/l view` (the tab owns those).
const BROWSE_HINT: &str = "j/k move  ,/. version  b base  d diff  e description  E edit  \
                           n new  r reload";

/// The hint row while naming a skill or editing its description.
const NAMING_HINT: &str = "Tab next field  Enter confirm  Esc cancel";

/// The hint row in the editor, before the cursor's `L{line}:C{col}`.
const EDIT_HINT: &str = "Ctrl+S save  Ctrl+E $EDITOR  Esc cancel";

/// The save that landed while the head moved: the draft is kept and the token moves to the head
/// the user has now been told about, so the next `Ctrl+S` is a deliberate overwrite-by-append.
fn skill_changed_elsewhere(head: i32) -> String {
    format!("this skill changed elsewhere and is now at v{head}; the draft is kept, and Ctrl+S appends to v{}", head + 1)
}
```

`render` and `render_browse` are `templates.rs:419-464` and `:876-949` copied with three edits:
`Block::new().borders(Borders::ALL).title(" Skills ")` for the list, `self.rows()`'s single
variant, and the row's text:

```rust
Row::Skill(name) => {
    let entry = self.snapshot.as_ref().and_then(|snapshot| snapshot.named(name));
    let head = entry.map_or(0, |entry| entry.versions.len() as i32);
    let activation = entry
        .and_then(|entry| crate::skills::SkillsSnapshot::attachment_of_first(entry, self))
        .map_or("—", |row| row.activation.as_str());
    // MOD-9 D84: the head body's token estimate, computed here in `render` from bytes the snapshot
    // already holds. Thirty rows is thirty short scans of strings in memory, no store round-trip,
    // and `TokenEstimator::estimate` is a pure function of one `&str` (estimate.rs:114) — so
    // `R-NF-3` holds. The estimator's own id goes in the pane title, so one id never carries two
    // arithmetics.
    let tokens = entry
        .and_then(|entry| entry.versions.last())
        .map_or_else(String::new, |head| {
            format!("~{}", TokenEstimator::DEFAULT.estimate(&head.body))
        });
    let name = if name.chars().count() > NAME_WIDTH {
        let cut: String = name.chars().take(NAME_WIDTH - 1).collect();
        format!("{cut}\u{2026}")
    } else {
        name.clone()
    };
    (format!("  {name:<NAME_WIDTH$} v{head:<3} {activation:<5} {tokens}"), theme.base)
}
```

**Layout** is the Templates view's: `Layout::vertical([Min(1), Length(notice_height), Length(1)])`
in `render`, and
`Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Min(1)])` in `render_browse`, with
the body and the diff wrapped and scrolled by the same `width`/`rows`/`visible` arithmetic
(`templates.rs:930-948`).

`pane` (`:953-1019`) is the templates `pane` with two edits: the `Naming` arm renders **two**
fields (the name line then the description line, `Tab` moving the cursor), and the body title
carries the estimator:

```rust
        if self.pane == Pane::Body {
            return (
                format!(
                    " {name} v{} (head v{}) ~{} tok ({}) ",
                    shown.version,
                    head.version,
                    TokenEstimator::DEFAULT.estimate(&shown.body),
                    TokenEstimator::DEFAULT.id,
                ),
                shown.body.lines().map(|line| Line::styled(line.to_owned(), theme.base)).collect(),
            );
        }
```

and the diff arm is `diff::unified` / `diff::lines` with the labels
`format!("{name} {label}")` and `format!("{name} v{}", shown.version)` — the Templates view's
labels byte for byte (`pane`, `:1005-1014`), and the `v{N} has no base to diff against` fallback.

**Keymap** (`on_browse_key`, `plain(&key)` gate, `templates.rs:470-498`'s shape):

| Key | Action | Notes |
|---|---|---|
| `j` / `Down` | next row | `move_cursor(true)` |
| `k` / `Up` | previous row | `move_cursor(false)` |
| `r` | re-read | `StoreRequest::Skills(ctx.scope.clone())` |
| `n` | new skill | `Mode::Naming { name: TextField::new(), description: TextField::new(), fixed_name: false }` |
| `e` | edit the selected skill's description | the same arm with `fixed_name: true` and the row's name and description pre-filled |
| `E` | open the body editor on the shown version | `Editor::new(name, Some(head), skill.updated_at, Some(shown), &body)` |
| `,` / `.` | previous / next version | `on_skill_key`, the templates `on_template_key` (`:501-580`) with `D` dropped |
| `b` | base is the shown version | `DiffBase::Version(shown)` |
| `d` | toggle the diff pane; `v1` with no predecessor says so | |
| `J` / `K` / `PageDown` / `PageUp` | scroll the pane | `self.scroll.on_key(key, self.pane_rows.get())` |
| `Tab` | next field, in `Naming` only | shadows the global next-tab, exactly as the templates naming prompt does |
| `Enter` | confirm the form, in `Naming` only | |
| `Esc` | cancel the form; leave the editor after one warning | |
| `Ctrl+S` | save, in `Editing` only | `FieldOutcome::Submit` from the `TextArea` |
| `Ctrl+E` | hand the draft to `$EDITOR` | `Action::EditExternally(ExternalEdit { text, stem: name })` |

Every key above misses `Keymap::default_global` — `q`, `Tab`, `Shift+Tab`, `1`–`9`, `?`, overlay
`Esc` (F-13) — and `w` is avoided. `on_browse_key`'s doc repeats the claim, as `templates.rs:471`
does, and `every_browse_key_misses_the_global_table` asserts it: the view resolves each of its
chords against `Keymap::default_global()` and `KeyScope::Global` and requires `None` for all but
`Tab`.

**The save** (`save`, the templates `save` (`:687-723`) with **`parse` removed**):

```rust
    /// `Ctrl+S` (D78's OQ-18 default). **No parse gate**: a skill body is markdown, not a template,
    /// so there is no byte-offset error to point at and no role's closed set. First match wins: a
    /// write in flight; then the save. A body byte-identical to the head's still appends, exactly
    /// as the Templates view appends unconditionally (PRD D5) — the diff pane makes the no-op
    /// visible *before* the save, which is where the plan puts it.
    fn save(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
            return;
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        let body = editor.area.text().to_owned();
        editor.sent = Some(body.clone());
        self.busy = Some(SAVE_NAME);
        self.notice = Some(Notice::Info(SAVING.to_owned()));
        ctx.request(StoreRequest::SaveSkill {
            scope: ctx.scope.clone(),
            name: editor.name.clone(),
            description: editor.description.clone(),
            body: crate::skills::SkillBody::new(body),
            // D101: both tokens, read from the same snapshot the editor opened on.
            expected: editor.updated_at,
            expected_version: editor.token,
        });
    }
```

`on_reply` matches `Skills` / `SkillsStale` and the two `Failed` arms exactly as the templates
`on_reply` (`:331-376`) does, with `land_save` and the stale arm's token move:

```rust
            StoreReply::SkillsStale(snapshot) => {
                if !in_scope(snapshot, ctx) { return; }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                if self.busy == Some(SAVE_NAME) {
                    self.busy = None;
                    if let Mode::Editing(editor) = &mut self.mode {
                        editor.sent = None;
                        if let Some(entry) = snapshot.named(&editor.name) {
                            // The draft stays; both tokens move to the head the user has now been
                            // told about, so the next `Ctrl+S` is a deliberate overwrite-by-append.
                            editor.token = entry.versions.last().map(|row| row.version);
                            editor.updated_at = Some(entry.updated_at);
                            self.notice = Some(Notice::Error(skill_changed_elsewhere(
                                editor.token.unwrap_or_default(),
                            )));
                        }
                    }
                }
                self.clamp_cursor();
            }
```

`land_save` is the templates `land_save` (`:824-871`) with the name in place of `(project, name)`
and no `omits_item`/`esc_armed` reset that belongs to the phase rule.

**Notice strings**, the complete set:

| When | Text | Kind |
|---|---|---|
| saved, draft unchanged | `saved v{n}` | Info |
| saved, later edits kept | `saved v{n} \u{2014} later edits kept, Ctrl+S saves them as v{n+1}` | Info |
| a save went out | `saving…` | Info |
| the head moved | `skill_changed_elsewhere(head)` (§4.2) | Error |
| a write in flight | `` `{busy}` is still in flight `` | Error |
| `$EDITOR` returned text | `edited in $EDITOR \u{2014} Ctrl+S saves` | Info |
| `$EDITOR` changed nothing | `no changes` (+ `WAIT_FLAG`) | Info |
| `Esc` over a modified draft | `unsaved changes \u{2014} Esc again discards` | Info |
| nothing read | `skills not read yet` | dim |
| the read was refused | `skills unavailable: {why}` | error |
| nothing under the cursor | `select a skill` | dim |
| `v1` with no predecessor | `v1 has no earlier version` | Info |
| the name rule refused it | the writer's `invalid_skill_name` sentence, verbatim (D100) | Error |

### 4.3 `skills/mod.rs` — the tab

`mod library;` after `mod templates;` (`:8`), `use library::SkillsView;`, and a third field:

```rust
pub struct SkillsTab {
    /// Which of the two views is shown.
    view: View,
    /// The Templates view, alive whichever view is shown: its replies land either way.
    templates: TemplatesView,
    /// The Skills view, alive whichever view is shown, for the same reason.
    library: SkillsView,
}
```

`wants_requests` (`:75-77`, F-12) returns two requests of **two different variants**, which the
staleness index keeps separately:

```rust
    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        // Two variants, one each. **Never** one variant per project: the staleness index keeps only
        // the newest of a variant, so N of one would leave all but one undrawn (F-2, the
        // `Catalogue(Scope)` doc).
        vec![
            StoreRequest::Templates(scope.clone()),
            StoreRequest::Skills(scope.clone()),
        ]
    }
```

`on_scope_change` forwards to both views. `on_key` becomes the per-view form:

```rust
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // An open editor or form owns every key it uses: `l` is a letter there, not a view switch
        // (the chat composer's and the Settings sections' rule). The check is **per view**, because
        // each holds its own `captures_input` — D82's guard is the two of them, not one.
        let capturing = match self.view {
            View::Skills => self.library.captures_input(),
            View::Templates => self.templates.captures_input(),
        };
        if capturing {
            return match self.view {
                View::Skills => self.library.on_key(key, ctx),
                View::Templates => self.templates.on_key(key, ctx),
            };
        }
        match key.code {
            KeyCode::Char('h' | 'l' | '[' | ']') | KeyCode::Left | KeyCode::Right => {
                self.toggle();
                Handled::Consumed
            }
            _ => match self.view {
                View::Skills => self.library.on_key(key, ctx),
                View::Templates => self.templates.on_key(key, ctx),
            },
        }
    }
```

`on_reply` and `on_external_edit` each forward to **both** views (a view drops what is not its
variant), `render` dispatches on `self.view`, and the switch line is **byte-identical**:
`Span::styled(" Skills ", …)`, `"\u{2502}"`, `Span::styled(" Templates ", …)`. `SKILLS_LATER` and
its `render` arm are **deleted** — the Skills view is no longer a stub — and the tab's title stays
`"Skills"`.

### 4.4 T3 hazards, snapshots and the gate

| # | Hazard | Evidence | Fix |
|---|---|---|---|
| **H-27** | A skill row needs three columns where a template row had two, so the list's widths move. | `templates.rs:82-84`: `LIST_WIDTH = 40`, `NAME_WIDTH = 44`; its row is `"  {name:<44} v{head:<3} {role}"`. | `library.rs` has its **own** `LIST_WIDTH = 46` / `NAME_WIDTH = 26`. `templates.rs` is not touched, so the six `templates__*.snap` files cannot move. |
| **H-28** | The switch line and the strip text are shared with the Templates view. | `skills/mod.rs:112-118`; `the_strip_text_is_unchanged` (`templates.rs:905-912`). | Byte-identical, and T3's gate re-runs `--test templates`. |
| **H-29** | A new `SkillsView` is `Default`, so a `Skills` reply arriving before the tab is opened is dropped by a stale view. | `on_reply`'s `in_scope` guard (`templates.rs:1086`). | The same `in_scope` guard, and the same `unavailable` sentence. |
| **H-30** | A `SkillBody` reaching a `Debug` prints the body. | `templates.rs`'s `TemplateBody` and its test. | `skills::SkillBody`; `library.rs` never constructs a `StoreRequest` with a `String`. |
| **H-31** | The `Skill` row's token estimate is computed in `render`, so it is recomputed on every frame. | D84's own arithmetic: 30 short scans of strings already in the snapshot. | Accepted, as D84 states. `TokenEstimator::estimate` (`estimate.rs:114`) is `Copy`, pure, no I/O and no clock. |
| **H-32** | `Tab` in `Mode::Naming` shadows the global next-tab, and a user who presses it to switch workspaces stays. | `keymap.rs::default_global` binds `Tab` globally; the templates naming prompt already does this. | The naming hint says `Tab next field  Enter confirm  Esc cancel`; `Esc` leaves. Same trade as the templates view, and the test `tab_in_the_form_does_not_switch_tabs` pins it. |

New snapshots and the tests that drive them, in `crates/htui/tests/skills.rs` over
`Harness::over(MemStore::demo())`, opening the tab with `2` and settling (the tab opens on the
**Skills** view, so there is no `l` — the templates helper's second key goes away):

| Snapshot | Test | What it pins |
|---|---|---|
| `skills__browse` | `the_skills_view_lists_the_library_with_versions_and_token_estimates` | the two demo rows with `v1`, `always` and `~N`; the pane title carries the head and the estimator id |
| `skills__diff_two_versions` | `any_two_skill_versions_diff` | `diff v1 \u{2192} v2` and the `+` line |
| `skills__editor_help` | `the_editor_lists_its_help` | the body on the left, the help pane on the right — the analogue of `templates__edit_help` |
| `skills__naming` | `the_name_form_takes_a_name_and_a_description` | both fields, `Tab` between them |
| `skills__stale` | `a_save_over_a_moved_head_keeps_the_draft` | the error notice and the kept draft |

plus, without snapshots: `a_save_appends_a_version_and_moves_the_head`,
`an_editor_keeps_the_draft_when_the_scope_changes`,
`the_editor_hands_off_to_a_fake_editor_and_returns_edited` (the `ExternalEditOutcome::Edited` path,
and **no** parse: a skill body is markdown, so there is no byte-offset error to point at),
`a_new_skill_starts_at_version_one`,
`every_browse_key_misses_the_global_table`,
`the_six_template_snapshots_do_not_move` — each of the six `templates__*.snap` files is read with
`include_str!` and asserted not to contain `"┌ Skills"`, with the reason in the message (the
coupling HANDOFF warns about, made into a test).

```bash
env $PGT RUST_BACKTRACE=0 cargo test -p htui --all-features --test skills -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui --all-features --test templates -- --test-threads=2
cargo insta review      # every skills__*.snap, named in the commit message
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

**Commits**: (1) red — `library.rs` with `todo!()` bodies, the mod changes, the tests and the
snapshot stubs; (2) green with the five snapshots.

---

## 5. T4: the attachments matrix, the activation form and the language map (D82, D83, D100)

**Files**: `crates/htui/src/ui/tabs/skills/matrix.rs` (new; `mod matrix;` added to
`skills/mod.rs` by T4 itself, now that `matrix.rs` exists), `crates/htui-core/src/model/language.rs`
(new, `pub mod language;` at `model/mod.rs:88` and one `pub use` arm), `crates/htui/tests/skills_matrix.rs`
(new), `crates/htui/tests/snapshots/skills_matrix__*.snap` (new).

### 5.1 `matrix.rs` — the state struct

```rust
/// The attachments matrix (MOD-9 milestone 3, D82): one column per skill, one row per level — the
/// global row above the projects and the phases — plus the activation form, the language expansion
/// and the repo picker. Holds no store handle and no `UserId` (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct MatrixView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<SkillsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`.
    unavailable: Option<String>,
    /// The highlighted **skill**, an index into [`skills`](MatrixView::skills).
    cursor: usize,
    /// The highlighted **level**: the global row, a project row, or a phase row.
    axis: Axis,
    /// The activation form, open over the selected `(skill, level)` cell; `None` is the list.
    form: Option<Form>,
    /// The pane's first drawn row.
    scroll: Scroll,
    /// The pane's rows at the last draw.
    pane_rows: Cell<usize>,
    /// The write in flight, by `StoreRequest::name`.
    busy: Option<&'static str>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
    /// The editor's last drawn height.
    page: Cell<u16>,
}

/// The three axes of the matrix, in the order the rows are drawn: the global row, then each scope
/// project, then each of that project's phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    /// `project_id IS NULL`: every project.
    Global,
    /// One project's row, and, under it, its phases.
    Project(ProjectId),
    /// One phase's row.
    Phase(ProjectId, PhaseId),
}

/// The activation form, open over one `(skill, level)` cell.
#[derive(Debug)]
struct Form {
    /// The cell being edited.
    skill_id: SkillId,
    /// Its level, so the write carries the right `(project_id, phase_id)`.
    axis: Axis,
    /// The winning attachment's `updated_at`, the CAS token (`None`: no row yet).
    token: Option<DateTime<Utc>>,
    /// `always`, `glob` or `off`.
    activation: Activation,
    /// Follows the latest, or pins a version the skill has.
    pin: Pin,
    /// The **typed** globs, one per line — the languages are unioned at save, not here.
    globs: TextField,
    /// The languages, one per line, each of which [`LANGUAGE_GLOBS`] expands at save.
    languages: TextField,
    /// `skill_binding.position`, typed.
    position: TextField,
    /// The `<repo>:` the repo picker wrote, or `None` for a bare glob.
    qualifier: Option<String>,
    /// Which field `Tab` is on, for the hint's `L{line}:C{col}`.
    field: usize,
}

/// Whether an attachment follows the latest version or pins one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pin {
    /// `pinned_version = NULL`: the latest version is in force.
    Latest,
    /// `pinned_version = Some(n)`, cleared back to [`Pin::Latest`] by the next `p`.
    Version(i32),
}
```

**Layout.** The same `render` / `render_browse` skeleton as `library.rs`, and the same
`LIST_WIDTH`/`pane` split. The **list** is a `format!`-packed row per skill in a bordered `Block`
titled `" Attachments "`, which is D82's amended claim: three files use `ratatui::widgets::Table`
(`settings/agents.rs:50`, `backlog/detail/documents.rs:7`, `backlog/detail/graph.rs:11`) and **all
three have a row cursor, none a selected cell** — so a row-packed block, not a table, and the
right-hand `pane()` carries the selected cell's detail. One row per skill:

```rust
    /// One matrix line, `format!`-packed: `<name>` then one cell per level, each `·` for no
    /// attachment, or `v1` / `v1g` / `·` — the version in force (`?` for a pin with no version)
    /// and `g` for a `glob` activation, so a column is read left to right as the three levels.
    fn cells(&self, entry: &SkillSummary) -> Vec<String> { … }
```

**Keymap** (`on_browse_key`, same `plain` gate, F-13's collision check):

| Key | Action |
|---|---|
| `j` / `Down` | next axis: global → each project → each of that project's phases |
| `k` / `Up` | previous axis |
| `g` / `G` | the global row / the last axis |
| `a` | attach at the selected level — `SetSkillBinding` with the cell's current values |
| `e` | open the form on the selected cell |
| `x` | detach the selected cell — `RemoveSkillBinding` with the cell's `updated_at` |
| `A` | cycle the activation `always → glob → off → always`, on the cell or in the form |
| `p` | cycle the pin: latest → v1 → v2 … → latest |
| `r` | re-read |
| `J` / `K` / `PageDown` / `PageUp` | scroll the pane |
| `Tab` | next field, in the form only |
| `Enter` | save the form |
| `Esc` | close the form, or cancel a `p` cycle mid-flight |

`g`, `G`, `a`, `e`, `x`, `A`, `p` all miss the global table (F-13); `w` is avoided.

**Notices**, on top of T3's shared set:

| When | Text | Kind |
|---|---|---|
| an attachment applied | `attached <name> at <level>` | Info |
| detached | `detached <name> from <level>` | Info |
| a spent token on a write | `this attachment changed elsewhere; it is unchanged` | Error |
| `glob` with no glob | `a glob attachment needs at least one glob` | Error |
| a qualified glob on a global row | `a global attachment cannot name a repo; use a project or phase row` | Error |
| a glob the matcher refuses | `GlobError`'s `Display`, verbatim (D100) | Error |
| a pin with no such version | `skill_binding.pinned_version {n} names no version of skill \`{name}\`` | Error |
| a language not in the map | `no language named \`{name}\` in the map` | Info (not a refusal: an unknown name contributes nothing and is said so) |

The last two sentences are the **same strings** `traits.rs` builds, re-spelled for the row the user
sees, because the view refuses before it sends (D100) and the writer refuses the same thing if the
view is bypassed.

### 5.2 `crates/htui-core/src/model/language.rs` (new — D83)

`pub mod language;` at `model/mod.rs:88`, between `pub mod kind;` (:87) and `pub mod link;` (:88),
plus one `pub use language::LANGUAGE_GLOBS;` arm.

```rust
//! The language → globs map of a `glob` attachment (ANA-22 §6 item 5, §9; plan D83).
//!
//! **Data, expanded at save.** The activation form shows the effective globs — the typed globs
//! unioned with every named language's expansion — before the save, and the save writes that union
//! into `skill_binding.globs`, with `languages` keeping what was typed. A later change to this
//! table therefore never changes a saved attachment, and the matcher reads only `globs` and never
//! this file. That is the whole reason it is data and not code.

/// Language name to the globs it expands into, **sorted by name** so a rendered list and a stored
/// `TEXT[]` are stable and a test can assert the order.
///
/// Fourteen entries, ANA-22 §9's list. The seed is deliberately small: a language not named here
/// contributes nothing, and the activation form says so rather than refusing.
pub const LANGUAGE_GLOBS: &[(&str, &[&str])] = &[
    ("c", &["**/*.c", "**/*.h"]),
    ("cpp", &["**/*.cc", "**/*.cpp", "**/*.cxx", "**/*.hh", "**/*.hpp", "**/*.hxx"]),
    ("csharp", &["**/*.cs"]),
    ("go", &["**/*.go"]),
    ("java", &["**/*.java"]),
    ("javascript", &["**/*.js", "**/*.cjs", "**/*.mjs"]),
    ("markdown", &["**/*.md", "**/*.markdown"]),
    ("python", &["**/*.py"]),
    ("rust", &["**/*.rs"]),
    ("shell", &["**/*.sh", "**/*.bash", "**/*.zsh"]),
    ("sql", &["**/*.sql"]),
    ("toml", &["**/*.toml"]),
    ("typescript", &["**/*.ts", "**/*.tsx"]),
    ("yaml", &["**/*.yaml", "**/*.yml"]),
];

/// The effective globs of an attachment: `typed` first, then every named language's patterns in
/// [`LANGUAGE_GLOBS`]'s order, de-duplicated and **in that order** — which is the order the form
/// shows before the save and the order the store receives, so the two cannot disagree.
///
/// A `name` the map does not hold contributes nothing and is **not** an error: the map is a
/// convenience and the writer's rule is the dialect, not the map.
#[must_use]
pub fn effective_globs<'a>(typed: impl IntoIterator<Item = &'a str>, languages: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |glob: &str| {
        if !out.iter().any(|seen| seen == glob) {
            out.push(glob.to_owned());
        }
    };
    for glob in typed {
        push(glob);
    }
    for language in languages {
        if let Some((_, patterns)) = LANGUAGE_GLOBS
            .iter()
            .find(|(name, _)| *name == language.trim())
        {
            for pattern in *patterns {
                push(pattern);
            }
        }
    }
    out
}

/// The names [`LANGUAGE_GLOBS`] holds, in order: the activation form's completion list.
#[must_use]
pub fn languages() -> Vec<&'static str> {
    LANGUAGE_GLOBS.iter().map(|(name, _)| *name).collect()
}
```

**Tests** in `language.rs`'s `mod tests` (the plan's): `the_map_is_sorted_and_has_no_duplicate` —
fourteen entries, strictly ascending names, no two pattern lists equal; and, the real one,
`every_entry_is_parsable_by_the_matcher` — every pattern in every entry is
`crate::prompt::glob::compile(p)` is `Ok`, with the failing pattern and the `GlobError` in the
message. That is the cross-check between T1's matcher and T4's data, and it would fail loudly if a
language were ever given a `**` in the wrong place.

### 5.3 The form's pre-checks (D100)

Before `SetSkillBinding` is requested, in the same order `skill_binding_refusal` uses:

```rust
    /// MOD-9 D100: the form's own copy of D78's five rules, refused **before** the request is
    /// sent so a maintainer sees the mistake without a round trip. Every sentence is the one the
    /// store would have produced — the glob cases are `GlobError`'s own `Display` — so the two
    /// halves of the milestone say the same thing about the same bytes, and T1's conformance case
    /// 4 is what pins the store's half.
    fn refusal(&self, form: &Form) -> Option<String> { … }
```

`Form::globs` and `Form::languages` are one `TextField` each, split on `\n` and trimmed; the
`qualifier` is prepended to **every** typed glob when it is `Some`, so the effective globs the form
shows are the globs the store receives.

### 5.4 T4 hazards, snapshots and the gate

| # | Hazard | Evidence | Fix |
|---|---|---|---|
| **H-33** | The map's order and the union's order drift, so a re-save changes `globs` for no reason. | D83's "the map is data expanded at save". | `effective_globs` is the **one** function both the form's preview and the save call; `the_language_map_expands_in_the_union_s_order` pins the order. |
| **H-34** | A language name with surrounding whitespace from a text field contributes nothing silently. | The field splits on `\n` and a maintainer types a trailing space. | `effective_globs` trims each name, and the form's preview shows what it will send. |
| **H-35** | `mod matrix;` committed before `matrix.rs` exists does not compile (R-30). | The plan's Intersections section. | T3 lands first and owns `mod library;`; T4 adds `mod matrix;` in the same commit as the file. |
| **H-36** | The matrix's two views draw a row-packed block while the plan's amended D82 mentions `Table`. | The fact-check found three `Table` users, all with a row cursor. | A `format!`-packed row per skill in a bordered `Block`, the templates list's own pattern (`templates.rs:876-949`), with the detail in `pane()`. |
| **H-37** | An unknown language silently contributes nothing. | D83: an unknown name is not a refusal. | `effective_globs` says so, and the form's notice names the language it did not know. |

New snapshots, in `crates/htui/tests/skills_matrix.rs`:

| Snapshot | Test | What it pins |
|---|---|---|
| `skills_matrix__overview` | `the_matrix_shows_a_global_row_above_the_projects_and_the_phases` | the axis order, one column per skill, `·` for an empty cell |
| `skills_matrix__effective_globs` | `the_language_map_expands_into_the_effective_globs_shown_before_the_save` | `shell` → its three patterns, unioned with a typed glob, in `effective_globs`' order (F-10: the plan said `rust` → two; `rust` has one) |
| `skills_matrix__form` | `the_activation_form_shows_the_effective_globs_and_the_languages` | both fields and the preview block |
| `skills_matrix__unbound` | `unbinding_removes_the_row_and_the_matrix_shows_it_gone` | the cell back to `·` |

plus: `attaching_at_a_level_writes_one_row_and_the_matrix_re_reads`,
`a_pin_follows_latest_until_it_is_set_and_cleared`,
`activating_glob_without_globs_is_refused_before_it_is_sent`,
`a_qualified_glob_is_refused_on_a_global_row`,
`a_glob_the_matcher_cannot_compile_is_refused_before_it_is_sent` — `src/**x/*.rs`,
`a/{b,{c,d}}/x.rs` and `x{,.txt}`, each `Notice::Error` and each dispatching **no** request
(asserted through the harness's `Emit` log, the `sent(harness)` helper `templates.rs:1137` uses) —
`a_repo_picker_writes_the_qualifier_from_the_project_s_repos`,
`a_spent_token_leaves_the_row_as_it_is_and_says_so`,
`every_matrix_browse_key_misses_the_global_table`.

```bash
env $PGT RUST_BACKTRACE=0 cargo test -p htui --all-features --test skills_matrix -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui --all-features --test skills -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui --all-features --test templates -- --test-threads=2
RUST_BACKTRACE=0 cargo test -p htui-core --all-features --lib model::language -- --test-threads=2
cargo insta review
cargo clippy -p htui -p htui-core --all-features --all-targets -- -D warnings
```

**Commits**: (1) red — `language.rs` with its table and tests, `matrix.rs` with `todo!()` bodies, the
`mod` line, the tests and the snapshot stubs; (2) green with the four snapshots.

---

## 6. T5: the clone gap (D85, OQ-16, OQ-17; F-7/D98)

**Files**: the plan's T5 list — `crates/htui-orch/src/graph.rs`, `crates/htui-core/src/store/mem.rs`,
`crates/htui-store/src/pg/write.rs`, `crates/htui-store/.sqlx/` (one file **replaced**),
`crates/htui-orch/src/engine.rs` — **plus** `crates/htui-orch/src/fake.rs`,
`crates/htui-store/src/backend.rs` and `crates/htui/src/run_worker.rs`, because F-7's
`GraphSource::phase_attachments` has four delegations and one of them is in the `htui` crate.

### 6.1 `NewStepGraph` and the nine sites

`crates/htui-core/src/model/kind.rs:152-161` gains the field after `description`, so the
struct-literal order and the JSON order are stable, and the note at `:165` ("`is_override` is not
here") moves to the field's own doc:

```rust
/// Arguments of [`crate::store::WriteStore::create_step_graph`]. The graph lands with no phases;
/// [`crate::store::WriteStore::create_phase`] adds them one row at a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewStepGraph {
    /// `step_graph.id`, minted client-side as a UUIDv7.
    pub id: StepGraphId,
    /// `step_graph.project_id`.
    pub project_id: ProjectId,
    /// `step_graph.name`, unique within the project.
    pub name: String,
    /// `step_graph.description`.
    pub description: String,
    /// `step_graph.is_override` (ANA-2 §4.1): a per-item clone, hidden from the graph list.
    /// **Not** here in milestone 2 — both stores hard-coded `false` (`mem.rs:2539` and the
    /// Postgres `INSERT`), which is the defect this milestone closes. Only
    /// `htui_orch::graph::override_graph` sets it `true`; every other construction site is a
    /// maintainer- or fixture-created graph and says `false` (plan D85, OQ-16).
    pub is_override: bool,
}
```

The **nine** construction sites, all of which gain `is_override: <value>` (R-28's compile error is
the point of the field):

| # | `file:line` | Function | Value | Why |
|---|---|---|---|---|
| 1 | `crates/htui/src/catalogue.rs:190` | `catalogue::serve`, `StoreRequest::CreateGraph` | `false` | a maintainer-created graph is never a clone; the `is_primary` demotion and the whole catalogue read `is_override` as false |
| 2 | `crates/htui-orch/src/graph.rs:410` | `override_graph` | **`true`** | the only clone |
| 3 | `crates/htui-orch/src/conformance.rs:824` | `repoint` | `false` | a fixture graph |
| 4 | `crates/htui-orch/src/conformance.rs:884` | `insert_verify_phase` | `false` | ditto |
| 5 | `crates/htui-core/src/store/conformance.rs:3254` | `step_graph_and_phase_round_trip` | `false` | ditto |
| 6 | `crates/htui-core/src/store/conformance.rs:3264` | the same function | `false` | ditto |
| 7 | `crates/htui-orch/src/engine.rs:6152` | `Harness::repoint` | `false` | a test harness |
| 8 | `crates/htui-orch/tests/fixtures.rs:54` | `feature_with_verify_snapshot_matches` | `false` | the graph the golden digests were taken against; a `true` here would move `FEATURE_TOPOLOGY` and the snapshot |
| 9 | `crates/htui-orch/tests/gix_isolator.rs:338` | `repoint` | `false` | a fixture |

The five `WriteStore` implementors forward `new` by value and do not break — that is the whole of
R-28's mitigation and it is why the threshold check mattered.

`mem.rs:2511`'s `create_step_graph` changes `is_override: false` (`:2539`) to `is_override:
new.is_override`; `pg/write.rs:2203`'s `INSERT` gains the column:

```sql
            INSERT INTO step_graph (id, project_id, name, description, is_override)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id         AS "id: StepGraphId",
                      project_id AS "project_id: ProjectId",
                      name,
                      description,
                      -- Inserted, not appended: see `step_graph_row` in `pg/read.rs`.
                      is_override,
                      created_at,
                      updated_at
```

with `new.is_override` as the fifth bind. **The query's text changes, so its `.sqlx` file is
replaced, not added: 268+N before and after, net 0** (D99, H-38).

### 6.2 `GraphSource::phase_attachments` and its four delegations (F-7, D98)

`crates/htui-orch/src/graph.rs`, the trait's method list, after `bound_skills`:

```rust
    /// The **raw** `skill_binding` rows of one phase, nothing resolved and nothing collapsed
    /// (MOD-9 D85).
    ///
    /// Distinct from [`bound_skills`](GraphSource::bound_skills) in the reason the clone needs it:
    /// a [`BoundSkill`] has resolved away the fields a copy must carry verbatim — `languages` above
    /// all, and the ids and `updated_at` the two writers take as tokens.
    ///
    /// Inherent on both stores and on `Backend` (`mem.rs`, `pg/read.rs`, `backend.rs`, landed in
    /// T1) for `prompt_template`'s reason: `skill*` is not mirrored.
    ///
    /// # Errors
    /// The backend's own failures; offline, [`StoreError::Unreachable`](htui_store::StoreError::Unreachable)
    /// with `PROMPT_ON_SERVER_ONLY`.
    async fn phase_attachments(
        &self,
        project: ProjectId,
        phase: PhaseId,
    ) -> Result<Vec<SkillBinding>>;
```

| Implementor | Site | Body |
|---|---|---|
| `graph.rs::TestSource` | beside its `bound_skills` delegation (`graph.rs:781-819`) | `self.store.phase_attachments(project, phase).await` |
| `fake.rs::MemStore` | beside `fake.rs:843-875` | `self.phase_attachments(project, phase).await` — the inherent method wins, as its doc says |
| `fake.rs::FakeGraphSource` | beside `fake.rs:954-986` | `GraphSource::phase_attachments(self.store, project, phase).await` |
| `run_worker.rs::BackendGraphs` | beside `run_worker.rs:2375` | `self.0.phase_attachments(project, phase).await`, with `SkillBinding` imported |

`fake.rs`'s "the store answers the source without recursing" test (`fake.rs:2100-2143`) is
**extended with the sixth read** rather than given a second test, so its "calls all N through the
trait" doc stays true — milestone 2's D66.

### 6.3 `override_graph`'s copy loop

`crates/htui-orch/src/graph.rs:399-454`. The graph literal gains `is_override: true`, and the phase
loop gains the copy:

```rust
    for row in &resolved.phases {
        let phase_id = PhaseId::new();
        store
            .create_phase(&StepGraphPhase {
                id: phase_id,
                graph_id: clone.id,
                ..row.phase.clone()
            })
            .await?;
        // MOD-9 D85: the source phase's own level-`Phase` attachments, copied onto the cloned
        // phase with fresh ids and the writer's `None` token — the cloned phase has no attachment
        // of its own, so nothing is replaced and no CAS can fail.
        //
        // **Project-level and global rows are deliberately not copied.** They are keyed by
        // `(skill_id, project_id, phase_id)` and the clone resolves the *same* project, so a copied
        // project row would shadow the original project's and the item's phases would stop
        // inheriting it. That is exactly the objection `override_clone_leaves_bindings_alone`
        // recorded, and the copy keeps its force for the rows it is about: a **phase** row is
        // always the most specific attachment, so a phase row copied onto the cloned phase is the
        // point of an override.
        for binding in source.phase_attachments(item.project_id, row.phase.id).await? {
            store
                .set_skill_binding(
                    NewSkillBinding {
                        id: SkillBindingId::new(),
                        skill_id: binding.skill_id,
                        project_id: Some(item.project_id),
                        phase_id: Some(phase_id),
                        // Verbatim, `languages` included: they are display only, and a saved
                        // attachment's languages never change (D83).
                        pinned_version: binding.pinned_version,
                        position: binding.position,
                        activation: binding.activation,
                        globs: binding.globs,
                        languages: binding.languages,
                    },
                    // The cloned phase carries none, so this is a create and the token is "I expect
                    // no row" (D101's reading of `None`).
                    None,
                )
                .await?;
        }
    }
```

One code path and one conformance case, as D85 requires: the copy is done with the new writer, not
a bulk path.

### 6.4 `OVERRIDE_SKILLS_NOTE`

`crates/htui-orch/src/engine.rs:88-89`. OQ-17's default, and the constant stays a constant because
the **source graph's name is not reachable** from the note: `phase_skills` (`:5126`) holds
`snapshot.graph`, and for an override that snapshot's graph *is* the clone. The source's name is in
the clone's own `description` — `override_graph` writes
`"Per-item override of \`{source}\` for {key}"` — so the note and the description together name it,
which is why a `String`-returning function is not needed.

```rust
/// MOD-9 D85: an override graph's phases carry a **copy** of the source graph's phase-level
/// attachments, made when the clone was written; they do not follow a later change to the source
/// graph, and the two graphs' rows are visible side by side in the Skills tab's matrix. The source
/// graph is named in this override's own `description`.
const OVERRIDE_SKILLS_NOTE: &str =
    "skills: this override's phase attachments were copied from the graph it was made from, and \
     do not follow later changes there";
```

`phase_skills`'s own doc (`:5126-5131`) is amended: "…and every phase of an override graph get a
note rather than a silent loss of their phase-level attachments (plan R-15)" becomes "…and every
phase of an override graph carries a **copy** of the source phase's attachments (D85), which the
note says, because a copy that silently stopped following the source would read as a bug."

### 6.5 Tests, hazards and the gate

The plan's T5 test list, with the two replacements it names:

| Test | Site | What it pins |
|---|---|---|
| `override_clone_copies_phase_attachments` (**replaces** `override_clone_leaves_bindings_alone`, `graph.rs:1459`) | `graph.rs` | the cloned `implement` resolves the same attachments the original's did; the cloned phases carry **no** project-level rows (the old test's own rationale, kept as its own assertion, with the message naming the `UNIQUE NULLS NOT DISTINCT` doubling it prevents); the original graph's rows are untouched (`bound_skills(PROJECT_HTUI, Some(PHASE_HTUI_IMPLEMENT))` still equals `bound_before`) |
| `an_override_graph_is_marked_as_one` | `graph.rs` | `store.step_graphs(PROJECT_HTUI)` shows `is_override` on the clone and not on the seeded graphs |
| `an_override_graph_notes_that_its_attachments_were_copied` (**replaces** `an_override_graph_notes_the_clone_gap`, `engine.rs:12924`) | `engine.rs` | the hand-patched snapshot (`let mut snapshot = …; snapshot.graph.is_override = true;`) keeps working and `spec.notes` contains `OVERRIDE_SKILLS_NOTE`'s **new** text |
| `create_step_graph_honours_its_new_is_override_field` | `mem.rs` | a `NewStepGraph { is_override: true, … }` reads back `is_override: true`, and `step_graph_rows`/the `StepGraph` it returns agree |
| `demo_graphs_are_not_overrides` (**new**, H-39) | `fixtures.rs` | every `demo_data().graphs` row has `is_override: false`, because `load_demo`'s `step_graph` `INSERT` names six columns and Postgres reads the default — the `demo_skill_rows_use_the_column_defaults` rule, one table over |
| `phase_attachments_reads_the_raw_rows_of_one_phase` | `graph.rs` | the delegation: `G::phase_attachments(PROJECT_HTUI, PHASE_HTUI_IMPLEMENT)` equals the seeded `BINDING_HTUI_IMPLEMENT_RUST_STYLE` row, `languages` included, and is **not** `bound_skills`' resolved shape |
| `the_store_answers_the_source_without_recursing` (extended) | `fake.rs:2100-2143` | the sixth read, alongside the five |

| # | Hazard | Evidence | Fix |
|---|---|---|---|
| **H-38** | `create_step_graph`'s `.sqlx` file is **replaced**, so a naive count check reads as a loss. | The `INSERT`'s text changes, so its hash changes. | D99: 268+N before and after; the commit names the replaced file. |
| **H-39** | `load_demo` writes no `is_override`, so a fixture graph that set it `true` would load differently into the two stores. | `pg/demo.rs`'s `INSERT INTO step_graph (id, project_id, name, description, created_at, updated_at)` — six columns; `fixtures.rs:2024`'s `demo_skill_rows_use_the_column_defaults` is the same rule for `skill*`. | `demo_graphs_are_not_overrides`, and every demo graph keeps `false`. |
| **H-40** | The copied row's `id` is fresh, so a second clone of the same source creates a second set — correct, but a test comparing two clones' row ids would see no match. | `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)` is per *phase*, and two clones have two different phases. | `override_clone_copies_phase_attachments` compares by `(skill_id, position, activation, globs, languages)`, not by `id`, with the reason in the message. |
| **H-41** | `an_override_graph_is_marked_as_one` needs `store.step_graphs(project)`, and the clone's name is `"{key}-override"` — which `graph.rs` also checks. | `override_graph` (`:399-454`) and the milestone-2 test both assert the name. | One assertion, one place; the name is unchanged by T5. |
| **H-42** | A fixture or seed whose `false` a test asserts moves. | Sites 8 and 9 of §6.1 are fixtures with golden snapshots. | Both say `false`, which is what they say today, so no golden moves. R-28's compile error is at every site. |
| **H-43** | `DeleteReach.skill_bindings == 3` and `a_global_row_survives_project_delete` must not move. | `fixtures::skill_bindings()` (`:574`) has exactly three rows and no conformance case calls `override_graph`. | T5 adds no fixture row and no case clones a graph, so the count is unchanged; the two tests are listed in §8 as tests that must not move. |

```bash
# .sqlx re-prepared: create_step_graph's text changed
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_m3 cargo sqlx prepare -- --all-features --all-targets)
ls crates/htui-store/.sqlx | wc -l                       # 268+N, unchanged from T1
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_m3 cargo sqlx prepare --check)
RUST_BACKTRACE=0 cargo test -p htui-core --all-features -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui-store --all-features -- --test-threads=2
RUST_BACKTRACE=0 cargo test -p htui-orch --all-features -- --test-threads=2
env $PGT RUST_BACKTRACE=0 cargo test -p htui --all-features -- --test-threads=2
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

**Commits**: (1) red — `NewStepGraph.is_override` with all nine sites annotated and the two stores'
bodies `todo!()`, the new tests, the note's new text pinned red; (2) green core + Postgres —
`mem.rs`, `pg/write.rs`, `.sqlx`, `demo_graphs_are_not_overrides`; (3) green orch — `GraphSource`,
its four delegations, `override_graph`'s copy loop, `OVERRIDE_SKILLS_NOTE` and the two replaced
tests.

---

## 7. Cross-task contracts

| Producer | Contract | Consumer |
|---|---|---|
| T1 | `NewSkill` / `NewSkillVersion` / `NewSkillBinding`, `SkillEntry`, `SkillAttachmentRow`, `Skill::name_is_valid`; `WriteStore::{upsert_skill, add_skill_version, set_skill_binding, remove_skill_binding}`; `invalid_skill_name`, `skill_key`, `skill_version_key`, `skill_binding_key`, `skill_refusal`, `skill_binding_refusal`, `skill_pin_refusal`; `MemStore`/`PgStore`/`Backend::{skill_library, skill_attachments, phase_attachments}` | T1, T4, T5 |
| T1 | `prompt::glob::{compile, matched_skills, Pattern, GlobError}` and the dialect table | T1's writer, T2, T4's form |
| T1 | `StoreRequest::{Skills, SaveSkill, SetSkillBinding, RemoveSkillBinding}`, `StoreReply::{Skills, SkillsStale}`, `SkillsSnapshot`, `SkillSummary`, `SkillBody`, `skills::{snapshot, serve, REQUEST_NAMES, READ_NAME}` | T3, T4 |
| T2 | `ExcerptSet.listed`, `PromptSpec::{skill_files, skill_matches}`, `ChoiceReason::{Matched, NoMatch}`, `SkillChoice.matched`, `select(candidates, placed, matches)`, `Isolator::changed_paths`, `Cli::changed_paths`, `PassInput.changed_paths`, `0008_skill_match.sql` | T3 (the snapshot's `always` reasons are unchanged), T5 (the note) |
| T3 | `skills/mod.rs`'s per-view `captures_input` and the two-variant `wants_requests` | T4 |
| T5 | `NewStepGraph.is_override`, `GraphSource::phase_attachments` | — |

T1 ∩ T2 = {`model/skill.rs`} (T1 adds the structs and the name rule, T2 rewrites `select` and the
two choice types) and, after F-1, {`prompt/mod.rs`} (T1 adds the `mod glob;` line, T2 the two
`PromptSpec` fields). T1 lands whole; T2 lands whole. **T3 ∩ T4** = {`skills/mod.rs`} as a
*sequencing* dependency, so they run serially. **T5 ∩ T1/T2/T3/T4** = ∅, except that T5's
`phase_attachments` reads were written in T1 by design (D98).

---

## 8. Count pins

| Pin | Where | Before → after |
|---|---|---|
| applied migrations | `migrations.rs:81` | `vec![1..=7]` → `vec![1..=8]` |
| `Pending` | `migrations.rs:877`; `tests/connect.rs:102`, `:204` | 7 → **8** |
| "seven embedded migrations" | `migrations.rs:878`; `connect.rs:103`, `:119`, `:205` | seven → **eight** |
| commented columns | `migrations.rs:424`, `:454`; `MOD7_COLUMN_COMMENTS` (4); `MOD9_COLUMN_COMMENTS` (5) | **34, unchanged** (D75: `0008` re-comments a column already in the list) |
| `TABLES` | `migrations.rs:101-105` | **39, unchanged** |
| `WriteStore` methods | `traits.rs:259-1333` | 79 → **83** |
| store `CASES` | `conformance.rs:43-121` | 77 → **81** |
| `EXPECTED_CASES` | `pg_conformance.rs:19` | 77 → **81** |
| `READ_CASES` | `conformance.rs:303-318` | **14, unchanged** |
| `StoreRequest` | `store_worker.rs:95-584` | 69 → **73** |
| `StoreReply` | `store_worker.rs:674-859` | 40 → **42** |
| `MIRRORED_TABLES` | `htui-store/tests/cache.rs:1392-1393` | **21, unchanged** |
| `.sqlx` files | `ls crates/htui-store/.sqlx \| wc -l` | 268 → **268+N** (T1), unchanged through T4, unchanged through T5 (one replaced) |
| `SkillChoice` keys | `model/skill.rs:776-783` | 7 → **8** (adds `matched`) |
| trim-record top-level keys, `v` | `prompt_digest.rs:979-999` | **13 and 2, unchanged** |
| `STAND_INS` | `preview.rs:61-70` | **8, unchanged** (one sentence re-worded) |
| `DeleteReach.skill_bindings` | `conformance.rs:2421`; `skill_attachments.rs`'s `a_global_row_survives_project_delete` | **3, unchanged** |
| `Activation` CHECK list | `model/mod.rs`'s `activation_matches_check_list` | **3, unchanged** |
| `NewStepGraph` construction sites | §6.1's nine | 9 → 9, each gaining a field |

### 8a. Tests that must NOT move

- `READ_CASES` 14, `MIRRORED_TABLES` 21, `TABLES` 39, the commented-column total 34.
- The **six** `templates__*.snap` files, byte-identical, and `the_strip_text_is_unchanged`
  (`templates.rs:905`) — `library.rs` has its own widths (H-27) and the switch line is untouched.
- `demo_skill_rows_use_the_column_defaults` (`fixtures.rs:2024`) and, added by T5,
  `demo_graphs_are_not_overrides` for the same rule one table over.
- `the_checks_refuse_a_phase_row_without_a_project_and_glob_without_globs` and its four constraint
  names, `a_global_row_survives_project_delete` (`skill_bindings == 3`).
- `prompt_digest.rs`'s 13 top-level keys and `v == 2`; `phase_beats_project_beats_global`,
  `off_at_a_narrower_level_wins_…`, `a_missing_pin_on_the_winner_does_not_fall_back`,
  `level_of_a_binding_follows_its_nullable_keys`, `collapse`'s two tests,
  `pinned_version_wins_over_latest`.
- Every demo prompt digest: `b9fb821f…` (FEAT-1), `d68f3503…` (ANA-2) — no demo attachment is
  `Glob`, so no rendered section changes (H-19).
- The three template conformance cases, unchanged in spelling and outcome.
- `inherent_prompt_reads_answer_the_fixture` — it compares the two stores over `demo_data()`, and
  every demo row keeps its column defaults.
- The `htui-orch` and `htui-agent` `CASES` counts (70 each).

---

## 9. Merge order and the workspace gate

T1, then T2, then T3, then T4, then T5 — each merged with its own crates' gates re-run on the real
tree before the next starts. After the fifth:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
env $PGT RUST_BACKTRACE=0 cargo test --workspace --all-features --no-fail-fast -- --test-threads=2
# Anything that touches the keyring fake, single-threaded.
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_m3 cargo sqlx prepare --check)
ls crates/htui-store/.sqlx | wc -l                       # 268+N, the number T1 recorded
git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

---

## 10. Risks (continuing the plan's R-25–R35)

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| **R-36** | We own the glob dialect forever, and a syntax a maintainer expects is refused at save time rather than matched. | Medium | Low | D70's price, stated once. The dialect is five metacharacters, written down in `glob.rs`'s doc, enforced by a test table that is the specification, and refused with a byte position (D103). Extending it is one function and one row of the table. A language or a pattern the tree does not hold is a `Constraint` naming the reason, not a silent no-match. |
| **R-37** | The two CAS tokens of `SaveSkill` are confusing to a maintainer: a save over a moved head can apply the description and not the body. | Low | Medium | D101: the worker short-circuits, so the outcome is `SkillsStale` and the view reloads against the fresh snapshot, which carries the new head. The two tokens cannot diverge in production (only `SaveSkill` writes a version and it upserts first), and case 2 pins a bare append. Documented on `add_skill_version` and on `SaveSkill`. |
| **R-38** | The matrix is one request over the whole scope, so a workspace with many projects reads every attachment on every reload. | Certain | Low | One indexed query, one reply, and the catalogue's own rule says anything else leaves the view undrawn. `skill_attachments` is scoped to `scope.project_ids` and the library read is global but small. |
| **R-39** | The copy in `override_graph` is N phase-level rows per phase, and a graph with many phases pays a read and a write each. | Medium | Low | One `GraphSource::phase_attachments` read per phase and one `set_skill_binding` write per row, in the clone path only, which is a once-per-item action. The read is the same one the engine uses, so there is no new query. |
| **R-40** | Two new UI modules in one tab directory, and a maintainer edits one and breaks the other. | Medium | Low | T3 and T4 run serially (R-30), each gate re-runs `--test templates`, and `the_six_template_snapshots_do_not_move` is a test rather than a review note. `skills/mod.rs` is shared in exactly two places, both of which §4.3 and §5.1 spell out. |

---

## 11. Where this blueprint departs from the plan

Every departure, in one place, with its finding and decision.

| Departure | Plan | Here | Why |
|---|---|---|---|
| `prompt/glob.rs` lands in **T1** | T2's file list | T1, commit 1 (D95, F-1) | D78's writer needs `compile` and T1 lands first. The module depends only on `RepoPath` and `BoundSkill`. |
| The three `New*` structs live in `model/skill.rs` | `model/kind.rs` (D77's file list) | D90, F-4 | `kind.rs` is ANA-2's kind/graph/template module; the skill rows are in `skill.rs`, and `model::skill` re-exports them. |
| The read is `skill_attachments(&[ProjectId])` plus `phase_attachments(project, phase)` | D86's `skill_attachments(project, phase)` | D92, F-2 | N requests of one variant leave all but one undrawn; the matrix needs the whole scope in one reply. |
| The model's row type is `SkillAttachmentRow` | D81's `SkillBindingRow` | D94, F-3 | `pg/rows.rs:257` already owns that name. |
| `PromptSpec.skill_matches` is `Option<BTreeMap<…>>` | D73's `BTreeMap` | D97, F-5 | `select` takes an `Option`; a bare map collapses `no_path` and `no_match`. |
| `{…}` is a whole component only | D71 is silent | D96, F-6 | One-level compiled form; nothing in the plan's table needs more. |
| T5 gains `fake.rs`, `backend.rs`, `run_worker.rs` | T5's list names four files | D98, F-7 | `override_graph` has no read; four `GraphSource` delegations, one in the `htui` crate. |
| `.sqlx` is prepared at `0007` in T1, checked at `0008` in T2 | T1 says `0007`, T2 says "`if demo.rs moved`" | D99, F-8 | `0008` lands between the two. |
| The view and the writer refuse a bad glob in the same words | D78 vs T4's test | D100, F-9 | One `GlobError`, one `Display`, two call sites. |
| Case 4's "a NUL name" moves to case 1 | T1's test list | §2.7 | `NewSkillBinding` has no name field. |
| The tier-2 test carries a `glob` candidate | — | H-22 | R-35's guard skips the call when no candidate is `Glob`. |
| T4's language test uses `shell`, not `rust` | "rust → its two patterns" | §5.4, F-10 | `rust` is one pattern under ANA-22 §9. |

---

## 12. Decisions (D90 onward)

| # | Decision |
|---|---|
| **D90** | `NewSkill`, `NewSkillVersion` and `NewSkillBinding` live in `crates/htui-core/src/model/skill.rs` beside the rows they write, not in `model/kind.rs` (amends the plan's T1 file list; D77's "newtype-free structs beside `NewPromptTemplate`" is honoured as a shape description). Re-exported from `model` in name order. |
| **D91** | All three carry their id, minted client-side — `NewItemKind` (`:95`), `NewRepo` (`hierarchy.rs:149`) and `NewPromptTemplate` (`:340`) all say "minted client-side as a UUIDv7". `NewSkillVersion` has no id: its key is `(skill_id, version)` and the store assigns the version, as `append_prompt_template` does. |
| **D92** | Two attachment reads, not one: `skill_attachments(&[ProjectId]) -> Vec<SkillAttachmentRow>` for the matrix, and `phase_attachments(project, phase) -> Vec<SkillBinding>` for the clone. Both inherent on `MemStore`/`PgStore`, both dispatched by `Backend`, both offline-refusing with `PROMPT_ON_SERVER_ONLY`. **Amends D86's signature** (F-2). |
| **D93** | `SkillEntry` and `SkillAttachmentRow` are `pub` model types in `model/skill.rs`, because `Backend`'s inherent returns must be nameable in the `htui` crate. `skills.rs` flattens `SkillEntry` into `SkillSummary` so a view never holds a `created_by` or a `created_at`. |
| **D94** | The model's row type is named `SkillAttachmentRow`, not `SkillBindingRow` (amends D81's spelling, F-3). `pg/rows.rs`'s milestone-2 `SkillBindingRow` is untouched. |
| **D95** | `crates/htui-core/src/prompt/glob.rs` — module, `mod` line and the whole dialect table — lands as **commit 1 of T1**, not in T2 (amends the plan's T2 file list; F-1). D71 and D78 are unchanged. |
| **D96** | `{a,b}` is legal **only as a whole component**. Nesting, an empty alternative, and a brace anywhere but the whole component are refused. `pre{a,b}fix.rs` is refused (F-6). |
| **D97** | `PromptSpec.skill_matches: Option<BTreeMap<SkillId, String>>` (amends D73's type, F-5). `Some` at `Engine::with_excerpts` and `preview::build`; `None` at the judge, the handoff, `phase_spec` and every fixture. `Some({})` is "the set ran and matched nothing". |
| **D98** | The clone's copy reads `GraphSource::phase_attachments(project, phase) -> Vec<SkillBinding>` — raw rows, because a `BoundSkill` has lost `languages` and the two tokens. The stores' and `Backend`'s methods land in **T1**; the trait method and its four delegations (`graph.rs::TestSource`, `fake.rs::MemStore`, `fake.rs::FakeGraphSource`, `run_worker.rs::BackendGraphs`) land in T5 (amends T5's file list, F-7). |
| **D99** | `.sqlx` is prepared in T1 against a scratch database migrated to `0007`; T2 migrates the same database to `0008` and runs `prepare --check` **without** re-preparing (a comment changes no query hash); T5 re-prepares, replacing `create_step_graph`'s file, so the count is unchanged across T5 (F-8). |
| **D100** | One `GlobError` type with one `Display` per refusal. The activation form refuses before it sends and the writer refuses the same strings, and both carry the same sentence; T1's case 4 pins the store's half and T4's test the view's (F-9). |
| **D101** | `SaveSkill` carries **two** CAS tokens — `expected: Option<DateTime<Utc>>` (the `skill.updated_at` the editor opened on) and `expected_version: Option<i32>` (the head version) — because the description and the append are two different tables and two different surfaces. The worker short-circuits on the first `Stale` and answers `SkillsStale` with a fresh snapshot. `None` is a token on both, meaning "I expect no row". |
| **D102** | The three prompts T3/T4/T5 claim for the global table are `q`, `Tab`, `Shift+Tab`, `1`–`9`, `?` and overlay `Esc` (`keymap.rs::default_global`); `w` is documented as T6's and is **not** bound, so it is avoided. Every browse key is asserted against the table by a test, as `templates.rs:471` asserts it in prose (F-13). |
| **D103** | `GlobError` is one variant per refusal, each carrying the byte offset the writer's `Constraint` and the form's cursor both name, plus `GlobError::at(&self) -> Option<usize>` beside `templates.rs`'s `error_at`. `Empty` is the one variant with no offset, so the cursor goes to 0 — the `MissingRequired` precedent. |
| **D104** | T1's three `.sqlx`-reading helpers are `skill`, `skill_head`, `skill_versions`, `skill_binding` and `attachment` — **five**, not the plan's "the per-row re-reads", because D78's pin check needs the skill's version numbers and a read of the one pinned version would answer a different question. `skill_binding_refusal` and `skill_pin_refusal` are split so the version read is paid only when a pin is set. |
| **D105** | Conformance cases 1–4 are registered immediately after `prompt_template_refuses_what_parse_refuses` in both `CASES` and `run_case`; a case's identity is its name and `run_case_accepts_every_name_in_cases` is the guard, so the alternative (appending at the end) is equally valid. |
| **D106** | `library.rs` has its own `LIST_WIDTH = 46` and `NAME_WIDTH = 26`; `templates.rs`'s 40 and 44 do not move. This is the structural reason the six `templates__*.snap` files cannot move (H-27). |
| **D107** | The `SaveSkill` reply is `Skills` and its spent-token reply is `SkillsStale`, the same two-variant shape as `Templates`/`TemplatesStale`, because both re-read the whole snapshot and a carried enum would put a fifth shape on the seam. |
| **D108** | The per-skill token estimate is `TokenEstimator::DEFAULT.estimate(&head.body)`, computed in `render` (D84) and named with `TokenEstimator::DEFAULT.id` in the pane title, so one id never carries two arithmetics. |
| **D109** | `skills/mod.rs`'s `wants_requests` returns **two** requests of two different variants. N of one variant is the failure the catalogue's doc describes; two of two is fine (F-12). |
| **D110** | T1's `.sqlx` delta is **+11** as the implementer writes it here; the commit records the real `ls | wc -l` and §8 pins that number for T5. A decomposition that folds `skill_versions` into `skill_head` is fine; a silent drift is not (H-14). |

---

## 13. Hazards (H-1 onward)

Numbered per task in §2.11, §3.8, §4.4, §5.4 and §6.5, and consolidated here so nothing is missed
while reading one section. **H-1 through H-14 are T1; H-15 through H-26 are T2; H-27 through H-32
are T3; H-33 through H-37 are T4; H-38 through H-43 are T5.**

| # | Task | One line |
|---|---|---|
| H-1 | T1 | D78's writer needs `glob::compile`; the module lands as T1 commit 1 (F-1, D95). |
| H-2 | T1 | A case that upserts `tests` or `rust-style` moves the demo row (F-11). |
| H-3 | T1 | `.sqlx` prepared against a database not migrated from zero keeps a stale nullability. |
| H-4 | T1 | `SaveSkill`'s two tokens can diverge, leaving a description written and a body not. |
| H-5 | T1 | `StoreRequest` derives `Debug` and `SaveSkill` carries a body. |
| H-6 | T1 | `pg/write.rs` binds `TEXT[]` wrongly and sqlx will not catch it. |
| H-7 | T1 | `ON CONFLICT (skill_id, project_id, phase_id)` may not infer the `UNIQUE NULLS NOT DISTINCT` constraint. |
| H-8 | T1 | `attachment`'s re-read spells `project_id = $2` and a global row compares to NULL. |
| H-9 | T1 | A duplicate `id` is `23505` on one side and prose on the other. |
| H-10 | T1 | `set_skill_binding` mints an id on a create and ignores it on a replace. |
| H-11 | T1 | The pin refusal needs a fourth read the plan does not name (D104). |
| H-12 | T1 | `StoreReply` and `StoreRequest` are matched in more places than the task lists. |
| H-13 | T1 | The `htui` suite is scheduling-dependent; the keyring fake is process-wide. |
| H-14 | T1 | `.sqlx` is 268+N and N is only knowable after the prepare (D110). |
| H-15 | T2 | `ExcerptRequest.changed_paths` stays dead without `htui-agent/src/excerpt.rs`. |
| H-16 | T2 | A `CHECK`-list test moves with `ChoiceReason` — it does not. |
| H-17 | T2 | `listed` is captured after `fill_lexical_heads` took `&mut listing`. |
| H-18 | T2 | `PromptSpec` has no `Default`, and there are more literals than §3.3 lists. |
| H-19 | T2 | A demo digest or snapshot moves; no demo attachment is `Glob`, so none does. |
| H-20 | T2 | Splitting `diff_of` changes `DiffBlock`'s bytes. |
| H-21 | T2 | A path git emits that `is_repo_relative` would drop. |
| H-22 | T2 | R-35's guard keeps `TIER2_PREV_DIFF` inert on a step with no `glob` candidate. |
| H-23 | T2 | `with_excerpts` re-reads what `forwarded` already read. |
| H-24 | T2 | The new code re-reads `run_steps` to find the previous attempt. |
| H-25 | T2 | The comment-only migration's pinned literal moves byte for byte. |
| H-26 | T2 | A scratch database already at `0007` cannot take `0008` twice. |
| H-27 | T3 | A skill row needs three columns where a template row had two (D106). |
| H-28 | T3 | The switch line and the strip text are shared with the Templates view. |
| H-29 | T3 | A `Skills` reply arriving before the tab is opened is dropped by a stale view. |
| H-30 | T3 | A `SkillBody` reaching a `Debug` prints the body. |
| H-31 | T3 | The estimate is recomputed on every frame (D84's own arithmetic, accepted). |
| H-32 | T3 | `Tab` in the naming form shadows the global next-tab. |
| H-33 | T4 | The map's order and the union's order drift. |
| H-34 | T4 | A language name with trailing whitespace contributes nothing silently. |
| H-35 | T4 | `mod matrix;` before `matrix.rs` exists does not compile (R-30). |
| H-36 | T4 | The matrix is a row-packed block, not a `Table` (D82 as amended). |
| H-37 | T4 | An unknown language contributes nothing. |
| H-38 | T5 | `create_step_graph`'s `.sqlx` file is replaced, so a count check reads as a loss. |
| H-39 | T5 | `load_demo` writes no `is_override` (H-39's test). |
| H-40 | T5 | Two clones of one source have different phase ids, so their row ids never match. |
| H-41 | T5 | `an_override_graph_is_marked_as_one` and the clone name are checked in two places. |
| H-42 | T5 | A fixture whose `false` a test asserts moves — sites 8 and 9 do not. |
| H-43 | T5 | `DeleteReach.skill_bindings == 3` must not move. |

<!-- BLUEPRINT-END -->
