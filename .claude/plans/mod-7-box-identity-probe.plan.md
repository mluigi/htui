# Plan: MOD-7 milestone 1 — a box knows itself

**Source**: `.claude/prds/mod-7-box-registry.prd.md`, milestone 1 (Delivery Milestones table, row 1):
"Registration keys on the id and survives a rename, a copied `box.toml` mints a new box, and the
probe fills hardware, `box_tool` and `probed_tags` at registration, on `htui` version change and on
demand; agents are probed on registration and missing ones offered for install. No UI yet beyond
what exists." Scope bullets "Box identity (D1)", "Box probe" and "Agent autodiscovery on
registration (`R-AGT-6`)"; success-metric rows "Box row completeness", "Tool list", "Re-probe
triggers", "Rename survives", "Copy refused", "Fingerprint confidentiality" and "UI never blocks".
Design authority: the PRD's gate decisions **PRD D0–PRD D7** (settled, not reopened here);
`docs/REQUIREMENTS.md` `R-BOX-1` (`:62-63`), `R-BOX-2` (`:64-67`), `R-BOX-3` (`:68-70`), `R-AGT-6`
(`:159-160`), `R-NF-3` (`:326-327`); `docs/ANA-16.md` §6.1 finding C4 (`:484`) and §8 (`:699`,
`:772`).

**Requirements**: `R-BOX-1` (hostname, OS family and version, architecture, CPU, RAM, GPU presence
and vendor at first registration), `R-BOX-2` (tool list with versions; re-probe on demand and when
the `htui` version changes; `last_seen` per session), `R-BOX-3` (probed tags only — declared tags and
quirks are milestone 2), `R-AGT-6` (agents probed on registration and on demand), `R-NF-3` (probe
off the UI task), ANA-16 C4 (box identity keyed on the `box.toml` id, not the hostname).

**Complexity**: Large. One forward-only migration (`0005`); `register_box` rewritten as a
transaction with a new return type; a keyed machine fingerprint in `htui-store`; two new
`WriteStore` methods across five implementations plus three conformance cases; a new
`htui_agent::box_probe` module (hardware, tool list, tag map as data); one new `StoreRequest`, one
new `StoreReply`, and a runtime hook on every `Online` swap. One new crate edge (`hmac 0.12`, already
compiled on Linux) and one target-gated edge (`windows-registry 0.6`, already compiled on Windows).
**(Amended at maintainer review.)** One new crate family, `sysinfo 0.39.6` with only its `system`
feature, which adds three packages to `Cargo.lock` (`sysinfo`, plus `ntapi` on Windows only and
`objc2-io-kit` on macOS only) and reuses the workspace's single `windows 0.62.2` (D6). The probe
spec is no longer compiled-only: a stored `app_setting` row overlays the compiled seed and its
digest is recorded on the box, so editing it triggers a re-probe (D17, D18). That digest is a third
`0005` column, which makes T2 serial after T1.

**Routing**: routed as **plan** by `/handoff-run MOD-7` (the PRD and its milestone table exist).
**Staffing: Opus 5.5 for every step — plan, fact-check, architect, implementers, verifiers and
reviewer (`rust-reviewer`, `.claude/workflow-config.json:2`); Fable is not used (maintainer standing
instruction).** Ultracode for the implementers only, one workflow per task, verify fan-out per
round; the architect and the reviewer stay plain agents.

**Numbering**: this is MOD-7's first plan. Its own decisions are **D1…D18** (D17 and D18 added at
maintainer review); the PRD's gate decisions are always cited as **PRD D0…PRD D7** and never
re-used. Risks start at **R-1**, open questions at **OQ-1**, tasks at **T0**. Later MOD-7 milestones
continue from **D19**, **R-12** and **OQ-13** (amended at maintainer review; was D17 and R-9).

**Status**: **confirmed** (2026-09-25) by the maintainer with OQ-11 and OQ-12's defaults, after two
fact-check passes (172 + 38 claims; maintainer-review amendments re-checked). Branch `mod-7-box-registry` at `1475b17`. The first pass (172
claims, six verifiers: 153 verified, 8 partial, 11 falsified) checked the draft; the maintainer then
answered every open question (OQ-1, OQ-3…OQ-8, OQ-10 as written; OQ-2 reversed to `sysinfo`; OQ-9
changed to a stored, overlayable probe spec); the second pass (38 claims) checked those amendments.
Marks: "(amended at fact-check)", "(amended at maintainer review)", "(amended at re-check)". The
ledger is the "Verified claims" table at the end; the second pass's rows are tagged
`[maintainer-review]`.

**Graphify note**: `graphify-out/` does not exist in this checkout (`ls graphify-out` fails), so
nothing here was read from it. Every tree fact below was read through the Gortex index or the file
itself at `1475b17`; each carries a `file:line`, and anything not re-opened at its line is marked
**UNVERIFIED**.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked. **(Amended at maintainer
review, 2026-09-25:** OQ-1 "yes, the default stands"; OQ-3…OQ-8 and OQ-10 "ok as written"; OQ-2
reversed and OQ-9 changed, both rewritten below; OQ-11 and OQ-12 are new and still open.)

- [x] **OQ-1 — A lost `box.toml` on the same machine.** *Answered: default stands.* PRD D1 keys on the id and says a mismatch
      mints; it is silent on the opposite case: a fresh id (the file was deleted, or the config
      directory was reset) arriving on a machine whose fingerprint an existing row already holds.
      Today's adopt-DB-id rule (`crates/htui-store/src/pg/mod.rs:344-349`) would hand back the old
      id by hostname. **Default adopted (D3):** no adoption of any kind. An id the database has
      never seen is a new row, always; the old row stays and milestone 2's section lists it.
      Adopting by fingerprint would re-merge exactly the cloned VMs ANA-16 C4 is about (a clone
      shares its machine-id unless regenerated, PRD Evidence), and adopting by hostname is C4
      itself. **Alternative:** adopt when exactly one row of this user carries the same fingerprint
      and has not been seen for N days.
- [x] **OQ-2 — A system-information crate or direct reads.** The PRD leaves it to the plan
      ("A new system-information dependency is a decision, justified in the plan").
      *Answered (amended at maintainer review): "we probably need it" — use `sysinfo`.* The draft
      default (no crate, per-OS files and commands) is withdrawn. **Adopted (D6):** `sysinfo =
      { version = "0.39.6", default-features = false, features = ["system"] }` in `htui-agent`
      for OS version, CPU brand and total RAM on all three platforms. It does not give GPU or the
      machine identity, so D7 (GPU) and D1 (fingerprint) stay per-OS. Resolved in a throwaway copy
      of the tree under `/tmp`: `Cargo.lock` gains exactly `sysinfo 0.39.6`, `ntapi 0.4.3`
      (Windows only) and `objc2-io-kit 0.3.2` (macOS only); on Linux `sysinfo` compiles against
      `libc` and `memchr` only, both already in the lock; on Windows it uses the lock's one
      `windows 0.62.2`, the same version `htui-agent` already names, with 20 more of its features
      switched on (`Wdk_System_*`, several `Win32_System_*`, `Win32_UI_Shell`) (amended at re-check). MIT, `rust-version = "1.95"` (workspace MSRV 1.98, `Cargo.toml:8`).
- [x] **OQ-3 — Does an integrated GPU earn the `gpu` tag?** *Answered: ok as written.* This box has only an AMD iGPU
      (`/sys/bus/pci/devices/0000:0f:00.0`, class `0x030000`, vendor `0x1002`, device `0x164e`;
      `lspci` names it "Raphael"). **Default adopted (D7):** yes — any display-class PCI device from
      a vendor in the data map is a GPU; virtual adapters (QEMU/Bochs `0x1234`, VMware `0x15ad`,
      Red Hat virtio `0x1af4`, Hyper-V `0x1414`) are not in the map and so never count.
      **Alternative:** discrete only, which on Linux needs per-vendor heuristics (an AMD APU and an
      AMD discrete card share a vendor id).
- [x] **OQ-4 — The compare-and-set token milestone 2's editors will use.** *Answered: ok as
      written.* The PRD's open question
      ("Which token the declared-tags and quirks CAS compares, given reconnects bump `updated_at`").
      Every reconnect bumps `box.updated_at` today (`last_seen_at` update in
      `pg/mod.rs:358-362` plus the `set_updated_at` trigger, `0001_init.sql:575-579`), and after
      this milestone every probe does too. **Default adopted (D14):** migration `0005` adds
      `box.edit_version INTEGER NOT NULL DEFAULT 0`, bumped only by human-edit writers (milestone
      2's declared tags and quirks, MOD-12's caps later) and never by registration or the probe.
      Milestone 1 adds the column and nothing that reads or writes it. **Alternative:** a
      value-compare CAS (`WHERE declared_tags = $old AND quirks = $old`) that needs no column.
- [x] **OQ-5 — What `box.htui_version` means.** *Answered: ok as written* (D18 adds the spec
      digest as a second trigger beside it; amended at maintainer review). It is documented as the re-probe trigger
      (`0001_init.sql:64`), but `register_box` overwrites it on every connect
      (`pg/mod.rs:360`), so a probe interrupted after a version bump would never be retried.
      **Default adopted (D5):** it becomes "the version at the last successful probe": inserted at
      first registration, never touched by a reconnect, rewritten only by the probe writer.
      **Alternative:** keep today's write and add a `probed_version` column.
- [x] **OQ-6 — How a missing agent with an install source is surfaced in milestone 1.**
      *Answered: ok as written.* PRD D7:
      offered, never installed; the milestone says "no UI yet beyond what exists".
      **Default adopted (D13):** the probe's report lands on the existing status line
      ("box probed … · agy is missing and can be installed: Settings > Agents, i"), and the agents
      section already renders the row `missing` with the MOD-20 `i` action
      (`crates/htui/src/ui/tabs/settings/agents.rs:866-893`, `:729-760`). No install request is
      sent, not even the pre-flight. **Alternative:** say nothing until milestone 2's section.
- [x] **OQ-7 — Does an on-demand box re-probe also re-probe the agents?** *Answered: ok as
      written.* **Default adopted (D11):**
      yes — one path, box then agents, whether the trigger is registration or `ProbeBox`. The
      agents section's own `r` stays as it is. **Alternative:** `ProbeBox` probes the box only.
- [x] **OQ-8 — An id that exists under another `app_user`.** *Answered: ok as written.* Single-user today (`R-USR-2`), so this
      can only be a copied file. **Default adopted (D3):** treated as a copy: a new id is minted.
- [x] **OQ-9 — The tool list.** `R-BOX-2` names compilers, build tools, "shells" and "container
      runtime" without listing the last two. *Answered (amended at maintainer review): "we probably
      need others too, so make it dynamic".* The draft default (a compiled list of sixteen) is
      withdrawn. **Adopted (D9, D17, D18):** the compiled `spec.json` becomes the **seed**; the
      live spec is the seed overlaid, merge-by-name, by an `app_setting` row keyed
      `box_probe_spec`, read at probe time through the existing `Backend::app_settings()`; an
      invalid stored row falls back to the seed and the report says so; the effective spec's digest
      is recorded on the box, so editing the row re-probes at the next `Online` swap. The seed
      widens from sixteen to thirty-nine tools (D9's list; `gradle` and `bazel` left out because their
      `--version` can start a daemon or download a binary). Milestone 2 ships the editor (D15);
      milestone 1's edit path is one documented SQL statement (D17).
- [ ] **OQ-11 — A tool on `PATH` whose version probe prints nothing** (new, amended at maintainer
      review). This box has one: `~/.volta/bin/pnpm` is a Volta shim that answers `Volta error:
      Could not find executable "pnpm"`. Mise, asdf and the Windows Store `python3` stub behave
      the same way. **Default adopted (D9):** when the spec gives a tool a `version` probe, the tool
      counts only if a version is captured; a tool with no `version` probe (`vulkaninfo`,
      `powershell`) counts on presence. So a broken shim is absent, and so is a tool that hangs past
      `VERSION_TIMEOUT`. This needs versions captured at all, so `probe_box` forces
      `ProbeEnv.versions` on (D8) (amended at re-check). **Alternative:** the draft's behaviour, present with an empty version.
- [ ] **OQ-12 — New derived tags beyond `R-BOX-3`'s ten** (new, amended at maintainer review).
      `R-BOX-3` seeds ten tags and says "Vocabulary is open". **Default adopted (D9):** the seed
      derives five more: `go` ← `go`; `node` ← `node`; `python` ← `python3`; `java` ← `javac` (a
      JDK, not a JRE, is what a build needs); `dotnet` ← `dotnet`. They are **not** inserted into
      `capability_tag`: nothing enforces that table (`box.probed_tags`, `box.declared_tags` and
      `item.required_tags` are plain `TEXT[]`, `0001_init.sql:65-66`, `:322`, with no foreign key),
      and no Rust code reads it (its only writers are the seed insert at `pg/mod.rs:275-278`; its
      only other mention is a doc line at `pg/demo.rs:27`). Milestone 2's tag picker can list
      `capability_tag` together with every tag seen on a box. **Alternative:** derive no new tags
      and leave language runtimes to declared tags.
- [x] **OQ-10 — `--demo` mode.** *Answered: ok as written.* **Default adopted (D11):** the registration probe never runs on a
      `Backend::Memory`; only an explicit `ProbeBox` does, as `ProbeAgents` does today.

---

## Summary

Today `register_box` upserts on `(user_id, hostname)` (`crates/htui-store/src/pg/mod.rs:353-377`)
and nothing else fills the box row: `os_version` is `''`, `cpu`, `ram_mb`, `gpu_*`, `probed_tags`
and `last_probed_at` are never written by production code, and `box_tool` is written only by the
demo loader (`crates/htui-store/src/pg/demo.rs:257`). Worse, a hostname change is a crash: `box.toml`
keeps its id across a rename (`crates/htui-store/src/identity.rs:190`), the upsert finds no row for
the new hostname and inserts the old id again. **Reproduced on the scratch database
`htui_prepare_check` inside a rolled-back transaction**: the second insert fails with
`duplicate key value violates unique constraint "box_pkey"`.

**Identity (T1).** `register_box` becomes one transaction keyed on the `box.toml` id. A known id
updates the hostname and `last_seen_at`; an unknown id inserts; a stored machine fingerprint that
disagrees with this machine's means the file was copied, and a new id is minted while the old row is
left exactly as it was (PRD D1). The fingerprint is HMAC-SHA256 over the OS machine identity
(`/etc/machine-id`, `IOPlatformUUID`, `MachineGuid`) with a compiled `htui` constant (D1); the raw
identity lives in a zeroized local for the length of one function call and is never a struct field,
a column, a log field or a file. Migration `0005` drops `box_user_id_hostname_key` (the constraint
name, read from `pg_constraint` on `htui_prepare_check`) and adds `machine_fingerprint`,
`edit_version` and `probe_spec_digest` (amended at re-check). `connect::try_connect` already writes an adopted id back to `box.toml`
(`crates/htui-store/src/connect.rs:432-449`); it now does so for a minted one.

**The probe (T3).** `htui_agent::box_probe` reads the GPU per OS and the rest through `sysinfo` (amended at re-check) (D6, D7), resolves a tool list
that is data — the same `ToolProbe` shape agent rows use, run through the existing tier-1 resolver
`resolve_tool` (`crates/htui-agent/src/probe.rs:354-413`) — and derives `probed_tags` from a
presence map that is also data (PRD D2, D9). Nothing in code names a tool or a tag.
**(Amended at maintainer review.)** OS version, CPU and RAM come from `sysinfo` (D6); GPU stays
per-OS (D7). The compiled spec is only the seed: an `app_setting` row `box_probe_spec` overlays it
by name, and the effective spec's digest is stored on the box so an edit re-probes (D17, D18).

**The write surface (T2).** `WriteStore::record_box_probe` writes the hardware columns,
`probed_tags`, `htui_version`, `last_probed_at` and `probe_spec_digest` (amended at re-check) and replaces this box's
`box_tool` set in one transaction, touching nothing a human owns. `WriteStore::boxes` reads every
box with its tools and its recorded spec digest (amended at re-check) —
what the conformance case needs to read back and exactly what milestone 2's section will list.

