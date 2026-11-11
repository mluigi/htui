# Plan: MOD-162 M1 - Retrieval benchmark harness and reproduction

**Source PRD**: `.claude/prds/mod-162-retrieval-benchmark.prd.md`
**Selected Milestone**: 1 - Harness and reproduction
**Complexity**: Large (new crate, ~12 files, one seam in `htui-store`, a re-authored query set)
**Route**: PRD path, ultracode for the implement phase (maintainer, 2026-11-10)

## Summary

A new workspace crate `htui-bench` (binary `htui-bench`, never shipped) rebuilds ANA-37's retrieval
benchmark: it builds the `d15e9fea` markdown corpus through htui's own point builders, indexes it in a
scratch Qdrant collection with the production embedder, runs every §5.1 backend plus htui's own
no-Qdrant baseline over a re-authored, hash-frozen query set, and writes a markdown and JSON report with
recall@1/5, MRR@10, Wilson 95% intervals, exact McNemar, and §5.4's AUC and recall at zero false
alarms. One small seam in `htui-store` lets the dense and sparse arms run alone through the production
search path instead of a copy.

## Design decisions (plan-level)

- **P1 One search path.** `QdrantStore` gains `pub enum SearchArms { Hybrid, Dense, Sparse }` and
  `pub async fn search_arms(&self, query: &SearchQuery, arms: SearchArms)`. `VectorStore::search` for
  `QdrantStore` becomes `self.search_arms(query, SearchArms::Hybrid)`; the hybrid request is built
  exactly as today (same prefetch depth `max(4×limit, 20)`, same RRF, same filter). `Dense` and `Sparse`
  issue the corresponding single prefetch's query directly (nearest on `dense` / sparse query on
  `sparse`, same filter, `limit` = `query.limit`). No other production behaviour changes (PRD
  out-of-scope: no search tuning).
- **P2 Corpus from git, not the work tree.** The corpus is read with `git show <ref>:<path>` and
  `git ls-tree` (default ref `d15e9fea`, overridable with `--corpus-ref`, `WORKTREE` = files on disk).
  Three sources, matching ANA-37 §5.1: open items `- [ ] **KEY - Title** …` of `HANDOFF.md` (71 at the
  pin), requirement bullets `- **R-AREA-N (prio).** …` of `docs/REQUIREMENTS.md` (121), decision files
  `docs/decisions/**/*.md` (130), each a closed item (resolution from its `# KEY - Title (word, …)`
  header; unknown words map to `done`) plus a `summary` document holding the whole file.
- **P3 Points through production code.** Rows are real `htui_core::model::{Item, Requirement,
  Document}` values with deterministic UUIDs (sha256 of a namespace plus the corpus ID) in one synthetic
  project; points come from `vector_sync::{item_point, requirement_point, document_points}` unchanged.
  A hit's corpus ID is `DEC:<key>` when `hit.resolution.is_some()`, else `hit.key` (the ANA-37 ID
  convention, `docs/ANA-37.md` §5.3).
