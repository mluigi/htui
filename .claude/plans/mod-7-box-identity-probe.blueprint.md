# Blueprint: MOD-7 milestone 1, "a box knows itself"

**Status**: **draft, for maintainer acceptance.** Findings F-A to F-T (§0) and decisions D19–D38 (§12) are proposed here. Where a finding says **Blocker**, the plan read literally either does not compile, fails its own named test, breaks the workspace gate, or leaves a hidden coupling between parallel tasks. The Fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-7-box-identity-probe.plan.md`, confirmed 2026-09-25 with every OQ default, OQ-11 and OQ-12 included. It covers D1–D18 and R-1 to R-11. Its "(amended at fact-check)", "(amended at maintainer review)" and "(amended at re-check)" notes, and its Verified-claims table, take precedence over its earlier prose, and this blueprint follows them. **PRD**: `.claude/prds/mod-7-box-registry.prd.md`, milestone 1. PRD D0–D7 win over this blueprint where they disagree.

**Verified at**: HEAD `56b68eb`, branch `mod-7-box-registry`. `git diff --stat 1475b17 HEAD -- crates/ Cargo.toml Cargo.lock` is empty, so the plan's line numbers still hold. Every signature below was checked by opening the real symbol, through Gortex `read`/`search`/`relations` or the file itself. **Line numbers are pre-edit**: a citation into a file a task edits moves after that task's first commit. `crates/htui-store/.sqlx/` holds 227 files. `df -h /` shows 128 GB free (71 % used).

**Graphify**: `graphify-out/` does not exist in this checkout, so nothing here comes from it.

**Scope**:
- **Order**:
  1. T0 alone.
  2. Wave A: lane 1 is T1 then T2 (serial, one worktree); lane 2 is T3 (own worktree).
  3. Merge T1, then T2, then T3, re-running the touched crates' gates on the real tree after each merge.
  4. T4.
  5. T5.
- **One migration**: `0005_box_identity.sql`.
- **Two new `WriteStore` methods**: `record_box_probe` and `boxes`.
- **One new module**: `htui_agent::box_probe`, with `mod.rs`, `hardware.rs`, `spec.rs` and `spec.json`.
- **Request enums**: one new `StoreRequest` (63 → 64) and one new `StoreReply` (34 → 35).
- **Dependencies**: three new workspace dependencies (`hmac`, `windows-registry`, `sysinfo`). `Cargo.lock` gains exactly three packages.
- **Pins that move**: store `CASES` 53 → 56, applied migrations `[1..4]` → `[1..5]`, pending 4 → 5, commented columns 25 → 29, `.sqlx` 227 → 229 (T1) → 234 (T2).

**House style (carried)**:
- `unsafe_code = "forbid"`. Every lib warns on `missing_docs`, and the workspace warns on `missing_debug_implementations`. The gate runs clippy with `-D warnings`.
- A hand-written redacting `Debug` for anything that could hold a secret.
- Nothing sets `updated_at` by hand.
- No `std` guard is held across an `.await`.
- Implementers commit incrementally, staging their own paths only (never `-A`, never `stash`). Every commit compiles, and a red commit uses `todo!()` bodies.
- Every gate is re-run with `--test-threads=1` on the real tree after the merge (project memory).
- The raw machine identity is never a struct field, a column, a log field, a test assertion over this machine's value, or a file.

---

## 0. Findings the plan fact-check missed

| # | Blocker? | Plan says | Tree at `56b68eb` | Fix |
|---|---|---|---|---|
| **F-A** | **Blocker** (compile/logic) | D2: `enum Registration { New, Known { renamed_from }, Copied { previous: BoxId } }`, `register_box(..) -> Result<Registration>`, and "the id it carries (the minted one for `Copied`) becomes `identity.box_id`". | `Copied { previous }` carries only the **old** id, so `bootstrap` (`pg/mod.rs:418-424`) has no way to learn the minted one. | D19: `Copied { previous: BoxId, minted: BoxId }`, plus `Registration::box_id(&self, presented: BoxId) -> BoxId`. `bootstrap` sets `identity.box_id = this_box = reg.box_id(identity.box_id)`. |
| **F-B** | **Blocker** (D17 cannot hold) | D6: `Hardware { .., gpu: Option<GpuVendor> }` is produced by `HardwareSource`, and the vendor map lives in `spec.json`'s `gpu_vendors`, which D17 lets a stored overlay replace. | A hardware source that takes no spec cannot honour an overlaid vendor map. The test `a_stored_vendor_replaces_by_pci_id_and_keeps_priority` could only pass by threading the spec into the seam, and `FixedHardware` would then have to re-implement the mapping. | D21: `Hardware.display_vendors: Vec<String>` holds raw lowercase PCI vendor ids (`"0x1002"`). `probe_box` maps them through `spec.gpu_vendors` in priority order (`box_probe::pick_gpu_vendor`). The hardware seam knows nothing about the spec. |
| **F-C** | **Blocker** (contradicts its own test) | D17: the whole overlay is rejected "when a tag rule names a tool the merged list lacks". T3 names `a_disabled_tool_and_its_tag_rule_drop_out`. | Disabling `cmake` leaves the **seeded** `cmake` rule naming an absent tool. Under D17 as written, the overlay is rejected and the seed probed, so the test fails. | D23: a disabled tool is first pruned from every **seeded** rule's `any_tool`. A seeded rule left with no tool and no `fact` is dropped. The "names an absent tool" refusal applies only to rules the overlay itself writes. |
| **F-D** | **Blocker** (the registration probe would fail silently) | D13: the registration task replies at `UNSOLICITED`. `run_probe`'s pattern (`agent_worker.rs:1742-1750`) reports a write failure as `StoreReply::Failed`. | `App::on_reply` surfaces `Failed` only **below** the freshness gate (`app/update.rs:164-173`). An `UNSOLICITED` `Failed` is dropped with no status line and no log. | D25: the box task never sends `Failed`. Every outcome is one `StoreReply::BoxProbed(BoxProbeReport)`, and failures go in `box_failed` / `agents_failed`. `observe_reply` (above the gate) renders `report.status_line()`. `ProbeBox`'s **pre-spawn** refusals stay `Failed` at the requester's own seq. |
| **F-E** | **Blocker** (gate: `-D warnings`) | D13: `agents.rs::declares_a_source` and `agent_worker.rs:2003` delegate to `htui_agent::launch::declares_install`. | `AgentLaunch` is used only inside those two bodies (`agents.rs:45` import used only at `:1303`; `agent_worker.rs:40` import used only at `:2004`). After delegation both imports are unused, and `unused_imports` fails clippy. | T4 removes `AgentLaunch` from `agents.rs:45` and from `agent_worker.rs:40` (which keeps `AgentSettings`). |
| **F-F** | **Blocker** (T4 cannot test, or `--demo` probes) | D11: "`Backend::Memory` never auto-probes (OQ-10)", while every T4 runtime test drives `on_online` over `Backend::Memory`. | Both claims hold only if the **call sites** are the guard. `on_online` itself must accept `Memory`. Separately, `go_online` can return early without swapping (`store_worker.rs:1709-1711`), leaving `Offline`. | D26: `on_online` refuses nothing by backend kind. It returns early when `!registration_probe` or `backend.writer()` is `None`. It is called only right after the two `go_online` calls, and `--demo`/the harness never reach them. |
| **F-G** | **Blocker** (harness and shutdown) | D11 adds `AgentRuntime.box_probe: Option<JoinHandle<()>>`, "like `install` and `auth`". | `finish_background` (`:466-497`) and `shutdown` (`:844-893`) name only `background`, `install` and `auth`. `Harness::drive_to_end` (`testkit.rs:418`) would never await a `ProbeBox` task, so its `BoxProbed` reply would never be driven. `shutdown` would leave the task, and its bounded children, running past the UI. | D27: both methods handle `box_probe`: `finish_background` awaits it under `limit` and aborts it on overrun, and `shutdown` aborts it (its `ChildGuard`s kill the children). |
| **F-H** | **Blocker** (the seam cannot be implemented) | D6: "`HardwareSource` … one method returning a boxed future of `Hardware`." | The macOS and Windows GPU children go through `run_bounded(what, &ResolvedLaunch, &ProbeEnv)` (`probe.rs:982`), which needs the `ProbeEnv` for `cwd` and `version_timeout`. A method with no env cannot bound them. | D22: `fn read<'a>(&'a self, env: &'a ProbeEnv) -> HardwareFuture<'a>`, and `HardwareSource: Send + Sync + Debug`. |
| **F-I** | **Blocker** (digest is circular) | D18: `digest = sha256_hex(&serde_json::to_string(&effective_spec))`. | `EffectiveSpec { spec, digest, error }` contains the digest itself, and `error` would make an ignored overlay hash differently from the seed. That contradicts `the_digest_is_stable_and_changes_with_the_spec` ("an ignored overlay → the seed's digest"). | D24: `spec::digest(&Spec)` hashes only the merged `Spec`, through its derived `Serialize` (`tools` is a `BTreeMap`; `tags` and `gpu_vendors` keep merged order). `EffectiveSpec.digest` is that value. |
| **F-J** | **Blocker** (Windows checkouts) | T3: "`tests/fixtures/box_probe/**` … PCI device trees". | Real PCI directory names are `0000:0f:00.0`. A committed path containing `:` cannot be checked out on Windows, so the whole repo would fail to clone there (MOD-16's box). | D29: no committed PCI trees. `tests/box_probe.rs` builds them in a `tempdir` at run time with `0000_0f_00.0`-style names; the scanner reads any directory name. Only captured **text** (`system_profiler`, `PNPDeviceID`) is committed. |
| **F-K** | **Blocker** (Postgres conformance) | T2: `record_box_probe_replaces_profile_and_tools` reads back "every hardware column … and `probed_at` as written". | `TIMESTAMPTZ` keeps microseconds; `MemStore` keeps nanoseconds. A `Utc::now()` probe compares equal on `MemStore` and fails on `PgStore`. The new `CHECK` also refuses a digest that is not 64 lowercase hex, which `MemStore` would accept. | D33: cases use fixed instants truncated to micros (`trunc_subsecs(TIMESTAMPTZ_DIGITS)`) and digests from `sha256_hex`. D32: `MemStore::record_box_probe` refuses a malformed digest with `Constraint`, matching the `CHECK`. |
| **F-L** | Non-blocker (correctness) | D5: `HTUI_VERSION = env!("CARGO_PKG_VERSION")` "in `htui_store` … is the one definition". | Each crate declares its own `version = "0.1.0"` (`crates/htui/Cargo.toml:3`, `crates/htui-store/Cargo.toml:3`), not `version.workspace`. A release that bumps only the binary's manifest would never re-probe. | D28: T4 adds `htui_version_is_the_binary_s_version`, which asserts `htui_store::HTUI_VERSION == env!("CARGO_PKG_VERSION")` inside `htui`, so a lopsided bump fails the gate. |
| **F-M** | Non-blocker (a probe can be lost) | D11: `on_online` "skip[s] … unless `claim_is_free`". | `claim_is_free` (`:1267-1295`) refuses on any unfinished `background` task, which includes a chat's staleness re-probe (`:1497`) and a preview (`:1006`). A reconnect that lands while one runs skips the registration probe until the next swap or restart. | Accepted as **R-12**. `on_online` logs `info!(reason = %err, "the registration probe waits for the next connect")`. `ProbeBox` stays available on demand. |
| **F-N** | Non-blocker (a false identity) | D1: "Anything unreadable, empty or timed out is `None`." | systemd writes `uninitialized` into `/etc/machine-id` on first-boot images (`machine-id(5)`), and every such image would share one fingerprint. | D20: the Linux reader accepts only 32 hex characters after trimming. Anything else is "not this file" and the reader tries the next one. |
| **F-O** | Non-blocker (the grep gate is not literal) | T3 Validate: the grep "finds nothing outside `#[cfg(test)]` modules". | A reviewer would have to separate test modules from code by eye. | D29: no `#[cfg(test)]` module in `box_probe/*.rs`. Every box-probe test lives in `tests/box_probe.rs`, so the grep over `src/box_probe/*.rs` must print nothing at all. |
| **F-P** | Non-blocker (the ProbeAgents env) | D12: `ProbeArgs.cwd` becomes `env: ProbeEnv`. | `probe()` builds its env from `std::env::current_dir()` (`:935-937`). | D35: `probe()` uses `self.probe_env()`, the injected env when present and `ProbeEnv::host(cwd)` otherwise. Existing `ProbeAgents` tests inject nothing, so their behaviour is unchanged. |
| **F-Q** | Non-blocker (the column-comment pin) | D4/T1: `ANA_COLUMN_COMMENTS` "gains the four `0005` comments" and "25 → 29". | That constant's doc says it holds the ANA texts of `0002`/`0003` "verbatim". Folding MOD-7's texts in would make that doc false. | D30: a sibling `MOD7_COLUMN_COMMENTS` (four rows). The test checks both lists verbatim, and its exclusivity query and message use the union (29). |
| **F-R** | Non-blocker (count pin) | "about 233" `.sqlx` files. | T1 removes 1 and adds 3, if the `Copied` insert **reuses** the first insert's text. T2 adds 5. | D31: 227 → **229** after T1, → **234** after T2. The exact count is recorded at close. |
| **F-S** | Non-blocker (a prepare that cannot re-run) | Validation: prepare against `htui_prepare_check` "migrated through 0005". | If `0005` is edited after the first `sqlx migrate run`, `htui_prepare_check` holds a checksum-mismatched `0005` and refuses the next migrate. | T1's gate recreates a dedicated scratch DB `htui_prepare_mod7` from zero before each prepare (§3.8). `htui_prepare_check` is migrated once, at T1 close, for the plan's own `--check` line. |
| **F-T** | Non-blocker (H-7 for T4 and T5) | T4/T5 inject "a fake `PATH`". | `ProbeEnv` also carries `home` (the Glob tier expands `~`, `probe.rs` `expand`) and `cwd` (the NodePackage tier reads `<cwd>/node_modules`). `fresh_db` seeds the real `claude`/`agy` rows. | Every T4/T5 env is built by one helper: `cwd` is a tempdir, `home: None`, `vars` holds `PATH` only, and `versions: true`. Every agent row is rewritten to the unresolvable launch before any probe (the `agent_worker.rs:5020` / `tests/probe.rs:17-35` pattern). |

### 0a. Settled answers to the brief's questions

| Question | Answer | Where |
|---|---|---|
| Where each type lives | `Fingerprint` → `htui_store::identity`. `Registration`, `HTUI_VERSION` → `htui_store::pg` (re-exported at the root). `BoxProbe`, `ProbedTool`, `BoxRecord` → `htui_core::model::box_`. `Hardware`, `HardwareSource`, `SystemHardware`, `FixedHardware` → `htui_agent::box_probe::hardware`. `Spec`, `TagRule`, `Fact`, `Ask`, `GpuVendor`, `EffectiveSpec` → `htui_agent::box_probe::spec`. `BoxProbeReport` → `htui::agent_worker`. There is no `ProbeSpec` type: the plan's word for it is `Spec`, and the "overlay" is a `serde_json::Value` walked by `spec::effective` (D23), with no public overlay type. | §2, §3, §5, §6 |
| Merge and digest | Computed **only** in `htui_agent::box_probe::spec::effective`. T4 calls it once per task and uses the same `EffectiveSpec` for the decision and the probe. | D23, D24 |
| How `probe_spec_digest` flows | `EffectiveSpec.digest` → `probe_box` copies it into `BoxProbe.spec_digest` → `record_box_probe` writes `box.probe_spec_digest` → `boxes()` returns `BoxRecord.probe_spec_digest` → `BoxRecord::needs_probe(HTUI_VERSION, &effective.digest)`. | §2, §4, §6 |
| The `app_settings` read path | `Backend::app_settings()` (`backend.rs:396-402`), called inside the box task over a `Backend` clone. The entry is `map.get(box_probe::SETTING_KEY)` with `SETTING_KEY = "box_probe_spec"`. | D26 |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (minimum; each compiles) | Gate |
|---|---|---|---|
| T0 foundations | root manifests, htui-core | 2: (a) types with `todo!()` `needs_probe` + 5 tests (red); (b) body + deps (green) | `cargo test -p htui-core --all-features -- --test-threads=1`; the `cargo tree` checks in §2.4 |
| T1 identity | htui-store (worktree A) | 4 (§3.9) | the Postgres `htui-store` line; prepare + `--check`; grep |
| T2 write surface | htui-core, htui-store, htui-agent spies (worktree A, after T1) | 2 (§4.6) | `htui-core`, `htui-store` (Postgres) and `htui-agent` gates; prepare + `--check` |
| T3 box probe | htui-agent (worktree B) | 3 (§5.8) | `cargo test -p htui-agent --all-features -- --test-threads=1`; clippy; the grep |
| merge | — | T1, then T2, then T3 | after **each** merge: the gates of the crates it touched, on the real tree; after T3 the `htui-agent` gate again, because T3's worktree lacked T2's trait methods |
| T4 trigger | htui | 4 (§6.9) | `cargo test -p htui --all-features -- --test-threads=1`; workspace clippy |
| T5 end to end | htui tests | 1–2 | the workspace gate (§9), then the live check |

---

## 2. T0: foundations (D5, D10, D18)

**First failing test**: `a_box_never_probed_needs_a_probe`.

### 2.1 `Cargo.toml` `[workspace.dependencies]` (after `sha2`, house-style comments)

```toml
# MOD-7 D1: the box fingerprint, HMAC-SHA256 keyed by the OS machine identity. 0.12 because it
# shares `digest 0.10` with `sha2` above and is already compiled on Linux (dbus-secret-service).
hmac               = "0.12"
# MOD-7 D1: `MachineGuid` on Windows (htui-store only). 0.6.1 is already in the lock via hyper-util.
windows-registry   = "0.6"
# MOD-7 D6 (OQ-2 answered "use sysinfo"): OS version, CPU brand and total RAM on all three
# platforms. `system` only: the defaults would also pull `component`, `disk`, `network` and `user`.
sysinfo            = { version = "0.39.6", default-features = false, features = ["system"] }
```

### 2.2 Manifests

- `crates/htui-store/Cargo.toml`:
  - `[dependencies]` gains `hmac = { workspace = true }`.
  - A **new** `[target.'cfg(windows)'.dependencies]` table (none exists today) holds `windows-registry = { workspace = true }`.
- `crates/htui-agent/Cargo.toml` `[dependencies]` gains `sysinfo = { workspace = true }`, with a `# MOD-7 D6` comment.

### 2.3 `crates/htui-core/src/model/box_.rs` (after `BoxTool`, `:83`)

```rust
/// One tool the box probe found (MOD-7 plan D9): a future `box_tool` row without its box or instant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbedTool {
    /// `box_tool.name`: the key of the probe spec's `tools` map, never a file name.
    pub name: String,
    /// `box_tool.version`; empty for a presence-only tool.
    pub version: String,
    /// `box_tool.path`: the file `which` resolved.
    pub path: String,
}

/// Everything one box probe learned, as `WriteStore::record_box_probe` writes it (MOD-7 D10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxProbe {
    /// The probed box.
    pub box_id: BoxId,
    /// `box.os_version`.
    pub os_version: String,
    /// `box.cpu`.
    pub cpu: String,
    /// `box.ram_mb`.
    pub ram_mb: Option<i32>,
    /// `box.gpu_present`.
    pub gpu_present: bool,
    /// `box.gpu_vendor`: the spec's vendor name, `None` without a GPU.
    pub gpu_vendor: Option<String>,
    /// The whole `box_tool` set; replaces the previous one. Names are unique.
    pub tools: Vec<ProbedTool>,
    /// `box.probed_tags`, sorted and deduplicated.
    pub probed_tags: Vec<String>,
    /// `box.htui_version`: the version that probed (D5).
    pub htui_version: String,
    /// `box.probe_spec_digest`: sha256 hex of the effective spec (D18).
    pub spec_digest: String,
    /// `box.last_probed_at` and every `box_tool.probed_at`.
    pub probed_at: DateTime<Utc>,
}

/// One box as `WriteStore::boxes` lists it (MOD-7 D10, D18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxRecord {
    /// The row.
    pub row: BoxRow,
    /// Its `box_tool` rows, name-byte-ordered.
    pub tools: Vec<BoxTool>,
    /// `box.probe_spec_digest`, which is not a `BoxRow` field (D14 keeps every constructor still).
    pub probe_spec_digest: Option<String>,
}

impl BoxRecord {
    /// Whether the next `Online` swap must probe this box (D5, D18): never probed, probed by another
    /// `htui`, or under another effective spec.
    #[must_use]
    pub fn needs_probe(&self, running: &str, spec_digest: &str) -> bool {
        self.row.last_probed_at.is_none()
            || self.row.htui_version != running
            || self.probe_spec_digest.as_deref() != Some(spec_digest)
    }
}
```

`model/mod.rs:99-101` re-exports `BoxProbe`, `BoxRecord` and `ProbedTool`.

**Tests** (`box_.rs` `mod tests`, over the existing `row()` helper):

| Test | Asserts |
|---|---|
| `a_box_never_probed_needs_a_probe` | `last_probed_at: None` gives `true`, even at the same version and digest. |
| `a_box_probed_by_another_version_needs_a_probe` | `htui_version "0.4.1"` against `"0.4.2"` gives `true`. |
| `a_box_probed_at_this_version_needs_none` | Same version and same digest give `false`. |
| `a_changed_spec_digest_needs_a_probe` | A stored digest `a…` against a presented `b…` gives `true`. |
| `a_box_with_no_recorded_digest_needs_a_probe` | `probe_spec_digest: None` gives `true`. |

### 2.4 Gate

```bash
cargo test -p htui-core --all-features -- --test-threads=1
cargo tree -i hmac@0.12.1 | head                       # now also <- htui-store
cargo tree --target x86_64-pc-windows-msvc -i windows-registry@0.6.1 | grep htui-store
git diff Cargo.lock | grep '^+name = '                 # exactly: ntapi, objc2-io-kit, sysinfo
grep -c '^name = "windows"$' Cargo.lock                # 1
cargo tree --target x86_64-unknown-linux-gnu -p sysinfo -e normal   # sysinfo, libc, memchr
```

---

## 3. T1: `htui-store`, the id is the key and the fingerprint checks it (D1–D5, D19, D20, D30, D31)

**First failing test**: `tests/box_identity.rs::a_hostname_change_keeps_the_box_id_and_registers`, written against the **old** signature. It fails with `StoreError::Constraint("box_pkey: …")`.

### 3.1 `migrations/0005_box_identity.sql` (full text)

```sql
-- --------------------------------------------------------------------------------------------
-- 0005_box_identity.sql - MOD-7 milestone 1: a box is keyed on its box.toml id, not its hostname.
-- Forward-only (R-STO-5): 0001_init.sql is never edited; this file moves what it has to.
--
-- ANA-16 C4 and PRD D1: UNIQUE (user_id, hostname) made a renamed box a duplicate primary key
-- (box.toml keeps the id across a rename) and a cloned hostname a merge into another box's row.
-- Registration now looks the row up by id and checks a keyed machine fingerprint instead.
--
-- Three columns, none mirrored (MIRRORED_TABLES unchanged, no cache migration):
--   machine_fingerprint  the keyed hash only; the CHECK makes "no raw identity" a schema fact.
--   edit_version         milestone 2's compare-and-set token; nothing reads or writes it yet.
--   probe_spec_digest    re-probes when app_setting.box_probe_spec changes (plan D18).
-- --------------------------------------------------------------------------------------------

ALTER TABLE box DROP CONSTRAINT box_user_id_hostname_key;

ALTER TABLE box ADD COLUMN machine_fingerprint TEXT
    CHECK (machine_fingerprint IS NULL OR machine_fingerprint ~ '^[0-9a-f]{64}$');
ALTER TABLE box ADD COLUMN edit_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE box ADD COLUMN probe_spec_digest TEXT
    CHECK (probe_spec_digest IS NULL OR probe_spec_digest ~ '^[0-9a-f]{64}$');

COMMENT ON COLUMN box.machine_fingerprint IS
    'MOD-7 D1: HMAC-SHA256 keyed by the OS machine identity (/etc/machine-id, IOPlatformUUID, MachineGuid) over the constant htui/box-fingerprint/v1, lowercase hex. NULL means no identity was readable. It checks a box.toml id and never keys the row: a mismatch means a copied box.toml, and a new box is minted.';
COMMENT ON COLUMN box.edit_version IS
    'MOD-7 D14: compare-and-set token of the declared_tags, quirks and settings editors, bumped by them only. Registration and the probe never write it, so a reconnect cannot stale an open editor.';
COMMENT ON COLUMN box.probe_spec_digest IS
    'MOD-7 D18: sha256 hex of the effective box probe spec (the compiled seed overlaid by app_setting.box_probe_spec) at the last successful probe. Another digest re-probes at the next connect. NULL means never probed since 0005.';
COMMENT ON COLUMN box.htui_version IS
    'MOD-7 D5: the htui version at the last successful probe, and the re-probe trigger (R-BOX-2). Inserted at first registration; rewritten only by the probe writer, never by a reconnect.';
```

The four comment texts contain no apostrophe, on purpose: `MOD7_COLUMN_COMMENTS` holds them as Rust literals byte for byte.

### 3.2 `identity.rs` (D1, D20)

```rust
/// The compiled message of the box fingerprint's HMAC (plan D1): the domain separator that keeps
/// the value uncorrelated with any other program's use of the same machine identity.
pub const FINGERPRINT_APP_ID: &[u8] = b"htui/box-fingerprint/v1";

/// HMAC-SHA256 keyed by the normalised OS machine identity over [`FINGERPRINT_APP_ID`] (PRD D1).
///
/// No `Display`, no `Serialize`; `Debug` prints `Fingerprint(<redacted>)`. [`as_hex`] exists only to
/// bind `box.machine_fingerprint`.
#[derive(Clone, PartialEq, Eq)]
pub struct Fingerprint([u8; 32]);

impl core::fmt::Debug for Fingerprint { /* f.write_str("Fingerprint(<redacted>)") */ }

