# Blueprint: MOD-2 milestone 9 — the prompt assembler and its preview

**Plan**: `.claude/plans/mod-2-prompt-assembler.plan.md` (D95–D108, T59–T70 binding; its "Verified
claims" table re-checked against `main` at `a601931`, 2026-09-11 — every row held; the points where
the plan and the tree disagree are section E, and the ones that need the maintainer before a line is
written are section J).
**Design authority**: `docs/ANA-5.md` §4.1 (`:269-432`), §4.2 (`:435-590`), §4.3 (`:592-787`), §4.4
(`:791-988`), §4.5 (`:990-1211`), §4.6 (`:1215-1351`), §4.7 (`:1354-1455`), §4.8 (`:1458-1478`),
§5.1–§5.4 (`:1483-1810`), §7 (`:1853-1871`), §8 (`:1874-2095`), §9 (`:2098-2220`), §12 criteria 1–21
(`:2263-2342`).

Where the plan and the tree disagree the tree wins and the point is marked **plan ≠ tree**; where the
plan and the ANA disagree the plan's confirmed decision wins; anything only a live run could settle
was **T69's**, and T69 has run (D108).

Conventions inherited unchanged from the milestone-5/6/7/8 blueprints: `#![warn(missing_docs)]`
(`crates/htui-core/src/lib.rs:9`), `unsafe_code = "forbid"`, MSRV 1.98, one `thiserror` type per
concern, no lock across an `.await` (`mem.rs:3-7`), `htui-store` never depends on `htui-agent`, the
scrubber stays behind the `Scrubber` trait (`scrub.rs:49-58`), nothing keyed on `agent.name`
(`R-AGT-5`; the estimator is keyed on a **model id**, B.3), `CASES` moves **22 → 23** with both
literals (`crates/htui-core/tests/mem_store.rs:25`, `crates/htui-store/tests/pg_conformance.rs:20`).
Every `cargo test -p htui …` line in the plan is run with `--features testkit` (E-1).

> **Splice note.** This artifact was delivered in two parts. One passage — the tail of the
> `RepoReader` trait in B.7 — was lost between them; its two method signatures are reconstructed
> here from ANA-5 §4.5's "exposes `list` and `read` and no write operation" (`:1194`) and from
> `select`'s use of them, and are marked as such. Everything else is the architect's text.

---

## E. Errors and gaps in the plan, for the maintainer

**E-2 and E-4 need a maintainer decision; the rest are corrections the implementer applies.**

| # | Plan says | What is true | Consequence / resolution assumed below |
|---|---|---|---|
| **E-1** | T60, T67, T68 validate with `cargo test -p htui --features demo,test-support` | `crates/htui/Cargo.toml:19-23` declares exactly one feature, `testkit`; `demo` is already on through the dependency lines (`:27,30`). The same slip milestone 8's blueprint caught as E-6 | Every line is `cargo test -p htui --features testkit …`; `prompt_preview.rs` opens with `#![cfg(feature = "testkit")]` like `backlog.rs:6` |
| **E-2** | D102/D103: the preview reads "from real store reads", and the risk table says an **offline** preview "says `app_setting_default`" | Three of the preview's inputs are online-only by ANA-5 §8's own table (`:1993-2002`): `prompt_template`, `skill*`/`skill_binding` and `box_tool` are all in the mirror's not-mirrored list (`cache_migrations/0001_mirror.sql:24-25`). An offline box has **no template body** to render, not merely no `app_setting` row | **Built as written, an offline preview fails on the first read with a bare `Unreachable`.** Two coherent answers: **(a) refuse on `Backend::Offline` before anything is spawned**, with one named sentence (`PROMPT_ON_SERVER_ONLY`, B.15) the way `REGISTRY_ON_SERVER_ONLY` refuses a probe (`agent_worker.rs:703-711`); or (b) preview offline from `DEFAULT_TEMPLATES` with the box row and no tools, declaring both in `notes`. This blueprint is written for **(a)**: (b) renders bytes no run produces, which is D103's own argument against stand-ins. D101's `DEFAULTS` still matter online — `app_setting` may lack a key on a database migrated before `0002` — and `budget_source: "app_setting_default"` is that case |
| **E-3** | T62's `READ_CASES` lists `project_settings_readable` | `project_settings` is inherent on the four backends (`mem.rs:242`, `pg/read.rs:620`, `cache/read.rs:777`, `backend.rs:312`), not a `ReadStore` method, so `run_read_case<S: ReadStore>` cannot call it | The case is `project_row_has_settings` and reads the new `ReadStore::project` (B.12); it asserts `settings["token_budget"] == 120_000` (`fixtures.rs:490-494`) |
| **E-4** | D103 / T67: the preview "records `no_path` per repo" and proves criterion 12 "from the running binary" | `DemoData` has **no `repo` field** and `load_demo` leaves `repo` alone (`pg/demo.rs:24-28`); nothing in any crate reads `repo` rows. A preview over the demo store has **no repo to say `no_path` about**, so `roots` would be `[]` and criterion 12 would be proved by a unit test only | **Scope addition, small, needs a yes**: `DemoData.repos: Vec<Repo>` (one primary repo per project, `name` = the project slug, `fixtures.rs` A.20), `load_demo` inserts them (A.30), and an inherent `repos(project) -> Vec<Repo>` read on `MemStore`/`PgStore` with a `Backend` arm (B.13). `repo` **is** mirrored (`0001_mirror.sql:62-66`) but the `CacheStore` arm is left to MOD-4 with the rest of the offline preview (E-2). Without this, D103's `no_path` sentence is unreachable in the binary |
| **E-5** | T60: `TEMPLATE_NAMES` 8 → 10 | Fixture template ids are minted as `project_index * 8 + template_index` (`fixtures.rs:560-564`, `:632-635`). At ten names, project 1's template 0 collides with project 0's template 8 — `demo_uuid` is a pure function of `(class, n)`, so two rows share a primary key and `load_demo` fails on the second insert | The multiplier moves to **10** with the doc comment at `:562-564`; the fixture test `demo_store_loads_the_fixture` gains `templates.len() == 30` and a distinct-id assertion (F-T60) |
| **E-6** | T62/T63: the inherent `PgStore` reads are `prompt_template`, `bound_skills`, `box_profile`, `app_settings`, `repo_paths` | The preview also needs `item_kind.name` for `{{item_kind}}` (ANA-5 `:325`) — no reader returns an `ItemKind`; `MemStore::State.kinds` is private (`mem.rs:58`) — and a **list** of the project's templates for the picker, which `prompt_template(project, name, version)` cannot give in one round trip | Two more inherent reads (B.13): `item_kind(id) -> Option<ItemKind>` and `prompt_templates(project) -> Vec<PromptTemplate>` (every version, `ORDER BY name, version`). ANA-2 §8's pinned-version `prompt_template(project, name, version)` stays MOD-4's — it is a `WHERE` over the same rows and this milestone resolves "latest" in Rust (D103) |
| **E-7** | File table: `fixtures.rs` gains "skills/bindings rows" (T62) | `load_demo` names `skill*` among the tables it leaves alone (`pg/demo.rs:27`), so a `DemoData` that carries skill rows would load them into `MemStore` and **not** into `PgStore`, and criterion 15's Postgres half would run against an empty table | `crates/htui-store/src/pg/demo.rs` joins the file table (A.30): `skill`, `skill_version`, `skill_binding`, `box_tool` and (E-4) `repo` inserts, in foreign-key order after `prompt_template` (`:225-240`). The `sqlx::query!` texts add five `.sqlx/` entries |
| **E-8** | T62: `set_step_prompt` on "the two `Writer` arms"; no verdict on the buffered one | `writer.rs:253-267`'s buffered `set_step_usage` is a **no-op** *because* `upload_pending` recomputes `run_step.usage` from the uploaded rows (D36, `:256-258`). Nothing can recompute `trim_record`: the `prompt` payload carries only its abridged `sections[]` (ANA-5 §5.1 `:1551-1554`), and the pending line format holds `session_event` columns only (`:71-72`) | `BufferedWriter::set_step_prompt` is **`Err(StoreError::Unreachable(PROMPT_ON_SERVER_ONLY))`**, not a no-op and not a buffered row (B.12). `R-STO-4` starts no graph run offline, so no caller reaches it in this milestone; if MOD-4 ever does, a loud refusal beats a silently absent audit row that `R-PRM-3` requires "recorded on the step" |
| **E-9** | T68 adds the two `RunStepSummary` fields "derived from `trim_record` in each of the three `runs()` projections" | The `set_step_prompt` conformance case (T62) has no way to **observe** the write: `ReadStore` returns neither `run_step.trim_record` nor `prompt_digest` (`mem.rs:1139-1141` says so of `usage`), and `CASES` is written against the trait alone (`conformance.rs:1-7`). `RunStepSummary.prompt_tokens`/`trimmed` are the only trait-visible trace of the row | The two fields and the `MemStore` projection land in **T62**, the `PgStore`/`CacheStore` projections in **T63**, and T68 keeps the fixture step and the pane (H). The case observes through `runs()` (B.12) |
| **E-10** | D102: the preview is "served as deferred work on an owned handle" | The only owned handle the tree has is `Writer` (`writer.rs:1-24`), which exposes `WriteStore` + `ReadStore` and none of the inherent reads the preview needs. `Backend` **is `Clone`** (`backend.rs:39-40`) and every arm is a pool handle or an `Arc` (`writer.rs:8-10`); `go_online` already clones a `CacheStore` out of it (`store_worker.rs:804`) | The deferred task takes **`backend.clone()`** (B.15). D4's "no other task can reach it" is about who owns the *swap* (`store_worker.rs:575-599`), which a snapshot clone cannot perform. The plan's file table gains `crates/htui/src/preview.rs` (A.35): the spec builder is 150 lines that do not belong inside a 1 783-line worker |
| **E-11** | Smaller slips, corrected in place | (i) `SEEDED_SETTINGS` is at `pg/mod.rs:47-50` (`:46` is its doc line). (ii) ANA-5 §8's `upstream_summaries(.., scope: &Scope)` (`:1990`) is `&PromptScope` under D95. (iii) T64 puts "scrub per section" in `digest.rs`; the pipeline of §4.7 (`:1387-1398`) is `assemble()`'s in `mod.rs`, and `digest.rs` owns only steps 5–7. (iv) D101's "test asserts `DEFAULTS` equals migration `0002`'s ten values" lives in `crates/htui-store/tests/migrations.rs` beside `:294-322` — `htui-core` cannot `include_str!` a sibling crate's migration without a path dependency. (v) `PROMPT_DIGEST` (`fixtures.rs:1228`) stays a literal, per ANA-5 `:2077-2081`. (vi) The plan's validation block's `cargo tree` clause is satisfied: `sha2` and `insta` are workspace entries (`Cargo.toml:29,37`) already compiled for `htui-store`/`htui-agent`/`htui` |

---

## P. Plan ≠ tree, and ANA ≠ plan, resolved

