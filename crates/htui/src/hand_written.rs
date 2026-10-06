//! The Backlog's hand-written notes and documents (MOD-13 milestone 5): the read that opens the
//! Notes compose area, a note write, the read that opens the Docs form and a document write. The
//! [`crate::item_writes`] shape: one module, one [`serve`], one or-arm in [`crate::store_worker`].
//!
//! **The four requests are the write surface** (D1). A note lands through
//! [`WriteStore::add_note`](htui_core::store::WriteStore::add_note), a document through
//! [`WriteStore::write_document`](htui_core::store::WriteStore::write_document) at the store's next
//! version of its kind (`R-ENT-12`, D5: append-only, never a compare-and-set). The worker fills
//! the ids, the author and the box from [`Backend::this_user`] and [`Backend::box_info`] and the
//! clock; the render side never holds a `UserId` (`R-NF-3`). A hand-written row has no step:
//! `via_step_id` and `produced_by_step_id` are `None`.
//!
//! **Offline is refused here, and that is the gate** (D2). All four requests take
//! [`Backend::writer`] before they read anything, and answer `Unreachable(DATABASE_UNREACHABLE)`
//! without one, the form reads included, so an offline box never opens a compose area.
//!
//! **The item is checked first, then the text** (D3). Every request reads its item through the
//! writer and answers `NotFound { entity: "item" }` for an unknown one before the validator runs.
//! The text is then checked by [`htui_core::model::hand_written`], the functions the compose area
//! called for its early feedback, so the two give the same sentence. A refusal is a
//! [`StoreError::Constraint`] through [`HandWrittenError`]'s `Display`.
//!
//! **A write's `Failed` is hedged unless it is a refusal** (D10): a COMMIT whose answer was lost
//! comes back as `Failed`, and a retry would land a second note or version. [`write_refused`]
//! tells the pane which `Failed`s were given before the insert.
//!
//! Redaction (D1, milestone 2 review L2): a body travels as [`HandText`], whose `Debug` prints its
//! length only. The `v` form's reply carries a whole [`Document`], so [`DocumentFormContext`]'s
//! `Debug` is hand-written and prints the body's length, never the body. A title is plain.

use chrono::Utc;
use htui_core::model::hand_written::{self as rules, HandWrittenError};
use htui_core::model::{Document, DocumentId, Item, ItemId, NewDocument, NewNote, NoteId, RunId};
use htui_core::store::{ReadStore as _, Result, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::store_worker::{StoreReply, StoreRequest};

/// The four requests, in [`StoreRequest::name`] order. One list, so a name changed in one place
/// and not the other fails `request_names_match_the_name_arms`.
pub const REQUEST_NAMES: [&str; 4] = ["note_form", "add_note", "document_form", "write_document"];

/// The read that opens the Notes compose area; its `Failed` opens nothing.
pub const NOTE_FORM_NAME: &str = REQUEST_NAMES[0];

/// The note write; its `Failed` is hedged unless [`write_refused`] (D10).
pub const ADD_NOTE_NAME: &str = REQUEST_NAMES[1];

/// The read that opens the Docs form; its `Failed` opens nothing.
pub const DOCUMENT_FORM_NAME: &str = REQUEST_NAMES[2];

/// The document write; hedged as [`ADD_NOTE_NAME`] is.
pub const WRITE_DOCUMENT_NAME: &str = REQUEST_NAMES[3];

/// Whether `name` is one of this module's four requests. Nothing routes on it: it exists for
/// `request_names_match_the_name_arms`, so it is test-only.
#[cfg(test)]
#[must_use]
pub fn is_hand_written_request(name: &str) -> bool {
    REQUEST_NAMES.contains(&name)
}

/// A note or document body on its way to the store (D1). `StoreRequest` derives `Debug`, and this
/// is user prose, so it prints its length only ([`crate::requirements::RequirementText`]'s rule).
#[derive(Clone, PartialEq, Eq)]
pub struct HandText(String);

impl HandText {
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

impl core::fmt::Debug for HandText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HandText")
            .field("len", &self.0.len())
            .finish()
    }
}

