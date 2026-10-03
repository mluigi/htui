//! The persona registry editor of `Settings > Personas` (MOD-26 milestone 2, D20-D22):
//! [`serve`], the five requests served in the store loop, and the write outcome. Nothing here
//! reads a clock or mints an id: the store stamps every instant and the section mints a new row's
//! id (B-13). Every refusal is the store's own sentence (I-8), sent bare (B-11).

use htui_core::model::PersonaId;
use htui_core::store::{CasOutcome, Result as StoreResult, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE};

use crate::persona_import::{self, PersonaImports};
use crate::store_worker::{StoreReply, StoreRequest};

/// The five request names, in [`StoreRequest`] order (MOD-26 M2 D21). The section's `Failed`
/// match reads from here; `StoreRequest::name`'s arms spell the same five and
/// `request_names_match_the_name_arms` pins them together.
pub const REQUEST_NAMES: [&str; 5] = [
    "personas",
    "create_persona",
    "update_persona",
    "delete_persona",
    "import_personas",
];

/// The read's name: a refused read leaves the section with no list.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// The import's name: what `busy` holds while the worker walks.
pub const IMPORT_NAME: &str = REQUEST_NAMES[4];

/// What one persona write did (MOD-26 M2 D21, B-10), carried by `StoreReply::PersonaWritten`
/// beside the re-read. Ids and names only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonaWrite {
    /// A new row landed.
    Created {
        /// The row.
        id: PersonaId,
        /// Its name, as stored.
        name: String,
    },
    /// The edit applied.
    Updated {
        /// The row.
        id: PersonaId,
        /// Its name after the edit (a rename is allowed).
        name: String,
    },
    /// The row is gone; no phase held it.
    Deleted {
        /// The row the request named.
        id: PersonaId,
    },
    /// The token was spent: nothing was written; the re-read holds the row as it is now.
    Stale {
        /// The row.
        id: PersonaId,
    },
    /// The row was already gone (`NotFound` on an update or a delete). Nothing was written.
    Gone {
        /// The row the request named.
        id: PersonaId,
    },
}

/// Serves the five persona requests (MOD-26 M2 D21) in the store loop.
///
/// Offline there is no writer, and all five are `Err(Unreachable(DATABASE_UNREACHABLE))` before
/// any read (`R-STO-4`; the import before any file is read). A store `Constraint` from a write is
/// `Ok(Failed)` carrying its sentence byte for byte (B-11); every other error propagates, so an
/// `Unreachable` still drops an `Online` backend onto the mirror. Every write that reached the
/// store answers `PersonaWritten` with the registry re-read. The import is the one exception to
/// propagating (R1 L-2): its report is never lost, so a re-read that fails after the batch rides
/// typed inside `PersonaImports`, and the worker still goes offline on its `Unreachable`
/// ([`crate::store_worker`]'s `lost_the_store`, R1 ADV-1).
///
/// # Errors
/// Whatever the store reports, `Unreachable` offline, and `Backend` for a request that is not one
/// of the five.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> StoreResult<StoreReply> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;

    let outcome = match request {
        StoreRequest::Personas => return Ok(StoreReply::Personas(writer.personas().await?)),
        StoreRequest::CreatePersona { new } => match writer.create_persona(new.clone()).await {
            Ok(row) => PersonaWrite::Created {
                id: row.id,
                name: row.name,
            },
            Err(StoreError::Constraint(sentence)) => return Ok(refused(request, sentence)),
            Err(other) => return Err(other),
        },
        StoreRequest::UpdatePersona {
            id,
            expected,
            patch,
        } => match writer.update_persona(*id, *expected, patch.clone()).await {
            Ok(CasOutcome::Applied(row)) => PersonaWrite::Updated {
                id: *id,
                name: row.name,
            },
            Ok(CasOutcome::Stale(_)) => PersonaWrite::Stale { id: *id },
            Err(StoreError::NotFound {
                entity: "persona", ..
            }) => PersonaWrite::Gone { id: *id },
            Err(StoreError::Constraint(sentence)) => return Ok(refused(request, sentence)),
            Err(other) => return Err(other),
        },
        StoreRequest::DeletePersona { id } => match writer.delete_persona(*id).await {
            Ok(()) => PersonaWrite::Deleted { id: *id },
            Err(StoreError::NotFound {
                entity: "persona", ..
            }) => PersonaWrite::Gone { id: *id },
            Err(StoreError::Constraint(sentence)) => return Ok(refused(request, sentence)),
            Err(other) => return Err(other),
        },
        StoreRequest::ImportPersonas { path } => {
            let report = persona_import::import(backend, path).await?;
            return Ok(imports(report, writer.personas().await));
        }
        other => {
            return Err(StoreError::Backend(format!(
                "not a persona request: {}",
                other.name()
            )));
        }
    };
    // Every outcome carries the registry as it is now (agent_settings' rule): the section renders
    // the whole list and never patches a row in. A re-read that fails after an applied write
    // answers `Failed`, the agent registry's residue.
    let personas = writer.personas().await?;
    Ok(StoreReply::PersonaWritten { personas, outcome })
}

