//! The skill library behind the Skills tab's library and attachment panes (MOD-9 milestone 3,
//! D81): one snapshot read per scope and the four compare-and-set writes, the
//! [`crate::templates`] shape.
//!
//! One read per event and never one per keystroke, one reply out. Every write re-reads the whole
//! snapshot rather than handing the view the row its outcome carries: the view renders from the
//! snapshot and never patches a row into it. A write that missed its token, or whose skill,
//! project or phase is gone, answers [`StoreReply::SkillsStale`] naming which write it was, so the
//! view knows which draft keeps its text ([`crate::box_settings`]' rule for a vanished box).
//!
//! Known residue, the one [`crate::templates`] and [`crate::prompt_settings`] carry: a re-read
//! that fails *after* an applied write answers `Failed`, so the view is told nothing happened
//! when the write has in fact landed.
//!
//! The worker fills `created_by` from [`Backend::this_user`] and mints the new skill's id; the
//! render side never holds a `UserId` (`R-NF-3`). Nothing here reads the clock: the store stamps
//! every instant.

use htui_core::model::{
    NewSkill, NewSkillVersion, ProjectId, Scope, Skill, SkillBinding, SkillBindingKey, SkillId,
    SkillVersion, StepGraph, StepGraphPhase,
};
use htui_core::store::{CasOutcome, Result, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE, PROMPT_ON_SERVER_ONLY, Writer};

use crate::store_worker::{StoreReply, StoreRequest};

/// The whole Skills view in one read (D81): the library, the global attachments, and each scope
/// project's attachments, graphs and repo names.
///
/// `PartialEq` only: [`StepGraph`] derives no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillsSnapshot {
    /// The library in `name` byte order, each skill with every version.
    pub skills: Vec<SkillEntry>,
    /// The global attachments (`skill_bindings(None)`), in D91 order.
    pub global: Vec<SkillBinding>,
    /// One entry per id of `scope.project_ids`, in that order.
    pub projects: Vec<ProjectSkills>,
}

/// One skill and its versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    /// The `skill` row.
    pub skill: Skill,
    /// Every version, ascending (R-27: the library is small, so all of them travel).
    pub versions: Vec<SkillVersion>,
}

/// One scope project's side of the attachment pane.
///
/// `PartialEq` only: [`StepGraph`] and [`StepGraphPhase`] derive no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSkills {
    /// Which project.
    pub project: ProjectId,
    /// Its project and phase attachments, `(skill_id, phase_id)` byte order with the project row
    /// first (D91).
    pub bindings: Vec<SkillBinding>,
    /// Its graphs in `name` byte order, **override graphs excluded** (a per-item clone is not
    /// where an attachment is authored), each with its phases by `position`.
    pub graphs: Vec<(StepGraph, Vec<StepGraphPhase>)>,
    /// Its repo names, byte order: D79's repo picker and the marks on a glob naming no repo.
    pub repos: Vec<String>,
}

impl SkillsSnapshot {
    /// One skill's entry; `None` when the library holds no such skill.
    #[must_use]
    pub fn entry(&self, skill: SkillId) -> Option<&SkillEntry> {
        self.skills.iter().find(|entry| entry.skill.id == skill)
    }

    /// The skill named `name`.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&SkillEntry> {
        self.skills.iter().find(|entry| entry.skill.name == name)
    }

    /// A skill's highest version: the compare-and-set token a save of that skill passes. `None`
    /// for an unknown skill and for one with no version row.
    #[must_use]
    pub fn head(&self, skill: SkillId) -> Option<&SkillVersion> {
        self.entry(skill)?
            .versions
            .iter()
            .max_by_key(|row| row.version)
    }

    /// One version of a skill.
    #[must_use]
    pub fn version(&self, skill: SkillId, version: i32) -> Option<&SkillVersion> {
        self.entry(skill)?
            .versions
            .iter()
            .find(|row| row.version == version)
    }

    /// A scope project's entry; `None` for a project outside the scope.
    #[must_use]
    pub fn project(&self, project: ProjectId) -> Option<&ProjectSkills> {
        self.projects.iter().find(|entry| entry.project == project)
    }

    /// The row at `key`: from [`global`](Self::global) when `key.project` is `None`, otherwise
    /// from that project's `bindings`. `None` when no row sits there, which is also the
    /// `expected: None` a new attachment passes.
    #[must_use]
    pub fn binding(&self, key: SkillBindingKey) -> Option<&SkillBinding> {
        let rows = match key.project {
            None => &self.global,
            Some(project) => &self.project(project)?.bindings,
        };
        rows.iter().find(|row| SkillBindingKey::of(row) == key)
    }
}

