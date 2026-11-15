//! Keeps the concepts index in step with Postgres (MOD-34 T4, `R-STO-8`).
//!
//! [`Indexer::sync`] is incremental and idempotent. Per project it lists the items, compares each
//! with what the index holds, and rebuilds only the items that moved: an item is stale when its
//! `updated_at` or its `status` differs from its item point (a status move does not touch
//! `updated_at`), or when the set of its latest documents differs from its document points.
//! Rebuilding an item rewrites all of its points, because every document point repeats the item's
//! status and resolution for the "decisions only" filter. Items the store no longer lists lose
//! their points.
//!
//! Requirements follow (MOD-50 D226): one point per row, withdrawn ones included, rebuilt when the
//! row's `version` differs from the point's (every amend and withdraw bumps it), and removed when
//! the row is gone.
//!
//! Owner documents follow (MOD-147 M4 D5): a project's or a workspace's scope row's own documents
//! (`item_id IS NULL`), the newest version of each kind indexed under its owner's slug. Such a
//! row is never updated (a new version is a new row), so a newest document with a point for each
//! of its sections is in step, one missing any is built (in one upsert), and the points of a
//! document no longer the newest of its kind are deleted. A workspace-scoped search reaches the scope row through
//! [`workspace_search_scope`].
//!
//! Deletes are guarded (MOD-159 D6): each project's deletes wait for one batch at its end, a
//! point of an item, requirement or owner document the listing missed joins it only when a
//! re-listing of its kind confirms the row is gone from the project (or, for an owner document,
//! no longer the newest of its kind), and a [`DeleteGuard`] (the indexer lease's renewal) is asked
//! right before a batch that is not empty. Upserts are idempotent and never guarded.
//!
//! [`Indexer::sync_changes`] is the incremental pass between two full reconciles (MOD-159 M2
//! D7): given the rows the store says were written since the last pass ([`IndexChanges`]), it
//! runs the same freshness test and rebuild on those rows alone, and deletes only the stale
//! points of what it rebuilds (stale sections, a rebuilt requirement's off-ID points, superseded
//! owner documents), under the same guard. It never deletes a row's points because the row is
//! gone or moved: that is the full reconcile's, which confirms orphans against a re-listing.
use crate::vector::{
    ConceptPoint, DocumentRef, IndexedPoint, Owner, PointType, Subject, VectorStore,
};
use htui_core::model::{
    Document, DocumentId, IndexChanges, Item, ItemFilter, ItemId, OwnerDocument, Project,
    ProjectId, Requirement, RequirementFilter, RequirementId, Scope, Status,
};
use htui_core::store::{ReadStore, StoreError};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use uuid::Uuid;

/// Longest section of a document embedded as one point, in characters. BGE-small reads about 512
/// tokens; 2000 characters of English prose stays under that.
pub const MAX_CHUNK_CHARS: usize = 2000;

/// Most requirement points written in one upsert.
const UPSERT_BATCH: usize = 256;

/// What one [`Indexer::sync`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    /// Items whose points were (re)built.
    pub items_rebuilt: usize,
    /// Items already in step.
    pub items_unchanged: usize,
    /// Requirements whose point was (re)built.
    pub requirements_rebuilt: usize,
    /// Requirements already in step.
    pub requirements_unchanged: usize,
    /// Points written.
    pub points_upserted: usize,
    /// Points removed: stale sections, superseded documents, items and requirements no longer
    /// listed.
    pub points_deleted: usize,
    /// The guard refused a delete batch (MOD-159 D6): the pass stopped there, with nothing of
    /// that batch deleted and no later project visited.
    pub abandoned: bool,
}

impl std::ops::AddAssign for SyncReport {
    fn add_assign(&mut self, other: Self) {
        self.items_rebuilt += other.items_rebuilt;
        self.items_unchanged += other.items_unchanged;
        self.requirements_rebuilt += other.requirements_rebuilt;
        self.requirements_unchanged += other.requirements_unchanged;
        self.points_upserted += other.points_upserted;
        self.points_deleted += other.points_deleted;
        self.abandoned |= other.abandoned;
    }
}

/// Asked right before each project's one delete batch, once its orphans are confirmed and only
/// when something is to go (MOD-159 D6). The indexer lease's holder
/// renews its lease here, so a pass that lost the lease stops before it deletes anything.
#[allow(async_fn_in_trait)]
pub trait DeleteGuard {
    /// Whether the pass may still delete. `false` stops it at once: the batch is dropped and
    /// [`SyncReport::abandoned`] is set.
    ///
    /// # Errors
    /// Whatever the guard's own check fails with; the pass fails with it, deleting nothing.
    async fn before_delete(&self) -> Result<bool, StoreError>;
}

/// The guard of a pass outside any lease (tests, and callers that hold none): always yes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unguarded;

impl DeleteGuard for Unguarded {
    async fn before_delete(&self) -> Result<bool, StoreError> {
        Ok(true)
    }
}

/// One project's deletes, gathered over its whole pass (MOD-159 D6), before the orphan
/// confirmation and the guard.
#[derive(Debug, Default)]
struct Doomed {
    /// Points no re-listing can save: stale sections of rebuilt items and owner documents, a
    /// rebuilt requirement's off-ID points, and owner documents superseded in the listing.
    stale: Vec<Uuid>,
    /// Points of items the listing missed, kept when a re-listing finds the item in the project.
    items: HashMap<ItemId, Vec<Uuid>>,
    /// Points of requirements the listing missed, kept as `items` are.
    requirements: HashMap<RequirementId, Vec<Uuid>>,
    /// Points of owner documents the listing missed, kept when a re-listing shows the document
    /// as the newest of its kind.
    documents: HashMap<DocumentId, Vec<Uuid>>,
}

impl Doomed {
    fn is_empty(&self) -> bool {
        self.stale.is_empty()
            && self.items.is_empty()
            && self.requirements.is_empty()
            && self.documents.is_empty()
    }

    /// The batch to delete: `stale`, and every orphan a re-listing of its kind does not show to
    /// be the project's after all. Each re-listing is the call the pass listed with (`one` is the
    /// pass's one-project scope), made once per kind with orphans, so a row counts as present
    /// exactly when that listing would index it; an item moved to another project is missing from
    /// it. A kept point is left for the next pass to settle.
    async fn confirm(
        self,
        read: &impl ReadStore,
        one: &Scope,
        project: ProjectId,
    ) -> Result<Vec<Uuid>, StoreError> {
        let mut batch = self.stale;
        if !self.items.is_empty() {
            let present: HashSet<ItemId> = read
                .items(one, &ItemFilter::default())
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            for (id, points) in self.items {
                if !present.contains(&id) {
                    batch.extend(points);
                }
            }
        }
        if !self.requirements.is_empty() {
            let present: HashSet<RequirementId> = read
                .requirements(project, &RequirementFilter::default())
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            for (id, points) in self.requirements {
                if !present.contains(&id) {
                    batch.extend(points);
                }
            }
        }
        if !self.documents.is_empty() {
            let newest: BTreeSet<DocumentId> =
                newest_owner_documents(read.owner_documents(project).await?)
                    .into_values()
                    .map(|doc| doc.id)
                    .collect();
            for (id, points) in self.documents {
                if !newest.contains(&id) {
                    batch.extend(points);
                }
            }
        }
        Ok(batch)
    }
}

/// Builds concept points from the store and reconciles the index with them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Indexer;

impl Indexer {
    /// Reconciles the index with every project of `scope`. Per project, the upserts go as they
    /// are built and every delete waits for one batch at the project's end (MOD-159 D6): each
    /// orphan (a point of an item, requirement or owner document the listing missed) is checked
    /// against a re-listing of its kind, and one that turns out to be the project's stays; then,
    /// when the batch is not empty, `guard` is asked, and a refusal ends the pass with
    /// [`abandoned`](SyncReport::abandoned) set and nothing of the batch deleted.
    ///
    /// # Errors
    /// The first failed store read, vector-store call or guard check.
    pub async fn sync(
        read: &impl ReadStore,
        scope: &Scope,
        store: &impl VectorStore,
        guard: &impl DeleteGuard,
    ) -> Result<SyncReport, StoreError> {
        let mut report = SyncReport::default();
        for &project in &scope.project_ids {
            let one = Scope {
                workspace_id: scope.workspace_id,
                project_ids: vec![project],
            };
            let items = read.items(&one, &ItemFilter::default()).await?;
            let Held {
                items: mut indexed,
                requirements: requirement_points,
                documents: owner_points,
            } = Held::read(store, project).await?;

            let mut doomed = Doomed::default();
            for summary in items {
                let old = indexed.remove(&summary.id).unwrap_or_default();
                let latest = latest_documents(read, summary.id).await?;
                if item_in_step(&old, summary.updated_at, summary.status, &latest) {
                    report.items_unchanged += 1;
                    continue;
                }
                let Some(item) = read.item(summary.id).await? else {
                    continue; // gone between the listing and now; the next sync drops it
                };
                report += rebuild_item(read, &item, &latest, &old, store, &mut doomed).await?;
            }

            report +=
                sync_requirements(read, project, requirement_points, store, &mut doomed).await?;
            // MOD-147 M4 D5: owner-document points are never among the item orphans.
            report += sync_owner_documents(read, project, owner_points, store, &mut doomed).await?;
            doomed.items = indexed
                .into_iter()
                .map(|(id, points)| (id, points.into_iter().map(|p| p.id).collect()))
                .collect();

            if doomed.is_empty() {
                continue;
            }
            // Confirm first and ask the guard last, right before the delete it guards, and only
            // when something is to go (review R1).
            let batch = doomed.confirm(read, &one, project).await?;
            if batch.is_empty() {
                continue;
            }
            if !guard.before_delete().await? {
                report.abandoned = true;
                return Ok(report);
            }
            report.points_deleted += batch.len();
            store.delete(batch).await?;
        }
        Ok(report)
    }

