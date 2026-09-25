# Blueprint for MOD-38: requirements schema, seam and close-out resolution

Companion to `.claude/plans/mod-38-requirements-schema.plan.md` (D4–D15, T1–T11) and the PRD
(D1–D3). This file is the shared contract: every name, signature, error sentence, case name and
pin number below is final. Paths are relative to `crates/` unless they start with `.claude/`,
`docs/` or a crate-external root. Line anchors are as of branch `claude/project-thread-joz4s4`
before T1; they drift as tasks land, so find by symbol when a line has moved.

## Blueprint findings (plan claims corrected here)

- **F1: D13 under-counts the cascade.** D13 adds 3 `DeleteReach` fields. A project delete also
  cascades `requirement_spec`, `requirement_key_counter` and `requirement_revision`, so 3 fields
  would break the zero-omission rule that `htui/src/hierarchy.rs:472-477` enforces. **Decision:**
  add 6 fields (§4.4).
- **F2: T6's file set misses the compile breaks.** Adding a field to `DeleteReach` breaks every
  exhaustive literal or destructure of it. There are 4:
  - `htui/src/hierarchy.rs:479` (`[_; 22]`);
  - `htui-store/tests/pg_criteria.rs:2515` (destructure, plus `claimed: [u64; 22]` and `TABLES`);
  - `htui-core/src/store/mem.rs:2845`;
  - `htui-store/src/pg/write.rs:355`.

  All four move into T6. Mem and Pg get `0` placeholders there; T7 and T8 make them real.
- **F3: T7–T9 read cases have no data.** `READ_CASES` read the fixture, and the plan adds the
  fixture requirement rows only in T10. The T7–T9 read cases could therefore never go green.
  **Decision:**
  - the fixture's requirement data and ids (`fixtures.rs`) plus the `pg/demo.rs` inserts move
    to **T6**;
  - `MemStore::from_demo` loading them is **T7** (`mem.rs`);
  - T10 keeps the `migrations.rs` row-count test, `hierarchy.rs` tests, the FIX-1 assertion and
    doc touch-ups.
- **F4: T8 is not the only `.sqlx` writer in its wave.** The claim "T8 is the only .sqlx writer
  in its wave" is false. Every cache refresh arm is `sqlx::query!` against Postgres
  (`cache/refresh.rs:841`, `903`), so T5 (the item arm) and T9 (four new arms) regenerate
  `.sqlx/*` too. The files are content-hashed per query, so the merge is additive (union of
  files). Add `htui-store/.sqlx/*` to the T5 and T9 file sets.
- **F5: T1's file list is wrong.** `prompt/{fixtures,render}.rs` hold no `Item` literal; their
  `status: Status::Closed` lines are `UpstreamEntry`s. The real `Item` literal sites are:
  - `htui-core/src/fixtures.rs:1016-1036`;
  - `htui-core/src/store/mem.rs:1225-1244` (the mint);
  - `htui-orch/src/closeout.rs:224-243`;
  - `htui-store/src/cache/read.rs:157-180`.

  The 3 `query_as!(Item, …)` sites (`pg/read.rs:130`, `pg/write.rs` `mint_item` and
  `update_item`) are compile errors only under `SQLX_OFFLINE` regeneration, and they belong to
  T2.
- **F6: `htui/tests/runs_pg.rs` belongs to T4, not only T3.** It builds
  `Command::CloseOut { item }` at `:759` and `:840`. T3 edits its direct `close_out` calls
  (`:779`, `:812`); T4 edits the two `Command` literals. The same goes for
  `htui/src/run_worker.rs:4184` (a test literal).
- **F7: part of T4's "tests first" is already true.** Plan T4 says
  `close_out_needs_no_live_run_and_a_closable_item` "gains `open` → `NotClosable`". It already
  asserts that (`htui-orch/src/command.rs:2151-2164`). T4 changes it to assert the returned
  default instead (§7).
- **F8: T3 overstates two case changes and misses two others.**
  - `illegal_transitions_are_constraint` is not "switched to `close_out`". Only its 4th pair
    changes, `(Done, Closed)` → `(Done, Open)` (`conformance.rs:6388`).
  - `no_delete_path` does switch, to a direct `close_out` (`:724`).
  - T3 must also edit `close_out_refuses_a_live_run`, which passes the resolution argument and
    whose `open` leg becomes `open` + `Done` (`:6300-6304`).
  - T3 must also edit the Mem twin test `mem.rs:7143-7230`.
- **F9: the concurrent-mint test goes in `pg_criteria.rs`.** Plan T8 puts it in
  `pg_conformance.rs`, but the precedent is `pg_criteria.rs:162`
  (`concurrent_mints_produce_consecutive_numbers`). The new test is
  `concurrent_requirement_mints_produce_consecutive_numbers` in `pg_criteria.rs`. Add
  `htui-store/tests/pg_criteria.rs` to T8's file set; `pg_conformance.rs` is not touched in T8.
- **F10: the constraint name gets a prefix.** ANA-11 and the plan call it
  `item_resolution_iff_closed`. 0001 names table constraints `chk_*`, `fk_*`, `uq_*` (0003
  drifted to `ck_`). **Decision:** `chk_item_resolution_iff_closed`.
- **F11: no placeholder refresh arms in T5.** Plan T5 says the four new refresh arms "are listed
  and answer 0". `run_pass` has an `unreachable!` default (`refresh.rs:288`) and a hazard note
  against fallbacks. **Decision:**
  - T5 appends the 4 names to `MIRRORED_TABLES` only; that is safe, because `cache/mod.rs:175`
    just clears them;
  - T9 adds the 4 cursor constants, the loop entries and the real arms together.
- **F12: `requirement.area_id` is safe without a cascade.** It has no `ON DELETE CASCADE`, which
  is the same shape as `item.kind_id → item_kind` (0001:311). Both parents are first-level
  project cascades, and the measured delete test (`pg_criteria.rs`
  `every_cascade_table_loses_exactly_what_the_report_names`) already proves that shape. No
  change is needed.
- **F13: offline citations can be partial.** A cross-project citation whose requirement or item
  is outside the mirrored scope is absent offline, because the cache reads INNER JOIN the
  mirrored `requirement` / `item`. This is accepted; it is the same as `links` offline.
- **F14: an unrelated `Resolution` already exists.** `htui/src/ui/tabs/chat/transcript.rs:36`
  declares `pub struct Resolution`. No file imports both, so there is no clash, but T4's `runs.rs`
  must import `htui_core::model::Resolution` by path, not by glob.
- **F15: the fixture timestamp CHECK has no reason to fire.** `resolution` is keyed on
  `status == Closed`, not on `ItemSpec.closed`, so `chk_item_resolution_iff_closed` holds on the
  demo insert by construction.
- **F16: known reds between tasks.**
  - After T6: the new cases (bodies are `unimplemented!()` on Mem, Pg and the cache), and
    `pg_criteria` measured delete (Pg reach is `0` while the demo seeds requirement rows). T8
    fixes the latter.
  - After T7: none new.

---

## 1. `htui-store/migrations/0006_requirements.sql` (T2, whole)

