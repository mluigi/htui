# Plan: MOD-37 milestone 3 - Git cost

**Source PRD**: `.claude/prds/mod-37-orchestrator-hardening.prd.md`
**Selected Milestone**: 3 - Git cost (R-37)
**Complexity**: Small
**Routing**: PRD path, M3 planned on its own (maintainer-confirmed 2026-10-02); ultracode not needed.

## Summary
`git::reconcile_parent` opens the primary checkout six times for every diff row of a `worktree` or
`copy` step. It opens once in `has_commit`, once in its own `open`, twice in `is_ancestor` (the
nested `has_commit` and `ancestor_walk`), once in `head` and once in `merge_walk`. The fix opens the
repository once in `reconcile_parent` and passes `&gix::Repository` to private variants that take a
repository instead of a path. The public path-taking functions keep their signatures and become thin
wrappers, so no caller changes. A `cfg(test)` thread-local open counter gives R-37 a regression test
that fails on the old code (6 opens) and passes on the new (1).

`merge_of`'s first-parent walk from `HEAD` down to `before` is the D136 semantics, not waste: a later
run's merge may sit on top of this step's merge. D142 already bounds it by hiding `before`. It stays
as it is, and the close-out note says why.

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| `reconcile_parent` opens the checkout six times | true | `isolate/git.rs:1594` `has_commit` → `open` (`:1424`); `:1597` `open`; `:1609` `is_ancestor` → `ancestor_walk` → `has_commit` (`:1478`) + `open` (`:1481`); `:1612` `head` → `open` (`:1400`); `:1613` `merge_of` → `merge_walk` → `open` (`:1532`). That is 1+1+2+1+1 = 6, matching lease blueprint §23.4's list |
| `reconcile_parent` has one production caller | true | `isolate/real.rs:1369` (`range_of`, inside `blocking`); repo-wide search finds no other call |
| `range_of` probes a `copy` tree with `has_commit` separately | true | `isolate/real.rs:1343`. That is a different repository, outside R-37 |
| `has_commit`, `head`, `is_ancestor`, `merge_of` have other callers that take a path | true | `real.rs:429, 810, 1045, 1079, 1139, 1165, 1174, 1178, 1184`; `copy.rs:913`; `git.rs:496, 678, 762`; tests. So the path API must stay |
| `ancestor_walk` / `merge_walk` are private and used by the D142 pins | true | `git.rs:1469, 1525`; tests `is_ancestor_never_walks_below_the_ancestor` (`:2374`), `merge_of_never_walks_below_the_base` (`:2399`) import them (`:2293`) |
| `isolate::git` is a `pub mod` | true | `isolate.rs:22`; no public signature changes, so `verify_change` has nothing to check |
| The `gix` calls used take `&self` | true | gix 0.87.1: `head` `repository/reference.rs:187`, `try_find_object` `object.rs:222`, `find_commit` `object.rs:83`, `rev_walk` `revision.rs:174` |
| `head`'s error text names the path | true | `git.rs:1401-1412` (`path.display()`, `unborn_head`). So `head_in` keeps a `path: &Path` for the messages and the text stays byte-identical |
| A `cfg(test)` thread-local counter compiles and works on the repo's toolchain (edition 2024) | true, **amended** | rustc probe: the test passes. But a top-level `use std::cell::Cell;` is an unused import in the non-test build and fails `-D warnings`. The counter must name `std::cell::Cell` in full inside the `cfg(test)` item. `unused_qualifications` (`Cargo.toml:178`) does not fire, because nothing imports `Cell` |
| No existing counter pattern for git opens | true | no `thread_local!` in `htui-orch/src`; the nearest pattern is D142's walks returning their own counts (`git.rs:1468, 1524`) |
| The test helper `commit_at` does not move `HEAD` | true | `git.rs:2350` uses `new_commit_as` (no ref update). The new test needs `HEAD` on the merge, so it uses `commit_as("HEAD", …)` or a ref edit |
| Task independence | n/a | one serial lane; every task touches `crates/htui-orch/src/isolate/git.rs` only (plus docs at close-out) |

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming | `isolate/git.rs:1464-1469` | a public wrapper plus a private worker (`is_ancestor` → `ancestor_walk`); the new workers are `*_in(repo: &gix::Repository, …)` as §23.4 names them |
| Errors | `isolate/git.rs:1389-1412` | `IsolateError::Git(format!(…))` with the same text; the move changes no message |
| Tests | `isolate/git.rs:2367-2414` | D142's pin: a unit test in `git.rs`'s `mod tests` asserting a count the old code exceeded, with a doc comment naming the plan item |
| Test fixtures | `isolate/git.rs:2337-2365` | `commit_at` / `long_history` over `tempfile` + `gix`; no `git` binary needed |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui-orch/src/isolate/git.rs` | UPDATE | the open counter, the `*_in` variants, `reconcile_parent` opening once, and the regression test |
| `HANDOFF.md`, PRD, this plan | UPDATE | close-out: R-37 struck, M3 row complete |

## Tasks
### Task 1: Regression test, red
- **Action**: In `git.rs`, add the counter: `#[cfg(test)] thread_local! { static OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }`, incremented first thing in `open` under `#[cfg(test)]`. Add `reconcile_parent_opens_the_checkout_once`. It builds a primary with `base`, a side `tip` on `base`, and a merge `[base, tip]` whose subject is `reconcile_message(step)`, with `HEAD` on the merge. It resets the counter, calls `reconcile_parent(dir, base, merge, step)`, and asserts the answer is `Some(base)` and the count is `1`. Add a second case for a commit the checkout does not hold: the answer is `None` and the count is `1`.
- **Mirror**: D142's pin doc comment and its count assertion (`git.rs:2367-2393`).
- **Validate**: `cargo test -p htui-orch --lib isolate::git::tests::reconcile_parent` shows the answer assertion passing and the count failing with `6`. Commit as the red test.