    /// The incremental pass (MOD-159 M2 D7): rebuilds the rows `changes` lists for the projects
    /// of `scope`, in scope order, with [`sync`](Self::sync)'s freshness test, reading each
    /// project's points once. A listed item or requirement that is gone, or that belongs to
    /// another project than the one it is listed under, is skipped: its own project's entry, or
    /// the full reconcile, settles it. A listed owner document runs the project's owner-document
    /// half as `sync` does. The one delete batch per project holds only stale points of what was
    /// rebuilt (no orphan: deleting a row's points is the full reconcile's, which confirms them),
    /// and `guard` is asked right before it, as in `sync`; a refusal ends the pass with
    /// [`abandoned`](SyncReport::abandoned) set and nothing of the batch deleted.
    ///
    /// # Errors
    /// The first failed store read, vector-store call or guard check.
    pub async fn sync_changes(
        read: &impl ReadStore,
        scope: &Scope,
        changes: &IndexChanges,
        store: &impl VectorStore,
        guard: &impl DeleteGuard,
    ) -> Result<SyncReport, StoreError> {
        let mut report = SyncReport::default();
        for &project in &scope.project_ids {
            let Some(listed) = changes.projects.get(&project) else {
                continue;
            };
            let Held {
                items: mut item_points,
                requirements: mut requirement_points,
                documents: owner_points,
            } = Held::read(store, project).await?;

            let mut doomed = Doomed::default();
            for &id in &listed.items {
                let Some(item) = read.item(id).await? else {
                    continue; // gone: the full reconcile drops its points
                };
                if item.project_id != project {
                    continue; // its own project's entry or the full reconcile settles it
                }
                let old = item_points.remove(&id).unwrap_or_default();
                let latest = latest_documents(read, id).await?;
                if item_in_step(&old, item.updated_at, item.status, &latest) {
                    report.items_unchanged += 1;
                    continue;
                }
                report += rebuild_item(read, &item, &latest, &old, store, &mut doomed).await?;
            }

            let mut points = Vec::new();
            for &id in &listed.requirements {
                let Some(row) = read.requirement(id).await? else {
                    continue;
                };
                if row.project_id != project {
                    continue;
                }
                let old = requirement_points.remove(&id).unwrap_or_default();
                if let Some(point) = rebuilt_requirement(&row, &old, &mut doomed) {
                    report.requirements_rebuilt += 1;
                    points.push(point);
                } else {
                    report.requirements_unchanged += 1;
                }
            }
            report.points_upserted += points.len();
            upsert_requirements(&points, store).await?;

            if !listed.owner_documents.is_empty() {
                report +=
                    sync_owner_documents(read, project, owner_points, store, &mut doomed).await?;
                // Only superseded documents' points stay doomed: an orphan is the full
                // reconcile's to confirm.
                doomed.documents.clear();
            }

            let batch = doomed.stale;
            if batch.is_empty() {
                continue;
            }
            if !guard.before_delete().await? {
                report.abandoned = true;
                return Ok(report);
            }
            report.points_deleted += batch.len();
            store.delete(batch).await?;
        }
        Ok(report)
    }
}

/// One project's points in the index, by owner.
#[derive(Debug, Default)]
struct Held {
    items: HashMap<ItemId, Vec<IndexedPoint>>,
    requirements: HashMap<RequirementId, Vec<IndexedPoint>>,
    documents: HashMap<DocumentId, Vec<IndexedPoint>>,
}

impl Held {
    /// One read of `project`'s points, grouped by owner.
    async fn read(store: &impl VectorStore, project: ProjectId) -> Result<Self, StoreError> {
        let mut held = Self::default();
        for point in store.indexed(project).await? {
            match point.owner {
                Owner::Item(id) => held.items.entry(id).or_default().push(point),
                Owner::Requirement(id) => held.requirements.entry(id).or_default().push(point),
                Owner::Document(id) => held.documents.entry(id).or_default().push(point),
            }
        }
        Ok(held)
    }
}

/// Whether an item's points `old` are in step with it: its item point carries `updated_at` and
/// `status` (a status move does not touch `updated_at`), and its document points are those of
/// the `latest` documents.
fn item_in_step(
    old: &[IndexedPoint],
    updated_at: chrono::DateTime<chrono::Utc>,
    status: Status,
    latest: &BTreeSet<DocumentId>,
) -> bool {
    let item_fresh = old.iter().any(|p| {
        p.point_type == PointType::Item && p.updated_at == updated_at && p.status == Some(status)
    });
    let old_docs: BTreeSet<DocumentId> = old.iter().filter_map(|p| p.document_id).collect();
    item_fresh && old_docs == *latest
}

/// Rebuilds all of `item`'s points (its own and those of its `latest` documents) in one upsert;
/// its `old` points the rebuild does not rewrite join `doomed`'s stale ones.
async fn rebuild_item(
    read: &impl ReadStore,
    item: &Item,
    latest: &BTreeSet<DocumentId>,
    old: &[IndexedPoint],
    store: &impl VectorStore,
    doomed: &mut Doomed,
) -> Result<SyncReport, StoreError> {
    let mut points = vec![item_point(item)];
    for id in latest {
        if let Some(doc) = read.document(*id).await? {
            points.extend(document_points(item, &doc));
        }
    }
    let keep: BTreeSet<Uuid> = points.iter().map(ConceptPoint::id).collect();
    let report = SyncReport {
        items_rebuilt: 1,
        points_upserted: points.len(),
        ..SyncReport::default()
    };
    store.upsert(points).await?;
    doomed
        .stale
        .extend(old.iter().map(|p| p.id).filter(|id| !keep.contains(id)));
    Ok(report)
}

/// `row`'s point when its `old` points do not hold it at its `version`, or `None` when they do.
/// A rebuilt row's off-ID points join `doomed`'s stale ones.
fn rebuilt_requirement(
    row: &Requirement,
    old: &[IndexedPoint],
    doomed: &mut Doomed,
) -> Option<ConceptPoint> {
    let point = requirement_point(row);
    let id = point.id();
    if old
        .iter()
        .any(|p| p.id == id && p.version == Some(row.version))
    {
        return None;
    }
    doomed
        .stale
        .extend(old.iter().map(|p| p.id).filter(|old| *old != id));
    Some(point)
}

/// Writes requirement points in bounded batches: a first build of a large spec would otherwise
/// embed and send every requirement of the project in one request.
async fn upsert_requirements(
    points: &[ConceptPoint],
    store: &impl VectorStore,
) -> Result<(), StoreError> {
    for batch in points.chunks(UPSERT_BATCH) {
        store.upsert(batch.to_vec()).await?;
    }
    Ok(())
}

/// The requirement half of one project's sync: one listing and stale points rebuilt in one
/// upsert. A rebuilt row's off-ID points join `doomed`'s stale ones, and the points of rows the
/// listing missed its orphans.
async fn sync_requirements(
    read: &impl ReadStore,
    project: ProjectId,
    mut indexed: HashMap<RequirementId, Vec<IndexedPoint>>,
    store: &impl VectorStore,
    doomed: &mut Doomed,
) -> Result<SyncReport, StoreError> {
    let mut report = SyncReport::default();
    let mut points = Vec::new();
    for row in read
        .requirements(project, &RequirementFilter::default())
        .await?
    {
        let old = indexed.remove(&row.id).unwrap_or_default();
        if let Some(point) = rebuilt_requirement(&row, &old, doomed) {
            report.requirements_rebuilt += 1;
            points.push(point);
        } else {
            report.requirements_unchanged += 1;
        }
    }
    for (id, old) in indexed {
        doomed
            .requirements
            .insert(id, old.into_iter().map(|p| p.id).collect());
    }
    report.points_upserted += points.len();
    upsert_requirements(&points, store).await?;
    Ok(report)
}

/// The newest version of each kind among `rows`, by kind.
fn newest_owner_documents(rows: Vec<OwnerDocument>) -> BTreeMap<String, OwnerDocument> {
    let mut latest: BTreeMap<String, OwnerDocument> = BTreeMap::new();
    for row in rows {
        match latest.get(&row.kind) {
            Some(newest) if newest.version >= row.version => {}
            _ => {
                latest.insert(row.kind.clone(), row);
            }
        }
    }
    latest
}

