//! The owner documents of Settings › Hierarchy (MOD-147 M3 D7): a project's documents, or a
//! workspace's (its scope row's), listed and written by hand. The [`crate::hand_written`] shape:
//! one module, one [`serve`], one or-arm in [`crate::store_worker`].
//!
//! **An owner is a project or a workspace** ([`DocumentOwner`]). A workspace's documents are its
//! scope row's (MOD-147 D1, D2): the worker resolves it through `scope_project`, and a workspace
//! without one answers `NotFound { entity: SCOPE_ROW }`, which reads "scope row of workspace `…`
//! not found": the workspace is there, its scope row is not. The reply names the owner as asked,
//! so the pane that asked recognises it.
//!
//! **The read works offline; the write does not.** That differs on purpose from
//! [`crate::hand_written`], which refuses its form reads offline too: the list is a read the
//! mirror answers ([`Backend::owner_documents`]), so an offline box still shows its documents, and
//! says whether a write would be taken (`writable`), so the pane refuses one before `$EDITOR`
//! opens. The write takes [`Backend::writer`] before it reads anything and answers
//! `Unreachable(DATABASE_UNREACHABLE)` without one ([`offline_refusal`]).
//!
//! **The write is append-only** (`R-ENT-12`): a new document of a kind is that kind's next
//! version, which the store allocates under the owner's row lock. The worker fills the id (a
//! UUIDv7), the author ([`Backend::this_user`]) and the clock; the render side never holds a
//! `UserId` (`R-NF-3`). The kind, title and body are checked by
//! [`htui_core::model::hand_written`], the rules the item documents use, so a refusal reads the
//! same sentence; a refusal is a [`StoreError::Constraint`].
//!
//! Redaction: a body travels as [`HandText`](crate::hand_written::HandText), whose `Debug` prints
//! its length only.

use chrono::Utc;
use htui_core::model::hand_written::{self as rules, HandWrittenError};
use htui_core::model::{DocumentId, NewOwnerDocument, ProjectId, WorkspaceId};
use htui_core::store::{ReadStore, Result, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE};

use crate::store_worker::{StoreReply, StoreRequest};

/// The two requests, in [`StoreRequest::name`] order. One list, so a name changed in one place
/// and not the other fails `request_names_match_the_name_arms`.
pub const REQUEST_NAMES: [&str; 2] = ["owner_documents", "write_owner_document"];

/// The list read; its `Failed` is the pane's "unavailable" state.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// The write; its `Failed` frees the pane for another.
pub const WRITE_NAME: &str = REQUEST_NAMES[1];

/// The `NotFound` entity of a workspace with no scope row (review Low 6): the sentence names the
/// row that is missing, not the workspace, which exists.
pub const SCOPE_ROW: &str = "scope row of workspace";

/// The write's answer offline, rendered as the worker's `Failed` carries it.
#[must_use]
pub fn offline_refusal() -> String {
    offline().to_string()
}

/// MOD-147 M3 review M2: whether `message`, a write's `Failed`, was given before the insert, so
/// nothing was written: the [`crate::hand_written::write_refused`] answers (offline, a
/// `Constraint`) and a workspace with no scope row ([`SCOPE_ROW`]). Anything else may follow a
/// COMMIT whose answer was lost, and the pane re-reads the list to check.
#[must_use]
pub fn write_refused(message: &str) -> bool {
    let missing = StoreError::NotFound {
        entity: SCOPE_ROW,
        id: String::new(),
    }
    .to_string();
    let scope_row_missing = missing
        .split_once("``")
        .is_some_and(|(head, tail)| message.starts_with(head) && message.ends_with(tail));
    crate::hand_written::write_refused(message) || scope_row_missing
}

/// Whose documents: a project's, or a workspace's through its scope row (D7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentOwner {
    /// A project's own documents.
    Project(ProjectId),
    /// A workspace's: its scope row's documents.
    Workspace(WorkspaceId),
}

