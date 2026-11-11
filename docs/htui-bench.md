# htui-bench: the retrieval benchmark (MOD-162)

`htui-bench` is a dev tool, never shipped. It rebuilds the retrieval benchmark of `docs/ANA-37.md` §5.1-5.5
inside the repo, so its numbers can be reproduced and any change to knowledge search (chunking, fusion, the
embedder, new point types) can be measured before it ships. It ranks htui's **production** search code
against lexical baselines on a frozen, hash-checked query set and writes a report.

Origin: `docs/ANA-37.md` P5; PRD `.claude/prds/mod-162-retrieval-benchmark.prd.md`; plan
`.claude/plans/mod-162-m1-retrieval-benchmark.plan.md`; blueprint `.claude/plans/mod-162-m1-blueprint.md`.

## What it measures

**Corpus.** Built from a pinned commit through `git show` (default `d15e9fea`, ANA-37's exact corpus), or from
the files on disk with `--corpus-ref WORKTREE`: the open items of `HANDOFF.md` (71 at the pin), the requirement
bullets of `docs/REQUIREMENTS.md` (121) and the decision files under `docs/decisions/` (130, each a closed item
plus a `summary` document). Rows are real `htui-core` values, and points come from `htui-store`'s own builders
(`item_point`, `requirement_point`, `document_points`). At the pin that is 1,099 points: 201 item,
121 requirement, 777 document (ANA-37 counted 759 document points; see "Threats").

**Queries.** `crates/htui-bench/queries/ana37-r2.json`, frozen with its sha256 in `ana37-r2.json.sha256`
before any backend ran (commit `e533bb66`). 72 queries: 24 reworded duplicates of an open item (class `a`), 20
natural-language questions whose answer uses other words (`b`), 16 identifier lookups (`c`), 12 with no
correct target (`d`). ANA-37's original 72 were lost with the sandbox's `/tmp`; this set is re-authored in the
same classes and counts. The query texts ANA-37 quotes in full are kept verbatim; a01, a04 and a20, quoted
truncated, were completed and carry `"completed": true`. `run` and `validate` refuse a file whose bytes do not
match the sidecar.

**Backends.**

| Name | What it is |
|---|---|
| **S** | htui's hybrid search (`QdrantStore::search`), limit 40, collapsed to one row per corpus ID |
| S10 | the same at htui's default limit of 10 |
| S-points | hybrid at limit 10, not collapsed (what htui actually returns) |
| S-dense, S-sparse | the two arms of the hybrid, alone, through `QdrantStore::search_arms` |
| L-and, L-or, L-and-len, L-or-len | Postgres `websearch_to_tsquery('english')` + `ts_rank_cd`, AND or OR, with or without length normalisation |
| G, G-dens | matched non-stopword keyword count, ties by occurrences or by density |
| H | htui's own search without Qdrant: the whole query as a case-insensitive substring of an item's key or title, or a requirement's key or body (`crates/htui-store/src/pg/read.rs:145-147, 1016-1018`) |

L, G and H are baselines ("what word matching finds"), not candidate designs.

**Report.** Per class: recall@1, recall@5, MRR@10, with Wilson 95% intervals; exact two-sided McNemar of S
against every other backend, per class and over a+b+c; ANA-37 §5.4's separability (AUC and recall at zero
false alarms, duplicates against no-target queries); the queries S finds and every lexical baseline misses,
and the reverse; latency percentiles; run metadata (pin, query hash, embedder identity, Qdrant version, host
load). The PRD's thresholds are checked and printed: S recall@5 on duplicates ≥ 20/24, and S ahead of the best
lexical baseline with p < 0.05.

