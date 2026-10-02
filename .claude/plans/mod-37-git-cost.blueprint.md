# Blueprint: MOD-37 milestone 3 - Git cost (R-37)

**Contract**: `.claude/plans/mod-37-git-cost.plan.md` (CONFIRMED 2026-10-02). Scope unchanged.
**File**: `crates/htui-orch/src/isolate/git.rs` only (docs at close-out).
**Probed**: every snippet below was applied in a throwaway detached worktree and run on the repo
toolchain: old code 6 / 4 / 1 opens (see Task 1), new code 1 / 1 / 1; `cargo fmt --check`,
`clippy -p htui-orch --all-targets --all-features -D warnings`, `clippy -p htui-orch --lib
-D warnings`, `--lib isolate::git` (50 passed) and `--test gix_isolator` (36 passed) all clean.

## Design decisions
- Workers take `&gix::Repository`; the public path API keeps its signatures and becomes
  `open` + delegate. No caller outside `git.rs` changes; `verify_change` has nothing to check.
- Every error message is byte-identical; the order in which errors and early returns happen is
  kept exactly (see the "order" rows in each wrapper below).
- The counter lives on `open` only. `testkit` and the test helpers call `gix::open` directly, so
  fixture building never touches it; the test still resets it right before the call.

## Signatures and bodies

### Counter (above `open`, ~1384)
```rust
/// R-37's pin: how many times this thread has called [`open`]. Test builds only; the type is
/// spelled in full because a top-level `use std::cell::Cell` is unused in the non-test build.
#[cfg(test)]
thread_local! {
    static OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
```
In `open`, first statement:
```rust
pub fn open(path: &Path) -> Result<gix::Repository, IsolateError> {
    #[cfg(test)]
    OPENS.set(OPENS.get() + 1);
    gix::open(path)
        .map_err(|err| IsolateError::Git(format!("cannot open {}: {err}", path.display())))
}
```
(`LocalKey<Cell<T>>::set/get`, stable since 1.73.) Tests reset with `super::OPENS.set(0)` and read
with `super::OPENS.get()` (or add `OPENS` to the `use super::{...}` list). Thread-local, so
parallel tests never see each other's opens.

### `head` / `head_in`
```rust
pub fn head(path: &Path) -> Result<String, IsolateError> {
    head_in(&open(path)?, path)
}

/// [`head`] over an open `repo`; `path` names it in the messages only (R-37).
fn head_in(repo: &gix::Repository, path: &Path) -> Result<String, IsolateError> {
    // old body from `let head = repo.head()...` to the end, unchanged
}
```
Order: open, then the body. Identical.

### `has_commit` / `has_commit_in`
```rust
pub fn has_commit(path: &Path, hex: &str) -> Result<bool, IsolateError> {
    has_commit_in(&open(path)?, hex)
}

/// [`has_commit`] over an open `repo` (R-37).
fn has_commit_in(repo: &gix::Repository, hex: &str) -> Result<bool, IsolateError> {
    let id = parse_oid(hex)?;
    // old try_find_object / is_some_and lines, unchanged ("cannot look up {hex}: {err}")
}
```
Order: open, then parse, then lookup. Identical.

### `is_ancestor` / `ancestor_walk`
```rust
pub fn is_ancestor(path: &Path, ancestor: &str, descendant: &str) -> Result<bool, IsolateError> {
    // The old order: both hashes parse, and a commit is its own ancestor, before anything opens.
    if parse_oid(ancestor)? == parse_oid(descendant)? {
        return Ok(true);
    }
    ancestor_walk(&open(path)?, ancestor, descendant).map(|(found, _)| found)
}

fn ancestor_walk(
    repo: &gix::Repository,
    ancestor: &str,
    descendant: &str,
) -> Result<(bool, usize), IsolateError> {
    let (wanted, tip) = (parse_oid(ancestor)?, parse_oid(descendant)?);
    if wanted == tip {
        return Ok((true, 0));
    }
    if !has_commit_in(repo, ancestor)? {
        return Ok((false, 0));
    }
    // the `let repo = open(path)?;` line is deleted; the rest is unchanged
    // (`repo.rev_walk(...)` now auto-derefs the `&gix::Repository`)
}
```
Order: the old `is_ancestor` parsed both hashes and answered `true` for `wanted == tip` without
opening, so `is_ancestor(missing_path, x, x)` was `Ok(true)`. A naive `open`-first wrapper would
turn that into `Err`; the parse + short-circuit in the wrapper keeps it. `ancestor_walk` keeps its
own short-circuit for `reconcile_parent` and the D142 pins. The double parse is two hex decodes.

