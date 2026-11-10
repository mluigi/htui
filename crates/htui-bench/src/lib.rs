//! MOD-162: htui's retrieval benchmark (`docs/htui-bench.md`). Rebuilds ANA-37 §5's measurement
//! in the repo: a corpus from a pinned commit, points through `htui_store::vector_sync`, the
//! production search (`QdrantStore::search_arms`), lexical baselines, a hash-frozen query set and
//! a report. A dev tool: nothing here ships in htui.
#![warn(missing_docs)]

pub mod corpus; // T3
pub mod lexical; // T4: G, G-dens, H
pub mod metrics; // T2
pub mod pg_fts; // T4: L-*
pub mod queries; // T5
pub mod report; // T6
pub mod run; // T6
pub mod types; // T2: shared vocabulary

pub use types::{Backend, CorpusDoc, CorpusId, DocKind, LVariant, QueryClass, Ranked};
