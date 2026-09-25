# Blueprint: MOD-9 milestone 1, "templates are editable"

**Status**: proposed (2026-09-25). Findings F-A to F-U (§0) and decisions D16–D36 (§12) are proposed **Implemented** `e971418`..`caacc96` (2026-09-25).
here. A finding marked **Blocker** means the plan, read literally, does not compile, fails its own
named test, breaks the workspace gate, leaves a hidden coupling between the parallel tasks, or cannot
be validated at all. The Fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-9-templates-editable.plan.md`, confirmed 2026-09-25 with every OQ
default (OQ-1..OQ-7). Its D1–D15, T1–T6, file sets and Verified-claims table are authoritative except
where §0 amends them. **PRD**: `.claude/prds/mod-9-skill-library-templates.prd.md`, milestone 1. PRD
D1–D6 win over this blueprint where they disagree.

**Verified at**: HEAD `f935153`, branch `claude/amazing-rubin-uipggb`.
`git diff --stat da30927 HEAD -- crates Cargo.toml Cargo.lock` is empty, so the plan's `file:line`
citations still hold. Every signature below was checked by opening the real symbol with
`grep`/`sed`/`Read`. **Line numbers are pre-edit**: a citation into a file a task edits moves after
that task's first commit. `crates/htui-store/.sqlx/` holds **234** files. `df -h /` shows 28 GB free.
Two probes ran on this box's Postgres 16.13 at `localhost:5432` (scratch databases, both dropped):
the CAS insert of §2.4 (sequential, new-name, `Some(0)`, FK and a two-session race), and a
`cargo sqlx prepare --check` that failed on `ort-sys` (F-A).

**Graphify**: `graphify-out/` does not exist and Gortex is not reachable in this session. Nothing
here comes from a code graph.

**Scope**:
- **Order**: Wave A is T1 ∥ T2 ∥ T3, one worktree each. Merge T1, then T2, then T3, re-running the
  touched crates' gates on the real tree after each merge. Then T4, then T5, then T6, all serial on
  the merged tree. **Precondition for Wave A: F-A is resolved.**
- **No migration.** `TABLES`, applied/pending migration pins and commented-column pins do not move.
- **One new `WriteStore` method**: `append_prompt_template`, across five implementors.
- **New modules**: `htui::editor` (T3), `htui::templates` (T4), `htui::ui::text_area` (T2),
  `htui::ui::diff` (T5), `htui::ui::tabs::skills::templates` (T5).
- **Request enums**: `StoreRequest` 64 → 66, `StoreReply` 35 → 37 (T4). No test pins either count
  (F-Q).
- **Dependencies**: none new in the lock. `tempfile` moves from dev- to normal dependency of `htui`
  (T3, lock unchanged); `similar = { workspace = true }` is declared by `htui` (T5; the lock's `htui`
  entry gains one line, F-F).
- **Pins that move**: store `CASES` 56 → 59 (T1: `conformance.rs`, `mem_store.rs:36-37`,
  `pg_conformance.rs:19`); `.sqlx` 234 → 235 (T1). `READ_CASES` stays 9.

**House style (carried)**:
- `unsafe_code = "forbid"`. Every lib root warns on `missing_docs`; the workspace warns on
  `missing_debug_implementations` and `unused_qualifications`; rustdoc denies
  `broken_intra_doc_links`, `private_intra_doc_links`, `redundant_explicit_links`
  (`Cargo.toml` `[workspace.lints.*]`). The gate runs clippy with `-D warnings`.
- `rustfmt.toml`: `edition = "2024"`, `max_width = 100`. Toolchain pinned `1.98.1`.
- A hand-written `Debug` that prints lengths, never text, for anything holding user text
  (`TextField`'s rule, `ui/text_field.rs:56-64`).
- Nothing sets `updated_at` by hand. No `std` guard is held across an `.await`.
- `parse` is the only template validator.
- Implementers commit incrementally, staging their own paths only (never `-A`, never `stash`). Every
  commit compiles; a red commit uses `todo!()` bodies.
- Every gate re-runs with `--test-threads=1` on the real tree after the merge (the keyring fake is
  process-wide).
- **No intra-doc link (`[`X`]`) from one Wave A task to an item another Wave A task creates**: each
  worktree runs `cargo doc` alone. Name such items in plain backticks.

---

## 0. Findings the plan fact-check missed

| # | Severity | Plan says | Tree at `f935153` | Fix |
|---|---|---|---|---|
| **F-A** | **Blocker** (no gate can run) | Validation: `cargo test -p htui-store …`, `cargo sqlx prepare`, the `htui` suite. | `htui-store` declares `ort = "=2.0.0-rc.4"` with `download-binaries` (`crates/htui-store/Cargo.toml`); `ort-sys`'s build script GETs `https://parcel.pyke.io/…/ortrs-msort_static-v1.18.1-x86_64-unknown-linux-gnu.tgz`, and this sandbox's proxy refuses the host ("Proxy failed to connect"). `~/.cache/ort.pyke.io/dfbin/…` is empty. `cargo check -p htui-store` fails, so neither `htui-store` nor `htui` builds here. `htui-core`, `htui-agent` and `htui-orch` do not depend on it and still build. | Before Wave A the maintainer adds **`parcel.pyke.io`** to the environment's allowed domains (Network access in the cloud environment's settings), or runs the milestone on a box that can reach it. **Resolved in this sandbox (2026-09-25), no repo change** — every gate command runs after `. /root/ort/env.sh`: `ORT_LIB_LOCATION=/root/ort/lib` pointing at `libonnxruntime.so.1.18.0` taken from the `onnxruntime-node` 1.18.0 npm tarball (npm is reachable), plus `LD_LIBRARY_PATH` for test binaries; `cargo build --workspace --all-targets` then finishes. Both are exported from `~/.bashrc`. |
| **F-B** | **Blocker** (stores disagree) | D3: `… WHERE COALESCE((SELECT max(version) …), 0) = COALESCE($6::int, 0)`. | Probed on 16.13: with `expected = Some(0)` on a name with no row, that shape **inserts v1**. `MemStore` under D4 answers `NotFound` for `(None head, Some(_))`, so the same call succeeds on one store and fails on the other. | D16: compare with `IS NOT DISTINCT FROM $6::int` (§2.4). Probed: `Some(0)` on a new name → `INSERT 0 0` → head `None` → `NotFound`, as `MemStore`. |
| **F-C** | **Blocker** (hidden T2↔T5 coupling) | D7: `lines(width, height, focused, &Theme)` "scrolls `top`/`left` so the cursor is visible". | Scrolling writes `top`/`left`, i.e. `&mut self`, but `Tab::render` is `&self` (`registry.rs:49`). T2 compiles and passes alone with `&mut self`; T5 then cannot call it from `render`. | D19: `top` and `left` are `Cell<usize>`; `lines(&self, …)`. The cursor-follow memory stays. |
| **F-D** | **Blocker** (fails its own test) | T3 test `a_missing_program_is_failed_and_names_visual_and_editor`; D9: `Failed` for "a spawn error (… — set $VISUAL or $EDITOR)". | D8 runs every editor through `sh -c`. With `EDITOR=/nonexistent`, `sh` spawns fine and exits **127** (126 for "not executable"), so no spawn error ever reaches `run`, and the test (and the live check's "the spawn message") fails. | D23: on Unix, exit 127/126 is reported with the start-failure sentence that names `$VISUAL`/`$EDITOR`; on Windows, `cmd`'s 9009. A real spawn error (no `sh`) keeps the same sentence. |
| **F-E** | Major (gate: `-D warnings`) | D12: `diff_style` is "moved … and imported back there". | `transcript.rs:13` `use ratatui::style::Style;` is used only by `diff_style` (`:628`). After the move it is unused, and `unused_imports` fails clippy. | D29: T5 deletes `transcript.rs:13` in the same commit that moves `diff_style`. |
| **F-F** | Major (gate literally false) | "No new package in `Cargo.lock`", checked by `git diff --exit-code Cargo.lock` (Validation, Acceptance). | The lock's `name = "htui"` entry lists its dependencies; `similar` is absent (two versions are locked, 2.7.0 and 3.2.0). Declaring it in T5 adds the line `"similar 3.2.0",` there, so `--exit-code` fails. (`tempfile` is already listed, so T3's check does hold.) | D28: T3 keeps `git diff --exit-code Cargo.lock`. T5's check is "the diff is exactly one added line, `"similar 3.2.0",`, inside the `htui` entry, and `grep -c '^name = ' Cargo.lock` is unchanged". |
| **F-G** | Major (a named snapshot cannot show its point) | T5 snapshot `templates__unknown_placeholder_cursor`: "the highlighted cursor cell on the `{`". | `Harness::render` returns `buffer_text` (`testkit.rs:534-541`): symbols only, no styles. The Harness exposes no view state (`app()` holds `Box<dyn Tab>`). The cursor cannot be observed. | D20: the editor's hint row ends with `L{line}:C{col}` (1-based, chars) from `TextArea::cursor_line_col`. Snapshots and asserts read it. |
| **F-H** | Major (T1 fails `htui-core`'s own gate) | T1 would document the Postgres-only race next to the cases. | `conformance.rs::every_cross_referenced_test_name_exists` (`:7782`) panics on any backticked `<file>.rs::<name>` whose file is not `conformance`, `mem` or `pg_criteria`, and on any bare backticked snake_case span with ≥4 underscores that `conformance.rs`/`mem.rs` do not define. `` `prompt_template_cas.rs::two_saves_at_one_head_write_one_row` `` and `` `two_saves_at_one_head_write_one_row` `` both trip it. | D30: in `conformance.rs`, name that test in plain words ("the Postgres race case in `htui-store`'s tests"). The three new case names are defined in `conformance.rs`, so backticking them is fine. |
| **F-I** | Major (terminal left cooked) | D9: `Resume` guard's `Drop` calls `enter` "unless panicking", on every path out. | `Drop` cannot return an error, so a failed `enter` (raw mode refused, `EnterAlternateScreen` write error) is swallowed and the loop keeps drawing into a cooked, non-alternate terminal. | D21: `run_suspended` returns `io::Result<ExternalEditOutcome>`. The normal path disarms the guard and calls `enter()` with `?`; `Drop` re-enters only for a dropped (cancelled) future. The event loop propagates the error, and `lib.rs` restores and exits. |
| **F-J** | Major (a read taken for the save) | D11: "`Templates` while `busy` → close the editor, select the saved name at its new head." | `Tab` away and back (or `2`) runs `activate_tab` → `wants_requests` → a `Templates` read (`state.rs:270-279`). It has a different discriminant from `SaveTemplate`, so both replies are fresh (`state.rs:304-308`), and a read served before the save would close the editor as "saved" over a snapshot without the new version. | D27: a `Templates` reply closes the editor only if `busy == Some("save_template")` **and** the snapshot's head of `(project, name)` has `version > token.unwrap_or(0)`. Otherwise it refreshes the snapshot and keeps the editor, the token and `busy`. |
| **F-K** | Minor | D11: "`Unchanged` → 'no changes' (plus 'use `code --wait`' when the editor exited in under a second)". | D9's `ExternalEditOutcome::Unchanged` carries nothing, so the view cannot know the elapsed time. | D24: `Unchanged { quick: bool }`, where `quick` means the editor returned within `QUICK_EXIT = 1 s`. |
| **F-L** | Minor | D9: `prefix(&format!("htui-{stem}-"))` with `stem` = the template name. | Names are free text (`kind.rs` column doc; D4 refuses only blank, edge whitespace and CR/LF). A name with `/` makes `tempfile` try a subdirectory, and `Builder::tempfile` fails. | D25: `run` sanitises `stem` to `[A-Za-z0-9_-]` (anything else becomes `_`), capped at 32 chars. |
| **F-M** | Minor (dead-code warning) | D8: `EditorCommand { value, fallback }`. | Nothing in D8–D11 reads `fallback`, so `dead_code` fires under `-D warnings`. | D23: the start-failure sentence reads it (§4.3). |
| **F-N** | Minor | T1 files: `traits.rs` gains `invalid_template_name` "beside `reserved_phase_name`". | `pg/write.rs` imports helpers through `htui_core::store::{…}`, and that list is an explicit re-export (`store/mod.rs:10-18`). | T1 adds `crates/htui-core/src/store/mod.rs` to its file set (no Wave A intersection) and re-exports D17's three helpers. |
| **F-O** | Minor | T3 test `a_failed_leave_spawns_nothing_and_enters_nothing`. | `ratatui::try_restore` disables raw mode **then** leaves the alternate screen (`ratatui-0.30.2/src/init.rs:554-560`). A failure on the second step leaves raw mode off and the alternate screen up, and "enter nothing" leaves the loop drawing cooked. | D21: a failed `leave` calls `enter()` once (`?`) and answers `Ok(Failed(..))` without spawning. The test becomes `a_failed_leave_spawns_nothing_and_enters_once`. |
| **F-P** | Minor | D9: `leave` = `ratatui::try_restore()`. | Every draw hides the cursor (`ratatui-core-0.1.2/src/terminal/render.rs:298`), and `try_restore` does not show it again, so a plain editor (`nano`, `ed`) starts with no visible cursor. | D22: `leave` calls `self.terminal.show_cursor()` before `try_restore()`. The next draw hides it again. |
| **F-Q** | Minor (record) | Count-pin table: "`StoreRequest` variants 64 → 66", "`StoreReply` 35 → 37". | Recounted (64 and 35), but **no test pins them**. The only exhaustive matches are `name()` (`store_worker.rs:549-620`) and `try_serve` (`:974`), plus `observe_reply`/`on_reply`, which use `_`. | Nothing to move; the counts are recorded in §10 only. |
| **F-R** | Minor (accepted) | D9: "the stream must go before the child starts". | `EventStream::drop` only sets a flag and wakes the reader (`crossterm-0.29.0/src/event/stream.rs:140-145`). The helper thread returns from `poll_internal` and exits a few µs later, asynchronously. | Accepted as **R-9** (§11). No barrier: `crossterm::event::poll(d)` would itself read the tty for `d`. |
| **F-S** | Minor (placement) | T3: "`state.rs` unit tests". | `state.rs` has no test module. The shell test scaffold (`Recorder`, `shell()`, `next_items_seq`) is private to `update.rs`'s `mod tests` (`:350-444`), which T3 already edits. | D26: T3's three shell tests go in `update.rs`'s `mod tests`. |
| **F-T** | Minor (disk) | Wave A: three worktrees. | 28 GB free, and three cold `target/` directories of this workspace do not fit comfortably. | D32: all three worktrees share one `CARGO_TARGET_DIR`. Registry dependencies are then built once; builds serialise on cargo's lock. `df -h /` must show ≥ 15 GB before Wave A. |
| **F-U** | Minor (pre-existing, **out of scope**, routed to the main thread) | Plan D9: "the panic path is already covered: the hook restores (`terminal.rs:35-43`)". | `terminal::init` installs htui's **conditional** hook first and then calls `ratatui::init()`, whose `try_init` wraps it in ratatui's **unconditional** `restore()` hook (`ratatui-0.30.2/src/init.rs:397-403`, `:566-572`). A panic that `htui_agent::excerpt::run_providers` contains (review M1, H-20) therefore still restores the terminal mid-session, and `restores_the_terminal()` is never consulted first. `tests/panic_hook.rs` tests only the predicate. | Not MOD-9's to fix: record it for a CLEAN/TOOL item (for example `ratatui::try_init_with_options` without a hook, or installing htui's hook after `ratatui::init` with `take_hook` dropping ratatui's). T3 must not make it worse: `enter` never calls `ratatui::init` (D22). |

### 0a. Settled answers to the brief's questions

| Question | Answer | Where |
|---|---|---|
| Are Wave A's file sets disjoint? | Yes. T1 is `htui-core`/`htui-store`/`htui-agent` only (plus `store/mod.rs`, F-N). T2 is `ui/text_area.rs` and `ui/mod.rs`. T3 is `editor.rs`, `terminal.rs`, `event_loop.rs`, `app/{action,state,update}.rs`, `ui/tabs/registry.rs`, `lib.rs` and `Cargo.toml`. No path appears twice. | §1, §8 |
| Does each compile and pass its gate alone on the base tree? | **T1**: yes. The five `WriteStore` implementors are all inside T1 (`grep -rn 'impl.*WriteStore for' crates/` → `pg/write.rs:384`, `mem.rs:4420`, `writer.rs:288`, `htui-agent/src/conformance.rs:674`, `htui-agent/tests/recorder.rs:354`), and no crate outside T1 implements the trait. **T2**: yes; it uses only `crate::ui::Theme`. **T3**: yes; it needs no `TextArea`, and `on_external_edit` is defaulted. The one hidden coupling was F-C, which bites only at T5. | §2.8, §3.5, §4.8 |
| T2's `ui/mod.rs` vs T5's | Serial (T5 runs after T2 is merged). T2 adds `pub mod text_area;` and a `pub use`; T5 adds `pub mod diff;`. | §6.1 |
| Tests pinning 64/35 | None exist (F-Q). | §10 |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (minimum; each compiles) | Gate |
|---|---|---|---|
| T1 writer | htui-core, htui-store, htui-agent (worktree A) | 3 (§2.9) | core, store (Postgres), agent tests; prepare + `--check`; clippy for the three crates; `cargo check --workspace` |
| T2 `TextArea` | htui `ui/` (worktree B) | 2 (§3.6) | `cargo test -p htui --all-features --lib ui::text_area`; clippy `-p htui` |
| T3 `$EDITOR` | htui shell (worktree C) | 3 (§4.9) | `cargo test -p htui --all-features --lib`; clippy `-p htui`; `git diff --exit-code Cargo.lock` |
| merge | — | T1, then T2, then T3 | after **each** merge, the gates of the crates it touched, on the real tree; after T3, the whole `htui` suite |
| T4 worker | htui | 2 (§5.6) | `htui` lib tests; clippy |
| T5 view | htui | 3 (§6.8) | `INSTA_UPDATE=always` once, review, then the whole `htui` suite; snapshot and lock checks |
| T6 Postgres | htui tests | 1–2 (§7) | `templates_pg` on Postgres |
| close | — | — | the workspace gate (§9), then the plan's live check |

Environment used by every gate:

```bash
export PG=postgres://postgres:htui@localhost:5432          # this box (plan: README's 5439 is not up)
export PGT="USERNAME=htui-ci HTUI_TEST_DATABASE_URL=$PG/postgres"   # TOOL-2 prefix
# local Postgres 16: `service postgresql start` if `pg_isready -h localhost -p 5432` says no response
```

---

## 2. T1: the writer on every store (D1–D4, D16–D18, D30, D31)

**First failing test**: `store::conformance::prompt_template_append_is_a_cas_on_the_head` over
`MemStore` (`cargo test -p htui-core --all-features --test mem_store`).

### 2.1 `crates/htui-core/src/model/kind.rs`

After `PromptTemplate` (`:317-335`), a new struct and an inherent block:

```rust
/// A `prompt_template` row to append (MOD-9 plan D1): everything but `version` and the two
/// instants, which the store assigns. `version` is the head's plus one, or 1 for a new name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPromptTemplate {
    /// `prompt_template.id`, minted client-side as a UUIDv7.
    pub id: PromptTemplateId,
    /// `prompt_template.project_id`.
    pub project_id: ProjectId,
    /// `prompt_template.name`; its role is `TemplateRole::of_name(name)`.
    pub name: String,
    /// `prompt_template.body`; the store refuses what `parse` refuses (plan D4).
    pub body: String,
    /// `prompt_template.created_by`.
    pub created_by: UserId,
}

impl PromptTemplate {
    /// Whether `name` may name a template (plan D4): non-empty, no leading or trailing whitespace,
    /// no `\n` or `\r`. Here rather than in either store for `ItemKind::prefix_is_valid`'s reason:
    /// both stores refuse the same strings with the same sentence.
    #[must_use]
    pub fn name_is_valid(name: &str) -> bool {
        !name.is_empty() && name.trim() == name && !name.contains(['\n', '\r'])
    }
}
```

In `mod tests` (`:337`), `use super::{ItemKind, PromptTemplate};` and a new test,
**`template_names_are_trimmed_single_line_and_non_empty`**: good = `implement`, `judge`, `a b`,
`é-1`; bad = `""`, `" plan"`, `"plan "`, `"a\nb"`, `"a\rb"`, `"\t"`.

`model/mod.rs:115-119`: `NewPromptTemplate` joins the `pub use kind::{…}` list (rustfmt orders it).

### 2.2 `crates/htui-core/src/store/traits.rs`

The `use crate::model::{…}` list (`:32-42`) gains `NewPromptTemplate` and `PromptTemplate`; new import
`use crate::prompt::template::{TemplateRole, parse};`.

After `phases` (`:620`), before `// settings (D7, D8)` (`:622`):

```rust
    // prompt_template (MOD-9 milestone 1, plan D1-D4)

    /// Appends version `head + 1` of `(new.project_id, new.name)` iff the head version is
    /// `expected` (`None`: the name has no row yet), as one compare-and-set. Rows are never
    /// updated or deleted (PRD D5): `step_graph_phase.template_version`, a run snapshot and
    /// `trim_record.template` refer to versions by number. The reads stay inherent
    /// (`MemStore::prompt_templates`), because `prompt_template` is not mirrored.
    ///
    /// Order, the same on every store (plan D4, blueprint D18): the token first, so a spent token
    /// answers `Stale` even for bad input; then the name and the body (`parse` in the role
    /// `TemplateRole::of_name(name)`); then the project and `created_by`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "prompt_template" }`
    /// when `expected` is `Some` and the name has no row;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for an invalid name, a body
    /// `parse` refuses, or a project or `created_by` that names no row. Nothing is written.
    async fn append_prompt_template(
        &self,
        new: NewPromptTemplate,
        expected: Option<i32>,
    ) -> Result<CasOutcome<PromptTemplate>>;
```

Beside `reserved_phase_name` (`:1204`), the three helpers of D17:

```rust
/// MOD-9 D4: a template name `PromptTemplate::name_is_valid` refuses, in the sentence both stores
/// give it.
#[must_use]
pub fn invalid_template_name(name: &str) -> String {
    format!("prompt_template.name `{}` must be non-empty, single-line and trimmed", name.escape_debug())
}

/// MOD-9 D17: the `NotFound` id of a `(project, name)` pair, so both stores spell it alike.
#[must_use]
pub fn prompt_template_key(project: ProjectId, name: &str) -> String {
    format!("{project}/{name}")
}

/// MOD-9 D4, D17: why a template may not be saved, or `None` when it may. The name rule first,
/// then `parse` in the name's role; the sentence is `TemplateError`'s `Display`.
#[must_use]
pub fn prompt_template_refusal(name: &str, body: &str) -> Option<String> {
    if !PromptTemplate::name_is_valid(name) {
        return Some(invalid_template_name(name));
    }
    parse(TemplateRole::of_name(name), body).err().map(|err| err.to_string())
}
```

`store/mod.rs:10-18` (F-N) re-exports `invalid_template_name`, `prompt_template_key` and
`prompt_template_refusal`.

### 2.3 `MemStore` (`crates/htui-core/src/store/mem.rs`)

- The field doc at `:106` becomes "`prompt_template`, read by the inherent
  [`MemStore::prompt_templates`] (MOD-2 plan D102) and appended to only by
  [`WriteStore::append_prompt_template`] and the project seed (MOD-9 D1)".
- Imports (`:21-50`): `NewPromptTemplate`; `prompt_template_key`, `prompt_template_refusal`.
- `State::append_prompt_template`, after `phase_rows` (`:2616-2625`):

```rust
    /// MOD-9 D1/D4/D18: the head of `(project, name)` is the token; token, then input, then keys.
    fn append_prompt_template(
        &mut self,
        new: NewPromptTemplate,
        expected: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<PromptTemplate>> {
        let head = self
            .templates
            .iter()
            .filter(|row| row.project_id == new.project_id && row.name == new.name)
            .max_by_key(|row| row.version)
            .cloned();
        match (head, expected) {
            (Some(head), _) if Some(head.version) != expected => return Ok(CasOutcome::Stale(head)),
            (None, Some(_)) => {
                return Err(StoreError::NotFound {
                    entity: "prompt_template",
                    id: prompt_template_key(new.project_id, &new.name),
                });
            }
            _ => {}
        }
        if let Some(refusal) = prompt_template_refusal(&new.name, &new.body) {
            return Err(StoreError::Constraint(refusal));
        }
        self.require_user(new.created_by, "prompt_template.created_by")?;
        if !self.projects.contains_key(&new.project_id) {
            return Err(StoreError::Constraint(references_no_row(
                "prompt_template.project_id",
                new.project_id,
                "project",
            )));
        }
        if self.templates.iter().any(|row| row.id == new.id) {
            return Err(StoreError::Constraint(already_exists("prompt_template", new.id)));
        }
        let row = PromptTemplate {
            id: new.id,
            project_id: new.project_id,
            name: new.name,
            version: expected.unwrap_or(0) + 1,
            body: new.body,
            created_by: new.created_by,
            created_at: now,
            updated_at: now,
        };
        self.templates.push(row.clone());
        Ok(CasOutcome::Applied(row))
    }
```

- The trait arm goes after `async fn phases` (`:4656-4658`):
  `let now = Utc::now(); self.write(|state| state.append_prompt_template(new, expected, now))`.

### 2.4 `PgStore` (`crates/htui-store/src/pg/write.rs`, inside `impl WriteStore for PgStore` at `:384`, after `phases` at `:2126-2128`)

Imports (`:1-33`): `NewPromptTemplate`, `PromptTemplate` from `htui_core::model`;
`prompt_template_key`, `prompt_template_refusal` from `htui_core::store`. `cas_miss` (`:52`) is
reused.

```rust
    /// D3 amended by blueprint D16: one `INSERT … SELECT … WHERE head IS NOT DISTINCT FROM $6 ON
    /// CONFLICT DO NOTHING`. Two saves at one head: the second blocks on the unique index and then
    /// inserts nothing (probed on 16.13). Zero rows is split by one head read, `cas_miss`'s shape.
    /// Bad input pays the head read first so a spent token still answers `Stale` (D18, review M2).
    async fn append_prompt_template(
        &self,
        new: NewPromptTemplate,
        expected: Option<i32>,
    ) -> Result<CasOutcome<PromptTemplate>> {
        let key = prompt_template_key(new.project_id, &new.name);
        if let Some(refusal) = prompt_template_refusal(&new.name, &new.body) {
            let head = self.prompt_template(new.project_id, &new.name, None).await?;
            return match (head, expected) {
                (Some(head), _) if Some(head.version) != expected => Ok(CasOutcome::Stale(head)),
                (None, Some(_)) => Err(StoreError::NotFound { entity: "prompt_template", id: key }),
                _ => Err(StoreError::Constraint(refusal)),
            };
        }
        let inserted = sqlx::query_as!(PromptTemplate, r#"…§ below…"#, /* binds */)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        if let Some(row) = inserted {
            return Ok(CasOutcome::Applied(row));
        }
        cas_miss(
            self.prompt_template(new.project_id, &new.name, None).await?,
            "prompt_template",
            key,
        )
    }
```

The one new query (full text):

```sql
INSERT INTO prompt_template (id, project_id, name, version, body, created_by)
SELECT $1, $2, $3, COALESCE($6::int, 0) + 1, $4, $5
 WHERE (SELECT max(version) FROM prompt_template
         WHERE project_id = $2 AND name = $3) IS NOT DISTINCT FROM $6::int
ON CONFLICT (project_id, name, version) DO NOTHING
RETURNING id         AS "id: PromptTemplateId",
          project_id AS "project_id: ProjectId",
          name,
          version,
          body,
          created_by AS "created_by: UserId",
          created_at,
          updated_at
```

Binds, in order: `new.id.as_uuid()`, `new.project_id.as_uuid()`, `new.name`, `new.body`,
`new.created_by.as_uuid()`, `expected` (`Option<i32>`).

Checked against the DDL (`0001_init.sql:266-276`):
- `version INTEGER NOT NULL CHECK (version >= 1)`: `COALESCE(..)+1 ≥ 1` whenever the `WHERE` holds.
- `created_at` and `updated_at` default to `now()`, and no trigger fires on insert (`:573-578`).
- `UNIQUE (project_id, name, version)` is the `ON CONFLICT` target.
- `project_id` and `created_by` FKs raise `23503`, which `map_sqlx` turns into `Constraint`.

`create_item_kind` (`:1672-1695`) is the in-tree precedent for `INSERT … SELECT … RETURNING` with
plain aliases. If `prepare` infers any `RETURNING` column as nullable, add `!` to that alias.

`MemStore`'s zero-row cases have the same answers, on the same inputs:
- `(None, Some(n))` → `NotFound`.
- `(Some(h), e ≠ h)` → `Stale(h)`.
- A lost race → `Stale(new head)`.

### 2.5 `Writer` and the two spies

- `crates/htui-store/src/writer.rs`: the import list (`:35-45`) gains `NewPromptTemplate`,
  `PromptTemplate`. After `phases` (`:617-622`): `match self { Self::Memory(store) =>
  store.append_prompt_template(new, expected).await, Self::Online(pg) =>
  pg.append_prompt_template(new, expected).await }`.
- `crates/htui-agent/src/conformance.rs` (`UsageSpy`, after `:887-889`) and
  `crates/htui-agent/tests/recorder.rs` (`SpyStore`, after `:582-584`): each forwards through
  `self.inner`, returning `StoreResult<CasOutcome<PromptTemplate>>`. Both files' `htui_core::model`
  import lists gain `NewPromptTemplate`, `PromptTemplate`.

### 2.6 Conformance cases (`CASES` 56 → 59)

In `CASES` (`conformance.rs:37-93`), after `"boxes_lists_every_box_with_its_tools"`, in this order.
Each gets a `run_case` arm after `:192` and a fn after `boxes_lists_every_box_with_its_tools`
(`:5112`). Every call uses `created_by: ids::USER` and a fresh `PromptTemplateId::new()`. Keep the
bodies out of `format!`-style macros, because `{{` is an escape there.

| Case | Asserts |
|---|---|
| `prompt_template_append_is_a_cas_on_the_head` | The demo `PROJECT_HTUI`/`implement` is at v1. `Some(1)` with body A → `Applied` with `version == 2`, `body == A`, `created_by == ids::USER`, `name == "implement"`. `Some(1)` with body B → `Stale(row)` where `row.version == 2 && row.body == A`. `Some(2)` → `Applied` v3. `("no-such", Some(7))` → `NotFound { entity: "prompt_template", .. }`. `Some(0)` on `"no-such"` → `NotFound` (F-B's regression). |
| `prompt_template_new_name_starts_at_one` | `"triage"` with `None` and a valid phase body (`"Triage {{item_key}}.\n\n{{item}}\n"`) → `Applied` v1. `None` again → `Stale` v1 with the first body. `"judge"` with `None` (already at v1) → `Stale` whose `body == htui_core::prompt::body_of("judge").unwrap()`. |
| `prompt_template_refuses_what_parse_refuses` | `implement`, `Some(1)`: a body with `{{itme}}` → `Constraint` containing `unknown prompt placeholder`. `judge`, `Some(1)`: `"no candidates here\n"` → `Constraint` containing `must use candidates`. `""` and `" plan"` with `None` → `Constraint` containing `prompt_template.name`. `implement` with the `{{itme}}` body and **`Some(7)`** → `Stale` v1, not `Constraint` (D18). A valid body at `Some(1)` → `Applied` **v2**, which proves the refusals wrote nothing. An unknown `ProjectId::new()` with `None` → `Constraint`. |

Pins:
- `crates/htui-core/tests/mem_store.rs:36-37`: `56` → `59`, and the message gains ", and MOD-9
  milestone 1's three for the template writer (plan D1-D4)".
- `crates/htui-store/tests/pg_conformance.rs:19`: `const EXPECTED_CASES: usize = 59;`.
- `READ_CASES` stays 9. No other file counts the store's `CASES`: the `CASES` in `htui-agent` and
  `htui-orch` are those crates' own suites.

### 2.7 Postgres race: `crates/htui-store/tests/prompt_template_cas.rs` (new, D31)

```rust
//! MOD-9 D3: two saves at one head write one row, on a real server. …
#![cfg(feature = "demo")]
use htui_store::testkit as common;
```

**`two_saves_at_one_head_write_one_row`** is `#[tokio::test(flavor = "multi_thread")]`:
1. `let Some(db) = common::demo_db().await else { return };`
2. Build two `NewPromptTemplate`s for `ids::PROJECT_HTUI`/`"implement"` with bodies `"A {{item}}\n"`
   and `"B {{item}}\n"`, and run both with `Some(1)` through `tokio::join!` on `&db.store`.
3. Assert exactly one `Applied` (v2) and one `Stale` (v2), and that
   `db.store.prompt_templates(PROJECT_HTUI)` holds versions `[1, 2]` for `implement`, with v2's body
   the `Applied` one.
4. `db.drop_db().await`.

No `query!` in the file, so `.sqlx` is untouched by it. The `conformance.rs` doc never backticks this
test's name (F-H, D30).

### 2.8 Build coupling

- The five implementors are the whole set. `htui`, `htui-orch` and `CacheStore` implement no
  `WriteStore`.
- `htui` compiles unchanged against T1: nothing calls the method until T4.
- `.sqlx` moves only in T1.
- T1 touches no `htui` file.

### 2.9 Gate and commits

```bash
cargo fmt --all -- --check
cargo test -p htui-core --all-features -- --test-threads=1
env $PGT cargo test -p htui-store --all-features -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1
# .sqlx: a scratch DB recreated from zero before every prepare (edits to queries re-run it)
psql $PG/postgres -c 'DROP DATABASE IF EXISTS htui_prepare_mod9' -c 'CREATE DATABASE htui_prepare_mod9'
DATABASE_URL=$PG/htui_prepare_mod9 sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_mod9 \
   cargo sqlx prepare -- --all-targets --all-features \
 && DATABASE_URL=$PG/htui_prepare_mod9 cargo sqlx prepare --check -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l                                   # 235
git status --short crates/htui-store/.sqlx                           # exactly one `??` file
cargo clippy -p htui-core -p htui-store -p htui-agent --all-features --all-targets -- -D warnings
cargo check --workspace --all-features --all-targets
cargo doc -p htui-core -p htui-store --no-deps
```

`sqlx-cli 0.9.0` is installed (`~/.cargo/bin/{cargo-sqlx,sqlx}`), which matches the workspace's
`sqlx 0.9.0`. The `psql` URIs carry the password.

Commits:
1. `test(mod-9): append_prompt_template is a CAS on the head, on every store` (red). This covers the
   model type and `name_is_valid` with its test; the trait method; `todo!()` bodies in `MemStore`
   and `PgStore`; real delegations in `Writer` and both spies; the three cases, arms and pins; the
   helpers; and `prompt_template_cas.rs`.
2. `feat(mod-9): append_prompt_template on MemStore and PgStore` (green). The two bodies and the one
   new `.sqlx` file.
3. Optional: `docs(mod-9): …` for doc-only wording.

---

## 3. T2: `TextArea` (D7, D19)

**First failing test**: `ui::text_area::tests::typing_inserts_at_the_cursor`.

### 3.1 `crates/htui/src/ui/text_area.rs` (new)

```rust
//! A small multi-line editor (MOD-9 D7; PRD D2): insert, delete, newline, arrows, Home/End,
//! PgUp/PgDn and a byte-offset cursor, so a `parse` error lands on its byte. No wrap, undo,
//! selection or paste (PRD risk row 5). Width in `char`s, as `TextField` (no `unicode-width`).

/// What one key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaOutcome { /// Edited or moved.
    Consumed, /// `Esc`.
    Cancel, /// Not an editing key: chords, `Tab`, `BackTab`, `F(n)`, …
    Pass }

/// A multi-line buffer. Never `Debug`s its text.
#[derive(Clone, Default)]
pub struct TextArea {
    text: String,
    /// Byte offset into `text`, always on a char boundary.
    cursor: usize,
    /// The char column `Up`/`Down` aim for; cleared by every horizontal move and edit.
    goal_col: Option<usize>,
    /// First visible line and first visible char column (D19): moved by `lines` to keep the cursor
    /// in view, remembered between frames so the view does not jump. `Cell` because `Tab::render`
    /// is `&self`.
    top: Cell<usize>,
    left: Cell<usize>,
}
```

`impl Debug` prints `len`, `lines` and `cursor`.

Public surface, exactly:

```rust
pub fn new() -> Self;
pub fn with_text(text: &str) -> Self;                 // cursor 0, top/left 0
pub fn text(&self) -> &str;
pub fn into_text(self) -> String;
pub fn cursor(&self) -> usize;                         // bytes
pub fn set_cursor(&mut self, byte: usize);             // min(len), then floor to is_char_boundary
pub fn cursor_line_col(&self) -> (usize, usize);       // 0-based line, 0-based char column
pub fn on_key(&mut self, key: KeyEvent, page: u16) -> AreaOutcome;
pub fn lines(&self, width: u16, height: u16, focused: bool, theme: &Theme) -> Vec<Line<'static>>;
```

`on_key` rules:
- Any of `CONTROL | ALT | SUPER | META | HYPER` → `Pass`. `SHIFT` is a capital, so it does not pass.
- `Char(c)` inserts unless `c.is_control()`, which is swallowed and answers `Consumed`.
- `Enter` inserts `\n`.
- `Backspace` and `Delete` remove the char before or at the cursor; removing `\n` joins lines.
- `Left` and `Right` cross line ends.
- `Up` and `Down` keep `goal_col`, clamped to the target line's length.
- `Home` and `End` go to the current line's bounds.
- `PageUp` and `PageDown` move by `page.max(1)` lines, clamped, keeping `goal_col`.
- `Esc` → `Cancel`. `Tab`, `BackTab`, `F(_)` and anything else → `Pass`.

`on_key` allocates no new `String`: it edits in place.

`lines` rules:
1. Clamp `top` so `top ≤ cursor_line < top + height`, and `left` so `left ≤ cursor_col < left + width`.
   Store both.
2. Return at most `height` lines, each the `[left, left + width)` char window of its line, styled
   `theme.base`.
3. While `focused`, the cursor cell (a space past the end of line) is styled `theme.selected`.

`height == 0` or `width == 0` returns an empty `Vec`.

### 3.2 `crates/htui/src/ui/mod.rs`

After `pub mod tabs;` (`:6`): `pub mod text_area;`. After `:11`:
`pub use text_area::{AreaOutcome, TextArea};`.

### 3.3 Tests (unit, in `text_area.rs`)

Written first, all red on `todo!()`: `typing_inserts_at_the_cursor`,
`enter_splits_the_line_and_backspace_joins_it_again`, `delete_at_line_end_joins_the_next_line`,
`left_and_right_cross_line_boundaries`, `up_and_down_keep_the_goal_column_through_a_short_line`,
`home_and_end_stay_on_the_line`, `page_down_moves_by_the_page_and_clamps`,
`set_cursor_floors_inside_a_multibyte_char` (`"é{{x"`: `set_cursor(1)` → 0, `set_cursor(2)` → 2),
`set_cursor_past_the_end_clamps_to_len`, `cursor_line_col_counts_chars_not_bytes`,
`control_chords_tab_and_function_keys_pass`, `esc_cancels`, `debug_prints_lengths_not_text`, and:
- `the_viewport_scrolls_to_keep_the_cursor_visible`: a 10-line text, `PageDown` with `page = 3`;
  `lines(5, 3, true, &theme)` contains the cursor line, and a second `lines` call with no key between
  returns the same window (the `Cell` memory).
- `lines_takes_shared_self` (D19): calls `lines` through a `&TextArea`, so F-C cannot come back.

### 3.4 Build coupling

Nothing outside `text_area.rs` uses it until T5. It needs no T1 or T3 item.

### 3.5 Gate

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib ui::text_area -- --test-threads=1
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

(`htui` builds only once F-A is resolved.)

### 3.6 Commits

1. `test(mod-9): TextArea's editing, cursor and viewport` (red): the module with `todo!()` bodies,
   the tests, and `ui/mod.rs`.
2. `feat(mod-9): TextArea, a small multi-line editor` (green).

---

## 4. T3: `$EDITOR` handoff, suspension, shell plumbing (D8–D10, D21–D26)

**First failing test**: `editor::tests::visual_wins_over_editor`.

### 4.1 Today's event loop and terminal (read in full)

- `event_loop::run` (`event_loop.rs:22-52`) builds `EventStream::new()` and `interval(TICK)`
  (250 ms, `Burst`) and draws once. Each turn is one `tokio::select!` with three arms (a terminal
  event → `app.on_terminal_event`, where `None` breaks; a reply → `Action::Reply`; a tick →
  `Action::Tick`), then the `should_quit` check, then a draw if `std::mem::take(&mut app.dirty)`.
- `terminal::init` (`terminal.rs:23-29`) calls `install_panic_hook()` (htui's conditional hook,
  `:35-43`), then `ratatui::init()`. That sets **ratatui's own** unconditional restore hook on top
  (`ratatui-0.30.2/src/init.rs:397-403`), enables raw mode and enters the alternate screen.
  `TerminalGuard::restore` is idempotent, and `Drop` restores. `lib.rs:106-108` is `init`, `run`,
  `restore`.
- **Nothing enables mouse capture, bracketed paste, focus events or keyboard-enhancement flags**
  (the grep for `EnableMouseCapture`, `EnableBracketedPaste` and `PushKeyboardEnhancementFlags` over
  `crates/htui` is empty). The state to save and restore is raw mode, the alternate screen and cursor
  visibility.
- The lock holds one `crossterm 0.29.0`, so `enable_raw_mode` here and ratatui's `disable_raw_mode`
  share one saved-termios static. Tracing writes to a file or nowhere (`lib.rs:150-172`), so no log
  paints over the editor.

### 4.2 `crates/htui/src/editor.rs` (new): types

```rust
//! The `$EDITOR` handoff (MOD-9 D8, D9): resolve the command, write a temp file, run the editor
//! with the TUI suspended, read the file back.

/// A resolved editor command (D8). `value` is handed to the platform shell verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand { value: String, fallback: bool }

impl EditorCommand {
    /// `VISUAL`, then `EDITOR` (first non-blank after trim), else `vi` (unix) / `notepad`
    /// (windows) with `fallback: true`.
    pub fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Self;
    /// `resolve` over `std::env::var`.
    #[must_use] pub fn from_env() -> Self;
    /// The command line shown in messages: `value`.
    #[must_use] pub fn value(&self) -> &str;
    /// unix: `sh -c "<value> \"$1\"" htui-editor <file>`;
    /// windows: `cmd` + `raw_arg(format!("/S /C \"{value} \"{file}\"\""))`.
    #[must_use] pub fn command(&self, file: &Path) -> std::process::Command;
}

/// What the view asks the shell to edit (D10). Text is never `Debug`ged.
#[derive(Clone, PartialEq, Eq)]
pub struct ExternalEdit {
    /// The body handed to the editor.
    pub text: String,
    /// The temp file's name stem, normally the template name; sanitised by `run` (D25).
    pub stem: String,
}
// impl Debug: ExternalEdit { text_len, stem }

/// What came back (D9, D24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalEditOutcome {
    /// The file changed; normalised text (`htui_core::prompt::render::normalise_newlines`).
    Edited(String),
    /// Byte-identical after normalising both sides. `quick`: the editor returned within
    /// [`QUICK_EXIT`], the "GUI editor without `--wait`" shape (R-3).
    Unchanged { quick: bool },
    /// Nothing changed and why, one sentence for the notice line.
    Failed(String),
}

/// Below this, an unchanged return is reported as "quick" (D24).
pub const QUICK_EXIT: Duration = Duration::from_secs(1);

/// Leaving and re-entering the TUI's terminal state (D9). `TerminalGuard` is the real one;
/// tests use a recording fake.
pub trait Suspend {
    /// Give the terminal to a child: show the cursor, leave raw mode and the alternate screen.
    fn leave(&mut self) -> io::Result<()>;
    /// Take it back: raw mode, alternate screen, clear so the next draw repaints whole.
    fn enter(&mut self) -> io::Result<()>;
}

pub async fn run(cmd: &EditorCommand, text: &str, stem: &str) -> ExternalEditOutcome;
pub async fn run_suspended<S: Suspend>(term: &mut S, cmd: &EditorCommand, edit: &ExternalEdit)
    -> io::Result<ExternalEditOutcome>;
```

### 4.3 `editor.rs`: bodies

**`run`**:
1. `let stem = sanitise(stem)` (D25).
2. Create `tempfile::Builder::new().prefix(&format!("htui-{stem}-")).suffix(".md").tempfile()`. On
   error → `Failed("could not create a temp file: {err}")`.
3. `write_all(text)`, then `into_temp_path()`. That closes the handle, and the path is removed on
   drop on every path, the dropped-future path included.
4. `let started = Instant::now()`, then
   `tokio::process::Command::from(cmd.command(&path)).kill_on_drop(true).status().await` (D25).
   Stdio is inherited.
5. Map the result:
   - `Err(e)` → `Failed(start_failure(cmd, &e.to_string()))`.
   - `Ok(status)` with code 127 or 126 (unix) or 9009 (windows) → `Failed(start_failure(cmd,
     "not found or not executable"))` (D23).
   - Any other non-success → ``Failed(format!("`{}` exited with {code}; nothing was changed",
     cmd.value()))``; `code` is `status.code()`, or `"a signal"` when there is none.
6. `std::fs::read(&path)`, then `String::from_utf8`. An error → `Failed("the edited file is not
   UTF-8; nothing was changed")`.
7. Compare `normalise_newlines(read)` with `normalise_newlines(text)`: equal →
   `Unchanged { quick: started.elapsed() < QUICK_EXIT }`, else `Edited(normalised)`.

**`start_failure(cmd, why)`** (F-M reads `fallback`):
- `fallback == true`: ``"no $VISUAL or $EDITOR is set and `vi` could not start ({why})"``.
- Otherwise: ``"could not start `{value}` ({why}) — set $VISUAL or $EDITOR"``.

**`run_suspended`** (D21):

```rust
pub async fn run_suspended<S: Suspend>(term: &mut S, cmd: &EditorCommand, edit: &ExternalEdit)
    -> io::Result<ExternalEditOutcome>
{
    if let Err(err) = term.leave() {
        term.enter()?;                                     // undo a half-leave (F-O)
        return Ok(ExternalEditOutcome::Failed(format!("could not leave the TUI: {err}")));
    }
    let resume = Resume { term, armed: true };             // Drop re-enters only if cancelled
    let outcome = run(cmd, &edit.text, &edit.stem).await;
    resume.finish()?;                                      // disarm, then enter() with `?`
    Ok(outcome)
}

struct Resume<'a, S: Suspend> { term: &'a mut S, armed: bool }
impl<S: Suspend> Resume<'_, S> {
    fn finish(mut self) -> io::Result<()> { self.armed = false; self.term.enter() }
}
impl<S: Suspend> Drop for Resume<'_, S> {
    fn drop(&mut self) {
        // A dropped future: nothing can report the error, so the best effort is to re-enter. Not
        // while panicking: both hooks and `TerminalGuard::drop` restore on that path, and
        // re-entering here would leave the alternate screen up behind the panic message.
        if self.armed && !std::thread::panicking() { let _ = self.term.enter(); }
    }
}
```

**Panic interplay.**
- A panic on the event-loop task while the editor runs unwinds through `Resume::drop`, which skips
  because of `panicking()`. Both hooks restore, then `TerminalGuard::drop` restores again. Every
  step is idempotent.
- A panic on another task while the editor runs fires the hooks, which write `LeaveAlternateScreen`
  into the editor's screen. That is cosmetic, and the editor keeps running (R-10).

### 4.4 `crates/htui/src/terminal.rs`

After `impl TerminalGuard` (`:63-76`):

```rust
impl crate::editor::Suspend for TerminalGuard {
    /// Show the cursor (every draw hid it), then `ratatui::try_restore` (D22). `restored` is not
    /// touched: this is a pause, not the end.
    fn leave(&mut self) -> std::io::Result<()> {
        self.terminal.show_cursor()?;
        ratatui::try_restore()
    }
    /// Raw mode and the alternate screen back, then `Terminal::clear`, which resets the back
    /// buffer so the next draw is whole (`ratatui-core-0.1.2/src/terminal/buffers.rs:147-173`).
    /// Not `ratatui::init()`: that would stack another panic hook (`init.rs:397-403`).
    fn enter(&mut self) -> std::io::Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        self.terminal.clear()
    }
}
```

### 4.5 `crates/htui/src/event_loop.rs`

After the `if app.should_quit { break; }` block (`:44-46`), before the draw (`:47-49`):

```rust
        // MOD-9 D9: a view asked for `$EDITOR`. The stream goes first: crossterm 0.29 parks a
        // thread in `poll_internal(None, ..)` that reads the tty until the stream is dropped
        // (`crossterm-0.29.0/src/event/stream.rs:44-55`, `:140-145`), and it would take the
        // editor's keys. A fresh stream after; `reset` so a long edit does not replay a burst of
        // ticks (`interval` is `Burst`). Replies queue in the unbounded channel meanwhile.
        if let Some((tab, edit)) = app.take_external_edit() {
            drop(events);
            let outcome =
                crate::editor::run_suspended(term, &crate::editor::EditorCommand::from_env(), &edit)
                    .await?;
            events = crossterm::event::EventStream::new();
            ticker.reset();
            app.finish_external_edit(tab, outcome);
        }
```

The module doc (`:1-5`) gains one sentence: "one post-step, not an arm, suspends the terminal for
`$EDITOR` (MOD-9 D9)." `events` is already `let mut`. The `?` return path leaves `events` moved,
which the borrow checker accepts because the function returns.

### 4.6 Shell plumbing

- **`app/action.rs`**: after `Promote` (`:45-50`):
  `/// Hand text to $VISUAL/$EDITOR (MOD-9 D10). Stamped with the emitting tab in App::drain; any
  other origin is refused. EditExternally(crate::editor::ExternalEdit),`.
- **`app/state.rs`**:
  - `App` (`:127-191`) gains `pub(super) pending_edit: Option<(TabId, ExternalEdit)>`, documented,
    and `App::new` (`:199-227`) sets `None`.
  - `drain` (`:313-330`) gains an arm:
    ```rust
    Action::EditExternally(edit) => match origin {
        Origin::Tab(id) => { self.pending_edit = Some((*id, edit)); self.dirty = true; }
        _ => self.update(Action::Error(EDITOR_NEEDS_A_TAB.to_owned())),
    },
    ```
  - New constant: `pub const EDITOR_NEEDS_A_TAB: &str = "only a tab can open the editor";`.
  - New methods:
    ```rust
    /// The edit a tab asked for, taken by the event loop (D10).
    pub fn take_external_edit(&mut self) -> Option<(TabId, ExternalEdit)>;
    /// Routes the editor's outcome to the tab that asked, with a Ctx, then drains its emits as
    /// `on_reply` does (update.rs:176-204). A tab no longer registered drops it. Sets `dirty`.
    pub fn finish_external_edit(&mut self, tab: TabId, outcome: ExternalEditOutcome);
    ```
- **`app/update.rs`**: `update` (`:22-40`) gains
  `Action::EditExternally(_) => self.status = Some(EDITOR_NEEDS_A_TAB.to_owned()),`. That is the
  unstamped path (`Origin::App`, or a keymap binding). `update` is the only exhaustive `Action`
  match in the crate.
- **`ui/tabs/registry.rs`**: after `focus_section` (`:57-59`):
  ```rust
  /// The `$EDITOR` handoff this tab asked for came back (MOD-9 D10). Defaulted, the trait's second
  /// default after `focus_section`, so no other tab changes.
  fn on_external_edit(&mut self, _outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {}
  ```
- **`lib.rs`**: `pub mod editor;` between `connection` and `event_loop` (`:15-16`).
- **`crates/htui/Cargo.toml`**: `tempfile = "3"` leaves `[dev-dependencies]` (`:69-70`, with its
  comment) and joins `[dependencies]` under
  `# MOD-9 D9: the $EDITOR temp file (removed on drop); already in the lock (was dev-only).`

### 4.7 Tests

`editor.rs` `mod tests` (written first):
- **Resolution:**
  - `visual_wins_over_editor`
  - `blank_values_fall_through`
  - `unix_falls_back_to_vi` (`cfg(unix)`)
  - `windows_falls_back_to_notepad` (`cfg(windows)`)
  - `the_unix_command_passes_the_file_as_dollar_one`: `get_program() == "sh"`, and
    `get_args() == ["-c", "<v> \"$1\"", "htui-editor", <file>]`.
- **`cfg(unix)` round trips** through `#!/bin/sh` scripts in a `tempfile::TempDir`. Write each with
  `std::fs::write` (the handle is closed at once, against `ETXTBSY`), `chmod 0o755`, and set the
  script path as `value`:
  - `a_fake_editor_that_appends_returns_edited`
  - `a_fake_editor_that_does_nothing_returns_unchanged` (asserts `quick: true`)
  - `a_crlf_writing_editor_is_normalised`
  - `a_non_zero_exit_is_failed_and_names_the_code` (`exit 3` → contains `exited with 3`)
  - `a_missing_program_is_failed_and_names_visual_and_editor` (`value = "/nonexistent/ed"` → exit
    127 → contains `$VISUAL or $EDITOR`; F-D)
  - `the_temp_file_is_gone_after_every_outcome`: the script copies `"$1"` into a side file, and each
    of the three outcomes then asserts `!path.exists()`.
  - `a_stem_with_a_path_separator_stays_in_the_temp_dir` (D25)
- **Suspension**, over `FakeTerminal { calls: Vec<&'static str>, fail_leave: bool, fail_enter: bool }`:
  - `leave_then_enter_on_success`
  - `enter_runs_after_a_failed_editor`
  - `a_failed_leave_spawns_nothing_and_enters_once` (F-O)
  - `a_failed_enter_is_an_io_error` (F-I)
  - `a_dropped_future_still_enters`: a script that `sleep 5`s, `tokio::time::timeout(50ms, ..)`,
    then assert `["leave", "enter"]`. `kill_on_drop` reaps the child.

`update.rs` `mod tests` (D26; a small `Asker` tab and `Popup` overlay defined there):
- `an_external_edit_from_a_tab_is_held_for_the_loop`
- `an_external_edit_from_an_overlay_is_refused_on_the_status_line`
- `finish_external_edit_reaches_only_the_asking_tab`

No test drives `event_loop::run`: it needs a tty. The joint is the plan's live check.

### 4.8 Build coupling

T3 uses nothing from T1 or T2. `Tab::on_external_edit` is defaulted, so no existing tab changes.
`Action` gains a variant, and `update.rs:22` is its only exhaustive match. `Cargo.lock` does not
move, because `tempfile` is already in `htui`'s lock entry.

### 4.9 Gate and commits

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib -- --test-threads=1
cargo clippy -p htui --all-features --all-targets -- -D warnings
git diff --exit-code Cargo.lock
grep -rn 'EDITOR\|VISUAL' crates --include='*.rs' | grep -v 'src/editor.rs'   # docs/tests only
```

Commits:
1. `test(mod-9): the $EDITOR runner and the terminal suspension` (red: `editor.rs` with `todo!()`,
   its tests, `lib.rs`, `Cargo.toml`).
2. `feat(mod-9): run $VISUAL/$EDITOR with the terminal suspended` (green, plus `terminal.rs` and
   `event_loop.rs`).
3. `feat(mod-9): Action::EditExternally and Tab::on_external_edit` (the shell tests and plumbing;
   the test half may be its own `test(mod-9): …` commit first).

---

## 5. T4: the worker surface (D5)

**First failing test**: `templates::tests::the_read_answers_every_scope_project_in_scope_order`.
Starts from T1 + T2 + T3 merged.

### 5.1 `crates/htui/src/templates.rs` (new)

```rust
//! The prompt templates behind the Skills tab's Templates view (MOD-9 milestone 1, D5): one read
//! per scope, one compare-and-set append, the `prompt_settings.rs` shape. Known residue, as there:
//! a re-read that fails after an applied write answers `Failed`.

/// Every scope project's templates, every version.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplatesSnapshot { pub projects: Vec<ProjectTemplates> }

/// One project's rows, `(name, version)` byte order as `Backend::prompt_templates` returns them.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectTemplates { pub project_id: ProjectId, pub templates: Vec<PromptTemplate> }

impl TemplatesSnapshot {
    /// The highest version of `(project, name)`.
    #[must_use] pub fn head(&self, project: ProjectId, name: &str) -> Option<&PromptTemplate>;
    /// One version of `(project, name)`.
    #[must_use] pub fn version(&self, project: ProjectId, name: &str, version: i32)
        -> Option<&PromptTemplate>;
}

pub const REQUEST_NAMES: [&str; 2] = ["templates", "save_template"];
pub const READ_NAME: &str = REQUEST_NAMES[0];

pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>;
```

**`serve`**:
- `Templates(scope)` → `StoreReply::Templates(Box::new(snapshot(backend, scope).await?))`.
  `snapshot` calls `backend.prompt_templates(id)` for each id in `scope.project_ids`, in order.
  Offline, that is `Unreachable(PROMPT_ON_SERVER_ONLY)` (`backend.rs:348-354`).
- `SaveTemplate { scope, project, name, body, expected }`, in this order:
  1. `let writer = backend.writer().ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;`
  2. `let created_by = backend.this_user().await?;` (`backend.rs:174`, as `hierarchy.rs:184`)
  3. `writer.append_prompt_template(NewPromptTemplate { id: PromptTemplateId::new(), project_id:
     *project, name: name.clone(), body: body.clone(), created_by }, *expected).await?`
  4. A fresh snapshot, then `Applied` → `Templates`, `Stale` → `TemplatesStale`
     (`catalogue.rs:309-315`'s `cas` shape).
- Any other request → `Err(StoreError::Backend(format!("not a template request: {}",
  other.name())))`.

`lib.rs`: `pub mod templates;` between `store_worker` and `terminal` (`:23-24`).

### 5.2 `store_worker.rs`

- **Imports** (`:38-45`): `use crate::templates::{self, TemplatesSnapshot};`.
- **`StoreRequest`**, after `ClearSetting` (`:500-509`):
  ```rust
  /// Every scope project's prompt templates, every version (MOD-9 D5).
  Templates(Scope),
  /// Append version `expected + 1` of `(project, name)` iff `expected` is its head (`None`: a new
  /// name). The worker fills `created_by` (`this_user`); the view never holds a `UserId`.
  SaveTemplate { scope: Scope, project: ProjectId, name: String, body: String,
                 expected: Option<i32> },
  ```
  Each field is documented.
- **`name()`**, after `:605`, under the comment `// The two of templates::REQUEST_NAMES, in that
  order (MOD-9 D5).`: `Self::Templates(..) => "templates"`, `Self::SaveTemplate { .. } =>
  "save_template"`.
- **`StoreReply`**, after `PromptSettingsStale` (`:767`): `Templates(Box<TemplatesSnapshot>)` and
  `TemplatesStale(Box<TemplatesSnapshot>)`, documented as the `PromptSettings` pair is.
- **`try_serve`**, after the prompt-settings arm (`:1049-1054`):
  ```rust
  // The two template requests, or-ed for the reason the arms above are: a guard does not count
  // towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12).
  StoreRequest::Templates(..) | StoreRequest::SaveTemplate { .. } => {
      templates::serve(backend, request).await?
  }
  ```
- `ProjectId` is already in the `htui_core::model` import (`:22-27`); nothing to add there.

### 5.3 Tests (unit, `templates.rs`, `#[tokio::test]` over `Backend::memory(MemStore::demo())`)

Written first:
- `the_read_answers_every_scope_project_in_scope_order`: the Platform workspace's scope; project ids
  in scope order; 10 rows each; `implement` at v1.
- `a_save_at_the_head_answers_templates_with_the_new_version`: `created_by == ids::USER`.
- `a_save_at_a_stale_head_answers_templates_stale_and_writes_nothing`
- `a_refused_body_answers_failed_with_the_parse_message`: through `store_worker::serve`; asserts
  `Failed { request: "save_template", message }` with `message` containing
  `unknown prompt placeholder`.
- `request_names_match_the_name_arms`: `READ_NAME == REQUEST_NAMES[0]`, and each name equals `name()`
  of a sample request.
- `an_offline_read_is_refused_with_the_server_only_sentence`, over `Backend::Offline` built as
  `tests/chat_offline.rs` does. Optional if the builder is heavy; T6 covers the save.

### 5.4 Build coupling

- Needs T1's trait method and `NewPromptTemplate`.
- Shares `lib.rs` with T3, which is why T4 runs after T3's merge.
- No view consumes the replies until T5.
- `observe_reply` has a `_` arm, and `on_reply` passes unknown replies.

### 5.5 Gate

```bash
cargo test -p htui --all-features --lib templates -- --test-threads=1
cargo test -p htui --all-features -- --test-threads=1        # R-6: nothing else moved
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

### 5.6 Commits

1. `test(mod-9): the template read and the CAS save on the store worker` (red).
2. `feat(mod-9): Templates and SaveTemplate on the store worker` (green).

---

## 6. T5: the Skills tab and the Templates view (D6, D11–D14, D19, D20, D27–D29)

**First failing test**: `tests/templates.rs::the_templates_view_lists_the_scope_s_templates`, the
`templates__browse` snapshot. Starts from T4 merged.

### 6.1 Files

- `git mv crates/htui/src/ui/tabs/skills.rs crates/htui/src/ui/tabs/skills/mod.rs`, then rewrite
  it. `ui/tabs/mod.rs:7`/`:13` do not change.
- New `ui/tabs/skills/templates.rs` and `ui/diff.rs`.
- `ui/mod.rs`: `pub mod diff;` before `pub mod layout;`.
- `ui/tabs/chat/transcript.rs`: delete `diff_style` (`:627-639`) and `use ratatui::style::Style;`
  (`:13`, F-E); import `use crate::ui::diff::diff_style;`. Behaviour is byte-identical, so chat
  snapshots do not move.
- `crates/htui/Cargo.toml` `[dependencies]`: `similar = { workspace = true }`, with the comment
  `# MOD-9 D12: the Templates view's line diff; 3.2.0 is already compiled for htui-agent.`

### 6.2 `ui/diff.rs`

```rust
/// `similar`'s unified diff, three lines of context, `---`/`+++` headers (acp::fs's shape).
#[must_use] pub fn unified(old: &str, new: &str, old_label: &str, new_label: &str) -> String;
/// `+` accents, `-` errors, headers and context dim — moved from the chat transcript (D12).
#[must_use] pub fn diff_style(line: &str, theme: &Theme) -> Style;
/// One styled line per diff line; an empty diff is the single dim line "no differences".
#[must_use] pub fn lines(diff: &str, theme: &Theme) -> Vec<Line<'static>>;
```

Unit tests: `identical_texts_render_no_differences`, `an_added_line_is_accented`.

### 6.3 `SkillsTab` (`skills/mod.rs`)

```rust
/// The Skills tab (MOD-9 PRD D1): `Skills | Templates`. The Skills view is milestone 3's.
#[derive(Debug, Default)]
pub struct SkillsTab { view: View, templates: TemplatesView }
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum View { #[default] Skills, Templates }
```

It is no longer `Copy`/`Eq`; nothing relied on that (the only uses are `app/mod.rs:17`/`:48` and
`ui/tabs/mod.rs:13`).
- **`ID`**: `TabId("skills")`. **Title**: `"Skills"`.
- **`wants_requests`**: `vec![StoreRequest::Templates(scope.clone())]`.
- **`on_scope_change`** → `templates.on_scope_change()`. **`on_reply`** → `templates.on_reply`,
  whichever view is shown. **`on_external_edit`** → `templates.on_external_edit`.
- **`on_key`**:
  1. If `view == Templates && templates.captures_input()`, delegate.
  2. Else `h`/`[`/`Left` and `l`/`]`/`Right` toggle the view (Settings' keys,
     `settings/mod.rs:331-338`).
  3. Else delegate when `view == Templates`.
  4. Else `Pass`.
- **`render`**:
  - Row 0 is the switch line: ` Skills │ Templates `, with the shown one in `theme.title` and the
    other dim.
  - Skills view: one dim line, "Skills are edited here from MOD-9 milestone 3." The MOD-12
    attribution is corrected, per the PRD's record corrections.

### 6.4 `TemplatesView` (`skills/templates.rs`): state

```rust
#[derive(Debug, Default)]
pub(super) struct TemplatesView {
    snapshot: Option<TemplatesSnapshot>,
    unavailable: Option<String>,          // a refused READ_NAME
    cursor: usize,                        // index into rows()
    shown: Option<i32>,                   // the version shown for the selected name; None = head
    base: Option<DiffBase>,               // `b` / `D`
    pane: Pane,                           // Body | Diff
    mode: Mode,
    busy: Option<&'static str>,           // one write in flight (settings/prompt.rs:255)
    notice: Option<Notice>,               // Info | Error, one line
    external: Option<Pending>,            // what `E`/`Ctrl+E` asked for, until on_external_edit
}
enum Row { Project(ProjectId), Template { project: ProjectId, name: String } }   // derived
enum DiffBase { Version(i32), Default }
enum Mode { Browse, Naming { project: ProjectId, field: TextField }, Editing(Editor) }
struct Editor { project: ProjectId, name: String, token: Option<i32>, area: TextArea,
                original: String, confirm_item: bool, esc_armed: bool }
struct Pending { project: ProjectId, name: String, token: Option<i32>, resume: Option<Editor> }
```

- `rows()` is built from the snapshot: per project in snapshot order, a `Project` header, then one
  `Template` row per distinct name in byte order.
- `captures_input()` is true for `Naming` and `Editing`.
- `on_scope_change` clears `snapshot`, `unavailable`, `mode`, `busy`, `external`, `cursor`, `shown`
  and `base`. It keeps `notice`, as `settings/prompt.rs:798-805` does.

### 6.5 State machine and keys

| Mode | Key / event | Effect |
|---|---|---|
| Browse | `j`/`Down`, `k`/`Up` | row cursor; `shown = None`, `base = None`, `pane = Body` |
| Browse | `,` / `.` | shown version −1 / +1 within the name's versions (clamped) |
| Browse | `b` | `base = Version(shown)`; notice "base v{N}" |
| Browse | `d` | toggle `pane` Body ↔ Diff; the default base is `shown − 1` (on v1: notice "v1 has no earlier version") |
| Browse | `D` | `base = Default`, `pane = Diff`; no `body_of(name)` → notice "`{name}` has no compiled default" |
| Browse | `e` | `Editing` on the shown body, `token = head.version` (OQ-5), cursor 0 |
| Browse | `E` | `external = Pending { resume: None, token: Some(head) }`; emit `Action::EditExternally(ExternalEdit { text: shown body, stem: name })` |
| Browse | `n` | `Naming` for the row's project, with a `TextField` |
| Browse | `r` | re-request `Templates(scope)` unless `busy` |
| Browse | on a `Project` row, `e`/`E`/`d`/`D`/`b`/`,`/`.` | notice "select a template" |
| Naming | `Enter` | Invalid name (`PromptTemplate::name_is_valid`) → notice `invalid_template_name`. Existing name in the project → "`{name}` exists — select it and press e". Otherwise `Editing` on `body_of(name).unwrap_or("")` with `token = None`. |
| Naming | `Esc` | Browse |
| Editing | printable, `Enter`, arrows, … | `area.on_key(key, page)`; any `Consumed` edit clears `confirm_item` and `esc_armed` |
| Editing | `Ctrl+S` | the D11 save flow (below) |
| Editing | `Ctrl+E` | `external = Pending { resume: Some(editor) }`; emit `EditExternally` with the draft |
| Editing | `Esc` (`Cancel`) | Unmodified → Browse. Modified and not armed → notice "unsaved changes — Esc again discards", arm. Armed → Browse. |
| Editing | `Tab`/`BackTab` (`Pass`) | `Handled::Pass`, so the global binding switches tabs and the draft survives |

**Save (`Ctrl+S`)**, first match wins:
1. `busy` → notice `` `{busy}` is still in flight ``.
2. `parse(TemplateRole::of_name(name), text)`:
   - An `Err` with `at` → `area.set_cursor(at)`, notice `err.to_string()`, no request.
   - `MissingRequired` → `set_cursor(text.len())`, notice, no request.
3. `Ok(p)` with role `Phase`, `p.omits_item()` and `!confirm_item` → notice "this phase template
   never places {{item}} — Ctrl+S again saves anyway", and set `confirm_item = true`.
4. Otherwise `busy = Some("save_template")` and send
   `SaveTemplate { scope, project, name, body: text, expected: token }`.

**Replies**:
- `Templates(s)`: store `s`, clear `unavailable`.
  - If `busy == Some("save_template")` **and** `s.head(project, name).version >
    token.unwrap_or(0)` (D27): `busy = None`, back to Browse with the cursor on that name,
    `shown = None`, notice "saved v{N}".
  - Otherwise the editor, the token and `busy` are untouched.
- `TemplatesStale(s)`: `busy = None`, store `s`, `token = Some(head)`. The notice is
  `TEMPLATE_CHANGED_ELSEWHERE`: "saved elsewhere since you opened it — v{N} is now the latest; your
  draft is kept and Ctrl+S saves it as v{N+1}". The draft is kept.
- `Failed { request, message }`:
  - `request == READ_NAME` → `unavailable = Some(message)`.
  - `request` is one of `REQUEST_NAMES` → `busy = None`, error notice. The editor stays.

**`on_external_edit(outcome)`**: `take()` the pending edit. No pending edit → ignore.
- `Edited(text)` → `Editing` on `text` with the pending token. Run the Ctrl+S `parse` without
  sending: the cursor goes to the error, else to 0 (OQ-3).
- `Unchanged { quick }` → the resumed editor, if any, is back as it was. Notice "no changes", plus
  " — a GUI editor needs its wait flag, e.g. `code --wait`" when `quick`.
- `Failed(msg)` → the resumed editor, if any, is back. Error notice `msg`.

Every key D13 lists misses the global table in Browse (`keymap.rs:199-239`: `q`, `Tab`,
`Shift+Tab`, `1`–`9`, `?`, and `w` from `app/mod.rs:63-68`). In Editing, `q`, digits, `?` and `w`
are text.

### 6.6 Render (body 100×27 at the Harness's 100×30; chrome takes 3 rows)

```
row 0      switch line                                                        (Length 1)
rows 1-24  Browse: [ list | pane ]   Editing: [ TextArea | help ]            (Min 1)
row 25     notice (Info dim / Error error), or blank                          (Length 1)
row 26     hint                                                               (Length 1)
```

**Browse**. `Layout::horizontal([Length(32), Min(1)])`.
- The list is a bordered block, ` Templates `:
  - Header rows show the project slug from `ctx.projects` (a missing `ProjectRef` falls back to the
    short id).
  - Template rows are `format!("  {name:<13} v{head:<3} {role}")`, with `role` from
    `TemplateRole::as_str`. The cursor row is `theme.selected`.
- The pane is bordered and titled ` {name} v{shown} (head v{h}) `, or ` diff v{a} → v{b} ` /
  ` diff default → v{b} `. It shows the body as dim-free `Line`s (`theme.base`, clipped, no wrap),
  or `diff::lines(diff::unified(..), theme)`.
- The hint is `j/k move  ,/. version  b base  d diff  D default  e edit  E $EDITOR  n new  r reload  h/l view`.
- Before the first reply the pane says "templates not read yet"; after a refused read it says
  "templates unavailable: {message}".

**Editing**. `Layout::horizontal([Min(1), Length(34)])`.
- The left block is ` {name} · editing from v{shown}, saves v{token+1} `, holding
  `area.lines(w-2, h-2, true, theme)`.
- The right block is ` {role} placeholders `:
  - One line per `Placeholder::ALL` entry with `allowed_in(role)`: `{{token}}`, then
    `section`/`scalar` (`is_section`), then ` required` if it is in `required_by(role)`.
  - Then, for `review`: "wire: first 3 lines `---` / `verdict: approve|request-changes` / `---`"
    (ANA-5 `:1323-1331`).
  - For `judge`: "wire: end with one ```json block {winner, reasons}" (`:1333-1344`).
- The hint is `Ctrl+S save  Ctrl+E $EDITOR  Esc cancel  L{l}:C{c}`, 1-based from `cursor_line_col`
  (D20).

**Naming**. The pane shows `new template in {slug}: ` + `field.line(..)`; the hint is
`Enter create  Esc cancel`.

### 6.7 Tests: `crates/htui/tests/templates.rs` (`#![cfg(feature = "testkit")]`)

Helpers:
- `open()`: `Harness::demo()`, `settle`, then keys `2` and `l`, then `settle`.
- `type_text(h, s)`: maps `' '` → `"space"` and `'\n'` → `"enter"`, and each other char to itself.
- `select(h, name)`

Snapshots (`insta::assert_snapshot!("<name>", h.render())` → `templates__<name>.snap`), written
first:

| Snapshot | Drive |
|---|---|
| `templates__browse` | `open()`, then `j` to `implement`: the tree, `implement v1 phase`, the body pane |
| `templates__edit_help` | `judge`, `e`: help lists `item_key phase task candidates`, `candidates` `required`, the judge wire note, `L1:C1` |
| `templates__unknown_placeholder_cursor` | `implement`, `e` (cursor at byte 0), `down` `down`, type `{{itme}}`, `ctrl-s`: the notice names the token at its byte, and the hint shows `L3:C1`, the `{` |
| `templates__missing_item_confirm` | `n`, type `triage`, `enter` (no compiled default, so an empty editor), type `Triage {{item_key}}`, `ctrl-s`: the confirm notice, nothing sent |
| `templates__diff_two_versions` | save v2 of `implement`, then `,` to v1, `b`, `.`, `d` |
| `templates__changed_elsewhere` | `implement`, `e`, type, `append_prompt_template` v2 straight into the shared `MemStore` clone, `ctrl-s`: the stale notice and the draft |

Asserts:
- `ctrl_s_with_missing_required_puts_the_cursor_at_the_end` (a `judge` body without
  `{{candidates}}`; the hint shows the last line and column)
- `a_second_ctrl_s_saves_without_item_and_a_new_version_appears`
- `judge_and_handoff_bodies_never_get_the_item_warning`
- `a_refused_save_sends_no_request`: after `ctrl-s` on `{{itme}}` and a `settle`, the shared
  `MemStore`'s `implement` head is still v1 and the notice is the parse error, not a `Failed`.
- `saving_from_v1_while_v2_is_head_writes_v3`
- `n_creates_a_new_name_at_version_one`
- `n_refuses_an_existing_name`
- `d_upper_diffs_against_the_compiled_default`
- `external_edit_is_requested_with_the_shown_body`: `E`, then
  `h.app().take_external_edit() == Some((SkillsTab::ID, edit))` with `edit.text == body_of("implement")`.
- `an_edited_external_result_opens_the_editor_validated`:
  `h.app().finish_external_edit(SkillsTab::ID, Edited("x {{bad}}\n"))`, then the hint shows `L1:C3`,
  and a store check shows nothing was sent.
- `a_failed_external_result_keeps_the_draft`
- `an_unchanged_quick_return_names_the_wait_flag` (D24)
- `tab_and_digits_while_editing`: `2` is typed; `Tab` leaves, and `2` comes back to the draft.
- `the_strip_text_is_unchanged`: ` 1 Backlog  2 Skills  3 Settings  4 Chat`.

One unit test lives in `skills/templates.rs`'s own `mod tests`, because `settle` serves in queue
order (save first), so the Harness cannot put a read reply ahead of the save's:
**`a_read_reply_does_not_close_the_editor_mid_save`** (D27), a `#[tokio::test]`.
1. Drive `TemplatesView` directly with
   `Ctx::new(&scope, &[], &TopBarState::default(), &keymap, &theme, Origin::Tab(SkillsTab::ID), &emit)`
   (`state.rs:88-106`).
2. Open the editor on `implement` and press `ctrl-s`; the view is now busy.
3. Feed `on_reply` a `Templates` snapshot from `templates::serve` over an untouched
   `Backend::memory(MemStore::demo())`, whose head equals the token. The editor is still open and
   `busy` is still set.
4. Feed one whose head is token + 1. The editor closes.

### 6.8 Gate and commits

```bash
INSTA_UPDATE=always cargo test -p htui --all-features --test templates -- --test-threads=1
# review every new crates/htui/tests/snapshots/templates__*.snap by eye, then:
env $PGT cargo test -p htui --all-features -- --test-threads=1
git status --short crates/htui/tests/snapshots crates/htui/src/snapshots   # only new templates__* files
cargo clippy -p htui --all-features --all-targets -- -D warnings
git diff Cargo.lock | grep '^[+-] ' # exactly: +  "similar 3.2.0",   (F-F, D28)
test "$(grep -c '^name = ' Cargo.lock)" = "$(git show HEAD:Cargo.lock | grep -c '^name = ')"
```

Commits:
1. `refactor(mod-9): diff_style moves to ui::diff` (the chat snapshots unchanged).
2. `test(mod-9): the Templates view — browse, edit, save gate, diff, $EDITOR` (red: the tab split,
   `TemplatesView` with `todo!()` handlers, the tests, the `Cargo.toml` line).
3. `feat(mod-9): the Skills tab's Templates view` (green, plus the reviewed `.snap` files).

---

## 7. T6: Postgres end to end (PRD hypothesis)

`crates/htui/tests/templates_pg.rs`, `#![cfg(feature = "testkit")]`. The stack is
`runs_pg.rs:248-266`'s:
1. `testkit::demo_db()`
2. A throwaway `CacheStore::open(tmp, "templates-pg", PgStore::schema_version())`
3. `testkit::mock_keyring()`
4. `Backend::Online { pg, cache }`
5. `Harness::over_backend(backend)`, then `drive`, then assert `top_bar.store == "online"`.

Each case returns early after `testkit::SKIP` without a server.

- `a_save_through_the_worker_lands_as_version_two_on_postgres`:
  1. `2`, `l`, select `implement`, `e`, type a marker line, `ctrl-s`, `settle`.
  2. `db.store.prompt_template(PROJECT_HTUI, "implement", None)` is v2, holds the marker, and has
     `created_by == db.store.this_user()`.
- `the_preview_uses_the_new_head`: `prompt_preview.rs`'s setup (`preview_request`, `:428`) over the
  same backend. The text contains the marker only after the save.
- `an_offline_save_is_refused_without_a_row`:
  1. Serve a `SaveTemplate` to `Backend::Offline { cache, .. }`. The reply is
     `Failed { request: "save_template", .. }` with `DATABASE_UNREACHABLE`.
  2. The Postgres head is still v1.

```bash
env $PGT cargo test -p htui --all-features --test templates_pg -- --test-threads=1
```

Commit: `test(mod-9): a template saved in the TUI lands as v2 on Postgres and reaches the preview`.

---

## 8. Cross-task contracts

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T1 `htui_core::model` | `NewPromptTemplate { id, project_id, name, body, created_by }`; `PromptTemplate::name_is_valid(&str) -> bool` | T4, T5 (`n`) |
| T1 `WriteStore` | `append_prompt_template(&self, NewPromptTemplate, Option<i32>) -> Result<CasOutcome<PromptTemplate>>` | T4, T5 tests (direct write), T6 |
| T1 `htui_core::store` | `invalid_template_name`, `prompt_template_key`, `prompt_template_refusal` | T5 (`n` notice) |
| T2 `htui::ui` | `TextArea::{new, with_text, text, into_text, cursor, set_cursor, cursor_line_col, on_key(KeyEvent, u16), lines(&self, u16, u16, bool, &Theme)}`, `AreaOutcome::{Consumed, Cancel, Pass}` | T5 |
| T3 `htui::editor` | `ExternalEdit { text, stem }`, `ExternalEditOutcome::{Edited(String), Unchanged { quick }, Failed(String)}`, `EditorCommand`, `Suspend`, `run`, `run_suspended`, `QUICK_EXIT` | T5, event loop |
| T3 `htui::app` | `Action::EditExternally(ExternalEdit)`, `App::take_external_edit`, `App::finish_external_edit(TabId, ExternalEditOutcome)`, `EDITOR_NEEDS_A_TAB`; `Tab::on_external_edit` | T5 |
| T4 `htui::templates` | `TemplatesSnapshot { projects }` + `head`/`version`, `ProjectTemplates { project_id, templates }`, `REQUEST_NAMES`, `READ_NAME`; `StoreRequest::{Templates(Scope), SaveTemplate {..}}`, `StoreReply::{Templates, TemplatesStale}(Box<TemplatesSnapshot>)` | T5, T6 |
| T5 `htui::ui::diff` | `unified`, `diff_style`, `lines` | chat transcript, T5 |

**Wave A hazards**:
- There is no shared path, and `Cargo.lock` does not move in Wave A.
- `.sqlx` moves only in T1, and `htui_prepare_mod9` is T1's alone.
- T2 and T3 build `htui`, which compiles `htui-store` with `SQLX_OFFLINE=true` (`.cargo/config.toml`).
- T2 and T3 did not see T1's trait method. Nothing in them calls it, but the full `htui` suite is
  re-run after the T3 merge anyway.

---

## 9. Merge order and the workspace gate

T1 ∥ T2 ∥ T3, then merge in this order:
1. T1: the core, store and agent gates of §2.9, minus prepare.
2. T2: the `htui` lib `ui::text_area` tests and clippy.
3. T3: `htui` lib tests, the whole `htui` suite, clippy.

Then T4 → T5 → T6. At close:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
env $PGT cargo test --workspace --all-features -- --test-threads=1
psql $PG/postgres -c 'DROP DATABASE IF EXISTS htui_prepare_mod9' -c 'CREATE DATABASE htui_prepare_mod9'
DATABASE_URL=$PG/htui_prepare_mod9 sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=$PG/htui_prepare_mod9 \
   cargo sqlx prepare --check -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l                                            # 235
cargo doc --workspace --no-deps
git diff f935153 -- Cargo.lock | grep '^[+-] '                                # only +"similar 3.2.0",
git diff --stat f935153 -- crates/htui/tests/snapshots crates/htui/src/snapshots  # only new templates__*
grep -rn 'EDITOR\|VISUAL' crates --include='*.rs' | grep -v 'src/editor.rs'   # docs/tests only
```

Then the plan's live check, unchanged, with `EDITOR=/nonexistent` expected to say "could not start
`/nonexistent` (not found or not executable) — set $VISUAL or $EDITOR" (D23).

---

## 10. Count pins

| Pin | Now | After | Task / where |
|---|---|---|---|
| Store `CASES` | 56 | 59 | T1 (`conformance.rs:37`, `mem_store.rs:36-37`, `pg_conformance.rs:19`) |
| `READ_CASES` | 9 | 9 | — |
| `.sqlx` files | 234 | 235 | T1 |
| `StoreRequest` / `StoreReply` | 64 / 35 | 66 / 37 | T4 (no test pins, F-Q) |
| `Action` variants | 11 | 12 | T3 (no pin) |
| `Tab` trait defaults | 1 | 2 | T3 |
| Migrations, `TABLES`, commented columns | — | unchanged | — |
| Strip-pinning snapshots | 7 | 7, unchanged | `tests/integration.rs:58`; six `tests/snapshots/*`, one `src/snapshots/htui__testkit__tests__shell_empty.snap` |
| `Cargo.lock` packages | n | n | `htui`'s entry +1 line (T5) |

---

## 11. Risks (continuing from the plan's R-8)

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-9 | `EventStream::drop` is asynchronous (F-R); a key typed within microseconds of pressing `E` could be read by the dying helper thread and replayed after the edit. | Very low | Accepted. The editor starts only after `leave`'s terminal writes. A replayed key lands in the view as ordinary input. |
| R-10 | A panic on a background task while the editor runs fires both hooks, which write `LeaveAlternateScreen` into the editor's screen. | Low | Cosmetic. The editor survives, and `enter` redraws whole afterwards. |
| R-11 | `ort-sys`'s download (F-A) makes `htui-store` unbuildable wherever `parcel.pyke.io` is unreachable, and so every `htui` gate with it. | Certain in this sandbox | Allow the host. Longer term, `ORT_LIB_LOCATION` or a vendored binary is a TOOL item for the main thread, not this milestone. |
| R-12 | Editors that always append a final newline (`vim` with `fixeol`) turn an untouched body without a trailing LF into `Edited`. | Medium | `parse` passes it, and the user sees the editor open on unchanged text and can `Esc`. It is not normalised away, because that would change what `Unchanged` means. |
| R-13 | ETXTBSY in `editor.rs`'s script tests if another test forks while a script's write handle is open. | Low | `std::fs::write` closes the handle before any spawn, and the gate runs `--test-threads=1`. |

---

## 12. Decisions (D16 onward)

Milestone 2's plan continues after the last number here: **D37**, **R-14**. The plan's note says D16
and R-9; this blueprint moves both on.

| # | Decision |
|---|---|
| D16 | The Postgres CAS compares the head with `IS NOT DISTINCT FROM $6::int`, not `COALESCE(max, 0) = COALESCE($6, 0)`. `Some(n)` never matches an absent head, on either store (F-B). §2.4 has the full text. |
| D17 | Three shared helpers in `traits.rs`, re-exported from `store/mod.rs`: `invalid_template_name`, `prompt_template_key` (`"{project}/{name}"`, the `NotFound` id) and `prompt_template_refusal(name, body) -> Option<String>` (name rule, then `parse` in `TemplateRole::of_name`). |
| D18 | Classification order on both stores: head vs token (`Stale` / `NotFound`), then name and body (`Constraint`), then `created_by` and project (`Constraint`), then the id. Postgres gets the first step from the `WHERE`, or from a head read on the bad-input path, and the last two from FKs and the PK. |
| D19 | `TextArea.top`/`left` are `Cell<usize>`; `lines(&self, ..)` scrolls and remembers (F-C). |
| D20 | The editor's hint row ends with `L{line}:C{col}`, 1-based chars, so the cursor is observable in text snapshots (F-G). |
| D21 | `run_suspended -> io::Result<ExternalEditOutcome>`. The normal path calls `enter()?`, and the drop guard re-enters only for a cancelled future and never while panicking. A failed `leave` re-enters once and answers `Failed` without spawning (F-I, F-O). The event loop `?`s the error, and `lib.rs` restores. |
| D22 | `TerminalGuard::leave` shows the cursor, then `try_restore`. `enter` is `enable_raw_mode` + `EnterAlternateScreen` + `Terminal::clear`. It never calls `ratatui::init` (no second hook). |
| D23 | Unix exit 127/126 and Windows 9009 are start failures. The sentence names `$VISUAL`/`$EDITOR`, reads `EditorCommand.fallback` (`vi`/`notepad` defaulted), and replaces "spawn error" as the only path (F-D, F-M). |
| D24 | `ExternalEditOutcome::Unchanged { quick: bool }`, `QUICK_EXIT = 1 s` (F-K). |
| D25 | `run` sanitises the stem to `[A-Za-z0-9_-]`, maximum 32 chars (F-L), and sets `kill_on_drop(true)` on the editor child, so a dropped future reaps it. |
| D26 | T3's three shell tests live in `update.rs`'s `mod tests`, next to `Recorder`/`shell()` (F-S). |
| D27 | A `Templates` reply closes the editor only when `busy == Some("save_template")` and the snapshot's head of `(project, name)` is above the token. Any other `Templates` reply refreshes and keeps the editor, the token and `busy` (F-J). |
| D28 | The lock check is exact: T3 keeps `git diff --exit-code Cargo.lock`. T5 allows exactly `+  "similar 3.2.0",` in `htui`'s entry, with an unchanged package count (F-F). |
| D29 | T5 removes `transcript.rs:13`'s `Style` import with the `diff_style` move (F-E). |
| D30 | `conformance.rs` never backticks the Postgres race test's name or its file (F-H). |
| D31 | The race case is `crates/htui-store/tests/prompt_template_cas.rs`, `#![cfg(feature = "demo")]`, with no `query!`. |
| D32 | Wave A worktrees share one `CARGO_TARGET_DIR`, and `df -h /` must show ≥ 15 GB before Wave A starts (F-T). |
| D33 | No barrier after `drop(events)` (F-R, R-9). |
| D34 | The Skills tab opens on the `Skills` view (the plan's `2` then `l`). `wants_requests` reads templates whichever view is shown, so switching views needs no request. |
| D35 | `Edited` text is the normalised read. The comparison normalises both sides, and nothing else (no trailing-LF folding, R-12). |
| D36 | `.sqlx` is regenerated against `htui_prepare_mod9`, dropped and recreated from zero before every `prepare`, on `localhost:5432` (this box), with the TOOL-2 `USERNAME=htui-ci` prefix on every Postgres test line. |
