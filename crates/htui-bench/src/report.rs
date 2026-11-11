//! The report: per-backend metrics, paired tests and the run's provenance (MOD-162 T6).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, bail};

use crate::metrics;
use crate::run::{BackendRun, QueryRecord, RESULTS_SCHEMA, Results};
use crate::types::{Backend, CorpusId, QueryClass, RANK_DEPTH};

/// A class group of the tables: a, b, c, and a+b+c.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// Class a, duplicates.
    A,
    /// Class b, questions.
    B,
    /// Class c, identifiers.
    C,
    /// Classes a, b and c together.
    Abc,
}

impl Group {
    /// Report order.
    pub const ALL: [Self; 4] = [Self::A, Self::B, Self::C, Self::Abc];

    /// Whether a query of `class` counts in this group (class d never does).
    pub const fn contains(self, class: QueryClass) -> bool {
        matches!(
            (self, class),
            (Self::A, QueryClass::A)
                | (Self::B, QueryClass::B)
                | (Self::C, QueryClass::C)
                | (Self::Abc, QueryClass::A | QueryClass::B | QueryClass::C)
        )
    }

    /// `a duplicates`, `b questions`, `c identifiers`, `a+b+c`.
    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "a duplicates",
            Self::B => "b questions",
            Self::C => "c identifiers",
            Self::Abc => "a+b+c",
        }
    }
}

/// Per backend and group: n, hits@1, hits@5, MRR@10, Wilson(hits@5).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Cell {
    /// Queries in the group.
    pub n: usize,
    /// Queries with a target at rank 1.
    pub hit1: usize,
    /// Queries with a target in the top 5.
    pub hit5: usize,
    /// Mean reciprocal rank, cut at 10.
    pub mrr10: f64,
    /// Wilson 95% interval of `hit5 / n`.
    pub wilson5: (f64, f64),
}

/// S against one other backend on recall@5 in one group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct McNemarRow {
    /// The backend S is paired with.
    pub other: Backend,
    /// The group.
    pub group: Group,
    /// Queries S finds in the top 5 and `other` misses.
    pub only_s: usize,
    /// The reverse.
    pub only_other: usize,
    /// Exact two-sided McNemar p.
    pub p: f64,
}

/// One row of §5.4's separability table.
#[derive(Debug, Clone, PartialEq)]
pub struct SeparabilityRow {
    /// `S hybrid RRF`, `S-dense cosine`, …
    pub name: String,
    /// The backend whose top-1 score is used.
    pub backend: Backend,
    /// AUC of class-a top-1 scores over class-d ones.
    pub auc: Option<f64>,
    /// Class-a recall above every class-d score.
    pub recall_zero_false_alarms: Option<f64>,
}

/// A query in one of the rescue lists.
#[derive(Debug, Clone, PartialEq)]
pub struct Rescue {
    /// The query's id.
    pub query: String,
    /// Its text.
    pub text: String,
    /// Its targets.
    pub targets: Vec<CorpusId>,
    /// First-target ranks of [`RESCUE_BACKENDS`], `None` outside the top 10 (or not run).
    pub ranks: Vec<(Backend, Option<usize>)>,
}

/// The backends whose ranks the rescue tables show.
pub const RESCUE_BACKENDS: [Backend; 6] = [
    Backend::S,
    Backend::SDense,
    Backend::SSparse,
    Backend::LOrLen,
    Backend::GDens,
    Backend::H,
];

/// p50 and p95 latency of one backend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyRow {
    /// The backend.
    pub backend: Backend,
    /// Median, ms; `None` when every call failed.
    pub p50: Option<f64>,
    /// 95th percentile, ms.
    pub p95: Option<f64>,
}

/// A backend's error on one query.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorRow {
    /// The query's id.
    pub query: String,
    /// The backend.
    pub backend: Backend,
    /// The message.
    pub message: String,
}

/// The PRD thresholds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Thresholds {
    /// S hits@5 on duplicates.
    pub s_hit5_a: usize,
    /// Duplicates.
    pub n_a: usize,
    /// `s_hit5_a / n_a ≥ 20/24`.
    pub recall_met: bool,
    /// The best lexical baseline on duplicates.
    pub best_lexical: Option<Backend>,
    /// McNemar of S against it on duplicates.
    pub p_vs_best: Option<f64>,
    /// `p_vs_best < 0.05` with S ahead.
    pub mcnemar_met: bool,
}

/// Everything the report shows, computed from [`Results`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    /// Backends with at least one row, in [`Backend::ALL`] order.
    pub backends: Vec<Backend>,
    /// Per backend and group.
    pub cells: BTreeMap<(Backend, Group), Cell>,
    /// S against every other backend, per group.
    pub mcnemar: Vec<McNemarRow>,
    /// Highest hits@5 on duplicates among the lexical baselines.
    pub best_lexical: Option<Backend>,
    /// §5.4.
    pub separability: Vec<SeparabilityRow>,
    /// S finds it in the top 5 and every lexical baseline misses.
    pub s_only: Vec<Rescue>,
    /// A lexical baseline finds it in the top 5 and S misses.
    pub lexical_only: Vec<Rescue>,
    /// Every backend misses the top 5.
    pub all_miss: Vec<Rescue>,
    /// Per backend.
    pub latency: Vec<LatencyRow>,
    /// Every failed call.
    pub errors: Vec<ErrorRow>,
    /// The PRD thresholds.
    pub thresholds: Thresholds,
}

