# Blueprint: MOD-90 + CLEAN-8, `htui worker` sees Settings writes through a keyring mark, plus the MOD-10 M4 residuals

**Plan:** `.claude/plans/mod-90-clean-8-worker-latch-residuals.plan.md` (confirmed 2026-10-07, D1–D6). Produced by `code-architect`.

- Lane A runs serially in the primary tree on `hr/MOD-90`.
- Lanes B, C and D each run in a worktree on `mod-90-core`, `mod-90-ui` and `mod-90-qdrant`, all cut from `62a8735a`.
- T9 and the close-out run serially after all merges.
- Line numbers are for `62a8735a`. They drift after the first commit, so anchor on names.

## Plan amendments (all five accepted by the maintainer, 2026-10-07)

- **A-1 (fact, T7): `on_reply` has four `Failed` arms, not three.**
  - The four are: `READ_NAME`, the four keyring writes plus `SET_PROJECT_SECRET_SCOPE`, `CHECK_SECRET_PROVIDER`, and `CHECK_SECRET_SCOPE` (`ui/tabs/settings/secrets.rs:1376-1412`).
  - `on_failed` takes all four, in that guard order. Its final `else` is a no-op, which replaces the `_ => {}` fall-through for any other `Failed`.
- **A-2 (design detail, T8): delete the four loop arms; don't call `serve` from them.** I verified the plan's "unverified" row:
  - **Today, `--demo` runs the loop arms.** The path is `lib.rs:144` (`args.demo` → `Started::detached(Backend::memory(..))`) → `spawn_hosted` (`lib.rs:185`) → `spawn_with_concepts` → the `QdrantInfo` arm at `store_worker.rs:2951`. That arm comes before `other => try_serve` (`:3078`).
  - **The harness and `SectionBench` run `try_serve`.** `testkit.rs:350-367` routes through `store_worker::serve`, so they get `Failed "handled in worker loop"` (`:2160-2166`).
  - **If the loop arms are deleted,** the four requests fall to `other => try_serve(&backend, other)`. Then `--demo`, the harness and the TUI share one path.
  - **Error text is byte-identical.** The `other` arm renders `failed(other.name(), &err)`, which is what each loop arm did by hand. A keyring error is `Backend`, never `Unreachable`, so `lost_the_store` never fires.
  - **One test must pin the real `--demo` path.** `qdrant_demo_reads_no_keyring` therefore asserts through both `store_worker::serve` and a spawned loop (`store_worker::spawn`), still inside `tests/settings.rs`.
- **A-3 (side effect, T8): a non-demo harness that opens Settings now reads the Qdrant keyring.**
  - `SettingsTab::wants_requests` (`settings/mod.rs:316-325`) sends every section's reads, `QdrantInfo` included.
  - Today the offline harnesses get `Failed "handled in worker loop"`. Those are `box_settings.rs:2001`, `hierarchy.rs:1801`, `keys.rs:82`, `kinds.rs:909`, `personas.rs:1351`, `prompt_settings.rs:917` and `connection.rs:1189`.
  - After T8 they run `QdrantSnapshot::fetch()`. Without a `mock_keyring` guard that is the real OS keyring, read-only. `SecretsInfo` already does the same through `try_serve`.
  - No `.snap` contains `qdrant` or `handled in worker loop`; only `concepts_search__error.snap` matches, and it is unrelated.
  - **Recommendation:** accept, and record it in the close-out.
- **A-4 (validation command, T1): the plan's `-- --test-threads=1 secrets` filter skips most of `tests/secrets_settings.rs`.**
  - The filter matches test names, and integration tests have no module path. `every_landed_keyring_write_stores_a_new_mark` does not contain "secrets".
  - Use `--lib secrets` plus `--test secrets_settings` (see the per-lane gates).
- **A-5 (test gap, D4): no fake can fail a single entry's read.**
  - `refuse_store` covers writes only. A new knob would need `htui-store/src/testkit.rs`, which is outside lane A.
  - **Recommendation:** no knob. Pin "mark read first" (D2) and "a failed mark read refuses" (D4) with `mock_keyring_broken`: the refusal must name `htui/infisical-write-mark`, which only happens if the mark is read first and its failure is fatal.
  - The D2 write order (mark last) is pinned by a keyring-refused identity write that leaves the mark unchanged.

---

## Lane A (primary tree, serial T1 → T2 → T3)

**Files (checked; no others needed):** `crates/htui-store/src/secret.rs`, `crates/htui/src/secrets.rs`, `crates/htui/src/secrets_settings.rs`, `crates/htui/tests/secrets_settings.rs`.

The existing `htui_store::testkit` helpers cover everything:
- `mock_keyring`
- `mock_keyring_broken`
- `refuse_fake_store`, which takes any `&'static str` user and is honoured by `write_slot`
- `fake_machine_identity`
- `BROKEN_KEYRING`

### T1: MOD-90 write mark

**htui-store `secret.rs`**

- After `INFISICAL_CLIENT_SECRET_USER` (`:49`):

```rust
/// Keyring user name of the Infisical write mark (MOD-90 D1): a UUIDv7 replaced by every
/// Settings > Secrets URL or identity write or clear, so another process sees the write.
pub const INFISICAL_WRITE_MARK_USER: &str = "infisical-write-mark";
```

- `FakeSlots` (`:59-70`): add `pub infisical_write_mark: Option<String>,` after `infisical_client_secret`. `Default` covers `testkit::mock_keyring:337`.
- `infisical_field` (`:383-390`): add `INFISICAL_WRITE_MARK_USER => &mut slots.infisical_write_mark,`.
- After `clear_infisical_url` (`:464`):

