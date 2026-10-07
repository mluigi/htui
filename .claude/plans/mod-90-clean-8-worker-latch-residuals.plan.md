# Plan: MOD-90 + CLEAN-8 — `htui worker` drops a refused login on any Settings write; MOD-10 M4 review residuals

**Source:** HANDOFF.md items MOD-90 and CLEAN-8 (both from MOD-10, `docs/decisions/mod/mod-10.md`).
**Routing:** plan path, no ultracode, both run together in sandbox `hr/MOD-90` (maintainer, 2026-10-07).
**Complexity:** Medium. Ten small changes across five crates. None needs a migration, a `.sqlx` change or a changed
snapshot.
**Status:** DONE 2026-10-07 (D1–D6 as recommended; blueprint amendments A-1..A-5 accepted; R1 M-1 shown in the UI by maintainer choice). Write-ups `docs/decisions/mod/mod-90.md`, `docs/decisions/clean/clean-8.md`.

## Decisions for the maintainer (read first)

- **D1: MOD-90 mechanism: a keyring-side write mark (recommended).** Add a fourth keyring entry,
  `htui/infisical-write-mark`, holding a fresh UUID (v7). Every Settings > Secrets URL or identity write or clear
  stores a new value there. `KeyringInfisical` reads it with the URL and identity and keeps it in its cache key, so
  any process, `htui worker` included, rebuilds its provider after any Settings write. That includes the same
  identity entered again, and the rebuild drops the refused-login latch. Nothing rebuilds unless a write happened,
  so a refused login is never retried on its own.
  - Rejected, a store row: it needs a migration and a store round trip on every resolution, and a provider-less or
    offline worker could not see it.
  - Rejected, a command: it needs a new IPC seam to the worker for one bit of state.
- **D2: mark write order.** The mark is written **last**, inside the same `keyring_io` critical section as the write
  it marks, and read **first**. A worker that reads concurrently with a Settings write (another process, so not
  under `keyring_io`) can then cause at most one extra rebuild, never a missed one. This is the same "one more,
  never one too few" rule as the in-process generation.
- **D3: a mark write that fails is best effort (recommended).** The URL or identity write already landed. The reply
  stays `SecretsWritten` and a `tracing::warn!` names the slot, never a value. The TUI still rebuilds through its
  in-process generation; only another process misses the write, which is today's behaviour, and its doc line
  ("restart it") stays as the fallback. The alternative, answering `Failed`, would report a landed write as failed.
- **D4: a mark read that fails refuses the walk**, as an unreadable URL or identity half does (`keyring_unreadable`,
  `Config`). A keyring that answers the URL and refuses the mark is not plausible enough to justify a quieter path. A
  missing mark (a keyring written before MOD-90) reads as `None`, which is a valid cache key.
- **D5: CLEAN-8 #2 scope.** Fix `ProjectPatch.secret` only, as the item names it. The same latent shape exists in
  `RepoPatch.remote_url` (`hierarchy.rs:180`) and `ItemPatch.step_graph_id` (`item.rs:279`). Neither is serialized
  in production, so they are recorded in the close-out rather than fixed.
- **D6: CLEAN-8 #9 scope.** Guard only the four Settings > Qdrant arms, as the item says. Concepts search and
  indexing also read the Qdrant keyring (`concepts_worker.rs:220`, `concepts.rs:97/334`); they are noted and left
  unchecked.

## Summary

- **MOD-90:** `KEYRING_WRITES` (`crates/htui/src/secrets.rs:36`) is per process, so `htui worker`'s own
  `KeyringInfisical` (`worker_cmd.rs:110`) rebuilds only on a changed URL, client ID or client secret. A same-identity
  re-entry in the TUI therefore leaves the worker latched until it restarts (`docs/htui-secrets.md:445-448`). The
  keyring mark (D1–D4) carries the write across processes.