/// Which write a [`StoreReply::SkillsStale`] answers (D81), so the view knows which draft keeps
/// its text and which row to reload against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleWhat {
    /// [`StoreRequest::EditSkill`]: the row changed since the form opened, or is gone.
    Skill(SkillId),
    /// [`StoreRequest::SaveSkillVersion`]: the head moved, or the skill is gone.
    Version(SkillId),
    /// [`StoreRequest::SetSkillBinding`]: the row at the key changed, appeared or went, or its
    /// skill, project or phase is gone.
    Binding(SkillBindingKey),
}

/// One read (D81, D97), through the writer: `skill_bindings(None)`; then per id of
/// `scope.project_ids`, in that order, `skill_bindings(Some(p))`, `step_graphs(p)` without the
/// override graphs, `phases(g)` per graph and `repos(p)`'s names; then `skills()` and
/// `skill_versions(s)` per skill.
///
/// Attachments are read **before** versions (the milestone 2 review's rule): a write landing
/// between the two can then only add a version no attachment names yet, never leave a pin naming
/// a version the read missed. N+1 reads on purpose, per event and never per keystroke (R-27,
/// R-35).
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot(writer: &Writer, scope: &Scope) -> Result<SkillsSnapshot> {
    let global = writer.skill_bindings(None).await?;
    let mut projects = Vec::with_capacity(scope.project_ids.len());
    for project in &scope.project_ids {
        let bindings = writer.skill_bindings(Some(*project)).await?;
        let mut graphs = Vec::new();
        for graph in writer.step_graphs(*project).await? {
            if graph.is_override {
                continue;
            }
            let phases = writer.phases(graph.id).await?;
            graphs.push((graph, phases));
        }
        let repos = writer
            .repos(*project)
            .await?
            .into_iter()
            .map(|repo| repo.name)
            .collect();
        projects.push(ProjectSkills {
            project: *project,
            bindings,
            graphs,
            repos,
        });
    }
    let mut skills = Vec::new();
    for skill in writer.skills().await? {
        let versions = writer.skill_versions(skill.id).await?;
        skills.push(SkillEntry { skill, versions });
    }
    Ok(SkillsSnapshot {
        skills,
        global,
        projects,
    })
}

/// The five request names, in [`StoreRequest`] order.
///
/// The view's `Failed` match reads from here. [`StoreRequest::name`]'s arms spell the same five as
/// literals, and the `request_names_match_the_name_arms` test pins them to this list, so a name
/// changed in one place and not the other fails there.
pub const REQUEST_NAMES: [&str; 5] = [
    "skills",
    "create_skill",
    "edit_skill",
    "save_skill_version",
    "set_skill_binding",
];

