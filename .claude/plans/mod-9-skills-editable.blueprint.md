# Blueprint: MOD-9 milestone 3, "skills are editable and attachable"

**Status**: PROPOSED 2026-09-26 by the code-architect, from the plan confirmed the same day.
Findings F-A to F-T (§0.2) and decisions D87–D107 (§12) are proposed here. **Blocker** means the
plan read literally does not compile, fails its own named test or the gate; **Major** means a named
test, pin or file list is wrong, or a design consequence the maintainer has not seen; **Minor** is a
citation, wording or placement. A finding marked **(maintainer)** needs a decision through the main
thread before the task it names starts; every other Fix is what the implementer builds.

**Plan**: `.claude/plans/mod-9-skills-editable.plan.md` (CONFIRMED, OQ-14/15 answered, OQ-16..20 at
their defaults). Its D70–D86 and OQ answers are binding; where §0.2 amends a file list or a test
placement, this file wins. **PRD**: `.claude/prds/mod-9-skill-library-templates.prd.md` row 3 (as
reworded by D70). **ANA**: `docs/ANA-22.md` §6 items 1-7, 12; §8.

**Verified at**: HEAD `fb8b0c8` (branch `claude/sharp-gauss-lsj18l`). `git diff --stat 98e6d2f HEAD --
crates Cargo.toml Cargo.lock` is empty, so the plan's citations (read at `98e6d2f`) still hold.
**Line numbers are pre-edit.** `crates/htui-store/.sqlx/` holds **264** files. `df -h /`: 16 GB
free. Probes on this box's Postgres 16 (scratch database `mod9m3_probe`, migrated `0001`..`0007`
with `psql -f`, dropped afterwards): every new statement's parameter types, the attach race on the
`NULLS NOT DISTINCT` key (§3.7), the `add_skill_version` compare-and-set, the constraint names.
`globset` 0.4.20 was compiled offline from `~/.cargo/registry` in the scratchpad and its matching
table probed (§0.3).

**Graphify / Gortex**: `graphify-out/` does not exist and Gortex is not reachable. Every fact was
read with `grep`/`sed`/`Read`.

**Scope at a glance**:
- **Order**: T1 → T2 → (T3 ∥ T4) → T5, T5 ∥ T3 allowed. Parallel tasks in their own worktrees, one
  shared `CARGO_TARGET_DIR`, `--test-threads=2` at most.
- **No migration**. `TABLES` 39, applied `1..=7`, commented columns 34, next `0008` — unchanged.
- **`WriteStore` +7 methods** (3 readers, 4 writers), five implementors. **`CASES` 74 → 80**,
  `READ_CASES` 14. **`StoreRequest` 68 → 73, `StoreReply` 39 → 41** (unpinned; counted at
  `store_worker.rs:95-579` and `:668-844`).
- **`.sqlx`**: 264 → **276** (T2: +12; T3: one replaced), §3.6.
- **Dependencies**: `globset` 0.4.20 (T1), the lock gains exactly one package. Every later task
  keeps `git diff --exit-code Cargo.lock`.

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs` on lib roots;
`missing_debug_implementations` and `unused_qualifications` warn; clippy `-D warnings`; rustdoc denies
broken/private intra-doc links; `rustfmt.toml` `max_width = 100`. Every new `pub` item has a doc
comment and `Debug`. Red tests first (bodies `todo!()`), then green; every commit compiles; no test
loosened; a moved pin names its reason in the assertion message; implementers stage their own paths.

---

## 0. Environment, findings, probes

### 0.1 Environment and gates (this box)

```bash
. /root/ort/env.sh      # sandbox only (R-19): ORT_LIB_LOCATION=/root/ort/lib + LD_LIBRARY_PATH
export PG=postgres://postgres:htui@localhost:5439
export PGPASSWORD=htui
export CARGO_TARGET_DIR=/home/user/htui/target          # shared by the T3/T4/T5 worktrees
# F-A: the cluster was DOWN at blueprint time (stale pid file). Start it if needed:
pg_isready -h localhost -p 5439 || pg_ctlcluster 16 main start
# gate, per crate (the plan's line):
USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=$PG/postgres \
  cargo test -p <crate> --all-features -- --test-threads=2
```

`SQLX_OFFLINE = "true"` is set in `.cargo/config.toml:5-6`, so build/clippy/doc never need a server;
`cargo sqlx prepare` overrides it in its child (the file's own comment, `:1-4`).

**sqlx-cli** is **not installed** (`cargo sqlx --version`: "no such command"). The workspace pins
`sqlx 0.9.0` (`Cargo.lock:5981-5982`); `cargo info sqlx-cli@0.9.0` resolves (rust-version 1.94 ≤
1.98). Install once:

```bash
cargo install sqlx-cli --version =0.9.0 --locked --no-default-features --features postgres,rustls
```

**Regenerating `.sqlx`** (T2 and T3; README `:492-511`), against a database migrated **fresh** to
`0007` — drop and recreate before every prepare (R-36):

```bash
psql "$PG/postgres" -c 'DROP DATABASE IF EXISTS htui_prepare_mod9m3' -c 'CREATE DATABASE htui_prepare_mod9m3'
DATABASE_URL=$PG/htui_prepare_mod9m3 cargo sqlx migrate run --source crates/htui-store/migrations
cd crates/htui-store
DATABASE_URL=$PG/htui_prepare_mod9m3 cargo sqlx prepare -- --all-targets --all-features
DATABASE_URL=$PG/htui_prepare_mod9m3 cargo sqlx prepare --check -- --all-targets --all-features
ls .sqlx | wc -l            # T2: 276; after T3: 276
```

`--all-targets --all-features` is mandatory (README `:507-509`): without it the `#[cfg(test)]`,
`tests/*.rs` and `demo` queries are garbage-collected.

**Per-crate gates**:

| Crate | Command (after the env block) |
|---|---|
| htui-core | `cargo test -p htui-core --all-features -- --test-threads=2` |
| htui-store | `USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=$PG/postgres cargo test -p htui-store --all-features -- --test-threads=2` |
| htui-agent | `cargo test -p htui-agent --all-features -- --test-threads=2` |
| htui-orch | `cargo test -p htui-orch --all-features -- --test-threads=2` |
| htui | `USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=$PG/postgres cargo test -p htui --all-features -- --test-threads=2` |
| all | `cargo build --workspace --all-features --all-targets`; `cargo clippy --workspace --all-features --all-targets -- -D warnings`; `cargo fmt --all -- --check` |

### 0.2 Findings the plan fact-check missed

