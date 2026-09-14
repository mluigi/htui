# Plan: CLEAN-1 (cargo doc for htui-agent)

## Objective
Fix `cargo doc -p htui-agent` failures caused by `rustdoc::private-intra-doc-links` and `rustdoc::broken-intra-doc-links` under `#![deny(warnings)]` or explicitly passed lint flags.

## Context
Running `cargo doc -p htui-agent` currently fails due to private-item intra-doc links and ambiguous links (e.g., a name referring to both a module and a function).

## Tasks
1. **Fix ambiguous intra-doc links**:
   - `crates/htui-agent/src/lib.rs:22`: Change `[`install`]` to `[`install()`]` or `[`mod@install`]`.
   - `crates/htui-agent/src/install/mod.rs:3`: Change `[`plan`]` to `[`plan()`]` or `[`mod@plan`]`.

2. **Fix private intra-doc links**:
   - `crates/htui-agent/src/acp/mod.rs:272`: Link to `crate::launch::launch_from`. Fix by removing the link or exposing the item in the docs. (I will change to code formatting `` `launch_from` `` or remove `crate::`).
   - `crates/htui-agent/src/acp/mod.rs:753`: Link to `answer_from_connection`. Fix with backticks.
   - `crates/htui-agent/src/cli/mod.rs:315`: Link to `crate::launch::launch_from`. Fix with backticks.
   - `crates/htui-agent/src/cli/mod.rs:683`: Link to `ChildGuard`. Fix with backticks.
   - `crates/htui-agent/src/install/archive.rs:75`: Link to `descend`. Fix with backticks.
   - `crates/htui-agent/src/install/http.rs:11`: Links to `HeadInfo`, `RegistryFetch`, `HttpError`. Fix with backticks.
   - `crates/htui-agent/src/launch.rs:829`: Link to `ChildGuard`. Fix with backticks.
   - `crates/htui-agent/src/probe.rs:798`: Link to `version_key`. Fix with backticks.

3. **Verify**:
   - Run `cargo doc -p htui-agent --document-private-items` and `cargo doc -p htui-agent` to ensure it passes.

## Verified Claims
| Claim | Verdict | Evidence |
|---|---|---|
| Ambiguous links are at lib.rs and install/mod.rs | Confirmed | Compiler output lists these exact lines |
| Private item links cause failure | Confirmed | Compiler output explicitly states "links to private item" |
| Tasks are independent | Confirmed | Files are distinct (`lib.rs`, `acp/mod.rs`, `cli/mod.rs`, `install/archive.rs`, `install/http.rs`, `launch.rs`, `probe.rs`) |

## File modifications
- `crates/htui-agent/src/lib.rs`
- `crates/htui-agent/src/install/mod.rs`
- `crates/htui-agent/src/acp/mod.rs`
- `crates/htui-agent/src/cli/mod.rs`
- `crates/htui-agent/src/install/archive.rs`
- `crates/htui-agent/src/install/http.rs`
- `crates/htui-agent/src/launch.rs`
- `crates/htui-agent/src/probe.rs`