**The trigger (T4).** After every `Online` swap the agent runtime reads this box's row and probes
when it was never probed or was probed by another `htui` version (D5, D11) — or, amended at
maintainer review, under another effective probe spec (D18) — so a reconnect costs reads, not a
probe. The probe runs on a runtime-owned task (the `ProbeAgents` pattern,
`crates/htui/src/agent_worker.rs:900-953`), holds the install claim while it runs, probes the box,
then this box's agents through the existing agent probe, and reports on the status line. A `missing`
agent whose row declares an install source is named with the `Settings > Agents, i` offer and never
installed (PRD D7). `StoreRequest::ProbeBox` exposes the same path on demand for milestone 2.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **The fingerprint.** `htui_store::identity` gains `pub struct Fingerprint([u8; 32])` with a hand-written `Debug` that prints `Fingerprint(<redacted>)`, no `Display`, no `Serialize`, and one accessor `as_hex(&self) -> String` used only to bind the column. `Fingerprint::from_machine_identity(raw: &str) -> Option<Self>` normalises (trim, ASCII-lowercase; empty → `None`) and computes **HMAC-SHA256 keyed by the normalised machine identity over the compiled message `FINGERPRINT_APP_ID = b"htui/box-fingerprint/v1"`** — modelled on systemd's application-specific machine id (`machine-id(5)`, `sd_id128_get_machine_app_specific`: HMAC-SHA256 keyed by the machine id over an application id). Unlike systemd it keys the normalised ASCII identity text rather than the 16 binary bytes, uses a string domain separator, and keeps all 256 bits without setting UUID bits, so its values never equal systemd's (amended at fact-check). The compiled constant is the domain separator that stops the value correlating with any other program's. Known answer, pinned by a test: raw `0123456789abcdef0123456789abcdef` → `a2e8a0ed177cf8b655e4a8c2d16f745209325966a8339fd26e7c6437dae364df` (computed with Python's `hmac` on this box). `pub async fn machine_fingerprint() -> Option<Fingerprint>` reads the OS once per process (`tokio::sync::OnceCell`): Linux `/etc/machine-id`, then `/var/lib/dbus/machine-id` (the first is `-r--r--r--` root-owned on this box); macOS the `IOPlatformUUID` line of `/usr/sbin/ioreg -rd1 -c IOPlatformExpertDevice` (a `tokio::process` child under a 5 s timeout, `kill_on_drop`); Windows `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` through `windows_registry::LOCAL_MACHINE.open(..)?.get_string(..)` (both exist in `windows-registry 0.6.1`: `lib.rs:64`, `key.rs:15`, `key.rs:149` of the vendored crate). The raw text is held in a `zeroize::Zeroizing<String>` local and dropped inside the function. Anything unreadable, empty or timed out is `None` — "no fingerprint available" (PRD D1). The HMAC comes from `hmac = "0.12"`, the version `dbus-secret-service` already compiles on Linux (`Cargo.lock:2892`) and the one that shares `digest 0.10` with the workspace's `sha2 0.10` (`Cargo.toml` `[workspace.dependencies]`). | PRD D1 fixes a keyed HMAC-SHA256 whose raw input never leaves the box; `R-SEC-2`'s redacting-`Debug` rule is already how `ProbeEnv` (`probe.rs:79`) and `AgentLaunch` (`launch.rs:90-99`) keep values out of logs. Hand-rolling RFC 2104 over `sha2` was considered and rejected: `hmac` is the RustCrypto implementation, already in the lock, and one small crate on macOS and Windows. |
| D2 | **Where the mismatch is decided: inside the store's transaction.** `Identity` (`identity.rs:21-27`) is **unchanged** — the fingerprint is not part of `box.toml`, and adding a field would break 13 struct literals: `crates/htui/src/store_worker.rs:2039`, `:2250`, `crates/htui/tests/connection.rs:736`, `identity.rs:72`, `:82`, `:196`, `:214` and `crates/htui-store/tests/migrations.rs:913`, `:961`, `:972`, `:1023`, `:1034`, `:1076` (amended at fact-check: the draft named three). Instead `PgStore::register_box(&self, identity: &Identity, fingerprint: Option<&Fingerprint>) -> Result<Registration>`, and `bootstrap` (`pg/mod.rs:418-424`) calls it with `identity::machine_fingerprint().await`. `pub enum Registration { New, Known { renamed_from: Option<String> }, Copied { previous: BoxId } }` is kept on the store (`PgStore::registration()`), and the id it carries (the minted one for `Copied`) becomes `identity.box_id` and `this_box` exactly as the adopted id does today. `try_connect` (`connect.rs:432-449`) keeps its write-back unchanged in mechanism and changes its log line: `Copied` is a `warn!` naming the previous and the minted **box ids** (never a fingerprint) and saying `box.toml` was carried from another machine. | One transaction is the only place two processes of one box registering together cannot both decide; the caller already owns the `box.toml` write-back, so nothing new crosses the crate boundary. A parameter instead of a field keeps the change inside `htui-store` (T1 touches no `htui` file). |
| D3 | **The SQL shape of `register_box`.** In one transaction, **first** `INSERT … ON CONFLICT (id) DO NOTHING RETURNING id` with the presented id, hostname, `os_family`, `''` for `os_version`, arch, `htui_version = HTUI_VERSION`, and the fingerprint (or `NULL`). (a) A row comes back → `New`. Otherwise the row exists and is now safe to lock: `SELECT user_id, machine_fingerprint, hostname FROM box WHERE id = $1 FOR UPDATE`, then (b) or (c). (Amended at fact-check: the draft began with the `SELECT … FOR UPDATE`, which locks nothing when the id is absent, so two processes of one box at first registration would both `INSERT` and the second would fail on `box_pkey`. Insert-first makes the losing process fall through to the lock. T1 adds `two_first_registrations_of_one_id_both_succeed`, two concurrent `register_box` calls on a fresh id: both `Ok`, one row.) (b) A row whose `user_id` is not `this_user` (OQ-8), or whose stored fingerprint and the presented one are both present and differ → `INSERT` a fresh row under `BoxId::new()` and leave the old row untouched, `last_seen_at` included → `Copied { previous }`. (c) Otherwise → `UPDATE box SET hostname = $h, os_family = $f, arch = $a, last_seen_at = clock_timestamp(), machine_fingerprint = COALESCE(machine_fingerprint, $fp)` → `Known { renamed_from }` when the hostname changed. A presented `None` never clears a stored fingerprint; a stored `NULL` (every row that exists before `0005`) is filled by the first connect that has one. **A different id on the same hostname is a different box, always** (OQ-1): there is no hostname lookup left anywhere in registration. `htui_version`, the probe columns, `declared_tags`, `quirks`, `settings` and `edit_version` are never written by registration (D5). | ANA-16 C4 (`docs/ANA-16.md:484`): "A container or remote host reporting a duplicate hostname merges into another box's row: shared slots, quota and adoption sweep, which resets trees the other box cannot see. Key workers on the `box.toml` id". Adopt-by-hostname is that merge; adopt-by-fingerprint is the same merge for cloned images. |
| D4 | **Migration `0005_box_identity.sql`** (forward-only, `R-STO-5`): `ALTER TABLE box DROP CONSTRAINT box_user_id_hostname_key;` `ALTER TABLE box ADD COLUMN machine_fingerprint TEXT CHECK (machine_fingerprint IS NULL OR machine_fingerprint ~ '^[0-9a-f]{64}$');` `ALTER TABLE box ADD COLUMN edit_version INTEGER NOT NULL DEFAULT 0;` **(amended at maintainer review)** `ALTER TABLE box ADD COLUMN probe_spec_digest TEXT CHECK (probe_spec_digest IS NULL OR probe_spec_digest ~ '^[0-9a-f]{64}$');` (D18), and `COMMENT ON COLUMN` for the three new columns **— four comments in all with `box.htui_version`'s, which break `the_ana_column_comments_are_present_and_verbatim` (`crates/htui-store/tests/migrations.rs:338`): it asserts that exactly the columns in `ANA_COLUMN_COMMENTS` (`:168`) carry a comment on the tables that list names — and it already names `box` (`:313`) ("exactly the twenty-five commented columns, and no others", `:406-409`). T1 therefore adds the four `box` rows to `ANA_COLUMN_COMMENTS`, verbatim as `0005` writes them, and moves the count 25 → 29 in that test's message and its `:380-381` comment** (amended at re-check) and for `box.htui_version` (D5's meaning), with a header in the style of `0004_max_agents_per_run_default.sql`. No new table (`TABLES.len()` stays 33). **No cache migration**: no new column is mirrored (the fingerprint is a registration fact, and `edit_version` and `probe_spec_digest` are read online only), `refresh_box` names its columns (`cache/refresh.rs:583-603`, `:611-620`), and `MIRRORED_TABLES` (`cache/mod.rs:40-58`) is unchanged. Two consequences, both deliberate: every mirror rebuilds once on upgrade because `PgStore::schema_version` moves from 4 to 5 (`CacheStore::open`, `cache/mod.rs:127-150`); and `refresh_box` gains `DELETE FROM box WHERE id <> ?` in its SQLite transaction, because a copied config directory carries a mirror holding the old box and the offline `box_info` reads `FROM box ORDER BY id LIMIT 1` (`cache/read.rs:1102-1106`) — the older UUIDv7 would win. | The constraint name is not guessed: `SELECT conname FROM pg_constraint WHERE conrelid = 'box'::regclass` on `htui_prepare_check` answers `box_user_id_hostname_key`. The `CHECK` makes "only the keyed hash reaches Postgres" a schema fact, not only a code fact. |
| D5 | **`htui_version` is the version at the last probe, and it decides re-probing.** `pub const HTUI_VERSION: &str = env!("CARGO_PKG_VERSION")` in `htui_store` (the crate that writes it today, `pg/mod.rs:371`) is the one definition: registration inserts it, the probe writer records it, and the decision compares with it. ~~`BoxRow::needs_probe(&self, running: &str) -> bool`~~ **(amended at maintainer review)** `BoxRecord::needs_probe(&self, running: &str, spec_digest: &str) -> bool` in `htui-core` = `self.row.last_probed_at.is_none() \|\| self.row.htui_version != running \|\| self.probe_spec_digest.as_deref() != Some(spec_digest)` — on `BoxRecord`, not `BoxRow`, because the digest (D18) is not a `BoxRow` field and D14 keeps every `BoxRow` constructor still. Because registration no longer overwrites the column, a probe that fails or is interrupted after an upgrade is retried at the next `Online` swap. | `R-BOX-2`: "Re-probe on demand and when `htui` version changes"; the column's own comment (`0001_init.sql:64`) already calls it the trigger. The PRD metric "not on every connect" is met by a read, not by a flag in memory. |
| D6 | **Hardware through `sysinfo`, GPU per OS (OQ-2; rewritten at maintainer review).** `htui_agent::box_probe::hardware` returns `Hardware { os_version, cpu, ram_mb: Option<i32>, gpu: Option<GpuVendor> }` behind a thin seam, `pub trait HardwareSource: Send + Sync` with one method returning a boxed future of `Hardware`. **Production, `SystemHardware { pci_root: PathBuf }`:** OS, CPU and RAM come from `sysinfo 0.39.6` (`default-features = false, features = ["system"]`, a direct `htui-agent` dependency): one `System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram()))` inside `tokio::task::spawn_blocking` (sysinfo reads files and calls the OS synchronously) — **the same closure also calls `System::name`, `os_version`, `long_os_version` and `kernel_version`, which read the OS on every call rather than from the `System` value** (amended at re-check); `os_version` = `System::name()` and `System::os_version()` joined by a space on Linux and Windows (`Ubuntu 24.04` here; `Windows 11 (<build>)` shape on Windows), **but `System::long_os_version()` on macOS, where `name()` + `os_version()` would read `Darwin 15.1.1` and `long_os_version()` reads `macOS 15.1.1 Sequoia`** (`sysinfo-0.39.6/src/unix/apple/system.rs:396`, `:400`, `:460`; `src/windows/system.rs:371`, `:375`, `:403`) (amended at re-check); on any platform an empty answer falls back to `long_os_version()`, then `kernel_version()`, else empty; `cpu` = the first `Cpu::brand()`, trimmed, else empty; `ram_mb` = `total_memory()` bytes / 1 048 576, floored, `None` when 0 or above `i32::MAX`. On this box a scratch binary against the same crate and features printed `name=Some("Ubuntu")`, `os_version=Some("24.04")`, `long_os_version=Some("Linux (Ubuntu 24.04)")`, brand `AMD Ryzen 5 7600X 6-Core Processor` (12 CPUs) and `total_memory=66485854208` → `63405` MiB, which is exactly `/proc/meminfo`'s `64927592 kB` floored — so `os_version` here is `Ubuntu 24.04` (the draft's own `/etc/os-release` reader gave `Ubuntu 24.04.5 LTS`). GPU stays ours (D7): Linux scans `<pci_root>/sys/bus/pci/devices` (production `/`); macOS answers `apple` on `aarch64` and otherwise parses `/usr/sbin/system_profiler SPDisplaysDataType`; Windows runs one CIM query for `Win32_VideoController.PNPDeviceID` through `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` and reads `VEN_xxxx`. **Dropped by the amendment:** the `/etc/os-release`, `/proc/cpuinfo` and `/proc/meminfo` readers, the macOS `sw_vers` and `sysctl` children, the Windows registry reads for OS and CPU, and RAM from CIM. **Tests inject facts, not files:** `FixedHardware(Hardware)` implements the seam, so every probe, tag and runtime test hands in the facts it wants; the only fixture trees left are the PCI scans, because that parser is ours and a directory of `class`/`vendor` files is its natural input. The `sysinfo` path itself is exercised by one Linux-only live test (`this_box_reports_real_hardware`). The GPU children run through the probe's existing bounded runner (`run_bounded`, `probe.rs:982-1039`, `VERSION_TIMEOUT` 15 s at `:45`), made `pub(crate)` **together with its result type `OneShot` and the fields `stdout`, `status` and `stderr`** (`probe.rs:960-967`; amended at fact-check: the function alone returns a type `box_probe` could not read, and `private_interfaces` would warn). `run_bounded` resolves its command through `launch::spawn`'s `which` on the **process** `PATH` (`launch.rs:1019-1021`), not `ProbeEnv`'s, so hardware children are named by absolute path, and a tag rule's `ask` runs the path `resolve_tool` returned, never a bare name (amended at fact-check). Every GPU parser is a pure function over captured text, compiled and tested on every platform; only the gathering is `cfg`-gated. Nothing here is fallible to the caller: an unreadable fact is empty or `None`. **`windows-registry` is no longer an `htui-agent` dependency**; it stays in `htui-store` for D1's `MachineGuid`. | The maintainer's answer to OQ-2. `sysinfo` removes three per-OS readers we would have had to maintain and could not run on macOS or Windows here, and it does the Windows RAM read that `std` cannot and that the workspace's `unsafe_code = "forbid"` (`Cargo.toml` `[workspace.lints.rust]`) keeps us from doing through `GlobalMemoryStatusEx`. Cost, resolved in a throwaway copy of the tree under `/tmp` (`cargo add sysinfo@0.39.6 --no-default-features -F system -p htui-agent`): `Cargo.lock` gains `sysinfo 0.39.6`, `ntapi 0.4.3` and `objc2-io-kit 0.3.2` and nothing else; `cargo tree --target x86_64-unknown-linux-gnu -p sysinfo` shows only `libc` and `memchr`; on Windows it depends on the lock's single `windows 0.62.2` (no duplicate) but **turns on 20 more of its features — `Wdk_System_*` and several `Win32_System_*` (among them `Memory`, `Registry` and `Performance`) plus `Win32_UI_Shell`** — and on `ntapi`, which needs **`winapi 0.3.9`, already compiled on Windows (via `crossterm`, `crossterm_winapi` and `findshlibs`); `ntapi` turns on eight more of its features** (amended at re-check); on macOS it newly compiles **`sysinfo`, `objc2-core-foundation` and `objc2-io-kit`** — `objc2-core-foundation` is in the lock but not compiled for macOS today (amended at re-check). MIT; `rust-version = "1.95"` against the workspace's 1.98. The Windows and macOS arms are still written and reviewed by eye (TOOL-3, `HANDOFF.md:599`) and verified by MOD-16 (Windows) and nobody yet (macOS; R-3). |
| D7 | **GPU presence and vendor on Linux without root (the PRD's second open question).** Scan `<root>/sys/bus/pci/devices/*/class` for a display-class value (`0x03xxxx`: VGA `0x0300`, 3D controller `0x0302`, display `0x0380`) and read the sibling `vendor`. Both files are `-r--r--r--` on this box, which reports exactly one device, `class=0x030000 vendor=0x1002`. The vendor id goes through a data map in priority order — `0x10de` nvidia, `0x1002` amd, `0x8086` intel, `0x106b` apple, `0x5143` qualcomm — and the first present vendor in that order wins, so an Intel iGPU beside an NVIDIA card reports `nvidia`. An unmapped vendor (every virtual adapter) is not a GPU (OQ-3). `lspci` (present here) and `nvidia-smi` (absent here) are **not** used: one is a parser of the same files, the other exists only with the proprietary driver. `/sys/class/drm/card0/device/vendor` also answers `0x1002` here but exists only when a DRM driver is bound, so it is not the source. | The same map feeds Windows (`VEN_10DE`) and Intel macOS, so vendor naming is one table. WSL2 exposes its GPU as `/dev/dxg`, not PCI, and reports no GPU (R-4). |
| D8 | **The box probe lives in `htui-agent`, as `crates/htui-agent/src/box_probe/`.** `htui-agent` depends on `htui-core` only (`crates/htui-agent/Cargo.toml` `[dependencies]`), and it already owns everything a probe needs: `ProbeEnv` (`probe.rs:55-70`, its injectable `PATH`, cwd, `versions` flag and per-child timeout), `resolve_tool` (`:354-413`), `capture_version` (`:1049-1065`), `extract_version` (`:928-944`), the bounded runner, `which` and `process-wrap`. `htui-store` must not spawn tools; `htui-core` has no process dependencies and stays pure; `htui` is a binary crate, and MOD-41's headless worker will need the probe without a UI. Entry point: ~~`probe_box(box_id, env, host: &HostRoot, htui_version, now)`~~ **(amended at maintainer review)** `pub async fn probe_box(box_id: BoxId, env: &ProbeEnv, hardware: &dyn HardwareSource, spec: &EffectiveSpec, htui_version: &str, now: DateTime<Utc>) -> BoxProbe`, where `BoxProbe` is the `htui-core` type T0 adds and `EffectiveSpec` is D17's. `sysinfo` joins `htui-agent`, the crate that already spawns and reads the host, for the same reason. **`probe_box` forces `versions = true` on its own copy of the `ProbeEnv`**: `versions` is `false` for `tools::resolve` and the chat re-probe (`probe.rs:65-67`), and under OQ-11 a box probe with versions off would report every versioned tool absent (amended at re-check). | One resolver, two callers — the rule MOD-2 D46 set for agents. |
| D9 | **The tool list and the tag map are one data document; the compiled copy is the seed** (amended at maintainer review — the live spec is this seed overlaid by a stored row, D17) (PRD D2: "The map is data, not a `match` on tool names scattered through code"). `crates/htui-agent/src/box_probe/spec.json`, embedded with `include_str!` and typed by `spec.rs`: `{ "tools": { "<name>": <ToolProbe> }, "tags": [<TagRule>], "gpu_vendors": [{ "pci": "0x10de", "name": "nvidia" }, …] }`. `tools` reuses `launch::ToolProbe` verbatim (`launch.rs:192-230`, `#[serde(tag = "kind", rename_all = "snake_case")]`), so every entry is a `{"kind": "path", "names": [..], "version": {"args": [..], "pattern": ".."}}` like the agent seeds (`crates/htui-core/seeds/agent_claude_cli.json:12-16`); a tool with no `version` is presence-only (`vulkaninfo`, `powershell`) and records `version = ''`, which the prompt renders bare (documented at `crates/htui-core/src/model/box_.rs:152-154`, rendered at `crates/htui-core/src/prompt/render.rs:397-402`). `TagRule { tag, any_tool: Vec<String>, fact: Option<Fact>, ask: Option<Ask> }` with `Fact::Gpu` and `Ask { args, matches }`: the rule fires when a named tool was found **and**, if `ask` is set, running that tool with `args` prints a line matching `matches`. The seeded rules: `rust` ← `rustc` or `cargo`; `cmake` ← `cmake`; `clang` ← `clang`; `msvc` ← `cl`; `mingw` ← `gcc` with `ask {args: ["-dumpmachine"], matches: "mingw"}` (this box answers `x86_64-linux-gnu`); `vcpkg` ← `vcpkg`; `docker` ← `docker` or `podman`; `vulkan` ← `vulkaninfo`; `gpu` ← `Fact::Gpu`. `heavy_build` is declared only. Tags come out sorted and deduplicated. **(Amended at maintainer review, OQ-9, OQ-11, OQ-12.)** *The seed list* grows from sixteen to thirty-nine tools, every one with a stable one-line version answer checked on this box where it is installed: the original sixteen, plus `go` (`go version` → `go version go1.25.7 linux/amd64`), `node` (`v22.19.0`), `npm` (`11.7.0`), `pnpm`, `bun` (`1.0.4`), `deno`, `python3` (names `python3`, `python`; `Python 3.14.7`), `pip` (names `pip3`, `pip`; `pip 26.2.1 from …`), `uv` (`uv 0.12.17 (…)`), `java` (`-version`, on stderr: `openjdk version "17.0.8" …`), `javac` (`javac 17.0.8`), `mvn` (`Apache Maven 3.9.9 (…)`), `dotnet`, `zig` (`zig version`), `make` (`GNU Make 4.3`), `meson`, `msbuild` (Developer-prompt `PATH` only, like `cl`), `git` (`git version 2.43.0`), `git-lfs` (`git-lfs/3.7.1 (…)`), `kubectl` (`version --client` → `Client Version: v1.37.0`), `helm` (`version --short` → `v3.18.4+gd80839c`), `nerdctl` and `qemu-img` (`qemu-img version 8.2.2 (…)`). `gradle` and `bazel` are **left out**: `gradle --version` starts a JVM, initialises `~/.gradle` on first use, and a wrapper downloads a distribution (amended at re-check), and `bazel` is usually `bazelisk`, whose first `--version` downloads a Bazel release — a probe must not touch the network. *Version capture decides presence (OQ-11):* a tool whose entry has a `version` probe counts only when a version is captured, so this box's broken Volta `pnpm` shim (`Volta error: Could not find executable "pnpm"`) is absent, as is a tool that hangs past `VERSION_TIMEOUT`; a presence-only entry counts when found. *Bounded fan-out:* tools resolve concurrently on a `tokio::task::JoinSet` behind an 8-permit `tokio::sync::Semaphore`, so thirty-nine lookups never mean thirty-nine children at once. *New derived tags (OQ-12):* `go` ← `go`, `node` ← `node`, `python` ← `python3`, `java` ← `javac`, `dotnet` ← `dotnet`; no `capability_tag` insert (OQ-12's reasons). *Overlay limits:* a stored overlay (D17) may add only `kind: "path"` tools whose `names` are bare file names (no separator, no `..`); `node_package` and `glob` stay agent-row mechanisms. | A tag is added by editing one JSON block — in the seed by a release, in the store by a user (D17); a test pins the seed's derived vocabulary to `R-BOX-3`'s seeded list minus `heavy_build` **plus OQ-12's five** (amended at maintainer review), **written out in the test** (amended at fact-check: `SEEDED_TAGS`, `pg/mod.rs:27-38`, is a private `htui-store` const and `htui-agent` has no edge to `htui-store`; sharing it would couple T3 to T1 through a new crate edge, so the test carries its own copy with a comment naming both sources). The `ask` form is what keeps "a MinGW `gcc`" out of code without keeping a second `gcc` row in `box_tool`. |
| D10 | **The store write surface.** `WriteStore` (`crates/htui-core/src/store/traits.rs:195`) gains `async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()>` — in one transaction, `UPDATE box SET os_version, cpu, ram_mb, gpu_present, gpu_vendor, probed_tags, htui_version, last_probed_at, probe_spec_digest WHERE id` (the last column amended at maintainer review, D18) (no row → `StoreError::NotFound { entity: "box", .. }`), `DELETE FROM box_tool WHERE box_id`, and one `INSERT … SELECT FROM UNNEST(..)` of the new set with `probed_at = probe.probed_at`; a duplicate tool name is `StoreError::Constraint`. It never writes `hostname`, `declared_tags`, `quirks`, `settings`, `machine_fingerprint`, `edit_version` or `last_seen_at`. And `async fn boxes(&self) -> Result<Vec<BoxRecord>>`, every box of this user with its tools and its recorded spec digest (`BoxRecord { row: BoxRow, tools: Vec<BoxTool>, probe_spec_digest: Option<String> }`; the digest amended at maintainer review — `MemStore` keeps it in a side map keyed by box id, because `BoxRow` does not carry it, D14), boxes ordered by id and tools by `name COLLATE "C"` so Postgres and `MemStore` agree byte for byte. Implemented by `MemStore` (`mem.rs:4322`), `PgStore` (`pg/write.rs:381`), `Writer` (`writer.rs:288`, the `upsert_agent_box` arms at `:341-346` are the mirror), and the two forwarding spies, `UsageSpy` (`crates/htui-agent/src/conformance.rs:674`) and `SpyStore` (`crates/htui-agent/tests/recorder.rs:354`). | The narrow-writer rule of MOD-2 D74 (`traits.rs:253-270`): a machine writer must not be able to overwrite what a human wrote. `boxes` is on `WriteStore` beside the other editor reads (`workspace`, `repos`, `setting`, `traits.rs:395-645`), because `WriteStore` carries no box read today and the conformance case has to read the write back; it is also, unchanged, milestone 2's list. |
| D11 | **The trigger and where it runs.** `AgentRuntime` gains `pub fn on_online(&mut self, backend: &Backend, replies: &UnboundedSender<ReplyEnvelope>)`, called by the store loop right after each `go_online` — the two sites that already call `runs.sweep` (`crates/htui/src/store_worker.rs:1281-1286` after `ApplyMigrations`, `:1568-1575` on `ConnEvent::Online`). It spawns onto `self.background` and returns at once; ~~the task reads `box_info` then `box_row`, and stops silently unless `BoxRow::needs_probe(HTUI_VERSION)`~~ **(amended at maintainer review)** the task reads `box_info` (this box's id), `Backend::app_settings()` (`crates/htui-store/src/backend.rs:396`, the stored `box_probe_spec` if any), builds the effective spec and its digest (D17), reads this box's `BoxRecord` from the writer's `boxes()`, and stops silently unless `BoxRecord::needs_probe(HTUI_VERSION, &digest)`. Nothing runs at loop start, so `Backend::Memory` (`--demo`, every harness) never auto-probes (OQ-10). **The registration probe is opt-in** (amended at fact-check): `AgentRuntime::production()` leaves it off, and only the binary's entry point turns it on — `crates/htui/src/lib.rs:92` changes from `store_worker::spawn(..)` to `store_worker::spawn_with(.., AgentRuntime::production().with_registration_probe())`. Without that, `crates/htui/tests/connection.rs` (which spawns through `store_worker::spawn` at `:122` and reaches `ConnEvent::Online` against real Postgres at `:908` and `:980-1004`) would probe the host's real `PATH` and seeded agents, breaking H-7. **The probe environment is injectable** (amended at fact-check): `AgentRuntime::with_probe_env(ProbeEnv, Arc<dyn HardwareSource>)` (the second argument amended at maintainer review; was `HostRoot`) is used by `on_online` and `ProbeBox`; production defaults to `ProbeEnv::host(cwd)` and `SystemHardware { pci_root: "/" }`, every test injects a fake `PATH` and a `FixedHardware` (amended at maintainer review). `StoreRequest::ProbeBox` (new, served ahead of `try_serve` like `ProbeAgents`) runs the same task with the decision skipped and answers its requester. **The box probe has its own slot** (amended at fact-check): `AgentRuntime.box_probe: Option<JoinHandle<()>>`, like `install` and `auth`. `on_online` and `ProbeBox` first run the finished-task sweep `serve` runs today (`agent_worker.rs:707-720`, factored into `fn sweep_finished(&mut self)` and called from both — without it a finished preview or chat re-probe left in `background` would make `on_online` skip for the whole session), then skip or refuse unless `claim_is_free` (`:1267-1295`). `probe` (`ProbeAgents`, `:900-953`), `install_plan` and `login` also refuse while `box_probe` is held, because `probe` checks only `auth` and `install` today (`:909-923`) and would otherwise write `agent_box` concurrently with the registration task. `ProbeBox` also refuses offline with `REGISTRY_ON_SERVER_ONLY`, exactly as `probe` does (`:926-931`). | `R-NF-3` by construction: the loop awaits nothing, the probe's children are bounded, and the pattern is the one `ProbeAgents` already proves (`Served::Deferred`, `store_worker.rs:1488-1510`). Three startup reads per `Online` swap — `box_info`, `app_settings()` and `boxes()` (amended at re-check) — are the whole cost of "not on every connect". |
| D12 | **The agent half is the existing probe, factored, not copied.** `run_probe` (`agent_worker.rs:1718-1768`) is split into `probe_agents_on(writer, box_id, agents, env) -> Result<Vec<AgentSummary>, StoreError>` (the loop body, unchanged in behaviour: disabled rows skipped, `ProbeOutcome::Kept` leaves a manual row alone per MOD-2 D51, a write failure stops) and the thin `run_probe` that replies. `ProbeArgs.cwd` becomes `env: ProbeEnv` so a test can hand both halves one fake `PATH` (production: `ProbeEnv::host(cwd)`, `probe.rs:99-115`). The box task calls `probe_box`, `record_box_probe`, then `probe_agents_on` for every agent row. | PRD scope: "The same trigger runs the existing agent probe for this box"; the PRD constraint that a probe finding nothing never overwrites a manual row is `ProbeOutcome::Kept`'s job already. |
| D13 | **The offer, not the install (PRD D7, OQ-6).** The task ends with one `StoreReply::BoxProbed(BoxProbeReport)` where `BoxProbeReport { tools: usize, probed_tags: Vec<String>, installable: Vec<String>, agents_failed: Option<String>, spec_error: Option<String> }` (`spec_error` amended at maintainer review: D17's sentence when a stored `box_probe_spec` was ignored, which the status line shows) and `installable` names every agent whose fresh row has `probe.status = "missing"` and whose launch document declares `discovery.install` (`launch.rs:119-126`). The registration trigger has no requester, so the reply goes to `Origin::App` at `UNSOLICITED: Seq = Seq::MAX` — a `seq` `App::dispatch` never issues (it counts up from 0, `app/state.rs:219`), so the freshness gate (`app/update.rs:164-168`) always drops it after `App::observe_reply` (`:238-262`), which runs first and writes the status line. `ProbeBox` replies at its own address. `declares_a_source` (`settings/agents.rs:1302-1307`) moves to `htui_agent::launch::declares_install(&Value) -> bool`, and **the second copy of the rule, `agent_worker.rs:2003` `declares_a_source(&Agent) -> Result<(), StoreError>` called by `install_plan` at `:1038`, delegates to it too** (amended at fact-check), so the section, the install pre-flight and the report use one rule. Nothing sends `InstallPlan` or `InstallConfirm`. | The existing `Settings > Agents` cell already reads `missing` and binds `i` to MOD-20's consented flow; the status line is the one piece of existing UI that can say so without a new view. |
| D14 | **`box.edit_version` for milestone 2 (OQ-4).** Added by `0005`, default 0, commented as "bumped by the declared-tags, quirks and settings editors only; registration and the probe never write it". `BoxRow` does not gain the field in this milestone (every `BoxRow` constructor — `fixtures.rs:417`, `mem.rs`, the `pg/read.rs` `query_as!` — would move for a value nothing reads yet). | The PRD constraint: "Every new box writer is a compare-and-set keyed on a token a reconnect does not bump". Adding it now spares milestone 2 a migration and a second pin move. |
| D15 | **What milestone 1 leaves to milestone 2, by name.** The Settings `box` section; a `StoreRequest` that serves `boxes()`; the declared-tags and quirks writers and their CAS on `edit_version`; `BoxRow.edit_version`; **the `box_probe_spec` editor** (a validated writer of that `app_setting` row, using the same `validate` D17's reader runs, and a view of the effective spec; amended at maintainer review); a `capability_tag`-plus-observed-tags vocabulary for the tag picker (OQ-12); `BoxInfo`'s missing fields (`os_version`, `quirks`, tools). Milestone 1 does expose `StoreRequest::ProbeBox` and `WriteStore::boxes`, because both fall out of this milestone's own needs. | The PRD brief: expose M2's requests only if cheap and natural. |
| D16 | **`R-BOX-2`'s "installed agents" stay in `agent_box`**, not in `box_tool`. The tool list is compilers, build tools, shells, container runtimes and `vulkaninfo`; agents are the agent probe's (D12). | One authority per fact (`R-AGT-6`). |
| D17 | **The live probe spec is data in the store (OQ-9; added at maintainer review).** The compiled `spec.json` (D9) is the **seed**. The live spec is the seed overlaid by the JSON value of the `app_setting` row keyed **`box_probe_spec`** (`app_setting (key TEXT PRIMARY KEY, value JSONB NOT NULL, updated_at …)`, `crates/htui-store/migrations/0001_init.sql:557-561`), read at probe time. **Read path: the existing `Backend::app_settings()`** (`crates/htui-store/src/backend.rs:396`: `MemStore::app_settings` `crates/htui-core/src/store/mem.rs:445`, `PgStore::app_settings` `crates/htui-store/src/pg/read.rs:1295-1301`, offline refused, `backend.rs:596`) — the call `run_worker` already makes for the isolator's copy cap, the engine's settings and the sweep's lease period (`crates/htui/src/run_worker.rs:896`, `:1227`, `:1956`) (amended at re-check). No new `ReadStore`/`WriteStore` method, no `.sqlx` file. **Not `SettingKey`:** `htui_core::prompt::settings::SettingKey` (`crates/htui-core/src/prompt/settings.rs:151`) is a closed enum of ten numeric prompt settings (`ExcerptFileLineCap` … `TokenBudget`, `:153-171`) with rung and range validation, and `set_setting` is that enum's writer; a JSON document with its own schema is neither a prompt setting nor a rung. **Merge, not replace:** the overlay is `{ "tools": { "<name>": <ToolProbe> \| {"disabled": true} }, "tags": [<TagRule> \| {"tag": "<t>", "disabled": true}], "gpu_vendors": [{"pci": "0x….", "name": "…"} \| {"pci": "0x….", "disabled": true}] }`, every key optional. A tool entry replaces the seed's entry of that name or adds one; `disabled` drops it. A tag entry replaces the seed's rule for that tag (in place) or appends; `disabled` drops it. A vendor entry replaces by `pci` id (in place, keeping the priority order) or appends at the lowest priority. So a user who adds `terraform` (amended at re-check) still receives the tools a later `htui` release seeds, which a wholesale replacement would hide from them for good. **Validation, all-or-nothing:** `spec::effective(seed: &Spec, stored: Option<&Value>) -> EffectiveSpec { spec: Spec, digest: String, error: Option<String> }` rejects the whole overlay — and uses the seed alone — when it does not parse, when a tool is not `kind: "path"` with bare `names` (D9's overlay limits), when a version `pattern` or an `ask.matches` does not compile as a `regex`, or when a tag rule names a tool the merged list lacks. `error` is one sentence naming the key and the first fault, carried into `BoxProbeReport.spec_error` and logged with `warn!`; the probe itself never fails on it. All-or-nothing, not per entry, so what was probed is always either exactly the seed or exactly what the user wrote, and the recorded digest says which. **Milestone 1's edit path:** SQL, documented in `spec.rs`'s module doc: `INSERT INTO app_setting (key, value) VALUES ('box_probe_spec', '{"tools": {"terraform": {"kind": "path", "names": ["terraform"], "version": {"args": ["version"], "pattern": "^Terraform v(\\S+)$"}}}}'::jsonb) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value;` then restart `htui` (the next `Online` swap sees a new digest, D18) or send `StoreRequest::ProbeBox`, which milestone 1 exposes but binds to no key. `DELETE FROM app_setting WHERE key = 'box_probe_spec'` returns to the seed, and that too changes the digest and re-probes. | The maintainer's answer to OQ-9 ("make it dynamic"). `app_setting` is the existing place for global tuning, its one reader already exists on every online backend, and `MemStore::set_app_setting` (`mem.rs:461`, tests only) plants a row for tests. The overlay can make every box run a named executable from its own `PATH` with chosen arguments; anyone who can write `app_setting` can already write `agent.launch`, which `htui` spawns — the same trust boundary (R-11). |
| D18 | **An edited spec re-probes: the effective spec's digest is recorded on the box (added at maintainer review).** `digest = htui_core::prompt::digest::sha256_hex(&serde_json::to_string(&effective_spec))` (`crates/htui-core/src/prompt/digest.rs:73`, the workspace's one hasher; the spec serialises deterministically because `tools` is a `BTreeMap` and `tags` and `gpu_vendors` keep their merged order). `BoxProbe` carries it (`spec_digest: String`), `record_box_probe` writes it into **`box.probe_spec_digest`**, a third column of `0005` (D4), and `boxes()` returns it in `BoxRecord` (D10). `BoxRecord::needs_probe` compares it beside `htui_version` (D5). So adding a tool is probed at the next `Online` swap with no version bump, and a reconnect with an unchanged spec still costs reads only. | A column rather than a `box_tool` pseudo-row or a suffix on `htui_version`: the first would show up in the prompt's tool list (`BoxProfile::project`, `box_.rs:175`), the second in its `htui_version` line. On `BoxRecord`, not `BoxRow`, to keep D14's promise that no `BoxRow` constructor moves. **Cost:** T2's queries name a column that exists only after T1's migration, so T2's `cargo sqlx prepare` and its Postgres conformance run need T1 merged — **T2 becomes serial after T1** (see Tasks). |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A probe on a runtime-owned task, the request answered `Deferred` | `AgentRuntime::probe` + `run_probe` | `crates/htui/src/agent_worker.rs:900-953`, `:1718-1768`; served ahead of `try_serve` (`store_worker.rs:961`) at `:1488-1510`, refused without a runtime at `:989-1005` |
| Work that follows every `Online` swap | `runs.sweep(&backend, &tx)` (MOD-4 D190) | `store_worker.rs:1286`, `:1575` |
| The one-claim rule for writers of `agent_box` | `claim_is_free` | `agent_worker.rs:1267-1295` |
| A narrow machine writer that cannot touch human-owned columns | `upsert_agent_box` / `set_agent_box_quota` | `crates/htui-core/src/store/traits.rs:253-310` |
| A redacting `Debug` | `ProbeEnv`, `AgentLaunch` | `crates/htui-agent/src/probe.rs:79`; `crates/htui-agent/src/launch.rs:90-99` |
| Tier-1 probe tests over a fake `PATH` of scripts | `env(tmp)` | `crates/htui-agent/tests/probe.rs:43-68` |
| Harness tests that never spawn a real agent (H-7) | `unresolvable_registry` | `crates/htui/tests/probe.rs:1-35` |
| A migration file header and forward-only note | `0004_max_agents_per_run_default.sql` | `crates/htui-store/migrations/0004_max_agents_per_run_default.sql:1-12` |
| A store conformance case and its two pins | `CASES` | `crates/htui-core/src/store/conformance.rs:37-91`; `crates/htui-core/tests/mem_store.rs:36-37`; `crates/htui-store/tests/pg_conformance.rs:19` |
| A reply every origin's status line reads | `App::observe_reply` | `crates/htui/src/app/update.rs:238-262` |
| Writing an id the database chose back to `box.toml` | `try_connect` | `crates/htui-store/src/connect.rs:432-449` |
| A Postgres test on a throwaway database | `testkit::fresh_db` | `crates/htui-store/src/testkit.rs:152` |
| The per-box SQLite mirror upsert | `refresh_box` | `crates/htui-store/src/cache/refresh.rs:605-652` |
| A global tuning value read from `app_setting` at the moment it is needed (amended at maintainer review, D17) | `Backend::app_settings()` in the run runtime: the isolator's copy cap, the engine's settings and the sweep's lease period (amended at re-check) | `crates/htui/src/run_worker.rs:896`, `:1227`, `:1956`; `crates/htui-store/src/backend.rs:396` |
| Planting an `app_setting` row in a test (D17) | `MemStore::set_app_setting` | `crates/htui-core/src/store/mem.rs:461` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `Cargo.toml` | edit | T0 | `hmac = "0.12"`, `windows-registry = "0.6"` and **`sysinfo = { version = "0.39.6", default-features = false, features = ["system"] }`** (amended at maintainer review) in `[workspace.dependencies]` |
| `Cargo.lock` | regenerate | T0 | the new edges; **three new packages, `sysinfo 0.39.6`, `ntapi 0.4.3`, `objc2-io-kit 0.3.2`** (amended at maintainer review; on Linux only `sysinfo` itself is newly compiled) |
| `crates/htui-store/Cargo.toml` | edit | T0 | `hmac`; `[target.'cfg(windows)'.dependencies] windows-registry` |
| `crates/htui-agent/Cargo.toml` | edit | T0 | ~~`windows-registry`~~ **`sysinfo = { workspace = true }`** (amended at maintainer review; wording amended at re-check — `htui-agent` never named `windows-registry`, and D6 does not read the registry from `htui-agent`) |
| `crates/htui-core/src/model/box_.rs` | edit | T0 | `BoxProbe` (with `spec_digest`), `ProbedTool`, `BoxRecord` (with `probe_spec_digest`), `BoxRecord::needs_probe` + unit tests (D5, D10, D18; the digest and the move off `BoxRow` amended at maintainer review) |
| `crates/htui-core/src/model/mod.rs` | edit | T0 | re-export the three types (`:99-101`) |
| `crates/htui-store/migrations/0005_box_identity.sql` | create | T1 | D4, with D18's `probe_spec_digest` (amended at maintainer review) |
| `crates/htui-store/src/identity.rs` | edit | T1 | `Fingerprint`, `machine_fingerprint`, the per-OS readers, unit tests; module doc `:1-5` corrected (D1) |
| `crates/htui-store/src/pg/mod.rs` | edit | T1 | `register_box` transaction, `Registration`, `HTUI_VERSION`, `bootstrap`, `registration()`; the docs at `:337-351` and `connect_with`'s adopt-DB-id paragraph `:104-108` rewritten (D2, D3, D5; the second amended at fact-check). `identity.rs:96`'s "used by the adopt-DB-id rule" is reworded in the same task |
| `crates/htui-store/src/connect.rs` | edit | T1 | `try_connect`'s log line for `Copied`; the `adopted` wording at `:251`, `:331` and the `try_connect` doc `:422-431` (D2; amended at fact-check) |
| `crates/htui-store/src/testkit.rs` | edit | T1 | the `demo_db` doc on the hostname collision (`:163-167`) reworded (amended at fact-check) |
| `crates/htui-store/src/lib.rs` | edit | T1 | re-export `HTUI_VERSION`, `Registration`, `identity::Fingerprint` |
| `crates/htui-store/src/cache/refresh.rs` | edit | T1 | prune other `box` rows in `refresh_box` (D4) |
| `crates/htui-store/src/cache/read.rs` | edit | T1 | the doc at `:1094-1101` ("holds the own `box` row and no other") made true again and says why |
| `crates/htui-store/tests/box_identity.rs` | create | T1 | the hazard first, then D1–D3 on Postgres |
| `crates/htui-store/tests/migrations.rs` | edit | T1 | applied `vec![1, 2, 3, 4, 5]` at `:74` and `:591`, `Pending(5)` (`:623`), `register_box_upserts_and_adopts` (`:1016-1091`) rewritten, a `0005` case; `ANA_COLUMN_COMMENTS` gains the four `0005` comments and `the_ana_column_comments_are_present_and_verbatim` moves 25 → 29 (amended at re-check) |
| `crates/htui-store/tests/connect.rs` | edit | T1 | `Pending(5)` (`:95`) and `pending, 5` with "all five" (`:108-111`, amended at fact-check); a copied `box.toml` is rewritten to the minted id |
| `crates/htui-store/tests/cache.rs` | edit | T1 | the mirror keeps only this box after a mint |
| `crates/htui-store/.sqlx/` | regenerate | T1, T2 | new and removed `query!` hashes (merge rule under Tasks) |
| `crates/htui-core/src/store/traits.rs` | edit | T2 | `record_box_probe`, `boxes` (D10) |
| `crates/htui-core/src/store/mem.rs` | edit | T2 | both methods on `MemStore`, and the per-box spec-digest side map (amended at maintainer review) |
| `crates/htui-core/src/store/conformance.rs` | edit | T2 | three cases; `CASES` 53 → 56 |
| `crates/htui-core/tests/mem_store.rs` | edit | T2 | pin 53 → 56 (`:36-37`) |
| `crates/htui-store/src/pg/write.rs` | edit | T2 | both methods on `PgStore`, including `probe_spec_digest` (amended at maintainer review) |
| `crates/htui-store/src/writer.rs` | edit | T2 | dispatch arms |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T2 | `EXPECTED_CASES` 53 → 56 (`:19`) |
| `crates/htui-agent/src/conformance.rs` | edit | T2 | `UsageSpy` forwards both methods |
| `crates/htui-agent/tests/recorder.rs` | edit | T2 | `SpyStore` forwards both methods |
| `crates/htui-agent/src/box_probe/mod.rs` | create | T3 | `probe_box`, bounded tool resolution, tag derivation (D8, D9; ~~`HostRoot`~~ replaced by D6's `HardwareSource`, amended at maintainer review) |
| `crates/htui-agent/src/box_probe/hardware.rs` | create | T3 | `HardwareSource`, `SystemHardware` over `sysinfo`, `FixedHardware`, the per-OS GPU gathering and its pure parsers (D6, D7; amended at maintainer review) |
| `crates/htui-agent/src/box_probe/spec.rs` | create | T3 | the typed document, the overlay type, `effective` (merge, validate, digest) and the documented SQL edit path (D9, D17, D18; amended at maintainer review) |
| `crates/htui-agent/src/box_probe/spec.json` | create | T3 | the seed: thirty-nine tools, fourteen tag rules, the GPU vendor map (D9; widened at maintainer review) |
| `crates/htui-agent/src/probe.rs` | edit | T3 | `run_bounded` (`:982`), `OneShot` and its fields (`:960-967`) → `pub(crate)` (amended at fact-check) |
| `crates/htui-agent/src/launch.rs` | edit | T3 | `pub fn declares_install(&Value) -> bool` (D13) |
| `crates/htui-agent/src/lib.rs` | edit | T3 | `pub mod box_probe;` (`:92-110`) and re-exports |
| `crates/htui-agent/tests/box_probe.rs` | create | T3 | PCI fixture roots, fake `PATH`, parsers, the seed and D17's overlay cases (amended at maintainer review) |
| `crates/htui-agent/tests/fixtures/box_probe/**` | create | T3 | PCI device trees and captured GPU command outputs only; no `/proc` or `/etc` fixtures since `sysinfo` replaced those readers (amended at maintainer review) |
| `crates/htui/src/agent_worker.rs` | edit | T4 | `on_online`, the `ProbeBox` path, `probe_agents_on`, `ProbeArgs.env`, `BoxProbeReport`, `box_probe` slot, `sweep_finished`, `with_registration_probe`, `with_probe_env`, `probe`/`install_plan`/`login` refusal while the slot is held, `declares_a_source` at `:2003` delegating (D11–D13; the last five amended at fact-check) |
| `crates/htui/src/lib.rs` | edit | T4 | `:92` spawns with `AgentRuntime::production().with_registration_probe()` (D11, amended at fact-check) |
| `crates/htui/src/store_worker.rs` | edit | T4 | `StoreRequest::ProbeBox`, `StoreReply::BoxProbed`, `UNSOLICITED`, `name()` arm (`:543`-area), `try_serve` refusal arm (`:989-1004`), loop routing (`:1488-1510`), `on_online` at the two swap sites; `BoxInfo`'s doc "No probe: that is MOD-7" (`:92`) |
| `crates/htui/src/testkit.rs` | edit | T4 | the harness's runtime-routed list (`:265-280`) gains `ProbeBox` |
| `crates/htui/src/app/update.rs` | edit | T4 | `observe_reply` writes the status line for `BoxProbed` |
| `crates/htui/src/ui/tabs/settings/agents.rs` | edit | T4 | `declares_a_source` (`:1302-1307`) delegates to `htui_agent::launch::declares_install` |
| `crates/htui/tests/box_probe.rs` | create | T4 | harness and runtime cases |
| `crates/htui/tests/box_probe_pg.rs` | create | T5 | the three triggers and the rename end to end on Postgres |

**Not touched, on purpose:** `cache_migrations/` (D4); `MIRRORED_TABLES`; `BoxRow`, `BoxInfo`,
`BoxProfile` and every `BoxRow` constructor (D14, D15); `Identity` and its struct literals (D2);
`crates/htui/src/ui/tabs/settings/mod.rs` and every Settings section other than the one-line
delegation in `agents.rs` (no new section in milestone 1); `htui-orch` (milestone 3);
`repo_box_path` and excerpts (milestone 4); `htui_core::prompt::settings` and every `SettingKey`
(D17 does not use them; amended at maintainer review); `docs/**`, `HANDOFF.md`, the PRD (the main thread
records deviations); `set_agent_box_quota`'s `Option` widening that `traits.rs:285-294` reserves for
"MOD-7" — nothing in this milestone unregisters an agent or carries a quota across boxes.

## Tasks

~~**T0 alone first. Then Wave A: T1 ∥ T2 ∥ T3 …**~~ **(Amended at maintainer review.) T0 alone
first. Then Wave A: the chain T1 → T2 in one lane, and T3 in a second lane in its own git worktree.
T2 starts from T1 merged, because D18's `probe_spec_digest` column exists only in T1's migration
and T2's queries name it. Merge order T1, T2, T3, with the touched crates' gates re-run on the real
tree after each merge. Then T4. Then T5.** Independence is decided by intersecting the file sets below and by build coupling (MOD-4 M5
D105): a red or mid-edit commit in a dependency crate stops every dependent crate compiling, which is
why each parallel task runs in its own worktree.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `Cargo.toml`, `Cargo.lock`, `crates/htui-store/Cargo.toml`, `crates/htui-agent/Cargo.toml`, `crates/htui-core/src/model/box_.rs`, `crates/htui-core/src/model/mod.rs` | first, alone, until green |
| T1 | `crates/htui-store/migrations/0005_box_identity.sql`, `crates/htui-store/src/identity.rs`, `crates/htui-store/src/pg/mod.rs`, `crates/htui-store/src/connect.rs`, `crates/htui-store/src/lib.rs`, `crates/htui-store/src/testkit.rs`, `crates/htui-store/src/cache/refresh.rs`, `crates/htui-store/src/cache/read.rs`, `crates/htui-store/tests/box_identity.rs`, `crates/htui-store/tests/migrations.rs`, `crates/htui-store/tests/connect.rs`, `crates/htui-store/tests/cache.rs`, `crates/htui-store/.sqlx/` | Wave A lane 1, own worktree, merged first |
| T2 | `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/.sqlx/`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | Wave A lane 1, **serial after T1** (amended at maintainer review), merged second |
| T3 | `crates/htui-agent/src/box_probe/mod.rs`, `crates/htui-agent/src/box_probe/hardware.rs`, `crates/htui-agent/src/box_probe/spec.rs`, `crates/htui-agent/src/box_probe/spec.json`, `crates/htui-agent/src/probe.rs`, `crates/htui-agent/src/launch.rs`, `crates/htui-agent/src/lib.rs`, `crates/htui-agent/tests/box_probe.rs`, `crates/htui-agent/tests/fixtures/box_probe/**` | Wave A lane 2, own worktree, parallel with T1 and T2, merged third |
| T4 | `crates/htui/src/agent_worker.rs`, `crates/htui/src/lib.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/testkit.rs`, `crates/htui/src/app/update.rs`, `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/box_probe.rs` | serial, after Wave A |
| T5 | `crates/htui/tests/box_probe_pg.rs` | serial, last |

**Intersections, checked.** T1 ∩ T3 = ∅ (different crates). T2 ∩ T3 = ∅: T2's `htui-agent` files are
`src/conformance.rs` and `tests/recorder.rs`, T3's are `src/box_probe/**`, `src/probe.rs`,
`src/launch.rs`, `src/lib.rs`, `tests/box_probe.rs` and its fixtures. T1 ∩ T2 = `{.sqlx/}` and
nothing else: T1 owns `pg/mod.rs`, `identity.rs`, `connect.rs`, `lib.rs` and `cache/*`, T2 owns
`pg/write.rs` and `writer.rs`. **(Amended at maintainer review.)** The file sets are unchanged, but
T1 ∩ T2 is no longer only a file question: D18 puts `probe_spec_digest` in T1's migration and T2's
`record_box_probe` and `boxes` queries name it, so T2's offline `query!` data and its Postgres
conformance run need a database migrated through T1's `0005`. **T2 is therefore serial after T1**,
and the draft's `.sqlx` merge rule (re-run `cargo sqlx prepare` after merging the second of two
parallel store tasks) is superseded: T2 simply runs `prepare` on a tree that already holds T1.
T3 stays parallel: it reads no store and adds no query — D17's stored spec reaches it as a
`serde_json::Value` that T4 passes in. **Cargo.lock** moves only in T0. **Build coupling:**
T0 adds types and dependencies nothing uses yet (no lint in the workspace flags an unused
dependency: `Cargo.toml` `[workspace.lints.rust]` names `unsafe_code`,
`missing_debug_implementations` and `unused_qualifications` only). T1 changes `register_box`'s
signature and return type, whose only callers are `bootstrap` (`pg/mod.rs:420`) and
`tests/migrations.rs:1029`, `:1040` — all in T1. T2 adds two trait methods and implements them in
all five implementations inside the same task, so its tree compiles alone. T3 adds a module, makes one private function and its result type `pub(crate)`, and (amended at
maintainer review) uses `sysinfo`, which T0 already declared. **No shared case list** moves outside T2; no snapshot moves
at all (T4 adds no rendered frame; the agents-section change is a function body).

Every implementer prompt carries: PRD D0–D7 win over this plan where they disagree; read the tree,
not `graphify-out/` (it does not exist); the raw machine identity is never a struct field, a column,
a log field, a test assertion string or a file; nothing sets `updated_at` by hand; **commit
incrementally** (uncommitted subagent work does not survive the session, and there is no stash on a
shared tree); verify your gate with `--test-threads=1` on the real tree after your merge.

### Task 0: foundations — dependencies and the shared types (D5, D10)
- **Files**: `Cargo.toml`, `Cargo.lock`, `crates/htui-store/Cargo.toml`,
  `crates/htui-agent/Cargo.toml`, `crates/htui-core/src/model/box_.rs`,
  `crates/htui-core/src/model/mod.rs`.
- **Tests first** (`box_.rs` unit tests): `a_box_never_probed_needs_a_probe`,
  `a_box_probed_by_another_version_needs_a_probe`,
  `a_box_probed_at_this_version_needs_none`; **(amended at maintainer review)**
  `a_changed_spec_digest_needs_a_probe`, `a_box_with_no_recorded_digest_needs_a_probe`, and all
  five written against `BoxRecord::needs_probe(running, digest)`.
- **Action**: add `BoxProbe { box_id, os_version, cpu, ram_mb: Option<i32>, gpu_present,
  gpu_vendor: Option<String>, tools: Vec<ProbedTool>, probed_tags: Vec<String>, htui_version,
  spec_digest, probed_at }`, `ProbedTool { name, version, path }`, `BoxRecord { row: BoxRow, tools:
  Vec<BoxTool>, probe_spec_digest: Option<String> }` and `BoxRecord::needs_probe` (amended at
  maintainer review: the digest fields, and the method moved from `BoxRow`); re-export them. Add
  the three workspace dependencies (`hmac`, `windows-registry`, `sysinfo`) and the three manifest
  entries (`htui-store`: `hmac` and `cfg(windows)` `windows-registry`; `htui-agent`: `sysinfo`)
  with a comment each, in the house style (`Cargo.toml`'s MOD-20 comments); the `sysinfo` comment
  names OQ-2's answer and why `default-features = false` (it drops `component`, `disk`, `network`
  and `user`).
  Every new `pub` item carries a doc comment and a `Debug` impl in every task: each lib root
  warns on `missing_docs` (`htui-core/src/lib.rs:9`, `htui-store/src/lib.rs:11`,
  `htui-agent/src/lib.rs:40`, `htui/src/lib.rs:10`) and the workspace warns on
  `missing_debug_implementations` (amended at fact-check).
- **Mirror**: `BoxProfile::project`'s doc and test style (`box_.rs:163-200`, `:246-280`).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`;
  `cargo tree -i hmac@0.12.1` (still one `hmac 0.12.1` in the lock; on Linux it was already
  compiled); `cargo tree --target x86_64-pc-windows-msvc -i windows-registry` (still `0.6.1`, now
  also a direct edge of `htui-store` only). **(Amended at maintainer review.)** `git diff
  Cargo.lock` adds exactly three `[[package]]` entries — `sysinfo 0.39.6`, `ntapi 0.4.3`,
  `objc2-io-kit 0.3.2` — and edits only the dependency lists of `htui-agent` and `htui-store`;
  `grep -c '^name = "windows"$' Cargo.lock` is still `1`;
  `cargo tree --target x86_64-unknown-linux-gnu -p sysinfo -e normal` shows only `libc` and
  `memchr`. Commit boundary: one red commit (the tests), one green.

### Task 1: `htui-store` — the id is the key, the fingerprint checks it (D1–D5)
- **Files**: as tabled.
- **Test first — the hazard, before any code moves**:
  `tests/box_identity.rs::a_hostname_change_keeps_the_box_id_and_registers` — register an
  `Identity` under hostname `A`, then the same id under `B`; assert `Ok`, the same id, one row,
  `hostname = B`. It fails today with `StoreError::Constraint` from `box_pkey` (the SQL-level
  failure is already reproduced, see Summary). Commit it red.
- **Then, still red**: in `tests/box_identity.rs` —
  `a_copied_box_toml_mints_a_new_box_and_leaves_the_old_row` (same id, two fingerprints: `Copied`,
  a new id, two rows, the old row's `hostname`, `machine_fingerprint` and `last_seen_at` byte-equal
  before and after); `a_matching_fingerprint_is_the_same_box`;
  `no_fingerprint_registers_by_id_alone` (stored `Some` + presented `None`, and `None` + `None`);
  `a_first_fingerprint_is_recorded_on_a_row_that_had_none`;
  `another_id_on_the_same_hostname_is_another_box` (C4: two rows, neither adopted);
  `two_first_registrations_of_one_id_both_succeed` (two concurrent `register_box` calls on a fresh
  id through two pool connections: both `Ok`, both `New`/`Known`, one row — D3's insert-first
  order, amended at fact-check);
  `an_id_under_another_user_is_not_adopted` (OQ-8);
  `the_stored_fingerprint_is_the_keyed_hash_never_the_raw_identity` (the column equals D1's known
  answer, and `SELECT count(*) FROM box WHERE row_to_json(box)::text LIKE '%' || $raw || '%'` is
  0); `a_reconnect_leaves_htui_version_and_the_probe_columns_alone` (set `htui_version = '0.0.0'`,
  `last_probed_at = NULL` and a `probe_spec_digest` by SQL, register again, all three unchanged —
  D5, D18; the digest amended at maintainer review).
  `identity.rs` unit tests: `the_fingerprint_is_hmac_sha256_known_answer`,
  `the_fingerprint_ignores_case_and_surrounding_whitespace`, `an_empty_identity_is_no_fingerprint`,
  `debug_never_prints_the_fingerprint`, `linux_reads_machine_id_then_the_dbus_copy` (over a temp
  root), `the_ioreg_line_is_parsed` (captured sample text), and, `cfg(target_os = "linux")`,
  `this_box_has_a_fingerprint_when_machine_id_is_readable`.
  `tests/migrations.rs`: both applied lists (`:74`, `:591`) and `Pending(5)` (`:623`);
  `register_box_upserts_and_adopts`
  becomes `register_box_keys_on_the_id` (its hostname-adopt half is D3's reversal; its
  `box.toml` write-back half stays); new `the_0005_migration_drops_the_hostname_key_and_adds_three_columns`
  (constraint absent from `pg_constraint`, `machine_fingerprint`, `edit_version` and
  `probe_spec_digest` present, both `CHECK`s refuse `'RAW'`; the third column amended at maintainer
  review).
  `the_ana_column_comments_are_present_and_verbatim` (`:338`) gains `0005`'s four `box` comments
  in `ANA_COLUMN_COMMENTS` (`:168`), verbatim, and its count moves 25 → 29 (message `:409`, comment
  `:380-381`) (amended at re-check).
  `tests/migrations.rs:557` doc and the `:586` expect message ("apply 0004") are reworded in the
  same edit. `tests/connect.rs`: `Pending(5)` at `:95` and `pending, 5` / "all five" at
  `:108-111`; `a_copied_box_toml_is_rewritten_to_the_minted_id` (plant a row
  under the throwaway `box.toml` id with a different fingerprint by SQL, `try_connect`, the file now
  holds another id; skipped when `machine_fingerprint()` is `None`). `tests/cache.rs`:
  `a_pass_keeps_only_this_box_in_the_mirror`.
- **Action**: D1–D5 as written. The log line in `try_connect` and every `tracing` field added here
  carries box ids only. Reword every stale adopt-by-hostname text: `pg/mod.rs:104-108`,
  `:337-351`; `identity.rs:1-5`, `:96`; `connect.rs:251`, `:331`, `:422-431`;
  `testkit.rs:163-167` (amended at fact-check).
- **Mirror**: `db_fingerprint` for hashing style (`identity.rs:132-146`); `seed_if_empty_as`'s
  transaction shape (`pg/mod.rs:257`, its `tx.commit()` at `:333`; the body between was not
  re-read, UNVERIFIED).
- **Validate**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres
  cargo test -p htui-store --all-features -- --test-threads=1`; migrate the scratch database
  through `0005`, then `cargo sqlx prepare -- --all-targets --all-features` from `crates/htui-store`
  and `prepare --check`; `grep -rn 'machine-id\|MachineGuid\|IOPlatformUUID' crates/htui-store/src`
  hits only the reader functions and their docs.

### Task 2: the probe write surface on every store (D10)
- **Files**: as tabled.
- **Tests first** (`store::conformance`, run by `mem_store.rs` and `pg_conformance.rs`):
  `record_box_probe_replaces_profile_and_tools` (write a probe with tools `{a, b}`, then one with
  `{b, c}`; `boxes()` shows exactly `{b, c}` with the second probe's versions, every hardware column,
  `probed_tags`, `htui_version`, `last_probed_at` and `BoxRecord.probe_spec_digest` (amended at
  maintainer review) as written, and `hostname`, `declared_tags`,
  `quirks`, `settings` unchanged from the fixture); `record_box_probe_refuses_an_unknown_box`
  (`NotFound`, entity `box`); `boxes_lists_every_box_with_its_tools` (the demo fixture's boxes by
  id, and `box_tools()`'s rows, `fixtures.rs:569`, by name). Pins move 53 → 56 in
  `mem_store.rs:36-37` and `pg_conformance.rs:19` in the same red commit.
- **Action**: the trait methods with docs in the style of `upsert_agent_box`'s; `MemStore` bumps
  the row's `updated_at` as the trigger would; `PgStore` in one transaction; `Writer` dispatch; the
  two spies forward.
- **Mirror**: `upsert_agent_box` in all five places (`traits.rs:270`; `writer.rs:341-346`;
  `conformance.rs:715-716` of `htui-agent`; `tests/recorder.rs:399-400`).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; the Postgres line for
  `htui-store`; `cargo test -p htui-agent --all-features -- --test-threads=1` (the spies compile);
  `cargo sqlx prepare` against a scratch database migrated through T1's `0005`, then
  `prepare --check` (amended at maintainer review: T2 starts from T1 merged, so this is an ordinary
  prepare, not the draft's post-merge re-run).

### Task 3: `htui-agent` — the box probe (D6–D9, D13's helper)
- **Files**: as tabled.
- **Tests first** (`tests/box_probe.rs` and module unit tests):
  `the_spec_parses_and_every_tag_rule_names_a_listed_tool`;
  `the_derived_vocabulary_is_r_box_3_without_heavy_build_plus_oq12` (amended at re-check);
  `tags_from_presence` (table-driven: `cargo` alone → `rust`; `podman` → `docker`; `gcc` whose
  `-dumpmachine` prints `x86_64-w64-mingw32` → `mingw`, and `x86_64-linux-gnu` → none; a GPU →
  `gpu`; `vulkaninfo` → `vulkan`; and (amended at re-check) `go` → `go`, `node` → `node`, `python3` → `python`,
  `javac` → `java` (`java` alone → none), `dotnet` → `dotnet`; output sorted and deduplicated);
  **`the_box_probe_forces_versions_on`** (a `ProbeEnv` with `versions: false` still yields captured
  versions and versioned tools present) (amended at re-check);
  `tools_over_a_fake_path_report_versions_and_skip_the_absent` (`#[cfg(unix)]`, scripts that print
  the captured first lines listed under Validation); ~~`a_hanging_tool_is_bounded` (…found with an
  empty version)~~ **`a_hanging_tool_is_bounded_and_absent`** (a script that sleeps,
  `version_timeout` 200 ms: bounded, and absent under OQ-11); **`a_shim_that_prints_no_version_is_absent`**
  (a script printing `Volta error: …`); **`a_presence_only_tool_counts_when_found`**;
  **`at_most_eight_tools_resolve_at_once`**;
  ~~`linux_hardware_from_a_fixture_root` (os-release, cpuinfo, meminfo…)~~
  **`linux_gpu_from_a_pci_fixture_root`** (one AMD display device → `amd`);
  `a_virtual_display_adapter_is_not_a_gpu` (`0x1234` only);
  `the_vendor_map_prefers_a_discrete_vendor` (`0x8086` and `0x10de` → `nvidia`);
  `a_missing_pci_tree_is_no_gpu_not_an_error`; `macos_and_windows_gpu_parsers` (captured
  `system_profiler` and CIM `PNPDeviceID` text); **`probe_box_takes_its_facts_from_the_hardware_seam`**
  (`FixedHardware` in, the same `os_version`, `cpu`, `ram_mb`, GPU out);
  `declares_install_reads_discovery_install`. **D17's spec (all new):**
  `no_stored_spec_is_the_seed`; `a_stored_tool_is_added`; `a_stored_tool_replaces_the_seeded_one`;
  `a_disabled_tool_and_its_tag_rule_drop_out`; `a_stored_tag_rule_replaces_in_place`;
  `a_stored_vendor_replaces_by_pci_id_and_keeps_priority`;
  `an_unparseable_overlay_falls_back_to_the_seed_with_a_sentence`;
  `a_glob_or_node_package_tool_is_refused` and `a_name_with_a_path_separator_is_refused`;
  `a_bad_version_pattern_is_refused`; `a_tag_rule_naming_an_absent_tool_is_refused`;
  `the_digest_is_stable_and_changes_with_the_spec` (seed twice → equal; seed plus one tool →
  different; an ignored overlay → the seed's digest). And, on Linux only,
  `this_box_reports_real_hardware` (through `SystemHardware`, i.e. `sysinfo`: non-empty
  `os_version` and `cpu`, `ram_mb > 0`; spawns nothing). (Amended at maintainer review: the file
  readers' cases are gone with the readers.)
- **Action**: D6–D9, D17 and D18's digest function as written; `run_bounded` becomes `pub(crate)`;
  `declares_install` added to `launch.rs` with the body of `declares_a_source`. `spec.rs`'s module
  doc carries D17's SQL edit path verbatim (amended at maintainer review).
- **Mirror**: `probe_tools`' loop and its `debug!`/`warn!` style (`probe.rs:430-485`), without its
  override tier (`HTUI_TOOL_<NAME>` is an agent-row mechanism, `probe.rs:437-460`).
- **Validate**: `cargo test -p htui-agent --all-features -- --test-threads=1`;
  `grep -rnE '"(rustc|cargo|gcc|clang|cmake|docker|podman|go|node|npm|python3|java|javac|dotnet|git|kubectl|vulkaninfo)"' crates/htui-agent/src/box_probe/*.rs`
  finds nothing outside `#[cfg(test)]` modules (every tool name lives in `spec.json`; the pattern
  widened at maintainer review to cover the new seed entries).

### Task 4: `htui` — the trigger, the request, the report (D11–D13)
- **Files**: as tabled.
- **Tests first**: `agent_worker.rs` unit tests over `Backend::Memory` with an injected `ProbeEnv`
  and a `FixedHardware` (amended at maintainer review; was a host root), and an unresolvable
  registry (the H-7 rule) —
  `an_online_swap_probes_a_box_that_was_never_probed` (the row is written, `box_tool` holds the
  fake tools, the agent rows read `missing`, one `BoxProbed` at `UNSOLICITED`);
  `an_online_swap_skips_a_box_probed_at_this_version_and_spec` (amended at re-check) (no task, no reply);
  `an_online_swap_reprobes_after_a_version_change`;
  `probe_box_is_refused_offline_before_spawning_anything` (mirror of
  `an_offline_backend_refuses_the_probe_before_spawning_anything`, `agent_worker.rs:5038`);
  `a_missing_agent_with_an_install_source_is_named_and_not_installed` (the report lists it; the
  runtime's install slot stays empty; no `Install` frame is sent);
  `an_install_is_refused_while_the_registration_probe_runs`;
  `a_manual_agent_row_survives_the_registration_probe` (MOD-2 D51);
  `a_finished_background_task_does_not_stop_the_registration_probe` (a completed preview left in
  `background` before the swap; `sweep_finished` clears it);
  `probe_agents_is_refused_while_the_registration_probe_runs`;
  `production_runtime_does_not_auto_probe_without_opt_in` (the `connection.rs` shape: an `Online`
  swap on `AgentRuntime::production()` spawns nothing);
  `the_install_pre_flight_and_the_section_share_declares_install` (amended at fact-check: the last
  four). **(Amended at maintainer review:)** `a_stored_spec_adds_a_tool_and_reprobes_at_the_next_swap`
  (probe, then `MemStore::set_app_setting("box_probe_spec", …)` adding a fake-`PATH` tool, then a
  second swap probes and `box_tool` holds it); `an_unchanged_spec_does_not_reprobe`;
  `removing_the_stored_spec_reprobes_with_the_seed`;
  `an_invalid_stored_spec_probes_the_seed_and_the_report_says_so` (`spec_error` set, the seed's
  digest recorded, the probe otherwise complete).
  `store_worker.rs`: the `name()` of `ProbeBox`; `try_serve` without a runtime refuses it by name;
  the loop answers `BoxInfo` while a probe task is still running (`R-NF-3`).
  `app/update.rs`: `a_box_probed_reply_lands_on_the_status_line_whatever_its_seq`.
  `tests/box_probe.rs` (harness): `probe_box_through_the_shell_reports_on_the_status_line`.
- **Action**: D11–D13. `on_online` is called immediately after the two `go_online` calls, before
  `runs.sweep`, and awaits nothing. **(Amended at maintainer review.)** The task reads
  `backend.app_settings()` once, hands `get("box_probe_spec")` to `spec::effective`, and uses that
  one `EffectiveSpec` for both the decision and the probe, so the digest compared and the digest
  recorded cannot differ; `ProbeBox` does the same without the decision. The status line adds
  `spec_error` when present.
- **Mirror**: `AgentRuntime::probe` and its tests (`agent_worker.rs:900-953`; offline refusal
  `:5038-5086`, single reply at the requester's address `:5088-5130`). The harness test
  `tests/box_probe.rs` builds its runtime with `with_probe_env` over a fake `PATH` and a
  `FixedHardware` (amended at maintainer review; was a fixture root), never the host (H-7; amended
  at fact-check).
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; the
  `x86_64-pc-windows-msvc` clippy line does not run on this box (TOOL-3), so every `cfg(windows)`
  line of T1 and T3 is re-read by the reviewer.

### Task 5: Postgres end to end and the live check (PRD metrics)
- **Files**: `crates/htui/tests/box_probe_pg.rs`.
- **Tests**: over `fresh_db` and an `Online` backend: `the_first_registration_probes_the_box`
  (hardware columns non-default, `last_probed_at` set, `box_tool` rows, `probed_tags`);
  `a_reconnect_at_the_same_version_does_not_probe` (`last_probed_at` byte-equal);
  `a_version_change_reprobes` (`UPDATE box SET htui_version = '0.0.0'`, next swap probes);
  `a_renamed_box_keeps_its_row_and_its_probe` (register under another hostname through
  `register_box`, no second row, the probe columns intact); **`a_stored_spec_change_reprobes`**
  (`INSERT INTO app_setting (key, value) VALUES ('box_probe_spec', …)` by SQL, the next swap probes
  and `box.probe_spec_digest` changes; amended at maintainer review).
- **Validate**: the workspace gate below, then the live check.

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named in the task. **The
first red test of the milestone is T1's `a_hostname_change_keeps_the_box_id_and_registers`.**

**Store conformance** (both stores): three new cases in T2. **Postgres**: T1's nine registration
cases, the `0005` case, the connect write-back and the mirror prune; T5's five end-to-end cases (amended at re-check).
**`htui-agent`**: T3's fixture and parser cases; no case spawns a real tool except through scripts
in a temporary `bin`. **(Amended at maintainer review:)** twelve (amended at re-check) spec-overlay cases (D17, D18), and
hardware facts injected through `FixedHardware` rather than `/proc` fixtures; `sysinfo` itself is
touched by the one Linux live case. **`htui`**: T4's runtime, loop and shell cases.

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| Store `CASES` | 53 | 56 | `crates/htui-core/src/store/conformance.rs:37-91`; `crates/htui-core/tests/mem_store.rs:36-37`; `crates/htui-store/tests/pg_conformance.rs:19` |
| `READ_CASES` | 9 | 9 | `conformance.rs:219-229` |
| Applied migrations | `[1, 2, 3, 4]` | `[1, 2, 3, 4, 5]` | `crates/htui-store/tests/migrations.rs:74` (`migrations_apply_on_a_clean_database`) and `:591` (`the_0004_bump_moves_only_an_untouched_six`, whose `MIGRATOR.run` at `:586` applies everything embedded, so `0005` lands there too) |
| Pending count on a bare database | 4 | 5 | `tests/migrations.rs:623`; `tests/connect.rs:95`, `:108-111` (the second amended at fact-check) |
| `TABLES` | 33 | 33 | `tests/migrations.rs:92-96` |
| Commented columns | 25 | 29 | `ANA_COLUMN_COMMENTS`, `tests/migrations.rs:168`; `the_ana_column_comments_are_present_and_verbatim`, `:338`, message `:409` (amended at re-check) |
| `StoreRequest` variants | 63 | 64 | `crates/htui/src/store_worker.rs:89` (counted: 63, see disagreements) |
| `StoreReply` variants | 34 | 35 | `store_worker.rs:617` |
| `.sqlx` files | 227 | about 233 (T1: −1 +3, T2: +3 for the writer and +2 for `boxes`; the exact figure is recorded at close). D17 adds none: it reuses `PgStore::app_settings`'s existing query (amended at maintainer review) | `crates/htui-store/.sqlx/` |
| `MIRRORED_TABLES` | 17 | 17 | `crates/htui-store/src/cache/mod.rs:40-58` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — A box registered before `0005` has no stored fingerprint; if a *copy* of its `box.toml` connects first after the upgrade, the copy records its fingerprint and the original machine is then the one that mints a new id | Low | Only possible when a copy already exists and connects first; the original's history stays on the old row, visible in milestone 2's list. Logged as `Copied` with both ids |
| **R-2** — Two processes on a machine with a copied `box.toml` connect at the same instant: each mints its own id, the last `box.toml` write wins, one row is orphaned | Low | `FOR UPDATE` serialises the lookup but not the two mints; the orphan is visible and harmless (no runs claim it) |
| **R-3** — The macOS arms (fingerprint and hardware) are verified by nobody: TOOL-3 blocks the Windows lint here, and no HANDOFF item owns a macOS run | Medium | Pure parsers over captured text are tested everywhere; a failed gather is an empty fact, never an error. Recorded for the main thread to route |
| **R-4** — WSL2 and containers report no GPU and often no machine identity | Certain there | `gpu` is declarable in milestone 2; "no fingerprint" is PRD D1's documented "id alone"; MOD-44 owns container boxes |
| **R-5** — `cl` is off `PATH` outside a Developer prompt, so `msvc` is rarely probed | High on Windows | OQ-9; declared tags in milestone 2; `vswhere` is MOD-16's |
| **R-6** — The registration probe spawns ~~about 16~~ up to about 40 short children (thirty-nine seeded tools plus `ask` and GPU children; amended at maintainer review) plus the agent probe's handshakes on first launch, after every upgrade and after every spec edit | Certain | Off the UI task (D11), at most eight at once (D9), each bounded by `VERSION_TIMEOUT`, never on an ordinary reconnect (D5, D18). `gradle` and `bazel` are left out of the seed because they can start a daemon or download (D9) |
| **R-7** — An agents section open while the registration probe runs keeps showing the old rows | Medium | The unsolicited reply is addressed to the shell, not the tab; the section re-reads on activation (`wants_requests`), and the status line names what changed |
| **R-8** — Deviations the main thread must record: the adopt-DB-id rule removed (D3), `htui_version`'s meaning (D5), an integrated GPU counts (D7), `hmac` added (D1), `edit_version` added ahead of its writer (D14); **and (amended at maintainer review) `sysinfo` added (D6), the probe spec as an `app_setting` row (D17), five derived tags outside `R-BOX-3`'s ten (OQ-12), a broken shim counted absent (OQ-11)** | Medium | Listed under "Where the PRD, HANDOFF or tree disagree"; each has a test that fails if the old reading returns |
| **R-9** — `sysinfo` widens the build: three new packages in the lock, 20 more features on the shared `windows 0.62.2` (`Wdk_System_*`, several `Win32_System_*` including `Memory`, `Registry` and `Performance`, and `Win32_UI_Shell`) for every Windows crate that uses it, `ntapi` on Windows, and `sysinfo`, `objc2-core-foundation` and `objc2-io-kit` newly compiled on macOS (added at maintainer review; widened (amended at re-check)) | Certain | `default-features = false, features = ["system"]`; on Linux the only new compiled crate is `sysinfo` itself; the Windows build is MOD-16's to run (TOOL-3). **Resolved (amended at re-check):** `winapi 0.3.9` is already compiled on Windows (via `crossterm`, `crossterm_winapi` and `findshlibs`); `ntapi` turns on eight more of its features |
| **R-10** — `sysinfo`'s `os_version` shape differs by platform and from the draft's (`Ubuntu 24.04` here, where `/etc/os-release` said `Ubuntu 24.04.5 LTS`); on Windows the shape is `Windows 11 (<build>)` and on macOS D6 uses `long_os_version()` (`macOS 15.1.1 Sequoia`), per `sysinfo-0.39.6/src/windows/system.rs:371`, `:375`, `:403` and `src/unix/apple/system.rs:396`, `:400`, `:460` (amended at re-check); neither is run here (added at maintainer review) | Medium | The fallback chain in D6 never yields an error; the prompt renders whatever string is stored; MOD-16 records the Windows value |
| **R-11** — The stored spec (D17) lets whoever can write `app_setting` make every box run an executable from its own `PATH` with chosen arguments at probe time (added at maintainer review) | Low | The same trust boundary as `agent.launch`, which `htui` already spawns; overlays are limited to `kind: "path"` with bare names (no absolute paths, no globs, no npm resolution); every child is bounded by `VERSION_TIMEOUT` and at most eight run at once |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
# from crates/htui-store, against a scratch database migrated through 0005 (the compose `htui`
# database is empty; `htui_prepare_check` is at 0004 today and must be migrated first):
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features
cargo doc --workspace --no-deps --keep-going   # no error beyond the baseline (UNVERIFIED at 1475b17)
grep -rn 'machine_fingerprint\|Fingerprint' crates --include='*.rs' | grep -i 'tracing::\|info!\|warn!\|debug!'   # expect nothing
# amended at maintainer review: the sysinfo footprint
cargo tree --target x86_64-unknown-linux-gnu -p sysinfo -e normal     # sysinfo, libc, memchr only
grep -c '^name = "windows"$' Cargo.lock                                # 1
```

`--test-threads=1` is not optional (the keyring fake is process-wide). The Windows clippy line
(`cargo clippy --target x86_64-pc-windows-msvc …`) dies in `ring`'s build script on this box
(TOOL-3, `HANDOFF.md:599`); the `cfg(windows)` code is reviewed by eye and its runtime is MOD-16's.

**Live check on this box (after T5).** Point `htui` at a fresh scratch database, accept the
migrations, then read the row back with
`docker exec htui-postgres psql -U postgres -d <scratch> -c "SELECT os_version, cpu, ram_mb,
gpu_present, gpu_vendor, probed_tags, last_probed_at FROM box"` and `… "SELECT name, version FROM
box_tool ORDER BY name"`. Expected from today's read-only probes: ~~`Ubuntu 24.04.5 LTS`~~ `Ubuntu 24.04` (from `sysinfo`, amended at maintainer review),
`AMD Ryzen 5 7600X 6-Core Processor`, `63405`, `true`, `amd`; tools `bash 5.2.21`, `cargo 1.98.1`,
`cmake 4.2.2`, `docker 29.8.1`, `gcc 13.3.0`, `ninja 1.13.2`, `rustc 1.98.1`,
`vcpkg 2025-12-16…`, `zsh 5.9` and no `clang`, `cl`, `podman`, `fish`, `pwsh`, `powershell` or
`vulkaninfo`; tags `{cmake, docker, gpu, rust, vcpkg}` (no `mingw`: `gcc -dumpmachine` answers
`x86_64-linux-gnu`). **(Amended at maintainer review, the widened seed:)** also `bun 1.0.4`,
`git 2.43.0`, `git-lfs 3.7.1`, `go 1.25.7`, `helm 3.18.4…`, `java 17.0.8`, `javac 17.0.8`,
`kubectl 1.37.0`, `make 4.3`, `mvn 3.9.9`, `node 22.19.0`, `npm 11.7.0`, `pip 26.2.1`,
`python3 3.14.7`, `qemu-img 8.2.2`, `uv 0.12.17` (each only if it resolves on the `PATH` `htui`
inherits — several live under `~/.volta`, `~/.sdkman`, `~/.gvm` and mise, so a launch from a
desktop entry may see fewer); **no** `pnpm` (the Volta shim prints no version, OQ-11), `deno`,
`dotnet`, `zig`, `meson`, `msbuild` or `nerdctl`; tags additionally `{go, java, node, python}`.
Then insert D17's `box_probe_spec` row adding `terraform` (not in the seed (amended at re-check)) and relaunch: the tool appears and
`probe_spec_digest` changes; delete the row and relaunch: back to the seed. Quit and relaunch: `last_probed_at` is unchanged. Change the hostname in
`box.toml` by hand and relaunch: same id, no error.

## Acceptance

- [ ] A hostname change keeps the box id, updates `hostname`, and raises no error (T1 red first, then
      green; T5 on Postgres).
- [ ] A `box.toml` presented with another machine's fingerprint mints a new box, leaves the old row
      byte-for-byte, and `box.toml` is rewritten to the new id.
- [ ] A different id on the same hostname is a second row; no path adopts by hostname or by
      fingerprint.
- [ ] Only the keyed hash reaches Postgres (schema `CHECK` and a test); the raw identity is in no
      struct field, column, log field or file; `Debug` of `Fingerprint` prints no hex.
- [ ] After first registration on this box: `os_version`, `cpu`, `ram_mb`, `gpu_present`,
      `gpu_vendor`, `last_probed_at`, `box_tool` and `probed_tags` are real (the live check).
- [ ] The probe runs at first registration, after an `htui_version` change and on `ProbeBox`; an
      ordinary reconnect costs three reads (`box_info`, `app_settings()`, `boxes()`) (amended at re-check) and no probe.
- [ ] Agents are probed after the box; a `missing` agent with an install source is named on the
      status line with `Settings > Agents, i`; nothing is fetched or installed.
- [ ] No tool name or tag name appears in `box_probe/*.rs` outside tests.
- [ ] **(Amended at maintainer review.)** OS version, CPU and RAM come from `sysinfo 0.39.6`
      (`system` feature only); `Cargo.lock` gains exactly `sysinfo`, `ntapi` and `objc2-io-kit`,
      and still holds one `windows` version; `htui-agent` does not name `windows-registry` (amended at re-check).
- [ ] **(Amended at maintainer review.)** With no `box_probe_spec` row the seed is probed; a row
      adds, replaces or disables tools, tag rules and vendors by name; an invalid row probes the
      seed and the status line says why; the probe never fails on the spec.
- [ ] **(Amended at maintainer review.)** Editing or deleting `box_probe_spec` re-probes at the
      next `Online` swap without an `htui` version change; an unchanged spec does not.
- [ ] Store `CASES` 56 in all three places; migrations `[1..5]`; `TABLES` 33; `.sqlx` regenerated
      and `prepare --check` clean; no cache migration; `0005` adds three `box` columns
      (the third amended at maintainer review).
- [ ] The workspace gate above is green.

## Where the PRD, HANDOFF or tree disagree

1. **`HANDOFF.md:258-261` says this item "*calls* `htui_agent::install` from its box registration
   and probe hook"**; PRD D7 says registration offers and never installs. This plan follows PRD D7:
   no install request of any kind is sent by the probe (D13).
2. **The brief for this plan counted 62 `StoreRequest` variants at the last close**; the enum at
   `crates/htui/src/store_worker.rs:89` has **63** today (counted by listing every variant line,
   `Workspaces` at `:91` through `RunActions` at `:534`). The pin table above starts from 63.
3. **`register_box`'s doc states the adopt-DB-id rule** (`pg/mod.rs:344-349`) and
   `tests/migrations.rs:1045` pins it; ANA-16 C4 and PRD D1 reverse it (D3).
4. **`identity.rs:3-4` says `box.toml` "is what makes `box.id` survive a hostname change
   (`R-BOX-4`)"** — true of the file, false of the database, where the same change is a duplicate
   primary key today. The doc is corrected in T1.
5. **`cache/read.rs:1096-1097` says the mirror "holds the own `box` row and no other"** — false
   after a copied config directory; D4 makes it true again.
6. **The PRD's re-probe metric reads "Tests driving `register_box` twice with the same and a bumped
   version"**; in this plan registration no longer decides probing (D5, D11), so the same property
   is proved by driving the `Online` hook twice (T4, T5), and T1 proves registration leaves the
   trigger alone.
7. **`R-BOX-2` lists "installed agents" inside the box probe**; they stay in `agent_box` (D16).
8. **PRD D2 derives `vulkan` from `vulkaninfo`, which `R-BOX-2`'s list does not name**; the tool
   list gains it as presence-only so the tag has a recorded fact (D9).
9. **`box.htui_version`'s comment calls it the re-probe trigger** (`0001_init.sql:64`) while
   `register_box` overwrites it per connect (`pg/mod.rs:360`); D5 resolves it in favour of the
   comment and `0005` re-documents it.
10. **`crates/htui/src/store_worker.rs:92` documents `BoxInfo` as "No probe: that is MOD-7"**;
    corrected in T4.
11. **(Amended at maintainer review.) The PRD's scope names three probe triggers** (first
    registration, `htui_version` change, on demand); D18 adds a fourth, an edited probe spec,
    because the maintainer made the spec data (OQ-9).
12. **(Amended at maintainer review.) `R-BOX-3` seeds ten tags; the seed map derives fourteen (nine of the ten, `heavy_build` being declared-only, plus OQ-12's five) (amended at re-check)**
    (OQ-12). `R-BOX-3` also says "Vocabulary is open", and `capability_tag` constrains nothing
    (`TEXT[]` columns with no foreign key, `0001_init.sql:65-66`, `:322`), so no row is added to
    it.
13. **(Amended at maintainer review.) The PRD's constraint "No tool or agent name outside data"**
    is kept and widened: the tool list is now data twice over, a compiled seed and a stored
    overlay (D17), and T3's grep gate covers the new names.

---

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| [store-identity] register_box at crates/htui-store/src/pg/mod.rs:353-377 upserts ON CONFLICT (user_id, hostname) | verified | pg/mod.rs:353 `pub async fn register_box`, :358 `ON CONFLICT (user_id, hostname) DO UPDATE`, Ok(row.id) closes at :377 |
| [store-identity] register_box doc spans :337-351 and states the adopt-DB-id rule at :344-349 | verified | pg/mod.rs:337 'Upserts this box on (user_id, hostname)...', :344 '**Adopt-DB-id rule**...' through :349; :350-351 # Errors |
| [store-identity] htui_version is overwritten on each connect (:360) | verified | pg/mod.rs:360 `htui_version = EXCLUDED.htui_version,` in the DO UPDATE SET |
| [store-identity] env!("CARGO_PKG_VERSION") at pg/mod.rs:371 | verified | Actually at pg/mod.rs:370 (1-line drift) |
| [store-identity] Only callers of register_box are bootstrap (:418-424, call at :420) and tests/migrations.rs:1029, :1040 (whole workspace searched) | verified | grep -rnE 'register_bo[x]' --include='*.rs' over the whole repo (all crates/tests/benches): only pg/mod.rs:353 (def), :420 (bootstrap, fn at :418-424), tests/migrations.rs:1017 (fn name), :1029, :104… |
| [store-identity] bootstrap runs from connect_with :152 | verified | connect_with starts pg/mod.rs:131; :151-152 `if migrations == MigrationState::UpToDate { store.bootstrap().await?; }` |
| [store-identity] bootstrap runs from apply_migrations :195-198 | verified | pg/mod.rs:195 `pub async fn apply_migrations`, :196 MIGRATOR.run, :197 self.bootstrap().await, :198 } |
| [store-identity] seed_if_empty_as transaction at :257 with tx.commit() at :333 | verified | pg/mod.rs:257 fn signature, :258 `self.pool.begin()`, :260 LOCK TABLE app_user IN SHARE ROW EXCLUSIVE MODE, :333 tx.commit() |
| [store-identity] SEEDED_TAGS at pg/mod.rs:27-38 | verified | pg/mod.rs:27 `const SEEDED_TAGS: [&str; 10] = [` ... :38 `];` |
| [store-identity] schema_version change forces the mirror rebuild (cache/mod.rs:127-150 CacheStore::open) | verified | cache/mod.rs:127 `pub async fn open(root, fingerprint, schema_version)`; rebuild = meta.schema_version != schema_version \|\| fingerprint differs, then removes the file set and rewrites meta (through… |
| [store-identity] identity.rs: Identity struct :21-27 with fields box_id, hostname | verified | identity.rs:21 derive, :22 `pub struct Identity {`, :24 box_id: BoxId, :26 hostname: String, :27 } |
| [store-identity] identity.rs module doc :1-5 (:3-4) says box.toml makes box.id survive a hostname change | verified | identity.rs:3 '`box.toml` is what makes `box.id` survive a hostname change (`R-BOX-4`)'; module doc spans :1-5 |
| [store-identity] db_fingerprint at identity.rs:132-146 | verified | fn is identity.rs:132-144 (doc from :126); end line drifts by 2 |
| [store-identity] test a_hostname_change_keeps_the_box_id at identity.rs:190 | verified | identity.rs:190 `fn a_hostname_change_keeps_the_box_id()` |
| [store-identity] Identity struct literals at crates/htui/src/store_worker.rs:2039, :2250 and crates/htui/tests/connection.rs:736 (are there others?) | partial | Those three exist (all hostname "HTUI-TEST" feeding PgStore::lazy). But `grep -rn 'Identity {'` finds 10 more: identity.rs:72, :82, :196, :214; tests/migrations.rs:913, :961, :972, :1023, :1034, :1076 — Amended: D2's rationale should say adding a field breaks 13 literals: the 3 in htui plus 4 in identity.rs and 6 in h… |
| [store-identity] connect.rs try_connect :432-449 writes the adopted id back via identity::store | verified | connect.rs:432 `pub async fn try_connect(`, :439-446 compare adopted.box_id != identity.box_id, tracing::info!, `identity::store(root, adopted)?`, :448 Ok, :449 } |
| [store-identity] tests/migrations.rs applied vec![1,2,3,4] at :74 and :591; MIGRATOR.run at :586 | verified | migrations.rs:74 `vec![1, 2, 3, 4],`; :586 `MIGRATOR.run(&db.pool).await.expect("apply 0004")`; :591 `assert_eq!(applied, vec![1, 2, 3, 4], "0004 ran")`. (pg_criteria.rs:3457 also has vec![1, 2, 3, 4… |
| [store-identity] Pending(4) at tests/migrations.rs:623 | verified | migrations.rs:623 `MigrationState::Pending(4),` (message at :624 says 'four embedded migrations', which also needs updating) |
| [store-identity] TABLES.len()==33 at tests/migrations.rs:92-96 | verified | migrations.rs:93-95 `assert_eq!( TABLES.len(), 33,`, message through ~:97 |
| [store-identity] register_box_upserts_and_adopts at tests/migrations.rs:1016-1091 with the adopt pin at :1045 | verified | #[tokio::test] :1016, fn :1017, ends :1091 (next item #[cfg(feature="demo")] :1093). The pin is `again, first.box_id,` at :1044 with message '(D6 adopt-DB-id)' at :1045. It also asserts one row per h… |
| [store-identity] tests/connect.rs Pending at :95 | verified | htui-store/tests/connect.rs:95 `MigrationState::Pending(4),` |
| [store-identity] MISSED: a second pending-count pin in htui-store/tests/connect.rs is not in the plan | falsified | tests/connect.rs:108-111: `ConnEvent::MigrationsPending(_, pending) => assert_eq!(pending, 4, "all four embedded migrations are waiting ...")`. The plan's pin table (plan :452) and T1 (:340) list only :95 — Amended: Add tests/connect.rs:110 (`pending, 4` becomes 5, and 'all four' in the message) to the 'Pending count … |
| [store-identity] refresh_box at cache/refresh.rs:583-652 names its columns (:583-603, :611-620) | verified | refresh.rs:583-603 const BOX_COLUMNS (19 explicit names), :605 fn refresh_box, :611-620 explicit SELECT column list `FROM box WHERE id = $1`, fn ends :652. The SQLite mirror box table (cache_migratio… |
| [store-identity] cache/read.rs box_info reads FROM box ORDER BY id LIMIT 1 at :1102-1106 | verified | read.rs:1102 `pub async fn box_info`, :1104-1105 `SELECT id, hostname, ... FROM box ORDER BY id LIMIT 1` |
| [store-identity] cache/read.rs doc :1094-1101 says the mirror holds the own box row and no other | verified | read.rs:1096 'The mirror holds the own `box` row and no other (§4.4)' |
| [store-identity] MIRRORED_TABLES at cache/mod.rs:40-58 has 17 entries including box | verified | cache/mod.rs:40 `pub const MIRRORED_TABLES: [&str; 17] = [`, 'box' third, `];` at :58 |
| [store-identity] 0001_init.sql box.htui_version comment at :64 calls it the re-probe trigger | verified | 0001_init.sql:64 `htui_version TEXT NOT NULL, -- re-probe trigger (R-BOX-2)`; UNIQUE (user_id, hostname) at :73 |
| [store-identity] set_updated_at trigger :575-579 covers box | verified | 0001_init.sql:574-580 DO block; :575 ARRAY['app_user','box',...], :578-579 CREATE TRIGGER trg_%s_updated_at BEFORE UPDATE |
| [store-identity] 0004_max_agents_per_run_default.sql header :1-12 | verified | Lines 1-12 are the comment header, :13 blank, :14 the UPDATE |
| [store-identity] testkit::fresh_db at crates/htui-store/src/testkit.rs:152 | verified | testkit.rs:152 `pub async fn fresh_db() -> Option<TestDb>` (bare_db connect at :133, then apply_migrations, which calls bootstrap) |
| [store-identity] The Postgres constraint name for UNIQUE(user_id, hostname) is box_user_id_hostname_key | verified | docker exec htui-postgres psql -d htui_prepare_check: box_os_family_check, box_pkey, box_user_id_hostname_key \| UNIQUE (user_id, hostname), box_user_id_fkey |
| [store-identity] htui_prepare_check is at migration 4 | verified | _sqlx_migrations there contains versions 1 init, 2 agent probe, 3 orchestration, 4 max agents per run default |
| [store-identity] D3: once UNIQUE(user_id,hostname) is dropped, no other code path looks up a box by hostname or assumes it is unique | verified | grep over crates for hostname in WHERE/JOIN/ON CONFLICT: the only by-hostname use outside register_box is the test count at migrations.rs:1054, which the plan rewrites. Run joins go by id (pg/read.rs… |
| [store-identity] htui-store Cargo.toml already has zeroize and tokio with the sync (OnceCell) and process features that D1 machine_fingerprint needs | verified | crates/htui-store/Cargo.toml:24 `zeroize = { workspace = true }`, :34 `tokio = { workspace = true }`. Workspace Cargo.toml:33-34 tokio features include "sync", "process", "time", "fs". The T0 manifes… |
| [store-identity] MISSED: other docs that state the adopt-DB-id / hostname-adoption rule are not in the plan's rewrite list | falsified | pg/mod.rs:104-108 (connect_with doc: '**Adopt-DB-id rule (plan D6)**: when this hostname already has a `box` row under a different id, the database's id wins'); identity.rs:96 ('used by the adopt-DB-id rule of register_box'); connect.rs:422-431 (try_connect doc 'when this hostname already had a box row under another i… |
| [store-identity] MISSED: D3 case (a) is racy. SELECT ... FOR UPDATE locks no row when the id is absent, so two first connects with the same fresh box.toml id both INSERT | falsified | Under READ COMMITTED, `SELECT ... WHERE id=$1 FOR UPDATE` on a missing row takes no lock. Two htui processes on one box launching together, both at first registration or both after an upgrade that emptied nothing, both take branch (a), and the second INSERT fails with box_pkey. The current ON CONFLICT upsert tolerates… |
| [deps-toolchain] hmac 0.12.1 is in Cargo.lock at ~:2892 | verified | Cargo.lock:2892 name = "hmac", :2893 version = "0.12.1", deps ["digest 0.10.7"]. A second hmac 0.13.0 (digest 0.11.3) is at :2901, so plan references must say hmac@0.12.1 (the plan's `cargo tree -i h… |
| [deps-toolchain] hmac 0.12.1 is pulled by dbus-secret-service on Linux | verified | `cargo tree -i hmac@0.12.1`: hmac v0.12.1 <- hkdf v0.12.4 <- dbus-secret-service v4.1.0 <- keyring v3.6.3 <- htui-store. The link is indirect, through hkdf. |
| [deps-toolchain] hmac 0.12 is already compiled on this box | verified | target/debug/deps contains libhmac-167fe0287ed5f0b0.rlib/.rmeta and other hmac-* artifacts. |
| [deps-toolchain] hmac 0.12 shares digest 0.10 with workspace sha2 0.10 (Cargo.toml [workspace.dependencies]) | verified | Cargo.toml [workspace.dependencies] has sha2 = "0.10" (the manifest's line 32 by grep offset). Cargo.lock:5786-5794 sha2 0.10.9 depends on digest 0.10.7. hmac 0.12.1 also depends on digest 0.10.7. |
| [deps-toolchain] windows-registry 0.6.1 is in Cargo.lock | verified | Cargo.lock:7411-7412 name = "windows-registry" version = "0.6.1" |
| [deps-toolchain] cargo tree --target x86_64-pc-windows-msvc -i windows-registry shows it under reqwest/hyper-util for both htui-agent and htui-store | verified | Output: windows-registry v0.6.1 <- hyper-util v0.1.20 <- hyper-rustls <- reqwest v0.13.5 <- htui-agent (direct) and <- qdrant-client <- htui-store. hyper-util's dep is `[target.cfg(windows)] windows-… |
| [deps-toolchain] windows-registry 0.6.1 API: LOCAL_MACHINE at lib.rs:64 | verified | src/lib.rs:64 `pub const LOCAL_MACHINE: &Key = &Key(HKEY_LOCAL_MACHINE);` |
| [deps-toolchain] windows-registry 0.6.1 API: Key::open at key.rs:15 | verified | src/key.rs:15 `pub fn open<T: AsRef<str>>(&self, path: T) -> Result<Self>` (read-only open via options().read()) |
| [deps-toolchain] windows-registry 0.6.1 API: Key::get_string at key.rs:149 | verified | src/key.rs:149 `pub fn get_string<T: AsRef<str>>(&self, name: T) -> Result<String>`. It goes through TryFrom<Value> for String (value.rs:73), which accepts REG_SZ/REG_EXPAND_SZ and fits MachineGuid, … |
| [deps-toolchain] windows-registry 0.6.1 has get_u32 usable for CurrentMajorVersionNumber | verified | src/key.rs:137 `pub fn get_u32<T: AsRef<str>>(&self, name: T) -> Result<u32>` calls get_u64 and try_into, so it works on a REG_DWORD. |
| [deps-toolchain] sysinfo is not in Cargo.lock | verified | No `name = "sysinfo"` in Cargo.lock. |
| [deps-toolchain] No system-information crate in any manifest | verified | None of sysinfo/systemstat/heim/sys-info/machine-uid/os_info/wmi appear in Cargo.toml or crates/*/Cargo.toml. Note: os_info 3.15.0 is in Cargo.lock:4407, but only transitively through sentry-contexts… |
| [deps-toolchain] [workspace.lints.rust] contains unsafe_code = "forbid" and only missing_debug_implementations and unused_qualifications besides | verified | Cargo.toml [workspace.lints.rust]: unsafe_code = "forbid", missing_debug_implementations = "warn", unused_qualifications = "warn". Also [workspace.lints.rustdoc] (broken_intra_doc_links, private_intr… |
| [deps-toolchain] No configured lint flags an unused dependency or an unused pub type (e.g. unused_crate_dependencies) | partial | unused_crate_dependencies is not configured in any Cargo.toml, crate root or .cargo/config.toml (the config only sets SQLX_OFFLINE), and clippy::all has no unused-dep lint. dead_code does not fire on pub items in the lib crates. However, every lib root carries `#![warn(missing_docs)]` (htui-agent/src/lib.rs:40, htui-c… |
| [deps-toolchain] zeroize is available as a workspace dependency | verified | Cargo.toml [workspace.dependencies] zeroize = "1.9". htui-store/Cargo.toml:24 already has zeroize = { workspace = true }. |
| [deps-toolchain] tokio workspace features include sync and process | verified | Cargo.toml:33-34 tokio features = ["rt-multi-thread", "sync", "macros", "time", "process", "io-util", "fs"]. htui-store uses tokio = { workspace = true }, so tokio::sync::OnceCell and tokio::process … |
| [deps-toolchain] HMAC-SHA256(key="0123456789abcdef0123456789abcdef", msg="htui/box-fingerprint/v1") = a2e8a0ed177cf8b655e4a8c2d16f745209325966a8339fd26e7c6437dae364df | verified | python3 -c "import hmac,hashlib;print(hmac.new(b'0123456789abcdef0123456789abcdef',b'htui/box-fingerprint/v1',hashlib.sha256).hexdigest())" -> a2e8a0ed177cf8b655e4a8c2d16f745209325966a8339fd26e7c6437… |
| [deps-toolchain] The construction is systemd's documented construction (machine-id(5), sd_id128_get_machine_app_specific), so the confidentiality argument is systemd's | partial | Plan D1 (line 160). sd_id128_get_machine_app_specific keys HMAC-SHA256 with the 16 binary machine-id bytes, uses the 16-byte app-id as the message, truncates to 128 bits and sets the UUID v4 variant/version bits. The plan keys with the ASCII hex text (32 bytes, and on macOS/Windows a UUID/GUID string rather than a mac… |
| [deps-toolchain] /etc/machine-id perms -r--r--r-- root-owned; /var/lib/dbus/machine-id is a symlink | verified | -r--r--r-- 1 root root 33 /etc/machine-id; lrwxrwxrwx /var/lib/dbus/machine-id -> /etc/machine-id. On this box the fallback is the same file. |
| [deps-toolchain] /etc/os-release PRETTY_NAME = "Ubuntu 24.04.5 LTS" | verified | PRETTY_NAME="Ubuntu 24.04.5 LTS" |
| [deps-toolchain] First model name in /proc/cpuinfo = AMD Ryzen 5 7600X 6-Core Processor | verified | grep -m1 'model name' /proc/cpuinfo -> AMD Ryzen 5 7600X 6-Core Processor |
| [deps-toolchain] MemTotal 64927592 kB giving 63405 MB floor | verified | MemTotal: 64927592 kB; 64927592/1024 = 63405 (integer floor) |
| [deps-toolchain] Exactly one display-class PCI device: 0000:0f:00.0 class 0x030000 vendor 0x1002 (device 0x164e), class/vendor files -r--r--r-- | verified | The loop over /sys/bus/pci/devices/*/class matching 0x03* printed a single line: 0000:0f:00.0 0x030000 0x1002 0x164e. Both class and vendor are -r--r--r-- root root. |
| [deps-toolchain] Tool versions: bash 5.2.21, cargo 1.98.1, cmake 4.2.2, docker 29.8.1, gcc 13.3.0, ninja 1.13.2, rustc 1.98.1, vcpkg 2025-12-16, zsh 5.9 | verified | bash 5.2.21(1)-release; cargo 1.98.1; cmake version 4.2.2; Docker version 29.8.1; gcc 13.3.0-6ubuntu2~24.04.1; ninja 1.13.2; rustc 1.98.1; vcpkg 2025-12-16-44bb3ce...; zsh 5.9 |
| [deps-toolchain] Absent: clang, cl, podman, fish, pwsh, powershell, vulkaninfo, nvidia-smi | verified | `command -v` returned nothing for each of the eight tools (checked in the agent shell PATH). |
| [deps-toolchain] gcc -dumpmachine = x86_64-linux-gnu | verified | gcc -dumpmachine -> x86_64-linux-gnu |
| [deps-toolchain] lspci present | verified | command -v lspci -> /usr/bin/lspci |
| [deps-toolchain] HANDOFF.md near :599 (TOOL-3) says the Windows clippy target dies in the ring build script | verified | HANDOFF.md:599 '- [ ] **TOOL-3 - The Windows lint target cannot be built on this box...'; :600 '`cargo clippy --target x86_64-pc-windows-msvc` dies in `ring`'s build script with `error occurred in cc… |
| [store-trait] WriteStore trait is declared at crates/htui-core/src/store/traits.rs:195 | verified | traits.rs:195 `pub trait WriteStore: ReadStore {`; the trait closes at :1058 |
| [store-trait] upsert_agent_box is at traits.rs:270 | verified | traits.rs:270 `async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()>;` |
| [store-trait] The narrow-writer doc (MOD-2 D74) spans traits.rs:253-310 | verified | The upsert_agent_box doc starts at :253, D74 is cited at :256 and :280, and set_agent_box_quota's signature ends at :306. Lines :307-310 are start_chat_run's doc, so the true end is :306 (4 lines of … |
| [store-trait] traits.rs:285-294 reserves an Option widening of set_agent_box_quota for MOD-7 | verified | :284 has the heading '# Nothing can clear them yet (review L-6)'. :290 says 'MOD-7 is the caller that will' and :292 says the parameters 'become `Option`s'. The doc says the widening would touch 'the… |
| [store-trait] The editor reads (workspace, repos, setting) are on WriteStore at traits.rs:395-645 | verified | workspace :395, repos :490, setting :645. All three sit inside the WriteStore body (:195-1058). |
| [store-trait] WriteStore has no box read today | verified | No method in the WriteStore body (:195-1058) returns a BoxRow or BoxTool. The only box-related methods are workspace_box_paths :436 and repo_box_paths :503, which are path tables. The supertrait Read… |
| [store-trait] WriteStore has exactly five implementations: MemStore, PgStore, Writer, UsageSpy, SpyStore | verified | Searched the whole workspace for 'WriteStore for' with grep -rnE over . excluding target/, and also with Gortex search.text. Exactly five impls: mem.rs:4322 MemStore, pg/write.rs:381 PgStore, writer.… |
| [store-trait] MemStore impl at mem.rs:4322 | verified | mem.rs:4322 `impl WriteStore for MemStore {` |
| [store-trait] PgStore impl at crates/htui-store/src/pg/write.rs:381 | verified | pg/write.rs:381 `impl WriteStore for PgStore {`. Its upsert_agent_box is at :785. |
| [store-trait] Writer impl at crates/htui-store/src/writer.rs:288, with the upsert_agent_box arms at :341-346 | verified | writer.rs:288 `impl WriteStore for Writer {`. :341 is `async fn upsert_agent_box`, the Memory and Online match arms follow, and the fn closes by about :346. |
| [store-trait] UsageSpy impl at crates/htui-agent/src/conformance.rs:674, forwarding upsert_agent_box at :715-716 | verified | conformance.rs:674 `impl<S: WriteStore> WriteStore for UsageSpy<'_, S> {`. :715 is `async fn upsert_agent_box(&self, row: &AgentBox) -> StoreResult<()> {` and :716 is `self.inner.upsert_agent_box(row… |
| [store-trait] SpyStore impl at crates/htui-agent/tests/recorder.rs:354, forwarding at :399-400 | verified | recorder.rs:354 `impl WriteStore for SpyStore {`. :399 is the upsert_agent_box fn and :400 is `self.inner.upsert_agent_box(row).await`. |
| [store-trait] WriteStore methods have no default bodies, so every new method must be implemented in all five impls | verified | In traits.rs:195-1058 no line ends in `{` except the trait header. All 66 fn items end in `;`. pg/write.rs:2285 also says 'the trait has no default bodies'. The plan's approach (T2 implements both me… |
| [store-trait] Store conformance CASES has 53 entries at crates/htui-core/src/store/conformance.rs:37-91 | verified | :37 is `pub const CASES: &[&str] = &[` and :91 is `];`. Counting quoted strings in :37-91 gives 53, with no duplicates. |
| [store-trait] The CASES count is pinned at crates/htui-core/tests/mem_store.rs:36-37 | verified | mem_store.rs:36 has `conformance::CASES.len(),` and :37 has `53,`. The pin's assertion message (:38-44) lists every milestone that added cases. Bumping to 56 should also extend that message with MOD-… |
| [store-trait] The CASES count is pinned at crates/htui-store/tests/pg_conformance.rs:19 (EXPECTED_CASES) | verified | pg_conformance.rs:19 `const EXPECTED_CASES: usize = 53;`, asserted at :24-25. There is no other pin on the htui-core CASES count. The pins at htui-orch/tests/fake_conformance.rs:16 (70) and in htui-a… |
| [store-trait] READ_CASES has 9 entries at conformance.rs:219-229 | verified | :219 is `pub const READ_CASES: &[&str] = &[` and :229 is `];`, with 9 strings between. The pin at mem_store.rs:47-48 is 9. The plan adds no read cases, so this stays unchanged. |
| [store-trait] The fixtures construct a BoxRow at crates/htui-core/src/fixtures.rs:417 | verified | fixtures.rs:417 is `fn boxes() -> Vec<BoxRow> {` and :418 is the `vec![BoxRow {` literal (id ids::BOX, user_id ids::USER, htui_version "0.1.0", last_probed_at Some(epoch())). |
| [store-trait] box_tools() is at fixtures.rs:569 | verified | fixtures.rs:569 `fn box_tools() -> Vec<BoxTool> {`. It builds 4 rows (rustc, cargo, git, cmake with an empty version), deliberately out of name order, all with box_id ids::BOX. |
| [store-trait] BoxRow has the htui_version and last_probed_at fields that BoxRow::needs_probe needs | verified | model/box_.rs:27 BoxRow has `pub htui_version: String` and `pub last_probed_at: Option<DateTime<Utc>>`. The expression `last_probed_at.is_none() \|\| htui_version != running` type-checks. |
| [store-trait] A BoxTool type exists with name, version and path | verified | box_.rs:71 `pub struct BoxTool { box_id: BoxId, name: String, version: String, path: String, probed_at: DateTime<Utc> }`. It also carries box_id and probed_at, which BoxRecord and record_box_probe mu… |
| [store-trait] BoxProfile::project's doc and test sit at box_.rs:163-200 and :246-280 | verified | :163 `impl BoxProfile {` (the project doc is at :169-174, the fn at :175) and :200 closes the impl. :246-280 is the doc and body of the test `profile_omits_ram_gpu_and_quirks_when_absent`. |
| [store-trait] BoxProfile renders a bare tool when the version is empty (box_.rs:152-154) | partial | box_.rs:152-154 is only the field doc on `tools`: 'An empty version renders the name bare.' The render itself is in crates/htui-core/src/prompt/render.rs:397-402 (`if version.is_empty() { name.clone() }`). — Amended: Cite the behaviour as prompt/render.rs:397-402 (box_profile render), or say box_.rs:152-154 documents … |
| [store-trait] model/mod.rs re-exports the box types at :99-101 | verified | model/mod.rs:99-101 `pub use box_::{ BoxInfo, BoxProfile, BoxRow, BoxSettings, BoxTool, DEFAULT_MAX_CONCURRENT_ITEMS, OsFamily, };` |
| [store-trait] MemStore already holds box rows and box_tool rows, so record_box_probe and boxes need no new state | verified | mem.rs State: `boxes: HashMap<BoxId, BoxRow>` at :85, `box_tools: Vec<BoxTool>` at :115, and `this_box` at :87. from_demo loads both at :210 and :229. MemStore::box_profile at :414-425 already joins … |
| [store-trait] A this-user notion exists for boxes() | verified | MemStore::this_user() at mem.rs:190-202 returns Option<UserId> (the earliest user by created_at). PgStore has a `this_user: UserId` field at pg/mod.rs:58 with an accessor `pub const fn this_user` at … |
| [store-trait] .sqlx files are named query-<hash>.json, so T1 and T2 add disjoint file names | verified | crates/htui-store/.sqlx holds 227 files. `grep -vc '^query-[0-9a-f]{64}\.json$'` returns 0, so every file is query-<sha256>.json. It is the only .sqlx directory in the repo. When T1 deletes the old r… |
| [store-trait] MISSED (not a break, a gotcha for T2): conformance.rs's self-test every_cross_referenced_test_name_exists (conformance.rs:7481-7560) scans the module's own doc comments. Any inline-code span with a s… | verified | conformance.rs:7482-7560, PENDING=[] at :7499. Also, case_names_are_unique (:7444-7460) requires the new names to be distinct across CASES and READ_CASES. |
| [agent-probe] probe.rs: VERSION_TIMEOUT at :45 is 15 s | verified | crates/htui-agent/src/probe.rs:45 `pub const VERSION_TIMEOUT: Duration = Duration::from_secs(15);` |
| [agent-probe] ProbeEnv at probe.rs:55-70 has an injectable PATH, cwd, versions flag and per-child timeout | verified | probe.rs:55-70 has the fields cwd, platform, home, vars: BTreeMap<String,String>, versions: bool and version_timeout: Duration. PATH is not a field of its own. It is carried in `vars` (doc :62 says `… |
| [agent-probe] ProbeEnv has a redacting Debug at probe.rs:79 | verified | probe.rs:79 `impl core::fmt::Debug for ProbeEnv` prints `vars_len` rather than the var values (:80-89) |
| [agent-probe] ProbeEnv::host(cwd) spans probe.rs:99-115 | partial | The fn starts at :99 (`pub fn host(cwd: PathBuf) -> Self`). :115 only closes the struct literal. The fn continues with the HTUI_AGENTS_ROOT seeding block and ends at :134 (`env` :133, `}` :134). — Amended: Cite probe.rs:99-134 |
| [agent-probe] resolve_tool spans probe.rs:354-413 | verified | The signature is at :354 and the fn closes at :413 after the Path, NodePackage and Glob arms |
| [agent-probe] resolve_tool can be called with a launch::ToolProbe and a ProbeEnv and no agent row, and it is pub | verified | probe.rs:354 `pub async fn resolve_tool(probe: &ToolProbe, env: &ProbeEnv) -> Result<Option<ToolResolution>>`. `pub mod probe` is at lib.rs:106 and ToolResolution is pub (:226). It takes no agent row… |
| [agent-probe] probe_tools spans probe.rs:430-485 | verified | The signature is at :430. The loop body runs past :470; the plan's end line was not contradicted. |
| [agent-probe] probe_tools has the HTUI_TOOL_<NAME> override tier at probe.rs:437-460 | verified | :437 `let key = env_override_key(name);` through :459 `continue;` and :460 `}` |
| [agent-probe] extract_version spans probe.rs:928-944 | verified | :928 `pub fn extract_version(output: &str, pattern: &str) -> Option<String>`, closing at :944 |
| [agent-probe] run_bounded spans probe.rs:982-1039 and is private | verified | :982 `async fn run_bounded(what: &str, launch: &ResolvedLaunch, env: &ProbeEnv) -> Option<OneShot>` has no pub, and the fn closes at :1039 |
| [agent-probe] capture_version spans probe.rs:1049-1065 | verified | :1049 `pub async fn capture_version(path: &Path, probe: &VersionProbe, env: &ProbeEnv) -> Option<String>`, closing at :1065 |
| [agent-probe] launch.rs: AgentLaunch has a redacting Debug at :90-99 | verified | launch.rs:90-99 `impl core::fmt::Debug for AgentLaunch` prints env through `RedactedEnv(&self.env)` |
| [agent-probe] launch.rs: discovery.install is at :119-126 | verified | launch.rs:119-126 holds the doc comment and `pub install: Option<Install>` inside `pub struct Discovery` (:103) |
| [agent-probe] launch.rs: ToolProbe spans :192-230 | verified | The derive is at :192, the serde attribute at :193, `pub enum ToolProbe` at :194 and the closing brace at :228. The true range is 191-228 (the doc comment is :191), so the drift is 2 lines. |
| [agent-probe] ToolProbe uses serde tag = "kind" and rename_all snake_case | verified | launch.rs:193 `#[serde(tag = "kind", rename_all = "snake_case")]` |
| [agent-probe] ToolProbe has a "path" variant with names and version {args, pattern} | verified | launch.rs:196-203 `Path { names: Vec<String>, version: Option<VersionProbe> }`. VersionProbe (:232-242) has args (serde default), pattern (required) and an optional `min`. |
| [agent-probe] ToolProbe's version is optional, so presence-only tools work | verified | launch.rs:200-202 `#[serde(default, skip_serializing_if = "Option::is_none")] version: Option<VersionProbe>`, doc: "Absent means presence is the whole probe". probe.rs:360-363 then yields version Non… |
| [agent-probe] htui-agent lib.rs module list is at :92-110 | verified | lib.rs:92 `pub mod acp;` through :110 `pub mod tools;`. conformance and fake are behind cfg(feature = "test-support"). |
| [agent-probe] htui-agent Cargo.toml depends on htui-core only among workspace crates | verified | crates/htui-agent/Cargo.toml [dependencies]: htui-core is the only htui-* entry |
| [agent-probe] htui-agent Cargo.toml has which, process-wrap and tokio process | verified | `which = { workspace = true }`, `process-wrap = { workspace = true }`, tokio features ["sync","rt","time","process","io-util","fs"] |
| [agent-probe] htui-agent Cargo.toml has a [target.'cfg(windows)'.dependencies] table with a windows entry | verified | `[target.'cfg(windows)'.dependencies]` then `windows = { workspace = true }` |
| [agent-probe] crates/htui-core/seeds/agent_claude_cli.json:12-16 shows a tool probe of that shape | verified | :12 "tools": {, :13 { "kind": "path", "names": ["claude"], :14-15 "version": { "args": ["--version"], "pattern": ... }, :16 } |
| [agent-probe] crates/htui-agent/tests/probe.rs:43-60 is the env(tmp) fake PATH helper | partial | `fn env(tmp: &Path) -> ProbeEnv` starts at :43 and puts PATH=tmp/bin into vars at :48-51. The ProbeEnv literal starts at :60 and runs to :67, and the fn closes at :68 (version_timeout 5 s at :66). — Amended: Cite tests/probe.rs:43-68 |
| [agent-probe] MOD-2 D46 "one resolver, two callers" exists in a plan or decision doc | verified | .claude/plans/mod-2-probe-autodiscovery.plan.md:92 `\| D46 \| **One resolver, two callers.** probe::resolve_tool(&ToolProbe, &Env) ...`. It is also cited in mod-2-probe-autodiscovery.blueprint.md:12 … |
| [agent-probe] htui-core has no process dependencies | verified | crates/htui-core/Cargo.toml [dependencies] lists only uuid, chrono, serde, serde_json, thiserror, sha2 and optional sqlx. There is no tokio, which or process-wrap. |
| [agent-probe] htui-store does not spawn tools today | verified | `grep -rn 'tokio::process\\|std::process::Command\\|process::Command' crates/htui-store/src` returns no matches |
| [agent-probe] htui-store has the tokio process feature available for D1's ioreg child | verified | crates/htui-store/Cargo.toml has `tokio = { workspace = true }`, and workspace Cargo.toml:33-34 gives tokio features ["rt-multi-thread","sync","macros","time","process","io-util","fs"]. `cargo tree -… |
| [agent-probe] MISSED: making run_bounded pub(crate) alone is not enough for box_probe to use it | falsified | run_bounded returns `Option<OneShot>`, and OneShot (probe.rs:960-967) is a module-private struct with private fields stdout, status and stderr. From crate::box_probe the fn would be callable, but its result's fields could not be read, which makes it useless for the sw_vers, sysctl, system_profiler, powershell and `gcc… |
| [agent-probe] MISSED: run_bounded children resolve their command against the process PATH, not ProbeEnv.vars PATH | partial | run_bounded calls launch::spawn (launch.rs:1019-1021), which does `which::which(&command)` on the real process PATH, and ResolvedLaunch.env is empty in capture_version (:1053). A hardware child named bare (`sw_vers`, `sysctl`, `powershell`) therefore ignores an injected fake PATH. It is only testable if the plan passe… |
| [htui-worker] store_worker.rs: StoreRequest enum at :89 has exactly 63 variants (Workspaces :91 .. RunActions :534) | verified | `pub enum StoreRequest` at :89. A Python depth-1 variant count gives 63, first Workspaces :91, last RunActions :534, closing brace :535. The name() match also has 63 arms (:543-:606). |
| [htui-worker] BoxInfo doc :92-93 says 'No probe: that is MOD-7' | verified | store_worker.rs:92 `/// This box's row for the top bar. No probe: that is MOD-7.` and :93 `BoxInfo,`. |
| [htui-worker] StoreRequest::name() match near :543 | verified | `pub const fn name` at :540, `match self {` at :542, `Self::Workspaces => "workspaces"` at :543. The match has no wildcard, so ProbeBox needs an arm. |
| [htui-worker] StoreReply enum at :617 has 34 variants | verified | `pub enum StoreReply` at :617, closing at :778. Depth-1 count is 34: Workspaces..RunActions, Failed (last at :772). |
| [htui-worker] try_serve refusal without a runtime at :989-1004 | verified | The comment is at :984-988. The or-pattern runs PromptPreview :989 .. AuthCancel :1002, and the arm body `StoreReply::Failed{.. "no agent runtime in this build"}` ends at :1005. try_serve has no wild… |
| [htui-worker] try_serve spans :1005-1050 | falsified | `async fn try_serve(backend: &Backend, request: &StoreRequest)` starts at :961 (`Ok(match request {` at :962). Its arms continue past :1069 (RunActions). :1005 is the end of the refusal arm, not the function's start. (The plan text itself only cites :989-1004 for the refusal arm.) — Amended: try_serve is at :961 to ~:… |
| [htui-worker] ProbeAgents served ahead of try_serve with Served::Deferred at :1488-1510 | verified | store_worker.rs:1488-1501 is the or-pattern including `StoreRequest::ProbeAgents` (:1494). `runtime.serve(...)` is at :1502, `Served::Deferred => continue` at :1505, and the arm closes at :1511. The … |
| [htui-worker] go_online followed by runs.sweep at :1281-1286 (after ApplyMigrations) | verified | The `StoreRequest::ApplyMigrations if held.is_some()` arm is at :1273. `go_online(` is at :1281-1284 and `runs.sweep(&backend, &tx);` at :1286. |
| [htui-worker] go_online followed by runs.sweep at :1568-1575 (ConnEvent::Online) | verified | `ConnEvent::Online(pg) => {` at :1565, `go_online(` at :1568-1571, `runs.sweep(&backend, &tx);` at :1575. |
| [htui-worker] go_online defined at :1701 | verified | store_worker.rs:1701 `async fn go_online(backend, pg, refresher, health, projects, base)`. Note that it can return early at :1709-1711 with no mirror, leaving the backend not Online. on_online is cal… |
| [htui-worker] D11: exactly two Online swap sites; first launch reaches Online through :1568-1575 | verified | Every go_online call in the repo: store_worker.rs:1281 and :1568, plus the definition at :1701. Besides go_online, `Backend::Online {` is only built in tests (store_worker.rs:2053, :2263; tests/runs_… |
| [htui-worker] agent_worker.rs: PROBE_TTL at :80 | verified | agent_worker.rs:80 `pub const PROBE_TTL: Duration = Duration::from_secs(24 * 60 * 60);` |
| [htui-worker] AgentRuntime::probe at :900-953 | verified | `async fn probe(` at :900. `self.background.push(tokio::spawn(run_probe(ProbeArgs{..})))` at :945-951, `Ok(Served::Deferred)` at :952, closing brace at :953. |
| [htui-worker] probe's offline refusal with REGISTRY_ON_SERVER_ONLY at :926-931 | verified | :926-928 `let writer = backend.writer().ok_or_else(\|\| StoreError::Unreachable(htui_store::REGISTRY_ON_SERVER_ONLY.to_owned()))?;` and the explanatory comment runs to :931. Auth and install checks c… |
| [htui-worker] claim_is_free at :1267-1295 | verified | `fn claim_is_free(&self) -> Result<(), StoreError>` runs :1267-1295. It refuses on auth (:1271), install (:1277) and `!self.background.is_empty()` (:1289). |
| [htui-worker] run_probe at :1718-1768 with ProbeArgs.cwd | verified | `async fn run_probe(args: ProbeArgs)` runs :1718-1768, and `ProbeEnv::host(cwd)` is at :1726. `struct ProbeArgs` at :1689-1695 has field `cwd: std::path::PathBuf` at :1693. |
| [htui-worker] needs_reprobe at :1791-1801 | verified | `pub fn needs_reprobe(agent: &Agent, on_box: Option<&AgentBox>, now: DateTime<Utc>) -> bool` runs :1791-1801. |
| [htui-worker] self.background exists and probe spawns onto it | verified | The field `background: Vec<JoinHandle<()>>` is at :303. probe pushes onto it at :945. Note that it is not probe-only: PromptPreview tasks (:1006) and chat re-probes (:1497) also go into `background`. |
| [htui-worker] Test an_offline_backend_refuses_the_probe_before_spawning_anything at :5038 (tests :5038-5100) | partial | The doc comment is at :5038 and the `async fn` at :5040. The test ends at :5086. :5088-5100+ is the next test, `the_probe_task_answers_once_at_the_requests_address_with_agents` (:5091). — Amended: Cite :5038-5086 for the offline-refusal test. If the second test is also the mirror, name it: :5088-~5130. |
| [htui-worker] InstallPlan/InstallConfirm arms at :795-810 | verified | In AgentRuntime::serve: `StoreRequest::InstallPlan { agent_id }` at :795-800, `StoreRequest::InstallConfirm { plan }` at :801-809, InstallCancel at :810. serve's match has an `other =>` wildcard (:83… |
| [htui-worker] app/state.rs: dispatch seq counts up from 0 (:219) | verified | state.rs:219 `next_seq: 0,` in App::new. dispatch at :283-296 does `let seq = self.next_seq; self.next_seq += 1;` (:284-285). No other writer was found. |
| [htui-worker] Seq::MAX is available (Seq is newtype or alias; no definition needed) | verified | store_worker.rs:48 `pub type Seq = u64;` is a plain alias, so `Seq::MAX` (u64::MAX) compiles as is. Only the `UNSOLICITED` const itself needs defining. u64::MAX is never reached by dispatch's counter. |
| [htui-worker] BoxInfo dispatched at state.rs:243 | verified | state.rs:243 `self.dispatch(Origin::App, StoreRequest::BoxInfo);` inside App::start (:241-246). |
| [htui-worker] update.rs: observe_reply (:238-262) runs BEFORE the freshness gate (:164-168) | verified | on_reply at :163. :164 `self.observe_reply(&envelope.reply);` comes before :165-168 `if !self.is_fresh(&envelope.origin, envelope.seq) { return; }`. observe_reply runs :238-264 (arms end :262). It al… |
| [htui-worker] An Origin::App reply with unknown seq does not panic or log an error | verified | is_fresh (state.rs:304-308) is a plain `.any()` over `latest` and returns false with no log. on_reply returns silently at :167. The `Action::Error` for Failed (:171-173) and on_app_reply (:176) are b… |
| [htui-worker] settings/agents.rs: declares_a_source at :1302-1307 | verified | agents.rs:1302-1307 `fn declares_a_source(launch: &Value) -> bool { serde_json::from_value::<AgentLaunch>(...).ok().and_then(\|l\| l.discovery).is_some_and(\|d\| d.install.is_some()) }`. |
| [htui-worker] settings/agents.rs: i action at :729-760 | verified | `fn begin_install` runs :729-762. declares_a_source is used at :751 and `ctx.request(StoreRequest::InstallPlan{..})` is at :761. The key binding `KeyCode::Char('i') => self.begin_install(ctx)` is at … |
| [htui-worker] settings/agents.rs: missing cell at :866-893 | verified | `fn on_box_cell` runs :866-895. The `missing \| unauthenticated \| failed` arm is at :886-888. |
| [htui-worker] testkit.rs: the harness's runtime-routed request list at :265-280 includes ProbeAgents | verified | testkit.rs:266-281, in Harness::drive: the tuple pattern lists PromptPreview..AuthCancel with ProbeAgents at :273. The fallback `(request, _) => store_worker::serve(..)` at :334 means a ProbeBox left… |
| [htui-worker] tests/probe.rs:1-35: unresolvable_registry helper (H-7) | verified | The module doc on the H-7 rule is at :1-7 and `async fn unresolvable_registry() -> MemStore` at :17-35. It is private to that integration test. For T4's agent_worker/store_worker unit tests, the in-c… |
| [htui-worker] Is there any StoreRequest/StoreReply variant-count pin test or exhaustive match in tests that T4 must update? | verified | No numeric variant-count pin exists in any test (searched for RunActions/Qdrant matches and REQUEST_NAMES-style lists). The only wildcard-free matches on StoreRequest are StoreRequest::name() (:542) … |
| [htui-worker] MISSED: on_online's claim check runs without the finished-task sweep that AgentRuntime::serve does first | falsified | claim_is_free (agent_worker.rs:1289) refuses whenever `self.background` is non-empty, and `self.install`/`self.auth` are checked too. Finished tasks, installs and logins are only swept at the top of `serve` (:707-720: `background.retain(!is_finished)`, `install.take_if`, `auth.take_if`, `previews.retain`). on_online i… |
| [htui-worker] MISSED: ProbeAgents does not honour the new registration probe, so the two race on agent_box | falsified | AgentRuntime::probe (:900-953) checks only `self.auth` and `self.install` (:909-923). It does not call claim_is_free and does not look at `self.background`. A Settings `r` while the registration task runs probe_agents_on would start a second task writing every agent_box row at the same time. D11 only says 'Both refuse… |
| [htui-worker] MISSED: a second copy of the install-source rule exists in agent_worker.rs, but D13 moves only the settings one | falsified | agent_worker.rs:2003 `fn declares_a_source(agent: &Agent) -> Result<(), StoreError>` parses `AgentLaunch.discovery.install` with the same logic. It is called by AgentRuntime::install_plan at :1038 (gortex usages). The plan (D13, files table :239, T3 :392-393) only moves settings/agents.rs:1302-1307 to `htui_agent::lau… |
| [htui-worker] MISSED (risk): the harness test tests/box_probe.rs could probe the real host's tools | falsified | H-7 (tests/probe.rs:1-7) forbids spawning real tools in `cargo test`. The harness path goes through `runtime.serve` (testkit.rs:283-286), and production run_probe builds `ProbeEnv::host(cwd)` (:1726). Unless AgentRuntime carries an injectable ProbeEnv/host root, `probe_box_through_the_shell_reports_on_the_status_line`… |
| [independence] T1 ∩ T2 = {crates/htui-store/.sqlx/} exactly | verified | Computed from the plan Tasks table (:261-266). T1's htui-store files are migrations/0005, identity.rs, pg/mod.rs, connect.rs, lib.rs, cache/refresh.rs, cache/read.rs, tests/{box_identity,migrations,c… |
| [independence] T1 ∩ T3 = ∅ | verified | T1 is entirely under crates/htui-store and T3 is entirely under crates/htui-agent. |
| [independence] T2 ∩ T3 = ∅ | verified | In htui-agent, T2 owns only src/conformance.rs and tests/recorder.rs. T3 owns src/box_probe/**, src/probe.rs, src/launch.rs, src/lib.rs, tests/box_probe.rs and tests/fixtures/box_probe/**. No overlap… |
| [independence] (a) register_box's only callers are bootstrap (pg/mod.rs:420) and tests/migrations.rs:1029, :1040, all in T1. Nothing outside T1 (including crates/htui) calls it or matches on its return value | verified | Gortex relations.usages on PgStore.register_box returns 3 edges: bootstrap at pg/mod.rs:420 and register_box_upserts_and_adopts at migrations.rs:1027/1038 (the call expressions start at :1029/:1040 p… |
| [independence] (b) T2 adds WriteStore methods and implements them in all five implementations inside the same task, so its tree compiles alone | verified | grep `impl.*\bWriteStore\b` over crates finds exactly five implementors: MemStore mem.rs:4322, PgStore pg/write.rs:381, Writer writer.rs:288, UsageSpy htui-agent/src/conformance.rs:674, SpyStore htui… |
| [independence] (c) T1 needs no T0 changes beyond the Cargo ones T0 lists (zeroize, tokio features) | verified | crates/htui-store/Cargo.toml already has `zeroize = { workspace = true }`, `sha2` and `tokio = { workspace = true }`. The workspace tokio features are rt-multi-thread, sync, macros, time, process, io… |
| [independence] (d) T3 needs nothing from T1 or T2 | verified | htui-agent's [dependencies] has htui-core and no htui-store, and probe_box takes htui_version as a parameter (D8), so it does not need HTUI_VERSION from T1. It needs only BoxProbe/BoxId (T0/htui-core… |
| [independence] (e) T1's pg/mod.rs and T2's pg/write.rs share no item that both would edit | verified | pg/write.rs imports only `crate::error::map_sqlx` and `crate::pg::PgStore` (write.rs:47-48). pg/mod.rs declares `mod write;` (:10) and needs no change for T2. T2 reads existing PgStore accessors, and… |
| [independence] (f) T0's BoxProbe/BoxRecord types are used by T2 and T3, so T0 lands first | verified | Per D10, record_box_probe(&BoxProbe) and boxes() -> Vec<BoxRecord> are T2. Per D8, probe_box returns BoxProbe, which is T3. None of the types exist today: model/mod.rs:99-101 re-exports only BoxInfo,… |
| [independence] (g) T4 needs HTUI_VERSION (T1), BoxRow::needs_probe (T0), record_box_probe (T2) and probe_box (T3), so it runs serially after Wave A | verified | The dependency chain is correct. T4 also needs launch::declares_install (T3). Backend::box_row already exists (htui-store/src/backend.rs:515). The Online swap sites are confirmed at store_worker.rs:1… |
| [independence] (h) No snapshot or count pin in an unlisted file moves (no .sqlx file count test, MIRRORED_TABLES unchanged, StoreRequest count only in T4 files, shared CASES pins only in T2 files) | verified | No test counts .sqlx files. The only `.sqlx` hits in .rs files are comments, and .sqlx currently holds 227 files. The migration pins `Pending(4)` and `vec![1, 2, 3, 4]` appear only at connect.rs:95, … |
| [independence] (i) T1's refresh_box `DELETE FROM box WHERE id <> ?` does not break any existing tests/cache.rs expectation | verified | The only mirror box assertion is tests/cache.rs:178-182, `mirror_count("box") == 1` ("the mirror holds the own box row and no other"), which the prune makes more true. The mirror opens with foreign_k… |
| [independence] No workspace lint or tool flags an unused dependency, so T0 can add deps nothing uses yet. [workspace.lints.rust] names only unsafe_code, missing_debug_implementations and unused_qualifications | verified | Cargo.toml [workspace.lints.rust] has exactly those three. [workspace.lints.rustdoc] has three intra-doc-link lints and [workspace.lints.clippy] has `all` only (no clippy::cargo). There is no .github… |
| [independence] The test at tests/migrations.rs:591 runs MIGRATOR.run, which applies all embedded migrations, so 0005 lands there too | verified | migrations.rs:572-575 is MIGRATOR.run_to(3), then :586 `MIGRATOR.run(&db.pool).await.expect("apply 0004")` applies everything remaining, and the :591 assert `vec![1, 2, 3, 4]` must become [1..5]. The… |
| [independence] MISSED: nothing outside T4's files triggers the registration probe, i.e. only Backend::Memory harnesses exist and none auto-probe | falsified | crates/htui/tests/connection.rs builds its worker with `store_worker::spawn(started, ...)` (connection.rs:122), which is `spawn_with(..., AgentRuntime::production())` (store_worker.rs:1119-1125). set_dsn_goes_online_without_a_restart (connection.rs:908, over common::fresh_db) and the two-database SetDsn case (:980-100… |
| [independence] MISSED: T3's test pins the derived tag vocabulary to SEEDED_TAGS (pg/mod.rs:27-38) | falsified | SEEDED_TAGS is a private `const` in crates/htui-store/src/pg/mod.rs:27 whose only user is :278. htui-agent has no htui-store dependency or dev-dependency (its dev-deps are tokio, tempfile and insta). T3's the_derived_vocabulary_is_r_box_3_without_heavy_build therefore cannot reference it. The pin would silently be a h… |
| [maintainer-review] Heading: the rows below were added at maintainer review (2026-09-25) by the planner; fact-check pass done 2026-09-25 on the working tree at `7c09d39` (38 rows: 22 verified, 12 partial, 4 falsified) | checked | Every planner row re-run independently: sysinfo in a fresh `/tmp/mr-htui-sysinfo` rsync of the tree (`cargo add … --offline`) and a scratch crate `/tmp/mr-sysinfo-check` with `unsafe_code = "forbid"`; tool versions re-run under `bash` (no aliases); tree reads through Gortex and `sed`. No real manifest, lock or source file was edited |
| [maintainer-review] `sysinfo 0.39.6` is MIT with `rust-version = "1.95"` | verified | vendored `sysinfo-0.39.6/Cargo.toml:13` `edition = "2024"`, `:14` `rust-version = "1.95"`, `:34` `license = "MIT"`; workspace MSRV 1.98 |
| [maintainer-review] Adding `sysinfo@0.39.6 --no-default-features -F system` to `htui-agent` adds exactly `sysinfo 0.39.6`, `ntapi 0.4.3`, `objc2-io-kit 0.3.2` to `Cargo.lock` | verified | re-run in `/tmp/mr-htui-sysinfo`: `Locking 3 packages … Adding ntapi v0.4.3, objc2-io-kit v0.3.2, sysinfo v0.39.6`; `diff` of the two locks: three new `[[package]]` blocks and one added line (`"sysinfo"`) in `htui-agent`'s dependency list, nothing else |
| [maintainer-review] On Linux `sysinfo` compiles against `libc` and `memchr` only | verified | `cargo tree --target x86_64-unknown-linux-gnu -p sysinfo -e normal` → `libc v0.2.189`, `memchr v2.8.3`; a before/after diff of the whole Linux normal-edge crate set adds `sysinfo v0.39.6` alone (both already compiled: `libc` via `backtrace`, `memchr` via `futures-util`) |
| [maintainer-review] On Windows `sysinfo` uses the lock's single `windows 0.62.2` (no duplicate) plus `ntapi`, enabling `Wdk_System_*` features | partial | Single `windows 0.62.2` (`Cargo.lock:7328-7329`), no duplicate, and the Windows crate-set diff adds only `ntapi` and `sysinfo`: true. But the feature delta on the shared `windows` is **20 features, not three**: `Wdk`, `Wdk_System`, `Wdk_System_SystemInformation`, `Wdk_System_SystemServices`, `Wdk_System_Threading`, and fifteen `Win32_*` (`Security_Authorization`, `System_Diagnostics_Debug`, `System_Kernel`, `System_Memory`, `System_Performance`, `System_Power`, `System_ProcessStatus`, `System_Registry`, `System_RemoteDesktop`, `System_SystemInformation`, `System_SystemServices`, `UI`, `UI_Shell`) — `cargo tree -e features -i windows@0.62.2 --target x86_64-pc-windows-msvc`, before vs after — Amended: D6 and R-9 should say "switches on twenty more `windows` features (the `Wdk_System_*` family plus `Win32_System_*` memory, registry, performance and `Win32_UI_Shell`) for every Windows crate that uses the shared `windows 0.62.2`" |
| [maintainer-review] `ntapi 0.4.3` depends on `winapi`, which is already one entry in `Cargo.lock` | verified | Traced (was partial): `winapi v0.3.9` is **already compiled on Windows today** — `cargo tree --locked --target x86_64-pc-windows-msvc -i winapi -e normal` → `crossterm v0.29.0` (from `htui` and `ratatui-crossterm`), `crossterm_winapi v0.9.1`, `findshlibs v0.10.2` (from `sentry-debug-images`). `ntapi` adds eight `winapi` features (`cfg`, `evntrace`, `in6addr`, `inaddr`, `minwinbase`, `ntsecapi`, `windef`, `winioctl`: 13 → 21). See the R-9 row below |
| [maintainer-review] On macOS `sysinfo` adds `objc2-io-kit` beside the existing `objc2-core-foundation` | partial | `cargo tree -p sysinfo --target aarch64-apple-darwin --depth 1` is as stated. But `objc2-core-foundation` is **in the lock without being compiled for macOS today**: `cargo tree --locked --target aarch64-apple-darwin -i objc2-core-foundation` prints nothing (its only dependant is `os_info` → `objc2-ui-kit`, an iOS path); the before/after crate-set diff on `aarch64-apple-darwin` and `x86_64-apple-darwin` adds three: `objc2-core-foundation v0.3.2`, `objc2-io-kit v0.3.2`, `sysinfo v0.39.6` — Amended: D6 "on macOS on `objc2-core-foundation` (already present)" → "on macOS it newly compiles `objc2-core-foundation` (already in the lock, not compiled for macOS today) and `objc2-io-kit`"; same fix to OQ-2's paragraph if it is restated there |
| [maintainer-review] `sysinfo` API used by D6 exists in 0.39.6 | verified | vendored `src/common/system.rs`: `new_with_specifics` `:87`, `cpus` `:524`, `total_memory` `:539` (doc: "Returns the RAM size in bytes"), `System::name` `:707`, `kernel_version` `:727`, `os_version` `:748`, `long_os_version` `:768`, `Cpu::brand` `:2877`. Re-compiled with `RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram())`; `brand()` is filled even with `CpuRefreshKind::nothing()` |
| [maintainer-review] `sysinfo` on this box answers `Ubuntu` / `24.04` / `Linux (Ubuntu 24.04)`, brand `AMD Ryzen 5 7600X 6-Core Processor` × 12, `66485854208` bytes = `63405` MiB | verified | `/tmp/mr-sysinfo-check` (`cargo run`): `name=Some("Ubuntu") os_version=Some("24.04") long=Some("Linux (Ubuntu 24.04)") kernel=Some("6.8.0-138-generic")`, `cpus=12 brand=Some("AMD Ryzen 5 7600X 6-Core Processor")`, `total_memory=66485854208 mib=63405`; `/proc/meminfo` `MemTotal: 64927592 kB` → 63405 MiB floored |
| [maintainer-review] `Backend::app_settings()` exists, answers on `Memory` and `Online`, refuses offline | verified | `crates/htui-store/src/backend.rs:396-402` (`Memory` → `store.app_settings()`, `Online` → `pg.app_settings()`, `Offline` → `Err(prompt_offline())`); `prompt_offline` at `:596` returns `StoreError::Unreachable(PROMPT_ON_SERVER_ONLY)` (`writer.rs:126`). `Backend` is `#[derive(Debug, Clone)]` (`backend.rs:48-49`), so the spawned task can own a clone, as `run_worker` does |
| [maintainer-review] `PgStore::app_settings` reads every `app_setting` row; `MemStore::app_settings` and the tests-only `set_app_setting` exist | verified | `pg/read.rs:1295-1301` (`SELECT key, value FROM app_setting ORDER BY key` into a `BTreeMap`); `mem.rs:445` (no fixture rows: empty unless planted, doc `:434-437`), `mem.rs:455-461` `pub fn set_app_setting` — doc-only "Tests only", **not** `cfg`-gated, so `htui`'s tests can call it |
| [maintainer-review] `run_worker` already reads `app_setting` through `Backend::app_settings()` | verified | `crates/htui/src/run_worker.rs:895-898` (`Shared::singletons`, feeds `copy_max_total_bytes` at `:909-912`), `:1227` (`Kit::read`, the map handed to the engine), `:1956` (`sweep_once`, the lease period); also `preview.rs:187`. See the D17 wording row below |
| [maintainer-review] `app_setting` is `(key TEXT PRIMARY KEY, value JSONB NOT NULL, updated_at TIMESTAMPTZ)` | verified | `0001_init.sql:557-561`; `updated_at` is `TIMESTAMPTZ NOT NULL DEFAULT now()`. No `CHECK` on `key`, no enum type, no FK; no later migration alters it (`0002`-`0004` only `INSERT`/`UPDATE` rows) |
| [maintainer-review] `SettingKey` is a closed enum of ten prompt settings, not a home for a JSON document | verified | `crates/htui-core/src/prompt/settings.rs:151` `pub enum SettingKey`, `ExcerptFileLineCap = 0` (`:153`) … `TokenBudget = 9` (`:171`), ten discriminants |
| [maintainer-review] `sha256_hex` is the workspace hasher `htui-agent` can reach through `htui-core` | verified | `crates/htui-core/src/prompt/digest.rs:73`; `pub mod digest` at `prompt/mod.rs:28`, `pub mod prompt` at `htui-core/src/lib.rs:12`, no feature gate; `htui-agent` depends on `htui-core` |
| [maintainer-review] Tag columns have no foreign key to `capability_tag`; nothing in Rust reads that table | verified | `0001_init.sql:43-47` (`capability_tag` has no dependants), `:65-66` `probed_tags`/`declared_tags TEXT[]`, `:322` and `:345` `required_tags TEXT[]` (the second on `item_revision`, not cited by the plan) — none with `REFERENCES`; not in any cache migration. Production Rust only inserts it (`pg/mod.rs:276`); **tests do read it**: `tests/migrations.rs:782` pins `count = 10` and `:827` `bool_and(seeded)` — which confirms OQ-12's "no insert" (an insert would move that pin) |
| [maintainer-review] Version first lines of the widened seed on this box | verified | Re-run under `bash` (zsh aliases `pip`/`uv` to `noglob …`): every captured line matches — `go version go1.25.7 linux/amd64`, `v22.19.0`, `11.7.0`, `1.0.4`, `Python 3.14.7` (and `python` → same), `pip 26.2.1 from …`, `uv 0.12.17 (x86_64-unknown-linux-gnu)`, `openjdk version "17.0.8" 2023-07-18` (stderr; stdout empty — `capture_version` appends the stderr tail, `STDERR_TAIL_LINES = 64`, `launch.rs:37`), `javac 17.0.8`, `Apache Maven 3.9.9 (8e85…)`, `GNU Make 4.3`, `git version 2.43.0`, `git-lfs/3.7.1 (GitHub; …)`, `Client Version: v1.37.0`, `v3.18.4+gd80839c`, `qemu-img version 8.2.2 (Debian …)`; absent `deno`, `dotnet`, `zig`, `meson`, `msbuild`, `nerdctl`, `bazel`. Resolved under `~/.gvm`, `~/.volta`, `~/.bun`, mise, `~/.sdkman`, linuxbrew — the live check's "may see fewer" caveat is right |
| [maintainer-review] A broken shim sits on `PATH` here: `~/.volta/bin/pnpm` prints `Volta error: Could not find executable "pnpm"` | verified | `command -v pnpm` → `/home/mluigi/.volta/bin/pnpm`; `pnpm --version` → `Volta error: Could not find executable "pnpm"` |
| [maintainer-review] `gradle` is a shell alias here, not an executable on `PATH` | verified | zsh: `alias gradle=gradle-or-gradlew`, a function from oh-my-zsh's `gradle` plugin; `bash -c 'command -v gradle'` → nothing; no `~/.sdkman/candidates/gradle`. Gradle is not installed at all, so the D9 rationale for it is unverifiable here (row below) |
| [maintainer-review] T3 needs no store access for D17: the stored spec reaches it as a `serde_json::Value` passed by T4 | verified | T3's file set is `htui-agent` only; `htui-agent/Cargo.toml` `[dependencies]` has no `htui-store`. T3 needs from T0 exactly `BoxProbe` (with `spec_digest`) and `ProbedTool` (`htui-core`) and the `sysinfo` manifest line; nothing from T1 or T2 |
| [maintainer-review] T2 must follow T1 because its queries name `probe_spec_digest` | verified | `.cargo/config.toml` `[env] SQLX_OFFLINE = "true"`, so T2's `query!` in `pg/write.rs` compiles only against `.sqlx` data prepared on a database that has the column; and `pg_conformance.rs` runs on `testkit::fresh_db`, which applies the embedded migrations — `0005` must be in the tree. Genuine dependency, not a precaution |
| [maintainer-review] MISSED: a new `app_setting` key violates no validation, enum or pin | verified | No `CHECK`/enum on `app_setting.key` (above). Every reader looks keys up by name: `prompt::settings::resolve_*` (`settings.rs:640-732`), `htui-orch` `graph.rs:479-489`, `recover.rs:59-78`, `run_worker.rs:909-912`, `connect.rs:46-48`. The Settings prompt section is built from ten `setting(App, SettingKey)` reads (`crates/htui/src/prompt_settings.rs:137`), so an extra row is never listed; `tests/prompt_settings.rs:159`, `:521` pin that snapshot's ten, not the table. `tests/migrations.rs:786-791` pins `count(app_setting) = 24` after seeding, which a row D17 never seeds does not move. Only side effect: `EngineParts`' `Debug` prints the whole map (`htui-orch/src/engine.rs:443`), so a stored `box_probe_spec` would appear in any `?parts` log — not a secret |
| [maintainer-review] MISSED: the probe task can reach `Backend::app_settings()` the way D11 says, and needs no T2 method for it | verified | `on_online` runs only after `go_online`, so the clone is `Online` and `pg.app_settings()` answers; `ProbeBox` refuses offline first (D11). On `Backend::Memory` the map is empty unless `set_app_setting` planted a row, so `--demo` probes the seed. T4 does need T2's `boxes()` for the decision (D11: "reads this box's `BoxRecord` from the writer's `boxes()`"); `backend.writer()` yields `Writer::Memory`/`Online` only (`writer.rs:58-63`), both of which T2 implements |
| [maintainer-review] MISSED: R-9's "whether `winapi` is already compiled on Windows today is UNVERIFIED" | partial | Now answered (see the `ntapi` row): yes, via `crossterm 0.29`, `crossterm_winapi 0.9.1` and `findshlibs 0.10.2` — Amended: R-9 "Whether `winapi` is already compiled … UNVERIFIED" → "`winapi 0.3.9` is already compiled on Windows (`crossterm`, `crossterm_winapi`, `sentry`'s `findshlibs`); `ntapi` switches on eight more of its features"; D6's "(which needs `winapi`, already one entry in the lock)" → "(which needs `winapi`, already compiled on Windows through `crossterm`)" |
| [maintainer-review] MISSED: `sysinfo` needs no `unsafe` on our side and builds under `unsafe_code = "forbid"` | verified | `/tmp/mr-sysinfo-check` with `[lints.rust] unsafe_code = "forbid"` and the D6 calls compiled and ran; the lint is per crate, so `sysinfo`'s internal `unsafe` (e.g. `unix/apple/system.rs:461`) does not trip it. `default-features = false` drops `component`, `disk`, `network`, `user` (`Cargo.toml:73-79`) and `multithread`/`rayon` is not default |
| [maintainer-review] MISSED: D6's `os_version` rule (`System::name()` + `System::os_version()`) on macOS and Windows | partial | Linux gives `Ubuntu 24.04` (verified). macOS: `name()` is `KERN_OSTYPE` with fallback `"Darwin"` (`src/unix/apple/system.rs:396-398`) while `os_version()` is `kern.osproductversion` (`:460-474`), so the join yields **`Darwin 15.1.1`** — the kernel's name with the product's version; `long_os_version()` gives `macOS 15.1.1 Sequoia` (`:400-420`). Windows: `name()` is the literal `"Windows"` (`src/windows/system.rs:371-373`), `os_version()` is `"<major> (<build>)"` (`:403-420`, doc example `10 (20348)`), so the join is `Windows 11 (22631)`-shaped — acceptable — Amended: D6 → "on macOS `System::long_os_version()` (`macOS 15.1.1 Sequoia`), elsewhere `name()` and `os_version()` joined; fallbacks as written"; R-10 cites these two source lines instead of UNVERIFIED for the shape (the runtime value stays MOD-16's) |
| [maintainer-review] MISSED: `sysinfo`'s OS-name functions also read the OS synchronously | partial | `System::name`, `os_version`, `long_os_version`, `kernel_version` are associated functions documented "**computed every time this function is called**" (`src/common/system.rs:646`, `:659`, and the four doc blocks at `:695-768`); on Windows they read the registry — Amended: D6 "one `System::new_with_specifics(..)` inside `tokio::task::spawn_blocking`" → "one `spawn_blocking` closure that builds the `System` **and** calls `name`/`os_version`/`long_os_version`/`kernel_version`" |
| [maintainer-review] MISSED: OQ-11's "a versioned tool counts only when a version is captured" versus `ProbeEnv.versions` | partial | `resolve_tool` runs `capture_version` only `if env.versions` (`probe.rs:359-362`); `tools::resolve` and the chat re-probe build envs with `versions: false` (`probe.rs:64-66`). A box probe handed such an env would record **every** versioned tool absent — Amended: D8/D9 add "`probe_box` requires `env.versions` (production `ProbeEnv::host` sets it; `probe_box` forces it on, or treats `versions == false` as presence-only, and a T3 test pins which)" |
| [maintainer-review] MISSED: D17's example edit adds `zig`, which D9 already seeds | falsified | D9's thirty-nine include `zig` (`zig version`); D17's SQL example and its sentence "So a user who adds `zig` still receives the tools a later `htui` release seeds" therefore describe a **replace**, not an add — Amended: use a tool the seed lacks in both places, e.g. `terraform` (`{"kind": "path", "names": ["terraform"], "version": {"args": ["version"], "pattern": "^Terraform v(\\S+)$"}}`) — and the live check's "insert a `box_probe_spec` row adding a tool" must likewise name an unseeded tool |
| [maintainer-review] MISSED: "`R-BOX-3` seeds ten tags; the seed map derives fifteen" (disagreement 12) | falsified | The seed derives nine of `R-BOX-3`'s ten (`heavy_build` is declared-only, D9) plus OQ-12's five = **fourteen**, which is also the Files table's "fourteen tag rules" (spec.json row) — Amended: "…the seed map derives fourteen (nine of the ten, `heavy_build` being declared-only, plus OQ-12's five)" |
| [maintainer-review] MISSED: Test plan counts after the amendments | falsified | Test plan "eleven spec-overlay cases (D17, D18)": T3's D17 list names **twelve** (`no_stored_spec_is_the_seed` … `the_digest_is_stable_and_changes_with_the_spec`); "T5's four end-to-end cases": T5 now lists **five** (`a_stored_spec_change_reprobes` added) — Amended: "twelve spec-overlay cases", "T5's five end-to-end cases" |
| [maintainer-review] MISSED: `0005`'s four `COMMENT ON COLUMN box.*` break `the_ana_column_comments_are_present_and_verbatim` | falsified | `tests/migrations.rs:338-411`: the query at `:381-397` lists every commented column of every table named in `ANA_COLUMN_COMMENTS` — which includes `box` (`box.settings`, `:313-317`) — and `:405-410` asserts "exactly the twenty-five commented columns, and no others". D4's comments on `machine_fingerprint`, `edit_version`, `probe_spec_digest` and `htui_version` make it 29 → red. T1 owns the file, so no re-cut, but no T1 step names it — Amended: T1's `tests/migrations.rs` bullet adds "the column-comment test (`:338`) learns `0005`'s four `box` comments (a `MOD7_COLUMN_COMMENTS` list checked verbatim beside `ANA_COLUMN_COMMENTS`, and the exclusivity count 25 → 29)" |
| [maintainer-review] MISSED: what an ordinary reconnect costs after D17/D18 | partial | D11's task now reads `box_info`, `app_settings()` (every row) and `boxes()` (every box of the user **with every tool**) per `Online` swap, then decides — Amended: D11 Why "A startup read per `Online` swap is the whole cost" → "three reads per `Online` swap"; Acceptance "an ordinary reconnect costs one read and no probe" → "costs three reads and no probe"; optionally note that `boxes()` is chosen over a one-box read only because it already exists for M2 |
| [maintainer-review] MISSED: Summary paragraphs not amended for D18 | partial | Summary "Identity (T1)": "Migration `0005` drops `box_user_id_hostname_key` … and adds `machine_fingerprint` and `edit_version`" omits the third column; "The write surface (T2)": "`record_box_probe` writes the hardware columns, `probed_tags`, `htui_version` and `last_probed_at`" omits `probe_spec_digest`, and "`boxes` reads every box with its tools" omits the digest — Amended: add "and `probe_spec_digest` (D18)" to each |
| [maintainer-review] MISSED: D9's `gradle` rationale | partial | `gradle` is absent here, so nothing was run. A plain `gradle --version` runs in the launcher JVM and is not documented to start the build daemon; what it does cost is a JVM start and, on a first run, initialising `~/.gradle` — and `./gradlew`, which the zsh alias prefers, downloads a distribution. The exclusion stands; its stated reason is unverified — Amended: D9 "`gradle --version` starts a JVM daemon and may take longer than `VERSION_TIMEOUT`" → "`gradle --version` starts a JVM and on first use initialises `~/.gradle` (and a wrapper downloads a distribution), so its cost is unbounded by design (unverified on this box: Gradle is not installed)". `bazel`'s reason is **verified**: `bazelisk --version` (mise, `bazel` itself absent) printed `Downloading https://releases.bazel.build/9.2.0/…` during this check |
| [maintainer-review] MISSED: test names not amended | partial | T3 `the_derived_vocabulary_is_r_box_3_without_heavy_build` now pins R-BOX-3 minus `heavy_build` **plus five**; T4 `an_online_swap_skips_a_box_probed_at_this_version` now also needs the same digest; T3's `tags_from_presence` table covers none of OQ-12's five — Amended: rename to `the_derived_vocabulary_is_r_box_3_without_heavy_build_plus_oq12`, `an_online_swap_skips_a_box_probed_at_this_version_and_spec`, and add `go`/`node`/`python`/`java`/`dotnet` rows to `tags_from_presence` (`javac` present, `java` alone → no `java` tag) |
| [maintainer-review] MISSED: residual stale wording | partial | (a) D5's formula writes `row.last_probed_at` / `row.htui_version` inside `BoxRecord::needs_probe(&self, …)` — should be `self.row.…`; (b) Acceptance "`htui-agent` no longer names `windows-registry`" — it never did (the draft only planned it; today `windows-registry 0.6.1` reaches Windows through `reqwest` → `hyper-util`, which also confirms the Complexity line's "already compiled on Windows"); (c) Summary "reads hardware per OS (D6, D7)" precedes its own amendment — Amended: (a) `self.row.last_probed_at.is_none() \|\| self.row.htui_version != running \|\| …`; (b) "`htui-agent` does not name `windows-registry`"; (c) "reads GPU per OS (D7) and the rest through `sysinfo` (D6)" |
| [maintainer-review] MISSED: D17 / Patterns row say `run_worker` reads `app_setting` "for `max_agents_per_run` and the lease settings" at `:896`, `:1227` | partial | `:896` reads `copy_max_total_bytes` (`run_worker.rs:909-912`); `:1227` hands the whole map to the engine, where `max_agents_per_run` (`htui-orch/src/graph.rs`) and the lease times (`recover.rs:59`) are resolved; `:1956` reads the sweep's lease period — Amended: "the call `run_worker` already makes for the isolator's copy cap, the engine's settings and the sweep's lease period (`run_worker.rs:896`, `:1227`, `:1956`)" |
| [maintainer-review] MISSED: re-cut independence recomputed from the current task table | verified | T1 ∩ T3 = ∅, T2 ∩ T3 = ∅ (both edit crate `htui-agent`, T2 `src/conformance.rs` + `tests/recorder.rs`, T3 `src/box_probe/**`, `probe.rs`, `launch.rs`, `lib.rs`, `tests/box_probe.rs` + fixtures; T3 implements no `WriteStore`, so T2's two new trait methods never need a T3 edit), T1 ∩ T2 = `{.sqlx/}` (now serial). T3's only upstream is T0. `Cargo.lock` moves only in T0. The one build-order hazard is the one already stated: T3's worktree lacks T2's trait methods, so the `htui-agent` gate must be re-run after T3 merges third |