/// The **read**'s name: a refused read leaves the view with no library, where a refused write
/// leaves the form or editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// Serves one skill request, off the UI task (D97).
///
/// The read answers [`StoreReply::Skills`]. A write answers `Skills` when it applied and
/// [`StoreReply::SkillsStale`] when its token was spent or the row it names is gone:
/// `NotFound` of the skill for `EditSkill`; of the skill or `skill_version` for
/// `SaveSkillVersion`; of the skill, project or `step_graph_phase` for `SetSkillBinding`.
/// Every other refusal stays an error, so [`crate::store_worker::serve`] answers `Failed` with the
/// store's sentence.
///
/// # Errors
/// Whatever the seam reports; offline, [`StoreError::Unreachable`] with `PROMPT_ON_SERVER_ONLY`
/// for the read (skills are not mirrored, the Templates read's sentence) and
/// `DATABASE_UNREACHABLE` for the four writes; [`StoreError::Backend`] for a request that is not
/// one of this module's five.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        StoreRequest::Skills(scope) => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))?;
            Ok(StoreReply::Skills(Box::new(
                snapshot(&writer, scope).await?,
            )))
        }
        StoreRequest::CreateSkill {
            scope,
            name,
            description,
            body,
        } => {
            let writer = write_access(backend)?;
            let created_by = backend.this_user().await?;
            // A refusal (a bad name, a blank body, a taken name) stays an error: there is no token
            // to have spent, so it is `Failed` with the store's sentence, never `SkillsStale`.
            writer
                .create_skill(NewSkill {
                    id: SkillId::new(),
                    name: name.clone(),
                    description: description.clone(),
                    body: body.as_str().to_owned(),
                    source: empty_source(),
                    created_by,
                })
                .await?;
            answer(&writer, scope, None).await
        }
        StoreRequest::EditSkill {
            scope,
            skill,
            expected,
            patch,
        } => {
            let writer = write_access(backend)?;
            let stale = match writer.update_skill(*skill, *expected, patch.clone()).await {
                Ok(CasOutcome::Applied(_)) => None,
                Ok(CasOutcome::Stale(_))
                | Err(StoreError::NotFound {
                    entity: "skill", ..
                }) => Some(StaleWhat::Skill(*skill)),
                Err(other) => return Err(other),
            };
            answer(&writer, scope, stale).await
        }
        StoreRequest::SaveSkillVersion {
            scope,
            skill,
            expected,
            body,
        } => {
            let writer = write_access(backend)?;
            let created_by = backend.this_user().await?;
            let new = NewSkillVersion {
                body: body.as_str().to_owned(),
                source: empty_source(),
                created_by,
            };
            let stale = match writer.add_skill_version(*skill, *expected, new).await {
                Ok(CasOutcome::Applied(_)) => None,
                Ok(CasOutcome::Stale(_))
                | Err(StoreError::NotFound {
                    entity: "skill" | "skill_version",
                    ..
                }) => Some(StaleWhat::Version(*skill)),
                Err(other) => return Err(other),
            };
            answer(&writer, scope, stale).await
        }
        StoreRequest::SetSkillBinding {
            scope,
            key,
            expected,
            change,
        } => {
            let writer = write_access(backend)?;
            let stale = match writer
                .set_skill_binding(*key, *expected, change.clone())
                .await
            {
                Ok(CasOutcome::Applied(_)) => None,
                Ok(CasOutcome::Stale(_))
                | Err(StoreError::NotFound {
                    entity: "skill" | "project" | "step_graph_phase",
                    ..
                }) => Some(StaleWhat::Binding(*key)),
                Err(other) => return Err(other),
            };
            answer(&writer, scope, stale).await
        }
        // `try_serve` routes exactly this module's five variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a skills request: {}",
            other.name()
        ))),
    }
}

/// The writer, or the refusal every skill write answers offline.
fn write_access(backend: &Backend) -> Result<Writer> {
    backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))
}