| # | Severity | Plan says | Tree at `fb8b0c8` | Fix |
|---|---|---|---|---|
| **F-A** | **Blocker** (environment) | "Postgres at …:5439 (running)"; `cargo sqlx prepare` in T2's Validate. | `pg_isready -p 5439` → no response; `pg_lsclusters` → `16 main 5439 down` (stale `/var/run/postgresql/16-main.pid`). `cargo sqlx` → "no such command". | §0.1: `pg_ctlcluster 16 main start` (done once here; it came up) and the pinned `cargo install sqlx-cli --version =0.9.0`. Sandbox only, no repo change. |
| **F-B** | **Blocker** (T2 does not compile) | D75 names `NewSkill`, `SkillPatch`, `NewSkillVersion`, `SkillBindingKey`, `BindingChange`, `Attachment`; no task lists where they live. | T2's file list has no `model/` file; `WriteStore` signatures (`traits.rs`) and T4/T5 both need the types. Precedent: `NewPromptTemplate` is in `model/kind.rs`, `BoxEdit` in `model/box_.rs`. | **D87**: T1 defines all six in `model/skill.rs` and re-exports them from `model/mod.rs:144-147` (both already in T1's list). |
| **F-C** | **Major** (file list) | T2 files: `store/traits.rs`, … | Every refusal helper is re-exported from `store/mod.rs:15-26` (`invalid_template_name`, `prompt_template_refusal`, …) and `pg/write.rs:43-54` imports them from `htui_core::store`. T2's new helpers (D88) need the same line. | T2 gains **`crates/htui-core/src/store/mod.rs`** (re-export line only). T3 does not touch it: intersections unchanged in kind. |
| **F-D** | **Major** (a green T2 goes red on a doc comment) | — | `conformance.rs:10134-10239` `every_cross_referenced_test_name_exists` scans **every inline-code span** of `conformance.rs`: a bare snake_case span with ≥ 4 underscores must name a `fn` in `conformance.rs` or `mem.rs`; `<file>.rs::<name>` must be one of `conformance`, `mem`, `pg_criteria`, `box_identity`, else it panics. The constraint names `` `skill_binding_glob_needs_globs` `` and `` `skill_binding_phase_needs_project` `` (4 underscores each) and a `` `skill_writers.rs::…` `` reference would fail it. | Rule for T2's doc comments in `conformance.rs`: name constraints in prose ("the glob-needs-globs check"), never in backticks; refer to the Postgres race tests as "the two-session tests in `htui-store`'s `tests/`" without a `.rs::` span (the milestone 1 template section's wording, `:5233-5235`). R-31. |
| **F-E** | **Major** (D83's form cannot type D74's own table) | D83: "languages (comma list), globs (comma list)". D74's table has `src/*.{rs,toml}`. | A plain `split(',')` cuts `src/*.{rs,toml}` into `src/*.{rs` and `toml}`, both refused (`UnclosedAlternates` / `UnopenedAlternates`, §0.3). | **D93**: `skill_glob::split_list(text)` splits on `,` only at brace depth 0 and outside `[...]`, honouring `\` escapes; trims; drops empties. T1 tests it; T5's form uses it for both fields; the form displays stored lists joined with `", "`. |
| **F-F** | **Major** (D74's signature cannot express D73's refusal; D78's order) | D74: `canonical_globs(typed, languages) -> Result<Vec<String>, GlobError>`, "`GlobError` names the glob and `globset`'s message"; D73: `expand -> Result<_, UnknownLanguage>`; D78 orders "languages known" **before** "every glob parses". | `canonical_globs` takes `languages`, so it can fail on a language, which a glob-only `GlobError` cannot carry. `globset::Error`'s `Display` already embeds the glob (`error parsing glob '[a': …`, `globset-0.4.20/src/lib.rs:243-251`), so wrapping it would name the glob twice. | **D94**: `GlobError` is an enum `{ Invalid { glob, message }, UnknownLanguage(UnknownLanguage) }`; `message` is `err.kind().to_string()`; `canonical_globs` expands languages **first**, then parses typed globs (D78's order), then appends. |
| **F-G** | **Major** (named test cannot land where the plan puts it) | T3: "conformance: `create_step_graph` round-trips `is_override`"; T3's `store/conformance.rs` scope is "constructor lines only"; D85: `CASES` 80 after T2's six. | A seventh new case makes `CASES` 81 against D85's 80, and a round-trip assertion is not a constructor line. | **D95**: no new case. T3 extends `step_graph_and_phase_round_trip` (`conformance.rs:3122`): its two constructors gain `is_override: false`, and after its last assertion it creates `release-override` with `is_override: true` and reads it back through `step_graphs`. T3's conformance.rs scope is those lines. |
| **F-H** | **Major — decided 2026-09-26: check first, then refuse (D96)** | D80: the copy calls `set_skill_binding(key with the new phase, None, Attach(copy))`; D79: the writer refuses a qualifier naming no current repo. | A source phase row whose qualified glob names a repo renamed or deleted since it was saved (ANA-22 §9's stale case, which D83 marks `?`) makes the copy **refused**. `override_graph` has by then created the graph and every phase (`graph.rs:396-414`), so it returns `ResolveError::Store(Constraint)` and leaves an orphan graph — the same failure shape as today's partial clone (`graph.rs:416-417` comment), but now triggered by a stale attachment rather than a store fault. | **Proposed default (D96)**: propagate the refusal unchanged; the user fixes the `?` row in the pane and re-clones (a second clone is refused by the `(project_id, name)` key anyway, `graph.rs:377-380`). Alternatives for the maintainer: (b) the copy bypasses D79 for globs already stored on the source row; (c) the copy drops stale qualified globs and notes it. `override_graph` has no production caller (`lib.rs:56` re-export, `graph.rs:1460` test), so (a) costs nothing today. T3 is unaffected unless (b)/(c) is chosen. |
| **F-I** | Minor | D82: "`T = TokenEstimator::DEFAULT.estimate(render::skills(&[as_bound]).content)` … the bytes the assembler would count". | The assembler estimates `render::wrap(&rendered)` (the `<section name="skills">` frame included) times the placeholder weight (`trim.rs:365-368`), once for all skills. `.content` of a one-skill render is that skill's own `<skill …>` block: its marginal cost minus the shared frame (~10 tokens). | D82 kept as written (**D99**); the doc on the helper says "this skill's block, as the assembler renders it, without the section frame it shares with the others". Demo values: `rust-style` v2 **42**, v1 **31**, `tests` v1 **32** (hand-computed: 103, 77, 79 chars ÷ 2.5, one prose span). |
| **F-J** | Minor | T2 lists `pg/rows.rs`. | `SkillBinding`, `Skill` and `SkillVersion` carry exactly the columns; `query_as!` builds them directly with the `AS "col: Type"` overrides (`bound_skills` already does so for `SkillVersion`, `pg/read.rs:1478-1491`). `SkillBindingRow` (`rows.rs:257-301`) exists only for the joined `name`. | `pg/rows.rs` needs **no edit**; it stays in T2's list harmlessly. |
| **F-K** | Minor | T4 test: "the `htui` project's graphs, phases and repo names"; D81 "override graphs excluded". | The demo holds **no repo** (`mem.rs:100-101`, `pg/demo.rs:27-29`), so repo names are `[]`. An override graph cannot be minted through `NewStepGraph { is_override }` while T3 (which adds the field) runs in parallel. | T4's test creates repo `core` with `create_repo`, and builds its store with `MemStore::from_demo` over `fixtures::demo_data()` with one pushed `StepGraph { is_override: true, .. }` literal (`DemoData.graphs` is `pub`, `fixtures.rs:356`; `StepGraph.is_override` exists, `kind.rs:141-142`). |
| **F-L** | Minor (doc drift) | D80 names three `engine.rs` deletions; T1 "name validation, doc". | Further stale prose: `model/skill.rs:4`, `:20-21`, `:115`, `:178`, `:254`, `:263`, `:310` place the matcher in "milestone 3" (now row 5, D86); `engine.rs:4991-4993` ("every phase of an override graph, get a note"); `graph.rs:6-7` and `:365-380` (the no-copy rationale); `kind.rs:149-150` (`NewStepGraph` doc) and `:165-166` (`StepGraphPatch`: "`is_override` is not here … nothing for this milestone to write"). | T1 rewords the `skill.rs` lines to "PRD milestone 5 (D86)". T3 rewords the other four places (§4.4). |
| **F-M** | Minor | D83: `ctrl-r` "inserts `<repo>:` at the globs cursor". | `TextField` has no insert-text API (`text_field.rs:113-240`: `on_key`, `with_text`, `take`, `clear`); `insert` is private. `text_field.rs` is in no task's list. | **D101**: the picker feeds each char of `<repo>:` to the globs field as a `KeyEvent` (`KeyCode::Char(c)`, no modifiers), which inserts at the cursor. No `text_field.rs` change. |
| **F-N** | Minor | D84: `SkillsTab` routes keys and replies. | `Tab::on_external_edit` (`registry.rs:63`) delivers one outcome per tab; both views can now hand off to `$EDITOR`. `ExternalEditOutcome` is `Clone` (`editor.rs:125`). | **D100**: `SkillsTab::on_external_edit` passes a clone to both views; each ignores it without its own pending handoff (`templates.rs:376-379` shape). |
| **F-O** | Minor | T5 tests `skills__attachments`, `the_winning_row_is_starred_per_project`. | The Harness enters the **Graphics** workspace (`tests/templates.rs:3-5`): one project, `vulkan-tutorials`, with **no** demo attachment (all three are on `htui`, `fixtures.rs:574-600`). | The attachment tests switch to Platform first: `w`, settle, `j`, `Enter`, settle (`tests/integration.rs:120-127`), then `2`. |
| **F-P** | Minor | D76: "a taken name is `Constraint(already_exists("skill", name))`". | On Postgres the 23505 arrives as `skill_name_key: duplicate key value …` through `map_sqlx` (`error.rs:36-45`); nothing maps a constraint name to a sentence today (`grep constraint()` → `error.rs:38` only). | **D92**: `pg/write.rs` gains a private `skill_name_taken(err, name)` that turns `db.constraint() == Some("skill_name_key")` into `already_exists("skill", name)` for `create_skill` and `update_skill`; every other error goes through `map_sqlx`. |
| **F-Q** | Minor (brief's question) | "timestamp stamping consistent with existing MOD-15 writers (find how they make `updated_at` monotonic)". | They do not: each arm takes `let now = Utc::now();` before the lock (`mem.rs:5320-5334`) and the `State` fn writes `row.updated_at = now` (`mem.rs:1945`). No monotonic helper exists; on Postgres the `BEFORE UPDATE` trigger writes `clock_timestamp()` (`0001_init.sql:564-580`). | The new `State` fns take `now` the same way (§3.3). R-34 records the (pre-existing) same-instant hazard. |
| **F-R** | Minor | D85: ".sqlx: the new statements, and one replaced". | 12 new statements (§3.6) and `create_step_graph`'s replaced. | 264 → **276**; recorded in the close-out as the plan asks. |
| **F-S** | Minor (record) | D72: `empty_alternates(false)`. | Probed (§0.3): `{a,}` is **accepted** under `empty_alternates(false)` and matches `a` (the empty branch is dropped, not refused); `a**b` is `a*b`; nested `{a,{b,c}}` is supported. | No change; T1's table pins the probed behaviour so a `globset` bump that changes it fails a test. |
| **F-T** | Minor | D71: `validate_name`. | The name reads like a `Result`; the sentence lives in `traits.rs` (D71). | `pub fn validate_name(name: &str) -> bool` with the doc "`true` when `name` may be stored"; the refusal is `invalid_skill_name(name)` (D88). |

**Plan's "Verified claims", re-checked spot-wise**: all hold at `fb8b0c8` — `traits.rs:1676` `CasOutcome`,
`:715-723` precedence, `:597` `repos`, `:1419` `graph_not_in_project`, `:1442`
`invalid_template_name`; `mem.rs:2492` `create_step_graph` (`is_override: false` at `:2520`), `:2696`
`append_prompt_template`, `:5216` impl; `pg/write.rs:78-89`, `:587`, `:1898`, `:2180-2203`,
`:2419-2479`; `pg/read.rs:1432`; `rows.rs:257`; five implementors; `CASES` 74 (`mem_store.rs:36`,
`pg_conformance.rs:19`), `READ_CASES` 14; nine `NewStepGraph {` constructors in seven files
(`htui-orch/src/conformance.rs:805,865`, `engine.rs:6021`, `graph.rs:397`,
`tests/gix_isolator.rs:338`, `tests/fixtures.rs:54`, `htui/src/catalogue.rs:190`,
`htui-core/src/store/conformance.rs:3125,3135`); `engine.rs:87`, `:5018`, `:12063`; `SKILLS_LATER`
at `skills/mod.rs:26`, `:127`, pinned by no test and shown by no snapshot; `globset` deps at
`Cargo.lock:228,672,3724,5183,5200`; `StoreRequest` 68, `StoreReply` 39.

### 0.3 Probes

**`globset` 0.4.20**, `GlobBuilder::new(g).literal_separator(true).backslash_escape(true)
.empty_alternates(false)`, matched with `Candidate::from_bytes` (scratch crate, offline):

| glob | path | result |
|---|---|---|
| `*.rs` | `src/a.rs` | false |
| `*.rs` | `a.rs` | true |
| `**/*.rs` | `a.rs` / `src/a/b.rs` | true / true |
| `src/*.{rs,toml}` | `src/a.toml` / `src/x/a.rs` | true / false |
| `[ab].md` | `a.md` / `c.md` | true / false |
| `?.md` | `a.md` / `ab.md` / `/.md` | true / false / false |
| `src/**` | `src/a/b` / `src` | true / false |
| `a**b` | `axxb` | true (treated as `a*b`) |
| `{a,}` | `a` | true (accepted) |
| `{a,{b,c}}` | `c` | true (nested supported) |
| `:foo`, `src/a:b.rs` | themselves | true (literal `:`) |
| `[a` | — | `kind()`: `unclosed character class; missing ']'` |
| `{a,b` | — | `unclosed alternate group; missing '}' (maybe escape '{' with '[{]'?)` |
| `a}` | — | `unopened alternate group; missing '{' (maybe escape '}' with '[}]'?)` |
| `a\` | — | `dangling '\'` |
| `[z-a]` | — | `invalid range; 'z' > 'a'` |

`Error`'s own `Display` is `error parsing glob '<g>': <kind>`; `GlobMatcher` derives `Clone, Debug`
only (`glob.rs:132-133`).

**Postgres 16** (scratch DB, `0001`..`0007`):
- Constraint names: `skill_name_key` (UNIQUE name), `skill_pkey`, `skill_created_by_fkey`,
  `skill_version_pkey` `(skill_id, version)`, `skill_binding_skill_id_project_id_phase_id_key`
  `UNIQUE NULLS NOT DISTINCT`, `skill_binding_glob_needs_globs`, `skill_binding_phase_needs_project`,
  `skill_binding_activation_check`.
- `INSERT … ON CONFLICT (skill_id, project_id, phase_id) DO NOTHING` **infers the NULLS NOT DISTINCT
  key**: a second global attach of one skill inserts 0 rows. Two sessions: the second **blocks** on
  the first's uncommitted row (2.0 s against a 3 s hold) and then inserts nothing.
- Parameter types: `project_id IS NOT DISTINCT FROM $1` → `{uuid}`; the key read →
  `{uuid,uuid,uuid}`; W4 (§3.6) → `{uuid,integer,text,jsonb,uuid}`, applied at head 2, 0 rows at a
  spent head, 0 rows for an unknown skill; W6 → `{uuid,timestamptz,integer,integer,text,text[],text[]}`.

---

## 1. Build order and validation, at a glance

| Task | Crates | Commits (min., each compiles) | Gate |
|---|---|---|---|
| T1 names, globs, languages, write types | htui-core, workspace `Cargo.toml`/lock | 2 (red, green) | core; `cargo tree -p htui-core -e normal \| grep -E 'globset\|^.*log '` shows `globset` and no `log` under it; `cargo build --workspace --all-features --all-targets` |
| T2 writers and readers | htui-core, htui-store, htui-agent | 3 (red, Mem green, Postgres) | core; store (Postgres); agent; `cargo sqlx prepare --check`; `.sqlx` = 276; workspace build |
| T3 clone gap | htui-core, htui-store, htui-orch, htui (`catalogue.rs`) | 2 | core; store; orch; `cargo build --workspace --all-features --all-targets` (T3 changes a struct every crate builds) |
| T4 store worker | htui | 2 | htui lib (`cargo test -p htui --all-features --lib skills`) then the htui gate |
| T5 Skills view | htui | 3 (red, library, attach) | htui (Postgres); `cargo insta review` of the seven new snapshots; `templates__*.snap` unchanged |
| merge | — | T3, then T4, then T5 | each merge: the touched crates' gates on the merged tree; then §9 |

---

## 2. T1: names, globs, languages, write types (D71–D74; D87, D93, D94, D105, D106)

**Files** (plan's list; the write types join `skill.rs`, F-B): `Cargo.toml`, `Cargo.lock`,
`crates/htui-core/Cargo.toml`, `crates/htui-core/src/model/skill.rs`,
`crates/htui-core/src/model/skill_glob.rs` (new), `crates/htui-core/src/model/skill_language.rs`
(new), `crates/htui-core/src/model/skill_languages.json` (new), `crates/htui-core/src/model/mod.rs`.

**First failing test**: `model::skill_glob::tests::the_d74_table_matches_as_documented`.

### 2.1 Dependency

`Cargo.toml` `[workspace.dependencies]`, after `sha2` (`:61`):

```toml
# MOD-9 D72 (OQ-15): skill attachment globs. `log` off; its other deps are already locked.
globset            = { version = "0.4.20", default-features = false }
```

`crates/htui-core/Cargo.toml` `[dependencies]`: `globset = { workspace = true }`. `cargo build`
adds `globset 0.4.20` to `Cargo.lock` and nothing else.

### 2.2 `model/skill.rs` additions

Module doc `:1-6` becomes: "… The writers are MOD-9 milestone 3's (`WriteStore::create_skill` and
three others); a `glob` attachment fires from PRD milestone 5 (D86)." Rewrite `:20-21`, `:115`,
`:178`, `:254`, `:263`, `:310` the same way ("milestone 3" → "PRD milestone 5, D86") (F-L).

```rust
/// D71: a skill name is 1-64 bytes of `[a-z0-9-]` with no leading, trailing or doubled hyphen
/// (ANA-22 §6 item 12, the Agent Skills rule). `true` when `name` may be stored.
///
/// Checked by the writers and the Skills view, not by a constraint, so a hand-written row still
/// loads; [`invalid_skill_name`](crate::store::invalid_skill_name) phrases the refusal.
#[must_use]
pub fn validate_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

/// Arguments of `WriteStore::create_skill` (D75, D77): the `skill` row **and** its version 1,
/// written together so no skill exists without a body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSkill {
    /// `skill.id`, minted client-side as a UUIDv7.
    pub id: SkillId,
    /// `skill.name`; must pass [`validate_name`].
    pub name: String,
    /// `skill.description`, the picker's one-liner; may be empty.
    pub description: String,
    /// Version 1's `skill_version.body`; refused when blank (D77, OQ-20).
    pub body: String,
    /// Version 1's `skill_version.source`: `{}` from the Skills view; milestone 4's import fills it.
    pub source: serde_json::Value,
    /// `skill.created_by` and version 1's `created_by`.
    pub created_by: UserId,
}

/// Edit passed to `WriteStore::update_skill` (D76); `None` leaves the column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillPatch {
    /// `skill.name`; must pass [`validate_name`] and be free (OQ-17: a rename is allowed).
    pub name: Option<String>,
    /// `skill.description`.
    pub description: Option<String>,
}

/// Arguments of `WriteStore::add_skill_version` (D75, D77); the version number is the store's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSkillVersion {
    /// `skill_version.body`; refused when blank.
    pub body: String,
    /// `skill_version.source`, `{}` from the Skills view.
    pub source: serde_json::Value,
    /// `skill_version.created_by`.
    pub created_by: UserId,
}

/// The natural key of one attachment (D78): `UNIQUE NULLS NOT DISTINCT (skill_id, project_id,
/// phase_id)`. `project: None` is global; `phase: Some` needs `project: Some`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SkillBindingKey {
    /// `skill_binding.skill_id`.
    pub skill: SkillId,
    /// `skill_binding.project_id`.
    pub project: Option<ProjectId>,
    /// `skill_binding.phase_id`.
    pub phase: Option<PhaseId>,
}

impl SkillBindingKey {
    /// The key a stored row sits under.
    #[must_use]
    pub fn of(binding: &SkillBinding) -> Self { /* skill_id, project_id, phase_id */ }

    /// The level this key names, by [`SkillBinding::level`]'s rule.
    #[must_use]
    pub fn level(self) -> SkillLevel { /* same match as SkillBinding::level */ }
}

/// What an attachment says (D75, D78): the editable columns, `globs` **as typed** — the writer
/// stores `canonical_globs(globs, languages)` and the normalised languages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// `skill_binding.pinned_version`; `None` follows the latest.
    pub pinned_version: Option<i32>,
    /// `skill_binding.position`, `>= 0`.
    pub position: i32,
    /// `skill_binding.activation`.
    pub activation: Activation,
    /// Typed globs: `<glob>` or `<repo>:<glob>` (D74).
    pub globs: Vec<String>,
    /// Typed language names (D73).
    pub languages: Vec<String>,
}

impl Attachment {
    /// A stored row's attachment, as D80's clone copies it: the stored `globs` passed as typed with
    /// the stored `languages`, which `canonical_globs` leaves unchanged (it is idempotent).
    #[must_use]
    pub fn of(binding: &SkillBinding) -> Self { /* clone the five fields */ }
}

/// What `WriteStore::set_skill_binding` does to the row at its key (D75).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingChange {
    /// Insert (`expected: None`) or replace (`expected: Some(updated_at)`) the row.
    Attach(Attachment),
    /// Delete the row.
    Detach,
}
```

`model/mod.rs:144-147` re-export gains `Attachment, BindingChange, NewSkill, NewSkillVersion,
SkillBindingKey, SkillPatch`; `pub mod skill_glob;` and `pub mod skill_language;` join the list
after `pub mod skill;` (`:95`); a second `pub use skill_glob::{GlobError, SkillGlob, SkillGlobs};`
and `pub use skill_language::UnknownLanguage;`. Functions stay path-qualified
(`model::skill_glob::canonical_globs`), as `resolve`/`select` do (milestone 2 D51).

### 2.3 `model/skill_languages.json` (D106) — complete

```json
{
  "c":          ["**/*.c", "**/*.h"],
  "cpp":        ["**/*.cc", "**/*.cpp", "**/*.cxx", "**/*.hh", "**/*.hpp", "**/*.hxx"],
  "csharp":     ["**/*.cs", "**/*.csproj", "**/*.sln"],
  "go":         ["**/*.go", "**/go.mod", "**/go.sum"],
  "java":       ["**/*.java", "**/pom.xml", "**/build.gradle", "**/build.gradle.kts"],
  "javascript": ["**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs", "**/package.json"],
  "markdown":   ["**/*.md", "**/*.markdown"],
  "python":     ["**/*.py", "**/*.pyi", "**/pyproject.toml"],
  "rust":       ["**/*.rs", "**/Cargo.toml"],
  "shell":      ["**/*.sh", "**/*.bash", "**/*.zsh"],
  "sql":        ["**/*.sql"],
  "toml":       ["**/*.toml"],
  "typescript": ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts", "**/tsconfig.json"],
  "yaml":       ["**/*.yaml", "**/*.yml"]
}
```

Keys are lowercase ASCII; no key appears twice; `rust` is exactly the acceptance's
`**/*.rs, **/Cargo.toml`.

### 2.4 `model/skill_language.rs` (new)

```rust
//! MOD-9 D73: the language map (ANA-22 §6 item 5), data not code: `skill_languages.json`,
//! compiled in and parsed once. The writer stores the expanded globs, so the map moving never
//! changes a saved attachment (R-26). No `app_setting` overlay (OQ-19).

use std::collections::BTreeMap;
use std::sync::LazyLock;

/// Language name → globs, in name byte order.
static MAP: LazyLock<BTreeMap<String, Vec<String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("skill_languages.json"))
        .expect("the compiled skill language map parses")
});

/// A language name the map does not hold, after trim and ASCII lowercase (D73).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownLanguage {
    /// The name as normalised.
    pub name: String,
}

// Hand-written (D105): "unknown language `klingon` (known: c, cpp, csharp, go, java, javascript,
// markdown, python, rust, shell, sql, toml, typescript, yaml)"
impl core::fmt::Display for UnknownLanguage { … }
impl std::error::Error for UnknownLanguage {}

/// Every language name, in byte order.
pub fn known() -> impl Iterator<Item = &'static str> { MAP.keys().map(String::as_str) }

/// D78's stored `languages`: each trimmed and ASCII-lowercased, empties dropped, duplicates
/// dropped keeping the first. Does not check the names.
#[must_use]
pub fn normalise(languages: &[String]) -> Vec<String>;

/// The globs of `languages` (normalised first), concatenated in input order, duplicates dropped
/// keeping the first.
///
/// # Errors
/// [`UnknownLanguage`] for the first name the map does not hold.
pub fn expand(languages: &[String]) -> Result<Vec<String>, UnknownLanguage>;
```

### 2.5 `model/skill_glob.rs` (new)

```rust
//! MOD-9 D74: attachment globs. One place turns typed text into stored globs, for the writer and
//! the Skills form's `effective:` line alike. Syntax is `globset`'s (ANA-22 §6 item 7): `*` and `?`
//! never cross `/`, `**` as a whole segment does, `{a,b}`, `[...]`, `\` escapes. Paths are
//! repo-relative with `/`. A bare glob matches in **any** repo of the step's scope (§6 item 6),
//! unlike `touched_paths`' primary-repo rule (R-30).

/// Characters that end the qualifier search: a `:` after any of these is part of the glob.
const META: [char; 7] = ['*', '?', '[', ']', '{', '}', '\\'];