- **P4 Backends.** `S` (hybrid, limit 40, collapsed to one row per corpus ID, first occurrence wins),
  `S10` (limit 10, collapsed), `S-points` (hybrid limit 10, uncollapsed; §11's check), `S-dense`,
  `S-sparse` (P1 arms, limit 40, collapsed), `L-and`, `L-or`, `L-and-len`, `L-or-len` (Postgres
  `websearch_to_tsquery('english')`, `ts_rank_cd(…, 32)`, `-len` = normalisation `2|32`; `L-or`
  rewrites the tsquery's `&` to `|`), `G`, `G-dens` (in-process matched non-stopword keyword count,
  ties by occurrences or by density), and **`H`**: htui's own no-Qdrant filter, mirrored in process -
  the whole query as one case-insensitive literal substring of an item's key or title, or of a
  requirement's key or body (`crates/htui-store/src/pg/read.rs:145-147, 1016-1018`), items before
  requirements in key order (htui ranks nothing). L, G and H are baselines, never candidate designs.
- **P5 Postgres for L only, through runtime queries.** `sqlx::query` (not the macros, so no `.sqlx`
  offline entries); the harness creates its own schema `htui_bench` in the database named by
  `--database-url` / `HTUI_BENCH_DATABASE_URL` (falls back to `HTUI_TEST_DATABASE_URL`), loads one row
  per corpus document, and drops the schema at the end. No htui migration is involved.
- **P6 Query freeze.** `crates/htui-bench/queries/ana37-r2.json` holds the 72 queries
  (`{id, class, text, targets[]}`; classes `a` 24, `b` 20, `c` 16, `d` 12, `d` with no targets);
  `ana37-r2.json.sha256` holds its sha256. The runner refuses a query file whose bytes do not hash to
  the sidecar, and `htui-bench validate` refuses targets that are not corpus IDs at the pin. The query
  texts ANA-37 quotes in full (§5.3: a03, a08, a09, a11, a15, a18, b01, b03, b04, b05, b07, b09, b11, b12,
  b14, b15, b20, c16) are kept verbatim; a01, a04, a20 are quoted truncated in §5.3 and are completed by
  the author, flagged `"completed": true`. The query file and sidecar are committed in a commit of their
  own **before** the first run of any backend (git order is the evidence).
- **P7 Statistics, no new crate.** Recall@k, MRR@10, Wilson 95% (z = 1.96), exact two-sided McNemar
  (binomial over discordant pairs, p capped at 1), AUC as Mann-Whitney with ties as one half, recall at
  zero false alarms (fraction of class `a` top-1 scores strictly above every class `d` top-1 score). All
  pure functions with test vectors taken from ANA-37 §5.2.
- **P8 Run is manual; the code is gated.** The crate is a workspace member, so `cargo fmt`, both
  clippy runs and `cargo test --workspace` cover it. Unit tests need no network. The live run needs the
  133 MB model (`model::ensure_model_noting`), a Qdrant and a Postgres, and is never part of the gate.
- **P9 Reproducibility.** `htui-bench compare <a.json> <b.json>` prints per-class recall@5 deltas and
  the queries whose top-10 changed; `run` drops and recreates its collection (`htui_bench_<pin>`) so
  every run starts clean.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui-store/src/vector_sync.rs:192-260` | `item_point` / `document_points`: free functions named after what they build |
| Errors | `crates/htui/src/concepts.rs:134-142` | `anyhow::Context` at the binary edge; `StoreError` inside library calls |
| Embedder load | `crates/htui/src/concepts.rs:134-142` | `ensure_model_noting` printing to stderr, then `RtenEmbedder::load` on `spawn_blocking` |
| Qdrant scratch collection | `crates/htui-store/src/vector.rs:757-765, 807-813` | `connect_to(settings, embedder, name)`, `drop_collection()` |
| Settings | `crates/htui-store/src/qdrant_settings.rs:27` | `QdrantSettings::new(url, api_key)`; never print the key |
| Live tests | `crates/htui-store/tests/qdrant_live.rs` | skip with a printed line when `HTUI_TEST_QDRANT_URL` is unset; throwaway collection per test |
| Unit tests | `crates/htui-store/src/vector_sync.rs:325-341` | `#[cfg(test)] mod tests`, sentence-named `#[test]` fns |
| Manifest | `crates/htui-store/Cargo.toml` | `*.workspace = true` package keys, `[lints] workspace = true`, comment per dependency |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-store/src/vector.rs` | UPDATE | T1 | `SearchArms`, `search_arms`; `search` delegates (P1) |
| `crates/htui-store/tests/qdrant_live.rs` | UPDATE | T1 | live cases: each arm finds what it should; hybrid unchanged |
| `Cargo.toml` | UPDATE | T2 | add `crates/htui-bench` to `members` |
| `Cargo.lock` | UPDATE | T2 | new package entry (no new external crate expected) |
| `crates/htui-bench/Cargo.toml` | CREATE | T2 | the dev-tool crate; `htui-store` with `local-embed` |
| `crates/htui-bench/src/main.rs` | CREATE | T2 (stub), T6 | CLI: `run`, `validate`, `freeze`, `compare` |
| `crates/htui-bench/src/lib.rs` | CREATE | T2 | declares every module (stubs) so T3-T5 never touch it |
| `crates/htui-bench/src/metrics.rs` | CREATE | T2 | P7 statistics |
| `crates/htui-bench/src/corpus.rs` | CREATE (stub T2) | T3 | P2/P3: git source, parsers, rows, points, ID mapping |
| `crates/htui-bench/src/lexical.rs` | CREATE (stub T2) | T4 | G, G-dens, H (P4) |
| `crates/htui-bench/src/pg_fts.rs` | CREATE (stub T2) | T4 | L variants (P4, P5) |
| `crates/htui-bench/src/queries.rs` | CREATE (stub T2) | T5 | query schema, sha256 check, target validation hook |
| `crates/htui-bench/queries/ana37-r2.json` (+ `.sha256`) | CREATE | T5 | the frozen set (P6) |
| `crates/htui-bench/src/run.rs` | CREATE (stub T2) | T6 | index, run backends, collect ranks, timings |
| `crates/htui-bench/src/report.rs` | CREATE (stub T2) | T6 | markdown + JSON report, `compare` |
| `docs/htui-bench.md` | CREATE | T7 | how to run, what each backend is, threats to validity |
| `docs/benchmarks/mod-162-m1-<date>.md` | CREATE | T7 | the reference run's report (or "owed on host") |

## Tasks

Waves: **W1** T1 ∥ T2 → **W2** T3 ∥ T4 ∥ T5 → **W3** T6 → **W4** T7. Every task is TDD: tests first,
red, then green; each commits on its own.

### Task 1: `htui-store` search arms (W1)
- **Files**: `crates/htui-store/src/vector.rs`, `crates/htui-store/tests/qdrant_live.rs`.
- **Action**: P1. Extract the request building so `Hybrid` is byte-for-byte today's request; add
  `Dense`/`Sparse`. A `Sparse` search whose query has no sparse terms returns no hits (today's hybrid
  simply omits the arm). Unit test the request shape where the builders allow; live tests (skip without
  `HTUI_TEST_QDRANT_URL`, `HashEmbedder`): an exact key is found by `Sparse`, `Hybrid` equals
  `VectorStore::search` on the same query.
- **Mirror**: `qdrant_live.rs` throwaway-collection cases.
- **Validate**: `cargo test -p htui-store --all-features vector`; `cargo clippy -p htui-store
  --all-features -- -D warnings`.

### Task 2: crate skeleton and statistics (W1)
- **Files**: `Cargo.toml`, `Cargo.lock`, `crates/htui-bench/{Cargo.toml, src/main.rs, src/lib.rs,
  src/metrics.rs}` and empty stubs `src/{corpus,lexical,pg_fts,queries,run,report}.rs`.
- **Action**: the crate compiles in the workspace with and without `--all-features`; `metrics.rs`
  implements P7. Test vectors (ANA-37 §5.2): McNemar 13/0 → 0.000244, 15/0 → 0.000061, 6/0 → 0.03125,
  5/0 → 0.0625, 8/5 → 0.581, 22/5 → 0.00151; Wilson 22/24 → (0.74, 0.98) to two places, 9/24 →
  (0.21, 0.57), 47/60 → (0.66, 0.87); AUC and zero-false-alarm recall on hand-built score lists.
- **Validate**: `cargo test -p htui-bench`; `cargo clippy --workspace -- -D warnings` and
  `--all-features`.

### Task 3: corpus builder (W2)
- **Files**: `crates/htui-bench/src/corpus.rs` only.
- **Action**: P2, P3. Parsers are pure functions over `&str` and unit-tested on inline fixtures
  (multi-line item bodies ending at the next item or heading; requirement bullets with continuation
  lines; decision headers in every shape seen at the pin: `(done, date)`, `(concluded, …)`,
  `(shipped, …)`, `(decided, …)`, `(rejected, …)`, `(done; verdict withdrawn …)`, `# KEY: Title` and no
  parenthesis). One test reads the real pin through git and asserts 71 / 121 / 130 documents and that
  the point builders yield 201 item, 121 requirement and ≥ 700 document points (ANA-37: 759); it
  prints a skip line and passes when the commit is unreachable (shallow clone).
