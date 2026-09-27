//! The skill library and its attachments behind the Skills tab's Skills view (MOD-9 milestone 3,
//! D81): one read per scope, one save, three attachment writers, the
//! [`crate::templates`] shape.
//!
//! One read per event and never one per keystroke, one reply out, and every write through
//! [`WriteStore`]. The view renders from the snapshot and never patches a row into it. The
//! re-read-rather-than-patch rule, the refusal arm that names the request that got here by mistake,
//! and `request_names_match_the_name_arms` are `templates.rs`'s and are carried over.
//!
//! Known residue, the same one `templates.rs` and [`crate::catalogue`] carry: a re-read that fails
//! *after* an applied write answers `Failed`, so the view is told nothing happened when a version
//! has in fact been appended.
//!
//! The worker fills `created_by` from [`Backend::this_user`], mints the ids and holds no
//! `UserId` (`R-NF-3`); nothing here reads the clock; the store stamps every instant.

use htui_core::model::{
    NewSkill, NewSkillBinding, NewSkillVersion, PhaseId, ProjectId, Scope, SkillAttachmentRow,
    SkillBindingId, SkillId, SkillVersion,
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
    pub id: SkillId,
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
        skill: SkillId,
        project: Option<ProjectId>,
        phase: Option<PhaseId>,
    ) -> Option<&SkillAttachmentRow> {
        self.attachments
            .iter()
            .find(|row| row.skill_id == skill && row.project_id == project && row.phase_id == phase)
    }
}

/// One read of the scope: the whole library, and the scope's attachments in one request (D92 —
/// **never** one request per project, F-2, because the shell's staleness index keys on the request
/// variant and would drop all but the last).
///
/// # Errors
/// Whatever the backend reports; offline, [`StoreError::Unreachable`] with
/// [`htui_store::PROMPT_ON_SERVER_ONLY`] for both reads, because the `skill*` tables are not
/// mirrored.
pub async fn snapshot(backend: &Backend, scope: &Scope) -> Result<SkillsSnapshot> {
    let skills = backend
        .skill_library()
        .await?
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
    Ok(SkillsSnapshot {
        skills,
        attachments,
    })
}

/// The four request names, in [`StoreRequest`] order.
///
/// The views' `Failed` match reads from here. [`StoreRequest::name`]'s arms spell the same four as
/// literals, because it is a `const fn`, and `request_names_match_the_name_arms` pins them to this
/// list, so a name changed in one place and not the other fails there.
pub const REQUEST_NAMES: [&str; 4] = [
    "skills",
    "save_skill",
    "set_skill_binding",
    "remove_skill_binding",
];

/// The **read**'s name: a refused read leaves the view with no library, where a refused write
/// leaves the editor over its text.
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
        // are inherent on each store and refuse offline with their own sentence, which is the one
        // the view shows.
        StoreRequest::Skills(scope) => Ok(StoreReply::Skills(Box::new(
            snapshot(backend, scope).await?,
        ))),
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
            // and the head version for the append — and the write is short-circuited on the first
            // `Stale`, so a stale token can never leave a moved description behind an unsaved body.
            let saved = match writer
                .upsert_skill(
                    NewSkill {
                        id: SkillId::new(),
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
            Ok(StoreReply::Skills(Box::new(
                snapshot(backend, scope).await?,
            )))
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
                CasOutcome::Applied(_) => Ok(StoreReply::Skills(Box::new(
                    snapshot(backend, scope).await?,
                ))),
                CasOutcome::Stale(_) => stale(backend, scope).await,
            }
        }
        StoreRequest::RemoveSkillBinding {
            scope,
            id,
            expected,
        } => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
            match writer.remove_skill_binding(*id, *expected).await? {
                CasOutcome::Applied(_) => Ok(StoreReply::Skills(Box::new(
                    snapshot(backend, scope).await?,
                ))),
                CasOutcome::Stale(_) => stale(backend, scope).await,
            }
        }
        // `try_serve` routes exactly this module's four variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a skill request: {}",
            other.name()
        ))),
    }
}