/// The answer to `StoreRequest::DocumentForm` (D1, D3): the item, and for `v` the version of the
/// asked kind the next step reads (`resolve_inputs`, ANA-2 §4.2 as amended by MOD-73), not the
/// latest of any producer: a fan-out loser's output is never the base (`R-ORCH-7`), and a
/// hand-written version newer than the step-produced pick is. See `v_base` for the seat.
///
/// `Debug` is hand-written (milestone 2 review L2): see the impl.
#[derive(Clone, PartialEq)]
pub struct DocumentFormContext {
    /// The item the form writes for.
    pub item: ItemId,
    /// The version of the kind the next step reads, body included, for `v`; `None` for `a` or a
    /// kind with no eligible row.
    pub base: Option<Document>,
}

/// Ids, the kind, the version, the title and the body's length, never the body (milestone 2
/// review L2), since this struct rides in a [`StoreReply`].
impl core::fmt::Debug for DocumentFormContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DocumentFormContext")
            .field("item", &self.item)
            .field("base", &self.base.as_ref().map(DocumentDigest))
            .finish()
    }
}

/// A [`Document`] as [`DocumentFormContext`] prints it: ids, kind, version, title and the body's
/// length. The title is plain (D1).
struct DocumentDigest<'a>(&'a Document);

impl core::fmt::Debug for DocumentDigest<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let document = self.0;
        f.debug_struct("Document")
            .field("id", &document.id)
            .field("kind", &document.kind)
            .field("version", &document.version)
            .field("title", &document.title)
            .field("produced_by_step_id", &document.produced_by_step_id)
            .field("body_len", &document.body.len())
            .finish()
    }
}

/// D10: whether `message`, a write's `Failed` as the worker renders it, was given before the
/// insert, so nothing was written. That is the offline refusal ([`DATABASE_UNREACHABLE`]), every
/// [`StoreError::Constraint`] (the validator's), and `NotFound { entity: "item" }` (D3's check).
/// Anything else may follow a COMMIT whose answer was lost, and the pane hedges it
/// ([`crate::item_writes::mint_refused`] plus `NotFound`).
#[must_use]
pub fn write_refused(message: &str) -> bool {
    // Rendered from the errors themselves, so the prefixes cannot drift from `StoreError`'s
    // `#[error]` text.
    let constraint = StoreError::Constraint(String::new()).to_string();
    let missing = StoreError::NotFound {
        entity: "item",
        id: String::new(),
    }
    .to_string();
    let item_missing = missing
        .split_once("``")
        .is_some_and(|(head, tail)| message.starts_with(head) && message.ends_with(tail));
    message == offline().to_string() || message.starts_with(&constraint) || item_missing
}

/// Review M1, round 1: whether `message`, a hedged write's `Failed`, is an `Unreachable`. The
/// worker drops an online backend to the mirror (`go_offline`) before it answers one, so the
/// re-read that checks the hedge is the mirror's, which cannot hold a write whose answer was
/// lost: not finding it there is no proof it was not written. Asked after [`write_refused`],
/// which takes the offline refusal itself.
#[must_use]
pub fn answered_from_the_mirror(message: &str) -> bool {
    message.starts_with(&StoreError::Unreachable(String::new()).to_string())
}

/// Serves one hand-written request, off the UI task (D1-D3).
///
/// `NoteForm` answers [`StoreReply::NoteForm`], `AddNote` [`StoreReply::NoteAdded`],
/// `DocumentForm` [`StoreReply::DocumentForm`] and `WriteDocument` [`StoreReply::DocumentWritten`].
///
/// # Errors
/// Offline: `Unreachable(DATABASE_UNREACHABLE)` for all four, before anything is read (D2).
/// `NotFound { entity: "item" }` for an unknown item (D3), before the validator. `Constraint`
/// for a validator refusal. `Backend` for a request that is not one of the four.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
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
            writer
                .add_note(NewNote {
                    id: NoteId::new(),
                    item_id: *item,
                    body,
                    created_by,
                    box_id,
                    via_step_id: None,
                    created_at: Utc::now(),
                })
                .await?;
            Ok(StoreReply::NoteAdded { item: *item })
        }
        StoreRequest::DocumentForm { item, kind } => {
            let writer = write_access(backend)?;
            existing(&writer, *item).await?;
            let base = match kind {
                Some(kind) => v_base(&writer, *item, kind).await?,
                None => None,
            };
            Ok(StoreReply::DocumentForm(Box::new(DocumentFormContext {
                item: *item,
                base,
            })))
        }
        StoreRequest::WriteDocument {
            item,
            kind,
            title,
            body,
        } => {
            let writer = write_access(backend)?;
            existing(&writer, *item).await?;
            let kind = rules::document_kind(kind).map_err(refused)?;
            let title = rules::document_title(title).map_err(refused)?;
            let body = rules::document_body(body.as_str()).map_err(refused)?;
            let created_by = backend.this_user().await?;
            let row = writer
                .write_document(NewDocument {
                    id: DocumentId::new(),
                    item_id: *item,
                    kind,
                    title,
                    body,
                    produced_by_step_id: None,
                    created_by,
                    created_at: Utc::now(),
                })
                .await?;
            Ok(StoreReply::DocumentWritten {
                item: *item,
                kind: row.kind,
                version: row.version,
            })
        }
        other => Err(StoreError::Backend(format!(
            "not a hand-written request: {}",
            other.name()
        ))),
    }
}

