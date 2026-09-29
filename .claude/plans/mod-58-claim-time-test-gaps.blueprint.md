# Blueprint: MOD-58 — two claim-time test gaps

**Architect: code-architect. Read-only except this file. No cargo command was run.**

**State note, read this first.** The brief said the worktree sits at `c9e24c9`. It does not any
more. As of this blueprint the branch is `mod-58` at **`6033d4f`**, on top of **`3dced4a`
"test(mod-58): pin claim-time tag check: target box first, no item never refused"**. The working
tree is clean apart from this blueprint.

While I was verifying, the tree moved twice, and both moves matter:

1. When I started, `mem.rs` carried an **uncommitted** hunk moving the target-box check below the
   tag computation — the T1 case-1 mutation. It has since been reverted. The production half of
   `mem.rs` is now byte-identical to `68c058f`.
2. That hunk was the **weaker** form of the mutation — the predicate hoisted into a `let
   not_claimable` binding, with `return Ok(Claim::NotClaimable)` placed after the *computation* of
   `missing` but still **before** the `if !missing.is_empty() { … }` write block. It is a false
   green: `MissingTags` still cannot be reached, so the case still passes while the rule is
   arguably broken. Commit `6033d4f` ("docs(mod-58): sharpen the case-1 mutation in the plan")
   records this and tightens the plan's wording. **My independent read reached the same conclusion
   by a different route** — see "Case 1's mutation" under Task 1 below — which is corroboration
   that the sharpened rule is the right one. The rest of the plan is unchanged by `6033d4f`, so
   every correction below still applies as written.

Consequences:

- **T1 is already implemented and committed** (`3dced4a`, 173 lines in `mem.rs`). I have verified it
  below against the plan and against every name the plan names. It is correct.
- **T0 is not started** — `crates/htui-store/tests/pg_criteria.rs`'s last commit is still
  `a9d2908` (MOD-7). The corrected T0 step list is the actionable half of this document.

Everything below was read directly from `/media/projects/htui-mod-58`.

**Gortex.** The daemon's only indexed checkout is `/home/mluigi/projects/htui` at head `614ac2f`
on branch `mod-9-m3` — a different tree, a different branch, and the primary checkout this task
forbids reading. There is no view selector for this worktree, and I may not start a daemon or
`track` a new one. So every fact here comes from direct reads of the worktree. (The Gortex
`PreToolUse` hook fires on every shell grep in this session; the worktree path is the binding
instruction and graph queries would have served the wrong code.)

---

## Corrections to the plan

Six discrepancies. C1 and C3 are the ones that would actually have cost an implementer time; none
of them invalidates the plan's design.

### C1 — `UNIQUE (user_id, hostname)` no longer exists

- **Plan says** (D2, and T0 case 1 step 1): "`UNIQUE (user_id, hostname)` (`0001_init.sql:73`) is
  why the hostname must differ"; Verified-claims #10 repeats it as "verified".
- **Reality**: `crates/htui-store/migrations/0005_box_identity.sql:15` is
  `ALTER TABLE box DROP CONSTRAINT box_user_id_hostname_key;`. The constraint is **dropped** in the
  live schema. `0001_init.sql:73` is correct only as history.
- **Fix**: keep `hostname = 'elsewhere'` — both live precedents do it
  (`connect.rs:146-150`, `cache.rs:1863-1866`) and it costs nothing — but drop the reason. A test
  that inserted a duplicate hostname would *not* trip a constraint. The distinct hostname is now
  only a readability measure.

### C2 — `race_item` takes two arguments and hard-codes empty `required_tags`

- **Plan says** (T0 case 1 step 2): "Mint an item with `required_tags: ["cuda"]` (`race_item`,
  `:64`, with the tags set)".
- **Reality**: `crates/htui-store/tests/pg_criteria.rs:64` is
  `fn race_item(kind_id: ItemKindId, title: &str) -> NewItem`, and it sets
  `required_tags: Vec::new()` at `:71`. There is no one-argument form and no tag parameter.
