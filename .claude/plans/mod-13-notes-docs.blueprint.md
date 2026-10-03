# Blueprint: MOD-13 milestone 5, notes and documents

This file makes the approved plan `.claude/plans/mod-13-notes-docs.plan.md` (D1–D12, T1–T5) ready to compile. It
does not reopen any of the plan's decisions, which the maintainer CONFIRMed on 2026-10-03. Every anchor was checked on
`hr/MOD-13` at `ab6e21de`, with Gortex and line-range reads. Where this file and the plan disagree on a detail, this
file wins (§0). §6 lists what the maintainer may want to decide. None of it blocks.

## 0. Errata (plan vs. tree)

| # | Plan claim | What the tree shows | Consequence |
|---|---|---|---|
| E1 | T2 validates with `cargo test -p htui --all-features --lib hand_written store_worker`, and T3 with `… --lib editor item_form backlog` | `cargo test` takes **one** `[TESTNAME]`. A second positional fails with `unexpected argument` (milestone 2's E1) | The filters go after `--`: `cargo test -p htui --all-features --lib -- hand_written store_worker`, and `cargo test -p htui --all-features --lib -- editor item_form backlog` |
| E2 | D7: the temp-file stem is `{key}-note` or `{key}-{kind}` | A sub-tab knows only the `ItemId` (`DetailTab::on_item_change(Option<ItemId>)`, `detail/mod.rs:70`). `NotesTab`/`DocumentsTab` hold no key (`notes.rs:19-26`, `documents.rs:22-29`), and D1's replies carry none | Both panes also land `StoreReply::Item(Box<Option<Item>>)`, which the Backlog already sends with the seven reads (`backlog/mod.rs:205`) and hands to every sub-tab. They keep `key: Option<String>` when the row's `id` is the pane's item, and clear it on `on_item_change`. The stem falls back to `item-note`/`item-{kind}` with no key. Replies keep D1's shapes |
| E3 | D4: a note body is `trim_end`, then not blank; a title is trimmed, not blank, single line; a document body is as the note | Postgres `text` cannot hold U+0000 (`22021`). Neither `add_note` nor `write_document` refuses it by rule: `has_nul` (`traits.rs:1991`) is called only for persona and skill columns. A NUL would land on `MemStore` but fail on `PgStore` as `Backend`, which D10 hedges as "it may have been written" when nothing was. The kind rule (no control character) already covers NUL | This is milestone 2's E12 again. The validator refuses a NUL in the title and both bodies, with `has_nul`'s sentence and the column named: `HandWrittenError::Nul("item_note.body" / "document.title" / "document.body")`. It is a `Constraint`, so `write_refused` treats it as a refusal. The UI cannot type one (`TextArea`/`TextField` drop controls, and `without_controls` drops them from an editor return), so this guards only a direct request |
| E4 | D7: "Both can never be open together: the item form opens only when no sub-tab captures" | That guard is one-directional (`backlog/mod.rs:411`). Press `a` (a `NoteForm` read in flight, nothing captures yet), then `e`/`N`: the item form opens. When the `NoteForm` reply lands, the Notes pane opens its area behind the form. `f` gives the same race with the filter form | Routing stays correct. Keys go to the item form first (`:582-584`), so only the key owner can ask for `$EDITOR`, and the event loop runs the hand-off and its outcome back to back (`event_loop.rs:54-64`). The invariant is still made true: `BacklogTab::on_reply` does not hand a `NoteForm`/`DocumentForm` reply to the registry while `self.form` or `self.item_form` is open (§3e). The pane's `opening` token stays armed and harmless, and the next `a`/`v` re-arms it |
| E5 | D11: the four snapshots change "deliberately and in one commit"; T3 owns `notes.rs`, T4 owns `documents.rs` | If T3 drew the Notes hint, T3's commit would fail `detail_notes`/`empty_notes` in `tests/backlog.rs` until T5. If T4 drew the Docs hint, T4's commit would do the same to the Docs pair | T3 and T4 draw **no** browse hint, so the four snapshots stay byte-identical through T4. The multi-line render is identical for one-line notes (V17), and the Docs cursor is style-only (V23). Both hints, their unit tests and the four re-accepted `.snap` files land in T5's first commit (§5a) |
| E6 | D5: "— v3 was written after you opened v2; both are kept" | That wording fits `v` only, where a base version was opened, and exactly one version landed in between. For `a` nothing was opened, and more than one version may land in between | `saved_as` (§4d) covers every case. For `a` it says "after you opened the form". With more than one version in between it says "v3–v4 were written … all are kept". With no version in between it is plain "saved as plan v3" |
| E7 | D6: "`external`, the field handed to `$EDITOR`" | D6 also says Ctrl+E is handled on the body only, so only one field can ever be handed out | `external: bool`. Ctrl+E on Kind or Title is swallowed (`Stay`), as on the item form's one-line fields |
| E8 | Patterns: sub-tab tests mirror `requirements.rs`' `Shell`/`drained` | `drained` panics on any action but `Store`/`Error` (`requirements.rs:629-640`). The compose area also emits `Action::EditExternally` | One shared test bench, `#[cfg(test)] pub(super) mod bench` in `detail/compose.rs`, used by the compose, Notes and Docs tests (§3c). `ReqsTab`'s own copy is untouched |
| E9 | T3: the moved helpers' cases go in `editor` | `strip_added_newline_drops_exactly_one_editor_newline` (`item_form.rs:2336-2352`) calls the private fn directly | That test moves to `editor.rs`' `mod tests` unchanged. The item form's through-the-form tests (`:2355+`, `:2380`, `:2396`) stay where they are |

Every other anchor in the plan's Patterns and Verified-claims tables holds, within two lines:
- the store: `traits.rs:1445`, `:1566`; `pg/write.rs:5218-5219`; `mem.rs:1198-1203`, `:4230-4239`, `:5117`, `:5380-5413`; `pg/read.rs:505-507`, `:805`;
- the worker: `item_writes.rs:50`, `:199`, `:216`, `:233-234`, `:295-302`; `backend.rs:148`, `:170`, `:252`; `store_worker.rs:116-122`, `:1045-1048`, `:1327`, `:1709-1711`;
- the Backlog: `backlog/mod.rs:545`, `:588`, `:646`, `:682`, `:688-694`, `:762-768`; `app/update.rs:285-286`; `app/state.rs:355-361`;
- the item form: `item_form.rs:310`, `:325`, `:530`, `:594`, `:623`; `editor.rs:20-28`;
- the widgets: `text_area.rs:191-199`, `text_field.rs:145-153`; `cells.rs:123`; `settings/mod.rs:86`;
- the model: `kind.rs:393-395`; `template.rs:52-58`; `0001_init.sql:392`, `:399`; `fixtures.rs:1211-1220`;
- conformance: `conformance.rs:52`, `:194`, `:10457`, `:11162`; `mem_store.rs:37`; `pg_conformance.rs:26`;
- the tests: `tests/backlog.rs:80`, `:1330`, `:1846`, `:2176`.

A grep for exhaustive matches finds two over `StoreRequest`, `name()` (`store_worker.rs:931`) and `try_serve` (`:1570`), and none over `StoreReply`. Both are T2's, in one commit. `DetailTab` gains a defaulted method, so the Backlog's test probe (`backlog/mod.rs:827-846`) still compiles. The compose area has its own outcome enum, so `ItemFormOutcome` is untouched.

## 1. Task 1: the validator and the conformance case (`htui-core`)

### 1a. `crates/htui-core/src/model/hand_written.rs` (new)
The module doc says:
- what it is: D4's one validator, called by the compose areas for early feedback and by `htui::hand_written` as the authority;
- that every function answers the canonical text;
- the kind rule (A2), and why `judge`/`handoff` are not refused;
- that there is no length cap;
- the NUL rule (E3).

It needs no imports beyond `thiserror` (already a dependency, `htui-core/Cargo.toml:21`).

```rust
/// MOD-4 R-43's editable item summary: the kind the Docs pane always offers (D9).
pub const SUMMARY_KIND: &str = "summary";

/// Why a hand-written note or document is refused. Every sentence names its field (D4); the
/// worker answers it as `StoreError::Constraint` through `Display`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandWrittenError {
    /// The note is blank once trimmed.
    #[error("a note needs text")]
    BlankNote,
    /// The kind is blank once trimmed.
    #[error("a document needs a kind")]
    BlankKind,
    /// The trimmed kind holds a control character (A2: `\n`, `\r`, `\t`, U+0000 and the rest).
    #[error("a document kind must not contain a line break, a tab or another control character")]
    ControlInKind,
    /// The title is blank once trimmed.
    #[error("a document needs a title")]
    BlankTitle,
    /// The trimmed title holds `\n` or `\r`.
    #[error("a document title must be one line")]
    MultilineTitle,
    /// The document body is blank once trimmed.
    #[error("a document needs a body")]
    BlankBody,
    /// E3: U+0000, which Postgres `text` cannot store; the column as `has_nul` names it.
    #[error("{0} must not contain a NUL character")]
    Nul(&'static str),
}

/// A note body as stored: `trim_end`, leading indentation kept (markdown).
/// # Errors
/// `BlankNote`, then `Nul("item_note.body")`.
pub fn note_body(text: &str) -> Result<String, HandWrittenError>;
/// A document kind as stored: trimmed.
/// # Errors
/// `BlankKind`, then `ControlInKind`.
pub fn document_kind(text: &str) -> Result<String, HandWrittenError>;
/// A document title as stored: trimmed.
/// # Errors
/// `BlankTitle`, then `MultilineTitle`, then `Nul("document.title")`.
pub fn document_title(text: &str) -> Result<String, HandWrittenError>;
/// A document body as stored: `trim_end`.
/// # Errors
/// `BlankBody`, then `Nul("document.body")`.
pub fn document_body(text: &str) -> Result<String, HandWrittenError>;
```

Bodies:
- **The two bodies**:
  1. `let kept = text.trim_end();`
  2. if `kept.trim_start().is_empty()`, the blank variant;
  3. else if `kept.contains('\0')`, `Nul(column)`;
  4. else `Ok(kept.to_owned())`.
- **Kind**:
  1. `let kind = text.trim();`
  2. if empty, `BlankKind`;
  3. else if `kind.chars().any(char::is_control)`, `ControlInKind`;
  4. else `Ok`.
- **Title**:
  1. trim;
  2. if empty, `BlankTitle`;
  3. else if `contains(['\n', '\r'])`, `MultilineTitle`;
  4. else if it contains `'\0'`, `Nul`;
  5. else `Ok`.

`mod tests` (pure; no fixture):
1. `a_blank_note_is_refused_and_trailing_blank_lines_go`: `""` and `"  \n"` give `BlankNote`, and `"  - item\n\n"` gives `"  - item"`.
2. `a_kind_is_trimmed_and_may_hold_a_space`: `" plan "` gives `"plan"`, and `"code review"` is kept.
3. `a_blank_kind_or_one_with_a_control_character_is_refused`: `""` and `"  "` give `BlankKind`. `"pl\nan"`, `"a\0"` and `"a\tb"` give `ControlInKind`.
4. `summary_and_judge_are_ordinary_kinds`: `SUMMARY_KIND`, `"judge"` and `"handoff"` are kept.
5. `a_title_is_trimmed_one_line_and_not_blank`: `"  Plan  "` gives `"Plan"`. `" "` gives `BlankTitle`. `"a\nb"` and `"a\rb"` give `MultilineTitle`.
6. `a_document_body_is_checked_as_a_note`: `"   "` gives `BlankBody`, and `"# T\n\nx\n\n"` gives `"# T\n\nx"`.
7. `a_nul_is_refused_with_has_nul_s_sentence` (E3):
   - The three columns are refused.
   - `HandWrittenError::Nul("document.body").to_string() == crate::store::has_nul("document.body")`.
8. `every_refusal_names_its_field`: each variant's `to_string()` contains, in order, `note`, `kind`, `kind`, `title`, `title`, `body`, `NUL`.

### 1b. `crates/htui-core/src/model/mod.rs`
- Add `pub mod hand_written;` between `pub mod frontmatter;` (`:84`) and `pub mod hierarchy;` (`:85`).
- Add `pub use hand_written::HandWrittenError;` between the `event` re-export (`:113`) and the `hierarchy` one (`:114`).
- Callers use the functions as `hand_written::note_body`.

### 1c. The D12 case, in `crates/htui-core/src/store/conformance.rs`
- **The `use crate::model::{…}` list** (`:16-40`): add `Document` and `Note`. Let rustfmt order them.
- **`CASES`**: append `"hand_written_rows_round_trip",` after `"a_phase_persona_binding_sets_keeps_and_clears",` (`:182`).
- **`run_case`**: add `"hand_written_rows_round_trip" => hand_written_rows_round_trip(store).await,` after the `a_phase_persona_binding…` arm (`:457-459`).
- **The function**: put it after `a_phase_persona_binding_sets_keeps_and_clears` (ends `:15851`), before `#[cfg(test)] mod tests` (`:15853`).

```rust
/// MOD-13 milestone 5 D12: a hand-written note (`via_step_id: None`, on the fixture box) and a
/// hand-written document (`produced_by_step_id: None`) read back field-equal through `notes`,
/// `documents` and `document`. The fixture's `plan` v2 is step-produced, so the hand-written
/// version follows a step's. `gate_answers_write_their_outcome`'s note carries a step, and
/// `write_document_allocates_its_version` asserts versions and ranking only.
async fn hand_written_rows_round_trip<S: WriteStore>(store: &S) {
    const CASE: &str = "hand_written_rows_round_trip";
    // 1. NewNote { id: NoteId::new(), item_id: ids::HTUI_FEAT_1, body: "  - by hand\nsecond line",
    //    created_by: ids::USER, box_id: Some(ids::BOX), via_step_id: None, created_at: seam_clock() }.
    //    `add_note` answers the `Note` built field by field from it; `notes(FEAT_1)` holds that
    //    exact row (found by id: no order assertion, the fixture's year is not pinned to now).
    // 2. NewDocument { id: DocumentId::new(), item_id: FEAT_1, kind: "plan", title: "Plan: by hand",
    //    body: "# Plan\n\nWritten by hand.", produced_by_step_id: None, created_by: ids::USER,
    //    created_at: seam_clock() }. `write_document` answers the `Document` with `version: 3`
    //    ("the fixture holds step-produced plan v1 and v2"); `document(id)` is `Some` of it;
    //    `documents(FEAT_1)` holds `expected.head()`.
}
```
Every assert names `{CASE}`, as the neighbours do. `seam_clock` (`:4496`) truncates the clock, so Postgres reads back the same instant.

### 1d. The counts
- **`crates/htui-core/tests/mem_store.rs:37`**: change `130` to `131`. In the message, the tail "…pass and park (R-5), and MOD-26 T1's five persona cases (plan D3-D5)" becomes "…pass and park (R-5), MOD-26 T1's five persona cases (plan D3-D5), and MOD-13 milestone 5's one for hand-written rows (plan D12)".
- **`crates/htui-store/tests/pg_conformance.rs`**:
  - The doc's last sentence becomes "…and MOD-26 T1's five persona cases (plan D3-D5) make it 130, and MOD-13 milestone 5's hand-written round trip (plan D12) makes it 131."
  - `EXPECTED_CASES` (`:26`) becomes `131`.
  - The message becomes `"… (131 since MOD-13 milestone 5's hand-written round trip)"`.

Validate:
- `cargo test -p htui-core --all-features --lib hand_written`
- `cargo test -p htui-core --all-features --test mem_store`
- `cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1`

**Commits:**
1. `feat(mod-13): hand-written text validator` (1a, 1b).
2. `test(mod-13): conformance case hand_written_rows_round_trip` (1c, 1d).

## 2. Task 2: worker requests (`htui`)

### 2a. `crates/htui/src/hand_written.rs` (new)
The module doc follows `item_writes.rs:1-37`'s shape and covers D1, D2, D3, D10 and the redaction rule. Imports:
```rust
use chrono::Utc;
use htui_core::model::hand_written::{self as rules, HandWrittenError};
use htui_core::model::{Document, DocumentId, Item, ItemId, NewDocument, NewNote, NoteId};
use htui_core::store::{ReadStore as _, Result, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::store_worker::{StoreReply, StoreRequest};
```
Public surface:
```rust
/// The four requests, in [`StoreRequest::name`] order (`request_names_match_the_name_arms`).
pub const REQUEST_NAMES: [&str; 4] = ["note_form", "add_note", "document_form", "write_document"];
/// The read that opens the Notes compose area; its `Failed` opens nothing.
pub const NOTE_FORM_NAME: &str = REQUEST_NAMES[0];
/// The note write; its `Failed` is hedged unless [`write_refused`] (D10).
pub const ADD_NOTE_NAME: &str = REQUEST_NAMES[1];
/// The read that opens the Docs form; its `Failed` opens nothing.
pub const DOCUMENT_FORM_NAME: &str = REQUEST_NAMES[2];
/// The document write; hedged as [`ADD_NOTE_NAME`] is.
pub const WRITE_DOCUMENT_NAME: &str = REQUEST_NAMES[3];

/// Whether `name` is one of this module's four requests.
#[must_use] pub fn is_hand_written_request(name: &str) -> bool;

/// A note or document body on its way to the store (D1). `StoreRequest` derives `Debug`, and this
/// is user prose, so it prints its length only ([`crate::requirements::RequirementText`]'s rule).
#[derive(Clone, PartialEq, Eq)]
pub struct HandText(String);
impl HandText {
    /// Wraps `text`.
    #[must_use] pub fn new(text: impl Into<String>) -> Self;
    /// The text.
    #[must_use] pub fn as_str(&self) -> &str;
}
// impl Debug: f.debug_struct("HandText").field("len", &self.0.len()).finish()

/// The answer to `StoreRequest::DocumentForm` (D1, D3): the item, and for `v` the latest version
/// of the asked kind, hand-written included (`documents_of_kinds`). `Debug` is hand-written.
#[derive(Clone, PartialEq)]
pub struct DocumentFormContext {
    /// The item the form writes for.
    pub item: ItemId,
    /// The kind's latest version, body included, for `v`; `None` for `a` or a kind with no rows.
    pub base: Option<Document>,
}
// impl Debug: debug_struct("DocumentFormContext").field("item").field("base", &self.base.as_ref().map(DocumentDigest))
// struct DocumentDigest<'a>(&'a Document): debug_struct("Document") with id, kind, version, title,
// produced_by_step_id, body_len. Never the body (milestone 2 review L2); the title is plain (D1).

/// D10: whether `message`, a write's `Failed` as the worker renders it, was given before the
/// insert, so nothing was written. That is the offline refusal, every `Constraint` (the
/// validator's), and `NotFound { entity: "item" }` (D3's check). Anything else may follow a
/// COMMIT whose answer was lost, and the pane hedges it (`item_writes::mint_refused` plus
/// `NotFound`).
#[must_use] pub fn write_refused(message: &str) -> bool;

/// Serves one hand-written request, off the UI task (D1-D3).
/// # Errors
/// Offline: `Unreachable(DATABASE_UNREACHABLE)` for all four, before anything is read (D2).
/// `NotFound { entity: "item" }` for an unknown item (D3), before the validator. `Constraint`
/// for a validator refusal. `Backend` for a request that is not one of the four.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>;
```
`write_refused`'s body renders every prefix from the errors themselves, as `mint_refused` does (`item_writes.rs:199-205`):
```rust
let constraint = StoreError::Constraint(String::new()).to_string();
let missing = StoreError::NotFound { entity: "item", id: String::new() }.to_string(); // "item `` not found"
let item_missing = missing
    .split_once("``")
    .is_some_and(|(head, tail)| message.starts_with(head) && message.ends_with(tail));
message == offline().to_string() || message.starts_with(&constraint) || item_missing
```
The private helpers copy `item_writes.rs:294-326`: `write_access`, `offline`, `existing`, plus `fn refused(err: HandWrittenError) -> StoreError { StoreError::Constraint(err.to_string()) }`.

**`serve`'s arms.** The order is writer, item, validator, identity, write:
```rust
StoreRequest::NoteForm { item } => {
    let writer = write_access(backend)?;
    existing(&writer, *item).await?;
    Ok(StoreReply::NoteForm { item: *item })
}
StoreRequest::AddNote { item, body } => {
    let writer = write_access(backend)?;
    existing(&writer, *item).await?;
    let body = rules::note_body(body.as_str()).map_err(refused)?;
    let created_by = backend.this_user().await?;
    let box_id = backend.box_info().await?.map(|row| row.box_id);
    writer.add_note(NewNote { id: NoteId::new(), item_id: *item, body, created_by, box_id,
                              via_step_id: None, created_at: Utc::now() }).await?;
    Ok(StoreReply::NoteAdded { item: *item })
}
StoreRequest::DocumentForm { item, kind } => {
    let writer = write_access(backend)?;
    existing(&writer, *item).await?;
    let base = match kind {
        Some(kind) => writer.documents_of_kinds(*item, core::slice::from_ref(kind)).await?.into_iter().next(),
        None => None,
    };
    Ok(StoreReply::DocumentForm(Box::new(DocumentFormContext { item: *item, base })))
}
StoreRequest::WriteDocument { item, kind, title, body } => {
    let writer = write_access(backend)?;
    existing(&writer, *item).await?;
    let kind = rules::document_kind(kind).map_err(refused)?;
    let title = rules::document_title(title).map_err(refused)?;
    let body = rules::document_body(body.as_str()).map_err(refused)?;
    let created_by = backend.this_user().await?;
    let row = writer.write_document(NewDocument { id: DocumentId::new(), item_id: *item, kind, title,
        body, produced_by_step_id: None, created_by, created_at: Utc::now() }).await?;
    Ok(StoreReply::DocumentWritten { item: *item, kind: row.kind, version: row.version })
}
other => Err(StoreError::Backend(format!("not a hand-written request: {}", other.name()))),
```

### 2b. `crates/htui/src/lib.rs`
Add `pub mod hand_written;` between `pub mod event_loop;` (`:22`) and `pub mod hierarchy;` (`:23`) (A1).

### 2c. `crates/htui/src/store_worker.rs`
- **Import**: add `use crate::hand_written::{self, DocumentFormContext, HandText};` after `:52` (`crate::hierarchy`). Let rustfmt order it.
- **`StoreRequest`**: append after `EditItem { … }` (ends `:926`):
  ```rust
  /// MOD-13 milestone 5 D1-D3: the read that opens the Notes compose area; answered with
  /// [`StoreReply::NoteForm`]. Refused offline before anything is read (D2).
  NoteForm { /// The item the note is for.
      item: ItemId },
  /// A hand-written note (`R-ENT-11`); the worker fills id, author, box and clock. Answered with
  /// [`StoreReply::NoteAdded`].
  AddNote { /// The item. 
      item: ItemId, /// The text; prints as its length (D1).
      body: HandText },
  /// The read that opens the Docs form: `kind` is `None` for `a`, the row's kind for `v`, whose
  /// latest version prefills the form (D9). Answered with [`StoreReply::DocumentForm`].
  DocumentForm { item: ItemId, kind: Option<String> },
  /// A hand-written document at the store's next version of `kind` (`R-ENT-12`, D5: never a
  /// compare-and-set). Answered with [`StoreReply::DocumentWritten`].
  WriteDocument { item: ItemId, kind: String, title: String, body: HandText },
  ```
  Each field gets its own `///` line, as the neighbours have.
- **`name()`**: after `Self::EditItem { .. } => "edit_item",` (`:1048`):
  ```rust
  // The four of `hand_written::REQUEST_NAMES`, in that order (MOD-13 milestone 5 D1).
  Self::NoteForm { .. } => "note_form",
  Self::AddNote { .. } => "add_note",
  Self::DocumentForm { .. } => "document_form",
  Self::WriteDocument { .. } => "write_document",
  ```
  No existing name collides. Gortex finds none of the four, and `ORCH_NAMES` (`htui-worker/src/views.rs:28`) holds none.
- **`StoreReply`**: insert after `ItemDiverged(…)` (`:1335`), before `Failed`:
  ```rust
  /// Answer to [`StoreRequest::NoteForm`]: the item exists and the store takes a write.
  NoteForm { /// The item. 
      item: ItemId },
  /// A note that landed (self-naming, MOD-59): the Notes pane closes its area on this alone.
  NoteAdded { item: ItemId },
  /// Answer to [`StoreRequest::DocumentForm`]; boxed, it carries a whole `Document`.
  DocumentForm(Box<DocumentFormContext>),
  /// A document that landed (self-naming): `version` is the one the store allocated (D5).
  DocumentWritten { item: ItemId, kind: String, version: i32 },
  ```
- **`try_serve`**: insert after the item or-arm (`:1708-1711`):
  ```rust
  // The four hand-written requests, or-ed for the reason the arms above are (MOD-13 milestone 5 D1).
  StoreRequest::NoteForm { .. }
  | StoreRequest::AddNote { .. }
  | StoreRequest::DocumentForm { .. }
  | StoreRequest::WriteDocument { .. } => hand_written::serve(backend, request).await?,
  ```

### 2d. Task 2 tests (`hand_written.rs` `mod tests`)
Helpers: `demo()` (copied from `item_writes.rs:351-354`), `strings`, `note(item, body)`, `write(item, kind, title, body)`, and `async fn offline_backend(name) -> (TempDir, Backend)`.
1. `request_names_match_the_name_arms`: one sample of each request, mapped through `name()`, equals `REQUEST_NAMES`. Each name is `is_hand_written_request`, and `"notes"` and `"item_form"` are not.
2. `the_note_form_read_answers_its_item`: `NoteForm { HTUI_FEAT_1 }` answers `StoreReply::NoteForm { item: HTUI_FEAT_1 }`.
3. `add_note_lands_a_hand_written_note_by_this_user_on_this_box`:
   - It answers `NoteAdded { HTUI_FEAT_1 }`.
   - `store.notes(FEAT_1)` has 3 rows. The new one has the body trimmed (`"Hi.\n\n"` gives `"Hi."`), `created_by == ids::USER`, `box_id == Some(ids::BOX)` and `via_step_id == None`.
4. `the_document_form_read_carries_the_latest_version_of_its_kind`:
   - `kind: None` gives `base: None`.
   - `Some("plan")` on `HTUI_FEAT_1` gives base `DOC_FEAT_1_PLAN_V2` at version 2, with its body.
   - `Some("review")` gives `None`.
5. `write_document_lands_the_next_version_by_hand`:
   - `plan` on FEAT-1 gives `DocumentWritten { item: FEAT_1, kind: "plan", version: 3 }`. Read back via `documents_of_kinds`, it has `produced_by_step_id: None` and `created_by == ids::USER`.
   - `" review "` lands `review` v1 (the kind is trimmed).
   - `summary` on `HTUI_ANA_2` (open) lands v1.
6. `an_unknown_item_is_not_found_for_all_four`: with `ItemId::new()`, all four answer `Err(NotFound { entity: "item", id })` through `serve`.
7. `every_refusal_fails_naming_the_field_and_writes_nothing`. This goes through `store_worker::serve`, so the or-arm is covered.

   | case | expected substring |
   |---|---|
   | `AddNote "  \n"` | `note` |
   | `WriteDocument` kind `""` | `kind` |
   | kind `"pl\nan"` | `kind` |
   | title `" "` | `title` |
   | title `"a\nb"` | `title` |
   | body `"   "` | `body` |
   | body `"x\0"` | `NUL` |

   Each answers `Failed { request: request.name(), message }`, with `message` starting `constraint violated: `. Note and document counts on FEAT-1 are unchanged after each case.
8. `offline_every_hand_written_request_is_refused_before_anything_is_read`:
   - The backend is `Backend::Offline { cache: CacheStore::open(root, "hand-written-offline", 1), since: None }`. The cache is empty, so a read would answer `NotFound`, not the refusal.
   - All four answer `Unreachable(DATABASE_UNREACHABLE)` through `serve`.
   - Through `store_worker::serve`, each is a `Failed` naming its request and containing `DATABASE_UNREACHABLE`.
9. `write_refused_tells_a_refusal_from_a_store_failure`:
   - True for the rendered `Failed` of the offline `AddNote`, a blank `AddNote` (`Constraint`), and an `AddNote` on an unknown item (`NotFound`).
   - False for `Backend("connection reset by peer")`, `Unreachable("connection reset")` and `NotFound { entity: "app_user", .. }`, each through `to_string()`.
10. `requests_and_the_form_reply_debug_print_no_body`:
    - The sentinel `SECRET-BODY` is absent from `{:?}` and `{:#?}` of `AddNote`, `WriteDocument` and `StoreReply::DocumentForm`. The reply's base is the v2 row with its body replaced by the sentinel.
    - The `WriteDocument` print holds the title, and the reply print holds `body_len`.
11. `a_request_that_is_not_hand_written_is_named`: `serve(BoxInfo)` gives `Backend("not a hand-written request: box_info")`.

Validate (E1): `cargo test -p htui --all-features --lib -- hand_written store_worker`.

**Commit:** `feat(mod-13): the hand-written worker requests` (2a–2d, in one commit: `name()` and `try_serve` are exhaustive).

## 3. Task 3: shared plumbing and the Notes sub-tab

### 3a. `crates/htui/src/editor.rs`: the helpers move (D7)
- Insert after `impl core::fmt::Debug for ExternalEditOutcome` (`:152-165`), before `pub trait Suspend` (`:167`):
  - `strip_added_newline` (from `item_form.rs:306-316`);
  - `without_controls` (from `:318-333`).
- Both become `#[must_use] pub(crate) fn`. Bodies and docs are verbatim, plus "Shared by the item form and the detail compose area (MOD-13 milestone 5 D7)".
- In `without_controls`' doc, the link becomes `` [`TextArea::on_paste`](crate::ui::TextArea::on_paste) ``. A bare link from `editor.rs` is broken, and `rustdoc::broken_intra_doc_links` is `deny` (workspace `Cargo.toml`).
- `mod tests`:
  - E9: move `strip_added_newline_drops_exactly_one_editor_newline` in verbatim, after `stems_are_sanitised_and_capped` (`:549-554`).
  - Add `without_controls_keeps_line_breaks_and_tabs`: `"a\tb\n\u{1b}[31mc\u{7}"` gives `"a\tb\n[31mc"`.

### 3b. `crates/htui/src/ui/tabs/backlog/item_form.rs`
- Delete `:306-333`.
- `:41` becomes `use crate::editor::{EDITED, ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG, strip_added_newline, without_controls};`. Calls at `:545-546` are unchanged.
- `fn ctrl_e` (`:276`) becomes `pub(super) fn ctrl_e`, as `ctrl_s` (`:270`) is. `const fn marker` (`:1015`) becomes `pub(super) const fn marker`. `pub(super)` here means `backlog` and its descendants, which include `backlog::detail::compose`.
- Delete the moved test at `:2336-2352`. Behaviour is unchanged.

### 3c. `crates/htui/src/ui/tabs/backlog/detail/compose.rs` (new): D6
The module doc covers:
- the shared area of the Notes and Docs panes;
- key order;
- Ctrl+E on the body only;
- the outcome back via `on_external_edit`;
- busy;
- redaction.

Imports: `crossterm::event::{KeyCode, KeyEvent}`; `htui_core::model::{Document, ItemId, hand_written}`; `ratatui::{Frame, layout::{Constraint, Layout, Rect}, text::{Line, Span}, widgets::{Block, Borders, Paragraph}}`; `crate::editor::{EDITED, ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG, strip_added_newline, without_controls}`; `crate::hand_written::{ADD_NOTE_NAME, HandText, WRITE_DOCUMENT_NAME}`; `crate::store_worker::StoreRequest`; `crate::ui::cells::{self, cell_width}`; `crate::ui::tabs::backlog::filter::CHORD`; `crate::ui::tabs::backlog::item_form::{HINT_AREA, HINT_TEXT, PAGE, ctrl_e, ctrl_s, marker}`; `crate::ui::tabs::settings::wrapped`; `crate::ui::{FieldOutcome, TextArea, TextField, Theme}`.

```rust
/// Width of a row's label, after the two-cell marker: `title` plus two.
const LABEL: usize = 7;
/// Rows the body keeps when a long notice grows (the item form's `BODY_MIN`).
const BODY_MIN: u16 = 3;

/// One focusable part of an area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// A document's kind, typed (`a`) only.
    Kind,
    /// A document's title.
    Title,
    /// The note or document body.
    Body,
}

/// What one key did, for the pane to act on (`ItemFormOutcome`'s shape).
#[derive(Debug)]
pub enum ComposeOutcome {
    /// Edited, moved, refused or swallowed; nothing to send.
    Stay,
    /// A chord the area does not own (`ctrl-c`): the pane passes it.
    Pass,
    /// `Esc`: close the area and drop the text.
    Cancel,
    /// Checked and ready; the area is already busy with it.
    Save(StoreRequest),
    /// Ctrl+E on the body (D7): the pane emits `Action::EditExternally`.
    External(ExternalEdit),
}

/// A document's kind field: typed for `a`, fixed for `v` (D9).
enum KindField { Typed(TextField), Fixed(String) }

/// The open compose area (D6). `Debug` is hand-written: lengths, never text.
pub struct Compose {
    item: ItemId,
    /// The item key for the temp-file stem, or `item` (E2).
    owner: String,
    kind: Option<KindField>,     // None for a note
    title: Option<TextField>,    // None for a note
    body: TextArea,
    focus: Part,
    busy: Option<&'static str>,  // ADD_NOTE_NAME / WRITE_DOCUMENT_NAME
    notice: Option<String>,
    external: bool,              // E7
}
```
`impl Debug for Compose` prints these fields:
- `item`;
- `kind`: the fixed string, or `"typed"` plus its length;
- `title_len`, `body_len` and `focus`;
- `busy`;
- `notice_len`;
- `external`.

It ends in `finish_non_exhaustive`. `KindField` gets the same treatment: a hand-written or derived `Debug` that prints only the fixed string or a length. `TextField`'s `Debug` is already length-only (`text_field.rs:77-85`).

```rust
impl Compose {
    /// `a` in Notes, after `NoteForm` answered: body only, focused, empty.
    #[must_use] pub fn note(item: ItemId, owner: Option<&str>) -> Self;
    /// `a` in Docs (`kind: None`: kind, title and body empty, focus Kind) or `v` (`kind: Some`:
    /// fixed; title and body from `base`, empty without one; focus Body).
    #[must_use] pub fn document(item: ItemId, owner: Option<&str>, kind: Option<String>, base: Option<&Document>) -> Self;
    /// The request in flight, by name.
    #[must_use] pub fn busy(&self) -> Option<&'static str>;
    /// The refusal, hedge or `$EDITOR` notice.
    #[must_use] pub fn notice(&self) -> Option<&str>;
    /// The kind a save would send: the fixed one, or the typed text trimmed (`None` while blank).
    #[must_use] pub fn kind(&self) -> Option<String>;
    /// Whether the kind is typed (`a`): the kinds hint is drawn only then.
    #[must_use] pub fn kind_is_typed(&self) -> bool;
    /// A reply ended the flight: busy cleared, the notice set or cleared.
    pub fn settle(&mut self, notice: Option<String>);
    /// Feeds one key (order below).
    pub fn on_key(&mut self, key: KeyEvent) -> ComposeOutcome;
    /// Into the focused field; dropped while busy or on a fixed kind.
    pub fn on_paste(&mut self, text: &str);
    /// D7: the item form's `on_external_edit` rules, for the body.
    pub fn on_external_edit(&mut self, outcome: ExternalEditOutcome);
}
```

**`on_key`'s order** follows `item_form.rs:610-680`:
1. `ctrl_s(&key)`: busy gives `Stay`, else `self.save()`.
2. `ctrl_e(&key)`: busy gives `Stay`. Focus `Body` gives `self.hand_off()`. Kind or Title gives `Stay` (E7).
3. `key.modifiers.intersects(CHORD)` gives `Pass`, so `ctrl-c` quits.
4. Busy gives `Stay` for every other key, `Esc` included.
5. `Tab`/`BackTab` cycle `self.parts()` and give `Stay`:
   - a note: `[Body]`;
   - a fixed kind: `[Title, Body]`;
   - a typed kind: `[Kind, Title, Body]`.
6. The focused field:
   - A `TextField` (a typed Kind, or Title):
     - `Submit` (Enter) moves the focus forward.
     - `Cancel` gives `Cancel`.
     - `Pass` with Up/Down moves the focus.
     - Every other outcome gives `Stay`.
   - Body, through `body.on_key(key, PAGE)`: `Cancel` gives `Cancel`, and every other outcome gives `Stay`.

**`save`:**
- A note:
  - `hand_written::note_body(body.text())`.
  - On `Err(e)`: `notice = Some(e.to_string())` and `Stay`.
  - On `Ok(text)`: `busy = Some(ADD_NOTE_NAME)`, then `Save(StoreRequest::AddNote { item, body: HandText::new(text) })`.
- A document:
  - It checks kind, title, then body (`document_kind`, `document_title`, `document_body`). The fixed kind is checked too, which is harmless.
  - The first refusal sets the notice, moves the focus to that part (a fixed kind's never fails), and answers `Stay`.
  - Otherwise `busy = Some(WRITE_DOCUMENT_NAME)` and `Save(WriteDocument { item, kind, title, body: HandText::new(body) })`, with the canonical values.

**`hand_off`:** `external = true`, then `External(ExternalEdit { text: body.text().to_owned(), stem })`. The notice is left alone. The stem is:
- `{owner}-note` for a note;
- `{owner}-{kind}` for a document, where kind is `self.kind()` or `document` while blank. `editor::run` sanitises it.

**`on_external_edit`:**
- `if !std::mem::take(&mut self.external) { return; }`.
- `Edited(r)`:
  1. `let r = without_controls(&r);`
  2. `let text = strip_added_newline(self.body.text(), &r);`
  3. If `text == body.text()`: the notice is `NO_CHANGES`.
  4. Else: `body = TextArea::with_text(text)`, `body.set_cursor(usize::MAX)`, `focus = Body`, and the notice is `EDITED`.
- `Unchanged { quick }`: the notice is `format!("{NO_CHANGES}{}", if quick { WAIT_FLAG } else { "" })`.
- `Failed(m)`: the notice is `m`.

These are `item_form.rs:530-563` verbatim, for the body.

```rust
/// D10: a write `Failed` that may have followed a COMMIT whose answer was lost (the mint hedge's
/// wording, `item_form::mint_may_have_landed`); `reread` is `thread` or `list`.
#[must_use]
pub fn may_have_landed(why: &str, reread: &str) -> String {
    format!("{why} \u{2014} it may have been written; the {reread} is being re-read, look for it before Ctrl+S")
}

/// Draws the area over the sub-tab's whole content rect (D6): a top rule titled `title`, the
/// one-line rows, the body, the notice and the hint. `kinds` is the Docs pane's hint text,
/// drawn under a typed kind only.
pub fn render(frame: &mut Frame<'_>, area: Rect, compose: &Compose, title: &str, kinds: Option<&str>, theme: &Theme);
```

**Render layout.** The top rule is `Block::new().borders(Borders::TOP).title(cells::clip(title, width))`. It costs one row and no column, so the inner width stays the pane's 43 at 100x30. The inner rect is 43x23.

| Row | Notes | Docs `a` | Docs `v` |
|---|---|---|---|
| `kind` row: `marker(focused)` + `format!("{:<LABEL$}", "kind")` + `TextField::line(room)` (typed), or the fixed kind in `theme.dim` | — | 1 | 1 |
| kinds hint: `cells::clip(&format!("  kinds: {kinds}"), w)` in `theme.dim` | — | 1 | — |
| `title` row (as `kind`) | — | 1 | 1 |
| body label: `marker(focus == Body)` + `"body"`, in `theme.title` when focused, else `theme.dim` | — | 1 | 1 |
| body: `body.lines(w, h, focus == Body, theme)` | `Min(body_min)` | `Min` | `Min` |
| notice: `wrapped(notice, w)` in `theme.error`, as the item form draws every notice (`item_form.rs:1100-1108`) | `Length(n)` | `Length(n)` | `Length(n)` |
| hint: `cells::clip(match focus { Kind \| Title => HINT_TEXT, Body => HINT_AREA }, w)` in `theme.dim` | 1 | 1 | 1 |

- **Notice height** follows the item form's rule (`item_form.rs:1037-1058`): `room = inner.height - fixed rows`; `n = wanted.min(room)`; `body_min = BODY_MIN.min(room - n)`; when `wanted > n`, drop the notice's first `wanted - n` rows. The hedge ends the sentence and must stay visible.
- **Hint widths:** `HINT_AREA` is 39 cells and `HINT_TEXT` 34, against 43. `kinds: plan, prd, summary` is 25. With `LABEL` 7, a row's head is 9 cells, which leaves 34 for the field.

`#[cfg(test)] pub(super) mod bench` (E8) holds:
- `pub struct Shell` (scope, projects, top bar, keymap, theme, `Emit`), as `requirements.rs:600-626`;
- `pub fn ctx(&self) -> Ctx<'_>`, with origin `Tab(BacklogTab::ID)`;
- `pub fn actions(&self) -> Vec<Action>`;
- `pub fn requests(&self) -> Vec<String>`: each `Action::Store`'s `{:?}`, with the other actions dropped;
- `pub fn key(code) -> KeyEvent`;
- `pub fn ctrl(c) -> KeyEvent`;
- `pub fn drawn(width, height, draw: impl FnOnce(&mut Frame<'_>, Rect)) -> String`, a `TestBackend` buffer as text.

**Compose tests** (`compose.rs` `mod tests`):
1. `ctrl_s_on_a_blank_note_refuses_in_the_area_and_sends_nothing`: `Stay`, the notice is `"a note needs text"`, and `busy() == None`.
2. `ctrl_s_sends_add_note_with_the_trimmed_text_and_is_busy`: paste `"Hi.\n\n"`. `Save(AddNote)` has `body.as_str() == "Hi."`, and `busy() == Some(ADD_NOTE_NAME)`.
3. `esc_cancels`: `Esc` gives `Cancel`.
4. `ctrl_c_and_other_chords_pass`: `ctrl('c')`, and an `ALT` `x`, give `Pass`.
5. `while_busy_every_plain_key_is_swallowed`: after test 2, `x`, `Esc`, `ctrl('s')` and `ctrl('e')` give `Stay`, and the text is unchanged.
6. `ctrl_e_on_the_body_hands_it_out_with_the_note_stem`: `note(item, Some("FEAT-1"))` with `"draft"` gives `External { text: "draft", stem: "FEAT-1-note" }`. With `owner: None` the stem is `item-note`.
7. `an_edited_outcome_lands_in_the_body_with_the_cursor_at_the_end`:
   - `Edited("New.\n")` gives body `"New."`, `body.cursor() == 4`, and the notice `EDITED`.
   - A second hand-off, then `Edited("New.\n")`, gives `NO_CHANGES`.
8. `unchanged_and_failed_keep_the_text`: `Unchanged { quick: true }` gives `NO_CHANGES` plus `WAIT_FLAG`, and `Failed("boom")` gives `"boom"`.
9. `an_outcome_with_nothing_handed_out_is_ignored`.
10. `a_typed_document_cycles_kind_title_body_and_a_fixed_kind_is_skipped`.
11. `ctrl_e_on_the_kind_or_title_is_swallowed`, and the stem with the typed kind `"code review"` is `X-code review`, before `run` sanitises it.
12. `a_document_refusal_focuses_its_part`: a blank title with a good kind leaves `focus == Title` and the notice naming `title`.
13. `a_paste_goes_to_the_focused_field`.
14. `debug_prints_no_text`: the sentinel `SECRET` is pasted into title and body, and `{:?}` holds neither.
15. `the_area_draws_its_hint_inside_the_pane`: a note at 43x23 holds `HINT_AREA`. A typed document focused on Kind holds `HINT_TEXT` and `kinds: plan`.

### 3d. `crates/htui/src/ui/tabs/backlog/detail/mod.rs`
- Add `pub mod compose;` between `pub mod body;` (`:10`) and `pub mod documents;` (`:11`). Import `use crate::editor::ExternalEditOutcome;` after `:25`.
- Module doc (`:1-8`): append "MOD-13 milestone 5: Notes and Docs write, each through a [`compose`] area of its own (D6), and an `$EDITOR` outcome comes back to the active sub-tab (D7)."
- **The trait**: insert after `has_active_run` (`:96-98`), before `:99`:
  ```rust
  /// MOD-13 milestone 5 D7: an `$EDITOR` outcome the Backlog had no item form for. Only the
  /// sub-tab that emitted `Action::EditExternally` acts on it; the default drops it.
  fn on_external_edit(&mut self, _outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {}
  ```
- **`DetailRegistry`**: insert after `on_key` (`:237-242`):
  ```rust
  /// D7: hands an `$EDITOR` outcome to the active sub-tab, the only one that could have asked
  /// (a capturing sub-tab keeps every key, so the strip cannot move while its area is open).
  pub fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
      match self.tabs.get_mut(self.active) {
          Some(tab) => tab.on_external_edit(outcome, ctx),
          None => tracing::debug!("no detail sub-tab to take the $EDITOR outcome"),
      }
  }
  ```

### 3e. `crates/htui/src/ui/tabs/backlog/mod.rs`
- **Module doc** (`:21-23`): append "Milestone 5: Notes and Docs write through their own compose areas. An `$EDITOR` outcome goes to the item form when one is open, else to the active sub-tab."
- **`on_external_edit`** (`:685-694`):
  ```rust
  /// MOD-13 milestone 4 D4, milestone 5 D7: the item form when one is open (it ignores an
  /// outcome it did not ask for), else the active detail sub-tab. Both cannot be asking: the
  /// key owner is the asker, and the item form owns keys first.
  fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
      match self.item_form.as_mut() {
          Some(form) => form.on_external_edit(outcome),
          None => self.detail.on_external_edit(outcome, ctx),
      }
  }
  ```
- **E4 guard in `on_reply`**: insert immediately before `self.detail.on_reply(reply, ctx);` (`:682`):
  ```rust
  // MOD-13 milestone 5 E4, the `:411` rule one level down: a compose area opens only while
  // neither form captures above it, so the item form and a sub-tab's area are never both open.
  if matches!(reply, StoreReply::NoteForm { .. } | StoreReply::DocumentForm(_))
      && (self.form.is_some() || self.item_form.is_some())
  {
      return;
  }
  ```

Tests, reusing `Bench`, `open_with` and `press`, plus a new probe:
```rust
/// A sub-tab that records the `$EDITOR` outcomes it is handed.
#[derive(Debug, Default)]
struct OutcomeProbe { seen: Rc<RefCell<Vec<ExternalEditOutcome>>> }
// DetailTab: id "outcomes", title "Outcomes", no-op on_item_change/on_reply/render, on_key Pass,
// on_external_edit pushes.
```
1. `an_outcome_with_no_item_form_reaches_the_active_sub_tab`: `bench.tab()` with `tab.detail` replaced by a registry holding only the probe. `on_external_edit(Edited("x"))` leaves the probe's `seen == [Edited("x")]`, and `bench.actions()` is empty.
2. `an_open_item_form_takes_the_outcome_not_the_sub_tab`:
   - `open_with(.., 'e')`, then swap the probe registry in, so the form stays open.
   - `to_body`, then `ctrl('e')`.
   - `on_external_edit(Edited("New body."))` leaves the form's notice `EDITED`, and the probe's `seen` empty.
3. `a_compose_read_answered_under_an_open_form_opens_nothing` (E4). It uses a second probe, `ComposeProbe`, whose `on_reply` sets its `capturing` flag on `StoreReply::NoteForm { .. }`, so the test does not depend on `notes.rs`:
   - `bench.tab()` with a registry holding only that probe. `press(.., KeyCode::Char('f'))` opens the filter form.
   - `tab.on_reply(&StoreReply::NoteForm { item })` leaves `tab.detail.captures_input()` false: the guard held the reply back.
   - `Esc` closes the filter form. A second `on_reply(NoteForm)` now reaches the probe, and `captures_input()` is true.
4. `an_outcome_with_no_form_open_is_dropped` (`:2864-2875`) stays green: Body is active and its default drops it. Its doc becomes "D4, milestone 5 D7: with no form open the outcome reaches the active sub-tab; Body asked for nothing and drops it".

### 3f. `crates/htui/src/ui/tabs/backlog/detail/notes.rs`: D8
The module doc gains the keys and D10.
```rust
#[derive(Debug, Default)]
pub struct NotesTab {
    notes: Vec<Note>,
    item: Option<ItemId>,
    /// The selected item's key, from `StoreReply::Item` (E2): the temp-file stem.
    key: Option<String>,
    scroll: Scroll,
    /// The `NoteForm` read `a` sent; its reply opens the area only for this item.
    opening: Option<ItemId>,
    /// The open compose area (D6).
    compose: Option<Compose>,
}
```
- **`lines(&self, theme: &Theme)`**: per note:
  - a dim stamp line;
  - then **one `Line` per `note.body.split('\n')`** row, in `theme.base` (D8);
  - blank-separated, as now.

  `split('\n')`, not `lines()`, so a one-line body (and `""`) gives exactly the one `Line` it gives today (V17). `render` keeps `Paragraph`, `Wrap { trim: false }` and `scroll`.
- **`on_item_change`**: also clears `key`, `opening` and `compose`.
- **`on_key`**:
  1. With `compose` open, map its outcome:
     - `Stay` gives `Consumed`.
     - `Pass` gives `Pass`.
     - `Cancel` drops the area and gives `Consumed`.
     - `Save(r)`: `ctx.request(r)`, then `Consumed`.
     - `External(e)`: `ctx.emit(Action::EditExternally(e))`, then `Consumed`.
  2. A `CONTROL`/`ALT` chord gives `Pass`.
  3. `Char('a')` with an item: `opening = Some(item)`, `ctx.request(NoteForm { item })`, `Consumed`. With no item, `Pass`.
  4. Otherwise `self.scroll.on_key(key, self.lines(ctx.theme).len())`, a real row count (D8; was `len * 3`).
- **`captures_input`** is `self.compose.is_some()`. **`on_paste`** gives the text to the compose area and answers `Consumed`, else `Pass`. **`on_external_edit`** goes to `compose`, else a `tracing::debug!`.
- **`on_reply`** (D1's routing: own requests, own item):
  ```rust
  match reply {
      StoreReply::Notes(notes) => { self.notes = notes.clone(); self.scroll.reset(); }
      StoreReply::Item(row) => if let Some(row) = row.as_ref() && Some(row.id) == self.item { self.key = Some(row.key.clone()); }
      StoreReply::NoteForm { item } if self.opening == Some(*item) && Some(*item) == self.item => {
          self.opening = None;
          self.compose = Some(Compose::note(*item, self.key.as_deref()));
      }
      StoreReply::NoteAdded { item } if Some(*item) == self.item
          && self.compose.as_ref().and_then(Compose::busy) == Some(ADD_NOTE_NAME) => {
          self.compose = None;
          ctx.request(StoreRequest::Notes(*item));
      }
      StoreReply::Failed { request, .. } if *request == NOTE_FORM_NAME => self.opening = None, // D2: App said it
      StoreReply::Failed { request, message } if *request == ADD_NOTE_NAME => {
          let Some(compose) = self.compose.as_mut().filter(|c| c.busy() == Some(ADD_NOTE_NAME)) else { return };
          if write_refused(message) { compose.settle(Some(message.clone())); }
          else {
              compose.settle(Some(may_have_landed(message, "thread")));
              if let Some(item) = self.item { ctx.request(StoreRequest::Notes(item)); }
          }
      }
      _ => {}
  }
  ```
  A `Failed` names no item, so a stale one is dropped by the staleness gate (one `(origin, AddNote)` key, `app/state.rs:311-313`). One for an item left behind finds no busy area: `on_item_change` dropped it.
- **`render`**:
  - With `compose`: `compose::render(frame, area, c, " New note ", None, ctx.theme)`.
  - Else as today: no item, no notes, or the thread. There is no browse hint until §5a (E5).

**Notes tests** (`notes.rs` `mod tests`, over `compose::bench`):
1. `a_sends_note_form_and_nothing_without_an_item`.
2. `the_note_form_reply_opens_the_area_and_it_captures`.
3. `a_note_form_reply_for_another_item_or_unasked_is_ignored`.
4. `ctrl_s_sends_add_note_with_the_text`: `requests()` holds `AddNote` with `HandText { len: 3 }`. The Debug shows the length, not `Hi.`.
5. `note_added_closes_the_area_and_rereads_the_thread`: `requests() == [format!("{:?}", StoreRequest::Notes(item))]`.
6. `a_refusal_keeps_the_text_and_says_the_sentence`: `Failed { ADD_NOTE_NAME, "constraint violated: a note needs text" }`.
7. `a_store_failure_hedges_and_rereads`:
   - `"store backend error: connection reset"` gives the notice `may_have_landed(.., "thread")`.
   - It sends a `Notes(item)` request.
   - The text is kept, and `busy() == None`.
8. `a_failed_note_form_opens_nothing_and_adds_no_error`: `actions()` is empty, and a late `NoteForm` reply opens nothing.
9. `ctrl_e_emits_an_external_edit_with_the_key_stem`: after `StoreReply::Item(Some(FEAT-1 row))`, the stem is `FEAT-1-note`.
10. `an_edited_outcome_reaches_the_area_through_on_external_edit`.
11. `a_two_line_note_renders_as_two_rows`: `lines()` of one note `"a\nb"` is 3 `Line`s (stamp, `a`, `b`).
12. `a_one_line_note_renders_as_before`: the demo notes' `lines()` equals the old shape (stamp, body, blank, stamp, body) span for span.
13. `w_passes_while_browsing` (D11): `w` gives `Handled::Pass`.
14. `an_item_change_drops_the_area`.

Validate (E1): `cargo test -p htui --all-features --lib -- editor item_form backlog`.

**Commits:**
1. `refactor(mod-13): the $EDITOR return helpers live in crate::editor` (3a, 3b).
2. `feat(mod-13): the shared compose area` (3c, plus `pub mod compose;` from 3d).
3. `feat(mod-13): $EDITOR outcomes reach the active detail sub-tab` (the rest of 3d, and 3e).
4. `feat(mod-13): the Notes sub-tab writes a note` (3f).

Order: 1, 2, 3, 4. §3e's tests use probes only, so commit 3 does not depend on `notes.rs`.

## 4. Task 4: the Docs sub-tab (`detail/documents.rs`, D5, D9, D10)

### 4a. State
```rust
/// What a `WriteDocument` in flight expects (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expected {
    /// `opened + 1` for `v`; for `a`, the highest version of the kind in the pane's list, plus one.
    version: i32,
    /// The base version `v` opened; `None` for `a`.
    opened: Option<i32>,
}

#[derive(Debug, Default)]
pub struct DocumentsTab {
    documents: Vec<DocumentHead>,
    item: Option<ItemId>,
    key: Option<String>,                          // E2
    /// Index of the row under the cursor (D9; drawn by style only).
    cursor: usize,
    scroll: Scroll,
    /// The `DocumentForm` read in flight: the item and `v`'s kind.
    opening: Option<(ItemId, Option<String>)>,
    /// The base version the open form came from (`v`), for [`Expected`].
    opened: Option<i32>,
    expected: Option<Expected>,
    compose: Option<Compose>,
    /// D5: the last save's sentence, under the table until the next key.
    notice: Option<String>,
}
```
`on_item_change` resets everything but the type, and `cursor` goes to 0.

### 4b. Keys
- With `compose` open: the Notes mapping (§3f). On `Save(r)`, first set `self.expected` from `r`:
  - `StoreRequest::WriteDocument { kind, .. }` gives `version = self.opened.map_or_else(|| max_version(kind) + 1, |v| v + 1)`, where `max_version` is the highest `version` among `self.documents` of that kind, or 0.
  - `opened` is `self.opened`.
- Otherwise:
  1. `self.notice = None` (D9: "until the next key").
  2. A `CONTROL`/`ALT` chord gives `Pass`.
  3. No item gives `Pass`.
  4. Match the key:
     - `J`/`K`: `move_cursor(±1)`. Clamp to the rows, and reset the scroll, as `ReqsTab` does (`requirements.rs:183-187`).
     - `a`: `open(item, None)`.
     - `v`: with a row under the cursor, `open(item, Some(row.kind.clone()))`; with none, nothing. Either way, `Consumed`.
     - Anything else: `self.scroll.on_key(key, self.documents.len())`, so PgUp/PgDn scroll as before.
- `open`:
  1. `opening = Some((item, kind.clone()))`;
  2. `notice = None`;
  3. `ctx.request(DocumentForm { item, kind })`.

### 4c. Replies
```rust
match reply {
    StoreReply::Documents(rows) => { self.documents = rows.clone(); self.cursor = self.cursor.min(rows.len().saturating_sub(1)); self.scroll.reset(); }
    StoreReply::Item(row) => /* key, as Notes */,
    StoreReply::DocumentForm(context) if matches!(&self.opening, Some((item, _)) if *item == context.item)
        && Some(context.item) == self.item => {
        let Some((item, kind)) = self.opening.take() else { return };
        // A base must be of the asked kind; anything else is not this read's answer.
        if let (Some(kind), Some(base)) = (&kind, &context.base) && base.kind != *kind { return; }
        self.opened = context.base.as_ref().map(|base| base.version);
        self.compose = Some(Compose::document(item, self.key.as_deref(), kind, context.base.as_ref()));
    }
    StoreReply::DocumentWritten { item, kind, version } if Some(*item) == self.item
        && self.compose.as_ref().and_then(Compose::busy) == Some(WRITE_DOCUMENT_NAME) => {
        let expected = self.expected.take().unwrap_or(Expected { version: *version, opened: None });
        self.notice = Some(saved_as(kind, *version, expected.version, expected.opened));
        self.compose = None; self.opened = None;
        ctx.request(StoreRequest::Documents(*item));
    }
    StoreReply::Failed { request, .. } if *request == DOCUMENT_FORM_NAME => self.opening = None,
    StoreReply::Failed { request, message } if *request == WRITE_DOCUMENT_NAME => /* as Notes, reread "list",
        ctx.request(Documents(item)); self.expected = None on either branch */,
    _ => {}
}
```

### 4d. The D5 sentence (E6)
```rust
/// D5: what a landed `WriteDocument` says under the table. `landed` past `expected` means
/// someone else wrote in between; nothing was overwritten (append-only, `0001_init.sql:399`).
#[must_use]
pub fn saved_as(kind: &str, landed: i32, expected: i32, opened: Option<i32>) -> String
```
| case | sentence |
|---|---|
| `landed <= expected` | `saved as plan v3` |
| `landed == expected + 1` | `saved as plan v4 \u{2014} v3 was written after you opened v2; both are kept` |
| `landed > expected + 1` | `saved as plan v5 \u{2014} v3\u{2013}v4 were written after you opened v2; all are kept` |
| `opened == None` (`a`) | the same, with `you opened the form` |

### 4e. Render
- **No item**: unchanged.
- **With `compose`**: `compose::render(frame, area, c, &title, Some(&self.kinds()), theme)`.
  - The title is `" New document "` for a typed kind.
  - For `v` it is `format!(" New version of {kind} (from v{opened}) ")`, or `format!(" New version of {kind} ")` with no base.
  - `fn kinds(&self) -> String`: the kinds of `self.documents` plus `SUMMARY_KIND`, in a `BTreeSet`, joined with `", "`. FEAT-1 gives `plan, prd, summary`, and ANA-2 gives `summary`.
- **Otherwise**: `[table, notice] = Layout::vertical([Min(0), Length(n)])`. `n` is the notice's `wrapped(.., width)` row count, 0 without one. With 0, the table rect is today's, so the snapshots do not move.
  - The empty message, or the table, goes in `table`. Rows use `.enumerate().skip(skip)`, and the cursor's row gets `.style(theme.selected)`.
  - `skip = self.scroll.skip().max((self.cursor + 1).saturating_sub(usize::from(table.height).saturating_sub(1)))`. The header takes one row.
  - The notice rows go in `theme.base`.

### 4f. Tests (`documents.rs` `mod tests`, over `compose::bench`; replies via `hand_written::serve(&Backend::memory(MemStore::demo()), ..)`)
1. `j_and_k_move_the_cursor_and_clamp`: FEAT-1's 3 rows; `K` at 0 stays; `J`×5 stops at 2.
2. `a_sends_document_form_with_no_kind`.
3. `v_sends_document_form_with_the_kind_under_the_cursor`: cursor 2 is `prd`.
4. `a_and_v_send_nothing_without_an_item_and_v_nothing_without_rows`: ANA-2 with no rows; `v` gives `Consumed` and no request.
5. `a_v_reply_opens_the_form_with_the_kind_fixed_and_the_base_text`:
   - `kind() == Some("plan")` and `!kind_is_typed()`.
   - The title is `Plan: TUI scaffold (revised)`, and the body is v2's.
   - The pane captures.
6. `an_a_reply_opens_an_empty_form_on_the_kind`.
7. `ctrl_s_sends_write_document`.
8. `written_at_the_expected_version_closes_rereads_and_says_saved`:
   - `v`, then Ctrl+S. `DocumentWritten { version: 3 }`.
   - The notice is `"saved as plan v3"`, the area is closed, and a `Documents(FEAT_1)` request goes out.
9. `written_past_the_expected_version_names_the_version_in_between`: the same flow with version 4 gives the E6 sentence.
10. `kind_title_and_body_refusals_show_in_the_form`.
11. `a_store_failure_hedges_and_rereads_the_list`.
12. `the_kinds_hint_names_the_item_s_kinds_and_summary`: drawn at 43x23.
13. `the_notice_clears_on_the_next_key`.
14. `w_passes_while_browsing`.
15. `saved_as_covers_every_case`: the four table rows of §4d.
16. `the_cursor_row_is_drawn_selected`: a `TestBackend` buffer cell style check on row 2 (header + 1).

Validate: `cargo test -p htui --all-features --lib -- documents compose`.

**Commit:** `feat(mod-13): the Docs sub-tab writes a document or a new version`.

## 5. Task 5: hints, integration, Postgres

### 5a. The D11 hints, one commit (E5)
- `notes.rs`: `const HINT: &str = "a add note";` (10 cells).
- `documents.rs`: `const HINT: &str = "J/K move \u{b7} a new \u{b7} v new version";` (32 cells).
- Each pane, with an item selected and no compose area, splits off a last `Length(1)` row and draws `Line::styled(cells::clip(HINT, width), theme.dim)`:
  - Notes: `[list, hint]`.
  - Docs: `[table, notice, hint]`.
- With no item: no hint (no snapshot shows that state for either pane).
- Unit tests: `the_hint_fits_the_detail_pane` (`cell_width(HINT) <= 43`) in each pane, and `the_hint_is_drawn_under_the_thread` / `…_under_the_table`.
- Re-accept exactly `backlog__detail_notes`, `backlog__detail_documents`, `backlog__empty_notes` and `backlog__empty_documents`:
  - `INSTA_UPDATE=always cargo test -p htui --features testkit --test backlog -- --test-threads=1`.
  - `git diff --stat crates/htui/tests/snapshots` must list those four files only.
  - Each diff is one line: the pane's last content row (frame row 28) gains the hint.

**Commit:** `feat(mod-13): Notes and Docs hint lines`, with the two panes and the four `.snap` files.

### 5b. `crates/htui/tests/backlog.rs`
Append a section after `offline_ctrl_e_asks_for_no_editor` (ends `:2382`):
`// Notes and documents (MOD-13 milestone 5, plan D1-D12).`
- **Imports** (`:24-31`): add `DocumentHead` and `Note` to the `htui_core::model` list.
- **Constants**: `const TO_DOCS: usize = 3;` and `const TO_NOTES: usize = 4;` (V22).
- **Helpers**:
  ```rust
  /// [`backlog_over`] on htui `FEAT-1`, `pane` sub-tabs right of Body.
  async fn on_feat_1(store: MemStore, pane: usize) -> Harness;   // backlog_over, down(TO_FEAT_1), sub_tab(pane)
  async fn notes_of(store: &MemStore, item: ItemId) -> Vec<Note>;
  async fn documents_of(store: &MemStore, item: ItemId) -> Vec<DocumentHead>;
  /// The detail pane's rows, borders and padding trimmed, joined by spaces: a wrapped notice reads whole.
  fn detail_text(frame: &str) -> String; // detail_pane(frame).lines().map(|l| l.trim_end_matches(['│', ' ']).trim()).filter(non-empty).join(" ")
  ```

**Cases.** All are 100x30.
1. `a_types_a_note_and_ctrl_s_adds_it_to_the_thread`:
   - `on_feat_1(store, TO_NOTES)`, `keys(["a"])`, `harness.paste("Written by hand.\nSecond line.")`, `keys(["ctrl-s"])`.
   - The last note has the body `"Written by hand.\nSecond line."`, `via_step_id == None`, `created_by == ids::USER` and `box_id == Some(ids::BOX)`.
   - The detail pane holds both lines on separate rows and no ` New note `, and `status == None`.
2. `v_on_plan_writes_plan_v3_by_hand`:
   - `on_feat_1(store, TO_DOCS)`, `keys(["v"])`.
   - The frame holds ` New version of plan (from v2) ` and `Plan: TUI scaffold (revised)`.
   - `harness.paste("Edited by hand.\n")` goes into the body at byte 0. Then `keys(["ctrl-s"])`.
   - `documents_of` has `plan` v3 with `produced_by_step_id == None`.
   - The detail pane holds `v3  hand`, and `detail_text` holds `saved as plan v3`. `status == None`.
3. `a_with_kind_summary_lands_summary_v1`:
   - `keys(["a"])`, `type_text("summary")`, `keys(["tab"])`, `type_text("Summary of FEAT-1")`, `keys(["tab"])`, `paste("The summary.")`, `keys(["ctrl-s"])`.
   - `summary` v1 is hand-written, the notice reads `saved as summary v1`, and the table lists `summary`.
4. `a_version_written_meanwhile_is_named_in_the_notice` (D5):
   - `keys(["v"])`. Then `store.write_document(NewDocument { id: DocumentId::new(), item_id: HTUI_FEAT_1, kind: "plan", title: "Theirs", body: "Theirs.", produced_by_step_id: None, created_by: ids::USER, created_at: Utc::now() })` lands v3.
   - `keys(["ctrl-s"])` lands v4.
   - `detail_text` contains `saved as plan v4 \u{2014} v3 was written after you opened v2; both are kept`.
   - Both v3 and v4 are listed.
5. `offline_a_opens_no_compose_in_either_pane` (D2):
   - Set up as `offline_ctrl_e_asks_for_no_editor` does (`:2367-2372`): the keyring guard, `tempdir` and `offline_backlog`. `ANA-1` is selected.
   - Record `cache.notes(HTUI_ANA_1)` and `cache.documents(HTUI_ANA_1)` lengths.
   - `sub_tab(TO_DOCS)`, status `None`, `keys(["a"])`. The status is `Some(format!("document_form: store unreachable: {DATABASE_UNREACHABLE}"))`, and there is no ` New document `.
   - `sub_tab(1)`, status `None`, `keys(["a"])` gives `note_form: …`, and there is no ` New note `.
   - Both lengths are unchanged.
6. `ctrl_e_hands_the_note_to_the_editor_and_ctrl_s_adds_it`. This mirrors `ctrl_e_hands_the_body_out_and_ctrl_s_saves_what_came_back` (`:2191-2242`):
   - `on_feat_1(.., TO_NOTES)`, `keys(["a"])`, `keys(["ctrl-e"])`.
   - `take_external_edit() == Some((BacklogTab::ID, ExternalEdit { text: String::new(), stem: "FEAT-1-note".to_owned() }))`, and a second take is `None`.
   - `finish_external_edit(BacklogTab::ID, Edited("From the editor.\n"))`. The frame holds `edited in $EDITOR`.
   - `keys(["ctrl-s"])`: the last note's body is `"From the editor."` (D5 dropped the newline), and `status == None`.
7. `the_note_compose_renders_in_the_notes_pane`: `keys(["a"])`, `paste("A hand-written note.\nIts second line.")`, then `insta::assert_snapshot!("note_compose", frame)`.
8. `the_document_form_renders_in_the_docs_pane`: `keys(["v"])`, then `insta::assert_snapshot!("document_form", frame)`. The snapshot shows:
   - the strip;
   - ` New version of plan (from v2) `;
   - `  kind   plan`;
   - `  title  Plan: TUI scaffold (revised)`;
   - `> body`;
   - v2's three body lines (`Demo body for the plan document, version 2.` is exactly 43 cells);
   - `HINT_AREA` on the last row.

**Snapshots:** two new files, `backlog__note_compose.snap` and `backlog__document_form.snap`. No other `.snap` moves in this commit.

**Commit:** `test(mod-13): notes and documents integration cases`.

### 5c. `crates/htui/tests/hand_written_pg.rs` (new)
`#![cfg(feature = "testkit")]`. The module doc follows `item_writes_pg.rs:1-14`'s shape. The `Stack` (`new`, `finish`) is copied from `item_writes_pg.rs:23-95`, without `platform`, `item_count` or `head`. Helpers:
- `fn memory() -> Backend { Backend::memory(MemStore::demo()) }`;
- `fn shape(reply: &StoreReply) -> String { format!("{reply:?}") }`. `Debug` holds no body, so this is the comparable outcome.

Cases, each `#[tokio::test(flavor = "multi_thread")]` and each `let Some(stack) = Stack::new("hand-written-pg-…").await else { return };`:
1. `a_note_and_a_document_land_on_postgres_as_on_memory`:
   - `AddNote { FEAT_1, "Written by hand.\n\n" }` gives `shape` equal on Pg and memory (`NoteAdded`).
   - The Pg note has body `"Written by hand."`, `via_step_id == None`, `created_by == stack.db.store.this_user()`, and `box_id == stack.backend.box_info().await?.map(|b| b.box_id)`.
   - `WriteDocument plan` gives `DocumentWritten { version: 3 }` on both. `review` gives v1. `summary` on `ANA_2` gives v1.
   - The Pg `document(id)` of `plan` v3 has `produced_by_step_id == None`.
2. `the_document_form_on_postgres_matches_memory`: `DocumentForm { FEAT_1, Some("plan") }` gives `base.map(|b| (b.id, b.version, b.title))` equal on both, `(DOC_FEAT_1_PLAN_V2, 2, …)`. `kind: None` gives `base: None`.
3. `refusals_and_an_unknown_item_write_nothing_on_postgres`:
   - A blank `AddNote` and a kind `"pl\nan"` give `Failed` with `shape` equal on both, naming `note` and `kind`.
   - All four on `ItemId::new()` give `Failed` with `item \`…\` not found`.
   - Pg's `notes(FEAT_1).len()` and `documents(FEAT_1).len()` are unchanged.

Validate:
- `cargo test -p htui --features testkit --test backlog -- --test-threads=1`
- `cargo test -p htui --all-features --test hand_written_pg -- --test-threads=1`

**Commit:** `test(mod-13): hand-written writes on Postgres`.

### 5d. Commit order and hazards
All eleven commits, in order:
1. T1 validator.
2. T1 conformance.
3. T2 worker.
4. T3 helper move.
5. T3 compose.
6. T3 routing.
7. T3 Notes.
8. T4 Docs.
9. T5 hints and the four snapshots.
10. T5 integration.
11. T5 Postgres.

Each commit passes `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Through commit 8, `cargo test -p htui --features testkit --test backlog -- --test-threads=1` stays green with no snapshot change (E5). Stage explicit paths: no `-A`, no stash, no amend. Every message ends with the `Co-Authored-By` line. Close with the plan's Validation block, run with `--no-fail-fast -- --test-threads=1`, and grep the output for `SIGABRT`.

Hazards:
- **Lints**:
  - `missing_docs`: every new `pub` item, including enum variants and struct fields, the four requests and four replies.
  - `missing_debug_implementations`: `HandText`, `DocumentFormContext`, `Compose` and `KindField` hand-write `Debug`; `Part`, `ComposeOutcome` and `Expected` derive it.
  - `unused_qualifications`: do not write `crate::editor::EDITED` where it is imported.
  - rustdoc's `private_intra_doc_links` is `deny` under `cargo doc`. No `pub` doc may link a private item (`Compose::save`, `Expected`).
- **Redaction**: no request or reply carries a body as `String`. `HandText`, `DocumentDigest`, `Compose` and `TextField` print lengths. §2d test 10 and §3c test 14 pin it.
- **Suite hygiene**:
  - `tests/*.rs` need `--features testkit` or `--all-features`, or they run 0 tests and report ok.
  - Run `--test-threads=1`: the keyring fake is process-wide.
  - `tests/hand_written_pg.rs` skips without `HTUI_TEST_DATABASE_URL` and panics under `CI`.
- **The `.sqlx` cache**: no new SQL. The workers call existing store methods only, so no `cargo sqlx prepare` is needed.
- **HANDOFF close-out (not a task here)**: the counts become 4 new `StoreRequest` and 4 new `StoreReply` variants, and conformance 131.

## 6. Questions for the maintainer (none blocking; the blueprint proceeds as written)
1. **An unchanged `v` save writes an identical new version.** D5 and D9 set no "nothing to save" rule, unlike the item form's `NOTHING_TO_SAVE`. Ctrl+S right after `v` lands `plan v3` with v2's text.
   - Recommendation: keep it (append-only, an explicit Ctrl+S).
   - Alternatively the form could refuse a save whose kind, title and body equal `base`'s, with one sentence and one test.
2. **`v` opens on the body.** `a` opens on the kind; the item form opens on Title. This blueprint focuses Body for `v`, the field a new version usually changes. A one-line change if you prefer Title.
3. **The compose area takes the whole sub-tab pane** (§3c). The thread or table is hidden while composing, as the item form hides the detail pane. The strip stays visible. Informational.
4. **A form read that lands while another sub-tab is shown** (`a`, then `l` before the reply) opens its area in the hidden pane. It captures once that pane is shown again. Informational: nothing is lost or misrouted, and the reveal guard and scope change still see it only while it is active (`captures_input` asks the active sub-tab).
5. **Notice colour.** The compose notice uses `theme.error` for every sentence, `EDITED` included, as the item form does (milestone 4 blueprint Q4). Informational.
6. **Additions beyond the plan's test list.** None of them changes a decision:
   - E3's NUL tests;
   - E4's guard and its test;
   - §3e tests 1–2 (routing in both directions);
   - §4f tests 15–16;
   - Pg case 2.