- **CLEAN-8:** nine residuals, each bounded to one to four files: L-7's body buffer, the 7 rust-reviewer NITs, and the
  Qdrant demo guard.

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Keyring slot | `crates/htui-store/src/secret.rs:398-466` | `read_slot`/`write_slot`/`remove_slot` + `FakeSlots` field + `infisical_field` arm |
| Write under lock | `crates/htui/src/secrets_settings.rs:340-351` `keyring_write` | `spawn_blocking` → `keyring_io()` → write → note |
| Provider cache key | `crates/htui/src/secrets.rs:124-150` `Cached::built_from` | every input compared; digest for secrets |
| Demo guard | `crates/htui/src/secrets_settings.rs:213-221, 278-289` | `Backend::Memory` → `NotApplicable` read, `DEMO_SESSION` refusal |
| Double option serde | `crates/htui-core/src/model/kind.rs:331-347` `present_option` | `default` + `deserialize_with` + `skip_serializing_if` |
| Tests | `crates/htui/src/secrets.rs:560` `a_settings_keyring_write_rebuilds_a_latched_provider` | `mock_keyring` guard, counting builder, `Arc::ptr_eq` |
| Demo tests | `crates/htui/tests/secrets_settings.rs:210, 223` | `demo_rows_are_not_applicable_and_read_no_keyring`, `demo_refuses_every_keyring_write` |
| Three-state serde test | `crates/htui-core/src/model/kind.rs:680` | `phase_patch_persona_round_trips_all_three_states` |

## Tasks

TDD throughout: write the red test, watch it fail, then implement. Each task commits on its own.

### Lane A (serial, primary tree): keyring seam

**Files:** `crates/htui-store/src/secret.rs`, `crates/htui/src/secrets.rs`, `crates/htui/src/secrets_settings.rs`,
`crates/htui/tests/secrets_settings.rs`.

#### T1: MOD-90 write mark

- **htui-store:** add `INFISICAL_WRITE_MARK_USER = "infisical-write-mark"`, plus `get_infisical_write_mark()` and
  `set_infisical_write_mark(&str)` over `read_slot`/`write_slot`. Add the `FakeSlots.infisical_write_mark` field and
  its `infisical_field` arm. Update the module doc's entry list (`secret.rs:22-26`) and its constants test (`:874-892`).
- **htui `secrets.rs`:**
  - `Read` returns a `KeyringRead { mark: Option<String>, url, identity }`. `read_keyring` reads the mark first, under
    `keyring_io` (D2, D4).
  - `Cached` gains `mark: Option<String>`, and `built_from` compares it.
  - `KEYRING_WRITES` stays, because L-1's check generation depends on it.
  - Update the `with_parts` test reads.
- **htui `secrets_settings.rs`:** `keyring_write` stores `Uuid::now_v7().to_string()` after `write()` lands, still under
  the lock, best effort with `tracing::warn!` (D3), then `note_keyring_write()`.
- **Red tests:**
  - `secrets.rs`: `a_write_mark_from_another_process_rebuilds_a_latched_provider`. The mark is changed straight through
    `htui_store` with no `note_keyring_write`, so another process is simulated.
  - `secrets.rs`: `an_unchanged_mark_and_identity_keep_the_latched_provider`. No retry on its own.
  - `secrets.rs`: `a_missing_mark_is_a_valid_key`.
  - `tests/secrets_settings.rs`: `every_landed_keyring_write_stores_a_new_mark`, covering all four writes. A refused
    write (blank half, demo) leaves the mark unchanged.
  - `tests/secrets_settings.rs`: `a_refused_mark_write_still_answers_written`, using `refuse_store = Some(mark)`.
- **Validate:**
  - `cargo test -p htui-store --features test-support secret`
  - `cargo test -p htui --features testkit -- --test-threads=1 secrets`

#### T2: CLEAN-8 #7, typed half-stored identity (after T1, same files)

- **htui-store:** add `pub enum IdentityRead { Stored(MachineIdentity), NotStored, HalfStored { missing: &'static str } }`
  and `read_machine_identity() -> Result<IdentityRead>`. `Err` now means a keyring failure only.
  - `get_machine_identity` wraps it and keeps today's sentence byte for byte.
  - Expose the sentence builder, e.g. `half_identity_sentence(missing)`.