/// Serves one owner-document request, off the UI task (D7).
///
/// `OwnerDocuments` answers [`StoreReply::OwnerDocuments`], `WriteOwnerDocument`
/// [`StoreReply::OwnerDocumentWritten`].
///
/// # Errors
/// `NotFound { entity: SCOPE_ROW }` for a workspace with no scope row. A write offline:
/// `Unreachable(DATABASE_UNREACHABLE)` before anything is read. `Constraint` for a kind, title or
/// body the rules refuse. `Backend` for a request that is not one of the two.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        StoreRequest::OwnerDocuments(owner) => {
            let project = owner_project(backend, *owner).await?;
            let documents = backend.owner_documents(project).await?;
            Ok(StoreReply::OwnerDocuments {
                owner: *owner,
                documents,
                writable: backend.writer().is_some(),
            })
        }
        StoreRequest::WriteOwnerDocument {
            owner,
            kind,
            title,
            body,
        } => {
            let writer = backend.writer().ok_or_else(offline)?;
            let project = owner_project(&writer, *owner).await?;
            let kind = rules::document_kind(kind).map_err(refused)?;
            let title = rules::document_title(title).map_err(refused)?;
            let body = rules::document_body(body.as_str()).map_err(refused)?;
            let created_by = backend.this_user().await?;
            let row = writer
                .write_owner_document(NewOwnerDocument {
                    id: DocumentId::new(),
                    project_id: project,
                    kind,
                    title,
                    body,
                    created_by,
                    created_at: Utc::now(),
                })
                .await?;
            Ok(StoreReply::OwnerDocumentWritten {
                owner: *owner,
                kind: row.kind,
                version: row.version,
            })
        }
        other => Err(StoreError::Backend(format!(
            "not an owner-document request: {}",
            other.name()
        ))),
    }
}

/// The project whose documents `owner` names: itself, or the workspace's scope row.
async fn owner_project(store: &impl ReadStore, owner: DocumentOwner) -> Result<ProjectId> {
    match owner {
        DocumentOwner::Project(project) => Ok(project),
        DocumentOwner::Workspace(workspace) => {
            store
                .scope_project(workspace)
                .await?
                .ok_or_else(|| StoreError::NotFound {
                    entity: SCOPE_ROW,
                    id: workspace.to_string(),
                })
        }
    }
}

/// The refusal of a write offline, before anything is read.
fn offline() -> StoreError {
    StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())
}