```sql
-- 0006_requirements.sql - MOD-38: ANA-11 (requirements) docs/ANA-11.md §5, and the §4.2 close-out
-- resolution. Forward-only (R-STO-5): 0001-0004 are never edited. Depends on 0001_init.sql
-- (project, app_user, box, item, run_step, set_updated_at()).
-- The cache-mirror companion is cache_migrations/0004_requirements.sql (plan D12).
--
-- Deltas from §5 (plan D8): the per-area key counter is minted by mint_item's
-- INSERT ... ON CONFLICT DO UPDATE ... RETURNING CTE, not by UPDATE ... RETURNING; the four
-- mutable tables get their set_updated_at() triggers by explicit CREATE TRIGGER (0001's loop is
-- not re-run); closed rows are backfilled to 'done' before chk_item_resolution_iff_closed. No
-- COMMENT ON COLUMN: ANA_COLUMN_COMMENTS stays at 25.

-- --------------------------------------------------------------------------------------------
-- 1. item.resolution (ANA-11 §4.2): set by close_out only, NULL until the item is closed
-- --------------------------------------------------------------------------------------------

ALTER TABLE item ADD COLUMN resolution TEXT CHECK (resolution IN
    ('done','concluded','rejected','withdrawn','superseded','duplicate'));

-- Every item closed before this migration closed through the old blocked/failed/done -> closed
-- edges, which only a finished item took. The UPDATE fires trg_item_updated_at, which is
-- harmless: the mirror rebuilds on schema_version 5.
UPDATE item SET resolution = 'done' WHERE status = 'closed';

ALTER TABLE item ADD CONSTRAINT chk_item_resolution_iff_closed
    CHECK ((status = 'closed') = (resolution IS NOT NULL));

-- --------------------------------------------------------------------------------------------
-- 2. requirement_spec (§4.4): one header per project
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement_spec (
    project_id  UUID PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
    owner_id    UUID NOT NULL REFERENCES app_user(id),
    preamble    TEXT NOT NULL DEFAULT '',
    version     INTEGER NOT NULL DEFAULT 1,      -- CAS token, no revision history (plan D9)
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- --------------------------------------------------------------------------------------------
-- 3. requirement_area
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement_area (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    code        TEXT NOT NULL CHECK (code ~ '^[A-Z][A-Z0-9]{1,15}$'),   -- 'ENT','STO','NF'
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    position    INTEGER NOT NULL DEFAULT 0,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, code)
);

-- --------------------------------------------------------------------------------------------
-- 4. requirement_key_counter (ANA-9 §4.1 semantics, one per area)
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement_key_counter (
    area_id     UUID PRIMARY KEY REFERENCES requirement_area(id) ON DELETE CASCADE,
    last_value  INTEGER NOT NULL CHECK (last_value >= 0)
);

-- --------------------------------------------------------------------------------------------
-- 5. requirement -- area_id has no cascade, the shape of item.kind_id
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    area_id      UUID NOT NULL REFERENCES requirement_area(id),
    area_code    TEXT NOT NULL,                  -- copied at mint, like item.key_prefix
    number       INTEGER NOT NULL CHECK (number >= 1),
    key          TEXT GENERATED ALWAYS AS ('R-' || area_code || '-' || number::text) STORED,
    body         TEXT NOT NULL,
    rationale    TEXT NOT NULL DEFAULT '',
    priority     TEXT NOT NULL CHECK (priority IN ('must','later')),
    state        TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','withdrawn')),
    version      INTEGER NOT NULL DEFAULT 1,     -- CAS, ANA-9 §4.2
    created_by   UUID NOT NULL REFERENCES app_user(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, area_code, number)
);
CREATE INDEX idx_requirement_updated_at ON requirement(project_id, updated_at);   -- cache cursor

-- --------------------------------------------------------------------------------------------
-- 6. requirement_revision: append-only, no updated_at, no trigger
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement_revision (
    requirement_id     UUID NOT NULL REFERENCES requirement(id) ON DELETE CASCADE,
    version            INTEGER NOT NULL,
    body               TEXT NOT NULL,
    rationale          TEXT NOT NULL,
    priority           TEXT NOT NULL,
    state              TEXT NOT NULL,
    author_id          UUID NOT NULL REFERENCES app_user(id),
    box_id             UUID REFERENCES box(id) ON DELETE SET NULL,
    reason             TEXT NOT NULL,            -- 'created','amended','withdrawn','imported','divergence_resolution'
    amended_by_item_id UUID REFERENCES item(id) ON DELETE SET NULL,   -- the deciding item
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (requirement_id, version)
);

-- --------------------------------------------------------------------------------------------
-- 7. item_requirement: citations; tombstoned like item_link
-- --------------------------------------------------------------------------------------------

CREATE TABLE item_requirement (
    item_id             UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    requirement_id      UUID NOT NULL REFERENCES requirement(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL CHECK (kind IN ('addresses','amends','withdraws','reserves')),
    requirement_version INTEGER NOT NULL,        -- stamp; suspect when requirement.version is newer
    proposed_by_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,   -- NULL = human or importer
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at          TIMESTAMPTZ,             -- tombstone; live citations have NULL
    PRIMARY KEY (item_id, requirement_id, kind)
);
CREATE INDEX idx_item_requirement_req ON item_requirement(requirement_id) WHERE deleted_at IS NULL;

-- --------------------------------------------------------------------------------------------
-- 8. updated_at triggers, 0001 §5.1's shape: BEFORE UPDATE only, so an INSERT that supplies
-- updated_at (the demo loader) keeps it. requirement_key_counter and requirement_revision have
-- no updated_at and are deliberately absent.
-- --------------------------------------------------------------------------------------------

CREATE TRIGGER trg_requirement_spec_updated_at BEFORE UPDATE ON requirement_spec
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_requirement_area_updated_at BEFORE UPDATE ON requirement_area
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_requirement_updated_at BEFORE UPDATE ON requirement
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_item_requirement_updated_at BEFORE UPDATE ON item_requirement
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
```

## 2. `htui-store/cache_migrations/0004_requirements.sql` (T5, whole)

```sql
-- ------------------------------------------------------------------------------------------------
-- Mirror side of ANA-11 (MOD-38, plan D12; `docs/ANA-11.md` 5.2 with one deviation).
--
-- `item` gains `resolution`. Four tables join the mirror, same table and column names as
-- Postgres, 4.4's type mapping (`0001_mirror.sql`): `requirement_spec` (the deviation from 5.2 -
-- one small row per project, so `requirement_spec` never has to answer "none or not cached"),
-- `requirement_area`, `requirement` and `item_requirement`. `item_requirement` holds live rows
-- only: a tombstone is a delete, as `item_link`'s is, so the column `deleted_at` is not here.
-- `requirement.key` is a plain column: the generated expression is Postgres's, the mirror
-- stores its value. `requirement_revision` and `requirement_key_counter` are not mirrored;
-- `requirement_revisions` answers `None` offline, as `step_events` does for an uncached step.
--
-- `cache_meta.schema_version` moves to 5 through `PgStore::schema_version()`, which forces a
-- full rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE item ADD COLUMN resolution TEXT;

CREATE TABLE requirement_spec (
    project_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, preamble TEXT NOT NULL DEFAULT '',
    version INTEGER NOT NULL DEFAULT 1, updated_at INTEGER NOT NULL);

CREATE TABLE requirement_area (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, code TEXT NOT NULL, title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '', position INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_requirement_area_project ON requirement_area(project_id, position, code);

CREATE TABLE requirement (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, area_id TEXT NOT NULL,
    area_code TEXT NOT NULL, number INTEGER NOT NULL, key TEXT NOT NULL,
    body TEXT NOT NULL, rationale TEXT NOT NULL DEFAULT '', priority TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active', version INTEGER NOT NULL DEFAULT 1,
    created_by TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_requirement_order ON requirement(project_id, area_code, number);

CREATE TABLE item_requirement (
    item_id TEXT NOT NULL, requirement_id TEXT NOT NULL, kind TEXT NOT NULL,
    requirement_version INTEGER NOT NULL, proposed_by_step_id TEXT,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    PRIMARY KEY (item_id, requirement_id, kind));
CREATE INDEX idx_cache_item_requirement_req ON item_requirement(requirement_id);
```

## 3. Rust definitions

### 3.1 `htui-core/src/model/item.rs` (T1)

`Resolution` is declared directly after `Status`'s `impl` block (after line 65):

```rust
str_enum!(
    /// `item.resolution` (ANA-11 §4.2): why a `closed` item closed. Set by
    /// [`WriteStore::close_out`](crate::store::WriteStore::close_out) only; `None` on every item
    /// that is not `closed` (`chk_item_resolution_iff_closed`).
    Resolution {
        /// The work was done.
        Done => "done",
        /// An analysis reached its verdict.
        Concluded => "concluded",
        /// Decided against.
        Rejected => "rejected",
        /// Dropped without a decision.
        Withdrawn => "withdrawn",
        /// Replaced by another item.
        Superseded => "superseded",
        /// The same as another item.
        Duplicate => "duplicate",
    }
);

impl Resolution {
    /// ANA-11 §4.2's close-out law (plan D4): `done` and `concluded` close only a `done` item;
    /// the other four close an `open`, `blocked`, `failed` or `done` one. Everything else,
    /// `closed` included, is refused.
    #[must_use]
    pub const fn closes_from(self, status: Status) -> bool {
        match self {
            Self::Done | Self::Concluded => matches!(status, Status::Done),
            Self::Rejected | Self::Withdrawn | Self::Superseded | Self::Duplicate => matches!(
                status,
                Status::Open | Status::Blocked | Status::Failed | Status::Done
            ),
        }
    }

    /// PRD D2 / plan D6: the resolution the Runs pane closes with until MOD-39's picker. `done`
    /// closes as `Done`; `blocked` and `failed` close as `Withdrawn`; nothing else is offered.
    #[must_use]
    pub const fn default_for(status: Status) -> Option<Self> {
        match status {
            Status::Done => Some(Self::Done),
            Status::Blocked | Status::Failed => Some(Self::Withdrawn),
            _ => None,
        }
    }
}
```

`Status::can_move_to` after D4. Its doc loses "or close-out", and "still reaches `closed` and
`open`" becomes "still reaches `open`". Add one line: "`closed` is reached only by close-out
(PRD D1, [`Resolution::closes_from`])."

```rust
            Self::Blocked => matches!(to, Self::Open | Self::AwaitingApproval),
            Self::Failed => matches!(to, Self::Queued),
            Self::Done => matches!(to, Self::Open),
```

`Item` gains one field, appended after `closed_at` (line 105):

```rust
    /// `item.resolution` (ANA-11 §4.2): `Some` exactly when `status` is `closed`.
    #[serde(default)]
    pub resolution: Option<Resolution>,
```

`ItemSummary`, `NewItem`, `ItemPatch` and `ItemRevision` are unchanged.

Tests in `item.rs` `mod tests`. The import becomes `use super::{Resolution, Status};`.

- `SANCTIONED` loses the rows `(Blocked, Closed)`, `(Failed, Closed)` and `(Done, Closed)`, and
  its doc gains the sentence "`→ closed` is not a transition since MOD-38 (PRD D1)".
- New table, beside it:

  ```rust
  /// ANA-11 §4.2's close-out law, transcribed from the document rather than from
  /// [`Resolution::closes_from`].
  const CLOSE_OUT_SANCTIONED: &[(Status, Resolution)] = &[
      (Status::Open, Resolution::Rejected),
      (Status::Open, Resolution::Withdrawn),
      (Status::Open, Resolution::Superseded),
      (Status::Open, Resolution::Duplicate),
      (Status::Blocked, Resolution::Rejected),
      (Status::Blocked, Resolution::Withdrawn),
      (Status::Blocked, Resolution::Superseded),
      (Status::Blocked, Resolution::Duplicate),
      (Status::Failed, Resolution::Rejected),
      (Status::Failed, Resolution::Withdrawn),
      (Status::Failed, Resolution::Superseded),
      (Status::Failed, Resolution::Duplicate),
      (Status::Done, Resolution::Done),
      (Status::Done, Resolution::Concluded),
      (Status::Done, Resolution::Rejected),
      (Status::Done, Resolution::Withdrawn),
      (Status::Done, Resolution::Superseded),
      (Status::Done, Resolution::Duplicate),
  ];
  ```

- `#[test] fn the_close_out_law_sanctions_exactly_the_ana_11_pairs()`: for every
  `Status × Resolution`, `closes_from` equals membership in `CLOSE_OUT_SANCTIONED`. It also
  asserts the table's length is 18.