/// The owner-document half of one project's sync (MOD-147 M4 D5): one listing, the newest version
/// of each kind built when it lacks any of its points, and the points of every other document
/// put in `doomed`: as stale when the listing holds that document (superseded), as an orphan when
/// it does not (MOD-159 D6). An owner document is never updated, so a newest document with a
/// point for each of its sections is in step (MOD-147 M4 R1 L1). When a document is to be built
/// and the owner row is gone, nothing is written or doomed for the project.
async fn sync_owner_documents(
    read: &impl ReadStore,
    project: ProjectId,
    mut indexed: HashMap<DocumentId, Vec<IndexedPoint>>,
    store: &impl VectorStore,
    doomed: &mut Doomed,
) -> Result<SyncReport, StoreError> {
    let mut report = SyncReport::default();
    let listing = read.owner_documents(project).await?;
    let listed: BTreeSet<DocumentId> = listing.iter().map(|doc| doc.id).collect();
    let latest = newest_owner_documents(listing);
    // MOD-147 M4 R1 L1: in step only with a point for every section, so a document an earlier
    // sync left half-written is built again. Its points are taken out of `indexed` either way;
    // a rebuild rewrites the ones it builds again under the same IDs and deletes the rest, so a
    // document holding more points than it has sections settles too (fix-round verifier, low).
    let mut build: Vec<(OwnerDocument, Vec<Uuid>)> = Vec::new();
    for doc in latest.into_values() {
        let held: Vec<Uuid> = indexed.remove(&doc.id).map_or_else(Vec::new, |points| {
            points.into_iter().map(|p| p.id).collect()
        });
        if held.len() != section_count(&doc.body) {
            build.push((doc, held));
        }
    }
    let mut documents = Vec::new();
    let mut stale: Vec<Uuid> = Vec::new();
    if !build.is_empty() {
        let Some(owner) = read.project(project).await? else {
            return Ok(report); // gone since the listing; the next sync drops its points
        };
        for (doc, held) in &build {
            let points = owner_document_points(&owner, doc);
            let keep: BTreeSet<Uuid> = points.iter().map(ConceptPoint::id).collect();
            stale.extend(held.iter().copied().filter(|id| !keep.contains(id)));
            documents.push(points);
        }
    }
    report.points_upserted += documents.iter().map(Vec::len).sum::<usize>();
    // MOD-147 M4 R1 L1: one upsert per document, as the item pass writes one per item, so a
    // failed call never leaves part of a document behind.
    for points in documents {
        store.upsert(points).await?;
    }
    doomed.stale.extend(stale);
    for (id, old) in indexed {
        let points = old.into_iter().map(|p| p.id);
        if listed.contains(&id) {
            doomed.stale.extend(points);
        } else {
            doomed.documents.insert(id, points.collect());
        }
    }
    Ok(report)
}

/// The points a document of `body` is indexed as: one per section, one for an empty body
/// ([`owner_document_points`], [`document_points`]).
fn section_count(body: &str) -> usize {
    chunk_markdown(body, MAX_CHUNK_CHARS).len().max(1)
}

/// MOD-147 M4 D6: the scope a workspace-scoped search and index cover: `scope` with the
/// workspace's scope row first ([`Scope::with_scope_row`]), so the workspace's own `WS` items and
/// documents are found beside its members'. `scope` as it is for the nil startup workspace (no
/// lookup), a workspace with no scope row, and a failed lookup, which is logged: a refused lookup
/// costs the workspace's own row, not the search.
pub async fn workspace_search_scope(read: &impl ReadStore, scope: &Scope) -> Scope {
    if scope.workspace_id.as_uuid().is_nil() {
        return scope.clone();
    }
    match read.scope_project(scope.workspace_id).await {
        Ok(Some(row)) => scope.with_scope_row(row),
        Ok(None) => scope.clone(),
        Err(error) => {
            tracing::warn!(
                workspace = %scope.workspace_id,
                %error,
                "search: the workspace's scope-row lookup failed; searching its members alone"
            );
            scope.clone()
        }
    }
}

/// The ID of the latest version of each document kind the item has.
async fn latest_documents(
    read: &impl ReadStore,
    item: ItemId,
) -> Result<BTreeSet<DocumentId>, StoreError> {
    let mut latest: BTreeMap<String, (i32, DocumentId)> = BTreeMap::new();
    for head in read.documents(item).await? {
        let entry = latest.entry(head.kind).or_insert((head.version, head.id));
        if head.version > entry.0 {
            *entry = (head.version, head.id);
        }
    }
    Ok(latest.into_values().map(|(_, id)| id).collect())
}

/// The item's own point: key, title and body.
#[must_use]
pub fn item_point(item: &Item) -> ConceptPoint {
    ConceptPoint {
        subject: item_subject(item),
        key: item.key.clone(),
        project_id: item.project_id,
        updated_at: item.updated_at,
        document: None,
        text: format!("{} {}\n\n{}", item.key, item.title, item.body),
    }
}

/// What every point of `item` says about it.
fn item_subject(item: &Item) -> Subject {
    Subject::Item {
        id: item.id,
        kind_id: item.kind_id,
        status: item.status,
        resolution: item.resolution,
    }
}

/// A requirement's point: its key, body and rationale (MOD-50 D224).
#[must_use]
pub fn requirement_point(row: &Requirement) -> ConceptPoint {
    ConceptPoint {
        subject: Subject::Requirement {
            id: row.id,
            area_code: row.area_code.clone(),
            priority: row.priority,
            state: row.state,
            version: row.version,
        },
        key: row.key.clone(),
        project_id: row.project_id,
        updated_at: row.updated_at,
        document: None,
        text: format!("{} {}\n\n{}", row.key, row.body, row.rationale)
            .trim_end()
            .to_owned(),
    }
}

/// One point per section of `doc`, each prefixed with the item key and the document's title and
/// kind so a section read alone still says what it belongs to.
#[must_use]
pub fn document_points(item: &Item, doc: &Document) -> Vec<ConceptPoint> {
    let heading = format!("{} {} ({})", item.key, doc.title, doc.kind);
    let mut sections = chunk_markdown(&doc.body, MAX_CHUNK_CHARS);
    if sections.is_empty() {
        sections.push(String::new());
    }
    sections
        .into_iter()
        .enumerate()
        .map(|(chunk, section)| ConceptPoint {
            subject: item_subject(item),
            key: item.key.clone(),
            project_id: item.project_id,
            updated_at: item.updated_at,
            document: Some(DocumentRef {
                id: doc.id,
                kind: doc.kind.clone(),
                version: doc.version,
                chunk: u32::try_from(chunk).unwrap_or(u32::MAX),
            }),
            text: format!("{heading}\n\n{section}").trim_end().to_owned(),
        })
        .collect()
}

/// One point per section of an owner document (MOD-147 M4 D4), each prefixed with the owner's
/// slug and the document's title and kind, as [`document_points`] does with an item's key. The
/// key is the slug (`htui`, or `@platform` for a workspace's scope row), the time the row's
/// `created_at`. An empty body gives one point.
#[must_use]
pub fn owner_document_points(owner: &Project, doc: &OwnerDocument) -> Vec<ConceptPoint> {
    let heading = format!("{} {} ({})", owner.slug, doc.title, doc.kind);
    let mut sections = chunk_markdown(&doc.body, MAX_CHUNK_CHARS);
    if sections.is_empty() {
        sections.push(String::new());
    }
    sections
        .into_iter()
        .enumerate()
        .map(|(chunk, section)| ConceptPoint {
            subject: Subject::OwnerDocument { id: doc.id },
            key: owner.slug.clone(),
            project_id: doc.project_id,
            updated_at: doc.created_at,
            document: Some(DocumentRef {
                id: doc.id,
                kind: doc.kind.clone(),
                version: doc.version,
                chunk: u32::try_from(chunk).unwrap_or(u32::MAX),
            }),
            text: format!("{heading}\n\n{section}").trim_end().to_owned(),
        })
        .collect()
}