/// `v`'s base (MOD-73 review M1): the version of `kind` the item's next step reads, so an edit
/// saved from it and approved at a gate carries the selected output forward, never a fan-out
/// loser's (`R-ORCH-7`). `MOD-13`'s "latest version" (`documents_of_kinds`) was that until a newer
/// hand-written version became a step input.
///
/// The seat is the item's most recent run (latest `queued_at`): at a gate, the run whose next step
/// reads the edit, so its own output ranks first exactly as the engine ranks it. With no run, a
/// fresh [`RunId`] matches no step, so every run's non-loser output ranks equally by version, as
/// a first run's would. One extra read (`runs`) on a key press; versions only grow, so the two
/// seats differ only when another run's output is newer than the most recent run's.
async fn v_base(writer: &Writer, item: ItemId, kind: &str) -> Result<Option<Document>> {
    let run = writer
        .runs(item)
        .await?
        .into_iter()
        .max_by_key(|run| run.queued_at)
        .map_or_else(RunId::new, |run| run.id);
    Ok(writer
        .resolve_inputs(item, run, &[kind.to_owned()])
        .await?
        .into_iter()
        .next()
        .and_then(|input| input.document))
}

/// The writer, or the refusal every hand-written request answers offline (D2).
fn write_access(backend: &Backend) -> Result<Writer> {
    backend.writer().ok_or_else(offline)
}

/// The refusal of every hand-written request offline, before anything is read.
fn offline() -> StoreError {
    StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())
}

/// A validator refusal as the worker answers it: its sentence, a `Constraint`.
fn refused(err: HandWrittenError) -> StoreError {
    StoreError::Constraint(err.to_string())
}

