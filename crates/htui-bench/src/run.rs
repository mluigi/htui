//! The runner: corpus, points, backends, ranked lists and timings per query (MOD-162 T6).
//!
//! [`run`] checks the frozen query set, builds the corpus, validates the targets, connects to
//! Postgres and Qdrant (failing fast, before the slow parts), loads the production embedder and
//! hands over to [`run_with`]'s pipeline: a clean collection (blueprint F8), the points upserted
//! in chunks of 64, the L schema, then every backend over every query.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, bail};
use chrono::{DateTime, Utc};
use htui_store::bm25;
use htui_store::embed::{DenseEmbedder, EmbedderIdentity, RtenEmbedder};
use htui_store::model;
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{Hit, QdrantStore, SearchArms, SearchQuery, VectorStore};
use qdrant_client::Qdrant;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::corpus::{self, Corpus, PointCounts};
use crate::lexical::{self, GTie, GrepIndex};
use crate::metrics;
use crate::pg_fts;
use crate::queries::{self, Query, QuerySet};
use crate::types::{
    Backend, CorpusId, DocKind, LVariant, QueryClass, RANK_DEPTH, Ranked, S_LIMIT, S10_LIMIT,
    ranked_from_hits,
};

/// `results.json`'s `schema`.
pub const RESULTS_SCHEMA: u32 = 1;

/// Points per `VectorStore::upsert` call.
const UPSERT_CHUNK: usize = 64;

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
        f.debug_struct("RunConfig")
            .field("repo", &self.repo)
            .field("corpus_ref", &self.corpus_ref)
            .field("queries", &self.queries)
            .field("out", &self.out)
            .field("qdrant", &self.qdrant)
            .field("database_url", &"<redacted>")
            .field("collection", &self.collection)
            .field("schema", &self.schema)
            .finish()
    }
}