- `#[test] fn the_default_resolution_is_a_sanctioned_close_out()`: for every `Status`,
  `default_for(s)` is `Some(r)` implies `r.closes_from(s)`. It also pins the map
  `[(Done, Some(Done)), (Blocked, Some(Withdrawn)), (Failed, Some(Withdrawn))]`, and `None` for
  the other 5.
- `#[test] fn nothing_transitions_into_closed()`: `!s.can_move_to(Status::Closed)` for every `s`.

### 3.2 `htui-core/src/model/mod.rs`

- Line 113 (T1):

  ```rust
  pub use item::{Item, ItemFilter, ItemPatch, ItemRevision, ItemSummary, NewItem, Resolution, Status};
  ```

- T1 test, beside `status_matches_check_list` (:171):

  ```rust
  #[test]
  fn resolution_matches_check_list() {
      check_enum(
          Resolution::ALL,
          &["done", "concluded", "rejected", "withdrawn", "superseded", "duplicate"],
      );
  }
  ```

- T6 adds `pub mod requirement;` in the mod list (80-96, alphabetical, after `quota`), and a
  re-export after `pub use quota…`:

  ```rust
  pub use requirement::{
      CitationKind, CoverageRow, ItemCitation, ItemRequirement, NewRequirement, NewRequirementArea,
      Priority, Requirement, RequirementArea, RequirementFilter, RequirementPatch,
      RequirementRevision, RequirementSpec, RequirementState, RequirementUpdate,
  };
  ```

  The `ids` re-export gains `RequirementAreaId, RequirementId`, alphabetised.
- T6 tests:
  - `priority_matches_check_list` (`["must","later"]`);
  - `requirement_state_matches_check_list` (`["active","withdrawn"]`);
  - `citation_kind_matches_check_list` (`["addresses","amends","withdraws","reserves"]`);
  - the `id_newtypes_round_trip` test (:330-350) gains the two new ids.

### 3.3 `htui-core/src/model/ids.rs` (T6)

Append to the `id_newtype!` list after `CommandRunId` (line 113):

```rust
    /// `requirement_area.id` (ANA-11 §5).
    RequirementAreaId,
    /// `requirement.id` (ANA-11 §5).
    RequirementId,
```

### 3.4 `htui-core/src/model/requirement.rs` (T6, new, whole)

```rust
//! Requirements, their areas, the spec header and citations (`docs/ANA-11.md` §4, §5).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{
    BoxId, ItemId, ProjectId, RequirementAreaId, RequirementId, StepId, UserId,
};
use crate::model::item::{ItemSummary, Resolution};
use crate::model::kind::ItemKind;

str_enum!(
    /// `requirement.priority` (ANA-11 §5).
    Priority {
        /// Required for the product to be what it claims.
        Must => "must",
        /// Wanted, not yet committed to.
        Later => "later",
    }
);

str_enum!(
    /// `requirement.state` (ANA-11 §5).
    RequirementState {
        /// In force.
        Active => "active",
        /// Retired by a deciding item; takes no new `addresses` / `reserves` citation.
        Withdrawn => "withdrawn",
    }
);

str_enum!(
    /// `item_requirement.kind` (ANA-11 §4.3).
    CitationKind {
        /// The item implements the requirement.
        Addresses => "addresses",
        /// The item decided a change to the requirement's text.
        Amends => "amends",
        /// The item decided to retire the requirement.
        Withdraws => "withdraws",
        /// The item claims the requirement for later work.
        Reserves => "reserves",
    }
);

/// A row of `requirement_spec`: one header per project (ANA-11 §4.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementSpec {
    /// `requirement_spec.project_id`, the primary key.
    pub project_id: ProjectId,
    /// `requirement_spec.owner_id`.
    pub owner_id: UserId,
    /// `requirement_spec.preamble`: preamble, out-of-scope and superseded-material prose.
    pub preamble: String,
    /// `requirement_spec.version`: the compare-and-set token (plan D9).
    pub version: i32,
    /// `requirement_spec.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `requirement_area`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementArea {
    /// `requirement_area.id`.
    pub id: RequirementAreaId,
    /// `requirement_area.project_id`.
    pub project_id: ProjectId,
    /// `requirement_area.code`, `^[A-Z][A-Z0-9]{1,15}$`: the `ENT` of `R-ENT-3`.
    pub code: String,
    /// `requirement_area.title`.
    pub title: String,
    /// `requirement_area.description`.
    pub description: String,
    /// `requirement_area.position`: list order within the project.
    pub position: i32,
    /// `requirement_area.updated_at`.
    pub updated_at: DateTime<Utc>,
}

impl RequirementArea {
    /// The `requirement_area.code` CHECK, byte for byte the `item_kind.prefix` one.
    #[must_use]
    pub fn code_is_valid(code: &str) -> bool {
        ItemKind::prefix_is_valid(code)
    }
}

/// Arguments of [`WriteStore::create_requirement_area`](crate::store::WriteStore::create_requirement_area).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRequirementArea {
    /// Client-minted id.
    pub id: RequirementAreaId,
    /// The owning project.
    pub project_id: ProjectId,
    /// The area code.
    pub code: String,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// The list position.
    pub position: i32,
}

/// A row of `requirement`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requirement {
    /// `requirement.id`.
    pub id: RequirementId,
    /// `requirement.project_id`.
    pub project_id: ProjectId,
    /// `requirement.area_id`.
    pub area_id: RequirementAreaId,
    /// `requirement.area_code`, copied at mint.
    pub area_code: String,
    /// `requirement.number`, per area, never reused.
    pub number: i32,
    /// `requirement.key`, generated: `R-<area_code>-<number>`.
    pub key: String,
    /// `requirement.body`.
    pub body: String,
    /// `requirement.rationale`.
    pub rationale: String,
    /// `requirement.priority`.
    pub priority: Priority,
    /// `requirement.state`.
    pub state: RequirementState,
    /// `requirement.version`: the compare-and-set token (ANA-9 §4.2).
    pub version: i32,
    /// `requirement.created_by`.
    pub created_by: UserId,
    /// `requirement.created_at`.
    pub created_at: DateTime<Utc>,
    /// `requirement.updated_at`.
    pub updated_at: DateTime<Utc>,
}

impl Requirement {
    /// Plan D11: a citation stamped at `stamp` is suspect when this requirement is newer.
    #[must_use]
    pub const fn makes_suspect(&self, stamp: i32) -> bool {
        self.version > stamp
    }
}

/// Arguments of [`WriteStore::mint_requirement`](crate::store::WriteStore::mint_requirement).
/// The area, project, code and number come from the area and its counter, never the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRequirement {
    /// Client-minted id.
    pub id: RequirementId,
    /// The body.
    pub body: String,
    /// The rationale.
    pub rationale: String,
    /// The priority.
    pub priority: Priority,
    /// `requirement.created_by`, and the `author_id` of revision 1.
    pub created_by: UserId,
    /// `box_id` of revision 1.
    pub box_id: Option<BoxId>,
}

/// The edit of [`WriteStore::amend_requirement`](crate::store::WriteStore::amend_requirement):
/// `None` leaves a column as it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequirementPatch {
    /// New body.
    pub body: Option<String>,
    /// New rationale.
    pub rationale: Option<String>,
    /// New priority.
    pub priority: Option<Priority>,
    /// The revision's `author_id`.
    pub author_id: UserId,
    /// The revision's `box_id`.
    pub box_id: Option<BoxId>,
    /// The revision's `reason`: `"amended"` for an ordinary amend.
    pub reason: String,
}

/// A row of `requirement_revision`: the requirement as it stood at `version`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementRevision {
    /// `requirement_revision.requirement_id`.
    pub requirement_id: RequirementId,
    /// `requirement_revision.version`.
    pub version: i32,
    /// `requirement_revision.body`.
    pub body: String,
    /// `requirement_revision.rationale`.
    pub rationale: String,
    /// `requirement_revision.priority`.
    pub priority: Priority,
    /// `requirement_revision.state`.
    pub state: RequirementState,
    /// `requirement_revision.author_id`.
    pub author_id: UserId,
    /// `requirement_revision.box_id`.
    pub box_id: Option<BoxId>,
    /// `requirement_revision.reason`: `created`, `amended`, `withdrawn`, `imported`,
    /// `divergence_resolution` (no CHECK, like `item_revision.reason`).
    pub reason: String,
    /// `requirement_revision.amended_by_item_id`: the deciding item.
    pub amended_by_item_id: Option<ItemId>,
    /// `requirement_revision.created_at`.
    pub created_at: DateTime<Utc>,
}

/// A row of `item_requirement`, tombstone included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemRequirement {
    /// `item_requirement.item_id`.
    pub item_id: ItemId,
    /// `item_requirement.requirement_id`.
    pub requirement_id: RequirementId,
    /// `item_requirement.kind`.
    pub kind: CitationKind,
    /// `item_requirement.requirement_version`: the stamp.
    pub requirement_version: i32,
    /// `item_requirement.proposed_by_step_id`; `None` = a human or the importer.
    pub proposed_by_step_id: Option<StepId>,
    /// `item_requirement.created_at`.
    pub created_at: DateTime<Utc>,
    /// `item_requirement.updated_at`.
    pub updated_at: DateTime<Utc>,
    /// `item_requirement.deleted_at`; `None` on a live citation.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// One live citation of an item, as
/// [`ReadStore::item_requirements`](crate::store::ReadStore::item_requirements) answers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemCitation {
    /// The cited requirement as it is now.
    pub requirement: Requirement,
    /// The citation kind.
    pub kind: CitationKind,
    /// The version the citation was stamped at.
    pub requirement_version: i32,
    /// The step that proposed it, if any.
    pub proposed_by_step_id: Option<StepId>,
    /// `requirement.version > requirement_version`, computed on read (plan D11).
    pub suspect: bool,
}

