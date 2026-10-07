# MOD-90 - Clear a refused Infisical login in `htui worker` when the same identity is re-entered (done, 2026-10-07)

**Requirements:** `R-SEC-4`, `R-TUI-8`.
**Origin:** MOD-10 (`docs/decisions/mod/mod-10.md`, blueprint A-4).
**Artifacts:** shared with CLEAN-8 (`docs/decisions/clean/clean-8.md`), which ran in the same sandbox run on
`hr/MOD-90`:
- plan [`.claude/plans/mod-90-clean-8-worker-latch-residuals.plan.md`](../../../.claude/plans/mod-90-clean-8-worker-latch-residuals.plan.md):
  D1-D6 and a verified-claims table;
- blueprint `.claude/plans/mod-90-clean-8-worker-latch-residuals.blueprint.md`: amendments A-1..A-5 (accepted),
  hazards H-1..H-31.

Decision numbers are local to that plan. Routed as **plan** (C3 only: the mechanism was open), no ultracode.

## The problem

A 401 latches a process's one `KeyringInfisical` provider: every later call answers `LoginRefusedEarlier` and sends
nothing. MOD-10 M4 made any Settings > Secrets write rebuild the TUI's provider through an in-process write
generation (`KEYRING_WRITES`). That generation is per process, so a separate `htui worker` rebuilt only when the
stored URL, client ID or client secret changed. Re-entering the same identity in the TUI left the worker latched
until it was restarted.

## What was built

- **D1 - a keyring write mark.** A fourth keyring entry, `htui/infisical-write-mark`, holds a UUIDv7. Every Settings
  URL or identity write or clear that lands stores a new one. `KeyringInfisical` reads it with the URL and the
  identity and keeps it in its cache key (`Cached::built_from` compares generation, mark, URL, client ID and secret
  digest). Any process sharing the OS keyring, `htui worker` included, therefore rebuilds after any Settings write,
  same values included, and drops the latch. Nothing rebuilds unless a write happened, so a refused login is never
  retried on its own. No migration, no new command, no store row. `409705d4`.
- **D2 - order.** The mark is written last, inside the same `keyring_io` critical section and blocking closure as the
  write it marks, and read first. A read racing another process's write costs at most one extra rebuild, never a
  missed one.
- **D3 - a refused mark write is best effort.** The URL or identity write has landed, so the reply stays
  `SecretsWritten`, and the in-process generation still rebuilds the TUI's provider. R1 M-1 (maintainer: show it in
  the UI) added `SecretsWritten.mark_stored`. When it is `false` the Secrets section shows a dim line under the rows,
  cleared by the next landed write that stores a mark: `the keyring refused htui/infisical-write-mark: a running htui
  worker sees the last write only after a restart if it left the values unchanged (an identity entered again)`. A
  `tracing::warn!` names the slot, never a value. `c9091f59`, snapshot `secrets_settings__write_mark_refused`
  `15d0420f`.
- **D4 - a mark read that fails refuses the walk** as an unreadable URL does (`Config`, `the OS keyring could not be
  read: …`). A keyring written before MOD-90 has no mark, which reads as `None` and is a valid key.
- `KEYRING_WRITES` and `provider_with_generation` stay as they were, because the provider check's generation (M4 R1
  L-1) depends on them. `secrets_settings::snapshot` does not read the mark.

Docs: `docs/htui-secrets.md` (Keyring entries, provider rebuild, Logins and lockout safety, the Settings line),
`01a48faf`, `c53b5ff6`.

## Tests

- `secrets.rs`:
  - `a_write_mark_from_another_process_rebuilds_a_latched_provider`, which simulates another process by writing the
    mark straight through `htui_store` with no generation bump. Per R1 L-3 it latches end to end:
    `BadCredentials`, then a fresh `BadCredentials` rather than `LoginRefusedEarlier`.
  - `an_unchanged_mark_and_identity_keep_the_latched_provider`, `a_missing_mark_is_a_valid_key`.
  - `a_broken_keyring_refuses_at_the_mark_read_first`, which pins D2's read order and D4.
  - `a_refused_mark_write_still_rebuilds_in_this_process`.
- `tests/secrets_settings.rs`:
  - `every_landed_keyring_write_stores_a_new_mark`: refused writes leave the mark unchanged. That covers the
    blank-half, normalisation, demo and keyring-refused cases; the last one pins D2's write-last order.
  - `a_refused_mark_write_still_answers_written`.
  - `a_refused_write_mark_shows_until_a_write_stores_one`.

## Review

rust-reviewer: approve with fixes (no CRITICAL or HIGH).
- **M-1:** the D3 warning reached only a `--log` file. Fixed in the UI, as above.
- **L-3:** the latch test, fixed as above.
- **L-4:** "same machine" in the docs, fixed.
- The NITs were applied in `2c285783` and `c53b5ff6`.

## Gates (merged tree, sandbox)

- `cargo fmt --check`; `cargo clippy --workspace --all-targets --all-features -D warnings`; the featureless
  `cargo clippy --workspace -D warnings`.
- `htui-core`, `htui-secrets`, and `htui-store` (excluding `qdrant_live`).
- `cargo insta test -p htui --features testkit --check -- --test-threads=1`: 52 binaries, 2698 passed before R1;
  re-run after R1, see the done-report.
- `qdrant_live` timed out under the parallel full-crate load and passed 10/10 when run alone. This run does not touch
  the Qdrant store.
- The host Postgres gate runs after `scripts/hr collect`.

## Carried

- A cross-process half-pair read (a worker reading between the two identity writes of a TUI) is still possible
  (`secrets.rs` `KEYRING_IO`: "another process is not covered"). The mark makes it self-heal at the next walk, but it
  does not prevent it.
- A worker on another machine has its own keyring and sees neither the identity nor the mark (by design).