/// `htui_bench_` + corpus_ref lowercased, every char outside [a-z0-9] → `_`, at most 48 chars
/// (`d15e9fea` → `htui_bench_d15e9fea`; `WORKTREE` → `htui_bench_worktree`).
pub fn collection_name(corpus_ref: &str) -> String {
    let tail: String = corpus_ref
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Every char is ASCII now, so a byte cut is a char cut.
    let mut name = format!("htui_bench_{tail}");
    name.truncate(48);
    name
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

/// What every collection and schema a run may delete starts with.
pub const SCRATCH_PREFIX: &str = "htui_bench";

/// Refuses a `--collection` or `--schema` that is not a scratch name: a run deletes and recreates
/// both (`DROP SCHEMA … CASCADE`, F8's collection delete), so `public` or `htui_concepts_v2` would
/// destroy real data (T6 verify finding).
///
/// # Errors
/// Either name does not start with [`SCRATCH_PREFIX`].
pub fn check_scratch_names(collection: &str, schema: &str) -> anyhow::Result<()> {
    for (what, name) in [("collection", collection), ("schema", schema)] {
        anyhow::ensure!(
            name.starts_with(SCRATCH_PREFIX),
            "refusing {what} {name:?}: a run deletes and recreates it, so it must start with \
             {SCRATCH_PREFIX:?}"
        );
    }
    Ok(())
}

/// The whole run: `queries::load_frozen` (refuses a changed file), `corpus::build`,
/// `queries::validate` (refuses, listing each unknown target), the production model, `run_with`.
///
/// # Errors
/// Any step's, with context.
pub async fn run(config: &RunConfig) -> anyhow::Result<Results> {
    check_scratch_names(&config.collection, &config.schema)?;
    let started_at = Utc::now();
    let frozen = queries::load_frozen(&config.queries)?;
    note(&format!(
        "query set {} ({} queries, sha256 {})",
        frozen.set.set,
        frozen.set.queries.len(),
        frozen.sha256
    ));
    let corpus = corpus::build(&config.repo, &config.corpus_ref)?;
    note_corpus(&corpus);
    check_targets(&corpus, &frozen.set)?;
    warn_on_another_ref(&frozen.set, &config.corpus_ref);

    let services = Services::connect(config).await?;
    let clock = Instant::now();
    let embedder = load_embedder().await?;
    let model_load = elapsed_ms(clock);
    note(&format!(
        "embedding model loaded ({:.1} s)",
        model_load / 1000.0
    ));

    let ctx = RunContext {
        config,
        corpus: &corpus,
        queries: &frozen.set,
        query_sha256: &frozen.sha256,
        started_at,
        model_load_ms: Some(model_load),
    };
    pipeline(&ctx, embedder, &services).await
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
    check_scratch_names(&config.collection, &config.schema)?;
    let started_at = Utc::now();
    let services = Services::connect(config).await?;
    let ctx = RunContext {
        config,
        corpus,
        queries,
        query_sha256,
        started_at,
        model_load_ms: None,
    };
    pipeline(&ctx, embedder, &services).await
}

/// `ensure_model_noting(|l| eprintln!("{l}"))`, then `RtenEmbedder::load` on `spawn_blocking`,
/// with `.context("cannot load the embedding model")` (as `crates/htui/src/concepts.rs:134-142`).
///
/// # Errors
/// The model cannot be fetched or loaded.
pub async fn load_embedder() -> anyhow::Result<RtenEmbedder> {
    let files = model::ensure_model_noting(|line| eprintln!("{line}"))
        .await
        .context("cannot load the embedding model")?;
    tokio::task::spawn_blocking(move || RtenEmbedder::load(&files))
        .await
        .context("the embedding model's loader stopped")?
        .context("cannot load the embedding model")
}

/// Refuses a query set naming a target that is not in the corpus, one line per unknown target.
///
/// # Errors
/// At least one unknown target.
pub fn check_targets(corpus: &Corpus, set: &QuerySet) -> anyhow::Result<()> {
    let unknown = queries::validate(&set.queries, &corpus.ids());
    if unknown.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = unknown.iter().map(ToString::to_string).collect();
    bail!(
        "{} unknown target(s) in {} against {}:\n{}",
        unknown.len(),
        set.set,
        corpus.corpus_ref,
        lines.join("\n")
    )
}

/// One query's [`QueryRecord`].
pub fn query_record(query: &Query) -> QueryRecord {
    QueryRecord {
        id: query.id.clone(),
        class: query.class,
        text: query.text.clone(),
        targets: query.targets.clone(),
        sparse_terms: bm25::query_vector(&query.text).indices.len(),
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
    let (ranked, error) = match outcome {
        Ok(ranked) => (ranked, None),
        Err(message) => (Vec::new(), Some(message)),
    };
    let ids: Vec<CorpusId> = ranked.iter().map(|row| row.id.clone()).collect();
    BackendRun {
        backend,
        query: query.id.clone(),
        first_rank: metrics::first_target_rank(&ids, &query.targets),
        ranked,
        latency_ms,
        error,
    }
}

/// The three load averages of `/proc/loadavg`'s text.
pub fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let mut fields = text.split_whitespace().map(str::parse::<f64>);
    let mut next = || fields.next()?.ok();
    Some([next()?, next()?, next()?])
}

/// Documents by kind, as [`RunMeta::docs`] records them.
pub fn doc_counts(corpus: &Corpus) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = ["open_item", "requirement", "decision"]
        .into_iter()
        .map(|kind| (kind.to_owned(), 0))
        .collect();
    for doc in &corpus.docs {
        let kind = match doc.kind {
            DocKind::OpenItem => "open_item",
            DocKind::Requirement => "requirement",
            DocKind::Decision => "decision",
        };
        *counts.entry(kind.to_owned()).or_default() += 1;
    }
    counts.insert("total".to_owned(), corpus.docs.len());
    counts
}

/// Progress, on stderr.
fn note(line: &str) {
    eprintln!("htui-bench: {line}");
}

fn note_corpus(corpus: &Corpus) {
    let counts = doc_counts(corpus);
    let count = |kind: &str| counts.get(kind).copied().unwrap_or_default();
    note(&format!(
        "corpus {} ({}): {} open items, {} requirements, {} decisions, {} documents",
        corpus.corpus_ref,
        corpus.commit.as_deref().unwrap_or("work tree"),
        count("open_item"),
        count("requirement"),
        count("decision"),
        count("total"),
    ));
}

fn warn_on_another_ref(set: &QuerySet, corpus_ref: &str) {
    if set.corpus_ref != corpus_ref {
        note(&format!(
            "warning: the query set was written against {}, the corpus is {corpus_ref}",
            set.corpus_ref
        ));
    }
}

