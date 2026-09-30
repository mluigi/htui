# CLEAN-7 - Leftovers of the removed offline chat buffer (done, 2026-09-30)

**Requirements:** `R-NF-3`.
**Origin:** MOD-39, 2026-09-29 (`docs/decisions/mod/mod-39.md`, which fixed the stale comments and
left the code and UI text here).

**What was done.** MOD-25 refused offline chats and CLEAN-2 deleted the buffer machinery; what they
left behind was removed, with no behaviour change beyond one line of UI copy. Plan
`.claude/plans/clean-7.plan.md` (D1-D5, fact-checked), blueprint `.claude/plans/clean-7.blueprint.md`.

- **Rebuild confirmation (D1).** Settings › Connection's rebuild question no longer says "the
  pending/ buffer" survives. `CacheStore::rebuild` clears the mirrored tables, `cache_cursor` and
  `last_full_refresh_at` and nothing else, and there is no `pending/` any more. Both copy pins (the
  unit test in `ui/tabs/settings/connection.rs` and `tests/connection.rs`) now assert the phrase is
  **absent**; the `connection__confirm` snapshot re-wraps the question from three lines to two, and
  the rows area above it gains the freed line. `cca06e0`.
- **`RefreshSettings::this_user` (D4)** removed: it carried `run.started_by` for the buffer's upload,
  and the refresh pass never read it since. `this_box` stays. `00e461e`, plus the `Started::settings`
  doc the closing scan caught, `a34ada4`.
- **Quota latch (D2).** `quota_latch_for` returns a `QuotaLatch` rather than an always-`Some`
  `Option`, `ChatArgs.quota_latch` follows, and `run_chat` latches unconditionally. Every chat path
  latched before and still does: `Backend::writer` answers `None` offline, so a chat that gets that
  far has a store with an `agent_box` table. `Recorder`'s own `Option<QuotaLatch>` stays, for the
  recorders that have none. `project_caps_for` and `quota_latch_for` lose the `_writer` they never
  read. `d16215d`.
- **`writer_label` (D3)** left `StoreReply::ChatAccepted` and `ChatArgs`; the Chat tab had matched it
  as `_`. `StoreReply` is not serialized, so no wire format moved. `Writer::label()` stays for the
  task `Debug` impls, and its doc now says so; `writer.rs`'s claim that "the `Buffered` arm is where
  the refusal lives" now names `Backend::writer` answering `None`. The two `#[ignore]`d live chat
  tests, which printed the label, were missed by the item text and caught by the plan fact-check.
  `42dd991`.
- **Test wording (D5).** Names and messages that said upload, uploader or buffered writer now say
  what they exercise: `a_chat_run_mints_both_rows_running_and_finish_closes_them` (`mem.rs`), the
  two `…_on_an_offline_backend` refusals and their messages (`agent_worker.rs`), the recorder's
  "fresh sum over the same persisted rows", `synthetic_event` (`tests/cache.rs`), and
  `chat_offline.rs`'s "nothing on screen says buffered". Bodies and assertions unchanged. `55e4032`.

**Review.** `rust-reviewer` approved with no CRITICAL/HIGH/MEDIUM findings and four LOW wording
findings, all applied in `dd56355`: the latch is always present because no writer exists off the
server (a memory chat latches too), not because "a chat starts only online"; the `mem.rs` test name
covers `finish_chat_run` closing both rows; `chat_offline.rs`'s message matches its whole-frame
assertion; and `this_user` outlived the buffer until CLEAN-7 rather than leaving with it.

**Verification.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features
-- -D warnings`, and `cargo test --workspace --all-features -- --test-threads=1` in the `hr/CLEAN-7`
sandbox: 2653 passed, 0 failed, 26 ignored. The real Postgres gate on the merged tree runs on the
host at collect.

**Left alone, deliberately.** Comments that describe the removed buffer as history (`backend.rs`,
`writer.rs` module doc, `run.rs:283`, `agent_worker.rs`'s chat-start note, the `chat_offline.rs`
module doc) are accurate and stay; the recorder's other "buffered" wording is its live flush buffer.
