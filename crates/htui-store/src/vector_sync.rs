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
//! row is never updated (a new version is a new row), so a newest document whose points exist is
//! in step, one without points is built, and the points of a document no longer the newest of its
//! kind are deleted. A workspace-scoped search reaches the scope row through
//! [`workspace_search_scope`].
use crate::vector::{
    ConceptPoint, DocumentRef, IndexedPoint, Owner, PointType, Subject, VectorStore,
};
use htui_core::model::{
    Document, DocumentId, Item, ItemFilter, ItemId, OwnerDocument, Project, ProjectId, Requirement,
    RequirementFilter, RequirementId, Scope,
};
use htui_core::store::{ReadStore, StoreError};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

/// Longest section of a document embedded as one point, in characters. BGE-small reads about 512
/// tokens; 2000 characters of English prose stays under that.
pub const MAX_CHUNK_CHARS: usize = 2000;

/// Most requirement or owner-document points written in one upsert.
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
}

impl std::ops::AddAssign for SyncReport {
    fn add_assign(&mut self, other: Self) {
        self.items_rebuilt += other.items_rebuilt;
        self.items_unchanged += other.items_unchanged;
        self.requirements_rebuilt += other.requirements_rebuilt;
        self.requirements_unchanged += other.requirements_unchanged;
        self.points_upserted += other.points_upserted;
        self.points_deleted += other.points_deleted;
    }
}

/// Builds concept points from the store and reconciles the index with them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Indexer;

impl Indexer {
    /// Reconciles the index with every project of `scope`.
    pub async fn sync(
        read: &impl ReadStore,
        scope: &Scope,
        store: &impl VectorStore,
    ) -> Result<SyncReport, StoreError> {
        let mut report = SyncReport::default();
        for &project in &scope.project_ids {
            let one = Scope {
                workspace_id: scope.workspace_id,
                project_ids: vec![project],
            };
            let items = read.items(&one, &ItemFilter::default()).await?;
            let mut indexed: HashMap<ItemId, Vec<IndexedPoint>> = HashMap::new();
            let mut requirement_points: HashMap<RequirementId, Vec<IndexedPoint>> = HashMap::new();
            let mut owner_points: HashMap<DocumentId, Vec<IndexedPoint>> = HashMap::new();
            for point in store.indexed(project).await? {
                match point.owner {
                    Owner::Item(id) => indexed.entry(id).or_default().push(point),
                    Owner::Requirement(id) => requirement_points.entry(id).or_default().push(point),
                    Owner::Document(id) => owner_points.entry(id).or_default().push(point),
                }
            }

            for summary in items {
                let old = indexed.remove(&summary.id).unwrap_or_default();
                let latest = latest_documents(read, summary.id).await?;
                let item_fresh = old.iter().any(|p| {
                    p.point_type == PointType::Item
                        && p.updated_at == summary.updated_at
                        && p.status == Some(summary.status)
                });
                let old_docs: BTreeSet<DocumentId> =
                    old.iter().filter_map(|p| p.document_id).collect();
                if item_fresh && old_docs == latest {
                    report.items_unchanged += 1;
                    continue;
                }
                let Some(item) = read.item(summary.id).await? else {
                    continue; // gone between the listing and now; the next sync drops it
                };
                let mut points = vec![item_point(&item)];
                for id in &latest {
                    if let Some(doc) = read.document(*id).await? {
                        points.extend(document_points(&item, &doc));
                    }
                }
                let keep: BTreeSet<Uuid> = points.iter().map(ConceptPoint::id).collect();
                let stale: Vec<Uuid> = old
                    .iter()
                    .map(|p| p.id)
                    .filter(|id| !keep.contains(id))
                    .collect();
                report.items_rebuilt += 1;
                report.points_upserted += points.len();
                report.points_deleted += stale.len();
                store.upsert(points).await?;
                store.delete(stale).await?;
            }

            report += sync_requirements(read, project, requirement_points, store).await?;
            // MOD-147 M4 D5: before the item orphans go, and never among them.
            report += sync_owner_documents(read, project, owner_points, store).await?;

            let orphans: Vec<Uuid> = indexed.into_values().flatten().map(|p| p.id).collect();
            report.points_deleted += orphans.len();
            store.delete(orphans).await?;
        }
        Ok(report)
    }
}

/// The requirement half of one project's sync: one listing, stale points rebuilt in one upsert,
/// and points of rows the store no longer lists deleted.
async fn sync_requirements(
    read: &impl ReadStore,
    project: ProjectId,
    mut indexed: HashMap<RequirementId, Vec<IndexedPoint>>,
    store: &impl VectorStore,
) -> Result<SyncReport, StoreError> {
    let mut report = SyncReport::default();
    let mut points = Vec::new();
    let mut stale = Vec::new();
    for row in read
        .requirements(project, &RequirementFilter::default())
        .await?
    {
        let old = indexed.remove(&row.id).unwrap_or_default();
        let point = requirement_point(&row);
        let id = point.id();
        if old
            .iter()
            .any(|p| p.id == id && p.version == Some(row.version))
        {
            report.requirements_unchanged += 1;
            continue;
        }
        stale.extend(old.iter().map(|p| p.id).filter(|old| *old != id));
        report.requirements_rebuilt += 1;
        points.push(point);
    }
    stale.extend(indexed.into_values().flatten().map(|p| p.id));
    report.points_upserted += points.len();
    report.points_deleted += stale.len();
    // Bounded batches: a first build of a large spec would otherwise embed and send every
    // requirement of the project in one request.
    for batch in points.chunks(UPSERT_BATCH) {
        store.upsert(batch.to_vec()).await?;
    }
    store.delete(stale).await?;
    Ok(report)
}

