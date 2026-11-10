# Blueprint: MOD-162 M1 - Retrieval benchmark harness

**Plan**: `.claude/plans/mod-162-m1-retrieval-benchmark.plan.md` (approved; decisions P1-P9 unchanged here).
**PRD**: `.claude/prds/mod-162-retrieval-benchmark.prd.md`. **Background**: `docs/ANA-37.md` §5.1-5.5, §11.
**Purpose**: the interfaces T1-T7 code against, so no wave-2 task waits for another. Where this file
and the plan disagree on a *fact*, this file carries the evidence (section 0); on a *decision*, the plan wins.

## 0. Plan facts corrected (evidence gathered 2026-11-10, branch `hr/MOD-162`)

| # | Plan says | Found | Evidence | Consequence |
|---|---|---|---|---|
| F1 | Requirement bullets are `- **R-AREA-N (prio).** …` (121) | 121 bullets in six header shapes: `(must)` 104, `.**` with no parenthesis 11 (all `R-LATER-1..13` but 3 and 7), `(must, promoted 2026-10-15)` 2, `(later)` 2, `(withdrawn)` 1, `(reopened, pending ANA-38)` 1 | `git show d15e9fea:docs/REQUIREMENTS.md \| grep -oE '^- \*\*R-[A-Z0-9]+-[0-9]+[^*]*\*\*'`, tallied | A parser keyed to `(must\|later)` finds 106. T3 rules in §3.2 cover all six |
| F2 | Decision headers: done 98, concluded 20, shipped 3, decided 3, rejected 1, 4 irregular (= 129) | 130 = done 98 + concluded 20 + shipped 3 + decided 3 + rejected 1 + **5** irregular: `(done; verdict withdrawn by MOD-25, …)` (ANA-10), `(status, 2026-09-14)` (ANA-3), `# MOD-35: Add Qdrant connection settings` (colon, no parenthesis), `# MOD-29 - Bugsink integration via Sentry crate` and `# TOOL-5 - Dev Postgres Crash Fix` (no parenthesis) | first line of every `docs/decisions/**/*.md` at the pin | T3's shape list adds `(status, …)`; all five map to `done` |
| F3 | Document points ≥ 700 (ANA-37: 759) | A line-for-line replica of `chunk_markdown(_, 2000)` over the 130 files at the pin gives **777** document points (1,099 points in all, not 1,081) | Python replica of `vector_sync.rs:266-287`, run 2026-11-10 | The `≥ 700` assertion holds. T3 prints the exact count. The 18-point gap from ANA-37 goes in the report's threats |
| F4 | T1 validate `cargo test -p htui-store --all-features vector` | That filter matches `vector::tests::*` only. The new live cases in `tests/qdrant_live.rs` have no "vector" in their names, so they never run | libtest name filtering | Commands in §7 |
| F5 | T4 validate `cargo test -p htui-bench --all-features lexical pg_fts` | `cargo test` takes one TESTNAME, so the second word is a usage error. Several filters must go after `--` | cargo CLI | Commands in §7 |
| F6 | (not stated) the query file is free text | `crates/htui-agent/tests/extensibility.rs:75-127` walks **every `.rs` and `.json` under `crates/`** for the forbidden literal (the word spelled z-e-t-a) | read 2026-11-10 | `queries/ana37-r2.json` is scanned. T5's shape check refuses that word (§3.4) |
| F7 | (not stated) `sqlx::query` takes a `String` | sqlx 0.9 takes `impl SqlSafeStr`. Only `&'static str` and `AssertSqlSafe<_>` implement it | `sqlx-core-0.9.0/src/sql_str.rs:52-114`, `query.rs:653` | SQL that contains the schema name is wrapped in `sqlx::AssertSqlSafe(sql)`, after `is_valid_schema` (§3.3) |
| F8 | P9 "drops and recreates its collection" | `QdrantStore::connect_to` refuses an existing collection from another embedder (`embedder mismatch`), and `drop_collection` needs a connected store. Example: a `HashEmbedder` test left the name behind | `vector.rs:757-813`, `check_collection` | `run` deletes through a raw `qdrant_client::Qdrant` first (§4.1 step 7) |

Verified as stated: 71 open items (keys unique, every bold header closes on its own line, a body is the
header line plus lines indented by two spaces, ending at a blank line, a heading or the next item); 130
decision files whose header key equals the upper-cased file stem; no key is both an open item and a
decision; the 322 corpus IDs are unique, and every ANA-37 §5.3 target (MOD-99, MOD-101, MOD-97,
TOOL-13, MOD-106, MOD-36, MOD-8, MOD-125, MOD-108, ANA-42, R-ORCH-15, MOD-130, DEC:ANA-4, R-TUI-18,
DEC:MOD-41, R-ORCH-7, ANA-33, MOD-94, MOD-98, R-ID-4, R-NF-3, R-STO-8) is one of them;
`pg/read.rs:145-147, 1016-1018` (H), ordered by `key_prefix, key_number` (`:166`) and
`area_code COLLATE "C", number` (`:1019`); `HashEmbedder` exists only under `test-support`
(`embed.rs:311-345`); every planned dependency is already in `Cargo.lock` (anyhow 1.0.104, clap 4.6.6,
qdrant-client 1.19.0, sqlx 0.9.0, tokio 1.53.1, serde 1.0.229, serde_json 1.0.151, sha2 0.10.9,
uuid 1.26.0, chrono 0.4.45). `uuid::Builder::from_custom_bytes` is not behind a feature (uuid 1.26
`builder.rs:602`).

**New dependencies: none.** The crate also needs one more file than the plan's table lists:
`crates/htui-bench/src/types.rs`. It holds the shared vocabulary, and T2 owns it inside its
`crates/htui-bench/**` set, so the disjointness proof is unchanged.

## 1. Constraints every implementer follows

- **TDD.** Write the tests named in your task first, watch them fail (red), then make them pass, then commit. Commit each task on its own (several commits are fine). Never leave work uncommitted.
- **Tests run single-threaded**: `-- --test-threads=1`. No `until ! pgrep -f "cargo test…"` wait loops.
- **The forbidden literal.** The four-letter word z-e-t-a must not appear in any file under `crates/`, whether `.rs` or `.json`, in code, comments, fixtures or query texts. Before you commit, check that `grep -rni 'z''eta' crates/htui-bench crates/htui-store` prints nothing.
- **No sqlx macros.** Use `sqlx::query` / `sqlx::raw_sql` only (no `query!`, `query_as!`), so no `.sqlx` entry and no `cargo sqlx prepare`. Wrap a dynamic SQL string in `sqlx::AssertSqlSafe(..)` (F7).
- **Workspace lints** (`[lints] workspace = true`): `unsafe_code = forbid`, `missing_debug_implementations` (every `pub` struct/enum derives or implements `Debug`), `unused_qualifications`, `clippy::all`. The gate turns warnings into errors. `lib.rs` adds `#![warn(missing_docs)]`, as `htui-store` does, so document every `pub` item, field and variant. Clippy's `all` group includes `too_many_arguments` (> 7): pass a struct instead.
- **No new external crate.** Use only `[workspace.dependencies]` entries. Anything else needs the maintainer.
- **Public signatures in this file are fixed.** A wave-2 task may add private items and extra `pub` items, but it never renames or changes a signature listed here, and it never edits a file it does not own (§6).
- **Secrets.** Never print a DSN, a Qdrant URL with userinfo, or an API key. `Debug` for anything holding one redacts it.
- **Errors.** Fallible functions in `htui-bench` return `anyhow::Result`. Where `htui-store` returns `StoreError`, add `.context(..)` at the call.
- **Live tests skip, never fail, when their service is not configured.** No URL in the environment means `eprintln!("skipped: <VAR> not set")` and return. Each live test owns a throwaway Qdrant collection or Postgres schema and drops it.

## 2. T1 - `htui-store` search arms (W1)

**Files**: `crates/htui-store/src/vector.rs`, `crates/htui-store/tests/qdrant_live.rs`.

### 2.1 Public API (added to `vector.rs`)

```rust
/// Which arms of the hybrid search [`QdrantStore::search_arms`] runs (MOD-162 P1). Production
/// searches `Hybrid` only, through [`VectorStore::search`]; the other two exist for the retrieval
/// benchmark, so it measures the production request rather than a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchArms {
    /// Both prefetch arms fused by RRF: exactly [`VectorStore::search`].
    Hybrid,
    /// The dense (cosine) arm alone.
    Dense,
    /// The sparse (BM25) arm alone. A query with no sparse term finds nothing.
    Sparse,
}

impl<E: DenseEmbedder> QdrantStore<E> {
    /// [`VectorStore::search`] with a choice of arms. `Hybrid` is that search byte for byte;
    /// `Dense` and `Sparse` send that arm's prefetch query as the top-level query, with the same
    /// vector, the same filter and `limit = query.limit`. `Sparse` embeds nothing.
    ///
    /// # Errors
    /// As [`VectorStore::search`]: [`QUERY_EMBED_FAILED`] when the model fails (not for
    /// `Sparse`), and a Qdrant error.
    pub async fn search_arms(&self, query: &SearchQuery, arms: SearchArms)
        -> Result<Vec<Hit>, StoreError>;
}
```

