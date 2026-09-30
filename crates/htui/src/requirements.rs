//! The requirements behind the Requirements tab and the item detail's Reqs sub-tab (MOD-39 plan
//! P1): one snapshot read per scope, one detail read per requirement, one citation read per item,
//! and the seven writes, the [`crate::templates`] shape.
//!
//! One read per event and never one per keystroke, one reply out. Every write re-reads rather than
//! handing the view the row its outcome carries: a tab write that applied answers
//! [`StoreReply::RequirementWritten`], a fresh [`RequirementsSnapshot`] beside what the store wrote
//! (MOD-59), so the tab lands it on its own reply and never by searching the snapshot; a citation
//! write answers a fresh [`ItemCitations`]. A re-read that fails after a tab write applied still
//! answers `RequirementWritten`, carrying the failure, because the write has landed. An amend or
//! withdraw that missed its version answers [`StoreReply::RequirementsStale`] (plan P3); every
//! other refusal stays an error, so [`crate::store_worker::serve`] answers `Failed` with the
//! store's sentence.
//!
//! **The maintainer gate** (PRD D1, plan P5): area create, mint, amend and withdraw are the
//! project's requirements owner's (`requirement_spec.owner_id`). A project with no spec is
//! anyone's until the first gated write claims it. Every input check runs *before* the gate and
//! the gate is the last step before the write (MOD-39 blueprint F-9), so a write refused for its
//! input never claims a spec. The store refusals the worker can see coming are checked there too:
//! an area code the project already has, and an amend or withdraw of a requirement withdrawn at
//! the version it names. What only the store decides (an author or box row gone since the worker
//! read it, a racing write) still refuses after the gate, and the claimed spec stays.
//!
//! Cite, uncite and re-confirm are not gated (ANA-11 §6). A cite is PRD D4's: `addresses` or
//! `reserves`, of a requirement of the item's own project. `amends` and `withdraws` are the
//! decisions an amend or withdraw records, which plan P9 never uncites, so a hand-made one would
//! be a decision nobody could take back and that no maintainer made.
//!
//! **The deciding item** of an amend or withdraw is typed as a key (plan P6) and looked up in the
//! requirement's project only, matched exactly after `trim` and ASCII upper-casing: the store's
//! text filter is a substring of key or title, so `ANA-1` would also find `ANA-10` (blueprint
//! F-11). An unknown key is refused before any write.
//!
//! Known residue, the citation writes' alone (MOD-59 D6): a re-read that fails *after* an applied
//! cite, uncite or re-confirm answers `Failed`, so the Reqs sub-tab is told nothing happened when
//! the write has in fact landed.
//!
//! The worker fills the author and the box from [`Backend::this_user`] and [`Backend::box_info`];
//! the render side never holds a `UserId` (`R-NF-3`). Nothing here reads the clock: the store
//! stamps every instant.

use htui_core::model::{
    CitationKind, CoverageRow, ItemCitation, ItemFilter, ItemId, NewRequirement,
    NewRequirementArea, ProjectId, Requirement, RequirementArea, RequirementAreaId,
    RequirementFilter, RequirementId, RequirementPatch, RequirementRevision, RequirementSpec,
    RequirementState, RequirementUpdate, Scope, UserId,
};
use htui_core::store::{
    CasOutcome, ReadStore as _, Result, StoreError, WriteStore as _, already_exists,
    invalid_area_code, requirement_withdrawn,
};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::store_worker::{StoreReply, StoreRequest};

/// A body or rationale on its way to the store. `StoreRequest` derives `Debug`, and this is user
/// prose, so it prints its length only ([`crate::templates::TemplateBody`]'s rule).
#[derive(Clone, PartialEq, Eq)]
pub struct RequirementText(String);

impl RequirementText {
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

impl core::fmt::Debug for RequirementText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RequirementText")
            .field("len", &self.0.len())
            .finish()
    }
}

/// The whole tab in one read (plan P4, blueprint F-6).
///
/// `PartialEq` only: [`Requirement`] derives no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct RequirementsSnapshot {
    /// One entry per id of `scope.project_ids`, in that order.
    pub projects: Vec<ProjectRequirements>,
    /// Whether the backend has a writer: `false` offline, where every write key answers
    /// `DATABASE_UNREACHABLE` without a request.
    pub writable: bool,
}

/// One scope project's side of the tab.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRequirements {
    /// Which project.
    pub project_id: ProjectId,
    /// The spec header; `None` until the first gated write claims it (PRD D1).
    pub spec: Option<RequirementSpec>,
    /// The project's areas, `(position, code)` order.
    pub areas: Vec<RequirementArea>,
    /// Every requirement of the project, every state, `(area_code, number)` order. Unfiltered:
    /// the view filters with [`matches_filter`] (plan P4).
    pub requirements: Vec<Requirement>,
    /// Whether this user may add areas and create, amend or withdraw requirements here: the
    /// spec's owner, or anyone while there is no spec (plan P5). A mirror that never saw this OS
    /// user is maintainer only of a project with no spec (blueprint F-10).
    pub maintainer: bool,
}

impl RequirementsSnapshot {
    /// Whether this snapshot answers `scope` (the same project ids, in order), so a reply for a
    /// scope already left is ignored.
    #[must_use]
    pub fn is_for(&self, scope: &Scope) -> bool {
        self.projects
            .iter()
            .map(|entry| entry.project_id)
            .eq(scope.project_ids.iter().copied())
    }

    /// One project's entry.
    #[must_use]
    pub fn project(&self, id: ProjectId) -> Option<&ProjectRequirements> {
        self.projects.iter().find(|entry| entry.project_id == id)
    }

    /// One requirement, from whichever project holds it.
    #[must_use]
    pub fn requirement(&self, id: RequirementId) -> Option<&Requirement> {
        self.projects
            .iter()
            .flat_map(|entry| entry.requirements.iter())
            .find(|row| row.id == id)
    }

    /// One area, from whichever project holds it.
    #[must_use]
    pub fn area(&self, id: RequirementAreaId) -> Option<&RequirementArea> {
        self.projects
            .iter()
            .flat_map(|entry| entry.areas.iter())
            .find(|row| row.id == id)
    }
}

impl ProjectRequirements {
    /// The area's requirements, in snapshot order.
    pub fn in_area(&self, area: RequirementAreaId) -> impl Iterator<Item = &Requirement> + '_ {
        self.requirements
            .iter()
            .filter(move |row| row.area_id == area)
    }
}

/// Plan P4's client-side filter: `needle`, trimmed, is a case-insensitive substring of the
/// requirement's key or body. An empty needle matches everything.
#[must_use]
pub fn matches_filter(requirement: &Requirement, needle: &str) -> bool {
    let needle = needle.trim();
    if needle.is_empty() {
        return true;
    }
    let needle = needle.to_lowercase();
    requirement.key.to_lowercase().contains(&needle)
        || requirement.body.to_lowercase().contains(&needle)
}

/// One requirement for the detail pane (plan P7).
#[derive(Debug, Clone, PartialEq)]
pub struct RequirementDetail {
    /// The requirement as it is now.
    pub requirement: Requirement,
    /// Its live citations, in the store's order.
    pub coverage: Vec<CoverageRow>,
    /// Its revisions in version order, each with its deciding key; `None` offline (the mirror
    /// holds no revisions), drawn as "revisions need the database".
    pub revisions: Option<Vec<RevisionRow>>,
}

/// One revision and the key of the item that decided it.
#[derive(Debug, Clone, PartialEq)]
pub struct RevisionRow {
    /// The revision.
    pub revision: RequirementRevision,
    /// The key of `amended_by_item_id`; `None` when the revision names no item or the item is
    /// gone.
    pub deciding_key: Option<String>,
}

/// One item's citations for the Reqs sub-tab (plan P8).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemCitations {
    /// Which item.
    pub item: ItemId,
    /// The item's project: where `candidates` come from.
    pub project_id: ProjectId,
    /// The item's live citations, `suspect` derived, in the store's order.
    pub citations: Vec<ItemCitation>,
    /// The project's active requirements: what `c` can cite.
    pub candidates: Vec<Requirement>,
    /// As [`RequirementsSnapshot::writable`].
    pub writable: bool,
}