/// One live citation of a requirement, as
/// [`ReadStore::requirement_coverage`](crate::store::ReadStore::requirement_coverage) answers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageRow {
    /// The citing item.
    pub item: ItemSummary,
    /// The citation kind.
    pub kind: CitationKind,
    /// `item.resolution` (the summary carries the status).
    pub resolution: Option<Resolution>,
    /// The version the citation was stamped at.
    pub requirement_version: i32,
    /// Plan D11, as [`ItemCitation::suspect`].
    pub suspect: bool,
}

/// The requirement list filter (plan D14). Every field is a conjunct; `None` does not filter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequirementFilter {
    /// `requirement.area_code` in this list.
    pub area_codes: Option<Vec<String>>,
    /// `requirement.state` in this list.
    pub states: Option<Vec<RequirementState>>,
    /// `requirement.priority` in this list.
    pub priorities: Option<Vec<Priority>>,
    /// Case-insensitive literal substring of `key` or `body`.
    pub text: Option<String>,
}

/// Result of an amend or withdraw (plan D9): the edit landed, or someone else committed first.
#[derive(Debug, Clone, PartialEq)]
pub enum RequirementUpdate {
    /// The compare-and-set matched; this is the new head.
    Updated(Requirement),
    /// The compare-and-set found a different version.
    Diverged {
        /// The row as it is now.
        head: Requirement,
        /// The revision at the version the caller edited from.
        ancestor: RequirementRevision,
    },
}
```

## 4. Store seam (`htui-core/src/store/traits.rs`, T3 and T6)

The imports (32-42) gain `Resolution`, `RequirementAreaId`, `RequirementId`, `CitationKind`,
`CoverageRow`, `ItemCitation`, `ItemRequirement`, `NewRequirement`, `NewRequirementArea`,
`Requirement`, `RequirementArea`, `RequirementFilter`, `RequirementPatch`,
`RequirementRevision`, `RequirementSpec` and `RequirementUpdate`.

### 4.1 `ReadStore` (T6): append after `resolve_inputs` (line 182)

Every read's `# Errors` is the same one line: "The backend's own failures only."

```rust
    // ---- ANA-11 §5.1: requirements (MOD-38) --------------------------------------------------

    /// The project's spec header, or `None` when it has none (mirrored, plan D12).
    async fn requirement_spec(&self, project: ProjectId) -> Result<Option<RequirementSpec>>;

    /// The project's areas in `(position, code)` order, code by bytes; empty for an unknown project.
    async fn requirement_areas(&self, project: ProjectId) -> Result<Vec<RequirementArea>>;

    /// The project's requirements matching `filter`, in `(area_code, number)` order, area_code by bytes.
    async fn requirements(
        &self,
        project: ProjectId,
        filter: &RequirementFilter,
    ) -> Result<Vec<Requirement>>;

    /// One requirement, or `None`.
    async fn requirement(&self, id: RequirementId) -> Result<Option<Requirement>>;

    /// The requirement's revisions in `version` order; `Some(vec![])` for an unknown id on
    /// `MemStore`/`PgStore`, and `None` = not cached (the mirror holds no revisions, plan D12).
    async fn requirement_revisions(
        &self,
        id: RequirementId,
    ) -> Result<Option<Vec<RequirementRevision>>>;

    /// The item's live citations with `suspect` derived (plan D11), in
    /// `(requirement.area_code, requirement.number, kind)` order, all by bytes; empty for an unknown item.
    async fn item_requirements(&self, item: ItemId) -> Result<Vec<ItemCitation>>;

    /// The requirement's live citations with each citing item's status and resolution, in
    /// `(item.key_prefix, item.key_number, item.id, kind)` order, text by bytes; empty for an unknown id.
    async fn requirement_coverage(&self, requirement: RequirementId) -> Result<Vec<CoverageRow>>;
```

The text filter is the same on every store (D14). A match is a case-insensitive literal
substring of `key` or of `body`:

- **Pg:** `position(lower($n::text) in lower(r.key)) > 0 OR position(lower($n::text) in
  lower(r.body)) > 0`.
- **Mem:** `to_lowercase().contains(...)`.
- **Cache:** `instr(lower(..), lower(?)) > 0`.

Ordering is by bytes, so Pg writes `ORDER BY r.area_code COLLATE "C", r.number` and the other
orderings likewise.

### 4.2 `WriteStore` (T6): append after `add_note` (line 1057)

```rust
    // ---- ANA-11 §5.1: requirements and citations (MOD-38) ------------------------------------

    /// Compare-and-set write of the spec header (plan D9): `expected_version: None` inserts
    /// version 1 when the project has no header, `Some(v)` updates the header at `v` to `v + 1`.
    /// A token that does not match the stored row (including `None` when a row exists) is
    /// `Ok(Stale(row as it is))`.
    ///
    /// # Errors
    /// `NotFound { entity: "requirement_spec" }` for `Some(_)` with no header; `Constraint` for an unknown project or owner.
    async fn set_requirement_spec(
        &self,
        project: ProjectId,
        expected_version: Option<i32>,
        owner_id: UserId,
        preamble: String,
    ) -> Result<CasOutcome<RequirementSpec>>;

    /// Inserts one `requirement_area`.
    ///
    /// # Errors
    /// `Constraint` for a code outside `^[A-Z][A-Z0-9]{1,15}$` ([`invalid_area_code`]), a code the project already has, a duplicate id or an unknown project.
    async fn create_requirement_area(&self, new: NewRequirementArea) -> Result<RequirementArea>;

    /// Mints the area's next number, the requirement and its revision 1 (`reason = "created"`)
    /// in one statement; a refused mint consumes no number (plan D8).
    ///
    /// # Errors
    /// `NotFound { entity: "requirement_area" }` for an unknown area; `Constraint` for a duplicate id or a `created_by` / `box_id` that names no row.
    async fn mint_requirement(
        &self,
        area: RequirementAreaId,
        new: NewRequirement,
    ) -> Result<Requirement>;

    /// Compare-and-set amend, one transaction (PRD D3): the row at `version + 1`, its revision
    /// with `amended_by_item_id = amended_by`, and `amended_by`'s `amends` citation upserted at the
    /// new version (a tombstone revived). Checked in the order NotFound, divergence, Constraint.
    ///
    /// # Errors
    /// `NotFound { entity: "requirement" }`; `Constraint` for a withdrawn requirement ([`requirement_withdrawn`]) or an `amended_by` / author / box that names no row.
    async fn amend_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        patch: RequirementPatch,
        amended_by: ItemId,
    ) -> Result<RequirementUpdate>;

    /// [`amend_requirement`](WriteStore::amend_requirement) for `state = withdrawn`: revision
    /// `reason = "withdrawn"`, and `withdrawn_by`'s `withdraws` citation at the new version.
    ///
    /// # Errors
    /// As [`amend_requirement`](WriteStore::amend_requirement), an already-withdrawn requirement included.
    async fn withdraw_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        withdrawn_by: ItemId,
        author_id: UserId,
        box_id: Option<BoxId>,
    ) -> Result<RequirementUpdate>;

    /// Upserts a live citation stamped at the requirement's current version, reviving a tombstone
    /// and overwriting `proposed_by_step_id` (plan D10).
    ///
    /// # Errors
    /// `NotFound` `"item"` then `"requirement"`; `Constraint` for `addresses`/`reserves` of a withdrawn requirement ([`withdrawn_requirement_cited`]) or an unknown step.
    async fn cite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        proposed_by: Option<StepId>,
    ) -> Result<ItemRequirement>;

    /// Tombstones a live citation (`deleted_at = now`).
    ///
    /// # Errors
    /// `NotFound { entity: "item_requirement", id: citation_key(..) }` when no live row matches.
    async fn uncite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<()>;

    /// Re-stamps a live citation at the requirement's current version, clearing `suspect`.
    ///
    /// # Errors
    /// `NotFound { entity: "item_requirement", id: citation_key(..) }` when no live row matches.
    async fn reconfirm(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<ItemRequirement>;
```

Two rules of `Diverged` and of the tombstones:

- `Diverged { head, ancestor }`: `ancestor` is the revision at `expected_version`. The miss path
  mirrors `update_item`'s (`pg/write.rs:69` `cas_miss`; Mem `update` at `mem.rs:1265`).
- A requirement is never deleted outside a project cascade, so there is no tombstone on
  `requirement`.

### 4.3 `close_out` (T3): replaces lines 1029-1050

```rust
    /// `R-TUI-9`'s three effects, one transaction (plan D6): the summary document at its next
    /// version, the commits upserted, and the item set to `closed` with `resolution` and
    /// `closed_at`. Close-out is the **only** way into `closed` (PRD D1; it amends ANA-2 §4.3);
    /// its law is [`Resolution::closes_from`], not [`legal_move`], so an `open` item closes as
    /// one of the four non-success resolutions (ANA-11 §4.2). Refused while any run of the item
    /// is active. Guard order: NotFound, live run, summary kind, summary item, `closes_from`,
    /// commit steps.
    ///
    /// # Errors
    /// Its own refusals, plus - because it performs their work - every refusal of
    /// [`record_commits`](WriteStore::record_commits) and
    /// [`write_document`](WriteStore::write_document). In full:
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "item" }`, or
    /// `{ entity: "run_step" }` for a commit row naming a step that does not exist;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when a run of the item is
    /// `queued | running | awaiting_approval`, when `summary.kind != "summary"`, when
    /// `summary.item_id != item`, when `!resolution.closes_from(status)`
    /// ([`resolution_not_closable`]), when a commit row names an unknown repo, or when the summary
    /// duplicates a document id or names an unknown `created_by` / `produced_by_step_id`. Any
    /// refusal writes nothing: every one of these is decided before the first write.
    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document>;