/// The metrics of one run (pure).
pub fn summarize(results: &Results) -> Summary {
    let view = View::new(results);
    let backends = view.backends.clone();

    let mut cells = BTreeMap::new();
    for &backend in &backends {
        for group in Group::ALL {
            cells.insert((backend, group), view.cell(backend, group));
        }
    }

    let mut mcnemar = Vec::new();
    if view.has(Backend::S) {
        for &other in backends.iter().filter(|&&b| b != Backend::S) {
            for group in Group::ALL {
                let (mut only_s, mut only_other) = (0, 0);
                for q in view.group(group) {
                    match (view.hit5(Backend::S, q), view.hit5(other, q)) {
                        (true, false) => only_s += 1,
                        (false, true) => only_other += 1,
                        _ => {}
                    }
                }
                mcnemar.push(McNemarRow {
                    other,
                    group,
                    only_s,
                    only_other,
                    p: metrics::mcnemar_exact(only_s, only_other),
                });
            }
        }
    }

    // Highest hits@5 on duplicates, then MRR@10, then `Backend::ALL` order (`backends` is in it).
    let mut best_lexical: Option<(Backend, Cell)> = None;
    for &backend in backends.iter().filter(|b| b.is_lexical()) {
        let cell = cells[&(backend, Group::A)];
        let better = best_lexical.is_none_or(|(_, best)| {
            cell.hit5 > best.hit5 || (cell.hit5 == best.hit5 && cell.mrr10 > best.mrr10)
        });
        if better {
            best_lexical = Some((backend, cell));
        }
    }
    let best_lexical = best_lexical.map(|(backend, _)| backend);

    let separability = SEPARABILITY
        .iter()
        .filter(|(_, backend)| view.has(*backend))
        .map(|&(name, backend)| {
            let scores = |class: QueryClass| -> Vec<f64> {
                view.queries
                    .iter()
                    .filter(|q| q.class == class)
                    .map(|q| view.top1(backend, q))
                    .collect()
            };
            let (positives, negatives) = (scores(QueryClass::A), scores(QueryClass::D));
            SeparabilityRow {
                name: name.to_owned(),
                backend,
                auc: metrics::auc(&positives, &negatives),
                recall_zero_false_alarms: metrics::recall_at_zero_false_alarms(
                    &positives, &negatives,
                ),
            }
        })
        .collect();

    let lexical: Vec<Backend> = backends
        .iter()
        .copied()
        .filter(|b| b.is_lexical())
        .collect();
    let (mut s_only, mut lexical_only, mut all_miss) = (Vec::new(), Vec::new(), Vec::new());
    for q in view.group(Group::Abc) {
        let s = view.hit5(Backend::S, q);
        let any_lexical = lexical.iter().any(|&b| view.hit5(b, q));
        if s && !any_lexical {
            s_only.push(view.rescue(q));
        }
        if any_lexical && !s {
            lexical_only.push(view.rescue(q));
        }
        if backends.iter().all(|&b| !view.hit5(b, q)) {
            all_miss.push(view.rescue(q));
        }
    }

    let latency = backends
        .iter()
        .map(|&backend| {
            let values: Vec<f64> = results
                .runs
                .iter()
                .filter(|r| r.backend == backend && r.error.is_none())
                .map(|r| r.latency_ms)
                .collect();
            LatencyRow {
                backend,
                p50: metrics::percentile(&values, 50.0),
                p95: metrics::percentile(&values, 95.0),
            }
        })
        .collect();

    let errors = results
        .runs
        .iter()
        .filter_map(|r| {
            r.error.as_ref().map(|message| ErrorRow {
                query: r.query.clone(),
                backend: r.backend,
                message: message.clone(),
            })
        })
        .collect();

    let s_a = cells
        .get(&(Backend::S, Group::A))
        .copied()
        .unwrap_or_default();
    let vs_best = best_lexical.and_then(|best| {
        mcnemar
            .iter()
            .find(|r: &&McNemarRow| r.other == best && r.group == Group::A)
            .copied()
    });
    let thresholds = Thresholds {
        s_hit5_a: s_a.hit5,
        n_a: s_a.n,
        recall_met: s_a.n > 0 && s_a.hit5 * RECALL_DEN >= RECALL_NUM * s_a.n,
        best_lexical,
        p_vs_best: vs_best.map(|r| r.p),
        mcnemar_met: vs_best.is_some_and(|r| r.p < 0.05 && r.only_s > r.only_other),
    };

    Summary {
        backends,
        cells,
        mcnemar,
        best_lexical,
        separability,
        s_only,
        lexical_only,
        all_miss,
        latency,
        errors,
        thresholds,
    }
}

/// PRD: S recall@5 on duplicates ≥ 20/24.
const RECALL_NUM: usize = 20;
const RECALL_DEN: usize = 24;

/// §5.4's rows: the name shown and the backend whose top-1 score it reads.
const SEPARABILITY: [(&str, Backend); 5] = [
    ("S hybrid RRF", Backend::S),
    ("S-dense cosine", Backend::SDense),
    ("S-sparse per query term", Backend::SSparse),
    ("L-or-len", Backend::LOrLen),
    ("G / G-dens fraction matched", Backend::G),
];

/// The results indexed for lookups.
struct View<'a> {
    queries: &'a [QueryRecord],
    runs: HashMap<(Backend, &'a str), &'a BackendRun>,
    backends: Vec<Backend>,
}

impl<'a> View<'a> {
    fn new(results: &'a Results) -> Self {
        let runs: HashMap<(Backend, &str), &BackendRun> = results
            .runs
            .iter()
            .map(|r| ((r.backend, r.query.as_str()), r))
            .collect();
        let present: BTreeSet<Backend> = results.runs.iter().map(|r| r.backend).collect();
        Self {
            queries: &results.queries,
            runs,
            backends: Backend::ALL
                .into_iter()
                .filter(|b| present.contains(b))
                .collect(),
        }
    }

    fn has(&self, backend: Backend) -> bool {
        self.backends.contains(&backend)
    }

    fn group(&self, group: Group) -> impl Iterator<Item = &'a QueryRecord> + use<'a> {
        self.queries.iter().filter(move |q| group.contains(q.class))
    }

    fn rank(&self, backend: Backend, q: &QueryRecord) -> Option<usize> {
        self.runs
            .get(&(backend, q.id.as_str()))
            .and_then(|r| r.first_rank)
    }

    fn hit5(&self, backend: Backend, q: &QueryRecord) -> bool {
        metrics::hit_at(self.rank(backend, q), 5)
    }

    /// The top-1 score §5.4 compares; 0.0 without a hit. S-sparse's is per query term.
    fn top1(&self, backend: Backend, q: &QueryRecord) -> f64 {
        let score = self
            .runs
            .get(&(backend, q.id.as_str()))
            .and_then(|r| r.ranked.first())
            .map_or(0.0, |row| row.score);
        if backend == Backend::SSparse {
            if q.sparse_terms == 0 {
                0.0
            } else {
                score / q.sparse_terms as f64
            }
        } else {
            score
        }
    }

    fn cell(&self, backend: Backend, group: Group) -> Cell {
        let ranks: Vec<Option<usize>> = self.group(group).map(|q| self.rank(backend, q)).collect();
        let n = ranks.len();
        let hit1 = ranks.iter().filter(|r| metrics::hit_at(**r, 1)).count();
        let hit5 = ranks.iter().filter(|r| metrics::hit_at(**r, 5)).count();
        let mrr10 = if n == 0 {
            0.0
        } else {
            ranks
                .iter()
                .map(|r| metrics::reciprocal_rank(*r, 10))
                .sum::<f64>()
                / n as f64
        };
        Cell {
            n,
            hit1,
            hit5,
            mrr10,
            wilson5: metrics::wilson_interval(hit5, n),
        }
    }