- **htui:** `secrets_settings::snapshot` and `identity_state` match the typed variant. The `starts_with` prefix sniff goes.
- `StoreError` is untouched: its one exhaustive match is `htui-mcp/src/tools/mod.rs:126`.
- **Red tests:**
  - htui-store: `read_machine_identity_reports_the_missing_half_typed`.
  - htui: `identity_state` maps an `Unreadable` error whose message starts with the prefix to `Unreadable`. This is a
    unit test on the pure function with a hand-built `StoreError::Backend`.

#### T3: CLEAN-8 #4, identity stored trimmed (after T2)

- `IdentityEntry::to_identity` trims both halves, which matches the section (`ui/tabs/settings/secrets.rs:728-730`).
- Update the docs at `secrets_settings.rs:107` and `:278`.
- **Red test:** `tests/secrets_settings.rs` `set_machine_identity_stores_the_trimmed_halves`.

### Lane B (worktree `mod-90-core`, serial inside): small-crate residuals

**Files:** `crates/htui-secrets/src/infisical.rs`, `crates/htui-core/src/model/hierarchy.rs`,
`crates/htui-core/src/model/kind.rs`, `crates/htui-core/src/secret.rs`, `crates/htui/src/hierarchy.rs`.

#### T4: CLEAN-8 #1, L-7 `read_body` wiped

- `read_body` returns `Zeroizing<Vec<u8>>`.
- Pre-size the buffer from `content_length().min(cap.bytes)`.
- Growth moves into a pure `append(buf, chunk, cap) -> bool`. It never lets the `Vec` reallocate in place: it allocates
  a new `Zeroizing` buffer with capacity `max(need, 2 × cap_now).min(cap.bytes)`, copies, and swaps, so the old buffer
  is wiped on drop. zeroize 1.9 wipes spare capacity too (verified below).
- Fix the one call site: `body.as_deref().ok()` gets `.map(Vec::as_slice)`.
- **Red test:** unit tests on `append`: pre-size exact, growth keeps the bytes, never past cap, over-cap → `false`.
- **Guard:** the four oversized-body tests in `crates/htui-secrets/tests/infisical.rs:1478-1530`.

#### T5: CLEAN-8 #2 + #3, `htui-core` serde

- Make `present_option` `pub(crate)` and apply it to `ProjectPatch.secret` (D5).
- `SecretScope::to_column` serializes a borrowed `ScopeColumnRef<'a>` with the same field order, so it no longer
  clones.
- **Red test:** `project_patch_secret_round_trips_all_three_states`.
- **Guards:** the column tests `secret.rs:802-860`, whose output must stay byte-identical.

#### T6: CLEAN-8 #6, one tree re-read

- `fresh_tree` becomes the only "snapshot or `NotFound` workspace" helper.
- `reread`, `cas` (which builds the tree once, then matches the outcome) and the two `infer` sites call it.
- This is a pure refactor.
- **Guards:** `tests/hierarchy.rs:298, 1018`, and `tests/secrets_settings.rs:659, 719, 2529`.

### Lane C (worktree `mod-90-ui`): Secrets section tidy

**Files:** `crates/htui/src/ui/tabs/settings/secrets.rs`.

#### T7: CLEAN-8 #5 + #8

- `rows()` becomes `row_count()` + `row_at(i)` with no allocation. `row()`, `move_cursor` and `clamp_cursor` use them.
- Split `on_reply`: `on_failed` takes the three `Failed` arms and `on_tree_gone` takes the `Hierarchy(None)|SecretsTree(None)` arm.
- Split `on_scope_written`: `on_scope_stale` takes the stale branch.
- Split `lines`: extract `project_lines` and `guide`.
- These are pure moves.
- **Guards:** all of `tests/secrets_settings.rs`, and the six `secrets_settings__*.snap`, which must stay byte-identical.
- **Unit test:** `row_at` over `0..row_count()` equals `FIXED` followed by the projects.

