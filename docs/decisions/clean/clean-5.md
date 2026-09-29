# CLEAN-5 - `a_merge_dropped_mid_hook_still_lands_and_leaves_no_merge_head` flakes under load (done, 2026-09-28)

**What was done.** Test-only fix in `crates/htui-orch/tests/gix_isolator.rs`. The fixed
`tokio::time::sleep(Duration::from_secs(3))` that stood between dropping the `merge_no_ff` future
and asserting the merge landed is now a poll, exactly as the HANDOFF item proposed.

**The flake.** The test drops a `merge_no_ff` future after 300 ms while the primary's
`pre-merge-commit` hook runs `sleep 2`, then asserted the merge had landed with parents
`[before, after]`, no `index.lock` and no `MERGE_HEAD`. The dropped `git merge` keeps running
after its future is dropped, so a fixed 3 s wait is a guess at how long a 2 s hook plus a merge
takes. Under load the merge had not finished by then, `git log -1 --format=%P HEAD` read the
pre-merge parent, and the case failed. It failed 1 of 3 runs on its own at MOD-7 milestone 4's
gate (2026-09-26) and was not caused by MOD-7.

**The fix.** Poll every 100 ms until `%P` reads `before after` and neither `index.lock` nor
`MERGE_HEAD` exists, then run the existing assertions unchanged. The poll carries its own
deadline of 30 s and reports the observed parents plus both lock states on expiry, so a real hang
stays loud instead of being mistaken for a slow merge — the fixed sleep had no such failure mode,
since a hung merge simply timed the test out with no diagnosis. The 30 s ceiling is also the test's
worst case rather than a guess: the hook alone is 2 s, so a merge that has not landed in half a
minute is not going to.

**Verification.**
- The test alone, `--test-threads=1`: passes in 2.07 s (was a floor of 3 s by construction).
- The same test under a 12-way CPU load on this 12-core box: passes in 2.13 s. This is the
  condition that produced the flake.
- The whole `gix_isolator` file, `--test-threads=1`: 34 passed, 0 failed, 8.38 s.
- `cargo fmt --check -p htui-orch` and `cargo clippy -p htui-orch --features test-support --tests`:
  clean.

**Scope.** One test body, no production code, no other test touched. `R-ORCH-8` is unaffected —
it was never the requirement at fault, only the wait in front of it.
