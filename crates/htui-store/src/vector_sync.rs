//! Keeps the concepts index in step with Postgres (MOD-34 T4, `R-STO-8`).
//!
//! [`Indexer::sync`] is incremental and idempotent. Per project it lists the items, compares each
//! with what the index holds, and rebuilds only the items that moved: an item is stale when its
//! `updated_at` or its `status` differs from its item point (a status move does not touch
//! `updated_at`), or when the set of its latest documents differs from its document points.
//! Rebuilding an item rewrites all of its points, because every document point repeats the item's
//! status for the "decisions only" filter. Items the store no longer lists lose their points.
use crate::vector::{ConceptPoint, DocumentRef, IndexedPoint, PointType, VectorStore};
use htui_core::model::{Document, DocumentId, Item, ItemFilter, ItemId, Scope};
use htui_core::store::{ReadStore, StoreError};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

/// Longest section of a document embedded as one point, in characters. BGE-small reads about 512
/// tokens; 2000 characters of English prose stays under that.
pub const MAX_CHUNK_CHARS: usize = 2000;

/// What one [`Indexer::sync`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    /// Items whose points were (re)built.
    pub items_rebuilt: usize,
    /// Items already in step.
    pub items_unchanged: usize,
    /// Points written.
    pub points_upserted: usize,
    /// Points removed: stale sections, superseded documents, items no longer listed.
    pub points_deleted: usize,
}

impl std::ops::AddAssign for SyncReport {
    fn add_assign(&mut self, other: Self) {
        self.items_rebuilt += other.items_rebuilt;
        self.items_unchanged += other.items_unchanged;
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
            for point in store.indexed(project).await? {
                indexed.entry(point.item_id).or_default().push(point);
            }

            for summary in items {
                let old = indexed.remove(&summary.id).unwrap_or_default();
                let latest = latest_documents(read, summary.id).await?;
                let item_fresh = old.iter().any(|p| {
                    p.point_type == PointType::Item
                        && p.updated_at == summary.updated_at
                        && p.status == summary.status
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

            let orphans: Vec<Uuid> = indexed.into_values().flatten().map(|p| p.id).collect();
            report.points_deleted += orphans.len();
            store.delete(orphans).await?;
        }
        Ok(report)
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
        point_type: PointType::Item,
        item_id: item.id,
        key: item.key.clone(),
        project_id: item.project_id,
        kind_id: item.kind_id,
        status: item.status,
        updated_at: item.updated_at,
        document: None,
        text: format!("{} {}\n\n{}", item.key, item.title, item.body),
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
            point_type: PointType::Document,
            item_id: item.id,
            key: item.key.clone(),
            project_id: item.project_id,
            kind_id: item.kind_id,
            status: item.status,
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
    use crate::vector::MemVectorStore;
    use chrono::Utc;
    use htui_core::fixtures::ids;
    use htui_core::model::{ItemPatch, NewDocument, Status};
    use htui_core::store::{MemStore, WriteStore};

    fn scope(projects: Vec<htui_core::model::ProjectId>) -> Scope {
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
                .filter(|p| p.point_type == PointType::Item)
                .count(),
            items.len()
        );
        let with_docs = item_with_documents(&read).await;
        let latest = latest_documents(&read, with_docs.id).await.unwrap();
        let indexed_docs: BTreeSet<DocumentId> = points
            .iter()
            .filter(|p| p.item_id == with_docs.id)
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
            .find(|p| p.item_id == item.id && p.point_type == PointType::Item)
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
                .filter(|p| p.item_id == item.id)
                .all(|p| p.status == to)
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
        ghost.item_id = ItemId::new();
        store.upsert(vec![ghost.clone()]).await.unwrap();

        let report = Indexer::sync(&read, &scope(vec![ids::PROJECT_HTUI]), &store)
            .await
            .unwrap();
        assert_eq!(report.points_deleted, 1);
        assert!(store.points().iter().all(|p| p.item_id != ghost.item_id));
        let agy_after = store
            .points()
            .iter()
            .filter(|p| p.project_id == ids::PROJECT_AGY)
            .count();
        assert_eq!(agy_before, agy_after);
    }
}