- **Mirror**: `vector_sync.rs` tests.
- **Validate**: `cargo test -p htui-bench corpus`.

### Task 4: lexical baselines (W2)
- **Files**: `crates/htui-bench/src/lexical.rs`, `crates/htui-bench/src/pg_fts.rs` only.
- **Action**: P4, P5. `lexical.rs`: G, G-dens and H as pure functions over a slice of corpus documents,
  unit-tested. `pg_fts.rs`: schema create/load/drop and the four L queries; a live test against
  `HTUI_TEST_DATABASE_URL` (skip line when unset) on a five-document corpus checks that `L-and` needs
  every term, `L-or` any, and that `-len` changes ranking for a long document.
- **Validate**: `cargo test -p htui-bench --all-features lexical pg_fts`.

### Task 5: the frozen query set (W2)
- **Files**: `crates/htui-bench/src/queries.rs`, `crates/htui-bench/queries/ana37-r2.json`,
  `crates/htui-bench/queries/ana37-r2.json.sha256` only.
- **Action**: P6. `queries.rs`: schema, load, sha256 check, class counts check, a `validate(targets,
  corpus_ids)` that names every unknown target. Author the 72 queries against the corpus at
  `d15e9fea` (read through `git show`): class `a` paraphrases of an existing open item avoiding its
  title words; `b` natural-language questions whose answer uses other words (targets may be items,
  requirements or `DEC:` decisions); `c` identifier lookups (keys and rare terms such as `ConPTY`,
  `Plackett-Luce`, `R-STO-8`); `d` plausible requests with no target, with near-miss distractors. The
  author records, per query, nothing beyond `{id, class, text, targets, completed?}`. Commit the JSON
  and sidecar **alone** before any backend runs (P6); a unit test checks that the committed file
  hashes to the sidecar and has 24/20/16/12.
