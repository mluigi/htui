# Plan: TOOL-6 Rare test flake in tests/launch.rs

## Tasks

### 1. Fix TOCTOU races in wait loops
- In `crates/htui-agent/tests/launch.rs`, the `a_signal_reaches_the_whole_process_group` test checks `!pidfile.exists()` to wait for a background process to start. However, shell redirection `> {}` creates the file before the background command starts writing to it. The test might read an empty file and panic on `assert!(alive(&helper).await)` because `alive("")` fails.
- Change the loop to try reading the file and break only when the content is non-empty. Use `std::fs::read_to_string(&pidfile)` inside the loop, trim it, and ensure it's not empty.

### 2. Update `an_interrupt_lets_the_child_exit_on_its_own_terms` similarly
- The test `an_interrupt_lets_the_child_exit_on_its_own_terms` waits with `!ready.exists()`. The `trap` is executed before the redirection `> {}` is evaluated by the shell so the file is created *after* the trap is installed. This is technically race-free but replacing it with reading a non-empty string or at least not relying just on `exists()` is safer and matches the fix above. Wait, actually, `printf ready > {}` means the file is created, then "ready" is written to it. Just to be robust against empty file reads (if we ever change the assertion), change this to read the file and check for "ready".

## Verification
- Run `cargo test -p htui-agent --features test-support --no-fail-fast --test launch` multiple times (e.g., in a bash loop) to ensure no regressions occur.

### Verified Claims
| Claim | Verdict | Evidence |
|---|---|---|
| The tests `a_signal_reaches_the_whole_process_group` uses shell redirection `>` | Verified | `crates/htui-agent/tests/launch.rs:857` |
| The `alive()` helper executes `kill -0 <pid>` | Verified | `crates/htui-agent/tests/launch.rs:790` |
| `std::fs::read_to_string` on an empty file returns an empty string | Verified | Standard library behavior |
