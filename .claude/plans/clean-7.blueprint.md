# CLEAN-7 blueprint: leftovers of the removed offline chat buffer

Plan: `.claude/plans/clean-7.plan.md` (D1-D5). Line numbers are from `27e507d`. Work serially, one
commit per task, and run `cargo fmt --all` before each commit. rustfmt is `max_width = 100`, and
clippy is `all = warn` with **pedantic off** (`Cargo.toml:144-146`).

## T1: rebuild confirmation copy (D1). `fix(clean-7): the rebuild confirmation names no pending/ buffer`
- `crates/htui/src/ui/tabs/settings/connection.rs:996`: delete `"pending/",`. After the `for` loop
  (after :1002), add:
  `assert!(!confirm_rebuild().contains("pending/"), "D1: `CacheStore::rebuild` has no pending/ buffer to spare");`
- `crates/htui/tests/connection.rs:1663`: `[..., "built_at", "pending/"]` becomes
  `["schema_version", "db_fingerprint", "built_at"]`. After that loop (:1665), add
  `assert!(!frame.contains("pending/"), "D1: no pending/ buffer survives, because none exists: {frame}");`
- `connection.rs:140`: `..., built_at, the pending/ buffer. Goes: ...` becomes `..., built_at. Goes: ...`.
  The doc at :133-137 stays as it is.
- `crates/htui/tests/snapshots/connection__confirm.snap`: run `cargo insta test -p htui --test connection
  --review`. The question goes from 3 lines to 2 (width 100, `frame_of` :1140). The pane is
  `Constraint::Length(pane.len())` (:848), so the `Min(3)` rows area also gains **one blank line**.
  That diff is expected. The hint stays on row 29 (`hint_of` :1147). The new lines are:
  `Rebuild the mirror? Survives: the file, schema_version, db_fingerprint, built_at. Goes: the 21`
  `mirrored tables, cache_cursor, last_full_refresh_at. The next refresh pass refills it. y / n`

## T2: `RefreshSettings::this_user` (D4). `refactor(clean-7): RefreshSettings drops the unread this_user`
- `crates/htui-store/src/cache/refresh.rs:15`: `use htui_core::model::{BoxId, ProjectId, UserId};` becomes
  `{BoxId, ProjectId}`. **Hazard:** without this change, `unused_imports` fails `-D warnings`.
  :15, :55 and :65 are the only `UserId` sites in the file.
- refresh.rs:39-42, the struct doc paragraph, becomes:
  ```
  /// `this_box` is not tuning; it is here because §6.2 step 2 mirrors the *own* `box` row, and
  /// because both [`Refresher::spawn`] and [`run_pass`] would otherwise need one more argument each
  /// (deviation from blueprint C.13, same components). Its twin `this_user` was for the upload of
  /// the offline chat buffer (MOD-25) and was removed with it (CLEAN-7).
  ```
- refresh.rs:53-55 (field and its 2-line doc) and :65 (`this_user: UserId::default(),`): delete them.
  `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` stays valid, since the remaining fields are
  Duration, Duration, i64 and BoxId.
- `crates/htui-store/src/connect.rs:613-617` doc becomes:
  ```
  /// Fills in what only a connected server knows: this box and the two cache settings.
  ///
  /// `this_box` comes from the store ([`PgStore::this_box`]); the interval and the overlap come from
  /// `app_setting`, which [`PgStore::seed_if_empty`] seeds with 30 s and 300 s. A missing,
  /// non-numeric or non-positive value keeps `base`'s value, and a failed read is a `warn!` rather
  /// than a refusal to mirror at all.
  ```
  :622 `this_user: pg.this_user(),`: delete it. The literal stays multi-line, because its fields
  exceed `struct_lit_width`.
- `crates/htui/src/store_worker.rs:2188-2189` becomes
  `// Read before the move: `this_box` and the two cache settings are what only a connected server`
  `// knows (blueprint C.13's `RefreshSettings`).`
- `crates/htui-store/tests/connect.rs:86`: delete `assert_eq!(settings.this_user, pg.this_user());`.
- `crates/htui-store/tests/cache.rs:48-49` doc becomes
  `/// Settings whose `this_box` points at the demo box, as `load_demo` leaves [`htui_store::PgStore`].`
  (exactly 100 columns). Delete :54 `this_user: db.store.this_user(),`. `db.store.this_user()` is
  still used at :1868, and no `UserId` import depends on this line.