    fn rescue(&self, q: &QueryRecord) -> Rescue {
        Rescue {
            query: q.id.clone(),
            text: q.text.clone(),
            targets: q.targets.clone(),
            ranks: RESCUE_BACKENDS
                .iter()
                .map(|&b| (b, self.rank(b, q).filter(|&r| r <= RANK_DEPTH)))
                .collect(),
        }
    }
}

/// `report.md` (pure).
pub fn render_markdown(results: &Results, summary: &Summary) -> String {
    let meta = &results.meta;
    let mut md = String::new();
    let _ = writeln!(
        md,
        "# htui-bench: {} on {}\n",
        meta.query_set, meta.corpus_ref
    );

    md.push_str("## Run\n\n| field | value |\n|---|---|\n");
    let points_total = meta.points.item + meta.points.requirement + meta.points.document;
    let points_per_s = if meta.timings_ms.index > 0.0 {
        format!(
            "{:.1}",
            points_total as f64 / (meta.timings_ms.index / 1000.0)
        )
    } else {
        "—".to_owned()
    };
    let docs: Vec<String> = meta.docs.iter().map(|(k, v)| format!("{k} {v}")).collect();
    let load = |l: Option<[f64; 3]>| {
        l.map_or("—".to_owned(), |[a, b, c]| {
            format!("{a:.2} {b:.2} {c:.2}")
        })
    };
    let rows: Vec<(&str, String)> = vec![
        ("started_at", meta.started_at.to_rfc3339()),
        ("finished_at", meta.finished_at.to_rfc3339()),
        ("corpus_ref", meta.corpus_ref.clone()),
        (
            "corpus_commit",
            meta.corpus_commit
                .clone()
                .unwrap_or_else(|| "work tree".into()),
        ),
        ("docs", docs.join(", ")),
        (
            "points",
            format!(
                "item {}, requirement {}, document {} (total {points_total})",
                meta.points.item, meta.points.requirement, meta.points.document
            ),
        ),
        ("points/s (index)", points_per_s),
        ("query_set", meta.query_set.clone()),
        ("query_file", meta.query_file.clone()),
        ("query_sha256", meta.query_sha256.clone()),
        ("embedder", meta.embedder.clone()),
        (
            "embedder_identity",
            serde_json::to_string(&meta.embedder_identity).unwrap_or_default(),
        ),
        ("qdrant_version", meta.qdrant_version.clone()),
        ("collection", meta.collection.clone()),
        ("postgres_version", meta.postgres_version.clone()),
        ("htui_commit", meta.htui_commit.clone()),
        ("htui_dirty", meta.htui_dirty.to_string()),
        ("host.os", meta.host.os.clone()),
        ("host.cpus", meta.host.cpus.to_string()),
        ("host.load_start", load(meta.host.load_start)),
        ("host.load_end", load(meta.host.load_end)),
        (
            "timings_ms.model_load",
            meta.timings_ms
                .model_load
                .map_or("— (embedder passed in)".to_owned(), |t| format!("{t:.0}")),
        ),
        ("timings_ms.index", format!("{:.0}", meta.timings_ms.index)),
        (
            "timings_ms.pg_load",
            format!("{:.0}", meta.timings_ms.pg_load),
        ),
        (
            "timings_ms.queries",
            format!("{:.0}", meta.timings_ms.queries),
        ),
        ("rank_depth", meta.rank_depth.to_string()),
    ];
    for (key, value) in rows {
        let _ = writeln!(md, "| {key} | {} |", cell_text(&value));
    }

    let t = &summary.thresholds;
    md.push_str("\n## Thresholds\n\n");
    let _ = writeln!(
        md,
        "- S recall@5 on duplicates: {}/{} ≥ {RECALL_NUM}/{RECALL_DEN} — {}",
        t.s_hit5_a,
        t.n_a,
        met(t.recall_met)
    );
    match (t.best_lexical, t.p_vs_best) {
        (Some(best), Some(p)) => {
            let _ = writeln!(
                md,
                "- S vs best lexical baseline ({}) on duplicates: p = {} < 0.05 — {}",
                best.name(),
                format_p(p),
                met(t.mcnemar_met)
            );
        }
        _ => {
            md.push_str(
                "- S vs best lexical baseline on duplicates: not measured (S or every lexical \
                 baseline missing) — not met\n",
            );
        }
    }
    md.push_str("- Reproducibility: compare with a second run (htui-bench compare)\n");

    let header = |md: &mut String, first: &str| {
        let groups: Vec<String> = Group::ALL
            .iter()
            .map(|&g| format!("{} ({})", g.label(), group_n(summary, g)))
            .collect();
        let _ = writeln!(md, "| {first} | {} |", groups.join(" | "));
        let _ = writeln!(md, "|---|{}", "---|".repeat(Group::ALL.len()));
    };

    md.push_str("\n## Results (recall@1 / recall@5 / MRR@10)\n\n");
    header(&mut md, "backend");
    for &backend in &summary.backends {
        let cells: Vec<String> = Group::ALL
            .iter()
            .map(|&g| {
                let c = summary
                    .cells
                    .get(&(backend, g))
                    .copied()
                    .unwrap_or_default();
                format!(
                    "{:.2} / {:.2} / {:.2}",
                    ratio(c.hit1, c.n),
                    ratio(c.hit5, c.n),
                    c.mrr10
                )
            })
            .collect();
        let _ = writeln!(md, "| {} | {} |", backend.name(), cells.join(" | "));
    }
    md.push_str("\nS-points ranks points, not corpus IDs (ANA-37 §11).\n");

    md.push_str("\n## Recall@5 with Wilson 95% intervals\n\n");
    header(&mut md, "backend");
    for &backend in &summary.backends {
        let cells: Vec<String> = Group::ALL
            .iter()
            .map(|&g| {
                let c = summary
                    .cells
                    .get(&(backend, g))
                    .copied()
                    .unwrap_or_default();
                format!("{}/{} ({:.2}-{:.2})", c.hit5, c.n, c.wilson5.0, c.wilson5.1)
            })
            .collect();
        let _ = writeln!(md, "| {} | {} |", backend.name(), cells.join(" | "));
    }

    md.push_str("\n## Paired exact McNemar on recall@5, S against each backend\n\n");
    if summary.mcnemar.is_empty() {
        md.push_str("none (S did not run)\n");
    } else {
        let groups: Vec<String> = Group::ALL
            .iter()
            .map(|g| format!("{0} S only | {0} other only | {0} p", g.label()))
            .collect();
        let _ = writeln!(md, "| backend | {} |", groups.join(" | "));
        let _ = writeln!(md, "|---|{}", "---|".repeat(3 * Group::ALL.len()));
        for &other in summary.backends.iter().filter(|&&b| b != Backend::S) {
            let cells: Vec<String> = Group::ALL
                .iter()
                .map(|&g| {
                    summary
                        .mcnemar
                        .iter()
                        .find(|r| r.other == other && r.group == g)
                        .map_or("— | — | —".to_owned(), |r| {
                            format!("{} | {} | {}", r.only_s, r.only_other, format_p(r.p))
                        })
                })
                .collect();
            let _ = writeln!(md, "| {} | {} |", other.name(), cells.join(" | "));
        }
    }

    md.push_str("\n## Separability of the top-1 score (duplicates against no-target)\n\n");
    md.push_str("| score | AUC | recall at zero false alarms |\n|---|---|---|\n");
    let two = |v: Option<f64>| v.map_or("—".to_owned(), |v| format!("{v:.2}"));
    for row in &summary.separability {
        let _ = writeln!(
            md,
            "| {} | {} | {} |",
            row.name,
            two(row.auc),
            two(row.recall_zero_false_alarms)
        );
    }

    for (title, list) in [
        (
            "S finds it in the top 5 and every lexical baseline misses",
            &summary.s_only,
        ),
        (
            "A lexical baseline finds it in the top 5 and S misses",
            &summary.lexical_only,
        ),
        ("Every backend misses the top 5", &summary.all_miss),
    ] {
        let _ = writeln!(md, "\n## {title}\n");
        if list.is_empty() {
            md.push_str("none\n");
            continue;
        }
        let names: Vec<&str> = RESCUE_BACKENDS.iter().map(|b| b.name()).collect();
        let _ = writeln!(md, "| query | text | targets | {} |", names.join(" / "));
        md.push_str("|---|---|---|---|\n");
        for rescue in list {
            let targets: Vec<&str> = rescue.targets.iter().map(CorpusId::as_str).collect();
            let ranks: Vec<String> = rescue
                .ranks
                .iter()
                .map(|(_, r)| r.map_or("–".to_owned(), |r| r.to_string()))
                .collect();
            let _ = writeln!(
                md,
                "| {} | {} | {} | {} |",
                rescue.query,
                cut_text(&rescue.text),
                targets.join(", "),
                ranks.join(" / ")
            );
        }
    }

    md.push_str("\n## Latency (ms, p50 / p95)\n\n| backend | p50 | p95 |\n|---|---|---|\n");
    let ms = |v: Option<f64>| v.map_or("—".to_owned(), |v| format!("{v:.1}"));
    for row in &summary.latency {
        let _ = writeln!(
            md,
            "| {} | {} | {} |",
            row.backend.name(),
            ms(row.p50),
            ms(row.p95)
        );
    }

    md.push_str("\n## Errors\n\n");
    if summary.errors.is_empty() {
        md.push_str("none\n");
    } else {
        md.push_str("| query | backend | message |\n|---|---|---|\n");
        for e in &summary.errors {
            let _ = writeln!(
                md,
                "| {} | {} | {} |",
                e.query,
                e.backend.name(),
                cell_text(&e.message)
            );
        }
    }

    md.push_str("\n## Threats to validity\n\n");
    md.push_str(THREATS);
    md
}

