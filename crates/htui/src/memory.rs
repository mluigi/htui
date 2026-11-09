//! MOD-163 M3 D21: the Hierarchy memory pane's store requests, the [`crate::requirements`] shape.
//!
//! One read per event and never one per keystroke. Every write re-reads the project's entries
//! rather than handing the pane the row its outcome carries: a write that landed answers
//! [`StoreReply::MemoryWritten`], carrying the re-read (or, when only the re-read failed, its
//! failure: the write has landed). An amend, retire or supersede that missed its version answers
//! [`StoreReply::MemoryStale`] with the row as it is now. Offline every write is refused before
//! anything is sent, and the read answers `entries: None` (the table is not mirrored, M1 D7).
//!
//! **The subject clash is typed, not parsed** (D21). A create or an amend the store refuses with a
//! [`StoreError::Constraint`] is followed by a re-read of the project's live entries; the one whose
//! folded subject ([`memory_subject_key`]) matches, other than the entry being amended (blueprint
//! P-7a), is answered as [`StoreReply::MemorySubjectTaken`]. A refusal with no such holder stays the
//! store's error, which [`crate::store_worker::serve`] answers as `Failed`.
//!
//! The worker fills the author and the box from [`Backend::this_user`] and [`Backend::box_info`]:
//! the render side never holds a `UserId` (`R-NF-3`). Nothing here reads the clock.

use htui_core::model::{
    MemoryEntry, MemoryEntryId, MemoryPatch, MemoryState, MemorySupersede, MemoryUpdate,
    NewMemoryEntry, ProjectId, memory_subject_key,
};
use htui_core::store::{ReadStore as _, Result, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::store_worker::{StoreReply, StoreRequest};

/// The five request names, in [`StoreRequest`] order.
///
/// The pane's `Failed` matches read from here. [`StoreRequest::name`]'s arms spell the same five
/// as literals, and the `request_names_match_the_name_arms` test pins them to this list.
pub const REQUEST_NAMES: [&str; 5] = [
    "memory_entries",
    "create_memory_entry",
    "amend_memory_entry",
    "retire_memory_entry",
    "supersede_memory_entry",
];

/// The pane's read: a refused read leaves the pane with no list.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// Whether `name` is one of the four memory writes.
#[must_use]
pub fn is_write(name: &str) -> bool {
    REQUEST_NAMES[1..].contains(&name)
}

/// What a landed memory write did, carried by [`StoreReply::MemoryWritten`] beside the re-read
/// (the self-naming outcome, [`crate::requirements::RequirementWrite`]'s shape). Ids and versions
/// only: the text stays in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryWrite {
    /// [`StoreRequest::CreateMemoryEntry`]: the entry landed at version 1.
    Created {
        /// The id the worker minted.
        id: MemoryEntryId,
    },
    /// [`StoreRequest::AmendMemoryEntry`] at the head.
    Amended {
        /// The entry.
        id: MemoryEntryId,
        /// The version written.
        version: i32,
    },
    /// [`StoreRequest::RetireMemoryEntry`] at the head.
    Retired {
        /// The entry.
        id: MemoryEntryId,
    },
    /// [`StoreRequest::SupersedeMemoryEntry`]: `old` is superseded by `new`.
    Superseded {
        /// The entry replaced.
        old: MemoryEntryId,
        /// The entry that replaced it.
        new: MemoryEntryId,
    },
}

impl MemoryWrite {
    /// The [`StoreRequest::name`] of the write this answers.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Created { .. } => REQUEST_NAMES[1],
            Self::Amended { .. } => REQUEST_NAMES[2],
            Self::Retired { .. } => REQUEST_NAMES[3],
            Self::Superseded { .. } => REQUEST_NAMES[4],
        }
    }

    /// The entry the pane puts its cursor on after the write: the new one of a supersede.
    #[must_use]
    pub const fn entry(self) -> MemoryEntryId {
        match self {
            Self::Created { id } | Self::Amended { id, .. } | Self::Retired { id } => id,
            Self::Superseded { new, .. } => new,
        }
    }
}