/// One glob, parsed: `<repo>:<glob>` or a bare `<glob>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkillGlob {
    /// The repo qualifier; `None` for a bare glob.
    pub repo: Option<String>,
    /// The glob proper, compiled once by [`SkillGlob::parse`].
    pub glob: String,
}

impl SkillGlob {
    /// Parses one **trimmed** entry. The qualifier is the text before the first `:` when it is
    /// non-empty and holds no `/` and no [`META`] char (so `src/a:b.rs` and `:foo` are bare).
    ///
    /// # Errors
    /// [`GlobError::Invalid`]: a NUL anywhere ("contains a NUL character"); nothing after the
    /// qualifier ("has no glob after `<repo>:`"); `globset` refusing the glob (its `kind()`).
    pub fn parse(text: &str) -> Result<Self, GlobError>;
}

/// `<repo>:<glob>` or `<glob>`: the canonical stored text.
impl core::fmt::Display for SkillGlob { … }

/// Why a list of globs cannot be stored (D94).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GlobError {
    /// One entry is not a glob. `glob` is the whole entry as typed (qualifier included).
    #[error("glob `{glob}`: {message}")]
    Invalid {
        /// The entry.
        glob: String,
        /// Why: `globset`'s `ErrorKind` text, or this module's own sentence.
        message: String,
    },
    /// A language name the map does not hold.
    #[error(transparent)]
    UnknownLanguage(#[from] crate::model::skill_language::UnknownLanguage),
}

/// D74 as amended by D94: expand `languages` (refusing an unknown one), then trim `typed`, drop
/// empties, parse each; the result is the typed entries in order followed by the expanded
/// language globs, duplicates dropped keeping the first. Idempotent on its own output.
///
/// # Errors
/// The first [`GlobError`], languages before globs (D78's order).
pub fn canonical_globs(typed: &[String], languages: &[String]) -> Result<Vec<String>, GlobError>;

/// D93: a comma list as the Skills form takes it. Splits on `,` only outside `{…}` (depth 0) and
/// `[…]`, with `\x` taken literally; trims each part; drops empties.
#[must_use]
pub fn split_list(text: &str) -> Vec<String>;

/// A compiled attachment, for PRD milestone 5's `select` (D86). Tested now, called there.
#[derive(Debug, Clone)]
pub struct SkillGlobs {
    /// Each glob's qualifier and matcher, in stored order.
    globs: Vec<(Option<String>, globset::GlobMatcher)>,
}

impl SkillGlobs {
    /// # Errors
    /// The first entry [`SkillGlob::parse`] refuses.
    pub fn compile(globs: &[String]) -> Result<Self, GlobError>;

    /// The first path, in `paths` order, that any glob matches in `repo` — a bare glob in any repo,
    /// a qualified one only in its own. Matching is on raw bytes (`Candidate::from_bytes`).
    #[must_use]
    pub fn first_match<'a>(
        &self,
        repo: &str,
        paths: impl IntoIterator<Item = &'a str>,
    ) -> Option<&'a str>;
}