fn elapsed_ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// What a run has before the pipeline starts.
struct RunContext<'a> {
    config: &'a RunConfig,
    corpus: &'a Corpus,
    queries: &'a QuerySet,
    query_sha256: &'a str,
    started_at: DateTime<Utc>,
    model_load_ms: Option<f64>,
}

/// Postgres and Qdrant, checked before anything slow.
struct Services {
    pool: PgPool,
    postgres_version: String,
    raw: Qdrant,
    qdrant_version: String,
}

impl Services {
    async fn connect(config: &RunConfig) -> anyhow::Result<Self> {
        if !pg_fts::is_valid_schema(&config.schema) {
            bail!(
                "schema {:?} is not a valid name (^[a-z_][a-z0-9_]{{0,62}}$)",
                config.schema
            );
        }
        let pool = pg_fts::connect(&config.database_url).await?;
        let postgres_version = pg_fts::server_version(&pool).await?;
        let raw = raw_client(&config.qdrant)?;
        let qdrant_version = raw
            .health_check()
            .await
            .context("Qdrant health check")?
            .version;
        note(&format!(
            "Postgres: {postgres_version}; Qdrant {qdrant_version}"
        ));
        Ok(Self {
            pool,
            postgres_version,
            raw,
            qdrant_version,
        })
    }
}

/// A client of the bench's own, for what `QdrantStore` does not expose. No compatibility check:
/// it prints to stdout (as `vector.rs` `client_config` notes).
fn raw_client(settings: &QdrantSettings) -> anyhow::Result<Qdrant> {
    let mut builder = Qdrant::from_url(settings.url.as_str()).skip_compatibility_check();
    if let Some(key) = &settings.api_key {
        builder = builder.api_key(key.as_str());
    }
    builder.build().context("building the Qdrant client")
}

/// Steps 7-13 of blueprint §4.1.
async fn pipeline<E: DenseEmbedder>(
    ctx: &RunContext<'_>,
    embedder: E,
    services: &Services,
) -> anyhow::Result<Results> {
    let config = ctx.config;
    let load_start = read_loadavg();
    let identity = embedder.identity();

    // F8: `connect_to` refuses a collection another embedder made, so delete it first.
    let name = config.collection.as_str();
    if services
        .raw
        .collection_exists(name)
        .await
        .with_context(|| format!("checking for collection {name}"))?
    {
        note(&format!("deleting the previous collection {name}"));
        services
            .raw
            .delete_collection(name)
            .await
            .with_context(|| format!("deleting collection {name}"))?;
    }
    let store = QdrantStore::connect_to(&config.qdrant, embedder, name)
        .await
        .with_context(|| format!("creating collection {name}"))?;

    let points = ctx.corpus.points();
    let total = points.len();
    let clock = Instant::now();
    let mut done = 0;
    for chunk in points.chunks(UPSERT_CHUNK) {
        store
            .upsert(chunk.to_vec())
            .await
            .with_context(|| format!("upserting points {}..{}", done, done + chunk.len()))?;
        done += chunk.len();
        note(&format!(
            "indexed {done}/{total} points ({:.1} s)",
            clock.elapsed().as_secs_f64()
        ));
    }
    let index_ms = elapsed_ms(clock);

    let measured = measure(ctx, &store, &services.pool).await;
    if let Err(err) = pg_fts::drop_schema(&services.pool, &config.schema).await {
        note(&format!(
            "warning: could not drop schema {}: {err:#}",
            config.schema
        ));
    }
    let measured = measured?;

    let (htui_commit, htui_dirty) = htui_checkout();
    let meta = RunMeta {
        started_at: ctx.started_at,
        finished_at: Utc::now(),
        corpus_ref: ctx.corpus.corpus_ref.clone(),
        corpus_commit: ctx.corpus.commit.clone(),
        docs: doc_counts(ctx.corpus),
        points: ctx.corpus.point_counts(),
        query_set: ctx.queries.set.clone(),
        query_file: config.queries.display().to_string(),
        query_sha256: ctx.query_sha256.to_owned(),
        embedder: identity.to_string(),
        embedder_identity: identity,
        qdrant_version: services.qdrant_version.clone(),
        collection: config.collection.clone(),
        postgres_version: services.postgres_version.clone(),
        htui_commit,
        htui_dirty,
        host: HostInfo {
            os: std::env::consts::OS.to_owned(),
            cpus: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
            load_start,
            load_end: read_loadavg(),
        },
        timings_ms: Timings {
            model_load: ctx.model_load_ms,
            index: index_ms,
            pg_load: measured.pg_load_ms,
            queries: measured.queries_ms,
        },
        rank_depth: RANK_DEPTH,
    };
    Ok(Results {
        schema: RESULTS_SCHEMA,
        meta,
        queries: ctx.queries.queries.iter().map(query_record).collect(),
        runs: measured.runs,
    })
}

