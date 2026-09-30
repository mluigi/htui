//! `--index-items` and `--search-items`: the concepts index from the command line (MOD-34 D3,
//! `R-STO-8`).
//!
//! Both run before any terminal work, like `--set-dsn`, print to the shell and exit. Neither
//! touches the TUI's start path: a missing Qdrant URL, a Qdrant that does not answer or a
//! Postgres that does not answer is one line on stderr and a non-zero exit, and nothing else is
//! affected (`docs/ANA-19.md` §2 invariant 3). The automatic sync belongs to the headless worker
//! (MOD-41) and the agent-facing tool to the MCP server (MOD-11).
//!
//! Both connect headless (`PgStore::connect_headless`, MOD-40 plan D8): a pending schema or a
//! build below the database's target version is refused, never migrated.
use anyhow::{Context as _, bail};
use htui_core::model::{ProjectId, RequirementState, Resolution, Scope};
use htui_store::embed::FastEmbedder;
use htui_store::pg::{CONNECT_TIMEOUT, PoolSize};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{Hit, PointType, QdrantStore, SearchQuery, VectorStore as _};
use htui_store::vector_sync::Indexer;
use htui_store::{HeadlessError, PgStore, identity, secret};

/// Options of `--search-items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// The query text.
    pub query: String,
    /// Restrict to one project, by slug.
    pub project: Option<String>,
    /// Decisions only (ANA-11 §4.2): items closed as [`DECISION_RESOLUTIONS`], and their documents.
    pub decisions: bool,
    /// Most hits printed.
    pub limit: u64,
}

/// Hits `--search-items` prints when `--limit` is not given.
pub const DEFAULT_LIMIT: u64 = 10;

/// The resolutions `--decisions` keeps (MOD-50 D223, maintainer): an item closed as withdrawn
/// ("dropped without a decision"), duplicate or superseded records no decision in force, and a
/// done item that was never closed out has none yet. Requirements carry no resolution, so they are
/// left out too.
pub const DECISION_RESOLUTIONS: [Resolution; 3] = [
    Resolution::Done,
    Resolution::Concluded,
    Resolution::Rejected,
];

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
    let pg = PgStore::connect_headless(&dsn, &identity, CONNECT_TIMEOUT, PoolSize::TUI)
        .await
        .map_err(headless_refusal)?;
    let embedder = FastEmbedder::new()?;
    let store = QdrantStore::connect(&settings, embedder)
        .await
        .context("cannot reach Qdrant")?;
    Ok((pg, store))
}

/// A headless connect's refusal as the line a user reads (MOD-40 plan D8's sentences, byte for
/// byte). `--index-items` exits 1 with it, `htui worker` exits 2 (MOD-41 plan D14).
pub fn headless_refusal(err: HeadlessError) -> anyhow::Error {
    match err {
        HeadlessError::MigrationsPending(n) => {
            anyhow::anyhow!("{n} schema migration(s) are pending; start `htui` once to apply them")
        }
        HeadlessError::Store(err) => {
            let context = store_context(&err);
            anyhow::Error::new(err).context(context)
        }
        below @ HeadlessError::BelowTarget { .. } => below.into(),
    }
}

