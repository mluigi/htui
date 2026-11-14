# Blueprint: MOD-130 M1 — design deltas

Design review of `.claude/plans/mod-130-m1-targets-and-child-boxes.plan.md` (2026-11-14) against the
tree, the PRD (D1..D18), ANA-42 §7 and the later milestones (M2 resolver and enqueue, M3 driver-keyed
claim, M4 display, MOD-44, MOD-131, MOD-132). Only deltas are listed; every section not named here
is sound as planned. Line numbers are the plan's at review time.

Twelve deltas. B1-B9 are corrections that a later step or the house style would otherwise force;
B10-B12 are recommended.

---

## B1 — T5: the mirror's timestamps are INTEGER microseconds, not TEXT

**What.** The T5 cache DDL (plan :657-660) types `probed_at`, `retired_at`, `created_at` and
`updated_at` as `TEXT`.

**Why.** `cache_migrations/0001_mirror.sql:5-12` fixes the mapping `TIMESTAMPTZ -> INTEGER`
(microseconds since the epoch; `ts_col` / `ts_bind`), and every mirrored table follows it
(`box` at `0001_mirror.sql:37-45`, `project` at `:57`). A `TEXT` column read through `ts_col` fails
at read time. The sentence after the DDL (:666) says to follow the conventions, but the DDL
contradicts it, and an implementer copies the DDL.

**Replace the T5 DDL's `CREATE TABLE target` with:**

```sql
CREATE TABLE target (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, locator TEXT NOT NULL DEFAULT '{}',
    box_id TEXT, probed_at INTEGER, probe_error TEXT, retired_at INTEGER,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
```

and the :666 sentence with: "UUID → TEXT, JSONB → TEXT, TIMESTAMPTZ → INTEGER microseconds through
`ts_bind`/`ts_col` (`0001_mirror.sql:5-12`). There are no CHECKs and no foreign keys (`:16-17`)."

## B2 — T2: `create_target` on a taken id is refused; it is not idempotent

**What.** Plan :415-416 says `create_target` is idempotent on `id` "as `create_project` does", and
test :446 says "A repeat create is idempotent".

**Why.** `create_project` is not idempotent. Mem refuses a stored id with "project `{id}` already
exists" (`mem.rs:3467-3472`), and Pg relies on the `23505` mapping (`pg/write.rs:4178-4180`). The
house sentence for a client-minted id that is already stored is `already_exists(entity, id)`
(`traits.rs:3658-3662`). The plan also leaves undefined what an equal id with different fields gives.
The local target's id with a non-local kind is such a case, and it falls through refusal 1.

**Replace :415-416 with:** "A taken `id` (the local target's included) is
`StoreError::Constraint(already_exists("target", id))`. It is checked after the input refusals 1-3
and before the name check, and Pg checks it in code before the INSERT."
**Replace test :446 with:** "A repeat create gives `already_exists("target", id)` exactly and writes
nothing; `create_target` with `id = LOCAL_TARGET_ID` and kind `docker` gives the same."

## B3 — P9, T2-T4: a reference that names no row is `references_no_row`, not `NotFound`

**What.** P9 says "A missing row is `StoreError::NotFound { entity: "target" | "box" | … }`". It is
applied to the target named *by* a level (T3 :506, T4 "Refusals shared by the three setters") and to
the parent box of `attach_child_box`.

**Why.** The house rule keeps `NotFound` for the row the method addresses, and uses
`Constraint(references_no_row(column, id, table))` (`traits.rs:3645-3649`) for a foreign key in the
input that names no row. Examples: `create_step_graph` (`traits.rs:1018-1024`) and `replace_graph`
(`:1143-1145`). `update_item` refuses an unknown kind as a `Constraint` too, after the version compare
(`mem.rs:2524-2540`). T3 also names a writer that does not exist: the item insert is `mint_item`
(`traits.rs:418`; Mem `State::mint`). `update_item`'s stale outcome is `UpdateOutcome::Diverged`, and
it comes before every refusal (`mem.rs:2511-2522`).

**Replace P9's NotFound sentence with:** "`NotFound { entity, id }` is for the addressed row only: the
`target` of `target`/`edit_target`/`retire_target`/`delete_target`/`attach_child_box`, the `project`
of `set_project_target` and the `step_graph` of `set_graph_target`. A target id carried as a value
that names no row is `Constraint(references_no_row("<table>.target_id", id, "target"))`, with column
`item.target_id`, `project.target_id`, `step_graph.target_id` or `app_setting.default_target`.
`attach_child_box`'s missing parent is `references_no_row("box.parent_box_id", parent, "box")`."