/// The ten request names, in [`StoreRequest`] order.
///
/// The views' `Failed` matches read from here. [`StoreRequest::name`]'s arms spell the same ten as
/// literals, and the `request_names_match_the_name_arms` test pins them to this list, so a name
/// changed in one place and not the other fails there.
pub const REQUEST_NAMES: [&str; 10] = [
    // Reads: the tab's two, then the sub-tab's.
    "requirements",
    "requirement_detail",
    "item_requirements",
    // The tab's four writes, gated (PRD D1).
    "create_requirement_area",
    "mint_requirement",
    "amend_requirement",
    "withdraw_requirement",
    // The sub-tab's three writes.
    "cite_requirement",
    "uncite_requirement",
    "reconfirm_citation",
];

/// The tab's snapshot read: a refused read leaves the tab with no tree.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// The tab's detail read.
pub const DETAIL_NAME: &str = REQUEST_NAMES[1];

/// The sub-tab's read.
pub const CITATIONS_NAME: &str = REQUEST_NAMES[2];

/// Whether `name` is one of the tab's four writes.
#[must_use]
pub fn is_tab_write(name: &str) -> bool {
    REQUEST_NAMES[3..7].contains(&name)
}

/// Whether `name` is one of the sub-tab's three writes.
#[must_use]
pub fn is_citation_write(name: &str) -> bool {
    REQUEST_NAMES[7..10].contains(&name)
}

/// A mint or amend whose body is blank after `trim`.
pub const BLANK_BODY: &str = "a requirement needs a body";

/// An area whose title is blank after `trim`.
pub const BLANK_AREA_TITLE: &str = "an area needs a title";

/// An amend or withdraw whose deciding key is blank after `trim`.
pub const DECIDING_KEY_NEEDED: &str = "an amend or a withdraw names its deciding item by key";

/// PRD D1's refusal; `project` is the slug. The view says the same sentence before sending.
#[must_use]
pub fn not_the_maintainer(project: &str) -> String {
    format!(
        "only the owner of {project}'s requirements can add areas or create, amend or withdraw \
         requirements"
    )
}

/// Plan P6: no item of that key in the requirement's project.
#[must_use]
pub fn no_deciding_item(key: &str, project: &str) -> String {
    format!("no item {key} in {project}")
}

/// Plan P9: an `amends` or `withdraws` citation records a decision and stays.
#[must_use]
pub fn decision_citation_stays(kind: CitationKind) -> String {
    format!("a `{kind}` citation records a decision; it is not uncited")
}

/// PRD D4: a human cites `addresses` or `reserves`; `amends` and `withdraws` are recorded by the
/// gated amend and withdraw.
#[must_use]
pub fn decision_citation_not_cited(kind: CitationKind) -> String {
    format!("a `{kind}` citation records a decision; an amend or a withdraw makes it, not a cite")
}

/// PRD D4: an item cites a requirement of its own project; `project` is the item's slug.
#[must_use]
pub fn requirement_of_another_project(key: &str, project: &str) -> String {
    format!("{key} is not a requirement of {project}")
}

/// One read of the scope: for each id of `scope.project_ids`, in that order, the spec, the areas
/// and every requirement. N reads on purpose, per event and never per keystroke.
///
/// # Errors
/// Whatever the backend reports.
pub async fn snapshot(backend: &Backend, scope: &Scope) -> Result<RequirementsSnapshot> {
    // A mirror that never synced this OS user answers `NotFound`, and that must not cost the
    // whole offline read (blueprint F-10): such a user owns no spec. Any other failure is the
    // read's, so a broken lookup never passes for "read-only" (MOD-39 review).
    let me = match backend.this_user().await {
        Ok(user) => Some(user),
        Err(StoreError::NotFound { .. }) => None,
        Err(other) => return Err(other),
    };
    let mut projects = Vec::with_capacity(scope.project_ids.len());
    for &project_id in &scope.project_ids {
        let spec = backend.requirement_spec(project_id).await?;
        let maintainer = spec.as_ref().is_none_or(|spec| Some(spec.owner_id) == me);
        projects.push(ProjectRequirements {
            project_id,
            areas: backend.requirement_areas(project_id).await?,
            requirements: backend
                .requirements(project_id, &RequirementFilter::default())
                .await?,
            spec,
            maintainer,
        });
    }
    Ok(RequirementsSnapshot {
        projects,
        writable: backend.writer().is_some(),
    })
}

/// One requirement with its coverage and revisions (plan P7).
///
/// # Errors
/// [`StoreError::NotFound`] `{ entity: "requirement" }` for an unknown id; whatever else the
/// backend reports.
pub async fn detail(backend: &Backend, id: RequirementId) -> Result<RequirementDetail> {
    let requirement = backend
        .requirement(id)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "requirement",
            id: id.to_string(),
        })?;
    let coverage = backend.requirement_coverage(id).await?;
    // `None` is the mirror's "not cached" and stays `None`: the pane says revisions need the
    // database rather than showing an empty history.
    let revisions = match backend.requirement_revisions(id).await? {
        Some(rows) => {
            let mut out = Vec::with_capacity(rows.len());
            for revision in rows {
                let deciding_key = match revision.amended_by_item_id {
                    Some(item) => backend.item(item).await?.map(|row| row.key),
                    None => None,
                };
                out.push(RevisionRow {
                    revision,
                    deciding_key,
                });
            }
            Some(out)
        }
        None => None,
    };
    Ok(RequirementDetail {
        requirement,
        coverage,
        revisions,
    })
}

/// One item's citations and what it could cite (plan P8).
///
/// # Errors
/// [`StoreError::NotFound`] `{ entity: "item" }` for an unknown item; whatever else the backend
/// reports.
pub async fn citations(backend: &Backend, item: ItemId) -> Result<ItemCitations> {
    let project_id = item_project(backend, item).await?;
    let active = RequirementFilter {
        states: Some(vec![RequirementState::Active]),
        ..RequirementFilter::default()
    };
    Ok(ItemCitations {
        item,
        project_id,
        citations: backend.item_requirements(item).await?,
        candidates: backend.requirements(project_id, &active).await?,
        writable: backend.writer().is_some(),
    })
}

/// What one tab write did (MOD-59 D2), carried by [`StoreReply::RequirementWritten`] beside the
/// scope re-read. Ids, codes, keys and versions only: a body stays in the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementWrite {
    /// [`StoreRequest::CreateRequirementArea`]: the area landed.
    Area {
        /// The area's id.
        id: RequirementAreaId,
        /// Its code, as stored (trimmed).
        code: String,
    },
    /// [`StoreRequest::MintRequirement`]: the requirement landed at version 1.
    Minted {
        /// The id the worker minted.
        id: RequirementId,
        /// Its key, `R-<code>-<number>`.
        key: String,
    },
    /// [`StoreRequest::AmendRequirement`] at the head: the new head.
    Amended {
        /// The requirement.
        id: RequirementId,
        /// Its key.
        key: String,
        /// The version written.
        version: i32,
    },
    /// [`StoreRequest::WithdrawRequirement`] at the head.
    Withdrawn {
        /// The requirement.
        id: RequirementId,
        /// Its key.
        key: String,
    },
}

impl RequirementWrite {
    /// The [`StoreRequest::name`] of the write this answers: what the tab's `busy` holds while it
    /// is in flight, so only that write lands on it (MOD-59 D4).
    #[must_use]
    pub const fn request_name(&self) -> &'static str {
        match self {
            Self::Area { .. } => REQUEST_NAMES[3],
            Self::Minted { .. } => REQUEST_NAMES[4],
            Self::Amended { .. } => REQUEST_NAMES[5],
            Self::Withdrawn { .. } => REQUEST_NAMES[6],
        }
    }
}