```rust
/// The stored write mark, if any (MOD-90 D1, D4: a missing one, from a keyring written before
/// MOD-90, is `None`). Blank reads as `None`, as every Infisical entry.
pub fn get_infisical_write_mark() -> Result<Option<String>> { read_slot(INFISICAL_WRITE_MARK_USER) }

/// Replaces the write mark. Honours the fake's `refuse_store`.
pub fn set_infisical_write_mark(mark: &str) -> Result<()> { write_slot(INFISICAL_WRITE_MARK_USER, mark) }
```

  No `clear_*`: nothing removes the mark (YAGNI).
- Module doc (`:22-26`): "four entries", add `INFISICAL_WRITE_MARK_USER` (`infisical-write-mark`), and one clause on what it is for.
- `the_infisical_entries_are_named_exactly_as_the_plan_says` (`:888`): add `assert_eq!(INFISICAL_WRITE_MARK_USER, "infisical-write-mark");`.

**htui `secrets.rs`**

- Beside `type Read` (`:97`):

```rust
/// One keyring read (MOD-90 D1): the write mark, read first (D2), the normalised URL and the
/// identity. All three are the provider's cache key.
struct KeyringRead {
    mark: Option<String>,
    url: String,
    identity: MachineIdentity,
}

type Read = dyn Fn() -> Result<KeyringRead, SecretError> + Send + Sync;
```

- `Cached` (`:113`) gains `mark: Option<String>` with the doc "the write mark the provider was built under (MOD-90)". `built_from` takes the read, so it gains no parameters:

```rust
fn built_from(&self, generation: u64, read: &KeyringRead, secret_digest: &[u8; 32]) -> bool {
    self.generation == generation
        && self.mark == read.mark
        && self.url == read.url
        && self.client_id == read.identity.client_id()
        && &self.secret_digest == secret_digest
}
```

- `read_keyring` (`:178`), in this exact order:

```rust
fn read_keyring() -> Result<KeyringRead, SecretError> {
    let _io = keyring_io();
    // MOD-90 D2: first. A Settings write stores it last, so a read racing another process's write
    // costs at most one extra rebuild, never a missed one. D4: unreadable refuses, as a URL does.
    let mark = htui_store::secret::get_infisical_write_mark().map_err(|err| keyring_unreadable(&err))?;
    let raw = /* unchanged */;
    let url = /* unchanged */;
    let identity = /* unchanged: get_machine_identity */;
    Ok(KeyringRead { mark, url, identity })
}
```

- `current` (`:191`):
  - `let read: KeyringRead = timeout(..)…??;`
  - digest from `read.identity`
  - `held.built_from(generation, &read, &digest)`
  - then `let KeyringRead { mark, url, identity } = read;`
  - build, and store `Cached { generation, mark, url, client_id, secret_digest, provider }`
- Docs to update:
  - module doc `:5-8` ("what it was built from" now includes the mark)
  - `KEYRING_WRITES` doc `:31-35` (in process; the mark carries a write across processes)
  - `KEYRING_IO` doc `:48` ("Another process is not covered" applies to half-pairs; the mark is written under this lock)
- `KEYRING_WRITES` and `provider_with_generation` are unchanged (L-1).
- **`secrets_settings::snapshot` does not read the mark.** The rows show state, not the mark, and an extra prompt per Settings read is a cost with no gain.

**Test helper changes in `secrets.rs`**

- `counting`: no change. It goes through `with_builder`, which uses the real `read_keyring`, so every existing `counting` test now also reads the mark (`None` unless stored).
- `with_parts` callers:
  - `the_check_learns_the_generation_its_provider_was_built_at` (`:650`): `Ok((URL.to_owned(), MachineIdentity::new(..)))` becomes `Ok(KeyringRead { mark: None, url: URL.to_owned(), identity: MachineIdentity::new(CLIENT_ID, CLIENT_SECRET) })`.
  - `a_keyring_that_never_answers_is_refused_after_the_timeout` returns `Err(..)` and needs no change.
- New helper `fn store_mark(mark: &str)` over `htui_store::secret::set_infisical_write_mark`.
- `keyring_now` (`:702`) gains the mark: `(Option<String>, bool, Option<String>)`. `a_settings_keyring_write_waits_for_a_keyring_read_in_progress` then also pins "the mark is written under the lock" (its `before` / "written nothing yet" assertions cover it).

**htui `secrets_settings.rs` `keyring_write` (`:340`)**

```rust
tokio::task::spawn_blocking(move || {
    let _io = crate::secrets::keyring_io();
    write()?;
    // MOD-90 D1, D2: last, inside the write's own critical section. D3: best effort; the write landed.
    let mark = uuid::Uuid::now_v7().to_string();
    if let Err(err) = secret::set_infisical_write_mark(&mark) {
        tracing::warn!(slot = secret::INFISICAL_WRITE_MARK_USER, %err,
            "the keyring write mark was not stored; another htui process sees this write after a restart");
    }
    Ok(crate::secrets::note_keyring_write())
})
```

Update the doc of `keyring_write` and of `serve` (`:270-283`: "every write that lands stores a new mark and bumps the generation").

**T1 red tests**

`crates/htui/src/secrets.rs` `mod tests`. Each test takes `mock_keyring` as its first statement.

1. `a_write_mark_from_another_process_rebuilds_a_latched_provider`
   - Arrange: `store_url`, `store_identity`, `store_mark("m-1")`, and `counting(|| FakeSecretProvider::new([Err(BadCredentials), Err(LoginRefusedEarlier)]))`.
   - Act: `first = provider()`; `store_identity(CLIENT_ID, CLIENT_SECRET)`; `store_mark("m-2")`. **No `note_keyring_write`.** Then `second = provider()` and `third = provider()`.
   - Assert: `builds == 2`, `!ptr_eq(first, second)`, `ptr_eq(second, third)`.