**Replace T3's "Store." paragraph's first sentence with:** "`mint_item` and `update_item` write
`target_id`. In `update_item` the target refusals run after the version compare (`Diverged` first)
and after the kind check, in the block that opens at `mem.rs:2524`, so nothing is half-applied."
Test `item_target_is_a_spec_field`: "an unknown target → `references_no_row("item.target_id", …)`";
"a spent version → `UpdateOutcome::Diverged`".

## B4 — P9: house spelling, the name trim, and NUL in the locator and the hostname

**What.** (a) `target_name_taken` reads "a target named {name} exists". (b) `target_name_refusal`
"trims", but it returns only a refusal, so it cannot store a trimmed name, and the plan does not say
which name is stored. (c) No rule refuses a NUL in `locator` or in `NewChildBox.hostname`.

**Why.** (a) The house form for a unique name is "`<table>.<column> `{value}` is taken`":
`graph_name_is_taken` (`traits.rs:3696`) and "project.slug `{}` is taken" (`mem.rs:3473-3476`).
(c) Postgres `jsonb` refuses `\u0000` with `22P05` and `text` refuses NUL with `22021`. `map_sqlx`
maps every non-`23` code to `StoreError::Backend` (`error.rs:41-46`), so Pg answers a backend error
where Mem stores the row. That breaks P9's "never a raw SQLSTATE" and the conformance parity. The
house rule exists for this case: `json_has_nul` (`traits.rs:3181`) and `has_nul` (`:3392`), used by
`run_plan_refusal` (`:3156-3177`).

**Replace in P9:**
- `target_name_taken(name)`: "target.name `{name}` is taken".
- `target_name_refused(name)`: "target.name is 1-64 characters with no control character and no
  leading or trailing space; {name:?} is not". The name is stored as given, never trimmed, and
  `name != name.trim()` is refused, so both stores compare and store the same bytes.

**Add to `locator_refusal` (T1), first:** `json_has_nul(locator)` → `has_nul("target.locator")`.
Make `json_has_nul` `pub(crate)`, or put `locator_refusal` in `traits.rs` beside `run_plan_refusal`.
**Add to `attach_child_box` (T2) as refusal 0, before any read:** `hostname` empty or containing NUL →
`has_nul("box.hostname")` (or a blank-hostname sentence). Add both to the refusal cases.

## B5 — P9: the in-use labels are ambiguous across projects

**What.** `workflow <name>` and `item <key>` (P9 :105-107).

**Why.** Every project is seeded with graphs of the same names: "`item_kind.name`, which is also
`step_graph.name`" (`seed.rs:47-48`). Graph names are unique per scope only (`graph_name_is_taken`,
`traits.rs:3694-3698`). Item keys are unique per project only. A target used by the `feature` graph of
two projects reads "workflow feature, workflow feature", and the person cannot find either.

**Replace the label sentence with:** "A user is labelled `project <slug>`; `workspace <slug without
@>` for a scope row; `workflow <name> of project <slug>` | `of workspace <slug>` | `in the library`;
`item <key> of project <slug>` (the scope row's slug without `@` for a workspace-owned item); or
`Settings default`. Labels are deduplicated and sorted by bytes."

## B6 — T1: `locator_refusal`'s exact key list misses the common secret shapes

**What.** Refuse a key that equals one of {password, passwd, secret, token, api_key, apikey,
private_key, credential, credentials}, ASCII case-insensitive.

**Why.** An exact match lets `access_token`, `auth_token`, `client_secret`, `AWS_SECRET_ACCESS_KEY`,
`privateKey`, `private-key` and `ssh_password` through. Those are the shapes a docker context or ssh
config carries. The PRD constraint says the scrubber is not a substitute (`R-SEC-2`, PRD "Names and
locators, never values"). Credentials in a URL's userinfo (`tcp://u:p@host`) are a value, not a key,
and pass any key rule.

**Replace the `locator_refusal` comment (plan :273) with:** "not an object; a NUL (B4); any key at any
depth whose normalisation (ASCII-lowercased, with `_`, `-` and `.` removed) *contains* one of
`password`, `passwd`, `secret`, `token`, `apikey`, `privatekey` or `credential`; or any string value
holding a URL with a password in its userinfo (`://user:pass@`)". The sentence names the key path:
`target_locator_refused("key `docker.client_secret` looks like a secret")`. Extend
`locator_refusal_names_a_secret_key_at_any_depth` with `access_token`, `clientSecret`,
`AWS_SECRET_ACCESS_KEY`, `private-key`, and a `tcp://u:p@h` value. Add one allowed case,
`{"host":"dev","user":"me","context":"default"}`.

