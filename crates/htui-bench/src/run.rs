//! The runner: corpus, points, backends, ranked lists and timings per query (MOD-162 T6).

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use htui_store::embed::{DenseEmbedder, EmbedderIdentity, RtenEmbedder};
use htui_store::qdrant_settings::QdrantSettings;
use serde::{Deserialize, Serialize};

use crate::corpus::{Corpus, PointCounts};
use crate::queries::{Query, QuerySet};
use crate::types::{Backend, CorpusId, QueryClass, Ranked};

/// `results.json`'s `schema`.
pub const RESULTS_SCHEMA: u32 = 1;

/// Where and what to run. `Debug` is hand-written and prints `database_url` as `<redacted>`.
pub struct RunConfig {
    /// The repository the corpus is read from.
    pub repo: PathBuf,
    /// A commit, or [`crate::corpus::WORKTREE`].
    pub corpus_ref: String,
    /// The frozen query file; its sidecar is `<file>.sha256`.
    pub queries: PathBuf,
    /// Where `results.json` and `report.md` go.
    pub out: PathBuf,
    /// The scratch Qdrant.
    pub qdrant: QdrantSettings,
    /// The Postgres DSN for the L baselines. Never printed.
    pub database_url: String,
    /// The run's Qdrant collection (recreated by every run).
    pub collection: String,
    /// The run's Postgres schema (dropped at the end).
    pub schema: String,
}

impl std::fmt::Debug for RunConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        Ok(())
    }
}

/// `htui_bench_` + corpus_ref lowercased, every char outside [a-z0-9] → `_`, at most 48 chars
/// (`d15e9fea` → `htui_bench_d15e9fea`; `WORKTREE` → `htui_bench_worktree`).
pub fn collection_name(corpus_ref: &str) -> String {
    let _ = corpus_ref;
    String::new()
}

/// One run's output: `results.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Results {
    /// [`RESULTS_SCHEMA`].
    pub schema: u32,
    /// Provenance.
    pub meta: RunMeta,
    /// The queries, in file order.
    pub queries: Vec<QueryRecord>,
    /// One row per query and backend, query-major, backends in [`Backend::ALL`] order.
    pub runs: Vec<BackendRun>,
}

/// A run's provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMeta {
    /// When the run started.
    pub started_at: DateTime<Utc>,
    /// When the last backend answered.
    pub finished_at: DateTime<Utc>,
    /// The corpus as requested.
    pub corpus_ref: String,
    /// The resolved commit; `None` for the work tree.
    pub corpus_commit: Option<String>,
    /// Documents by kind: `open_item`, `requirement`, `decision`, `total`.
    pub docs: BTreeMap<String, usize>,
    /// Points by type.
    pub points: PointCounts,
    /// The query set's name.
    pub query_set: String,
    /// The query file's path.
    pub query_file: String,
    /// The query file's sha256.
    pub query_sha256: String,
    /// The embedder, as [`EmbedderIdentity`] displays it.
    pub embedder: String,
    /// The embedder's full identity.
    pub embedder_identity: EmbedderIdentity,
    /// The Qdrant server's version.
    pub qdrant_version: String,
    /// The run's collection.
    pub collection: String,
    /// `SELECT version()`; never the DSN.
    pub postgres_version: String,
    /// `git rev-parse HEAD` of the htui checkout the bench was built from.
    pub htui_commit: String,
    /// Whether that checkout had uncommitted changes.
    pub htui_dirty: bool,
    /// The machine.
    pub host: HostInfo,
    /// Phase timings.
    pub timings_ms: Timings,
    /// Rows recorded per ranked list.
    pub rank_depth: usize,
}

/// The machine a run ran on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostInfo {
    /// `std::env::consts::OS`.
    pub os: String,
    /// `available_parallelism`.
    pub cpus: usize,
    /// `/proc/loadavg` before indexing; `None` where there is none.
    pub load_start: Option<[f64; 3]>,
    /// `/proc/loadavg` after the last query.
    pub load_end: Option<[f64; 3]>,
}

/// Phase timings, in milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timings {
    /// Loading the embedding model; `None` when the caller passed an embedder (`run_with`).
    pub model_load: Option<f64>,
    /// Embedding and upserting every point.
    pub index: f64,
    /// Loading the L schema.
    pub pg_load: f64,
    /// Every backend over every query.
    pub queries: f64,
}

