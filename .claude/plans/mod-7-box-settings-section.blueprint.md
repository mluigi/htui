# Blueprint: MOD-7 milestone 2, "the maintainer sees and edits it"

**Status**: **proposed** (2026-09-26). Findings F-A to F-K (§0) and decisions D54–D66 (§11) are this
blueprint's. Where a finding says **Blocker**, the plan read literally either fails its own gate,
cannot be implemented as written, or leaves a behaviour two tests would disagree on. The Fix column is
what the implementer builds.

**Plan**: `.claude/plans/mod-7-box-settings-section.plan.md` at `a4a8d8e`, confirmed at the CONFIRM
gate on 2026-09-26 with the defaults of OQ-13 to OQ-17 and OQ-19. **Milestone 2 is T0–T5.** T6 (the
`box_probe_spec` editor) is deferred to **MOD-51** and is not blueprinted here: its recorded shape
stays in the plan's D52 and Task 6. `ctrl-c` not quitting is **MOD-52** and is not touched. The
plan's "(amended at fact-check)" notes and its Verified-claims table take precedence over its
earlier prose, and this blueprint follows them. PRD D0–D7 win over this blueprint where they
disagree.

**Verified at**: HEAD `a4a8d8e`, branch `mod-7-m2`. `git diff --stat 9a4911a HEAD -- crates
Cargo.toml Cargo.lock` is empty, so the plan's line numbers still hold. Every anchor below was
located through Gortex (`search`, `read`, `relations`) and then re-read at its line in the file,
because the index is stale for files MOD-38 touched (Gortex puts `Writer::boxes` at `writer.rs:375`
and `PgStore::boxes` at `pg/write.rs:940`; the files have them at `:436` and `:1151`). **Line numbers
are pre-edit**: a citation into a file a task edits moves after that task's first commit.
`crates/htui-store/.sqlx/` holds 263 files, `crates/htui/tests/snapshots/` holds 75 `.snap` files,
and `df -h /` shows 92 GB free (80 % used).

**Graphify**: `graphify-out/` does not exist in this checkout, so nothing here comes from it.

**Coupling verdict.** The fact-check's task independence holds. No finding below adds a file to a
task or makes two parallel tasks share a file, a generated artefact or a pin. The wave structure
stands unchanged: **T0 alone; Wave A = lane 1 (T1 then T3) ∥ lane 2 (T2); Wave B = T4 ∥ T5; T5's
live check after both Wave B lanes merge.** One plan error (F-A) is inside T1, and one gap (F-C) is
inside T4.

**Scope**:
- **Order**:
  1. T0 alone.
  2. Wave A: lane 1 is T1 then T3 (serial, one worktree); lane 2 is T2 (own worktree).
  3. Merge T1, then T2, then T3, re-running the touched crates' gates on the real tree after each
     merge.
  4. Wave B: T4 and T5, each in its own worktree. Merge T4, then T5.
  5. The live check on the real tree.
- **No migration**: the next is still `0007`.
- **One new `WriteStore` method**: `edit_box`, on all five implementations.
- **New modules**: `htui::box_settings` (worker side), `htui::ui::text_area` (widget),
  `htui::ui::tabs::settings::boxes` (section).
- **Request enums**: `StoreRequest` 64 → 66 (`Boxes`, `EditBox`); `StoreReply` 35 → 37 (`Boxes`,
  `BoxesStale`).
- **Pins that move**: store `CASES` 68 → 71; `.sqlx` 263 → 263 (T0, three hashes change) → 264
  (T1); snapshots 75 → 81; Settings strip 6 sections / 54 columns → 7 / 61.

**House style (carried)**:
- `unsafe_code = "forbid"`, `missing_debug_implementations = "warn"`, `unused_qualifications =
  "warn"`, clippy `all` at warn with `-D warnings` in the gate, clippy pedantic **not** enabled
  (`Cargo.toml:120-134`). Every lib warns on `missing_docs`; rustdoc denies broken and private
  intra-doc links. The workspace lints are not edited.
- Every new box writer is a compare-and-set on `box.edit_version`. Nothing sets `updated_at` by hand
  on Postgres (the `set_updated_at` trigger does); `MemStore` stamps `now` as its twin of the
  trigger, as `record_box_probe` does (`mem.rs:1665`).
- No store handle, no `UserId` and no minted `BoxId` on the render side (`R-NF-3`): a section names
  requests and renders replies; every `BoxId` it sends came out of a snapshot.
- No `std` guard is held across an `.await`.
- Implementers commit incrementally, staging their own paths only (never `-A`, never `stash`). Every
  commit compiles; a red commit uses `todo!()` bodies (but see F-F for the section).
- Every gate is re-run with `--test-threads=1` on the real tree after the merge.

---

## 0. Findings the plan fact-check missed