### Lane D (worktree `mod-90-qdrant`): Qdrant demo guard

**Files:** `crates/htui/src/qdrant_settings_info.rs`, `crates/htui/src/store_worker.rs`,
`crates/htui/src/ui/tabs/settings/qdrant.rs`, `crates/htui/tests/settings.rs`.

#### T8: CLEAN-8 #9

- Add `qdrant_settings_info::serve(backend, request)`:
  - On `Backend::Memory`, `QdrantInfo` answers `QdrantState::NotApplicable` (a new variant) and the three writes
    answer `Failed` with `secrets_settings::DEMO_SESSION`. That constant is reused unchanged, so this lane never edits
    `secrets_settings.rs`.
  - Otherwise it runs today's arm bodies.
- Move the four loop arms (`store_worker.rs:2951-2997`) into it. `try_serve`'s "handled in worker loop" arm
  (`:2160-2166`) calls it.
- **UI:**
  - `NotApplicable` renders "n/a in a demo session".
  - `e`/`c` are refused with a sentence and nothing is sent.
  - No auto-opened editor.
- **Red tests (`tests/settings.rs`):**
  - `qdrant_demo_reads_no_keyring`
  - `qdrant_demo_refuses_every_keyring_write`
  - `qdrant_demo_section_offers_no_edit`
- **Guards:** the five Qdrant tests `tests/settings.rs:5554-5711`, and `tests/secrets_settings.rs:499`.

### T9 (serial, primary tree, after all lanes merge): docs

**Files:** `docs/htui-secrets.md`.

- **Keyring entries** (`:53-70`): add the mark row and one sentence on what it is for.
- **Provider rebuild** (`:247-252`): the "or, in the TUI, after any URL or identity write" clause now covers any process.
- **Logins** (`:444-448`): drop "restart it"; say a worker sees the write at its next resolution. Keep "restart it" as
  the fallback only when the mark could not be stored (D3).
- **Settings > Qdrant demo** (`:141-142`): replace the "not guarded" sentence with the guard.
- **Identity form** (`:94-97`): "spaces around either half are dropped".
- **What is wiped** (`:405-414`): the response body buffer.

### Close-out (P2 for both)

- Update HANDOFF lines 435 and 446 and the summary table counts (`:476-477`).
- Write `docs/decisions/mod/mod-90.md`, and record CLEAN-8 as resolved (per `workflow-docs.md`). Update the MOD-10
  "Carried" lines `mod-10.md:121, 157-158`.
- Run the validator.

## Lane independence (file-set intersection)

| Lane | Files |
|---|---|
| A | `htui-store/src/secret.rs`, `htui/src/secrets.rs`, `htui/src/secrets_settings.rs`, `htui/tests/secrets_settings.rs` |
| B | `htui-secrets/src/infisical.rs`, `htui-core/src/model/{hierarchy,kind}.rs`, `htui-core/src/secret.rs`, `htui/src/hierarchy.rs` |
| C | `htui/src/ui/tabs/settings/secrets.rs` |
| D | `htui/src/qdrant_settings_info.rs`, `htui/src/store_worker.rs`, `htui/src/ui/tabs/settings/qdrant.rs`, `htui/tests/settings.rs` |
| T9 | `docs/htui-secrets.md` |

The pairwise intersections of A, B, C and D are empty. Same-named files are distinct paths: `htui-core/src/secret.rs`
≠ `htui-store/src/secret.rs`, `htui-core/src/model/hierarchy.rs` ≠ `htui/src/hierarchy.rs`, and
`ui/tabs/settings/secrets.rs` ≠ `secrets_settings.rs`. Lanes B and C only read `tests/secrets_settings.rs` as guards;
they do not edit it. Nothing touches `Cargo.lock`, `.sqlx`, `migrations/` or snapshots. A, B, C and D run in
parallel; B, C and D run in their own worktrees on named branches, because a shared tree would let one lane's
half-edit break another's build. T9 and the close-out run serially after the merges.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless gate
cargo test -p htui-core --features test-support
cargo test -p htui-store --features test-support
cargo test -p htui-secrets
cargo test -p htui --features testkit -- --test-threads=1    # keyring fake is process-wide
cargo insta test -p htui --features testkit --check -- --test-threads=1   # snapshots byte-identical
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