### `merge_of` / `merge_walk`
```rust
pub fn merge_of(path: &Path, head: &str, base: &str, after: &str)
    -> Result<Option<String>, IsolateError> {
    // The old order: every hash parses before the repository opens.
    for hex in [head, base, after] {
        parse_oid(hex)?;
    }
    merge_walk(&open(path)?, head, base, after).map(|(merge, _)| merge)
}

fn merge_walk(
    repo: &gix::Repository,
    head: &str,
    base: &str,
    after: &str,
) -> Result<(Option<String>, usize), IsolateError> {
    let (tip, base, after) = (parse_oid(head)?, parse_oid(base)?, parse_oid(after)?);
    // the `let repo = open(path)?;` line is deleted; the rest is unchanged
}
```
Order: parse x3, then open (the old `merge_walk` order). Keep the existing multi-line signature
layout; rustfmt decides.

### `reconcile_parent`
```rust
    let id = parse_oid(after)?;
    let repo = open(checkout)?;
    if !has_commit_in(&repo, after)? {
        return Ok(None);
    }
    let commit = repo
        .find_commit(id)
        // ... unchanged through the subject check and the `(first, second)` hex strings ...
    if !ancestor_walk(&repo, before, &first)?.0 {
        return Ok(None);
    }
    let head = head_in(&repo, checkout)?;
    let (merge, _) = merge_walk(&repo, &head, before, &second)?;
    Ok((merge == Some(id.to_hex().to_string())).then_some(first))
```
Order: parse(after), open, parse(after) inside `has_commit_in`, lookup, early `None` before
`find_commit`. Identical to the old `parse` then `has_commit` (open, parse, lookup). Its doc
comment's `[`merge_of`]` link stays (public to public).

Lint notes: no doc comment on a `pub fn` may link a private worker (`private_intra_doc_links =
deny`); the new links are all private-to-public or private-to-private. No `use` added at top
level.

## Regression tests (in `mod tests`, after `merge_of_never_walks_below_the_base`)

Fixture helper, beside `commit_at`. `commit_at` writes without moving a ref; this one moves `HEAD`:
```rust
    /// An empty-tree commit on `parents` with `message`, written through `HEAD`: gix's
    /// `commit_as` expects `HEAD`'s branch to sit on the first parent (`MustExistAndMatch`,
    /// `gix-0.87.1/src/repository/object.rs:423-431`) and moves it to the new commit.
    fn commit_on_head(dir: &std::path::Path, parents: &[&str], message: &str) -> String {
        let repo = gix::open(dir).expect("the repository opens");
        let who = gix::actor::SignatureRef {
            name: gix::bstr::BStr::new(b"htui test"),
            email: gix::bstr::BStr::new(b"test@localhost"),
            time: "1600009000 +0000",
        };
        let parents: Vec<gix::ObjectId> = parents
            .iter()
            .map(|hex| gix::ObjectId::from_hex(hex.as_bytes()).expect("a hex object id"))
            .collect();
        let tree = gix::ObjectId::empty_tree(gix::hash::Kind::Sha1);
        repo.commit_as(who, who, "HEAD", message, tree, parents)
            .expect("the commit is written and HEAD moves")
            .detach()
            .to_hex()
            .to_string()
    }
```
Why it works: with parents `[first, ..]` and reference `"HEAD"`, gix 0.87.1 sets `expected =
MustExistAndMatch(first)` and `deref: true`, so the branch `HEAD` names must already point at the
first parent. `repo_with_one_commit` / `commit_file` leave `HEAD` there (they use the same
`commit_as("HEAD", ..)`), so the merge's first parent must be the commit `commit_file` last made.
`testkit::who` is private to `testkit`, hence the inline signature (as `commit_at` does).