## T3: `project_caps_for` / `quota_latch_for` (D2). `refactor(clean-7): quota_latch_for answers a QuotaLatch; the chat helpers drop _writer`
- `crates/htui/src/agent_worker.rs:2049-2053`: `fn project_caps_for(_writer: &Writer, project_id: ProjectId,
  settings: Option<Value>)` becomes `fn project_caps_for(project_id: ProjectId, settings: Option<Value>)`.
  Its doc (:2040-2048) does not mention the writer and stays.
- :2075-2080: `fn quota_latch_for(_writer: &Writer, agent: &Agent, box_id: BoxId, source: QuotaSource)
  -> Option<QuotaLatch>` becomes `fn quota_latch_for(agent: &Agent, box_id: BoxId, source: QuotaSource)
  -> QuotaLatch`, and the body `Some(QuotaLatch { .. })` becomes `QuotaLatch { .. }`. Doc :2068-2070
  becomes:
  ```
  /// The decision is made **here**, at chat start, rather than discovered on the first `usage` row.
  /// Every chat gets one: a chat starts only online (MOD-25), and the `None` an offline chat once got
  /// (the mirror has no `agent_box` table, plan D52) left with the offline path (CLEAN-7).
  ```
  Keep :2072-2074 (`R-AGT-5`).
- Call sites: at :916-920 and :1818-1822 drop the `&writer,` argument. At :921 and :1833 the call becomes
  `quota_latch_for(&summary.agent, box_id, settings.quota.source)`. `writer` stays live at both sites
  (:922 `step_events` and :1836 `start_chat_run`, then moved into `ChatArgs`), so there is no
  unused-variable fallout.
- `ChatArgs` :2021-2023 becomes:
  ```
  /// Plan D66-D68: the `agent_box` row this chat latches its allowance into
  /// ([`quota_latch_for`]). Always one, since a chat starts only online; `Recorder`'s own latch
  /// stays an `Option` for the recorders that have none.
  quota_latch: QuotaLatch,
  ```
- `Debug` :2033-2035: delete the two-line comment and `.field("quota_latch", ...)`. The
  `missing_fields_in_debug` lint is pedantic, so it does not fire.
- `run_chat` :3420-3422: `if let Some(latch) = quota_latch { recorder = recorder.with_quota_latch(latch); }`
  becomes `recorder = recorder.with_quota_latch(quota_latch);` (`record.rs:559` takes `QuotaLatch`).
  The comment at :3417-3419 stays true.
- Imports: `Writer` (:57) and `QuotaLatch` (:47) are still used, at :1998/:2095/:2148/... and at the
  field and return type. Nothing becomes unused.

## T4: `writer_label` (D3). `refactor(clean-7): writer_label leaves ChatAccepted and ChatArgs`
- `crates/htui/src/store_worker.rs:942-947`: delete the field and its 5-line doc. `StoreReply` is
  `#[derive(Debug, Clone)]` only (:868), so this changes no wire format.
- `agent_worker.rs`: delete :893 and :1768 (`let writer_label = writer.label();`), :968 and :1928
  (`writer_label,`), the field and doc at :1999-2001, the destructure entry at :3355, and the
  `ChatAccepted` entry at :3405.
- `crates/htui/src/ui/tabs/chat/mod.rs:549`: delete `writer_label: _,`. The pattern
  `{ step_id, session_ref, caps }` stays exhaustive. Delete :822 and :1099 (`writer_label: "memory",`).
  Module doc :16-19 becomes:
  ```
  //! Milestone 4 also let a chat start with the store unreachable, in which case its rows went to the
  //! offline buffer instead of to Postgres, and the header said so (D42) from a writer label the
  //! acceptance carried. Since MOD-25 an offline chat is refused before it starts and the buffer has
  //! been removed; CLEAN-7 took the label off `StoreReply::ChatAccepted`.
  ```
- `crates/htui/tests/chat_live_cli.rs:290-296` and `chat_live_agy.rs:270-276`: delete `writer_label,`
  (keep `..`). `println!("accepted: session {session_ref:?} into `{writer_label}`")` becomes
  `println!("accepted: session {session_ref:?}")`.
- **Plan gap:** `crates/htui-store/src/writer.rs:60-61`, the `Writer::label` doc, still says "for logs
  and for the chat header (plan D42 ...)". After T4 its only users are the 5 `Debug` impls
  (`agent_worker.rs:2105/2205/2443/2614/2806`) and the unit test at `writer.rs:1165`. New doc:
  ```
  /// The backend label this writer belongs to, for the `Debug` of every task that holds one. Plan
  /// D42 also put it in the chat header, to tell a recorded conversation from one only on this
  /// disk; that went with the offline buffer (MOD-25), and the label left `ChatAccepted` in CLEAN-7.
  ```