```

The `transition` doc (205-211) gains: "`to = closed` is refused from every status (PRD D1); use
[`close_out`](WriteStore::close_out)."

Store bodies:

- **Mem (`mem.rs:4065-4111`)**:
  - Replace `legal_move(status, Status::Closed)?;` with:

    ```rust
    if !resolution.closes_from(status) {
        return Err(StoreError::Constraint(resolution_not_closable(item, status, resolution)));
    }
    ```

  - Replace `self.transition(item, status, Status::Closed, now)?;` with the direct write (D5):

    ```rust
    let row = self.items.get_mut(&item).expect("require_item found it above");
    row.status = Status::Closed;
    row.resolution = Some(resolution);
    row.updated_at = now;
    row.closed_at = Some(now);
    ```

  - The wrapper at `:4761` threads `resolution`.
- **Pg (`pg/write.rs:3902-4023`)**:
  - `legal_move(status, Status::Closed)?` at :3957 becomes the same `closes_from` check.
  - The UPDATE at :3987 becomes:

    ```
    UPDATE item SET status = 'closed', resolution = $3, closed_at = clock_timestamp()
     WHERE id = $1 AND status = $2
    ```

    It binds `resolution.as_str()`.
- `Writer` (`writer.rs:879`), `UsageSpy` (`htui-agent/src/conformance.rs:1061`) and `SpyStore`
  (`htui-agent/tests/recorder.rs:755`) forward the extra argument.

### 4.4 `DeleteReach` (T6): append after `documents` (line 1442)

```rust
    /// `requirement_spec` rows (0 or 1), MOD-38.
    pub requirement_specs: u64,
    /// `requirement_area` rows.
    pub requirement_areas: u64,
    /// `requirement_key_counter` rows, which cascade from `requirement_area`.
    pub requirement_key_counters: u64,
    /// `requirement` rows.
    pub requirements: u64,
    /// `requirement_revision` rows of the project's requirements.
    pub requirement_revisions: u64,
    /// `item_requirement` rows whose item **or** requirement is in the project, tombstones
    /// included: a cross-project citation goes with either end, as `links` does.
    pub item_requirements: u64,
```

The struct doc's first paragraph gains: "MOD-38's six requirement fields are counted on both
stores."

- **Pg `project_reach` (T8)** adds the CTEs:

  ```sql
  ra AS (SELECT id FROM requirement_area WHERE project_id = $1),
  rq AS (SELECT id FROM requirement      WHERE project_id = $1)
  ```

  and the selects:

  ```sql
  (SELECT count(*) FROM requirement_spec WHERE project_id = $1)          AS "requirement_specs!",
  (SELECT count(*) FROM ra)                                              AS "requirement_areas!",
  (SELECT count(*) FROM requirement_key_counter
    WHERE area_id IN (SELECT id FROM ra))                                AS "requirement_key_counters!",
  (SELECT count(*) FROM rq)                                              AS "requirements!",
  (SELECT count(*) FROM requirement_revision
    WHERE requirement_id IN (SELECT id FROM rq))                         AS "requirement_revisions!",
  (SELECT count(*) FROM item_requirement
    WHERE item_id IN (SELECT id FROM i)
       OR requirement_id IN (SELECT id FROM rq))                         AS "item_requirements!"
  ```

- `workspace_reach` (pg :284, mem :2777) already uses `..DeleteReach::default()` and needs no
  change.
- **Mem `delete_project` (T7)** removes all six. It also sets
  `requirement_revision.amended_by_item_id = None` where that id names a deleted item, which is
  the `ON DELETE SET NULL` of a revision in another project.
- `hierarchy.rs` labels, in field order:

  | Field | Label |
  |---|---|
  | `requirement_specs` | `"requirement specs"` |
  | `requirement_areas` | `"requirement areas"` |
  | `requirement_key_counters` | `"requirement key counters"` |
  | `requirements` | `"requirements"` |
  | `requirement_revisions` | `"requirement revisions"` |
  | `item_requirements` | `"citations"` |

## 5. Error-string helpers

They live in `htui-core/src/store/traits.rs`, in the helper block (1136-1334). Put them after
`summary_names_another_item` (≈:1235) under a new banner:

```
// ---- MOD-38: ANA-11's refusals ----
```

Each one is re-exported from `htui-core/src/store/mod.rs:15-24`: add the names to the
`pub use traits::{…}` list, alphabetised.

```rust
/// ANA-11 §4.2: the close-out law refuses this pair (T3).
#[must_use]
pub fn resolution_not_closable(item: ItemId, status: Status, resolution: Resolution) -> String {
    format!("item {item} is `{status}`; it cannot close as `{resolution}` (ANA-11 §4.2)")
}

/// Plan D10: a withdrawn requirement is not amended or withdrawn again (T6).
#[must_use]
pub fn requirement_withdrawn(key: &str) -> String {
    format!("requirement {key} is withdrawn")
}

/// Plan D10: a withdrawn requirement takes no new `addresses` / `reserves` citation (T6).
#[must_use]
pub fn withdrawn_requirement_cited(key: &str, kind: CitationKind) -> String {
    format!("requirement {key} is withdrawn; it takes no new `{kind}` citation")
}

/// The `requirement_area.code` CHECK, in words; both stores check it before the insert (T6).
#[must_use]
pub fn invalid_area_code(code: &str) -> String {
    format!("requirement_area.code `{code}` is not `^[A-Z][A-Z0-9]{{1,15}}$`")
}