Put `search_arms` in the existing `impl<E: DenseEmbedder> QdrantStore<E>` block (it starts at `vector.rs:742`), and add `QueryPoints` to the `qdrant_client::qdrant::{…}` import.

### 2.2 The refactor

1. Add a private, pure request builder next to `search_filter` (`:675`):
   ```rust
   /// The request one search sends, or `None` when it would find nothing (a `Sparse` search with
   /// no term, or a dense arm with no vector). Pure, so the request shape is unit-tested.
   fn search_request(
       collection: &str,
       query: &SearchQuery,
       dense: Option<Vec<f32>>,
       sparse: Vec<(u32, f32)>,
       arms: SearchArms,
   ) -> Option<QueryPoints>
   ```
   - `Hybrid` is today's code from `:1295-1319`, **moved verbatim**: `filter = search_filter(query)`, `depth = query.limit.saturating_mul(4).max(20)`, `QueryPointsBuilder::new(collection).add_prefetch(PrefetchQueryBuilder::default().query(Query::new_nearest(dense)).using(DENSE).filter(filter.clone()).limit(depth)).query(Query::new_fusion(Fusion::Rrf)).filter(filter.clone()).limit(query.limit).with_payload(true)`, and the sparse prefetch is added only when `!sparse.is_empty()`. The function returns `Some(request.build())`.
   - `Dense`: `QueryPointsBuilder::new(collection).query(Query::new_nearest(dense)).using(DENSE).filter(search_filter(query)).limit(query.limit).with_payload(true).build()`.
   - `Sparse`: `None` when `sparse.is_empty()`. Otherwise the same shape with `.query(Query::from(sparse)).using(SPARSE)`.
   - `Hybrid` or `Dense` with `dense == None` returns `None`. The caller never does this.
2. `search_arms` body: the same early return (`query.projects.is_empty() || query.limit == 0` gives `Ok(vec![])`); then the embedding, unless `arms == Sparse`, with **the same `embed_failed` closure and messages as today**; then `sparse_pairs(&bm25::query_vector(&query.text))`; then `search_request(..)`, where `None` gives `Ok(vec![])`; then `self.client.query(request)` and the same `filter_map(|p| hit(&p.payload, p.score))`.
3. `impl VectorStore for QdrantStore<E>`: `async fn search(&self, query) { self.search_arms(query, SearchArms::Hybrid).await }`. The trait does not change. `MemVectorStore` and every caller stay untouched.

The only change on the hybrid path is that `client.query` receives a built `QueryPoints` instead of the builder (it takes `impl Into<QueryPoints>`, which is the same message).

### 2.3 Tests (write first)

Unit tests in `vector.rs` `mod tests` (they reuse `scoped(text)` from `:1506`). `QueryPoints` is a prost message with public fields and `PartialEq`:

- `hybrid_request_is_the_request_search_sent_before_mod_162`. The test module holds a private `fn pre_mod_162_request(collection, query, dense, sparse) -> QueryPoints`, a verbatim copy of today's builder lines from `:1294-1319` that ends in `.build()`. Assert `search_request(.., SearchArms::Hybrid) == Some(pre_mod_162_request(..))` in three cases: limit 10 with sparse terms, limit 3 without sparse terms, and a query with types, statuses and resolutions set.
- `hybrid_request_prefetches_four_times_the_limit_and_at_least_twenty`: limit 10 gives both prefetch `limit == Some(40)`; limit 3 gives `Some(20)`; top-level `limit == Some(limit)`.
- `hybrid_request_omits_the_sparse_arm_without_terms`: `prefetch.len() == 1`, `prefetch[0].using == Some(DENSE)`.
- `dense_request_queries_the_dense_vector_alone`: `prefetch.is_empty()`, `using == Some("dense")`, `limit == Some(query.limit)`, `filter == Some(search_filter(&query))`.
- `sparse_request_queries_the_sparse_vector_alone`: as above with `SPARSE`.
- `sparse_request_without_terms_is_none` and `dense_arms_without_a_vector_are_none`.

Live tests in `tests/qdrant_live.rs` (already `#![cfg(feature = "test-support")]`; use the `throwaway()` and `query()` helpers and `MemStore::demo()` with `Indexer::sync`, as at `:83-120`; import `SearchArms`):

- `each_search_arm_finds_an_exact_key`: an htui item's key is found by `Sparse` and by `Hybrid`. `Dense` returns `Ok`, at most `limit` hits, and none of them has an empty snippet.
- `the_hybrid_arms_are_the_vector_store_search`: for the key query, `"the plan"` and a no-term query (`"?!"`), `search_arms(Hybrid)` returns the same `Vec<Hit>` as `VectorStore::search`, scores included.
- `a_sparse_search_without_terms_finds_nothing`: `"?!"` with `Sparse` gives `Ok(vec![])`; `Dense` still answers `Ok`.
- `every_arm_is_scoped`: `projects: vec![]` gives an empty result for all three arms.

Each live test drops its collection.

## 3. T2-T5 - the `htui-bench` crate

### 3.0 T2: manifest, `lib.rs`, `types.rs`, `metrics.rs`, stubs (W1)

**Root `Cargo.toml`**: append `"crates/htui-bench"` to `members`. Nothing goes in `[workspace.dependencies]`, because no crate depends on the bench.

**`crates/htui-bench/Cargo.toml`** (complete):

```toml
[package]
name        = "htui-bench"
version     = "0.1.0"
description = "MOD-162: the retrieval benchmark over htui's own search; a dev tool, never shipped"

edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
publish.workspace      = true

[dependencies]
# Rows the point builders take (`Item`, `Requirement`, `Document`) and `QdrantUrl`.
htui-core     = { workspace = true }
# The production search, point builders and embedder (MOD-162 P1, P3). `local-embed`: `RtenEmbedder`
# and `model::ensure_model_noting`, as the `htui` binary loads them.
htui-store    = { workspace = true, features = ["local-embed"] }
# A raw client for what `QdrantStore` does not expose: the server version and deleting a collection
# another embedder made (blueprint F8). Already in the graph through htui-store.
qdrant-client = { workspace = true }
# The L baselines (P5). Runtime `sqlx::query` only: no macros, so no `.sqlx` offline entries.
sqlx          = { workspace = true }
tokio         = { workspace = true }
clap          = { workspace = true }
anyhow        = { workspace = true }
serde         = { workspace = true }
serde_json    = { workspace = true }
# The query-file freeze (P6) and the deterministic row UUIDs (P3).
sha2          = { workspace = true }
uuid          = { workspace = true }
chrono        = { workspace = true }

[dev-dependencies]
# `HashEmbedder` (behind `test-support`) for the runner's live wiring test; no model download.
htui-store = { workspace = true, features = ["test-support"] }

[lints]
workspace = true
```

There is no `[[bin]]` section: `src/main.rs` is the default binary `htui-bench`, and `src/lib.rs` is the library `htui_bench`. `Cargo.lock` gains only the `htui-bench` package entry, so build once and commit the lock.

**`src/lib.rs`** (final; T2 writes it and nobody else touches it):

```rust
//! MOD-162: htui's retrieval benchmark (`docs/htui-bench.md`). Rebuilds ANA-37 §5's measurement
//! in the repo: a corpus from a pinned commit, points through `htui_store::vector_sync`, the
//! production search (`QdrantStore::search_arms`), lexical baselines, a hash-frozen query set and
//! a report. A dev tool: nothing here ships in htui.
#![warn(missing_docs)]

pub mod corpus;   // T3
pub mod lexical;  // T4: G, G-dens, H
pub mod metrics;  // T2
pub mod pg_fts;   // T4: L-*
pub mod queries;  // T5
pub mod report;   // T6
pub mod run;      // T6
pub mod types;    // T2: shared vocabulary

pub use types::{Backend, CorpusDoc, CorpusId, DocKind, LVariant, QueryClass, Ranked};
```

**How stubs avoid warnings.** The binary declares no `mod` of its own and reaches everything through `htui_bench::…`, and every stub item is `pub`, so `dead_code` never fires on a library item. Stub bodies:
- return an empty value (`Vec::new()`, `None`, `BTreeSet::new()`, `Resolution::Done`, `Uuid::nil()`), or `anyhow::bail!("<module>::<fn>: not implemented yet (MOD-162 T<n>)")` for fallible functions. Never `todo!()` or `unimplemented!()`.
- consume every parameter with `let _ = (a, b, c);`, never `_`-prefixed names (wave 2 would have to rename them).
- read a private field from a stub method with `let _ = &self.field;`. Better still, stubs have no private fields: the public fields below are final.
- `run.rs` and `report.rs` stubs hold only their `//!` doc line, because T6 owns all three of its files.
- `main.rs` stub: `fn main() -> anyhow::Result<()> { anyhow::bail!("htui-bench: the CLI lands with MOD-162 T6") }`.

#### `src/types.rs` (T2, full implementation)

