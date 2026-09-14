# Plan: Fix `TOOL-4` - `tests/auth.rs` Flake

## Diagnosis
The flake is a classic TOCTOU (Time of Check to Time of Use) / race condition in `crates/htui/tests/auth.rs`, specifically in `o_opens_the_link_through_the_injected_opener`.

1. The test triggers the `o` key to open a URL.
2. The UI code calls `open_url` which spawns the `opener.sh` child process and **immediately** returns `Ok(())` (the child is reaped in the background).
3. The UI receives `AuthFrame::Opened` and renders `"link opened"`.
4. The test's `until` loop unblocks as soon as it sees `"link opened"` in the frame.
5. The test immediately asserts `assert_eq!(rig.opened(), vec![LINK.to_owned()])`.

If the OS scheduler delays the `opener.sh` process from running and writing to the `opened` file by even a few milliseconds (highly likely under CPU load or parallel test execution), `rig.opened()` returns an empty list, and the assertion fails instantly.

## Verified Claims
| Claim | Verdict | Evidence |
|---|---|---|
| `open_url` returns before the child completes | Verified | `crates/htui-agent/src/auth/browser.rs:126-129` spawns a background tokio task for `child.wait()` and immediately returns `Ok(())`. |
| The UI shows "link opened" upon `Ok(())` | Verified | `crates/htui/src/agent_worker.rs:2036` returns `StoreReply::Auth(AuthFrame::Opened)`. `agents.rs:600` matches this and sets `self.notice = Some(OPENED.to_owned())`. |
| The assertion checks the file immediately after the UI update | Verified | `crates/htui/tests/auth.rs:535` asserts `rig.opened() == vec![LINK]` directly after `rig.until` returns. |

## Execution Tasks
- [x] **Task 1** (Independent)
  - **Files:** `crates/htui/tests/auth.rs`
  - **Action:** In `o_opens_the_link_through_the_injected_opener`, wrap the `assert_eq!` check for `rig.opened()` in a polling loop (similar to `until`), giving the background child process time to execute and write to the file. We can poll `rig.opened()` against the expected `vec![LINK.to_owned()]` with a timeout based on `PATIENCE`, falling back to the strict `assert_eq!` on timeout or success to provide a clear failure message if it truly fails.