- **Validate**: `cargo test -p htui-bench queries`.

### Task 6: runner, report and CLI (W3)
- **Files**: `crates/htui-bench/src/{run.rs, report.rs, main.rs}`.
- **Action**: `run` = check the query hash → build the corpus at the pin → validate targets →
  load the model → recreate `htui_bench_<pin>` → upsert in batches of 64 with progress on stderr →
  load the L schema → run every backend per query, recording the ranked corpus IDs, scores and
  latency → write `<out>/results.json` and `<out>/report.md` (per-class table as ANA-37 §5.2, Wilson
  intervals, the McNemar table for S against every other backend per class and over a+b+c, §5.4's
  separability table, "S finds and every lexical misses" and the reverse, run metadata: pin, query
  sha256, embedder identity, Qdrant version, host load, timings). `compare` (P9). The pipeline after
  "results collected" is pure and unit-tested on a fixed results fixture; the live path is exercised by
  T7.
- **Validate**: `cargo test -p htui-bench`; `cargo run -p htui-bench -- validate`.

### Task 7: documentation and the reference run (W4)
- **Files**: `docs/htui-bench.md`, `docs/benchmarks/mod-162-m1-<date>.md`.
- **Action**: usage doc (scratch Qdrant and Postgres, env vars, the one command, `compare`, threats to
  validity carried over from ANA-37 §5.9 plus "re-authored queries"). Run the benchmark twice here; if
  the sandbox Qdrant times out (known: creates 3.5-19 s), record the run as owed on the host with the
  exact command. Judge against the PRD thresholds: S recall@5 on `a` ≥ 20/24, McNemar p < 0.05 against
  the best of L/G/H, second run within one query per class.
- **Validate**: the report exists and states each threshold met or not; `bash scripts/gate.sh`.

## Validation