```rust
use htui_core::model::Item;
use htui_store::vector::Hit;

/// Depth of every ranked list recorded per query and backend (MRR@10, recall@5, report ranks).
pub const RANK_DEPTH: usize = 10;
/// `S`, `S-dense`, `S-sparse`: hits asked of Qdrant before collapsing (ANA-37 §5.1).
pub const S_LIMIT: u64 = 40;
/// `S10` and `S-points`: htui's default limit (`crates/htui/src/concepts.rs` `DEFAULT_LIMIT`).
pub const S10_LIMIT: u64 = 10;
/// Prefix of a closed item's (a decision's) corpus ID (ANA-37 §5.3).
pub const DECISION_PREFIX: &str = "DEC:";

/// A corpus document's ID, as queries name targets: `MOD-99` (open item), `R-STO-8`
/// (requirement), `DEC:ANA-4` (decision, the closed item and its summary document).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct CorpusId(String);
impl CorpusId {
    pub fn new(id: impl Into<String>) -> Self;
    pub fn decision(key: &str) -> Self;          // "DEC:" + key
    pub fn as_str(&self) -> &str;
    pub fn is_decision(&self) -> bool;
    /// P3: `DEC:<key>` when `hit.resolution.is_some()`, else `hit.key`.
    pub fn of_hit(hit: &Hit) -> Self;
    /// `DEC:<key>` for a closed item (`resolution.is_some()`), else its key.
    pub fn of_item(item: &Item) -> Self;
}
impl std::fmt::Display for CorpusId { /* the string */ }

/// Which source a document comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind { OpenItem, Requirement, Decision }

/// One corpus document: what L and G search, what queries target.
#[derive(Debug, Clone, PartialEq)]
pub struct CorpusDoc {
    pub id: CorpusId,
    pub kind: DocKind,
    /// `MOD-99`, `R-STO-8`, `ANA-4` (no `DEC:`).
    pub key: String,
    /// Item or decision title; empty for a requirement.
    pub title: String,
    /// The lexical text (§3.2): for items and requirements the point text exactly, for a
    /// decision the whole file.
    pub text: String,
    /// `HANDOFF.md:553`, `docs/REQUIREMENTS.md:290`, `docs/decisions/ana/ana-4.md` (report only).
    pub source: String,
}

/// One row of a ranked list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Ranked { pub id: CorpusId, pub score: f64 }

/// Every backend, in report order. Serialised as its `name()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Backend {
    #[serde(rename = "S")] S,
    #[serde(rename = "S10")] S10,
    #[serde(rename = "S-points")] SPoints,
    #[serde(rename = "S-dense")] SDense,
    #[serde(rename = "S-sparse")] SSparse,
    #[serde(rename = "L-and")] LAnd,
    #[serde(rename = "L-or")] LOr,
    #[serde(rename = "L-and-len")] LAndLen,
    #[serde(rename = "L-or-len")] LOrLen,
    #[serde(rename = "G")] G,
    #[serde(rename = "G-dens")] GDens,
    #[serde(rename = "H")] H,
}
impl Backend {
    pub const ALL: [Self; 12];                    // the order above
    pub const fn name(self) -> &'static str;      // "S", "S-points", "L-or-len", …
    /// L-*, G, G-dens, H: the baselines ("every lexical" in the report).
    pub const fn is_lexical(self) -> bool;
    pub const fn l_variant(self) -> Option<LVariant>;
}
impl std::str::FromStr for Backend { type Err = String; /* by name() */ }

/// The four Postgres full-text variants (P4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LVariant { And, Or, AndLen, OrLen }
impl LVariant {
    /// `ts_rank_cd` normalisation: 32 for And/Or, 34 (= 2|32) for the `-len` pair.
    pub const fn normalization(self) -> i32;
    /// Or, OrLen: the tsquery's `&` rewritten to `|`.
    pub const fn is_or(self) -> bool;
}

/// Query classes (ANA-37 §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum QueryClass {
    #[serde(rename = "a")] A, #[serde(rename = "b")] B, #[serde(rename = "c")] C, #[serde(rename = "d")] D,
}
impl QueryClass {
    pub const ALL: [Self; 4];
    pub const fn letter(self) -> &'static str;         // "a"…"d"
    pub const fn label(self) -> &'static str;          // "duplicates", "questions", "identifiers", "no target"
    pub const fn expected_count(self) -> usize;        // 24, 20, 16, 12
}

/// Hits to a ranked list of at most `depth` rows, IDs by [`CorpusId::of_hit`], score as f64.
/// `collapse`: one row per corpus ID, first occurrence wins (S, S10, S-dense, S-sparse);
/// otherwise one row per point, duplicates kept (S-points, ANA-37 §11).
pub fn ranked_from_hits(hits: &[Hit], collapse: bool, depth: usize) -> Vec<Ranked>;
```

The `// comments` mark the content; every item gets a `///` doc (missing_docs).

#### `src/metrics.rs` (T2, full implementation, P7)

```rust
/// z for the 95% Wilson interval.
pub const WILSON_Z: f64 = 1.96;
/// 1-based rank of the first entry of `ranked` that is one of `targets`; `None` when none is.
pub fn first_target_rank(ranked: &[CorpusId], targets: &[CorpusId]) -> Option<usize>;
/// `rank <= k`.
pub fn hit_at(rank: Option<usize>, k: usize) -> bool;
/// `1 / rank` when `rank <= cutoff`, else 0 (MRR@cutoff summand).
pub fn reciprocal_rank(rank: Option<usize>, cutoff: usize) -> f64;
/// Wilson score interval (low, high), clamped to [0, 1]; `n == 0` gives (0.0, 1.0).
pub fn wilson_interval(successes: usize, n: usize) -> (f64, f64);
/// Exact two-sided McNemar: `min(1, 2 · P[X <= min(b, c)])`, X ~ Bin(b + c, 1/2); `b + c == 0` gives 1.
/// Computed as t0 = 0.5^n, t(i+1) = t(i)·(n−i)/(i+1), summed in f64 (no overflow for n ≤ 1000).
pub fn mcnemar_exact(only_a: usize, only_b: usize) -> f64;
/// Mann-Whitney AUC of positives over negatives, a tie counting one half; `None` if either is empty.
pub fn auc(positives: &[f64], negatives: &[f64]) -> Option<f64>;
/// Fraction of positives strictly above every negative; `None` when positives is empty; an empty
/// negatives slice makes the threshold −∞.
pub fn recall_at_zero_false_alarms(positives: &[f64], negatives: &[f64]) -> Option<f64>;
/// Nearest-rank percentile (p in 0..=100) of a copy sorted with `f64::total_cmp`; `None` if empty.
pub fn percentile(values: &[f64], p: f64) -> Option<f64>;
```

T2 tests, written first: McNemar 13/0 → 0.000244, 15/0 → 0.000061, 6/0 → 0.03125, 5/0 → 0.0625, 8/5 → 0.581, 22/5 → 0.00151, 0/0 → 1, and symmetry (`f(b,c) == f(c,b)`). Each of these compares within a tolerance (`(x - want).abs() < 5e-4·want` or a 3-significant-digit check), never `==` on floats. Wilson 22/24 → (0.74, 0.98), 9/24 → (0.21, 0.57) and 47/60 → (0.66, 0.87), each to two places. AUC [3,2] against [1] is 1.0; [1] against [1] is 0.5; [1,3] against [2] is 0.5. Zero false alarms: [0.9, 0.5, 0.8] against [0.7] gives 2/3, and [0.5] against [0.5] gives 0. `first_target_rank`, `reciprocal_rank`, `percentile` (p50 and p95 of 1..=20 are 10 and 19). From `types.rs`: `of_hit` with and without a resolution; `ranked_from_hits` collapse (first wins, depth respected) against uncollapsed duplicates; `Backend` and `QueryClass` names round-trip through `FromStr` and through serde.

### 3.1 Deterministic IDs and rows (T3)

```rust
/// Namespaced UUID (P3): the first 16 bytes of sha256("htui-bench/v1\0" + name), stamped as a
/// version-8 (custom) UUID by `uuid::Builder::from_custom_bytes(..).into_uuid()`.
pub fn bench_uuid(name: &str) -> Uuid;
```

Names, so that equal inputs give equal points on every run:

| Row | Name | Field |
|---|---|---|
| project | `project` | `ProjectId` |
| open item | `item:<KEY>` | `ItemId` (also its point ID) |
| decision item | `item:DEC:<KEY>` | `ItemId` |
| decision summary | `document:DEC:<KEY>` | `DocumentId` |
| item kind | `kind:<PREFIX>` | `ItemKindId` |
| requirement | `requirement:<R-KEY>` | `RequirementId` |
| requirement area | `area:<CODE>` | `RequirementAreaId` |
| user | `user` | `UserId` (`created_by`) |

Constant fields: `BENCH_TIME` = `DateTime::<Utc>::from_timestamp(1_760_000_000, 0)`, used for created_at and updated_at (and closed_at on closed items).