/// One query as run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryRecord {
    /// `a01`.
    pub id: String,
    /// Its class.
    pub class: QueryClass,
    /// The text searched.
    pub text: String,
    /// Corpus IDs that answer it; empty for class d.
    pub targets: Vec<CorpusId>,
    /// `htui_store::bm25::query_vector(text).indices.len()`: §5.4's "per query term".
    pub sparse_terms: usize,
}

/// One backend's answer to one query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendRun {
    /// The backend.
    pub backend: Backend,
    /// The query's id.
    pub query: String,
    /// At most [`crate::types::RANK_DEPTH`] rows.
    pub ranked: Vec<Ranked>,
    /// 1-based rank of the first target in `ranked`.
    pub first_rank: Option<usize>,
    /// Wall time of the call, embedding included.
    pub latency_ms: f64,
    /// The backend's error; `ranked` is then empty.
    pub error: Option<String>,
}

/// The whole run: `queries::load_frozen` (refuses a changed file), `corpus::build`,
/// `queries::validate` (refuses, listing each unknown target), the production model, `run_with`.
///
/// # Errors
/// Any step's, with context.
pub async fn run(config: &RunConfig) -> anyhow::Result<Results> {
    let _ = config;
    anyhow::bail!("run::run: not implemented yet (MOD-162 T6)")
}

/// Everything after the model is loaded; generic so a test runs it with `HashEmbedder`.
///
/// # Errors
/// Postgres or Qdrant unreachable, a failed upsert or schema load.
pub async fn run_with<E: DenseEmbedder>(
    config: &RunConfig,
    corpus: &Corpus,
    queries: &QuerySet,
    query_sha256: &str,
    embedder: E,
) -> anyhow::Result<Results> {
    let _ = (config, corpus, queries, query_sha256, embedder);
    anyhow::bail!("run::run_with: not implemented yet (MOD-162 T6)")
}

/// `ensure_model_noting(|l| eprintln!("{l}"))`, then `RtenEmbedder::load` on `spawn_blocking`,
/// with `.context("cannot load the embedding model")` (as `crates/htui/src/concepts.rs:134-142`).
///
/// # Errors
/// The model cannot be fetched or loaded.
pub async fn load_embedder() -> anyhow::Result<RtenEmbedder> {
    anyhow::bail!("run::load_embedder: not implemented yet (MOD-162 T6)")
}

/// Refuses a query set naming a target that is not in the corpus, one line per unknown target.
///
/// # Errors
/// At least one unknown target.
pub fn check_targets(corpus: &Corpus, set: &QuerySet) -> anyhow::Result<()> {
    let _ = (corpus, set);
    Ok(())
}

/// One query's [`QueryRecord`].
pub fn query_record(query: &Query) -> QueryRecord {
    QueryRecord {
        id: query.id.clone(),
        class: query.class,
        text: query.text.clone(),
        targets: query.targets.clone(),
        sparse_terms: 0,
    }
}

/// One backend's row for one query: an `Err` keeps its message and no rows; `first_rank` from the
/// rows and the query's targets.
pub fn backend_run(
    backend: Backend,
    query: &Query,
    outcome: Result<Vec<Ranked>, String>,
    latency_ms: f64,
) -> BackendRun {
    let _ = outcome;
    BackendRun {
        backend,
        query: query.id.clone(),
        ranked: Vec::new(),
        first_rank: None,
        latency_ms,
        error: None,
    }
}

/// The three load averages of `/proc/loadavg`'s text.
pub fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let _ = text;
    None
}

