//! The prompt templates behind the Skills tab's Templates view (MOD-9 milestone 1, D5): one read
//! per scope, one compare-and-set append, the [`crate::prompt_settings`] shape.
//!
//! One read per event and never one per keystroke, one reply out, and the one write through
//! `WriteStore::append_prompt_template`, whose token is the head version the editor opened on
//! (PRD D5, plan D1). The view renders from the snapshot and never patches a row into it.
//!
//! Known residue, the same one [`crate::prompt_settings`] and [`crate::catalogue`] carry: a
//! re-read that fails *after* an applied write answers `Failed`, so the view is told nothing
//! happened when a version has in fact been appended.
//!
//! The worker fills `created_by` from [`Backend::this_user`], as [`crate::hierarchy`] does: the
//! render side never holds a `UserId` (`R-NF-3`). Nothing here reads the clock; the store stamps
//! both instants.

use htui_core::model::{NewPromptTemplate, ProjectId, PromptTemplate, PromptTemplateId, Scope};
use htui_core::store::{CasOutcome, Result, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE};

use crate::store_worker::{StoreReply, StoreRequest};

/// Every scope project's templates, every version.
///
/// `Eq` is absent because [`PromptTemplate`] derives `PartialEq` only.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplatesSnapshot {
    /// One entry per id of `scope.project_ids`, in that order.
    pub projects: Vec<ProjectTemplates>,
}

/// A body on its way to the store in [`StoreRequest::SaveTemplate`]. `StoreRequest` derives
/// `Debug`, and a template body is user text, so this prints its length only (the rule of
/// [`crate::editor::ExternalEdit`] and [`crate::ui::TextArea`]).
#[derive(Clone, PartialEq, Eq)]
pub struct TemplateBody(String);

impl TemplateBody {
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

impl core::fmt::Debug for TemplateBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TemplateBody")
            .field("len", &self.0.len())
            .finish()
    }
}

/// One project's rows, `(name, version)` byte order as `Backend::prompt_templates` returns them.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectTemplates {
    /// Which project.
    pub project_id: ProjectId,
    /// Every version of every name the project holds.
    pub templates: Vec<PromptTemplate>,
}

impl TemplatesSnapshot {
    /// The highest version of `(project, name)`: the compare-and-set token a save of that name
    /// passes. `None` when the project is not in the snapshot or holds no such name.
    #[must_use]
    pub fn head(&self, project: ProjectId, name: &str) -> Option<&PromptTemplate> {
        self.named(project, name).max_by_key(|row| row.version)
    }

    /// One version of `(project, name)`.
    #[must_use]
    pub fn version(&self, project: ProjectId, name: &str, version: i32) -> Option<&PromptTemplate> {
        self.named(project, name).find(|row| row.version == version)
    }

    /// Every version of `(project, name)`, in snapshot order.
    fn named<'a, 'n>(
        &'a self,
        project: ProjectId,
        name: &'n str,
    ) -> impl Iterator<Item = &'a PromptTemplate> + use<'a, 'n> {
        self.projects
            .iter()
            .filter(move |entry| entry.project_id == project)
            .flat_map(|entry| entry.templates.iter())
            .filter(move |row| row.name == name)
    }
}

/// One read of the scope: `Backend::prompt_templates` for each id of `scope.project_ids`, in that
/// order. N reads on purpose, per event and never per keystroke, as [`crate::prompt_settings`]
/// does. Unlike that module, an id that names no project yields an empty entry rather than being
/// skipped: `prompt_templates` answers an empty list for it, and a second read to tell the two
/// apart would buy the view nothing.
///
/// # Errors
/// Whatever the backend reports; offline, [`StoreError::Unreachable`] with
/// [`htui_store::PROMPT_ON_SERVER_ONLY`].
pub async fn snapshot(backend: &Backend, scope: &Scope) -> Result<TemplatesSnapshot> {
    let mut projects = Vec::with_capacity(scope.project_ids.len());
    for id in &scope.project_ids {
        projects.push(ProjectTemplates {
            project_id: *id,
            templates: backend.prompt_templates(*id).await?,
        });
    }
    Ok(TemplatesSnapshot { projects })
}

/// The two request names, in [`StoreRequest`] order.
///
/// [`StoreRequest::name`]'s arms and the view's `Failed` match both read from here, so a third
/// request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 2] = ["templates", "save_template"];