## B7 — T2-T4 (Pg): retire and the level setters race; lock the target row

**What.** Only `attach_child_box` takes `FOR UPDATE` on the target. `retire_target` checks "in use"
and then writes `retired_at`. `set_project_target`, `set_graph_target`, `set_default_target`,
`mint_item` and `update_item` check "not retired" and then write.

**Why.** Two concurrent transactions can each pass their check. Then a level names a retired target,
which D14 forbids. No foreign key guards retire, and an FK check takes `FOR KEY SHARE`, which does
not conflict with an UPDATE of `retired_at`. The same race on delete reaches the FK and comes back as
`map_sqlx`'s raw "fk_…: …" `Constraint` (`error.rs:37-44`), not a P9 sentence.

**Add to P9, after "checked in code before the statement runs":** "On Pg, every writer that names a
target as a value reads it with `SELECT retired_at FROM target WHERE id = $1 FOR SHARE` in its own
transaction, before its check. `retire_target`, `delete_target` and `attach_child_box` read it with
`FOR UPDATE`. A setter that waits behind a retire sees `target_retired`, and one that waits behind a
delete sees `references_no_row`. Mem is serial under its write lock." Add one `pg_criteria` test,
`a_level_set_behind_a_retire_is_refused`: two connections, retire holding the lock, then the setter
gives `target_retired`.

## B8 — T1: seed the local target in Mem's reader, not in `State`

**What.** Plan :307: "a new `State.targets` …, seeded with `seed::local_target`", and P2 "MemStore
seeds the same row when its state is built".

**Why.** `State` is `#[derive(Debug, Default)]` (`mem.rs:245-246`), so a derived `BTreeMap` field
starts empty. `MemStore::from_demo` builds `State` field by field (`mem.rs:479-…`), and both
conformance suites run on demo stores (`pg_conformance.rs` `demo_db`, Mem `MemStore::demo()`). "Seed
when built" therefore needs two edits, and a third constructor would silently lose the row. The house
precedent for an immutable built-in row is to synthesise it in the reader:
`MemStore::project_types` answers `seed::PROJECT_TYPES` at `DateTime::UNIX_EPOCH`
(`mem.rs:11642-11648`). D2 makes the local target immutable, so the reader approach is exact.

**Replace plan :307 with:** "`MemStore`: `State.targets: BTreeMap<TargetId, Target>` holds created
targets only. `targets()` merges `seed::local_target(DateTime::UNIX_EPOCH)` in P10 order, and
`target(LOCAL_TARGET_ID)` answers it. Every writer checks `LOCAL_TARGET_ID` first (built in, or
`already_exists` for `create_target`, B2). The usage scan in retire and delete never sees it."
Amend P2: "MemStore answers the same row from its readers (epoch timestamps); the conformance case
compares every field but the timestamps."

## B9 — T4: `replace_graph` ignores `target_id`, and `override_graph` must carry it

**What.** `StepGraph` gains `target_id`, but the plan does not say what the two whole-graph writers do
with it.

**Why.** `replace_graph` writes only `name` and `description` from `body.graph`
(`traits.rs:1119-1121`). That is right, because the pin is `set_graph_target`'s alone, but it must be
stated and pinned. `htui-orch`'s `override_graph` (`graph.rs:1178-1474`) clones the item's workflow
into a per-item override graph with `create_step_graph` and `replace_graph`, builds a `StepGraph`
literal (about `:1366`), then repoints the item at the clone. If the source's `target_id` is not
copied, overriding an item's workflow silently moves the item from the workflow's pinned target to
inherit. The plan's ripple fixes the literal to compile and nothing more.

**Add to T4:**
- `replace_graph`'s doc gains "`target_id` is never written; `set_graph_target` is its writer".
  `graph_target_bumps_revision` asserts that a `replace_graph` whose body carries another
  `target_id` leaves the stored one.
- `override_graph` copies the source graph's `target_id` onto the clone. After `replace_graph`, when
  `source_row.target_id` is `Some(t)`, it calls `set_graph_target(clone.id, clone.updated_at,
  Some(t))`. A refusal there (the target was retired meanwhile) leaves the orphan, as every other
  mid-clone failure does, and the next call adopts it. A re-clone copies from the kind's default
  graph, which is its source.
- Test in `htui-orch` `graph.rs`: `an_override_keeps_the_workflows_target`. Add
  `crates/htui-orch/src/graph.rs` to T4's files.