- **Item**: `key_prefix` and `key_number` split at the last `-`; `priority` 0; `required_tags` and `touched_paths` empty; `step_graph_id` None; `version` 1.
- **Open item**: `status = Open`, `resolution = None`, `closed_at = None`.
- **Decision item**: `status = Closed`, `resolution = Some(r)`, `closed_at = Some(BENCH_TIME)`, `body = ""`.
- **Requirement**: `area_code` and `number` from the key, `rationale = ""`, `version` 1.
- **Document**: `kind = "summary"`, `version = 1`, `title` = the decision title, `body` = the whole file, `produced_by_step_id = None`.

### 3.2 T3 - `src/corpus.rs` (W2)

```rust
/// ANA-37's corpus commit.
pub const DEFAULT_PIN: &str = "d15e9fea";
/// `--corpus-ref` value meaning "the files on disk", not a commit.
pub const WORKTREE: &str = "WORKTREE";

/// The three sources of one corpus, before parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    /// The resolved full commit sha; `None` for the work tree.
    pub commit: Option<String>,
    /// `HANDOFF.md`.
    pub handoff: String,
    /// `docs/REQUIREMENTS.md`.
    pub requirements: String,
    /// `(repo-relative path, text)` of every `docs/decisions/**/*.md`, sorted by path.
    pub decisions: Vec<(String, String)>,
}

/// An open item of `HANDOFF.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItem { pub key: String, pub title: String, pub body: String, pub line: usize }
/// A requirement bullet of `docs/REQUIREMENTS.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementBullet {
    pub key: String, pub area_code: String, pub number: i32,
    pub priority: Priority, pub state: RequirementState, pub body: String, pub line: usize,
}
/// A decision file's first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionHeader { pub key: String, pub title: String, pub resolution: Resolution }

/// The corpus: documents for L, G and the query targets; rows for H and the point builders.
#[derive(Debug, Clone, PartialEq)]
pub struct Corpus {
    /// As requested: a rev, or [`WORKTREE`].
    pub corpus_ref: String,
    /// Resolved commit; `None` for the work tree.
    pub commit: Option<String>,
    /// The one synthetic project (`bench_uuid("project")`).
    pub project: ProjectId,
    /// Open items in file order, then requirements in file order, then decisions in path order.
    pub docs: Vec<CorpusDoc>,
    /// Open items, then decision items, in `docs` order.
    pub items: Vec<Item>,
    /// In `docs` order.
    pub requirements: Vec<Requirement>,
    /// One `summary` per decision item, in the same order.
    pub documents: Vec<Document>,
}

/// Points by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct PointCounts { pub item: usize, pub requirement: usize, pub document: usize }

impl Corpus {
    pub fn ids(&self) -> BTreeSet<CorpusId>;
    pub fn doc(&self, id: &CorpusId) -> Option<&CorpusDoc>;
    /// `item_point` for every item, `document_points(item, doc)` for every summary,
    /// `requirement_point` for every requirement, in that order; `htui_store::vector_sync`
    /// unchanged (P3).
    pub fn points(&self) -> Vec<ConceptPoint>;
    /// Counts by `ConceptPoint::point_type()`.
    pub fn point_counts(&self) -> PointCounts;
}

pub fn bench_uuid(name: &str) -> Uuid;
/// `git -C repo rev-parse --verify <rev>^{commit}`, then `git show <sha>:HANDOFF.md`,
/// `git show <sha>:docs/REQUIREMENTS.md`, `git ls-tree -r --name-only <sha> -- docs/decisions`
/// (only `*.md`) and `git show` of each. An unreachable rev is an error that names the rev and says
/// "unreachable in this clone (shallow?); pass --corpus-ref".
pub fn read_git(repo: &Path, rev: &str) -> anyhow::Result<Sources>;
/// The same three sources from disk under `root`; decisions found by a recursive `std::fs` walk
/// (no new crate), sorted by repo-relative path with `/` separators.
pub fn read_worktree(root: &Path) -> anyhow::Result<Sources>;
/// Pure: parse, build rows and documents. An error names a decision file it cannot key.
pub fn build_from_sources(corpus_ref: &str, sources: Sources) -> anyhow::Result<Corpus>;
/// `read_git` then `build_from_sources`.
pub fn build_from_git(repo: &Path, rev: &str) -> anyhow::Result<Corpus>;
/// [`WORKTREE`] gives `read_worktree`, anything else `build_from_git`.
pub fn build(repo: &Path, corpus_ref: &str) -> anyhow::Result<Corpus>;
pub fn parse_open_items(handoff: &str) -> Vec<OpenItem>;
pub fn parse_requirements(text: &str) -> Vec<RequirementBullet>;
pub fn parse_decision_header(first_line: &str) -> Option<DecisionHeader>;
/// `Resolution::from_str(word)`, falling back to `Done` (shipped, decided, status, anything).
pub fn resolution_of(word: &str) -> Resolution;
```

Git runs as `std::process::Command::new("git")`. Read stdout as UTF-8 (`String::from_utf8`, error on failure) and check the exit status.

**Parsing rules.** Line numbers are 1-based. Lines are split with `str::lines()`, which also drops a `\r`.

1. **Open items** (whole file):
   - A header line matches `^- \[ \] \*\*(KEY) - (TITLE)\*\*(REST)$`, where KEY is `[A-Z][A-Z0-9]*-[0-9]+` and TITLE is the bold text after the first ` - `, up to the first closing `**`, trimmed and kept verbatim (it may end in `(from ANA-13).`).
   - The block continues on every following line that starts with two spaces. It ends at a blank line, a `#` heading, any other line, or the next header.
   - `body` = `REST.trim()`, then each continuation line with its first two spaces stripped, joined with `\n`, the whole trimmed.
   - `- [x]` lines and bullets without the bold `KEY - ` are ignored.
   - At the pin this yields 71 items, keys unique.
2. **Requirements** (whole file):
   - A header matches `^- \*\*(R-([A-Z][A-Z0-9]*)-([0-9]+))( \(([^)]*)\))?\.\*\*(REST)$`.
   - Continuation and termination work as for open items. `body` = `REST.trim()` plus the stripped continuation lines, joined with `\n`, trimmed.
   - Let `word` be the parenthesis content before its first `,` or `;`, trimmed and lowercased (absent when there is no parenthesis).
   - `state` = `Withdrawn` if `word == "withdrawn"`, else `Active`.
   - `priority` = `Later` if `word == "later"`, or if there is no parenthesis and the area is `LATER`; else `Must`.
   - At the pin this yields 121 bullets: `R-STO-7` withdrawn, `R-ENT-13` active must, `R-LATER-3/-7` must, and the other `R-LATER-*` later (F1).
3. **Decision header**, `parse_decision_header(first_line)`:
   - Strip `# `. KEY = leading `[A-Z][A-Z0-9]*-[0-9]+`; no key gives `None`.
   - Next comes the separator ` - ` or `: ` (anything else: `None`).
   - If the rest ends with `)`, find the matching `(` scanning backwards with depth counting. If the group's first word, ending at `,`, `;`, a space or `)`, is non-empty lowercase ASCII, it is the status group: `resolution = resolution_of(word)` and `title` = the text before the group, trimmed.
   - Otherwise `title` = the rest, trimmed, and `resolution = Done`.
   - Every shape at the pin, each a test case: `(done, 2026-09-03)`, `(concluded, …)`, `(shipped, …)`, `(decided, …)`, `(rejected, …)`, `(done; verdict withdrawn by MOD-25, …)`, `(status, 2026-09-14)`, `# MOD-35: Add Qdrant connection settings`, and no parenthesis at all (`# TOOL-5 - Dev Postgres Crash Fix`). A title holding `(from X)` followed by a status group keeps its inner parenthesis.
   - In `build_from_sources`, the key comes from the header. When the header has none, the upper-cased file stem is used (at the pin they agree for all 130).
4. **`CorpusDoc.text`**:
   - item: `item_point(&item).text`, which is `"{key} {title}\n\n{body}"`.
   - requirement: `requirement_point(&row).text`.
   - decision: the file text, verbatim.
   - `title`: the item or decision title; `""` for requirements.
   - `source`: `HANDOFF.md:<line>`, `docs/REQUIREMENTS.md:<line>`, or the decision path.

**T3 tests (first):**
- Open items: a fixture with three items, one whose body runs to a blank line, one to a `###` heading, one to the next item; a `- [x]` line; a header with trailing text after `**`; the title verbatim.
- Requirements: one fixture line per F1 shape, with the priority and state each gives; continuation lines; termination at a blank line, a heading and the next bullet.
- Decision headers: every shape in rule 3, plus `resolution_of` (`"shipped"` gives `Done`, `"rejected"` gives `Rejected`).
- `bench_uuid` is deterministic, distinct across names and version 8.
- `build_from_sources` on a fixture: `CorpusDoc.text == item_point(&items[i]).text`; decision IDs are `DEC:`; `points()` gives one item point per item, ≥ 1 document point per decision and one point per requirement; all point IDs are unique.
- `the_pinned_corpus_is_ana37s` reads `env!("CARGO_MANIFEST_DIR")/../..` through git at `DEFAULT_PIN` and asserts:
  - docs by kind: 71 / 121 / 130;
  - `ids().len() == 322`, and `MOD-99`, `R-STO-8` and `DEC:ANA-4` are present;
  - `point_counts()`: item == 201, requirement == 121, document ≥ 700, with `eprintln!` of the exact count (expected 777, F3).
  - It prints `skipped: d15e9fea unreachable` and passes when `git rev-parse` fails (no git, shallow clone).