**Failed calls are never read as results.** A backend call that errors (a Qdrant timeout, a dropped Postgres)
still counts as a miss in the cells, but the cell carries a `(k err)` marker, the Errors table lists it (an
S-points failure that only repeats S10's is marked as such and counted once), and stderr prints the number of
distinct failed calls. A lexical backend with any failed call on a duplicate query is never chosen as the best
lexical baseline. If S, or every lexical baseline, failed on a duplicate query, both threshold lines read
`not measured (N failed calls)` instead of met or not met. A p-value is printed to three significant digits,
with more digits when rounding would carry it across 0.05. `htui_dirty` in the run metadata is `null`
("unknown") when git could not tell.

## Running it

Needs a scratch Qdrant (1.19.x, as `compose.yaml` pins), a Postgres, and the 133 MB embedding model, which
the first run downloads into the user cache (`htui-store::model`).

```bash
# no network: hash, shape and every target against the corpus
cargo run -p htui-bench -- validate

# the benchmark (release: embedding 1,099 points takes minutes)
export HTUI_BENCH_QDRANT_URL=http://localhost:6334      # falls back to HTUI_TEST_QDRANT_URL
export HTUI_BENCH_DATABASE_URL=postgres://...           # falls back to HTUI_TEST_DATABASE_URL
# export HTUI_BENCH_QDRANT_API_KEY=...                  # only if the Qdrant needs one
cargo run -p htui-bench --release -- run --out target/bench/run1
cargo run -p htui-bench --release -- run --out target/bench/run2

# run-to-run differences: per-class recall@5 deltas and the queries whose top 10 changed
cargo run -p htui-bench -- compare target/bench/run1 target/bench/run2
```

`run` writes `results.json` and `report.md` to `--out` and refuses an existing `results.json` unless
`--force`. Each run deletes and recreates its Qdrant collection (default `htui_bench_<corpus ref>`) and its
Postgres schema (default `htui_bench`, dropped at the end); both names must start with `htui_bench`, so a run
can never drop `public` or the production `htui_concepts_v2`. The DSN and the API key are never printed.

To change the query set: edit the JSON, run `htui-bench freeze` to rewrite the sidecar, and commit both in a
commit of their own **before** running any backend on them; git order is the evidence that the set was frozen
first. A new set gets a new file name rather than editing `ana37-r2.json`.

## Tests

`cargo test -p htui-bench --all-features -- --test-threads=1` covers the parsers, the pinned corpus counts,
the statistics (checked against ANA-37's published p-values and intervals), the query freeze, the L SQL, and
the report pipeline on a fixture. Two kinds are live and skip with a printed line when their service is not
configured: the Postgres full-text test (`HTUI_TEST_DATABASE_URL`) and `run_with_wires_a_tiny_corpus`
(`HTUI_TEST_QDRANT_URL` and `HTUI_TEST_DATABASE_URL`, with the test embedder, no model download). The benchmark
run itself is manual and never part of the gate.

## Threats to validity

Carried over from ANA-37 §5.9, plus what this rebuild adds:

- **Re-authored queries.** The set is not ANA-37's; results are compared with ANA-37 against the thresholds,
  not cell by cell.
- **The query author knew the targets.** Class `a` avoids title words, which biases against lexical backends;
  class `c` uses distinctive terms, which favours them.
- **Small samples.** 24/20/16/12 queries give wide intervals.
- **Small, skewed corpus,** and not htui's real data shape: items and decisions are modelled from markdown
  (MOD-8 is open); notes, transcripts and `docs/ANA-*.md` are not in it.
- **Chunk count.** `chunk_markdown` gives 777 document points at the pin, not ANA-37's 759; the cause of the
  18-point gap is not known (likely a harness difference in ANA-37's point construction).
- **Hybrid results are not deterministic.** Qdrant orders tied RRF scores arbitrarily, so the S rows can
  differ between runs; `compare` shows by how much.
- **L and G are untuned and carry no IDF;** H is htui's real no-Qdrant behaviour and is deliberately weak.
- **L-and is "AND as parsed".** `websearch_to_tsquery` reads a bare `or` as the OR operator, and `&` binds
  tighter than `|`, so a query like a05 ("approve or refuse …") runs as `'approv' | ('refus' & …)` under L-and:
  far looser than AND. The L variants report what Postgres full-text search does with the text as written.
- **The live `search == search_arms(Hybrid)` test is no evidence on its own.** `search` delegates to
  `search_arms(Hybrid)`, so the live test only shows Qdrant answers the same request the same way up to tie
  order. What guards production behaviour is the unit test that compares the built request byte for byte with
  the pre-MOD-162 builder (`hybrid_request_is_the_request_search_sent_before_mod_162`).
- **Latency** depends on the host's load, which the report records.