/// The fixed threats section (blueprint §4.2).
const THREATS: &str = "\
- The author knew the targets when writing the queries (ANA-37 §5.9).
- The queries are re-authored from ANA-37 §5.3, not ANA-37's lost set: easier or harder in ways no \
table here shows.
- Hybrid (RRF) ties reorder between runs; compare two runs before reading a one-query difference.
- The corpus is the markdown at a pinned commit, not htui's Postgres rows.
- This corpus chunks to 777 document points at the pin against ANA-37's 759 (blueprint F3).
";

fn met(yes: bool) -> &'static str {
    if yes { "met" } else { "not met" }
}

fn ratio(k: usize, n: usize) -> f64 {
    if n == 0 { 0.0 } else { k as f64 / n as f64 }
}

/// Queries in a group (the same for every backend).
fn group_n(summary: &Summary, group: Group) -> usize {
    summary
        .backends
        .first()
        .and_then(|&b| summary.cells.get(&(b, group)))
        .map_or(0, |c| c.n)
}

/// Writes `<out>/results.json` and `<out>/report.md`, creating `out`.
///
/// # Errors
/// A directory or file that cannot be written.
pub fn write(results: &Results, out: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut json = serde_json::to_string_pretty(results).context("serialising the results")?;
    json.push('\n');
    let results_path = out.join("results.json");
    std::fs::write(&results_path, json)
        .with_context(|| format!("writing {}", results_path.display()))?;
    let report_path = out.join("report.md");
    std::fs::write(&report_path, render_markdown(results, &summarize(results)))
        .with_context(|| format!("writing {}", report_path.display()))?;
    Ok(())
}

/// A results file, or a directory holding `results.json`.
///
/// # Errors
/// A missing or unreadable file, or another schema.
pub fn load_results(path: &Path) -> anyhow::Result<Results> {
    let file = if path.is_dir() {
        path.join("results.json")
    } else {
        path.to_path_buf()
    };
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let results: Results =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
    if results.schema != RESULTS_SCHEMA {
        bail!(
            "{} has schema {}, this build reads schema {RESULTS_SCHEMA}",
            file.display(),
            results.schema
        );
    }
    Ok(results)
}

/// Hits@5 of one backend and group in two runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompareRow {
    /// The backend.
    pub backend: Backend,
    /// The group.
    pub group: Group,
    /// Queries compared in the group.
    pub n: usize,
    /// Hits@5 in run A.
    pub hit5_a: usize,
    /// Hits@5 in run B.
    pub hit5_b: usize,
    /// `hit5_b - hit5_a`.
    pub delta: i64,
}

/// The queries whose top 10 differs between two runs, for one backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedTop {
    /// The backend.
    pub backend: Backend,
    /// Query ids, in run A's order.
    pub queries: Vec<String>,
}

/// Two runs side by side (P9).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Comparison {
    /// Both runs used the same query file (equal sha256).
    pub same_queries: bool,
    /// Run A's query file hash.
    pub sha256_a: String,
    /// Run B's.
    pub sha256_b: String,
    /// Query ids both runs hold.
    pub common_queries: usize,
    /// Per backend and group.
    pub rows: Vec<CompareRow>,
    /// Per backend.
    pub changed: Vec<ChangedTop>,
    /// |Δ hits@5| ≤ 1 for S in each of groups a, b and c (the PRD reproducibility metric).
    pub s_within_one_per_class: bool,
}