### 3.3 T4 - `src/lexical.rs` and `src/pg_fts.rs` (W2)

**`lexical.rs`:**

```rust
/// Postgres' `english.stop` (snowball, 127 words), verbatim, so G drops what L drops.
pub const STOPWORDS: &[&str] = &[ /* list below */ ];
/// G's keywords: lowercase; split on every char that is not alphanumeric, `-` or `_`; trim `-`/`_`
/// at both ends; drop tokens shorter than 2 chars and stopwords; dedupe, first occurrence order.
/// `"R-STO-8"` → `["r-sto-8"]`; `"Plackett-Luce"` → `["plackett-luce"]`.
pub fn keywords(query: &str) -> Vec<String>;

/// G's tie-break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GTie { /// G. Occurrences, /// G-dens. Density }

/// Lower-cased corpus texts, built once (timing G measures the search, not the lowercasing).
#[derive(Debug, Clone)]
pub struct GrepIndex { pub ids: Vec<CorpusId>, pub lower: Vec<String>, pub words: Vec<usize> }
impl GrepIndex {
    /// `lower[i] = docs[i].text.to_lowercase()`, `words[i] = max(1, whitespace word count)`.
    pub fn new(docs: &[CorpusDoc]) -> Self;
    /// G / G-dens over the docs. For each doc: `matched` = keywords k with `lower.contains(k)`,
    /// `occurrences` = Σ `lower.matches(k).count()`, `density` = occurrences / words. Docs with
    /// matched == 0 are dropped. Sort by (matched desc, then occurrences or density desc, then doc
    /// order asc); take `limit`. `Ranked.score` = matched / keywords.len() (the §5.4 "fraction
    /// matched"). No keywords → empty.
    pub fn search(&self, query: &str, tie: GTie, limit: usize) -> Vec<Ranked>;
}

/// H (P4): htui's no-Qdrant filter, mirrored. needle = `query.to_lowercase()` (not trimmed: the
/// production filter passes the text as typed). Items whose `key` or `title` lowercased contains
/// it, ordered by (`key_prefix` bytes, `key_number`); then requirements whose `key` or `body`
/// does, ordered by (`area_code` bytes, `number`). Item IDs by `CorpusId::of_item`. Score 1.0.
/// First `limit`.
pub fn htui_filter(items: &[Item], requirements: &[Requirement], query: &str, limit: usize) -> Vec<Ranked>;
```

STOPWORDS (Postgres `tsearch_data/english.stop`, in order): i me my myself we our ours ourselves you
your yours yourself yourselves he him his himself she her hers herself it its itself they them their
theirs themselves what which who whom this that these those am is are was were be been being have has
had having do does did doing a an the and but if or because as until while of at by for with about
against between into through during before after above below to from up down in out on off over under
again further then once here there when where why how all any both each few more most other some such
no nor not only own same so than too very s t can will just don should now. (127 words.)

**`pg_fts.rs`** (P5):

```rust
/// The run's schema.
pub const SCHEMA: &str = "htui_bench";
/// `^[a-z_][a-z0-9_]{0,62}$`: the schema name is interpolated into SQL, so it is checked first.
pub fn is_valid_schema(name: &str) -> bool;
/// The search statement for `variant` (pure; unit-tested). Panics never; callers check the schema.
pub fn search_sql(schema: &str, variant: LVariant) -> String;
/// A small pool (`PgPoolOptions::new().max_connections(2)`); the error never contains the DSN.
pub async fn connect(url: &str) -> anyhow::Result<sqlx::PgPool>;
/// `SELECT version()`.
pub async fn server_version(pool: &sqlx::PgPool) -> anyhow::Result<String>;
/// Drops `schema` if it exists, creates it and its table, loads one row per doc, ANALYZEs.
pub async fn load(pool: &sqlx::PgPool, schema: &str, docs: &[CorpusDoc]) -> anyhow::Result<()>;
/// Top `limit` docs for `text` under `variant`, ties by doc order.
pub async fn search(pool: &sqlx::PgPool, schema: &str, variant: LVariant, text: &str, limit: usize)
    -> anyhow::Result<Vec<Ranked>>;
/// `DROP SCHEMA IF EXISTS <schema> CASCADE`.
pub async fn drop_schema(pool: &sqlx::PgPool, schema: &str) -> anyhow::Result<()>;
```

SQL, one statement per `sqlx::query(sqlx::AssertSqlSafe(..)).execute(pool)` (`{s}` = the checked schema):

```sql
DROP SCHEMA IF EXISTS {s} CASCADE
CREATE SCHEMA {s}
CREATE TABLE {s}.doc (
    ord  integer PRIMARY KEY,
    id   text    NOT NULL UNIQUE,
    body text    NOT NULL,
    tsv  tsvector GENERATED ALWAYS AS (to_tsvector('english', body)) STORED
)
INSERT INTO {s}.doc (ord, id, body) SELECT * FROM UNNEST($1::int4[], $2::text[], $3::text[])
CREATE INDEX doc_tsv ON {s}.doc USING gin (tsv)
ANALYZE {s}.doc
```

`body` = `CorpusDoc.text`; `ord` = index in `docs`. Search (`$1` text, `$2` int4 normalisation, `$3` int8 limit):

```sql
SELECT d.id, ts_rank_cd(d.tsv, w.q, $2)::float8 AS score
  FROM {s}.doc d
 CROSS JOIN (SELECT {TSQ} AS q) w
 WHERE d.tsv @@ w.q
 ORDER BY score DESC, d.ord
 LIMIT $3
```

`{TSQ}` = `websearch_to_tsquery('english', $1)` for And/AndLen;
`replace(websearch_to_tsquery('english', $1)::text, ' & ', ' | ')::tsquery` for Or/OrLen. The rewrite
works on the output text, and the `::tsquery` cast re-parses it **without** stemming the
already-stemmed lexemes again (`to_tsquery('english', ..)` would). An all-stopword query gives an
empty tsquery, which matches nothing. A Postgres error is returned as `Err`; the runner records it
and counts the query as no hits.

**T4 tests (first):**
- lexical: `keywords` drops stopwords and keeps `r-sto-8` and `plackett-luce`.
- G ranks by matched count, then by occurrences or by density: a fixture where one long doc wins G and a short doc wins G-dens.
- No keyword means no rows; `score` is the fraction matched.
- H is case-insensitive, matches item key and title but **not** item body, matches requirement key and body, puts items before requirements, maps a closed item to `DEC:`, orders `MOD-9` before `MOD-10`, and treats the whole query as one needle (`"alpha beta"` does not match a title `"beta alpha"`).
- pg_fts unit: `is_valid_schema` (rejects `a;b`, `Upper`, `""`); `search_sql` contains `websearch_to_tsquery` with or without `replace(`.
- `pg_fts_variants_on_a_five_document_corpus`, live, needs `HTUI_TEST_DATABASE_URL`; schema `htui_bench_t_<uuid v7 simple>`, dropped at the end:
  - Documents: 1 `alpha beta`; 2 `alpha gamma`; 3 `beta gamma delta`; 4 `alpha beta` ×50 plus 2,000 filler words; 5 `unrelated text`.
  - `L-and "alpha beta"` gives exactly {1, 4}; `L-or` gives {1, 2, 3, 4}.
  - `L-or` ranks doc 4 first and `L-or-len` ranks doc 1 first (length normalisation changes the ranking).
  - `"the and"` gives `Ok(vec![])` for all four variants.

### 3.4 T5 - `src/queries.rs` and the frozen set (W2)

**File format, `crates/htui-bench/queries/ana37-r2.json`**: UTF-8, LF, final newline, one query object per line for reviewable diffs:

```json
{
  "set": "ana37-r2",
  "corpus_ref": "d15e9fea",
  "queries": [
    {"id": "a01", "class": "a", "text": "crash telemetry address is hard-wired …", "targets": ["MOD-99"], "completed": true},
    {"id": "d01", "class": "d", "text": "…", "targets": []}
  ]
}
```

**Sidecar, `ana37-r2.json.sha256`**: `sha256sum` format, `<64 lowercase hex>␠␠ana37-r2.json\n`, so `cd crates/htui-bench/queries && sha256sum -c ana37-r2.json.sha256` verifies it. T5 makes it with exactly that command (`sha256sum ana37-r2.json > ana37-r2.json.sha256`, run inside the directory).