/// An import's answer: the report, and the registry re-read after it (R1 L-2). Rows were written
/// by then, so a re-read that fails (the store lost mid-batch, most often) is carried beside the
/// report rather than dropping the report; the section keeps its list and shows the sentence. The
/// error stays typed so an `Unreachable` still takes the backend offline (R1 ADV-1). A write that
/// met `Unreachable` with a re-read that then answered is not a loss: the store is there.
fn imports(
    report: Vec<persona_import::PersonaOutcome>,
    reread: StoreResult<Vec<htui_core::model::Persona>>,
) -> StoreReply {
    StoreReply::PersonaImports(Box::new(PersonaImports {
        personas: reread,
        report,
    }))
}

/// A store refusal as the section shows it (I-8, B-11): `Failed` carrying the sentence itself,
/// never `constraint violated: …`.
fn refused(request: &StoreRequest, sentence: String) -> StoreReply {
    StoreReply::Failed {
        request: request.name(),
        message: sentence,
    }
}

#[cfg(test)]
mod tests {
    use super::{IMPORT_NAME, PersonaWrite, READ_NAME, REQUEST_NAMES, imports, serve};
    use crate::persona_import::PersonaOutcome;
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use chrono::{DateTime, Duration};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        NewPersona, Persona, PersonaId, PersonaPatch, PersonaPermission, PersonaTools, PhasePatch,
    };
    use htui_core::store::{
        CasOutcome, MemStore, StoreError, WriteStore as _, already_exists, invalid_persona_name,
        persona_is_bound,
    };
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    async fn offline() -> (tempfile::TempDir, Backend) {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "personas-offline", 1)
            .await
            .expect("open a throwaway mirror");
        (root, Backend::Offline { cache, since: None })
    }

    fn new_persona(name: &str) -> NewPersona {
        NewPersona {
            id: PersonaId::new(),
            name: name.to_owned(),
            description: "Scouts ahead.".to_owned(),
            body: "You scout.\n".to_owned(),
            tools: PersonaTools::default(),
            permission: PersonaPermission::default(),
        }
    }

    fn create(name: &str) -> StoreRequest {
        StoreRequest::CreatePersona {
            new: new_persona(name),
        }
    }

    /// The registry as the read answers it.
    async fn registry(backend: &Backend) -> Vec<Persona> {
        match serve(backend, &StoreRequest::Personas).await {
            Ok(StoreReply::Personas(rows)) => rows,
            other => panic!("the read answered {other:?}"),
        }
    }

    async fn row(backend: &Backend, name: &str) -> Persona {
        registry(backend)
            .await
            .into_iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("`{name}` is in the registry"))
    }

    /// The registry and outcome of a `PersonaWritten`, or a panic naming what came back.
    #[track_caller]
    fn written(reply: StoreReply) -> (Vec<Persona>, PersonaWrite) {
        match reply {
            StoreReply::PersonaWritten { personas, outcome } => (personas, outcome),
            other => panic!("expected `PersonaWritten`, not {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_read_answers_the_registry_by_name() {
        let names: Vec<String> = registry(&demo())
            .await
            .into_iter()
            .map(|row| row.name)
            .collect();
        assert_eq!(names, ["architect", "reviewer"]);
    }

    #[tokio::test]
    async fn a_create_answers_created_with_the_reread() {
        let backend = demo();
        let request = create("scout");
        let StoreRequest::CreatePersona { new } = &request else {
            unreachable!()
        };
        let id = new.id;

        let (personas, outcome) = written(
            serve(&backend, &request)
                .await
                .expect("the create is served"),
        );

        assert_eq!(
            outcome,
            PersonaWrite::Created {
                id,
                name: "scout".to_owned()
            }
        );
        assert!(
            personas
                .iter()
                .any(|row| row.id == id && row.name == "scout")
        );
    }

    /// B-11: the store's sentence, byte for byte, never `constraint violated: …`.
    #[tokio::test]
    async fn a_refused_create_answers_the_stores_sentence_bare() {
        let backend = demo();
        for (name, sentence) in [
            ("Bad", invalid_persona_name("Bad")),
            ("reviewer", already_exists("persona", "reviewer")),
        ] {
            let reply = serve(&backend, &create(name)).await.expect("served");
            let StoreReply::Failed { request, message } = reply else {
                panic!("a refused create is `Failed`, not {reply:?}")
            };
            assert_eq!(request, "create_persona");
            assert_eq!(message, sentence);
            assert!(!message.contains("constraint violated"), "{message}");
        }
        assert_eq!(registry(&backend).await.len(), 2, "nothing was written");
    }

    #[tokio::test]
    async fn an_update_applies_only_the_patch_it_carries() {
        let backend = demo();
        let before = row(&backend, "architect").await;
        let request = StoreRequest::UpdatePersona {
            id: before.id,
            expected: before.updated_at,
            patch: PersonaPatch {
                description: Some("Designs first.".to_owned()),
                ..PersonaPatch::default()
            },
        };

        let (personas, outcome) = written(serve(&backend, &request).await.expect("served"));

        assert_eq!(
            outcome,
            PersonaWrite::Updated {
                id: before.id,
                name: "architect".to_owned()
            }
        );
        let after = personas
            .into_iter()
            .find(|row| row.id == before.id)
            .expect("the row is in the re-read");
        assert_eq!(after.description, "Designs first.");
        assert_eq!(
            (&after.name, &after.body, &after.tools, &after.permission),
            (
                &before.name,
                &before.body,
                &before.tools,
                &before.permission
            ),
            "every other field is the row's"
        );
    }

    #[tokio::test]
    async fn a_spent_token_answers_stale_and_writes_nothing() {
        let backend = demo();
        let before = row(&backend, "architect").await;
        let request = StoreRequest::UpdatePersona {
            id: before.id,
            expected: before.updated_at - Duration::seconds(1),
            patch: PersonaPatch {
                description: Some("Designs first.".to_owned()),
                ..PersonaPatch::default()
            },
        };

        let (_, outcome) = written(serve(&backend, &request).await.expect("served"));

        assert_eq!(outcome, PersonaWrite::Stale { id: before.id });
        assert_eq!(row(&backend, "architect").await, before, "unchanged");
    }

    #[tokio::test]
    async fn an_update_of_a_gone_row_answers_gone() {
        let backend = demo();
        let id = PersonaId::new();
        let request = StoreRequest::UpdatePersona {
            id,
            expected: DateTime::UNIX_EPOCH,
            patch: PersonaPatch::default(),
        };

        let (_, outcome) = written(serve(&backend, &request).await.expect("served"));

        assert_eq!(outcome, PersonaWrite::Gone { id });
    }

    #[tokio::test]
    async fn a_delete_answers_deleted_and_the_row_is_gone() {
        let backend = demo();
        let id = ids::PERSONA_REVIEWER;

        let (personas, outcome) = written(
            serve(&backend, &StoreRequest::DeletePersona { id })
                .await
                .expect("served"),
        );

        assert_eq!(outcome, PersonaWrite::Deleted { id });
        assert!(personas.iter().all(|row| row.id != id), "{personas:?}");
    }

    /// D14: a bound persona is refused with `persona_is_bound`'s sentence, bare (B-11).
    #[tokio::test]
    async fn a_bound_delete_answers_the_bound_sentence() {
        let store = MemStore::demo();
        let graph = store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("read")
            .into_iter()
            .find(|graph| graph.name == "feature")
            .expect("the htui project has a `feature` graph");
        let phase = store
            .phases(graph.id)
            .await
            .expect("read")
            .into_iter()
            .find(|phase| phase.name == "review")
            .expect("the `feature` graph has a `review` phase");
        let bound = store
            .update_phase(
                phase.id,
                phase.updated_at,
                PhasePatch {
                    persona: Some(Some(ids::PERSONA_REVIEWER)),
                    ..PhasePatch::default()
                },
            )
            .await
            .expect("the bind is served");
        assert!(matches!(bound, CasOutcome::Applied(_)), "{bound:?}");
        let backend = Backend::memory(store);

        let reply = serve(
            &backend,
            &StoreRequest::DeletePersona {
                id: ids::PERSONA_REVIEWER,
            },
        )
        .await
        .expect("served");

        let StoreReply::Failed { request, message } = reply else {
            panic!("a bound delete is `Failed`, not {reply:?}")
        };
        assert_eq!(request, "delete_persona");
        assert_eq!(
            message,
            persona_is_bound(
                "reviewer",
                &[("htui".to_owned(), "feature".to_owned(), "review".to_owned())]
            )
        );
        assert_eq!(registry(&backend).await.len(), 2, "nothing was deleted");
    }

    #[tokio::test]
    async fn a_delete_of_a_gone_row_answers_gone() {
        let backend = demo();
        let id = PersonaId::new();

        let (_, outcome) = written(
            serve(&backend, &StoreRequest::DeletePersona { id })
                .await
                .expect("served"),
        );

        assert_eq!(outcome, PersonaWrite::Gone { id });
    }

    /// One of each of the five, in [`REQUEST_NAMES`] order.
    fn samples() -> [StoreRequest; 5] {
        [
            StoreRequest::Personas,
            create("scout"),
            StoreRequest::UpdatePersona {
                id: ids::PERSONA_REVIEWER,
                expected: DateTime::UNIX_EPOCH,
                patch: PersonaPatch::default(),
            },
            StoreRequest::DeletePersona {
                id: ids::PERSONA_REVIEWER,
            },
            StoreRequest::ImportPersonas {
                path: "/nowhere".to_owned(),
            },
        ]
    }

    /// Personas are not mirrored: offline all five are refused before any read, under their own
    /// names through the worker.
    #[tokio::test]
    async fn offline_every_request_is_refused_with_the_unreachable_sentence() {
        let (_root, backend) = offline().await;
        for (request, name) in samples().iter().zip(REQUEST_NAMES) {
            assert_eq!(
                serve(&backend, request).await.err(),
                Some(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())),
                "{name}"
            );
            match store_worker::serve(&backend, request).await {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, name);
                    assert!(message.contains(DATABASE_UNREACHABLE), "{message}");
                }
                other => panic!("an offline {name} is refused, not {other:?}"),
            }
        }
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let names: Vec<&str> = samples().iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
        assert_eq!(READ_NAME, REQUEST_NAMES[0]);
        assert_eq!(IMPORT_NAME, REQUEST_NAMES[4]);
    }

    #[tokio::test]
    async fn a_foreign_request_is_refused_by_name() {
        assert_eq!(
            serve(&demo(), &StoreRequest::Workspaces).await.err(),
            Some(StoreError::Backend(
                "not a persona request: workspaces".to_owned()
            ))
        );
    }

    /// R1 L-2: a re-read that fails after the batch loses nothing: the report rides with the
    /// read's sentence in place of the registry.
    #[test]
    fn a_failed_reread_keeps_the_import_report() {
        let report = vec![PersonaOutcome::Imported {
            name: "scout".to_owned(),
            path: "/srv/agents/scout.md".to_owned(),
            dropped: Vec::new(),
        }];
        let lost = StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned());

        let reply = imports(report.clone(), Err(lost.clone()));

        let StoreReply::PersonaImports(imports) = reply else {
            panic!("the report is kept: {reply:?}")
        };
        assert_eq!(imports.report, report);
        assert_eq!(
            imports.personas,
            Err(lost),
            "the error stays typed (R1 ADV-1)"
        );
    }

    /// `try_serve` routes the import here; the report rides beside a registry read after it.
    #[tokio::test]
    async fn an_import_is_routed_here_and_answers_its_report_beside_the_registry() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("scout.md");
        std::fs::write(
            &path,
            "---\nname: scout\ndeny-kinds: execute\n---\n\nYou scout.\n",
        )
        .expect("write");

        let reply = store_worker::serve(
            &backend,
            &StoreRequest::ImportPersonas {
                path: dir.path().display().to_string(),
            },
        )
        .await;

        let StoreReply::PersonaImports(imports) = reply else {
            panic!("`import_personas` answers `PersonaImports`, not {reply:?}")
        };
        assert!(
            matches!(
                &imports.report[..],
                [PersonaOutcome::Imported { name, .. }] if name == "scout"
            ),
            "{:?}",
            imports.report
        );
        assert!(
            imports
                .personas
                .as_ref()
                .is_ok_and(|rows| rows.iter().any(|row| row.name == "scout")),
            "the registry is read after the writes"
        );
    }
}