/// A rules refusal as the worker answers it: its sentence, a `Constraint`.
fn refused(err: HandWrittenError) -> StoreError {
    StoreError::Constraint(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentOwner, READ_NAME, REQUEST_NAMES, SCOPE_ROW, WRITE_NAME, offline_refusal, serve,
        write_refused,
    };
    use crate::hand_written::HandText;
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use htui_core::fixtures::ids;
    use htui_core::model::document::GOALS_KIND;
    use htui_core::model::hand_written::HandWrittenError;
    use htui_core::model::{OwnerDocument, WorkspaceId};
    use htui_core::store::{MemStore, ReadStore as _, StoreError};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};
    use tempfile::TempDir;

    /// The demo store and a backend over it; clones share state.
    fn demo() -> (MemStore, Backend) {
        let store = MemStore::demo();
        (store.clone(), Backend::memory(store))
    }

    fn write(owner: DocumentOwner, kind: &str, title: &str, body: &str) -> StoreRequest {
        StoreRequest::WriteOwnerDocument {
            owner,
            kind: kind.to_owned(),
            title: title.to_owned(),
            body: HandText::new(body),
        }
    }

    async fn listed(backend: &Backend, owner: DocumentOwner) -> Vec<OwnerDocument> {
        listed_writable(backend, owner).await.0
    }

    /// The list and the reply's `writable`.
    async fn listed_writable(
        backend: &Backend,
        owner: DocumentOwner,
    ) -> (Vec<OwnerDocument>, bool) {
        match serve(backend, &StoreRequest::OwnerDocuments(owner)).await {
            Ok(StoreReply::OwnerDocuments {
                owner: answered,
                documents,
                writable,
            }) if answered == owner => (documents, writable),
            other => panic!("the read answered {other:?}"),
        }
    }

    async fn written(backend: &Backend, request: &StoreRequest) -> (String, i32) {
        match serve(backend, request).await {
            Ok(StoreReply::OwnerDocumentWritten { kind, version, .. }) => (kind, version),
            other => panic!("the write answered {other:?}"),
        }
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let owner = DocumentOwner::Project(ids::PROJECT_HTUI);
        let samples = [
            StoreRequest::OwnerDocuments(owner),
            write(owner, "goals", "Goals", "Ship."),
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
        assert_eq!([READ_NAME, WRITE_NAME], REQUEST_NAMES);
    }

    #[tokio::test]
    async fn a_project_owner_lists_its_documents_and_no_item_documents() {
        let (store, backend) = demo();
        assert!(
            !store
                .documents(ids::HTUI_FEAT_1)
                .await
                .expect("read")
                .is_empty(),
            "the demo item has documents of its own"
        );
        let owner = DocumentOwner::Project(ids::PROJECT_HTUI);
        assert_eq!(listed_writable(&backend, owner).await, (Vec::new(), true));

        written(&backend, &write(owner, "plan", "Plan", "Body.")).await;
        let documents = listed(&backend, owner).await;
        assert_eq!(documents.len(), 1, "{documents:?}");
        assert_eq!(documents[0].project_id, ids::PROJECT_HTUI);
        assert_eq!(documents[0].kind, "plan");
    }

    #[tokio::test]
    async fn a_workspace_owner_lists_its_scope_rows_documents() {
        let (store, backend) = demo();
        let owner = DocumentOwner::Workspace(ids::WORKSPACE_GRAPHICS);
        written(&backend, &write(owner, GOALS_KIND, "Goals", "Ship it.")).await;

        let documents = listed(&backend, owner).await;
        assert_eq!(documents.len(), 1, "{documents:?}");
        assert_eq!(documents[0].project_id, ids::PROJECT_SCOPE_GRAPHICS);
        assert_eq!(
            store
                .owner_documents(ids::PROJECT_SCOPE_GRAPHICS)
                .await
                .expect("read"),
            documents,
            "the scope row holds them"
        );
        // Another workspace's scope row holds none of them.
        assert_eq!(
            listed(&backend, DocumentOwner::Workspace(ids::WORKSPACE_PLATFORM)).await,
            []
        );
    }

    #[tokio::test]
    async fn a_workspace_without_a_scope_row_answers_not_found() {
        let (_, backend) = demo();
        let workspace = WorkspaceId::new();
        let owner = DocumentOwner::Workspace(workspace);
        for request in [
            StoreRequest::OwnerDocuments(owner),
            write(owner, "goals", "Goals", "Body."),
        ] {
            let reply = serve(&backend, &request).await;
            assert!(
                matches!(
                    &reply,
                    Err(StoreError::NotFound {
                        entity: SCOPE_ROW,
                        ..
                    })
                ),
                "{reply:?}"
            );
            let message = reply.expect_err("refused").to_string();
            assert_eq!(
                message,
                format!("scope row of workspace `{workspace}` not found")
            );
            assert!(write_refused(&message), "nothing was written: {message}");
        }
    }

    #[tokio::test]
    async fn a_write_mints_v1_then_v2_by_the_box_user() {
        let (_, backend) = demo();
        let owner = DocumentOwner::Workspace(ids::WORKSPACE_GRAPHICS);
        assert_eq!(
            written(&backend, &write(owner, " goals ", "Goals", "First.\n\n")).await,
            ("goals".to_owned(), 1)
        );
        assert_eq!(
            written(&backend, &write(owner, "goals", "Goals, again", "Second.")).await,
            ("goals".to_owned(), 2)
        );
        let documents = listed(&backend, owner).await;
        let versions: Vec<(i32, &str, &str)> = documents
            .iter()
            .map(|row| (row.version, row.title.as_str(), row.body.as_str()))
            .collect();
        assert_eq!(
            versions,
            [(1, "Goals", "First."), (2, "Goals, again", "Second.")]
        );
        for row in &documents {
            assert_eq!(row.created_by, ids::USER);
            assert_eq!(row.produced_by_step_id, None);
            assert_eq!(row.id.as_uuid().get_version_num(), 7, "a UUIDv7 id");
        }
    }

    #[tokio::test]
    async fn a_bad_kind_is_refused_with_the_rules_sentence() {
        let (store, backend) = demo();
        let owner = DocumentOwner::Project(ids::PROJECT_HTUI);
        for (kind, error) in [
            ("  ", HandWrittenError::BlankKind),
            ("go\u{7}als", HandWrittenError::ControlInKind),
        ] {
            let reply = serve(&backend, &write(owner, kind, "Title", "Body.")).await;
            assert!(
                matches!(&reply, Err(StoreError::Constraint(message)) if *message == error.to_string()),
                "{kind:?}: {reply:?}"
            );
        }
        assert!(
            store
                .owner_documents(ids::PROJECT_HTUI)
                .await
                .expect("read")
                .is_empty(),
            "nothing was written"
        );
    }

    /// An offline backend over a throwaway mirror holding one owner document of `htui`.
    async fn offline_backend() -> (TempDir, Backend) {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "owner-documents", 1)
            .await
            .expect("open a throwaway mirror");
        let file = cache.dir().join(htui_store::cache::CACHE_FILE);
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", file.display()))
            .await
            .expect("open the mirror's file");
        sqlx::query(
            "INSERT INTO document (id, project_id, item_id, kind, version, title, body, \
             produced_by_step_id, created_by, created_at) \
             VALUES (?, ?, NULL, 'goals', 1, 'Goals', 'Mirrored.', NULL, ?, 0)",
        )
        .bind(htui_core::model::DocumentId::new().to_string())
        .bind(ids::PROJECT_HTUI.to_string())
        .bind(ids::USER.to_string())
        .execute(&pool)
        .await
        .expect("seed the mirror");
        pool.close().await;
        (root, Backend::Offline { cache, since: None })
    }

    #[tokio::test]
    async fn offline_a_write_is_unreachable_and_a_read_answers_from_the_mirror() {
        let (_root, backend) = offline_backend().await;
        let owner = DocumentOwner::Project(ids::PROJECT_HTUI);

        let reply = serve(&backend, &write(owner, "goals", "Goals", "Body.")).await;
        assert!(
            matches!(&reply, Err(StoreError::Unreachable(message)) if message == DATABASE_UNREACHABLE),
            "{reply:?}"
        );
        let message = reply.expect_err("refused").to_string();
        assert_eq!(message, offline_refusal());
        assert!(write_refused(&message), "nothing was written: {message}");

        let (documents, writable) = listed_writable(&backend, owner).await;
        assert!(!writable, "offline, the list says the write is refused");
        let mirrored: Vec<(&str, i32, &str)> = documents
            .iter()
            .map(|row| (row.kind.as_str(), row.version, row.body.as_str()))
            .collect();
        assert_eq!(mirrored, [("goals", 1, "Mirrored.")]);
    }

    /// Review M2: the refusals given before anything is written, and nothing else.
    #[test]
    fn write_refused_names_the_answers_given_before_the_insert() {
        let refused = [
            StoreError::Constraint("a document needs a title".to_owned()),
            StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()),
            StoreError::NotFound {
                entity: SCOPE_ROW,
                id: WorkspaceId::new().to_string(),
            },
        ];
        for error in refused {
            assert!(write_refused(&error.to_string()), "{error:?}");
        }
        let hedged = [
            StoreError::Backend("connection reset by peer".to_owned()),
            StoreError::Unreachable("the pool is closed".to_owned()),
            StoreError::NotFound {
                entity: "workspace",
                id: WorkspaceId::new().to_string(),
            },
        ];
        for error in hedged {
            assert!(!write_refused(&error.to_string()), "{error:?}");
        }
    }

    /// Both requests reach this module through the worker's dispatch.
    #[tokio::test]
    async fn the_worker_dispatches_both_requests_here() {
        let (_, backend) = demo();
        let owner = DocumentOwner::Workspace(ids::WORKSPACE_GRAPHICS);
        let reply = store_worker::serve(&backend, &write(owner, "goals", "Goals", "Body.")).await;
        assert!(
            matches!(&reply, StoreReply::OwnerDocumentWritten { version: 1, .. }),
            "{reply:?}"
        );
        let reply = store_worker::serve(&backend, &StoreRequest::OwnerDocuments(owner)).await;
        assert!(
            matches!(&reply, StoreReply::OwnerDocuments { documents, .. } if documents.len() == 1),
            "{reply:?}"
        );
    }
}