```rust
/// The frozen set, compiled-in path (for `cargo run`).
pub const DEFAULT_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/queries/ana37-r2.json");

/// One query.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub id: String,
    pub class: QueryClass,
    pub text: String,
    pub targets: Vec<CorpusId>,
    /// A §5.3 text quoted truncated and completed by the author (a01, a04, a20).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub completed: bool,
}
/// The file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySet { pub set: String, pub corpus_ref: String, pub queries: Vec<Query> }
/// A set whose bytes matched its sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenQueries { pub path: PathBuf, pub sha256: String, pub set: QuerySet }
/// A target that names no corpus document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTarget { pub query: String, pub target: CorpusId }
impl std::fmt::Display for UnknownTarget { /* "a07: MOD-999 is not a corpus ID" */ }

pub fn sha256_hex(bytes: &[u8]) -> String;
/// `<file>.sha256`.
pub fn sidecar_path(file: &Path) -> PathBuf;
/// `"{sha}  {file_name}\n"`.
pub fn sidecar_line(sha256: &str, file_name: &str) -> String;
/// First whitespace-separated token; must be 64 lowercase hex chars.
pub fn parse_sidecar(text: &str) -> anyhow::Result<String>;
/// Pure freeze check: `sha256_hex(bytes) == parse_sidecar(sidecar)`, else an error that names both
/// hashes and says the query file changed after its freeze. Returns the hash.
pub fn check_frozen(bytes: &[u8], sidecar: &str) -> anyhow::Result<String>;
pub fn parse(bytes: &[u8]) -> anyhow::Result<QuerySet>;
/// Every problem in one error, one per line: counts per class 24/20/16/12; ids unique and of the form
/// `<class letter><two digits>`, numbered 01..=count; non-empty text; class `a` exactly one target
/// with neither a `DEC:` nor an `R-` prefix (an open item); `b` and `c` at least one target; `d`
/// none; `completed` only on class a/b; no text or target containing the forbidden literal
/// (case-insensitive; spell it in code as `concat!("z", "eta")`).
pub fn check_shape(set: &QuerySet) -> anyhow::Result<()>;
/// Read file and sidecar, `check_frozen`, `parse`, `check_shape`.
pub fn load_frozen(path: &Path) -> anyhow::Result<FrozenQueries>;
/// Every target, in file order, that is not in `corpus_ids` (empty means valid).
pub fn validate(queries: &[Query], corpus_ids: &BTreeSet<CorpusId>) -> Vec<UnknownTarget>;
```

**Authoring rules (P6).** Classes and counts: a 24, b 20, c 16, d 12. Corpus IDs at the pin, so T5 can validate before T3 lands:

```bash
{ git show d15e9fea:HANDOFF.md | grep -oE '^- \[ \] \*\*[A-Z][A-Z0-9]*-[0-9]+' | cut -c9-
  git show d15e9fea:docs/REQUIREMENTS.md | grep -oE '^- \*\*R-[A-Z0-9]+-[0-9]+' | cut -c5-
  git ls-tree -r --name-only d15e9fea docs/decisions | sed -E 's#.*/##; s#\.md$##' | tr a-z A-Z | sed 's/^/DEC:/'
} > /tmp/corpus-ids.txt   # 322 lines, 322 unique (checked 2026-11-10)
```

Verbatim texts and their known targets (ANA-37 §5.3). Each row is a query of the set:

| id | text (verbatim unless *completed*) | targets |
|---|---|---|
| a01 | *completed*, starts `crash telemetry address is hard-wired` | MOD-99 |
| a03 | `save the process return status of every agent invocation` | MOD-101 |
| a04 | *completed*, contains `flat-rate plan` then `provider credentials from my shell`, in order | MOD-97 |
| a05 | author's (ANA-37 names only the target) | MOD-94 |
| a08 | `docs checker should fail when a write-up does not mention requirements` | TOOL-13 |
| a09 | `never two repositories flagged as the main one` | MOD-106 |
| a10 | author's | MOD-98 |
| a11 | `choose among assistants with odds proportional to strength` | MOD-36 |
| a15 | `bring markdown backlog and decision log into the database` | MOD-8 |
| a18 | `pause work in progress and pick it up later` | MOD-125 |
| a20 | *completed*, contains `check out a repository onto a machine` then `keep the git login`, in order | MOD-108 |
| b01 | `where do we decide which machine runs a step` | ANA-42, R-ORCH-15, MOD-130 |
| b03 | `keep passwords out of the logs` | author finds |
| b04 | `can two people work on the same backlog` | author finds |
| b05 | `decide against an in-memory cache server` | author finds |
| b07 | `which decision chose the protocol we use to talk to coding assistants` | DEC:ANA-4 |
| b08 | author's | R-ID-4 |
| b09 | `how are crash reports collected` | author finds |
| b10 | author's | R-NF-3 |
| b11 | `engine for finding similar tickets by meaning` | author finds |
| b12 | `finished work merged back into the main branch` | author finds |
| b14 | `big compile jobs queued so the computer is not overloaded` | author finds |
| b15 | `what happens to running work when I close the app` | R-TUI-18, DEC:MOD-41 |
| b20 | `how do agents split one task into pieces that run at the same time` | R-ORCH-7, ANA-33 |
| c09 | author's, an identifier lookup for `ConPTY` | author finds |
| c11 | author's, an identifier lookup for `Plackett-Luce` | author finds |
| c16 | `R-STO-8` | R-STO-8 |

The rest of the set is author-written to P6's rules: class a paraphrases of one open item that avoid
its title words; b questions whose answer uses other words; c keys and rare terms; d plausible requests
with no target and near-miss distractors in the corpus. The `completed` flag appears on a01, a04 and
a20 only. Record nothing per query beyond the five fields. **Commit the JSON and sidecar alone**
(`docs(MOD-162): freeze the ana37-r2 query set`) before any backend runs, then commit `queries.rs`.

**T5 tests (first):**
- `the_committed_set_hashes_to_its_sidecar`: `include_bytes!` and `include_str!` of the two files, then `check_frozen`.
- `the_committed_set_has_ana37s_shape`: `parse` and `check_shape`; 24/20/16/12; the 18 verbatim texts above are equal byte for byte; the completed fragments occur in order; `completed` is set exactly on a01, a04, a20.
- `check_frozen` refuses one flipped byte; `parse_sidecar` refuses a short or upper-case hash.
- `check_shape` names each violation: wrong count, duplicate id, a class-d target, an a-target with `DEC:`.
- `validate` names every unknown target and returns empty on a complete set.

Validating the committed set against the real corpus is T6's `validate` subcommand (W3).

## 4. T6 - runner, report, CLI (W3; T1 merged)

### 4.1 `src/run.rs`

```rust
/// Where and what to run. `Debug` is hand-written and prints `database_url` as `<redacted>`.
pub struct RunConfig {
    pub repo: PathBuf,
    pub corpus_ref: String,
    pub queries: PathBuf,
    pub out: PathBuf,
    pub qdrant: QdrantSettings,
    pub database_url: String,
    pub collection: String,
    pub schema: String,
}
/// `htui_bench_` + corpus_ref lowercased, every char outside [a-z0-9] → `_`, at most 48 chars
/// (`d15e9fea` → `htui_bench_d15e9fea`; `WORKTREE` → `htui_bench_worktree`).
pub fn collection_name(corpus_ref: &str) -> String;
/// The whole run: `queries::load_frozen` (refuses a changed file), `corpus::build`,
/// `queries::validate` (refuses, listing each unknown target), the production model, `run_with`.
pub async fn run(config: &RunConfig) -> anyhow::Result<Results>;
/// Everything after the model is loaded; generic so a test runs it with `HashEmbedder`.
pub async fn run_with<E: DenseEmbedder>(
    config: &RunConfig, corpus: &Corpus, queries: &QuerySet, query_sha256: &str, embedder: E,
) -> anyhow::Result<Results>;
/// `ensure_model_noting(|l| eprintln!("{l}"))`, then `RtenEmbedder::load` on `spawn_blocking`,
/// with `.context("cannot load the embedding model")` (as `crates/htui/src/concepts.rs:134-142`).
pub async fn load_embedder() -> anyhow::Result<RtenEmbedder>;
```

Pipeline in `run`/`run_with`. Progress goes to stderr, prefixed `htui-bench:`.