/// Compares two runs (pure).
pub fn compare(a: &Results, b: &Results) -> Comparison {
    let same_queries = a.meta.query_sha256 == b.meta.query_sha256;
    let in_b: BTreeSet<&str> = b.queries.iter().map(|q| q.id.as_str()).collect();
    let common: Vec<&QueryRecord> = a
        .queries
        .iter()
        .filter(|q| in_b.contains(q.id.as_str()))
        .collect();
    let (view_a, view_b) = (View::new(a), View::new(b));
    let backends: Vec<Backend> = Backend::ALL
        .into_iter()
        .filter(|&backend| view_a.has(backend) || view_b.has(backend))
        .collect();

    let mut rows = Vec::new();
    for &backend in &backends {
        for group in Group::ALL {
            let queries: Vec<&QueryRecord> = common
                .iter()
                .copied()
                .filter(|q| group.contains(q.class))
                .collect();
            let hits = |view: &View<'_>| queries.iter().filter(|q| view.hit5(backend, q)).count();
            let (hit5_a, hit5_b) = (hits(&view_a), hits(&view_b));
            rows.push(CompareRow {
                backend,
                group,
                n: queries.len(),
                hit5_a,
                hit5_b,
                delta: hit5_b as i64 - hit5_a as i64,
            });
        }
    }

    let top = |view: &View<'_>, backend: Backend, id: &str| -> Vec<CorpusId> {
        view.runs.get(&(backend, id)).map_or_else(Vec::new, |r| {
            r.ranked
                .iter()
                .take(RANK_DEPTH)
                .map(|row| row.id.clone())
                .collect()
        })
    };
    let changed = backends
        .iter()
        .map(|&backend| ChangedTop {
            backend,
            queries: common
                .iter()
                .filter(|q| top(&view_a, backend, &q.id) != top(&view_b, backend, &q.id))
                .map(|q| q.id.clone())
                .collect(),
        })
        .collect();

    let s_within_one_per_class = view_a.has(Backend::S)
        && view_b.has(Backend::S)
        && rows
            .iter()
            .filter(|r| r.backend == Backend::S && r.group != Group::Abc)
            .all(|r| r.delta.abs() <= 1);

    Comparison {
        same_queries,
        sha256_a: a.meta.query_sha256.clone(),
        sha256_b: b.meta.query_sha256.clone(),
        common_queries: common.len(),
        rows,
        changed,
        s_within_one_per_class,
    }
}

/// The comparison as markdown (pure).
pub fn render_comparison(c: &Comparison) -> String {
    let mut md = String::new();
    if !c.same_queries {
        let _ = writeln!(
            md,
            "**Warning:** the runs used different query files (sha256 {} against {}); compared \
             over the {} query ids both hold.\n",
            c.sha256_a, c.sha256_b, c.common_queries
        );
    }
    md.push_str("# htui-bench compare\n\n## Recall@5 hits, run A → run B (Δ)\n\n");
    let groups: Vec<String> = Group::ALL
        .iter()
        .map(|&g| {
            let n = c.rows.iter().find(|r| r.group == g).map_or(0, |r| r.n);
            format!("{} ({n})", g.label())
        })
        .collect();
    let _ = writeln!(md, "| backend | {} |", groups.join(" | "));
    let _ = writeln!(md, "|---|{}", "---|".repeat(Group::ALL.len()));
    let mut backends: Vec<Backend> = c.rows.iter().map(|r| r.backend).collect();
    backends.dedup();
    for backend in backends {
        let cells: Vec<String> = Group::ALL
            .iter()
            .map(|&g| {
                c.rows
                    .iter()
                    .find(|r| r.backend == backend && r.group == g)
                    .map_or("—".to_owned(), |r| {
                        format!("{} → {} ({:+})", r.hit5_a, r.hit5_b, r.delta)
                    })
            })
            .collect();
        let _ = writeln!(md, "| {} | {} |", backend.name(), cells.join(" | "));
    }

    md.push_str(
        "\n## Queries whose top 10 changed\n\n| backend | count | queries |\n|---|---|---|\n",
    );
    for ch in &c.changed {
        let _ = writeln!(
            md,
            "| {} | {} | {} |",
            ch.backend.name(),
            ch.queries.len(),
            ch.queries.join(", ")
        );
    }
    let _ = writeln!(
        md,
        "\nS recall@5 within one query per class: {}",
        if c.s_within_one_per_class {
            "yes"
        } else {
            "no"
        }
    );
    md
}

/// A McNemar p: `{:.4}`, or `< 0.0001` below that.
fn format_p(p: f64) -> String {
    if p >= 0.0001 {
        format!("{p:.4}")
    } else {
        "< 0.0001".to_owned()
    }
}

/// A table cell: at most 80 chars (`…` marks a cut), `|` escaped.
fn cut_text(text: &str) -> String {
    let cut: String = if text.chars().count() > 80 {
        text.chars().take(79).chain(std::iter::once('…')).collect()
    } else {
        text.to_owned()
    };
    cell_text(&cut)
}