/// Serves one requirement request, off the UI task.
///
/// The tab's read answers [`StoreReply::Requirements`]. A tab write answers
/// [`StoreReply::RequirementWritten`] when it applied, even when only the re-read after it failed
/// (MOD-59 D1, D5), and an amend or withdraw that missed its version answers
/// [`StoreReply::RequirementsStale`]; a refusal stays an error.
///
/// # Errors
/// Whatever the seam reports; offline, [`StoreError::Unreachable`] with `DATABASE_UNREACHABLE`
/// for the seven writes; [`StoreError::Constraint`] for a refused input or a user who is not the
/// maintainer; [`StoreError::Backend`] for a request that is not one of this module's ten.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        StoreRequest::Requirements(scope) => Ok(StoreReply::Requirements(Box::new(
            snapshot(backend, scope).await?,
        ))),
        StoreRequest::RequirementDetail(id) => Ok(StoreReply::RequirementDetail(Box::new(
            detail(backend, *id).await?,
        ))),
        StoreRequest::ItemRequirements(item) => Ok(StoreReply::ItemCitations(Box::new(
            citations(backend, *item).await?,
        ))),
        StoreRequest::CreateRequirementArea {
            scope,
            project,
            code,
            title,
        } => {
            let writer = write_access(backend)?;
            let code = code.trim();
            if !RequirementArea::code_is_valid(code) {
                return Err(StoreError::Constraint(invalid_area_code(code)));
            }
            let title = title.trim();
            if title.is_empty() {
                return Err(StoreError::Constraint(BLANK_AREA_TITLE.to_owned()));
            }
            // Read before the gate: a code the project already has is the store's refusal, and
            // the store would give it only after the gate had claimed a spec (blueprint F-9).
            let areas = writer.requirement_areas(*project).await?;
            if areas.iter().any(|area| area.code == code) {
                return Err(StoreError::Constraint(already_exists(
                    "requirement_area.code",
                    code,
                )));
            }
            let position = areas
                .iter()
                .map(|area| area.position)
                .max()
                .map_or(0, |last| last + 1);
            let me = backend.this_user().await?;
            gate(backend, &writer, *project, me).await?;
            let area = writer
                .create_requirement_area(NewRequirementArea {
                    id: RequirementAreaId::new(),
                    project_id: *project,
                    code: code.to_owned(),
                    title: title.to_owned(),
                    description: String::new(),
                    position,
                })
                .await?;
            let landed = RequirementWrite::Area {
                id: area.id,
                code: area.code,
            };
            answer(backend, scope, Some(landed)).await
        }
        StoreRequest::MintRequirement {
            scope,
            project,
            area,
            body,
            rationale,
            priority,
        } => {
            let writer = write_access(backend)?;
            refuse_blank_body(body)?;
            // The area must be the project's: the gate checks `project`, so an area of another
            // project would slip a write past that project's owner (blueprint F-7).
            if !backend
                .requirement_areas(*project)
                .await?
                .iter()
                .any(|row| row.id == *area)
            {
                return Err(StoreError::NotFound {
                    entity: "requirement_area",
                    id: area.to_string(),
                });
            }
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            gate(backend, &writer, *project, me).await?;
            let row = writer
                .mint_requirement(
                    *area,
                    NewRequirement {
                        id: RequirementId::new(),
                        body: body.as_str().to_owned(),
                        rationale: rationale.as_str().to_owned(),
                        priority: *priority,
                        created_by: me,
                        box_id,
                    },
                )
                .await?;
            let landed = RequirementWrite::Minted {
                id: row.id,
                key: row.key,
            };
            answer(backend, scope, Some(landed)).await
        }
        StoreRequest::AmendRequirement {
            scope,
            id,
            expected_version,
            body,
            rationale,
            priority,
            deciding,
        } => {
            let writer = write_access(backend)?;
            refuse_blank_body(body)?;
            let project = revisable(backend, *id, *expected_version).await?;
            let deciding = deciding_item(backend, scope, project, deciding).await?;
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            gate(backend, &writer, project, me).await?;
            let patch = RequirementPatch {
                body: Some(body.as_str().to_owned()),
                rationale: Some(rationale.as_str().to_owned()),
                priority: Some(*priority),
                author_id: me,
                box_id,
                reason: "amended".to_owned(),
            };
            let landed = match writer
                .amend_requirement(*id, *expected_version, patch, deciding)
                .await?
            {
                RequirementUpdate::Updated(row) => Some(RequirementWrite::Amended {
                    id: row.id,
                    key: row.key,
                    version: row.version,
                }),
                RequirementUpdate::Diverged { .. } => None,
            };
            answer(backend, scope, landed).await
        }
        StoreRequest::WithdrawRequirement {
            scope,
            id,
            expected_version,
            deciding,
        } => {
            let writer = write_access(backend)?;
            let project = revisable(backend, *id, *expected_version).await?;
            let deciding = deciding_item(backend, scope, project, deciding).await?;
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            gate(backend, &writer, project, me).await?;
            let landed = match writer
                .withdraw_requirement(*id, *expected_version, deciding, me, box_id)
                .await?
            {
                RequirementUpdate::Updated(row) => Some(RequirementWrite::Withdrawn {
                    id: row.id,
                    key: row.key,
                }),
                RequirementUpdate::Diverged { .. } => None,
            };
            answer(backend, scope, landed).await
        }
        StoreRequest::CiteRequirement {
            item,
            requirement,
            kind,
        } => {
            let writer = write_access(backend)?;
            // PRD D4, checked before the store: it would take `amends` / `withdraws` too, and a
            // requirement of any project.
            if matches!(kind, CitationKind::Amends | CitationKind::Withdraws) {
                return Err(StoreError::Constraint(decision_citation_not_cited(*kind)));
            }
            let project = item_project(backend, *item).await?;
            let cited =
                backend
                    .requirement(*requirement)
                    .await?
                    .ok_or_else(|| StoreError::NotFound {
                        entity: "requirement",
                        id: requirement.to_string(),
                    })?;
            if cited.project_id != project {
                return Err(StoreError::Constraint(requirement_of_another_project(
                    &cited.key,
                    &project_label(backend, project).await?,
                )));
            }
            // `None`: a human citation, which is what `proposed_by_step_id` records (ANA-11 §4.3).
            writer.cite(*item, *requirement, *kind, None).await?;
            answer_citations(backend, *item).await
        }
        StoreRequest::UnciteRequirement {
            item,
            requirement,
            kind,
        } => {
            let writer = write_access(backend)?;
            if matches!(kind, CitationKind::Amends | CitationKind::Withdraws) {
                return Err(StoreError::Constraint(decision_citation_stays(*kind)));
            }
            writer.uncite(*item, *requirement, *kind).await?;
            answer_citations(backend, *item).await
        }
        StoreRequest::ReconfirmCitation {
            item,
            requirement,
            kind,
        } => {
            let writer = write_access(backend)?;
            writer.reconfirm(*item, *requirement, *kind).await?;
            answer_citations(backend, *item).await
        }
        // `try_serve` routes exactly this module's ten variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a requirement request: {}",
            other.name()
        ))),
    }
}

/// The writer, or the refusal every requirement write answers offline.
fn write_access(backend: &Backend) -> Result<Writer> {
    backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))
}

/// [`BLANK_BODY`] for a body that is blank after `trim`.
fn refuse_blank_body(body: &RequirementText) -> Result<()> {
    if body.as_str().trim().is_empty() {
        Err(StoreError::Constraint(BLANK_BODY.to_owned()))
    } else {
        Ok(())
    }
}

/// The project of a requirement about to be amended or withdrawn at `expected_version`. A
/// requirement is never deleted, so `NotFound` is not a race here and stays an error rather than a
/// stale answer.
///
/// A requirement withdrawn at `expected_version` is refused here, in the store's words
/// ([`requirement_withdrawn`]): the store refuses it only after [`gate`] has claimed a spec
/// (blueprint F-9). At any other version it goes on, because the store checks divergence first
/// and answers stale.
async fn revisable(
    backend: &Backend,
    id: RequirementId,
    expected_version: i32,
) -> Result<ProjectId> {
    let row = backend
        .requirement(id)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "requirement",
            id: id.to_string(),
        })?;
    if row.version == expected_version && row.state == RequirementState::Withdrawn {
        return Err(StoreError::Constraint(requirement_withdrawn(&row.key)));
    }
    Ok(row.project_id)
}

/// The item's project.
async fn item_project(backend: &Backend, item: ItemId) -> Result<ProjectId> {
    backend
        .item(item)
        .await?
        .map(|row| row.project_id)
        .ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: item.to_string(),
        })
}

/// The project's slug, or its id when the row is gone: for refusal sentences only.
async fn project_label(backend: &Backend, project: ProjectId) -> Result<String> {
    Ok(backend
        .project(project)
        .await?
        .map_or_else(|| project.to_string(), |row| row.slug))
}