2. `an_unchanged_mark_and_identity_keep_the_latched_provider`
   - Arrange: as test 1 with `store_mark("m-1")`.
   - Act: `resolve_project(Some(&source), &provider_project())` twice.
   - Assert: `BadCredentials`, then `LoginRefusedEarlier`, and `builds == 1`.
3. `a_missing_mark_is_a_valid_key`
   - Arrange: URL and identity, no mark.
   - Act: two `provider()` calls.
   - Assert: `builds == 1` and `ptr_eq`.
   - Then `store_mark("m-1")` and `provider()`: `builds == 2` (`None` → `Some` is a change).
4. `a_broken_keyring_refuses_at_the_mark_read_first` (A-5)
   - Arrange: `mock_keyring_broken`.
   - Act: `refusal(&source)`.
   - Assert: the `Config` sentence starts with `"the OS keyring could not be read: "` and contains `"infisical-write-mark"`. `builds == 0`.
5. `a_refused_mark_write_still_rebuilds_in_this_process` (D3)
   - Arrange: URL and identity, `offline()`, `refuse_fake_store(INFISICAL_WRITE_MARK_USER)`, and a latched `counting`. Add `refuse_fake_store` to the `htui_store::testkit` import.
   - Act: `first = provider()`; `serve_write(SetMachineIdentity(same))`; `second = provider()`.
   - Assert: `builds == 2` (through the generation) and the mark is still `None`.

`crates/htui/tests/secrets_settings.rs`, after `every_landed_keyring_write_answers_a_higher_generation` (`:352`):

6. `every_landed_keyring_write_stores_a_new_mark`
   - Arrange: `common::mock_keyring`, `offline("secrets-mark")`, `last = secret::get_infisical_write_mark()?` (must be `None`).
   - Act and assert, landed writes: for `[SetInfisicalUrl(STORED_URL), SetMachineIdentity(identity_entry(CLIENT_ID, SECRET)), ClearMachineIdentity, ClearInfisicalUrl]`, `keyring_written_of(serve(..), name)`. The mark is `Some` and differs from `last`.
   - Act and assert, refused writes, each leaving the mark equal to `last`:
     - `SetMachineIdentity(identity_entry(CLIENT_ID, "  "))` (blank half, `failed_of`)
     - `SetInfisicalUrl("http://192.168.1.10")` (normalisation)
     - `serve(&demo(), ..)` for all four (demo)
     - last, `common::refuse_fake_store(secret::INFISICAL_CLIENT_SECRET_USER)` then `SetMachineIdentity(..)` (keyring-refused; the D2 "mark last" pin)
7. `a_refused_mark_write_still_answers_written`
   - Arrange: `mock_keyring`, `common::refuse_fake_store(secret::INFISICAL_WRITE_MARK_USER)`, `offline("secrets-mark-refused")`.
   - Act: the four writes.
   - Assert: each reply is `keyring_written_of(reply, name)` (`SecretsWritten`); `generation_of` rises; `fake_machine_identity()` holds the pair after the set; the mark is still `None`.

**T1 hazards**

- **H-1 (order).** The mark is read first and written last. Both happen inside one `keyring_io()` guard, the write side in the same `spawn_blocking` closure as `write()`. Reversing either order makes a cross-process race miss a rebuild.
- **H-2.** Write the mark only after `write()?` succeeded. A failed write must leave the mark untouched; test 6's last case pins this.
- **H-3.** `note_keyring_write()` still runs when the mark write fails (D3). The TUI's rebuild relies on the generation.
- **H-4.** `infisical_field` ends in `unreachable!`. A missing arm panics the fake on the first mark read, so every `counting` test fails, not only the new ones.
- **H-5 (log content).** The `warn!` names the slot and the keyring error, which names `service/user` and the platform error, never a value. The mark is not secret, but don't log it either.
- **H-6.** Don't add the mark to `snapshot()` or to `SecretsSnapshot`. Lane C matches on `SecretsSnapshot` and `IdentityState`, so their shapes must not change in lane A.
- **H-7.** `Uuid::now_v7()` needs the `v7` feature (`Cargo.toml:59`, verified). Generate it inside the closure. No `Cargo.lock` change.

### T2: CLEAN-8 #7, typed half-stored identity

**htui-store `secret.rs`**, beside `HALF_STORED_IDENTITY` (`:471`). Keep the const; tests use it at `tests/secrets_settings.rs:292, 1458`.

```rust
/// What the keyring holds of the machine identity (CLEAN-8 #7).
#[derive(Debug)]
pub enum IdentityRead {
    /// Both halves.
    Stored(MachineIdentity),
    /// Neither half.
    NotStored,
    /// One half; `missing` is the absent half's user (`INFISICAL_CLIENT_ID_USER` or `…_SECRET_USER`).
    HalfStored { missing: &'static str },
}

/// The half-identity sentence, byte for byte what `get_machine_identity`'s error carries.
#[must_use]
pub fn half_identity_sentence(missing: &str) -> String {
    format!("{HALF_STORED_IDENTITY}: {SERVICE}/{missing} is missing; enter the identity again")
}

/// The identity, typed. `Err` is a keyring failure only.
pub fn read_machine_identity() -> Result<IdentityRead>   // today's body, with the std::mem::take move kept

pub fn get_machine_identity() -> Result<Option<MachineIdentity>> {   // the wrapper; same sentence
    match read_machine_identity()? {
        IdentityRead::Stored(identity) => Ok(Some(identity)),
        IdentityRead::NotStored => Ok(None),
        IdentityRead::HalfStored { missing } => Err(half_identity(missing)),
    }
}
```