| # | Blocker? | Plan says | Tree at `a4a8d8e` | Fix |
|---|---|---|---|---|
| **F-A** | **Blocker** (the `htui-core` gate) | T1: "A module test after `conformance.rs:9509` checks that every test name a doc comment in that module cites exists. So a case doc may cite `a_reconnect_leaves_edit_version_and_the_edited_fields_alone` only under exactly that name." | `every_cross_referenced_test_name_exists` (`conformance.rs:9529`) checks two shapes. A bare backticked snake_case name with four or more underscores must be **defined in `conformance.rs` or `mem.rs`** (`:9619-9625`), and the box-identity test lives in `htui-store/tests/box_identity.rs`, so citing it bare fails. A `<file>.rs::<name>` span whose file is not `conformance`, `mem` or `pg_criteria` **panics** (`:9588-9592`). Either spelling breaks the gate. | D54: T1 adds a `"box_identity"` arm to the scanner's file table, read at run time from `../htui-store/tests/box_identity.rs` exactly as `pg_criteria` is (`:9559-9563`), and case docs cite `box_identity.rs::a_reconnect_leaves_edit_version_and_the_edited_fields_alone` and `box_identity.rs::another_users_box_is_not_found_by_edit_box`. The `mem.rs` unit test may be cited bare. `conformance.rs` is already T1's file. |
| **F-B** | **Blocker** (the store half of D42 is unspecified) | D42: two functions, `is_declared_tag` and `declared_tags_from_text(text)`; "both stores call the same validator before writing and store the sorted, deduplicated list". | The store receives `BoxEdit.declared_tags: Option<Vec<String>>`, not text. `declared_tags_from_text` splits on `,` and trims, so reaching it through `tags.join(",")` would let the store accept `"a,b"` as two tags and `" gpu"` as `gpu`: a silent rewrite the plan forbids ("never lower-cased on the user's behalf"). | D55: a third function, `canonical_declared_tags(&[String]) -> Result<Vec<String>, String>`, validates each element **strictly** (no trim, no split, an empty element is refused), then sorts and deduplicates. Both stores call it. `declared_tags_from_text` is split, trim, drop empties, then `canonical_declared_tags`. One private `declared_tag_refusal(tag)` builds the one sentence. |
| **F-C** | **Blocker** (two of T4's tests would disagree) | D48: "A plain `Boxes` reply never touches that token … it replaces the list and leaves the typed text and the token alone", and "`Boxes` after a write closes the editor". | `StoreReply::Boxes` carries nothing that says which request it answers, and `EditBox` is answered with `Boxes` on `Applied`. Without a marker the section cannot tell a reload from the reply to its own save. The kinds section has the same problem and solves it with `busy: Option<&'static str>` (`kinds.rs:1322-1356`, M4 H-9). | D56: `BoxesSection.busy: Option<&'static str>`, set to `"edit_box"` when `EditBox` is sent. A `Boxes` reply with `busy` set closes the editor and clears `busy`; with `busy` unset it leaves the editor, its text and its token alone. A second save while `busy` is set sends nothing and says `edit_box in flight`. H-9's bounded residue is carried and documented. Two tests are added (§6.6). |
| **F-D** | Non-blocker (snapshot churn) | D51: the spec line shows "the first twelve hex digits of the digest"; D47: the detail pane shows "last probe". | The seed's digest is `sha256` of `spec.json` merged. Any future edit of `spec.json` would move every `box_settings__*` snapshot that draws the spec line, for a reason unrelated to the section. A relative time ("3m ago") would move with the wall clock. | D57: every `box_settings` snapshot that draws the spec line runs under an `insta` filter mapping `\b[0-9a-f]{12}\b` to `<digest>` (`insta` already has the `filters` feature, `crates/htui/Cargo.toml:62`). Times render absolute, `%Y-%m-%d %H:%M UTC`. |
| **F-E** | Non-blocker (compile) | D41 lists the five implementations. | `traits.rs`'s model import (`:33-41`) has `BoxRecord` but not `BoxRow`. `htui-agent/src/conformance.rs` (`:32-44`) and `tests/recorder.rs` import `BoxRecord` but neither `BoxRow` nor `BoxEdit`. | T1 adds `BoxEdit` and `BoxRow` to those three import lists, and `BoxEdit` to `mem.rs:23`, `writer.rs` and `pg/write.rs:23`. |
| **F-F** | Non-blocker (a red commit that breaks other suites) | "a red commit uses `todo!()` bodies". | `SettingsTab::on_reply` hands **every** reply to **every** section (`settings/mod.rs:352-356`), and `connection.rs::the_product_registers_connection_after_prompt` (`:2050`) drives the production `register_all`. A `BoxesSection` with a `todo!()` `on_reply` registered there panics that suite. | T4's red commit does not register the section, and its skeleton has no `todo!()` in any method the shell calls. `register_all` gains the line in T4's last green commit. |
| **F-G** | Non-blocker (a test that reads the keyring) | T4: snapshot `box_settings__offline`. | `App::start` issues `ConnectionInfo`, which over a non-`Memory` backend reaches the OS keyring (`tests/prompt_settings.rs:854-856`). | The offline snapshot test holds `htui_store::testkit::mock_keyring().await` and keeps its `tempdir` alive for the whole test, as `offline_is_unavailable_with_the_worker_sentence` does. |
| **F-H** | Non-blocker (settles a plan "or") | D41: the filtered re-read "either filters `box_row`'s result in Rust (no new statement) or adds a query (`.sqlx` +2)"; the Count-pins row says 264 "or +2". | `PgStore::box_row` (`pg/read.rs:1796`) is an inherent `pub async fn` that `impl WriteStore for PgStore` can call, and `PgStore::this_user` is `pub const fn`. The invalid-tag precedence path (NotFound, then Stale, then Constraint) needs the same read, which `update_item_kind` already does with its own reader (`pg/write.rs:1942-1955`). | D58: both the miss path and the invalid-tag path call `self.box_row(id)` and keep it only when `row.user_id == self.this_user()`. T1 adds exactly **one** statement: `.sqlx` 263 → **264**. |
| **F-I** | Non-blocker (a phantom write) | D48: "only when the text differs from what the editor opened on". | Tags: `gpu` and `gpu, ` are different text and the same list. Quirks: a row written with `\r\n` by hand (SQL) would never equal the widget's `\n`-joined text. | D59: the tag editor compares the **parsed** list with `canonical_declared_tags(opened list)`; the quirks editor compares `TextArea::text()` with the text the `TextArea` produced when it opened (`with_text` normalises `\r\n` and a lone `\r` to `\n`). |
| **F-J** | Non-blocker (a stale comment) | D46: "one or-ed arm". | The four or-ed arms carry running counts in their comments ("the twenty-one above" at `store_worker.rs:1050`, "the twenty-four above" at `:1056`). Above the new arm there are 12 + 9 + 3 + 4 = 28. | The new arm's comment says "for the same reason the twenty-eight above are". |
| **F-K** | Non-blocker (lock discipline) | D41 on `MemStore`. | `MemStore::this_user` takes the read lock (`mem.rs:210-221`); calling it inside a `write` closure would deadlock on the `RwLock`. | `MemStore::edit_box` reads `self.this_user()` and `Utc::now()` **before** `self.write(..)`, as `boxes` (`mem.rs:5193-5196`) does. |

### 0a. Settled answers

| Question | Answer | Where |
|---|---|---|
| Where each new item lives | `BoxRow.edit_version`, `BoxEdit`, `DECLARED_TAG_MAX`, `is_declared_tag`, `canonical_declared_tags`, `declared_tags_from_text` → `htui_core::model::box_` (re-exported from `model`). `WriteStore::edit_box` → `htui_core::store::traits`. `BoxesSnapshot`, `SpecView`, `spec_view`, `snapshot`, `serve`, `REQUEST_NAMES`, `READ_NAME` → `htui::box_settings`. `TextArea` → `htui::ui::text_area` (re-exported from `htui::ui`). `BoxesSection` → `htui::ui::tabs::settings::boxes` (re-exported from `settings`). | §2–§6 |
| How the token flows | Postgres `box.edit_version` → `BoxRow.edit_version` (T0, three statements) → `BoxesSnapshot.boxes[i].row.edit_version` (T3) → the section's editor records it on open (T4) → `StoreRequest::EditBox { expected }` → `WriteStore::edit_box(id, expected, edit)` → `UPDATE … WHERE edit_version = $2` → `Applied` (token + 1) or `Stale` (the row as it is now). | §2, §3, §5, §6 |
| Who bumps the token | `edit_box` only. `register_box`, `record_box_probe`, `refresh_box` and the demo loader never name the column. | D39 (plan) |
| Where the effective spec is computed | Only in `htui_agent::box_probe::spec::effective`, called by `box_settings::spec_view` on the worker side. The section renders `SpecView`. | §5 |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (minimum; each compiles) | Gate |
|---|---|---|---|
| T0 foundations | htui-core, htui-store | 3 (§2.6) | `cargo test -p htui-core --all-features -- --test-threads=1`; `SQLX_OFFLINE=true cargo build -p htui-store --all-features`; prepare + `--check`; `.sqlx` = 263 |
| T1 writer | htui-core, htui-store, htui-agent (worktree A) | 3 (§3.9) | `htui-core`; Postgres `pg_conformance` and `box_identity`; `htui-agent`; prepare + `--check`; `.sqlx` = 264 |
| T2 widget | htui (worktree B) | 2 (§4.4) | `cargo test -p htui --all-features --lib ui::text_area -- --test-threads=1`; clippy on `htui` |
| T3 worker side | htui (worktree A, after T1) | 2 (§5.6) | `cargo test -p htui --all-features -- --test-threads=1` |
| merge | — | T1, then T2, then T3 | after **each** merge: the gates of the crates it touched, on the real tree |
| T4 section | htui (worktree C) | 3 (§6.8) | `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta` review of the six new snapshots only |
| T5 end to end | htui tests (worktree D) | 1 (§7.3) | the Postgres `box_probe_pg` suite, then the workspace gate (§9) |
| merge | — | T4, then T5 | the workspace gate on the real tree; then the live check |

---

## 2. T0: foundations, the token on the row and the tag rule (D40, D42, D55)

**First failing test**: `a_declared_tag_list_is_split_on_commas_trimmed_sorted_and_deduplicated`.

**Files**: `crates/htui-core/src/model/box_.rs`, `crates/htui-core/src/model/mod.rs`,
`crates/htui-core/src/fixtures.rs`, `crates/htui-store/src/pg/read.rs`,
`crates/htui-store/src/pg/write.rs`, `crates/htui-store/.sqlx/`.

### 2.1 `box_.rs`: the field (`BoxRow`, `:27-66`)

After `updated_at` (`:64-65`), the last field:

```rust
    /// `box.edit_version`: the compare-and-set token of the declared-tags and quirks editors
    /// (MOD-7 milestone 2, D39). Bumped by `WriteStore::edit_box` and by nothing else:
    /// registration and the box probe never write it, so a reconnect cannot stale an open editor.
    pub edit_version: i32,
```

Plain backticks, not an intra-doc link: the method arrives in T1, and rustdoc denies broken links.
The doc is not revisited in T1, so `box_.rs` stays a T0-only file.

### 2.2 `box_.rs`: the edit and the tag rule (after `impl BoxRecord`, `:133-142`, before `BoxInfo`, `:144`)

```rust
/// The longest declared tag, in chars (MOD-7 D42).
pub const DECLARED_TAG_MAX: usize = 64;

/// One human edit of a box row (MOD-7 milestone 2, D41): `Some` writes that column, `None` leaves
/// it alone. An edit with both `None` is legal and still bumps `edit_version`.
///
/// `Debug` is derived on purpose: quirks are not secret (they go into every prompt's box
/// section), and `htui`'s `StoreRequest` carries this type and derives `Debug`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoxEdit {
    /// `box.declared_tags`, whole; the store validates, sorts and deduplicates it.
    pub declared_tags: Option<Vec<String>>,
    /// `box.quirks`, whole, lines separated by `\n`; stored as given (D43).
    pub quirks: Option<String>,
}

/// Whether `tag` is a declared tag (MOD-7 D42): 1 to [`DECLARED_TAG_MAX`] chars of
/// `[a-z0-9_-]`, the first one a letter or a digit. Every tag the seed derives already is.
#[must_use]
pub fn is_declared_tag(tag: &str) -> bool;

/// The stored form of a declared-tag list (MOD-7 D42, D55): each element validated **as given**
/// (no trim, no split, no case change), then sorted by bytes and deduplicated. Both stores call
/// this before they write, so a refusal carries one sentence on `MemStore` and on Postgres.
///
/// # Errors
///
/// The refusal sentence of the first element [`is_declared_tag`] rejects, in input order.
pub fn canonical_declared_tags(tags: &[String]) -> Result<Vec<String>, String>;

/// A declared-tag list as the user types it (MOD-7 D42, D50): split on `,`, each piece trimmed,
/// empty pieces dropped, then [`canonical_declared_tags`]. Empty text is no tags.
///
/// # Errors
///
/// [`canonical_declared_tags`]'s sentence.
pub fn declared_tags_from_text(text: &str) -> Result<Vec<String>, String>;

/// The one refusal sentence (private).
fn declared_tag_refusal(tag: &str) -> String {
    format!(
        "declared tag `{tag}` is not 1-{DECLARED_TAG_MAX} characters of a-z, 0-9, `_` and `-` \
         starting with a letter or a digit"
    )
}
```

Nothing in the rule lower-cases a tag: `GPU` is refused, never rewritten.

### 2.3 Constructors that move (all in T0)

- `box_.rs` test helper `row()` (`:272-294`): `edit_version: 0,` after `updated_at: at(),`. The three
  spreads (`:316`, `:330`, `:400`) and `record()` (`:329-335`) need nothing.
- `fixtures.rs` demo box (`:459-479`): `edit_version: 0,` after `updated_at: demo_at(2, 8),`
  (`:478`). The column default is 0 and `pg/demo.rs:65-71` names its columns without it, so Postgres
  and `MemStore` load the same value.
- `pg/write.rs` `boxes` (`:1151-1226`): the `SELECT` (`:1155-1160`) gains `edit_version` after
  `updated_at` and before `probe_spec_digest`; the literal (`:1200-1220`) gains
  `edit_version: row.edit_version,`. The doc at `:1140-1141` ("the digest is not a `BoxRow` field")
  stays true.
- `pg/read.rs` `box_profile` (`:1519`, SELECT list `:1522-1541`) and `box_row` (`:1796`, SELECT list
  `:1799-1818`): `edit_version` after `updated_at` in both. `query_as!(BoxRow, ..)` needs every
  field, which is the only reason `box_profile` (a prompt projection) reads it.
- `model/mod.rs:100-103`: the `box_` re-export gains `BoxEdit`, `DECLARED_TAG_MAX`,
  `canonical_declared_tags`, `declared_tags_from_text`, `is_declared_tag`.

`git grep -n 'BoxRow {'` finds literals only at `box_.rs:273`, `fixtures.rs:459` and
`pg/write.rs:1200` (re-checked at `a4a8d8e`); no other crate builds a `BoxRow`, and no serialized
`BoxRow` is read from disk (`DemoData` is not `Deserialize`).

### 2.4 `.sqlx`

Three hashes change (`box_profile`, `box_row`, `boxes`), no file is added: **263**. Prepare against a
scratch database migrated through `0006` (the compose `htui` database is empty; project memory):

```bash
docker compose exec -T postgres dropdb -U postgres --if-exists htui_prepare_mod7m2
docker compose exec -T postgres createdb -U postgres htui_prepare_mod7m2
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m2 \
  sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m2 \
  cargo sqlx prepare -- --all-targets --all-features)
```

The scratch database is recreated before every prepare in T0 and T1 (no migration is edited this
milestone, so this is belt and braces, carried from milestone 1's F-S).

### 2.5 Tests (`box_.rs` `mod tests`)

| Test | Asserts |
|---|---|
| `a_declared_tag_is_lowercase_digits_underscore_and_dash` | `is_declared_tag` accepts `gpu`, `heavy_build`, `x86-64`, `9p`, `a`; refuses `""`, `_x`, `-x`, `GPU`, `gpu tag`, `gpü`, `a,b`. |
| `a_declared_tag_list_is_split_on_commas_trimmed_sorted_and_deduplicated` | `declared_tags_from_text(" vulkan, gpu ,,gpu, heavy_build ")` is `Ok(["gpu", "heavy_build", "vulkan"])`. |
| `an_empty_tag_list_is_no_tags` | `""`, `"  "` and `" , , "` are `Ok([])`. |
| `an_uppercase_or_spaced_tag_is_refused_by_name` | `"gpu, Vulkan"` is `Err` whose text contains `` `Vulkan` `` and equals `declared_tag_refusal("Vulkan")`; `"gpu, heavy build"` names `` `heavy build` ``. |
| `every_seeded_and_derived_tag_is_a_valid_declared_tag` | The fifteen, written out: `gpu vulkan msvc mingw clang cmake vcpkg docker rust heavy_build` (`R-BOX-3`, `pg/mod.rs:27-38`) and `go node python java dotnet` (milestone 1 OQ-12). Each passes `is_declared_tag`. |
| `a_sixty_five_char_tag_is_refused` | `"a".repeat(64)` passes and `"a".repeat(65)` does not. |
| `the_store_list_is_validated_strictly_not_trimmed_or_split` (D55) | `canonical_declared_tags` refuses `[" gpu"]`, `["a,b"]` and `[""]`, and turns `["vulkan", "gpu", "gpu"]` into `["gpu", "vulkan"]`. |

### 2.6 Gate and commits

```bash
cargo test -p htui-core --all-features -- --test-threads=1
# prepare as in §2.4, then:
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m2 \
  cargo sqlx prepare --check -- --all-targets --all-features)
SQLX_OFFLINE=true cargo build -p htui-store --all-features
ls crates/htui-store/.sqlx | wc -l          # 263
```

Commits: (a) red: `BoxEdit`, the constant, the three functions with `todo!()` bodies, the seven
tests; (b) green: the bodies; (c) the field, the four constructors and statements, the re-exports,
`.sqlx`. Each compiles; (c) is mechanical and needs the prepare.

---

## 3. T1: the writer on every store (D41, D54, D58)

**First failing test**: `store::conformance::edit_box_is_cas_on_edit_version` over `MemStore` (through
`run_case_accepts_every_name_in_cases`). T1 starts from T0 merged.

**Files**: `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`,
`crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`,
`crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`,
`crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/box_identity.rs`,
`crates/htui-store/.sqlx/`, `crates/htui-agent/src/conformance.rs`,
`crates/htui-agent/tests/recorder.rs`. The plan's list, unchanged.

### 3.1 `traits.rs` (after `boxes`, `:384-391`)

```rust
    /// The declared-tags and quirks editors' compare-and-set (MOD-7 milestone 2, D41): writes the
    /// columns `edit` names, and `edit_version + 1`, where the row is this user's and its
    /// `edit_version` is `expected`, in one statement. A narrow human writer (MOD-2 D74): never
    /// `hostname`, the probe columns, `htui_version`, `settings`, `machine_fingerprint`,
    /// `probe_spec_digest`, `last_seen_at` or `box_tool`. Registration and the probe never write
    /// `edit_version`, so neither can stale an open editor.
    ///
    /// Answers [`CasOutcome::Applied`] with the row as written, or [`CasOutcome::Stale`] with the
    /// row as it is now when `expected` is spent (nothing is written).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "box"` for an
    /// unknown id **or a box of another `app_user`** (the reach of [`boxes`](WriteStore::boxes));
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) carrying
    /// [`canonical_declared_tags`](crate::model::canonical_declared_tags)'s sentence for a tag it
    /// refuses. Precedence: `NotFound`, then `Stale`, then `Constraint`.
    async fn edit_box(&self, id: BoxId, expected: i32, edit: BoxEdit) -> Result<CasOutcome<BoxRow>>;
```

The model import (`:33-41`) gains `BoxEdit` and `BoxRow` (F-E). The doc cites no test name in
backticks (F-A applies to `conformance.rs`, not here, but the habit is the same).

### 3.2 `MemStore` (`mem.rs`)

- Import (`:23`): `BoxEdit`, `canonical_declared_tags`.
- `State::edit_box` after `box_records` (`:1683-1710`):

```rust
    /// `edit_version` compare-and-set on one box of `user` (MOD-7 D41): `NotFound` for an unknown
    /// id or another user's box, then `Stale` for a spent token, then `Constraint` for a tag, and
    /// only then the write. `now` stands in for Postgres's `set_updated_at` trigger.
    fn edit_box(
        &mut self,
        user: Option<UserId>,
        id: BoxId,
        expected: i32,
        edit: BoxEdit,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<BoxRow>>;
```

  Body, in order: `let Some(row) = self.boxes.get(&id).filter(|row| Some(row.user_id) == user) else
  { NotFound { entity: "box", id } }`; `if row.edit_version != expected { return
  Ok(CasOutcome::Stale(row.clone())) }`; `let tags =
  edit.declared_tags.as_deref().map(canonical_declared_tags).transpose().map_err(StoreError::Constraint)?`;
  then `get_mut`, write `declared_tags` when `Some`, `quirks` when `Some`, `edit_version += 1`,
  `updated_at = now`, and answer `Applied(row.clone())`.
- `WriteStore for MemStore`, after `boxes` (`:5193-5196`):

```rust
    async fn edit_box(&self, id: BoxId, expected: i32, edit: BoxEdit) -> Result<CasOutcome<BoxRow>> {
        let user = self.this_user();
        let now = Utc::now();
        self.write(|state| state.edit_box(user, id, expected, edit, now))
    }
```

  `this_user()` is read before the `write` (F-K).
- Unit test after `boxes_lists_only_this_user_s_boxes_in_id_order` (`:6356`):
  **`edit_box_refuses_another_user_s_box_as_not_found`**. Builds `demo_data()` plus a stranger
  `AppUser` (`created_at = Utc::now()`, so the demo user stays `this_user`) and a box
  `BoxId::from_uuid(Uuid::from_u128(2))` cloned from the fixture box under the stranger, through
  `MemStore::from_demo`. `edit_box(foreign, 0, tags ["gpu"])` is `NotFound { entity: "box", .. }`,
  even with the right token; `store.box_row(foreign)` (`mem.rs:615`) is byte-equal before and after;
  `edit_box(foreign, 7, tags ["BAD"])` is still `NotFound` (precedence).

### 3.3 `PgStore` (`pg/write.rs`, inside `impl WriteStore for PgStore` at `:586`, after `boxes`, `:1226`)

```rust
    /// The editors' compare-and-set (MOD-7 D41, D58): one `UPDATE` with the user filter and the
    /// token in its `WHERE`, `COALESCE` so a `None` field keeps its column. A miss re-reads the row
    /// through [`PgStore::box_row`], kept only when it is this user's, and [`cas_miss`] decides
    /// `Stale` or `NotFound`. An invalid tag list takes the same read first, so `NotFound` and
    /// `Stale` win over `Constraint` (the order `update_item_kind` keeps, `:1942-1955`).
    async fn edit_box(&self, id: BoxId, expected: i32, edit: BoxEdit) -> Result<CasOutcome<BoxRow>>;
```

Body:
1. `let tags = match edit.declared_tags.as_deref().map(canonical_declared_tags).transpose() { Ok(t)
   => t, Err(sentence) => { let current = self.box_row(id).await?.filter(|row| row.user_id ==
   self.this_user()); return match current { None => Err(NotFound { entity: "box", id }),
   Some(row) if row.edit_version != expected => Ok(CasOutcome::Stale(row)), Some(_) =>
   Err(StoreError::Constraint(sentence)) }; } };`
2. The one new statement, `sqlx::query_as!(BoxRow, …)`, `fetch_optional(&self.pool)`:

```sql
UPDATE box
   SET declared_tags = COALESCE($3, declared_tags),
       quirks        = COALESCE($4, quirks),
       edit_version  = edit_version + 1
 WHERE id = $1 AND user_id = $5 AND edit_version = $2
RETURNING id             AS "id: BoxId",
          user_id        AS "user_id: htui_core::model::UserId",
          hostname,
          os_family      AS "os_family: htui_core::model::OsFamily",
          os_version, arch, cpu, ram_mb, gpu_present, gpu_vendor, htui_version,
          probed_tags, declared_tags, quirks, settings,
          registered_at, last_seen_at, last_probed_at, updated_at, edit_version
```

   Binds: `id.as_uuid()`, `expected`, `tags.as_deref()` (`Option<&[String]>`), `edit.quirks`
   (`Option<String>`), `self.this_user().as_uuid()`. If `prepare` infers a `NOT NULL` column as
   nullable, add a `!` override on that column only.
3. `Some(row)` → `Ok(Applied(row))`; `None` → `cas_miss(self.box_row(id).await?.filter(|row|
   row.user_id == self.this_user()), "box", id)`.

The import (`:23`) gains `BoxEdit` and `canonical_declared_tags`. No `pg/read.rs` edit.

### 3.4 `Writer` and the two spies

- `writer.rs`, after `boxes` (`:436-441`): `edit_box`, `match self { Self::Memory(store) =>
  store.edit_box(id, expected, edit).await, Self::Online(pg) => pg.edit_box(id, expected,
  edit).await }`. Import gains `BoxEdit`, `BoxRow`.
- `htui-agent/src/conformance.rs` (`UsageSpy`, after `boxes`, `:756-758`) and
  `htui-agent/tests/recorder.rs` (`SpyStore`, after `boxes`, `:440-442`): each forwards through
  `self.inner`, returning `StoreResult<CasOutcome<BoxRow>>`. Imports gain `BoxEdit` and `BoxRow`
  (F-E).

### 3.5 Conformance cases (`CASES` 68 → 71)

Appended to `CASES` after `"project_delete_counts_requirements"` (`:110`); `run_case` arms after
`:233`; the three `async fn`s after `boxes_lists_every_box_with_its_tools` (`:5166-5200`), beside the
milestone 1 box cases and their helpers (`fixture_box` `:4919`, `probe_clock` `:4929`,
`probed_tool`, `box_probe`). Import (`:16-31`) gains `BoxEdit`. Every read-back compares against
`store.boxes()`; timestamps come from the stores, so no truncation is needed.

| Case | Asserts |
|---|---|
| `edit_box_is_cas_on_edit_version` | The fixture row reads `edit_version == 0`. (1) `edit_box(ids::BOX, 0, tags ["heavy_build","gpu","gpu"])` is `Applied(row)` with `edit_version == 1`, `declared_tags == ["gpu","heavy_build"]`, `quirks == ""`, `updated_at > fixture.updated_at`, and `hostname`, `os_family`, `os_version`, `arch`, `cpu`, `ram_mb`, `gpu_present`, `gpu_vendor`, `htui_version`, `probed_tags`, `settings`, `registered_at`, `last_seen_at`, `last_probed_at` equal to the fixture's; `boxes()` then shows that row, the same tools and the same `probe_spec_digest` as before. (2) `edit_box(ids::BOX, 1, quirks "line one\nline two")` is `Applied` with `edit_version == 2`, the tags of (1) untouched, and the newline kept byte for byte. (3) `edit_box(ids::BOX, 2, BoxEdit::default())` is `Applied` with `edit_version == 3` and nothing else but `updated_at` moved. (4) `edit_box(ids::BOX, 1, tags ["x"])` is `Stale(row)` where `row` equals the current `boxes()` row (`edit_version == 3`), and `boxes()` is unchanged. (5) `edit_box(BoxId::new(), 0, tags ["x"])` is `NotFound { entity: "box", .. }`. The doc names the Postgres-only halves: `box_identity.rs::a_reconnect_leaves_edit_version_and_the_edited_fields_alone` and `box_identity.rs::another_users_box_is_not_found_by_edit_box`, and the `MemStore` half `edit_box_refuses_another_user_s_box_as_not_found` (F-A). |
| `edit_box_survives_a_probe_between_read_and_write` | Read `t = boxes()[0].row.edit_version` (0). `record_box_probe(&box_probe(ids::BOX, probe_clock(0), vec![probed_tool("cargo", "1.0")], "between"))`. `edit_box(ids::BOX, t, tags ["gpu","heavy_build"])` is `Applied` with `edit_version == t + 1`; the returned row still has `os_version == "os between"`, the probe's `probed_tags`, `htui_version` and `last_probed_at == Some(probe_clock(0))`; `boxes()` still has tools `[cargo 1.0]` and `probe_spec_digest == Some(sha256_hex("between"))`. The doc says this is the in-trait half of "a reconnect cannot stale", because `register_box` is not a trait method. |
| `edit_box_refuses_an_invalid_tag` | `edit_box(ids::BOX, 0, tags ["GPU"])` is `Err(Constraint(s))` with `s == canonical_declared_tags(&["GPU".into()]).unwrap_err()`; `boxes()` unchanged, `edit_version == 0`. A valid edit moves the token to 1. `edit_box(ids::BOX, 0, tags ["bad tag"])` is `Ok(Stale(_))`, not `Constraint`. `edit_box(BoxId::new(), 0, tags ["BAD"])` is `NotFound`. |

**Scanner (D54, F-A)**: in `every_cross_referenced_test_name_exists`, beside `pg_criteria`
(`:9559-9563`):

```rust
        let box_identity = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../htui-store/tests/box_identity.rs"),
        )
        .expect("read crates/htui-store/tests/box_identity.rs");
```

and the file table (`:9585-9593`) gains `"box_identity" => box_identity.as_str(),`. The doc of the
test gains one sentence naming the new file.

**Pins**: `crates/htui-core/tests/mem_store.rs:37` becomes `71`, and the message gains "MOD-7
milestone 2's three for the box editors' compare-and-set (plan D41)"; `crates/htui-store/tests/
pg_conformance.rs:19` becomes `const EXPECTED_CASES: usize = 71;`. `READ_CASES` stays 14.

### 3.6 Postgres-only cases (`crates/htui-store/tests/box_identity.rs`, appended)

| Test | Asserts |
|---|---|
| `a_reconnect_leaves_edit_version_and_the_edited_fields_alone` | `fresh_db`; `me = db.store.this_box()`. `edit_box(me, 0, BoxEdit { declared_tags: Some(["gpu"]), quirks: Some("a\nb") })` is `Applied` at `edit_version 1`. Read `updated_at::text`. `register_box(&identity(me, &unique_hostname("RENAMED")), None)` is `Known { renamed_from: Some(_) }`. By SQL: `edit_version == 1`, `declared_tags == {gpu}`, `quirks == "a\nb"`, `updated_at` moved (the trigger fired). `edit_box(me, 1, quirks "c")` is `Applied` at `edit_version 2`. |
| `another_users_box_is_not_found_by_edit_box` | The stranger user and box of `an_id_under_another_user_is_not_adopted` (`:368-390`). `edit_box(foreign, 0, tags ["gpu"])` is `NotFound { entity: "box", .. }`; `whole_row(&db.pool, foreign)` (`:85`) is byte-equal before and after. |

Imports gain `htui_core::model::BoxEdit`, `htui_core::store::{CasOutcome, StoreError, WriteStore as
_}`.

### 3.7 `.sqlx`

One statement is added: **264**. Prepare as in §2.4, against a recreated `htui_prepare_mod7m2`.

### 3.8 Gate

```bash
cargo test -p htui-core --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features --test pg_conformance --test box_identity -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1
# prepare as in §2.4, then --check
ls crates/htui-store/.sqlx | wc -l          # 264
```

### 3.9 Commit boundaries (T1)

1. (a) Red: the trait method; `todo!()` in `MemStore` and `PgStore`; the `Writer` and spy forwards
   (pure, not `todo!()`); the three cases, their arms, the scanner arm and both pins; the `mem.rs`
   unit test; the two `box_identity.rs` tests. Compiles offline: the `PgStore` stub has no query.
2. (b) Green: `MemStore`.
3. (c) Green: `PgStore` and `.sqlx`.

---

## 4. T2: `TextArea` (PRD D3, D44)

**First failing test**: `ui::text_area::tests::enter_splits_the_line_at_the_cursor`.

**Files**: `crates/htui/src/ui/text_area.rs` (new), `crates/htui/src/ui/mod.rs` (`pub mod
text_area;` after `:7`; `pub use text_area::TextArea;` after `:11`). `text_field.rs` is not touched:
the horizontal window is re-implemented, not extracted (plan D44).

### 4.1 Surface

```rust
//! One small multi-line text area (MOD-7 milestone 2, PRD D3): the quirks editor's widget, the
//! multi-line sibling of [`TextField`](crate::ui::TextField). Hard lines only (no soft wrap),
//! counted in `char`s. `ctrl-s` submits, because `Enter` breaks the line and no terminal mode
//! that reports `Ctrl+Enter` is enabled (plan OQ-16). No history, selection, undo, mask,
//! bracketed paste or `Zeroizing`: what it holds is not secret.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};

use crate::ui::{FieldOutcome, Theme};

/// A multi-line buffer with a `(row, col)` cursor, counted in `char`s.
#[derive(Clone)]
pub struct TextArea {
    /// The lines, never empty: an empty area is one empty line. No line holds `\n` or `\r`.
    lines: Vec<String>,
    /// Line index, `0..lines.len()`.
    row: usize,
    /// Char index into `lines[row]`, `0..=chars`.
    col: usize,
}

impl Default for TextArea { /* one empty line, cursor (0, 0) */ }

/// Never the text: sections derive `Debug`. Prints `line_count` and `len` only.
impl core::fmt::Debug for TextArea { /* … */ }

impl TextArea {
    #[must_use] pub fn new() -> Self;
    /// `text` split into lines on `\n`, after `\r\n` and a lone `\r` become `\n` (D59); cursor at
    /// the end of the last line.
    #[must_use] pub fn with_text(text: &str) -> Self;
    /// Feeds one key (§4.2).
    pub fn on_key(&mut self, key: KeyEvent) -> FieldOutcome;
    /// The lines joined by `\n`.
    #[must_use] pub fn text(&self) -> String;
    /// Whether the area is one empty line.
    #[must_use] pub fn is_empty(&self) -> bool;
    /// How many chars, the `\n` between lines included (so `len() == text().chars().count()`).
    #[must_use] pub fn len(&self) -> usize;
    /// How many lines (at least 1).
    #[must_use] pub fn line_count(&self) -> usize;
    /// The cursor as `(row, col)`.
    #[must_use] pub const fn cursor(&self) -> (usize, usize);
    /// At most `height` lines, each at most `width` cells (§4.3).
    #[must_use]
    pub fn lines(&self, width: u16, height: u16, focused: bool, theme: &Theme) -> Vec<Line<'static>>;
}
```

### 4.2 Keys

- Any modifier other than `SHIFT` (`CONTROL`, `ALT`, `SUPER`, `META`, `HYPER`) → `Pass`, **except**
  `CONTROL` + `Char('s' | 'S')` with no other of those four → `Submit`. So `ctrl-c` passes (the
  house rule; MOD-52 owns whether it quits).
- `Char(c)`: inserts unless `c.is_control()` (swallowed) → `Consumed`.
- `Enter` (with or without `SHIFT`): splits `lines[row]` at `col`; the tail becomes `lines[row + 1]`;
  cursor `(row + 1, 0)` → `Consumed`.
- `Backspace`: at `col > 0` removes the char before; at `(r, 0)` with `r > 0` joins `lines[r]` onto
  `lines[r - 1]`, cursor at the old end of `r - 1`; at `(0, 0)` nothing → `Consumed`.
- `Delete`: before the end removes the char at `col`; at the end of a line that is not the last,
  joins the next line on; else nothing → `Consumed`.
- `Left`/`Right`: move one char; `Left` at column 0 goes to the end of the previous line, `Right` at
  the end goes to column 0 of the next; at the buffer ends nothing → `Consumed`.
- `Up`/`Down`: one line, the column clamped to the target line's length (no remembered goal
  column); at the first or last line nothing → `Consumed`.
- `Home`/`End`: the start or end of the current line → `Consumed`.
- `Esc` → `Cancel`. Everything else (`Tab`, `BackTab`, `PageUp`, `PageDown`, `F(n)`, …) → `Pass`.

### 4.3 Window

`lines(width, height, ..)` returns an empty `Vec` when either is 0. Vertically the window ends at the
cursor row: `top = row.saturating_sub(height - 1)`, rows `top..min(top + height, line_count)` (the
caller pads). The cursor row is drawn by `TextField::line`'s algorithm (`text_field.rs:225-277`),
copied: a window of chars ending at the cursor, a leading dim `…` when its start is clipped, the
cursor cell in `theme.selected` while `focused` (else `theme.base`). Every other row is drawn from
column 0: when it has more than `width` chars, its first `width - 1` chars and a trailing dim `…`.

### 4.4 Tests (unit, `text_area.rs` `mod tests`) and commits

| Test | Asserts |
|---|---|
| `enter_splits_the_line_at_the_cursor` | `with_text("abcd")`, `Left` ×2, `Enter` → `text() == "ab\ncd"`, `cursor() == (1, 0)`. |
| `backspace_at_column_zero_joins_the_previous_line` | `"ab\ncd"` at `(1, 0)`, `Backspace` → `"abcd"`, cursor `(0, 2)`. |
| `delete_at_the_end_joins_the_next_line` | `"ab\ncd"` at `(0, 2)`, `Delete` → `"abcd"`, cursor `(0, 2)`. |
| `up_and_down_keep_the_column_clamped` | `"abcdef\nxy\nlonger"` at `(0, 5)`: `Down` → `(1, 2)`, `Down` → `(2, 2)`; `Up` at row 0 and `Down` at the last row are `Consumed` and move nothing. |
| `left_and_right_cross_line_ends` | `Left` at `(1, 0)` → `(0, len0)`; `Right` at `(0, len0)` → `(1, 0)`; at `(0, 0)` and at the end nothing moves. |
| `ctrl_s_submits_and_esc_cancels` | `ctrl-s` → `Submit`; `Esc` → `Cancel`; neither changes the text. |
| `other_chords_tab_and_function_keys_pass` | `ctrl-c`, `alt-x`, `ctrl-alt-s`, `Tab`, `BackTab`, `F(5)` → `Pass`; the text is unchanged. |
| `a_control_char_is_swallowed_not_inserted` | `Char('\u{7}')` → `Consumed`, text unchanged. |
| `text_joins_lines_with_newline_and_with_text_round_trips` | `with_text("a\n\nb").text() == "a\n\nb"`, `line_count() == 3`; `with_text("a\r\nb\rc").text() == "a\nb\nc"`; `with_text("").is_empty()`; `new().line_count() == 1`. |
| `debug_never_prints_the_text` | `format!("{:?}", with_text("secret-ish\nnote"))` contains `line_count` and `len` and neither `secret-ish` nor `note`. |
| `the_window_keeps_the_cursor_row_visible` | Ten lines, cursor on row 9, `lines(20, 3, ..)` returns rows 7, 8, 9 (checked by content). |
| `a_long_cursor_line_is_windowed_like_a_text_field` | One 30-char line, cursor at the end, `lines(10, 1, true, ..)`: the rendered text starts with `…` and ends with the cursor cell; equals `TextField::with_text(same).line(10, true, ..)` span for span. |
| `a_multi_byte_char_counts_as_one` | `with_text("aé")`: `len() == 2`, `Backspace` leaves `"a"`, `cursor() == (0, 1)`. |

Commits: (a) red: the surface with `todo!()` bodies and every test; (b) green. Gate: `cargo test -p
htui --all-features --lib ui::text_area -- --test-threads=1`; `cargo clippy -p htui --all-features
--all-targets -- -D warnings`.

---

## 5. T3: the worker side (D45, D46)

**First failing test**: `box_settings::boxes_lists_the_demo_box_as_this_box` (integration file).
T3 starts from T1 merged (lane 1).

**Files**: `crates/htui/src/box_settings.rs` (new), `crates/htui/src/lib.rs` (`pub mod
box_settings;`, alphabetically after `pub mod app;`, `:13`), `crates/htui/src/store_worker.rs`,
`crates/htui/tests/box_settings.rs` (new, worker half).

### 5.1 `box_settings.rs`

```rust
//! The boxes behind `Settings > Boxes` (MOD-7 milestone 2, D45): every box of this user with its
//! tools and recorded spec digest, which of them is this box, and the effective probe spec.
//!
//! One read per event, one reply out; the section renders only the last snapshot and never
//! patches a row into it (`prompt_settings.rs:4-6`). Writes are `WriteStore::edit_box`, a
//! compare-and-set on `box.edit_version` (D39, D41).
//!
//! Known residue, the one `crate::prompt_settings` records (`:8-11`): a re-read that fails after
//! an applied write answers `Failed`, though the row has changed.
//!
//! Nothing here reads the clock or mints an id.

use htui_agent::box_probe::spec;
use htui_core::model::{BoxId, BoxRecord};
use htui_core::store::{CasOutcome, Result, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};
use serde_json::Value;

use crate::store_worker::{StoreReply, StoreRequest};

/// One read: this box, every box of this user, and the effective probe spec.
///
/// `PartialEq` only: [`BoxRecord`] derives no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxesSnapshot {
    /// This process's box, from `Backend::box_info`; `None` before registration.
    pub this_box: Option<BoxId>,
    /// `WriteStore::boxes`: this user's boxes by id, each with tools and recorded digest.
    pub boxes: Vec<BoxRecord>,
    /// The spec the next probe would run under.
    pub spec: SpecView,
}