/// The **read**'s name: a refused read leaves the view with no tree, where a refused save leaves
/// the editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// Serves one template request, off the UI task.
///
/// # Errors
/// Whatever the seam reports; offline, [`StoreError::Unreachable`] with `PROMPT_ON_SERVER_ONLY`
/// for the read and `DATABASE_UNREACHABLE` for the save; [`StoreError::Backend`] for a request
/// that is not one of this module's two.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        // The read goes through `Backend` rather than a `Writer`: `prompt_templates` is inherent on
        // each store and refuses offline with its own sentence, which is the one the view shows.
        StoreRequest::Templates(scope) => Ok(StoreReply::Templates(Box::new(
            snapshot(backend, scope).await?,
        ))),
        StoreRequest::SaveTemplate {
            scope,
            project,
            name,
            body,
            expected,
        } => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
            let created_by = backend.this_user().await?;
            let outcome = writer
                .append_prompt_template(
                    NewPromptTemplate {
                        id: PromptTemplateId::new(),
                        project_id: *project,
                        name: name.clone(),
                        body: body.as_str().to_owned(),
                        created_by,
                    },
                    *expected,
                )
                .await?;
            // The worker re-reads rather than handing the view the one row the outcome carries:
            // the view renders a tree, and a row patched in locally would be a second source of
            // truth (the `cas` shape of `catalogue.rs` and `prompt_settings.rs`).
            let fresh = Box::new(snapshot(backend, scope).await?);
            Ok(match outcome {
                CasOutcome::Applied(_) => StoreReply::Templates(fresh),
                CasOutcome::Stale(_) => StoreReply::TemplatesStale(fresh),
            })
        }
        // `try_serve` routes exactly this module's two variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a template request: {}",
            other.name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{READ_NAME, REQUEST_NAMES, TemplateBody, TemplatesSnapshot, serve};
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use htui_core::fixtures::ids;
    use htui_core::model::{ProjectId, Scope};
    use htui_core::store::{MemStore, StoreError};
    use htui_store::{Backend, CacheStore, PROMPT_ON_SERVER_ONLY};

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

    async fn read(backend: &Backend, scope: &Scope) -> TemplatesSnapshot {
        match serve(backend, &StoreRequest::Templates(scope.clone())).await {
            Ok(StoreReply::Templates(snapshot)) => *snapshot,
            other => panic!("the read answered {other:?}"),
        }
    }

    fn save(
        scope: &Scope,
        project: ProjectId,
        name: &str,
        body: &str,
        expected: Option<i32>,
    ) -> StoreRequest {
        StoreRequest::SaveTemplate {
            scope: scope.clone(),
            project,
            name: name.to_owned(),
            body: TemplateBody::new(body),
            expected,
        }
    }

    #[tokio::test]
    async fn the_read_answers_every_scope_project_in_scope_order() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        assert_eq!(scope.project_ids, vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);

        let snapshot = read(&backend, &scope).await;

        let order: Vec<ProjectId> = snapshot.projects.iter().map(|p| p.project_id).collect();
        assert_eq!(
            order, scope.project_ids,
            "one entry per scope project, in scope order"
        );
        for project in &snapshot.projects {
            assert_eq!(
                project.templates.len(),
                10,
                "one version of each default name"
            );
            assert!(
                project
                    .templates
                    .iter()
                    .all(|row| row.project_id == project.project_id),
                "a project's entry holds its own rows only"
            );
            let keys: Vec<(&[u8], i32)> = project
                .templates
                .iter()
                .map(|row| (row.name.as_bytes(), row.version))
                .collect();
            let mut sorted = keys.clone();
            sorted.sort_unstable();
            assert_eq!(
                keys, sorted,
                "(name, version) byte order, as the read returns it"
            );
            let implement = snapshot
                .head(project.project_id, "implement")
                .expect("every project holds `implement`");
            assert_eq!(implement.version, 1);
            assert_eq!(
                snapshot.version(project.project_id, "implement", 1),
                Some(implement)
            );
        }
        assert_eq!(snapshot.head(ids::PROJECT_HTUI, "no-such-name"), None);
        assert_eq!(snapshot.head(ids::PROJECT_VULKAN, "implement"), None);
    }

    /// `StoreRequest` derives `Debug`; a body is user text, so the save prints its length only
    /// (`ExternalEdit`'s and `TextArea`'s rule).
    #[test]
    fn a_save_request_debug_prints_the_body_length_not_the_body() {
        let scope = Scope {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_ids: vec![ids::PROJECT_VULKAN],
        };
        let request = save(
            &scope,
            ids::PROJECT_VULKAN,
            "implement",
            "secret {{item}}",
            Some(1),
        );
        let shown = format!("{request:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("len: 15"), "{shown}");
        assert!(
            shown.contains("implement"),
            "the name is not user text: {shown}"
        );
    }

    #[tokio::test]
    async fn a_save_at_the_head_answers_templates_with_the_new_version() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        let v1 = before
            .head(ids::PROJECT_HTUI, "implement")
            .expect("the fixture holds `implement`")
            .clone();
        let body = format!("{}\nOne more line.\n", v1.body);

        let reply = serve(
            &backend,
            &save(&scope, ids::PROJECT_HTUI, "implement", &body, Some(1)),
        )
        .await;

        let Ok(StoreReply::Templates(after)) = reply else {
            panic!("an applied save answers `Templates`, got {reply:?}")
        };
        let head = after
            .head(ids::PROJECT_HTUI, "implement")
            .expect("the new head is in the fresh snapshot");
        assert_eq!(head.version, 2);
        assert_eq!(head.body, body);
        assert_eq!(head.created_by, ids::USER, "the worker fills `created_by`");
        assert_eq!(
            after.version(ids::PROJECT_HTUI, "implement", 1),
            Some(&v1),
            "append-only: v1 is untouched"
        );
        assert_eq!(
            after
                .projects
                .iter()
                .map(|p| p.project_id)
                .collect::<Vec<_>>(),
            scope.project_ids,
            "the reply re-reads the whole scope"
        );
    }

    #[tokio::test]
    async fn a_save_at_a_stale_head_answers_templates_stale_and_writes_nothing() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let first = serve(
            &backend,
            &save(
                &scope,
                ids::PROJECT_HTUI,
                "implement",
                "first {{item_key}}\n",
                Some(1),
            ),
        )
        .await;
        assert!(
            matches!(first, Ok(StoreReply::Templates(_))),
            "the first save applies: {first:?}"
        );
        let before = read(&backend, &scope).await;

        let reply = serve(
            &backend,
            &save(
                &scope,
                ids::PROJECT_HTUI,
                "implement",
                "second {{item_key}}\n",
                Some(1),
            ),
        )
        .await;

        let Ok(StoreReply::TemplatesStale(fresh)) = reply else {
            panic!("a spent token answers `TemplatesStale`, got {reply:?}")
        };
        assert_eq!(
            *fresh, before,
            "the stale reply is the store as it is, unchanged"
        );
        let head = fresh
            .head(ids::PROJECT_HTUI, "implement")
            .expect("the head is still there");
        assert_eq!(head.version, 2);
        assert_eq!(head.body, "first {{item_key}}\n");
    }

    #[tokio::test]
    async fn a_refused_body_answers_failed_with_the_parse_message() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;

        let reply = store_worker::serve(
            &backend,
            &save(
                &scope,
                ids::PROJECT_HTUI,
                "implement",
                "Work on {{itme}}.\n",
                Some(1),
            ),
        )
        .await;

        let StoreReply::Failed { request, message } = reply else {
            panic!("a refused body answers `Failed`, got {reply:?}")
        };
        assert_eq!(request, "save_template");
        assert!(
            message.contains("unknown prompt placeholder"),
            "the parse message reaches the view: {message}"
        );
        assert_eq!(read(&backend, &scope).await, before, "nothing is written");
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        assert_eq!(READ_NAME, REQUEST_NAMES[0]);
        let samples = [
            StoreRequest::Templates(scope.clone()),
            save(&scope, ids::PROJECT_HTUI, "implement", "", None),
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
    }

    #[tokio::test]
    async fn an_offline_read_is_refused_with_the_server_only_sentence() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "templates-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline { cache, since: None };
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };

        let reply = serve(&backend, &StoreRequest::Templates(scope)).await;

        assert_eq!(
            reply.err(),
            Some(StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))
        );
    }
}