1. `load_frozen(&config.queries)`. A hash mismatch is fatal.
2. `corpus::build(&config.repo, &config.corpus_ref)`; print the doc counts.
3. `queries::validate`: non-empty means print each, then bail. A set whose `corpus_ref` differs from `config.corpus_ref` gets a warning.
4. Postgres: `pg_fts::connect`, `server_version`. Fail fast, before the slow indexing.
5. Qdrant: a raw `qdrant_client::Qdrant::from_url(url).skip_compatibility_check()` (+ `.api_key`), then `health_check().await?.version` (the compatibility check prints to stdout, so skip it, as `vector.rs:1028` does).
6. `load_embedder()` (timed), only in `run`.
7. Clean collection (F8): `if raw.collection_exists(name) { raw.delete_collection(name) }`, then `QdrantStore::connect_to(&qdrant, embedder, name)`.
8. `corpus.points()` upserted in chunks of 64 through `VectorStore::upsert`, timed, printing `indexed {n}/{total} points ({s:.1} s)` per chunk.
9. `pg_fts::load(pool, schema, &corpus.docs)` (timed); build `GrepIndex::new(&corpus.docs)`.
10. Warm-up, untimed: one `search_arms` per arm and one `pg_fts::search` on the first query.
11. For each query in file order, for each backend in `Backend::ALL` order, time the call with `Instant` (embedding included for S/S-dense, as ANA-37 §5.5 measured):
    - `SearchQuery { text, projects: vec![corpus.project], types/statuses/resolutions: vec![], limit }`.
    - **S**: `search_arms(Hybrid)` with `S_LIMIT`, `ranked_from_hits(.., collapse = true, RANK_DEPTH)`.
    - **S10**: `Hybrid` with `S10_LIMIT`, collapsed.
    - **S-points**: the **same response** as S10, uncollapsed (no second call; latency copied).
    - **S-dense** / **S-sparse**: `Dense` / `Sparse` with `S_LIMIT`, collapsed.
    - **L-***: `pg_fts::search(.., b.l_variant(), text, RANK_DEPTH)`.
    - **G** / **G-dens**: `grep.search(text, Occurrences / Density, RANK_DEPTH)`.
    - **H**: `lexical::htui_filter(&corpus.items, &corpus.requirements, text, RANK_DEPTH)`.
    - An `Err` becomes `error: Some(msg)` with an empty `ranked`.
    - `first_rank = metrics::first_target_rank(ids(ranked), targets)`.
12. `pg_fts::drop_schema`, also on the error path (best effort, logged). The Qdrant collection is left in place for inspection; the next run recreates it.
13. Return `Results`. `main` writes it through `report::write`.

**`Results` schema** (`results.json`, `serde_json::to_string_pretty` + `\n`), `schema: 1`:

```rust
pub struct Results { pub schema: u32, pub meta: RunMeta, pub queries: Vec<QueryRecord>, pub runs: Vec<BackendRun> }
pub struct RunMeta {
    pub started_at: DateTime<Utc>, pub finished_at: DateTime<Utc>,
    pub corpus_ref: String, pub corpus_commit: Option<String>,
    pub docs: BTreeMap<String, usize>,          // "open_item", "requirement", "decision", "total"
    pub points: PointCounts,
    pub query_set: String, pub query_file: String, pub query_sha256: String,
    pub embedder: String,                       // EmbedderIdentity Display
    pub embedder_identity: EmbedderIdentity,
    pub qdrant_version: String, pub collection: String,
    pub postgres_version: String,               // never the DSN
    pub htui_commit: String, pub htui_dirty: bool,   // git rev-parse HEAD / status --porcelain
    pub host: HostInfo,                         // os (std::env::consts::OS), cpus (available_parallelism),
                                                // load_start/load_end: Option<[f64; 3]> from /proc/loadavg
    pub timings_ms: Timings,                    // model_load (None in run_with), index, pg_load, queries
    pub rank_depth: usize,
}
pub struct QueryRecord {
    pub id: String, pub class: QueryClass, pub text: String, pub targets: Vec<CorpusId>,
    /// `htui_store::bm25::query_vector(text).indices.len()`: §5.4's "per query term".
    pub sparse_terms: usize,
}
pub struct BackendRun {
    pub backend: Backend, pub query: String, pub ranked: Vec<Ranked>,
    pub first_rank: Option<usize>, pub latency_ms: f64, pub error: Option<String>,
}
```

All of these derive `Debug, Clone, PartialEq, Serialize, Deserialize`; `HostInfo` and `Timings` are structs in `run.rs`.

### 4.2 `src/report.rs`

```rust
/// A class group of the tables: a, b, c, and a+b+c.
pub enum Group { A, B, C, Abc }
/// Per backend and group: n, hits@1, hits@5, MRR@10, Wilson(hits@5).
pub struct Cell { pub n: usize, pub hit1: usize, pub hit5: usize, pub mrr10: f64, pub wilson5: (f64, f64) }
pub struct Summary { /* cells: BTreeMap<(Backend, Group), Cell>, mcnemar rows, separability rows,
                        rescue lists, latency, errors, thresholds */ }
pub fn summarize(results: &Results) -> Summary;                 // pure
pub fn render_markdown(results: &Results, summary: &Summary) -> String;   // pure
/// Writes `<out>/results.json` and `<out>/report.md`, creating `out`.
pub fn write(results: &Results, out: &Path) -> anyhow::Result<()>;
/// A results file, or a directory holding `results.json`.
pub fn load_results(path: &Path) -> anyhow::Result<Results>;
pub struct Comparison { /* see below */ }
pub fn compare(a: &Results, b: &Results) -> Comparison;          // pure
pub fn render_comparison(c: &Comparison) -> String;              // pure
```

Every struct and enum here derives `Debug`; the `(Backend, Group)` key derives `Ord`.

Semantics:

- **Recall@k / MRR@10** are over classes a, b, c only. `hit@k = first_rank <= k`. S-points ranks points, not IDs (ANA-37 §11).
- **McNemar**: S against each of the other 11 backends, per group, on recall@5. `only_S` counts queries where S hits and the other misses, `only_other` the reverse, then `mcnemar_exact`.
- **Best lexical baseline** (PRD threshold): among `Backend::is_lexical()`, the highest hit5 on group A; ties go to the higher MRR@10, then to `Backend::ALL` order.
- **Separability (§5.4)**: positives are class-a queries, negatives class-d queries, each scored by its top-1 (no hit scores 0.0). The rows:
  - `S hybrid RRF`: S `ranked[0].score`.
  - `S-dense cosine`: S-dense.
  - `S-sparse per query term`: S-sparse top-1 / `sparse_terms` (0 terms gives 0.0).
  - `L-or-len`: its top-1.
  - `G / G-dens fraction matched`: G's top-1 score (the same as G-dens's).
  - Columns: AUC and recall at zero false alarms (`metrics::auc`, `recall_at_zero_false_alarms`).
- **Rescues**, over a+b+c:
  - "S finds it in the top 5 and every lexical baseline misses": S hit5, and every `is_lexical()` backend misses top 5.
  - "A lexical baseline finds it in the top 5 and S misses".
  - "Every backend misses the top 5".
  - Ranks shown as S / S-dense / S-sparse / L-or-len / G-dens / H, with `–` outside the top 10.
- **Latency**: p50 and p95 per backend over all 72 queries (`metrics::percentile`), errors excluded.
- **Thresholds** (PRD):
  - `S recall@5 on duplicates: h/24 ≥ 20/24 — met|not met`.
  - `S vs best lexical baseline (<name>) on duplicates: p = … < 0.05 — met|not met`.
  - `Reproducibility: compare with a second run (htui-bench compare)`.

`report.md` layout, in order:

1. `# htui-bench: <set> on <corpus_ref>`
2. `## Run`: a key/value table of every `RunMeta` field (sha256 in full), plus points/s.
3. `## Thresholds`.
4. `## Results (recall@1 / recall@5 / MRR@10)`: rows are backends, columns `a duplicates (24) | b questions (20) | c identifiers (16) | a+b+c (60)`, cells `0.42 / 0.92 / 0.59` (two decimals), as in ANA-37 §5.2.
5. `## Recall@5 with Wilson 95% intervals`: cells `22/24 (0.74-0.98)`.
6. `## Paired exact McNemar on recall@5, S against each backend`: rows are the other backends, one column triple `S only / other only / p` per group. p is printed `{:.4}` when ≥ 0.0001, else `< 0.0001`.
7. `## Separability of the top-1 score (duplicates against no-target)`: AUC and recall at zero false alarms, two decimals, `—` for `None`.
8. The three rescue sections: tables of query id, text (cut to 80 chars with `…`), targets and ranks.
9. `## Latency (ms, p50 / p95)`.
10. `## Errors`: query, backend, message, or `none`.
11. `## Threats to validity`, a fixed text: the author knew the targets (ANA-37 §5.9); the queries are re-authored, not ANA-37's lost set; hybrid ties reorder between runs; the corpus is markdown at a pin, not Postgres rows; this corpus has 777 document points against ANA-37's 759 (F3).

**`compare`**:
- `same_queries`: equal `query_sha256`. When it differs, print a warning first and compare by query id over the ids both runs hold.
- Per backend and group: hit5 in run A, hit5 in run B, and the delta.
- Per backend: the query ids whose top-10 `ranked` id list differs, and their count.
- `s_within_one_per_class`: |Δ hit5| ≤ 1 for S in each of groups A, B and C (the PRD reproducibility metric).
- Rendered as markdown on stdout.

### 4.3 `src/main.rs` (clap derive)