/// The owner-document half of one project's sync (MOD-147 M4 D5): one listing, the newest version
/// of each kind built when it has no points yet, and the points of every other document deleted.
/// An owner document is never updated, so points under a newest document's ID are in step. When
/// a document is to be built and the owner row is gone, nothing is written for the project.
async fn sync_owner_documents(
    read: &impl ReadStore,
    project: ProjectId,
    mut indexed: HashMap<DocumentId, Vec<IndexedPoint>>,
    store: &impl VectorStore,
) -> Result<SyncReport, StoreError> {
    let mut report = SyncReport::default();
    let mut latest: BTreeMap<String, OwnerDocument> = BTreeMap::new();
    for row in read.owner_documents(project).await? {
        match latest.get(&row.kind) {
            Some(newest) if newest.version >= row.version => {}
            _ => {
                latest.insert(row.kind.clone(), row);
            }
        }
    }
    let build: Vec<OwnerDocument> = latest
        .into_values()
        .filter(|doc| indexed.remove(&doc.id).is_none())
        .collect();
    let mut points = Vec::new();
    if !build.is_empty() {
        let Some(owner) = read.project(project).await? else {
            return Ok(report); // gone since the listing; the next sync drops its points
        };
        for doc in &build {
            points.extend(owner_document_points(&owner, doc));
        }
    }
    let stale: Vec<Uuid> = indexed.into_values().flatten().map(|p| p.id).collect();
    report.points_upserted += points.len();
    report.points_deleted += stale.len();
    for batch in points.chunks(UPSERT_BATCH) {
        store.upsert(batch.to_vec()).await?;
    }
    store.delete(stale).await?;
    Ok(report)
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
        ItemPatch, NewDocument, NewItem, NewOwnerDocument, RequirementPatch, RequirementState,
        Resolution, Status, WorkspaceId,
    };
    use htui_core::store::{MemStore, WriteStore};

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
        let report = Indexer::sync(&read, &scope(vec![ids::PROJECT_HTUI]), &store)
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
        Indexer::sync(&read, &s, &store).await.unwrap();
        let before = store.upserted();
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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
        Indexer::sync(&read, &s, &store).await.unwrap();
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
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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
        Indexer::sync(&read, &s, &store).await.unwrap();
        let item = item_with_documents(&read).await;
        let to = Status::ALL
            .iter()
            .copied()
            .find(|&to| to != item.status && htui_core::store::legal_move(item.status, to).is_ok())
            .expect("the demo item can move");
        assert!(read.transition(item.id, item.status, to).await.unwrap());
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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
        Indexer::sync(&read, &s, &store).await.unwrap();
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
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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

        let report = Indexer::sync(&read, &scope(vec![ids::PROJECT_HTUI]), &store)
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
        let first = Indexer::sync(&read, &s, &store).await.unwrap();
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

        let second = Indexer::sync(&read, &s, &store).await.unwrap();
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
        Indexer::sync(&read, &s, &store).await.unwrap();
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
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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
        Indexer::sync(&read, &s, &store).await.unwrap();
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
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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
        Indexer::sync(&read, &s, &store).await.unwrap();
        let row = read.requirement(ids::REQ_STO_1).await.unwrap().unwrap();
        let mut ghost = requirement_point(&row);
        if let Subject::Requirement { id, .. } = &mut ghost.subject {
            *id = RequirementId::new();
        }
        store.upsert(vec![ghost.clone()]).await.unwrap();
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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
        let report = Indexer::sync(&read, &scope(vec![p]), &store).await.unwrap();

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
        Indexer::sync(&read, &s, &store).await.unwrap();
        let before = store.upserted();
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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
        Indexer::sync(&read, &s, &store).await.unwrap();
        assert_eq!(owner_document_points(&store).len(), 2);
        let new = write_owner_document(&read, p, GOALS_KIND, "Goals", "## A\nrewritten").await;
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
        assert_eq!(report.items_rebuilt, 0);
        assert_eq!(report.points_upserted, 1, "the new version's one section");
        assert_eq!(report.points_deleted, 2, "the old version's two sections");
        let points = owner_document_points(&store);
        assert_eq!(indexed_documents(&points), BTreeSet::from([new.id]));
        assert!(!indexed_documents(&points).contains(&old.id));
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
        Indexer::sync(&read, &s, &store).await.unwrap();
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
        let report = Indexer::sync(&read, &s, &store).await.unwrap();
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
        Indexer::sync(&read, &wide, &store).await.unwrap();

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
        Indexer::sync(&read, &scope(vec![ids::PROJECT_HTUI]), &store)
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
}