```bash
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-features -- -D warnings
cargo test -p htui-store --all-features -- --test-threads=1
cargo test -p htui-bench --all-features -- --test-threads=1
cargo run -p htui-bench --release -- run --out target/bench/m1-run1   # manual, needs model+Qdrant+Postgres
bash scripts/gate.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Refactoring `search` changes hybrid behaviour | Low | Hybrid request built by the same code; live test `search == search_arms(Hybrid)`; reviewer checks the diff |
| Re-authored queries easier or harder than the lost set | Medium | P6 verbatim texts; thresholds, not cells; threat stated in the report |
| Sandbox Qdrant too slow for the reference run | High | Run owed on host with the exact command; unit tests carry the gate |
| Model download (133 MB) unavailable in the sandbox | Medium | Same: owed on host |
| `rten` debug build too slow for 1,081 points | Low | Root manifest already builds `rten*` at opt-level 3 in dev; recommend `--release` anyway |
| Decision-file headers vary | Certain | Parser tested on every shape seen at the pin; unknown words map to `done` |
| Literal `zeta` in a query or fixture fails htui-agent's extensibility test | Low | Implementer prompts forbid it |
| L-or tsquery rewrite produces an invalid tsquery for odd input | Medium | Rewrite on `websearch_to_tsquery`'s output text, then `to_tsquery`; an error counts as no hits and is reported |

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `item_point`, `requirement_point`, `document_points`, `chunk_markdown` are public and take htui-core rows | verified | `crates/htui-store/src/vector_sync.rs:192, 215, 237, 266` |
| No public dense-only or sparse-only search exists; the client field is private | verified | `vector.rs:728-732` (fields private), `:1280-1328` (only hybrid) |
| Hybrid prefetch depth is `max(4×limit, 20)`, fusion RRF, sparse arm omitted when empty | verified | `vector.rs:1296, 1305, 1309-1317` |
| `connect_to` and `drop_collection` are public | verified | `vector.rs:757, 807` |
| The production embedder loads via `ensure_model_noting` + `RtenEmbedder::load` behind `local-embed` | verified | `crates/htui/src/concepts.rs:134-142`; `embed.rs:119-153`; `htui-store/Cargo.toml:22` |
| Corpus at `d15e9fea`: 71 open items, 121 requirement bullets, 130 decision files | verified | `git show d15e9fea:…` counts, 2026-11-10; matches ANA-37 §5.1 |
| `d15e9fea` is reachable in this clone | verified | `git cat-file -t d15e9fea` → commit |
| Decision headers vary: done 98, concluded 20, shipped 3, decided 3, rejected 1, 4 irregular | verified | first lines of all 130 files at the pin |
| htui's no-Qdrant filter is a literal case-insensitive substring of item key/title and requirement key/body | verified | `crates/htui-store/src/pg/read.rs:145-147, 1016-1018` (ANA-37's `:105-110, 948` have drifted) |
| A new workspace member is covered by fmt, both clippy runs and the test step | verified | `Cargo.toml:2-3`; `scripts/gate.sh:82-84, 137` |
| `sqlx::query` (runtime) needs no `.sqlx` offline entry; only the macros do | verified (docs) | sqlx offline mode applies to `query!`-family macros only |
| Sandbox Qdrant is 1.19.1, as compose pins | verified | `GET localhost:6333/` → 1.19.1 |
| ANA-37 McNemar figures match exact two-sided binomial | verified | 13/0 → 2·2⁻¹³ = 0.000244; 8/5 → 2·2380/8192 = 0.581; 22/5 → 2·101584/2²⁷ = 0.00151 |
| Task independence W2: T3 {corpus.rs}, T4 {lexical.rs, pg_fts.rs}, T5 {queries.rs, queries/*} | verified disjoint | file lists above; `lib.rs`, `Cargo.toml`, `Cargo.lock` owned by T2 only |
| Task independence W1: T1 {htui-store vector.rs, qdrant_live.rs}, T2 {root Cargo.toml, Cargo.lock, crates/htui-bench/**} | verified disjoint | T2 adds a path dependency on `htui-store` but no file of it; `Cargo.lock` is T2's alone (T1 adds no dependency) |
| T6 depends on T1's `SearchArms` | dependency, not a conflict | T6 runs in W3 after T1 merged |

**Amended by the blueprint's fact-check** (`.claude/plans/mod-162-m1-blueprint.md` §0, F1-F8; no decision
changed): requirement bullets come in six header shapes, not one (F1); decision headers have 5 irregular
shapes incl. `(status, …)`, not 4 (F2); the pin chunks to 777 document points, not ANA-37's 759 (F3,
reported as a threat); T1's and T4's test commands corrected (F4, F5); the query JSON is scanned by
htui-agent's forbidden-literal test (F6); sqlx 0.9 needs `AssertSqlSafe` for dynamic SQL (F7); `run`
deletes a stale collection through a raw client, since `connect_to` refuses another embedder's (F8). The
blueprint adds `crates/htui-bench/src/types.rs`, owned by T2; disjointness unchanged.

## Acceptance

- [ ] All tasks complete, each committed
- [ ] Gate green except known sandbox-only Qdrant/Postgres timeouts, named
- [ ] Hybrid search behaviour unchanged (`search == search_arms(Hybrid)`)
- [ ] Query file and sidecar committed before the first backend run
- [ ] Reference report written, or owed on the host with the exact command
- [ ] Patterns mirrored, not reinvented