/// The one builder (D72): `literal_separator(true)`, `backslash_escape(true)`,
/// `empty_alternates(false)`.
fn matcher(glob: &str) -> Result<globset::GlobMatcher, GlobError>;
```

### 2.6 Tests (T1)

`model/skill.rs` `mod tests`:
- `skill_names_follow_the_agent_skills_rule`: accepts `rust-style`, `tests`, `a`, 64 × `a`;
  refuses ``, 65 × `a`, `Rust`, `-a`, `a-`, `a--b`, `a_b`, `a b`, `ä`.
- `a_key_and_an_attachment_copy_a_row`: `SkillBindingKey::of` / `Attachment::of` over a phase row;
  `key.level() == SkillLevel::Phase`.

`model/skill_glob.rs` `mod tests`:
- `the_d74_table_matches_as_documented`: every row of §0.3's match table through
  `SkillGlobs::compile(&[g]).first_match("r", [p])`, plus `htui:**/*.rs` → repo `htui`, glob
  `**/*.rs`; `src/a:b.rs` → bare; `:foo` → bare.
- `a_bad_glob_is_refused_by_name`: `src/[a` → `GlobError::Invalid { glob: "src/[a", .. }`, message
  contains `unclosed character class`; `htui:` → "has no glob after `htui:`"; `"a\0"` → NUL.
- `canonical_globs_orders_dedups_and_appends_languages`: typed `[" src/** ", "", "**/*.rs"]`,
  languages `["Rust"]` → `["src/**", "**/*.rs", "**/Cargo.toml"]`.
- `canonical_globs_is_idempotent_on_its_output`: `canonical_globs(&out, &["rust"]) == out` and
  `canonical_globs(&out, &[]) == out`.
- `canonical_globs_refuses_a_language_before_a_glob`: typed `["[a"]`, languages `["klingon"]` →
  `GlobError::UnknownLanguage`.
- `split_list_keeps_braces_and_classes_whole`: `"src/*.{rs,toml}, docs/**,[a,b].md,, a\\,b "` →
  `["src/*.{rs,toml}", "docs/**", "[a,b].md", "a\\,b"]`.
- `first_match_respects_the_qualifier`: globs `["htui:src/**", "**/*.md"]`: repo `htui`, paths
  `["a.rs", "src/x.rs"]` → `Some("src/x.rs")`; repo `web`, `["src/x.rs", "README.md"]` →
  `Some("README.md")`; repo `web`, `["src/x.rs"]` → `None`.

`model/skill_language.rs` `mod tests`:
- `every_seed_glob_compiles` (through `SkillGlob::parse`).
- `the_fourteen_seed_names_are_present` (`known()` equals the 14 of D73, byte order).
- `names_are_trimmed_and_lowercased`: `expand(&["Rust"])`, `expand(&[" rust "])` →
  `["**/*.rs", "**/Cargo.toml"]`; `normalise(&[" Rust ", "rust", "", "TOML"])` → `["rust", "toml"]`.
- `an_unknown_language_is_refused_by_name`: `expand(&["klingon"])` → `UnknownLanguage { name:
  "klingon" }`; its `Display` contains `` `klingon` `` and `rust`.

### 2.7 Gate and commits

```bash
cargo test -p htui-core --all-features -- --test-threads=2
cargo tree -p htui-core -e normal | grep -E 'globset|log'    # globset v0.4.20, no `log` under it
cargo build --workspace --all-features --all-targets
cargo clippy -p htui-core --all-features --all-targets -- -D warnings
git diff --stat Cargo.lock                                  # one package added: globset
```

Commits: (1) red — the modules with `todo!()` bodies, the JSON, the tests, the dependency;
(2) green.

---

## 3. T2: writers and readers (D75–D79, D85; D88–D92)

**Files** (plan's list **plus** F-C): `crates/htui-core/src/store/traits.rs`,
**`crates/htui-core/src/store/mod.rs`**, `crates/htui-core/src/store/mem.rs`,
`crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`,
`crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/pg/rows.rs`
(no edit, F-J), `crates/htui-store/src/writer.rs`, `crates/htui-store/.sqlx/` (+12),
`crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/skill_writers.rs` (new),
`crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`.

**First failing test**: `cargo test -p htui-core --all-features --test mem_store`
(`demo_store_loads_the_fixture`, pin 80).

### 3.1 `store/traits.rs`: the seven methods

After `append_prompt_template` (`:724-728`), before `// settings (D7, D8)`:

```rust
    // skill, skill_version, skill_binding (MOD-9 milestone 3, plan D75-D79)
    //
    // Readers sit beside the writers, as MOD-15's do (`:472-473`), so the conformance suite can
    // read back what it wrote on both stores; the not-mirrored rule of this file's header is about
    // `ReadStore` and the mirror, which these do not touch. Every writer is a compare-and-set, and
    // the precedence is `append_prompt_template`'s: the token first, then `NotFound`, then
    // `Constraint` (D75). The refusal sentences are the pure helpers below (D88).

    /// Every skill, ordered by `name` bytes (`COLLATE "C"`).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn skills(&self) -> Result<Vec<Skill>>;

    /// One skill's versions, ascending; empty for an unknown skill.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn skill_versions(&self, skill: SkillId) -> Result<Vec<SkillVersion>>;

    /// `None`: the global attachments; `Some(p)`: `p`'s project and phase attachments. Ordered by
    /// `(skill_id, phase_id)` bytes, `NULL` phase first (D91).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn skill_bindings(&self, project: Option<ProjectId>) -> Result<Vec<SkillBinding>>;

    /// Inserts the skill and its version 1 together (D75, D77, OQ-16).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for
    /// [`new_skill_refusal`]'s sentences, a taken name (`already_exists("skill", name)`), a taken
    /// id, or a `created_by` that names no user. Nothing is written.
    async fn create_skill(&self, new: NewSkill) -> Result<(Skill, SkillVersion)>;

    /// Renames and/or re-describes under CAS on `skill.updated_at` (D76, OQ-17). Order: `NotFound`
    /// (unknown id), then `Stale` (spent token, even for bad input), then `Constraint`.
    ///
    /// # Errors
    /// `NotFound { entity: "skill" }`; `Constraint` for [`skill_patch_refusal`]'s sentences or a
    /// taken name (`already_exists("skill", name)`).
    async fn update_skill(
        &self,
        id: SkillId,
        expected: DateTime<Utc>,
        patch: SkillPatch,
    ) -> Result<CasOutcome<Skill>>;

    /// Appends version `expected + 1` iff the head is `expected` (`0`: "the skill has no version",
    /// a hand-written or imported row). Order (D89): a head other than `expected` is `Stale(head)`;
    /// then `NotFound { entity: "skill" }`; then, with no version at all and `expected != 0`,
    /// `NotFound { entity: "skill_version", id: skill_version_key(skill, expected) }`; then
    /// `Constraint`. `skill.updated_at` is not touched.
    ///
    /// # Errors
    /// As above; `Constraint` for [`skill_body_refusal`]'s sentences or an unknown `created_by`.
    async fn add_skill_version(
        &self,
        skill: SkillId,
        expected: i32,
        new: NewSkillVersion,
    ) -> Result<CasOutcome<SkillVersion>>;

    /// Attaches, changes or detaches the one row at `key` (D78). `expected: None` is "I expect no
    /// row"; `Some(t)` is that row's `updated_at`. Order (D90): the row at `key` is read and a
    /// token that does not match it answers `Stale(row or None)`; then `NotFound` for the skill,
    /// the project, the phase (`entity`: `"skill"`, `"project"`, `"step_graph_phase"`); then
    /// `Detach` deletes (no row: `Applied(None)`); then [`check_attachment`]'s `Constraint`s; then
    /// the write. `Applied` carries the row as stored (`None` after a detach).
    ///
    /// # Errors
    /// As above.
    async fn set_skill_binding(
        &self,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
    ) -> Result<CasOutcome<Option<SkillBinding>>>;
```

The import list (`:35-48`) gains `Attachment, BindingChange, NewSkill, NewSkillVersion, Skill,
SkillBinding, SkillBindingKey, SkillId, SkillPatch, SkillVersion`.

### 3.2 `store/traits.rs`: refusal helpers (D88), beside `invalid_template_name` (`:1440-1448`)

Exact sentences. `store/mod.rs:15-26` re-exports every item below (F-C).

```rust
/// MOD-9 D71: a name [`validate_name`](crate::model::skill::validate_name) refuses.
#[must_use]
pub fn invalid_skill_name(name: &str) -> String {
    format!(
        "skill.name `{}` must be 1-64 of a-z, 0-9 and single inner hyphens",
        name.escape_debug()
    )
}

/// MOD-9 D77 (OQ-20): a blank body renders an empty `<skill>` block that costs tokens and says
/// nothing.
pub const BLANK_SKILL_BODY: &str = "a skill needs text";

/// A `text` column Postgres cannot hold (`22021`), refused by rule on both stores.
#[must_use]
pub fn has_nul(column: &str) -> String {
    format!("{column} must not contain a NUL character")
}

/// D77: why a body may not be stored — blank first, then a NUL (`skill_version.body`).
#[must_use]
pub fn skill_body_refusal(body: &str) -> Option<String>;

/// D71, D77: `create_skill`'s input, in order: name, description NUL (`skill.description`), body.
#[must_use]
pub fn new_skill_refusal(name: &str, description: &str, body: &str) -> Option<String>;

/// D76: `update_skill`'s input: the name (if any), then the description's NUL (if any).
#[must_use]
pub fn skill_patch_refusal(patch: &SkillPatch) -> Option<String>;

/// The `NotFound` id of a missing version: `"{skill}/v{version}"`.
#[must_use]
pub fn skill_version_key(skill: SkillId, version: i32) -> String;

/// D78: a phase key with no project.
#[must_use]
pub fn phase_attachment_needs_a_project(phase: PhaseId) -> String {
    format!("a phase attachment needs its project: step_graph_phase {phase} was given none")
}

/// D78: the phase's graph belongs to another project (`graph_not_in_project`'s shape, `:1419`).
#[must_use]
pub fn phase_not_in_project(phase: PhaseId, project: ProjectId) -> String {
    format!("step_graph_phase {phase} is not in project {project}")
}

/// D78: a pin to a version the skill does not have.
#[must_use]
pub fn pin_names_no_version(skill: &str, version: i32) -> String {
    format!("skill_binding.pinned_version `{version}` names no version of skill `{skill}`")
}

/// D78: `position < 0`.
#[must_use]
pub fn negative_position(position: i32) -> String {
    format!("skill_binding.position `{position}` must be 0 or more")
}

/// D78: a qualified glob on a global row.
#[must_use]
pub fn global_glob_names_a_repo(glob: &str, repo: &str) -> String {
    format!("glob `{glob}` names repo `{repo}`, but a global attachment belongs to no project")
}

/// D79 (OQ-18), verbatim.
#[must_use]
pub fn glob_names_unknown_repo(glob: &str, repo: &str, project_slug: &str) -> String {
    format!("glob `{glob}` names repo `{repo}`, which project `{project_slug}` does not have")
}

/// D78: `activation = glob` with no effective glob (the DB's glob-needs-globs check backs it).
pub const GLOB_NEEDS_GLOBS: &str =
    "an attachment with activation `glob` needs at least one glob or language";

/// What a store looked up before [`check_attachment`] (D88): the facts the rule needs, so the
/// rule itself is pure and has one definition.
#[derive(Debug, Clone, Copy)]
pub struct BindingFacts<'a> {
    /// The key being written.
    pub key: SkillBindingKey,
    /// The project owning `key.phase`'s graph (`None` when `key.phase` is `None`).
    pub phase_project: Option<ProjectId>,
    /// `skill.name`, for the pin sentence.
    pub skill_name: &'a str,
    /// Every version number the skill has.
    pub versions: &'a [i32],
    /// `key.project`'s repo names (empty for a global key).
    pub repos: &'a [String],
    /// `key.project`'s slug (empty for a global key), for D79's sentence.
    pub project_slug: &'a str,
}

/// The columns [`check_attachment`] derives: what is stored beside the attachment's own three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAttachment {
    /// `canonical_globs(attachment.globs, attachment.languages)`.
    pub globs: Vec<String>,
    /// `skill_language::normalise(attachment.languages)`.
    pub languages: Vec<String>,
}

/// D78's `Constraint` chain, in order, first refusal wins:
/// 1. `key.phase` without `key.project` → [`phase_attachment_needs_a_project`];
/// 2. `phase_project != key.project` → [`phase_not_in_project`];
/// 3. `pinned_version: Some(n)` not in `versions` → [`pin_names_no_version`];
/// 4. `position < 0` → [`negative_position`];
/// 5. `canonical_globs` → the `GlobError`'s `Display` (languages, then globs);
/// 6. per canonical glob with a qualifier: global key → [`global_glob_names_a_repo`]; repo not in
///    `repos` → [`glob_names_unknown_repo`];
/// 7. `Glob` with no canonical glob → [`GLOB_NEEDS_GLOBS`].
///
/// # Errors
/// The sentence.
pub fn check_attachment(
    facts: &BindingFacts<'_>,
    attachment: &Attachment,
) -> core::result::Result<StoredAttachment, String>;
```

The file's header doc (`:21-26`) gains one sentence: "MOD-9 milestone 3 adds the skill readers
to `WriteStore` beside their writers (plan D75), as MOD-15 did; the bound-skill read the prompt
uses stays inherent."

### 3.3 `store/mem.rs` (D87–D91; F-Q)

State fields `:114-119`: docs name the new writers ("`skill`, read by `bound_skills` and
`WriteStore::skills`, written by `create_skill` and `update_skill`" etc.). New `impl State` fns
after `append_prompt_template` (`:2696-2749`); each arm in `impl WriteStore for MemStore` after
`:5468` is one line that takes `let now = Utc::now();` before `self.write(…)`, exactly as MOD-15's
(`mem.rs:5320-5334`). No `MemFault` variant: nothing needs a skill write to be unreachable (the
view's refusal path is tested offline, §5.6).

```rust
    /// D91: name byte order.
    fn skill_rows(&self) -> Vec<Skill>;
    /// D91: one skill's versions by `version`.
    fn skill_version_rows(&self, skill: SkillId) -> Vec<SkillVersion>;
    /// D91: `project_id == project`, sorted by `(skill_id, phase_id)` (`None < Some`, as `NULLS
    /// FIRST`; `Uuid`'s `Ord` is byte order, as Postgres's `uuid` comparison).
    fn skill_binding_rows(&self, project: Option<ProjectId>) -> Vec<SkillBinding>;
    /// The row at a natural key.
    fn skill_binding_at(&self, key: SkillBindingKey) -> Option<&SkillBinding>;

    /// Order: `new_skill_refusal`; `require_user(created_by, "skill.created_by")` (`:1867-1876`);
    /// a taken id → `already_exists("skill", id)`; a taken name → `already_exists("skill", name)`.
    /// Writes `Skill { created_at: now, updated_at: now, .. }` and `SkillVersion { version: 1,
    /// created_at: now, .. }`.
    fn create_skill(&mut self, new: NewSkill, now: DateTime<Utc>) -> Result<(Skill, SkillVersion)>;

    /// `update_workspace`'s shape (`:1906-1951`): NotFound("skill") → Stale(current) →
    /// `skill_patch_refusal` → taken name (another id) → apply both `Some` fields,
    /// `updated_at = now` (an all-`None` patch still stamps, as the Postgres trigger does).
    fn update_skill(
        &mut self,
        id: SkillId,
        expected: DateTime<Utc>,
        patch: SkillPatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Skill>>;

    /// D89's order; pushes `SkillVersion { version: expected + 1, created_at: now, .. }`.
    fn add_skill_version(
        &mut self,
        skill: SkillId,
        expected: i32,
        new: NewSkillVersion,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<SkillVersion>>;

    /// D90's order. NotFound checks: `skills`, `projects`, `phase(id)` (`:1858`). `phase_project`
    /// is `graphs[&phase.graph_id].project_id`; `versions`/`repos`/slug from state. Attach with no
    /// row pushes `SkillBinding { id: SkillBindingId::new(), updated_at: now, .. }`; with a row,
    /// replaces its five columns in place and stamps `now` (the id is kept); Detach `retain`s the
    /// row out.
    fn set_skill_binding(
        &mut self,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Option<SkillBinding>>>;
```

### 3.4 Precedence, spelled once (D89, D90)

`add_skill_version(skill, expected, new)`:
1. `head` = the highest version row of `skill` (`None` when there is none).
2. `Some(h)` and `h.version != expected` → `Ok(Stale(h))`.
3. No `skill` row → `NotFound { entity: "skill", id: skill }`.
4. `head` is `None` and `expected != 0` → `NotFound { entity: "skill_version", id:
   skill_version_key(skill, expected) }`.
5. `skill_body_refusal(body)` → `Constraint`; unknown `created_by` → `Constraint`.
6. Append `expected + 1`.

`set_skill_binding(key, expected, change)`:
1. `current` = row at `key`.
2. `current.map(|r| r.updated_at) != expected` → `Ok(Stale(current))` (so `Some(t)` with no row is
   `Stale(None)`, `None` with a row is `Stale(Some(row))`).
3. Skill, then project (`key.project: Some`), then phase (`key.phase: Some`) exist, else `NotFound`
   (`"skill"`, `"project"`, `"step_graph_phase"`).
4. `Detach`: `current` is `None` → `Applied(None)`; else delete → `Applied(None)`.
5. `Attach(a)`: `check_attachment(&facts, &a)` → `Constraint(sentence)`.
6. Write (insert when `current` is `None`, replace otherwise) → `Applied(Some(row))`.

`update_skill`: `NotFound` → `Stale` → `Constraint` (the MOD-15 order; a "token first" reading has
no row to compare against for an unknown id).

### 3.5 `store/conformance.rs`: six cases (D85)

`CASES` (`:43-118`) gains, **appended after** `"prompt_template_refuses_what_parse_refuses"`:

```
"skill_create_reads_back_with_version_one",
"skill_update_is_a_compare_and_set_and_refuses_a_taken_name",
"skill_version_append_is_a_compare_and_set_on_the_head",
"skill_binding_attach_change_detach_are_compare_and_set",
"skill_binding_refuses_by_rule",
"skill_binding_stores_expanded_globs_and_languages_as_typed",
```

and `run_case` (`:132-297`) one arm each, before `other => panic!`, in the multi-line `=> { …; }`
form the long names use (`:283-291`). New section header after `:5420`-ish:
`// MOD-9 milestone 3: the skill writers (plan D75-D79)` with **F-D's rule** for its doc comments.
Helpers: `fn new_skill(name: &str, body: &str) -> NewSkill` (fresh id, description `""`, source
`json!({})`, `ids::USER`), `fn version(body: &str) -> NewSkillVersion`, `fn attachment(activation:
Activation, position: i32) -> Attachment` (no pin, no globs, no languages), `fn key(skill: SkillId,
project: Option<ProjectId>, phase: Option<PhaseId>) -> SkillBindingKey`. Use `applied`/`stale`
(`:1937-1951`).

1. **`skill_create_reads_back_with_version_one`**: `create_skill(new_skill("docs-style", "Write in
   the active voice."))` → `(skill, v1)`: name, description `""`, `created_by == ids::USER`,
   `created_at == updated_at`; `v1 == SkillVersion { skill_id: skill.id, version: 1, body, source:
   json!({}), created_by: ids::USER, created_at: v1.created_at }`. `skills()` names ==
   `["docs-style", "rust-style", "tests"]`; `skill_versions(skill.id) == [v1]`;
   `skill_versions(ids::SKILL_RUST_STYLE)` versions `[1, 2]`; `skill_versions(SkillId::new())` empty.
   Refusals, each `Constraint` and each checked by message needle, then `skills().len() == 3`:
   name `"Docs"` (needle `skill.name`); name `"tests"` (needle `` `tests` already exists ``); body
   `"  \n"` (needle `a skill needs text`); `created_by: UserId::new()` (any `Constraint`).
2. **`skill_update_is_a_compare_and_set_and_refuses_a_taken_name`**: `row` = `rust-style` from
   `skills()`. `update_skill(row.id, row.updated_at, SkillPatch { name: Some("rust-house") })` →
   applied: name changed, description unchanged, `updated_at > row.updated_at`. Same token again with
   `description: Some("x")` → `stale` carrying `rust-house`. Fresh token, `name: Some("tests")` →
   `Constraint` needle `` `tests` already exists ``. Spent token with `name: Some("A B")` → `Stale`
   (token first). Fresh token, `name: Some("a--b")` → `Constraint` needle `skill.name`. Unknown id →
   `NotFound { entity: "skill", .. }`. `skill_versions(row.id)` still `[1, 2]`.
3. **`skill_version_append_is_a_compare_and_set_on_the_head`**: `add_skill_version(RUST_STYLE, 2,
   version("v3 body"))` → applied `version == 3`, body, `source == {}`, `created_by`. Again at 2 →
   `stale` with `(3, "v3 body")`. At 2 with a blank body → `Stale` (token first). At 3 with a blank
   body → `Constraint` needle `a skill needs text`. `SkillId::new()` at 0 → `NotFound { entity:
   "skill" }`. `skill_versions` → `[1, 2, 3]`. (The `"skill_version"` arm needs a skill with no
   version, which no writer can make — `create_skill` always writes v1 — so it is `MemStore`'s unit
   test, §3.9.)
4. **`skill_binding_attach_change_detach_are_compare_and_set`**: global key `(TESTS, None, None)`:
   attach `(None, attachment(Always, 3))` → applied `Some(row)`, `row.project_id == None`,
   `skill_bindings(None) == [row]`; again with `None` → `stale(Some(row))`; change with
   `Some(row.updated_at)` to `(Off, 4)` → applied `Some(row2)`, `row2.id == row.id`, `updated_at`
   advanced; change with the spent `row.updated_at` → `stale(Some(row2))`; detach with the spent
   token → `stale(Some(row2))`; detach with `row2.updated_at` → applied `None`, `skill_bindings(None)`
   empty; detach again with `Some(row2.updated_at)` → `stale(None)`; detach with `None` and no row →
   applied `None`. Phase key `(RUST_STYLE, Some(PROJECT_HTUI), Some(PHASE_HTUI_IMPLEMENT))`: attach
   with `None` → `stale(Some(demo row))` (its id is `ids::BINDING_HTUI_IMPLEMENT_RUST_STYLE`); change
   with the demo row's `updated_at` (read from `skill_bindings(Some(PROJECT_HTUI))`) to pin `Some(2)`
   → applied, `pinned_version == Some(2)`. `skill_bindings(Some(PROJECT_HTUI))` has three rows in D91
   order.
5. **`skill_binding_refuses_by_rule`**: `create_repo(new_repo(PROJECT_HTUI, "core", true))` first;
   then `before = (skill_bindings(None), skill_bindings(Some(PROJECT_HTUI)),
   skill_bindings(Some(PROJECT_AGY)))`. Each below is `Constraint` with the needle given, and after
   all of them the triple equals `before`. Unless a key is named, the key is `(TESTS,
   Some(PROJECT_AGY), None)` with `expected: None` (agy has no row, so the token passes):
   - phase of another project: key `(TESTS, Some(PROJECT_AGY), Some(PHASE_HTUI_IMPLEMENT))` → `is
     not in project`;
   - phase without project: `(TESTS, None, Some(PHASE_HTUI_IMPLEMENT))` → `needs its project`;
   - missing pin: pin `Some(9)` → `pinned_version`;
   - unknown language `["klingon"]` → `klingon`;
   - bad glob `["src/[a"]` → `src/[a`;
   - qualified on global: `(TESTS, None, None)`, globs `["core:**/*.rs"]` → `global attachment`;
   - unknown repo: `(TESTS, Some(PROJECT_AGY), None)`, globs `["nosuchrepo:**/*.rs"]` → `nosuchrepo`;
   - glob without globs: activation `Glob`, no globs, no languages → `needs at least one glob`;
   - negative position `-1` → `0 or more`.
   `NotFound`: skill `SkillId::new()` → `"skill"`; project `ProjectId::new()` → `"project"`; phase
   `PhaseId::new()` under `PROJECT_HTUI` → `"step_graph_phase"`. Token first: `(TESTS,
   Some(PROJECT_HTUI), None)` (a demo row) with `expected: None` and a bad glob → `Stale`. Positive
   control last: `(TESTS, Some(PROJECT_HTUI), Some(PHASE_HTUI_IMPLEMENT))`, globs
   `["core:**/*.rs"]`, activation `Glob` → applied.
6. **`skill_binding_stores_expanded_globs_and_languages_as_typed`**: `create_repo(core)`; key
   `(TESTS, Some(PROJECT_HTUI), Some(PHASE_HTUI_IMPLEMENT))`, `Attachment { activation: Glob, globs:
   [" core:src/** ", "", "**/*.rs"], languages: [" Rust ", "rust", "toml"], .. }` → applied `row`:
   `row.globs == ["core:src/**", "**/*.rs", "**/Cargo.toml", "**/*.toml"]`, `row.languages ==
   ["rust", "toml"]`. Re-save `Attachment::of(&row)` with `Some(row.updated_at)` → applied, identical
   `globs` and `languages` (the D80 copy shape).

`tests/mem_store.rs:36-49`: `74` → **`80`**, message appends ", and MOD-9 milestone 3's six for the
skill writers (plan D75-D79)". `htui-store/tests/pg_conformance.rs:19`: `EXPECTED_CASES = 80`.

### 3.6 Postgres (D92). `pg/read.rs` readers, `pg/write.rs` writers

All readers are `pub(crate)` inherent fns next to `step_graph_row`/`repo_rows` (`read.rs:2162-2340`),
each doc'd "`# Errors` Whatever the driver reports, through [`map_sqlx`]". Column order follows
field order (`query_as!` comment, `read.rs:2169-2171`). **New `.sqlx` files: 12** (R1–R3, R5, R6,
W1–W7).

**R1** `skill_rows(&self) -> Result<Vec<Skill>>`:
```sql
SELECT id         AS "id: SkillId",
       name,
       description,
       created_by AS "created_by: UserId",
       created_at,
       updated_at
  FROM skill
 ORDER BY name COLLATE "C"
```
**R2** `skill_row(&self, id: SkillId) -> Result<Option<Skill>>`: the same columns, `FROM skill WHERE
id = $1`, `fetch_optional`.

**R3** `skill_version_rows(&self, skill: SkillId) -> Result<Vec<SkillVersion>>`:
```sql
SELECT skill_id   AS "skill_id: SkillId",
       version,
       body,
       source,
       created_by AS "created_by: UserId",
       created_at
  FROM skill_version
 WHERE skill_id = $1
 ORDER BY version
```
(The head is `rows.last()`; no separate statement.)

**R5** `skill_binding_rows(&self, project: Option<ProjectId>) -> Result<Vec<SkillBinding>>`,
`$1 = project.map(ProjectId::as_uuid)`:
```sql
SELECT id             AS "id: SkillBindingId",
       skill_id       AS "skill_id: SkillId",
       project_id     AS "project_id?: ProjectId",
       phase_id       AS "phase_id: PhaseId",
       pinned_version,
       position,
       activation     AS "activation: Activation",
       globs,
       languages,
       updated_at
  FROM skill_binding
 WHERE project_id IS NOT DISTINCT FROM $1
 ORDER BY skill_id, phase_id NULLS FIRST
```
**R6** `skill_binding_row(&self, key: SkillBindingKey) -> Result<Option<SkillBinding>>`: the same
columns, `WHERE skill_id = $1 AND project_id IS NOT DISTINCT FROM $2 AND phase_id IS NOT DISTINCT
FROM $3`, `fetch_optional`.

**`create_skill`** — one explicit transaction, two statements (D92):
```rust
if let Some(refusal) = new_skill_refusal(&new.name, &new.description, &new.body) {
    return Err(StoreError::Constraint(refusal));
}
let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
let skill = sqlx::query_as!(Skill, W1, …).fetch_one(&mut *tx).await
    .map_err(|err| skill_name_taken(err, &new.name))?;
let version = sqlx::query_as!(SkillVersion, W2, …).fetch_one(&mut *tx).await.map_err(map_sqlx)?;
tx.commit().await.map_err(map_sqlx)?;
Ok((skill, version))
```
**W1**:
```sql
INSERT INTO skill (id, name, description, created_by)
VALUES ($1, $2, $3, $4)
RETURNING id         AS "id: SkillId",
          name,
          description,
          created_by AS "created_by: UserId",
          created_at,
          updated_at
```
**W2**:
```sql
INSERT INTO skill_version (skill_id, version, body, source, created_by)
VALUES ($1, 1, $2, $3, $4)
RETURNING skill_id   AS "skill_id: SkillId",
          version,
          body,
          source,
          created_by AS "created_by: UserId",
          created_at
```
A taken name is `skill_name_key` (23505) → `already_exists("skill", name)`; a taken id
(`skill_pkey`) and an unknown user (`skill_created_by_fkey`, 23503) → `map_sqlx`'s `Constraint`.
The dropped `tx` rolls back.

```rust
/// D92 (F-P): `skill_name_key`'s 23505 in the sentence `MemStore` gives a taken name; anything
/// else through [`map_sqlx`].
fn skill_name_taken(err: sqlx::Error, name: &str) -> StoreError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("skill_name_key") => {
            StoreError::Constraint(already_exists("skill", name))
        }
        _ => map_sqlx(err),
    }
}
```

**`update_skill`**: `skill_patch_refusal` first; on a refusal, `skill_row(id)`: `None` → `NotFound
{ "skill" }`, `Some(r)` with `r.updated_at != expected` → `Stale(r)`, else `Constraint(refusal)`.
Otherwise **W3** (`fetch_optional`, error through `skill_name_taken` when `patch.name` is `Some`);
`None` → `cas_miss(self.skill_row(id).await?, "skill", id)`:
```sql
UPDATE skill SET
    name        = COALESCE($3, name),
    description = COALESCE($4, description)
 WHERE id = $1 AND updated_at = $2
RETURNING id         AS "id: SkillId",
          name,
          description,
          created_by AS "created_by: UserId",
          created_at,
          updated_at
```

**`add_skill_version`**: `skill_body_refusal` first; on a refusal, classify by D89 steps 1–4 with
`skill_version_rows(skill).last()` and `skill_row(skill)`, else `Constraint`. Otherwise **W4**
(`fetch_optional`; `$2 = expected`):
```sql
INSERT INTO skill_version (skill_id, version, body, source, created_by)
SELECT $1, $2::int + 1, $3, $4, $5
 WHERE EXISTS (SELECT 1 FROM skill WHERE id = $1)
   AND (SELECT COALESCE(max(version), 0) FROM skill_version WHERE skill_id = $1) = $2::int
ON CONFLICT (skill_id, version) DO NOTHING
RETURNING skill_id   AS "skill_id: SkillId",
          version,
          body,
          source,
          created_by AS "created_by: UserId",
          created_at
```
Zero rows → D89 steps 1–4 (head read, then skill read). An unknown `created_by` is 23503 →
`Constraint`. Two appends at one head: the second blocks on `skill_version_pkey`, then inserts
nothing (the `append_prompt_template` mechanism, `pg/write.rs:2412-2417`).

**`set_skill_binding`** (D90), reads then one write:
```rust
let current = self.skill_binding_row(key).await?;
if current.as_ref().map(|row| row.updated_at) != expected {
    return Ok(CasOutcome::Stale(current));
}
let skill = self.skill_row(key.skill).await?.ok_or_else(|| not_found("skill", key.skill))?;
let project = match key.project {
    Some(id) => Some(ReadStore::project(self, id).await?.ok_or_else(|| not_found("project", id))?),
    None => None,
};
let phase_project = match key.phase {
    Some(id) => {
        let phase = self.phase_row(id).await?.ok_or_else(|| not_found("step_graph_phase", id))?;
        self.step_graph_row(phase.graph_id).await?.map(|graph| graph.project_id)
    }
    None => None,
};
match change {
    BindingChange::Detach => { /* W7 on current (if any); rows_affected 0 → Stale(re-read R6) */ }
    BindingChange::Attach(attachment) => {
        let versions: Vec<i32> = self.skill_version_rows(key.skill).await?.iter().map(|v| v.version).collect();
        let repos: Vec<String> = match key.project {
            Some(id) => self.repo_rows(id).await?.into_iter().map(|repo| repo.name).collect(),
            None => Vec::new(),
        };
        let stored = check_attachment(&BindingFacts { … }, &attachment).map_err(StoreError::Constraint)?;
        let written = match &current { None => /* W5 */, Some(row) => /* W6 with row.id, row.updated_at */ };
        match written {
            Some(row) => Ok(CasOutcome::Applied(Some(row))),
            None => Ok(CasOutcome::Stale(self.skill_binding_row(key).await?)),
        }
    }
}
```
(`not_found` is a local closure or `StoreError::NotFound { entity, id: id.to_string() }` inline.)

**W5** attach, `$1 = SkillBindingId::new().as_uuid()` (client-minted v7, D92), `$7 =
attachment.activation.as_str()`, `$8 = &stored.globs[..]`, `$9 = &stored.languages[..]`:
```sql
INSERT INTO skill_binding
       (id, skill_id, project_id, phase_id, pinned_version, position, activation, globs, languages)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
ON CONFLICT (skill_id, project_id, phase_id) DO NOTHING
RETURNING id             AS "id: SkillBindingId",
          skill_id       AS "skill_id: SkillId",
          project_id     AS "project_id?: ProjectId",
          phase_id       AS "phase_id: PhaseId",
          pinned_version,
          position,
          activation     AS "activation: Activation",
          globs,
          languages,
          updated_at
```
**W6** change:
```sql
UPDATE skill_binding SET
    pinned_version = $3,
    position       = $4,
    activation     = $5,
    globs          = $6,
    languages      = $7
 WHERE id = $1 AND updated_at = $2
RETURNING (the ten columns of W5)
```
**W7** detach (`query!`, `.execute`, `rows_affected()`):
```sql
DELETE FROM skill_binding WHERE id = $1 AND updated_at = $2
```
Zero-row outcomes: W5 → a concurrent attach won (probed, §0.3) → `Stale(R6)`; W6/W7 → the token
was spent between the read and the write → `Stale(R6)` (`Some` or `None`). A project or phase
deleted between the reads and the write is the FK's 23503 → `Constraint`. Transaction boundaries:
none beyond the single statement; the reads are advisory and the write's own `WHERE` or `ON
CONFLICT` decides, as `update_workspace` (`:1503-1510`).

`WriteStore for PgStore` arms after `append_prompt_template` (`:2479`): `skills` → `skill_rows`,
`skill_versions` → `skill_version_rows`, `skill_bindings` → `skill_binding_rows`, and the four
writers above. Imports (`:22-54`): the model types and the D88 helpers.

### 3.7 `tests/skill_writers.rs` (new, Postgres, `#![cfg(feature = "demo")]`)

The `prompt_template_cas.rs` shape (winner held open by hand, `pg_stat_activity` wait loop, 10 s
bounds). Unchecked `sqlx::query` inserts (no `.sqlx` file).
- **`two_appends_at_one_head_write_one_version`**: winner inserts `skill_version (SKILL_RUST_STYLE,
  3, 'A', '{}', USER)` uncommitted; loser `add_skill_version(SKILL_RUST_STYLE, 2, version("B"))`
  waits on the lock; not answered within 200 ms; commit; the loser answers `Stale(h)` with
  `(h.version, h.body) == (3, "A")`; `skill_versions` → `[1, 2, 3]`, v3 body `A`.
- **`two_attaches_of_one_key_write_one_row`**: winner inserts `skill_binding (id, skill_id,
  project_id, phase_id, position) VALUES ($1, SKILL_TESTS, NULL, NULL, 7)` uncommitted; loser
  `set_skill_binding(key(TESTS, None, None), None, Attach(attachment(Always, 9)))` waits; commit;
  loser answers `Stale(Some(row))` with `row.id` the winner's and `position == 7`;
  `skill_bindings(None)` has exactly that row.

Module doc: why two sessions (`MemStore`'s write lock serialises every call), the probe numbers of
§0.3.

### 3.8 Delegation (`writer.rs`, the two spies)

`writer.rs` after `append_prompt_template` (`:698-707`): seven `match self { Self::Memory(store) =>
store.x(…).await, Self::Online(pg) => pg.x(…).await }` arms. `htui-agent/src/conformance.rs` after
`:933-939` and `htui-agent/tests/recorder.rs` after `:628-634`: seven `self.inner.x(…).await`
lines; imports gain the model types.

### 3.9 `MemStore` unit test (mem.rs `mod tests`)

`a_skill_with_no_version_takes_version_one_at_zero` (D89): `MemStore::from_demo` with one pushed
`Skill` and no version; `add_skill_version(id, 1, …)` → `NotFound { entity: "skill_version", id:
"<id>/v1" }`; at `0` → applied v1. (Only a hand-written row has no version; `PgStore`'s path is the
same SQL with `COALESCE(max, 0)`.)

### 3.10 Gate and commits

```bash
cargo test -p htui-core --all-features -- --test-threads=2
# .sqlx per §0.1, then:
USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=$PG/postgres cargo test -p htui-store --all-features -- --test-threads=2
cargo test -p htui-agent --all-features -- --test-threads=2
cargo build --workspace --all-features --all-targets
cargo clippy -p htui-core -p htui-store -p htui-agent --all-features --all-targets -- -D warnings
git diff --exit-code Cargo.lock
```

Commits: (1) red — trait methods, helpers with `todo!()`, the six cases, pins 80, delegation arms,
`MemStore`/`PgStore` bodies `todo!()`; (2) green `MemStore`; (3) Postgres — `pg/{read,write}.rs`,
`.sqlx` (+12), `skill_writers.rs`.

---

## 4. T3: the clone gap (D80; D95, D96)

**Files** (plan's list; scopes widened by F-G, F-L): `crates/htui-core/src/model/kind.rs`,
`crates/htui-core/src/store/mem.rs` (`create_step_graph` only),
`crates/htui-core/src/store/conformance.rs` (the two constructors and D95's lines in
`step_graph_and_phase_round_trip`), `crates/htui-store/src/pg/write.rs` (`create_step_graph` only),
`crates/htui-store/.sqlx/` (one replaced), `crates/htui-orch/src/graph.rs`,
`crates/htui-orch/src/engine.rs` (D80's deletions, `:4991-4993` doc, `:6021`),
`crates/htui-orch/src/conformance.rs`, `crates/htui-orch/tests/gix_isolator.rs`,
`crates/htui-orch/tests/fixtures.rs`, `crates/htui/src/catalogue.rs`.

**First failing test**: `graph::tests::override_clone_carries_phase_attachments_and_marks_itself`.

### 4.1 `NewStepGraph` (`kind.rs:149-161`)

```rust
    /// `step_graph.is_override` (ANA-2 §4.1): `true` only for `override_graph`'s per-item clone
    /// (MOD-9 D80); every other constructor passes `false`.
    pub is_override: bool,
```
after `description`. The struct doc (`:149-150`) is unchanged; `StepGraphPatch`'s doc (`:162-166`)
becomes "`is_override` is not here: it is set once, at create (`NewStepGraph::is_override`, MOD-9
D80), and never edited."

The nine constructor lines gain `is_override: false,` except `graph.rs:397` (`true`):
`htui-orch/src/conformance.rs:805`, `:865`; `engine.rs:6021`; `tests/gix_isolator.rs:338`;
`tests/fixtures.rs:54`; `htui/src/catalogue.rs:190`; `htui-core/src/store/conformance.rs:3125`,
`:3135`.

### 4.2 Both `create_step_graph`s

`mem.rs:2520`: `is_override: new.is_override,`. `pg/write.rs:2180-2203` (the one replaced `.sqlx`):
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
with `new.is_override` as `$5`.

D95 (F-G), end of `step_graph_and_phase_round_trip`:
```rust
    let marked = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "release-override".to_owned(),
            description: String::new(),
            is_override: true,
        })
        .await
        .expect(CASE);
    assert!(marked.is_override, "{CASE}: create writes is_override");
    assert!(!graph.is_override, "{CASE}: and false stays false");
    let listed = store.step_graphs(ids::PROJECT_HTUI).await.expect(CASE);
    assert!(
        listed.iter().any(|row| row.id == marked.id && row.is_override),
        "{CASE}: is_override reads back"
    );
```
(Placed after every existing assertion, so the name-list assertion at `:3147-3162` is unchanged.)

### 4.3 `override_graph` (`graph.rs:386-441`)

**D96 as decided (F-H, maintainer 2026-09-26): check first, then refuse.** Before
`create_step_graph`, read the source phases' attachments and the project's repos, and refuse if any
qualified glob names a repo the project no longer has, so a refusal writes nothing and leaves no
orphan graph:

```rust
    // D96 (F-H): every source phase attachment must survive `set_skill_binding`'s repo check
    // (D79) before the clone writes anything, so a stale `<repo>:` glob refuses the clone whole.
    let sources: Vec<PhaseId> = resolved.phases.iter().map(|row| row.phase.id).collect();
    let copies: Vec<SkillBinding> = store
        .skill_bindings(Some(item.project_id))
        .await?
        .into_iter()
        .filter(|binding| binding.phase_id.is_some_and(|phase| sources.contains(&phase)))
        .collect();
    if copies.iter().any(|binding| !binding.globs.is_empty()) {
        let repos = store.repos(item.project_id).await?;
        for binding in &copies {
            for glob in &binding.globs {
                let Ok(SkillGlob { repo: Some(repo), .. }) = SkillGlob::parse(glob) else {
                    continue; // bare globs name no repo; a stored glob that no longer parses is
                              // the writer's refusal below, which cannot happen for a stored row
                };
                if !repos.iter().any(|known| known.name == repo) {
                    let slug = store
                        .project(item.project_id)
                        .await?
                        .map_or_else(|| item.project_id.to_string(), |project| project.slug);
                    return Err(ResolveError::Store(StoreError::Constraint(
                        glob_names_unknown_repo(glob, &repo, &slug),
                    )));
                }
            }
        }
    }

    let clone = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            project_id: item.project_id,
            name: format!("{}-override", item.key),
            description: format!(
                "Per-item override of `{}` for {}",
                resolved.graph.name, item.key
            ),
            is_override: true,
        })
        .await?;

    // Old phase id → cloned phase id, in the source's order (D80).
    let mut cloned: Vec<(PhaseId, PhaseId)> = Vec::with_capacity(resolved.phases.len());
    for row in &resolved.phases {
        let id = PhaseId::new();
        store
            .create_phase(&StepGraphPhase {
                id,
                graph_id: clone.id,
                ..row.phase.clone()
            })
            .await?;
        cloned.push((row.phase.id, id));
    }

    // Every phase-level attachment of a source phase, onto its clone (D80). Project and global
    // rows apply to the clone already; a copy of them would be a second row at the same level.
    for binding in &copies {
        let Some(source) = binding.phase_id else { continue };
        let Some(&(_, phase)) = cloned.iter().find(|(old, _)| *old == source) else { continue };
        let key = SkillBindingKey {
            skill: binding.skill_id,
            project: Some(item.project_id),
            phase: Some(phase),
        };
        let copy = BindingChange::Attach(Attachment::of(binding));
        if let CasOutcome::Stale(_) = store.set_skill_binding(key, None, copy).await? {
            // A fresh phase id has no row; a `Stale` here is a store bug, said rather than hidden.
            return Err(ResolveError::Store(StoreError::Constraint(format!(
                "a skill attachment already sits on the cloned phase {phase}"
            ))));
        }
    }
    // (the item repoint follows, unchanged: `:416-440`)
```
The stale-repo case is refused before any write (**D96**, F-H — maintainer: check first); any other
`Constraint` from the copy still propagates. Imports (`graph.rs:15-21`): `Attachment, BindingChange,
SkillBinding, SkillBindingKey, SkillGlob`, `CasOutcome`, `htui_core::store::glob_names_unknown_repo`.
Test (§4.5): `override_clone_refuses_a_stale_repo_glob_before_writing` — a source phase row whose
globs carry `gone:**/*.rs` (planted directly in `MemStore` state, since the writer would refuse it)
makes `override_graph` return `Constraint(glob_names_unknown_repo("gone:**/*.rs", "gone", slug))`,
and `step_graphs(project)` is unchanged (no `-override` graph).

### 4.4 Docs and deletions

- `graph.rs:6-7` (module doc) and `:365-380`: the clone is deep over `step_graph_phase` **and** the
  source phases' attachments (ANA-22 §2, D80); `is_override` is written; `phase_agent` and
  re-override still owed (keep those two sentences). The old "would double every project binding"
  rationale goes (the plan's "Where … disagree").
- `engine.rs:85-88` (`OVERRIDE_SKILLS_NOTE`), `:5017-5019` (the push), `:12059-12075` (doc + test
  `an_override_graph_notes_the_clone_gap`) deleted. `:4991-4993` doc: "…and a phase renamed or
  deleted since the snapshot gets a note rather than a silent loss of its phase-level attachments
  (plan R-15). An override graph's phases carry their own copies (MOD-9 D80)."

### 4.5 Test (`graph.rs`, replaces `override_clone_leaves_bindings_alone` `:1438-1500`)

**`override_clone_carries_phase_attachments_and_marks_itself`** (`MemStore::demo()`, `feat_1`):
`source_rows = skill_bindings(Some(PROJECT_HTUI))` and `bound_before = bound_skills(PROJECT_HTUI,
Some(PHASE_HTUI_IMPLEMENT))` before; clone; the clone's `is_override` is true and `step_graphs`
reads it back so; phases copied as today (name, input_kinds, gate, gate_hard, template_name; fresh
ids); the clone's `implement` has one row `(RUST_STYLE, Some(PROJECT_HTUI), Some(cloned
implement))` with `pinned_version == Some(1)`, `position == 2`, `activation == Always`, empty
`globs`/`languages`; no other cloned phase has a row; `bound_skills(PROJECT_HTUI, Some(cloned
implement)) == bound_before`; the source rows are untouched (`skill_bindings(Some(PROJECT_HTUI))`
minus the new row equals `source_rows`, ids and `updated_at` included).

### 4.6 Gate and commits

```bash
cargo test -p htui-core --all-features -- --test-threads=2
# .sqlx per §0.1 (one file replaced; count stays 276)
USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=$PG/postgres cargo test -p htui-store --all-features -- --test-threads=2
cargo test -p htui-orch --all-features -- --test-threads=2
cargo build --workspace --all-features --all-targets
```
Commits: (1) red — the field, nine constructors, the new test, D95's lines; (2) green — both stores,
`override_graph`, deletions, docs, `.sqlx`.

---

## 5. T4: store worker (D81; D97)

**Files**: `crates/htui/src/skills.rs` (new), `crates/htui/src/store_worker.rs`,
`crates/htui/src/lib.rs` (`pub mod skills;` after `:28`, alphabetical before `store_worker`).

**First failing test**: `skills::tests::request_names_match_the_name_arms`.

### 5.1 `crate::skills` types

```rust
//! The Skills view's store module (MOD-9 milestone 3, D81): one snapshot read per scope and the
//! four compare-and-set writes, `crate::templates`' shape. The worker fills `created_by` from
//! `Backend::this_user`; the view holds no `UserId` (`R-NF-3`). Known residue, as in
//! `crate::templates`: a re-read that fails after an applied write answers `Failed`.

use chrono::{DateTime, Utc};
use htui_core::model::{
    BindingChange, NewSkill, NewSkillVersion, ProjectId, Scope, Skill, SkillBinding,
    SkillBindingKey, SkillId, SkillPatch, SkillVersion, StepGraph, StepGraphPhase,
};
use htui_core::store::{CasOutcome, Result, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE, PROMPT_ON_SERVER_ONLY, Writer};

use crate::store_worker::{StoreReply, StoreRequest};

/// The whole Skills view in one read (D81). `PartialEq` only: `StepGraph` has no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillsSnapshot {
    /// The library, `name` byte order, each with every version.
    pub skills: Vec<SkillEntry>,
    /// The global attachments (`skill_bindings(None)`).
    pub global: Vec<SkillBinding>,
    /// One entry per id of `scope.project_ids`, in that order.
    pub projects: Vec<ProjectSkills>,
}

/// One skill and its versions, ascending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    /// The row.
    pub skill: Skill,
    /// Every version (R-27: the library is small).
    pub versions: Vec<SkillVersion>,
}

/// One scope project's side of the pane.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSkills {
    /// Which project.
    pub project: ProjectId,
    /// Its project and phase attachments (D91 order).
    pub bindings: Vec<SkillBinding>,
    /// Its graphs in name byte order, **override graphs excluded**, each with its phases by
    /// position.
    pub graphs: Vec<(StepGraph, Vec<StepGraphPhase>)>,
    /// Its repo names, byte order (D79's picker and `?` marks).
    pub repos: Vec<String>,
}

impl SkillsSnapshot {
    /// One skill's entry. `#[must_use]`.
    pub fn entry(&self, skill: SkillId) -> Option<&SkillEntry>;
    /// The skill named `name`.
    pub fn by_name(&self, name: &str) -> Option<&SkillEntry>;
    /// A skill's highest version (the CAS token of a save).
    pub fn head(&self, skill: SkillId) -> Option<&SkillVersion>;
    /// One version.
    pub fn version(&self, skill: SkillId, version: i32) -> Option<&SkillVersion>;
    /// A project's entry.
    pub fn project(&self, project: ProjectId) -> Option<&ProjectSkills>;
    /// The row at `key`, from `global` or the project's `bindings`.
    pub fn binding(&self, key: SkillBindingKey) -> Option<&SkillBinding>;
}

/// Which write a `SkillsStale` answers (D81), so the view knows which draft keeps its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleWhat {
    /// `EditSkill`: the row changed, or is gone.
    Skill(SkillId),
    /// `SaveSkillVersion`: the head moved, or the skill is gone.
    Version(SkillId),
    /// `SetSkillBinding`: the row at the key changed, appeared, went, or its skill/project/phase
    /// is gone.
    Binding(SkillBindingKey),
}

/// The five request names, in `StoreRequest` order; `StoreRequest::name`'s arms spell the same.
pub const REQUEST_NAMES: [&str; 5] =
    ["skills", "create_skill", "edit_skill", "save_skill_version", "set_skill_binding"];

/// The read's name: a refused read leaves the view with no library.
pub const READ_NAME: &str = REQUEST_NAMES[0];
```

`snapshot(writer: &Writer, scope: &Scope) -> Result<SkillsSnapshot>` reads in this order (D97; the
milestone 2 review-finding-5 rule: attachments before versions, so no pin can name a version the
read missed): `skill_bindings(None)`; per scope project `skill_bindings(Some(p))`,
`step_graphs(p)` filtered `!is_override`, `phases(g)` per graph, `repos(p)` names; then `skills()`
and `skill_versions(s)` per skill.

### 5.2 `StoreRequest` / `StoreReply` (`store_worker.rs`)

After `SaveTemplate` (`:529-544`), reusing `crate::templates::TemplateBody` for bodies (its `Debug`
prints the length only, `templates.rs:30-57`) (D97):

```rust
    /// The Skills view (MOD-9 D81): library, attachments, graphs, repo names.
    Skills(Scope),
    /// `create_skill`: the row and v1 together (D75, D77). The worker fills `created_by`.
    CreateSkill {
        /// The scope the reply re-reads.
        scope: Scope,
        /// `skill.name`.
        name: String,
        /// `skill.description`.
        description: String,
        /// Version 1's body; its `Debug` is its length.
        body: TemplateBody,
    },
    /// `update_skill` under CAS on `skill.updated_at` (D76).
    EditSkill {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Which skill.
        skill: SkillId,
        /// `skill.updated_at` the form opened on.
        expected: DateTime<Utc>,
        /// What changed.
        patch: SkillPatch,
    },
    /// `add_skill_version`: append `expected + 1` iff `expected` is the head.
    SaveSkillVersion {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Which skill.
        skill: SkillId,
        /// The head version the editor opened on.
        expected: i32,
        /// The new body; its `Debug` is its length.
        body: TemplateBody,
    },
    /// `set_skill_binding` (D78).
    SetSkillBinding {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The attachment's key.
        key: SkillBindingKey,
        /// The row's `updated_at` the form opened on; `None` for "no row".
        expected: Option<DateTime<Utc>>,
        /// Attach or detach.
        change: BindingChange,
    },
```

`name()` (`:645-647` block) gains, after the templates pair:
```rust
            // The five of `skills::REQUEST_NAMES`, in that order (MOD-9 D81).
            Self::Skills(..) => "skills",
            Self::CreateSkill { .. } => "create_skill",
            Self::EditSkill { .. } => "edit_skill",
            Self::SaveSkillVersion { .. } => "save_skill_version",
            Self::SetSkillBinding { .. } => "set_skill_binding",
```
`StoreReply` after `TemplatesStale` (`:812-814`):
```rust
    /// The Skills view's snapshot, freshly read: the answer to `Skills` and to every skill write
    /// that applied (MOD-9 D81).
    Skills(Box<SkillsSnapshot>),
    /// A skill write missed its token, or its skill, project or phase is gone: the snapshot as it
    /// is now and which write it answers. The draft keeps its text.
    SkillsStale {
        /// The snapshot as it is now.
        snapshot: Box<SkillsSnapshot>,
        /// Which write went stale.
        what: StaleWhat,
    },
```
`try_serve` (`:1108-1112`), after the templates arm:
```rust
        // The five skill requests, or-ed for the reason the arms above are (MOD-15 M3 plan F-12).
        StoreRequest::Skills(..)
        | StoreRequest::CreateSkill { .. }
        | StoreRequest::EditSkill { .. }
        | StoreRequest::SaveSkillVersion { .. }
        | StoreRequest::SetSkillBinding { .. } => skills::serve(backend, request).await?,
```
Import at `:45`: `use crate::skills::{self, SkillsSnapshot, StaleWhat};`.

### 5.3 `serve` and the reply mapping (D97)

Read: `backend.writer().ok_or_else(|| StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))`
— the prompt path's offline sentence, as the Templates read (`templates.rs:477-495`). Writes:
`DATABASE_UNREACHABLE` (`templates.rs:152-154`), `created_by = backend.this_user().await?` for the
two that write a body. Every write re-reads with `snapshot(&writer, scope)` (the `cas` shape).

| Request | `Ok(Applied)` / `Ok(value)` | `Ok(Stale(_))` | `Err(NotFound { entity, .. })` | other `Err` | offline |
|---|---|---|---|---|---|
| `Skills` | `Skills(snap)` | — | — | `Failed { "skills" }` | `Failed`, `PROMPT_ON_SERVER_ONLY` |
| `CreateSkill` | `Skills` | — | `Failed` | `Failed { "create_skill" }` (name, body, taken) | `Failed`, `DATABASE_UNREACHABLE` |
| `EditSkill` | `Skills` | `SkillsStale { Skill(id) }` | `"skill"` → `SkillsStale { Skill(id) }` | `Failed { "edit_skill" }` | as above |
| `SaveSkillVersion` | `Skills` | `SkillsStale { Version(id) }` | `"skill"`, `"skill_version"` → `SkillsStale { Version(id) }` | `Failed { "save_skill_version" }` | as above |
| `SetSkillBinding` | `Skills` | `SkillsStale { Binding(key) }` | `"skill"`, `"project"`, `"step_graph_phase"` → `SkillsStale { Binding(key) }` | `Failed { "set_skill_binding" }` | as above |

`Failed` is produced by `store_worker::serve` from the `Err` (`:1014-1018`); `serve` itself returns
`Err`. A request not of the five → `Err(Backend("not a skills request: {name}"))`.

### 5.4 Tests (`skills.rs` `mod tests`, over `Backend::memory`)

- **`the_snapshot_carries_the_library_the_attachments_and_the_scope_s_graphs`**: store =
  `MemStore::from_demo(data)` with one pushed `StepGraph { id: StepGraphId::new(), project_id:
  PROJECT_HTUI, name: "FEAT-1-override", is_override: true, .. }` (F-K), then
  `create_repo(NewRepo { name: "core", project_id: PROJECT_HTUI, .. })`. Platform scope (htui, agy):
  skills `["rust-style", "tests"]` with versions `[[1, 2], [1]]`; `global` empty; htui `bindings` the
  three demo rows in D91 order; agy none; htui graph names `["analysis", "bug", "feature",
  "refactor", "tooling"]` (the override excluded) with 15 phases in all; htui `repos == ["core"]`,
  agy `[]`.
- **`each_write_answers_skills_when_it_applies`**: `CreateSkill("docs-style")` → `Skills` holding
  it at v1; `EditSkill` (fresh token, description) → `Skills`; `SaveSkillVersion(expected 1)` →
  `Skills` with v2; `SetSkillBinding(global, None, Attach)` → `Skills` whose `global` holds it.
- **`a_spent_token_answers_skills_stale_with_what_went_stale`**: `EditSkill(rust-style, token −1 s)`
  → `SkillsStale { what: Skill(id) }`; `SaveSkillVersion(rust-style, 1)` → `Version(id)`;
  `SetSkillBinding(the demo phase key, None, Attach)` → `Binding(key)`; `EditSkill(SkillId::new())`
  → `Skill(_)`; `SetSkillBinding` on `PhaseId::new()` under htui → `Binding(_)`.
- **`a_refused_write_answers_failed_with_the_store_sentence`**: `CreateSkill("Bad")` through
  `store_worker::serve` → `Failed { request: "create_skill", message }` containing `skill.name`;
  the store unchanged.
- **`an_offline_read_is_refused_with_the_server_only_sentence`** and
  **`an_offline_write_is_refused_with_the_unreachable_sentence`** (the `templates.rs:437-495` pair,
  `Backend::Offline` over a throwaway `CacheStore`).
- **`request_names_match_the_name_arms`**: the five samples' `name()` == `REQUEST_NAMES`;
  `READ_NAME == REQUEST_NAMES[0]`.
- **`a_skill_body_request_debug_prints_its_length_not_its_text`** (the `templates.rs:275-296`
  shape, for `SaveSkillVersion`).

Gate: `cargo test -p htui --all-features --lib skills`, then the htui gate.

---

## 6. T5: the Skills view (D82–D84; D98–D104)

**Files**: `crates/htui/src/ui/tabs/skills/mod.rs`, `…/skills/library.rs` (new),
`…/skills/attach.rs` (new), `crates/htui/tests/skills.rs` (new), `crates/htui/tests/skills_pg.rs`
(new), `crates/htui/tests/snapshots/skills__*.snap` (seven new). No `text_field.rs` change (D101).

**First failing test**: `tests/skills.rs` `the_library_lists_the_skills_with_body_and_estimate`.

### 6.1 Tab shell (`skills/mod.rs`, D84, D100)

- `mod library; mod attach;` `use library::LibraryView;`. `SkillsTab { view, templates, library:
  LibraryView }`. `SKILLS_LATER` and its `Paragraph` go; module doc `:1-6` says the Skills view is
  milestone 3's library and attachments pane.
- `wants_requests` → `vec![StoreRequest::Templates(scope.clone()), StoreRequest::Skills(scope.clone())]`.
- `on_scope_change` → both views.
- `on_key`: `captured = match self.view { Templates => self.templates.captures_input(), Skills =>
  self.library.captures_input() }`; captured → that view's `on_key`; else `h`/`l`/`[`/`]`/arrows
  toggle; else the shown view's `on_key`.
- `on_reply` → both views. `on_external_edit` → `self.templates.on_external_edit(outcome.clone(),
  ctx); self.library.on_external_edit(outcome, ctx);` (each ignores without its own pending).
- `render` → `View::Skills => self.library.render(frame, body, ctx)`.

### 6.2 `LibraryView` state (`library.rs`)

```rust
/// The Skills view: the library, one skill's body or diff, the editor, and the attachments pane.
/// Holds no store handle and no `UserId` (`R-NF-3`); renders from the last `SkillsSnapshot`.
#[derive(Debug, Default)]
pub(super) struct LibraryView {
    snapshot: Option<SkillsSnapshot>,
    unavailable: Option<String>,        // after a refused `READ_NAME`
    cursor: usize,                      // index into snapshot.skills
    shown: Option<i32>,                 // version shown; None = head
    base: Option<i32>,                  // diff base; None = shown − 1
    pane: Pane,                         // Body | Diff
    scroll: Scroll,                     // backlog::detail::Scroll
    pane_rows: Cell<usize>,
    mode: Mode,
    attach: Option<AttachPane>,         // Some while the pane is open (D83)
    busy: Option<&'static str>,         // one write in flight
    sent: Option<Sent>,                 // what it carries (D98)
    notice: Option<Notice>,
    external: Option<Pending>,          // $EDITOR handoff, templates.rs:266-275 shape
    page: Cell<u16>,
}

#[derive(Debug, Default)]
enum Mode {
    #[default] Browse,
    /// `n`, step 1.
    Naming { field: TextField },
    /// `n`, step 2 (may be empty).
    Describing { name: String, field: TextField },
    /// `i`: rename / re-describe.
    Info(InfoForm),
    /// `e`, `n`'s step 3, an `$EDITOR` return.
    Editing(Editor),
}

#[derive(Debug)]
struct InfoForm { skill: SkillId, token: DateTime<Utc>, name: TextField, description: TextField, focus: usize }

/// Never `Debug`s a body (custom `Debug`: lengths).
struct Editor { target: Target, token: i32, from: Option<i32>, area: TextArea, original: String, esc_armed: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target { New { name: String, description: String }, Version { skill: SkillId, name: String } }

/// The write in flight (D98). Custom `Debug`: body lengths only.
pub(super) enum Sent {
    Create { name: String, body: String },
    Rename { skill: SkillId, token: DateTime<Utc>, patch: SkillPatch },
    Version { skill: SkillId, token: i32, body: String },
    Binding { key: SkillBindingKey, token: Option<DateTime<Utc>>, change: BindingChange, label: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Notice { Info(String), Error(String) }

pub(super) fn captures_input(&self) -> bool {
    !matches!(self.mode, Mode::Browse) || self.attach.as_ref().is_some_and(AttachPane::captures_input)
}
```

### 6.3 Library keys

Browse (plain keys only, `templates.rs:285-288` `plain`; `h`/`l` are the tab's):

| Key | Action |
|---|---|
| `j` / `Down`, `k` / `Up` | next / previous skill; `shown`, `base`, `pane`, `scroll`, `notice` reset |
| `,` / `.` | older / newer version shown (clamped) |
| `b` | `base = shown`; notice `base v{n}` |
| `d` | toggle Body/Diff; no base and no predecessor → notice `v{n} has no earlier version` |
| `e` | `Editing { Version, token: head, from: shown, text: shown body }` |
| `E` | `Action::EditExternally(ExternalEdit { text: shown body, stem: name })`, `Pending { editor, resume: false }` |
| `n` | `Naming` |
| `i` | `Info` on the selected skill (`token = skill.updated_at`), name/description prefilled |
| `a` | `attach = Some(AttachPane::new(skill))` |
| `r` | `notice = None`; `ctx.request(Skills(scope))` |
| `J`/`K`/`PgDn`/`PgUp` | `scroll.on_key` |
| anything else | `Pass` |

With no skill selected (empty library) `,` `.` `b` `d` `e` `E` `i` `a` do nothing but notice
`select a skill` when the list is non-empty.

- **Naming** (`TextField`): `Enter` → `!validate_name` → `Error(invalid_skill_name(name))`, field
  kept; name in snapshot → `Error("`{name}` exists \u{2014} select it and press e")`; else
  `Describing`. `Esc` → Browse.
- **Describing**: `Enter` → `Editing { New { name, description }, token: 0, from: None, text: "" }`;
  `Esc` → Browse.
- **Info**: `Tab`/`BackTab`/`Up`/`Down` move focus; `Enter` or `Ctrl+S` save; `Esc` → Browse. Save:
  busy → `in_flight`; `!validate_name(name)` → `Error(invalid_skill_name)`; `patch` holds only the
  changed fields; empty patch → `Info("no changes")`; else send `EditSkill { expected: token, patch }`.
- **Editing** (`TextArea`): `Ctrl+S` save; `Ctrl+E` hand-off (`resume: true`); `Esc`: busy →
  `in_flight`, unchanged or armed → Browse, else arm + `UNSAVED`; `Tab`/`BackTab` pass (the shell
  switches tabs, draft kept). Save, first match wins: busy → `in_flight`; `skill_body_refusal(text)`
  → `Error(sentence)` (blank → `a skill needs text`), nothing sent; `Version` target and text ==
  head body → `Info("v{head} already says this \u{2014} nothing to save")` (D77: the view prevents
  it); else send `CreateSkill` (New) or `SaveSkillVersion { expected: token }`.
- `$EDITOR` return (`on_external_edit`): `Edited(text)` opens the editor on it with notice
  `edited in $EDITOR \u{2014} Ctrl+S saves` (blank → `Error(BLANK_SKILL_BODY)`); `Unchanged` /
  `Failed` as `templates.rs:405-419`.

Sending (all writes, D98): `busy = Some(name)`, `sent = Some(…)`, `notice = Info("saving\u{2026}")`,
`ctx.request(…)`.

### 6.4 Replies and the "landed" predicates (D98)

`in_scope(snapshot, ctx)`: `snapshot.projects[*].project` equals `ctx.scope.project_ids` (as
`templates.rs:1082-1088`); out of scope → ignored.

- `Skills(s)`: store it, `unavailable = None`, clamp cursor, then if `busy` is set and `sent` has
  **landed** in `s`: clear `busy`/`sent`, close the draft, notice as below. Not landed → the draft
  stays (a read served before the write, blueprint milestone 1 D27's reason).

| `Sent` | landed iff | on landing |
|---|---|---|
| `Create { name, body }` | `s.by_name(name)` has v1 with `body` | cursor onto it; Browse; `created \`{name}\` v1 \u{b7} ~{T} tokens` |
| `Version { skill, token, body }` | `s.version(skill, token + 1)` has `body` | `saved v{token+1} \u{b7} ~{T} tokens`; draft typed since → editor stays, token moves (`templates.rs:878-891`) |
| `Rename { skill, token, patch }` | `entry(skill).skill.updated_at != token` and each `Some` field of `patch` equals the row's | Browse; `saved \`{name}\`` |
| `Binding { key, change: Attach(a), token, .. }` | `s.binding(key)` is `Some(row)` with `row.updated_at != token`, `(row.pinned_version, row.position, row.activation) == (a.pinned_version, a.position, a.activation)` and `row.languages == normalise(a.languages)` | pane back to Browse; `attached to {label}` |
| `Binding { key, change: Detach, .. }` | `s.binding(key)` is `None` | pane Browse; `detached from {label}` |

- `SkillsStale { snapshot, what }` (in scope): store the snapshot; if `busy`: clear it and `sent`, and

| `what` | draft | notice (`Notice::Error`) |
|---|---|---|
| `Version(id)`, head `h` exists | editor kept, `token = h` | `saved elsewhere since you opened it \u{2014} v{h} is now the latest; your draft is kept and Ctrl+S saves it as v{h+1}` (`templates.rs:96-104`'s sentence) |
| `Skill(id)`, row exists | Info form kept, `token = row.updated_at` | `changed elsewhere since you opened it \u{2014} your text is kept; Ctrl+S saves it over the new row` |
| `Skill(id)` / `Version(id)`, gone | Browse | `the skill is gone` |
| `Binding(key)` | form kept, `token = s.binding(key).map(updated_at)` | `this attachment changed elsewhere \u{2014} your form is kept; Ctrl+S saves over it` |

- `Failed { request == READ_NAME }` → `unavailable = Some(message)`. `Failed` of the other four →
  `busy`/`sent` cleared, draft kept, `Notice::Error(message)` (the store's sentence, e.g. D79's).

### 6.5 Library render (100 × 30)

Content split as templates (`templates.rs:436-477`): `[content Min(1), notice Length(1..=2), hint
Length(1)]`; content = `[list Length(LIST_WIDTH = 32), pane Min(1)]`, or the whole content for the
attachments pane / the editor.
- List block ` Skills `; one row per skill, `format!("  {name:<22} v{head:<3}")`, a name over 22
  chars cut to 21 + `…`; selected row `theme.selected`; cursor-follow offset as `:1000-1005`.
- Pane, Body: title `format!(" {name} v{shown} (head v{head}) \u{b7} ~{T} tokens ")`, lines: the
  description in `theme.dim` and a blank line when non-empty, then the body lines; wrap + scroll as
  templates. Diff: title `format!(" diff v{base} \u{2192} v{shown} ")`, lines
  `diff::lines(&diff::unified(base_body, shown_body, "{name} v{base}", "{name} v{shown}"), theme)`.
- Naming / Describing: the pane shows `new skill: ` / `description of {name}: ` and the field
  (`templates.rs:1011-1018`).
- Info: pane title ` {name} \u{b7} rename `; two lines `name         {field}` and `description  {field}`.
- Editing: the whole content: one block titled `format!(" {name} \u{b7} editing from v{from}, saves
  v{token+1} \u{b7} ~{T} tokens ")` (`New`: `" {name} \u{b7} new, saves v1 \u{b7} ~{T} tokens "`),
  `T` recomputed from the draft on every draw.
- `T` (D99, F-I): `estimate(name, version, body) = render::skills(&[BoundSkill { skill_id:
  SkillId::default(), name, version: Some(version), position: 0, body, level: SkillLevel::Global,
  activation: Activation::Always, globs: vec![] }]).map_or(0, |r| TokenEstimator::DEFAULT
  .estimate(&r.content))`.
- Hints (`theme.dim`, one leading space; all ≤ 98 cols):
  - Browse: `j/k  ,/. version  b base  d diff  e edit  E $EDITOR  n new  i info  a attach  r reload  h/l view` (96)
  - Naming/Describing: `Enter next  Esc cancel`
  - Info: `Tab field  Ctrl+S save  Esc cancel`
  - Editing: `Ctrl+S save  Ctrl+E $EDITOR  Esc cancel  L{line}:C{col}` (as `templates.rs:456-459`)
- Fixed strings: `skills not read yet`, `skills unavailable: {why}`, `select a skill`,
  `unsaved changes \u{2014} Esc again discards`, `` `{busy}` is still in flight ``, `no changes`,
  the wait-flag suffix of `templates.rs:66-67`.

### 6.6 `AttachPane` (`attach.rs`, D83, D101–D104)

```rust
/// The attachments of one skill (D83): a global row, then per scope project its row and its
/// non-override graphs' phases. Holds no snapshot; every method takes the view's.
#[derive(Debug)]
pub(super) struct AttachPane {
    skill: SkillId,
    cursor: usize,          // index into rows(snapshot)
    mode: AttachMode,
}

#[derive(Debug, Default)]
enum AttachMode {
    #[default] Browse,
    Form(Form),
    Picker { form: Form, cursor: usize },
    ConfirmDetach { row: ARow, token: DateTime<Utc> },
}

/// Indices into the snapshot, the `settings/kinds.rs:101-134` precedent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ARow { Global, Project { p: usize }, Phase { p: usize, g: usize, i: usize } }

#[derive(Debug)]
struct Form {
    row: ARow,
    key: SkillBindingKey,
    token: Option<DateTime<Utc>>,
    activation: Activation,
    pin: TextField,        // "latest" (or empty) or a version number
    position: TextField,
    languages: TextField,  // comma list, split_list
    globs: TextField,      // comma list, split_list (D93)
    focus: FormField,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum FormField { #[default] Activation, Pin, Position, Languages, Globs }

/// What a key did, for the view to act on (busy, notice, send).
pub(super) enum AttachOutcome { Consumed, Pass, Close, Notice(Notice), Save { request: StoreRequest, sent: Sent } }

pub(super) fn captures_input(&self) -> bool { !matches!(self.mode, AttachMode::Browse) }
```

**Rows** (`rows(snapshot)`): `Global`; then for each `p` in `snapshot.projects`: `Project { p }`,
then for each `(g, (graph, phases))` in `graphs` (name order) each `i` (position order):
`Phase { p, g, i }`.

**Row text**: `format!("{star} {label:<28} {summary}")`, `label` cut to 27 + `…`:
- label: `global`; the project's slug (`ctx.projects`, else the id's first 8 chars, as
  `templates.rs:1091-1102`); `format!("  {graph} \u{203a} {phase}")`.
- summary: no row → `\u{2014}`; else parts joined by ` \u{b7} `: `activation.as_str()`; `latest` or
  `v{n}`; `pos {p}`; when `globs` is non-empty `{n} globs` plus ` ?` if any qualified glob names a
  repo not in the project's `repos` (D79's rename case); when `activation == Glob`, `fires from
  milestone 5` (R-28).
- star (**D104**): for each project `P` and each of `P`'s rows' phases `X` (none → `phase: None`),
  `resolve(rows_for(P, X), versions)` where `rows_for` is this skill's global rows plus `P`'s rows
  with `phase_id` `None` or `X`, each paired with the skill name, and `versions` the skill's; the
  one candidate's `level` stars `Global` (the global row), `Project` (`P`'s row) or `Phase` (`X`'s
  row). An `Off` winner stars too (it is what the run records).

**Keys**

| Mode | Key | Action |
|---|---|---|
| Browse | `j`/`Down`, `k`/`Up` | move (clamped) |
| | `Enter` | `Form` for the row: existing row → activation, pin (`latest` or `n`), position, `languages` joined `", "`, `globs` = stored globs **minus** `expand(stored languages)` joined `", "` (**D102**), `token = Some(updated_at)`; none → `Always`, `latest`, `0`, empty, empty, `token = None` |
| | `x` | row present → `ConfirmDetach`, `Notice(Info("detach `{skill}` from {label}? y detaches, any other key keeps it"))`; none → `Notice(Info("nothing attached here"))` |
| | `r` | `ctx.request(Skills(scope))` |
| | `Esc`, `a` | `Close` |
| | other | `Pass` (so `h`/`l` still toggle views; the pane stays open) |
| Form | `Tab`/`Down` / `BackTab`/`Up` | next / previous field |
| | `Space` on Activation | cycle `Always → Glob → Off → Always` |
| | other plain chars on Activation | consumed, ignored |
| | `Ctrl+R` | Global row → `Notice(Error("a global attachment's globs cannot name a repo"))`; project has no repos → `Notice(Error("project `{slug}` has no repos"))`; else `Picker { form, cursor: 0 }` |
| | `Ctrl+S`, `Enter` | save (below) (**D103**) |
| | `Esc` | Browse |
| | other `CONTROL` chords | `Pass` |
| | text keys | the focused `TextField` |
| Picker | `j`/`k` | move |
| | `Enter` | feed `format!("{repo}:")` char by char to `form.globs` as `KeyEvent::from(KeyCode::Char(c))` (**D101**), `focus = Globs`, back to `Form` |
| | `Esc` | back to `Form` |
| ConfirmDetach | `y` | `Save { SetSkillBinding { expected: Some(token), change: Detach }, Sent::Binding { .. } }`, back to Browse |
| | `CONTROL` chord | `Pass` |
| | anything else | Browse, `Notice(Info("kept"))` (modal, `kinds.rs:815-819`) |

**Form save**: pin: empty or `latest` → `None`, a positive integer → `Some(n)`, else
`Error("pin is `latest` or a version number")`; position: empty → 0, an integer ≥ 0, else
`Error("position is a whole number, 0 or more")`; then `canonical_globs(split_list(globs),
split_list(languages))` → `Err(e)` → `Error(e.to_string())`; else `Save { SetSkillBinding { scope,
key, expected: token, change: Attach(Attachment { pinned_version, position, activation, globs:
split_list(globs), languages: split_list(languages) }) }, Sent::Binding { .. } }`. The store
re-checks everything (D78, D79).

**Render**: Browse → the content is one block titled ` attachments \u{b7} {skill} ` holding the
rows (cursor-follow offset; `theme.selected` on the cursor). Form/Picker → `[rows Min(3), panel
Length(9)]`. Form panel title ` attach {skill} \u{b7} {label} ` (`token: None`) or ` edit {skill}
\u{b7} {label} `; seven lines, label column 12:
```
activation  always  (Space cycles)
pin         latest
position    0
languages   rust
globs       src/**
effective:  src/**, **/*.rs, **/Cargo.toml
a bare glob matches in every repo of the project; <repo>:<glob> matches in that repo only
```
`effective:` recomputed on every draw: `Ok(v)` → `v.join(", ")` or `none`; `Err(e)` →
`theme.error` `e.to_string()`; then, on the global row, the first qualified glob →
`global_glob_names_a_repo`; on a project/phase row, the first qualifier not in `repos` →
`glob_names_unknown_repo(glob, repo, slug)` (the store's sentences, shown before save). Picker
panel title ` repos of {slug} `, one repo per line, cursor `theme.selected`. Hints: Browse `j/k
move  Enter edit  x detach  r reload  Esc back`; Form `Tab/Up/Down field  Space activation  Ctrl+R
repo  Ctrl+S save  Esc cancel`; Picker `j/k move  Enter insert  Esc back`; ConfirmDetach `y detach
any other key keeps it`.

The view, on `AttachOutcome::Save`: busy → `Notice(Error(in_flight))`, nothing sent; else the §6.3
send. On `Close` → `attach = None`. On a landed/Stale binding reply the view calls
`attach.on_landed()` (Form/Confirm → Browse) or `attach.on_stale(snapshot)` (`form.token =
snapshot.binding(key).map(|r| r.updated_at)`).

### 6.7 Tests (`tests/skills.rs`, `#![cfg(feature = "testkit")]`)

Helpers copied from `tests/templates.rs:24-94` (`open_over`, `type_text`, `hint`, `notice`) with
`open_over` pressing `2` only (the Skills view is the default); `open_platform_over(store)`: settle,
`w`, settle, `j`, `enter`, settle, `2`, settle (F-O); `select(harness, name)`: `k` × 20, then `j`
until the frame contains `format!("\u{250c} {name} v")`.

| Test | Setup and keys | Asserts |
|---|---|---|
| `the_library_lists_the_skills_with_body_and_estimate` → **`skills__library`** | `open()`; `select("rust-style")` | frame contains `rust-style v2 (head v2) \u{b7} ~42 tokens`; snapshot |
| `the_editor_opens_on_the_shown_version` → **`skills__edit`** | `select("rust-style")`, `e`, `Z` | hint contains `Ctrl+S save`; title `editing from v2, saves v3`; snapshot |
| `any_two_versions_diff` → **`skills__diff_two_versions`** | `select("rust-style")`, `,`, `b`, `.`, `d` | title `diff v1 \u{2192} v2`; a `+` line with `One error enum per crate.`; snapshot |
| `the_attachments_pane_lists_global_projects_and_phases` → **`skills__attachments`** | platform; `select("rust-style")`, `a` | rows `global`, `htui`, `analysis \u{203a} research`, …; snapshot |
| `the_form_shows_the_effective_globs_before_save` → **`skills__attach_form_effective_globs`** | platform; `select("tests")`, `a`, `j` (htui), `enter`, `space`, `tab` × 3, type `rust`, `tab`, type `src/**` | `effective:  src/**, **/*.rs, **/Cargo.toml`; snapshot |
| `ctrl_r_picks_a_repo_of_the_project` → **`skills__repo_picker`** | store + `create_repo(htui "core", primary)`, `(htui "web")`; platform; `select("tests")`, `a`, `j`, `enter`, `ctrl-r` → snapshot; `enter` | picker lists `core`, `web`; after `enter`, frame contains `globs       core:` |
| `a_save_over_a_moved_head_keeps_the_draft` → **`skills__changed_elsewhere`** | `select("rust-style")`, `e`, `X`; `add_skill_version(RUST_STYLE, 2, …)` on the store clone; `ctrl-s`; settle | notice contains `v3 is now the latest` and `saves it as v4`; hint `Ctrl+S save`; snapshot |
| `a_new_skill_needs_a_valid_name_and_a_body` | `n`, `Docs`, `enter` → notice `skill.name \`Docs\``; `backspace` × 4, `docs-style`, `enter`, `How docs read`, `enter`, `ctrl-s` → notice `a skill needs text`; `Write in the active voice.`, `ctrl-s`, settle | store `skills()` holds `docs-style`, `skill_versions` `[1]` with that body; notice `created \`docs-style\` v1` |
| `a_saved_version_moves_the_head_and_the_estimate` | `select("tests")` (title `~32 tokens`); `e`; type `Always name the case. `; `ctrl-s`; settle | title `tests v2 (head v2)` with `~N tokens`, `N > 32`; notice `saved v2 \u{b7} ~N tokens`; store head v2 |
| `the_winning_row_is_starred_per_project` | platform; `select("rust-style")`, `a` | `* htui` and `*   feature \u{203a} implement`; `global`, `agy` unstarred. Then store: `set_skill_binding((RUST_STYLE, None, None), None, Attach(Always))`; `r`; settle → `* global`, `* htui` still; `agy` unstarred |
| `a_glob_row_says_it_fires_from_milestone_5` | store: demo `tests` project row changed to `Glob`, languages `["rust"]`; platform; `select("tests")`, `a` | htui row contains `glob \u{b7} latest \u{b7} pos 0 \u{b7} 2 globs \u{b7} fires from milestone 5` |
| `detach_asks_first` | platform; `select("tests")`, `a`, `j`, `x` → notice `detach \`tests\` from htui?`; `n` → `kept`, row still in store; `x`, `y`, settle | `skill_bindings(Some(PROJECT_HTUI))` has no `tests` project row; notice `detached from htui` |
| `tab_and_digits_still_switch_tabs_with_a_draft_open` | `select("rust-style")`, `e`, `2` → frame `2Prefer`; `tab` → active `settings`; `2`; settle | back on Skills, frame `2Prefer`, hint `Ctrl+S save` |
| `the_strip_text_is_unchanged` | `open()` | ` 1 Backlog  2 Skills  3 Settings  4 Chat` |

`tests/skills_pg.rs` (the `templates_pg.rs:1-120` `Stack`; Graphics, `vulkan-tutorials`):
**`a_skill_created_edited_and_attached_in_the_tui_lands_on_postgres`** — `2`, `n`, `pg-skill`,
`enter`, `enter`, `First.`, `ctrl-s`, drive; `e`, `Second. `, `ctrl-s`, drive; `a`, `j` (vulkan),
`enter`, `ctrl-s`, drive. Then `db.store`: `skills()` holds `pg-skill`; `skill_versions` bodies
`["First.", "Second. First."]`; `skill_bindings(Some(PROJECT_VULKAN))` one row for it, `Always`,
`pinned_version: None`, `position: 0`.

### 6.8 Gate and commits

```bash
USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=$PG/postgres cargo test -p htui --all-features -- --test-threads=2
cargo insta review          # accept the seven skills__*.snap; templates__*.snap must be untouched
git diff --exit-code crates/htui/tests/snapshots/templates__*.snap
cargo clippy -p htui --all-features --all-targets -- -D warnings
```
Commits: (1) red — tests and empty views; (2) library; (3) attachments pane + `skills_pg.rs`.

---

## 7. Cross-task contracts and file sets, recomputed

| Producer | Contract | Consumer |
|---|---|---|
| T1 | `validate_name`; `NewSkill`, `SkillPatch`, `NewSkillVersion`, `SkillBindingKey`, `Attachment` (+`::of`), `BindingChange`; `skill_glob::{SkillGlob, SkillGlobs, GlobError, canonical_globs, split_list}`; `skill_language::{expand, normalise, known, UnknownLanguage}` | T2, T3, T5 |
| T2 | seven `WriteStore` methods on five implementors; `store::{invalid_skill_name, BLANK_SKILL_BODY, has_nul, skill_body_refusal, new_skill_refusal, skill_patch_refusal, skill_version_key, phase_attachment_needs_a_project, phase_not_in_project, pin_names_no_version, negative_position, global_glob_names_a_repo, glob_names_unknown_repo, GLOB_NEEDS_GLOBS, BindingFacts, StoredAttachment, check_attachment}` | T3, T4, T5 |
| T3 | `NewStepGraph.is_override` | every constructor (T3 owns all nine) |
| T4 | `crate::skills::{SkillsSnapshot, SkillEntry, ProjectSkills, StaleWhat, REQUEST_NAMES, READ_NAME}`; five requests; `Skills`, `SkillsStale` | T5 |

| Task | Files (final) |
|---|---|
| T1 | `Cargo.toml`, `Cargo.lock`, `crates/htui-core/Cargo.toml`, `crates/htui-core/src/model/{skill.rs, skill_glob.rs, skill_language.rs, skill_languages.json, mod.rs}` |
| T2 | `crates/htui-core/src/store/{traits.rs, mod.rs, mem.rs, conformance.rs}`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/{write.rs, read.rs, rows.rs (no edit)}`, `crates/htui-store/src/writer.rs`, `crates/htui-store/.sqlx/`, `crates/htui-store/tests/{pg_conformance.rs, skill_writers.rs}`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` |
| T3 | `crates/htui-core/src/model/kind.rs`, `crates/htui-core/src/store/{mem.rs, conformance.rs}` (scoped), `crates/htui-store/src/pg/write.rs` (scoped), `crates/htui-store/.sqlx/`, `crates/htui-orch/src/{graph.rs, engine.rs, conformance.rs}`, `crates/htui-orch/tests/{gix_isolator.rs, fixtures.rs}`, `crates/htui/src/catalogue.rs` |
| T4 | `crates/htui/src/{skills.rs, store_worker.rs, lib.rs}` |
| T5 | `crates/htui/src/ui/tabs/skills/{mod.rs, library.rs, attach.rs}`, `crates/htui/tests/{skills.rs, skills_pg.rs}`, `crates/htui/tests/snapshots/skills__*.snap` |

**Intersections**: T1 ∩ T2 = ∅ (T2 consumes T1: serial). T2 ∩ T3 = {`mem.rs`, `conformance.rs`,
`pg/write.rs`, `.sqlx/`}: serial (T3 after T2). T3 ∩ T4 = ∅; T3 ∩ T5 = ∅; T4 ∩ T5 = ∅ (T5 consumes
T4: serial). `store/mod.rs` (F-C) is T2's alone. **Build coupling**: T3 adds a field to a struct
seven files construct — it owns all of them; T4's test avoids `NewStepGraph` (F-K) so T4 ∥ T3
compiles on either base. T5 constructs no `NewStepGraph`.

---

## 8. Count pins

| Pin | Where | Before → after |
|---|---|---|
| store `CASES` | `mem_store.rs:36`, `pg_conformance.rs:19` | 74 → **80** (T2) |
| `READ_CASES` | `mem_store.rs:46` | 14 (unchanged) |
| orch `CASES` | `htui-orch/tests/fake_conformance.rs:16` | 70 (unchanged) |
| `StoreRequest` / `StoreReply` | `store_worker.rs` (unpinned) | 68 / 39 → **73 / 41** (T4) |
| `.sqlx` files | `ls crates/htui-store/.sqlx \| wc -l` | 264 → **276** (T2 +12; T3 replaces one) |
| `TABLES`, applied, commented columns, next migration | `migrations.rs` | 39, `1..=7`, 34, `0008` (unchanged) |
| `MIRRORED_TABLES` | `htui-store/tests/cache.rs` | 21 (unchanged) |
| `Cargo.lock` packages | — | +1 (`globset`, T1 only) |
| `WriteStore` methods | `traits.rs` | +7 |

---

## 9. Merge order and the workspace gate

`main` merged into the branch before T2 and before review (R-25). T1, then T2, then T3 and T4 from
T2's head, merged T3 then T4 (re-run touched crates' gates after each), then T5. Then:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
. /root/ort/env.sh; USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=$PG/postgres \
  cargo test --workspace --all-features --no-fail-fast -- --test-threads=2
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_mod9m3 cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

---

## 10. Risks (continuing from the plan's R-30)

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-31 | A backticked constraint name or `.rs::` reference in `conformance.rs`'s new doc comments trips `every_cross_referenced_test_name_exists` (F-D). | Medium | Low (a red test, found at once) | §3.5's rule in the T2 prompt. |
| R-32 | `MemStore` and `PgStore` refuse the same input with different sentences. | Medium | Low | D88: every rule and sentence in `traits.rs`; the stores only look facts up. Conformance asserts needles. |
| R-33 | An override clone is refused by a stale qualified glob on a source phase (F-H). | Low (no production caller) | Low | D96: refused before any write, naming the glob; the pane marks the row `?` so it is fixed first. |
| R-34 | Two writes stamped in the same instant leave a spent `updated_at` token valid (pre-existing: `Utc::now()` in `MemStore`, `clock_timestamp()` µs on Postgres). | Very low | Low | Same as MOD-15; the conformance cases read tokens back rather than assume ordering. |
| R-35 | The snapshot is 2 + projects × (3 + graphs) + skills reads per reload (≈ 20 for Platform). | Certain | Low | Per event, never per keystroke (R-27's reasoning). |
| R-36 | `.sqlx` regenerated against a database not migrated fresh keeps a stale nullability (`project_id?`). | Low | Low | §0.1's drop-and-recreate; D61's explicit `?`. |
| R-37 | The sandbox's Postgres is down or sqlx-cli missing at gate time (F-A). | Seen once | Low | §0.1's `pg_ctlcluster` and pinned install. |
| R-38 | The view's landed predicate takes another session's byte-identical write for its own (D98). | Very low | None (the stored content is what was sent) | Accepted, as milestone 1's D27. |

---

## 11. Where this blueprint departs from the plan

F-B (write types in T1's `skill.rs`), F-C (`store/mod.rs` joins T2), F-E (brace-aware list split),
F-F (`GlobError` enum; languages before globs), F-G (the `is_override` round trip extends an
existing case; T3's conformance scope), F-J (`rows.rs` untouched), F-K (T4's test setup), F-L (doc
drift assigned), F-M (picker via key events). **F-H was decided by the maintainer: check first, then refuse** (§4.3; it supersedes the
first default, which let the refusal propagate mid-copy). F-I keeps D82 as written and corrects only its rationale.

---

## 12. Decisions (D87 onward)

| # | Decision |
|---|---|
| D87 | `NewSkill`, `SkillPatch`, `NewSkillVersion`, `SkillBindingKey`, `Attachment`, `BindingChange` live in `model/skill.rs`, written by T1 and re-exported from `model` (F-B). `NewSkill` and `NewSkillVersion` carry `source: serde_json::Value` (D77's `{}` from the view). |
| D88 | Every skill refusal is a pure helper in `store/traits.rs` (§3.2), re-exported from `store`; `check_attachment(&BindingFacts, &Attachment)` is D78's chain with one definition; both stores only gather the facts. |
| D89 | `add_skill_version`'s `expected: i32` uses `0` for "no version yet"; precedence §3.4 (Stale → NotFound skill → NotFound `skill_version` → Constraint). `skill.updated_at` is not bumped by an append. |
| D90 | `set_skill_binding`: token first against the row at the key (`Some(t)` with no row is `Stale(None)`), then NotFound skill/project/phase, then Detach (no row: `Applied(None)`), then `check_attachment`, then the write. |
| D91 | Reader orders: `skills` by name bytes; `skill_versions` by version; `skill_bindings` by `(skill_id, phase_id NULLS FIRST)`. |
| D92 | Postgres: `create_skill` is one explicit transaction of two `INSERT`s; `skill_name_key`'s 23505 maps to `already_exists("skill", name)` (create and update); the attach is `INSERT … ON CONFLICT (skill_id, project_id, phase_id) DO NOTHING` (probed on the NULLS NOT DISTINCT key); the binding id is minted client-side (`SkillBindingId::new()`); readers `query_as!` straight into the model types (no `rows.rs` change); zero-row writes re-read the key (`Stale`). |
| D93 | `skill_glob::split_list` is the one comma-list splitter (brace depth and `[…]` aware, `\` escapes), used by the form for globs and languages (F-E). |
| D94 | `GlobError { Invalid { glob, message }, UnknownLanguage }`; `message` is `globset`'s `kind()` text or this module's sentence (NUL, empty after qualifier); `canonical_globs` expands languages first (F-F). |
| D95 | T3's `is_override` round trip is added to `step_graph_and_phase_round_trip`; `CASES` stays 80 (F-G). |
| D96 | **(maintainer, F-H, 2026-09-26): check first, then refuse.** `override_graph` reads the source phases' attachments and the project's repos before `create_step_graph`; a qualified glob naming no current repo refuses the clone with `Constraint(glob_names_unknown_repo(..))` and nothing is written (§4.3). |
| D97 | T4: the read refuses offline with `PROMPT_ON_SERVER_ONLY`, writes with `DATABASE_UNREACHABLE`; bodies travel as `TemplateBody`; the snapshot reads attachments, graphs and repos before skills and versions; NotFound → `SkillsStale` per §5.3's table. |
| D98 | T5's writes land by content (§6.4's predicates), the milestone 1 D27 rule: a `Skills` reply that does not contain the sent write leaves the draft. |
| D99 | The token estimate is D82's `.content` estimate (the skill's own block); the frame the assembler adds once for all skills is not counted (F-I). |
| D100 | `SkillsTab::on_external_edit` hands a clone to both views; `LibraryView::captures_input` includes a non-Browse attachments pane (F-N). |
| D101 | The repo picker inserts `<repo>:` by feeding char `KeyEvent`s to the globs `TextField` (F-M). |
| D102 | The form prefills `globs` with the stored globs minus the expansion of the stored languages, so a re-save is the same attachment and the field shows what was typed. |
| D103 | In the attachment form `Enter` and `Ctrl+S` both save and `Esc` closes without asking (the kinds editor's shape, `kinds.rs:765-812`); the library editor keeps the templates editor's `Esc`-twice rule. |
| D104 | The star marks the level of `resolve`'s winner per `(project, phase)` over non-override phases; an `Off` winner stars. |
| D105 | `UnknownLanguage`'s `Display` names the language and lists the known ones. |
| D106 | The language map's contents are §2.3's JSON. |
| D107 | New `MemStore` writers stamp `Utc::now()` taken in the arm, as MOD-15's; no `MemFault` variant is added (F-Q). |