/// The effective probe spec as the section shows it (D45, D51); computed here, never on the
/// render side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecView {
    /// `EffectiveSpec.digest`: compare with `BoxRecord.probe_spec_digest`.
    pub digest: String,
    /// Whether a stored `box_probe_spec` overlay is in force (stored **and** accepted).
    pub overlay: bool,
    /// `EffectiveSpec.error`: why a stored overlay was ignored (it starts with
    /// `spec::SPEC_IGNORED`).
    pub error: Option<String>,
}

/// The view of `spec::effective(spec::seed(), stored)`. Pure.
#[must_use]
pub fn spec_view(stored: Option<&Value>) -> SpecView;

/// One read (D45): `backend.box_info()` for this box, `writer.boxes()`, and
/// `backend.app_settings()` for `spec::SETTING_KEY`.
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot(backend: &Backend, writer: &Writer) -> Result<BoxesSnapshot>;

/// Serves `Boxes` and `EditBox` (D45, D46). `Err(Unreachable(DATABASE_UNREACHABLE))` offline
/// (the writer is `None`), for the read too. `EditBox` answers `Boxes` on `Applied`,
/// `BoxesStale` on `Stale`, and `BoxesStale` on `NotFound { entity: "box" }` too, so a box that
/// vanished under an open editor reaches the section as a snapshot without it.
///
/// # Errors
/// Whatever the seam reports (a `Constraint` becomes `Failed` with the tag sentence), plus
/// `Unreachable` offline and `Backend` for a request that is not one of the two.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>;