- **Fix**: override the field —
  `NewItem { required_tags: vec!["cuda".to_owned()], ..race_item(kind, "needs a GPU toolchain") }` —
  and **supply the `kind_id`**, which the plan never mentions. Either `ids::KIND_HTUI_FEAT` (the
  fixture kind, and what the committed MemStore case uses) or `race_kind(&db.pool).await`
  (`pg_criteria.rs:47`, mints a fresh `RACE` kind). `State::mint` (`mem.rs:1221-1229`) and the
  `item_kind` FK both refuse a kind that does not exist or belongs to another project, so this is
  a real `Constraint`/`23503` if omitted.

### C3 — the Postgres D94 mutation is not expressible as written

- **Plan says** (Test plan): "T0, case 2: swap `items.get` for `require_item` (`mem.rs:3612`) and
  the join for an inner join that demands a row (`pg/write.rs:3075`). The case must fail with
  `NotFound { entity: "item" }` / a non-empty tag list."
- **Reality**: `pg/write.rs:3075` **is already an inner join** — `JOIN item i ON i.id = r.item_id`.
  There is no "left" form to swap away from, and no `require_item` equivalent anywhere in the SQL
  half. D94 on Postgres is not held by a *shape* that can be inverted; it is held by the *absence*
  of a row-level demand.
- **Fix — the mutation that actually bites on the Postgres side**: add a `require_item`-equivalent
  between the claimability check (`:3063`) and the tag query (`:3071`), e.g. read the run's
  `item_id` and answer `Err(StoreError::NotFound { entity: "item", .. })` (or
  `Claim::NotClaimable`) when it is `NULL`. The case then fails on the admitted assertion. Both
  variants are legitimate; the `NotFound` one mirrors `MemStore` exactly and is the better
  demonstration. The **MemStore** half of the plan's mutation (`items.get` → `require_item` at
  `mem.rs:3612`) is correct as written and needs no change.

### C4 — T0 is missing the "the tag rule is armed" assertion

- **Plan says** (T0 case 1 step 3): assert `NotClaimable`, the run row unchanged, the item still
  `queued`. Nothing proves the tag rule was actually engaged.
- **Why it matters**: if the raw insert or the `required_tags` override silently failed to arm the
  tag rule, the run would still answer `NotClaimable` — **for the wrong reason**, and the pin would
  be worthless. This is precisely hazard the committed MemStore case already closes: `mem.rs:7928-7935`
  asserts `store.missing_tags(item, ids::BOX) == ["cuda"]` with the message "the tag rule is armed".
- **Fix**: add the same assertion to T0 case 1, between the mint and the claim:
  `db.store.missing_tags(item, ids::BOX).await` must equal `vec!["cuda".to_owned()]`. This is the
  only assertion in either store's case 1 that fails when the *other* half of the rule breaks, and
  it costs one line.

### C5 — `claim_run` has two signatures; the plan's five-argument call is the trait one

- **Plan says** (T1 case 1 step 5): `claim_run(run, ids::BOX, owner, at, until)`.
- **Reality**: `WriteStore::claim_run` (`traits.rs:873-880`) takes exactly those five;
  `MemStore`'s private inherent `State::claim_run` (`mem.rs:3586-3595`) takes six — it adds `now`
  last. Both stores' `PgStore::claim_run` (`pg/write.rs:3027-3033`) takes the trait's five.
- **No change needed** — the plan's call site is the right one. Recorded so an implementer reading
  `mem.rs:3586` does not think the plan dropped an argument.

### C6 — `#[tokio::test]` vs the file's convention in `pg_criteria.rs`

- **Plan says**: "two `#[tokio::test]` cases" in T0.
- **Reality**: `pg_criteria.rs` has 35 `#[tokio::test(flavor = "multi_thread")]` and **zero** plain
  `#[tokio::test]`. A plain one compiles and runs fine against a pool; this is style only.