Test 1 (red, the D136 shape, the only shape that exercises all six opens):
```rust
    /// R-37 (lease blueprint §23.4): `reconcile_parent` opens the primary once per diff row, not
    /// six times. The primary moved under the step (another run's commit sits on `before`), so
    /// every check runs: the lookup, the ancestor walk past `before`, `HEAD`, the merge walk.
    #[test]
    fn reconcile_parent_opens_the_checkout_once() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let before = repo_with_one_commit(dir.path());
        let moved = commit_file(dir.path(), "g", "other run\n", "another run's work");
        let tip = commit_at(dir.path(), &[&before], 1);
        let step = htui_core::model::StepId::new();
        let merge = commit_on_head(dir.path(), &[&moved, &tip], &super::reconcile_message(step));
        assert_eq!(super::head(dir.path()).expect("HEAD reads"), merge, "HEAD is on the merge");

        super::OPENS.set(0);
        let parent = super::reconcile_parent(dir.path(), &before, &merge, step).expect("it reads");
        assert_eq!(parent.as_deref(), Some(moved.as_str()), "the merge's first parent");
        assert_eq!(super::OPENS.get(), 1, "one open of the checkout");
    }
```
Test 2 (guard pin, green on both old and new code):
```rust
    /// R-37: a commit the checkout does not hold answers `None` after that one open, before any
    /// commit is read.
    #[test]
    fn reconcile_parent_opens_once_for_a_commit_it_does_not_hold() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let before = repo_with_one_commit(dir.path());
        let other = tempfile::tempdir().expect("a temporary directory");
        empty_repo(other.path());
        let elsewhere = commit_file(other.path(), "x", "elsewhere\n", "a line of its own");
        let step = htui_core::model::StepId::new();

        super::OPENS.set(0);
        let parent =
            super::reconcile_parent(dir.path(), &before, &elsewhere, step).expect("it reads");
        assert_eq!(parent, None, "the checkout does not hold it");
        assert_eq!(super::OPENS.get(), 1, "one open, then the early None");
    }
```
`repo_with_one_commit`, `commit_file`, `empty_repo` are already imported from `super::testkit`.
Do NOT make `elsewhere` with `repo_with_one_commit`: `testkit::who` has a fixed time, so the same
file/message yields the same hash and the "other" commit would be held. `StepId::new()` is
`htui_core::model::StepId::new()` (the pattern the file's tests use at `:3243` etc.).

Measured on the old code (probe):
| Case | Old opens | New opens | Answer |
|---|---|---|---|
| Test 1, moved primary (`before` != first parent) | **6** | 1 | `Some(moved)` |
| Plan's shape, `before` == first parent | 4 | 1 | `Some(base)` |
| Test 2, not held | 1 | 1 | `None` |

So the red step: test 1 fails only on the count assertion, `left: 6, right: 1`; test 2 passes.

### D142 pins (Task 2 edit)
In `is_ancestor_never_walks_below_the_ancestor` and `merge_of_never_walks_below_the_base`, add
`let repo = super::open(dir.path()).expect("the repository opens");` after the fixture lines and
pass `&repo` in place of `dir.path()` to each `ancestor_walk` / `merge_walk` call (a binding keeps
the calls on one line under rustfmt).

## Build sequence
1. **Commit 1 (red)**: counter + `#[cfg(test)]` line in `open`, `commit_on_head`, tests 1 and 2.
   Run `cargo test -p htui-orch --all-features --lib isolate::git::tests::reconcile_parent`:
   test 1 fails `6 != 1`, test 2 passes. Commit as the red test.
2. **Commit 2 (fix)**: the `_in` workers, wrappers, `reconcile_parent`, D142 pin call sites. Run
   the same filter (both green), then `cargo test -p htui-orch --all-features --lib isolate::git`
   and `cargo test -p htui-orch --all-features --test gix_isolator`.
3. **Commit 3 (docs)**: plan Task 3 unchanged.

## Validation (the plan's, with the one correction below)
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy -p htui-orch --lib -- -D warnings          # the non-test build: no stray cfg(test) import
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1   # grep SIGABRT
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Deviations
1. **The regression shape.** The plan's test (`before` = `base` = the merge's first parent) does
   not hit 6 opens: `ancestor_walk` short-circuits on `wanted == tip` before `has_commit`/`open`,
   so the old code measures **4**. Test 1 uses D136's moved-primary shape (`before`, another
   commit `moved` on it, `tip` on `before`, merge `[moved, tip]`), which measures **6** and
   answers `Some(moved)`. The plan's "count failing with 6" holds only for this shape.
2. **`--lib` needs `--all-features`.** `cargo test -p htui-orch --lib ...` without features does
   not compile (`crate::fake`, `engine::fake_parts` are behind `test-support`). Tasks 1-2's
   validate commands take `--all-features` (the plan's final Validation block already does).
3. **Wrapper order.** `is_ancestor` and `merge_of` parse their hashes (and `is_ancestor`
   short-circuits `wanted == tip`) before `open`, so a bad path no longer outranks a bad hash and
   `is_ancestor(_, x, x)` still answers `true` without opening. The plan said "call `open(path)`
   once and delegate"; that literal reading would change behaviour.
4. **D142 pins** take `&repo` from a `super::open(..).expect(..)` binding, not `&open(dir.path())`
   (`open` returns `Result`, and `open` is not in the tests' import list).

## Hazards
- Test 2 is not red on the old code (1 open either way); it pins that the early `None` stays
  before `find_commit`. State that in the red commit message so the reviewer does not expect two
  failures.
- `cargo fmt` reflows anything longer than 100 columns; run it before each commit.
- No blockers.
