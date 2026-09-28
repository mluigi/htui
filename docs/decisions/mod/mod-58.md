# MOD-58 - Two claim-time test gaps (done, 2026-09-28)

**Requirement:** `R-ORCH-10` (the claim-time half of the capability refusal).
**Origin:** MOD-7 milestone 3 review. The rules were implemented and documented; no test pinned
them.
**Design authority:** MOD-7 milestone 3 plan **D80**, **D81** and blueprint **D94**
(`.claude/plans/mod-7-capability-refusal.plan.md`, `.blueprint.md:958`). This item adds no design
of its own.
**Artifacts:** plan and blueprint,
[`.claude/plans/mod-58-claim-time-test-gaps.plan.md`](../../../.claude/plans/mod-58-claim-time-test-gaps.plan.md)
and [`.blueprint.md`](../../../.claude/plans/mod-58-claim-time-test-gaps.blueprint.md). No PRD —
routed as a plan on 2026-09-28, 0 of C1–C4 fired.
**Commits:** `c9e24c9`..`726dea8`, nine commits, on `mod-58`.

## What shipped

Four tests, no production behaviour change. The branch is 1 089 insertions and 3 deletions across
four files; the deletions are three comment lines and one replaced assertion, all inside tests or
comments.

**(a) `NotClaimable` outranks `MissingTags`, and writes nothing.** Both stores decide claimability
before the tag check (`mem.rs:3602` before `:3610`; `pg/write.rs:3063` before `:3071`), so a run
whose `target_box_id` names another box is refused even when its item needs a tag the claiming box
lacks. The shipped conformance case pinned only the *failed-run* half; the target-box half was
unpinned. `a_missing_tags_run_aimed_at_another_box_is_not_claimable` now covers it in both stores,
and asserts the refusal wrote nothing three ways: the run row is byte-equal to what `create_run`
returned, the item is still `queued`, and the run holds no lease.

**(b) D94 — a run with no item is never refused for tags.** `MemStore` asks `items.get`, not
`require_item`, mirroring the Postgres join that finds no row.
`a_run_with_no_item_is_never_refused_for_tags` covers it in both stores. The two stores are
deliberately asymmetric: `MemStore` pins both of D94's arms (`item_id` NULL, and `item_id` naming a
row that is gone), Postgres pins only the NULL arm, because `run.item_id` is
`ON DELETE CASCADE` (`0001_init.sql:450`) and a deleted item takes its run, so the second shape
cannot exist there.

## Three decisions worth keeping

**No shared `WriteStore` method.** The shared conformance suite is trait-generic and cannot mint a
second `box` row — `traits.rs` has no box-creating method, and claiming with an unknown `box_id`
answers `NotFound`, not `NotClaimable`, so the target-box rule needs a box that really exists. Both
cases therefore live per store, each planting its own second box: a raw untyped
`sqlx::query` insert in `pg_criteria.rs` (the form that keeps `.sqlx/` at 268), and a clone of the
fixture's row inserted into `State.boxes` through the private `MemStore::write`. The cost is that
the rule is pinned in two files that could drift; the review checked and they do not.

**TDD inverts here.** Both tasks *are* tests, and the behaviour was already believed correct, so
there was no red-then-green. Each pin instead carries a **mutation check**: break the rule, confirm
the case fails, revert, and report the failure line. Four mutations, all biting. The plan's first
version of mutation 1 was not a mutation at all — hoisting the claimability predicate into a binding
and returning after the `let missing` computation leaves the `MissingTags` write block after the
return, so nothing is written and the case still passes. It has to move past the whole block.

**The `MemStore` D94 mutation proved less than first reported.** The realistic
`and_then(|item| self.require_item(&item).ok())` swap is caught by arm B only: with `item_id: None`,
`and_then` short-circuits before the closure runs. Arm A needs the `map`-shaped demand. The case as a
whole does bite, and the Postgres side's mutation was verified specific by running case 1 as a
control. The claim in the plan is narrowed accordingly.

## What the review caught

The configured reviewer's verdict was approve-with-fixes, and its two MEDIUM findings were both
**false claims in comments inherited from the plan**:

- the D94 doc credited a chat run as a run with no item. No chat run reaches that check —
  `start_chat_run` inserts at `running`, so a chat run is `NotClaimable` at the *status* half. The
  plan's own verified claim said so; the comment contradicted the plan that produced it. Neither
  D94 arm is in fact reachable through the public API (`NewRun.item_id` is non-optional, there is no
  `delete_item`), so the case is a **defensive** pin and now says so.
- the same false chat-run claim sat in a **production comment** inside `State::claim_run`
  (`mem.rs:3606-3609`), directly above the code the tests pin. Corrected under maintainer
  approval; it is a comment, and no behaviour changed.

Three robustness gaps closed: the "wrote nothing" pins did not cover `run.lease_owner`, which is
observable through neither store's `Run` struct (a `refresh_lease` assertion now covers it); the
Postgres D94 insert leaned on the `repo_scope` column default for its soundness argument (now
written explicitly); and both D94 cases were silently coupled to the fixture's slot arithmetic (now
stated in the docs, and asserted as a precondition on the Postgres side).

One cross-store asymmetry is recorded, not fixed: `timestamptz` is microsecond-resolution, so the
Postgres lease comes back truncated from the nanoseconds it was handed, while `MemStore` keeps every
digit. The Postgres D94 assertion truncates with `TIMESTAMPTZ_DIGITS`; the `MemStore` twin needs no
such truncation.

## Pins

No count pin moved. Store `CASES` 77, `.sqlx` 268, migrations `0001`..`0007` with `0008` next, and
the `crates/htui` snapshot count are all unchanged — this item added no conformance case, no `query!`
statement, and no schema.

## Gates

`cargo fmt --all -- --check` and `cargo clippy --workspace --all-features --all-targets
-- -D warnings` clean on the real tree. `cargo test -p htui-core --all-features` 323 passed
(321 before this item). `cargo test -p htui-store --test pg_criteria --all-features` 37 passed
(35 before) against the dev Postgres on 5439, `--test-threads=1`, and the full workspace suite with
the same flag. `~/.pgpass` now carries the dev database's credential, so the gate runs a
password-free DSN and no secret reaches the command line.

Both new Postgres cases create and drop their own database per case, like every other case in
`pg_criteria.rs`, so they are independent of the `USERNAME=htui-ci` prefix the other Postgres
suites use.
