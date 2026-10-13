# MOD-95 - auto_queue overshoot test: the batch closes as drained (done, 2026-10-13)

**Requirements:** `R-ORCH-6`, `R-AGT-7`.
**Origin:** found at MOD-92's gate (`docs/decisions/mod/mod-92.md`, Follow-ups); also seen at MOD-91's.
**Artifacts:** plan `.claude/plans/mod-95-overshoot-test.plan.md` (decision table, verified-claims
table). Routed as **plan** (C3 only), no ultracode. No blueprint: one test in one file.

## The problem

`htui-worker` `tests/auto_queue.rs` `a_batch_overshoots_its_cap_by_at_most_one_attempt_pg` failed
at `:2210` on every tree since the MOD-12 M3 and CLEAN-9 merges, `main` included:
`closed_reason` read `Some("drained")` where the test pinned the batch open ("the batch is still
open after the overshoot").

## The decision: the code is right, the test was stale

- **The close is the maintainer's MOD-12 review L4** (2026-10-07, `docs/decisions/mod/mod-12.md`
  "M1 review L4: a stalled batch closes"). With a free slot and nothing admissible, `admit` closes
  the batch `drained` once no run of its own is live, so a later `Q` never starts spending; `P`
  does. It landed in `03a60f46` (M3 T2, the only commit that adds `Drain::Stalled`). The Mem test
  `a_batch_whose_every_entry_is_held_closes` pins it for cap-held entries.
- **CLEAN-9's pin `d0be442f` predates it on its own branch.** It was written two hours later but on
  a branch without `03a60f46` (`git merge-base --is-ancestor` says no), in the pre-L4 world where a
  capped batch stayed open. The two met only on merge. The HANDOFF line credited the close to
  `b9604b57` (MOD-12 M1 review M1, the keyed drain). The stall close itself is `03a60f46`.
- **The arm.** The refused retry (`refuse_no_candidate`, "batch cap reached (1200 of 1000
  micros)") moves A1 `in_progress -> blocked`. Its entry stays, but nothing is ready, so the batch
  closes through `admit`'s `order.is_empty()` stall. The test pins only the store-visible outcome.
- Keeping a cap-stalled batch open (fixing the code instead) was rejected: it reverses L4 and
  breaks the Mem pin.

## What changed

- The test now asserts `closed_reason = 'drained'` and `PgStore::open_batch_of(BOX) == None` after
  the overshoot. It keeps the later entry's sweep (no run admitted, spend unchanged), now as "no
  batch is open, so a later entry waits for a resume". Doc comment updated. `eb23a817`.
- Non-vacuity: with `drain` stubbed to return early, the new assertion fails
  (`left: None, right: Some("drained")`). Reverted.
- Stale docs: `docs/ANA-2.md`'s MOD-12 M2 as-built paragraph said a capped batch "stays open"; it
  now states the L4 close. `docs/decisions/clean/clean-9.md` notes its pin is superseded.
  `cb9f5d07`.
- No product code changed. Pg coverage of "the cap declines an entry in an open batch" is gone, as
  that state exists under L4 only with a live batch run or no free slot. The cap-held close is
  pinned on Mem, and the engine's cap refusal by `conformance.rs` `a_batch_at_its_cap_starts_no_attempt`.

## Review and gate

**Review.** `rust-reviewer`: no CRITICAL/HIGH/MEDIUM, 3 LOW (the comment named close arms that
cannot fire here; ANA-2 stale paragraph; CLEAN-9 write-up), all applied in `cb9f5d07`.
**Gate** (sandbox, `HTUI_TEST_DATABASE_URL` set): `cargo fmt --all -- --check`; `cargo clippy
--workspace --all-features --all-targets -- -D warnings`; `cargo clippy --workspace -- -D warnings`;
`cargo test -p htui-worker --all-features --no-fail-fast -- --test-threads=1`: 100 passed, 0 failed
(`auto_queue` 32 of 32, its `_pg` cases ran against Postgres). Workspace-wide tests not re-run: the
change is one `htui-worker` test.

## Follow-ups

- Owed on the host: the Postgres gate on the merged tree.