/// The backends' rows and the timings around them.
struct Measured {
    runs: Vec<BackendRun>,
    pg_load_ms: f64,
    queries_ms: f64,
}

/// Steps 9-11: the L schema, the grep index, a warm-up, then every backend per query.
async fn measure<E: DenseEmbedder>(
    ctx: &RunContext<'_>,
    store: &QdrantStore<E>,
    pool: &PgPool,
) -> anyhow::Result<Measured> {
    let schema = ctx.config.schema.as_str();
    let corpus = ctx.corpus;
    let clock = Instant::now();
    pg_fts::load(pool, schema, &corpus.docs)
        .await
        .context("loading the L schema")?;
    let pg_load_ms = elapsed_ms(clock);
    let grep = GrepIndex::new(&corpus.docs);

    // Warm-up, untimed and unrecorded: the first calls pay for connections and caches.
    if let Some(first) = ctx.queries.queries.first() {
        let query = search_query(corpus, &first.text, S_LIMIT);
        for arms in [SearchArms::Hybrid, SearchArms::Dense, SearchArms::Sparse] {
            let _ = store.search_arms(&query, arms).await;
        }
        let _ = pg_fts::search(pool, schema, LVariant::Or, &first.text, RANK_DEPTH).await;
    }

    let total = ctx.queries.queries.len();
    let clock = Instant::now();
    let mut runs = Vec::with_capacity(total * Backend::ALL.len());
    for (done, query) in ctx.queries.queries.iter().enumerate() {
        // S-points reads S10's response (no second call), and S10 comes first in `Backend::ALL`.
        let mut s10: Option<(Result<Vec<Hit>, String>, f64)> = None;
        for backend in Backend::ALL {
            let (outcome, latency) = match backend {
                Backend::S => qdrant(store, corpus, query, SearchArms::Hybrid, S_LIMIT, true).await,
                Backend::S10 => {
                    let (hits, latency) =
                        timed_hits(store, corpus, query, SearchArms::Hybrid, S10_LIMIT).await;
                    let outcome = ranked(&hits, true);
                    s10 = Some((hits, latency));
                    (outcome, latency)
                }
                Backend::SPoints => match &s10 {
                    Some((hits, latency)) => (ranked(hits, false), *latency),
                    None => (Err("S10 did not run".to_owned()), 0.0),
                },
                Backend::SDense => {
                    qdrant(store, corpus, query, SearchArms::Dense, S_LIMIT, true).await
                }
                Backend::SSparse => {
                    qdrant(store, corpus, query, SearchArms::Sparse, S_LIMIT, true).await
                }
                Backend::LAnd | Backend::LOr | Backend::LAndLen | Backend::LOrLen => {
                    let variant = backend.l_variant().unwrap_or(LVariant::And);
                    let clock = Instant::now();
                    let outcome =
                        pg_fts::search(pool, schema, variant, &query.text, RANK_DEPTH).await;
                    (outcome.map_err(|e| format!("{e:#}")), elapsed_ms(clock))
                }
                Backend::G | Backend::GDens => {
                    let tie = if backend == Backend::G {
                        GTie::Occurrences
                    } else {
                        GTie::Density
                    };
                    let clock = Instant::now();
                    let rows = grep.search(&query.text, tie, RANK_DEPTH);
                    (Ok(rows), elapsed_ms(clock))
                }
                Backend::H => {
                    let clock = Instant::now();
                    let rows = lexical::htui_filter(
                        &corpus.items,
                        &corpus.requirements,
                        &query.text,
                        RANK_DEPTH,
                    );
                    (Ok(rows), elapsed_ms(clock))
                }
            };
            runs.push(backend_run(backend, query, outcome, latency));
        }
        let done = done + 1;
        if done % 10 == 0 || done == total {
            note(&format!(
                "ran {done}/{total} queries ({:.1} s)",
                clock.elapsed().as_secs_f64()
            ));
        }
    }
    Ok(Measured {
        runs,
        pg_load_ms,
        queries_ms: elapsed_ms(clock),
    })
}