/// `skill_version.source` for a version written from the view: `{}` (D77, D87).
fn empty_source() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// The re-read every write answers with: `Skills` when `stale` is `None`, otherwise `SkillsStale`
/// naming the write.
async fn answer(writer: &Writer, scope: &Scope, stale: Option<StaleWhat>) -> Result<StoreReply> {
    let fresh = Box::new(snapshot(writer, scope).await?);
    Ok(match stale {
        None => StoreReply::Skills(fresh),
        Some(what) => StoreReply::SkillsStale {
            snapshot: fresh,
            what,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{READ_NAME, REQUEST_NAMES, SkillsSnapshot, StaleWhat, serve};
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use crate::templates::TemplateBody;
    use chrono::{DateTime, Duration, Utc};
    use htui_core::fixtures::{self, ids};
    use htui_core::model::{
        Activation, Attachment, BindingChange, NewRepo, PhaseId, RepoId, Scope, SkillBindingKey,
        SkillId, SkillPatch, StepGraph, StepGraphId,
    };
    use htui_core::store::{MemStore, StoreError, WriteStore as _};
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

    /// The snapshot of a `Skills` reply, or a panic naming what came back instead.
    async fn applied(backend: &Backend, request: &StoreRequest) -> SkillsSnapshot {
        match serve(backend, request).await {
            Ok(StoreReply::Skills(snapshot)) => *snapshot,
            other => panic!("{} answered {other:?}", request.name()),
        }
    }

    /// The snapshot and `what` of a `SkillsStale` reply, or a panic naming what came back.
    async fn stale(backend: &Backend, request: &StoreRequest) -> (SkillsSnapshot, StaleWhat) {
        match serve(backend, request).await {
            Ok(StoreReply::SkillsStale { snapshot, what }) => (*snapshot, what),
            other => panic!("{} answered {other:?}", request.name()),
        }
    }

    fn always() -> Attachment {
        Attachment {
            pinned_version: None,
            position: 0,
            activation: Activation::Always,
            globs: Vec::new(),
            languages: Vec::new(),
        }
    }

    fn create(scope: &Scope, name: &str, body: &str) -> StoreRequest {
        StoreRequest::CreateSkill {
            scope: scope.clone(),
            name: name.to_owned(),
            description: format!("{name}, briefly"),
            body: TemplateBody::new(body),
        }
    }

    fn edit(
        scope: &Scope,
        skill: SkillId,
        expected: DateTime<Utc>,
        description: &str,
    ) -> StoreRequest {
        StoreRequest::EditSkill {
            scope: scope.clone(),
            skill,
            expected,
            patch: SkillPatch {
                name: None,
                description: Some(description.to_owned()),
            },
        }
    }

    fn save(scope: &Scope, skill: SkillId, expected: i32, body: &str) -> StoreRequest {
        StoreRequest::SaveSkillVersion {
            scope: scope.clone(),
            skill,
            expected,
            body: TemplateBody::new(body),
        }
    }

    fn bind(
        scope: &Scope,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
    ) -> StoreRequest {
        StoreRequest::SetSkillBinding {
            scope: scope.clone(),
            key,
            expected,
            change,
        }
    }

    /// The demo's phase-level attachment: `rust-style` on `htui`'s `implement` phase.
    fn demo_phase_key() -> SkillBindingKey {
        SkillBindingKey {
            skill: ids::SKILL_RUST_STYLE,
            project: Some(ids::PROJECT_HTUI),
            phase: Some(ids::PHASE_HTUI_IMPLEMENT),
        }
    }

    /// F-K: the demo with one override graph on `htui` and repo `core`, so the snapshot has an
    /// override to leave out and a repo name to carry.
    #[tokio::test]
    async fn the_snapshot_carries_the_library_the_attachments_and_the_scope_s_graphs() {
        let mut data = fixtures::demo_data();
        data.graphs.push(StepGraph {
            id: StepGraphId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "FEAT-1-override".to_owned(),
            description: String::new(),
            is_override: true,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        });
        let store = MemStore::from_demo(data);
        store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "core".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the demo takes a repo on `htui`");
        let backend = Backend::memory(store);
        let scope = platform_scope(&backend).await;
        assert_eq!(scope.project_ids, vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);

        let snapshot = read(&backend, &scope).await;

        let library: Vec<(&str, Vec<i32>)> = snapshot
            .skills
            .iter()
            .map(|entry| {
                (
                    entry.skill.name.as_str(),
                    entry.versions.iter().map(|row| row.version).collect(),
                )
            })
            .collect();
        assert_eq!(
            library,
            vec![("rust-style", vec![1, 2]), ("tests", vec![1])],
            "the library in name order, each with every version ascending"
        );
        assert!(
            snapshot.global.is_empty(),
            "the demo attaches nothing globally"
        );
        assert_eq!(
            snapshot
                .projects
                .iter()
                .map(|p| p.project)
                .collect::<Vec<_>>(),
            scope.project_ids,
            "one entry per scope project, in scope order"
        );

        let htui = snapshot
            .project(ids::PROJECT_HTUI)
            .expect("htui is in the scope");
        assert_eq!(
            htui.bindings.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![
                ids::BINDING_HTUI_RUST_STYLE,
                ids::BINDING_HTUI_IMPLEMENT_RUST_STYLE,
                ids::BINDING_HTUI_TESTS,
            ],
            "the three demo rows, (skill_id, phase_id) order with the project row first (D91)"
        );
        let graphs: Vec<&str> = htui
            .graphs
            .iter()
            .map(|(graph, _)| graph.name.as_str())
            .collect();
        assert_eq!(
            graphs,
            ["analysis", "bug", "feature", "refactor", "tooling"],
            "name order, the override graph left out"
        );
        assert_eq!(
            htui.graphs
                .iter()
                .map(|(_, phases)| phases.len())
                .sum::<usize>(),
            15
        );
        for (graph, phases) in &htui.graphs {
            assert!(phases.iter().all(|phase| phase.graph_id == graph.id));
            assert!(
                phases.windows(2).all(|w| w[0].position < w[1].position),
                "{}'s phases by position",
                graph.name
            );
        }
        assert_eq!(htui.repos, ["core"]);

        let agy = snapshot
            .project(ids::PROJECT_AGY)
            .expect("agy is in the scope");
        assert!(agy.bindings.is_empty());
        assert!(agy.repos.is_empty());

        let rust_style = snapshot
            .by_name("rust-style")
            .expect("the demo holds rust-style");
        assert_eq!(rust_style.skill.id, ids::SKILL_RUST_STYLE);
        assert_eq!(snapshot.entry(ids::SKILL_RUST_STYLE), Some(rust_style));
        assert_eq!(
            snapshot.head(ids::SKILL_RUST_STYLE).map(|row| row.version),
            Some(2)
        );
        assert_eq!(
            snapshot
                .version(ids::SKILL_RUST_STYLE, 1)
                .map(|row| row.version),
            Some(1)
        );
        assert_eq!(snapshot.head(SkillId::new()), None);
        assert_eq!(
            snapshot.binding(demo_phase_key()).map(|row| row.id),
            Some(ids::BINDING_HTUI_IMPLEMENT_RUST_STYLE)
        );
        assert_eq!(
            snapshot.binding(SkillBindingKey {
                project: None,
                phase: None,
                ..demo_phase_key()
            }),
            None,
            "no global row"
        );
        assert_eq!(snapshot.project(ids::PROJECT_VULKAN), None);
    }

    #[tokio::test]
    async fn each_write_answers_skills_when_it_applies() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let created = applied(
            &backend,
            &create(&scope, "docs-style", "Write the doc first.\n"),
        )
        .await;
        let entry = created
            .by_name("docs-style")
            .expect("the created skill is in the fresh snapshot")
            .clone();
        assert_eq!(
            entry
                .versions
                .iter()
                .map(|row| (row.version, row.body.as_str()))
                .collect::<Vec<_>>(),
            [(1, "Write the doc first.\n")]
        );
        assert_eq!(
            entry.skill.created_by,
            ids::USER,
            "the worker fills `created_by`"
        );
        assert_eq!(entry.versions[0].created_by, ids::USER);
        assert_eq!(entry.versions[0].source, serde_json::json!({}));
        let id = entry.skill.id;

        let edited = applied(
            &backend,
            &edit(&scope, id, entry.skill.updated_at, "Docs, then code."),
        )
        .await;
        assert_eq!(
            edited.entry(id).map(|e| e.skill.description.as_str()),
            Some("Docs, then code.")
        );

        let saved = applied(
            &backend,
            &save(&scope, id, 1, "Write the doc first, always.\n"),
        )
        .await;
        let head = saved.head(id).expect("the skill has a head");
        assert_eq!(head.version, 2);
        assert_eq!(head.body, "Write the doc first, always.\n");
        assert_eq!(head.created_by, ids::USER);

        let key = SkillBindingKey {
            skill: id,
            project: None,
            phase: None,
        };
        let attached = applied(
            &backend,
            &bind(&scope, key, None, BindingChange::Attach(always())),
        )
        .await;
        let row = attached
            .binding(key)
            .expect("the global attachment is in the fresh snapshot")
            .clone();
        assert_eq!(attached.global, std::slice::from_ref(&row));

        let detached = applied(
            &backend,
            &bind(&scope, key, Some(row.updated_at), BindingChange::Detach),
        )
        .await;
        assert!(detached.global.is_empty(), "the detach re-reads without it");
    }

    #[tokio::test]
    async fn a_spent_token_answers_skills_stale_with_what_went_stale() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        let rust_style = before
            .entry(ids::SKILL_RUST_STYLE)
            .expect("the demo holds rust-style")
            .skill
            .clone();

        let (fresh, what) = stale(
            &backend,
            &edit(
                &scope,
                rust_style.id,
                rust_style.updated_at - Duration::seconds(1),
                "never stored",
            ),
        )
        .await;
        assert_eq!(what, StaleWhat::Skill(rust_style.id));
        assert_eq!(fresh, before, "a stale edit writes nothing");

        let (fresh, what) =
            stale(&backend, &save(&scope, rust_style.id, 1, "never stored\n")).await;
        assert_eq!(what, StaleWhat::Version(rust_style.id), "the head is 2");
        assert_eq!(fresh, before);

        let key = demo_phase_key();
        let (fresh, what) = stale(
            &backend,
            &bind(&scope, key, None, BindingChange::Attach(always())),
        )
        .await;
        assert_eq!(
            what,
            StaleWhat::Binding(key),
            "a row already sits at the key"
        );
        assert_eq!(fresh, before);

        let gone = SkillId::new();
        let (fresh, what) = stale(&backend, &edit(&scope, gone, rust_style.updated_at, "x")).await;
        assert_eq!(
            what,
            StaleWhat::Skill(gone),
            "a skill gone is a miss like a spent token"
        );
        assert_eq!(fresh, before);

        let (fresh, what) = stale(&backend, &save(&scope, gone, 0, "never stored\n")).await;
        assert_eq!(what, StaleWhat::Version(gone));
        assert_eq!(fresh, before);

        let no_phase = SkillBindingKey {
            phase: Some(PhaseId::new()),
            ..key
        };
        let (fresh, what) = stale(
            &backend,
            &bind(&scope, no_phase, None, BindingChange::Attach(always())),
        )
        .await;
        assert_eq!(what, StaleWhat::Binding(no_phase), "a phase gone is a miss");
        assert_eq!(fresh, before);
    }

    #[tokio::test]
    async fn a_refused_write_answers_failed_with_the_store_sentence() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;

        let reply = store_worker::serve(&backend, &create(&scope, "Bad", "A body.\n")).await;

        let StoreReply::Failed { request, message } = reply else {
            panic!("a refused name answers `Failed`, got {reply:?}")
        };
        assert_eq!(request, "create_skill");
        assert!(
            message.contains("skill.name"),
            "the store's sentence reaches the view: {message}"
        );
        assert_eq!(read(&backend, &scope).await, before, "nothing is written");
    }

    async fn offline() -> (tempfile::TempDir, Backend) {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "skills-offline", 1)
            .await
            .expect("open a throwaway mirror");
        (root, Backend::Offline { cache, since: None })
    }

    fn htui_scope() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        }
    }

    /// Skills are not mirrored, so offline there is nothing to read: the Templates read's
    /// sentence, and `Failed` for `skills` through the worker.
    #[tokio::test]
    async fn an_offline_read_is_refused_with_the_server_only_sentence() {
        let (_root, backend) = offline().await;
        let request = StoreRequest::Skills(htui_scope());

        assert_eq!(
            serve(&backend, &request).await.err(),
            Some(StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))
        );
        match store_worker::serve(&backend, &request).await {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, READ_NAME);
                assert!(message.contains(PROMPT_ON_SERVER_ONLY), "{message}");
            }
            other => panic!("an offline read is refused, not {other:?}"),
        }
    }

    /// Offline there is no writer, so each of the four writes is refused before anything is sent,
    /// with the unreachable-database sentence, under its own name.
    #[tokio::test]
    async fn an_offline_write_is_refused_with_the_unreachable_sentence() {
        let (_root, backend) = offline().await;
        let scope = htui_scope();
        let writes = [
            create(&scope, "docs-style", "A body.\n"),
            edit(&scope, ids::SKILL_RUST_STYLE, DateTime::UNIX_EPOCH, "x"),
            save(&scope, ids::SKILL_RUST_STYLE, 2, "A body.\n"),
            bind(
                &scope,
                demo_phase_key(),
                Some(DateTime::UNIX_EPOCH),
                BindingChange::Detach,
            ),
        ];

        for (write, name) in writes.iter().zip(&REQUEST_NAMES[1..]) {
            assert_eq!(
                serve(&backend, write).await.err(),
                Some(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())),
                "{name}"
            );
            match store_worker::serve(&backend, write).await {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, *name);
                    assert!(message.contains(DATABASE_UNREACHABLE), "{message}");
                }
                other => panic!("an offline {name} is refused, not {other:?}"),
            }
        }
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let scope = htui_scope();
        assert_eq!(READ_NAME, REQUEST_NAMES[0]);
        let samples = [
            StoreRequest::Skills(scope.clone()),
            create(&scope, "docs-style", ""),
            edit(&scope, ids::SKILL_TESTS, DateTime::UNIX_EPOCH, ""),
            save(&scope, ids::SKILL_TESTS, 1, ""),
            bind(&scope, demo_phase_key(), None, BindingChange::Detach),
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
    }

    /// `StoreRequest` derives `Debug`; a skill body is user text, so a create and a save print
    /// its length only (`TemplateBody`'s rule).
    #[test]
    fn a_skill_body_request_debug_prints_its_length_not_its_text() {
        let scope = htui_scope();
        for request in [
            save(&scope, ids::SKILL_TESTS, 1, "secret rule"),
            create(&scope, "docs-style", "secret rule"),
        ] {
            let shown = format!("{request:?}");
            assert!(!shown.contains("secret"), "{shown}");
            assert!(shown.contains("len: 11"), "{shown}");
        }
    }

    /// A request that is not one of the five is refused by name rather than served.
    #[tokio::test]
    async fn a_foreign_request_is_refused_by_name() {
        let backend = demo();
        assert_eq!(
            serve(&backend, &StoreRequest::Workspaces).await.err(),
            Some(StoreError::Backend(
                "not a skills request: workspaces".to_owned()
            ))
        );
    }
}