/// The two request names, in `StoreRequest` order.
pub const REQUEST_NAMES: [&str; 2] = ["boxes", "edit_box"];

/// The read's name: a refused read leaves the section with no list; a refused write leaves the
/// editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];
```

`serve` body: `writer` or `Unreachable`; `Boxes` → `Ok(StoreReply::Boxes(Box::new(snapshot(backend,
&writer).await?)))`; `EditBox { box_id, expected, edit }` → `match writer.edit_box(*box_id,
*expected, edit.clone()).await { Ok(CasOutcome::Applied(_)) => Boxes(fresh), Ok(CasOutcome::Stale(_))
| Err(StoreError::NotFound { entity: "box", .. }) => BoxesStale(fresh), Err(other) => Err(other) }`
where `fresh` is one `snapshot` after the write; the last arm is `Err(StoreError::Backend(format!("not
a box settings request: {}", other.name())))`.

### 5.2 `store_worker.rs`

- Imports (`:22-27`, `:38-42`): `BoxEdit`, `BoxId` in the model list; `use crate::box_settings::{self,
  BoxesSnapshot};`.
- `StoreRequest`, after `ProbeBox` (`:209`), and `ProbeBox`'s doc (`:205-208`) ends "The
  `Settings > Boxes` section binds it to `p`, on this box only (MOD-7 milestone 2, D49)." instead of
  "Milestone 1 binds it to no key (plan D15)":

```rust
    /// Every box of this user, this box marked, and the effective probe spec (MOD-7 milestone 2,
    /// D45): answered with [`StoreReply::Boxes`]. Served in the loop, like the catalogue reads.
    Boxes,
    /// A declared-tags or quirks edit, compare-and-set on `box.edit_version` (MOD-7 D41, D46).
    /// Answered with [`StoreReply::Boxes`] when it applied and [`StoreReply::BoxesStale`] when the
    /// token was spent or the box is gone; a refused tag list is [`StoreReply::Failed`].
    EditBox {
        /// The box, as a snapshot listed it.
        box_id: BoxId,
        /// The `edit_version` the editor opened on.
        expected: i32,
        /// Only the edited field is `Some`.
        edit: BoxEdit,
    },
```

- `name()`, after `Self::ProbeBox => "probe_box",` (`:570`): a comment "The two of
  `box_settings::REQUEST_NAMES`, in that order (MOD-7 milestone 2, D46)." and `Self::Boxes =>
  "boxes", Self::EditBox { .. } => "edit_box",`.
- `StoreReply`, after `BoxProbed` (`:784`), before `Failed`:

```rust
    /// This user's boxes, freshly read: the answer to [`StoreRequest::Boxes`] and to every box
    /// edit that applied (MOD-7 milestone 2, D46).
    Boxes(Box<BoxesSnapshot>),
    /// A box edit missed its token, or its box is gone (D46, D48): the boxes as they are now, for
    /// the editor to reload against. The editor keeps its typed text and retries only on save.
    BoxesStale(Box<BoxesSnapshot>),
```

- `try_serve`, after the connection arm (`:1056-1064`), before `StoreState` (`:1065`):

```rust
        // The two box requests, or-ed for the same reason the twenty-eight above are: a guard
        // does not count towards exhaustivity in a wildcard-free `match`, so `_ if …` would be an
        // E0004 here (MOD-15 M3 plan F-12, MOD-7 milestone 2 D46).
        StoreRequest::Boxes | StoreRequest::EditBox { .. } => {
            box_settings::serve(backend, request).await?
        }
```

The loop, `testkit.rs`'s runtime-routed list and `agent_worker.rs` end in wildcards and are not
touched (fact-checked).

### 5.3 Tests: `crates/htui/tests/box_settings.rs` (worker half, `#![cfg(feature = "testkit")]`)

Module doc in the shape of `tests/prompt_settings.rs:1-12`. Helpers: `demo() -> Backend`
(`Backend::memory(MemStore::demo())`); `boxes(reply) -> BoxesSnapshot` and `stale(reply) ->
BoxesSnapshot` (`#[track_caller]`, panicking with the reply); `refusal(reply) -> (&'static str,
String)`; `edit(box_id, expected, tags: Option<&[&str]>, quirks: Option<&str>) -> StoreRequest`;
`two_boxes() -> MemStore` (`demo_data()` plus a second box of the demo user,
`BoxId::from_uuid(Uuid::from_u128(1))`, hostname `SECOND-BOX`, through `MemStore::from_demo`).

| Test | Asserts |
|---|---|
| `boxes_lists_the_demo_box_as_this_box` | `serve(&demo(), &Boxes)` is `Boxes(s)` with `s.this_box == Some(ids::BOX)`, one record, hostname `DESKTOP-HTUI`, `edit_version == 0`, four tools; `s.spec == spec_view(None)`, `overlay == false`, `error == None`, `digest == spec::digest(spec::seed())`. |
| `boxes_lists_every_box_of_this_user_by_id` | Over `two_boxes()`: two records in id order (`SECOND-BOX` first), `this_box` still `ids::BOX`. |
| `edit_box_applied_answers_the_fresh_snapshot` | `edit(ids::BOX, 0, Some(["vulkan","gpu"]), None)` is `Boxes(s)` with the row's `declared_tags == ["gpu","vulkan"]`, `edit_version == 1`, `quirks == ""`. |
| `edit_box_with_a_spent_token_answers_boxes_stale_and_writes_nothing` | One applied edit, then the same token again with other tags: `BoxesStale(s)` whose row still has the first edit's tags and `edit_version == 1`. |
| `edit_box_on_a_vanished_box_answers_boxes_stale_without_it` | `edit(BoxId::new(), 0, Some(["gpu"]), None)` is `BoxesStale(s)` and `s.boxes` does not contain that id (and still holds `ids::BOX`). |
| `edit_box_with_an_invalid_tag_is_failed_with_the_sentence` | `edit(ids::BOX, 0, Some(["GPU"]), None)` is `Failed { request: "edit_box", message }` with `message` containing `canonical_declared_tags(&["GPU".into()]).unwrap_err()`; a following `Boxes` shows `edit_version == 0`. |
| `the_spec_view_names_a_stored_overlay_and_an_ignored_one` | `MemStore::demo()` with `set_app_setting(spec::SETTING_KEY, terraform)` (the `terraform_spec()` document of `box_probe_pg.rs:104-114`, written out): `overlay == true`, `error == None`, `digest == spec::effective(spec::seed(), Some(&terraform)).digest` and different from the seed's. With `json!(42)`: `overlay == false`, `error` starts with `spec::SPEC_IGNORED`, `digest == spec::digest(spec::seed())`. |
| `offline_both_are_refused_with_the_database_sentence` | A `Backend::Offline` over a fresh `CacheStore` in a `tempdir` (`tests/prompt_settings.rs:468-480`): `Boxes` and `EditBox` are `Failed` under `REQUEST_NAMES[0]` and `[1]`, each message containing `DATABASE_UNREACHABLE`. |
| `a_request_from_elsewhere_is_named_not_panicked` | `box_settings::serve(&demo(), &StoreRequest::BoxInfo)` is `Err(StoreError::Backend(m))` with `m` naming `box_info`. |

`store_worker.rs` unit test, beside `name_arms_are_stable` (`:2720`):
**`box_requests_are_named_as_box_settings_lists_them`**: `[Boxes.name(), EditBox { box_id:
BoxId::new(), expected: 0, edit: BoxEdit::default() }.name()] == box_settings::REQUEST_NAMES`.

### 5.4 Build coupling

T3 consumes `WriteStore::edit_box`, `BoxEdit`, `BoxRow.edit_version` (T0, T1) and milestone 1's
`htui_agent::box_probe::spec::{seed, effective, digest, SETTING_KEY, SPEC_IGNORED}`. It does not use
`TextArea`, so it is independent of T2.

### 5.5 Gate

```bash
cargo test -p htui --all-features -- --test-threads=1
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

### 5.6 Commit boundaries (T3)

1. (a) Red: `box_settings.rs` with the types, constants and `todo!()` bodies for `spec_view`,
   `snapshot`, `serve`; the two variants of each enum, `name()`, the `try_serve` arm and the
   `ProbeBox` doc; `lib.rs`; every test above.
2. (b) Green: the bodies.

---

## 6. T4: the section (D47–D51, D56, D57, D59)

**First failing test**: `activation_asks_for_boxes`. T4 starts from T2 and T3 merged (Wave B,
worktree C).

**Files**: `crates/htui/src/ui/tabs/settings/boxes.rs` (new),
`crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/app/mod.rs`,
`crates/htui/tests/settings.rs`, `crates/htui/tests/box_settings.rs` (section half, appended),
`crates/htui/tests/snapshots/box_settings__*.snap` (six new). **Not** `box_settings.rs`: any helper the
section needs lives in `boxes.rs`, so T5's worktree never races it.

### 6.1 `settings/mod.rs`

- `pub mod boxes;` in the list (`:11-17`, alphabetical: after `pub mod agents;`) and `pub use
  boxes::BoxesSection;` (`:32-37`).
- Module doc (`:3-6`): "MOD-7's box profile" becomes "MOD-7's `Boxes` section (every box of the
  user, this box marked)". The `is_error` doc (`:60-63`) adds "and the boxes section".

### 6.2 `app/mod.rs`

The import (`:14-16`) gains `BoxesSection`; `register_all` (`:49-58`) gains
`Box::new(BoxesSection::new()),` after `Box::new(QdrantSection::new()),`, with the comment "Last
(MOD-7 milestone 2, D47): appending moves no existing section's line." **In T4's last green commit
only** (F-F).

### 6.3 `boxes.rs`: state

```rust
//! `Settings > Boxes` (MOD-7 milestone 2, PRD D4, `R-TUI-8`): every box of this user with its
//! profile, tools, probed and declared tags, quirks and last probe; declared tags and quirks
//! edited as a compare-and-set a reconnect cannot stale; the probe on this box only.
//!
//! Holds no store handle and mints no id (`R-NF-3`): every `BoxId` it sends came out of a
//! [`BoxesSnapshot`].

/// The section.
#[derive(Debug, Default)]
pub struct BoxesSection {
    /// The last `Boxes` or `BoxesStale`; `None` before the first.
    snapshot: Option<BoxesSnapshot>,
    /// Why the read was refused (`Failed { request: "boxes" }`); cleared by the next snapshot.
    unavailable: Option<String>,
    /// The selected box, by id so a reload that inserts a box does not move the selection.
    selected: Option<BoxId>,
    /// Browse, or one editor.
    mode: Mode,
    /// The write in flight (D56): `Some("edit_box")` between an `EditBox` and its reply.
    busy: Option<&'static str>,
    /// A `ProbeBox` is in flight: the hint says so. Cleared by `BoxProbed` or its `Failed`.
    probing: bool,
    /// The one notice line; drawn in `theme.error` when `is_error` says so.
    notice: Option<String>,
}

/// Browse, or one editor open over one box.
#[derive(Debug, Default)]
enum Mode { #[default] Browse, Tags(Editor<TextField>), Quirks(Editor<TextArea>) }

/// An open editor: the box, the token it opened on, the value it opened on, the widget.
#[derive(Debug)]
struct Editor<W> {
    /// From the snapshot row the editor opened on.
    box_id: BoxId,
    /// `edit_version` at open; replaced only by a `BoxesStale` (D48).
    expected: i32,
    /// Tags: `canonical_declared_tags(opened list)` rendered back to a list; quirks: the
    /// normalised opening text. Never refreshed (D59, the kinds section's `stored_budget` rule).
    opened_on: Opened,
    /// The widget.
    input: W,
}
```

`Opened` is `enum Opened { Tags(Vec<String>), Quirks(String) }`, or two concrete editor structs:
the implementer's choice. `TextField` and `TextArea` both redact their `Debug`, so the derived
`Debug`s print no typed text; `opened_on` is stored data, not typed text.

Sentences (module constants): `NOT_READ = "boxes not read yet"`, `UNAVAILABLE = "boxes
unavailable"`, `NO_BOXES = "no box is registered for this user yet"`, `THIS_BOX_ONLY = "the probe
runs on this box only"`, `IN_FLIGHT = "edit_box in flight"`, and the shared `CHANGED_ELSEWHERE`,
`CHANGED_ELSEWHERE_CLOSED`, `DELETED_ELSEWHERE` from `settings/mod.rs:103-118`.

### 6.4 `boxes.rs`: behaviour

`impl SettingsSection for BoxesSection`: `ID = SectionId("boxes")`, `title() == "Boxes"`,
`wants_requests(_) == vec![StoreRequest::Boxes]` whatever the scope, `on_scope_change` does nothing,
`captures_input() == !matches!(self.mode, Mode::Browse)`.

**Keys, Browse** (`h`/`l`/`[`/`]`/arrows are the tab's before a section sees them):
- `j`/`Down`, `k`/`Up`: move `selected` within `snapshot.boxes`, stopping at the ends → `Consumed`.
- `t`: opens `Mode::Tags` over the selected row: `TextField::with_text(&row.declared_tags.join(",
  "))`, `expected = row.edit_version`, `opened_on = canonical(row.declared_tags)` (a stored list that
  fails the rule opens anyway; its refusal comes on save).
- `e`: opens `Mode::Quirks`: `TextArea::with_text(&row.quirks)`, `opened_on = that area's text()`.
- `p`: when `snapshot.this_box == Some(selected)`, `ctx.request(StoreRequest::ProbeBox)`, `probing =
  true`, notice cleared; otherwise no request and `notice = THIS_BOX_ONLY` (PRD D4).
- `r`: `ctx.request(StoreRequest::Boxes)`, always allowed (kinds' reason, `kinds.rs:1449-1456`).
- `Esc` only while a notice shows: clears it. Everything else → `Pass`.
- With no snapshot or an empty list, `t`/`e`/`p` do nothing and are `Consumed`.

**Keys, an editor open** (every key goes to the widget):
- `Submit` (`Enter` for tags, `ctrl-s` for quirks): if `busy` is set, `notice = IN_FLIGHT` and
  nothing is sent (D56). Tags: `declared_tags_from_text(text)`; `Err(sentence)` → `notice =
  sentence`, editor stays, nothing sent; `Ok(list)` equal to `opened_on` → editor closes, nothing
  sent; else `ctx.request(EditBox { box_id, expected, edit: BoxEdit { declared_tags: Some(list),
  quirks: None } })`, `busy = Some("edit_box")`. Quirks: `text()` equal to `opened_on` → close;
  else `EditBox` with `quirks: Some(text)` only. The editor stays open until the reply.
- `Cancel` (`Esc`): back to Browse, notice cleared, `busy` untouched.
- `Consumed` → `Consumed`. `Pass`: `CONTROL` chords → `Handled::Pass` (so `ctrl-c` reaches the
  shell); anything else → `Consumed` (the kinds rule, `kinds.rs:744-760`).

**Replies**:
- `Boxes(s)`: `unavailable = None`, snapshot replaced, `selected` kept if still listed, else the
  first box (or this box on the first snapshot). If `busy.take().is_some()` → an editor open over a
  box closes and the notice clears; if `busy` was `None` → the editor, its text and its token are
  untouched (D48, D56). If the editor's box is no longer listed on a plain `Boxes`, the editor stays
  (a save will answer `BoxesStale` without it, which closes it).
- `BoxesStale(s)`: `busy = None`, snapshot replaced. Editor open: its box in `s` → `expected =
  row.edit_version`, text kept, `opened_on` kept, `notice = CHANGED_ELSEWHERE`; its box absent →
  Browse, `notice = DELETED_ELSEWHERE`. No editor → `notice = CHANGED_ELSEWHERE_CLOSED`.
- `BoxProbed(_)`: `probing = false`; `ctx.request(StoreRequest::Boxes)` (the report carries counts,
  not rows; D49).
- `Failed { request: "boxes", message }`: `unavailable = Some(message)`; `busy` untouched.
- `Failed { request: "edit_box", message }`: `busy = None`, editor stays, `notice = message`.
- `Failed { request: "probe_box", message }`: `probing = false`, `notice = message`.
- Anything else: ignored.

### 6.5 `boxes.rs`: render

Layout top to bottom: body (`Min(3)`), hint (`Length(1)`), notice (`Length` of its wrapped lines,
`wrapped` from `settings/mod.rs:78`, 0 when none). With `unavailable` set the body is one line
`"{UNAVAILABLE}: {why}"` in `theme.error` (wins over any snapshot, the kinds rule); no snapshot →
`message(NOT_READ)`; an empty list → `message(NO_BOXES)`.

Otherwise the body splits horizontally: the list (`Length(28)`) and the detail (`Min(0)`).

- **List**, one row per box, the selected one in `theme.selected`: `hostname`, then ` …{last 8 hex of
  id.simple()}` **only** when another listed box has the same hostname (PRD `:262`), then ` (this
  box)` on this box. Clipped with a trailing `…` at the pane width.
- **Detail**, label column 14 wide, for the selected box:
  `host` hostname (+ ` (this box)`); `os` `{os_family} {os_version} · {arch}`; `cpu`; `ram`
  `{n} MB` or `unknown`; `gpu` the vendor when `gpu_present`, `present` without a vendor, `none`
  otherwise; `htui` `htui_version`; `last probe` `%Y-%m-%d %H:%M UTC` or `never probed` (D57);
  `spec` `probed under the current spec: yes|no` (`probe_spec_digest == Some(spec.digest)`);
  `probed tags` stored order joined by `, ` or `(none)`; `declared tags` the same, or the tag
  editor's `TextField::line` followed by one dim line `seen: …` (the sorted, deduplicated union of
  every listed box's probed and declared tags, D50); `quirks` `(none)`, or each line of the note (the
  first on the label row, the rest indented to the value column), or the quirks editor's
  `TextArea::lines(width, 6, true, theme)`; `tools` `name version` (bare name for an empty version)
  joined by `, `, wrapped with `wrapped`. Last, one line for the spec: `probe spec: seed · {first 12
  hex}` or `probe spec: seed + stored overlay · {first 12 hex}`, and when `spec.error` is `Some`, a
  second line with it in `theme.error` (D51).
- **Hint** (dim): Browse `j/k move · t tags · e quirks · p probe this box · r reload` (plus ` ·
  probing…` while `probing`); tags `Enter saves · Esc cancels · comma-separated`; quirks `ctrl-s
  saves · Esc cancels · Enter breaks the line`; while `busy`, `saving…`.

### 6.6 Tests: `crates/htui/tests/box_settings.rs` (section half) and `tests/settings.rs`

Helpers: `bench_with(snapshot) -> (SectionBench, BoxesSection)` feeding one `Boxes` reply and
draining; `snap_of(store)` = `box_settings::snapshot` over a `Backend::memory`; `boxes_over(store)
-> Harness` = `Harness::over(store).with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
BoxesSection::new())])))` then `settle()`; `requests(bench) -> Vec<StoreRequest>` from
`Action::Store`. Every snapshot assertion runs inside `insta::with_settings!({ filters =>
vec![(r"\b[0-9a-f]{12}\b", "<digest>")] }, { … })` (D57).

| Test | Asserts |
|---|---|
| `activation_asks_for_boxes` | `BoxesSection::new().wants_requests(&scope)` is exactly `[Boxes]` for two different scopes. |
| `j_and_k_move_the_selection` | Over the two-box snapshot: `j` then `k` move between the two rows and stop at the ends (`render_section` shows the selected row). |
| `t_opens_the_tag_editor_prefilled_and_enter_sends_only_the_tags` | `t` → `captures_input()`; the frame shows `gpu`; type `, vulkan`, `enter` → exactly one `EditBox { box_id: ids::BOX, expected: 0, edit: BoxEdit { declared_tags: Some(["gpu","vulkan"]), quirks: None } }`. |
| `e_opens_the_quirks_editor_and_ctrl_s_sends_only_the_quirks` | `e`, type `a`, `enter`, `b`, `ctrl-s` → one `EditBox` with `quirks: Some("a\nb")` and `declared_tags: None`. |
| `unchanged_text_closes_the_editor_without_a_request` | `t`, `enter` → no request, Browse. `t`, type `, gpu`, `enter` (same list) → no request. `e`, `ctrl-s` → no request. |
| `an_invalid_tag_keeps_the_editor_open_and_sends_nothing` | `t`, type `, Bad Tag`, `enter` → no request, still `captures_input()`, the rendered notice contains `` `Bad Tag` ``. |
| **`a_reload_between_open_and_save_keeps_the_editor_token`** | A snapshot whose row has `edit_version = 3`; `t`; feed a `Boxes` whose row has `edit_version = 3` and new `updated_at`, `last_seen_at`, `os_version`, `last_probed_at`; the editor is still open with its text; `enter` → `EditBox { expected: 3, .. }`. The section half of the PRD metric. |
| `boxes_stale_keeps_the_text_takes_the_new_token_and_says_changed_elsewhere` | Open at 0, type, save (busy); feed `BoxesStale` whose row is at 5; the editor is open with the typed text, the notice is `CHANGED_ELSEWHERE`; `enter` → `EditBox { expected: 5, .. }` with the same tags. |
| `a_stale_snapshot_without_the_box_closes_the_editor` | Open; `BoxesStale` whose list lacks the box → Browse, notice `DELETED_ELSEWHERE`. With no editor open, `BoxesStale` → `CHANGED_ELSEWHERE_CLOSED`. |
| `the_reply_to_a_save_closes_the_editor` (D56) | Open, type, `enter` (busy); feed `Boxes` → Browse, no notice. |
| `a_second_save_while_saving_sends_nothing` (D56) | Open, type, `enter`, `enter` → exactly one `EditBox`; the notice says `edit_box in flight`. |
| `p_on_this_box_sends_probe_box` | Select `ids::BOX` (this box), `p` → exactly `[ProbeBox]`; the hint shows `probing…`. |
| `p_on_another_box_sends_nothing_and_says_why` | Select `SECOND-BOX`, `p` → no request; notice `the probe runs on this box only`. |
| `a_box_probed_reply_asks_for_boxes_again` | Feed `BoxProbed(BoxProbeReport::default())` → exactly `[Boxes]`; `probing` cleared (hint). |
| `a_failed_probe_box_lands_on_the_notice_line` | Feed `Failed { request: "probe_box", message: "…" }` → the notice shows the message. |
| `the_section_captures_input_only_while_an_editor_is_open` | Browse false; `t` true; `esc` false; `e` true; `ctrl-s` with changed text still true until the reply. |
| `ctrl_c_passes_through_an_open_quirks_editor` | `e`, then `ctrl-c` → `Handled::Pass` (asserts `Pass` only: nothing in the shell quits on `ctrl-c`, MOD-52). |
| snapshot `box_settings__demo` | `boxes_over(MemStore::demo())` rendered: the strip ` Boxes `, `DESKTOP-HTUI (this box)`, `windows 10.0.26200 · x86_64`, `last probe`, `probed under the current spec: no`, tools `cargo 1.98.0, cmake, git 2.51.0, rustc 1.98.0`. |
| snapshot `box_settings__two_boxes` | A hand-built `BoxesSnapshot` of two rows with the hostname `DESKTOP-HTUI` and ids `from_u128(1)` and `ids::BOX` → both list rows carry their `…` id suffix, one `(this box)`. `render_section(&section, 100)`. |
| snapshot `box_settings__offline` | `Harness::over_backend(Backend::Offline{..})` under `mock_keyring` (F-G) → `boxes unavailable: store unreachable: this box browses …`. |
| snapshot `box_settings__tag_editor` | After `t` and typing `, vulkan`: the field line and the dim `seen: cmake, gpu, msvc, rust` line. |
| snapshot `box_settings__quirks_editor` | After `e` and two typed lines. |
| snapshot `box_settings__stale` | After `BoxesStale` over an open editor: the kept text and the `changed elsewhere …` notice. |

`tests/settings.rs`: `the_section_strip_fits_the_frame` (`:948-965`) gains
`Box::new(BoxesSection::new())` last (and the import at `:11-14`); its doc (`:935-946`) is rewritten:
seven sections (`Agents`, `Hierarchy`, `Kinds`, `Prompt`, `Connection`, `Qdrant`, `Boxes`) cost 61 of
the 100 columns. `connection.rs`'s stale "five sections" doc is not T4's file and is left for the
main thread (plan disagreement 3).

### 6.7 Build coupling

T4 consumes T2's `TextArea` and `FieldOutcome` (existing), T3's `BoxesSnapshot`, `SpecView`,
`REQUEST_NAMES`, `READ_NAME`, `StoreRequest::{Boxes, EditBox, ProbeBox}`, `StoreReply::{Boxes,
BoxesStale, BoxProbed, Failed}`, T0's `BoxEdit`, `declared_tags_from_text`,
`canonical_declared_tags`. It edits nothing T5 reads.

### 6.8 Gate and commits

```bash
cargo test -p htui --all-features -- --test-threads=1
cargo insta test -p htui --all-features --review   # accept exactly the six box_settings__* files
git status --porcelain crates/htui/tests/snapshots | grep -v 'box_settings__'   # nothing
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

1. (a) Red: `boxes.rs` skeleton (id, title, `wants_requests`, `captures_input` real; `on_key` →
   `Pass`, `on_reply` and `render` no-ops, **no** `todo!()`), the `settings/mod.rs` lines, the strip
   test, every section test. Not registered (F-F).
2. (b) Green: keys and replies.
3. (c) Green: render, the six snapshots, `register_all`.

---

## 7. T5: Postgres end to end (PRD metric "Declared tags and quirks CAS")

**Files**: `crates/htui/tests/box_probe_pg.rs` only. T5 starts from T3 merged (Wave B, worktree D) and
needs nothing of T4. `box_probe_pg.rs` uses runtime `sqlx::query*` calls; `crates/htui` has no
`.sqlx/`.

### 7.1 Additions to the file

- Module doc (`:1-23`): one paragraph naming milestone 2's four cases, driven through
  `htui::store_worker::serve` with `Boxes`/`EditBox` over the same `Stack`.
- Imports: `htui::box_settings::BoxesSnapshot`, `htui::store_worker::{StoreRequest, serve}`,
  `htui_core::model::{BoxEdit, UserId}`.
- `Stack` helpers (inside `impl Stack`, after `boxes_of_this_user`, `:241-248`):
  - `async fn serve(&self, request: StoreRequest) -> StoreReply { serve(&self.backend,
    &request).await }`.
  - `async fn edit_version(&self, id: BoxId) -> i32` and `async fn declared(&self, id: BoxId) ->
    (Vec<String>, String)` (tags and quirks by SQL).
  - `async fn plant_box(&self, owner: UserId, hostname: &str) -> BoxId` (the `INSERT INTO box (id,
    user_id, hostname, os_family, os_version, arch, htui_version) …` of
    `htui-store/tests/box_identity.rs:525-536`) and `async fn plant_user(&self) -> UserId`.
- Free fns `applied(reply) -> BoxesSnapshot` and `stale(reply) -> BoxesSnapshot`, panicking with the
  reply.

### 7.2 Tests

| Test | Asserts |
|---|---|
| `an_open_editor_survives_a_reconnect_and_a_registration_probe` | `the_report(&stack.swap().await)` (first probe). `s = applied(Boxes)`; `token = row(this box).edit_version` (0); `updated_at` read as text. `register_box(&Identity { box_id, hostname: "reconnected" }, None)` as `a_renamed_box_keeps_its_row_and_its_probe` does (`:388-399`); the `terraform_spec()` row inserted into `app_setting` (as `a_stored_spec_change_reprobes`, `:436-441`); `the_report(&stack.swap().await)` (the registration probe re-probes and writes). `updated_at` and `probe_spec_digest` have moved. `EditBox { box_id, expected: token, edit: tags ["gpu","heavy_build"] }` answers `Boxes` whose row has those tags and `edit_version == token + 1`; the hostname is `reconnected` and the probe columns are the second probe's. |
| `a_concurrent_edit_is_reported_and_never_overwritten` | Both editors read token 0. The first `EditBox` (tags `["gpu"]`) answers `Boxes`; the second (tags `["vulkan"]`, token 0) answers `BoxesStale` whose row has `["gpu"]` and `edit_version == 1`; SQL still reads `{gpu}`. |
| `another_box_of_this_user_is_editable` | `plant_box(this_user, "SECOND")`; `Boxes` lists two; `EditBox` on the planted box with its token (0) and quirks `"x"` answers `Boxes`; SQL reads quirks `x`, `edit_version 1` on it and nothing changed on this box. |
| `another_users_box_is_not_listed_and_not_editable` | `plant_user`, `plant_box(stranger, "ELSEWHERE")`; `Boxes` does not list it; `EditBox` on it answers `BoxesStale` without it; SQL reads its tags, quirks and `edit_version` unchanged. |

### 7.3 Gate and commit

```bash
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features --test box_probe_pg -- --test-threads=1
```

One commit. The four cases are expected green on arrival (they verify T1 and T3 on Postgres); a red
one is a T1 or T3 defect, reported to the main thread rather than patched from T5's worktree. The
live check (§9) is not run here.

---

## 8. Cross-task contracts

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T0 `htui_core::model` | `BoxRow.edit_version: i32` | T1, T3, T4, T5 |
| T0 | `BoxEdit { declared_tags: Option<Vec<String>>, quirks: Option<String> }` (`Debug, Clone, Default, PartialEq, Eq`) | T1, T3, T4, T5 |
| T0 | `DECLARED_TAG_MAX: usize = 64`, `is_declared_tag(&str) -> bool`, `canonical_declared_tags(&[String]) -> Result<Vec<String>, String>`, `declared_tags_from_text(&str) -> Result<Vec<String>, String>` | T1 (stores), T4 (section) |
| T1 `WriteStore` | `edit_box(&self, id: BoxId, expected: i32, edit: BoxEdit) -> Result<CasOutcome<BoxRow>>`; `NotFound { entity: "box" }` for an unknown or foreign box; precedence NotFound, Stale, Constraint | T3, T5 |
| T2 `htui::ui` | `TextArea::{new, with_text, on_key -> FieldOutcome, text, is_empty, len, line_count, cursor, lines(width, height, focused, &Theme)}` | T4 |
| T3 `htui::box_settings` | `BoxesSnapshot { this_box, boxes, spec }`, `SpecView { digest, overlay, error }`, `spec_view`, `snapshot`, `serve`, `REQUEST_NAMES = ["boxes", "edit_box"]`, `READ_NAME` | T4, T5 |
| T3 `htui::store_worker` | `StoreRequest::{Boxes, EditBox { box_id, expected, edit }}`, `StoreReply::{Boxes(Box<BoxesSnapshot>), BoxesStale(Box<BoxesSnapshot>)}` | T4, T5 |

**Parallel-lane hazards**:
- Wave A: T2 touches only `ui/text_area.rs` and `ui/mod.rs`; lane 1 touches `htui-core`,
  `htui-store`, `htui-agent` (T1) and `box_settings.rs`, `lib.rs`, `store_worker.rs`,
  `tests/box_settings.rs` (T3). `lib.rs` declares `pub mod ui;` and none of its children, so T2
  leaves it alone. T2's worktree lacks T1, which it does not need.
- Wave B: T4 touches the section, `settings/mod.rs`, `app/mod.rs`, `tests/settings.rs`,
  `tests/box_settings.rs` and six new snapshots; T5 touches only `tests/box_probe_pg.rs`. No shared
  file, no shared generated artefact.
- `.sqlx` moves in T0 and T1 only (serial). `StoreRequest`/`StoreReply` move in T3 only. `CASES` and
  its two pins move in T1 only. Snapshots are created in T4 only, under the new prefix
  `box_settings__`.
- Two or three worktrees mean two or three `target/` directories: check `df -h /` before each wave
  (92 GB free at `a4a8d8e`; project memory: disk pressure crash-loops the dev Postgres).

---

## 9. Merge order and the workspace gate

T0 → [T1 → T3] ∥ T2 → merge T1 (core, store, agent gates), T2 (htui lib gate), T3 (htui gate) →
[T4 ∥ T5] → merge T4 (htui gate), T5 (the Postgres `box_probe_pg` suite) → workspace gate → live
check.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l                          # 264
ls crates/htui/tests/snapshots/*.snap | wc -l               # 81
cargo doc --workspace --no-deps --keep-going                # exactly the two baseline errors
git diff --stat 9a4911a -- crates/htui-store/migrations crates/htui-store/cache_migrations   # empty
git diff --stat 9a4911a -- crates/htui/tests/snapshots | grep -v box_settings__             # nothing moved
```

Before believing a Postgres failure, run `df -h /` and re-run the case alone. The live check is the
plan's (§Validation), unchanged, on the real tree after T4 and T5 are both merged.

---

## 10. Count pins

| Pin | Now | After | Task |
|---|---|---|---|
| Store `CASES` | 68 | 71 | T1 (`conformance.rs:42-111`, `mem_store.rs:37`, `pg_conformance.rs:19`) |
| `READ_CASES` | 14 | 14 | — |
| `htui-orch` `CASES` | 70 | 70 | — |
| `StoreRequest` variants | 64 | 66 | T3 |
| `StoreReply` variants | 35 | 37 | T3 |
| `.sqlx` files | 263 | 263 (T0: three hashes change) → 264 (T1) | T0, T1 |
| Migrations | `0001`..`0006` | unchanged; next `0007` | — |
| `MIRRORED_TABLES` | 21 | 21 | — |
| `crates/htui/tests/snapshots/*.snap` | 75 | 81 (`box_settings__demo`, `__two_boxes`, `__offline`, `__tag_editor`, `__quirks_editor`, `__stale`) | T4 |
| Settings strip | 6 sections, 54 columns | 7 sections, 61 columns | T4 (`tests/settings.rs:948-965`) |
| Settings sections registered | 6 | 7 (`Boxes` last) | T4 (`app/mod.rs:49-58`) |

`HANDOFF.md:31-32` and `:470` are the main thread's to update.

---

## 11. Decisions (D54 onward) and risks (R-29 onward)

| # | Decision |
|---|---|
| D54 | The conformance cross-reference scanner reads `htui-store/tests/box_identity.rs` at run time, like `pg_criteria.rs`; case docs cite the Postgres halves as `box_identity.rs::<name>` (F-A). |
| D55 | `canonical_declared_tags` is the strict store-side rule; `declared_tags_from_text` is the lenient text parser layered on it; one private sentence builder (F-B). |
| D56 | `BoxesSection.busy` attributes a `Boxes` reply to a save (the kinds section's H-9 rule): only a reply while `busy` closes an editor; a second save while `busy` sends nothing (F-C). |
| D57 | Box settings snapshots filter 12-hex digests to `<digest>`; times render absolute UTC (F-D). |
| D58 | `PgStore::edit_box` re-reads through `box_row` filtered to `this_user` on a miss and on an invalid tag list; one new statement (F-H). |
| D59 | "Unchanged" compares parsed tag lists and normalised quirks text; `TextArea::with_text` normalises `\r\n` and `\r` (F-I). |
| D60 | `TextArea` `Up`/`Down` keep no goal column; the column is clamped per move. |
| D61 | `TextArea`'s vertical window ends at the cursor row; rows other than the cursor's clip with a trailing `…`. |
| D62 | The section selects by `BoxId`, not by index, so a reload that inserts a box does not move the selection. |
| D63 | `p` sends `ProbeBox` even while `probing`; the runtime's `BOX_PROBE_RUNNING` refusal is the answer (no second local sentence). |
| D64 | `EditBox` and `Boxes` are placed after `ProbeBox` in `StoreRequest`; `Boxes`/`BoxesStale` after `BoxProbed` in `StoreReply`; the `try_serve` arm after the connection arm. |
| D65 | T4's red commit registers nothing and has no `todo!()` in a shell-called method; `register_all` changes in its last commit (F-F). |
| D66 | T5's four cases are verification of T1 and T3 and land in one green commit; a failure is routed back, not patched in T5. |

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-29 | H-9's residue (D56): a read that lands between a save and its reply (a scope change or tab re-activation) is taken for the reply and closes the editor; if the save then comes back `BoxesStale`, the retry affordance is lost. | Low | `CHANGED_ELSEWHERE_CLOSED` says so and nothing was written; the kinds section carries the same, argued at `kinds.rs:1322-1340`. |
| R-30 | Postgres infers a `RETURNING` column of the new `UPDATE` as nullable and `query_as!(BoxRow, ..)` fails to compile. | Low | `!` overrides on that column only (§3.3); `set_requirement_spec`'s `UPDATE … RETURNING` compiles without any. |
| R-31 | A stored `declared_tags` written by SQL that fails the rule (for example `GPU`) opens in the tag editor and cannot be saved unchanged-but-fixed without editing it. | Low | The editor opens anyway and the refusal names the tag; the user fixes it in place. Nothing lower-cases it silently. |
| R-32 | The seed digest in the spec line moves whenever `spec.json` changes. | Certain over time | D57's filter keeps snapshots still; the text assertions check the prefix only. |