/// Plan P6 and blueprint F-11: the item of `project` whose key is exactly `key`, trimmed and
/// ASCII upper-cased (item keys are ASCII upper). The store's text filter narrows the read, and
/// the exact match drops what it also finds (`ANA-10` for `ANA-1`, a title containing the key).
///
/// The read's scope is `project` alone, in the request's workspace: `items` never looks outside
/// its scope, so the request's own scope would refuse every key of a requirement whose project it
/// has left (a scope switch while the form was open).
async fn deciding_item(
    backend: &Backend,
    scope: &Scope,
    project: ProjectId,
    key: &str,
) -> Result<ItemId> {
    let wanted = key.trim().to_ascii_uppercase();
    if wanted.is_empty() {
        return Err(StoreError::Constraint(DECIDING_KEY_NEEDED.to_owned()));
    }
    let only = Scope {
        workspace_id: scope.workspace_id,
        project_ids: vec![project],
    };
    let filter = ItemFilter {
        project_ids: Some(vec![project]),
        text: Some(wanted.clone()),
        ..ItemFilter::default()
    };
    let found = backend
        .items(&only, &filter)
        .await?
        .into_iter()
        .find(|row| row.key == wanted);
    match found {
        Some(row) => Ok(row.id),
        None => Err(StoreError::Constraint(no_deciding_item(
            &wanted,
            &project_label(backend, project).await?,
        ))),
    }
}

/// Plan P5, PRD D1, blueprint F-9: the last step before a gated write. The spec's owner passes;
/// with no spec, this write claims it for `me` first. A `Stale` claim means another session
/// created the spec between the read and the write, and its owner decides, as it would have had
/// the read come a moment later.
async fn gate(backend: &Backend, writer: &Writer, project: ProjectId, me: UserId) -> Result<()> {
    let read = writer.requirement_spec(project).await?;
    gate_after_read(backend, writer, project, me, read).await
}

/// [`gate`] from its read on, apart so a test can hand it the `None` that a racing claim has made
/// stale.
async fn gate_after_read(
    backend: &Backend,
    writer: &Writer,
    project: ProjectId,
    me: UserId,
    read: Option<RequirementSpec>,
) -> Result<()> {
    let owner = match read {
        Some(spec) => spec.owner_id,
        None => match writer
            .set_requirement_spec(project, None, me, String::new())
            .await?
        {
            CasOutcome::Applied(spec) => spec.owner_id,
            // Another session's claim won; its owner decides, and that may not be `me`.
            CasOutcome::Stale(spec) => spec.owner_id,
        },
    };
    if owner == me {
        Ok(())
    } else {
        Err(StoreError::Constraint(not_the_maintainer(
            &project_label(backend, project).await?,
        )))
    }
}

/// The scope re-read after a tab write (MOD-59 D1, D5): `RequirementWritten` naming what `landed`
/// wrote, or `RequirementsStale` when an amend or withdraw missed its version (`None`). The worker
/// re-reads rather than handing the view the row the outcome carries: the view renders a tree, and
/// a row patched in locally would be a second source of truth. A re-read that fails after an
/// applied write still answers `RequirementWritten`, because the write landed; after a missed one
/// it stays an error (D3).
async fn answer(
    backend: &Backend,
    scope: &Scope,
    landed: Option<RequirementWrite>,
) -> Result<StoreReply> {
    let fresh = snapshot(backend, scope).await;
    match landed {
        Some(outcome) => Ok(written(fresh, outcome)),
        None => Ok(StoreReply::RequirementsStale(Box::new(fresh?))),
    }
}

/// MOD-59 D5: the reply to a tab write that applied, whatever its re-read came to. A failed
/// re-read travels as its `StoreError` rendered through `Display`, the sentence `Failed` would
/// carry.
fn written(reread: Result<RequirementsSnapshot>, outcome: RequirementWrite) -> StoreReply {
    StoreReply::RequirementWritten {
        snapshot: reread.map(Box::new).map_err(|err| err.to_string()),
        outcome,
    }
}

/// The item's citations re-read after a citation write.
async fn answer_citations(backend: &Backend, item: ItemId) -> Result<StoreReply> {
    Ok(StoreReply::ItemCitations(Box::new(
        citations(backend, item).await?,
    )))
}

#[cfg(test)]
mod tests {
    use super::{
        BLANK_AREA_TITLE, BLANK_BODY, CITATIONS_NAME, DECIDING_KEY_NEEDED, DETAIL_NAME,
        ItemCitations, READ_NAME, REQUEST_NAMES, RequirementDetail, RequirementText,
        RequirementWrite, RequirementsSnapshot, decision_citation_not_cited,
        decision_citation_stays, gate_after_read, is_citation_write, is_tab_write, matches_filter,
        no_deciding_item, not_the_maintainer, requirement_of_another_project, serve, written,
    };
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use chrono::TimeDelta;
    use htui_core::fixtures::{self, ids};
    use htui_core::model::{
        AppUser, CitationKind, ItemId, NewRequirement, NewRequirementArea, Priority, ProjectId,
        RequirementAreaId, RequirementId, RequirementState, RequirementUpdate, Scope, UserId,
    };
    use htui_core::store::{
        MemStore, ReadStore as _, StoreError, WriteStore as _, already_exists, invalid_area_code,
        requirement_withdrawn,
    };
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    /// The demo with a second user created a day before the fixture's, so `MemStore::this_user`
    /// (the earliest row) is the stranger and `htui`'s spec owner, `ids::USER`, is not.
    fn stranger_first() -> Backend {
        let mut data = fixtures::demo_data();
        let at = fixtures::demo_at(0, 0) - TimeDelta::days(1);
        data.users.push(AppUser {
            id: UserId::new(),
            name: "stranger".to_owned(),
            email: None,
            created_at: at,
            updated_at: at,
        });
        Backend::memory(MemStore::from_demo(data))
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

    async fn read(backend: &Backend, scope: &Scope) -> RequirementsSnapshot {
        match serve(backend, &StoreRequest::Requirements(scope.clone())).await {
            Ok(StoreReply::Requirements(snapshot)) => *snapshot,
            other => panic!("the read answered {other:?}"),
        }
    }

    async fn detail_of(backend: &Backend, id: RequirementId) -> RequirementDetail {
        match serve(backend, &StoreRequest::RequirementDetail(id)).await {
            Ok(StoreReply::RequirementDetail(detail)) => *detail,
            other => panic!("the detail read answered {other:?}"),
        }
    }

    async fn citations_of(backend: &Backend, item: ItemId) -> ItemCitations {
        match serve(backend, &StoreRequest::ItemRequirements(item)).await {
            Ok(StoreReply::ItemCitations(citations)) => *citations,
            other => panic!("the citation read answered {other:?}"),
        }
    }

    /// The `(requirement, kind, suspect)` of every citation, for compact assertions.
    fn cited(citations: &ItemCitations) -> Vec<(RequirementId, CitationKind, bool)> {
        citations
            .citations
            .iter()
            .map(|row| (row.requirement.id, row.kind, row.suspect))
            .collect()
    }

    fn keys(snapshot: &RequirementsSnapshot, project: ProjectId) -> Vec<String> {
        snapshot
            .project(project)
            .expect("the project is in the snapshot")
            .requirements
            .iter()
            .map(|row| row.key.clone())
            .collect()
    }

    fn create_area(scope: &Scope, project: ProjectId, code: &str, title: &str) -> StoreRequest {
        StoreRequest::CreateRequirementArea {
            scope: scope.clone(),
            project,
            code: code.to_owned(),
            title: title.to_owned(),
        }
    }

    fn mint(
        scope: &Scope,
        project: ProjectId,
        area: RequirementAreaId,
        body: &str,
    ) -> StoreRequest {
        StoreRequest::MintRequirement {
            scope: scope.clone(),
            project,
            area,
            body: RequirementText::new(body),
            rationale: RequirementText::new("Because."),
            priority: Priority::Must,
        }
    }

    fn amend(scope: &Scope, id: RequirementId, expected: i32, deciding: &str) -> StoreRequest {
        StoreRequest::AmendRequirement {
            scope: scope.clone(),
            id,
            expected_version: expected,
            body: RequirementText::new("Every item has a stable, unique key of the form PREFIX-N."),
            rationale: RequirementText::new("Keys are what people type and search."),
            priority: Priority::Must,
            deciding: deciding.to_owned(),
        }
    }

    fn withdraw(scope: &Scope, id: RequirementId, expected: i32, deciding: &str) -> StoreRequest {
        StoreRequest::WithdrawRequirement {
            scope: scope.clone(),
            id,
            expected_version: expected,
            deciding: deciding.to_owned(),
        }
    }

    #[tokio::test]
    async fn the_read_answers_every_scope_project_in_scope_order() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        assert_eq!(scope.project_ids, vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);

        let snapshot = read(&backend, &scope).await;

        assert!(snapshot.is_for(&scope));
        assert!(snapshot.writable, "a memory store has a writer");
        let order: Vec<ProjectId> = snapshot.projects.iter().map(|p| p.project_id).collect();
        assert_eq!(
            order, scope.project_ids,
            "one entry per scope project, in order"
        );

        let htui = snapshot.project(ids::PROJECT_HTUI).expect("htui");
        assert_eq!(
            htui.spec.as_ref().map(|spec| spec.owner_id),
            Some(ids::USER)
        );
        let codes: Vec<&str> = htui.areas.iter().map(|a| a.code.as_str()).collect();
        assert_eq!(codes, ["ENT", "STO"]);
        assert_eq!(
            keys(&snapshot, ids::PROJECT_HTUI),
            ["R-ENT-1", "R-ENT-2", "R-STO-1"]
        );
        assert!(htui.maintainer, "this user owns htui's spec");
        let ent: Vec<&str> = htui
            .in_area(ids::AREA_ENT)
            .map(|row| row.key.as_str())
            .collect();
        assert_eq!(ent, ["R-ENT-1", "R-ENT-2"]);
        assert_eq!(
            snapshot
                .requirement(ids::REQ_STO_1)
                .map(|row| row.key.as_str()),
            Some("R-STO-1")
        );
        assert_eq!(
            snapshot.area(ids::AREA_STO).map(|row| row.code.as_str()),
            Some("STO")
        );

        let agy = snapshot.project(ids::PROJECT_AGY).expect("agy");
        assert_eq!(agy.spec, None);
        assert!(agy.areas.is_empty() && agy.requirements.is_empty());
        assert!(agy.maintainer, "a project with no spec is anyone's");

        let other = Scope {
            workspace_id: scope.workspace_id,
            project_ids: vec![ids::PROJECT_AGY, ids::PROJECT_HTUI],
        };
        assert!(!snapshot.is_for(&other), "the order is part of the scope");
    }