`half_identity(missing)` becomes `StoreError::Backend(half_identity_sentence(missing))`.

**htui `secrets_settings.rs`**

- `snapshot` (`:227`): `identity: identity_state(secret::read_machine_identity())`.
- `identity_state` (`:251`) loses the `starts_with` sniff:

```rust
fn identity_state(read: Result<secret::IdentityRead>) -> IdentityState {
    match read {
        Ok(secret::IdentityRead::Stored(_)) => IdentityState::Stored,
        Ok(secret::IdentityRead::NotStored) => IdentityState::NotStored,
        Ok(secret::IdentityRead::HalfStored { missing }) => {
            IdentityState::HalfStored(secret::half_identity_sentence(missing))
        }
        Err(err) => IdentityState::Unreadable(seam_sentence(&err)),
    }
}
```

`secrets.rs::read_keyring` keeps calling `get_machine_identity`, so a walk's refusal is unchanged (`a_half_stored_identity_refuses_naming_the_keyring_only`).

**T2 red tests**

1. htui-store `machine_identity_tests`: `read_machine_identity_reports_the_missing_half_typed`
   - Arrange: `mock_keyring`; `super::write_slot(INFISICAL_CLIENT_ID_USER, "cid-1")`.
   - Assert: `matches!(read_machine_identity(), Ok(IdentityRead::HalfStored { missing: INFISICAL_CLIENT_SECRET_USER }))`.
   - Assert (byte-identical pin): `get_machine_identity()` is `Err(StoreError::Backend(m))` with `m == "the Infisical machine identity is half stored: htui/infisical-client-secret is missing; enter the identity again"` (a literal).
   - Then `clear_machine_identity`, write only the secret, and assert `HalfStored { missing: INFISICAL_CLIENT_ID_USER }`. An empty keyring is `NotStored`; both halves give `Stored(i)` with `i.client_id() == "cid-1"`.
2. htui `secrets_settings.rs` `mod tests` (unit, pure): `an_error_worded_like_a_half_identity_is_still_unreadable`
   - `let m = format!("{}: htui/infisical-client-id is missing", secret::HALF_STORED_IDENTITY);`
   - `assert_eq!(identity_state(Err(StoreError::Backend(m.clone()))), IdentityState::Unreadable(m));`
   - This compiles on both signatures, so it is red today because it returns `HalfStored`.
   - Add a companion: `identity_state(Ok(IdentityRead::HalfStored { missing: secret::INFISICAL_CLIENT_ID_USER }))` equals `HalfStored(<the literal sentence>)`.

**T2 hazards**

- **H-8.** The sentence must stay byte-identical. It is quoted in `docs/htui-secrets.md:64-66` and pinned by `tests/secrets_settings.rs:280-298, 1458` and `secrets.rs` `a_half_stored_identity_refuses_naming_the_keyring_only`. Build it in one place (`half_identity_sentence`), used by both `half_identity` and `identity_state`.
- **H-9.** `IdentityState` stays as it is (`HalfStored(String)`), because lane C matches on it. `StoreError` is untouched (`htui-mcp/src/tools/mod.rs:126`).
- **H-10.** Keep the zero-copy `std::mem::take(&mut *secret)` move inside `read_machine_identity`.

### T3: CLEAN-8 #4, identity stored trimmed

- `IdentityEntry::to_identity` (`secrets_settings.rs:129`): `MachineIdentity::new(self.client_id().trim(), self.expose_client_secret().trim())`. Pass the `&str` slices: no intermediate plain `String`.
- Docs:
  - `IdentityEntry::new` (`:107`): "the section trims them and refuses a blank one; `to_identity` trims again, so a worker write stores the trimmed halves".
  - `serve` (`:278`): note the trim.
- The blank check in `serve` (`trim().is_empty()`) is unchanged.

**T3 red test** (`tests/secrets_settings.rs`, beside `set_machine_identity_stores_both_halves_and_answers_a_fresh_snapshot`): `set_machine_identity_stores_the_trimmed_halves`

- Arrange: `mock_keyring`, `offline("secrets-trimmed")`.
- Act: `serve(SetMachineIdentity(identity_entry(" cid-typed-1\t", "  zq7-client-secret-0123456789 ")))`.
- Assert: `keyring_written_of(.., "set_machine_identity").identity == Stored` and `common::fake_machine_identity() == (Some(CLIENT_ID), Some(SECRET))`.

**Lane A gates** (after each task):
- `cargo test -p htui-store --features test-support secret`
- `cargo test -p htui --features testkit --lib secrets -- --test-threads=1`
- `cargo test -p htui --features testkit --test secrets_settings -- --test-threads=1`

Commits:
1. `fix(mod-90): a keyring write mark carries Settings writes to htui worker`
2. `refactor(clean-8): typed half-stored identity`
3. `fix(clean-8): the machine identity is stored trimmed`

---

## Lane B (worktree `mod-90-core`, serial T4 → T5 → T6)

**Files (confirmed; no others):** `crates/htui-secrets/src/infisical.rs`, `crates/htui-core/src/model/kind.rs`, `crates/htui-core/src/model/hierarchy.rs`, `crates/htui-core/src/secret.rs`, `crates/htui/src/hierarchy.rs`.

### T4: CLEAN-8 #1, `read_body` wiped

Beside `read_body` (`infisical.rs:692`):