### Task 2: Open once
- **Action**: Add private `has_commit_in(repo, hex)`, `head_in(repo, path)` (the path is for the messages only) and `ancestor_walk(repo, …)` / `merge_walk(repo, …)`, which take `&gix::Repository` instead of a path. `ancestor_walk` uses `has_commit_in`. The public `has_commit`, `head`, `is_ancestor` and `merge_of` call `open(path)` once and delegate. In `reconcile_parent`, call `parse_oid(after)` first (as now), then `open(checkout)`, then the `_in` variants. The early `None` for a commit the checkout does not hold stays before the `find_commit`. Change the two D142 tests to pass `&open(dir.path())`.
- **Mirror**: the existing wrapper/worker split; keep every error message byte-identical.
- **Validate**: Task 1's test is green; `cargo test -p htui-orch --lib isolate::git`; `cargo test -p htui-orch --all-features --test gix_isolator` (the D141/D146 reconcile tests at `:2918, :3011, :3090` are the behaviour pins).

### Task 3: Close-out docs
- **Action**: Strike R-37 in HANDOFF's MOD-37 line ("closed by MOD-37 phase 3") with a phase 3 note. Set the PRD M3 row to `complete`. Add an "As built" section and a DONE status to this plan.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Out of scope (noted, not changed)
- `range_of`'s `copy` probe (`real.rs:1343`) opens another repository; it stays one open.
- `RealIsolator`'s reconcile path (`real.rs:1165-1184`) opens the checkout through four separate
  `blocking` hops (`head`, `merge_of`, `is_ancestor` ×2) under the admin lock. That is the same
  kind of cost, but not R-37's (lease blueprint §23.4 names `reconcile_parent` only). Folding it
  would change an async sequence that holds a lock, so it is left for a separate item if wanted.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy -p htui-orch --lib -- -D warnings          # the non-test build: no stray cfg(test) import
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1   # grep SIGABRT
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| A thread-local counter misses opens on another thread | L | `reconcile_parent` is synchronous; the unit test calls it directly, not through `blocking` |
| An error message changes in the move | L | `head_in` keeps the path parameter; the existing unborn-HEAD and reconcile tests pin the text |
| The `cfg(test)` line in `open` reads as production noise | L | one line with a doc comment naming R-37; nothing ships in release builds |
| `htui-orch` stack headroom | L | no new futures; still gate with `--no-fail-fast` and grep SIGABRT |

## Acceptance
- [ ] Task 1's test was red (6 opens) before Task 2
- [ ] Validation passes on the real tree, `--test-threads=1`
- [ ] Reviewer gate run over the full change set
- [ ] HANDOFF R-37 closed; PRD M3 complete

---
*Status: CONFIRMED (2026-10-02) - implementation in progress.*