/// A table cell's text: `|` escaped, line breaks as spaces.
fn cell_text(text: &str) -> String {
    text.replace('|', "\\|").replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::PointCounts;
    use crate::metrics;
    use crate::run::{BackendRun, HostInfo, QueryRecord, RESULTS_SCHEMA, RunMeta, Timings};
    use crate::types::{RANK_DEPTH, Ranked};
    use chrono::DateTime;
    use htui_store::embed::{DENSE_DIM, EmbedderIdentity};

    type List = &'static [(&'static str, f64)];

    fn record(id: &str, class: QueryClass, targets: &[&str]) -> QueryRecord {
        QueryRecord {
            id: id.into(),
            class,
            text: format!("the text of {id}"),
            targets: targets.iter().map(|t| CorpusId::new(*t)).collect(),
            sparse_terms: 3,
        }
    }

    fn backend_run(backend: Backend, query: &QueryRecord, list: List, latency: f64) -> BackendRun {
        let ranked: Vec<Ranked> = list
            .iter()
            .map(|(id, score)| Ranked {
                id: CorpusId::new(*id),
                score: *score,
            })
            .collect();
        let ids: Vec<CorpusId> = ranked.iter().map(|r| r.id.clone()).collect();
        BackendRun {
            backend,
            query: query.id.clone(),
            first_rank: metrics::first_target_rank(&ids, &query.targets),
            ranked,
            latency_ms: latency,
            error: None,
        }
    }

    fn meta() -> RunMeta {
        let identity = EmbedderIdentity::hash(DENSE_DIM);
        RunMeta {
            started_at: DateTime::from_timestamp(1_760_000_000, 0).expect("time"),
            finished_at: DateTime::from_timestamp(1_760_000_100, 0).expect("time"),
            corpus_ref: "d15e9fea".into(),
            corpus_commit: Some("d15e9fea0000000000000000000000000000beef".into()),
            docs: [
                ("open_item", 3),
                ("requirement", 1),
                ("decision", 1),
                ("total", 5),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
            points: PointCounts {
                item: 4,
                requirement: 1,
                document: 5,
            },
            query_set: "fixture".into(),
            query_file: "queries/fixture.json".into(),
            query_sha256: "ab".repeat(32),
            embedder: identity.to_string(),
            embedder_identity: identity,
            qdrant_version: "1.19.1".into(),
            collection: "htui_bench_d15e9fea".into(),
            postgres_version: "PostgreSQL 17.0".into(),
            htui_commit: "eb2d60fa".into(),
            htui_dirty: false,
            host: HostInfo {
                os: "linux".into(),
                cpus: 8,
                load_start: Some([0.5, 0.25, 0.125]),
                load_end: None,
            },
            timings_ms: Timings {
                model_load: None,
                index: 2_000.0,
                pg_load: 50.0,
                queries: 900.0,
            },
            rank_depth: RANK_DEPTH,
        }
    }

    /// 6 queries (2 a, 1 b, 1 c, 2 d) and 5 backends; the expected numbers are worked out in the
    /// comments of the tests below.
    fn fixture() -> Results {
        let queries = vec![
            record("a01", QueryClass::A, &["MOD-1"]),
            record("a02", QueryClass::A, &["MOD-2"]),
            record("b01", QueryClass::B, &["R-X-1", "DEC:ANA-4"]),
            record("c01", QueryClass::C, &["MOD-3"]),
            record("d01", QueryClass::D, &[]),
            record("d02", QueryClass::D, &[]),
        ];
        let table: [(Backend, [List; 6]); 5] = [
            (
                Backend::S,
                [
                    &[("MOD-1", 0.9)],
                    &[
                        ("MOD-9", 0.8),
                        ("MOD-8", 0.7),
                        ("MOD-7", 0.6),
                        ("MOD-6", 0.5),
                        ("MOD-5", 0.4),
                        ("MOD-2", 0.3),
                    ],
                    &[("MOD-5", 0.7), ("DEC:ANA-4", 0.6)],
                    &[("MOD-3", 0.95)],
                    &[("MOD-5", 0.3)],
                    &[],
                ],
            ),
            (
                Backend::SDense,
                [
                    &[("MOD-4", 0.8), ("MOD-1", 0.7)],
                    &[("MOD-2", 0.85)],
                    &[],
                    &[("MOD-4", 0.6)],
                    &[("MOD-5", 0.9)],
                    &[("MOD-6", 0.2)],
                ],
            ),
            (
                Backend::LOrLen,
                [&[], &[("MOD-2", 0.1)], &[], &[("MOD-3", 0.5)], &[], &[]],
            ),
            (
                Backend::G,
                [
                    &[("MOD-7", 0.5)],
                    &[("MOD-8", 0.5), ("MOD-2", 0.5)],
                    &[],
                    &[("MOD-3", 1.0)],
                    &[("MOD-5", 0.5)],
                    &[],
                ],
            ),
            (Backend::H, [&[], &[], &[], &[("MOD-3", 1.0)], &[], &[]]),
        ];
        let mut runs = Vec::new();
        for (i, query) in queries.iter().enumerate() {
            for (backend, lists) in &table {
                runs.push(backend_run(
                    *backend,
                    query,
                    lists[i],
                    10.0 * (i + 1) as f64,
                ));
            }
        }
        let failed = runs
            .iter_mut()
            .find(|r| r.backend == Backend::H && r.query == "d02")
            .expect("H on d02");
        failed.error = Some("boom".into());
        failed.latency_ms = 100.0;
        Results {
            schema: RESULTS_SCHEMA,
            meta: meta(),
            queries,
            runs,
        }
    }

    fn close(got: f64, want: f64) -> bool {
        (got - want).abs() < 1e-9
    }

    fn cell(summary: &Summary, backend: Backend, group: Group) -> Cell {
        *summary
            .cells
            .get(&(backend, group))
            .unwrap_or_else(|| panic!("no cell for {backend:?} {group:?}"))
    }

    fn rescue_ids(list: &[Rescue]) -> Vec<&str> {
        list.iter().map(|r| r.query.as_str()).collect()
    }

    #[test]
    fn summarize_counts_hits_and_mrr_per_group() {
        let s = summarize(&fixture());
        assert_eq!(
            s.backends,
            [
                Backend::S,
                Backend::SDense,
                Backend::LOrLen,
                Backend::G,
                Backend::H
            ]
        );
        // S: a01 rank 1, a02 rank 6, b01 rank 2, c01 rank 1.
        let a = cell(&s, Backend::S, Group::A);
        assert_eq!((a.n, a.hit1, a.hit5), (2, 1, 1));
        assert!(close(a.mrr10, (1.0 + 1.0 / 6.0) / 2.0), "{a:?}");
        assert_eq!(a.wilson5, metrics::wilson_interval(1, 2));
        let b = cell(&s, Backend::S, Group::B);
        assert_eq!((b.n, b.hit1, b.hit5), (1, 0, 1));
        assert!(close(b.mrr10, 0.5));
        let abc = cell(&s, Backend::S, Group::Abc);
        assert_eq!((abc.n, abc.hit1, abc.hit5), (4, 2, 3));
        assert!(close(abc.mrr10, (1.0 + 1.0 / 6.0 + 0.5 + 1.0) / 4.0));
        // S-dense: a01 rank 2, a02 rank 1, b01 and c01 missed.
        let dense = cell(&s, Backend::SDense, Group::A);
        assert_eq!((dense.hit1, dense.hit5), (1, 2));
        assert!(close(dense.mrr10, 0.75));
        assert_eq!(cell(&s, Backend::SDense, Group::C).hit5, 0);
        // H finds c01 only; its error on d02 (class d) counts nowhere.
        let h = cell(&s, Backend::H, Group::Abc);
        assert_eq!((h.n, h.hit1, h.hit5), (4, 1, 1));
        assert!(close(h.mrr10, 0.25));
        // A backend absent from the run has no cell.
        assert!(!s.cells.contains_key(&(Backend::SSparse, Group::A)));
    }

    #[test]
    fn summarize_pairs_s_with_every_backend() {
        let s = summarize(&fixture());
        let row = |other: Backend, group: Group| {
            *s.mcnemar
                .iter()
                .find(|r| r.other == other && r.group == group)
                .unwrap_or_else(|| panic!("no McNemar row {other:?} {group:?}"))
        };
        assert_eq!(s.mcnemar.len(), 4 * Group::ALL.len());
        assert!(s.mcnemar.iter().all(|r| r.other != Backend::S));
        let dense_a = row(Backend::SDense, Group::A);
        assert_eq!((dense_a.only_s, dense_a.only_other), (0, 1));
        assert!(close(dense_a.p, 1.0));
        let l_a = row(Backend::LOrLen, Group::A);
        assert_eq!((l_a.only_s, l_a.only_other), (1, 1));
        let h_abc = row(Backend::H, Group::Abc);
        assert_eq!((h_abc.only_s, h_abc.only_other), (2, 0));
        assert!(close(h_abc.p, metrics::mcnemar_exact(2, 0)));
    }

    #[test]
    fn summarize_picks_the_best_lexical_baseline_and_the_thresholds() {
        let s = summarize(&fixture());
        // L-or-len and G both hit 1 of 2 duplicates; L-or-len's MRR (0.5) beats G's (0.25).
        assert_eq!(s.best_lexical, Some(Backend::LOrLen));
        let t = &s.thresholds;
        assert_eq!((t.s_hit5_a, t.n_a), (1, 2));
        assert!(!t.recall_met);
        assert_eq!(t.best_lexical, Some(Backend::LOrLen));
        assert!(close(t.p_vs_best.expect("a p"), 1.0));
        assert!(!t.mcnemar_met);
    }

    #[test]
    fn summarize_measures_separability_of_the_top_score() {
        let s = summarize(&fixture());
        let names: Vec<&str> = s.separability.iter().map(|r| r.name.as_str()).collect();
        // S-sparse did not run, so its row is absent.
        assert_eq!(
            names,
            [
                "S hybrid RRF",
                "S-dense cosine",
                "L-or-len",
                "G / G-dens fraction matched"
            ]
        );
        let row = |name: &str| {
            s.separability
                .iter()
                .find(|r| r.name == name)
                .expect("row")
                .clone()
        };
        // S: positives 0.9, 0.8; negatives 0.3, 0.0 (none).
        assert_eq!(row("S hybrid RRF").auc, Some(1.0));
        assert_eq!(row("S hybrid RRF").recall_zero_false_alarms, Some(1.0));
        // S-dense: 0.8, 0.85 against 0.9, 0.2.
        assert_eq!(row("S-dense cosine").auc, Some(0.5));
        assert_eq!(row("S-dense cosine").recall_zero_false_alarms, Some(0.0));
        // L-or-len: 0.0, 0.1 against 0.0, 0.0.
        assert_eq!(row("L-or-len").auc, Some(0.75));
        assert_eq!(row("L-or-len").recall_zero_false_alarms, Some(0.5));
        // G: 0.5, 0.5 against 0.5, 0.0.
        assert_eq!(row("G / G-dens fraction matched").auc, Some(0.75));
        assert_eq!(
            row("G / G-dens fraction matched").recall_zero_false_alarms,
            Some(0.0)
        );
    }

    #[test]
    fn s_sparse_separability_divides_by_the_query_terms() {
        let mut results = fixture();
        let extra: Vec<BackendRun> = results
            .runs
            .iter()
            .filter(|r| r.backend == Backend::S)
            .map(|r| BackendRun {
                backend: Backend::SSparse,
                ..r.clone()
            })
            .collect();
        results.runs.extend(extra);
        // d01 has no sparse term: its score counts as 0.
        results.queries[4].sparse_terms = 0;
        let s = summarize(&results);
        let row = s
            .separability
            .iter()
            .find(|r| r.name == "S-sparse per query term")
            .expect("the S-sparse row");
        // positives 0.3, 0.267 against 0.0 (no terms), 0.0 (no hit).
        assert_eq!(row.auc, Some(1.0));
        assert_eq!(row.recall_zero_false_alarms, Some(1.0));
    }

    #[test]
    fn summarize_lists_the_rescues() {
        let s = summarize(&fixture());
        assert_eq!(rescue_ids(&s.s_only), ["a01", "b01"]);
        assert_eq!(rescue_ids(&s.lexical_only), ["a02"]);
        assert!(s.all_miss.is_empty());
        let a01 = &s.s_only[0];
        assert_eq!(a01.targets, vec![CorpusId::new("MOD-1")]);
        assert_eq!(
            a01.ranks,
            vec![
                (Backend::S, Some(1)),
                (Backend::SDense, Some(2)),
                (Backend::SSparse, None),
                (Backend::LOrLen, None),
                (Backend::GDens, None),
                (Backend::H, None),
            ]
        );
    }

    #[test]
    fn summarize_takes_latency_and_errors() {
        let s = summarize(&fixture());
        let latency = |b: Backend| {
            *s.latency
                .iter()
                .find(|r| r.backend == b)
                .expect("latency row")
        };
        assert_eq!(latency(Backend::S).p50, Some(30.0));
        assert_eq!(latency(Backend::S).p95, Some(60.0));
        // H's failed call (100 ms) is excluded.
        assert_eq!(latency(Backend::H).p95, Some(50.0));
        assert_eq!(
            s.errors,
            vec![ErrorRow {
                query: "d02".into(),
                backend: Backend::H,
                message: "boom".into()
            }]
        );
    }

    #[test]
    fn render_markdown_has_thresholds_tables_and_rank_markers() {
        let results = fixture();
        let md = render_markdown(&results, &summarize(&results));
        assert!(
            md.starts_with("# htui-bench: fixture on d15e9fea\n"),
            "{md}"
        );
        for heading in [
            "## Run",
            "## Thresholds",
            "## Results (recall@1 / recall@5 / MRR@10)",
            "## Recall@5 with Wilson 95% intervals",
            "## Paired exact McNemar on recall@5, S against each backend",
            "## Separability of the top-1 score (duplicates against no-target)",
            "## Latency (ms, p50 / p95)",
            "## Errors",
            "## Threats to validity",
        ] {
            assert!(md.contains(heading), "missing {heading}\n{md}");
        }
        assert!(
            md.contains("S recall@5 on duplicates: 1/2 ≥ 20/24 — not met"),
            "{md}"
        );
        assert!(
            md.contains(
                "S vs best lexical baseline (L-or-len) on duplicates: p = 1.0000 < 0.05 — not met"
            ),
            "{md}"
        );
        assert!(
            md.contains("Reproducibility: compare with a second run (htui-bench compare)"),
            "{md}"
        );
        assert!(
            md.contains("| a duplicates (2) | b questions (1) | c identifiers (1) | a+b+c (4) |"),
            "{md}"
        );
        assert!(
            md.contains("| S | 0.50 / 0.50 / 0.58 | 0.00 / 1.00 / 0.50 | 1.00 / 1.00 / 1.00 | 0.50 / 0.75 / 0.67 |"),
            "{md}"
        );
        assert!(md.contains("| S | 1/2 ("), "{md}");
        assert!(
            md.contains("| a01 | the text of a01 | MOD-1 | 1 / 2 / – / – / – / – |"),
            "{md}"
        );
        assert!(md.contains("| d02 | H | boom |"), "{md}");
        assert!(md.contains(&"ab".repeat(32)), "the full sha256\n{md}");
        // The F3 threat reads the run's own point counts, never the pin's 777.
        assert!(
            md.contains(
                "- This corpus chunks to 5 document points (10 points in all) at d15e9fea against \
                 ANA-37's 759 (blueprint F3).\n"
            ),
            "the F3 threat\n{md}"
        );
        assert!(!md.contains("777"), "{md}");
    }

    #[test]
    fn the_f3_threat_follows_the_run_metadata() {
        let mut results = fixture();
        results.meta.corpus_ref = "WORKTREE".into();
        results.meta.points.document = 812;
        let md = render_markdown(&results, &summarize(&results));
        assert!(
            md.contains(
                "- This corpus chunks to 812 document points (817 points in all) at WORKTREE \
                 against ANA-37's 759 (blueprint F3).\n"
            ),
            "{md}"
        );
    }

    #[test]
    fn mcnemar_p_prints_small_values_as_a_bound() {
        assert_eq!(format_p(0.5), "0.5000");
        assert_eq!(format_p(0.0001), "0.0001");
        assert_eq!(format_p(0.000_061), "< 0.0001");
    }

    #[test]
    fn long_texts_are_cut_to_eighty_chars() {
        let cut = cut_text(&"x".repeat(100));
        assert_eq!(cut.chars().count(), 80);
        assert!(cut.ends_with('…'));
        assert_eq!(cut_text("short | piped"), "short \\| piped");
    }

    #[test]
    fn results_round_trip_through_json_and_disk() {
        let results = fixture();
        let json = serde_json::to_string_pretty(&results).expect("serialises");
        let back: Results = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, results);
        assert!(json.contains("\"backend\": \"L-or-len\""), "{json}");
        assert!(json.contains("\"class\": \"a\""), "{json}");

        let dir = std::env::temp_dir().join(format!(
            "htui-bench-report-{}",
            uuid::Uuid::now_v7().simple()
        ));
        write(&results, &dir).expect("written");
        let text = std::fs::read_to_string(dir.join("results.json")).expect("results.json");
        assert!(text.ends_with("}\n"));
        assert!(dir.join("report.md").is_file());
        assert_eq!(load_results(&dir).expect("a directory"), results);
        assert_eq!(
            load_results(&dir.join("results.json")).expect("a file"),
            results
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_results_refuses_another_schema() {
        let mut results = fixture();
        results.schema = 99;
        let dir = std::env::temp_dir().join(format!(
            "htui-bench-schema-{}",
            uuid::Uuid::now_v7().simple()
        ));
        write(&results, &dir).expect("written");
        let err = format!("{:#}", load_results(&dir).expect_err("refused"));
        assert!(err.contains("schema 99"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `fixture()` with S's ranked list on `query` replaced.
    fn with_s(mut results: Results, query: &str, list: List) -> Results {
        let record = results
            .queries
            .iter()
            .find(|q| q.id == query)
            .expect("query")
            .clone();
        let run = results
            .runs
            .iter_mut()
            .find(|r| r.backend == Backend::S && r.query == query)
            .expect("S row");
        *run = backend_run(Backend::S, &record, list, run.latency_ms);
        results
    }

    #[test]
    fn compare_reports_deltas_and_changed_top_tens() {
        let a = fixture();
        let b = with_s(fixture(), "a02", &[("MOD-2", 0.9)]);
        let c = compare(&a, &b);
        assert!(c.same_queries);
        assert_eq!(c.common_queries, 6);
        let row = |backend: Backend, group: Group| {
            *c.rows
                .iter()
                .find(|r| r.backend == backend && r.group == group)
                .expect("row")
        };
        let s_a = row(Backend::S, Group::A);
        assert_eq!((s_a.n, s_a.hit5_a, s_a.hit5_b, s_a.delta), (2, 1, 2, 1));
        assert_eq!(row(Backend::S, Group::Abc).delta, 1);
        assert_eq!(row(Backend::G, Group::A).delta, 0);
        let changed_s = c
            .changed
            .iter()
            .find(|ch| ch.backend == Backend::S)
            .expect("S");
        assert_eq!(changed_s.queries, ["a02"]);
        assert!(
            c.changed
                .iter()
                .filter(|ch| ch.backend != Backend::S)
                .all(|ch| ch.queries.is_empty())
        );
        assert!(c.s_within_one_per_class);

        // Two duplicates move: outside one query per class.
        let worse = with_s(fixture(), "a01", &[]);
        let c2 = compare(&worse, &b);
        assert_eq!(
            c2.rows
                .iter()
                .find(|r| r.backend == Backend::S && r.group == Group::A)
                .expect("row")
                .delta,
            2
        );
        assert!(!c2.s_within_one_per_class);

        let md = render_comparison(&c);
        assert!(md.contains("| S |"), "{md}");
        assert!(md.contains("1 → 2 (+1)"), "{md}");
        assert!(md.contains("a02"), "{md}");
        assert!(md.contains("within one query per class: yes"), "{md}");
        assert!(!md.contains("Warning"), "{md}");
    }

    #[test]
    fn compare_warns_on_another_query_file_and_uses_shared_ids() {
        let a = fixture();
        let mut b = fixture();
        b.meta.query_sha256 = "cd".repeat(32);
        b.queries.retain(|q| q.id != "b01");
        b.runs.retain(|r| r.query != "b01");
        let c = compare(&a, &b);
        assert!(!c.same_queries);
        assert_eq!(c.common_queries, 5);
        assert_eq!(
            c.rows
                .iter()
                .find(|r| r.backend == Backend::S && r.group == Group::B)
                .expect("row")
                .n,
            0
        );
        let md = render_comparison(&c);
        assert!(md.starts_with("**Warning:**"), "{md}");
    }
}