/// The bench's search: the one synthetic project, no filters.
fn search_query(corpus: &Corpus, text: &str, limit: u64) -> SearchQuery {
    SearchQuery {
        text: text.to_owned(),
        projects: vec![corpus.project],
        types: vec![],
        statuses: vec![],
        resolutions: vec![],
        limit,
    }
}

/// One timed `search_arms` call (embedding included, as ANA-37 §5.5 measured).
async fn timed_hits<E: DenseEmbedder>(
    store: &QdrantStore<E>,
    corpus: &Corpus,
    query: &Query,
    arms: SearchArms,
    limit: u64,
) -> (Result<Vec<Hit>, String>, f64) {
    let search = search_query(corpus, &query.text, limit);
    let clock = Instant::now();
    let hits = store.search_arms(&search, arms).await;
    (hits.map_err(|e| e.to_string()), elapsed_ms(clock))
}

async fn qdrant<E: DenseEmbedder>(
    store: &QdrantStore<E>,
    corpus: &Corpus,
    query: &Query,
    arms: SearchArms,
    limit: u64,
    collapse: bool,
) -> (Result<Vec<Ranked>, String>, f64) {
    let (hits, latency) = timed_hits(store, corpus, query, arms, limit).await;
    (ranked(&hits, collapse), latency)
}

fn ranked(hits: &Result<Vec<Hit>, String>, collapse: bool) -> Result<Vec<Ranked>, String> {
    match hits {
        Ok(hits) => Ok(ranked_from_hits(hits, collapse, RANK_DEPTH)),
        Err(message) => Err(message.clone()),
    }
}

fn read_loadavg() -> Option<[f64; 3]> {
    parse_loadavg(&std::fs::read_to_string("/proc/loadavg").ok()?)
}

/// `git rev-parse HEAD` and whether `git status --porcelain` is non-empty, for the checkout this
/// binary was built from; `("unknown", false)` without git.
fn htui_checkout() -> (String, bool) {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let git = |args: &[&str]| -> Option<String> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    match git(&["rev-parse", "HEAD"]) {
        Some(commit) => {
            let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
            (commit, dirty)
        }
        None => ("unknown".to_owned(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{Sources, build_from_sources};
    use htui_core::qdrant::QdrantUrl;

    #[test]
    fn only_scratch_names_may_be_deleted() {
        assert!(check_scratch_names("htui_bench_d15e9fea", "htui_bench").is_ok());
        assert!(check_scratch_names("htui_bench_t_0190", "htui_bench_t_0190").is_ok());
        for (collection, schema) in [
            ("htui_concepts_v2", "htui_bench"),
            ("htui_bench_x", "public"),
            ("", "htui_bench"),
            ("htui_bench_x", "htui"),
        ] {
            let err = check_scratch_names(collection, schema).expect_err("refused");
            assert!(format!("{err}").contains("htui_bench"), "{err}");
        }
    }

    /// No service is reachable at the fixture's addresses: the refusal must come first.
    #[tokio::test]
    async fn run_with_refuses_a_real_schema_before_connecting() {
        let config = RunConfig {
            schema: "public".into(),
            ..config("postgres://nobody@127.0.0.1:1/none")
        };
        let corpus = build_from_sources("fixture", fixture_sources()).expect("builds");
        let err = run_with(
            &config,
            &corpus,
            &fixture_set(),
            &"0".repeat(64),
            htui_store::embed::HashEmbedder::new(htui_store::embed::DENSE_DIM),
        )
        .await
        .expect_err("refused");
        assert!(format!("{err}").contains("refusing schema"), "{err}");
    }

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
        assert_eq!(collection_name("Ünï"), "htui_bench__n_");
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
            bm25::query_vector("R-STO-8").indices.len()
        );
        assert!(record.sparse_terms > 0);
        assert_eq!(
            query_record(&query("d01", QueryClass::D, "?!", &[])).sparse_terms,
            0
        );
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
        let raw = Qdrant::from_url(&qdrant)
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

        let pool = pg_fts::connect(&database).await.expect("Postgres");
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