```rust
/// L-7 (CLEAN-8 #1): appends `chunk` within `cap` without ever letting `buf` reallocate in place:
/// past its capacity it moves into a fresh zeroizing buffer of `max(need, 2 × capacity).min(cap)`,
/// and the old one is wiped as it drops. `false`, with `buf` untouched, when `chunk` would pass `cap`.
fn append(buf: &mut Zeroizing<Vec<u8>>, chunk: &[u8], cap: usize) -> bool {
    let Some(need) = buf.len().checked_add(chunk.len()).filter(|need| *need <= cap) else {
        return false;
    };
    if need > buf.capacity() {
        let grown = need.max(buf.capacity().saturating_mul(2)).min(cap);
        let mut next = Zeroizing::new(Vec::with_capacity(grown));
        next.extend_from_slice(buf);
        core::mem::swap(buf, &mut next); // `next` (the old buffer) drops wiped, spare capacity included
    }
    buf.extend_from_slice(chunk); // need <= capacity: never reallocates
    true
}

async fn read_body(endpoint: &'static str, mut response: reqwest::Response, cap: BodyCap)
    -> Result<Zeroizing<Vec<u8>>, SecretError>
```

- **Body of `read_body`:** the declared-length refusal is unchanged. Then:
  - `let presize = response.content_length().map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX)).min(cap.bytes);`
  - `let mut body = Zeroizing::new(Vec::with_capacity(presize));`
  - per chunk: `if !append(&mut body, &chunk, cap.bytes) { return Err(too_large(status)); }`
- **Call sites:**
  - `login` (`:375-378`): `login_refusal(body.as_deref().ok().map(Vec::as_slice))`.
  - `&body` passed to `accept_login`, `map_login_status`, `decode` and `map_list_status` deref-coerces to `&[u8]`, so nothing else changes.

**T4 red tests** (`infisical.rs` `mod tests`, `:891`):
1. `append_fills_a_presized_buffer_in_place`
   - Arrange: `Zeroizing::new(Vec::with_capacity(10))` and `p = buf.as_ptr()`.
   - Act: `append(&[1;4])`, then `append(&[2;6])`, with `cap` 10.
   - Assert: both `true`, `buf.as_ptr() == p`, and the bytes are `[1,1,1,1,2,…]`.
2. `append_grows_by_a_fresh_buffer_and_keeps_the_bytes`
   - Arrange: capacity 2.
   - Act: append `b"abc"`, then `b"de"`, with `cap` 64.
   - Assert: `&buf[..] == b"abcde"` and `buf.capacity() >= 5`.
3. `append_never_grows_past_the_cap`
   - Act: with `cap` 5, append `b"abc"` (capacity 0 → 3), then `b"de"`.
   - Assert: `true` and `buf.capacity() <= 5`.
4. `append_over_the_cap_refuses_and_changes_nothing`
   - Arrange: `b"abcd"` with `cap` 5.
   - Act: `append(b"xy")`.
   - Assert: `false`, `&buf[..] == b"abcd"`, and capacity unchanged.
   - Also `append(&[], 5)` on a full buffer is `true`.

**T4 hazards**

- **H-11.** Never call `extend_from_slice` or `reserve` on `buf` past its capacity. In-place growth leaves the old allocation unwiped, which is the whole bug.
- **H-12.** `need <= cap` is checked before growth, so `grown >= need` always holds. Keep `checked_add`; today's check is `chunk.len() > cap - len`, which is overflow-free.
- **H-13.** `Vec::with_capacity` guarantees "at least". Test 3's `<= cap` holds for `u8` in std today; if it ever flakes, relax it to `<= cap.next_power_of_two()`, not the algorithm.
- **H-14.** A 401 still never waits on `body?` (D5). Keep `let body = read_body(..).await;` before the 401 branch exactly as it is.

**Guard:** `crates/htui-secrets/tests/infisical.rs:1478-1530` (the four oversized-body tests).

### T5: CLEAN-8 #2 + #3, `htui-core` serde

- `kind.rs:341`: `fn present_option` becomes `pub(crate) fn present_option`.
- `ProjectPatch.secret` (`model/hierarchy.rs:113`):

```rust
/// … (existing doc) … On the wire an absent key is `None` and `null` is `Some(None)` (CLEAN-8 #2).
#[serde(
    default,
    deserialize_with = "crate::model::kind::present_option",
    skip_serializing_if = "Option::is_none"
)]
pub secret: Option<Option<SecretScope>>,
```

- `secret.rs`, beside the private `ScopeColumn` (`:53`):

```rust
/// [`ScopeColumn`] borrowed, for [`SecretScope::to_column`]: same fields, same order, no clone.
#[derive(Serialize)]
struct ScopeColumnRef<'a> {
    project_id: &'a str,
    environment: &'a str,
    path: &'a str,
}
```

  `to_column` becomes `serde_json::to_string(&ScopeColumnRef { project_id: &self.project_id, environment: &self.environment, path: &self.path })`. `ScopeColumn` and the `try_from`/`into` derive on `SecretScope` stay as they are.

**T5 red test** (a new `#[cfg(test)] mod tests` at the end of `htui-core/src/model/hierarchy.rs`; the file has none today): `project_patch_secret_round_trips_all_three_states`

- Mirror of `kind.rs:680`.
- Cases: `(None, "{}")`, `(Some(None), r#"{"secret":null}"#)`, `(Some(Some(SecretScope::new("p1","dev","/")?)), r#"{"secret":{"project_id":"p1","environment":"dev","path":"/"}}"#)`.
- For each case:
  - Build `ProjectPatch { secret, ..Default::default() }`.
  - Assert `from_str(to_string(patch)) == patch`.
  - Assert `from_str(literal).secret == secret`.
- Red today: `Some(None)` reads back as `None`.

**T5 hazards**