/// Documents by kind, as [`RunMeta::docs`] records them.
pub fn doc_counts(corpus: &Corpus) -> BTreeMap<String, usize> {
    let _ = corpus;
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{Sources, build_from_sources};
    use htui_core::qdrant::QdrantUrl;

    fn settings() -> QdrantSettings {
        QdrantSettings::new(
            QdrantUrl::parse("http://localhost:6334").expect("a valid URL"),
            Some("the-api-key".into()),
        )
    }

    fn config(database_url: &str) -> RunConfig {
        RunConfig {
            repo: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")),
            corpus_ref: "fixture".into(),
            queries: PathBuf::from("fixture.json"),
            out: std::env::temp_dir().join("htui-bench-unit"),
            qdrant: settings(),
            database_url: database_url.into(),
            collection: "htui_bench_fixture".into(),
            schema: "htui_bench_fixture".into(),
        }
    }

    fn query(id: &str, class: QueryClass, text: &str, targets: &[&str]) -> Query {
        Query {
            id: id.into(),
            class,
            text: text.into(),
            targets: targets.iter().map(|t| CorpusId::new(*t)).collect(),
            completed: false,
        }
    }

    fn fixture_sources() -> Sources {
        Sources {
            commit: None,
            handoff: "- [ ] **MOD-7 - Crash telemetry address**\n  The address is hard-wired in \
                      the binary.\n\n- [ ] **TOOL-3 - Docs checker**\n  Fail when a write-up \
                      omits requirements.\n\n- [ ] **ANA-9 - Pick a protocol**\n  Which protocol \
                      talks to the coding assistants.\n"
                .into(),
            requirements: "- **R-STO-8 (must).** Store every item in Postgres.\n- **R-TUI-2 \
                           (must).** Close the app without losing running work.\n"
                .into(),
            decisions: vec![(
                "docs/decisions/mod/mod-41.md".into(),
                "# MOD-41 - Keep running work on close (done, 2026-09-02)\n\nRunning work \
                 survives closing the app.\n"
                    .into(),
            )],
        }
    }

    fn fixture_set() -> QuerySet {
        QuerySet {
            set: "tiny".into(),
            corpus_ref: "fixture".into(),
            queries: vec![
                query(
                    "a01",
                    QueryClass::A,
                    "the telemetry address is baked into the binary",
                    &["MOD-7"],
                ),
                query(
                    "b01",
                    QueryClass::B,
                    "what happens to running work when I close the app",
                    &["R-TUI-2", "DEC:MOD-41"],
                ),
                query("c01", QueryClass::C, "R-STO-8", &["R-STO-8"]),
                query("d01", QueryClass::D, "a recipe for sourdough bread", &[]),
            ],
        }
    }

    #[test]
    fn collection_names_follow_the_corpus_ref() {
        assert_eq!(collection_name("d15e9fea"), "htui_bench_d15e9fea");
        assert_eq!(collection_name("WORKTREE"), "htui_bench_worktree");
        assert_eq!(collection_name("origin/main~2"), "htui_bench_origin_main_2");
        let long = collection_name(&"a".repeat(100));
        assert_eq!(long.len(), 48);
        assert!(long.starts_with("htui_bench_aaa"));
        assert_eq!(collection_name("Ünï"), "htui_bench____");
    }

    #[test]
    fn run_config_debug_hides_the_dsn_and_the_api_key() {
        let dsn = "postgres://bench:s3cret-pw@db.example:5432/htui";
        let shown = format!("{:?}", config(dsn));
        assert!(!shown.contains("s3cret-pw"), "{shown}");
        assert!(!shown.contains(dsn), "{shown}");
        assert!(!shown.contains("the-api-key"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(shown.contains("htui_bench_fixture"), "{shown}");
        assert!(shown.contains("fixture"), "{shown}");
    }

    #[test]
    fn a_backend_run_ranks_its_first_target() {
        let q = query("b01", QueryClass::B, "x", &["R-TUI-2", "DEC:MOD-41"]);
        let rows = vec![
            Ranked {
                id: CorpusId::new("MOD-7"),
                score: 0.9,
            },
            Ranked {
                id: CorpusId::new("DEC:MOD-41"),
                score: 0.5,
            },
        ];
        let run = backend_run(Backend::S, &q, Ok(rows.clone()), 12.5);
        assert_eq!(run.backend, Backend::S);
        assert_eq!(run.query, "b01");
        assert_eq!(run.ranked, rows);
        assert_eq!(run.first_rank, Some(2));
        assert_eq!(run.latency_ms, 12.5);
        assert_eq!(run.error, None);

        let missed = backend_run(Backend::G, &q, Ok(rows[..1].to_vec()), 1.0);
        assert_eq!(missed.first_rank, None);

        let failed = backend_run(Backend::LOr, &q, Err("syntax error".into()), 3.0);
        assert!(failed.ranked.is_empty());
        assert_eq!(failed.first_rank, None);
        assert_eq!(failed.error.as_deref(), Some("syntax error"));
    }

    #[test]
    fn a_query_record_counts_its_sparse_terms() {
        let q = query("c01", QueryClass::C, "R-STO-8", &["R-STO-8"]);
        let record = query_record(&q);
        assert_eq!(record.id, "c01");
        assert_eq!(record.targets, vec![CorpusId::new("R-STO-8")]);
        assert_eq!(
            record.sparse_terms,
            htui_store::bm25::query_vector("R-STO-8").indices.len()
        );
        assert!(record.sparse_terms > 0);
        assert_eq!(query_record(&query("d01", QueryClass::D, "?!", &[])).sparse_terms, 0);
    }

    #[test]
    fn loadavg_parses_its_three_averages() {
        assert_eq!(
            parse_loadavg("0.52 1.03 2.50 3/1024 4242\n"),
            Some([0.52, 1.03, 2.5])
        );
        assert_eq!(parse_loadavg(""), None);
        assert_eq!(parse_loadavg("0.5 x 1"), None);
    }

    #[test]
    fn doc_counts_count_each_kind_and_the_total() {
        let corpus = build_from_sources("fixture", fixture_sources()).expect("builds");
        let counts = doc_counts(&corpus);
        assert_eq!(counts.get("open_item"), Some(&3));
        assert_eq!(counts.get("requirement"), Some(&2));
        assert_eq!(counts.get("decision"), Some(&1));
        assert_eq!(counts.get("total"), Some(&6));
    }

    #[test]
    fn check_targets_names_every_unknown_target() {
        let corpus = build_from_sources("fixture", fixture_sources()).expect("builds");
        check_targets(&corpus, &fixture_set()).expect("the fixture set is valid");
        let mut set = fixture_set();
        set.queries[0].targets = vec![CorpusId::new("MOD-999")];
        set.queries[2].targets = vec![CorpusId::new("R-NOPE-1")];
        let err = format!("{:#}", check_targets(&corpus, &set).expect_err("refused"));
        assert!(err.contains("a01: MOD-999 is not a corpus ID"), "{err}");
        assert!(err.contains("c01: R-NOPE-1 is not a corpus ID"), "{err}");
    }

    /// The URL in `var`, or the skip line.
    fn live_env(var: &str) -> Option<String> {
        let value = std::env::var(var).ok().filter(|v| !v.trim().is_empty());
        if value.is_none() {
            eprintln!("skipped: {var} not set");
        }
        value
    }

    #[tokio::test]
    async fn run_with_wires_a_tiny_corpus() {
        use htui_store::embed::{DENSE_DIM, HashEmbedder};

        let qdrant = live_env("HTUI_TEST_QDRANT_URL");
        let database = live_env("HTUI_TEST_DATABASE_URL");
        let (Some(qdrant), Some(database)) = (qdrant, database) else {
            return;
        };
        let name = format!("htui_bench_t_{}", uuid::Uuid::now_v7().simple());
        let out = std::env::temp_dir().join(&name);
        let config = RunConfig {
            qdrant: QdrantSettings::new(QdrantUrl::parse(&qdrant).expect("a valid URL"), None),
            database_url: database.clone(),
            collection: name.clone(),
            schema: name.clone(),
            out: out.clone(),
            ..config(&database)
        };
        let corpus = build_from_sources("fixture", fixture_sources()).expect("builds");
        let set = fixture_set();

        let results = run_with(
            &config,
            &corpus,
            &set,
            &"0".repeat(64),
            HashEmbedder::new(DENSE_DIM),
        )
        .await;

        // The collection is the test's to drop, whatever happened.
        let raw = qdrant_client::Qdrant::from_url(&qdrant)
            .skip_compatibility_check()
            .build()
            .expect("client");
        let _ = raw.delete_collection(name.as_str()).await;

        let results = results.expect("the tiny run completes");
        assert_eq!(results.schema, RESULTS_SCHEMA);
        assert_eq!(results.queries.len(), 4);
        assert_eq!(results.runs.len(), 4 * Backend::ALL.len());
        for q in &set.queries {
            let backends: Vec<Backend> = results
                .runs
                .iter()
                .filter(|r| r.query == q.id)
                .map(|r| r.backend)
                .collect();
            assert_eq!(backends, Backend::ALL, "{}", q.id);
        }
        let sparse = results
            .runs
            .iter()
            .find(|r| r.query == "c01" && r.backend == Backend::SSparse)
            .expect("an S-sparse row");
        assert!(sparse.error.is_none(), "{sparse:?}");
        assert!(sparse.first_rank.is_some(), "{sparse:?}");
        assert_eq!(results.meta.collection, name);
        assert_eq!(results.meta.embedder, format!("hash/{DENSE_DIM}"));
        assert_eq!(results.meta.timings_ms.model_load, None);
        assert_eq!(
            results.meta.points.requirement, 2,
            "{:?}",
            results.meta.points
        );

        let pool = crate::pg_fts::connect(&database).await.expect("Postgres");
        let left: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.schemata WHERE schema_name = $1",
        )
        .bind(&name)
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(left, 0, "the schema is dropped at the end");

        crate::report::write(&results, &out).expect("report written");
        assert!(out.join("results.json").is_file());
        assert!(out.join("report.md").is_file());
        let _ = std::fs::remove_dir_all(&out);
    }
}