## B10 — P4, T1 rule 6, T2: model `ssh_docker` like `docker` (a container child), one target per container (recommended)

**What.** Rule 6 takes `host` = "its parent (docker) or itself (ssh_docker)". So an `ssh_docker`
target binds the remote *machine*, and `attach_child_box` accepts `Docker` only. Also, a container box
already bound to another target can be attached again (refusal 6 checks only the parent).

**Why.** D9 checks tags and agent candidates on `target_box_id` "because the toolchain and agents are
inside the container", and D12 says "only a container is a child". A remote docker's container is a
container on the remote machine, so its box is a child of that machine (MOD-131 registers the
machine). Under the plan's shape, M3's tag check for an `ssh_docker` run would read the remote host's
tags instead of the container's, and MOD-131 would have to change `status`'s host rule,
`target_not_docker` and P4. The PRD risk table assumes one container box per image (one per target).
Two docker targets sharing one child box would share and overwrite each other's probe.

**Replace P4 with:** "Only `attach_child_box` sets `target.box_id` in M1, for a `docker` or
`ssh_docker` target (a container child of a machine: this box for `docker`, the remote machine
MOD-131 registers for `ssh_docker`). MOD-131 adds the `ssh` binder (a machine row)."
**Replace rule 6 with:** "A `Docker` or `SshDocker` target whose `host` (always `bound`'s parent) lacks
`docker` …".
**Replace T2 refusal 3 with:** "`kind ∉ {Docker, SshDocker}` → `target_takes_no_container(name,
kind)`: "only a docker or ssh_docker target takes a container box; {name} is {kind}"".
**Add T2 refusal 6b:** "the child exists and another target's `box_id` is it →
`box_bound_to_other_target(child, other_name)`: "box {child} is bound to target {other_name}"".
Add both to `attach_child_box_creates_reuses_and_refuses`. If the maintainer keeps the plan's shape,
P4 instead records that MOD-131 owns changing rule 6 and refusal 3.

## B11 — T4: the default target's clear rule and a typed reader (recommended)

**What.** `set_default_target(None, None)` is undefined. `default_target()` returns a raw
`StoredSetting` whose `value` is "a JSON string of the id".

**Why.** The precedent the plan copies refuses a clear without a token before any read:
`BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN` (`traits.rs:719-722`, `mem.rs:5662-5666`). The box probe spec's
reader is untyped because its validation belongs to `htui-agent` (`traits.rs:687-690`). This value is
the store's own, and M2's resolver, M4 and MOD-123 would otherwise each parse the string and decide
what a malformed one means.

**Replace the two T4 signatures with:**

```rust
pub struct DefaultTarget { pub target: TargetId, pub updated_at: DateTime<Utc> }   // model/target.rs
async fn default_target(&self) -> Result<Option<DefaultTarget>>;
async fn set_default_target(&self, target: Option<TargetId>, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<Option<DefaultTarget>>>;
```

Add to P9 `pub const DEFAULT_TARGET_CLEAR_NEEDS_A_TOKEN: &str = "clearing the default target needs
the row's updated_at"`, refused before any read when both arguments are `None`. A stored value that
does not parse as a target id is `StoreError::Backend` naming the key. Add the clear refusal to
`default_target_is_cas_on_updated_at`.

## B12 — T1: the `os_family: None` sites the plan does not list

**What.** T1 names only `render.rs`'s `box_profile` as a reader that must handle `None`.

**Why.** The compiler will also stop at these sites, and each needs a decision, not a guess:
- `label::os_family(OsFamily)` (`htui/src/ui/kit/label.rs:272`), used by the Boxes section
  (`settings/boxes.rs:1127`);
- Mem `box_info` (`mem.rs:604`, into `BoxInfo.os_family: OsFamily`);
- `pg/demo.rs:75` and `testkit.rs:703` (`row.os_family.as_str()` into an INSERT);
- `cache/refresh.rs:656`, which binds into the mirror's `box.os_family TEXT NOT NULL`
  (`0001_mirror.sql:38`).

**Add to T1's "Other model changes":** "`label::os_family` takes `Option<OsFamily>` and answers
`not probed` for `None`, and the Boxes row reads `os  not probed`. `box_info` (Mem), `refresh_box`,
`pg/demo.rs` and `testkit.rs` handle only machines, so a `None` there is a store bug: they return
`StoreError::Backend("box {id} has no os_family")` rather than invent a value. `refresh_box`'s query
gains `AND kind = 'machine'`." Add `htui/src/ui/kit/label.rs`, `pg/demo.rs`, `testkit.rs` and
`cache/refresh.rs` to T1's files.