- **Plan gap, in the same file:** the comment at `writer.rs:205-206` says "The `Buffered` arm is where
  the refusal lives." `Writer` has had no `Buffered` arm since MOD-25, so this is false. Replace it
  with "means. A store that cannot record never gets this far: `Backend::writer` answers `None`
  (MOD-25)." Neither the plan's closing scan nor its tasks cover this line.

## T5: test names and messages (D5). `test(clean-7): offline-backend wording in test names and messages`
- `crates/htui-core/src/store/mem.rs:6846`: `a_chat_run_mints_the_two_rows_the_offline_upload_would`
  becomes `a_chat_run_mints_its_run_and_step_rows_running`. The doc at :6844 stays.
- `crates/htui-agent/tests/recorder.rs:1531`: the message becomes
  `"the recorder's sum and a fresh sum over the same persisted rows are one document"`.
- `agent_worker.rs` :5828, :5976 and :7170: `"the buffered writer's own sentence, not a second one: {message}"`
  becomes `"the offline backend's own sentence, not a second one: {message}"`. The panics at :5831,
  :5979 and :7173 change from `"a buffered writer refuses the {probe|plan|login}: {other:?}"` to
  `"an offline backend refuses the {probe|plan|login}: {other:?}"`.
- Renames: :5945 becomes `a_plan_is_refused_before_any_request_on_an_offline_backend` and :7141
  becomes `a_start_is_refused_before_any_spawn_on_an_offline_backend`. This matches its sibling at
  ~:5797, `an_offline_backend_refuses_the_probe_before_spawning_anything`. `"install-buffered"` at
  :5947 becomes `"install-offline"` and `"login-buffered"` at :7143 becomes `"login-offline"`
  (`CacheStore::open` fingerprint strings, test-local). No name collisions (checked with rg).
- `crates/htui/tests/chat_offline.rs:240`: the message becomes `"the header names no buffer: {rendered}"`.
  The `!contains("buffered")` at :239 stays.
- `crates/htui-store/tests/cache.rs`: `fn pending_event` at :115 and its calls at :1176, :1641, :1716
  and :1745 become `synthetic_event`. The doc at :114 already says "synthetic". No collision exists.

## Hazards (checked against the tree)
1. `UserId` becomes unused in `refresh.rs:15` (T2). This one is certain and would fail
   `-D warnings`.
2. The T1 snapshot diff has two parts: the rewrap plus one extra blank row. The pane height depends
   on its content (connection.rs:848). No other test depends on the question's row position. :1595
   and :1707 use `contains("Rebuild the mirror?")`.
3. There is no `needless_pass_by_value`, `unnecessary_wraps` or `missing_fields_in_debug` risk,
   because pedantic is off. `#[must_use]` on `Writer::label` still holds, since the `Debug` impls
   use its value.
4. `Writer`/`QuotaLatch` imports in `agent_worker.rs` stay used. `writer` locals stay used. `ChatArgs`
   is built only at :965 and :1925.
5. `HANDOFF.md:765-775` (the CLEAN-7 item) quotes `writer_label`, `offline_upload` and `pending/`.
   The closing scan should exclude it, and it should be closed in a separate `docs(handoff)` commit
   (the MOD-64 precedent, `dbf03a2`).

## Remaining wording outside the tasks
- Stale and not covered: `writer.rs:60-61` and `:205-206`. Both are folded into T4 above.
- Accurate history, left alone: `writer.rs:11-12,93`, `backend.rs:12-16,62,140-141`,
  `testkit.rs:11`, `pg/write.rs:1531`, `traits.rs:1977` ("offline upload path", with a space, so it
  does not match the scan), `cache/mod.rs:31`, `model/run.rs:283`, `record.rs:1190`,
  `agent_worker.rs:1763-1764`, `chat_offline.rs:1-17`. The other "buffered" hits are the recorder's
  live flush buffer and are out of scope.
- `docs/` (excluding `decisions/` and `ANA-*`) has no hits for `writer_label` or `RefreshSettings`.

## Verify (T6)
`cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
then `cargo test --workspace --all-features -- --test-threads=1`. The closing scan is
`rg -n '\bwriter_label\b|offline_upload|uploader.s|buffered writer' crates` (expect none) and
`rg -n 'pending/' crates/htui/src/ui/tabs/settings/connection.rs crates/htui/tests/connection.rs`
(expect only the two negative assertions).