```rust
#[derive(Debug, clap::Parser)]
#[command(name = "htui-bench", about = "MOD-162 retrieval benchmark (dev tool)")]
struct Cli { #[command(subcommand)] command: Command }

#[derive(Debug, clap::Subcommand)]
enum Command { Run(RunArgs), Validate(SetArgs), Freeze(FreezeArgs), Compare(CompareArgs) }

#[derive(Debug, clap::Args)]
struct SetArgs {
    /// Commit to build the corpus from, or WORKTREE for the files on disk.
    #[arg(long, default_value = htui_bench::corpus::DEFAULT_PIN)] corpus_ref: String,
    /// The repository (default: this workspace).
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))] repo: PathBuf,
    /// The frozen query file; its sidecar is `<file>.sha256`.
    #[arg(long, default_value = htui_bench::queries::DEFAULT_FILE)] queries: PathBuf,
}
#[derive(Debug, clap::Args)]
struct RunArgs {
    #[command(flatten)] set: SetArgs,
    /// Output directory for results.json and report.md (refused if results.json exists, unless --force).
    #[arg(long)] out: PathBuf,
    #[arg(long)] force: bool,
    /// Qdrant gRPC URL; falls back to HTUI_TEST_QDRANT_URL.
    #[arg(long, env = "HTUI_BENCH_QDRANT_URL", hide_env_values = true)] qdrant_url: Option<String>,
    /// Postgres DSN for the L baselines; falls back to HTUI_TEST_DATABASE_URL.
    #[arg(long, env = "HTUI_BENCH_DATABASE_URL", hide_env_values = true)] database_url: Option<String>,
    /// Default: htui_bench_<corpus ref>.
    #[arg(long)] collection: Option<String>,
    /// Postgres schema for the L baselines (dropped at the end).
    #[arg(long, default_value = htui_bench::pg_fts::SCHEMA)] schema: String,
}
#[derive(Debug, clap::Args)]
struct FreezeArgs {
    #[arg(long, default_value = htui_bench::queries::DEFAULT_FILE)] queries: PathBuf,
    /// Overwrite a sidecar holding another hash.
    #[arg(long)] force: bool,
}
#[derive(Debug, clap::Args)]
struct CompareArgs { a: PathBuf, b: PathBuf }
```

- The API key comes only from the environment, as `HTUI_BENCH_QDRANT_API_KEY` (never a flag, never printed).
- URL fallbacks are resolved in code: the flag or `HTUI_BENCH_*`, then `HTUI_TEST_*`, else an error naming both variables. `QdrantUrl::parse` checks the Qdrant URL.
- `#[tokio::main] async fn main() -> anyhow::Result<()>`.
- **run**: on success, stdout gets the two written paths and `S recall@5 on duplicates: h/24`.
- **validate**: `load_frozen`, `corpus::build`, `validate`. Prints `72 queries (sha256 …) valid against <ref> (322 documents)`, or each unknown target and exit 1. No network.
- **freeze**: writes `sidecar_line(sha, file_name)`. When an existing sidecar matches, it does nothing and says so; when it differs, it refuses without `--force`.
- **compare**: `load_results` on both, then `render_comparison` to stdout.

**T6 tests (first):**
- report: a hand-built `Results` fixture (6 queries: 2 a, 1 b, 1 c, 2 d; backends S, S-dense, L-or-len, G, H), on which `summarize` gives the expected hit counts, MRR, McNemar counts, best lexical, AUC, and the rescue lists. `render_markdown` contains the threshold lines, a §5.2-style row and the `–` rank marker.
- compare: two fixtures differing in one S rank give the right deltas, changed ids and `s_within_one_per_class`. `results.json` round-trips through serde.
- run: `collection_name`; `RunConfig` `Debug` hides the DSN.
- main: `Cli::try_parse_from` for each subcommand; `run` without `--out` is an error.
- `run_with_wires_a_tiny_corpus`, live, needs both `HTUI_TEST_QDRANT_URL` and `HTUI_TEST_DATABASE_URL` (else a skip line). It uses `HashEmbedder::new(DENSE_DIM)`, `build_from_sources` on a 3-item, 2-requirement, 1-decision fixture, a 4-query `QuerySet`, a throwaway collection and schema, and a temp `out` under `std::env::temp_dir()`. It asserts 12 backend rows per query, an exact-key query found by S-sparse, and that the schema is gone afterwards. It is slow on the sandbox Qdrant (creates take 3.5-19 s).

## 5. T7 - docs and the reference run (W4)

`docs/htui-bench.md`: what the benchmark measures; prerequisites (a scratch Qdrant: `docker compose up -d qdrant`, gRPC 6334; a Postgres DSN; 133 MB model fetched on first run); env vars (`HTUI_BENCH_QDRANT_URL`, `HTUI_BENCH_QDRANT_API_KEY`, `HTUI_BENCH_DATABASE_URL`, falling back to `HTUI_TEST_*`); the one command `cargo run -p htui-bench --release -- run --out target/bench/<name>`; `validate`, `freeze`, `compare`; one line per backend; threats to validity (ANA-37 §5.9, plus re-authored queries, plus F3). Always write output under `target/` (the forbidden-literal scan reads `.json` under `crates/`). `docs/benchmarks/mod-162-m1-<date>.md`: two runs, `compare` output, each PRD threshold stated met or not, or "owed on host" with the exact command.

## 6. File ownership (disjoint per wave)

| Task | Wave | Owns (creates or edits) | Must not touch |
|---|---|---|---|
| T1 | W1 | `crates/htui-store/src/vector.rs`, `crates/htui-store/tests/qdrant_live.rs` | anything in `crates/htui-bench`, `Cargo.lock` |
| T2 | W1 | root `Cargo.toml` (members only), `Cargo.lock`, `crates/htui-bench/Cargo.toml`, `src/lib.rs`, `src/types.rs`, `src/metrics.rs`, `src/main.rs` (stub), stubs `src/{corpus,lexical,pg_fts,queries,run,report}.rs` | `crates/htui-store/**` |
| T3 | W2 | `crates/htui-bench/src/corpus.rs` | every other file |
| T4 | W2 | `crates/htui-bench/src/lexical.rs`, `crates/htui-bench/src/pg_fts.rs` | every other file |
| T5 | W2 | `crates/htui-bench/src/queries.rs`, `crates/htui-bench/queries/ana37-r2.json`, `…/ana37-r2.json.sha256` | every other file |
| T6 | W3 | `crates/htui-bench/src/{run.rs, report.rs, main.rs}` | `lib.rs`, `types.rs`, `Cargo.toml` (T2 already lists every dependency T6 needs) |
| T7 | W4 | `docs/htui-bench.md`, `docs/benchmarks/mod-162-m1-<date>.md` | code |

W2 tasks run in separate worktrees (or serially). A half-written module in a shared tree breaks the other tasks' builds, because all three compile the same crate. A W2 task that believes a fixed signature is wrong stops and reports; it does not edit `types.rs`.

## 7. Validation commands

```bash
# every task, before each commit
cargo fmt --check
grep -rni 'z''eta' crates/htui-bench crates/htui-store/src/vector.rs crates/htui-store/tests/qdrant_live.rs   # prints nothing

# T1
cargo test -p htui-store --all-features --lib vector:: -- --test-threads=1
cargo test -p htui-store --all-features --test qdrant_live -- --test-threads=1   # live with HTUI_TEST_QDRANT_URL
cargo clippy -p htui-store --all-features -- -D warnings
cargo clippy -p htui-store -- -D warnings

# T2
cargo test -p htui-bench -- --test-threads=1
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-features -- -D warnings
cargo run -p htui-bench; test $? -eq 1        # the stub links and refuses

# T3
cargo test -p htui-bench --lib corpus -- --test-threads=1
cargo clippy -p htui-bench -- -D warnings && cargo clippy -p htui-bench --all-features -- -D warnings

# T4
cargo test -p htui-bench --lib -- lexical pg_fts --test-threads=1   # pg live with HTUI_TEST_DATABASE_URL
cargo clippy -p htui-bench -- -D warnings && cargo clippy -p htui-bench --all-features -- -D warnings

# T5
(cd crates/htui-bench/queries && sha256sum -c ana37-r2.json.sha256)
cargo test -p htui-bench --lib queries -- --test-threads=1
cargo clippy -p htui-bench -- -D warnings

# T6
cargo test -p htui-bench --all-features -- --test-threads=1
cargo run -p htui-bench -- validate
cargo clippy --workspace -- -D warnings && cargo clippy --workspace --all-features -- -D warnings

# T7 (manual; needs model + Qdrant + Postgres)
cargo run -p htui-bench --release -- run --out target/bench/m1-run1
cargo run -p htui-bench --release -- run --out target/bench/m1-run2
cargo run -p htui-bench -- compare target/bench/m1-run1 target/bench/m1-run2
bash scripts/gate.sh
```

The gate's clippy runs without `--all-targets`, so test code is not linted there. Keep tests warning-free anyway.

## 8. Build sequence

1. **W1**: T1 ∥ T2. T2 publishes every signature above as compiling stubs, so W2 never waits on T1.
2. **W2**: T3 ∥ T4 ∥ T5. T5's freeze commit lands before anything runs a backend; T6's `run` is the first thing that can.
3. **W3**: T6, after T1 is merged (it calls `search_arms`) and T3-T5 are merged.
4. **W4**: T7. Reference runs, docs, gate.