    #[tokio::test]
    async fn a_non_owner_reads_maintainer_false_and_every_gated_write_is_refused() {
        let backend = stranger_first();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        assert!(
            !before.project(ids::PROJECT_HTUI).expect("htui").maintainer,
            "the stranger does not own htui's spec"
        );

        let writes = [
            create_area(&scope, ids::PROJECT_HTUI, "API", "Interface"),
            mint(
                &scope,
                ids::PROJECT_HTUI,
                ids::AREA_ENT,
                "Every item has a title.",
            ),
            amend(&scope, ids::REQ_ENT_1, 2, "ANA-2"),
            withdraw(&scope, ids::REQ_STO_1, 1, "ANA-2"),
        ];
        let refusal = not_the_maintainer("htui");
        for write in &writes {
            match store_worker::serve(&backend, write).await {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, write.name());
                    assert!(message.contains(&refusal), "{request}: {message}");
                }
                other => panic!("{} is refused, not {other:?}", write.name()),
            }
        }

        assert_eq!(read(&backend, &scope).await, before, "nothing is written");
    }

    /// MOD-59 D1, D2 too: an applied area create answers `RequirementWritten`, naming the area as
    /// stored.
    #[tokio::test]
    async fn the_first_gated_write_claims_a_project_without_a_spec() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let reply = serve(
            &backend,
            &create_area(&scope, ids::PROJECT_AGY, " API ", " Interface "),
        )
        .await;

        let Ok(StoreReply::RequirementWritten {
            snapshot: Ok(after),
            outcome: RequirementWrite::Area { id, code },
        }) = reply
        else {
            panic!("an applied area create answers `RequirementWritten`, got {reply:?}")
        };
        assert_eq!(code, "API", "the code as stored, trimmed");
        let agy = after.project(ids::PROJECT_AGY).expect("agy");
        let spec = agy.spec.as_ref().expect("the write claimed a spec");
        assert_eq!(spec.owner_id, ids::USER, "owned by this user");
        assert_eq!(spec.preamble, "");
        assert!(agy.maintainer);
        let [area] = agy.areas.as_slice() else {
            panic!("one area, got {:?}", agy.areas)
        };
        assert_eq!(
            (area.code.as_str(), area.title.as_str()),
            ("API", "Interface")
        );
        assert_eq!(area.position, 0, "the first area of a project");
        assert_eq!(area.id, id, "the outcome names the area written");

        let second = serve(
            &backend,
            &create_area(&scope, ids::PROJECT_HTUI, "UI", "Screens"),
        )
        .await;
        let Ok(StoreReply::RequirementWritten {
            snapshot: Ok(after),
            outcome: RequirementWrite::Area { code, .. },
        }) = second
        else {
            panic!("a second area applies, got {second:?}")
        };
        assert_eq!(code, "UI");
        let htui = after.project(ids::PROJECT_HTUI).expect("htui");
        let placed: Vec<(&str, i32)> = htui
            .areas
            .iter()
            .map(|a| (a.code.as_str(), a.position))
            .collect();
        assert_eq!(placed, [("ENT", 0), ("STO", 1), ("UI", 2)], "last + 1");
    }

    #[tokio::test]
    async fn a_refused_input_claims_no_spec() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let bad_code = serve(
            &backend,
            &create_area(&scope, ids::PROJECT_AGY, "bad", "Title"),
        )
        .await;
        assert_eq!(
            bad_code.err(),
            Some(StoreError::Constraint(invalid_area_code("bad")))
        );
        let blank_title = serve(
            &backend,
            &create_area(&scope, ids::PROJECT_AGY, "API", "  "),
        )
        .await;
        assert_eq!(
            blank_title.err(),
            Some(StoreError::Constraint(BLANK_AREA_TITLE.to_owned()))
        );
        let blank_body = serve(
            &backend,
            &mint(&scope, ids::PROJECT_AGY, ids::AREA_ENT, " \n"),
        )
        .await;
        assert_eq!(
            blank_body.err(),
            Some(StoreError::Constraint(BLANK_BODY.to_owned()))
        );

        let after = read(&backend, &scope).await;
        assert_eq!(
            after.project(ids::PROJECT_AGY).expect("agy").spec,
            None,
            "a refused write claims nothing"
        );
    }

    /// MOD-59 D1, D2: an applied mint answers `RequirementWritten`, naming the id the worker minted
    /// and the key the store gave it.
    #[tokio::test]
    async fn mint_answers_requirement_written_with_the_next_key() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let reply = serve(
            &backend,
            &mint(
                &scope,
                ids::PROJECT_HTUI,
                ids::AREA_ENT,
                "Every item has a title.",
            ),
        )
        .await;

        let Ok(StoreReply::RequirementWritten {
            snapshot: Ok(after),
            outcome,
        }) = reply
        else {
            panic!("an applied mint answers `RequirementWritten`, got {reply:?}")
        };
        let minted = after
            .project(ids::PROJECT_HTUI)
            .expect("htui")
            .requirements
            .iter()
            .find(|row| row.key == "R-ENT-3")
            .expect("the next number of ENT");
        assert_eq!(minted.version, 1);
        assert_eq!(minted.body, "Every item has a title.");
        assert_eq!(minted.rationale, "Because.");
        assert_eq!(minted.priority, Priority::Must);
        assert_eq!(minted.state, RequirementState::Active);
        assert_eq!(minted.created_by, ids::USER, "the worker fills the author");
        assert_eq!(
            outcome,
            RequirementWrite::Minted {
                id: minted.id,
                key: "R-ENT-3".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn mint_into_an_area_of_another_project_is_refused() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let reply = serve(
            &backend,
            &mint(&scope, ids::PROJECT_AGY, ids::AREA_ENT, "Body."),
        )
        .await;

        assert_eq!(
            reply.err(),
            Some(StoreError::NotFound {
                entity: "requirement_area",
                id: ids::AREA_ENT.to_string(),
            })
        );
        let after = read(&backend, &scope).await;
        assert_eq!(after.project(ids::PROJECT_AGY).expect("agy").spec, None);
        assert_eq!(
            keys(&after, ids::PROJECT_HTUI),
            ["R-ENT-1", "R-ENT-2", "R-STO-1"]
        );
    }

    #[tokio::test]
    async fn amend_at_the_head_records_the_deciding_item() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let reply = serve(&backend, &amend(&scope, ids::REQ_ENT_1, 2, " ana-2 ")).await;

        let Ok(StoreReply::RequirementWritten {
            snapshot: Ok(after),
            outcome,
        }) = reply
        else {
            panic!("an applied amend answers `RequirementWritten`, got {reply:?}")
        };
        assert_eq!(
            outcome,
            RequirementWrite::Amended {
                id: ids::REQ_ENT_1,
                key: "R-ENT-1".to_owned(),
                version: 3,
            },
            "the new head (MOD-59 D2)"
        );
        let head = after.requirement(ids::REQ_ENT_1).expect("R-ENT-1");
        assert_eq!(head.version, 3);
        assert_eq!(
            head.body,
            "Every item has a stable, unique key of the form PREFIX-N."
        );
        let detail = detail_of(&backend, ids::REQ_ENT_1).await;
        let revisions = detail.revisions.expect("a memory store keeps revisions");
        let v3 = revisions.last().expect("the new revision");
        assert_eq!(v3.revision.version, 3);
        assert_eq!(v3.revision.reason, "amended");
        assert_eq!(v3.revision.author_id, ids::USER);
        assert_eq!(
            v3.revision.box_id,
            Some(ids::BOX),
            "the worker fills the box"
        );
        assert_eq!(v3.deciding_key.as_deref(), Some("ANA-2"));
        assert_eq!(
            cited(&citations_of(&backend, ids::HTUI_ANA_1).await),
            [(ids::REQ_ENT_1, CitationKind::Addresses, true)],
            "ANA-1's v1 citation is still suspect"
        );
    }

    #[tokio::test]
    async fn amend_at_a_stale_version_answers_requirements_stale_and_writes_nothing() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;

        let reply = serve(&backend, &amend(&scope, ids::REQ_ENT_1, 1, "ANA-2")).await;

        let Ok(StoreReply::RequirementsStale(fresh)) = reply else {
            panic!("a spent version answers `RequirementsStale`, got {reply:?}")
        };
        assert_eq!(
            *fresh, before,
            "the stale reply is the store as it is, unchanged"
        );
        assert_eq!(
            fresh.requirement(ids::REQ_ENT_1).expect("R-ENT-1").version,
            2
        );
    }

    #[tokio::test]
    async fn an_unknown_or_partial_deciding_key_is_refused_before_any_write() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;

        for (typed, refusal) in [
            ("FEAT-99", no_deciding_item("FEAT-99", "htui")),
            ("ana", no_deciding_item("ANA", "htui")),
            ("  ", DECIDING_KEY_NEEDED.to_owned()),
        ] {
            let amended = serve(&backend, &amend(&scope, ids::REQ_ENT_1, 2, typed)).await;
            assert_eq!(
                amended.err(),
                Some(StoreError::Constraint(refusal.clone())),
                "amend by {typed:?}"
            );
            let withdrawn = serve(&backend, &withdraw(&scope, ids::REQ_STO_1, 1, typed)).await;
            assert_eq!(
                withdrawn.err(),
                Some(StoreError::Constraint(refusal)),
                "withdraw by {typed:?}"
            );
        }

        assert_eq!(read(&backend, &scope).await, before, "nothing is written");
    }

    #[tokio::test]
    async fn withdraw_marks_the_requirement_withdrawn_and_cites_the_deciding_item() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let reply = serve(&backend, &withdraw(&scope, ids::REQ_STO_1, 1, "ANA-2")).await;

        let Ok(StoreReply::RequirementWritten {
            snapshot: Ok(after),
            outcome,
        }) = reply
        else {
            panic!("an applied withdraw answers `RequirementWritten`, got {reply:?}")
        };
        assert_eq!(
            outcome,
            RequirementWrite::Withdrawn {
                id: ids::REQ_STO_1,
                key: "R-STO-1".to_owned(),
            },
            "MOD-59 D2"
        );
        let row = after.requirement(ids::REQ_STO_1).expect("still listed");
        assert_eq!(row.state, RequirementState::Withdrawn);
        assert_eq!(row.version, 2);
        assert!(
            cited(&citations_of(&backend, ids::HTUI_ANA_2).await).contains(&(
                ids::REQ_STO_1,
                CitationKind::Withdraws,
                false
            )),
            "the deciding item cites the withdraw"
        );

        let again = serve(&backend, &withdraw(&scope, ids::REQ_STO_1, 1, "ANA-2")).await;
        assert!(
            matches!(again, Ok(StoreReply::RequirementsStale(_))),
            "a second withdraw at the old version is stale: {again:?}"
        );
    }

    /// MOD-59 D5: a re-read that fails after a tab write applied still answers
    /// `RequirementWritten`, the failure rendered as `Failed` would render it. `MemStore` cannot
    /// fail a read inside one `serve`, so the mapping is pinned here.
    #[test]
    fn a_reread_that_fails_after_a_tab_write_still_answers_requirement_written() {
        let outcome = RequirementWrite::Minted {
            id: ids::REQ_ENT_2,
            key: "R-ENT-2".to_owned(),
        };

        let reply = written(
            Err(StoreError::Unreachable("gone".to_owned())),
            outcome.clone(),
        );

        match reply {
            StoreReply::RequirementWritten {
                snapshot: Err(message),
                outcome: landed,
            } => {
                assert_eq!(message, "store unreachable: gone");
                assert_eq!(landed, outcome);
            }
            other => panic!("a landed write answers `RequirementWritten`, not {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_detail_of_r_ent_1_lists_its_coverage_and_revisions() {
        let backend = demo();

        let detail = detail_of(&backend, ids::REQ_ENT_1).await;

        assert_eq!(detail.requirement.key, "R-ENT-1");
        let coverage: Vec<(&str, CitationKind, bool)> = detail
            .coverage
            .iter()
            .map(|row| (row.item.key.as_str(), row.kind, row.suspect))
            .collect();
        assert_eq!(
            coverage,
            [
                ("ANA-1", CitationKind::Addresses, true),
                ("ANA-2", CitationKind::Amends, false),
            ]
        );
        let revisions: Vec<(i32, Option<&str>)> = detail
            .revisions
            .as_deref()
            .expect("a memory store keeps revisions")
            .iter()
            .map(|row| (row.revision.version, row.deciding_key.as_deref()))
            .collect();
        assert_eq!(revisions, [(1, None), (2, Some("ANA-2"))]);

        let unknown = RequirementId::new();
        assert_eq!(
            serve(&backend, &StoreRequest::RequirementDetail(unknown))
                .await
                .err(),
            Some(StoreError::NotFound {
                entity: "requirement",
                id: unknown.to_string(),
            })
        );
    }

    #[tokio::test]
    async fn item_requirements_of_ana_1_is_suspect_until_reconfirmed() {
        let backend = demo();

        let before = citations_of(&backend, ids::HTUI_ANA_1).await;

        assert_eq!(before.item, ids::HTUI_ANA_1);
        assert_eq!(before.project_id, ids::PROJECT_HTUI);
        assert!(before.writable);
        assert_eq!(
            cited(&before),
            [(ids::REQ_ENT_1, CitationKind::Addresses, true)]
        );
        let candidates: Vec<&str> = before.candidates.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(candidates, ["R-ENT-1", "R-ENT-2", "R-STO-1"]);

        let reply = serve(
            &backend,
            &StoreRequest::ReconfirmCitation {
                item: ids::HTUI_ANA_1,
                requirement: ids::REQ_ENT_1,
                kind: CitationKind::Addresses,
            },
        )
        .await;

        let Ok(StoreReply::ItemCitations(after)) = reply else {
            panic!("a re-confirm answers `ItemCitations`, got {reply:?}")
        };
        assert_eq!(
            cited(&after),
            [(ids::REQ_ENT_1, CitationKind::Addresses, false)]
        );
        assert_eq!(
            after.citations[0].requirement_version, 2,
            "re-stamped at the head"
        );

        let unknown = ItemId::new();
        assert_eq!(
            serve(&backend, &StoreRequest::ItemRequirements(unknown))
                .await
                .err(),
            Some(StoreError::NotFound {
                entity: "item",
                id: unknown.to_string(),
            })
        );
    }

    #[tokio::test]
    async fn cite_then_uncite_on_feat_1() {
        let backend = demo();
        let addresses_ent_2 = (ids::REQ_ENT_2, CitationKind::Addresses, false);

        let cite = serve(
            &backend,
            &StoreRequest::CiteRequirement {
                item: ids::HTUI_FEAT_1,
                requirement: ids::REQ_ENT_2,
                kind: CitationKind::Addresses,
            },
        )
        .await;
        let Ok(StoreReply::ItemCitations(cited_now)) = cite else {
            panic!("a cite answers `ItemCitations`, got {cite:?}")
        };
        assert_eq!(cited_now.item, ids::HTUI_FEAT_1);
        assert!(
            cited(&cited_now).contains(&addresses_ent_2),
            "{cited_now:?}"
        );

        let uncite = serve(
            &backend,
            &StoreRequest::UnciteRequirement {
                item: ids::HTUI_FEAT_1,
                requirement: ids::REQ_ENT_2,
                kind: CitationKind::Addresses,
            },
        )
        .await;
        let Ok(StoreReply::ItemCitations(after)) = uncite else {
            panic!("an uncite answers `ItemCitations`, got {uncite:?}")
        };
        assert!(!cited(&after).contains(&addresses_ent_2), "{after:?}");
        assert_eq!(
            cited(&after),
            [(ids::REQ_STO_1, CitationKind::Addresses, false)],
            "FEAT-1's fixture citation stays"
        );
    }

    #[tokio::test]
    async fn uncite_of_a_decision_citation_is_refused() {
        let backend = demo();
        let request = StoreRequest::UnciteRequirement {
            item: ids::HTUI_ANA_2,
            requirement: ids::REQ_ENT_1,
            kind: CitationKind::Amends,
        };

        match store_worker::serve(&backend, &request).await {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, "uncite_requirement");
                assert!(
                    message.contains(&decision_citation_stays(CitationKind::Amends)),
                    "{message}"
                );
            }
            other => panic!("an `amends` uncite is refused, not {other:?}"),
        }
        assert_eq!(
            cited(&citations_of(&backend, ids::HTUI_ANA_2).await),
            [(ids::REQ_ENT_1, CitationKind::Amends, false)],
            "the decision stays"
        );
    }

    /// An agy area `API` and its `R-API-1`, written straight to the store so agy keeps no spec (the
    /// schema allows areas without one); `withdrawn` also withdraws it, leaving it at version 2.
    async fn agy_requirement(backend: &Backend, withdrawn: bool) -> RequirementId {
        let writer = backend.writer().expect("a memory store has a writer");
        let area = RequirementAreaId::new();
        writer
            .create_requirement_area(NewRequirementArea {
                id: area,
                project_id: ids::PROJECT_AGY,
                code: "API".to_owned(),
                title: "Interface".to_owned(),
                description: String::new(),
                position: 0,
            })
            .await
            .expect("an area needs no spec");
        let id = RequirementId::new();
        writer
            .mint_requirement(
                area,
                NewRequirement {
                    id,
                    body: "Body.".to_owned(),
                    rationale: String::new(),
                    priority: Priority::Must,
                    created_by: ids::USER,
                    box_id: None,
                },
            )
            .await
            .expect("a mint needs no spec");
        if withdrawn {
            let update = writer
                .withdraw_requirement(id, 1, ids::AGY_FEAT_1, ids::USER, None)
                .await
                .expect("a withdraw needs no spec");
            assert!(
                matches!(update, RequirementUpdate::Updated(_)),
                "{update:?}"
            );
        }
        id
    }

    /// PRD D4: a hand-made `amends` or `withdraws` citation would be a decision no maintainer
    /// made and plan P9 would never let go of; and an item cites its own project's requirements.
    #[tokio::test]
    async fn a_cite_is_addresses_or_reserves_of_the_items_own_project() {
        let backend = demo();
        let before = citations_of(&backend, ids::HTUI_FEAT_1).await;

        for kind in [CitationKind::Amends, CitationKind::Withdraws] {
            let request = StoreRequest::CiteRequirement {
                item: ids::HTUI_FEAT_1,
                requirement: ids::REQ_ENT_2,
                kind,
            };
            match store_worker::serve(&backend, &request).await {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, "cite_requirement");
                    assert!(
                        message.contains(&decision_citation_not_cited(kind)),
                        "{message}"
                    );
                }
                other => panic!("a `{kind}` cite is refused, not {other:?}"),
            }
        }
        assert_eq!(
            citations_of(&backend, ids::HTUI_FEAT_1).await,
            before,
            "nothing is cited"
        );

        let elsewhere = serve(
            &backend,
            &StoreRequest::CiteRequirement {
                item: ids::AGY_FEAT_1,
                requirement: ids::REQ_ENT_2,
                kind: CitationKind::Addresses,
            },
        )
        .await;
        assert_eq!(
            elsewhere.err(),
            Some(StoreError::Constraint(requirement_of_another_project(
                "R-ENT-2", "agy"
            )))
        );
        assert!(
            citations_of(&backend, ids::AGY_FEAT_1)
                .await
                .citations
                .is_empty(),
            "agy's FEAT-1 cites nothing"
        );
    }

    /// Blueprint F-9 on a project with areas but no spec: a duplicate area code, an amend or
    /// withdraw of a requirement withdrawn at the version it names, a blank body and an unknown
    /// deciding key are all refused before the gate could claim the spec.
    #[tokio::test]
    async fn a_refusal_the_worker_can_see_coming_claims_no_spec() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let id = agy_requirement(&backend, true).await;

        let duplicate = serve(
            &backend,
            &create_area(&scope, ids::PROJECT_AGY, " API ", "Again"),
        )
        .await;
        assert_eq!(
            duplicate.err(),
            Some(StoreError::Constraint(already_exists(
                "requirement_area.code",
                "API"
            )))
        );

        let withdrawn = Some(StoreError::Constraint(requirement_withdrawn("R-API-1")));
        let amended = serve(&backend, &amend(&scope, id, 2, "FEAT-1")).await;
        assert_eq!(amended.err(), withdrawn, "amend at the head");
        let withdrawn_again = serve(&backend, &withdraw(&scope, id, 2, "FEAT-1")).await;
        assert_eq!(withdrawn_again.err(), withdrawn, "withdraw at the head");

        let mut blank = amend(&scope, id, 1, "FEAT-1");
        if let StoreRequest::AmendRequirement { body, .. } = &mut blank {
            *body = RequirementText::new("  ");
        }
        assert_eq!(
            serve(&backend, &blank).await.err(),
            Some(StoreError::Constraint(BLANK_BODY.to_owned()))
        );
        let unknown = Some(StoreError::Constraint(no_deciding_item("FEAT-99", "agy")));
        let amended = serve(&backend, &amend(&scope, id, 1, "FEAT-99")).await;
        assert_eq!(amended.err(), unknown, "amend by an unknown key");
        let withdrawn_by = serve(&backend, &withdraw(&scope, id, 1, "FEAT-99")).await;
        assert_eq!(withdrawn_by.err(), unknown, "withdraw by an unknown key");

        assert_eq!(
            read(&backend, &scope)
                .await
                .project(ids::PROJECT_AGY)
                .expect("agy")
                .spec,
            None,
            "a refused write claims nothing"
        );
    }

    /// Plan P5's race: the gate read no spec, and another session's claim landed before this one.
    /// The `Stale` claim hands the decision to the winner's owner.
    #[tokio::test]
    async fn a_claim_that_lost_the_race_leaves_the_decision_to_the_winner() {
        let backend = stranger_first();
        let stranger = backend
            .this_user()
            .await
            .expect("the stranger is this user");
        let writer = backend.writer().expect("a memory store has a writer");
        let spec = backend
            .requirement_spec(ids::PROJECT_HTUI)
            .await
            .expect("a memory read");
        assert_eq!(spec.as_ref().map(|spec| spec.owner_id), Some(ids::USER));

        // `None`: the read that htui's existing spec has since made stale.
        let lost = gate_after_read(&backend, &writer, ids::PROJECT_HTUI, stranger, None).await;
        assert_eq!(
            lost.err(),
            Some(StoreError::Constraint(not_the_maintainer("htui"))),
            "the winner is not the stranger"
        );
        gate_after_read(&backend, &writer, ids::PROJECT_HTUI, ids::USER, None)
            .await
            .expect("the winner's owner passes");

        assert_eq!(
            backend
                .requirement_spec(ids::PROJECT_HTUI)
                .await
                .expect("a memory read"),
            spec,
            "a stale claim writes nothing"
        );
    }

    /// Blueprint F-10: a store with no row for this user reads rather than failing, and such a user
    /// maintains a project with no spec and no other.
    #[tokio::test]
    async fn a_user_the_store_does_not_know_maintains_only_a_project_with_no_spec() {
        let mut data = fixtures::demo_data();
        data.users.clear();
        let backend = Backend::memory(MemStore::from_demo(data));
        assert!(backend.this_user().await.is_err(), "no user row");
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
        };

        let snapshot = read(&backend, &scope).await;

        let htui = snapshot.project(ids::PROJECT_HTUI).expect("htui");
        assert!(htui.spec.is_some(), "htui has a spec");
        assert!(!htui.maintainer, "and nobody unknown owns it");
        assert!(
            snapshot.project(ids::PROJECT_AGY).expect("agy").maintainer,
            "a project with no spec is anyone's"
        );
    }

    /// Plan P6 searches the requirement's project, whatever scope the request carries: a form sent
    /// after a scope switch still finds its deciding item.
    #[tokio::test]
    async fn the_deciding_item_is_found_outside_the_request_scope() {
        let backend = demo();
        let agy_only = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_AGY],
        };

        let reply = serve(&backend, &amend(&agy_only, ids::REQ_ENT_1, 2, "ANA-2")).await;

        let Ok(StoreReply::RequirementWritten {
            snapshot: Ok(after),
            ..
        }) = reply
        else {
            panic!("the amend applies, got {reply:?}")
        };
        assert!(after.is_for(&agy_only), "the answer is the request's scope");
        let revisions = detail_of(&backend, ids::REQ_ENT_1)
            .await
            .revisions
            .expect("a memory store keeps revisions");
        assert_eq!(
            revisions.last().map(|row| row.deciding_key.as_deref()),
            Some(Some("ANA-2"))
        );
    }

    /// `StoreRequest` derives `Debug`; a body and a rationale are user prose, so a request prints
    /// their lengths only.
    #[test]
    fn requirement_text_debug_prints_lengths_not_text() {
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let request = StoreRequest::MintRequirement {
            scope,
            project: ids::PROJECT_HTUI,
            area: ids::AREA_ENT,
            body: RequirementText::new("secret body"),
            rationale: RequirementText::new("private why"),
            priority: Priority::Later,
        };

        let shown = format!("{request:?}");

        assert!(!shown.contains("secret"), "{shown}");
        assert!(!shown.contains("private"), "{shown}");
        assert!(shown.contains("len: 11"), "{shown}");
        assert!(
            shown.contains("Later"),
            "the priority is not prose: {shown}"
        );
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let citation = |make: fn(ItemId, RequirementId, CitationKind) -> StoreRequest| {
            make(ids::HTUI_FEAT_1, ids::REQ_ENT_2, CitationKind::Addresses)
        };
        let samples = [
            StoreRequest::Requirements(scope.clone()),
            StoreRequest::RequirementDetail(ids::REQ_ENT_1),
            StoreRequest::ItemRequirements(ids::HTUI_ANA_1),
            create_area(&scope, ids::PROJECT_HTUI, "API", "Interface"),
            mint(&scope, ids::PROJECT_HTUI, ids::AREA_ENT, "Body."),
            amend(&scope, ids::REQ_ENT_1, 2, "ANA-2"),
            withdraw(&scope, ids::REQ_ENT_1, 2, "ANA-2"),
            citation(|item, requirement, kind| StoreRequest::CiteRequirement {
                item,
                requirement,
                kind,
            }),
            citation(|item, requirement, kind| StoreRequest::UnciteRequirement {
                item,
                requirement,
                kind,
            }),
            citation(|item, requirement, kind| StoreRequest::ReconfirmCitation {
                item,
                requirement,
                kind,
            }),
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);

        assert_eq!([READ_NAME, DETAIL_NAME, CITATIONS_NAME], REQUEST_NAMES[..3]);
        for (index, name) in REQUEST_NAMES.iter().enumerate() {
            assert_eq!(is_tab_write(name), (3..7).contains(&index), "{name}");
            assert_eq!(is_citation_write(name), (7..10).contains(&index), "{name}");
        }
        assert!(!is_tab_write("templates") && !is_citation_write("templates"));

        // MOD-59 D4: each outcome names the write it answers, the name the tab's `busy` holds.
        let outcomes = [
            RequirementWrite::Area {
                id: ids::AREA_ENT,
                code: "ENT".to_owned(),
            },
            RequirementWrite::Minted {
                id: ids::REQ_ENT_1,
                key: "R-ENT-1".to_owned(),
            },
            RequirementWrite::Amended {
                id: ids::REQ_ENT_1,
                key: "R-ENT-1".to_owned(),
                version: 3,
            },
            RequirementWrite::Withdrawn {
                id: ids::REQ_ENT_1,
                key: "R-ENT-1".to_owned(),
            },
        ];
        for (outcome, request) in outcomes.iter().zip(&samples[3..7]) {
            assert_eq!(outcome.request_name(), request.name(), "{outcome:?}");
        }
    }

    /// Offline there is no writer: the read answers from the mirror, which a fresh one holds no
    /// requirement rows in, with `writable: false`; every write is refused before anything is sent.
    /// (The mirror seeds no requirement tables in tests, so the `revisions: None` branch is the
    /// detail renderer's unit test, MOD-39 blueprint F-14.)
    #[tokio::test]
    async fn offline_the_snapshot_is_read_only_and_writes_are_refused() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "requirements-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline { cache, since: None };
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
        };

        let snapshot = read(&backend, &scope).await;

        assert!(!snapshot.writable);
        assert!(snapshot.is_for(&scope));
        for project in &snapshot.projects {
            assert_eq!(project.spec, None);
            assert!(project.areas.is_empty() && project.requirements.is_empty());
        }

        let citation = (ids::HTUI_FEAT_1, ids::REQ_ENT_2, CitationKind::Addresses);
        let writes = [
            create_area(&scope, ids::PROJECT_HTUI, "API", "Interface"),
            mint(&scope, ids::PROJECT_HTUI, ids::AREA_ENT, "Body."),
            amend(&scope, ids::REQ_ENT_1, 2, "ANA-2"),
            withdraw(&scope, ids::REQ_ENT_1, 2, "ANA-2"),
            StoreRequest::CiteRequirement {
                item: citation.0,
                requirement: citation.1,
                kind: citation.2,
            },
            StoreRequest::UnciteRequirement {
                item: citation.0,
                requirement: citation.1,
                kind: citation.2,
            },
            StoreRequest::ReconfirmCitation {
                item: citation.0,
                requirement: citation.1,
                kind: citation.2,
            },
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
    async fn matches_filter_is_case_insensitive_over_key_and_body() {
        let backend = demo();
        let row = detail_of(&backend, ids::REQ_ENT_1).await.requirement;

        for needle in ["", "  ", "r-ent-1", "ENT", "STABLE KEY", " prefix-n "] {
            assert!(matches_filter(&row, needle), "{needle:?}");
        }
        for needle in ["R-STO", "storage", "people type"] {
            assert!(
                !matches_filter(&row, needle),
                "{needle:?} (the rationale is not searched)"
            );
        }
    }

    #[tokio::test]
    async fn a_request_of_another_family_is_refused_by_name() {
        let backend = demo();

        assert_eq!(
            serve(&backend, &StoreRequest::Workspaces).await.err(),
            Some(StoreError::Backend(
                "not a requirement request: workspaces".to_owned()
            ))
        );
    }
}