/// Splits markdown at `#` and `##` headings, then splits any section longer than `max` characters
/// at line boundaries (a single longer line is kept whole rather than cut mid-word). Blank
/// sections are dropped.
#[must_use]
pub fn chunk_markdown(text: &str, max: usize) -> Vec<String> {
    let mut sections: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let heading = line.starts_with("# ") || line.starts_with("## ");
        let too_long = current.chars().count() + line.chars().count() + 1 > max;
        if (heading || too_long) && !current.trim().is_empty() {
            sections.push(std::mem::take(&mut current));
        } else if heading {
            current.clear();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        sections.push(current);
    }
    sections
        .into_iter()
        .map(|s| s.trim_end().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::{MemVectorStore, SearchQuery};
    use chrono::Utc;
    use htui_core::fixtures::ids;
    use htui_core::model::document::GOALS_KIND;
    use htui_core::model::{
        CoverageRow, DocumentHead, ItemCitation, ItemPatch, ItemSummary, LinkGraph, MemoryEntry,
        MemoryEntryId, MemoryRevision, MemoryState, NewDocument, NewItem, NewOwnerDocument, Note,
        ProjectChanges, PromptScope, RequirementArea, RequirementPatch, RequirementRevision,
        RequirementSpec, RequirementState, Resolution, ResolvedInput, Run, RunId, RunPlan, RunStep,
        RunStepCommit, RunStepTree, RunSummary, SessionEvent, Status, StepId, ToolCallCount,
        UpstreamEntry, WaitingCandidate, WorkspaceId,
    };
    use htui_core::store::{IndexChangesStore, MemStore, Result as StoreResult, WriteStore};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn item_of(p: &ConceptPoint) -> Option<ItemId> {
        p.subject.owner().item()
    }

    fn scope(projects: Vec<ProjectId>) -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: projects,
        }
    }

    async fn item_with_documents(read: &MemStore) -> Item {
        let summaries = read
            .items(&scope(vec![ids::PROJECT_HTUI]), &ItemFilter::default())
            .await
            .unwrap();
        for s in summaries {
            if !read.documents(s.id).await.unwrap().is_empty() {
                return read.item(s.id).await.unwrap().unwrap();
            }
        }
        panic!("the demo fixture has an htui item with documents");
    }

    #[test]
    fn markdown_splits_at_headings_and_length() {
        let text = "intro\n# Title\nbody\n## One\na\n### Deeper stays\nb\n## Two\nc";
        assert_eq!(
            chunk_markdown(text, 1000),
            [
                "intro",
                "# Title\nbody",
                "## One\na\n### Deeper stays\nb",
                "## Two\nc"
            ]
        );
        let long = "line of text\n".repeat(10);
        let parts = chunk_markdown(&long, 40);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.chars().count() <= 40));
        assert!(chunk_markdown("\n\n  \n", 100).is_empty());
    }

    #[tokio::test]
    async fn first_sync_indexes_items_and_their_latest_documents() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let report = Indexer::sync(&read, &scope(vec![ids::PROJECT_HTUI]), &store, &Unguarded)
            .await
            .unwrap();
        let items = read
            .items(&scope(vec![ids::PROJECT_HTUI]), &ItemFilter::default())
            .await
            .unwrap();
        assert_eq!(report.items_rebuilt, items.len());
        assert_eq!(report.items_unchanged, 0);
        let points = store.points();
        assert_eq!(report.points_upserted, points.len());
        assert!(points.iter().all(|p| p.project_id == ids::PROJECT_HTUI));
        assert_eq!(
            points
                .iter()
                .filter(|p| p.point_type() == PointType::Item)
                .count(),
            items.len()
        );
        let with_docs = item_with_documents(&read).await;
        let latest = latest_documents(&read, with_docs.id).await.unwrap();
        let indexed_docs: BTreeSet<DocumentId> = points
            .iter()
            .filter(|p| item_of(p) == Some(with_docs.id))
            .filter_map(|p| p.document.as_ref().map(|d| d.id))
            .collect();
        assert_eq!(indexed_docs, latest);
    }

    #[tokio::test]
    async fn a_second_sync_with_no_change_writes_nothing() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let before = store.upserted();
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.items_rebuilt, 0);
        assert_eq!(report.points_upserted, 0);
        assert_eq!(report.points_deleted, 0);
        assert_eq!(store.upserted(), before);
    }

    #[tokio::test]
    async fn an_edited_item_is_rebuilt_alone() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let item = item_with_documents(&read).await;
        read.update_item(
            item.id,
            item.version,
            ItemPatch {
                body: Some("rewritten body about vector search".into()),
                author_id: ids::USER,
                ..ItemPatch::default()
            },
        )
        .await
        .unwrap();
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.items_rebuilt, 1);
        let text = store
            .points()
            .into_iter()
            .find(|p| item_of(p) == Some(item.id) && p.point_type() == PointType::Item)
            .unwrap()
            .text;
        assert!(text.contains("rewritten body about vector search"));
    }

    #[tokio::test]
    async fn a_status_move_reaches_the_document_points() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let item = item_with_documents(&read).await;
        let to = Status::ALL
            .iter()
            .copied()
            .find(|&to| to != item.status && htui_core::store::legal_move(item.status, to).is_ok())
            .expect("the demo item can move");
        assert!(read.transition(item.id, item.status, to).await.unwrap());
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.items_rebuilt, 1);
        assert!(
            store
                .points()
                .iter()
                .filter(|p| item_of(p) == Some(item.id))
                .all(|p| matches!(p.subject, Subject::Item { status, .. } if status == to))
        );
    }

    #[tokio::test]
    async fn a_new_document_version_replaces_the_old_one() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let item = item_with_documents(&read).await;
        let old = read.documents(item.id).await.unwrap().remove(0);
        let new = read
            .write_document(NewDocument {
                id: DocumentId::new(),
                item_id: item.id,
                kind: old.kind.clone(),
                title: "Revised".into(),
                body: "## Verdict\nuse qdrant".into(),
                produced_by_step_id: None,
                created_by: ids::USER,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.items_rebuilt, 1);
        let docs: BTreeSet<DocumentId> = store
            .points()
            .iter()
            .filter_map(|p| p.document.as_ref().map(|d| d.id))
            .collect();
        assert!(docs.contains(&new.id));
        assert!(!docs.contains(&old.id));
    }

    #[tokio::test]
    async fn points_of_unlisted_items_are_deleted_and_other_projects_kept() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        Indexer::sync(
            &read,
            &scope(vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]),
            &store,
            &Unguarded,
        )
        .await
        .unwrap();
        let agy_before = store
            .points()
            .iter()
            .filter(|p| p.project_id == ids::PROJECT_AGY)
            .count();
        // An item the store does not have, as if it had been removed since the last sync.
        let mut ghost = item_point(&item_with_documents(&read).await);
        let ghost_id = ItemId::new();
        if let Subject::Item { id, .. } = &mut ghost.subject {
            *id = ghost_id;
        }
        store.upsert(vec![ghost.clone()]).await.unwrap();

        let report = Indexer::sync(&read, &scope(vec![ids::PROJECT_HTUI]), &store, &Unguarded)
            .await
            .unwrap();
        assert_eq!(report.points_deleted, 1);
        assert!(store.points().iter().all(|p| item_of(p) != Some(ghost_id)));
        let agy_after = store
            .points()
            .iter()
            .filter(|p| p.project_id == ids::PROJECT_AGY)
            .count();
        assert_eq!(agy_before, agy_after);
    }

    fn requirement_points(store: &MemVectorStore) -> Vec<ConceptPoint> {
        store
            .points()
            .into_iter()
            .filter(|p| p.point_type() == PointType::Requirement)
            .collect()
    }

    #[tokio::test]
    async fn requirements_get_one_point_each_and_a_second_sync_writes_nothing() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        let first = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let rows = read
            .requirements(ids::PROJECT_HTUI, &RequirementFilter::default())
            .await
            .unwrap();
        assert!(!rows.is_empty(), "the demo fixture has htui requirements");
        assert_eq!(first.requirements_rebuilt, rows.len());
        let points = requirement_points(&store);
        assert_eq!(points.len(), rows.len());
        for row in &rows {
            let p = points
                .iter()
                .find(|p| p.subject.owner().requirement() == Some(row.id))
                .expect("a point per row");
            assert_eq!(p.key, row.key);
            assert!(p.text.starts_with(&format!("{} {}", row.key, row.body)));
            assert!(p.text.contains(&row.rationale));
        }

        let second = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(second.requirements_rebuilt, 0);
        assert_eq!(second.requirements_unchanged, rows.len());
        assert_eq!(second.points_upserted, 0);
        assert_eq!(second.points_deleted, 0);
    }

    #[tokio::test]
    async fn an_amended_requirement_is_rebuilt_alone() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let row = read.requirement(ids::REQ_STO_1).await.unwrap().unwrap();
        read.amend_requirement(
            row.id,
            row.version,
            RequirementPatch {
                body: Some("Search finds requirements by meaning.".into()),
                rationale: None,
                priority: None,
                author_id: ids::USER,
                box_id: Some(ids::BOX),
                reason: "amended".into(),
            },
            ids::HTUI_FEAT_3,
        )
        .await
        .unwrap();
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.requirements_rebuilt, 1);
        assert_eq!(report.items_rebuilt, 0);
        assert_eq!(report.points_upserted, 1);
        let p = requirement_points(&store)
            .into_iter()
            .find(|p| p.subject.owner().requirement() == Some(row.id))
            .unwrap();
        assert!(p.text.contains("Search finds requirements by meaning."));
        assert!(
            matches!(p.subject, Subject::Requirement { version, .. } if version == row.version + 1)
        );
    }

    #[tokio::test]
    async fn a_withdrawn_requirement_keeps_its_point_with_its_state() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let row = read.requirement(ids::REQ_STO_1).await.unwrap().unwrap();
        read.withdraw_requirement(
            row.id,
            row.version,
            ids::HTUI_FEAT_3,
            ids::USER,
            Some(ids::BOX),
        )
        .await
        .unwrap();
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.requirements_rebuilt, 1);
        let p = requirement_points(&store)
            .into_iter()
            .find(|p| p.subject.owner().requirement() == Some(row.id))
            .expect("a withdrawn requirement stays indexed");
        assert!(matches!(
            p.subject,
            Subject::Requirement {
                state: RequirementState::Withdrawn,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn points_of_requirements_no_longer_listed_are_deleted() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let row = read.requirement(ids::REQ_STO_1).await.unwrap().unwrap();
        let mut ghost = requirement_point(&row);
        if let Subject::Requirement { id, .. } = &mut ghost.subject {
            *id = RequirementId::new();
        }
        store.upsert(vec![ghost.clone()]).await.unwrap();
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.points_deleted, 1);
        assert!(store.points().iter().all(|p| p.id() != ghost.id()));
    }

    async fn write_owner_document(
        read: &MemStore,
        project: ProjectId,
        kind: &str,
        title: &str,
        body: &str,
    ) -> OwnerDocument {
        read.write_owner_document(NewOwnerDocument {
            id: DocumentId::new(),
            project_id: project,
            kind: kind.into(),
            title: title.into(),
            body: body.into(),
            created_by: ids::USER,
            created_at: Utc::now(),
        })
        .await
        .unwrap()
    }

    fn owner_document_points(store: &MemVectorStore) -> Vec<ConceptPoint> {
        store
            .points()
            .into_iter()
            .filter(|p| matches!(p.subject, Subject::OwnerDocument { .. }))
            .collect()
    }

    fn indexed_documents(points: &[ConceptPoint]) -> BTreeSet<DocumentId> {
        points
            .iter()
            .filter_map(|p| p.document.as_ref().map(|d| d.id))
            .collect()
    }

    /// MOD-147 M4 D4, D5: each kind's newest version only, keyed by the owner's slug, headed
    /// `"{slug} {title} ({kind})"`; an empty body is one point.
    #[tokio::test]
    async fn the_first_sync_indexes_each_kinds_newest_owner_document_keyed_by_the_slug() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        assert!(
            read.owner_documents(ids::PROJECT_HTUI)
                .await
                .unwrap()
                .is_empty(),
            "the demo htui project owns no document"
        );
        let p = ids::PROJECT_HTUI;
        let v1 = write_owner_document(&read, p, GOALS_KIND, "Goals", "## Aim\nfirst").await;
        let v2 = write_owner_document(
            &read,
            p,
            GOALS_KIND,
            "Goals",
            "## Aim\nsecond\n## Next\nmore",
        )
        .await;
        let notes = write_owner_document(&read, p, "notes", "Notes", "").await;
        let report = Indexer::sync(&read, &scope(vec![p]), &store, &Unguarded)
            .await
            .unwrap();

        let points = owner_document_points(&store);
        assert_eq!(
            indexed_documents(&points),
            BTreeSet::from([v2.id, notes.id])
        );
        assert!(!indexed_documents(&points).contains(&v1.id));
        assert_eq!(points.len(), 3, "two goals sections, one empty notes point");
        let slug = read.project(p).await.unwrap().unwrap().slug;
        for point in &points {
            let doc = point
                .document
                .as_ref()
                .expect("an owner document is a document point");
            assert_eq!(point.key, slug);
            assert_eq!(point.project_id, p);
            assert_eq!(point.point_type(), PointType::Document);
            assert_eq!(point.subject.owner(), Owner::Document(doc.id));
            assert_eq!(
                point.id(),
                crate::vector::document_point_id(doc.id, doc.chunk)
            );
        }
        let goals = points
            .iter()
            .find(|q| {
                q.document
                    .as_ref()
                    .is_some_and(|d| d.id == v2.id && d.chunk == 0)
            })
            .unwrap();
        assert_eq!(
            goals.text,
            format!("{slug} Goals (goals)\n\n## Aim\nsecond")
        );
        assert_eq!(goals.updated_at, v2.created_at);
        assert_eq!(goals.document.as_ref().unwrap().version, 2);
        let empty = points
            .iter()
            .find(|q| q.document.as_ref().is_some_and(|d| d.id == notes.id))
            .unwrap();
        assert_eq!(empty.text, format!("{slug} Notes (notes)"));
        assert_eq!(report.points_upserted, store.points().len());
    }

    /// MOD-147 M4 D5: an owner document in step is neither rewritten nor deleted.
    #[tokio::test]
    async fn a_second_sync_writes_and_deletes_no_owner_document_point() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        write_owner_document(
            &read,
            ids::PROJECT_HTUI,
            GOALS_KIND,
            "Goals",
            "## Aim\nship",
        )
        .await;
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let before = store.upserted();
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.points_upserted, 0);
        assert_eq!(report.points_deleted, 0);
        assert_eq!(store.upserted(), before);
        assert_eq!(owner_document_points(&store).len(), 1);
    }

    /// MOD-147 M4 D5: a new version's points replace the old version's.
    #[tokio::test]
    async fn a_new_owner_document_version_replaces_the_old_ones_points() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        let p = ids::PROJECT_HTUI;
        let old = write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\none\n## B\ntwo").await;
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(owner_document_points(&store).len(), 2);
        let new = write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\nrewritten").await;
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.items_rebuilt, 0);
        assert_eq!(report.points_upserted, 1, "the new version's one section");
        assert_eq!(report.points_deleted, 2, "the old version's two sections");
        let points = owner_document_points(&store);
        assert_eq!(indexed_documents(&points), BTreeSet::from([new.id]));
        assert!(!indexed_documents(&points).contains(&old.id));
    }

    /// MOD-147 M4 R1 L1: a newest owner document missing one of its sections' points (a failed
    /// upsert on an earlier sync) is not in step: the next sync writes all of its sections again
    /// and deletes none of them.
    #[tokio::test]
    async fn a_half_indexed_owner_document_is_rebuilt_whole() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        let goals = write_owner_document(
            &read,
            ids::PROJECT_HTUI,
            GOALS_KIND,
            "Goals",
            "## A\none\n## B\ntwo",
        )
        .await;
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(owner_document_points(&store).len(), 2);
        store
            .delete(vec![crate::vector::document_point_id(goals.id, 1)])
            .await
            .unwrap();
        assert_eq!(owner_document_points(&store).len(), 1);

        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(
            report.points_upserted, 2,
            "both sections, rewritten together"
        );
        assert_eq!(report.points_deleted, 0);
        let points = owner_document_points(&store);
        assert_eq!(points.len(), 2);
        assert_eq!(indexed_documents(&points), BTreeSet::from([goals.id]));

        let again = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(again.points_upserted, 0, "whole again, so in step");
    }

    /// MOD-147 M4 R1 (fix-round verifier, low): a newest owner document holding more points than
    /// it has sections (a chunking change without a payload stamp bump) is rebuilt once and loses
    /// the extra points, rather than being rewritten on every sync with the extras kept.
    #[tokio::test]
    async fn an_owner_document_with_extra_points_loses_them_and_settles() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        let goals = write_owner_document(
            &read,
            ids::PROJECT_HTUI,
            GOALS_KIND,
            "Goals",
            "## A\none\n## B\ntwo",
        )
        .await;
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let mut extra = owner_document_points(&store)
            .into_iter()
            .next()
            .expect("an owner-document point");
        if let Some(doc) = extra.document.as_mut() {
            doc.chunk = 5;
        }
        store.upsert(vec![extra]).await.unwrap();
        assert_eq!(owner_document_points(&store).len(), 3);

        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.points_upserted, 2, "its two sections, rewritten");
        assert_eq!(report.points_deleted, 1, "the extra point");
        let points = owner_document_points(&store);
        assert_eq!(points.len(), 2);
        assert_eq!(indexed_documents(&points), BTreeSet::from([goals.id]));

        let again = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(again.points_upserted, 0, "in step from now on");
        assert_eq!(again.points_deleted, 0);
    }

    /// MOD-147 M4 D5: the item pass never takes an owner-document point for an item orphan.
    #[tokio::test]
    async fn an_item_sync_leaves_owner_document_points_alone() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        write_owner_document(
            &read,
            ids::PROJECT_HTUI,
            GOALS_KIND,
            "Goals",
            "## Aim\nship",
        )
        .await;
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let owned = owner_document_points(&store);
        let item = item_with_documents(&read).await;
        read.update_item(
            item.id,
            item.version,
            ItemPatch {
                body: Some("an edit beside an owner document".into()),
                author_id: ids::USER,
                ..ItemPatch::default()
            },
        )
        .await
        .unwrap();
        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.items_rebuilt, 1);
        assert_eq!(report.points_deleted, 0);
        assert_eq!(owner_document_points(&store), owned);
    }

    fn search(projects: &[ProjectId], text: &str) -> SearchQuery {
        SearchQuery {
            text: text.into(),
            projects: projects.to_vec(),
            types: vec![],
            statuses: vec![],
            resolutions: vec![],
            limit: 100,
        }
    }

    /// MOD-147 M4 D6: syncing the Platform workspace's search scope indexes its scope row's `WS`
    /// items and its goals document, which a search of the widened projects finds and one of the
    /// members alone does not.
    #[tokio::test]
    async fn a_sync_over_the_workspace_search_scope_indexes_the_scope_row() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let ws = read
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_SCOPE_PLATFORM,
                kind_id: ids::KIND_SCOPE_PLATFORM_WS,
                title: "Cross-project zircon search".into(),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .unwrap();
        let goals = write_owner_document(
            &read,
            ids::PROJECT_SCOPE_PLATFORM,
            GOALS_KIND,
            "Goals",
            "## Aim\none zircon search for the platform",
        )
        .await;
        let members = scope(vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);
        let wide = workspace_search_scope(&read, &members).await;
        Indexer::sync(&read, &wide, &store, &Unguarded)
            .await
            .unwrap();

        let points = store.points();
        assert!(
            points
                .iter()
                .any(|p| item_of(p) == Some(ws.id) && p.key == ws.key && ws.key.starts_with("WS-")),
            "the scope row's WS item"
        );
        let goal = points
            .iter()
            .find(|p| p.subject.owner() == Owner::Document(goals.id))
            .expect("the workspace's goals document");
        assert_eq!(goal.key, "@platform");
        assert_eq!(goal.project_id, ids::PROJECT_SCOPE_PLATFORM);

        let found = |hits: &[crate::vector::Hit]| {
            (
                hits.iter().any(|h| h.owner == Owner::Item(ws.id)),
                hits.iter().any(|h| h.owner == Owner::Document(goals.id)),
            )
        };
        let wide_hits = store
            .search(&search(&wide.project_ids, "zircon"))
            .await
            .unwrap();
        assert_eq!(found(&wide_hits), (true, true));
        let member_hits = store
            .search(&search(&members.project_ids, "zircon"))
            .await
            .unwrap();
        assert_eq!(found(&member_hits), (false, false));
    }

    /// MOD-147 M4 D6: the scope row goes first; a nil workspace and one without a scope row keep
    /// the scope as asked.
    #[tokio::test]
    async fn the_workspace_search_scope_puts_the_scope_row_first_or_keeps_the_scope() {
        let read = MemStore::demo();
        let members = scope(vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);
        let wide = workspace_search_scope(&read, &members).await;
        assert_eq!(
            wide.project_ids,
            [
                ids::PROJECT_SCOPE_PLATFORM,
                ids::PROJECT_HTUI,
                ids::PROJECT_AGY
            ]
        );
        assert_eq!(wide.workspace_id, members.workspace_id);
        let nil = Scope {
            workspace_id: WorkspaceId::from_uuid(Uuid::nil()),
            project_ids: vec![ids::PROJECT_HTUI],
        };
        assert_eq!(workspace_search_scope(&read, &nil).await, nil);
        let rowless = Scope {
            workspace_id: WorkspaceId::from_uuid(Uuid::from_u128(0xdead)),
            project_ids: vec![ids::PROJECT_HTUI],
        };
        assert_eq!(workspace_search_scope(&read, &rowless).await, rowless);
    }

    /// A guard that answers `answer` and counts how often it was asked (MOD-159 D6).
    struct CountingGuard {
        answer: bool,
        asked: AtomicUsize,
    }

    impl CountingGuard {
        fn answering(answer: bool) -> Self {
            Self {
                answer,
                asked: AtomicUsize::new(0),
            }
        }

        fn asked(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }
    }

    impl DeleteGuard for CountingGuard {
        async fn before_delete(&self) -> Result<bool, StoreError> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            Ok(self.answer)
        }
    }

    /// Plants the point of an item the store does not have under `project`, as if the item had
    /// been deleted since the last sync; its ID.
    async fn plant_ghost_item(
        read: &MemStore,
        store: &MemVectorStore,
        project: ProjectId,
    ) -> ItemId {
        let mut ghost = item_point(&item_with_documents(read).await);
        let ghost_id = ItemId::new();
        if let Subject::Item { id, .. } = &mut ghost.subject {
            *id = ghost_id;
        }
        ghost.project_id = project;
        store.upsert(vec![ghost]).await.unwrap();
        ghost_id
    }

    /// MOD-159 D6: a guard answering `false` stops the pass before its delete batch: every
    /// point stays, and the report says the pass was abandoned.
    #[tokio::test]
    async fn a_refused_guard_deletes_nothing_and_marks_the_pass_abandoned() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let ghost = plant_ghost_item(&read, &store, ids::PROJECT_HTUI).await;
        let before = store.points();

        let guard = CountingGuard::answering(false);
        let report = Indexer::sync(&read, &s, &store, &guard).await.unwrap();
        assert!(report.abandoned);
        assert_eq!(report.points_deleted, 0);
        assert_eq!(guard.asked(), 1);
        assert_eq!(store.points(), before);
        assert!(store.points().iter().any(|p| item_of(p) == Some(ghost)));

        let report = Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        assert!(!report.abandoned);
        assert_eq!(
            report.points_deleted, 1,
            "the deleted item's point still goes"
        );
        assert!(store.points().iter().all(|p| item_of(p) != Some(ghost)));
    }

    /// MOD-159 D6: the guard is asked once per project with something to delete, never on a
    /// pass that only writes or does nothing, and a refusal stops the pass at once.
    #[tokio::test]
    async fn the_guard_is_asked_once_per_project_with_something_to_delete() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let both = scope(vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);
        let first = CountingGuard::answering(true);
        Indexer::sync(&read, &both, &store, &first).await.unwrap();
        assert_eq!(first.asked(), 0, "a first build deletes nothing");

        let item = item_with_documents(&read).await;
        read.update_item(
            item.id,
            item.version,
            ItemPatch {
                body: Some("an edit that only writes".into()),
                author_id: ids::USER,
                ..ItemPatch::default()
            },
        )
        .await
        .unwrap();
        let writes = CountingGuard::answering(true);
        let report = Indexer::sync(&read, &both, &store, &writes).await.unwrap();
        assert_eq!(report.items_rebuilt, 1);
        assert_eq!(
            writes.asked(),
            0,
            "a rebuild under the same point IDs deletes nothing"
        );

        plant_ghost_item(&read, &store, ids::PROJECT_HTUI).await;
        plant_ghost_item(&read, &store, ids::PROJECT_AGY).await;
        let refused = CountingGuard::answering(false);
        let report = Indexer::sync(&read, &both, &store, &refused).await.unwrap();
        assert!(report.abandoned);
        assert_eq!(refused.asked(), 1, "the refusal stops the pass at once");

        let granted = CountingGuard::answering(true);
        let report = Indexer::sync(&read, &both, &store, &granted).await.unwrap();
        assert!(!report.abandoned);
        assert_eq!(granted.asked(), 2);
        assert_eq!(report.points_deleted, 2);

        let idle = CountingGuard::answering(true);
        Indexer::sync(&read, &both, &store, &idle).await.unwrap();
        assert_eq!(idle.asked(), 0, "nothing to delete, nothing asked");
    }

    /// `MemStore` as a pass that listed before some rows existed: its first listing of each kind
    /// misses them, while a re-listing finds them (MOD-159 D6). A read by ID of the hidden item
    /// or requirement answers `None`, so only a re-listing can keep their points (review R1).
    struct Unlisted<'a> {
        inner: &'a MemStore,
        item: Option<ItemId>,
        requirement: Option<RequirementId>,
        document: Mutex<Option<DocumentId>>,
        items_listed: AtomicBool,
        requirements_listed: AtomicBool,
    }

    impl<'a> Unlisted<'a> {
        fn over(inner: &'a MemStore) -> Self {
            Self {
                inner,
                item: None,
                requirement: None,
                document: Mutex::new(None),
                items_listed: AtomicBool::new(false),
                requirements_listed: AtomicBool::new(false),
            }
        }
    }

    impl ReadStore for Unlisted<'_> {
        /// The first listing misses `item`; every later one is the store's.
        async fn items(&self, scope: &Scope, filter: &ItemFilter) -> StoreResult<Vec<ItemSummary>> {
            let mut rows = self.inner.items(scope, filter).await?;
            if !self.items_listed.swap(true, Ordering::SeqCst) {
                rows.retain(|row| Some(row.id) != self.item);
            }
            Ok(rows)
        }
        async fn item(&self, id: ItemId) -> StoreResult<Option<Item>> {
            if Some(id) == self.item {
                return Ok(None);
            }
            self.inner.item(id).await
        }
        async fn links(&self, id: ItemId, hops: u8) -> StoreResult<LinkGraph> {
            self.inner.links(id, hops).await
        }
        async fn documents(&self, id: ItemId) -> StoreResult<Vec<DocumentHead>> {
            self.inner.documents(id).await
        }
        async fn notes(&self, id: ItemId) -> StoreResult<Vec<Note>> {
            self.inner.notes(id).await
        }
        async fn runs(&self, id: ItemId) -> StoreResult<Vec<RunSummary>> {
            self.inner.runs(id).await
        }
        async fn step_events(&self, step: StepId) -> StoreResult<Option<Vec<SessionEvent>>> {
            self.inner.step_events(step).await
        }
        async fn document(&self, id: DocumentId) -> StoreResult<Option<Document>> {
            self.inner.document(id).await
        }
        async fn documents_of_kinds(
            &self,
            item: ItemId,
            kinds: &[String],
        ) -> StoreResult<Vec<Document>> {
            self.inner.documents_of_kinds(item, kinds).await
        }
        async fn upstream_summaries(
            &self,
            id: ItemId,
            hops: u8,
            scope: &PromptScope,
        ) -> StoreResult<Vec<UpstreamEntry>> {
            self.inner.upstream_summaries(id, hops, scope).await
        }
        async fn project(&self, id: ProjectId) -> StoreResult<Option<Project>> {
            self.inner.project(id).await
        }
        async fn run(&self, id: RunId) -> StoreResult<Option<Run>> {
            self.inner.run(id).await
        }
        async fn run_steps(&self, run: RunId) -> StoreResult<Vec<RunStep>> {
            self.inner.run_steps(run).await
        }
        async fn step_trees(&self, step: StepId) -> StoreResult<Vec<RunStepTree>> {
            self.inner.step_trees(step).await
        }
        async fn step_commits(&self, step: StepId) -> StoreResult<Vec<RunStepCommit>> {
            self.inner.step_commits(step).await
        }
        async fn resolve_inputs(
            &self,
            item: ItemId,
            run: RunId,
            kinds: &[String],
        ) -> StoreResult<Vec<ResolvedInput>> {
            self.inner.resolve_inputs(item, run, kinds).await
        }
        async fn requirement_spec(
            &self,
            project: ProjectId,
        ) -> StoreResult<Option<RequirementSpec>> {
            self.inner.requirement_spec(project).await
        }
        async fn requirement_areas(&self, project: ProjectId) -> StoreResult<Vec<RequirementArea>> {
            self.inner.requirement_areas(project).await
        }
        /// The first listing misses `requirement`; every later one is the store's.
        async fn requirements(
            &self,
            project: ProjectId,
            filter: &RequirementFilter,
        ) -> StoreResult<Vec<Requirement>> {
            let mut rows = self.inner.requirements(project, filter).await?;
            if !self.requirements_listed.swap(true, Ordering::SeqCst) {
                rows.retain(|row| Some(row.id) != self.requirement);
            }
            Ok(rows)
        }
        async fn requirement(&self, id: RequirementId) -> StoreResult<Option<Requirement>> {
            if Some(id) == self.requirement {
                return Ok(None);
            }
            self.inner.requirement(id).await
        }
        async fn requirement_revisions(
            &self,
            id: RequirementId,
        ) -> StoreResult<Option<Vec<RequirementRevision>>> {
            self.inner.requirement_revisions(id).await
        }
        async fn item_requirements(&self, item: ItemId) -> StoreResult<Vec<ItemCitation>> {
            self.inner.item_requirements(item).await
        }
        async fn requirement_coverage(
            &self,
            requirement: RequirementId,
        ) -> StoreResult<Vec<CoverageRow>> {
            self.inner.requirement_coverage(requirement).await
        }
        async fn requirement_refinements(
            &self,
            id: RequirementId,
        ) -> StoreResult<Vec<Requirement>> {
            self.inner.requirement_refinements(id).await
        }
        async fn tool_call_counts(&self, item: ItemId) -> StoreResult<Vec<ToolCallCount>> {
            self.inner.tool_call_counts(item).await
        }
        async fn waiting_candidates(&self, scope: &Scope) -> StoreResult<Vec<WaitingCandidate>> {
            self.inner.waiting_candidates(scope).await
        }
        async fn memory_entries(
            &self,
            project: ProjectId,
            states: Option<Vec<MemoryState>>,
        ) -> StoreResult<Option<Vec<MemoryEntry>>> {
            self.inner.memory_entries(project, states).await
        }
        async fn memory_revisions(
            &self,
            id: MemoryEntryId,
        ) -> StoreResult<Option<Vec<MemoryRevision>>> {
            self.inner.memory_revisions(id).await
        }
        async fn scope_project(&self, workspace: WorkspaceId) -> StoreResult<Option<ProjectId>> {
            self.inner.scope_project(workspace).await
        }
        /// The first listing misses `document`; every later one is the store's.
        async fn owner_documents(&self, project: ProjectId) -> StoreResult<Vec<OwnerDocument>> {
            let mut rows = self.inner.owner_documents(project).await?;
            let hidden = self.document.lock().unwrap().take();
            rows.retain(|row| Some(row.id) != hidden);
            Ok(rows)
        }
        async fn run_plans(&self, run: RunId) -> StoreResult<Vec<RunPlan>> {
            self.inner.run_plans(run).await
        }
    }

    /// MOD-159 D6: an item created after the listing, whose points another indexer already
    /// wrote, is the project's on a re-listing, so its points stay for the next pass. Nothing
    /// else was to go, so the guard is never asked (review R1: it is asked after the
    /// confirmation, only for a batch that is not empty).
    #[tokio::test]
    async fn an_item_created_after_the_listing_keeps_its_points() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let item = item_with_documents(&read).await;
        let before = store.points();
        assert!(
            before
                .iter()
                .filter(|p| item_of(p) == Some(item.id))
                .count()
                > 1
        );

        let late = Unlisted {
            item: Some(item.id),
            ..Unlisted::over(&read)
        };
        let guard = CountingGuard::answering(true);
        let report = Indexer::sync(&late, &s, &store, &guard).await.unwrap();
        assert_eq!(
            guard.asked(),
            0,
            "the re-listing kept its points, so nothing is to go"
        );
        assert_eq!(report.points_deleted, 0);
        assert_eq!(store.points(), before);
    }

    /// MOD-159 D6: as an item, a requirement created after the listing.
    #[tokio::test]
    async fn a_requirement_created_after_the_listing_keeps_its_point() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let before = store.points();
        assert!(
            before
                .iter()
                .any(|p| p.subject.owner().requirement() == Some(ids::REQ_STO_1))
        );

        let late = Unlisted {
            requirement: Some(ids::REQ_STO_1),
            ..Unlisted::over(&read)
        };
        let guard = CountingGuard::answering(true);
        let report = Indexer::sync(&late, &s, &store, &guard).await.unwrap();
        assert_eq!(guard.asked(), 0, "nothing is to go");
        assert_eq!(report.points_deleted, 0);
        assert_eq!(store.points(), before);
    }

    /// MOD-159 D6: an owner document that became the newest of its kind after the listing keeps
    /// its points; one the re-listing shows superseded loses them.
    #[tokio::test]
    async fn an_owner_document_newest_after_the_listing_keeps_its_points() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        let p = ids::PROJECT_HTUI;
        let v1 = write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\none\n## B\ntwo").await;
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let before = store.points();

        let late = Unlisted {
            document: Mutex::new(Some(v1.id)),
            ..Unlisted::over(&read)
        };
        let guard = CountingGuard::answering(true);
        let report = Indexer::sync(&late, &s, &store, &guard).await.unwrap();
        assert_eq!(guard.asked(), 0, "nothing is to go");
        assert_eq!(report.points_deleted, 0);
        assert_eq!(store.points(), before);

        // v2 now exists and v1's points are again unlisted: the re-listing shows v1 superseded.
        let v2 = write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\nrewritten").await;
        let late = Unlisted {
            document: Mutex::new(Some(v1.id)),
            ..Unlisted::over(&read)
        };
        let report = Indexer::sync(&late, &s, &store, &Unguarded).await.unwrap();
        assert_eq!(report.points_deleted, 2, "v1's two sections");
        let points = owner_document_points(&store);
        assert_eq!(indexed_documents(&points), BTreeSet::from([v2.id]));
    }

    /// MOD-159 D6 (review R1): a point of a real item held under another project, as when the
    /// item moved there, is an orphan of the project it names: that project's listing misses the
    /// item, so the point goes.
    #[tokio::test]
    async fn an_item_moved_to_another_project_loses_its_points() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let item = item_with_documents(&read).await;
        assert_eq!(item.project_id, ids::PROJECT_HTUI);
        let mut moved = item_point(&item);
        moved.project_id = ids::PROJECT_AGY;
        store.upsert(vec![moved]).await.unwrap();

        let guard = CountingGuard::answering(true);
        let report = Indexer::sync(&read, &scope(vec![ids::PROJECT_AGY]), &store, &guard)
            .await
            .unwrap();
        assert_eq!(guard.asked(), 1);
        assert_eq!(report.points_deleted, 1, "{report:?}");
        assert!(store.points().iter().all(|p| item_of(p) != Some(item.id)));
    }

    #[tokio::test]
    async fn a_closed_item_carries_its_resolution_on_every_point() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let fix = read.item(ids::HTUI_FIX_1).await.unwrap().unwrap();
        assert_eq!(fix.status, Status::Closed);
        // The demo's closed item has no documents; give it one, so the document half is pinned.
        read.write_document(NewDocument {
            id: DocumentId::new(),
            item_id: fix.id,
            kind: "summary".into(),
            title: "Summary".into(),
            body: "## Verdict\nrestore the terminal".into(),
            produced_by_step_id: None,
            created_by: ids::USER,
            created_at: Utc::now(),
        })
        .await
        .unwrap();
        Indexer::sync(&read, &scope(vec![ids::PROJECT_HTUI]), &store, &Unguarded)
            .await
            .unwrap();
        let points: Vec<_> = store
            .points()
            .into_iter()
            .filter(|p| item_of(p) == Some(fix.id))
            .collect();
        assert!(
            points.iter().any(|p| p.point_type() == PointType::Document),
            "{points:?}"
        );
        assert!(points.iter().all(|p| matches!(
            p.subject,
            Subject::Item {
                resolution: Some(Resolution::Done),
                ..
            }
        )));
        assert!(
            store
                .points()
                .iter()
                .filter(|p| matches!(p.subject, Subject::Item { status, .. } if status != Status::Closed))
                .all(|p| matches!(p.subject, Subject::Item { resolution: None, .. }))
        );
    }

    // MOD-159 M2 D7: the incremental pass.

    /// The store's clock now: the `since` of a change read covering every write after it.
    async fn clock(read: &MemStore) -> chrono::DateTime<Utc> {
        read.index_clock().await.unwrap()
    }

    /// What `read` says changed in `projects` after `since`.
    async fn changes_since(
        read: &MemStore,
        projects: &[ProjectId],
        since: chrono::DateTime<Utc>,
    ) -> IndexChanges {
        read.index_changes(projects, since).await.unwrap()
    }

    /// A hand-built change set: `project` with these rows.
    fn listing(
        project: ProjectId,
        items: &[ItemId],
        requirements: &[RequirementId],
        owner_documents: &[DocumentId],
    ) -> IndexChanges {
        IndexChanges {
            projects: BTreeMap::from([(
                project,
                ProjectChanges {
                    items: items.iter().copied().collect(),
                    requirements: requirements.iter().copied().collect(),
                    owner_documents: owner_documents.iter().copied().collect(),
                },
            )]),
        }
    }

    async fn mint_htui_item(read: &MemStore, title: &str) -> Item {
        read.mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            title: title.into(),
            body: "a body about incremental indexing".into(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .unwrap()
    }

    async fn new_version_of(read: &MemStore, item: ItemId, kind: &str, body: &str) -> Document {
        read.write_document(NewDocument {
            id: DocumentId::new(),
            item_id: item,
            kind: kind.into(),
            title: "Revised".into(),
            body: body.into(),
            produced_by_step_id: None,
            created_by: ids::USER,
            created_at: Utc::now(),
        })
        .await
        .unwrap()
    }

    async fn edit_body(read: &MemStore, id: ItemId, body: &str) {
        let item = read.item(id).await.unwrap().unwrap();
        read.update_item(
            item.id,
            item.version,
            ItemPatch {
                body: Some(body.into()),
                author_id: ids::USER,
                ..ItemPatch::default()
            },
        )
        .await
        .unwrap();
    }

    fn item_text(store: &MemVectorStore, id: ItemId) -> String {
        store
            .points()
            .into_iter()
            .find(|p| item_of(p) == Some(id) && p.point_type() == PointType::Item)
            .expect("the item has its point")
            .text
    }

    #[tokio::test]
    async fn sync_changes_indexes_a_new_item_and_its_documents() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let since = clock(&read).await;
        let item = mint_htui_item(&read, "Incremental index").await;
        let doc = new_version_of(&read, item.id, "summary", "## One\na\n## Two\nb").await;
        let changes = changes_since(&read, &s.project_ids, since).await;

        let guard = CountingGuard::answering(true);
        let report = Indexer::sync_changes(&read, &s, &changes, &store, &guard)
            .await
            .unwrap();
        assert_eq!(report.items_rebuilt, 1, "{report:?}");
        assert_eq!(report.points_upserted, 3, "the item and two sections");
        assert_eq!(report.points_deleted, 0);
        assert_eq!(guard.asked(), 0, "nothing is to go");
        let points: Vec<_> = store
            .points()
            .into_iter()
            .filter(|p| item_of(p) == Some(item.id))
            .collect();
        assert_eq!(points.len(), 3);
        assert_eq!(indexed_documents(&points), BTreeSet::from([doc.id]));
    }

    #[tokio::test]
    async fn sync_changes_rebuilds_only_the_listed_items() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let rows = read.items(&s, &ItemFilter::default()).await.unwrap();
        let (listed, unlisted) = (rows[0].id, rows[1].id);
        let unlisted_before = item_text(&store, unlisted);
        edit_body(&read, listed, "the listed edit").await;
        edit_body(&read, unlisted, "the unlisted edit").await;

        let report = Indexer::sync_changes(
            &read,
            &s,
            &listing(ids::PROJECT_HTUI, &[listed], &[], &[]),
            &store,
            &Unguarded,
        )
        .await
        .unwrap();
        assert_eq!(report.items_rebuilt, 1, "{report:?}");
        assert_eq!(report.requirements_rebuilt, 0);
        assert!(item_text(&store, listed).contains("the listed edit"));
        assert_eq!(
            item_text(&store, unlisted),
            unlisted_before,
            "an unlisted stale item waits for its own listing or the full reconcile"
        );
    }

    #[tokio::test]
    async fn a_new_document_version_reaches_the_index_through_its_item() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let item = item_with_documents(&read).await;
        let old = read.documents(item.id).await.unwrap().remove(0);
        let since = clock(&read).await;
        let new = new_version_of(&read, item.id, &old.kind, "## Verdict\nuse qdrant").await;
        let changes = changes_since(&read, &s.project_ids, since).await;
        assert_eq!(
            changes.projects[&ids::PROJECT_HTUI].items,
            BTreeSet::from([item.id]),
            "a document version lists its item"
        );

        let guard = CountingGuard::answering(true);
        let report = Indexer::sync_changes(&read, &s, &changes, &store, &guard)
            .await
            .unwrap();
        assert_eq!(report.items_rebuilt, 1);
        assert!(report.points_deleted > 0, "{report:?}");
        assert_eq!(guard.asked(), 1);
        let points: Vec<_> = store
            .points()
            .into_iter()
            .filter(|p| item_of(p) == Some(item.id))
            .collect();
        let docs = indexed_documents(&points);
        assert!(docs.contains(&new.id));
        assert_eq!(docs, latest_documents(&read, item.id).await.unwrap());
    }

    #[tokio::test]
    async fn sync_changes_rebuilds_an_amended_requirement() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let row = read.requirement(ids::REQ_STO_1).await.unwrap().unwrap();
        let since = clock(&read).await;
        read.amend_requirement(
            row.id,
            row.version,
            RequirementPatch {
                body: Some("Search finds requirements incrementally.".into()),
                rationale: None,
                priority: None,
                author_id: ids::USER,
                box_id: Some(ids::BOX),
                reason: "amended".into(),
            },
            ids::HTUI_FEAT_3,
        )
        .await
        .unwrap();
        let changes = changes_since(&read, &s.project_ids, since).await;

        let report = Indexer::sync_changes(&read, &s, &changes, &store, &Unguarded)
            .await
            .unwrap();
        assert_eq!(report.requirements_rebuilt, 1, "{report:?}");
        assert_eq!(
            report.requirements_unchanged, 0,
            "only the listed row is read"
        );
        let p = requirement_points(&store)
            .into_iter()
            .find(|p| p.subject.owner().requirement() == Some(row.id))
            .unwrap();
        assert!(p.text.contains("Search finds requirements incrementally."));
        assert!(
            matches!(p.subject, Subject::Requirement { version, .. } if version == row.version + 1)
        );
    }

    #[tokio::test]
    async fn sync_changes_replaces_a_superseded_owner_document() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let p = ids::PROJECT_HTUI;
        let s = scope(vec![p]);
        let v1 = write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\none\n## B\ntwo").await;
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        // An owner document the store does not have: an orphan, the full reconcile's to delete.
        let owner = read.project(p).await.unwrap().unwrap();
        let ghost = OwnerDocument {
            id: DocumentId::new(),
            kind: "notes".into(),
            ..v1.clone()
        };
        store
            .upsert(super::owner_document_points(&owner, &ghost))
            .await
            .unwrap();
        let since = clock(&read).await;
        let v2 = write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\nrewritten").await;
        let changes = changes_since(&read, &s.project_ids, since).await;
        assert_eq!(
            changes.projects[&p].owner_documents,
            BTreeSet::from([v2.id])
        );

        let guard = CountingGuard::answering(true);
        let report = Indexer::sync_changes(&read, &s, &changes, &store, &guard)
            .await
            .unwrap();
        assert_eq!(report.points_deleted, 2, "v1's two sections, {report:?}");
        assert_eq!(guard.asked(), 1);
        let points = owner_document_points(&store);
        assert_eq!(
            indexed_documents(&points),
            BTreeSet::from([v2.id, ghost.id]),
            "the orphan stays for the full reconcile"
        );
    }

    #[tokio::test]
    async fn sync_changes_leaves_a_deleted_rows_points_to_the_full_reconcile() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let s = scope(vec![ids::PROJECT_HTUI]);
        Indexer::sync(&read, &s, &store, &Unguarded).await.unwrap();
        let ghost = plant_ghost_item(&read, &store, ids::PROJECT_HTUI).await;
        let before = store.points();

        let guard = CountingGuard::answering(true);
        let report = Indexer::sync_changes(
            &read,
            &s,
            &listing(ids::PROJECT_HTUI, &[ghost], &[RequirementId::new()], &[]),
            &store,
            &guard,
        )
        .await
        .unwrap();
        assert_eq!(report, SyncReport::default());
        assert_eq!(guard.asked(), 0);
        assert_eq!(store.points(), before);
    }

    #[tokio::test]
    async fn sync_changes_skips_an_item_listed_under_a_project_it_left() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let both = scope(vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);
        Indexer::sync(&read, &both, &store, &Unguarded)
            .await
            .unwrap();
        let item = item_with_documents(&read).await;
        let row = read.requirement(ids::REQ_STO_1).await.unwrap().unwrap();
        edit_body(&read, item.id, "an edit listed under the wrong project").await;
        let before = store.points();

        let guard = CountingGuard::answering(true);
        let report = Indexer::sync_changes(
            &read,
            &both,
            &listing(ids::PROJECT_AGY, &[item.id], &[row.id], &[]),
            &store,
            &guard,
        )
        .await
        .unwrap();
        assert_eq!(report, SyncReport::default());
        assert_eq!(guard.asked(), 0);
        assert_eq!(store.points(), before);
    }

    #[tokio::test]
    async fn a_refused_guard_stops_sync_changes_with_nothing_deleted() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let both = scope(vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);
        Indexer::sync(&read, &both, &store, &Unguarded)
            .await
            .unwrap();
        let item = item_with_documents(&read).await;
        let old = read.documents(item.id).await.unwrap().remove(0);
        let agy = read
            .items(&scope(vec![ids::PROJECT_AGY]), &ItemFilter::default())
            .await
            .unwrap()[0]
            .id;
        let held = |store: &MemVectorStore| {
            let points: Vec<_> = store
                .points()
                .into_iter()
                .filter(|p| item_of(p) == Some(item.id))
                .collect();
            indexed_documents(&points)
        };
        let before = held(&store);
        let since = clock(&read).await;
        new_version_of(&read, item.id, &old.kind, "## Verdict\nrefused").await;
        edit_body(&read, agy, "an agy edit after the refusal").await;
        let changes = changes_since(&read, &both.project_ids, since).await;
        assert_eq!(changes.projects.len(), 2);

        let guard = CountingGuard::answering(false);
        let report = Indexer::sync_changes(&read, &both, &changes, &store, &guard)
            .await
            .unwrap();
        assert!(report.abandoned);
        assert_eq!(report.points_deleted, 0);
        assert_eq!(guard.asked(), 1);
        let after = held(&store);
        assert!(
            before.is_subset(&after) && after.len() > before.len(),
            "the superseded version's points stay beside the new one's: {before:?} {after:?}"
        );
        assert!(
            !item_text(&store, agy).contains("after the refusal"),
            "no later project is visited"
        );
    }

    #[tokio::test]
    async fn after_sync_changes_a_full_sync_rebuilds_nothing() {
        let read = MemStore::demo();
        let store = MemVectorStore::new();
        let p = ids::PROJECT_HTUI;
        let both = scope(vec![p, ids::PROJECT_AGY]);
        write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\none").await;
        Indexer::sync(&read, &both, &store, &Unguarded)
            .await
            .unwrap();
        let since = clock(&read).await;

        mint_htui_item(&read, "Minted after the reconcile").await;
        let item = item_with_documents(&read).await;
        let old = read.documents(item.id).await.unwrap().remove(0);
        new_version_of(&read, item.id, &old.kind, "## Verdict\nnewer").await;
        let fix = read.item(ids::HTUI_FIX_1).await.unwrap().unwrap();
        edit_body(&read, fix.id, "an edited closed item").await;
        let agy = read
            .items(&scope(vec![ids::PROJECT_AGY]), &ItemFilter::default())
            .await
            .unwrap()[0]
            .clone();
        let to = Status::ALL
            .iter()
            .copied()
            .find(|&to| to != agy.status && htui_core::store::legal_move(agy.status, to).is_ok())
            .expect("the agy item can move");
        assert!(read.transition(agy.id, agy.status, to).await.unwrap());
        let row = read.requirement(ids::REQ_STO_1).await.unwrap().unwrap();
        read.withdraw_requirement(
            row.id,
            row.version,
            ids::HTUI_FEAT_3,
            ids::USER,
            Some(ids::BOX),
        )
        .await
        .unwrap();
        write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\ntwo\n## B\nthree").await;

        let changes = changes_since(&read, &both.project_ids, since).await;
        let report = Indexer::sync_changes(&read, &both, &changes, &store, &Unguarded)
            .await
            .unwrap();
        assert!(report.items_rebuilt >= 4, "{report:?}");
        assert_eq!(report.requirements_rebuilt, 1);

        let full = Indexer::sync(&read, &both, &store, &Unguarded)
            .await
            .unwrap();
        assert_eq!(full.items_rebuilt, 0, "{full:?}");
        assert_eq!(full.requirements_rebuilt, 0);
        assert_eq!(full.points_upserted, 0);
        assert_eq!(full.points_deleted, 0);
    }
}
