# CLEAN-7: Leftovers of the removed offline chat buffer

Routed as **plan** (CLEAN default; C1–C3 ✗, C4 borderline), ultracode not needed — maintainer
accepted 2026-09-30. Sandbox run on `hr/CLEAN-7`.

## Goal

Remove what the deleted offline chat buffer left behind (MOD-25 refused offline chats, MOD-39 fixed
the comments): one false line of UI copy, two dead fields, two dead parameters, an `Option` that
is always `Some`, and test names and messages that still talk about uploading or buffering. `R-NF-3`.
**No behaviour change**, apart from the rebuild confirmation no longer naming a `pending/` buffer that
`CacheStore::rebuild` never had anything to do with.

## Decisions

- **D1 — rebuild copy.** Drop `, the pending/ buffer` from the Survives list. `CacheStore::rebuild`
  (`htui-store/src/cache/mod.rs:177`) deletes the mirrored tables, `cache_cursor` and
  `last_full_refresh_at` and does nothing else, and no `pending/` directory exists anymore. The two
  pins that name `pending/` gain a **negative** assertion, so the claim cannot come back quietly.
- **D2 — `quota_latch_for` returns `QuotaLatch`, not `Option<QuotaLatch>`.** `ChatArgs.quota_latch`
  becomes `QuotaLatch`, and `run_chat` calls `recorder.with_quota_latch(latch)` unconditionally.
  `Recorder`'s own `Option<QuotaLatch>` stays: it has real `None` cases (tests, and a backend that
  refused the latch once, `htui-agent/src/record.rs:423-425`). `ChatArgs`'s `Debug` drops
  `quota_latch` because it is now always true (it printed `is_some()`). Keep the helper rather than
  inlining it, since its doc carries the `R-AGT-5` note.
- **D3 — `writer_label` leaves the protocol completely:** removed from `StoreReply::ChatAccepted`, from
  `ChatArgs`, and from their constructions and destructurings. `Writer::label()` stays, because five
  `Debug` impls in `agent_worker.rs` use it.
- **D4 — `RefreshSettings::this_user` is removed** (field, `Default`, `connect::refresh_settings`, and
  the tests that set or compare it). `PgStore::this_user`/`Backend::this_user` are unrelated and stay.
- **D5 — test renames are wording only.** Every renamed test keeps its body and assertions.
  `chat_offline.rs` keeps its name and its two tests: the offline refusal is still real, and the
  online negative still pins that the header names no buffer. Only its message wording changes.

## Tasks (serial — one change set, shared files; no fan-out)

TDD order inside each task: change the pins first (so they fail against the old code where there
is a behavioural pin), then the code.

### T1 — Rebuild confirmation copy (D1)
Files: `crates/htui/src/ui/tabs/settings/connection.rs` (`confirm_rebuild` ~138-143, unit test
`the_rebuild_copy_names_both_lists` ~990-1003), `crates/htui/tests/connection.rs`
(`rebuild_needs_a_confirmation_and_names_both_lists` ~1656-1672),
`crates/htui/tests/snapshots/connection__confirm.snap` (re-accepted; the line re-wraps).
- Tests: remove `"pending/"` from both Survives lists; assert `!contains("pending/")` in both.
- Code: drop `, the pending/ buffer` from the format string.
- Snapshot: `cargo insta test -p htui --test connection` / review; the only diff is the re-wrapped
  confirm line.

### T2 — `RefreshSettings::this_user` (D4)
Files: `crates/htui-store/src/cache/refresh.rs` (struct doc ~39-42, field ~53-55, `Default` ~65),
`crates/htui-store/src/connect.rs` (~615 doc, ~622), `crates/htui/src/store_worker.rs` (~2188
comment), `crates/htui-store/tests/connect.rs` (~86), `crates/htui-store/tests/cache.rs` (~48-55
`settings` and its doc).

### T3 — `project_caps_for` / `quota_latch_for` (D2)
File: `crates/htui/src/agent_worker.rs` — both helpers lose `_writer`, and their two call sites each
(~916-921, ~1818-1833) stop passing it. `quota_latch_for` returns `QuotaLatch`, and its doc stops
describing the removed `None` case. The `ChatArgs.quota_latch` field and its doc (~2021-2023), the
`Debug` impl (~2035), and `run_chat` (~3423) follow.

### T4 — `writer_label` (D3)
Files: `crates/htui/src/store_worker.rs` (`StoreReply::ChatAccepted` ~935-948),
`crates/htui/src/agent_worker.rs` (locals ~893, ~1768; `ChatArgs` constructions ~968, ~1928; field
~1999-2001; `run_chat` destructure ~3355 and `ChatAccepted` ~3405),
`crates/htui/src/ui/tabs/chat/mod.rs` (module doc ~17-19, match ~549, test constructions ~808-822,
~1095-1099), `crates/htui/tests/chat_live_cli.rs` (~293-296) and `crates/htui/tests/chat_live_agy.rs`
(~273-276). Both live tests are `#[ignore]`d but still compile; their `println!` keeps the session
ref and drops the label.