/// The line above a headless connect's store error: only an unreachable server is "cannot
/// reach"; a newer or drifted schema, a partial version or a malformed target was reached and
/// refused this build (MOD-40 plan D8).
fn store_context(err: &htui_core::store::StoreError) -> &'static str {
    match err {
        htui_core::store::StoreError::Unreachable(_) => "cannot reach Postgres",
        _ => "Postgres refused this htui",
    }
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
        "indexed: {} item(s) rebuilt, {} unchanged; {} requirement(s) rebuilt, {} unchanged; \
         {} point(s) written, {} removed",
        report.items_rebuilt,
        report.items_unchanged,
        report.requirements_rebuilt,
        report.requirements_unchanged,
        report.points_upserted,
        report.points_deleted
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
        statuses: Vec::new(),
        resolutions: if options.decisions {
            DECISION_RESOLUTIONS.to_vec()
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

/// One result line: key, where it matched, score and snippet. Where it matched names a closed
/// item's resolution and a withdrawn requirement (MOD-50 D227): `--decisions` lists `rejected`
/// next to `done`, and a rejected decision must not read as an adopted one.
#[must_use]
pub fn format_hit(hit: &Hit) -> String {
    let place = match (&hit.point_type, &hit.document) {
        (PointType::Document, Some((_, kind))) => format!("{kind} document"),
        (PointType::Requirement, _) => "requirement".to_owned(),
        _ => "item".to_owned(),
    };
    let place = match (hit.resolution, hit.state) {
        (Some(resolution), _) => format!("{place} ({resolution})"),
        (None, Some(RequirementState::Withdrawn)) => format!("{place} (withdrawn)"),
        _ => place,
    };
    // The snippet is already stripped of control characters (`vector::snippet`); the key and the
    // document kind are stripped here, so nothing stored can drive the terminal it is printed on.
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    format!(
        "{:<10} {:<26} {:.3}  {}",
        clean(&hit.key),
        clean(&place),
        hit.score,
        hit.snippet
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::{DocumentId, ItemId, RequirementId};
    use htui_store::vector::Owner;
    use uuid::Uuid;

    fn hit(point_type: PointType, document: Option<&str>) -> Hit {
        Hit {
            point_type,
            owner: Owner::Item(ItemId(Uuid::nil())),
            key: "ANA-11".into(),
            document: document.map(|kind| (DocumentId(Uuid::nil()), kind.to_owned())),
            resolution: None,
            state: None,
            score: 0.5,
            snippet: "a decision is a closed item".into(),
        }
    }

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
    fn only_an_unreachable_server_is_cannot_reach() {
        use htui_core::store::StoreError;
        assert_eq!(
            store_context(&StoreError::Unreachable("refused".into())),
            "cannot reach Postgres"
        );
        assert_eq!(
            store_context(&StoreError::Backend("the schema is newer".into())),
            "Postgres refused this htui"
        );
    }

    #[test]
    fn hits_print_key_place_score_and_snippet() {
        let line = format_hit(&hit(PointType::Document, Some("summary")));
        assert!(line.starts_with("ANA-11"));
        assert!(line.contains("summary document"));
        assert!(line.contains("0.500"));
        assert!(line.ends_with("a decision is a closed item"));
    }

    #[test]
    fn an_open_item_prints_as_before() {
        let line = format_hit(&hit(PointType::Item, None));
        assert!(line.contains("item "), "{line}");
        assert!(!line.contains('('), "{line}");
    }

    #[test]
    fn a_closed_item_and_its_documents_name_the_resolution() {
        let rejected = Hit {
            resolution: Some(Resolution::Rejected),
            ..hit(PointType::Item, None)
        };
        assert!(format_hit(&rejected).contains("item (rejected)"));
        let done_doc = Hit {
            resolution: Some(Resolution::Done),
            ..hit(PointType::Document, Some("summary"))
        };
        assert!(format_hit(&done_doc).contains("summary document (done)"));
    }

    #[test]
    fn requirements_print_as_such_and_say_when_withdrawn() {
        let active = Hit {
            owner: Owner::Requirement(RequirementId(Uuid::nil())),
            key: "R-STO-8".into(),
            state: Some(RequirementState::Active),
            ..hit(PointType::Requirement, None)
        };
        let line = format_hit(&active);
        assert!(line.starts_with("R-STO-8"), "{line}");
        assert!(line.contains("requirement "), "{line}");
        assert!(!line.contains('('), "{line}");
        let withdrawn = Hit {
            state: Some(RequirementState::Withdrawn),
            ..active
        };
        assert!(format_hit(&withdrawn).contains("requirement (withdrawn)"));
    }

    #[test]
    fn decisions_are_done_concluded_and_rejected() {
        assert_eq!(
            DECISION_RESOLUTIONS,
            [
                Resolution::Done,
                Resolution::Concluded,
                Resolution::Rejected
            ]
        );
    }
}