/// The states the pane lists (D20): the live ones (`active | suspect`) unless `all`, which reads
/// every state.
#[must_use]
pub fn states(all: bool) -> Option<Vec<MemoryState>> {
    (!all).then(|| vec![MemoryState::Active, MemoryState::Suspect])
}

/// Serves one memory request, off the UI task.
///
/// # Errors
/// [`StoreError::Unreachable`] with `DATABASE_UNREACHABLE` for a write offline, before anything
/// is sent; the store's own refusal otherwise (a subject clash with a live holder is answered as
/// [`StoreReply::MemorySubjectTaken`] instead); [`StoreError::Backend`] for a request that is not
/// one of this module's five.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        StoreRequest::MemoryEntries { project, all } => Ok(StoreReply::Memory {
            project: *project,
            entries: backend.memory_entries(*project, states(*all)).await?,
        }),
        StoreRequest::CreateMemoryEntry {
            project,
            all,
            kind,
            subject,
            body,
        } => {
            let writer = write_access(backend)?;
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            let new = NewMemoryEntry {
                id: MemoryEntryId::new(),
                kind: *kind,
                subject: subject.clone(),
                body: body.clone(),
                created_by: me,
                box_id,
            };
            match writer.create_memory_entry(*project, new).await {
                Ok(row) => {
                    Ok(written(backend, *project, *all, MemoryWrite::Created { id: row.id }).await)
                }
                Err(StoreError::Constraint(refusal)) => {
                    clash(backend, *project, subject.as_str(), None, refusal).await
                }
                Err(other) => Err(other),
            }
        }
        StoreRequest::AmendMemoryEntry {
            project,
            all,
            id,
            expected,
            kind,
            subject,
            body,
        } => {
            let writer = write_access(backend)?;
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            let patch = MemoryPatch {
                kind: Some(*kind),
                subject: Some(subject.clone()),
                body: Some(body.clone()),
                author_id: me,
                box_id,
            };
            match writer.amend_memory_entry(*id, *expected, patch).await {
                Ok(MemoryUpdate::Updated(row)) => {
                    let landed = MemoryWrite::Amended {
                        id: row.id,
                        version: row.version,
                    };
                    Ok(written(backend, *project, *all, landed).await)
                }
                Ok(MemoryUpdate::Diverged { head, .. }) => {
                    stale(backend, *project, *all, head).await
                }
                // P-7a: the amended entry holds its own folded subject, so it is never the holder.
                Err(StoreError::Constraint(refusal)) => {
                    clash(backend, *project, subject.as_str(), Some(*id), refusal).await
                }
                Err(other) => Err(other),
            }
        }
        StoreRequest::RetireMemoryEntry {
            project,
            all,
            id,
            expected,
        } => {
            let writer = write_access(backend)?;
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            match writer
                .retire_memory_entry(*id, *expected, me, box_id)
                .await?
            {
                MemoryUpdate::Updated(row) => {
                    Ok(written(backend, *project, *all, MemoryWrite::Retired { id: row.id }).await)
                }
                MemoryUpdate::Diverged { head, .. } => stale(backend, *project, *all, head).await,
            }
        }
        StoreRequest::SupersedeMemoryEntry {
            project,
            all,
            old,
            expected,
            kind,
            subject,
            body,
        } => {
            let writer = write_access(backend)?;
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            let new = NewMemoryEntry {
                id: MemoryEntryId::new(),
                kind: *kind,
                subject: subject.clone(),
                body: body.clone(),
                created_by: me,
                box_id,
            };
            // D21 types the clash of a create or an amend only: a supersede's refusal stays the
            // store's, answered as `Failed`.
            match writer.supersede_memory_entry(*old, *expected, new).await? {
                MemorySupersede::Superseded { new, .. } => {
                    let landed = MemoryWrite::Superseded {
                        old: *old,
                        new: new.id,
                    };
                    Ok(written(backend, *project, *all, landed).await)
                }
                MemorySupersede::Diverged { head, .. } => {
                    stale(backend, *project, *all, head).await
                }
            }
        }
        // `try_serve` routes exactly this module's five variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is told which request it
        // sent.
        other => Err(StoreError::Backend(format!(
            "not a memory request: {}",
            other.name()
        ))),
    }
}

