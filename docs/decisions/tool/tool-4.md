# TOOL-4 - `crates/htui/tests/auth.rs` flakes as a whole binary, and the assertion is not yet captured (concluded, 2026-09-14)

## Context
The test `o_opens_the_link_through_the_injected_opener` in `crates/htui/tests/auth.rs` experienced an intermittent failure (1 in 43 runs under load). The flake caused the whole binary to fail instantly (in 0.15s), but the failure output was not captured in earlier CI/reproduction attempts. 

## Diagnosis
The failure was a TOCTOU (Time of Check to Time of Use) race condition. The UI action `open_url` spawns a background `opener.sh` process and returns immediately. The UI updates to "link opened", which unblocks the test's `until` polling loop. Immediately after, the test asserted `assert_eq!(rig.opened(), vec![LINK.to_owned()])`. 

Under load, the background `opener.sh` script had not yet finished writing to the `opened` file by the time the test read it, resulting in an empty vector and an immediate assertion failure.

## Resolution
The test was updated to poll `rig.opened()` for up to `PATIENCE` (60 seconds) with `TICK` (10ms) intervals, allowing the background child process time to execute and write to the file. The strict `assert_eq!` was maintained as a fallback to ensure clear diffs on a true failure.

Commit: 8c95bbb (plus the current commit wrapping up the fix).
