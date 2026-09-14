# TOOL-6 - `crates/htui-agent/tests/launch.rs` failed once under whole-crate load and has not been reproduced (shipped, 2026-09-14)

Diagnosed as a shell-redirection race condition in the test fixtures (`an_interrupt_lets_the_child_exit_on_its_own_terms` and `a_signal_reaches_the_whole_process_group`). The shell creates the output file for `>` before the preceding command writes to it. The tests were checking `.exists()` on this file and sometimes reading 0 bytes, then passing an empty string to `kill -0` (via `alive()`), which failed the assertion.

Fixed by waiting for the file to contain non-empty content before proceeding, resolving the flake.
