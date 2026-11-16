# MOD-132 M1 R1a — review fix round: htui-agent target channel

Fix round over the M1 change set (`a335fdb2..3a5526c9`), from the `rust-reviewer` pass of 2026-11-16. **Confirm each
finding against the code before acting**; a finding that does not hold goes in your report's `rejected_findings`
with the reason. Tests first where a behaviour changes.

Context: `crates/htui-agent/src/target_channel.rs` holds the argv-only `TargetChannel` seam (plan P1, P2) and the
test-only `StandInChannel` (P3). MOD-44 (`docker exec -i`) and MOD-131 (`ssh -T`) will implement the trait later.

## Findings

- **L3 — write the remote-channel obligations into the trait docs** (`target_channel.rs` ~103-139). A real channel must:
  - map its *transport's* own failures to `ChannelError::Unreachable` or `Spawn`, never to a command exit code
    (`ssh` exits 255 for its own failures; the docker CLI exits non-zero for a missing or stopped container). Otherwise
    a caller reads `Some(1)` with empty stdout as a git answer;
  - on a deadline, kill the remote process too, not only the local client. If it cannot, it must wrap the remote side
    (e.g. `timeout`/`setsid`). Soften `TimedOut`'s "everything it started was killed" to say what the stand-in
    guarantees and what a remote channel owes;
  - resolve paths: the argv carries absolute target paths for real channels; a relative path is only meaningful for the
    stand-in (its home). Say so;
  - `docker exec -i <ctr> argv` takes no `--` after the container name (docker stops parsing its flags there, so a `--`
    would become the command); ssh joins argv in the remote login shell and so needs POSIX quoting, assuming a POSIX
    shell. Document both as notes for the implementers. No code for them here.
- **L7 — make the stand-in's git environment hermetic** (`target_channel.rs` ~272-291). Also remove
  `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_COUNT`, `GIT_CONFIG_PARAMETERS`, `GIT_CONFIG_NOSYSTEM`, any
  `GIT_CONFIG_KEY_*`/`GIT_CONFIG_VALUE_*`, and `XDG_CONFIG_HOME`. Set `XDG_CONFIG_HOME` to `<home>/.config` and
  `GIT_CONFIG_NOSYSTEM=1`, so "target" git reads only the stand-in home. Extend the environment test to check them.

## Files you own

`crates/htui-agent/src/target_channel.rs`, `crates/htui-agent/tests/it/target_channel.rs`.

## Check

`cargo test -p htui-agent --all-features --no-fail-fast --test it target_channel::`. Then
`cargo clippy -p htui-agent --all-features --all-targets -- -D warnings` and `cargo clippy -p htui-agent -- -D warnings`.
Then `RUSTDOCFLAGS='-D warnings' cargo doc -p htui-agent --no-deps --all-features`, and `cargo fmt -p htui-agent --check`.