- **H-15.** All three attributes are required.
  - Without `default`, an absent key is an error.
  - Without `skip_serializing_if`, `None` serializes to `null`, which now reads back as `Some(None)`. That is the same bug, mirrored.
- **H-16.** The `to_column` bytes must be identical. Guards: `scope_column_is_compact_and_ordered`, `to_column_is_unchanged`, `a_scope_serialises_as_its_column`, `scope_round_trips_through_the_column` (`secret.rs:802-860`). Field order must stay `project_id, environment, path`.
- **H-17 (D5).** Don't touch `RepoPatch.remote_url` or `ItemPatch.step_graph_id`. Record them in the close-out.

### T6: CLEAN-8 #6, one tree re-read (pure refactor, `crates/htui/src/hierarchy.rs`)

- `fresh_tree` (`:493`) is unchanged and is the only place that builds `NotFound { entity: "workspace", id: ws.to_string() }`.
- `reread` (`:455`): `Ok(StoreReply::Hierarchy(Some(Box::new(fresh_tree(writer, ws, this_box).await?))))`.
- `cas` (`:470`):

```rust
let tree = Box::new(fresh_tree(writer, ws, this_box).await?);
Ok(match outcome {
    CasOutcome::Applied(_) => StoreReply::Hierarchy(Some(tree)),
    CasOutcome::Stale(_) => StoreReply::HierarchyStale(tree),
})
```

- `infer` (`:577`, `:654`): `let tree = fresh_tree(writer, ws, this_box).await?;` and `let fresh = fresh_tree(..).await?;`. The `box_id(this_box)?` check stays first.
- After the change, `search text 'entity: "workspace",'` in `crates/htui/src` must return exactly one hit.

**T6 hazards**

- **H-18.** The error shape is exact. `entity` stays `"workspace"`, `id` stays `ws.to_string()`, and there is still one read per write.
- **H-19.** In `infer`, the box refusal must still come before any read.

**Guards:** `tests/hierarchy.rs:298, 1018` and `tests/secrets_settings.rs:659, 719, 2529`. Run them on this branch; they belong to lane A and stay unedited here.

**Lane B gates:**
- `cargo test -p htui-secrets`
- `cargo test -p htui-core --features test-support`
- `cargo test -p htui --features testkit --lib hierarchy -- --test-threads=1`
- `cargo test -p htui --features testkit --test hierarchy --test secrets_settings -- --test-threads=1`

One commit per task.

---

## Lane C (worktree `mod-90-ui`, T7)

**Files (confirmed):** `crates/htui/src/ui/tabs/settings/secrets.rs` only. Lane A keeps `IdentityState`, `UrlState`, `SecretsSnapshot` and `StoreReply` unchanged (H-6, H-9), so nothing couples the two.

### T7: CLEAN-8 #5 + #8 (pure moves)

```rust
/// How many rows: the four fixed, then one per project of the tree.
fn row_count(&self) -> usize { Row::FIXED.len() + self.tree.as_ref().map_or(0, |tree| tree.projects.len()) }

/// Row `index` in cursor order; `index < row_count()`.
fn row_at(&self, index: usize) -> Row {
    match Row::FIXED.get(index) {
        Some(row) => *row,
        None => Row::Project(index - Row::FIXED.len()),
    }
}
```

- `rows()` (`:406`) is deleted.
- `row()`: `self.row_at(self.cursor.min(self.row_count() - 1))`.
- `move_cursor` and `clamp_cursor`: `self.row_count() - 1`.

`on_reply` (`:1348`) delegates:

```rust
StoreReply::Hierarchy(None) | StoreReply::SecretsTree(None) => self.on_tree_gone(),
StoreReply::Failed { request, message } => self.on_failed(request, message, ctx),

/// `tree = None`, `clamp_cursor`, `closed_if_gone`, in that order.
fn on_tree_gone(&mut self)

/// The four Failed arms in today's guard order (A-1); any other request is ignored.
fn on_failed(&mut self, request: &'static str, message: &str, ctx: &Ctx<'_>)
```

`on_scope_written` (`:1159`): the `ScopeWrite::Stale` arm body moves to:

```rust
/// A stale scope write: refresh the open form's token, close it if the project left, or say so.
fn on_scope_stale(&mut self, project: ProjectId, tree: &HierarchySnapshot)
```

`lines` (`:919`) keeps the fixed rows and the blank line, then:

```rust
/// The `Projects` heading and one row per project, or `NO_WORKSPACE`.
fn project_lines(&self, lines: &mut Vec<Line<'static>>, width: u16, theme: &Theme)

/// The guide under the rows: `LATCHED`, else `HEALTH_GUIDE` on Health, else none.
fn guide(&self) -> Option<&'static str>
```

**T7 unit test** (`mod tests`, `#[tokio::test]`): `row_at_walks_the_fixed_rows_then_the_projects`

- Arrange: `let tree = crate::hierarchy::snapshot(&MemStore::demo(), htui_core::fixtures::ids::WORKSPACE_GRAPHICS, None).await.unwrap().unwrap();` and a section with `tree: Some(tree.clone())`.
- Assert: `(0..row_count()).map(row_at)` equals `FIXED` chained with `(0..n).map(Row::Project)`, and `n > 0`.
- Then `tree: None`: the walk is exactly `FIXED`.

**T7 hazards**

- **H-20.** `row_at` must not use `FIXED.get(i).copied().unwrap_or(Row::Project(i - 4))`. `unwrap_or` evaluates `i - 4` eagerly, which underflows for `i < 4` and panics in debug. Use `match` or `unwrap_or_else`.
- **H-21.** `on_failed` keeps the guard order and side effects exactly:
  - `READ_NAME` sets `unavailable` only.
  - The write arm re-reads the tree first for a scope write, then `busy = None`, closes only a question, then `refuse`.
  - The checks clear `checking` only when it matches.