| # | Says | Tree / plan says | Resolution |
|---|---|---|---|
| P-1 | ANA-5 §4.4 step 9: "Assemble, normalise, scrub, digest" (`:878`) | §4.7's pipeline scrubs at step 2, before trim (`:1388-1390`); D100 confirms per-section scrubbing | §4.7 wins: scrub per section after render, before estimate and trim (D.1 step 3). The recorder's own pass over the payload is then an assertion (`record.rs:580-583` still runs it) |
| P-2 | §4.7 steps 5 then 6: collapse LF runs, **then** normalise line endings (`:1393-1396`) | On CRLF input `\r\n\r\n\r\n` has no LF run to collapse, so collapse-then-normalise leaves three LFs and criteria 4 and 6 fail together | Line endings are normalised **first**, per section at render time (B.4 `normalise_newlines`) and once more over the substituted whole; the collapse runs after. Same bytes for LF input, correct bytes for CRLF |
| P-3 | §4.4's `documents:*` row: "drop whole documents from the end of `input_kinds` order, **then** head+tail the survivors" (`:888`) | §5.1's worked example (`:1528-1534`) head+tails `documents:plan` while `documents:review` — the later kind — is untouched, so the example runs head+tail **before** any drop and from the **first** kind | The example wins because criterion 9's arithmetic is written against it: head+tail in `input_kinds` order to each document's floor, then drop from the end of the order, never position 0 (D.3) |
| P-4 | §4.4's marker "`[... htui elided 412 lines / 18 903 bytes ...]`" and "one space-free ASCII form" (`:902-906`) | The example's thousands separator is a space | Numbers are plain decimal digits: `[... htui elided 412 lines / 18903 bytes ...]`, so the marker equals `elided_bytes` verbatim and criterion 9's last clause is a string compare |
| P-5 | §4.4: `for_agent(name)` keys the estimator on `agent.name` (`:932-934`) | Milestone-8 conventions: nothing keyed on an agent's name (`R-AGT-5`) | `TokenEstimator::for_model(model_id)` keys on the model id a `run_step.model` carries (a wire fact); `DEFAULT` for `None`. The two constant rows get **distinct ids** (`chars-v2`, `chars-v1-gpt`): §5.1 says "every token figure in the record is by this estimator and no other" (`:1544`), which one id for two constant sets cannot honour. **Mandatory under D108**, which gives the two rows different provenance as well as different numbers |
| P-6 | §5.1: `budget_source` is `phase`, `project` or `app_setting` (`:1543`) | D101 adds a fourth rung (the compiled-in table); the plan's risk row names it `app_setting_default` | `BudgetSource` has four variants (B.8) |
| P-7 | §5.1: "`v` first so a later reader can branch" (`:1486`) | `serde_json::Map` is a `BTreeMap` (plan verified claim), so keys serialise sorted and `v` lands last; `JSONB` reorders keys anyway | Nothing preserves key order; `v` is a key, not a position (H-19) |
| P-8 | §4.5's `ExcerptRequest.roots: &[RepoRoot]` "repo slug + readable absolute root" (`:1154`) | The preview resolves no root (D103) and `run_step_tree` does not exist | `RepoRoot.root: PathBuf` stays; the preview passes an empty slice and the ranker is never asked to read (D.5) |
| P-9 | §4.2's `sections[]`: "`template` is always the first entry, by convention" (`:574`) | A body may open with a placeholder | `template` is first unconditionally; every other entry follows first-occurrence span order (B.5) |
| P-10 | §4.2 skills render: "`<skill name=".." version="N">` block per binding" (`:480`) | No closing form is given | `<skill name="…" version="N">` / `</skill>`, the `<section>` rules applied one level down (B.4) |
| P-11 | §4.5 audit: `provider_set: ["builtin@1"]` strings (`:1123`); criterion 13 wants "a non-`ok` status" per failing provider (`:2301-2302`) | Strings carry no status | A closed grammar over the same string: `name@version` when ok, `name@version:error` / `:panic` / `:timeout` otherwise (B.7) |
| P-12 | `ReadStore::documents_of_kinds(item, kinds)` "already named at ANA-2 §8" (`:1989`) | ANA-2's resolver (`docs/ANA-2.md:395-406`) prefers this run's output and excludes losers — both need `run_step` rows MOD-4 owns | The method is the **latest-version-per-kind** read in `kinds` order; an empty `kinds` means every kind in byte order (the preview's D103 form). MOD-4 layers the run preference on top or adds its own read (B.12) |

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-core/Cargo.toml` | UPDATE | T59 | `sha2 = { workspace = true }` under `[dependencies]` after `thiserror` (`:21`); `insta = { workspace = true }` under `[dev-dependencies]` after `tokio` (`:26`). No feature change: `test-support = ["demo"]` (`:13`) already gates `prompt::fixtures` |
| 2 | `crates/htui-core/src/lib.rs` | UPDATE | T59 | `pub mod prompt;` after `pub mod model;` (`:11`) — alphabetical with `scrub`, `store` |
| 3 | `crates/htui-core/src/prompt/mod.rs` | CREATE | T59, T64, T65 | B.1: the public surface and `assemble()`. T59 creates it with `pub mod template; pub mod estimate;` only; T64 and T65 extend it |
| 4 | `crates/htui-core/src/prompt/template.rs` | CREATE | T59 | B.2 |
| 5 | `crates/htui-core/src/prompt/estimate.rs` | CREATE | T59 | B.3 (constants measured, D108) |
| 6 | `crates/htui-core/src/prompt/defaults.rs` | CREATE | T60 | B.9: `DEFAULT_TEMPLATES`, `COMMAND_QUEUE_TEXT` |
| 7 | `crates/htui-core/src/prompt/render.rs` | CREATE | T64 | B.4 |
| 8 | `crates/htui-core/src/prompt/digest.rs` | CREATE | T64 | B.6 |
| 9 | `crates/htui-core/src/prompt/trim.rs` | CREATE | T65 | B.5, D |
| 10 | `crates/htui-core/src/prompt/settings.rs` | CREATE | T65 | B.8 |
| 11 | `crates/htui-core/src/prompt/excerpt.rs` | CREATE | T61 (`PathPrefix`), T66 (the rest) | B.7 |
| 12 | `crates/htui-core/src/prompt/fixtures.rs` | CREATE | T64, T65, T66 | B.10, `#[cfg(feature = "test-support")]` |
| 13 | `crates/htui-core/src/model/skill.rs` | CREATE | T61 | B.11 |
| 14 | `crates/htui-core/src/model/link.rs` | UPDATE | T61 | `UpstreamEntry` + `sort_canonical` after `LinkGraph` (`:92`); nothing else |
| 15 | `crates/htui-core/src/model/box_.rs` | UPDATE | T61 | `BoxProfile` + `BoxProfile::project(&BoxRow, Vec<BoxTool>)` after `BoxInfo` (`:92`) |
| 16 | `crates/htui-core/src/model/scope.rs` | UPDATE | T61 | `PromptScope` after `Scope`'s impl (`:42`). **Not**: any `Scope` field or method |
| 17 | `crates/htui-core/src/model/run.rs` | UPDATE | T62 | `RunStepSummary.prompt_tokens: Option<i32>`, `.trimmed: bool` after `finished_at` (`:286`); `pub fn prompt_summary(trim_record: Option<&Value>) -> (Option<i32>, bool)` (B.11) |
| 18 | `crates/htui-core/src/model/mod.rs` | UPDATE | T61 | `pub mod skill;` (`:92` neighbourhood); re-exports: `box_::BoxProfile`, `link::UpstreamEntry`, `scope::PromptScope`, `skill::{BoundSkill, Skill, SkillBinding, SkillVersion}` in the existing `pub use` lines (`:96-124`) |
| 19 | `crates/htui-core/src/store/traits.rs` | UPDATE | T62 | Four `ReadStore` methods after `step_events` (`:47`); `set_step_prompt` after `finish_chat_run` (`:201`); the `// links, notes, documents, skills, templates, box ...` comment (`:202`) loses `documents` and `skills`; module doc (`:8-11`) gains a milestone-9 paragraph |
| 20 | `crates/htui-core/src/fixtures.rs` | UPDATE | T60, T62, T68 | T60: `TEMPLATE_NAMES` 8 → 10, bodies from `DEFAULT_TEMPLATES`, id multiplier 10 (E-5), the `prompt` payload's `"prd"` → `"documents:prd"` (`:1247`). T62: `class::{SKILL = 16, SKILL_BINDING = 17, REPO = 18}`; `DemoData.{skills, skill_versions, skill_bindings, box_tools, repos}`; five new `item_link` rows (C.4); `ids::{SKILL_RUST_STYLE, SKILL_TESTS, BINDING_*, REPO_HTUI, REPO_AGY, REPO_VULKAN}`. T68: `STEP_IMPL.trim_record = Some(prompt::fixtures::demo_trim_record())` (`:1148-1155`) |
| 21 | `crates/htui-core/src/store/mem.rs` | UPDATE | T62 | `State.{skills, skill_versions, skill_bindings, box_tools, repos, repo_paths, app_settings}`; `#[expect(dead_code)]` removed from `templates` (`:65-67`) and `kinds` gains a reader; the five trait impls; inherent `prompt_templates`, `bound_skills`, `box_profile`, `app_settings`, `repos`, `repo_paths`, `item_kind`; `run_steps` (`:497-521`) fills the two new fields through `prompt_summary` |
| 22 | `crates/htui-core/src/store/conformance.rs` | UPDATE | T62 | `CASES` gains `"set_step_prompt_writes_digest_and_trim"` last (`:46`); `run_case` arm; `READ_CASES`, `run_read_case`, `run_all_reads` after `run_all` (`:108`) — B.12 |
| 23 | `crates/htui-core/tests/mem_store.rs` | UPDATE | T62 | `22` → `23` (`:25`) with the message extended; new `mem_store_read_conformance` calling `run_all_reads(|| async { MemStore::demo() })` |
| 24 | `crates/htui-core/tests/prompt_golden.rs` | CREATE | T64 | H |
| 25 | `crates/htui-core/tests/prompt_digest.rs` | CREATE | T65 | H |
| 26 | `crates/htui-core/tests/snapshots/prompt_golden__*.snap` | CREATE | T64+ | one per golden (`insta` writes them under `tests/snapshots/`) |
| 27 | `crates/htui-store/src/pg/read.rs` | UPDATE | T63 | The four `ReadStore` reads (C.1 for the CTE); inherent `prompt_templates`, `bound_skills`, `box_profile`, `app_settings`, `repos`, `repo_paths`, `item_kind`; the steps query of `runs` (`:314-337`) gains two columns (B.13) |
| 28 | `crates/htui-store/src/pg/rows.rs` | UPDATE | T63 | `StepRow.{prompt_tokens: Option<i32>, trimmed: bool}` **appended** (positional binding, `:22-24`); `UpstreamRow` + `into_entry` |
| 29 | `crates/htui-store/src/pg/write.rs` | UPDATE | T63 | `set_step_prompt` after `set_step_usage` (`:366`): one `UPDATE … SET prompt_digest = $2, trim_record = $3 WHERE id = $1`, `rows_affected() == 0` → `NotFound { entity: "run_step" }` (the `:359-364` shape) |
| 30 | `crates/htui-store/src/pg/demo.rs` | UPDATE | T63 | E-7: `repo`, `box_tool`, `skill`, `skill_version`, `skill_binding` inserts; the doc at `:24-28` loses those names |
| 31 | `crates/htui-store/src/cache/read.rs` | UPDATE | T63 | The four `ReadStore` reads over the mirror (C.2 for the CTE; `document` body is mirrored `0001_mirror.sql:97-101`, `project.settings` `:57-61`); the `runs` steps query (`:480-485`) gains the two JSON-derived columns |
| 32 | `crates/htui-store/src/backend.rs` | UPDATE | T63 | Four `ReadStore` arms (`:324-380`); seven inherent dispatchers after `project_settings` (`:318`) whose `Offline` arm is `Err(Unreachable(PROMPT_ON_SERVER_ONLY))`; `pub const PROMPT_ON_SERVER_ONLY` beside `REGISTRY_ON_SERVER_ONLY`'s re-export |
| 33 | `crates/htui-store/src/writer.rs` | UPDATE | T62 | `BufferedWriter::set_step_prompt` (E-8) after `set_step_usage` (`:267`); `Writer::set_step_prompt` after `:468`; the doc list at `:71-82` gains a bullet |
| 34 | `crates/htui-store/.sqlx/query-*.json` | UPDATE | T63 | `cargo sqlx prepare -- --all-targets --all-features` from `crates/htui-store`; ~14 new files |
| 35 | `crates/htui/src/preview.rs` | CREATE | T67 | B.15: `build_spec`, `PromptPreview`, `run_preview` |
| 36 | `crates/htui/src/store_worker.rs` | UPDATE | T67 | `StoreRequest::PromptPreview { item, template_name, scope }` after `StepEvents` (`:92`); `name()` arm `"prompt_preview"`; `StoreReply::PromptPreview(Box<PromptPreview>)` after `StepEvents` (`:277`); the runtime-arm lists at `:544-558` and `:697-708` gain the variant |
| 37 | `crates/htui/src/agent_worker.rs` | UPDATE | T67 | `serve` (`:497`) gains the `PromptPreview` arm → `self.preview(backend, replies, addr, ..)` pushing `tokio::spawn(preview::run_preview(..))` onto `background` (`:296-301`, the probe's shape at `:729-735`) |
| 38 | `crates/htui/src/lib.rs` | UPDATE | T67 | `pub mod preview;` |
| 39 | `crates/htui/src/ui/tabs/backlog/detail/prompt.rs` | CREATE | T67 | B.16 |
| 40 | `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | T67 | `pub mod prompt;` (`:9-13`), `pub use prompt::PromptTab;` (`:27-31`); the strip doc at `:216` gains `Prompt`; module doc "five sub-tabs" → six (`:1`, `:52`) |
| 41 | `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | T67 | `detail.register(Box::new(PromptTab::new()));` after `NotesTab` (`:69`); `go()` issues a sixth read (`:112-116`); import (`:16-18`); the field doc at `:44` |
| 42 | `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | T68 | Step rows of height 2 when `prompt_tokens.is_some() \|\| trimmed` (B.17); the module doc gains the legend. **Not**: the column constraints (`:250-260`), the cursor, `Enter` |
| 43 | `crates/htui/tests/prompt_preview.rs` | CREATE | T67 | H |
| 44 | `crates/htui/tests/backlog.rs` | UPDATE | T67, T68 | `the_five_sub_tabs_render_the_selected_item` (`:136-149`) and `every_sub_tab_says_so_when_it_has_nothing` (`:152-172`) gain a sixth row and are renamed `…six…` |
| 45 | `crates/htui/tests/snapshots/backlog__detail_{body,runs,graph,documents,notes}.snap`, `backlog__empty_*.snap`, `backlog__list_*.snap`, `replay__runs_step_selected.snap`, `integration__demo_shell.snap` | RE-ACCEPT | T67, T68 | The sub-tab strip gains ` Prompt ` (every detail snapshot); the Runs pane gains one line under `implement`. Reviewed line by line: nothing else may move |
| 46 | `crates/htui/tests/snapshots/backlog__detail_prompt.snap`, `backlog__empty_prompt.snap`, `prompt_preview__*.snap` | CREATE | T67 | H |
| 47 | `crates/htui-store/tests/pg_conformance.rs` | UPDATE | T63 | `EXPECTED_CASES` 22 → 23 (`:20`); new `pg_store_read_conformance` looping `READ_CASES` with the `:33-43` shape |
| 48 | `crates/htui-store/tests/cache.rs` | UPDATE | T63 | `mirror_reads_equal_the_reference_store` (`:229`) gains the four reads against `MemStore` (the file's own rule, `:10-13`); new `the_mirror_passes_the_read_cases` binding `run_all_reads` over a refreshed `CacheStore` (criterion 19's second clause, criterion 7's second) |
| 49 | `crates/htui-store/tests/migrations.rs` | UPDATE | T65 | After the `:294-322` read: `assert_eq!(rows, htui_core::prompt::settings::DEFAULTS.as_rows())` (D101's test, E-11 iv) |
| 50 | `crates/htui-store/tests/pg_criteria.rs` | UPDATE | T63 | `set_step_prompt_writes_both_columns_and_only_them`: raw `SELECT prompt_digest, trim_record, usage FROM run_step` (the `:98-104` helper widened) — criterion 11's store half |
| 51 | `crates/htui-store/tests/writer_buffered.rs` | UPDATE | T62 | `set_step_prompt_is_refused_offline` beside `set_step_usage_is_a_no_op` (`:301`) |
| 52 | `crates/htui-agent/src/excerpt.rs` | CREATE | T66 | B.7 (agent half) |
| 53 | `crates/htui-agent/src/lib.rs` | UPDATE | T66 | `pub mod excerpt;` after `event` (`:95`); `pub use excerpt::{FsRepoReader, run_providers};` |
| 54 | `crates/htui-agent/src/conformance.rs` | UPDATE | T62 | `UsageSpy::set_step_prompt` delegating (`:616` impl) |
| 55 | `crates/htui-agent/tests/recorder.rs` | UPDATE | T62, T65 | `SpyStore::set_step_prompt` delegating (`:274` impl); T65: `record_prompt_over_an_assembled_prompt_recomputes_the_same_digest` (criterion 11's payload half) |
| 56 | `crates/htui-agent/tests/excerpt.rs` | CREATE | T66 | H |
| 57 | `crates/htui-agent/tests/estimator_live.rs` | CREATE | T69 | H, `#[ignore]` |
| 58 | `README.md` | UPDATE | T70 | `:186-187` "five detail sub-tabs" → six with **Prompt**; a `### The Prompt sub-tab` subsection: what it renders, `n`/`p`, what it deliberately does not do (send, write), and E-2's offline refusal |
| 59 | `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-2.md`, `.claude/prds/mod-2-agent-driver-chat.prd.md`, the plan | UPDATE/CREATE | T70 | T70 |

---

## B. Interfaces, exactly

Design notation that compiles in spirit: real types, real bounds, real lifetimes. `pub(crate)` where
nothing outside `htui_core::prompt` needs the name.

### B.1 `crates/htui-core/src/prompt/mod.rs` — the surface (T59 skeleton, T64/T65 body)

Module doc, first paragraph: ANA-5 §4.8's placement — everything pure here, the filesystem half in
`htui_agent::excerpt`; one assembler, three roles (§7); `assemble()` is pure by contract
(invariant 2): no store handle, no I/O, no clock, no map iteration reaches the rendered bytes.

```rust
pub mod defaults;
pub mod digest;
pub mod estimate;
pub mod excerpt;
pub mod render;
pub mod settings;
pub mod template;
pub mod trim;
#[cfg(feature = "test-support")]
pub mod fixtures;

pub use estimate::TokenEstimator;
pub use settings::{Budget, BudgetSource, DEFAULTS};
pub use template::{ParsedTemplate, Placeholder, Span, TemplateError, TemplateRole, parse};
pub use trim::{Section, SectionEntry, TrimRecord, TrimStrategy};

/// Everything the assembler needs (ANA-5 §8 `:1917-1942`, widened where the tree needed it).
#[derive(Debug, Clone, PartialEq)]
pub struct PromptSpec {
    pub role: TemplateRole,
    pub template: TemplateRef,
    pub body: String,                    // prompt_template.body of that version
    pub item_key: String,                // "<project.slug>:<item.key>"
    pub item_title: String,
    pub item_kind: String,               // item_kind.name
    pub item_body: String,
    pub phase: String,
    pub output_kind: Option<String>,
    pub attempt: i32,
    pub documents: Vec<InputDocument>,   // input_kinds order, one winner per kind
    pub upstream: Vec<UpstreamEntry>,    // canonical order (§4.7 rule 2) — assemble() re-sorts anyway
    pub box_profile: BoxProfile,
    pub skills: Vec<BoundSkill>,         // R-SKL-2 order — assemble() re-sorts and dedups anyway
    pub excerpts: ExcerptSet,            // read and windowed; audit half filled by the ranker
    pub command_queue: bool,
    pub verify_failure: Option<VerifyFailure>,
    pub previous_diff: Option<DiffBlock>,
    pub judge: Option<JudgeInputs>,      // Some iff role == Judge, else AssembleError::RoleInputs
    pub handoff: Option<HandoffInputs>,  // Some iff role == Handoff
    pub budget: Budget,
    pub max_skill_tokens: i64,           // app_setting.max_skill_tokens, resolved by the caller
    pub estimator: TokenEstimator,
    pub notes: Vec<String>,              // caller notes (D103 stand-ins); copied first into trim_record.notes
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TemplateRef { pub name: String, pub version: i32 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDocument { pub kind: String, pub version: i32, pub body: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyFailure { pub exit_code: i32, pub output: String }

/// A diff at three sizes: the range attribute, the stat, the unified text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffBlock { pub range: String, pub stat: String, pub diff: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeInputs { pub task: String, pub candidates: Vec<JudgeCandidate>, pub reverse: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeCandidate {
    pub fanout_index: i32,
    pub verify: Option<bool>,            // Some(true) = "pass", Some(false) = "fail", None = attribute omitted
    pub exit_code: Option<i32>,
    pub document: Option<InputDocument>,
    pub diff: Option<DiffBlock>,
    pub verification_tail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffInputs {
    pub step_summary: StepSummary,
    pub diff_so_far: Option<DiffBlock>,
    pub failure_reason: String,
}

/// §4.6(c)'s deterministic summary of a step's own events. Built by `StepSummary::from_events`
/// (render.rs); the assembler never sees an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepSummary {
    pub turns: u32,
    pub events: u32,
    pub tool_calls: Vec<(String, u32)>,  // (tool_kind, count) in first-seen order
    pub files_edited: Vec<String>,       // edit_proposal payload paths, first-seen order, deduped
    pub errors: Vec<String>,             // "`<code>` — <message> at turn N" per error row
    pub last_assistant_tail: String,     // the last assistant_text payload.text, head+tail windowed at 40 lines
}

/// The result. `text` is canonical (§4.7) and is what is sent, digested and persisted.
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledPrompt {
    pub text: String,
    pub digest: String,                  // sha256 lowercase hex, 64 chars
    pub sections: Vec<Section>,          // == trim.sections; kept here so a reader need not open the record
    pub trim: TrimRecord,
}

impl AssembledPrompt {
    /// The `prompt` payload's `sections[]` — THE one map (ANA-5 risk 12). Delegates to
    /// `TrimRecord::section_entries`; nothing else constructs a `SectionEntry`.
    pub fn payload_sections(&self) -> Vec<SectionEntry> { self.trim.section_entries() }
    /// `{ "text", "digest", "sections" }` — §5.2's payload, ready for `Recorder::record_prompt`'s
    /// `sections` argument (`record.rs:571-575` takes text and sections separately; this is the array).
    pub fn payload_sections_value(&self) -> serde_json::Value;
}

/// The closed vocabulary of §4.2 as a type. `Box` is a variant, not the prelude type: never glob-import.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionName {
    Template, Item, Documents(String), Upstream, Box, Skills, Excerpts,
    VerifyFailure, PreviousDiff, CommandQueue,
    JudgeTask, JudgeCandidate(i32),
    StepSummary, DiffSoFar, FailureReason,
}
impl SectionName {
    /// `documents:<kind>`, `judge_candidate:<i>`, else the fixed name. This string is the
    /// `name="…"` attribute and the `sections[].name` key — one function, two uses.
    pub fn render(&self) -> String;
    pub const fn is_protected(&self, role: TemplateRole) -> bool;   // §4.4's set; FailureReason only for Handoff
}
impl core::fmt::Display for SectionName { /* = render() */ }
impl serde::Serialize for SectionName { /* serialize_str(&self.render()) */ }

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssembleError {
    /// Criterion 3's exact text; a row that bypassed MOD-9's validator. Covers `UnknownPlaceholder`
    /// and `WrongRole` from `parse` — both are "a placeholder this template may not use".
    #[error("unknown prompt placeholder: {{{{{token}}}}}")]
    UnknownPlaceholder { token: String },
    /// `Unterminated` and `MissingRequired`, with the scanner's own text.
    #[error("prompt template does not parse: {0}")]
    Template(TemplateError),
    #[error("prompt budget too small: protected sections need {needed} tokens, target is {target}")]
    BudgetTooSmall { needed: i64, target: i64 },
    #[error("skills exceed max_skill_tokens ({tokens} > {cap})")]
    SkillsExceedCap { tokens: i64, cap: i64 },
    /// D100: the scrubber found residue in one section's content. Names the section, never the text.
    #[error("unmasked {rule} in section `{section}` at `{path}`")]
    Unmasked { section: String, rule: &'static str, path: String },
    #[error("a {role:?} template needs {what}, which the spec does not carry")]
    RoleInputs { role: TemplateRole, what: &'static str },
}

/// The one entry point. Pure: same `PromptSpec` and same scrubber rules, same bytes, on any box.
/// `scrubber` is D100's: each section's content is wrapped in `Value::String`, scrubbed, unwrapped.
pub fn assemble(spec: &PromptSpec, scrubber: &dyn crate::scrub::Scrubber)
    -> Result<AssembledPrompt, AssembleError>;
```

`assemble()` is the §4.7 pipeline in order, each step a private function in the module that owns it
(D.1). It takes the scrubber as an argument rather than a field of `PromptSpec` because
`PromptSpec: Clone + PartialEq` and `dyn Scrubber` is neither; `MinimalScrubber::new([])`
(`scrub.rs:114-120`) is what every test passes, and it still fails closed on the prefix rules.

### B.2 `crates/htui-core/src/prompt/template.rs` (T59)

ANA-5 `:352-396` verbatim as the type sketch, with the scanner fixed:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateRole { Phase, Judge, Handoff }
impl TemplateRole {
    /// `judge` and `handoff` are reserved names (§4.6); everything else is a phase template.
    pub fn of_name(name: &str) -> Self;
    pub const fn as_str(self) -> &'static str;   // "phase" | "judge" | "handoff" — trim_record.template.role
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Placeholder { /* the twenty of ANA-5 :361-367, in that order */ }
impl Placeholder {
    pub const ALL: &'static [Self];
    pub const fn token(self) -> &'static str;
    pub fn from_token(token: &str) -> Option<Self>;
    pub const fn allowed_in(self, role: TemplateRole) -> bool;   // the three closed sets, :319-341
    pub const fn is_section(self) -> bool;                       // false for the six scalars
    pub const fn required_by(role: TemplateRole) -> &'static [Self];  // Judge: [Candidates]; Handoff: [StepSummary, FailureReason]; Phase: []
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span { Literal(String), Slot(Placeholder) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTemplate { pub spans: Vec<Span>, pub used: Vec<Placeholder> }   // `used`: first-occurrence order, deduped
impl ParsedTemplate {
    /// MOD-9's warning, not an error: a phase body that never places `{{item}}` (§4.1 `:402-403`).
    pub fn omits_item(&self) -> bool;
    /// The literal spans joined — what `template` is estimated over.
    pub(crate) fn literal_text(&self) -> String;
}

pub fn parse(role: TemplateRole, body: &str) -> Result<ParsedTemplate, TemplateError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError { /* ANA-5 :385-395 verbatim, byte offsets */ }
```

**The scanner, fixed** (criterion 2's mapping, one spelling per body): walk `body.char_indices()`; at
byte `i` where `body[i..]` starts with `{{`: if it starts with `{{{{` → push literal `{{`, advance 4.
Else find the next `}}` **before the next `\n` or end of input**; none → `Unterminated { at: i }`.
The bytes between are `token`; if `token` does not match `^[a-z][a-z0-9_]*$` (so `" item "`,
`"Item"`, `""`) or `Placeholder::from_token` is `None` (`"itme"`) →
`UnknownPlaceholder { token, at: i }`; if `!allowed_in(role)` → `WrongRole { token, role, at: i }`.
Literal runs are merged into one `Span::Literal`. A `}}` outside a placeholder is literal. After the
walk, `required_by(role)` minus `used` → the first missing → `MissingRequired`. Four checks in
ANA-5's order (`:398-399`) fall out of that sequence.

### B.3 `crates/htui-core/src/prompt/estimate.rs` (T59; D108)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenEstimator { pub id: &'static str, pub prose_cpt: u16, pub code_cpt: u16 }  // cpt × 10

impl TokenEstimator {
    /// `chars-v2`: 2.5 prose / 2.4 code — **measured** (D108) on this box against `claude`
    /// 2.1.267 (`claude-opus-5[1m]`): `input_tokens + cache_creation_input_tokens +
    /// cache_read_input_tokens` on `result`, differenced against a minimal-prompt baseline;
    /// 40 000 chars of prose → 15 958 tokens (2.507), 20 000 → 8 018 (2.494, linear to 0.5%),
    /// 40 000 chars of fenced code → 16 393 (2.440). ANA-5 §4.4's 3.5 / 3.0 was 28.4% off in the
    /// overflow direction, outside D99's ±25%. The default for an unknown model, as the more
    /// conservative row.
    pub const DEFAULT: Self = Self { id: "chars-v2", prose_cpt: 25, code_cpt: 24 };
    /// `chars-v1-gpt`: 4.0 / 3.3 — the GPT and Gemini families, **unverified**: `agy_acp_server`
    /// emits no `usage_update` (milestone 7), so there is nothing to difference. Kept at ANA-5's
    /// figures and named apart so `trim_record.estimator` says which arithmetic produced a record
    /// (P-5, mandatory under D108).
    pub const WIDE: Self = Self { id: "chars-v1-gpt", prose_cpt: 40, code_cpt: 33 };
    /// Keyed on a model id, never on an agent's name (`R-AGT-5`): `gpt-*`, `o[0-9]*`, `gemini-*`
    /// → WIDE; everything else, including `None`, → DEFAULT.
    pub fn for_model(model: Option<&str>) -> Self;
    /// Splits `s` into prose and code spans and sums `ceil(chars × 10 / cpt)` per span.
    /// **Characters, not bytes**: `s.chars().count()`. Two toggles, each a whole line: a line
    /// whose trimmed start is three or more backticks flips prose↔code (a fence), and a line
    /// starting with `<file ` opens code until the `</file>` line — the excerpt render is not
    /// fenced (§4.5 `:1114-1117`) and would otherwise count as prose.
    pub fn estimate(self, s: &str) -> i64;
}
```

`estimate("")` is `0`; the toggles are exact so the estimate is a pure function of the bytes
(invariant 2). The `estimator` key of every `trim_record` written by this milestone is therefore
`chars-v2`; `chars-v1` never appears in a stored row.

**What D108 changes elsewhere.** No expected trimmed *set* moves:
`the_oversize_fixture_lands_at_or_under_target` asserts the set
`{excerpts, upstream, previous_diff, documents:plan}` and the inequalities, never token figures, and
§5.1's numbers were illustrative rather than fixture figures — but B.10's `phase_oversize()` byte
sizes were sized against 3.5/3.0 and were already too small to be 1.5× over a 120 000 budget at
either rate, so they are restated for 2.5/2.4: budget 44 000 (target 39 600), item body 20 000 chars,
`documents:plan` 60 000, `documents:review` 10 000, `previous_diff` 30 000 fenced, upstream 12 000,
excerpts 20 000 (code), skills 8 000 — roughly 65 700 tokens (1.66× over), clearing the deficit in
exactly §5.1's order: excerpts dropped, upstream at its one-liner floor, the diff at its stat, and
`documents:plan` giving up the residual with `documents:review` and `item` untouched. Everything else
is generated after the constants exist (`demo_trim_record()`, the golden `.snap`s, the Runs pane's
`~Nk` line), so nothing pre-computed is stale; the one text to touch is B.16's illustrative header
line, whose `· chars-v1` reads `· chars-v2`.

### B.4 `crates/htui-core/src/prompt/render.rs` (T64)

Every renderer returns content **without** the wrapper; `wrap` is applied once, after trim. Every
content string has its line endings normalised on the way in (P-2), so every later byte count, marker
and hash is over LF text.

```rust
/// One rendered section before wrapping: what trim mutates and what the estimator measures once wrapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rendered {
    pub(crate) name: SectionName,
    pub(crate) attrs: Vec<(&'static str, String)>,   // source order; values already attribute-escaped
    pub(crate) content: String,                      // LF-normalised, no wrapper, no trailing LF
}

pub(crate) fn normalise_newlines(s: &str) -> String;                 // \r\n → \n, lone \r → \n, leading U+FEFF dropped
pub(crate) fn attr(value: &str) -> String;                           // & → &amp;, " → &quot;, any newline → one space (§4.2 rule 4)
pub(crate) fn title_attr(title: &str) -> String;                     // attr() then 120-byte char-boundary cut + "..."
pub(crate) fn one_line_title(title: &str) -> String;                 // §4.3: \r,\n → space, runs of spaces → one, 100 bytes + "..."
pub(crate) fn fence_for(content: &str) -> String;                    // backticks = longest run in content + 1, min 3 (§4.2 rule 3)
pub(crate) fn wrap(section: &Rendered) -> String;                    // "<section name=\"…\" k=\"v\">\n" + content + "\n</section>"

pub(crate) fn item(spec: &PromptSpec) -> Rendered;                   // attrs key, title
pub(crate) fn document(doc: &InputDocument) -> Rendered;             // documents:<kind>, attrs kind, version
pub(crate) fn upstream(entries: &[UpstreamEntry], states: &[UpstreamState]) -> Option<Rendered>;
pub(crate) fn box_profile(profile: &BoxProfile) -> Rendered;         // §4.2's fixed field list
pub(crate) fn skills(skills: &[BoundSkill]) -> Option<Rendered>;     // <skill name="…" version="N">\nbody\n</skill> blocks joined by "\n"
pub(crate) fn excerpts(files: &[Excerpt]) -> Option<Rendered>;       // §4.5's framing paragraph + <file> blocks, path order
pub(crate) fn verify_failure(v: &VerifyFailure) -> Rendered;         // attr exit_code; content = fence + output + fence
pub(crate) fn diff(name: SectionName, d: &DiffBlock, stat_only: bool) -> Rendered;
pub(crate) fn command_queue() -> Rendered;                           // defaults::COMMAND_QUEUE_TEXT
pub(crate) fn judge_task(text: &str) -> Rendered;
pub(crate) fn judge_candidate(c: &JudgeCandidate, stat_only: bool) -> Rendered;
pub(crate) fn step_summary(s: &StepSummary) -> Rendered;             // attrs turns, events; §4.6(c)'s four lines + tail
pub(crate) fn failure_reason(text: &str) -> Rendered;
pub(crate) fn elision_marker(lines: u32, bytes: u64) -> String;      // "[... htui elided {lines} lines / {bytes} bytes ...]"

impl StepSummary { pub fn from_events(events: &[SessionEvent]) -> Self; }
impl UpstreamEntry { pub(crate) fn one_liner(&self, pending: bool) -> String; pub(crate) fn summary_block(&self, body: &str) -> String; }
```

The upstream render, byte-exact (`:710-745`): Summary blocks first in canonical order, each
`### {qualified_key} - {title} ({status}, {depth} hop|hops)\n\n{body}` joined by `\n\n`; then, if any
one-liner exists, `\n\nalso upstream:\n` followed by `- {key} - {title} ({status})` lines
(` - no summary yet` appended for Pending) in canonical order. `UpstreamState { Summary, Pending,
Stub, Dropped }` per entry is the ladder's output (D.3); an entry whose `summary` is `Some` but whose
state is `Pending` renders the Pending line — that is the ladder's degrade.

The excerpt `<file>` block: `<file path="{repo}:{path}" lines="{first}-{last}" reason="{reason}"(
truncated="true")>` then `{n:>4} | {line}` per line (D98: line numbers on, both families), then the
marker when truncated, then `</file>`. Line-number width is the width of the **last** line number of
the file, minimum 4, so a file never re-aligns under trim.

The box profile (`:516-536`): `hostname:`, `os: {os_family} {os_version} ({arch})`, `cpu: {cpu}`
(omitted when empty), `ram: {ram_mb} MB` when `Some`, `gpu: {vendor}` when
`gpu_present && vendor.is_some()`, `htui: {htui_version}`, `tools: name version, …` (24 max,
`, +N more`; omitted when no rows; a bare name when `version` is empty), `quirks: …` when non-empty
with newlines → `; `. No `path`, no tags, no settings.

### B.5 `crates/htui-core/src/prompt/trim.rs` (T65)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimStrategy { None, HeadTail, TailCut, StatOnly, StubLadder, Dropped }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Section {
    pub name: SectionName,
    pub tokens_before: i64,
    pub tokens_after: i64,
    pub strategy: TrimStrategy,
    pub trimmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")] pub elided_lines: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub elided_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub stubbed: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub dropped: Option<u32>,
}

/// §5.2's abridged projection. Constructed by `TrimRecord::section_entries` and by nothing else.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SectionEntry { pub name: String, pub tokens: i64, pub trimmed: bool }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TemplateRecord { pub name: String, pub version: i32, pub role: TemplateRole }

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TrimRecord {
    pub v: u8,
    pub template: TemplateRecord,
    pub budget: i64,
    pub budget_source: BudgetSource,
    pub reserve: f64,
    pub target: i64,
    pub estimator: &'static str,
    pub estimated_before: i64,
    pub estimated_after: i64,
    pub sections: Vec<Section>,
    pub excerpts: ExcerptAudit,
    pub notes: Vec<String>,
}
impl TrimRecord {
    pub fn to_value(&self) -> serde_json::Value;             // serde_json::to_value; BTreeMap keys → byte-stable
    pub fn section_entries(&self) -> Vec<SectionEntry> {     // THE map, ANA-5 :1551-1554
        self.sections.iter().map(|s| SectionEntry { name: s.name.render(), tokens: s.tokens_after, trimmed: s.trimmed }).collect()
    }
}

pub(crate) fn trim_order(role: TemplateRole, spec: &PromptSpec) -> Vec<SectionName>;
pub(crate) fn run(role: TemplateRole, rendered: &mut Vec<Rendered>, template_tokens: i64,
                  target: i64, est: TokenEstimator, spec: &PromptSpec) -> Result<Vec<Section>, AssembleError>;
pub(crate) fn head_tail(content: &str, floor_lines: usize, reclaim: i64, est: TokenEstimator) -> Option<(String, u32, u64)>;
pub(crate) fn tail_cut(content: &str, floor_lines: usize, reclaim: i64, est: TokenEstimator) -> Option<(String, u32, u64)>;
```

### B.6 `crates/htui-core/src/prompt/digest.rs` (T64)

```rust
/// §4.7 steps 5–6 in the order P-2 fixes: normalise newlines, collapse 3+ LF to 2, trim trailing
/// whitespace, append exactly one LF.
pub fn canonical(text: &str) -> String;
/// Step 7: sha256 over the UTF-8 bytes, lowercase hex, 64 chars. `sha2::Sha256`, the same call
/// `record.rs:589` makes — so the recorder's recomputation is an identity (H-7).
pub fn sha256_hex(text: &str) -> String;
```

### B.7 `crates/htui-core/src/prompt/excerpt.rs` (T61 the prefix, T66 the rest) and `crates/htui-agent/src/excerpt.rs` (T66)

Core, pure:

```rust
/// ANA-2 §4.7's truncation (`docs/ANA-2.md:1074-1077`): cut at the first of `*?[{`, then at the last
/// `/`; repo-qualified (`repo:glob`), a bare glob meaning `primary`. `**` → the empty prefix.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathPrefix { pub repo: String, pub prefix: String }
impl PathPrefix {
    pub fn parse(touched: &str, primary_repo: &str) -> Self;
    pub fn matches(&self, repo: &str, path: &str) -> bool;     // repo == && path.starts_with(prefix) (bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)] pub struct RepoPath { pub repo: String, pub path: String }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct RepoRoot { pub repo: String, pub root: std::path::PathBuf, pub source: RootSource }
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum RootSource { RunStepTree, RepoBoxPath, NoPath }

#[derive(Debug, Clone)]
pub struct ExcerptRequest<'a> {
    pub item_key: &'a str,
    pub item_body: &'a str,
    pub phase: &'a str,
    pub document_bodies: &'a [&'a str],          // resolved input documents (tier 3 reads them too, :1028)
    pub touched_prefixes: &'a [PathPrefix],
    pub changed_paths: &'a [RepoPath],
    pub roots: &'a [RepoRoot],
    pub budget_tokens: i64,
    pub caps: ExcerptCaps,
    pub deadline: core::time::Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptCandidate { pub repo: String, pub path: String, pub lines: Option<(u32, u32)>, pub weight: u16, pub reason: String }

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("excerpt provider `{provider}`: {message}")]
pub struct ProviderError { pub provider: String, pub message: String }

/// §4.5's seam, five rules in the doc comment verbatim. Sync: a provider is a lookup, and the
/// deadline runner (`htui_agent::excerpt::run_providers`) gives it a thread.
pub trait ExcerptProvider: Send + Sync + core::fmt::Debug {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn propose(&self, req: &ExcerptRequest<'_>) -> Result<Vec<ExcerptCandidate>, ProviderError>;
}

/// Read-only by type (invariant 6): list and read, nothing else.
/// *(These two signatures are the splice reconstruction — see the note at the top.)*
pub trait RepoReader: Send + Sync + core::fmt::Debug {
    /// Repo-relative `/`-separated paths in byte order, content-skip rules already applied;
    /// `true` when the scan cap bit and the listing is partial (`scan_truncated`).
    fn list(&self, root: &RepoRoot, cap: u32) -> Result<(Vec<String>, bool), ProviderError>;
    /// The file's text, LF-normalised. A path the reader cannot read is an error, never a panic.
    fn read(&self, root: &RepoRoot, path: &str) -> Result<String, ProviderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum ExcerptReason { TouchedPath, PrevDiff, Mentioned, Identifier, Lexical, Provider }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    pub repo: String, pub path: String,
    pub first_line: u32, pub last_line: u32,
    pub truncated: bool, pub elided_lines: u32, pub elided_bytes: u64,
    pub rank: u32, pub weight: u16, pub reason: ExcerptReason, pub provider: Option<String>,
    pub content: String,                          // the windowed lines, LF, unnumbered
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExcerptCaps { pub max_files: u32, pub file_line_cap: u32, pub head_lines: u32, pub max_file_bytes: u64 }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RootRecord { pub repo: String, pub source: RootSource, pub scan_truncated: bool }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileRecord { pub repo: String, pub path: String, pub lines: String, pub rank: u32, pub weight: u16,
                        pub reason: ExcerptReason, pub truncated: bool, pub bytes: u64, pub sha256: String }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExcerptAudit {
    pub provider_set: Vec<String>,   // P-11's grammar
    pub roots: Vec<RootRecord>,
    pub considered: u32,
    pub selected: u32,
    pub caps: ExcerptCaps,
    pub files: Vec<FileRecord>,      // filled by assemble() from the survivors; empty when the section was dropped
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExcerptSet { pub files: Vec<Excerpt>, pub audit: ExcerptAudit, pub notes: Vec<String> }

/// Path-only skip rules, applied by the ranker over the listing so a `FakeRepoReader` exercises
/// them (criterion 14): `.git/`, the secret denylist (`:1071`), lockfiles and minified assets (`:1075`).
pub fn skip_by_path(path: &str) -> Option<&'static str>;   // Some(rule name)

/// The built-in provider: tiers 1–4 over the listing, tier 5 over the first 4 KB. `builtin@1`.
#[derive(Debug, Default, Clone, Copy)] pub struct BuiltinRanker;

/// §4.5 steps 3–9 over a reader and a candidate list already merged from every provider.
/// Pure over `reader`: with `FakeRepoReader` no filesystem is touched.
pub fn select(reader: &dyn RepoReader, req: &ExcerptRequest<'_>, merged: Vec<ExcerptCandidate>,
              providers: Vec<String>, est: TokenEstimator) -> ExcerptSet;
```

Agent half:

```rust
/// `std::fs` over a root: depth-first, entries sorted by file-name byte order at every level
/// (`probe.rs:745-800`'s walk, sorted), the content skip rules in §4.5's order (`.git/` pruned at
/// the directory; the denylist and lockfile rules via `skip_by_path`; a `.gitignore` subset
/// matcher — anchored and unanchored literal prefixes, `*.ext` suffixes, trailing `/` for
/// directories, `!` ignored — read once per directory; NUL in the first 8 KB; size over
/// `max_file_bytes`), the scan cap and `truncated`.
#[derive(Debug, Default, Clone, Copy)] pub struct FsRepoReader;
impl RepoReader for FsRepoReader { /* list, read */ }

/// Runs every provider on its own thread under `deadline`; a provider that errors, panics (the
/// join handle's `Err`) or misses the deadline is dropped and recorded (`name@version:status`).
/// `builtin` is always first and never dropped.
pub fn run_providers(providers: &[&dyn ExcerptProvider], req: &ExcerptRequest<'_>)
    -> (Vec<ExcerptCandidate>, Vec<String>);
```

Providers proposing is sync and the assembler is sync; the store reads that fill `PromptSpec` are the
only awaits, in the caller.

### B.8 `crates/htui-core/src/prompt/settings.rs` (T65; D101)

```rust
/// The ten §5.3 defaults, verbatim from migration 0002 (`0002_agent_probe.sql:68-79`); asserted
/// equal to the rows by `crates/htui-store/tests/migrations.rs`.
pub const DEFAULTS: Defaults = Defaults { token_budget: 120_000, prompt_reserve_fraction_bp: 1_000,
    prompt_upstream_hops: 2, max_skill_tokens: 20_000, excerpt_max_files: 12, excerpt_file_line_cap: 400,
    excerpt_head_lines: 200, excerpt_max_file_bytes: 524_288, excerpt_max_scan_files: 20_000,
    excerpt_provider_deadline_ms: 1_500 };
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Defaults { /* the ten, integers; reserve in basis points */ }
impl Defaults { pub fn as_rows(&self) -> Vec<(&'static str, serde_json::Value)>; }   // key byte order

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum BudgetSource { Phase, Project, AppSetting, AppSettingDefault }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget { pub tokens: i64, pub source: BudgetSource, pub reserve_bp: u32 }
impl Budget {
    /// `floor(budget × (1 − reserve))` in integer arithmetic: `tokens × (10_000 − reserve_bp) / 10_000`.
    pub const fn target(&self) -> i64;
    pub fn reserve(&self) -> f64 { f64::from(self.reserve_bp) / 10_000.0 }
}

/// The chain of `docs/ANA-2.md:284`, plus D101's rung.
pub fn resolve_budget(phase: Option<i32>, project: Option<&Value>, app: &BTreeMap<String, Value>) -> Budget;
/// `project.settings.upstream_hops`, then `app_setting.prompt_upstream_hops`, then 2; clamped to
/// `1..=2` with a note when clamped (§4.3 step 2).
pub fn resolve_hops(project: Option<&Value>, app: &BTreeMap<String, Value>, notes: &mut Vec<String>) -> u8;
pub fn resolve_max_skill_tokens(app: &BTreeMap<String, Value>) -> i64;
pub fn resolve_excerpt_caps(app: &BTreeMap<String, Value>) -> (ExcerptCaps, u32 /*scan*/, Duration /*deadline*/);
```

A key that is absent, `null`, non-numeric or non-positive falls to the next rung (the
`connect.rs:299-302` rule); `prompt_reserve_fraction` is read as `f64`, rounded to basis points,
clamped to `0..=5_000`.

### B.9 `crates/htui-core/src/prompt/defaults.rs` (T60; D104)

```rust
/// §5.4's ten bodies verbatim, in §5.4 order: prd, plan, implement, review, research, verdict,
/// reproduce, fix, judge, handoff. MOD-15 seeds from this; `fixtures.rs` seeds from this.
pub const DEFAULT_TEMPLATES: [(&str, TemplateRole, &str); 10];
/// The `command_queue` section's two sentences (§4.2 `:484`, §4.4 `:827`). Fixed here because
/// every byte of a section is a digest input.
pub const COMMAND_QUEUE_TEXT: &str =
    "Route builds, test suites and verification through the `command_run` tool rather than a shell: htui queues them per box under `R-MCP-4` and records their output on this step. Run everything else directly.";
pub fn body_of(name: &str) -> Option<&'static str>;
```

The bodies are `include_str!`-free string literals so `cargo doc` shows them; the `review` front
matter and the `judge` verdict block are byte-identical to `:1684-1694` and `:1785-1793` (MOD-4
parses both).

### B.10 `crates/htui-core/src/prompt/fixtures.rs` (T64, T65, T66; `test-support`)

One `PromptSpec` constructor per case, every string a literal, no clock, no id:
`phase_implement_attempt2()` (the §5.1 shape: two documents, one previous diff, one upstream summary
at depth 1 and two one-liners, one skill, two excerpts), `phase_all_empty()` (criterion 4),
`phase_oversize()` (criterion 9, sized for `chars-v2` per B.3), `phase_protected_too_big()` and
`phase_skills_over_cap()` (criterion 10), `judge_three_candidates()` (criterion 17),
`handoff_basic()` (criterion 18's assembler half), `demo_trim_record()` (the `STEP_IMPL` fixture
value for T68 — `assemble(&phase_implement_attempt2())`'s record with `budget_source: Project`).
`with_crlf(spec) -> PromptSpec` maps every body to CRLF (criterion 6).

### B.11 The model additions (T61, T62)

`model/skill.rs`:

```rust
/// A row of `skill` (§5.6, `0001_init.sql:406-413`).
pub struct Skill { pub id: SkillId, pub name: String, pub description: String, pub created_by: UserId, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }
/// A row of `skill_version` (`:419-425`).
pub struct SkillVersion { pub skill_id: SkillId, pub version: i32, pub body: String, pub created_by: UserId, pub created_at: DateTime<Utc> }
/// A row of `skill_binding` (`:431-441`); `phase_id: None` is project level.
pub struct SkillBinding { pub id: SkillBindingId, pub skill_id: SkillId, pub project_id: ProjectId, pub phase_id: Option<PhaseId>, pub pinned_version: Option<i32>, pub position: i32, pub updated_at: DateTime<Utc> }
/// The R-SKL-2 resolution of one binding: the skill, the version in force, the position. Not a table.
pub struct BoundSkill { pub skill_id: SkillId, pub name: String, pub version: i32, pub position: i32, pub body: String }
impl BoundSkill {
    /// §4.2's collapse: phase bindings override project bindings per `skill_id`; order
    /// `(position, name bytes)`; each skill once. Pure — every backend and the assembler call it.
    pub fn collapse(project: Vec<BoundSkill>, phase: Vec<BoundSkill>) -> Vec<BoundSkill>;
}
```

`SkillBindingId` joins `ids.rs`'s list (`:77-112`) — `skill_binding.id` is a `UUID PRIMARY KEY`
(`0001_init.sql:432`) and the crate rule is one newtype per key (`ids.rs:3-5`).

`model/link.rs` after `:92`: `UpstreamEntry` exactly as ANA-5 `:761-770` plus

```rust
impl UpstreamEntry {
    /// §4.7 rule 2: `(depth, qualified_key bytes, item_id)`. `str::cmp` is byte order in Rust, so
    /// this is the one sort both SQL backends and `MemStore` run after their own `ORDER BY`.
    pub fn sort_canonical(entries: &mut [UpstreamEntry]);
    pub fn is_summary(&self) -> bool { self.in_scope && self.summary.is_some() }
    pub fn is_pending(&self) -> bool { self.in_scope && self.summary.is_none() }
}
```

`model/box_.rs` after `:92`: `BoxProfile { hostname, os_family: OsFamily, os_version, arch, cpu,
ram_mb: Option<i32>, gpu_vendor: Option<String>, htui_version, tools: Vec<(String, String)>, quirks:
String }` with `BoxProfile::project(row: &BoxRow, mut tools: Vec<BoxTool>) -> Self` (sorts tools by
name bytes, keeps `(name, version)`, drops `path`, `gpu_vendor` only when `gpu_present`).

`model/scope.rs` after `:42`:

```rust
/// D95: the bound of the upstream walk — a workspace, or one project when there is none (`R-ENT-2`).
/// Not `Scope`: `Scope` is "always one workspace" (`:8`) and has twenty-plus consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptScope { pub workspace: Option<WorkspaceId>, pub project: ProjectId }
impl PromptScope {
    pub fn from_scope(scope: &Scope, project: ProjectId) -> Self { Self { workspace: Some(scope.workspace_id), project } }
    pub const fn project_only(project: ProjectId) -> Self { Self { workspace: None, project } }
}
```

`model/run.rs`: the two fields after `finished_at` (`:286`), documented as D106's projection, and

```rust
/// D106's derivation, in one place so `MemStore` and the two SQL projections agree by test rather
/// than by luck: `estimated_after` as `i32`, and whether any `sections[].trimmed` is `true`.
pub fn prompt_summary(trim_record: Option<&Value>) -> (Option<i32>, bool);
```

### B.12 The store seam (T62), and the six `WriteStore` implementors

`traits.rs`, `ReadStore` after `step_events` (`:47`):

```rust
    /// One document with its body (ANA-5 §8; mirrored with its body, `0001_mirror.sql:97-101`).
    async fn document(&self, id: DocumentId) -> Result<Option<Document>>;
    /// The latest version of each `kind`, in `kinds` order, kinds with no row omitted. An empty
    /// `kinds` means every kind the item has, in kind byte order (P-12; the D103 preview form).
    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>>;
    /// Upstream items over `blocked_by` and `origin` edges, up to `hops` (clamped `1..=2`), one row
    /// per item at its minimum depth, classified against `scope` (R-PRM-1, R-PRM-2): ANA-9 §7.3 as
    /// amended by ANA-5 §4.3, in canonical order (`UpstreamEntry::sort_canonical`).
    async fn upstream_summaries(&self, id: ItemId, hops: u8, scope: &PromptScope) -> Result<Vec<UpstreamEntry>>;
    /// One project row with its `settings` (ANA-5 §8; `project.settings` is mirrored, `:57-61`).
    async fn project(&self, id: ProjectId) -> Result<Option<Project>>;
```

`WriteStore` after `finish_chat_run` (`:201`):

```rust
    /// Writes `run_step.prompt_digest` and `run_step.trim_record`, both, and nothing else (ANA-5
    /// §4.4 `:959-970`): the pre-flight audit of R-PRM-3 / R-ORCH-11, written at stage 3 before a
    /// session starts. The same digest `Recorder::record_prompt` will recompute over the same
    /// text (`record.rs:589`), so the two writers of the column agree by construction.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] with `entity: "run_step"` when the step does not exist.
    async fn set_step_prompt(&self, step: StepId, digest: &str, trim: &Value) -> Result<()>;
```

| Implementor | `set_step_prompt` does |
|---|---|
| `MemStore` (`mem.rs:1057`) | `self.write(\|state\| state.set_step_prompt(step, digest, trim, now))`: `steps.get_mut` or `NotFound`; `row.prompt_digest = Some(digest.to_owned())`, `row.trim_record = Some(trim.clone())`, `updated_at = now` — the `:792-812` shape without the `COALESCE` |
| `PgStore` (`pg/write.rs:39`) | `UPDATE run_step SET prompt_digest = $2, trim_record = $3 WHERE id = $1`, `rows_affected() == 0` → `NotFound` (`:359-364`). The trigger owns `updated_at` |
| `BufferedWriter` (`writer.rs:186`) | **`Err(StoreError::Unreachable(PROMPT_ON_SERVER_ONLY))`** (E-8). Reasoning, stated in its doc: `set_step_usage`'s no-op (`:253-267`) is licensed by `upload_pending` recomputing the figure from the uploaded rows (D36); `trim_record` has no such source — the payload's `sections[]` is its lossy projection — and a silent no-op would leave a step with a `prompt` row and no audit, which is the one thing `R-PRM-3`'s "recorded on the step" forbids. `R-STO-4` starts no graph run offline, so nothing reaches this arm in this milestone; when MOD-4 does, the refusal is the signal that an offline graph step needs a design, not a stub |
| `Writer` (`writer.rs:419`) | Three-arm delegation in the `:457-468` shape |
| `UsageSpy` (`conformance.rs:616`) | `self.inner.set_step_prompt(step, digest, trim).await` — no log; the spy watches `set_step_usage` and the latch |
| `SpyStore` (`recorder.rs:274`) | Delegation, plus a `prompt_calls: Mutex<Vec<(StepId, String)>>` log so T65's recorder case can assert the digest it was given |

`conformance.rs`: the 23rd case, `set_step_prompt_writes_digest_and_trim`: write
`("9f8e", json!({ "estimated_after": 34_000, "sections": [{ "name": "template", "trimmed": false },
{ "name": "excerpts", "trimmed": true }], "v": 1 }))` on `ids::STEP_IMPL`; `runs(ids::HTUI_FEAT_1)` →
the step with `id == STEP_IMPL` has `prompt_tokens == Some(34_000)` and `trimmed == true`; a second
write with `"sections": []` and `"estimated_after": 12` → `Some(12)`, `false`;
`set_step_prompt(StepId::new(), ..)` → `NotFound { entity: "run_step" }` (the `:1110-1120` shape).
`MemStore::demo()`'s untouched `STEP_PLAN` keeps `prompt_tokens == None`.

`READ_CASES` (D96), additive, after `run_all` (`:108`):

```rust
/// Read-only cases, so the mirror can be a target too (plan D96). Sorted, never renamed.
pub const READ_CASES: &[&str] = &[
    "document_body_round_trip",
    "documents_of_kinds_latest_per_kind_in_order",
    "project_row_has_settings",
    "upstream_diamond_dedup",
    "upstream_in_scope_no_summary",
    "upstream_out_of_scope_stub",
];
pub async fn run_read_case<S: ReadStore>(name: &str, store: &S);
pub async fn run_all_reads<S, F, Fut>(make: F) where S: ReadStore, F: Fn() -> Fut, Fut: Future<Output = S>;
```

The cases, over the fixture of C.4: `document_body_round_trip` — `document(ids::DOC_FEAT_1_PLAN_V2)`
equals the fixture row body (`fixtures.rs:1096`), `document(DocumentId::new())` is `None`.
`documents_of_kinds_latest_per_kind_in_order` — `documents_of_kinds(HTUI_FEAT_1, &["prd","plan"])` is
`[prd v1, plan v2]`; with `&["plan","prd","missing"]` it is `[plan v2, prd v1]`; with `&[]` it is
`[plan v2, prd v1]` (byte order). `project_row_has_settings` — E-3. `upstream_diamond_dedup` —
`upstream_summaries(AGY_FIX_1, 2, &platform)` has exactly one `HTUI_ANA_1` at `depth == 2`,
`summary == Some(…)`, and the whole vector equals the canonical-sorted copy of itself; with
`hops == 1` `HTUI_ANA_1` is absent. `upstream_in_scope_no_summary` — `AGY_ANA_1` and `HTUI_TOOL_1`
are `in_scope && summary.is_none()` at depth 1. `upstream_out_of_scope_stub` — `VULKAN_FEAT_1` is
`!in_scope`; and under `PromptScope::project_only(PROJECT_AGY)` `HTUI_ANA_1` is `!in_scope` with
`summary == None` even though a summary document exists (the amended query's separability,
`:630-634`).

Bindings: `mem_store.rs` runs `run_all_reads` over `MemStore::demo()`; `pg_conformance.rs` loops
`READ_CASES` over `demo_db().store` (`:33-43`); `cache.rs` runs it over a `CacheStore` after one
`run_pass` over `all_projects()` (`:236-238`).

### B.13 `PgStore` inherent reads, `CacheStore`, `Backend` (T63)

Inherent on `PgStore` (`pg/read.rs` after `:628`) and `MemStore` (after `:244`), dispatched by
`Backend` (after `:318`) with the `Offline` arm refusing (E-2):

```rust
pub async fn prompt_templates(&self, project: ProjectId) -> Result<Vec<PromptTemplate>>;   // ORDER BY name, version
pub async fn bound_skills(&self, project: ProjectId, phase: Option<PhaseId>) -> Result<Vec<BoundSkill>>;
pub async fn box_profile(&self, id: BoxId) -> Result<Option<BoxProfile>>;                  // box + box_tool ORDER BY name
pub async fn app_settings(&self) -> Result<BTreeMap<String, Value>>;
pub async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>>;                        // ORDER BY is_primary DESC, name
pub async fn repo_paths(&self, box_id: BoxId) -> Result<Vec<RepoBoxPath>>;
pub async fn item_kind(&self, id: ItemKindId) -> Result<Option<ItemKind>>;
```

`bound_skills` SQL: two selects — project-level (`phase_id IS NULL`) and phase-level
(`phase_id = $2`) — each joined to `skill` and to the version in force
(`COALESCE(b.pinned_version, (SELECT MAX(version) FROM skill_version WHERE skill_id = b.skill_id))`),
then `BoundSkill::collapse` in Rust so the SQL and `MemStore` share one rule. `MemStore::State` gains
`skills: HashMap<SkillId, Skill>`, `skill_versions: Vec<SkillVersion>`,
`skill_bindings: Vec<SkillBinding>`, `box_tools: Vec<BoxTool>`, `repos: Vec<Repo>`,
`repo_paths: Vec<RepoBoxPath>`, `app_settings: BTreeMap<String, Value>` (empty unless a test sets it
through a new `MemStore::set_app_setting(key, value)` — the only writer, test-only by doc).

`CacheStore` implements the four `ReadStore` reads (C.2) and none of the inherents; `Backend`'s
`Offline` arm of each inherent: `Err(StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))`.
`pub const PROMPT_ON_SERVER_ONLY: &str = "the prompt preview needs the server: templates, skills and
box tools are not mirrored";` beside `REGISTRY_ON_SERVER_ONLY` (`writer.rs:353`), re-exported from
`htui_store`.

`RunStepSummary` projections (E-9): Postgres, appended to the steps query (`pg/read.rs:317-332`):

```sql
       (s.trim_record->>'estimated_after')::int                                         AS "prompt_tokens?",
       COALESCE((SELECT bool_or((e->>'trimmed')::bool)
                   FROM jsonb_array_elements(s.trim_record->'sections') e), false)      AS "trimmed!"
```

SQLite, appended to `cache/read.rs:480-485`'s text:

```sql
       json_extract(trim_record, '$.estimated_after')                                    AS prompt_tokens,
       COALESCE((SELECT max(json_extract(e.value, '$.trimmed'))
                   FROM json_each(run_step.trim_record, '$.sections') e), 0)            AS trimmed
```

read with `get::<Option<i64>>` → `i32::try_from`, and `bool_col(get::<i64>)`. `MemStore::run_steps`
(`:497-521`) calls `prompt_summary(step.trim_record.as_ref())`. The cache parity test (`cache.rs:229`,
`runs` at `:311-315`) is what proves the three agree once T68's fixture step carries a record.

### B.14 Not a seam change: the recorder

No `record.rs` change. `record_prompt` (`:571-603`) scrubs the payload (must find nothing — the
assembler scrubbed, and masking is idempotent, `scrub.rs:52`), recomputes `Sha256(text)` (`:589`) —
identical to `digest::sha256_hex` on the same bytes — and passes it to `set_step_usage` as `Some`
(`:1066-1075`), which rewrites `prompt_digest` with the value `set_step_prompt` already wrote. D97's
"the assembler path passes `None`" is therefore moot rather than unimplemented: both writers write
one value, and H-7's test pins it.

### B.15 `crates/htui/src/preview.rs` and the worker (T67; D102, D103, E-10)

```rust
/// What the Prompt sub-tab receives: one reply per request, the assembler's verdict either way.
#[derive(Debug, Clone)]
pub struct PromptPreview {
    pub item: ItemId,
    pub available: Vec<String>,          // the project's template names minus the two reserved ones
    pub template: Option<TemplateRef>,   // the row rendered, None when `available` is empty
    pub outcome: Result<AssembledPrompt, String>,   // Err = AssembleError::to_string(): a refusal is a preview too
}

/// D103's spec builder: eight reads, every stand-in declared in `notes`. Never writes.
pub async fn build_spec(backend: &Backend, item: ItemId, template_name: Option<&str>, scope: &Scope)
    -> Result<(PromptSpec, Vec<String>), StoreError>;
/// The deferred task: `build_spec`, `assemble`, one reply at `addr`. Owns a `Backend` clone.
pub async fn run_preview(backend: Backend, item: ItemId, template_name: Option<String>, scope: Scope, frames: Frames);
```

`AgentRuntime::preview` (the `probe` shape, `agent_worker.rs:678-735`): refuse `Backend::Offline`
before spawning with `failed("prompt_preview", &StoreError::Unreachable(PROMPT_ON_SERVER_ONLY))`;
else `self.background.push(tokio::spawn(preview::run_preview(backend.clone(), ..)))` and
`Ok(Served::Deferred)`.

`StoreRequest::PromptPreview { item: ItemId, template_name: Option<String>, scope: Scope }` (`name()`
→ `"prompt_preview"`); `StoreReply::PromptPreview(Box<PromptPreview>)`. The Backlog tab's `go()`
(`backlog/mod.rs:112-116`) issues it as the sixth read with `template_name: None` and
`scope: ctx.scope.clone()`; freshness is `App::is_fresh`'s per-`(origin, discriminant)` rule
(`state.rs:279-290`), so a burst of `j` presses drops every stale preview.

### B.16 `crates/htui/src/ui/tabs/backlog/detail/prompt.rs` (T67)

`pub const ID: DetailId = DetailId("prompt")`, title `"Prompt"`, no store handle (`runs.rs:5`,
`R-NF-3`). State: `item: Option<ItemId>`, `preview: Option<PromptPreview>`, `scroll: Scroll`,
`lines: Vec<String>` (rebuilt on reply). `on_item_change` clears all three and resets the scroll
(`runs.rs:140-145`'s shape). `on_key`: `n` / `p` cycle `available` around `preview.template.name` and
`ctx.request(StoreRequest::PromptPreview { .. })` — `Handled::Consumed`, `Handled::Pass` when there
is nothing to cycle; everything else `self.scroll.on_key(key, self.lines.len())`. `on_reply`:
`StoreReply::PromptPreview(p) if p.item == self.item` → store, rebuild `lines`, reset scroll.
`render`: `message` for no item / no reply yet (`"Assembling the prompt…"`) / `available.is_empty()`
(`"No prompt templates in this project."`); otherwise a `Paragraph` over `lines` scrolled by
`self.scroll.offset()` (`body.rs:115`'s call), no wrap. The line list, top to bottom:

```
template   implement v1 (phase)          n/p: other template
digest     9f2c…b1e2
budget     120000 (project) · reserve 0.1 · target 108000 · chars-v2
tokens     165000 → 108000
section            before   after  strategy
template              210     210  none
documents:plan      71800   65090  head_tail  elided 512 lines / 23485 bytes
…
excerpts   builtin@1 · roots: htui no_path · considered 0 · selected 0
notes      preview: documents are latest-per-kind; ANA-2 input_kinds resolution arrives with MOD-4
           preview: …
───
<the canonical text, verbatim>
```

An `Err` outcome renders the same header with `refused    prompt budget too small: …` in place of the
digest line and no text. Nothing here parses the text.

### B.17 The Runs pane (T68; D106)

`runs.rs:215-235`: a step whose `prompt_tokens.is_some() || trimmed` renders as a **two-line** row
(`Row::height(2)`, the run rows' own `ROW`), the second line placed in the phase cell: `~34k` or
`~34k !` (`!` = `trimmed`). Format: `< 1 000` → `~812`, `< 1 000 000` → `~34k` (integer thousands,
rounded), else `~1.2M`. A step with neither stays one line, so `backlog__detail_runs.snap` changes by
exactly one line and column widths (`:250-260`) do not move — the reason for a second line rather
than a fifth column: the pane is 43 columns and the four columns plus spacing are 41 (`:22-25`), and
`~120k !` does not fit any cell without clipping a number.

---

## C. The amended §7.3 walk, three backends

### C.1 Postgres (`pg/read.rs`, `sqlx::query_as!`)

```sql
WITH RECURSIVE up AS (
    SELECT l.to_item_id AS item_id, 1 AS depth
      FROM item_link l
     WHERE l.from_item_id = $1 AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL
    UNION
    SELECT l.to_item_id, up.depth + 1
      FROM item_link l JOIN up ON l.from_item_id = up.item_id
     WHERE up.depth < $2 AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL
),
best AS (SELECT item_id, MIN(depth) AS depth FROM up GROUP BY item_id),
scope AS (
    SELECT project_id FROM workspace_project WHERE workspace_id = $3::uuid
    UNION SELECT $4::uuid WHERE $3::uuid IS NULL
)
SELECT i.id                       AS "item_id!",
       p.slug || ':' || i.key     AS "qualified_key!",
       i.title                    AS "title!",
       i.status                   AS "status!",
       best.depth                 AS "depth!",
       (s.project_id IS NOT NULL) AS "in_scope!",
       CASE WHEN s.project_id IS NOT NULL THEN d.body END AS "summary?"
  FROM best
  JOIN item i    ON i.id = best.item_id
  JOIN project p ON p.id = i.project_id
  LEFT JOIN scope s ON s.project_id = i.project_id
  LEFT JOIN LATERAL (SELECT body FROM document
                      WHERE item_id = i.id AND kind = 'summary'
                      ORDER BY version DESC LIMIT 1) d ON TRUE
 ORDER BY best.depth, qualified_key, i.id
```

Binds: `id.as_uuid()`, `i32::from(hops)`, `scope.workspace.map(WorkspaceId::as_uuid)` (an
`Option<Uuid>` — the `$3::uuid` cast is what lets `query!` type a nullable parameter),
`scope.project.as_uuid()`. The `!`/`?` overrides are needed because `query_as!` infers every
recursive-CTE column nullable (the `links` query already does this, `:174-180`). `UpstreamRow`
(`pg/rows.rs`) carries `depth: i32` and narrows as `LinkNodeRow::into_node` does (`:153-163`). Then
`UpstreamEntry::sort_canonical(&mut rows)` **after** the SQL `ORDER BY`, because Postgres orders
`qualified_key` by collation and the mirror by bytes (`:675-681`).

### C.2 SQLite (`cache/read.rs`, runtime `sqlx::query`)

The three mechanical substitutions of `cache/read.rs:7-12` plus no `LATERAL`: a correlated subquery,
and no `deleted_at`.

```sql
WITH RECURSIVE up(item_id, depth) AS (
        SELECT to_item_id, 1 FROM item_link
         WHERE from_item_id = ? AND kind IN ('blocked_by','origin')
    UNION
        SELECT l.to_item_id, up.depth + 1
          FROM item_link l JOIN up ON l.from_item_id = up.item_id
         WHERE up.depth < ? AND l.kind IN ('blocked_by','origin')
),
best AS (SELECT item_id, MIN(depth) AS depth FROM up GROUP BY item_id),
scope AS (
        SELECT project_id FROM workspace_project WHERE workspace_id = ?
    UNION
        SELECT ? WHERE ? IS NULL
)
SELECT i.id AS item_id, p.slug || ':' || i.key AS qualified_key, i.title, i.status, best.depth,
       (s.project_id IS NOT NULL) AS in_scope,
       CASE WHEN s.project_id IS NOT NULL THEN
            (SELECT body FROM document d WHERE d.item_id = i.id AND d.kind = 'summary'
              ORDER BY d.version DESC LIMIT 1) END AS summary
  FROM best
  JOIN item i    ON i.id = best.item_id
  JOIN project p ON p.id = i.project_id
  LEFT JOIN scope s ON s.project_id = i.project_id
 ORDER BY best.depth, qualified_key, i.id
```

Five binds in order: `id.to_string()`, `i64::from(hops)`, `workspace.map(|w| w.to_string())` (an
`Option<String>`), `project.to_string()`, `workspace.map(..)` again — plain `?` placeholders bound
positionally, the file's own style (`:313-330`), so the workspace is bound twice rather than relying
on `?NNN`. Decoding through `uuid_col`, `text`, `get::<Status>`, `bool_col(get::<i64>)`, `opt_text`
(`:52-115`). Then the same `sort_canonical`.

### C.3 `MemStore` (`mem.rs`, beside `link_graph` `:406-459`)

Breadth-first over `links` where
`deleted_at.is_none() && matches!(kind, BlockedBy | Origin) && from_item_id == current`, following
**`to_item_id` only** (directed, unlike `link_graph`), `hops` clamped to `1..=2`; a `seen` set gives
`MIN(depth)` for free; per reached item:
`qualified_key = format!("{}:{}", project_slug(item.project_id), item.key)`,
`in_scope = scope.workspace.map_or(item.project_id == scope.project, |ws| workspace_projects.iter()
.any(|m| m.workspace_id == ws && m.project_id == item.project_id))`,
`summary = in_scope.then(|| documents.iter().filter(kind == "summary" && item_id == id)
.max_by_key(version).map(body))`. Then `sort_canonical`. `hops == 0` returns an empty vector.

### C.4 The fixture diamond (`fixtures.rs:956-993` gains five rows)

Every existing pinned set survives: `links_hops_1_vs_2` pins the exact hop-1 and hop-2 sets of
`HTUI_FEAT_2` (`conformance.rs:609`, `:647-657`) and no new edge touches `FEAT_2`, `FEAT_1` or
`AGY_FEAT_1`; the ready list (`:758`) is unchanged because only `origin` edges and a `blocked_by` to
a **done** item are added.

| # | from | kind | to | Role in the walk from `AGY_FIX_1` |
|---|---|---|---|---|
| 8 | `AGY_FIX_1` | `blocked_by` | `AGY_ANA_1` (done, no summary) | depth 1, **Pending** |
| 9 | `AGY_FIX_1` | `origin` | `HTUI_TOOL_1` (awaiting_approval) | depth 1, **Pending** |
| 10 | `AGY_ANA_1` | `blocked_by` | `HTUI_ANA_1` (done, `DOC_ANA_1_SUMMARY`) | depth 2 via B |
| 11 | `HTUI_TOOL_1` | `blocked_by` | `HTUI_ANA_1` | depth 2 via C — the diamond |
| 12 | `AGY_FIX_1` | `origin` | `VULKAN_FEAT_1` (Graphics workspace) | depth 1, **Stub** |

The doc at `:955` becomes "eleven live edges and one tombstone"; `cache.rs:602`'s tombstone case and
the Graph snapshot (root `FEAT-1`, hops 1) are unaffected — H-14 lists both for the re-run. The
canonical render from `AGY_FIX_1` under the Platform scope is one Summary block (`htui:ANA-1`,
2 hops) then `also upstream:` with `agy:ANA-1 … - no summary yet`, `htui:TOOL-1 … - no summary yet`,
`vulkan-tutorials:FEAT-1 …`.

---

## D. The assembly and the trim, as procedures

### D.1 `assemble()` — §4.7's pipeline (`:1385-1398`), step by step

1. **Parse.** `template::parse(spec.role, &spec.body)`; `UnknownPlaceholder`/`WrongRole` →
   `AssembleError::UnknownPlaceholder`, the rest → `Template`. Role inputs: `Judge` without
   `spec.judge` → `RoleInputs { what: "judge inputs" }`; likewise `Handoff`.
2. **Render every section at full size** (B.4), one `Rendered` per placeholder in `parsed.used` order
   that has data; scalars are substituted directly (`item_title` through `attr`, never a section).
   `upstream` is re-sorted by `sort_canonical` and skills by
   `BoundSkill::collapse(spec.skills.clone(), vec![])` first — the assembler does not trust its
   caller's order (rules 2 and 4).
3. **Scrub per section (D100, P-1).** `let mut v = Value::String(content); scrubber.scrub(&mut v)?` →
   on `Err(Unmasked { path, rule })` → `AssembleError::Unmasked { section: name.render(), rule, path }`;
   the masked string replaces `content`. Attribute values are scrubbed the same way.
4. **Estimate.** `tokens_before[i] = est.estimate(&wrap(&rendered[i]))`;
   `template_tokens = est.estimate(&parsed.literal_text())`. A placeholder used *k* times counts its
   section *k* times (`:404-405`).
5. **The two refusals, in this order.** `skills_tokens > spec.max_skill_tokens` → `SkillsExceedCap`
   (before the budget, because it is the sharper message).
   `protected = template_tokens + Σ tokens of sections with is_protected(role)`; `protected > target`
   → `BudgetTooSmall { needed: protected, target }`.
6. **Trim** (D.2) when `Σ + template_tokens > target`.
7. **Substitute** in `parsed.spans` order: `Literal` verbatim, `Slot(p)` → the scalar, or
   `wrap(&rendered)` for a section with data, or `""` (empty section, no entry).
8. **Canonicalise and digest**: `digest::canonical` then `sha256_hex`.
9. **Record**: `TrimRecord { v: 1, template: {name, version, role}, budget, budget_source, reserve,
   target, estimator: est.id, estimated_before, estimated_after, sections, excerpts: audit with
   files[] from the surviving `Excerpt`s (sha256 over each `<file>` block's rendered bytes), notes:
   spec.notes ++ assembler notes }`. `sections[0]` is `template` always (P-9).

No step reads a clock, an environment variable or a map in iteration order; `BTreeMap` is the only
map type inside `prompt/`.

### D.2 The trim, §4.4's nine steps (`:853-879`) as one loop

Inputs: `rendered: Vec<Rendered>` in render order, `before: Vec<i64>`, `template_tokens`, `target`,
`est`, and the order from `trim_order(role, spec)`:

- **Phase**: `[Excerpts, Upstream, PreviousDiff, VerifyFailure, Documents(k…) as a group, Item]`
  (§4.4 `:834-845`; P-3 places `verify_failure` before the document group and the group before `item`).
- **Judge**: `[JudgeCandidate(*) as a group, JudgeTask]`.
- **Handoff**: `[DiffSoFar, StepSummary, Documents(k…) as a group]`.

```
deficit = Σ before + template_tokens − target
for step in order while deficit > 0:
    match strategy(step):
      Excerpts        → drop files from the highest rank number down, re-render and re-estimate after each,
                        until deficit ≤ 0 or no file remains; no file left → drop the section (entry {0, Dropped})
      Upstream        → the ladder, one rung at a time with re-estimation after each rung:
                        (a) depth-2 Summary rows → Pending one-liners (stubbed += 1 each, canonical order),
                        (b) depth-2 rows → Dropped (dropped += 1 each),
                        (c) depth-1 Summary rows → Pending;
                        floor = all rows one-liners; then drop the section
      PreviousDiff |
      DiffSoFar       → stat_only = true (re-render); still over → drop the section
      VerifyFailure   → tail_cut to floor 200 lines; still over → drop the section
      Documents group → for each document in input_kinds order: head_tail to floor max(25% lines, 40);
                        still over → drop documents from the END of the order, never index 0
      Item            → head_tail to floor max(50% lines, 80); never dropped
      JudgeCandidate  → cap = (target − template_tokens − judge_task tokens) / N; each candidate over
                        cap → stat_only; still over cap → drop that candidate (MOD-4 reads it as
                        "escalate to human selection", :891)
      JudgeTask       → head_tail to floor max(50%, 80); never dropped
      StepSummary     → head_tail to floor 20 lines
    re-estimate the changed section: after[i] = est.estimate(&wrap(&rendered[i])); deficit recomputed
```

**Re-estimation after every rung** is what the ANA asks for (`:874-877`), and it is why every strategy
is written as "change, re-wrap, re-estimate" rather than as arithmetic on the old count: the marker's
own tokens and the wrapper's are then always included.

`head_tail(content, floor, reclaim, est)`: `n` lines; binary-search the largest `k ∈ [floor, n)` such
that `est(head(⌈k/2⌉) + marker + tail(⌊k/2⌋)) ≤ est(content) − reclaim` (monotone in `k`, so ~12
estimates on a 4 000-line document rather than 4 000); `elided_lines = n − k`, `elided_bytes = Σ bytes
of the elided lines including their LF`; returns `None` when even `k = floor` does not reclaim enough
— the caller then records the floor state and moves to the drop. `tail_cut` is the same with the
marker first. A section that reached a floor without clearing the deficit still records
`strategy: HeadTail`, `trimmed: true` and its `elided_*`; only the drop rewrites `strategy: Dropped`,
`tokens_after: 0`.

The one map: `Section` rows are built here — one per section that had data, in render order,
`template` first — and **`SectionEntry` only ever comes from `TrimRecord::section_entries`** (B.5).
Criterion 8's test is `assembled.payload_sections() == assembled.trim.section_entries()` element for
element, plus a grep that no other `SectionEntry {` literal exists under `crates/` outside `trim.rs`
and its tests.

### D.3 Which entries exist (`:566-569`)

| Case | Entry |
|---|---|
| Section rendered ≥ 1 byte and survived | `{ tokens_after > 0, strategy, trimmed }` |
| Section had data and was dropped whole (or a document / candidate was) | `{ tokens_after: 0, strategy: Dropped, trimmed: true }` |
| Placeholder present, no data | **no entry**, empty substitution |
| `item` with an empty body | entry with `tokens_after` = the wrapper's tokens (`:329`) |

### D.4 The preview's spec (T67, D103) — which read answers which field

```
build_spec(backend, item, template_name, scope):
  item      = backend.item(item)?                 ReadStore::item        → item_body, item_title, key; project_id, kind_id
  project   = backend.project(item.project_id)?   ReadStore::project     → item_key = "{slug}:{key}"; project.settings
  kind      = backend.item_kind(item.kind_id)?    inherent (E-6)         → item_kind = kind.name
  templates = backend.prompt_templates(project)?  inherent               → available; the latest version of the chosen name
  settings  = backend.app_settings()?             inherent               → Budget via resolve_budget(None, Some(&project.settings), &settings)
                                                                           hops, max_skill_tokens, excerpt caps
  docs      = backend.documents_of_kinds(item, &[])?   ReadStore (new)   → documents: every kind but `summary`, byte order (note 1)
  upstream  = backend.upstream_summaries(item, hops, &PromptScope::from_scope(scope, project_id))?
  box       = backend.box_info()? → backend.box_profile(box_id)?  inherent → box_profile
  skills    = backend.bound_skills(project_id, None)?  inherent         → project bindings only (note 2)
  repos     = backend.repos(project_id)?, paths = backend.repo_paths(box_id)?
                                                                        → roots: [] (D103), audit.roots = RootRecord{NoPath} per repo
  spec = PromptSpec { role: TemplateRole::of_name(name), template: {name, version: latest}, body,
                      phase: name, output_kind: Some(name) (note 3), attempt: 1,
                      command_queue: false (note 4), verify_failure: None, previous_diff: None,
                      judge: None, handoff: None, excerpts: ExcerptSet { files: [], audit, notes },
                      estimator: TokenEstimator::DEFAULT, notes: [note 1..4] }
```

The four declared stand-ins, verbatim strings in `preview.rs` and asserted by the T67 tests:
(1) `preview: documents are latest-per-kind; ANA-2 input_kinds resolution arrives with MOD-4`;
(2) `preview: project-level skill bindings only; a phase binding needs a phase id (MOD-4)`;
(3) `preview: output_kind defaults to the template name; the phase row's value arrives with MOD-4`;
(4) `preview: command_queue exposure is a phase setting (R-MCP-4); absent until MOD-4`. Plus, from the
excerpt audit, `preview: no run_step_tree and no repo_box_path row for <repo>; excerpts skipped` per
repo. `budget_source` is decided **inside `settings::resolve_budget`** and nowhere else.

The default template when `template_name` is `None`: the first of `DEFAULT_TEMPLATES`'s names present
in `available` (`prd` for every seeded project). The two reserved names are excluded from `available`
because their inputs are MOD-4's (D107).

### D.5 Excerpts in the preview

`roots` is empty, so `select` is called with no reader work to do: `considered: 0`, `selected: 0`,
`files: []`, one `RootRecord { source: NoPath }` per repo. The ranker's tiers are exercised by T66's
tests over `FakeRepoReader`, never by the preview — which is why criterion 12 and criterion 13 are
different tests with different harnesses.

---

## G. Ordering and determinism checklist — §4.7's nine rules

| Rule (`:1410-1439`) | Enforced at | Proved by |
|---|---|---|
| 1 Section order = the parsed span order | `assemble()` D.1 step 7 iterates `parsed.spans`; `Section` rows pushed in first-occurrence order | `prompt_golden::implement_attempt2`; `prompt_digest::a_reordered_body_reorders_sections_and_changes_the_digest` |
| 2 Upstream `(depth, key bytes, item_id)` after `MIN(depth)` | `UpstreamEntry::sort_canonical` in all three backends (C.1–C.3) **and** again in D.1 step 2 | `READ_CASES::upstream_diamond_dedup` (three targets); `cache.rs::the_mirror_passes_the_read_cases`; `prompt_digest::upstream_order_is_canonical_whatever_the_caller_sent` |
| 3 Documents in `input_kinds` order, one winner per kind | `documents_of_kinds` returns `kinds` order (B.12); the preview passes byte order and says so | `READ_CASES::documents_of_kinds_latest_per_kind_in_order`; `prompt_golden::implement_attempt2` |
| 4 Skills `(position, name bytes)`, deduped by `skill_id` | `BoundSkill::collapse`, called by the SQL wrapper, `MemStore` and D.1 step 2 | `model::skill::tests::*`; `prompt_digest::skill_order_is_unchanged_by_store_order` (criterion 15) |
| 5 Excerpts rendered `(repo bytes, path bytes)`, quantised ranks | `select` sorts `(weight desc, repo, path)` with tier-5 scores `(score × 1000) as u32`; `render::excerpts` sorts by `(repo, path)` | `excerpt::{render_order_is_path_order_not_rank_order, tier_five_scores_are_quantised_before_the_sort}` |
| 6 Judge candidates by `fanout_index`, reversed only for the second call | the assembler sorts by `fanout_index` unless `spec.judge.reverse` | `prompt_golden::judge_three_candidates`; `prompt_digest::the_second_judge_call_differs_only_in_candidate_order` |
| 7 No wall-clock value in the digested text | `prompt/` contains no `chrono`/`SystemTime` import | `prompt_digest::the_prompt_module_reads_no_clock` — `include_str!` over the eight source files |
| 8 No absolute path, run id, step id, or (Phase role) `fanout_index` | `Excerpt.path` repo-relative by type; `box_profile` drops `BoxTool.path`; `PromptSpec` has no `RunId`/`StepId` field | `prompt_digest::fan_out_siblings_get_identical_bytes` (criterion 5) — equal digests **and** a grep for the tree root, run id, step id, `fanout_index=`, `(?m)^[A-Za-z]:\\`, ` /(home\|tmp\|scratch)/` |
| 9 Template version pinned, recorded | `PromptSpec.template` is an input; the assembler never resolves `latest` | `prompt_golden::*`; `prompt_preview::the_preview_records_the_latest_version_it_chose` |

Two further rules this blueprint adds: **10** every content string is LF before anything measures it
(P-2; `prompt_digest::crlf_inputs_digest_identically`, criterion 6, plus "the text contains no `\r`");
**11** the elision marker's two numbers equal the record's
(`prompt_digest::every_marker_matches_its_record`).

---

## H. Build order, per-task entry and exit criteria, and the test names

Order: **T69 ∥ T59 ∥ T61** (the three independent fronts) — then **T60 → T62 → T63** and
**T64 → T65 → T66** as two chains that both feed **T67 → T68 → T70**. T64 depends on T59 and T61; T65
on T64; T66 on T61 and T65; T67 on T63, T65, T66; T68 on T63 and T65. Commits one per task:
`feat(core): the {{name}} scanner and the chars-v2 estimator (T59)` ·
`feat(core): the ten default bodies and the fixture corpus (T60, D104)` ·
`feat(core): skill, UpstreamEntry, BoxProfile, PromptScope, PathPrefix (T61, D95, D105)` ·
`feat(core,store,agent): four reads, set_step_prompt, READ_CASES — CASES 22 → 23 (T62, D96)` ·
`feat(store): the amended §7.3 walk on Postgres and the mirror; the inherent prompt reads (T63)` ·
`feat(core): the section wrapper, the renders and the canonical digest (T64)` ·
`feat(core): kept-first trim, settings chain and assemble() (T65, D100, D101)` ·
`feat(core,agent): the excerpt ranker, FsRepoReader and the provider seam (T66, D98)` ·
`feat(tui): the Prompt sub-tab and the deferred preview (T67, D102, D103)` ·
`feat(tui): ~34k and ! on the Runs pane (T68, D106)` ·
`test(agent): the estimator differential (T69, D99, D108)` ·
`docs(mod-2): milestone 9 and MOD-2 close-out (T70)`.

Every task: `cargo fmt --all -- --check` first; `cargo clippy --workspace --all-targets --all-features
-- -D warnings` before the commit. The Windows lint target is unavailable (TOOL-3) and nothing here is
`cfg`-shaped, so nothing is listed as unreviewed.

### T69 — `crates/htui-agent/tests/estimator_live.rs` (measurement done; D108)

**Exit**: the probe committed and repeatable: the four-run shape (baseline, prose 40 000, prose
20 000, code 40 000) over in-tree deterministic corpora, reading **all three** input fields, asserting
the measured ratios within a stated tolerance and asserting the two prose sizes agree within 2% so a
broken baseline subtraction fails loudly. Run line in the module doc:
`cargo test -p htui-agent --features test-support --test estimator_live -- --ignored --nocapture`.

### T59 — `template.rs`, `estimate.rs`, the module skeleton

**Entry**: nothing. **Exit**: `cargo test -p htui-core prompt::` green; `cargo doc -p htui-core
--no-deps` clean. Tests, in-module: `template::tests::{every_placeholder_round_trips_its_token,
the_three_role_sets_are_closed, quadruple_brace_is_a_literal_double_brace,
spaces_inside_braces_are_unknown, a_typo_is_unknown_with_its_offset, uppercase_is_unknown,
a_bare_open_is_unterminated, candidates_in_a_phase_body_is_wrong_role,
a_judge_body_without_candidates_is_missing_required, a_handoff_body_needs_two,
literal_close_braces_pass_through, duplicates_substitute_twice_and_used_lists_once, omits_item_warns}`;
`estimate::tests::{empty_is_zero, chars_not_bytes, fenced_spans_use_the_code_rate,
file_blocks_use_the_code_rate, ceil_per_span_not_per_total, for_model_keys_on_the_id_only,
the_constants_are_the_measured_ones}`.

### T61 — `skill.rs`, `UpstreamEntry`, `BoxProfile`, `PromptScope`, `PathPrefix`

**Entry**: nothing. **Exit**: `cargo test -p htui-core model` green; `cargo build --workspace` green.
Tests: `model::skill::tests::{phase_overrides_project_once, equal_positions_break_on_name_bytes,
pinned_version_wins_over_latest}`; `model::link::tests::{sort_canonical_is_depth_then_key_bytes_then_id,
a_non_ascii_key_sorts_by_bytes}`; `model::box_::tests::{profile_omits_ram_gpu_and_quirks_when_absent,
tools_are_name_sorted_capped_at_24_with_more, a_bare_version_renders_the_name_alone,
path_never_reaches_the_profile}`; `model::scope::tests::{prompt_scope_from_scope_keeps_the_workspace,
project_only_has_none}`; `prompt::excerpt::tests::path_prefix_table`.

### T60 — `defaults.rs` and the fixture corpus

**Entry**: T59. **Exit**: `cargo test -p htui-core --features test-support` and `cargo test -p htui
--features testkit` green — **no `.snap` should change**; a changed one is a finding. Tests:
`defaults::tests::{every_default_body_parses_in_its_role,
every_phase_body_is_wrong_role_for_judge_and_handoff_and_vice_versa,
the_review_body_states_the_front_matter_verbatim, the_judge_body_states_the_verdict_block_verbatim,
command_queue_text_is_two_sentences}`; `fixtures::tests::{ten_templates_per_project_from_the_default_bodies,
template_ids_are_distinct_across_projects, the_demo_prompt_payload_uses_the_section_vocabulary}`.

### T62 — the seam, `MemStore`, `READ_CASES`, the six implementors

**Entry**: T60, T61. **Exit**: `cargo build --workspace`, `cargo test -p htui-core --features
test-support` green with `CASES.len() == 23`, `cargo test -p htui-store --test writer_buffered` green,
`cargo test -p htui-agent --features test-support` green unchanged. Red first: `mem_store.rs`'s `23`;
`READ_CASES` bound to `MemStore::demo()`. Tests: the 23rd case and the six read cases (B.12);
`mem.rs::tests::{set_step_prompt_writes_both_columns, a_preview_style_bound_skills_read_collapses_overrides,
upstream_walk_is_directed_and_kind_filtered}`; `writer_buffered.rs::set_step_prompt_is_refused_offline`;
`run.rs::tests::prompt_summary_reads_estimated_after_and_any_trimmed`.

### T63 — `PgStore`, `CacheStore`, `Backend`, `.sqlx`

**Entry**: T62. **Exit**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui-store
--all-features --no-fail-fast` green — `pg_conformance` at 23, `pg_store_read_conformance`,
`the_mirror_passes_the_read_cases`, `mirror_reads_equal_the_reference_store` extended,
`pg_criteria::set_step_prompt_writes_both_columns_and_only_them`,
`pg_criteria::inherent_prompt_reads_answer_the_fixture`; `cargo sqlx prepare --check` clean.

### T64 — `render.rs`, `digest.rs`, `prompt/fixtures.rs`, `prompt_golden.rs`

**Entry**: T59, T61. **Exit**: `cargo insta test -p htui-core --test prompt_golden --features
test-support` accepted with every hunk read. Tests: `prompt_implement_attempt2`, `prompt_all_empty`
(criterion 4's three greps before the snapshot), `prompt_judge_three_candidates`,
`prompt_handoff_basic`, `prompt_prd_default_body`. In-module: `render::tests::{fence_is_one_longer_than_the_longest_run_min_three,
nested_fences_do_not_close_early, attr_escapes_quote_and_amp_and_collapses_newlines,
title_truncates_at_120_bytes_on_a_char_boundary, one_line_title_collapses_runs_and_cuts_at_100,
upstream_summary_blocks_precede_one_liners, the_pending_suffix_is_only_on_pending_rows,
hop_pluralises_at_two, excerpt_line_numbers_are_right_aligned_to_the_last_line,
step_summary_from_events_counts_and_tails, wrap_of_empty_content_has_no_blank_line,
a_section_tag_in_content_is_inert}`; `digest::tests::{canonical_collapses_three_lfs_to_two,
canonical_normalises_crlf_before_collapsing, canonical_strips_a_bom_and_ends_with_one_lf,
sha256_matches_the_recorder_call}`.

### T65 — `trim.rs`, `settings.rs`, `assemble()`, `prompt_digest.rs`

**Entry**: T64. **Exit**: `cargo test -p htui-core --features test-support --test prompt_digest` green;
`migrations.rs::the_ana5_defaults_match_the_compiled_in_table` green under Postgres. Tests:
`same_spec_same_bytes_twice`, `crlf_inputs_digest_identically`, `fan_out_siblings_get_identical_bytes`,
`the_oversize_fixture_lands_at_or_under_target`, `every_marker_matches_its_record`,
`payload_sections_are_the_records_projection`, `a_protected_set_over_target_refuses_before_anything`,
`skills_over_the_cap_refuse`, `an_unknown_placeholder_refuses_with_the_stage_3_text`,
`scrub_residue_names_the_section_not_the_text`, `upstream_order_is_canonical_whatever_the_caller_sent`,
`skill_order_is_unchanged_by_store_order`, `the_second_judge_call_differs_only_in_candidate_order`,
`a_candidate_over_its_share_renders_stat_only`, `a_handoff_summary_carries_only_the_windowed_tail`,
`the_prompt_module_reads_no_clock`, `reserve_target_is_integer_arithmetic`. In-module
`settings::tests::{chain_phase_project_app_default, a_non_positive_rung_falls_through, hops_clamp_notes,
reserve_is_basis_points}`. Plus `recorder.rs::record_prompt_over_an_assembled_prompt_recomputes_the_same_digest`.

### T66 — `excerpt.rs` (core) and `htui-agent/src/excerpt.rs`

**Entry**: T61, T65. **Exit**: `cargo test -p htui-core prompt::excerpt` and `cargo test -p htui-agent
--features test-support --test excerpt` green. Tests: `tier_weights_are_max_not_sum`,
`ties_break_on_path_bytes`, `tier_five_scores_are_quantised_before_the_sort`,
`tier_five_never_exceeds_a_third_of_max_files`, `render_order_is_path_order_not_rank_order`,
`a_file_over_the_line_cap_is_head_windowed_with_the_marker`,
`a_denied_file_under_a_touched_prefix_is_never_selected_and_is_noted`,
`every_provider_failure_leaves_a_valid_prompt`, `no_roots_means_no_section_and_a_no_path_root_per_repo`,
`fs_reader_lists_in_byte_order_at_every_level`,
`fs_reader_skips_git_gitignored_binary_large_and_lockfiles_in_order`, `scan_cap_sets_truncated`,
`sha256_is_over_the_rendered_block`.

### T67 — the preview

**Entry**: T63, T65, T66. **Exit**: `cargo test -p htui --features testkit --no-fail-fast` green with
the strip snapshots re-accepted and the new ones created. Tests:
`the_prompt_sub_tab_previews_feat_1`, `the_preview_declares_its_four_stand_ins`,
`n_cycles_to_the_next_template_and_p_back`, `the_preview_writes_nothing`, `a_stale_preview_is_dropped`,
`an_item_with_no_templates_says_so`, `the_preview_records_the_latest_version_it_chose`,
`the_preview_refuses_offline_with_one_sentence`, plus `backlog.rs::the_six_sub_tabs_render_the_selected_item`
and `every_sub_tab_says_so_when_it_has_nothing`.

### T68 — the Runs indicators

**Entry**: T63, T65. **Exit**: `cargo test -p htui --features testkit --test backlog` and `--test replay`
green with each snapshot gaining exactly one line. Tests:
`runs.rs::tests::{a_step_with_a_trim_record_takes_two_lines,
the_figure_is_thousands_with_a_bang_when_trimmed, a_step_without_one_stays_one_line}`;
`fixtures::tests::step_impl_carries_the_golden_trim_record`.

### T70 — sweep and close-out

**Entry**: everything else committed. **Exit**: `validate-workflow-docs.sh` → 0 errors. The criteria
table below goes into `docs/decisions/mod/mod-2.md`; the `HANDOFF.md` MOD-2 checklist line (`:90-104`
plus its nine phase notes) is **deleted**; the write-up opens
`# MOD-2 - Agent driver + chat tab (done, 2026-09-11)` in the `mod-21.md:1-9` shape, all nine phases,
every decision id D1–D108 findable; `DECISIONS.md:6` gains the index line above MOD-21; the summary
table (`HANDOFF.md:855-860`) drops the MOD count by one; the status line (`:17-28`) gains milestone 9
and MOD-2's close and drops MOD-20; the PRD's row 9 → `complete` and its five open questions answered
with D95–D99. Criterion 18's persistence half is written as MOD-4's by name in the close-out and in
the `HANDOFF.md` MOD-4 line.

### The criteria map (the T70 table, pre-filled)

> **Superseded at close-out — cite `docs/decisions/mod/mod-2.md` instead.** This table was written
> before the tests existed and five of its names do not resolve in the shipped tree: `prompt_all_empty`
> and `prompt_judge_three_candidates` are `insta` **snapshot** names, not test functions (the
> functions are `the_all_empty_prompt_omits_every_optional_section` and
> `the_judge_prompt_renders_its_golden_bytes`); `upstream_diamond_dedup` is a `READ_CASES` **case**
> name, executed by three binding tests; `set_step_prompt_writes_both_columns_and_only_them` shipped
> split in two (`store::mem::tests::set_step_prompt_writes_both_columns` and
> `pg_criteria.rs::set_step_prompt_writes_only_the_digest_and_the_record`); and
> `record_prompt_over_an_assembled_prompt_recomputes_the_same_digest` was never written under that
> name — the two-writer identity is `prompt::digest::tests::sha256_matches_the_recorder_call`. Every
> name in the close-out table was resolved against
> `cargo test --workspace --all-features -- --list` on 2026-09-15.

| §12 | Test |
|---|---|
| 1 | `defaults::tests::every_default_body_parses_in_its_role`, `…wrong_role_for_judge_and_handoff_and_vice_versa` |
| 2 | `template::tests::{spaces_inside_braces_are_unknown, a_typo_is_unknown_with_its_offset, uppercase_is_unknown, a_bare_open_is_unterminated, quadruple_brace_is_a_literal_double_brace}` |
| 3 | `prompt_digest::an_unknown_placeholder_refuses_with_the_stage_3_text` (the `blocked` transition is MOD-4's) |
| 4 | `prompt_golden::prompt_all_empty` |
| 5 | `prompt_digest::fan_out_siblings_get_identical_bytes` |
| 6 | `prompt_digest::crlf_inputs_digest_identically` |
| 7 | `READ_CASES::upstream_diamond_dedup` over three targets; `cache.rs::the_mirror_passes_the_read_cases` |
| 8 | `prompt_digest::payload_sections_are_the_records_projection` |
| 9 | `prompt_digest::{the_oversize_fixture_lands_at_or_under_target, every_marker_matches_its_record}` |
| 10 | `prompt_digest::{a_protected_set_over_target_refuses_before_anything, skills_over_the_cap_refuse}` |
| 11 | `pg_criteria::set_step_prompt_writes_both_columns_and_only_them` + `recorder.rs::record_prompt_over_an_assembled_prompt_recomputes_the_same_digest` |
| 12 | `excerpt::no_roots_means_no_section_and_a_no_path_root_per_repo`; `prompt_preview::the_prompt_sub_tab_previews_feat_1` |
| 13 | `excerpt::every_provider_failure_leaves_a_valid_prompt` |
| 14 | `excerpt::a_denied_file_under_a_touched_prefix_is_never_selected_and_is_noted` |
| 15 | `prompt_digest::skill_order_is_unchanged_by_store_order`; `model::skill::tests::*` |
| 16 | `defaults::tests::the_review_body_states_the_front_matter_verbatim` — the parser half is MOD-4's |
| 17 | `prompt_golden::prompt_judge_three_candidates`; `prompt_digest::{the_second_judge_call_differs_only_in_candidate_order, a_candidate_over_its_share_renders_stat_only}` |
| 18 | `prompt_digest::a_handoff_summary_carries_only_the_windowed_tail` — the `follow_up` persistence half **re-deferred to MOD-4 (D107)** |
| 19 | `pg_conformance` (23), `pg_store_read_conformance`, `mem_store_read_conformance`, `the_mirror_passes_the_read_cases` |
| 20 | the plan's validation block; `cargo tree` diff empty at the workspace level |
| 21 | D95, D96, D97, D98, D99+D108 |

---

## I. Hazards

| # | Failure mode | Detected by | Closed by |
|---|---|---|---|
| H-1 | **An absolute path leaks into digested text** through `BoxTool.path`, `RepoRoot.root`, or a provider's candidate | G rule 8's grep | `BoxProfile::project` drops `path`; `select()` rejects any candidate whose `path` starts with `/`, `\`, or `[A-Za-z]:` and notes it; `RepoRoot.root` is never rendered |
| H-2 | **A run id, step id or `fanout_index` reaches a Phase prompt** | the same grep | `PromptSpec` has no id field; `JudgeCandidate.fanout_index` renders only under `Judge` |
| H-3 | **Map iteration reaches rendered bytes** | `same_spec_same_bytes_twice` built from **two** `MemStore::demo()` instances | `BTreeMap` only inside `prompt/`; every store read ends in an explicit sort |
| H-4 | **The fence rule fails on nested fences** | `render::tests::nested_fences_do_not_close_early` | `fence_for` scans the whole content for the longest run |
| H-5 | **Estimator counts bytes** — 4× off on CJK | `estimate::tests::chars_not_bytes` | written over `chars()` |
| H-6 | **`MemStore` and SQL disagree on dedup or scope** | three-target `READ_CASES`; `the_mirror_passes_the_read_cases` | C.1–C.3 from one rule; `upstream_walk_is_directed_and_kind_filtered` |
| H-7 | **Two writers of `prompt_digest` disagree** | `record_prompt_over_an_assembled_prompt_recomputes_the_same_digest` | masking is idempotent and one scrubber is used for both |
| H-8 | **A held or dropped section emits the wrong entry** | `the_oversize_fixture_lands_at_or_under_target`; `prompt_all_empty` | D.3's table; `Section` rows created only in `trim::run` |
| H-9 | **Float arithmetic in the target** | `reserve_is_basis_points`, `reserve_target_is_integer_arithmetic` | `reserve_bp: u32`, integer `target()` |
| H-10 | **CRLF changes `elided_bytes`** and therefore the digest | `crlf_inputs_digest_identically` | P-2: LF-normalise before any byte is counted |
| H-11 | **The `insta` corpus hides a real change** | structural asserts before every snapshot; `prompt_digest.rs` has no snapshots | two suites; the phase note lists every re-accepted `.snap` |
| H-12 | **Fixture template ids collide at ten names** | `template_ids_are_distinct_across_projects`; `load_demo` fails | multiplier → 10 |
| H-13 | **`query_as!` positional binding shift** | compile | appended at the end of struct and `SELECT` |
| H-14 | **New fixture links break a pinned set** | `links_hops_1_vs_2`, the ready list, the tombstone case, `backlog__detail_graph.snap` | C.4 touches none of them |
| H-15 | **Skill rows load into `MemStore` only** | `pg_criteria::inherent_prompt_reads_answer_the_fixture` | A.30 |
| H-16 | **An offline preview fails on the first read** and `go_offline` is never triggered | `the_preview_refuses_offline_with_one_sentence` | E-2's check before spawning |
| H-17 | **A `Backend` clone outlives a swap** | the task maps every `StoreError` to `Failed` and exits | documented in `run_preview`; one-shot task |
| H-18 | **`is_fresh` drops the picker's reply** | `n_cycles_to_the_next_template_and_p_back` | the picker's request is newest by construction |
| H-19 | **Key order in `trim_record`** | `trim::tests::to_value_keys_are_sorted` | keys are sorted; `JSONB` reorders anyway |
| H-20 | **The provider deadline leaks a thread** | documented | `run_providers` names it; a provider is a lookup by contract |
| H-21 | **`documents_of_kinds(item, &[])` returns `summary`** | `the_prompt_sub_tab_previews_feat_1`; a unit test over `HTUI_ANA_1` | `build_spec` filters `summary` and says so |
| H-22 | **Skip rules only in `FsRepoReader`** would pass on the filesystem and fail on the fake | the denylist test runs over `FakeRepoReader` | the ranker applies the path rules itself |
| H-23 | **A `<file>` block estimated as prose** | `file_blocks_use_the_code_rate` | the `<file` toggle in B.3 |
| H-24 | **`SectionName::Box` shadows `Box<T>`** | compile | no glob import; the variant doc forbids it |
| H-25 | **`READ_CASES` names an inherent method** | compile | `project_row_has_settings` reads `ReadStore::project` |
| H-26 | **The second Runs line clips at 43 columns** | `the_figure_is_thousands_with_a_bang_when_trimmed` renders at pane width | seven characters max in a 12-wide cell |
| H-27 | **`app_settings` returns a `HashMap`** | type | returns `BTreeMap` |
| H-28 | **A template body containing `</section>`** read as structure | `a_section_tag_in_content_is_inert` | nothing parses the text |
| H-29 | **`sqlx` marks every CTE column nullable** | compile / `cargo sqlx prepare` | C.1's `!`/`?` overrides |
| H-30 | **`0002` and `DEFAULTS` drift** | `the_ana5_defaults_match_the_compiled_in_table` | E-11 iv |

---

## J. Open questions for the maintainer

None of these blocks T59, T61 or T69; **E-2 and E-4 block T67 and should be answered before T62
starts** (E-4 changes the fixture T62 writes).

1. **E-2**: refuse the preview offline (assumed), or preview from `DEFAULT_TEMPLATES` with declared notes?
2. **E-4**: add `Repo` fixture rows and the `repos` inherent read (assumed), or accept that criterion
   12's `no_path` is proved by the unit test only and the binary shows `roots: []`?
3. **P-3**: the documents strategy follows §5.1's example rather than §4.4's row text. If the row text
   was the intent, the oversize fixture's expected trimmed set changes and the ANA amendment should say so.
4. **P-5 / D108**: two estimator ids (`chars-v2` measured, `chars-v1-gpt` inherited) rather than one.
5. **B.17**: the Runs pane's second line rather than a fifth column or a replaced `started` cell.
6. **D.4**: the preview's default template is the first `DEFAULT_TEMPLATES` name the project has (`prd`).
7. **B.9**: `COMMAND_QUEUE_TEXT`'s two sentences are this blueprint's wording; the ANA fixes the
   section and not the text. It is a digest input, so it should be settled before T64's snapshots are accepted.

---

## K. What this milestone does NOT touch

- **MOD-4**: `ResolvedPhase`, `run_step_tree`, `verify_outcome`/`verify_exit_code`, the previous
  attempt's diff, `input_kinds` resolution, the `judge`/`handoff` callers and the `follow_up`
  persistence of criterion 18, the `blocked` transition of criterion 3, the `review` front-matter
  parser of criterion 16, `Backend`'s `Offline` arms for the seven inherent reads, and
  `prompt_template(project, name, version)` by pinned version.
- **MOD-9**: `upsert_skill`, `add_skill_version`, `set_skill_binding`, the editor that calls `parse`;
  the `Skills` tab stays a stub.
- **MOD-15**: seeding the ten `prompt_template` rows into new projects, the reserved-name refusal in
  the phase editor, the Settings-tab exposure of the ten keys.
- **MOD-7 / MOD-13**: `repo_box_path` writers, `touched_paths` editing.
- **MOD-10**: the scrubber implementation; D100 wraps the existing trait.
- **MOD-11**: the `box_profile` read tool; `BoxProfile::project` is the projection it must return.
- **ANA-3**: any real `ExcerptProvider`; `builtin@1` is the only registered one.
- **`Recorder`**: no signature, no arm (B.14). **`record.rs`, `event_loop.rs`, migrations,
  `cache_migrations/`**: nothing.
- **`Scope`**: not one field (D95). **`set_step_usage`**: not one parameter (D97). **`CASES`' 22
  existing names**: unchanged.
- **The chat tab, the ACP and CLI transports, `conformance::CASES` of `htui-agent`** (15 names): the
  two test doubles gain a delegating method and nothing else.