/// The fresh snapshot a spent token answers with: the store as it is, unchanged, which is what the
/// editor reloads against (D101).
async fn stale(backend: &Backend, scope: &Scope) -> Result<StoreReply> {
    Ok(StoreReply::SkillsStale(Box::new(
        snapshot(backend, scope).await?,
    )))
}

#[cfg(test)]
mod tests {
    use super::{READ_NAME, REQUEST_NAMES, SkillBody, SkillsSnapshot, serve};
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use htui_core::fixtures::ids;
    use htui_core::model::{Activation, PhaseId, ProjectId, Scope, SkillBindingId, SkillId};
    use htui_core::store::{MemStore, StoreError};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PROMPT_ON_SERVER_ONLY};

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    /// The Platform workspace's scope: `htui` then `agy`, by `workspace_project.position`.
    async fn platform_scope(backend: &Backend) -> Scope {
        let StoreReply::Workspaces(workspaces) =
            store_worker::serve(backend, &StoreRequest::Workspaces).await
        else {
            panic!("workspaces answered with the wrong variant")
        };
        let platform = workspaces
            .iter()
            .find(|w| w.slug == "platform")
            .expect("the demo fixture holds the `platform` workspace");
        Scope::from_workspace(platform)
    }

    async fn read(backend: &Backend, scope: &Scope) -> SkillsSnapshot {
        match serve(backend, &StoreRequest::Skills(scope.clone())).await {
            Ok(StoreReply::Skills(snapshot)) => *snapshot,
            other => panic!("the read answered {other:?}"),
        }
    }

    fn save(
        scope: &Scope,
        name: &str,
        description: &str,
        body: &str,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        expected_version: Option<i32>,
    ) -> StoreRequest {
        StoreRequest::SaveSkill {
            scope: scope.clone(),
            name: name.to_owned(),
            description: description.to_owned(),
            body: SkillBody::new(body),
            expected,
            expected_version,
        }
    }

    fn set(
        scope: &Scope,
        skill_id: SkillId,
        project: Option<ProjectId>,
        phase: Option<PhaseId>,
        position: i32,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> StoreRequest {
        StoreRequest::SetSkillBinding {
            scope: scope.clone(),
            skill_id,
            project_id: project,
            phase_id: phase,
            pinned_version: None,
            position,
            activation: Activation::Always,
            globs: Vec::new(),
            languages: Vec::new(),
            expected,
        }
    }

    /// D92: one request for the whole scope, and the snapshot answers the whole library and every
    /// attachment that applies — never one request per project, which the shell's staleness index
    /// (which keys on the request variant) would drop all but the last of.
    #[tokio::test]
    async fn the_read_answers_the_whole_library_and_the_whole_scope() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        assert_eq!(scope.project_ids, vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);

        let snapshot = read(&backend, &scope).await;

        let names: Vec<&str> = snapshot.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["rust-style", "tests"],
            "the whole library, in `skill.name` byte order"
        );
        let versions: Vec<i32> = snapshot.skills[0]
            .versions
            .iter()
            .map(|version| version.version)
            .collect();
        assert_eq!(
            versions,
            vec![1, 2],
            "ascending, so the last is the head a pin or an unpinned attachment resolves to"
        );
        assert_eq!(
            snapshot.head("rust-style").map(|row| row.version),
            Some(2),
            "the head is the last version, not the max of an unordered list"
        );
        assert_eq!(
            snapshot
                .version("rust-style", 1)
                .map(|row| row.body.as_str()),
            Some("Prefer `expect` with a reason.")
        );
        assert_eq!(snapshot.named("no-such-skill"), None);
        assert_eq!(snapshot.head("no-such-skill"), None);
        assert_eq!(snapshot.version("rust-style", 9), None);

        let rows: Vec<SkillBindingId> = snapshot.attachments.iter().map(|row| row.id).collect();
        assert_eq!(
            rows,
            vec![
                ids::BINDING_HTUI_RUST_STYLE,
                ids::BINDING_HTUI_TESTS,
                ids::BINDING_HTUI_IMPLEMENT_RUST_STYLE,
            ],
            "the matrix's order: globals first, then the projects, then the phases, and name \
             bytes breaking a tie"
        );
        let project_row = snapshot
            .attachment(ids::SKILL_TESTS, Some(ids::PROJECT_HTUI), None)
            .expect("`tests` is attached to `htui`");
        assert_eq!(project_row.project_slug.as_deref(), Some("htui"));
        assert_eq!(project_row.phase_name, None, "a project row joins no phase");
        assert_eq!(
            snapshot
                .attachment(ids::SKILL_TESTS, None, None)
                .map(|row| row.id),
            None,
            "the demo holds no global attachment, and one for another project is not in scope"
        );
        assert_eq!(
            snapshot
                .attachment(ids::SKILL_RUST_STYLE, Some(ids::PROJECT_AGY), None)
                .map(|row| row.id),
            None,
            "a scope project's own rows only"
        );
        assert_eq!(
            snapshot
                .attachment(
                    ids::SKILL_RUST_STYLE,
                    Some(ids::PROJECT_HTUI),
                    Some(ids::PHASE_HTUI_IMPLEMENT)
                )
                .and_then(|row| row.phase_name.as_deref()),
            Some("implement"),
            "a phase row carries the joined name, and is its own attachment rather than a winner"
        );
    }

    /// `StoreRequest` derives `Debug`; a body is user text, so the save prints its length only
    /// (`ExternalEdit`'s and `TextArea`'s rule).
    #[test]
    fn a_save_request_debug_prints_the_body_length_not_the_body() {
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let request = save(
            &scope,
            "house-rules",
            "the ones we always follow",
            "secret",
            None,
            None,
        );
        let shown = format!("{request:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("len: 6"), "{shown}");
        assert!(
            shown.contains("house-rules"),
            "the name is not user text: {shown}"
        );
    }

    /// OQ-18: a save upserts the row and then appends the body as the next version, so one reply
    /// moves both and the head is the version the editor did not have.
    #[tokio::test]
    async fn a_save_at_the_head_appends_and_answers_skills() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        let entry = before
            .named("rust-style")
            .expect("the fixture holds `rust-style`");
        let (token, description) = (entry.updated_at, entry.description.clone());
        let v2 = before
            .version("rust-style", 2)
            .expect("the fixture holds v2")
            .clone();
        let body = "Prefer `expect` with a reason. One error enum per crate.\nAnd a `where` \
                    clause.\n";

        let reply = serve(
            &backend,
            &save(
                &scope,
                "rust-style",
                &description,
                body,
                Some(token),
                Some(2),
            ),
        )
        .await;

        let Ok(StoreReply::Skills(after)) = reply else {
            panic!("an applied save answers `Skills`, got {reply:?}")
        };
        let head = after
            .head("rust-style")
            .expect("the new head is in the fresh snapshot");
        assert_eq!(head.version, 3, "the head's plus one");
        assert_eq!(head.body, body);
        assert_eq!(head.created_by, ids::USER, "the worker fills `created_by`");
        assert_eq!(
            after.version("rust-style", 2).map(|row| row.body.clone()),
            Some(v2.body),
            "append-only: v2 is untouched"
        );
        assert_eq!(
            after.named("rust-style").map(|row| row.id),
            Some(ids::SKILL_RUST_STYLE),
            "the name is the key, so an edit writes the row back under its own id"
        );
        assert_eq!(
            after.skills.len(),
            before.skills.len(),
            "a save adds a version, never a second library entry"
        );
    }

    /// D101: the two tokens are two surfaces and the write short-circuits on the first `Stale`, so
    /// a spent `skill.updated_at` leaves the moved description *and* the unsaved body both
    /// unwritten — and the reply is the store as it is.
    #[tokio::test]
    async fn a_save_at_a_spent_token_answers_skills_stale_and_writes_nothing() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        let entry = before
            .named("rust-style")
            .expect("the fixture holds `rust-style`");
        let (token, description) = (entry.updated_at, entry.description.clone());
        let first = serve(
            &backend,
            &save(
                &scope,
                "rust-style",
                &description,
                "first\n",
                Some(token),
                Some(2),
            ),
        )
        .await;
        assert!(
            matches!(first, Ok(StoreReply::Skills(_))),
            "the first save applies: {first:?}"
        );
        let middle = read(&backend, &scope).await;

        let reply = serve(
            &backend,
            &save(
                &scope,
                "rust-style",
                "a description the first save never wrote",
                "second\n",
                Some(token),
                Some(2),
            ),
        )
        .await;

        let Ok(StoreReply::SkillsStale(fresh)) = reply else {
            panic!("a spent token answers `SkillsStale`, got {reply:?}")
        };
        assert_eq!(
            *fresh, middle,
            "the stale reply is the store as it is, unchanged"
        );
        assert_eq!(
            fresh.head("rust-style").map(|row| row.body.as_str()),
            Some("first\n"),
            "the second body was never appended"
        );
        assert_eq!(
            fresh
                .named("rust-style")
                .map(|row| row.description.as_str()),
            Some(description.as_str()),
            "the short circuit is on the first `Stale`, so the description did not move either"
        );
    }

    /// D77 / OQ-20: the Agent Skills rule is the writer's, so its sentence is what the view shows
    /// and nothing was written on the way to it.
    #[tokio::test]
    async fn a_refused_name_answers_failed_with_the_agent_skills_rule() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;

        let reply =
            store_worker::serve(&backend, &save(&scope, "House Rules", "d", "b", None, None)).await;

        let StoreReply::Failed { request, message } = reply else {
            panic!("a refused name answers `Failed`, got {reply:?}")
        };
        assert_eq!(request, "save_skill");
        assert!(
            message.contains("must be 1-64 characters of `[a-z0-9-]`"),
            "the writer's sentence reaches the view: {message}"
        );
        assert_eq!(read(&backend, &scope).await, before, "nothing is written");
    }

    /// D78: `set_skill_binding` is an upsert on the unique key, so a spent `updated_at` is `Stale`
    /// and the row another writer changed survives.
    #[tokio::test]
    async fn a_set_at_a_spent_token_answers_skills_stale_and_the_row_is_as_it_was() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        let row = before
            .attachment(ids::SKILL_TESTS, Some(ids::PROJECT_HTUI), None)
            .expect("`tests` is attached to `htui`")
            .clone();
        let request = |position| {
            set(
                &scope,
                ids::SKILL_TESTS,
                Some(ids::PROJECT_HTUI),
                None,
                position,
                Some(row.updated_at),
            )
        };

        let first = serve(&backend, &request(7)).await;
        assert!(
            matches!(first, Ok(StoreReply::Skills(_))),
            "the first set applies: {first:?}"
        );
        let middle = read(&backend, &scope).await;
        let moved = middle
            .attachment(ids::SKILL_TESTS, Some(ids::PROJECT_HTUI), None)
            .expect("the row is still there");
        assert_eq!(
            moved.position, 7,
            "the replace wrote the columns it was given"
        );
        assert_eq!(moved.id, row.id, "and kept the row's own id");

        let reply = serve(&backend, &request(9)).await;

        let Ok(StoreReply::SkillsStale(fresh)) = reply else {
            panic!("a spent token answers `SkillsStale`, got {reply:?}")
        };
        assert_eq!(
            *fresh, middle,
            "the stale reply is the store as it is, unchanged"
        );
        assert_eq!(
            fresh
                .attachment(ids::SKILL_TESTS, Some(ids::PROJECT_HTUI), None)
                .map(|left| left.position),
            Some(7),
            "the row is as the writer that won left it"
        );
    }

    /// OQ-19: an unbind removes the row rather than writing `activation = off`, and the reply
    /// re-reads the whole scope so the matrix drops it without a second request.
    #[tokio::test]
    async fn an_unbind_answers_skills_without_the_row() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        let row = before
            .attachment(ids::SKILL_TESTS, Some(ids::PROJECT_HTUI), None)
            .expect("`tests` is attached to `htui`")
            .clone();

        let reply = serve(
            &backend,
            &StoreRequest::RemoveSkillBinding {
                scope: scope.clone(),
                id: row.id,
                expected: row.updated_at,
            },
        )
        .await;

        let Ok(StoreReply::Skills(after)) = reply else {
            panic!("an applied unbind answers `Skills`, got {reply:?}")
        };
        assert_eq!(
            after.attachment(ids::SKILL_TESTS, Some(ids::PROJECT_HTUI), None),
            None,
            "the row is gone, not an attachment reading as `off`"
        );
        assert_eq!(
            after.attachments.len(),
            before.attachments.len() - 1,
            "and only that row: the library and the phase attachment are untouched"
        );
        assert_eq!(
            after.skills, before.skills,
            "an unbind never touches a version"
        );
    }

    /// OQ-19 / R-32: an unbind is as safe as every other write. A spent token is `Stale`, the row
    /// another writer changed survives, and the view is told to reload rather than shown a
    /// removal that never happened.
    #[tokio::test]
    async fn an_unbind_at_a_spent_token_answers_skills_stale_and_the_row_survives() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        let row = before
            .attachment(ids::SKILL_TESTS, Some(ids::PROJECT_HTUI), None)
            .expect("`tests` is attached to `htui`")
            .clone();
        let first = serve(
            &backend,
            &set(
                &scope,
                ids::SKILL_TESTS,
                Some(ids::PROJECT_HTUI),
                None,
                7,
                Some(row.updated_at),
            ),
        )
        .await;
        assert!(
            matches!(first, Ok(StoreReply::Skills(_))),
            "the write that spends the token applies: {first:?}"
        );
        let middle = read(&backend, &scope).await;

        let reply = serve(
            &backend,
            &StoreRequest::RemoveSkillBinding {
                scope: scope.clone(),
                id: row.id,
                expected: row.updated_at,
            },
        )
        .await;

        let Ok(StoreReply::SkillsStale(fresh)) = reply else {
            panic!("a spent token answers `SkillsStale`, got {reply:?}")
        };
        assert_eq!(
            *fresh, middle,
            "the stale reply is the store as it is, unchanged"
        );
        assert_eq!(
            fresh
                .attachment(ids::SKILL_TESTS, Some(ids::PROJECT_HTUI), None)
                .map(|left| left.position),
            Some(7),
            "a row another writer changed survives the unbind that lost the race"
        );
    }

    /// The staleness index keys on [`StoreRequest::name`], so a name spelled one way in
    /// [`REQUEST_NAMES`] and another in the `name` arms would silently supersede a reply the view
    /// was waiting for.
    #[test]
    fn request_names_match_the_name_arms() {
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        assert_eq!(READ_NAME, REQUEST_NAMES[0]);
        let samples = [
            StoreRequest::Skills(scope.clone()),
            save(&scope, "house-rules", "", "", None, None),
            set(&scope, ids::SKILL_TESTS, None, None, 0, None),
            StoreRequest::RemoveSkillBinding {
                scope,
                id: ids::BINDING_HTUI_TESTS,
                expected: chrono::Utc::now(),
            },
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
    }

    /// Offline there is no writer, so the save is refused before anything is sent: the worker
    /// answers `Failed` for `save_skill` with the unreachable-database sentence. There is no
    /// server here to check for a row, and none is needed: nothing could have reached one.
    #[tokio::test]
    async fn an_offline_save_is_refused_with_the_unreachable_sentence() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "skills-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline { cache, since: None };
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let request = save(&scope, "house-rules", "", "a body", None, None);

        assert_eq!(
            serve(&backend, &request).await.err(),
            Some(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))
        );
        match store_worker::serve(&backend, &request).await {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, "save_skill");
                assert!(
                    message.contains(DATABASE_UNREACHABLE),
                    "the refusal names the unreachable database: {message}"
                );
            }
            other => panic!("an offline save is refused, not {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_offline_read_is_refused_with_the_server_only_sentence() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "skills-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline { cache, since: None };
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };

        let reply = serve(&backend, &StoreRequest::Skills(scope)).await;

        assert_eq!(
            reply.err(),
            Some(StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))
        );
    }
}