/// The `id` of an `item_requirement` `NotFound`: its primary key, `item/requirement/kind` (T6).
#[must_use]
pub fn citation_key(item: ItemId, requirement: RequirementId, kind: CitationKind) -> String {
    format!("{item}/{requirement}/{kind}")
}
```

Existing helpers are reused, with these argument spellings. MemStore uses the sentences; Pg may
surface its own FK/UNIQUE text through `map_sqlx`. Conformance therefore matches only the
`Constraint` variant for these, and asserts exact sentences only for the 4 new sentence helpers
above.

- `already_exists("requirement_area", id)`, `already_exists("requirement_area.code", code)`,
  `already_exists("requirement", id)`.
- `references_no_row(..)`, one per foreign key:

  | Column | Table |
  |---|---|
  | `"requirement_spec.project_id"` | `"project"` |
  | `"requirement_spec.owner_id"` | `"app_user"` |
  | `"requirement_area.project_id"` | `"project"` |
  | `"requirement_revision.amended_by_item_id"` | `"item"` |
  | `"item_requirement.proposed_by_step_id"` | `"run_step"` |

- Mem's `require_author` (`mem.rs:4228`) for `created_by` / `author_id`.

Two helpers are deliberately **not** added:

- `closed_only_via_close_out`: `transition(_, _, Closed)` is already refused by `legal_move` →
  `illegal_move("item", from, "closed")`.
- A `RevisionReason` enum: `reason` has no CHECK.

## 6. Conformance cases and pins

### 6.1 `CASES` (`htui-core/src/store/conformance.rs:37-91`, arms in `run_case`)

The pins move 53 → **55** in T3, then → **65** in T6. There are two pin sites:

- `htui-core/tests/mem_store.rs:36-45`: the literal and the message. The message gains ", MOD-38's
  two close-out cases (plan D4) and ten requirement cases (plan D9-D14)".
- `htui-store/tests/pg_conformance.rs:19` `EXPECTED_CASES`.

`htui-orch/tests/fake_conformance.rs:16` stays at 70, because T4 adds no orch case.

**T3 (append after `release_lease_frees_the_run_for_its_own_sweep`):**

1. **`close_out_resolution_law`.**
   - For every HTUI fixture item (every `Status` appears once, per
     `fixtures::every_status_appears_in_the_htui_project`) whose `runs()` holds no active run,
     and every `Resolution` `r` with `!r.closes_from(status)`: `close_out` is exactly
     `Constraint(resolution_not_closable(item, status, r))`. The item row and `documents(item)`
     are unchanged.
   - Then `HTUI_ANA_2` (`open`) + `Done` is refused, and `HTUI_ANA_2` + `Withdrawn` succeeds:
     `status == Closed`, `resolution == Some(Withdrawn)`, `closed_at.is_some()`, and the
     summary is at v1.
   - `HTUI_FIX_1` (`closed`) refuses all six.
2. **`transition_never_reaches_closed`.**
   - For every HTUI fixture item, `transition(item, status, Closed)` is `Constraint` whose
     sentence equals `illegal_move("item", status, Status::Closed)`, and the row is
     byte-identical.
   - An unknown id is still `NotFound` first (plan D14 precedence).

**T3 edits to existing cases:**

- `no_delete_path` (:715-730): replace the two transitions with
  `close_out(HTUI_ANA_2, Resolution::Withdrawn, new_document(HTUI_ANA_2, "summary", None), &[])`
  from `open`, and update the leading comment.
- `close_out_refuses_a_live_run` (:6255):
  - every call gains a resolution: the live-run leg `Resolution::Withdrawn`; the three refusal
    legs become 4-tuples, with the ANA_2 leg `(HTUI_ANA_2, Resolution::Done, …,
    "open does not close as done (ANA-11 §4.2)")` and the other two `Resolution::Done`;
  - the unknown-item leg `Withdrawn`; the success leg `Resolution::Done`;
  - it adds `assert_eq!(closed.resolution, Some(Resolution::Done))`.
- `illegal_transitions_are_constraint` (:6388): the 4th pair is `(Status::Done, Status::Open)`.
  Its comment becomes "`done` reaches `open` and nothing else, so seven of its eight targets are
  refusals…".

**T6 (append after `transition_never_reaches_closed`; failing until T7/T8):**

3. **`requirement_mint_is_per_area_and_never_reused`.**
   - Minting in `AREA_ENT` gives `R-ENT-3` (number 3, v1, `Active`, `area_code "ENT"`, project
     HTUI). Minting in `AREA_STO` gives `R-STO-2`.
   - A mint with `created_by: UserId::nil()` is `Constraint`, and so is one with a duplicate id.
     The next ENT mint is `R-ENT-4`: no number was burned.
   - An unknown area is `NotFound{"requirement_area"}`.
   - `requirement_revisions(R-ENT-3)` is `Some([v1 reason "created"])`.
4. **`requirement_area_create_checks_the_code`.**
   - `"ent"`, `"E"`, `"E-1"` and a 17-byte code are each exactly
     `Constraint(invalid_area_code(code))`.
   - `ENT` in HTUI (taken), a duplicate id and an unknown project are `Constraint`.
   - `ENT` in AGY is accepted. `requirement_areas(AGY)` reads it back in `(position, code)`
     order.
5. **`requirement_amend_is_cas_and_names_the_item`.**
   - `amend(REQ_STO_1, 1, body "…", amended_by HTUI_FEAT_3)` gives `Updated` at v2. Revision 2
     has `amended_by_item_id == Some(FEAT_3)` and `reason == patch.reason`.
   - A second amend at 1 gives `Diverged { head: v2, ancestor: revision v1 }`.
   - An unknown id is `NotFound{"requirement"}`. An unknown `amended_by` is `Constraint`, and
     the version stays 2.
6. **`requirement_withdraw_refuses_new_addresses`.**
   - `withdraw(REQ_ENT_2, 1, HTUI_FEAT_3, USER, Some(BOX))` gives `Updated` with `state ==
     Withdrawn` at v2, revision reason `"withdrawn"`.
   - `cite(AGY_FEAT_1, ENT_2, Addresses)` is exactly
     `Constraint(withdrawn_requirement_cited("R-ENT-2", Addresses))`. Reviving FEAT_2's
     tombstoned `Reserves` is refused the same way.
   - A second withdraw and an amend are exactly `Constraint(requirement_withdrawn("R-ENT-2"))`.
7. **`amend_records_the_deciding_citation`** (PRD D3).
   - After amending `REQ_STO_1` by FEAT_3, `item_requirements(FEAT_3)` holds `Amends` at
     `requirement_version 2`, `suspect false`.
   - `uncite` then a second amend by FEAT_3 revives it at v3. It is still one row.
8. **`a_newer_version_makes_a_citation_suspect_until_reconfirmed`.**
   - FEAT_1 → STO_1 `Addresses` at v1 is not suspect. After amending STO_1 it is suspect in both
     `item_requirements(FEAT_1)` and `requirement_coverage(STO_1)`.
   - `reconfirm` restamps it to 2 and clears suspect.
   - `reconfirm` of an absent triple is `NotFound{"item_requirement", citation_key(..)}`.
9. **`uncite_tombstones_and_cite_revives`.**
   - `uncite(FIX_1, STO_1, Addresses)` drops the citation from both reads. A second `uncite`
     and a `reconfirm` are `NotFound`.
   - `cite` revives it at the current version. `cite` revives the fixture tombstone FEAT_2 →
     ENT_2 `Reserves`.
   - An unknown item is `NotFound{"item"}`, an unknown requirement is
     `NotFound{"requirement"}`, and an unknown step is `Constraint`.
10. **`coverage_lists_citing_items_with_resolution`.**
    - `cite(HTUI_ANA_2, STO_1, Addresses)`, then
      `close_out(HTUI_ANA_2, Withdrawn, …)`.
    - `requirement_coverage(STO_1)` is, in key order, ANA-2 (Closed, `Some(Withdrawn)`), FEAT-1
      (InProgress, `None`) and FIX-1 (Closed, `Some(Done)`).
11. **`spec_is_cas`.**
    - `requirement_spec(AGY)` is `None`. `set(AGY, Some(1), …)` is
      `NotFound{"requirement_spec"}`.
    - Then `set(AGY, None, USER, "p")` → `Applied(v1)`; `set(AGY, None, …)` → `Stale(v1 "p")`;
      `set(AGY, Some(1), "q")` → `Applied(v2)`; `set(AGY, Some(1), "r")` → `Stale(v2 "q")`.
    - `set(ProjectId::new(), None, …)` is `Constraint`.
12. **`project_delete_counts_requirements`.**
    - `cite(AGY_FEAT_1, REQ_STO_1, Addresses)`, then `delete_reach(Project(HTUI))` ==
      `delete_project` report, with `(requirement_specs, requirement_areas,
      requirement_key_counters, requirements, requirement_revisions, item_requirements) ==
      (1, 2, 2, 3, 4, 6)`.
    - After the delete, `item_requirements(AGY_FEAT_1)` is empty.

### 6.2 `READ_CASES` (:219-229, arms in `run_read_case`)

The pin moves 9 → **14** in T6 (`mem_store.rs:47-50`; the message gains ", and MOD-38's five
requirement reads (plan D12)"). These cases run on Mem, Pg and the mirror (`tests/cache.rs:1041`).

1. **`requirement_spec_and_areas_read_back`.**
   - `requirement_spec(HTUI)` is the fixture row: v1, owner USER.
   - `requirement_spec(AGY)` is `None`.
   - `requirement_areas(HTUI)` is `[ENT, STO]`, positions 0 and 1; `requirement_areas(AGY)` is
     empty.
2. **`requirements_filter_by_area_state_priority_and_text`.**
   - Default filter: `[R-ENT-1, R-ENT-2, R-STO-1]`.
   - `area_codes ["STO"]` → `[R-STO-1]`; `priorities [Later]` → `[R-ENT-2]`;
     `states [Withdrawn]` → `[]`.
   - `text "r-ent"` → both ENT rows; `text "MIRROR"` → `[R-STO-1]` (body match).
   - `requirement(id)` equals the list row; an unknown id is `None`.
3. **`item_citations_derive_suspect`.**
   - `item_requirements(HTUI_ANA_1)` is `[R-ENT-1 Addresses, stamp 1, suspect true]`.
   - `HTUI_ANA_2` is `[R-ENT-1 Amends, stamp 2, suspect false]`.
   - `HTUI_FEAT_2` is `[]` (tombstone). `AGY_FEAT_1` is `[]`.
4. **`coverage_carries_status_and_resolution`.**
   - `requirement_coverage(REQ_ENT_1)` is `[ANA-1 Addresses suspect, ANA-2 Amends]`.
   - `REQ_STO_1` is `[FEAT-1 (InProgress, None), FIX-1 (Closed, Some(Done))]`.
   - `item(HTUI_FIX_1).resolution == Some(Done)`.
5. **`requirement_revisions_or_not_cached`.**
   - `requirement_revisions(REQ_ENT_1)` is `None` (the mirror), or exactly
     `Some([v1 "created" amended_by None, v2 "amended" amended_by Some(HTUI_ANA_2)])`.
   - An unknown id is `None` or `Some([])`.

### 6.3 Non-conformance tests (named here, owned by the listed task)

- **T2**, `htui-store/tests/migrations.rs`:
  - `item_resolution_iff_closed_rejects_both_halves`: a raw INSERT of `closed` with NULL
    `resolution` is refused, and so is `open` with `'done'`.
  - `closed_rows_backfill_to_done`: `run_to(4)`, insert a closed item, `MIGRATOR.run`, then read
    `resolution = 'done'`.
- **T5**, `htui-store/tests/cache.rs`: `a_closed_item_mirrors_its_resolution`, plus an
  `assert_eq!(MIRRORED_TABLES.len(), 21)`.
- **T8**, `htui-store/tests/pg_criteria.rs`: `concurrent_requirement_mints_produce_consecutive_numbers`,
  modelled on `:162`. It runs N concurrent `mint_requirement(AREA_ENT)` and expects numbers
  `3..3+N` with no gap and no duplicate.
- **T9**, `htui-store/tests/cache.rs`:
  - `the_mirror_drops_a_tombstoned_citation`: `uncite` on Pg, `run_pass`, then the row is gone
    from `item_requirement`.
  - `a_suspect_citation_reads_offline`: amend on Pg, `run_pass`, then the cache's
    `item_requirements` has `suspect` set.

## 7. Orchestrator and TUI (T4)

- **`htui-orch/src/command.rs:130-133`:**

  ```rust
      /// `R-TUI-9`'s close-out (plan D167): one `summary` document and the item `closed` as
      /// `resolution` (MOD-38 PRD D2). The engine builds the summary; the caller names the item
      /// and the resolution - until MOD-39, the one [`close_out_enabled`] answers.
      CloseOut {
          /// The item to close.
          item: ItemId,
          /// Why it closes (ANA-11 §4.2).
          resolution: Resolution,
      },
  ```

- **`command.rs:1165` `close_out_enabled`:**

  ```rust
  /// ... the refusals `WriteStore::close_out` re-checks inside its own transaction, and the
  /// resolution the Runs pane closes with ([`Resolution::default_for`], plan D6).
  ///
  /// # Errors
  /// [`EngineError::RunStatus`] naming the first live run, then [`EngineError::NotClosable`]
  /// when `default_for(item.status)` is `None`.
  pub fn close_out_enabled(item: &Item, runs: &[Run]) -> Result<Resolution, EngineError> {
      if let Some(live) = runs.iter().find(|run| run.status.is_active()) { /* unchanged */ }
      Resolution::default_for(item.status).ok_or(EngineError::NotClosable {
          item: item.id,
          status: item.status,
      })
  }
  ```

  In its test (:2127), the `is_ok()` loop becomes `assert_eq!(close_out_enabled(..).ok(),
  Resolution::default_for(status))`, which is `Done` → `Done` and `Failed`/`Blocked` →
  `Withdrawn`. The refusals are unchanged.
- **`engine.rs`:**
  - :577 becomes
    `Command::CloseOut { item, resolution } => self.close_out(item, resolution).await`.
  - :1480 becomes `async fn close_out(&self, item: ItemId, resolution: Resolution)`. It keeps
    `crate::command::close_out_enabled(&reads.item, &reads.runs)?;` with the value discarded;
    until MOD-39 an `open` item is refused here. It then calls
    `self.parts.store.close_out(item, resolution, summary, &[])`.
  - T3's placeholder at :1493 is
    `let resolution = Resolution::default_for(reads.item.status).unwrap_or(Resolution::Withdrawn);`,
    which T4 deletes.
  - `close_out_preview` (:1978): `let resolution = crate::command::close_out_enabled(..)?;`
    then `closeout::preview(&reads.item, resolution, &reads.summaries, &reads.commits, &heads)`.
  - The test at :11656 adds `resolution` to its asserted tuple.
- **`closeout.rs`:**
  - `Preview` gains, after `status`:

    ```rust
        /// What the close-out closes as: `Resolution::default_for(status)` until MOD-39.
        pub resolution: Resolution,
    ```

  - `pub fn preview(item: &Item, resolution: Resolution, runs: &[RunSummary], commits:
    &[(StepId, Vec<RunStepCommit>)], heads: &[DocumentHead]) -> Preview`.
  - The test literal at :593 adds `resolution`.
- **`htui-orch/src/conformance.rs`:**
  - :5409 and :5462 become `Command::CloseOut { item, resolution: Resolution::Done }`, or
    whatever `default_for` gives for the driven status.
  - After :5450 add `assert_eq!(row.resolution, Some(…))`.
- **`htui/src/run_worker.rs`:**
  - :366 is `close_out_enabled(..).map(|_| ())`.
  - :2055 becomes `Command::CloseOut { item, .. }`.
  - The test literal at :4184 adds `resolution: Resolution::Withdrawn`.
- **`htui/src/ui/tabs/backlog/detail/runs.rs`:**
  - :716 becomes `Command::CloseOut { item, resolution: preview.resolution }`.
  - The Warn line at :1260-1268 becomes

    ```
    "close {} as {} · {} runs · {} commit rows · summary v{}"
    ```

    with `(key, resolution, runs, rows, version)`.
  - The test `preview()` (:2492) adds `resolution`; the expected Command (:2610) adds it.
  - New render test `the_close_out_warn_names_the_resolution`: a `failed` item shows
    `"as withdrawn"`.
  - Snapshot `htui/tests/snapshots/backlog__runs_closeout_warn.snap` is re-accepted.
- **`htui/tests/runs_pg.rs`:**
  - :759 and :840 become `Command::CloseOut { item, resolution: Resolution::Done }`, matching
    the item's `done` status there.
  - T3 already did :779 and :812 (direct `close_out(item, Resolution::Done, …)`).
  - After :906, assert `resolution == Some(Done)`.

## 8. Fixture contract (T6 data, T7 Mem load, T10 assertions)

`htui-core/src/fixtures.rs`:

- `mod class`: `REQUIREMENT_AREA: u8 = 18`, `REQUIREMENT: u8 = 19`. Spec, counter, revision and
  citation rows have composite keys and need no class.
- `ids` (`demo_ids!`, imports gain `RequirementAreaId, RequirementId`):
  - `AREA_ENT: RequirementAreaId = (class::REQUIREMENT_AREA, 0)`
  - `AREA_STO: RequirementAreaId = (class::REQUIREMENT_AREA, 1)`
  - `REQ_ENT_1: RequirementId = (class::REQUIREMENT, 0)`
  - `REQ_ENT_2: RequirementId = (class::REQUIREMENT, 1)`
  - `REQ_STO_1: RequirementId = (class::REQUIREMENT, 2)`

  `every_id_is_distinct` (:1760) lists all five.
- `DemoData` gains, after `documents` or at the end:

  ```rust
      /// `requirement_spec` rows (ANA-11 §5): HTUI only.
      pub requirement_specs: Vec<RequirementSpec>,
      /// `requirement_area` rows.
      pub requirement_areas: Vec<RequirementArea>,
      /// `requirement_key_counter` rows, keyed by area.
      pub requirement_key_counter: HashMap<RequirementAreaId, i32>,
      /// `requirement` rows.
      pub requirements: Vec<Requirement>,
      /// `requirement_revision` rows.
      pub requirement_revisions: Vec<RequirementRevision>,
      /// `item_requirement` rows, tombstones included.
      pub item_requirements: Vec<ItemRequirement>,
  ```

- Data. All rows are in `PROJECT_HTUI`, `created_by`/`author_id` is `USER`, and `box_id` is
  `Some(BOX)`.
  - **Spec:** v1, owner USER, preamble `"Requirements of the htui demo project."`,
    `updated_at demo_at(2, 9)`.
  - **Areas:**
    - `AREA_ENT`: `ENT`, `"Entity model"`, description `""`, position 0, `demo_at(2, 9)`.
    - `AREA_STO`: `STO`, `"Storage"`, description `""`, position 1, `demo_at(2, 9)`.
  - **Counters:** `{AREA_ENT: 2, AREA_STO: 1}`.
  - **Requirements:**
    - `REQ_ENT_1` = R-ENT-1, `Must`, `Active`, **v2**, body
      `"Every item has a stable key of the form PREFIX-N."`, rationale
      `"Keys are what people type."`, created `demo_at(2, 10)`, updated `demo_at(4, 10)`.
    - `REQ_ENT_2` = R-ENT-2, `Later`, `Active`, v1, body
      `"An item may carry a free-form priority."`, rationale `""`, created and updated
      `demo_at(2, 11)`.
    - `REQ_STO_1` = R-STO-1, `Must`, `Active`, v1, body
      `"Postgres is the source of truth; the cache is a read-only mirror."`, rationale `""`,
      created and updated `demo_at(2, 12)`.
  - **Revisions (4):**

    | Requirement | Version | Body | Reason | amended_by | At |
    |---|---|---|---|---|---|
    | ENT_1 | v1 | `"Every item has a key."` | `"created"` | None | `demo_at(2, 10)` |
    | ENT_1 | v2 | current | `"amended"` | `Some(HTUI_ANA_2)` | `demo_at(4, 10)` |
    | ENT_2 | v1 | current | `"created"` | None | `demo_at(2, 11)` |
    | STO_1 | v1 | current | `"created"` | None | `demo_at(2, 12)` |

  - **Citations (5).** All have `proposed_by_step_id: None`; `created_at = updated_at =
    demo_at(3, n)` with `n = 0..4` in this order.

    | Item | Requirement | Kind | Stamp | Note |
    |---|---|---|---|---|
    | `HTUI_ANA_1` | ENT_1 | `Addresses` | 1 | suspect |
    | `HTUI_ANA_2` | ENT_1 | `Amends` | 2 | `updated_at demo_at(4, 10)` |
    | `HTUI_FEAT_1` | STO_1 | `Addresses` | 1 | |
    | `HTUI_FIX_1` | STO_1 | `Addresses` | 1 | |
    | `HTUI_FEAT_2` | ENT_2 | `Reserves` | 1 | `deleted_at Some(demo_at(5, 9))` |

- `items()` (:1016, T1): `resolution: (spec.status == Status::Closed).then_some(Resolution::Done)`,
  which makes FIX-1 `Done`.
- **`pg/demo.rs` (T6):** `query!` inserts in FK order, after the `item_link` loop (:461-477)
  and before the commit: spec, areas, counters, requirements (never `key`, which is generated),
  revisions, citations. It passes `updated_at` explicitly (the trigger is BEFORE UPDATE only).
  The item insert (:351-378, **T2**) adds `resolution` (`row.resolution.map(Resolution::as_str)`).

## 9. Per-task checklist

### T1: `Resolution` and the close-out law

- [ ] `htui-core/src/model/item.rs:8-65`: `Resolution`, `closes_from`, `default_for`,
  `can_move_to` loses the 3 `→ Closed` edges, and the doc is updated (§3.1).
- [ ] `item.rs:105`: `Item.resolution`.
- [ ] `item.rs:267-296`: `SANCTIONED` loses 3 rows; add `CLOSE_OUT_SANCTIONED` and the 3 tests.
- [ ] `model/mod.rs:113`: re-export; `mod.rs:~180`: `resolution_matches_check_list`.
- [ ] `model/mod.rs:293` (`status_terminality_matches_readiness_rule`) is unaffected, because
  `is_terminal` does not change. Leave it.
- [ ] `Item` literals:
  - `fixtures.rs:1016-1036` (resolution keyed on `Status::Closed`);
  - `store/mem.rs:1243` (`resolution: None`);
  - `htui-orch/src/closeout.rs:242` (`None`);
  - `htui-store/src/cache/read.rs:178` (`resolution: None` for now).
- [ ] Gate: `cargo test -p htui-core --lib`. The conformance suites stay red until T3.

### T2: Migration 0006 and item selects

- [ ] `htui-store/migrations/0006_requirements.sql`: §1, verbatim.
- [ ] `pg/read.rs:130-158`: add `resolution AS "resolution: htui_core::model::Resolution"`.
  Check the other `FROM item` selects in `pg/read.rs` that build `Item`.
- [ ] `pg/write.rs:398-461` `mint_item`: CTE `i` RETURNING gains `resolution`; the SELECT gains
  `i.resolution AS "resolution?: htui_core::model::Resolution"`.
- [ ] `pg/write.rs:481-592` `update_item`: the same in its `Item` select(s).
- [ ] `pg/demo.rs:351-378`: the item insert gains `resolution`.
- [ ] `htui-store/tests/migrations.rs`:
  - `TABLES` 33 → 39 (18-51, adding the 6 new tables);
  - `vec![1, 2, 3, 4]` → `vec![1, 2, 3, 4, 5]` (:74, :591);
  - the 0004 test (:559) runs `run_to(4)` explicitly;
  - the two §6.3 T2 tests.
- [ ] `cargo sqlx prepare -- --all-targets --all-features`; commit `.sqlx/*`.

### T3: `close_out(item, resolution, …)`

- [ ] `store/traits.rs:1029-1050`: the §4.3 signature and doc, plus the `transition` doc line.
- [ ] `traits.rs` helpers: `resolution_not_closable`; `store/mod.rs:15-24` re-export.
- [ ] `mem.rs:4065-4111`: D5 body (§4.3); `mem.rs:4761` wrapper; test `mem.rs:7143-7230` (every
  call gains a resolution; the ANA_2 leg is `Done`; assert `resolution`).
- [ ] `pg/write.rs:3902-4023`: the `closes_from` guard (:3957) and the UPDATE (:3987).
- [ ] `writer.rs:879-889`, `htui-agent/src/conformance.rs:1061-1067`,
  `htui-agent/tests/recorder.rs:755-761`: forward `resolution`.
- [ ] `store/conformance.rs`: 2 new cases + arms; edit `no_delete_path`,
  `close_out_refuses_a_live_run` and `illegal_transitions_are_constraint` (§6.1).
- [ ] Pins: `mem_store.rs:37` → 55; `pg_conformance.rs:19` → 55.
- [ ] `htui-store/tests/pg_criteria.rs:553`: the stale-from check moves to
  `(Status::Done, Status::Open)`.
- [ ] `htui/tests/runs_pg.rs:779, :812`: `close_out(item, Resolution::Done, …)`.
- [ ] `htui-orch/src/engine.rs:1493`: the placeholder resolution (§7).
- [ ] `.sqlx/*` regenerated (the close_out UPDATE).

### T4: Orchestrator and Runs pane (∥ T5)

- [ ] `htui-orch/src/command.rs:130-133` and `:1165-1180`, test `:2127` (§7).
- [ ] `engine.rs:577`, `:1480-1498`, `:1978-1988`, test `:11656`.
- [ ] `closeout.rs:19-58`, test literals `:224` (none needed after T1) and `:593`.
- [ ] `htui-orch/src/conformance.rs:5409`, `:5450`, `:5462`.
- [ ] `htui/src/run_worker.rs:366`, `:2055`, `:4184`.
- [ ] `htui/src/ui/tabs/backlog/detail/runs.rs:716`, `:1260-1268`, `:2492`, `:2610`, plus the
  new render test.
- [ ] `htui/tests/runs_pg.rs:759`, `:840`, `:906`.
- [ ] Snapshot `htui/tests/snapshots/backlog__runs_closeout_warn.snap`.

### T5: Cache migration and `item.resolution` (∥ T4)

- [ ] `htui-store/cache_migrations/0004_requirements.sql`: §2, verbatim.
- [ ] `cache/mod.rs:40-58`: `[&str; 21]`, appending `"requirement_spec", "requirement_area",
  "requirement", "item_requirement"`; the doc gains a MOD-38 sentence.
- [ ] `cache/refresh.rs:820-839`: `ITEM_COLUMNS` gains `"resolution"` last; `refresh_item`
  (:841-901) selects it and binds `row.resolution` (TEXT).
- [ ] `cache/read.rs:156-180`: `resolution: get::<Option<Resolution>>(row, "resolution")?`, and
  the `item` select (:332-343) gains the column.
- [ ] `htui-store/tests/cache.rs`: the §6.3 T5 test.
- [ ] `.sqlx/*` regenerated (the `refresh_item` query).

### T6: Models, seam, fixture, cases

- [ ] `model/requirement.rs` (new, §3.4); `model/ids.rs:113` (§3.3); `model/mod.rs`
  (§3.2 T6 items).
- [ ] `store/traits.rs`:
  - §4.1 reads after :182;
  - §4.2 writes after :1057;
  - §4.4 `DeleteReach` fields after :1442;
  - §5 helpers (minus T3's), with the `store/mod.rs` re-exports.
- [ ] Forwarding, complete:
  - `htui-store/src/backend.rs:614-759` (3 arms per read: Memory, Online{pg}, Offline{cache});
  - `writer.rs:159-286` reads and `:288-897` writes;
  - `htui-agent/src/conformance.rs:603-672` and `:674-1072`;
  - `htui-agent/tests/recorder.rs:282-352` and `:354-766`.
- [ ] Stubs (`unimplemented!("MOD-38 T7")` / `T8` / `T9`):
  - `mem.rs` `impl ReadStore` (:4237-4320) and `impl WriteStore` (:4322-4774);
  - `pg/read.rs:60-832`; `pg/write.rs` WriteStore impl;
  - `cache/read.rs:240-1034`.
- [ ] `DeleteReach` compile sites (F2):
  - `mem.rs:2845` (six `0`s, `// T7`);
  - `pg/write.rs:355` (six `0`s, `// T8`);
  - `htui/src/hierarchy.rs:479-526` (`[_; 28]`, §4.4 labels);
  - `htui-store/tests/pg_criteria.rs` (`TABLES` + `requirement_spec, requirement_area,
    requirement_key_counter, requirement, requirement_revision, item_requirement`, destructure,
    `claimed: [u64; 28]`).
- [ ] `fixtures.rs`: class codes, ids, `DemoData` fields and data (§8); `every_id_is_distinct`.
- [ ] `pg/demo.rs`: the requirement inserts (§8); `.sqlx/*` regenerated.
- [ ] `store/conformance.rs`: the 10 T6 cases in `CASES` and the 5 in `READ_CASES`, with arms.
  Pins: `mem_store.rs` 65 / 14; `pg_conformance.rs` 65.

### T7: MemStore (`mem.rs` only)

- [ ] `State` (:81-166) gains:

  ```rust
  requirement_specs: HashMap<ProjectId, RequirementSpec>,
  requirement_areas: HashMap<RequirementAreaId, RequirementArea>,
  requirement_key_counter: HashMap<RequirementAreaId, i32>,
  requirements: HashMap<RequirementId, Requirement>,
  requirement_revisions: Vec<RequirementRevision>,
  item_requirements: Vec<ItemRequirement>,
  ```

  The last two follow the `links` pattern.
- [ ] `from_demo` (:207-252) loads them.
- [ ] Reads (§4.1 ordering and text rules).
- [ ] Writes (§4.2): check order, `require_author` before the counter (as `mint` :1198), and
  the §5 sentences.
- [ ] `project_reach` (:2845) makes the counts real; `delete_project` (:2976) removes them and
  applies the `SET NULL` rule (§4.4).
- [ ] Gate: `cargo test -p htui-core --features test-support,demo --test mem_store`, all green.

### T8: PgStore (`pg/*`, `.sqlx`, `pg_criteria.rs`)

- [ ] `pg/read.rs`: the 7 reads. `suspect` is `r.version > ir.requirement_version`; coverage
  joins `item` for `ItemSummary` + `resolution`.
- [ ] `pg/write.rs` `mint_requirement`:
  - the CTE is `a` (area row) → `c` (`INSERT INTO requirement_key_counter (area_id, last_value)
    SELECT id, 1 FROM a ON CONFLICT (area_id) DO UPDATE SET last_value =
    requirement_key_counter.last_value + 1 RETURNING last_value`) → `r` (insert with
    `a.project_id, a.id, a.code, c.last_value`) → `v` (revision 1 `'created'`);
  - zero rows → `NotFound{"requirement_area"}`.
- [ ] Amend and withdraw: one tx. `SELECT … FOR UPDATE` → NotFound; version mismatch →
  `Diverged` (as `cas_miss` :69); withdrawn → `Constraint`; then UPDATE, revision INSERT, and the
  citation upsert:

  ```sql
  INSERT … ON CONFLICT (item_id, requirement_id, kind)
  DO UPDATE SET requirement_version = EXCLUDED.requirement_version, deleted_at = NULL
  ```

- [ ] `cite`, `uncite`, `reconfirm`, `set_requirement_spec` and `create_requirement_area` (code
  pre-checked in Rust).
- [ ] `project_reach` (:304-379): §4.4 SQL.
- [ ] `pg_criteria.rs`: `concurrent_requirement_mints_produce_consecutive_numbers`.
- [ ] `cargo sqlx prepare -- --all-targets --all-features`.

### T9: Cache (`cache/refresh.rs`, `cache/read.rs`, `tests/cache.rs`, `.sqlx`)

- [ ] `refresh.rs:208`: consts `REQUIREMENT_SPEC`, `REQUIREMENT_AREA`, `REQUIREMENT`,
  `ITEM_REQUIREMENT`.
- [ ] The loop list (:257-269) appends them in that order after `RUN_STEP_TREE`, and explicit
  arms go before the `unreachable!` (:288).
- [ ] New fns:
  - `refresh_requirement_spec` (`WHERE project_id = $1 AND updated_at > $2`);
  - `refresh_requirement_area`;
  - `refresh_requirement`;
  - `refresh_item_requirement`: `WHERE (item_id IN (items of $1) OR requirement_id IN
    (requirements of $1)) AND updated_at > $2`, with a tombstone → `DELETE`, modelled on
    `refresh_item_link` :903-972.
- [ ] Each fn has its `*_COLUMNS` const.
- [ ] `cache/read.rs`: the 7 reads; `requirement_revisions` → `Ok(None)`; INNER JOINs (F13).
- [ ] `tests/cache.rs`: the §6.3 T9 tests. The read cases run via `:1041`.
- [ ] `.sqlx/*` regenerated (four new `query!`s).

### T10: Fixture assertions and delete text

- [ ] `htui-store/tests/migrations.rs:1095` `load_demo_round_trips_a_count_per_table` gains
  `requirement_spec 1`, `requirement_area 2`, `requirement_key_counter 2`, `requirement 3`,
  `requirement_revision 4`, `item_requirement 5`.
- [ ] `fixtures.rs` tests:
  - `the_fixture_has_one_suspect_citation`: exactly `HTUI_ANA_1 → REQ_ENT_1`;
  - `fix_1_is_closed_as_done`;
  - `requirement_counters_sit_above_every_minted_number`, modelled on :1820.
- [ ] `htui/src/hierarchy.rs` tests (:576-610): one `reach_parts` case including `"3
  requirements"` and `"6 citations"`. `hierarchy__delete_*.snap` delete `vulkan-tutorials`,
  which holds no requirements, so they are unchanged.

### T11: Close-out

- [ ] As in the plan. The write-up also records F1 (6 fields, not 3) and F10
  (`chk_item_resolution_iff_closed`).