- **H-22.** `project_lines` must keep `style_of(Row::FIXED.len() + index)`. The cursor style is computed from the absolute index.
- **H-23.** The six `secrets_settings__*.snap` files must stay byte-identical. Run `cargo insta test … --check`; never `accept`.

**Lane C gates:**
- `cargo test -p htui --features testkit --lib settings::secrets -- --test-threads=1`
- `cargo insta test -p htui --features testkit --check --test secrets_settings -- --test-threads=1`
- `cargo clippy -p htui --all-targets --all-features -- -D warnings`

---

## Lane D (worktree `mod-90-qdrant`, T8)

**Files (confirmed, with A-2):** `crates/htui/src/qdrant_settings_info.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/settings/qdrant.rs`, `crates/htui/tests/settings.rs`. `secrets_settings::DEMO_SESSION` is read, never edited. `htui_store::{Backend, Started}` are already a test dependency (`tests/chat.rs:1852`).

### T8: CLEAN-8 #9

**`qdrant_settings_info.rs`**

```rust
pub enum QdrantState {
    Stored,
    NotStored,
    Unreadable(String),
    /// `Backend::Memory`: no keyring consulted (CLEAN-8 #9, D6).
    NotApplicable,
}

/// CLEAN-8 #9: the four Settings > Qdrant requests, for `try_serve` (the loop's `other` arm,
/// the harness and `--demo` alike). On `Backend::Memory` the read is `NotApplicable` and every
/// write is refused with `DEMO_SESSION` before the keyring is reached.
///
/// # Errors
/// A keyring call's `StoreError::Backend`; `Backend` for a request that is not one of the four.
pub(crate) async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>
```

- Imports: `htui_store::Backend` and `crate::store_worker::{StoreReply, StoreRequest}`.
- Body, in this arm order:
  1. demo `QdrantInfo` → `StoreReply::Qdrant(QdrantSnapshot { url_state: NotApplicable, key_state: NotApplicable, url_summary: None })`
  2. demo writes → `StoreReply::Failed { request: request.name(), message: crate::secrets_settings::DEMO_SESSION.to_owned() }`
  3. then today's four arm bodies verbatim, with `if let Err(err) = res { failed(..) }` becoming `blocking_keyring(..).await?;`
  4. `other` → `Err(StoreError::Backend(format!("not a qdrant request: {}", other.name())))`

**`store_worker.rs`**

- `try_serve` (`:2160-2166`): `… | StoreRequest::ClearQdrantSettings => crate::qdrant_settings_info::serve(backend, request).await?,`. Replace the comment with "the loop, the harness and `--demo` all serve these here (CLEAN-8 #9)".
- The loop: **delete** the four arms (`:2951-2997`), per A-2.

**`ui/tabs/settings/qdrant.rs`**

- `const DEMO_ROW: &str = "n/a in a demo session";`
- `use crate::secrets_settings::DEMO_SESSION;`
- `fn demo(&self) -> bool { self.state() == Some(&QdrantState::NotApplicable) }`
- `blocked()`: after the busy check, `if self.demo() { self.refuse(DEMO_SESSION.to_owned()); return true; }`
- `render`: add `Some(QdrantState::NotApplicable) => DEMO_ROW.to_owned()` to `url_val` (`:450`) and `key_val` (`:463`). The guide match at `:508` has `_` and is unreachable in a demo.
- `on_snapshot`'s auto-open stays keyed on `NotStored`, so `NotApplicable` opens nothing.
- The hint is unchanged, mirroring the Secrets section, which also shows `e edit` in a demo and refuses on the key.

**T8 red tests** (`tests/settings.rs`, after `a_qdrant_reload_just_before_a_write_is_not_taken_as_its_answer`)

1. `qdrant_demo_reads_no_keyring` — no keyring guard on purpose (as `secrets_settings.rs:209`).
   - `store_worker::serve(&Backend::memory(MemStore::demo()), &QdrantInfo)` is `Qdrant(s)` with both states `NotApplicable` and `url_summary == None`.
   - Then the A-2 loop path: `htui::store_worker::spawn(htui_store::Started::detached(Backend::memory(MemStore::demo())), req_rx, rep_tx)`. Send `RequestEnvelope { seq: 1, origin: Origin::App, request: QdrantInfo }`. The reply is the same `NotApplicable` snapshot.
   - Red today: `Failed "handled in worker loop"` and a real keyring read.
2. `qdrant_demo_refuses_every_keyring_write`
   - For `[SetQdrantUrl("https://q.example:6334"), SetQdrantApiKey(Redacted::new("qk-typed-123".into())), ClearQdrantSettings]`, `store_worker::serve(&demo, &r)` is `Failed { request: r.name(), message: DEMO_SESSION }`.
   - Repeat through the spawned loop for one write.
3. `qdrant_demo_section_offers_no_edit` (`SectionBench`)
   - Act: reply with the `NotApplicable` snapshot.
   - Assert:
     - `!captures_input()` (no auto-opened editor)
     - the render contains `"n/a in a demo session"`
     - for `"e"` and `"c"`: `Handled::Consumed`, `requests_of` is empty, `!captures_input()`, and the render contains `DEMO_SESSION`
     - `"r"` still sends `[QdrantInfo]`

**T8 hazards**

