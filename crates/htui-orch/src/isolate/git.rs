//! Every git fact this crate knows, and the only place it spawns a process.
//!
//! Two halves, one file, because they answer the same question from two directions (plan D42):
//!
//! - **The `gix` half** — every *read* and every ref *write*. Synchronous functions over `&Path`
//!   that return owned values, each called by `isolate/real.rs` under
//!   [`tokio::task::spawn_blocking`]; no `gix::Repository` ever crosses an `.await`, because it is
//!   `Send` and *not* `Sync` (`gix-0.87.1/src/types.rs:148`) and `IsolatorFuture` is `Send`.
//! - **The `git` half** — a `Cli` that spawns the binary for the five verbs `gix` 0.87.1 does not
//!   implement: `worktree add`, `worktree remove`, `merge --no-ff`, `merge --abort` and
//!   `reset --hard` (plan OQ-1, resolved by the maintainer on 2026-09-22; D23, D25, D47;
//!   `docs/ANA-2.md:1777` as amended). Nothing is ever parsed from a verb's stdout: success is
//!   exit 0 *plus* a `gix` post-condition, and failure is the last non-empty stderr line.
//!
//! `git worktree prune` is never spawned (plan D46): our entries are created `--lock`ed and prune
//! refuses locked entries, so the verb's only reachable effect is on worktrees this orchestrator
//! did not create. A stale entry is reported instead.