/// The writer, or the refusal every memory write answers offline, before anything is sent.
fn write_access(backend: &Backend) -> Result<Writer> {
    backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))
}

/// [`StoreReply::MemoryWritten`]: the write landed, so a failed re-read is carried, not raised.
async fn written(
    backend: &Backend,
    project: ProjectId,
    all: bool,
    landed: MemoryWrite,
) -> StoreReply {
    let entries = match backend.memory_entries(project, states(all)).await {
        Ok(Some(entries)) => Ok(entries),
        Ok(None) => Err(DATABASE_UNREACHABLE.to_owned()),
        Err(error) => Err(error.to_string()),
    };
    StoreReply::MemoryWritten {
        project,
        written: landed,
        entries,
    }
}

/// [`StoreReply::MemoryStale`]: nothing was written; the head and the list as they are now.
async fn stale(
    backend: &Backend,
    project: ProjectId,
    all: bool,
    head: MemoryEntry,
) -> Result<StoreReply> {
    let entries = backend
        .memory_entries(project, states(all))
        .await?
        .unwrap_or_default();
    Ok(StoreReply::MemoryStale {
        project,
        head,
        entries,
    })
}

/// D21: a create or amend refused with a `Constraint`. The live entry whose folded subject is
/// `subject`'s, other than `except`, is the holder; with none, the refusal stays the store's.
async fn clash(
    backend: &Backend,
    project: ProjectId,
    subject: &str,
    except: Option<MemoryEntryId>,
    refusal: String,
) -> Result<StoreReply> {
    let key = memory_subject_key(subject);
    let live = backend
        .memory_entries(project, states(false))
        .await?
        .unwrap_or_default();
    match live
        .into_iter()
        .find(|entry| Some(entry.id) != except && memory_subject_key(&entry.subject) == key)
    {
        Some(existing) => Ok(StoreReply::MemorySubjectTaken { project, existing }),
        None => Err(StoreError::Constraint(refusal)),
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryWrite, READ_NAME, REQUEST_NAMES, is_write, serve, states};
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        MemoryBody, MemoryEntry, MemoryEntryId, MemoryKind, MemoryState, MemorySubject, ProjectId,
    };
    use htui_core::scrub::MinimalScrubber;
    use htui_core::store::{MemStore, StoreError};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    fn subject(text: &str) -> MemorySubject {
        MemorySubject::new(text, &MinimalScrubber::new([])).expect("a clean subject")
    }

    fn body(text: &str) -> MemoryBody {
        MemoryBody::new(text, &MinimalScrubber::new([])).expect("a clean body")
    }

    fn create(project: ProjectId, all: bool, title: &str) -> StoreRequest {
        StoreRequest::CreateMemoryEntry {
            project,
            all,
            kind: MemoryKind::Fact,
            subject: subject(title),
            body: body("Body text."),
        }
    }

    fn amend(id: MemoryEntryId, expected: i32, title: &str, text: &str) -> StoreRequest {
        StoreRequest::AmendMemoryEntry {
            project: ids::PROJECT_HTUI,
            all: false,
            id,
            expected,
            kind: MemoryKind::Gotcha,
            subject: subject(title),
            body: body(text),
        }
    }

    fn retire(id: MemoryEntryId, expected: i32) -> StoreRequest {
        StoreRequest::RetireMemoryEntry {
            project: ids::PROJECT_HTUI,
            all: false,
            id,
            expected,
        }
    }

    fn supersede(old: MemoryEntryId, expected: i32, title: &str) -> StoreRequest {
        StoreRequest::SupersedeMemoryEntry {
            project: ids::PROJECT_HTUI,
            all: false,
            old,
            expected,
            kind: MemoryKind::Decision,
            subject: subject(title),
            body: body("The replacement."),
        }
    }

    /// Creates `title` in `htui` and answers the landed entry.
    async fn created(backend: &Backend, title: &str) -> MemoryEntry {
        match serve(backend, &create(ids::PROJECT_HTUI, false, title)).await {
            Ok(StoreReply::MemoryWritten {
                written: MemoryWrite::Created { id },
                entries: Ok(entries),
                ..
            }) => entries
                .into_iter()
                .find(|entry| entry.id == id)
                .expect("the re-read holds the new entry"),
            other => panic!("the create answered {other:?}"),
        }
    }

    async fn entries(backend: &Backend, all: bool) -> Vec<MemoryEntry> {
        match serve(
            backend,
            &StoreRequest::MemoryEntries {
                project: ids::PROJECT_HTUI,
                all,
            },
        )
        .await
        {
            Ok(StoreReply::Memory {
                entries: Some(entries),
                ..
            }) => entries,
            other => panic!("the read answered {other:?}"),
        }
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let id = MemoryEntryId::new();
        let samples = [
            StoreRequest::MemoryEntries {
                project: ids::PROJECT_HTUI,
                all: false,
            },
            create(ids::PROJECT_HTUI, false, "A subject"),
            amend(id, 1, "A subject", "A body."),
            retire(id, 1),
            supersede(id, 1, "A subject"),
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
        assert_eq!(READ_NAME, REQUEST_NAMES[0]);
        for (index, name) in REQUEST_NAMES.iter().enumerate() {
            assert_eq!(is_write(name), index > 0, "{name}");
        }
        assert!(!is_write("requirements"));

        let outcomes = [
            MemoryWrite::Created { id },
            MemoryWrite::Amended { id, version: 2 },
            MemoryWrite::Retired { id },
            MemoryWrite::Superseded { old: id, new: id },
        ];
        for (outcome, request) in outcomes.iter().zip(&samples[1..]) {
            assert_eq!(outcome.name(), request.name(), "{outcome:?}");
        }
    }

    /// `StoreRequest` derives `Debug`; a subject and a body are user prose, so a request prints
    /// their lengths only.
    #[test]
    fn a_memory_request_prints_lengths_only() {
        let request = StoreRequest::CreateMemoryEntry {
            project: ids::PROJECT_HTUI,
            all: false,
            kind: MemoryKind::Lesson,
            subject: subject("private subject"),
            body: body("secret body"),
        };

        let shown = format!("{request:?}");

        assert!(!shown.contains("private"), "{shown}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(
            shown.contains("len: 15") && shown.contains("len: 11"),
            "{shown}"
        );
        assert!(shown.contains("Lesson"), "the kind is not prose: {shown}");
    }

    #[test]
    fn states_are_the_live_two_unless_all() {
        assert_eq!(
            states(false),
            Some(vec![MemoryState::Active, MemoryState::Suspect])
        );
        assert_eq!(states(true), None);
    }

    #[tokio::test]
    async fn entries_reads_live_by_default_and_every_state_with_all() {
        let backend = demo();
        let kept = created(&backend, "Kept").await;
        let gone = created(&backend, "Gone").await;
        let reply = serve(&backend, &retire(gone.id, gone.version)).await;
        assert!(
            matches!(reply, Ok(StoreReply::MemoryWritten { .. })),
            "{reply:?}"
        );

        let live = entries(&backend, false).await;
        let every = entries(&backend, true).await;

        assert_eq!(
            live.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            [kept.id]
        );
        assert_eq!(every.len(), 2);
        assert!(
            every
                .iter()
                .any(|entry| entry.id == gone.id && entry.state == MemoryState::Retired)
        );
    }

    #[tokio::test]
    async fn create_answers_written_with_the_fresh_list() {
        let backend = demo();

        let reply = serve(&backend, &create(ids::PROJECT_HTUI, false, "Build cache"))
            .await
            .expect("the create lands");

        let StoreReply::MemoryWritten {
            project,
            written: MemoryWrite::Created { id },
            entries: Ok(entries),
        } = reply
        else {
            panic!("the create answered {reply:?}")
        };
        assert_eq!(project, ids::PROJECT_HTUI);
        let row = entries
            .iter()
            .find(|entry| entry.id == id)
            .expect("the re-read holds it");
        assert_eq!(row.subject, "Build cache");
        assert_eq!(row.kind, MemoryKind::Fact);
        assert_eq!(row.version, 1);
    }

    #[tokio::test]
    async fn a_create_on_a_taken_subject_answers_the_holder() {
        let backend = demo();
        let holder = created(&backend, "Build cache").await;

        let reply = serve(&backend, &create(ids::PROJECT_HTUI, false, " build CACHE ")).await;

        match reply {
            Ok(StoreReply::MemorySubjectTaken { project, existing }) => {
                assert_eq!(project, ids::PROJECT_HTUI);
                assert_eq!(existing.id, holder.id);
            }
            other => panic!("a taken subject answers the holder, not {other:?}"),
        }
        assert_eq!(
            entries(&backend, true).await.len(),
            1,
            "nothing was written"
        );
    }

    #[tokio::test]
    async fn an_amend_onto_another_entrys_subject_answers_the_holder_not_itself() {
        let backend = demo();
        let alpha = created(&backend, "Alpha").await;
        let beta = created(&backend, "Beta").await;

        let reply = serve(&backend, &amend(beta.id, beta.version, "ALPHA", "Body.")).await;

        match reply {
            Ok(StoreReply::MemorySubjectTaken { existing, .. }) => {
                assert_eq!(existing.id, alpha.id);
            }
            other => panic!("the amend answers the holder, not {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_amend_keeping_its_own_subject_is_no_clash() {
        let backend = demo();
        let alpha = created(&backend, "Alpha").await;

        let reply = serve(
            &backend,
            &amend(alpha.id, alpha.version, "alpha", "New body."),
        )
        .await;

        match reply {
            Ok(StoreReply::MemoryWritten {
                written: MemoryWrite::Amended { id, version },
                entries: Ok(entries),
                ..
            }) => {
                assert_eq!((id, version), (alpha.id, 2));
                let row = entries.iter().find(|entry| entry.id == id).expect("listed");
                assert_eq!(
                    (row.subject.as_str(), row.body.as_str()),
                    ("alpha", "New body.")
                );
                assert_eq!(row.kind, MemoryKind::Gotcha);
            }
            other => panic!("the amend lands, not {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_stale_amend_answers_stale_with_the_head() {
        let backend = demo();
        let alpha = created(&backend, "Alpha").await;
        let first = serve(&backend, &amend(alpha.id, 1, "Alpha", "Mine.")).await;
        assert!(
            matches!(first, Ok(StoreReply::MemoryWritten { .. })),
            "{first:?}"
        );

        let again = serve(&backend, &amend(alpha.id, 1, "Alpha", "Theirs.")).await;

        match again {
            Ok(StoreReply::MemoryStale {
                project,
                head,
                entries,
            }) => {
                assert_eq!(project, ids::PROJECT_HTUI);
                assert_eq!((head.id, head.version), (alpha.id, 2));
                assert_eq!(head.body, "Mine.", "nothing was written");
                assert_eq!(entries.len(), 1);
            }
            other => panic!("a stale amend answers stale, not {other:?}"),
        }
        let stale_retire = serve(&backend, &retire(alpha.id, 1)).await;
        assert!(
            matches!(stale_retire, Ok(StoreReply::MemoryStale { .. })),
            "{stale_retire:?}"
        );
        let stale_supersede = serve(&backend, &supersede(alpha.id, 1, "Alpha")).await;
        assert!(
            matches!(stale_supersede, Ok(StoreReply::MemoryStale { .. })),
            "{stale_supersede:?}"
        );
    }

    #[tokio::test]
    async fn retire_and_supersede_answer_written() {
        let backend = demo();
        let alpha = created(&backend, "Alpha").await;
        let beta = created(&backend, "Beta").await;

        let retired = serve(&backend, &retire(alpha.id, alpha.version)).await;
        let replaced = serve(&backend, &supersede(beta.id, beta.version, "Beta")).await;

        match retired {
            Ok(StoreReply::MemoryWritten {
                written: MemoryWrite::Retired { id },
                entries: Ok(entries),
                ..
            }) => {
                assert_eq!(id, alpha.id);
                assert!(
                    entries.iter().all(|entry| entry.id != alpha.id),
                    "live only"
                );
            }
            other => panic!("the retire lands, not {other:?}"),
        }
        match replaced {
            Ok(StoreReply::MemoryWritten {
                written: MemoryWrite::Superseded { old, new },
                entries: Ok(entries),
                ..
            }) => {
                assert_eq!(old, beta.id);
                let ids: Vec<MemoryEntryId> = entries.iter().map(|entry| entry.id).collect();
                assert_eq!(ids, [new]);
                assert_eq!(entries[0].kind, MemoryKind::Decision);
            }
            other => panic!("the supersede lands, not {other:?}"),
        }
    }

    /// A `Constraint` with no live holder (here: an amend of a retired entry) stays the store's
    /// refusal, answered `Failed` by the worker.
    #[tokio::test]
    async fn a_refusal_with_no_live_holder_stays_failed() {
        let backend = demo();
        let alpha = created(&backend, "Alpha").await;
        let reply = serve(&backend, &retire(alpha.id, alpha.version)).await;
        assert!(
            matches!(reply, Ok(StoreReply::MemoryWritten { .. })),
            "{reply:?}"
        );

        let request = amend(alpha.id, 2, "Alpha", "Body.");
        assert!(
            matches!(
                serve(&backend, &request).await,
                Err(StoreError::Constraint(_))
            ),
            "a closed entry is the store's refusal"
        );
        match store_worker::serve(&backend, &request).await {
            StoreReply::Failed { request, .. } => assert_eq!(request, "amend_memory_entry"),
            other => panic!("answered {other:?}"),
        }
    }

    #[tokio::test]
    async fn offline_reads_answer_none_and_every_write_is_refused_before_sending() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "memory-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline { cache, since: None };

        match serve(
            &backend,
            &StoreRequest::MemoryEntries {
                project: ids::PROJECT_HTUI,
                all: false,
            },
        )
        .await
        {
            Ok(StoreReply::Memory { project, entries }) => {
                assert_eq!(project, ids::PROJECT_HTUI);
                assert_eq!(entries, None);
            }
            other => panic!("the offline read answers None, not {other:?}"),
        }

        let id = MemoryEntryId::new();
        let writes = [
            create(ids::PROJECT_HTUI, false, "A subject"),
            amend(id, 1, "A subject", "A body."),
            retire(id, 1),
            supersede(id, 1, "A subject"),
        ];
        for write in &writes {
            assert_eq!(
                serve(&backend, write).await.err(),
                Some(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())),
                "{}",
                write.name()
            );
            match store_worker::serve(&backend, write).await {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, write.name());
                    assert!(message.contains(DATABASE_UNREACHABLE), "{message}");
                }
                other => panic!("an offline {} is refused, not {other:?}", write.name()),
            }
        }
    }

    #[tokio::test]
    async fn a_foreign_request_is_named() {
        let backend = demo();

        assert_eq!(
            serve(&backend, &StoreRequest::Workspaces).await.err(),
            Some(StoreError::Backend(
                "not a memory request: workspaces".to_owned()
            ))
        );
    }

    /// The shell's `serve` routes the five here: a read answers `Memory`, and each write lands.
    #[tokio::test]
    async fn try_serve_routes_the_five() {
        let backend = demo();
        let read = StoreRequest::MemoryEntries {
            project: ids::PROJECT_HTUI,
            all: true,
        };
        assert!(matches!(
            store_worker::serve(&backend, &read).await,
            StoreReply::Memory {
                entries: Some(_),
                ..
            }
        ));
        let StoreReply::MemoryWritten {
            written: MemoryWrite::Created { id },
            ..
        } = store_worker::serve(&backend, &create(ids::PROJECT_HTUI, false, "Alpha")).await
        else {
            panic!("the create is routed")
        };
        let writes = [
            (amend(id, 1, "Alpha", "Two."), "amend_memory_entry"),
            (supersede(id, 2, "Alpha"), "supersede_memory_entry"),
        ];
        for (write, name) in writes {
            match store_worker::serve(&backend, &write).await {
                StoreReply::MemoryWritten { written, .. } => assert_eq!(written.name(), name),
                other => panic!("{name} answered {other:?}"),
            }
        }
        let live = entries(&backend, false).await;
        let StoreReply::MemoryWritten { written, .. } =
            store_worker::serve(&backend, &retire(live[0].id, live[0].version)).await
        else {
            panic!("the retire is routed")
        };
        assert_eq!(written.name(), "retire_memory_entry");
    }
}
