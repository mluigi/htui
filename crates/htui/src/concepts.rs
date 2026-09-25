//! `--index-items` and `--search-items`: the concepts index from the command line (MOD-34 D3,
//! `R-STO-8`).
//!
//! Both run before any terminal work, like `--set-dsn`, print to the shell and exit. Neither
//! touches the TUI's start path: a missing Qdrant URL, a Qdrant that does not answer or a
//! Postgres that does not answer is one line on stderr and a non-zero exit, and nothing else is
//! affected (`docs/ANA-19.md` §2 invariant 3). The automatic sync belongs to the headless worker
//! (MOD-41) and the agent-facing tool to the MCP server (MOD-11).
use anyhow::{Context as _, bail};
use htui_core::model::{ProjectId, Scope, Status};
use htui_store::embed::FastEmbedder;
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{Hit, PointType, QdrantStore, SearchQuery, VectorStore as _};
use htui_store::vector_sync::Indexer;
use htui_store::{MigrationState, PgStore, identity, secret};

/// Options of `--search-items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// The query text.
    pub query: String,
    /// Restrict to one project, by slug.
    pub project: Option<String>,
    /// Closed and done items and their documents only (ANA-11 §4.2's decisions).
    pub decisions: bool,
    /// Most hits printed.
    pub limit: u64,
}

/// Hits `--search-items` prints when `--limit` is not given.
pub const DEFAULT_LIMIT: u64 = 10;

/// The statuses `--decisions` keeps: the two terminal ones a decision can close with until MOD-38
/// adds `item.resolution`.
pub const DECISION_STATUSES: [Status; 2] = [Status::Done, Status::Closed];

/// Builds the Qdrant settings from what the keyring holds.
///
/// # Errors
///
/// When no URL is stored, naming where to set one.
pub fn settings_from(
    url: Option<String>,
    api_key: Option<String>,
) -> anyhow::Result<QdrantSettings> {
    let Some(url) = url else {
        bail!("no Qdrant URL is stored; set one in Settings > Qdrant");
    };
    Ok(QdrantSettings::new(url, api_key)?)
}

async fn open() -> anyhow::Result<(PgStore, QdrantStore<FastEmbedder>)> {
    let settings = settings_from(secret::get_qdrant_url()?, secret::get_qdrant_api_key()?)?;
    let Some(dsn) = secret::get_dsn()? else {
        bail!("no Postgres DSN is stored; run `htui --set-dsn` first");
    };
    let identity = identity::load_or_mint(&identity::config_root()?)?;
    let connected = PgStore::connect(&dsn, &identity)
        .await
        .context("cannot reach Postgres")?;
    if let MigrationState::Pending(n) = connected.migrations {
        bail!("{n} schema migration(s) are pending; start `htui` once to apply them");
    }
    let embedder = FastEmbedder::new()?;
    let store = QdrantStore::connect(&settings, embedder)
        .await
        .context("cannot reach Qdrant")?;
    Ok((connected.store, store))
}

/// Every workspace's projects as scopes, narrowed to one project slug when given.
async fn scopes(pg: &PgStore, project: Option<&str>) -> anyhow::Result<Vec<Scope>> {
    let mut scopes = Vec::new();
    let mut found = project.is_none();
    for ws in pg.workspaces().await? {
        let mut scope = Scope::from_workspace(&ws);
        if let Some(slug) = project {
            let wanted: Vec<ProjectId> = ws
                .projects
                .iter()
                .filter(|p| p.slug == slug)
                .map(|p| p.project_id)
                .collect();
            scope.project_ids.retain(|id| wanted.contains(id));
        }
        found |= !scope.project_ids.is_empty();
        if !scope.project_ids.is_empty() {
            scopes.push(scope);
        }
    }
    if !found {
        bail!("no project with slug `{}`", project.unwrap_or_default());
    }
    Ok(scopes)
}

/// `--index-items`: brings the index in step with every project (or one), then reports.
///
/// # Errors
///
/// Missing settings, an unreachable Postgres or Qdrant, a model that cannot load.
pub async fn index_items(project: Option<&str>) -> anyhow::Result<()> {
    let (pg, store) = open().await?;
    let mut report = htui_store::vector_sync::SyncReport::default();
    for scope in scopes(&pg, project).await? {
        report += Indexer::sync(&pg, &scope, &store).await?;
    }
    eprintln!(
        "indexed: {} item(s) rebuilt, {} unchanged; {} point(s) written, {} removed",
        report.items_rebuilt, report.items_unchanged, report.points_upserted, report.points_deleted
    );
    Ok(())
}

/// `--search-items`: prints the best hits, one per line.
///
/// # Errors
///
/// As [`index_items`].
pub async fn search_items(options: &SearchOptions) -> anyhow::Result<()> {
    let (pg, store) = open().await?;
    let projects: Vec<ProjectId> = scopes(&pg, options.project.as_deref())
        .await?
        .into_iter()
        .flat_map(|s| s.project_ids)
        .collect();
    let query = SearchQuery {
        text: options.query.clone(),
        projects,
        types: Vec::new(),
        statuses: if options.decisions {
            DECISION_STATUSES.to_vec()
        } else {
            Vec::new()
        },
        limit: options.limit,
    };
    let hits = store.search(&query).await?;
    if hits.is_empty() {
        eprintln!("no matches (run `htui --index-items` if the index is empty)");
    }
    for hit in &hits {
        println!("{}", format_hit(hit));
    }
    Ok(())
}

/// One result line: key, where it matched, score and snippet.
#[must_use]
pub fn format_hit(hit: &Hit) -> String {
    let place = match (&hit.point_type, &hit.document) {
        (PointType::Document, Some((_, kind))) => format!("{kind} document"),
        _ => "item".to_owned(),
    };
    // The snippet is already stripped of control characters (`vector::snippet`); the key and the
    // document kind are stripped here, so nothing stored can drive the terminal it is printed on.
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    format!(
        "{:<10} {:<18} {:.3}  {}",
        clean(&hit.key),
        clean(&place),
        hit.score,
        hit.snippet
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::{DocumentId, ItemId};
    use uuid::Uuid;

    #[test]
    fn a_missing_url_names_where_to_set_it() {
        let err = settings_from(None, None).unwrap_err().to_string();
        assert!(err.contains("Settings > Qdrant"), "{err}");
    }

    #[test]
    fn a_stored_url_and_key_become_settings() {
        let s = settings_from(Some("http://localhost:6334".into()), Some("k".into())).unwrap();
        assert_eq!(s.url, "http://localhost:6334");
        assert!(s.api_key.is_some());
        assert!(settings_from(Some("localhost:6334".into()), None).is_err());
    }

    #[test]
    fn hits_print_key_place_score_and_snippet() {
        let hit = Hit {
            point_type: PointType::Document,
            item_id: ItemId(Uuid::nil()),
            key: "ANA-11".into(),
            document: Some((DocumentId(Uuid::nil()), "summary".into())),
            score: 0.5,
            snippet: "a decision is a closed item".into(),
        };
        let line = format_hit(&hit);
        assert!(line.starts_with("ANA-11"));
        assert!(line.contains("summary document"));
        assert!(line.contains("0.500"));
        assert!(line.ends_with("a decision is a closed item"));
    }
}