impl Fingerprint {
    /// Trims and ASCII-lowercases `raw`; empty → `None`. Otherwise the keyed hash.
    #[must_use]
    pub fn from_machine_identity(raw: &str) -> Option<Self>;
    /// Lowercase hex, 64 characters: what the column stores.
    #[must_use]
    pub fn as_hex(&self) -> String;
}

/// This machine's fingerprint, read from the OS once per process; `None` when no identity is readable.
pub async fn machine_fingerprint() -> Option<Fingerprint>;   // static tokio::sync::OnceCell<Option<Fingerprint>>
```

Body of `from_machine_identity`:
1. `let key = Zeroizing::new(raw.trim().to_ascii_lowercase());` and return `None` if it is empty.
2. `let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_bytes()).ok()?;` (HMAC accepts any key length).
3. `mac.update(FINGERPRINT_APP_ID);`
4. `Some(Self(mac.finalize().into_bytes().into()))`.

Readers (private; the raw text is only ever a `Zeroizing` local):

| Fn | cfg | Behaviour |
|---|---|---|
| `fn machine_id_under(root: &Path) -> Option<Zeroizing<String>>` | `any(target_os = "linux", test)` | Reads `<root>/etc/machine-id`, then `<root>/var/lib/dbus/machine-id`. A file counts only if its trimmed content is 32 hex characters (**D20**: `uninitialized` and an empty file fall through). |
| `fn ioreg_platform_uuid(text: &str) -> Option<&str>` | `any(target_os = "macos", test)` | Takes the value of the line containing `"IOPlatformUUID" = "…"`. |
| `async fn read_os_identity() -> Option<Zeroizing<String>>` | per OS | **Linux**: `spawn_blocking(|| machine_id_under(Path::new("/")))`. **macOS**: `tokio::process::Command::new("/usr/sbin/ioreg").args(["-rd1", "-c", "IOPlatformExpertDevice"]).kill_on_drop(true).output()` under `tokio::time::timeout(Duration::from_secs(5), ..)`; `stdout` goes into `Zeroizing<Vec<u8>>` and is parsed. **Windows**: `spawn_blocking(|| windows_registry::LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Cryptography").and_then(|k| k.get_string("MachineGuid")).ok().map(Zeroizing::new))`. **Other**: `None`. |

The doc fixes are in the same edit: module doc `:1-5`, and `store`'s "used by the adopt-DB-id rule" (`:96`) becomes "used when registration minted a new id for a copied `box.toml`".

Unit tests (`identity.rs` `mod tests`):

| Test | Asserts |
|---|---|
| `the_fingerprint_is_hmac_sha256_known_answer` | `from_machine_identity("0123456789abcdef0123456789abcdef").as_hex() == "a2e8a0ed177cf8b655e4a8c2d16f745209325966a8339fd26e7c6437dae364df"`. |
| `the_fingerprint_ignores_case_and_surrounding_whitespace` | `"  0123…CDEF\n"` equals the lowercase value. |
| `an_empty_identity_is_no_fingerprint` | `""` and `" \n"` give `None`. |
| `debug_never_prints_the_fingerprint` | `format!("{fp:?}") == "Fingerprint(<redacted>)"`. |
| `linux_reads_machine_id_then_the_dbus_copy` | In a temp root: only the dbus copy gives it; both give `/etc`'s; `uninitialized` in `/etc` falls through to dbus; neither gives `None`. |
| `the_ioreg_line_is_parsed` | A captured sample gives the UUID; text without the line gives `None`. |
| `this_box_has_a_fingerprint_when_machine_id_is_readable` | `cfg(target_os = "linux")`. If `/etc/machine-id` is readable, `machine_fingerprint().await.is_some()`. Asserts nothing about the value. |

### 3.3 `pg/mod.rs` (D2, D3, D5, D19)

```rust
/// The `htui` version this build records (plan D5): inserted at first registration, rewritten by
/// the probe writer, compared by `BoxRecord::needs_probe`.
pub const HTUI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What [`PgStore::register_box`] found (plan D2, D3, blueprint D19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// The id was new: one row inserted.
    New,
    /// The id was known and belongs to this machine; `renamed_from` is the old hostname on a rename.
    Known {
        /// The hostname the row carried before this registration, when it changed.
        renamed_from: Option<String>,
    },
    /// `box.toml` was carried from another machine or another user: `previous` is left untouched
    /// and this machine registered as `minted`.
    Copied {
        /// The row the copied `box.toml` named.
        previous: BoxId,
        /// The id this machine now has.
        minted: BoxId,
    },
}

impl Registration {
    /// The id this box has after registering `presented`.
    #[must_use]
    pub const fn box_id(&self, presented: BoxId) -> BoxId;
}
```

- `PgStore` (`:53-59`) gains `registration: Option<Registration>`. It is `None` in `connect_with` (`:146-151`) and `lazy` (`:176-182`) until `bootstrap`.
- A new accessor: `pub const fn registration(&self) -> Option<&Registration>`.
- `bootstrap` (`:418-424`):

```rust
self.this_user = self.seed_if_empty().await?;
let fingerprint = crate::identity::machine_fingerprint().await;
let registration = self.register_box(&self.identity, fingerprint.as_ref()).await?;
let id = registration.box_id(self.identity.box_id);
self.identity.box_id = id;
self.this_box = id;
self.registration = Some(registration);
```

`register_box` (replaces `:337-377`; its callers are `bootstrap` and `tests/migrations.rs` only):

```rust
pub async fn register_box(&self, identity: &Identity, fingerprint: Option<&Fingerprint>) -> Result<Registration>
```

It runs as one transaction (`self.pool.begin()`), with three statements. The `Copied` insert reuses statement (a)'s text (D31).

```sql
-- (a) insert-first: the only statement that can create the row
INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version, machine_fingerprint)
VALUES ($1, $2, $3, $4, '', $5, $6, $7)
ON CONFLICT (id) DO NOTHING
RETURNING id AS "id!: BoxId"
```

```sql
-- (lock) reached only when (a) returned nothing: the row exists and is now lockable
SELECT user_id AS "user_id: UserId", hostname, machine_fingerprint
  FROM box WHERE id = $1 FOR UPDATE
```

```sql
-- (c) the same machine: display fields and last_seen_at only; a stored fingerprint is never cleared
UPDATE box
   SET hostname = $2, os_family = $3, arch = $4, last_seen_at = clock_timestamp(),
       machine_fingerprint = COALESCE(machine_fingerprint, $5)
 WHERE id = $1
```

Decision after the lock:
- `copied = row.user_id != self.this_user || matches!((row.machine_fingerprint.as_deref(), presented.as_deref()), (Some(s), Some(p)) if s != p)`, where `presented = fingerprint.map(Fingerprint::as_hex)`.
- **Copied**: `let minted = BoxId::new();` then (a) with `minted`. It must return a row; otherwise it is `StoreError::Backend("a fresh box id already exists")`. The answer is `Copied { previous: identity.box_id, minted }`.
- **Otherwise**: run (c) and answer `Known { renamed_from: (row.hostname != identity.hostname).then_some(row.hostname) }`.

`tx.commit()`. `htui_version`, `probed_tags`, `declared_tags`, `quirks`, `settings`, `edit_version` and every probe column are written by no statement here (D5).

Docs rewritten: `connect_with` `:104-108`, `register_box` `:337-351`, `this_box` "after the adopt-DB-id rule", and `identity()` "adopted". The replacement wording is "the id registration answered, which differs from `box.toml`'s only for a copied file".

### 3.4 `connect.rs`

`try_connect` (`:422-449`), after `connect_with`:

```rust
match connected.store.registration() {
    Some(Registration::Copied { previous, minted }) => tracing::warn!(
        previous = %previous, minted = %minted,
        "box.toml was carried from another machine; this machine registered as a new box"),
    Some(Registration::Known { renamed_from: Some(old) }) => tracing::info!(
        box_id = %identity.box_id, from = %old, to = %identity.hostname, "this box was renamed"),
    _ => {}
}
let registered = connected.store.identity();
if registered.box_id != identity.box_id {
    identity::store(root, registered)?;
}
```

Doc and comment fixes: the "adopted id" at `:251` and `:331`, and the `try_connect` doc at `:422-431`. No field logged here is a fingerprint.

### 3.5 `lib.rs`, `testkit.rs`, `cache/*`

- `lib.rs:35-36`: `pub use identity::{Fingerprint, Identity};` and `pub use pg::{Connected, HTUI_VERSION, MigrationState, PgStore, Registration};`.
- `testkit.rs:163-167`: the `demo_db` doc sentence about "collide on `box_pkey`" becomes: "a second connect would find its `box.toml` id under a user that is no longer the oldest and mint a new box (`Registration::Copied`)".
- `cache/refresh.rs::refresh_box` (`:605-652`): after the upsert loop, inside the same SQLite `tx`:
  ```rust
  sqlx::query("DELETE FROM box WHERE id <> ?").bind(this_box.to_string()).execute(&mut *tx).await.map_err(map_sqlx)?;
  ```
  The comment says a copied config directory carries a mirror holding the old box, and `box_info` reads `ORDER BY id LIMIT 1`.
- `cache/read.rs:1094-1101`: the doc stays true and says why (the prune).

### 3.6 Tests

`tests/box_identity.rs` (new; `use htui_store::testkit as common;`, every case skips without `HTUI_TEST_DATABASE_URL`):

| Test | Asserts |
|---|---|
| `a_hostname_change_keeps_the_box_id_and_registers` | Register `A` then the same id as `B`: both `Ok`, the same id, one row, `hostname = B`. After commit (b), the second answer is `Known { renamed_from: Some(A) }`. |
| `a_copied_box_toml_mints_a_new_box_and_leaves_the_old_row` | Same id under fingerprints `fp1` then `fp2` (built by `from_machine_identity` over two synthetic values): `Copied { previous, minted }` with `minted != previous`, two rows, and the old row's `hostname`, `machine_fingerprint` and `last_seen_at` byte-equal before and after. |
| `a_matching_fingerprint_is_the_same_box` | `fp1` twice gives `Known`, one row. |
| `no_fingerprint_registers_by_id_alone` | Stored `Some(fp1)` + presented `None` gives `Known` and keeps `fp1`. `None` + `None` gives `Known`. |
| `a_first_fingerprint_is_recorded_on_a_row_that_had_none` | Row planted with a NULL fingerprint, presented `fp1`: the column becomes `fp1.as_hex()`. |
| `another_id_on_the_same_hostname_is_another_box` | Two ids, one hostname: two rows, both `New`. |
| `two_first_registrations_of_one_id_both_succeed` | `tokio::join!` of two `register_box` calls on a fresh id: both `Ok`, answers `{New, Known{None}}` in either order, one row. Ten rounds. |
| `an_id_under_another_user_is_not_adopted` | A second `app_user` and a box under it planted by SQL; presenting that id gives `Copied`, and the planted row is untouched. |
| `the_stored_fingerprint_is_the_keyed_hash_never_the_raw_identity` | The column equals the known answer, and `SELECT count(*) FROM box WHERE row_to_json(box)::text LIKE '%' \|\| $raw \|\| '%'` is 0. |
| `a_reconnect_leaves_htui_version_and_the_probe_columns_alone` | `htui_version='0.0.0'`, `last_probed_at=NULL` and `probe_spec_digest=repeat('a',64)` set by SQL; registering again leaves all three unchanged. |

`tests/migrations.rs`:
- `:74` and `:591` become `vec![1, 2, 3, 4, 5]` (messages name `0005_box_identity.sql`). `:586` becomes `expect("apply 0004 and 0005")`. `:623-624` becomes `Pending(5)` / "five embedded migrations".
- `register_box_upserts_and_adopts` (`:1016-1091`) becomes **`register_box_keys_on_the_id`**:
  - first registration answers `New`;
  - the same hostname under another id answers `New` with two rows;
  - `db.store.identity().box_id == db.store.this_box()`;
  - the `box.toml` write-back half is kept, using the second id.
- New **`the_0005_migration_drops_the_hostname_key_and_adds_three_columns`**: `box_user_id_hostname_key` is absent from `pg_constraint`; the three columns are present; `UPDATE box SET machine_fingerprint = 'RAW'` and `… probe_spec_digest = 'RAW'` each fail with SQLSTATE `23514`; `edit_version` defaults to 0.
- D30: a new `const MOD7_COLUMN_COMMENTS: &[(&str, &str, &str)]` holds the four `box` rows verbatim from §3.1. `the_ana_column_comments_are_present_and_verbatim` (`:338`) iterates `ANA_COLUMN_COMMENTS.iter().chain(MOD7_COLUMN_COMMENTS)` for both the verbatim loop and the exclusivity list. The `:380-381` comment and the `:409` message become "twenty-nine".

`tests/connect.rs`:
- `:95` becomes `Pending(5)`; `:110-111` become `pending, 5` / "all five".
- New **`a_copied_box_toml_is_rewritten_to_the_minted_id`**:
  - It returns early when `identity::machine_fingerprint().await.is_none()`.
  - Setup: `fresh_db`, a new temp root, `load_or_mint`, then plant `INSERT INTO box (…, machine_fingerprint) VALUES ($id, (SELECT id FROM app_user ORDER BY created_at, id LIMIT 1), 'elsewhere', 'linux', '', 'x86_64', '0.0.0', repeat('a', 64))`.
  - Then `try_connect(&db.url, &identity, root, CONNECT_TIMEOUT)`.
  - Asserts: `box.toml` now holds another id, equal to `store.this_box()`, and the planted row is untouched.

`tests/cache.rs`: new **`a_pass_keeps_only_this_box_in_the_mirror`**. On `demo_db`, insert a stray `box` row (random id) into the mirror by SQLite `INSERT`, then run `run_pass`. `mirror_count("box") == 1`, and its id is `ids::BOX`.

### 3.7 Build coupling

- `register_box`'s callers are `bootstrap` and `tests/migrations.rs:1029`/`:1040` only (Gortex `usages`: 3 edges), all in T1.
- `PgStore` literals exist only in `pg/mod.rs`.
- `Identity` is unchanged (13 literals untouched).
- Nothing outside `htui-store` matches on the return type.

### 3.8 Gate

```bash
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features -- --test-threads=1
docker exec htui-postgres psql -U postgres -c 'DROP DATABASE IF EXISTS htui_prepare_mod7' \
  -c 'CREATE DATABASE htui_prepare_mod7'
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7 \
  sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7 \
  cargo sqlx prepare -- --all-targets --all-features \
  && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7 \
  cargo sqlx prepare --check -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l          # 229
grep -rn 'machine-id\|MachineGuid\|IOPlatformUUID' crates/htui-store/src   # readers + docs only
cargo clippy -p htui-store --all-features --all-targets -- -D warnings
```

At T1 close, migrate `htui_prepare_check` once (`sqlx migrate run`) so the plan's own `--check` line holds.

### 3.9 Commit boundaries (T1)

1. (a) Red: the hazard test on the old signature.
2. (b) Red: migration `0005`, `Fingerprint` with `todo!()` bodies, `Registration`, `HTUI_VERSION`, the new `register_box` signature with a `todo!()` body, `bootstrap`, every test above, and the pin moves.
3. (c) Green: bodies, docs, `try_connect`, `refresh_box`, `.sqlx`.
4. (d) Doc-only rewording, if not already done in (c).

---

## 4. T2: the probe write surface on every store (D10, D32, D33)

**First failing test**: `store::conformance::record_box_probe_replaces_profile_and_tools` over `MemStore`. T2 starts from T1 merged.

### 4.1 `traits.rs` (after `set_agent_box_quota`, `:300-306`)

```rust
/// Writes one box probe (MOD-7 D10): the hardware columns, `probed_tags`, `htui_version`,
/// `last_probed_at` and `probe_spec_digest`, and replaces this box's `box_tool` set, in one
/// transaction. A narrow machine writer (MOD-2 D74): never `hostname`, `declared_tags`, `quirks`,
/// `settings`, `machine_fingerprint`, `edit_version` or `last_seen_at`.
///
/// # Errors
/// `NotFound { entity: "box", id }` when no row has `probe.box_id`; `Constraint` when a tool name
/// repeats or `spec_digest` is not 64 lowercase hex. Either way nothing is written.
async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()>;

/// Every box of this user with its tools and recorded spec digest (MOD-7 D10, D18): boxes by id,
/// tools by name byte order (`COLLATE "C"`), so every store answers byte for byte.
async fn boxes(&self) -> Result<Vec<BoxRecord>>;
```

The import list gains `BoxProbe` and `BoxRecord`. The doc text must not put a non-existent test name in backticks: `conformance.rs`'s `every_cross_referenced_test_name_exists` (`:7481-7560`) scans module doc comments.

### 4.2 `MemStore` (`mem.rs`)

- `State` (`:81`) gains `box_probe_digests: HashMap<BoxId, String>`, with the doc "`box.probe_spec_digest`, which `BoxRow` does not carry (D14)". `from_demo`'s literal (`:208-247`) gains `box_probe_digests: HashMap::new()`.
- `State::record_box_probe(&mut self, probe: &BoxProbe, now: DateTime<Utc>) -> Result<()>` checks in this order:
  1. The digest must be 64 lowercase hex, else `Constraint(format!("box.probe_spec_digest `{}` is not a lowercase sha256 hex digest", probe.spec_digest))` (D32).
  2. Tool names must be unique, else `Constraint(format!("box_tool `{name}` is listed twice for box `{}`", probe.box_id))`.
  3. The row must exist, else `NotFound { entity: "box", id: probe.box_id.to_string() }`.
  
  Then it sets the nine columns, sets `row.updated_at = now` (the trigger's job), runs `box_tools.retain(|t| t.box_id != id)`, pushes the new rows with `probed_at = probe.probed_at`, and inserts the digest.
- `State::box_records(&self, user: Option<UserId>) -> Vec<BoxRecord>`: `None` gives an empty list. Boxes with `user_id == user` are sorted by `id`; tools are filtered by box and sorted by `name.as_bytes()`; the digest comes from the side map.
- `WriteStore for MemStore`:
  - `record_box_probe` is `let now = Utc::now(); self.write(|s| s.record_box_probe(probe, now))`.
  - `boxes` is `let user = self.this_user(); Ok(self.read(|s| s.box_records(user)))`.

### 4.3 `PgStore` (`pg/write.rs`, inside `impl WriteStore for PgStore` at `:381`)

`record_box_probe` runs in one `pool.begin()`:

```sql
UPDATE box
   SET os_version = $2, cpu = $3, ram_mb = $4, gpu_present = $5, gpu_vendor = $6,
       probed_tags = $7, htui_version = $8, last_probed_at = $9, probe_spec_digest = $10
 WHERE id = $1
RETURNING id AS "id!: BoxId"
```

`fetch_optional`; `None` gives `NotFound { entity: "box", id: probe.box_id.to_string() }`.

```sql
DELETE FROM box_tool WHERE box_id = $1
```

```sql
INSERT INTO box_tool (box_id, name, version, path, probed_at)
SELECT $1, t.name, t.version, t.path, $5
  FROM UNNEST($2::text[], $3::text[], $4::text[]) AS t(name, version, path)
```

A `23505` becomes `Constraint` through `map_sqlx`, and a `23514` (digest) likewise. The transaction then drops, which rolls back.

`boxes` (two reads):

```sql
SELECT id AS "id: BoxId", user_id AS "user_id: htui_core::model::UserId", hostname,
       os_family AS "os_family: htui_core::model::OsFamily", os_version, arch, cpu, ram_mb,
       gpu_present, gpu_vendor, htui_version, probed_tags, declared_tags, quirks, settings,
       registered_at, last_seen_at, last_probed_at, updated_at, probe_spec_digest
  FROM box WHERE user_id = $1 ORDER BY id
```

```sql
SELECT box_id AS "box_id: BoxId", name, version, path, probed_at
  FROM box_tool WHERE box_id = ANY($1) ORDER BY box_id, name COLLATE "C"
```

Both are `query!` (not `query_as!(BoxRow, ..)`, because of the extra column). Rows are mapped by hand into `BoxRecord`. `$1` of the first is `self.this_user().as_uuid()`. `PgStore::this_user` is `pub const fn` (`pg/mod.rs:457-461`). T2 edits no `pg/mod.rs` and no `rows.rs`.

### 4.4 `Writer` and the two spies

- `writer.rs` (after `set_agent_box_quota`'s arms): two methods, each `match self { Memory(s) => s.X(..).await, Online(pg) => pg.X(..).await }`.
- `htui-agent/src/conformance.rs` (`UsageSpy`, after `:715-716`) and `htui-agent/tests/recorder.rs` (`SpyStore`, after `:399-400`): each forwards both methods through `self.inner`.

### 4.5 Conformance cases (`CASES` 53 → 56, appended after `"release_lease_frees_the_run_for_its_own_sweep"`, plus `run_case` arms)

| Case | Asserts |
|---|---|
| `record_box_probe_replaces_profile_and_tools` | `at1`/`at2` are fixed instants truncated to micros. The probe of `ids::BOX` writes tools `{a 1.0, b 2.0}` at `at1` with digest `sha256_hex("one")`, then `{b 2.1, c 3.0}` at `at2` with `sha256_hex("two")`. `boxes()` gives one record for `ids::BOX` with: tools exactly `[b 2.1, c 3.0]`, `probed_at == at2`; every hardware column, `probed_tags`, `htui_version` and `last_probed_at == Some(at2)` as written; `probe_spec_digest == Some(sha256_hex("two"))`; `hostname`, `declared_tags`, `quirks`, `settings`, `registered_at` and `last_seen_at` equal to the fixture's; `updated_at` later than the fixture's. Then a probe listing `b` twice is a `Constraint`, and `boxes()` is unchanged. |
| `record_box_probe_refuses_an_unknown_box` | `BoxId::new()` gives `NotFound { entity: "box", .. }`; `boxes()` is unchanged. |
| `boxes_lists_every_box_with_its_tools` | Exactly one record. `row.id == ids::BOX`, `hostname "DESKTOP-HTUI"`, `probed_tags ["rust","msvc","cmake"]`, `declared_tags ["gpu"]`. Tools are `cargo, cmake, git, rustc` in that order with the fixture's versions and paths. `probe_spec_digest == None`. |

Pins:
- `crates/htui-core/tests/mem_store.rs:37` becomes `56`, and the message gains "MOD-7 milestone 1's three for the box probe writer (plan D10)".
- `crates/htui-store/tests/pg_conformance.rs:19` becomes `const EXPECTED_CASES: usize = 56;`.
- `READ_CASES` stays at 9.

A `mem.rs` unit test, **`a_probe_digest_that_is_not_hex_is_a_constraint`**, pins D32 on `MemStore` (Postgres's half is T1's `CHECK` test).

### 4.6 Gate and commits

```bash
cargo test -p htui-core --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1
# prepare against htui_prepare_mod7 as in §3.8 (recreate + migrate first), then --check
ls crates/htui-store/.sqlx | wc -l          # 234
```

Commits: (a) red = trait methods + `todo!()` in all five impls + three cases + pins; (b) green = bodies + `.sqlx`.

---

## 5. T3: `htui-agent`, the box probe (D6–D9, D17, D18 digest, D21–D24, D29, D36)

**First failing test**: `the_spec_parses_and_every_tag_rule_names_a_listed_tool`. T3 depends on T0 only.

### 5.1 Module layout

```
crates/htui-agent/src/box_probe/mod.rs       probe_box, pick_gpu_vendor, tags_from_presence, MAX_CONCURRENT_TOOLS
crates/htui-agent/src/box_probe/hardware.rs  Hardware, HardwareFuture, HardwareSource, SystemHardware, FixedHardware,
                                             ram_mb, pci_display_vendors, system_profiler_vendors, pnp_device_vendors
crates/htui-agent/src/box_probe/spec.rs      Spec, TagRule, Fact, Ask, GpuVendor, EffectiveSpec, SETTING_KEY,
                                             SPEC_IGNORED, seed, effective, digest, validate
crates/htui-agent/src/box_probe/spec.json    the seed (§5.5)
```

- `lib.rs` (`:92-110`) gains `pub mod box_probe;` in alphabetical position.
- No crate-root re-export: callers write `htui_agent::box_probe::…`. This matches `probe` and `launch`, which are also used by path.
- **No `#[cfg(test)]` module in any `box_probe/*.rs`** (D29).

### 5.2 `probe.rs` and `launch.rs` edits

- `probe.rs:960-967`: `pub(crate) struct OneShot { pub(crate) stdout: String, pub(crate) status: ExitStatus, pub(crate) stderr: Vec<String> }`. `:982`: `pub(crate) async fn run_bounded(..)`. No other change.
- `launch.rs`, after `Install` (`:136-150`):

```rust
/// Whether an `agent.launch` document declares `discovery.install` (MOD-20 D12). One rule for the
/// Settings section, the install pre-flight and the box probe's report (MOD-7 D13). A document
/// that does not parse declares nothing.
#[must_use]
pub fn declares_install(launch: &serde_json::Value) -> bool {
    serde_json::from_value::<AgentLaunch>(launch.clone())
        .ok()
        .and_then(|launch| launch.discovery)
        .is_some_and(|discovery| discovery.install.is_some())
}
```

### 5.3 `hardware.rs` (D6, D7, D21, D22)

```rust
/// What the hardware seam reports: facts only, never an error (plan D6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hardware {
    /// `box.os_version`; empty when unreadable.
    pub os_version: String,
    /// `box.cpu`; empty when unreadable.
    pub cpu: String,
    /// `box.ram_mb`.
    pub ram_mb: Option<i32>,
    /// Lowercase `0x`-prefixed PCI vendor ids of every display-class device (blueprint D21),
    /// deduplicated; the probe spec's `gpu_vendors` names them.
    pub display_vendors: Vec<String>,
}

/// The boxed future a [`HardwareSource`] answers with.
pub type HardwareFuture<'a> = Pin<Box<dyn Future<Output = Hardware> + Send + 'a>>;

/// The seam between the box probe and the host (plan D6, blueprint D22).
pub trait HardwareSource: Send + Sync + core::fmt::Debug {
    /// Reads the facts; `env` bounds any child (`cwd`, `version_timeout`).
    fn read<'a>(&'a self, env: &'a ProbeEnv) -> HardwareFuture<'a>;
}

/// Production: `sysinfo` for OS, CPU and RAM; GPU per OS (D7).
#[derive(Debug, Clone)]
pub struct SystemHardware { pci_root: PathBuf }
impl SystemHardware {
    /// Scans `<pci_root>/sys/bus/pci/devices` on Linux.
    #[must_use] pub fn new(pci_root: PathBuf) -> Self;
    /// `new("/")`.
    #[must_use] pub fn host() -> Self;
}

/// Tests: hands back exactly these facts.
#[derive(Debug, Clone)]
pub struct FixedHardware(pub Hardware);

/// `total_memory()` bytes → MiB, floored; `None` at 0 or above `i32::MAX`.
#[must_use] pub fn ram_mb(total_bytes: u64) -> Option<i32>;
/// Linux: `<root>/sys/bus/pci/devices/*/{class,vendor}`, `class` starting `0x03`; a missing tree is empty.
#[must_use] pub fn pci_display_vendors(root: &Path) -> Vec<String>;
/// macOS `system_profiler SPDisplaysDataType`: every `(0x....)` on a line containing `Vendor`.
#[must_use] pub fn system_profiler_vendors(text: &str) -> Vec<String>;
/// Windows CIM `PNPDeviceID` lines: `VEN_10DE` → `0x10de`.
#[must_use] pub fn pnp_device_vendors(text: &str) -> Vec<String>;
```

`SystemHardware::read`:
1. `let facts = tokio::task::spawn_blocking(system_facts).await.unwrap_or_default();`
2. `display_vendors = gpu_vendor_ids(&self.pci_root, env).await`.

`system_facts()` builds one `System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram()))` **and** calls the OS-name associated functions inside the same closure (all four read the OS per call).

`os_version`:
- macOS: `System::long_os_version()`.
- Elsewhere: `System::name()` and `System::os_version()` joined by a space.
- If empty, fall back to `long_os_version()`, then `kernel_version()`, else `""`.

`cpu` is the first `Cpu::brand()`, trimmed. `ram_mb` is `ram_mb(sys.total_memory())`.

`gpu_vendor_ids`, one cfg arm per OS:

| cfg | Body |
|---|---|
| `target_os = "linux"` | `spawn_blocking(move \|\| pci_display_vendors(&root))` |
| `target_os = "macos"` | On `aarch64`, `vec!["0x106b".to_owned()]` (Apple's PCI vendor id). Otherwise `run_bounded("system_profiler", &ResolvedLaunch { command: "/usr/sbin/system_profiler".into(), args: vec!["SPDisplaysDataType".into()], env: BTreeMap::new() }, env)` → `system_profiler_vendors(&out.stdout)`. |
| `windows` | `command = format!(r"{}\System32\WindowsPowerShell\v1.0\powershell.exe", env.var("SystemRoot").unwrap_or(r"C:\Windows"))`, `args ["-NoProfile","-NonInteractive","-Command","Get-CimInstance -ClassName Win32_VideoController \| ForEach-Object { $_.PNPDeviceID }"]` → `pnp_device_vendors`. |
| other | `Vec::new()` |

Every child command is an absolute path: `run_bounded` resolves on the **process** `PATH`.

### 5.4 `spec.rs` (D9, D17, D23, D24)

```rust
/// The `app_setting` key of the stored overlay (plan D17).
pub const SETTING_KEY: &str = "box_probe_spec";
/// The first words of every overlay refusal (blueprint D23).
pub const SPEC_IGNORED: &str = "box_probe_spec ignored";

/// The probe spec: tools, tag rules and the GPU vendor map (plan D9). The compiled `spec.json` is
/// the seed; a stored overlay is merged into it by name ([`effective`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Tool name → probe; `launch::ToolProbe` verbatim, always `kind: path`.
    pub tools: BTreeMap<String, ToolProbe>,
    /// In order; the first rule per tag wins nothing: each fires on its own.
    pub tags: Vec<TagRule>,
    /// Priority order: the first present vendor names the GPU.
    pub gpu_vendors: Vec<GpuVendor>,
}

/// One derived tag (PRD D2): exactly one of `any_tool` (non-empty) or `fact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagRule {
    /// The tag.
    pub tag: String,
    /// Fires when any of these tools is present (and `ask`, if set, matches).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_tool: Vec<String>,
    /// Fires on a box fact instead of a tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact: Option<Fact>,
    /// Runs the first present tool of `any_tool` with `args`; fires when a line matches `matches`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
}

/// A box fact a rule can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fact {
    /// A display device from a mapped vendor.
    Gpu,
}

/// A question asked of a present tool (the MinGW `gcc` case).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    /// Arguments.
    pub args: Vec<String>,
    /// A regex, matched against each trimmed line of stdout then the stderr tail.
    pub matches: String,
}

/// One PCI vendor id and the name `box.gpu_vendor` records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuVendor {
    /// `0x` + four lowercase hex digits.
    pub pci: String,
    /// E.g. `nvidia`.
    pub name: String,
}

/// The spec a probe runs under, its digest, and why a stored overlay was ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSpec {
    /// The merged spec (or the seed).
    pub spec: Spec,
    /// [`digest`] of `spec`.
    pub digest: String,
    /// `Some(sentence)` when a stored overlay was rejected and the seed used.
    pub error: Option<String>,
}

/// The compiled seed, parsed once (`LazyLock`; `the_spec_parses…` pins that it parses and validates).
#[must_use] pub fn seed() -> &'static Spec;
/// Merges `stored` into `seed`, all or nothing (D17, D23). Never fails: a rejected overlay is the
/// seed plus `error`.
#[must_use] pub fn effective(seed: &Spec, stored: Option<&serde_json::Value>) -> EffectiveSpec;
/// `htui_core::prompt::digest::sha256_hex(&serde_json::to_string(spec))` (D24).
#[must_use] pub fn digest(spec: &Spec) -> String;
/// The checks the seed and every merged result must pass; `Err` is the fault without the prefix.
///
/// # Errors
/// The first fault, as one sentence naming its key.
pub fn validate(spec: &Spec) -> Result<(), String>;
```

`spec.rs`'s module doc carries D17's SQL edit path verbatim, with `terraform` (not `zig`), plus the `DELETE` that returns to the seed.

**`effective` algorithm (D23)**. Every fault string is the message after `"{SPEC_IGNORED}: "`.

1. `stored == None` gives `EffectiveSpec { spec: seed.clone(), digest: digest(seed), error: None }`.
2. The value is not an object: `"the value is not a JSON object"`. An unknown top-level key: `` "unknown key `k`" ``.
3. **`tools`** is an object. For each `(name, entry)`:
   - `name` must be non-empty and not `.` or `..`, with no `/` or `\`, else `` "tools: `name` is not a bare tool name" ``.
   - An entry that is exactly `{"disabled": true}` removes `name` and records it in `disabled`.
   - Otherwise it deserialises into `ToolProbe` (`"tools.name: <serde error>"`).
   - The entry must be `ToolProbe::Path`, else `` "tools.name: only `kind: path` tools may be added" ``.
   - `names` must be non-empty, and each must be bare as above (`` "tools.name: `x` is not a bare file name" ``).
   - `version.pattern` must compile, else `"tools.name.version.pattern: <first line of regex error>"`.
   - Insert or replace.
4. **Prune seeded rules**: for every seed rule, drop from `any_tool` each name absent from the merged tools. Drop a rule whose `any_tool` became empty and that has no `fact` (F-C).
5. **`tags`** is an array. An element exactly `{"tag": t, "disabled": true}` removes rule `t`. Otherwise it deserialises into `TagRule` (`"tags[i]: <serde error>"`) and is checked:
   - `tag` is non-empty;
   - exactly one of non-empty `any_tool` / `fact` (`"tags.t: a rule names tools or a fact, not both or neither"`);
   - every `any_tool` entry is in the merged tools (`` "tags.t: names `x`, which no tool provides" ``);
   - `ask` requires `any_tool`;
   - `ask.matches` compiles.
   
   The rule then replaces the rule with the same `tag` in place, or is appended.
6. **`gpu_vendors`** is an array. `{"pci": p, "disabled": true}` removes `p`. Otherwise it is a `GpuVendor` with `pci` matching `^0x[0-9a-f]{4}$` (`"gpu_vendors.p: … "`) and a non-empty `name`. It replaces the vendor with that `pci` in place (keeping priority), or is appended last.
7. The merged `Spec` must pass `validate`.

Any fault means the seed plus `error = Some(format!("{SPEC_IGNORED}: {fault}"))`, and `tracing::warn!(%error, …)`. Disabling a name that does not exist is a no-op, not a fault.

### 5.5 `spec.json` (the seed: 39 tools, 14 rules, 5 vendors)

```json
{
  "tools": {
    "bash":       {"kind": "path", "names": ["bash"],       "version": {"args": ["--version"], "pattern": "^GNU bash, version (\\d+(?:\\.\\d+)+)"}},
    "bun":        {"kind": "path", "names": ["bun"],        "version": {"args": ["--version"], "pattern": "^(\\d+(?:\\.\\d+)+)$"}},
    "cargo":      {"kind": "path", "names": ["cargo"],      "version": {"args": ["--version"], "pattern": "^cargo (\\S+)"}},
    "cl":         {"kind": "path", "names": ["cl"],         "version": {"args": [], "pattern": "Compiler Version (\\d+(?:\\.\\d+)+)"}},
    "clang":      {"kind": "path", "names": ["clang"],      "version": {"args": ["--version"], "pattern": "clang version (\\d+(?:\\.\\d+)+)"}},
    "cmake":      {"kind": "path", "names": ["cmake"],      "version": {"args": ["--version"], "pattern": "^cmake version (\\S+)"}},
    "deno":       {"kind": "path", "names": ["deno"],       "version": {"args": ["--version"], "pattern": "^deno (\\S+)"}},
    "docker":     {"kind": "path", "names": ["docker"],     "version": {"args": ["--version"], "pattern": "^Docker version ([^,\\s]+)"}},
    "dotnet":     {"kind": "path", "names": ["dotnet"],     "version": {"args": ["--version"], "pattern": "^(\\d+(?:\\.\\d+)+\\S*)$"}},
    "fish":       {"kind": "path", "names": ["fish"],       "version": {"args": ["--version"], "pattern": "^fish, version (\\S+)"}},
    "gcc":        {"kind": "path", "names": ["gcc"],        "version": {"args": ["--version"], "pattern": "^gcc\\S*\\s.*\\s(\\d+(?:\\.\\d+)+)$"}},
    "git":        {"kind": "path", "names": ["git"],        "version": {"args": ["--version"], "pattern": "^git version (\\S+)"}},
    "git-lfs":    {"kind": "path", "names": ["git-lfs"],    "version": {"args": ["version"], "pattern": "^git-lfs/(\\S+)"}},
    "go":         {"kind": "path", "names": ["go"],         "version": {"args": ["version"], "pattern": "^go version go(\\S+)"}},
    "helm":       {"kind": "path", "names": ["helm"],       "version": {"args": ["version", "--short"], "pattern": "^v([^+\\s]+)"}},
    "java":       {"kind": "path", "names": ["java"],       "version": {"args": ["-version"], "pattern": "version \"([^\"]+)\""}},
    "javac":      {"kind": "path", "names": ["javac"],      "version": {"args": ["-version"], "pattern": "^javac (\\S+)"}},
    "kubectl":    {"kind": "path", "names": ["kubectl"],    "version": {"args": ["version", "--client"], "pattern": "^Client Version: v(\\S+)"}},
    "make":       {"kind": "path", "names": ["make"],       "version": {"args": ["--version"], "pattern": "^GNU Make (\\S+)"}},
    "meson":      {"kind": "path", "names": ["meson"],      "version": {"args": ["--version"], "pattern": "^(\\d+(?:\\.\\d+)+)$"}},
    "msbuild":    {"kind": "path", "names": ["msbuild"],    "version": {"args": ["-version"], "pattern": "^MSBuild version (\\d+(?:\\.\\d+)+)"}},
    "mvn":        {"kind": "path", "names": ["mvn"],        "version": {"args": ["--version"], "pattern": "^Apache Maven (\\S+)"}},
    "nerdctl":    {"kind": "path", "names": ["nerdctl"],    "version": {"args": ["--version"], "pattern": "^nerdctl version (\\S+)"}},
    "ninja":      {"kind": "path", "names": ["ninja"],      "version": {"args": ["--version"], "pattern": "^(\\d+(?:\\.\\d+)+)$"}},
    "node":       {"kind": "path", "names": ["node"],       "version": {"args": ["--version"], "pattern": "^v(\\S+)"}},
    "npm":        {"kind": "path", "names": ["npm"],        "version": {"args": ["--version"], "pattern": "^(\\d+(?:\\.\\d+)+)$"}},
    "pip":        {"kind": "path", "names": ["pip3", "pip"], "version": {"args": ["--version"], "pattern": "^pip (\\S+)"}},
    "pnpm":       {"kind": "path", "names": ["pnpm"],       "version": {"args": ["--version"], "pattern": "^(\\d+(?:\\.\\d+)+)$"}},
    "podman":     {"kind": "path", "names": ["podman"],     "version": {"args": ["--version"], "pattern": "^podman version (\\S+)"}},
    "powershell": {"kind": "path", "names": ["powershell"]},
    "pwsh":       {"kind": "path", "names": ["pwsh"],       "version": {"args": ["--version"], "pattern": "^PowerShell (\\S+)"}},
    "python3":    {"kind": "path", "names": ["python3", "python"], "version": {"args": ["--version"], "pattern": "^Python (\\S+)"}},
    "qemu-img":   {"kind": "path", "names": ["qemu-img"],   "version": {"args": ["--version"], "pattern": "^qemu-img version (\\S+)"}},
    "rustc":      {"kind": "path", "names": ["rustc"],      "version": {"args": ["--version"], "pattern": "^rustc (\\S+)"}},
    "uv":         {"kind": "path", "names": ["uv"],         "version": {"args": ["--version"], "pattern": "^uv (\\S+)"}},
    "vcpkg":      {"kind": "path", "names": ["vcpkg"],      "version": {"args": ["--version"], "pattern": "version (\\S+)"}},
    "vulkaninfo": {"kind": "path", "names": ["vulkaninfo"]},
    "zig":        {"kind": "path", "names": ["zig"],        "version": {"args": ["version"], "pattern": "^(\\d+\\.\\d+\\.\\d+\\S*)$"}},
    "zsh":        {"kind": "path", "names": ["zsh"],        "version": {"args": ["--version"], "pattern": "^zsh (\\S+)"}}
  },
  "tags": [
    {"tag": "rust",   "any_tool": ["rustc", "cargo"]},
    {"tag": "cmake",  "any_tool": ["cmake"]},
    {"tag": "clang",  "any_tool": ["clang"]},
    {"tag": "msvc",   "any_tool": ["cl"]},
    {"tag": "mingw",  "any_tool": ["gcc"], "ask": {"args": ["-dumpmachine"], "matches": "mingw"}},
    {"tag": "vcpkg",  "any_tool": ["vcpkg"]},
    {"tag": "docker", "any_tool": ["docker", "podman"]},
    {"tag": "vulkan", "any_tool": ["vulkaninfo"]},
    {"tag": "gpu",    "fact": "gpu"},
    {"tag": "go",     "any_tool": ["go"]},
    {"tag": "node",   "any_tool": ["node"]},
    {"tag": "python", "any_tool": ["python3"]},
    {"tag": "java",   "any_tool": ["javac"]},
    {"tag": "dotnet", "any_tool": ["dotnet"]}
  ],
  "gpu_vendors": [
    {"pci": "0x10de", "name": "nvidia"},
    {"pci": "0x1002", "name": "amd"},
    {"pci": "0x8086", "name": "intel"},
    {"pci": "0x106b", "name": "apple"},
    {"pci": "0x5143", "name": "qualcomm"}
  ]
}
```

`gradle` and `bazel` are absent on purpose (D9).

### 5.6 `mod.rs`: `probe_box` (D8, D9, D21, D36)

```rust
/// At most this many tool lookups and version children at once (plan D9).
pub const MAX_CONCURRENT_TOOLS: usize = 8;

/// One box probe (plan D8): hardware through `hardware`, tools and tags through `spec`, on this
/// copy of `env` with `versions` forced on (OQ-11). Never fails; an unreadable fact is empty.
pub async fn probe_box(
    box_id: BoxId,
    env: &ProbeEnv,
    hardware: &dyn HardwareSource,
    spec: &EffectiveSpec,
    htui_version: &str,
    now: DateTime<Utc>,
) -> BoxProbe;

/// The first vendor of `map`, in its order, whose `pci` is among `found` (plan D7).
#[must_use]
pub fn pick_gpu_vendor<'a>(found: &[String], map: &'a [GpuVendor]) -> Option<&'a str>;

/// The tags `rules` derive (PRD D2), sorted and deduplicated. `asked` holds each `ask` rule's answer
/// by tag; a rule with an `ask` and no answer does not fire.
#[must_use]
pub fn tags_from_presence(
    rules: &[TagRule],
    present: &BTreeSet<String>,
    gpu_present: bool,
    asked: &BTreeMap<String, bool>,
) -> Vec<String>;
```

Body of `probe_box`:

1. `let env = Arc::new(ProbeEnv { versions: true, ..env.clone() });`
2. `let hw = hardware.read(&env).await;`
3. `let gpu = pick_gpu_vendor(&hw.display_vendors, &spec.spec.gpu_vendors);`
4. Tools, on one `JoinSet` behind `Arc<Semaphore::new(MAX_CONCURRENT_TOOLS)>`. Per `(name, probe)`, holding a permit, run `resolve_tool(&probe, &env)`:
   - `Ok(Some(r))` where the probe has no `version`, **or** `r.version.is_some()`, gives `ProbedTool { name, version: r.version.unwrap_or_default(), path: r.path.to_string_lossy().into_owned() }`;
   - everything else is absent (OQ-11), logged at `debug!`;
   - `Err` is a `warn!` and absent.
   
   Results are collected into a `BTreeMap<String, ProbedTool>`.
5. Asks: for each rule with an `ask` whose first present `any_tool` entry exists, take a permit and run `run_bounded("ask", &ResolvedLaunch { command: tool.path.clone(), args: ask.args.clone(), env: BTreeMap::new() }, &env)`. It answers `true` if any trimmed line of stdout, then the stderr tail, matches `Regex::new(&ask.matches)`.
6. `probed_tags = tags_from_presence(&spec.spec.tags, &present, gpu.is_some(), &asked)`.
7. The result is `BoxProbe { box_id, os_version: hw.os_version, cpu: hw.cpu, ram_mb: hw.ram_mb, gpu_present: gpu.is_some(), gpu_vendor: gpu.map(str::to_owned), tools: map.into_values().collect(), probed_tags, htui_version: htui_version.to_owned(), spec_digest: spec.digest.clone(), probed_at: now }`.

### 5.7 Tests (`crates/htui-agent/tests/box_probe.rs`; helper `env(tmp)` mirrors `tests/probe.rs:43-68` with `home: None`)

| Test | Asserts |
|---|---|
| `the_spec_parses_and_every_tag_rule_names_a_listed_tool` | `validate(seed())` is `Ok`, and every rule tool is a key of `tools`. |
| `the_seed_lists_thirty_nine_path_tools` | 39 entries, every one `ToolProbe::Path` with bare `names`; `gradle` and `bazel` are absent. |
| `the_derived_vocabulary_is_r_box_3_without_heavy_build_plus_oq12` | The set of `tag`s equals `{gpu, vulkan, msvc, mingw, clang, cmake, vcpkg, docker, rust}` ∪ `{go, node, python, java, dotnet}`, written out, with a comment naming `pg/mod.rs:27-38` and OQ-12. |
| `tags_from_presence` | Table: `cargo` → `rust`; `podman` → `docker`; `gcc` + `asked{mingw:true}` → `mingw`, `false` → none; gpu → `gpu`; `vulkaninfo` → `vulkan`; `go`, `node`, `python3` → `python`, `javac` → `java`, `java` alone → none, `dotnet`. The output is sorted and deduplicated. |
| `the_box_probe_forces_versions_on` | `#[cfg(unix)]`. Env with `versions: false`: a versioned fake tool is present with its version. |
| `tools_over_a_fake_path_report_versions_and_skip_the_absent` | `#[cfg(unix)]`. Scripts print the live first lines (`rustc 1.98.1`, `cmake version 4.2.2`, `Docker version 29.8.1, build x`, `go version go1.25.7 linux/amd64`, `v22.19.0`); versions are extracted, and the other seed tools are absent. |
| `an_ask_runs_the_resolved_tool` | `#[cfg(unix)]`. A `gcc` script answers `-dumpmachine` with `x86_64-w64-mingw32` → `mingw`; with `x86_64-linux-gnu` → no `mingw`. |
| `a_hanging_tool_is_bounded_and_absent` | `#[cfg(unix)]`. `sleep 30`, `version_timeout` 200 ms: returns in under 5 s, tool absent. |
| `a_shim_that_prints_no_version_is_absent` | `#[cfg(unix)]`. A `pnpm` script printing `Volta error: Could not find executable "pnpm"` is absent. |
| `a_presence_only_tool_counts_when_found` | `#[cfg(unix)]`. A `vulkaninfo` script that prints nothing is present with `version ""`, tag `vulkan`. |
| `at_most_eight_tools_resolve_at_once` | `#[cfg(unix)]`, multi-thread. An overlay adds 16 tools whose script appends `s` to a log, sleeps 0.3 s, then appends `e` and prints `1.0`. The maximum running count scanned over the log is ≤ 8 and ≥ 2. |
| `linux_gpu_from_a_pci_fixture_root` | A tempdir tree with `0000_0f_00.0/{class=0x030000, vendor=0x1002}` → `pci_display_vendors == ["0x1002"]`, `pick_gpu_vendor` → `amd`. |
| `a_virtual_display_adapter_is_not_a_gpu` | `0x1234` only → no vendor. |
| `the_vendor_map_prefers_a_discrete_vendor` | `0x8086` + `0x10de` → `nvidia`. |
| `a_missing_pci_tree_is_no_gpu_not_an_error` | An empty root → `[]`. |
| `macos_and_windows_gpu_parsers` | Captured `system_profiler` text → `["0x10de","0x8086"]`; `PCI\VEN_10DE&DEV_2684…` → `["0x10de"]`. |
| `probe_box_takes_its_facts_from_the_hardware_seam` | `FixedHardware` in → the same `os_version`, `cpu`, `ram_mb`; `display_vendors ["0x10de"]` → `gpu_present`, `gpu_vendor Some("nvidia")`, tag `gpu`; `spec_digest == effective.digest`. |
| `declares_install_reads_discovery_install` | `true` with `discovery.install`, `false` without, `false` for an unparseable launch. |
| `no_stored_spec_is_the_seed` | `effective(seed(), None)` gives `spec == *seed()`, no error. |
| `a_stored_tool_is_added` | `{"tools":{"terraform":{…}}}` → present, seed tools kept. |
| `a_stored_tool_replaces_the_seeded_one` | `zig` with other args → replaced. |
| `a_disabled_tool_and_its_tag_rule_drop_out` | Disabling `cmake` removes tool and rule `cmake`; disabling `cargo` keeps `rust ← rustc`. |
| `a_stored_tag_rule_replaces_in_place` | A new `docker` rule sits at the seed's index. |
| `a_stored_vendor_replaces_by_pci_id_and_keeps_priority` | `0x1002` renamed keeps index 1; a new vendor is appended last. |
| `an_unparseable_overlay_falls_back_to_the_seed_with_a_sentence` | `json!(42)` → the seed, and `error` starts with `"box_probe_spec ignored: "`. |
| `a_glob_or_node_package_tool_is_refused` | Both kinds → the seed plus an error naming the tool. |
| `a_name_with_a_path_separator_is_refused` | `names:["bin/x"]` and key `"../x"` → refused. |
| `a_bad_version_pattern_is_refused` | `"pattern":"("` → refused. |
| `a_tag_rule_naming_an_absent_tool_is_refused` | An overlay rule naming `nosuch` → refused. |
| `the_digest_is_stable_and_changes_with_the_spec` | Seed twice → equal; seed plus one tool → different; ignored overlay → `digest(seed())`; the digest is 64 lowercase hex. |
| `this_box_reports_real_hardware` | `cfg(target_os = "linux")`. `SystemHardware::host()` over an env with an empty `PATH`: non-empty `os_version` and `cpu`, `ram_mb > 0`. |

### 5.8 Gate and commits

```bash
cargo test -p htui-agent --all-features -- --test-threads=1
cargo clippy -p htui-agent --all-features --all-targets -- -D warnings
grep -rnE '"(rustc|cargo|gcc|clang|cmake|docker|podman|go|node|npm|python3|java|javac|dotnet|git|kubectl|vulkaninfo)"' \
  crates/htui-agent/src/box_probe/*.rs          # prints nothing (D29)
```

Commits:
1. (a) Red: `probe.rs`/`launch.rs` visibility and `declares_install` with a `todo!()` body, the module with `todo!()` bodies, `spec.json`, and every test.
2. (b) Green: `spec.rs` and `hardware.rs`.
3. (c) Green: `mod.rs`'s `probe_box`.

---

## 6. T4: `htui`, the trigger, the request and the report (D11–D13, D25–D28, D34, D35)

**First failing test**: `agent_worker::tests::an_online_swap_probes_a_box_that_was_never_probed`.

### 6.1 `agent_worker.rs`: surface

```rust
/// What `claim_is_free`, `probe` and `ProbeBox` refuse with while a box probe holds the slot (D27).
pub const BOX_PROBE_RUNNING: &str = "a box probe is running on this box; try again once it has finished";

/// What one box probe did (MOD-7 D13, blueprint D25): the only thing the task ever sends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoxProbeReport {
    /// `box_tool` rows written.
    pub tools: usize,
    /// `box.probed_tags` written.
    pub probed_tags: Vec<String>,
    /// Enabled agents whose fresh row is `missing` and whose launch declares `discovery.install`.
    pub installable: Vec<String>,
    /// The agent half's failure, when it had one.
    pub agents_failed: Option<String>,
    /// `EffectiveSpec::error`: why a stored `box_probe_spec` was ignored.
    pub spec_error: Option<String>,
    /// The box half's failure (no box, a read or the write); nothing was written.
    pub box_failed: Option<String>,
}

impl BoxProbeReport {
    /// The status-line sentence (D25).
    #[must_use]
    pub fn status_line(&self) -> String;
}
```

`status_line` is built from these parts, in order:
- The head: if `box_failed` is set, `"box probe failed: {m}"`. Otherwise `"box probed: {tools} tools · tags {tags joined ", "}"`, or `"… · no tags"` when there are none.
- If `installable` is non-empty: `" · {names joined ", "} {is|are} missing and can be installed: Settings > Agents, i"`.
- If `agents_failed` is set: `" · agent probe failed: {m}"`.
- If `spec_error` is set: `" · {spec_error}"`.

`AgentRuntime` fields (`:292-345`):
- `box_probe: Option<JoinHandle<()>>`
- `registration_probe: bool`
- `probe_env: Option<ProbeEnv>`
- `hardware: Option<Arc<dyn HardwareSource>>`

`new()` (`:357-371`) sets them to `None, false, None, None`. The hand-written `Debug` (`:347-354`) gains `.field("box_probe", &self.box_probe.is_some())`.

```rust
/// Opt in to the registration probe (D11): only the binary's entry point does.
#[must_use] pub fn with_registration_probe(mut self) -> Self;
/// The env and hardware every box probe and `ProbeAgents` use (D11, D35); tests inject a fake `PATH`.
#[must_use] pub fn with_probe_env(mut self, env: ProbeEnv, hardware: Arc<dyn HardwareSource>) -> Self;
/// Whether a box probe is still running.
#[must_use] pub fn box_probe_running(&self) -> bool;
/// Called by the store loop right after each `go_online` (D11, D26). Awaits nothing.
pub fn on_online(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>);
fn sweep_finished(&mut self);                                        // serve's :707-720 + box_probe.take_if(finished)
fn probe_env(&self) -> Result<(ProbeEnv, Arc<dyn HardwareSource>), StoreError>;  // injected, else host(cwd) + SystemHardware::host()
async fn probe_box(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>, addr: ReplyAddr) -> Result<Served, StoreError>;
```

Free functions:

```rust
async fn probe_agents_on(writer: &Writer, box_id: BoxId, mut agents: Vec<AgentSummary>, env: &ProbeEnv)
    -> Result<Vec<AgentSummary>, StoreError>;          // run_probe's loop body, unchanged in behaviour
async fn run_box_probe(args: BoxProbeArgs);
struct BoxProbeArgs { backend: Backend, writer: Writer, env: ProbeEnv, hardware: Arc<dyn HardwareSource>, decide: bool, frames: Frames }
```

`BoxProbeArgs` gets a hand-written `Debug` over the writer label, `decide` and `env`.

### 6.2 `agent_worker.rs`: behaviour

- **`serve`** (`:707-720`) calls `self.sweep_finished()`. It gains the arm `StoreRequest::ProbeBox => match self.probe_box(backend, replies, addr).await { Ok(s) => s, Err(err) => Served::Reply(failed("probe_box", &err)) }`.
- **`claim_is_free`** (`:1267`): the first check is `if self.box_probe_running() { return Err(StoreError::Backend(BOX_PROBE_RUNNING.to_owned())); }`. This covers `install_plan`, `install_confirm` and `auth_start`.
- **`probe`** (`:900-953`): after the install check, `if self.box_probe_running() { return Err(StoreError::Backend(BOX_PROBE_RUNNING.to_owned())); }`. The `cwd` block is replaced by `let (env, _) = self.probe_env()?;`, and `ProbeArgs { writer, box_id, agents, env, frames }` (D35).
- **`run_probe`** (`:1718-1768`) becomes `probe_agents_on(&writer, box_id, agents, &env)`: `Ok(a)` replies `Agents(a)`, and `Err(e)` replies `Failed { "probe_agents", e }`.
- **`probe_box`** (the `ProbeBox` path), in order and before anything spawns:
  1. `backend.writer()` or `Unreachable(REGISTRY_ON_SERVER_ONLY)`;
  2. `registered_box(backend).await?`;
  3. `self.claim_is_free()?`;
  4. `self.probe_env()?`.
  
  It then spawns `run_box_probe(BoxProbeArgs { backend: backend.clone(), writer, env, hardware, decide: false, frames: Frames::new(replies.clone(), addr) })` into `self.box_probe` and answers `Served::Deferred`.
- **`on_online`**:
  1. If `!self.registration_probe`, return.
  2. `self.sweep_finished()`.
  3. If `claim_is_free()` fails, `info!` and return (R-12).
  4. If `backend.writer()` is `None`, return.
  5. If `probe_env()` fails, `warn!` and return.
  6. Spawn `run_box_probe(.., decide: true, frames: Frames::new(replies.clone(), ReplyAddr { seq: UNSOLICITED, origin: Origin::App }))` into `self.box_probe`.
- **`run_box_probe`** (the reads go through the `Backend` clone and the writes through the `Writer` taken at spawn; D26):
  1. `box_id` from `backend.box_info()`. `Ok(None)` gives `box_failed = "this box is not registered"`; `Err(e)` gives `box_failed = e`; either way the report is sent and the task returns.
  2. `backend.app_settings()`; `Err` → `box_failed`. `let effective = box_probe::spec::effective(box_probe::spec::seed(), map.get(box_probe::spec::SETTING_KEY));` then `report.spec_error = effective.error.clone()`.
  3. If `decide`:
     - `writer.boxes()`, find `box_id`;
     - not found → `warn!` and return with no reply;
     - `!needs_probe(htui_store::HTUI_VERSION, &effective.digest)` → return with no reply (a reconnect costs three reads);
     - `Err` → `warn!` and return.
  4. `let probe = box_probe::probe_box(box_id, &env, hardware.as_ref(), &effective, HTUI_VERSION, Utc::now()).await;` then `writer.record_box_probe(&probe)`. `Err` → `box_failed`, send, return. Otherwise fill `tools` and `probed_tags`.
  5. `backend.agents()` → `probe_agents_on(&writer, box_id, agents, &env)`. `Ok(s)` fills `installable` = enabled rows with `on_box.and_then(ProbeSnapshot::from_row)` status `ProbeStatus::Missing` and `launch::declares_install(&agent.launch)`. `Err` → `agents_failed`.
  6. `frames.reply(&frames.addr(), StoreReply::BoxProbed(report))`.
- **`finish_background`** (`:466`): after `background`, `if let Some(task) = self.box_probe.take() { abort-on-timeout like background }`. **`shutdown`**: `if let Some(task) = self.box_probe.take() { task.abort(); }` beside the `background` aborts (D27).
- **`declares_a_source`** (`:2003-2016`): `if htui_agent::launch::declares_install(&agent.launch) { return Ok(()); }` followed by the existing `Err`. `:40` becomes `use htui_agent::launch::AgentSettings;` (F-E).

### 6.3 `store_worker.rs`

- After `StoreRequest::ProbeAgents` (`:200`):

```rust
/// Probe this box, then its agents, now (MOD-7 D11): the registration probe's path without the
/// "needs a probe" decision. Served by the agent runtime's own task; answered once with
/// [`StoreReply::BoxProbed`], or refused with [`StoreReply::Failed`] before anything spawns
/// (offline: `REGISTRY_ON_SERVER_ONLY`; a claim held). Milestone 1 binds it to no key (plan D15).
ProbeBox,
```

- `pub const UNSOLICITED: Seq = Seq::MAX;`, next to `Seq` (`:48`), with the doc "the address of a reply nobody asked for: `App::dispatch` counts up from 0 and never reaches it, so the freshness gate drops it after `observe_reply` (MOD-7 D13)".
- `StoreReply::BoxProbed(crate::agent_worker::BoxProbeReport)`, placed before `Failed`, with the doc "one per box probe, at the requester's address or `UNSOLICITED`".
- `name()`: `Self::ProbeBox => "probe_box"`.
- The `try_serve` refusal or-pattern (`:989-1005`) and the loop's runtime routing (`:1488-1501`) each gain `| StoreRequest::ProbeBox`, and their comments' counts go 14 → 15.
- Both swap sites (`:1281-1284` and `:1568-1571`) gain `runtime.on_online(&backend, &tx);` right after `go_online(..).await`, before `runs.sweep`.
- `:92` becomes `/// This box's row for the top bar.`, dropping "No probe: that is MOD-7".

### 6.4 `testkit.rs`, `lib.rs`, `app/update.rs`, `agents.rs`

- `testkit.rs:266-281`: `| StoreRequest::ProbeBox` is added to the runtime-routed tuple.
- `lib.rs:92`: `let worker = store_worker::spawn_with(started, request_rx, reply_tx, AgentRuntime::production().with_registration_probe());` with `use crate::agent_worker::AgentRuntime;`.
- `app/update.rs` `observe_reply` (`:238-262`) gains the arm `StoreReply::BoxProbed(report) => self.status = Some(report.status_line()),`.
- `agents.rs:1302-1307`: the body becomes `htui_agent::launch::declares_install(launch)`, and `AgentLaunch` is removed from `:45` (F-E).

### 6.5 Test env and fixtures (H-7, F-T)

The `agent_worker.rs` `mod tests` gains three helpers:
- `fn fake_env(tmp: &Path) -> ProbeEnv`: `cwd: tmp`, `platform: probe::platform_key()`, `home: None`, `vars: {PATH: tmp/bin}`, `versions: true`, `version_timeout: 5 s`.
- `fn script(bin, name, body)` (unix, mode 0755).
- `fn never_probed() -> MemStore`: `fixtures::demo_data()` with `boxes[0].last_probed_at = None`, loaded through `MemStore::from_demo`, then passed through the existing `unresolvable_registry` rewrite (`:5020-5036`).

Runtimes are built as `AgentRuntime::new(DriverFactory::production()).with_registration_probe().with_probe_env(fake_env(tmp), Arc::new(FixedHardware(Hardware { os_version: "Test OS 1".into(), cpu: "Test CPU".into(), ram_mb: Some(2048), display_vendors: vec!["0x1002".into()] })))`. The box task is awaited with `runtime.finish_background(Duration::from_secs(10))`.

### 6.6 Tests

`agent_worker.rs` unit tests:

| Test | Asserts |
|---|---|
| `an_online_swap_probes_a_box_that_was_never_probed` | After `on_online` + finish, `boxes()` shows `os_version "Test OS 1"`, `gpu_vendor amd`, `box_tool` = the fake scripts, `htui_version == HTUI_VERSION` and the digest of the seed. Every agent row's snapshot status is `missing`. Exactly one `BoxProbed` arrives, at `seq == UNSOLICITED`, `origin == Origin::App`. |
| `an_online_swap_skips_a_box_probed_at_this_version_and_spec` | A second `on_online` spawns a task that reads and returns: no reply, and `last_probed_at` is unchanged. |
| `an_online_swap_reprobes_after_a_version_change` | A `record_box_probe` with `htui_version "0.0.0"` is planted; the swap probes and rewrites `HTUI_VERSION`. |
| `probe_box_is_refused_offline_before_spawning_anything` | Mirror of `:5038-5086`: an `Offline` backend gives `Failed { "probe_box", REGISTRY_ON_SERVER_ONLY }`, `!box_probe_running()`, `background_len() == 0`. |
| `the_probe_box_task_answers_once_at_the_request_s_address` | Mirror of `:5088-5130`: one `BoxProbed` at the request's `(origin, seq)`. |
| `a_missing_agent_with_an_install_source_is_named_and_not_installed` | One agent row with `discovery.install` and an unresolvable tool: `installable == [name]`, `!install_running()`, no `Install` frame. |
| `an_install_is_refused_while_the_registration_probe_runs` | A fake tool sleeps 2 s. While `box_probe_running()`, `InstallPlan` → `Failed` with `BOX_PROBE_RUNNING`. |
| `probe_agents_is_refused_while_the_registration_probe_runs` | Same setup: `ProbeAgents` → `Failed` with `BOX_PROBE_RUNNING`. |
| `a_manual_agent_row_survives_the_registration_probe` | A planted `source: manual` row is byte-equal afterwards (MOD-2 D51). |
| `a_finished_background_task_does_not_stop_the_registration_probe` | `background.push(tokio::spawn(async {}))`, yield until finished, then `on_online` → `box_probe_running()`. |
| `production_runtime_does_not_auto_probe_without_opt_in` | `AgentRuntime::production()`: `on_online` → `!box_probe_running()`, `background_len()` unchanged. |
| `the_install_pre_flight_and_the_section_share_declares_install` | For three launch documents, `declares_a_source(&agent).is_ok() == declares_install(&agent.launch)`. |
| `a_stored_spec_adds_a_tool_and_reprobes_at_the_next_swap` | Probe; `set_app_setting("box_probe_spec", json!({"tools":{"terraform":{…}}}))` with a `terraform` script; the second swap probes, `box_tool` holds `terraform`, and the digest changed. |
| `an_unchanged_spec_does_not_reprobe` | Two swaps with the same row: one probe. |
| `removing_the_stored_spec_reprobes_with_the_seed` | Plant, probe, `set_app_setting` to `Value::Null`-free removal (a fresh `from_demo` store carrying the recorded probe); the next swap probes, and the digest equals `digest(seed())`. |
| `an_invalid_stored_spec_probes_the_seed_and_the_report_says_so` | `json!(42)`: `spec_error` starts with `SPEC_IGNORED`, the recorded digest is the seed's, and tools were written. |
| `htui_version_is_the_binary_s_version` | `htui_store::HTUI_VERSION == env!("CARGO_PKG_VERSION")` (D28). |

For `removing_the_stored_spec_reprobes_with_the_seed`: `MemStore` has no `remove_app_setting`. The case builds the "after" store from the "before" store's recorded probe plus no setting. If the implementer prefers, a tests-only `MemStore::clear_app_setting` is **not** allowed in T4 (`mem.rs` is T2's). The re-built store is the prescribed way.

`store_worker.rs` tests:

| Test | Asserts |
|---|---|
| `probe_box_is_named_and_refused_without_a_runtime` | `name() == "probe_box"`; `serve(&backend, &ProbeBox)` → `Failed { "probe_box", "no agent runtime in this build" }`. |
| `the_loop_answers_box_info_while_a_box_probe_is_in_flight` | Mirror of the `ProbeAgents` case (`:2410`), with a sleeping fake tool. |

`app/update.rs` test:

| Test | Asserts |
|---|---|
| `a_box_probed_reply_lands_on_the_status_line_whatever_its_seq` | `Action::Reply` at `UNSOLICITED` → `app.status == Some(report.status_line())`, and no tab saw it. |

`crates/htui/tests/box_probe.rs` (new, `#![cfg(feature = "testkit")]`):

| Test | Asserts |
|---|---|
| `probe_box_through_the_shell_reports_on_the_status_line` | `Harness::over(unresolvable demo store).with_agent_runtime(runtime with with_probe_env)`; `app().update(Action::Store(StoreRequest::ProbeBox))`; `drive_to_end()`; `app().status` starts with `"box probed: "`. |

### 6.7 Build coupling

T4 consumes the following public surface, all present after the Wave A merges:
- **T1**: `htui_store::HTUI_VERSION`.
- **T2**: `WriteStore::{record_box_probe, boxes}` and `BoxRecord::needs_probe`.
- **T3**: `htui_agent::box_probe::{probe_box, hardware::{HardwareSource, SystemHardware, FixedHardware, Hardware}, spec::{effective, seed, digest, SETTING_KEY, SPEC_IGNORED}}` and `htui_agent::launch::declares_install`.
- **Existing**: `htui_agent::probe::{ProbeSnapshot, ProbeStatus, platform_key}`.

### 6.8 Gate

```bash
cargo test -p htui --all-features -- --test-threads=1
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

### 6.9 Commit boundaries (T4)

1. (a) Red: `BoxProbeReport`, `ProbeBox`, `BoxProbed`, `UNSOLICITED`, the three request lists, `name()`, and runtime fields and methods with `todo!()` bodies (`on_online`, `probe_box`, `run_box_probe`), plus every test.
2. (b) Green: `sweep_finished`, `probe_env`, `probe_agents_on` (the `run_probe` split), `claim_is_free`/`probe` refusals, `finish_background`/`shutdown`.
3. (c) Green: `run_box_probe`, `on_online`, `probe_box`, the swap sites, `lib.rs`.
4. (d) Green: `observe_reply`, the `declares_install` delegations, the import removals, the `:92` doc.

---

## 7. T5: Postgres end to end (PRD metrics)

`crates/htui/tests/box_probe_pg.rs`, `#![cfg(feature = "testkit")]`. The header and backend construction mirror `crates/htui/tests/runs_pg.rs`:
- `fresh_db`;
- a `CacheStore::open(root, "box-probe-pg", PgStore::schema_version())`;
- `Backend::Online { pg: db.store.clone(), cache }`;
- a `mock_keyring` guard.

Every seeded agent row is rewritten to the unresolvable launch through `db.store.upsert_agent` (F-T). The runtime is `AgentRuntime::production().with_registration_probe().with_probe_env(fake_env, Arc::new(FixedHardware(..)))`. Each swap is `runtime.on_online(&backend, &tx); runtime.finish_background(10 s).await`.

| Test | Asserts |
|---|---|
| `the_first_registration_probes_the_box` | `os_version`, `cpu`, `ram_mb` and `gpu_present` are non-default; `last_probed_at` is set; `box_tool` rows exist; `probed_tags` are set; `probe_spec_digest = digest(seed())`. |
| `a_reconnect_at_the_same_version_does_not_probe` | A second swap: `last_probed_at` byte-equal (`::text`). |
| `a_version_change_reprobes` | `UPDATE box SET htui_version = '0.0.0'`; the next swap probes and `htui_version = HTUI_VERSION`. |
| `a_renamed_box_keeps_its_row_and_its_probe` | `db.store.register_box(&Identity { box_id, hostname: "renamed".into() }, None)` → `Known { renamed_from: Some(_) }`; one row; the probe columns are intact. |
| `a_stored_spec_change_reprobes` | `INSERT INTO app_setting (key, value) VALUES ('box_probe_spec', '{"tools":{"terraform":…}}'::jsonb)`; the next swap probes and `probe_spec_digest` changes. |

---

## 8. Cross-task contracts

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T0 `htui_core::model` | `BoxProbe { box_id, os_version, cpu, ram_mb: Option<i32>, gpu_present, gpu_vendor: Option<String>, tools: Vec<ProbedTool>, probed_tags: Vec<String>, htui_version, spec_digest, probed_at }` | T2 (write), T3 (build), T4 |
| T0 | `ProbedTool { name, version, path }` | T2, T3 |
| T0 | `BoxRecord { row: BoxRow, tools: Vec<BoxTool>, probe_spec_digest: Option<String> }`, `needs_probe(&self, running: &str, spec_digest: &str) -> bool` | T2 (build), T4 (decide) |
| T1 `htui_store` | `HTUI_VERSION: &str`, `Registration { New, Known { renamed_from }, Copied { previous, minted } }`, `PgStore::register_box(&self, &Identity, Option<&Fingerprint>) -> Result<Registration>`, `PgStore::registration()` | T4, T5 |
| T1 schema | `box.probe_spec_digest TEXT CHECK hex64` | T2 queries |
| T2 `WriteStore` | `record_box_probe(&self, &BoxProbe) -> Result<()>`, `boxes(&self) -> Result<Vec<BoxRecord>>` | T4, T5 |
| T3 `htui_agent::box_probe` | `probe_box(BoxId, &ProbeEnv, &dyn HardwareSource, &EffectiveSpec, &str, DateTime<Utc>) -> BoxProbe`; `spec::{seed, effective, digest, SETTING_KEY, SPEC_IGNORED, EffectiveSpec { spec, digest, error }}`; `hardware::{HardwareSource::read(&self, &ProbeEnv), SystemHardware::host(), FixedHardware(Hardware { os_version, cpu, ram_mb, display_vendors })}` | T4, T5 |
| T3 `launch` | `declares_install(&serde_json::Value) -> bool` | T4 |
| T4 `htui` | `StoreRequest::ProbeBox`, `StoreReply::BoxProbed(BoxProbeReport { tools, probed_tags, installable, agents_failed, spec_error, box_failed })`, `UNSOLICITED`, `BOX_PROBE_RUNNING`, `AgentRuntime::{on_online, with_registration_probe, with_probe_env, box_probe_running}` | T5, milestone 2 |

**Parallel-lane hazards (T3 ∥ T1→T2)**:
- There is no shared file. T3 touches `htui-agent/src/{box_probe/**, probe.rs, launch.rs, lib.rs}` and `tests/box_probe.rs`; T2 touches `htui-agent/src/conformance.rs` and `tests/recorder.rs`.
- `Cargo.lock` moves only in T0. `.sqlx` moves only in T1 and T2.
- T3's worktree does not have T2's trait methods, so its `htui-agent` gate is re-run after the merge.
- `htui_prepare_mod7` is T1/T2's alone; T3 builds with `SQLX_OFFLINE=true`.
- Two worktrees mean two `target/` directories. Check `df -h /` before Wave A.

---

## 9. Merge order and the workspace gate

T0 → [T1 → T2] ∥ T3 → merge T1 (store gate), T2 (core, store, agent gates), T3 (agent gate) → T4 (htui gate) → T5.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features)
cargo doc --workspace --no-deps --keep-going
grep -rn 'machine_fingerprint\|Fingerprint' crates --include='*.rs' | grep -E 'info!|warn!|debug!|tracing::'   # nothing
cargo tree --target x86_64-unknown-linux-gnu -p sysinfo -e normal
grep -c '^name = "windows"$' Cargo.lock      # 1
```

The live check is the plan's (§Validation), unchanged.

---

## 10. Count pins

| Pin | Now | After | Task |
|---|---|---|---|
| Store `CASES` | 53 | 56 | T2 (`conformance.rs`, `mem_store.rs:37`, `pg_conformance.rs:19`) |
| `READ_CASES` | 9 | 9 | — |
| Applied migrations | `[1..4]` | `[1..5]` | T1 (`migrations.rs:74`, `:591`) |
| Pending | 4 | 5 | T1 (`migrations.rs:623`, `connect.rs:95`, `:110`) |
| Commented columns | 25 | 29 | T1 (D30) |
| `TABLES` | 33 | 33 | — |
| `StoreRequest` / `StoreReply` | 63 / 34 | 64 / 35 | T4 |
| `.sqlx` | 227 | 229 → 234 | T1, T2 |
| `MIRRORED_TABLES` | 17 | 17 | — |

---

## 11. Risks (continuing from the plan's R-11)

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-12 | A reconnect that lands while a chat re-probe or a preview holds `background` skips the registration probe until the next swap (F-M). | Low | `info!` names it; `ProbeBox` exists; the next launch retries because nothing was recorded. |
| R-13 | An offline launch from a copied config directory shows the old box's hostname until the first online pass prunes the mirror. | Low | The first `Online` pass prunes (T1 `refresh_box`); registration has already minted. |
| R-14 | On macOS, `gcc` is Apple clang: its first line fails the `gcc` pattern, so `gcc` is absent (OQ-11), which is arguably right. `java` stubs without a JDK are absent the same way. | Certain there | Data: an overlay can replace the pattern (D17). |
| R-15 | `boxes()` per swap reads every box and tool of the user; this grows with fleet size. | Low | Milestone 1 has one to three boxes. A one-box read is milestone 2's if needed. |
| R-16 | An edited `0005` after the first scratch migrate breaks `sqlx migrate run` on that database. | Medium | F-S: recreate `htui_prepare_mod7` before every prepare. |
| R-17 | `hmac`'s internal ipad/opad buffers are derived from the raw identity and are not zeroized. | Low | They are XOR-derived, not the raw text, and live for one call on the stack of a process that already read the file. Accepted. |

---

## 12. Decisions (D19 onward)

| # | Decision |
|---|---|
| D19 | `Registration::Copied { previous, minted }` and `Registration::box_id(presented)`. The `Copied` insert reuses statement (a)'s text. |
| D20 | The Linux machine-id reader accepts only 32 hex characters; `uninitialized` or empty falls through. The fingerprint is cached per process in a `tokio::sync::OnceCell`. macOS `ioreg` runs under 5 s with `kill_on_drop`; Windows reads the registry in `spawn_blocking`. |
| D21 | `Hardware.display_vendors` holds raw PCI ids. `probe_box` names the GPU through `spec.gpu_vendors` (`pick_gpu_vendor`). |
| D22 | `HardwareSource: Send + Sync + Debug`, `fn read<'a>(&'a self, env: &'a ProbeEnv) -> HardwareFuture<'a>`. |
| D23 | The overlay merge algorithm and fault sentences of §5.4. Disabled tools are pruned from seeded rules; only overlay-written rules are refused for naming absent tools. |
| D24 | `digest = sha256_hex(serde_json::to_string(&Spec))`, over the merged `Spec` only. It is computed only in `spec::effective`/`spec::digest`. |
| D25 | The box task's only reply is `BoxProbed(BoxProbeReport)`, with `box_failed`/`agents_failed`. `observe_reply` renders `status_line()`. Pre-spawn `ProbeBox` refusals are `Failed`. |
| D26 | `on_online` guards on the opt-in and on `writer()`, never on the backend kind. The call sites are the `--demo` guard. The task reads through a `Backend` clone and writes through the `Writer` taken at spawn. |
| D27 | The `box_probe` slot is swept, claimed, awaited in `finish_background` and aborted in `shutdown`. One sentence: `BOX_PROBE_RUNNING`. |
| D28 | `htui_version_is_the_binary_s_version` pins `htui_store::HTUI_VERSION` to the binary's package version. |
| D29 | No committed PCI fixture trees (no `:` paths): tempdir trees at run time. No `#[cfg(test)]` modules in `box_probe/*.rs`, so the grep gate is literal. |
| D30 | `MOD7_COLUMN_COMMENTS` beside `ANA_COLUMN_COMMENTS`; the exclusivity count is 29 over the union. |
| D31 | `.sqlx` 227 → 229 (T1) → 234 (T2); prepare runs against a recreated `htui_prepare_mod7`. |
| D32 | `MemStore::record_box_probe` refuses a non-hex digest with `Constraint`, as Postgres's `CHECK` does. |
| D33 | Conformance cases use micro-truncated fixed instants and `sha256_hex` digests. |
| D34 | The now-unused `AgentLaunch` imports in `agents.rs:45` and `agent_worker.rs:40` are removed with the delegation. |
| D35 | `ProbeAgents` uses the injected `ProbeEnv` when present (`probe_env()`), else `ProbeEnv::host(cwd)`. |
| D36 | `tags_from_presence` is pure over precomputed `ask` answers. An `ask` runs the first present tool of its rule by its resolved absolute path, through `run_bounded`, under the same eight-permit semaphore. |
| D37 | `boxes()` on Postgres filters by `PgStore::this_user` and reads tools with `ANY($1) ORDER BY box_id, name COLLATE "C"`; `MemStore` answers the same order from `this_user()`. |
| D38 | Seed version patterns are those of §5.5. Presence-only entries are `powershell` and `vulkaninfo`. The `mingw` rule's `ask` is `{"args": ["-dumpmachine"], "matches": "mingw"}`. |