### T5 — Test names and messages (D5)
- `crates/htui-core/src/store/mem.rs:6846` `a_chat_run_mints_the_two_rows_the_offline_upload_would`
  → `a_chat_run_mints_its_run_and_step_rows_running`.
- `crates/htui-agent/tests/recorder.rs:1531` "…and the uploader's sum…" → "the recorder's sum and a
  fresh sum over the same persisted rows are one document".
- `crates/htui/src/agent_worker.rs`: the "buffered writer" messages become "offline backend"
  (~5828/5831, ~5976/5979, ~7170/7173); `a_plan_is_refused_before_any_request_on_a_buffered_writer`
  → `…_on_an_offline_backend` (~5944); `a_start_is_refused_before_any_spawn_on_a_buffered_writer`
  → `…_on_an_offline_backend` (~7141); mirror names `"install-buffered"` / `"login-buffered"` →
  `"install-offline"` / `"login-offline"` (~5947, ~7143).
- `crates/htui/tests/chat_offline.rs:239-240` message "a memory chat is not buffered" → "the header
  names no buffer".
- `crates/htui-store/tests/cache.rs` `pending_event` → `synthetic_event` (definition ~115, calls
  ~1176, ~1641, ~1716, ~1745).

### T6 — Verify
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features -- --test-threads=1` (sandbox Postgres/Qdrant via env)
- Closing text scan: no `writer_label`, no `offline_upload`, no `uploader's`, no `buffered writer`,
  and no `pending/` in `ui/tabs/settings/connection.rs` or `tests/connection.rs` beyond the negative
  assertions.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Rebuild confirm says "the pending/ buffer" survives (connection.rs ~140) | ✓ | `connection.rs:140` |
| …asserted in the unit test (~996), `tests/connection.rs`, and the `connection__confirm` snapshot | ✓ | `connection.rs:996`; `tests/connection.rs:1663`; snapshot lines 27-28 |
| `CacheStore::rebuild` touches only mirrored tables, `cache_cursor`, `last_full_refresh_at` | ✓ | `htui-store/src/cache/mod.rs:177-196` |
| `RefreshSettings::this_user` is set and never read | ✓ | writes: `connect.rs:622`, `refresh.rs:65`, `tests/cache.rs:54`; only read: `tests/connect.rs:86` (asserting the write) |
| The other `RefreshSettings` literals in `tests/cache.rs` use `..` and do not name `this_user` | ✓ | `tests/cache.rs:732-734, 1098-1101, 1234-1240` |
| `quota_latch_for` always answers `Some`; both helpers take an unused `_writer` | ✓ | `agent_worker.rs:2049-2089` |
| `Recorder` has real `None` latch cases, so its `Option` stays | ✓ | `record.rs:423-425, 504, 1249` |
| `ChatArgs.quota_latch` is read only by `Debug` and `run_chat`'s `if let Some` | ✓ | `agent_worker.rs:2035, 3366, 3423` |
| `writer_label` rides `ChatAccepted` and `ChatArgs`; the Chat tab ignores it | ✓ | `store_worker.rs:947`; `agent_worker.rs:2001`; `chat/mod.rs:549` (`writer_label: _`) |
| Other readers of `writer_label` | **amended** | the item missed `chat_live_cli.rs:293-296` and `chat_live_agy.rs:273-276` (print it); now in T4 |
| `Writer::label()` still has users after T4 | ✓ | 5 `Debug` impls, `agent_worker.rs:2105/2205/2443/2614/2806` |
| `StoreReply` is not serialized (field removal is not a wire format change) | ✓ | `store_worker.rs:868` `#[derive(Debug, Clone)]` only |
| Other `ChatAccepted` matches use `..` (unaffected) | ✓ | `tests/chat_live.rs:77`, `tests/chat.rs:1676` |
| Stale names: mem.rs `…offline_upload_would`, recorder.rs ~1531, agent_worker ~5828/5976/7170, chat_offline.rs, tests/cache.rs `pending_event` | ✓ | `mem.rs:6846`; `recorder.rs:1531`; `agent_worker.rs:5828/5976/7170`; `chat_offline.rs:239-240`; `tests/cache.rs:115` |
| `synthetic_event` does not collide in `tests/cache.rs` | ✓ | no `fn synthetic_event`/`session_event` fn there |
| `recorder.rs`'s other "buffered" hits describe the live flush buffer, not the offline one | ✓ | `recorder.rs:1356, 2275, 2583-2674, 3386-3546` — out of scope |
| Live chat tests compile under `--all-features` even though `#[ignore]`d | ✓ | `chat_live_cli.rs:84` `#![cfg(feature = "testkit")]`, `#[ignore]` at 243; agy `#[ignore]` at 168 |
| Task independence | n/a | serial by decision: T3 and T4 share `agent_worker.rs`; T2 and T4 share `store_worker.rs` |