The Postgres gate runs on the host after `scripts/hr collect`. T5's change to `ProjectPatch` does not touch the SQL,
and `to_column`'s text is byte-identical.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A keyring backend prompts per entry, so the extra mark read adds a prompt | Low | Same service and application as the three entries already read; the read stays inside `KEYRING_TIMEOUT` |
| A cross-process half-pair read (worker reads during a TUI identity write) | Low, pre-existing | Out of scope (`secrets.rs:48` "Another process is not covered"); D2 keeps the mark from making it worse |
| `QdrantState` gains a variant, breaking exhaustive matches | Certain, contained | Only `qdrant.rs:450/463/508` match it; the lane fixes them |
| Parallel lanes contend for CPU (four cargo builds on 12 cores) | Medium | Worktrees have separate targets; disk has 2.7 T free; gates re-run on the merged tree |
| `fresh_tree` dedupe changes the error on a vanished workspace | Low | Same `NotFound { "workspace", ws }` everywhere; guard tests listed |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `htui worker` builds its own `KeyringInfisical` | ✓ | `worker_cmd.rs:110`; the TUI's is at `lib.rs:307` |
| Only `secrets_settings::serve` writes the Infisical entries in production | ✓ | grep of `set_machine_identity\|set_infisical_url\|clear_*` outside tests: only `secrets_settings.rs:302-313` |
| Infisical fake slots honour `refuse_store` and add a field per user | ✓ | `htui-store/src/secret.rs:58-70, 382-427` |
| `uuid` with `v7` is available in `htui` | ✓ | `Cargo.toml:59` (`v7`, `serde`); `crates/htui/Cargo.toml:63` |
| `StoreError` has exactly one exhaustive match outside its crate | ✓ | `htui-mcp/src/tools/mod.rs:126-138` |
| `present_option` exists and is private | ✓ | `htui-core/src/model/kind.rs:341` |
| `ProjectPatch` is never serialized in production (`StoreRequest` is `Debug, Clone` only) | ✓ | `store_worker.rs:146` |
| The Secrets form trims both halves before building `IdentityEntry` | ✓ | `ui/tabs/settings/secrets.rs:728-737` |
| The Qdrant loop arms have no `Backend::Memory` check; `try_serve` answers "handled in worker loop" | ✓ | `store_worker.rs:2160-2166, 2951-2997` |
| The docs state Qdrant is unguarded in demo | ✓ | `docs/htui-secrets.md:141-142` |
| No insta snapshot renders the Qdrant section | ✓ | only `concepts_search__error.snap` mentions Qdrant, unrelated |
| `read_body` has exactly two callers | ✓ | `infisical.rs:323, 375` |
| zeroize 1.9 `Vec` zeroize wipes spare capacity | ✓ | `zeroize-1.9.0/src/lib.rs:526-536` (`spare_capacity_mut().zeroize()`) |
| `clippy::pedantic` is off (the long functions are style, not lint) | ✓ | `Cargo.toml:199` |
| Lanes A, B, C and D are file-disjoint | ✓ | intersection table above |
| A demo session reaches the Qdrant arms through the spawned loop | unverified, test-checked | T8's red test `qdrant_demo_reads_no_keyring` exercises it |

## Acceptance

- [ ] An `htui worker` holding a latched provider rebuilds after another process's Settings write of the same
      identity, and never rebuilds without a write.
- [ ] All nine CLEAN-8 residuals are applied (#2 scoped per D5, #9 per D6).
- [ ] The validation block passes, and snapshots are unchanged.
- [ ] Docs updated, HANDOFF/DECISIONS bookkeeping done, validator green.