- **Fix**: use `#[tokio::test(flavor = "multi_thread")]` to match the file. (The `mod tests` block in
  `mem.rs` uses plain `#[tokio::test]` throughout, so the plan's T1 phrasing is right there.)

### Also checked, no correction

- `Mint_item` is **not** a type and is not needed. The shape is `store.mint_item(NewItem { … })`:
  `WriteStore::mint_item` (`mem.rs:5281`) → `State::mint` (`mem.rs:1220`). The precedent the plan
  points at (`mem.rs:8997-9013`) is field-for-field identical to what the committed case uses.
- `graph_run` (`mem.rs:7625`) is `fn graph_run(item: ItemId, project: ProjectId, scope: Vec<RepoId>)
  -> NewRun`, and hard-codes `target_box_id: ids::BOX` and `started_by: ids::USER`. Struct-update
  syntax on a function-call result (`NewRun { target_box_id: other, ..graph_run(…) }`) is valid
  Rust and is what the committed case does.
- `race_run` (`pg_criteria.rs:902`) is `fn race_run(item: ItemId) -> NewRun`, `repo_scope` empty.
- `common::demo_db()` / `db.drop_db()` / `db.store` / `db.pool` all exist
  (`testkit.rs:170`, `:196`, `:41`, `:43`); `db.store` is a `PgStore`, `db.pool` a `PgPool`.
- `MemStore::write` (`mem.rs:743`) and `State.boxes` (`mem.rs:91`) are exactly where the plan says,
  and the child `mod tests` (`mem.rs:5878`, `#[cfg(all(test, feature = "test-support"))]`) reaches
  both. `State.runs: HashMap<RunId, Run>` (`mem.rs:152`) and `Run.item_id: Option<ItemId>`
  (`model/run.rs:172`) — so `s.runs.get_mut(&run).unwrap().item_id = None` compiles.
- `BoxRow` is `crates/htui-core/src/model/box_.rs:27`, `Debug + Clone + PartialEq`. The tests module
  does **not** import it and does not need to: the row is obtained by value from
  `store.box_row(ids::BOX)`, whose type is inferred.
- `NewRun.item_id` is a non-optional `ItemId` (`model/run.rs:359`) — D94 is unreachable through
  `create_run`, as the plan says.
- `Claim` is `PartialEq + Eq` (`model/overlap.rs:129`), so `assert_eq!` works.
- `missing_tags_failure` is `crate::model::overlap::missing_tags_failure`
  (`model/overlap.rs:189`); `State::require_item` is `mem.rs:3355`. Neither is needed by name in
  these four cases.
- `.sqlx` is still 268; migrations are still `0001`..`0007`; no trigger fires on `INSERT INTO run`
  (`0001_init.sql:578` creates BEFORE UPDATE triggers only, and `0006` touches only `item`).

---

## Task 1 — `MemStore`: **implemented, committed, verified**

`3dced4a`, `crates/htui-core/src/store/mem.rs:7889-7978` (case 1) and `:7980-8060` (case 2).
The plan's steps 1-6 are all present and each one does what D3 and D6 asked:

| Plan step | As implemented | Verdict |
|---|---|---|
| 1. `MemStore::demo()` | `mem.rs:7895` | ✓ |
| 2. Plant the second box | `7899-7909`: `BoxId::new()`, `store.box_row(ids::BOX)`, `hostname = "elsewhere"`, inserted through `store.write` | ✓ |
| 3. Mint `required_tags: ["cuda"]` | `7911-7927`, the `mem.rs:8997` precedent verbatim with `ids::KIND_HTUI_FEAT` | ✓ |
| 4. `create_run(NewRun { target_box_id: other, ..graph_run(…) })` | `7936-7942` | ✓ compiles |
| 5. `Claim::NotClaimable` | `7949-7962` | ✓ |
| 6. Run row unchanged, item still `queued`, `target_box_id == other` | `7943-7946`, `7963-7977` | ✓ |
| D94 arm 1, `item_id: None` | `7990-8016` | ✓ |
| D94 arm 2, item row gone | `8020-8059`, on a separately minted item as the plan requires | ✓ |

Two things it does that the plan did not ask for, both of them improvements worth keeping and both
of which C4 asks T0 to copy:

- `7928-7935`: `missing_tags(item, ids::BOX) == ["cuda"]` — the "tag rule is armed" guard.
- `8013-8016`: `claimed.item_id == None` after admission — the run really is the one that was claimed.
- `8027`: the orphaned item is minted with `required_tags: ["cuda"]`, so the arm would fail
  loudly under either half of an inverted rule.

**The line trace the brief asked for** — `State::claim_run` (`mem.rs:3586-3702`) for a `queued`
run, `target_box_id == ids::BOX`, so no step panics, errors, or refuses:

1. `require_run(run)` — present, `Ok`.
2. `boxes.contains_key(&ids::BOX)` — the fixture row, present.
3. `:3602` status `Queued` and `target_box_id == box_id` → falls through (this is the check the
   live mutation has moved).
4. `:3610-3614` `claimed.item_id.and_then(|item| self.items.get(&item))`:
   - **arm 1, `item_id: None`** → `and_then` short-circuits to `None` → `unwrap_or_default()` →
     empty `missing` → step 5 skipped.
   - **arm 2, `Some(x)` with no `x` in `self.items`** → `items.get` yields `None` → same empty
     `missing` → step 5 skipped. No `require_item`, so no `NotFound`; nothing else in the function
     dereferences `item_id`.
5. `:3640-3663` slot count: `live` = runs with `executing_box_id == Some(ids::BOX)` and status
   `Running | AwaitingApproval`. `running` is 0 (arm 1) then 1 (arm 2) against the fixture box's
   `max_concurrent_items: 2` (`fixtures.rs:474`) → not `SlotFull`.
6. `:3664-3683` overlap: both runs carry `repo_scope: Vec::new()`, and the filter is
   `row.repo_scope.iter().any(|repo| claimed.repo_scope.contains(repo))` — an empty scope shares no
   repo, so `live` is empty (hazard H-10) → not `Overlaps`. `scope_of` on the run's snapshot is
   total on any input (`model/overlap.rs:112-124`: a missing `scope` key yields
   `RunScope::conservative`), so even a malformed snapshot cannot panic here.
7. `:3685-3694` admitted: `status = Running`, `executing_box_id = Some(ids::BOX)`,
   `started_at`/`lease_box_id`/`lease_expires_at` set; `lease_owners.insert(run, owner)`.
8. `:3695-3701` item transition guarded on `claimed.item_id` **and** on the item being `Queued`.
   Arm 1: `item_id` is `None` → skipped. Arm 2: the row is gone → `items.get` is `None` → skipped.
   Both are silent no-ops, which is why the run is admitted and no `Err` escapes.
9. `Ok(Claim::Admitted)`.

Nothing here writes a `BoxRow`, an item or a second run, so the two arms cannot interfere. Arm 1
strands the fixture item `HTUI_ANA_2` at `queued` — harmless, and the comment at `8018-8019`
explains why arm 2 mints its own item instead of reusing it.

**Mutation check for T1.** Case 1's mutation is already done and the plan has been corrected
because of it. The faithful mutation moves the whole
`if claimed.status != … || claimed.target_box_id != box_id { return Ok(Claim::NotClaimable); }`
**below the entire `if !missing.is_empty() { … return Ok(Claim::MissingTags { missing }); }` block
(`mem.rs:3615-3637`)** — not merely below the `let missing = …` computation. With the return
placed after the *computation* but still before the write block, `MissingTags` remains unreachable,
nothing is written, and the case passes: a false green, not a mutation. That is the weaker form
the implementer first applied and then caught (`6033d4f`), and it is the failure mode to avoid on
the `PgStore` side, where the equivalent block is `pg/write.rs:3089-3110`.

Under the faithful mutation the case must fail at `mem.rs:7960` with
`Claim::MissingTags { missing: ["cuda"] }`, and the D3 assertions at `:7963-7977` must fail too (the
run is `failed` with a `failure` string and the item is `blocked`). Case 2's mutation
(`items.get` → `require_item` at `:3612`) makes **both** arms answer
`Err(NotFound { entity: "item" })` — arm 1 too, since `require_item` on a `None` id is the natural
way to write it. Report the observed failure line for each of T1's two cases.

---

## Task 0 — `PgStore`: corrected step list

**File**: `crates/htui-store/tests/pg_criteria.rs`, inserted after
`finish_run_holds_the_item_while_another_run_is_live` (ends `:3676`), so the new cases begin at
`:3677`.

**Boilerplate for both cases** (the file's own, confirmed at `:3557` and `:3675`):

```rust
#[tokio::test(flavor = "multi_thread")]            // C6: 35/35 in this file
async fn …() {
    let Some(db) = common::demo_db().await else { return; };
    …
    db.drop_db().await;                             // last line of both cases
}
```

`demo_db` loads the fixture and then **deletes the seeded `app_user` and its box**
(`testkit.rs:178-187`), leaving `ids::USER` as the only `app_user` and `ids::BOX` as the only box
with `max_concurrent_items: 2` (`fixtures.rs:474`). The fixture holds one `Done` and one `Queued`
run (`fixtures.rs:1400-1429`) and zero `running` ones, so both cases start with a free box and an
empty overlap set. A `queued` fixture run takes no slot and shares no repo, so it is inert here.

### Case 1 — `a_missing_tags_run_aimed_at_another_box_is_not_claimable`

1. Plant the second box with the untyped form, copying `connect.rs:146-150` exactly (it is a
   working precedent against the live schema, `cache.rs:1863-1866` is the second):

   ```rust
   let other = BoxId::new();
   sqlx::query(
       "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version) \
        VALUES ($1, (SELECT id FROM app_user ORDER BY created_at, id LIMIT 1), 'elsewhere', \
                'linux', '', 'x86_64', '0.0.0')",
   )
   .bind(other.as_uuid())
   .execute(&db.pool)
   .await
   .expect("plant the second box");
   ```

   Column list is complete and valid: `id`, `user_id`, `hostname`, `os_family`, `os_version`,
   `arch`, `htui_version` are the seven `NOT NULL`-without-default columns of `0001_init.sql:53-74`;
   `cpu`, `gpu_present`, `probed_tags`, `declared_tags`, `quirks`, `settings` all default;
   `ram_mb`, `gpu_vendor`, `last_probed_at`, `machine_fingerprint` are nullable
   (`0005_box_identity.sql:17-18`). `os_family = 'linux'` satisfies the `CHECK … IN
   ('windows','linux','macos')` at `:57`. The `app_user` sub-select resolves to `ids::USER`.
   **Keep `'elsewhere'` — but see C1: the `UNIQUE (user_id, hostname)` that made it necessary was
   dropped in `0005`; it is now cosmetic.** Untyped, so `.sqlx` stays 268 (confirmed: 268).

2. Mint the item. `race_item` needs a kind id (**C2**):
   `let kind = race_kind(&db.pool).await;` or `ids::KIND_HTUI_FEAT`. Then
   `let item = db.store.mint_item(NewItem { required_tags: vec!["cuda".to_owned()], ..race_item(kind, "needs a GPU toolchain") }).await.expect("the mint lands").id;`

3. **Assert the tag rule is armed (C4 — this is the step the plan omits and it is the one that
   keeps the case honest):**
   `assert_eq!(db.store.missing_tags(item, ids::BOX).await.expect("the tags read back"), vec!["cuda".to_owned()], "the claiming box probes rust/msvc/cmake and declares gpu");`
   `fixtures.rs:471-472` is where those tags come from; `cuda` is in neither set.

4. `let created = db.store.create_run(NewRun { target_box_id: other, ..race_run(item) }).await.expect("the run is queued");`
   `target_box_id` must be the planted row or the `REFERENCES box(id)` FK (`0001_init.sql:455`)
   answers `23503`. Assert `created.target_box_id == other` so the case cannot pass by aiming at
   `ids::BOX`.

5. `let claim = db.store.claim_run(created.id, ids::BOX, owner, at, until).await.expect("the claim is answered");`
   `assert_eq!(claim, Claim::NotClaimable, …)`. This is `pg/write.rs:3063` returning before the
   tag query at `:3071`; the transaction is dropped unwritten.

6. D3: `assert_eq!(db.store.run(created.id).await.expect("the run reads back"), Some(created.clone()), "the refusal wrote nothing to the run")`
   and `assert_eq!(db.store.item(item).await.expect(…).expect(…).status, Status::Queued, "…not blocked")`.
   A whole-row compare is safe here: `Run` is `PartialEq` (`model/run.rs:167`) and both sides read
   the same jsonb-normalised `graph_snapshot` and the same stored `timestamptz`. If it turns out to
   be noisy, assert the fields that matter instead — `status`, `failure.is_none()`,
   `finished_at.is_none()` — but the whole-row form is what D3 asks for and it should hold.

**Why it cannot pass for the wrong reason**: step 3 proves the tag rule is armed; the run's
`target_box_id` is not the claiming box; so only the target-box rule can produce `NotClaimable`.
Invert the order and step 5 sees `Claim::MissingTags { missing: ["cuda"] }`, and steps 5-6 fail
(the run would be `failed` with a `failure` string, the item `blocked`).

**Mutation, the sharpened form** (see the T1 note and `6033d4f`): move the
`return Ok(Claim::NotClaimable)` at `pg/write.rs:3063-3065` to **after the whole
`if !missing.is_empty() { … return Ok(Claim::MissingTags { missing }); }` block at
`pg/write.rs:3089-3110`** — not merely after the `let missing = …` fetch. Moving it after only the
fetch leaves the write block below the return, `MissingTags` stays unreachable, and the case passes
while the rule is inverted. That is the false green the T1 implementer hit.

### Case 2 — `a_run_with_no_item_is_never_refused_for_tags`

1. Insert the run directly. The plan's column list is **complete and valid** — I checked every
   column of `run` (`0001_init.sql:447-464`) plus what `0003` added:

   ```rust
   let id = RunId::new();
   let at = Utc::now();
   sqlx::query(
       "INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, \
        graph_snapshot, started_by, queued_at) \
        VALUES ($1, $2, NULL, 'graph', 'manual', 'queued', $3, '{}'::jsonb, $4, $5)",
   )
   .bind(id.as_uuid())
   .bind(ids::PROJECT_HTUI.as_uuid())
   .bind(ids::BOX.as_uuid())
   .bind(ids::USER.as_uuid())
   .bind(at)
   .execute(&db.pool)
   .await
   .expect("plant a queued run with no item");
   ```

   NOT NULL without a default: `project_id`, `kind`, `mode`, `target_box_id`, `started_by` — all
   supplied. `status` and `queued_at` have defaults but are supplied. `repo_scope` is
   `NOT NULL DEFAULT '{}'` (`0003:40`) and is correctly omitted. `item_id` is explicitly `NULL`,
   allowed by `0001_init.sql:450`. `ck_run_graph_snapshot` is
   `CHECK (kind <> 'graph' OR graph_snapshot IS NOT NULL) NOT VALID` (`0003:47-48`): `NOT VALID`
   is still enforced for new rows, and `'{}'::jsonb` satisfies it. `kind`/`mode`/`status` satisfy
   the `CHECK`s at `0001:451-454`. Untyped → `.sqlx` stays 268.

2. `assert_eq!(db.store.claim_run(id, ids::BOX, owner, at, until).await.expect("the claim is answered"), Claim::Admitted, "a run with no item has no tags to refuse on");`
   then `assert_eq!(claimed.status, RunStatus::Running)` and
   `assert_eq!(claimed.executing_box_id, Some(ids::BOX))` from `db.store.run(id)`.

   **The trace, step by step, for a NULL `item_id`:**
   - `SELECT status, target_box_id, repo_scope, graph_snapshot FROM run WHERE id = $1 FOR UPDATE`
     (`pg/write.rs:3029-3040`) — found; `status = 'queued'`, `target_box_id = ids::BOX`.
   - `SELECT settings FROM box WHERE id = $1 FOR UPDATE` (`:3051-3060`) — found, no `NotFound`.
   - `:3063` claimability → falls through.
   - `:3071-3086` the tag query. **`JOIN item i ON i.id = r.item_id` is an inner join and
     `r.item_id` is `NULL`, so the join produces no row and `missing` is empty.** This is the whole
     of D94 on the Postgres side — an absence, not a branch. `missing.is_empty()` → no write.
   - `:3110-3128` limit: the box's own `settings` decodes `max_concurrent_items: 2`; `running` is 0
     → not `SlotFull`.
   - `:3130-3152` the live-rows query filters `repo_scope && $2::uuid[]` with `$2` empty → no rows
     → not `Overlaps`. `scope_of` is total on `'{}'` (`model/overlap.rs:112-124`), so `mine` cannot
     panic.
   - `:3154-3171` the admitted `UPDATE run` — touches no `item_id` column.
   - `:3173-3179` `UPDATE item … WHERE id = (SELECT item_id FROM run WHERE id = $1)`: the subquery
     returns `NULL`, `id = NULL` matches no row, **zero rows updated, and the comment at `:3172`
     explicitly says zero rows here is expected and not an error**. This is the answer to the
     brief's question 4: the admitted branch touches the item only through that subquery and
     through the `MissingTags` branch, so a NULL `item_id` breaks nothing in it.
   - `:3184-3185` commit, `Ok(Claim::Admitted)`.

   `repo_scope = '{}'` matters twice: it makes the overlap query empty (H-10) and it means the case
   cannot be refused for any reason other than the tag rule — which is exactly what the case claims
   is never reached.

3. **Mutation (C3)**: the plan's "swap the join for an inner join that demands a row" cannot be done,
   because `:3075` already is one. Add a `require_item`-equivalent between `:3063` and `:3071` that
   answers `NotFound { entity: "item" }` (or `NotClaimable`) when the run's `item_id` is `NULL`, and
   confirm the case fails there. Revert.

---

## What an implementer must know that the plan does not say

1. **T1's mutation must be redone in the sharpened form.** The first attempt was a false green
   (predicate hoisted, `return` placed after the `let missing` computation but still before the
   `MissingTags` write block). The plan at `6033d4f` now says so; the faithful mutation moves the
   `NotClaimable` return below the *entire* `if !missing.is_empty() { … }` block. Do not repeat the
   weak form — it reports green while the rule is still effectively inverted.
2. **T0's case 1 needs the `missing_tags` guard (C4).** Without it the case passes whether or not
   the tag rule is armed, and the mutation check for case 1 then proves nothing.
3. **`race_item` needs two arguments, and a `kind_id` you must choose (C2).**
4. **The `UNIQUE (user_id, hostname)` justification is dead (C1).** Do not write a comment citing
   `0001_init.sql:73`; it was dropped by `0005_box_identity.sql:15`.
5. **The PG D94 mutation is an addition, not a substitution (C3).**
6. `#[tokio::test(flavor = "multi_thread")]` in `pg_criteria.rs` (C6).
7. Fixture arithmetic that makes both T0 cases safe: `max_concurrent_items: 2`, zero `running` runs
   at start, one `queued` run (inert), `ids::USER` the only `app_user`, `cuda` in neither
   `probed_tags` (`rust`, `msvc`, `cmake`) nor `declared_tags` (`gpu`).
8. Both T0 cases must be inside `demo_db()`/`drop_db()`; a `pg_criteria` failure on this box is
   suspect until `df -h /` has been checked (project memory: the dev Postgres crash-loops under
   disk pressure) and the case re-run alone. `--test-threads=1` is not optional.