- **H-24.** The new variant breaks exhaustive matches only at `qdrant.rs` `render` (`:450, :463`). The references query shows no other exhaustive match (`settings.rs:5533, 5618` and `secrets_settings.rs:198` construct it, and `on_snapshot` uses `==`).
- **H-25.** Delete the loop arms, don't duplicate them. Otherwise `--demo` keeps the unguarded path and only the harness is fixed, which is exactly the gap the plan marked unverified.
- **H-26.** Failure text stays identical: `failed(request.name(), &err)` in `serve` and in the loop's `other` arm. `request.name()` for the four is `qdrant_info`, `set_qdrant_url`, `set_qdrant_api_key` and `clear_qdrant_settings`, as the loop hard-coded. `ClearQdrantSettings` keeps `clear_qdrant_url()?; clear_qdrant_api_key()` in one blocking call.
- **H-27 (A-3).** The offline harnesses now run `fetch()` on Settings open. Run the full `cargo insta test --check` and the offline Settings tests (`box_settings`, `hierarchy`, `keys`, `kinds`, `personas`, `prompt_settings`, `connection`).
- **H-28 (D6).** Leave the concepts paths' Qdrant reads (`concepts_worker.rs:220`, `concepts.rs:97/334`) alone.

**Lane D gates:**
- `cargo test -p htui --features testkit --lib qdrant -- --test-threads=1`
- `cargo test -p htui --features testkit --test settings --test secrets_settings -- --test-threads=1`
- the full `cargo insta test -p htui --features testkit --check -- --test-threads=1`
- the two clippy gates

---

## T9 (primary tree, after all merges): `docs/htui-secrets.md`

| Anchor | Edit |
|---|---|
| Keyring entries `:52-70` | "three entries" becomes "four"; add the row `htui/infisical-write-mark` (a random value replaced by every Settings > Secrets URL or identity write or clear); add one sentence saying it lets another htui process (`htui worker`) see a write |
| Identity form `:94-97` | after "Both halves are required.": "Spaces around either half are dropped." |
| Qdrant demo `:141-142` | replace "Settings > Qdrant is not guarded…" with: in a demo session Settings > Qdrant reads `n/a in a demo session`, refuses `e` and `c`, and sends nothing |
| Provider rebuild `:247-252` | "or, in the TUI, after any URL or identity write" becomes "or after any URL or identity write in Settings, in this process or another" |
| What is wiped `:405-414` | add: the Infisical response body buffer, wiped as it grows and when it drops |
| Logins `:444-448` | drop "restart it": an `htui worker` sees the write at its next walk; restart it only if the warning said the write mark could not be stored |

The close-out follows the plan: HANDOFF `:435, :446, :476-477`, `docs/decisions/mod/mod-90.md`, CLEAN-8 resolved, `mod-10.md:121, 157-158`, then the validator. Record:
- A-1 to A-5
- H-27 (A-3)
- D5's two latent fields
- D6's unguarded concepts reads

## Data flow (after T1)

1. **Settings write in the TUI.** `serve` → `keyring_write` runs on a blocking thread. It takes `keyring_io()`, runs `write()` (URL or identity slots), then `set_infisical_write_mark(uuid v7)` (best effort, `warn!` on failure), then `note_keyring_write()`. The reply is `SecretsWritten { generation, snapshot }`, and the snapshot does not read the mark.
2. **Any process's provider.** `KeyringInfisical::current` loads `generation`, then `read_keyring` on a blocking thread takes `keyring_io()` and reads mark, then URL, then identity, returning `KeyringRead`.
3. **Cache check.** `built_from(generation, &read, digest)`. Any difference (generation in process, mark across processes, URL, client ID, or secret digest) builds a new provider without the latch. No difference reuses it, and the latch holds.
4. **The worker.** `htui worker` has no generation bump from the TUI, so the mark alone carries the write. An absent mark is `None`, which is a stable key.

## Hazards across lanes

- **H-29 (worktrees).** Gortex `edit`/`refactor` writes to the primary checkout. Lane B, C and D implementers must edit their worktree paths with anchored scripted replacements (memory: workflow-worktree-implementers, gortex-in-hr-sandbox). Each worktree builds its own `target/` (about 10 GB), so check `df -h .` first.
- **H-30.** `cargo test -p htui` without `--features testkit` runs 0 integration tests and still reports ok. Keep `--test-threads=1` throughout: the keyring fake is process-wide.
- **H-31.** Run the featureless clippy gate (`cargo clippy --workspace -- -D warnings`) on each lane. `--all-features` can hide dead code.

## Build sequence and merge order

1. **Lane A, primary tree, on `hr/MOD-90`:**
   - T1: htui-store constants, slot and API, then the test, then `KeyringRead`, `Cached` and `read_keyring`, then `keyring_write`. Tests go in red first. Commit.
   - T2: `IdentityRead` and `half_identity_sentence`, then `identity_state`. Commit.
   - T3. Commit.
2. **Lanes B, C and D, in parallel with A,** each in `git worktree add ../htui-mod-90-{core,ui,qdrant} -b mod-90-{core,ui,qdrant} 62a8735a`:
   - B: T4 → T5 → T6, one commit each.
   - C: T7.
   - D: T8.
3. **Merges into `hr/MOD-90`,** once lane A's three commits have landed. Each is `--no-ff`, followed by that lane's gate on the merged tree:
   1. `mod-90-core` (B)
   2. `mod-90-ui` (C, whose guards include lane A's `tests/secrets_settings.rs`)
   3. `mod-90-qdrant` (D, last because it has the widest test effect, A-3)

   The file sets are disjoint, so no conflicts are expected. A conflict means a lane strayed outside its set: stop and report it.
4. **Full validation** on the merged tree (the plan's block, using A-4's gate form). Then remove the worktrees before `git branch -d`.
5. **T9 docs, then the close-out.**