/// The item, or `NotFound { entity: "item" }` (D3).
async fn existing(writer: &Writer, id: ItemId) -> Result<Item> {
    writer.item(id).await?.ok_or_else(|| StoreError::NotFound {
        entity: "item",
        id: id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ADD_NOTE_NAME, DOCUMENT_FORM_NAME, DocumentFormContext, HandText, NOTE_FORM_NAME,
        REQUEST_NAMES, WRITE_DOCUMENT_NAME, answered_from_the_mirror, is_hand_written_request,
        serve, write_refused,
    };
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use htui_core::fixtures::ids;
    use htui_core::model::ItemId;
    use htui_core::store::{MemStore, ReadStore as _, StoreError};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};
    use tempfile::TempDir;

    /// The demo store and a backend over it; clones share state.
    fn demo() -> (MemStore, Backend) {
        let store = MemStore::demo();
        (store.clone(), Backend::memory(store))
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    fn note(item: ItemId, body: &str) -> StoreRequest {
        StoreRequest::AddNote {
            item,
            body: HandText::new(body),
        }
    }

    fn write(item: ItemId, kind: &str, title: &str, body: &str) -> StoreRequest {
        StoreRequest::WriteDocument {
            item,
            kind: kind.to_owned(),
            title: title.to_owned(),
            body: HandText::new(body),
        }
    }

    /// An offline backend over an empty mirror: a read would answer `NotFound`, not the refusal.
    async fn offline_backend(name: &str) -> (TempDir, Backend) {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), name, 1)
            .await
            .expect("open a throwaway mirror");
        (root, Backend::Offline { cache, since: None })
    }

    /// One sample of each request, `FEAT-1`'s.
    fn samples(item: ItemId) -> [StoreRequest; 4] {
        [
            StoreRequest::NoteForm { item },
            note(item, "Hi."),
            StoreRequest::DocumentForm {
                item,
                kind: Some("plan".to_owned()),
            },
            write(item, "plan", "Plan", "Body."),
        ]
    }

    async fn counts(store: &MemStore) -> (usize, usize) {
        let notes = store.notes(ids::HTUI_FEAT_1).await.expect("notes").len();
        let documents = store
            .documents(ids::HTUI_FEAT_1)
            .await
            .expect("documents")
            .len();
        (notes, documents)
    }

    async fn latest(store: &MemStore, item: ItemId, kind: &str) -> htui_core::model::Document {
        store
            .documents_of_kinds(item, &strings(&[kind]))
            .await
            .expect("read")
            .into_iter()
            .next()
            .expect("the kind has a row")
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let samples = samples(ids::HTUI_FEAT_1);
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
        assert_eq!(
            [
                NOTE_FORM_NAME,
                ADD_NOTE_NAME,
                DOCUMENT_FORM_NAME,
                WRITE_DOCUMENT_NAME
            ],
            REQUEST_NAMES
        );
        for name in REQUEST_NAMES {
            assert!(is_hand_written_request(name), "{name}");
        }
        assert!(!is_hand_written_request("notes") && !is_hand_written_request("item_form"));
    }

    #[tokio::test]
    async fn the_note_form_read_answers_its_item() {
        let (_, backend) = demo();
        let reply = serve(
            &backend,
            &StoreRequest::NoteForm {
                item: ids::HTUI_FEAT_1,
            },
        )
        .await;
        assert!(
            matches!(reply, Ok(StoreReply::NoteForm { item }) if item == ids::HTUI_FEAT_1),
            "{reply:?}"
        );
    }

    #[tokio::test]
    async fn add_note_lands_a_hand_written_note_by_this_user_on_this_box() {
        let (store, backend) = demo();
        let reply = serve(&backend, &note(ids::HTUI_FEAT_1, "Hi.\n\n")).await;
        assert!(
            matches!(reply, Ok(StoreReply::NoteAdded { item }) if item == ids::HTUI_FEAT_1),
            "{reply:?}"
        );
        let notes = store.notes(ids::HTUI_FEAT_1).await.expect("notes");
        assert_eq!(notes.len(), 3);
        let new = notes
            .iter()
            .find(|row| row.id != ids::NOTE_1 && row.id != ids::NOTE_2)
            .expect("the new note");
        assert_eq!(new.body, "Hi.");
        assert_eq!(new.item_id, ids::HTUI_FEAT_1);
        assert_eq!(new.created_by, ids::USER);
        assert_eq!(new.box_id, Some(ids::BOX));
        assert_eq!(new.via_step_id, None);
    }

    async fn form(backend: &Backend, kind: Option<&str>) -> DocumentFormContext {
        form_of(backend, ids::HTUI_FEAT_1, kind).await
    }

    async fn form_of(backend: &Backend, item: ItemId, kind: Option<&str>) -> DocumentFormContext {
        let request = StoreRequest::DocumentForm {
            item,
            kind: kind.map(str::to_owned),
        };
        match serve(backend, &request).await {
            Ok(StoreReply::DocumentForm(context)) => *context,
            other => panic!("the form read answered {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_document_form_read_carries_the_latest_version_of_its_kind() {
        let (_, backend) = demo();

        let new = form(&backend, None).await;
        assert_eq!(new.item, ids::HTUI_FEAT_1);
        assert_eq!(new.base, None);

        let plan = form(&backend, Some("plan")).await;
        assert_eq!(plan.item, ids::HTUI_FEAT_1);
        let base = plan.base.expect("plan has versions");
        assert_eq!(base.id, ids::DOC_FEAT_1_PLAN_V2);
        assert_eq!(base.version, 2);
        assert!(!base.body.is_empty(), "the base carries its body");

        assert_eq!(form(&backend, Some("review")).await.base, None);
    }

    /// MOD-73 review M1: `v`'s base is the version the next step reads (`resolve_inputs`), not
    /// the latest of any producer. The seed's `ANA-1` research fan-out has a loser at v3 over the
    /// selected v2; prefilling v3 and saving it as a hand-written edit would carry the loser
    /// forward (`R-ORCH-7`). A newer hand-written version is then the base, as it is the input.
    #[tokio::test]
    async fn the_document_form_base_is_what_the_next_step_reads_never_a_loser() {
        let (store, backend) = demo();
        assert_eq!(
            latest(&store, ids::HTUI_ANA_1, "research").await.id,
            ids::DOC_ANA_1_RESEARCH_V3,
            "the seed's loser is the kind's highest version"
        );

        let base = form_of(&backend, ids::HTUI_ANA_1, Some("research"))
            .await
            .base
            .expect("research has an eligible version");
        assert_eq!(
            base.id,
            ids::DOC_ANA_1_RESEARCH_V2,
            "the selected output, not the loser"
        );

        let reply = serve(
            &backend,
            &write(ids::HTUI_ANA_1, "research", "Research, by hand", "Edited."),
        )
        .await;
        assert!(
            matches!(reply, Ok(StoreReply::DocumentWritten { version: 4, .. })),
            "{reply:?}"
        );
        let base = form_of(&backend, ids::HTUI_ANA_1, Some("research"))
            .await
            .base
            .expect("research has an eligible version");
        assert_eq!(base.version, 4);
        assert_eq!(base.produced_by_step_id, None);
        assert_eq!(base.body, "Edited.");
    }

    #[tokio::test]
    async fn write_document_lands_the_next_version_by_hand() {
        let (store, backend) = demo();

        let reply = serve(
            &backend,
            &write(ids::HTUI_FEAT_1, "plan", "Plan, by hand", "Steps.\n"),
        )
        .await;
        match reply {
            Ok(StoreReply::DocumentWritten {
                item,
                kind,
                version,
            }) => {
                assert_eq!(item, ids::HTUI_FEAT_1);
                assert_eq!(kind, "plan");
                assert_eq!(version, 3);
            }
            other => panic!("the write answered {other:?}"),
        }
        let row = latest(&store, ids::HTUI_FEAT_1, "plan").await;
        assert_eq!(row.version, 3);
        assert_eq!(row.title, "Plan, by hand");
        assert_eq!(row.body, "Steps.");
        assert_eq!(row.produced_by_step_id, None);
        assert_eq!(row.created_by, ids::USER);

        let reply = serve(
            &backend,
            &write(ids::HTUI_FEAT_1, " review ", " Review ", "Fine."),
        )
        .await;
        assert!(
            matches!(
                &reply,
                Ok(StoreReply::DocumentWritten { kind, version: 1, .. }) if kind == "review"
            ),
            "{reply:?}"
        );
        assert_eq!(
            latest(&store, ids::HTUI_FEAT_1, "review").await.title,
            "Review"
        );

        let reply = serve(&backend, &write(ids::HTUI_ANA_2, "summary", "Sum", "Up.")).await;
        assert!(
            matches!(
                &reply,
                Ok(StoreReply::DocumentWritten { item, kind, version: 1 })
                    if *item == ids::HTUI_ANA_2 && kind == "summary"
            ),
            "{reply:?}"
        );
    }

    #[tokio::test]
    async fn an_unknown_item_is_not_found_for_all_four() {
        let (store, backend) = demo();
        let before = counts(&store).await;
        let unknown = ItemId::new();
        for request in &samples(unknown) {
            assert_eq!(
                serve(&backend, request).await.err(),
                Some(StoreError::NotFound {
                    entity: "item",
                    id: unknown.to_string(),
                }),
                "{}",
                request.name()
            );
        }
        assert_eq!(counts(&store).await, before);
        // Review L3: FEAT-1's counts cannot move under a write to another item; the unknown
        // item's own rows are what a write that slipped through would have added.
        assert!(store.notes(unknown).await.expect("notes").is_empty());
        assert!(
            store
                .documents(unknown)
                .await
                .expect("documents")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn every_refusal_fails_naming_the_field_and_writes_nothing() {
        let (store, backend) = demo();
        let before = counts(&store).await;
        let feat = ids::HTUI_FEAT_1;
        let cases = [
            (note(feat, "  \n"), "note"),
            (write(feat, "", "Plan", "Body."), "kind"),
            (write(feat, "pl\nan", "Plan", "Body."), "kind"),
            (write(feat, "plan", " ", "Body."), "title"),
            (write(feat, "plan", "a\nb", "Body."), "title"),
            (write(feat, "plan", "Plan", "   "), "body"),
            (write(feat, "plan", "Plan", "x\0"), "NUL"),
        ];
        for (request, named) in &cases {
            match store_worker::serve(&backend, request).await {
                StoreReply::Failed {
                    request: name,
                    message,
                } => {
                    assert_eq!(name, request.name());
                    assert!(message.contains(named), "{named}: {message}");
                    assert!(message.starts_with("constraint violated: "), "{message}");
                }
                other => panic!("{named}: refused, not {other:?}"),
            }
            assert_eq!(counts(&store).await, before, "{named}");
        }
    }

    /// D2: the writer is taken before anything is read, so an offline box never opens a form.
    #[tokio::test]
    async fn offline_every_hand_written_request_is_refused_before_anything_is_read() {
        let (_root, backend) = offline_backend("hand-written-offline").await;
        for request in &samples(ids::HTUI_FEAT_1) {
            assert_eq!(
                serve(&backend, request).await.err(),
                Some(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())),
                "{}",
                request.name()
            );
            match store_worker::serve(&backend, request).await {
                StoreReply::Failed {
                    request: name,
                    message,
                } => {
                    assert_eq!(name, request.name());
                    assert!(message.contains(DATABASE_UNREACHABLE), "{message}");
                }
                other => panic!("an offline {} is refused, not {other:?}", request.name()),
            }
        }
    }

    /// D10: a refusal given before the insert, as the worker renders it into `Failed`, is told
    /// from a store failure that may follow a lost COMMIT.
    #[tokio::test]
    async fn write_refused_tells_a_refusal_from_a_store_failure() {
        let (_, backend) = demo();
        let (_root, offline) = offline_backend("hand-written-refused").await;
        let refusals = [
            ("offline", &offline, note(ids::HTUI_FEAT_1, "Hi.")),
            ("a blank note", &backend, note(ids::HTUI_FEAT_1, "  ")),
            ("an unknown item", &backend, note(ItemId::new(), "Hi.")),
        ];
        for (why, backend, request) in refusals {
            let StoreReply::Failed { message, .. } = store_worker::serve(backend, &request).await
            else {
                panic!("{why}: the request is refused")
            };
            assert!(write_refused(&message), "{why}: {message}");
        }

        for failure in [
            StoreError::Backend("connection reset by peer".to_owned()),
            StoreError::Unreachable("connection reset".to_owned()),
            StoreError::NotFound {
                entity: "app_user",
                id: "x".to_owned(),
            },
        ] {
            assert!(!write_refused(&failure.to_string()), "{failure}");
        }
    }

    /// Review M1, round 1: a dropped connection on the write is rendered `Unreachable`, after
    /// which the worker serves the re-read from the mirror; a server's own error is not.
    #[test]
    fn answered_from_the_mirror_is_an_unreachable_failure() {
        let reset = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
        let dropped = htui_store::map_sqlx(sqlx::Error::Io(reset)).to_string();
        assert!(!write_refused(&dropped), "{dropped}");
        assert!(answered_from_the_mirror(&dropped), "{dropped}");
        for failure in [
            StoreError::Backend("connection reset by peer".to_owned()),
            StoreError::NotFound {
                entity: "app_user",
                id: "x".to_owned(),
            },
        ] {
            assert!(!answered_from_the_mirror(&failure.to_string()), "{failure}");
        }
    }

    /// D1: `StoreRequest` derives `Debug`; a body prints as its length, the title as itself.
    #[tokio::test]
    async fn requests_and_the_form_reply_debug_print_no_body() {
        let (store, _) = demo();
        let mut base = latest(&store, ids::HTUI_FEAT_1, "plan").await;
        base.body = "SECRET-BODY".to_owned();
        let add = note(ids::HTUI_FEAT_1, "SECRET-BODY");
        let write = write(ids::HTUI_FEAT_1, "plan", "Visible title", "SECRET-BODY");
        let reply = StoreReply::DocumentForm(Box::new(DocumentFormContext {
            item: ids::HTUI_FEAT_1,
            base: Some(base),
        }));
        for printed in [format!("{add:?}"), format!("{add:#?}")] {
            assert!(!printed.contains("SECRET-BODY"), "{printed}");
        }
        for printed in [format!("{write:?}"), format!("{write:#?}")] {
            assert!(!printed.contains("SECRET-BODY"), "{printed}");
            assert!(printed.contains("Visible title"), "{printed}");
        }
        for printed in [format!("{reply:?}"), format!("{reply:#?}")] {
            assert!(!printed.contains("SECRET-BODY"), "{printed}");
            assert!(printed.contains("body_len"), "{printed}");
        }
    }

    #[tokio::test]
    async fn a_request_that_is_not_hand_written_is_named() {
        let (_, backend) = demo();
        assert_eq!(
            serve(&backend, &StoreRequest::BoxInfo).await.err(),
            Some(StoreError::Backend(
                "not a hand-written request: box_info".to_owned()
            ))
        );
    }
}
