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

**Routing**: routed as **plan** by `/handoff-run MOD-7` (the PRD and its milestone table exist).
**Staffing: Opus 5.5 for every step — plan, fact-check, architect, implementers, verifiers and
reviewer (`rust-reviewer`, `.claude/workflow-config.json:2`); Fable is not used (maintainer standing
instruction).** Ultracode for the implementers only, one workflow per task, verify fan-out per
round; the architect and the reviewer stay plain agents.

**Numbering**: this is MOD-7's first plan. Its own decisions are **D1…D16**; the PRD's gate decisions
are always cited as **PRD D0…PRD D7** and never re-used. Risks start at **R-1**, open questions at
**OQ-1**, tasks at **T0**. Later MOD-7 milestones continue from D17 and R-9.

**Status**: **fact-checked** (2026-09-25), not yet confirmed. Branch `mod-7-box-registry` at
`1475b17`. 172 claims checked by six parallel verifiers: 153 verified, 8 partial, 11 falsified
(every falsification was a missed site or hazard, not a wrong decision). Amendments are marked
"(amended at fact-check)"; the ledger is the "Verified claims" table at the end.

**Graphify note**: `graphify-out/` does not exist in this checkout (`ls graphify-out` fails), so
nothing here was read from it. Every tree fact below was read through the Gortex index or the file
itself at `1475b17`; each carries a `file:line`, and anything not re-opened at its line is marked
**UNVERIFIED**.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked.

- [ ] **OQ-1 — A lost `box.toml` on the same machine.** PRD D1 keys on the id and says a mismatch
      mints; it is silent on the opposite case: a fresh id (the file was deleted, or the config
      directory was reset) arriving on a machine whose fingerprint an existing row already holds.
      Today's adopt-DB-id rule (`crates/htui-store/src/pg/mod.rs:344-349`) would hand back the old
      id by hostname. **Default adopted (D3):** no adoption of any kind. An id the database has
      never seen is a new row, always; the old row stays and milestone 2's section lists it.
      Adopting by fingerprint would re-merge exactly the cloned VMs ANA-16 C4 is about (a clone
      shares its machine-id unless regenerated, PRD Evidence), and adopting by hostname is C4
      itself. **Alternative:** adopt when exactly one row of this user carries the same fingerprint
      and has not been seen for N days.
- [ ] **OQ-2 — A system-information crate or direct reads.** The PRD leaves it to the plan
      ("A new system-information dependency is a decision, justified in the plan").
      **Default adopted (D6):** no crate. Linux reads four world-readable files, verified on this
      box; macOS runs `sw_vers` and `sysctl` and `system_profiler`; Windows reads the registry
      through `windows-registry` (already compiled on Windows) and asks PowerShell's CIM for RAM and
      display adapters. **Alternative:** `sysinfo`, which covers OS version, CPU and RAM on all
      three platforms but not GPU and not the machine identity — so the per-OS code exists either
      way, and `sysinfo` is not in `Cargo.lock` today.
- [ ] **OQ-3 — Does an integrated GPU earn the `gpu` tag?** This box has only an AMD iGPU
      (`/sys/bus/pci/devices/0000:0f:00.0`, class `0x030000`, vendor `0x1002`, device `0x164e`;
      `lspci` names it "Raphael"). **Default adopted (D7):** yes — any display-class PCI device from
      a vendor in the data map is a GPU; virtual adapters (QEMU/Bochs `0x1234`, VMware `0x15ad`,
      Red Hat virtio `0x1af4`, Hyper-V `0x1414`) are not in the map and so never count.
      **Alternative:** discrete only, which on Linux needs per-vendor heuristics (an AMD APU and an
      AMD discrete card share a vendor id).
- [ ] **OQ-4 — The compare-and-set token milestone 2's editors will use.** The PRD's open question
      ("Which token the declared-tags and quirks CAS compares, given reconnects bump `updated_at`").
      Every reconnect bumps `box.updated_at` today (`last_seen_at` update in
      `pg/mod.rs:358-362` plus the `set_updated_at` trigger, `0001_init.sql:575-579`), and after
      this milestone every probe does too. **Default adopted (D14):** migration `0005` adds
      `box.edit_version INTEGER NOT NULL DEFAULT 0`, bumped only by human-edit writers (milestone
      2's declared tags and quirks, MOD-12's caps later) and never by registration or the probe.
      Milestone 1 adds the column and nothing that reads or writes it. **Alternative:** a
      value-compare CAS (`WHERE declared_tags = $old AND quirks = $old`) that needs no column.
- [ ] **OQ-5 — What `box.htui_version` means.** It is documented as the re-probe trigger
      (`0001_init.sql:64`), but `register_box` overwrites it on every connect
      (`pg/mod.rs:360`), so a probe interrupted after a version bump would never be retried.
      **Default adopted (D5):** it becomes "the version at the last successful probe": inserted at
      first registration, never touched by a reconnect, rewritten only by the probe writer.
      **Alternative:** keep today's write and add a `probed_version` column.
- [ ] **OQ-6 — How a missing agent with an install source is surfaced in milestone 1.** PRD D7:
      offered, never installed; the milestone says "no UI yet beyond what exists".
      **Default adopted (D13):** the probe's report lands on the existing status line
      ("box probed … · agy is missing and can be installed: Settings > Agents, i"), and the agents
      section already renders the row `missing` with the MOD-20 `i` action
      (`crates/htui/src/ui/tabs/settings/agents.rs:866-893`, `:729-760`). No install request is
      sent, not even the pre-flight. **Alternative:** say nothing until milestone 2's section.
- [ ] **OQ-7 — Does an on-demand box re-probe also re-probe the agents?** **Default adopted (D11):**
      yes — one path, box then agents, whether the trigger is registration or `ProbeBox`. The
      agents section's own `r` stays as it is. **Alternative:** `ProbeBox` probes the box only.
- [ ] **OQ-8 — An id that exists under another `app_user`.** Single-user today (`R-USR-2`), so this
      can only be a copied file. **Default adopted (D3):** treated as a copy: a new id is minted.
- [ ] **OQ-9 — The tool list.** `R-BOX-2` names compilers, build tools, "shells" and "container
      runtime" without listing the last two. **Default adopted (D9):** `rustc`, `cargo`, `gcc`,
      `clang`, `cl`, `cmake`, `ninja`, `vcpkg`, `bash`, `zsh`, `fish`, `pwsh`, `powershell`,
      `docker`, `podman`, plus `vulkaninfo` (presence only, so PRD D2's `vulkan` tag has a fact to
      stand on). `cl` is found only when it is on `PATH` (a Developer prompt); locating Visual
      Studio through `vswhere` is MOD-16's. **Alternative:** a shorter list without `fish` and
      `powershell`.
- [ ] **OQ-10 — `--demo` mode.** **Default adopted (D11):** the registration probe never runs on a
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
name, read from `pg_constraint` on `htui_prepare_check`) and adds `machine_fingerprint` and
`edit_version`. `connect::try_connect` already writes an adopted id back to `box.toml`
(`crates/htui-store/src/connect.rs:432-449`); it now does so for a minted one.

**The probe (T3).** `htui_agent::box_probe` reads hardware per OS (D6, D7), resolves a tool list
that is data — the same `ToolProbe` shape agent rows use, run through the existing tier-1 resolver
`resolve_tool` (`crates/htui-agent/src/probe.rs:354-413`) — and derives `probed_tags` from a
presence map that is also data (PRD D2, D9). Nothing in code names a tool or a tag.

**The write surface (T2).** `WriteStore::record_box_probe` writes the hardware columns,
`probed_tags`, `htui_version` and `last_probed_at` and replaces this box's `box_tool` set in one
transaction, touching nothing a human owns. `WriteStore::boxes` reads every box with its tools —
what the conformance case needs to read back and exactly what milestone 2's section will list.

**The trigger (T4).** After every `Online` swap the agent runtime reads this box's row and probes
when it was never probed or was probed by another `htui` version (D5, D11) — so a reconnect costs
one read, not a probe. The probe runs on a runtime-owned task (the `ProbeAgents` pattern,
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
| D4 | **Migration `0005_box_identity.sql`** (forward-only, `R-STO-5`): `ALTER TABLE box DROP CONSTRAINT box_user_id_hostname_key;` `ALTER TABLE box ADD COLUMN machine_fingerprint TEXT CHECK (machine_fingerprint IS NULL OR machine_fingerprint ~ '^[0-9a-f]{64}$');` `ALTER TABLE box ADD COLUMN edit_version INTEGER NOT NULL DEFAULT 0;` and `COMMENT ON COLUMN` for both new columns and for `box.htui_version` (D5's meaning), with a header in the style of `0004_max_agents_per_run_default.sql`. No new table (`TABLES.len()` stays 33). **No cache migration**: neither new column is mirrored (the fingerprint is a registration fact and `edit_version` is read online only), `refresh_box` names its columns (`cache/refresh.rs:583-603`, `:611-620`), and `MIRRORED_TABLES` (`cache/mod.rs:40-58`) is unchanged. Two consequences, both deliberate: every mirror rebuilds once on upgrade because `PgStore::schema_version` moves from 4 to 5 (`CacheStore::open`, `cache/mod.rs:127-150`); and `refresh_box` gains `DELETE FROM box WHERE id <> ?` in its SQLite transaction, because a copied config directory carries a mirror holding the old box and the offline `box_info` reads `FROM box ORDER BY id LIMIT 1` (`cache/read.rs:1102-1106`) — the older UUIDv7 would win. | The constraint name is not guessed: `SELECT conname FROM pg_constraint WHERE conrelid = 'box'::regclass` on `htui_prepare_check` answers `box_user_id_hostname_key`. The `CHECK` makes "only the keyed hash reaches Postgres" a schema fact, not only a code fact. |
| D5 | **`htui_version` is the version at the last probe, and it decides re-probing.** `pub const HTUI_VERSION: &str = env!("CARGO_PKG_VERSION")` in `htui_store` (the crate that writes it today, `pg/mod.rs:371`) is the one definition: registration inserts it, the probe writer records it, and the decision compares with it. `BoxRow::needs_probe(&self, running: &str) -> bool` in `htui-core` = `last_probed_at.is_none() \|\| htui_version != running`. Because registration no longer overwrites the column, a probe that fails or is interrupted after an upgrade is retried at the next `Online` swap. | `R-BOX-2`: "Re-probe on demand and when `htui` version changes"; the column's own comment (`0001_init.sql:64`) already calls it the trigger. The PRD metric "not on every connect" is met by a read, not by a flag in memory. |
| D6 | **Hardware without a system-information crate (OQ-2).** `htui_agent::box_probe::hardware` returns `Hardware { os_version, cpu, ram_mb: Option<i32>, gpu: Option<GpuVendor> }`. **Linux** (verified on this box, all readable without root): `os_version` = `PRETTY_NAME` of `<root>/etc/os-release`, falling back to `<root>/usr/lib/os-release` (here `Ubuntu 24.04.5 LTS`); `cpu` = the first `model name` of `<root>/proc/cpuinfo` (here `AMD Ryzen 5 7600X 6-Core Processor`); `ram_mb` = `MemTotal` kB of `<root>/proc/meminfo` divided by 1024, floored (here `64927592 kB` → `63405`); GPU per D7. `<root>` is injectable (production `/`) so every Linux path is a fixture test. **macOS**: `sw_vers -productVersion`, `sysctl -n machdep.cpu.brand_string` and `sysctl -n hw.memsize`; GPU `apple` on `aarch64`, else the vendor id parsed from `system_profiler SPDisplaysDataType`. **Windows**: `CurrentMajorVersionNumber`, `CurrentMinorVersionNumber` and `CurrentBuild` under `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` and `ProcessorNameString` under `HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0` through `windows-registry`; RAM and display adapters from one `powershell -NoProfile -NonInteractive -Command` CIM query (`Win32_ComputerSystem.TotalPhysicalMemory`, `Win32_VideoController.PNPDeviceID`, whose `VEN_xxxx` feeds D7's map). Every child runs through the probe's existing bounded runner (`run_bounded`, `probe.rs:982-1039`, `VERSION_TIMEOUT` 15 s at `:45`), made `pub(crate)` **together with its result type `OneShot` and the fields `stdout`, `status` and `stderr`** (`probe.rs:960-967`; amended at fact-check: the function alone returns a type `box_probe` could not read, and `private_interfaces` would warn). `run_bounded` resolves its command through `launch::spawn`'s `which` on the **process** `PATH` (`launch.rs:1019-1021`), not `ProbeEnv`'s, so hardware children are named by absolute path (`/usr/bin/sw_vers`, `/usr/sbin/sysctl`, `/usr/sbin/system_profiler`; `powershell` via `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`), and a tag rule's `ask` runs the path `resolve_tool` returned, never a bare name (amended at fact-check). Every parser is a pure function over captured text, compiled and tested on every platform; only the gathering is `cfg`-gated. Nothing here is fallible to the caller: an unreadable fact is empty or `None`. | Zero new compiled crates: `windows-registry 0.6.1` is already built on Windows under `reqwest → hyper-rustls → hyper-util` for both `htui-agent` and `htui-store` (`cargo tree --target x86_64-pc-windows-msvc -i windows-registry`). `std` has no RAM API and the `windows` crate's `GlobalMemoryStatusEx` is `unsafe`, which the workspace forbids (`Cargo.toml` `[workspace.lints.rust] unsafe_code = "forbid"`). The Windows and macOS arms are written and reviewed by eye (TOOL-3, `HANDOFF.md:599`) and verified by MOD-16 (Windows) and nobody yet (macOS; R-3). |
| D7 | **GPU presence and vendor on Linux without root (the PRD's second open question).** Scan `<root>/sys/bus/pci/devices/*/class` for a display-class value (`0x03xxxx`: VGA `0x0300`, 3D controller `0x0302`, display `0x0380`) and read the sibling `vendor`. Both files are `-r--r--r--` on this box, which reports exactly one device, `class=0x030000 vendor=0x1002`. The vendor id goes through a data map in priority order — `0x10de` nvidia, `0x1002` amd, `0x8086` intel, `0x106b` apple, `0x5143` qualcomm — and the first present vendor in that order wins, so an Intel iGPU beside an NVIDIA card reports `nvidia`. An unmapped vendor (every virtual adapter) is not a GPU (OQ-3). `lspci` (present here) and `nvidia-smi` (absent here) are **not** used: one is a parser of the same files, the other exists only with the proprietary driver. `/sys/class/drm/card0/device/vendor` also answers `0x1002` here but exists only when a DRM driver is bound, so it is not the source. | The same map feeds Windows (`VEN_10DE`) and Intel macOS, so vendor naming is one table. WSL2 exposes its GPU as `/dev/dxg`, not PCI, and reports no GPU (R-4). |
| D8 | **The box probe lives in `htui-agent`, as `crates/htui-agent/src/box_probe/`.** `htui-agent` depends on `htui-core` only (`crates/htui-agent/Cargo.toml` `[dependencies]`), and it already owns everything a probe needs: `ProbeEnv` (`probe.rs:55-70`, its injectable `PATH`, cwd, `versions` flag and per-child timeout), `resolve_tool` (`:354-413`), `capture_version` (`:1049-1065`), `extract_version` (`:928-944`), the bounded runner, `which` and `process-wrap`. `htui-store` must not spawn tools; `htui-core` has no process dependencies and stays pure; `htui` is a binary crate, and MOD-41's headless worker will need the probe without a UI. Entry point: `pub async fn probe_box(box_id: BoxId, env: &ProbeEnv, host: &HostRoot, htui_version: &str, now: DateTime<Utc>) -> BoxProbe`, where `BoxProbe` is the `htui-core` type T0 adds. | One resolver, two callers — the rule MOD-2 D46 set for agents. |
| D9 | **The tool list and the tag map are one compiled data document** (PRD D2: "The map is data, not a `match` on tool names scattered through code"). `crates/htui-agent/src/box_probe/spec.json`, embedded with `include_str!` and typed by `spec.rs`: `{ "tools": { "<name>": <ToolProbe> }, "tags": [<TagRule>], "gpu_vendors": [{ "pci": "0x10de", "name": "nvidia" }, …] }`. `tools` reuses `launch::ToolProbe` verbatim (`launch.rs:192-230`, `#[serde(tag = "kind", rename_all = "snake_case")]`), so every entry is a `{"kind": "path", "names": [..], "version": {"args": [..], "pattern": ".."}}` like the agent seeds (`crates/htui-core/seeds/agent_claude_cli.json:12-16`); a tool with no `version` is presence-only (`vulkaninfo`, `powershell`) and records `version = ''`, which the prompt renders bare (documented at `crates/htui-core/src/model/box_.rs:152-154`, rendered at `crates/htui-core/src/prompt/render.rs:397-402`). `TagRule { tag, any_tool: Vec<String>, fact: Option<Fact>, ask: Option<Ask> }` with `Fact::Gpu` and `Ask { args, matches }`: the rule fires when a named tool was found **and**, if `ask` is set, running that tool with `args` prints a line matching `matches`. The seeded rules: `rust` ← `rustc` or `cargo`; `cmake` ← `cmake`; `clang` ← `clang`; `msvc` ← `cl`; `mingw` ← `gcc` with `ask {args: ["-dumpmachine"], matches: "mingw"}` (this box answers `x86_64-linux-gnu`); `vcpkg` ← `vcpkg`; `docker` ← `docker` or `podman`; `vulkan` ← `vulkaninfo`; `gpu` ← `Fact::Gpu`. `heavy_build` is declared only. Tags come out sorted and deduplicated. | A tag is added by editing one JSON block; a test pins the derived vocabulary to `R-BOX-3`'s seeded list minus `heavy_build`, **written out in the test** (amended at fact-check: `SEEDED_TAGS`, `pg/mod.rs:27-38`, is a private `htui-store` const and `htui-agent` has no edge to `htui-store`; sharing it would couple T3 to T1 through a new crate edge, so the test carries its own copy with a comment naming both sources). The `ask` form is what keeps "a MinGW `gcc`" out of code without keeping a second `gcc` row in `box_tool`. |
| D10 | **The store write surface.** `WriteStore` (`crates/htui-core/src/store/traits.rs:195`) gains `async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()>` — in one transaction, `UPDATE box SET os_version, cpu, ram_mb, gpu_present, gpu_vendor, probed_tags, htui_version, last_probed_at WHERE id` (no row → `StoreError::NotFound { entity: "box", .. }`), `DELETE FROM box_tool WHERE box_id`, and one `INSERT … SELECT FROM UNNEST(..)` of the new set with `probed_at = probe.probed_at`; a duplicate tool name is `StoreError::Constraint`. It never writes `hostname`, `declared_tags`, `quirks`, `settings`, `machine_fingerprint`, `edit_version` or `last_seen_at`. And `async fn boxes(&self) -> Result<Vec<BoxRecord>>`, every box of this user with its tools (`BoxRecord { row: BoxRow, tools: Vec<BoxTool> }`), boxes ordered by id and tools by `name COLLATE "C"` so Postgres and `MemStore` agree byte for byte. Implemented by `MemStore` (`mem.rs:4322`), `PgStore` (`pg/write.rs:381`), `Writer` (`writer.rs:288`, the `upsert_agent_box` arms at `:341-346` are the mirror), and the two forwarding spies, `UsageSpy` (`crates/htui-agent/src/conformance.rs:674`) and `SpyStore` (`crates/htui-agent/tests/recorder.rs:354`). | The narrow-writer rule of MOD-2 D74 (`traits.rs:253-270`): a machine writer must not be able to overwrite what a human wrote. `boxes` is on `WriteStore` beside the other editor reads (`workspace`, `repos`, `setting`, `traits.rs:395-645`), because `WriteStore` carries no box read today and the conformance case has to read the write back; it is also, unchanged, milestone 2's list. |
| D11 | **The trigger and where it runs.** `AgentRuntime` gains `pub fn on_online(&mut self, backend: &Backend, replies: &UnboundedSender<ReplyEnvelope>)`, called by the store loop right after each `go_online` — the two sites that already call `runs.sweep` (`crates/htui/src/store_worker.rs:1281-1286` after `ApplyMigrations`, `:1568-1575` on `ConnEvent::Online`). It spawns onto `self.background` and returns at once; the task reads `box_info` then `box_row`, and stops silently unless `BoxRow::needs_probe(HTUI_VERSION)`. Nothing runs at loop start, so `Backend::Memory` (`--demo`, every harness) never auto-probes (OQ-10). **The registration probe is opt-in** (amended at fact-check): `AgentRuntime::production()` leaves it off, and only the binary's entry point turns it on — `crates/htui/src/lib.rs:92` changes from `store_worker::spawn(..)` to `store_worker::spawn_with(.., AgentRuntime::production().with_registration_probe())`. Without that, `crates/htui/tests/connection.rs` (which spawns through `store_worker::spawn` at `:122` and reaches `ConnEvent::Online` against real Postgres at `:908` and `:980-1004`) would probe the host's real `PATH` and seeded agents, breaking H-7. **The probe environment is injectable** (amended at fact-check): `AgentRuntime::with_probe_env(ProbeEnv, HostRoot)` is used by `on_online` and `ProbeBox`; production defaults to `ProbeEnv::host(cwd)` and `/`, every test injects a fake `PATH` and fixture root. `StoreRequest::ProbeBox` (new, served ahead of `try_serve` like `ProbeAgents`) runs the same task with the decision skipped and answers its requester. **The box probe has its own slot** (amended at fact-check): `AgentRuntime.box_probe: Option<JoinHandle<()>>`, like `install` and `auth`. `on_online` and `ProbeBox` first run the finished-task sweep `serve` runs today (`agent_worker.rs:707-720`, factored into `fn sweep_finished(&mut self)` and called from both — without it a finished preview or chat re-probe left in `background` would make `on_online` skip for the whole session), then skip or refuse unless `claim_is_free` (`:1267-1295`). `probe` (`ProbeAgents`, `:900-953`), `install_plan` and `login` also refuse while `box_probe` is held, because `probe` checks only `auth` and `install` today (`:909-923`) and would otherwise write `agent_box` concurrently with the registration task. `ProbeBox` also refuses offline with `REGISTRY_ON_SERVER_ONLY`, exactly as `probe` does (`:926-931`). | `R-NF-3` by construction: the loop awaits nothing, the probe's children are bounded, and the pattern is the one `ProbeAgents` already proves (`Served::Deferred`, `store_worker.rs:1488-1510`). A startup read per `Online` swap is the whole cost of "not on every connect". |
| D12 | **The agent half is the existing probe, factored, not copied.** `run_probe` (`agent_worker.rs:1718-1768`) is split into `probe_agents_on(writer, box_id, agents, env) -> Result<Vec<AgentSummary>, StoreError>` (the loop body, unchanged in behaviour: disabled rows skipped, `ProbeOutcome::Kept` leaves a manual row alone per MOD-2 D51, a write failure stops) and the thin `run_probe` that replies. `ProbeArgs.cwd` becomes `env: ProbeEnv` so a test can hand both halves one fake `PATH` (production: `ProbeEnv::host(cwd)`, `probe.rs:99-115`). The box task calls `probe_box`, `record_box_probe`, then `probe_agents_on` for every agent row. | PRD scope: "The same trigger runs the existing agent probe for this box"; the PRD constraint that a probe finding nothing never overwrites a manual row is `ProbeOutcome::Kept`'s job already. |
| D13 | **The offer, not the install (PRD D7, OQ-6).** The task ends with one `StoreReply::BoxProbed(BoxProbeReport)` where `BoxProbeReport { tools: usize, probed_tags: Vec<String>, installable: Vec<String>, agents_failed: Option<String> }` and `installable` names every agent whose fresh row has `probe.status = "missing"` and whose launch document declares `discovery.install` (`launch.rs:119-126`). The registration trigger has no requester, so the reply goes to `Origin::App` at `UNSOLICITED: Seq = Seq::MAX` — a `seq` `App::dispatch` never issues (it counts up from 0, `app/state.rs:219`), so the freshness gate (`app/update.rs:164-168`) always drops it after `App::observe_reply` (`:238-262`), which runs first and writes the status line. `ProbeBox` replies at its own address. `declares_a_source` (`settings/agents.rs:1302-1307`) moves to `htui_agent::launch::declares_install(&Value) -> bool`, and **the second copy of the rule, `agent_worker.rs:2003` `declares_a_source(&Agent) -> Result<(), StoreError>` called by `install_plan` at `:1038`, delegates to it too** (amended at fact-check), so the section, the install pre-flight and the report use one rule. Nothing sends `InstallPlan` or `InstallConfirm`. | The existing `Settings > Agents` cell already reads `missing` and binds `i` to MOD-20's consented flow; the status line is the one piece of existing UI that can say so without a new view. |
| D14 | **`box.edit_version` for milestone 2 (OQ-4).** Added by `0005`, default 0, commented as "bumped by the declared-tags, quirks and settings editors only; registration and the probe never write it". `BoxRow` does not gain the field in this milestone (every `BoxRow` constructor — `fixtures.rs:417`, `mem.rs`, the `pg/read.rs` `query_as!` — would move for a value nothing reads yet). | The PRD constraint: "Every new box writer is a compare-and-set keyed on a token a reconnect does not bump". Adding it now spares milestone 2 a migration and a second pin move. |
| D15 | **What milestone 1 leaves to milestone 2, by name.** The Settings `box` section; a `StoreRequest` that serves `boxes()`; the declared-tags and quirks writers and their CAS on `edit_version`; `BoxRow.edit_version`; `BoxInfo`'s missing fields (`os_version`, `quirks`, tools). Milestone 1 does expose `StoreRequest::ProbeBox` and `WriteStore::boxes`, because both fall out of this milestone's own needs. | The PRD brief: expose M2's requests only if cheap and natural. |
| D16 | **`R-BOX-2`'s "installed agents" stay in `agent_box`**, not in `box_tool`. The tool list is compilers, build tools, shells, container runtimes and `vulkaninfo`; agents are the agent probe's (D12). | One authority per fact (`R-AGT-6`). |

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

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `Cargo.toml` | edit | T0 | `hmac = "0.12"`, `windows-registry = "0.6"` in `[workspace.dependencies]` |
| `Cargo.lock` | regenerate | T0 | the two new edges; on Linux no new crate is compiled |
| `crates/htui-store/Cargo.toml` | edit | T0 | `hmac`; `[target.'cfg(windows)'.dependencies] windows-registry` |
| `crates/htui-agent/Cargo.toml` | edit | T0 | `[target.'cfg(windows)'.dependencies] windows-registry` beside the existing `windows` entry |
| `crates/htui-core/src/model/box_.rs` | edit | T0 | `BoxProbe`, `ProbedTool`, `BoxRecord`, `BoxRow::needs_probe` + unit tests (D5, D10) |
| `crates/htui-core/src/model/mod.rs` | edit | T0 | re-export the three types (`:99-101`) |
| `crates/htui-store/migrations/0005_box_identity.sql` | create | T1 | D4 |
| `crates/htui-store/src/identity.rs` | edit | T1 | `Fingerprint`, `machine_fingerprint`, the per-OS readers, unit tests; module doc `:1-5` corrected (D1) |
| `crates/htui-store/src/pg/mod.rs` | edit | T1 | `register_box` transaction, `Registration`, `HTUI_VERSION`, `bootstrap`, `registration()`; the docs at `:337-351` and `connect_with`'s adopt-DB-id paragraph `:104-108` rewritten (D2, D3, D5; the second amended at fact-check). `identity.rs:96`'s "used by the adopt-DB-id rule" is reworded in the same task |
| `crates/htui-store/src/connect.rs` | edit | T1 | `try_connect`'s log line for `Copied`; the `adopted` wording at `:251`, `:331` and the `try_connect` doc `:422-431` (D2; amended at fact-check) |
| `crates/htui-store/src/testkit.rs` | edit | T1 | the `demo_db` doc on the hostname collision (`:163-167`) reworded (amended at fact-check) |
| `crates/htui-store/src/lib.rs` | edit | T1 | re-export `HTUI_VERSION`, `Registration`, `identity::Fingerprint` |
| `crates/htui-store/src/cache/refresh.rs` | edit | T1 | prune other `box` rows in `refresh_box` (D4) |
| `crates/htui-store/src/cache/read.rs` | edit | T1 | the doc at `:1094-1101` ("holds the own `box` row and no other") made true again and says why |
| `crates/htui-store/tests/box_identity.rs` | create | T1 | the hazard first, then D1–D3 on Postgres |
| `crates/htui-store/tests/migrations.rs` | edit | T1 | applied `vec![1, 2, 3, 4, 5]` at `:74` and `:591`, `Pending(5)` (`:623`), `register_box_upserts_and_adopts` (`:1016-1091`) rewritten, a `0005` case |
| `crates/htui-store/tests/connect.rs` | edit | T1 | `Pending(5)` (`:95`) and `pending, 5` with "all five" (`:108-111`, amended at fact-check); a copied `box.toml` is rewritten to the minted id |
| `crates/htui-store/tests/cache.rs` | edit | T1 | the mirror keeps only this box after a mint |
| `crates/htui-store/.sqlx/` | regenerate | T1, T2 | new and removed `query!` hashes (merge rule under Tasks) |
| `crates/htui-core/src/store/traits.rs` | edit | T2 | `record_box_probe`, `boxes` (D10) |
| `crates/htui-core/src/store/mem.rs` | edit | T2 | both methods on `MemStore` |
| `crates/htui-core/src/store/conformance.rs` | edit | T2 | three cases; `CASES` 53 → 56 |
| `crates/htui-core/tests/mem_store.rs` | edit | T2 | pin 53 → 56 (`:36-37`) |
| `crates/htui-store/src/pg/write.rs` | edit | T2 | both methods on `PgStore` |
| `crates/htui-store/src/writer.rs` | edit | T2 | dispatch arms |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T2 | `EXPECTED_CASES` 53 → 56 (`:19`) |
| `crates/htui-agent/src/conformance.rs` | edit | T2 | `UsageSpy` forwards both methods |
| `crates/htui-agent/tests/recorder.rs` | edit | T2 | `SpyStore` forwards both methods |
| `crates/htui-agent/src/box_probe/mod.rs` | create | T3 | `probe_box`, tool resolution, tag derivation, `HostRoot` (D8, D9) |
| `crates/htui-agent/src/box_probe/hardware.rs` | create | T3 | per-OS gathering and pure parsers (D6, D7) |
| `crates/htui-agent/src/box_probe/spec.rs` | create | T3 | the typed data document (D9) |
| `crates/htui-agent/src/box_probe/spec.json` | create | T3 | tools, tag rules, GPU vendor map (D9) |
| `crates/htui-agent/src/probe.rs` | edit | T3 | `run_bounded` (`:982`), `OneShot` and its fields (`:960-967`) → `pub(crate)` (amended at fact-check) |
| `crates/htui-agent/src/launch.rs` | edit | T3 | `pub fn declares_install(&Value) -> bool` (D13) |
| `crates/htui-agent/src/lib.rs` | edit | T3 | `pub mod box_probe;` (`:92-110`) and re-exports |
| `crates/htui-agent/tests/box_probe.rs` | create | T3 | fixture host roots, fake `PATH`, parsers, the spec |
| `crates/htui-agent/tests/fixtures/box_probe/**` | create | T3 | host-root trees and captured command outputs |
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
`repo_box_path` and excerpts (milestone 4); `docs/**`, `HANDOFF.md`, the PRD (the main thread
records deviations); `set_agent_box_quota`'s `Option` widening that `traits.rs:285-294` reserves for
"MOD-7" — nothing in this milestone unregisters an agent or carries a quota across boxes.

## Tasks

**T0 alone first. Then Wave A: T1 ∥ T2 ∥ T3, each in its own git worktree, merged T1, then T2,
then T3, with the touched crates' gates re-run on the real tree after each merge. Then T4. Then
T5.** Independence is decided by intersecting the file sets below and by build coupling (MOD-4 M5
D105): a red or mid-edit commit in a dependency crate stops every dependent crate compiling, which is
why each parallel task runs in its own worktree.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `Cargo.toml`, `Cargo.lock`, `crates/htui-store/Cargo.toml`, `crates/htui-agent/Cargo.toml`, `crates/htui-core/src/model/box_.rs`, `crates/htui-core/src/model/mod.rs` | first, alone, until green |
| T1 | `crates/htui-store/migrations/0005_box_identity.sql`, `crates/htui-store/src/identity.rs`, `crates/htui-store/src/pg/mod.rs`, `crates/htui-store/src/connect.rs`, `crates/htui-store/src/lib.rs`, `crates/htui-store/src/testkit.rs`, `crates/htui-store/src/cache/refresh.rs`, `crates/htui-store/src/cache/read.rs`, `crates/htui-store/tests/box_identity.rs`, `crates/htui-store/tests/migrations.rs`, `crates/htui-store/tests/connect.rs`, `crates/htui-store/tests/cache.rs`, `crates/htui-store/.sqlx/` | Wave A, own worktree, merged first |
| T2 | `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/.sqlx/`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | Wave A, own worktree, merged second |
| T3 | `crates/htui-agent/src/box_probe/mod.rs`, `crates/htui-agent/src/box_probe/hardware.rs`, `crates/htui-agent/src/box_probe/spec.rs`, `crates/htui-agent/src/box_probe/spec.json`, `crates/htui-agent/src/probe.rs`, `crates/htui-agent/src/launch.rs`, `crates/htui-agent/src/lib.rs`, `crates/htui-agent/tests/box_probe.rs`, `crates/htui-agent/tests/fixtures/box_probe/**` | Wave A, own worktree, merged third |
| T4 | `crates/htui/src/agent_worker.rs`, `crates/htui/src/lib.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/testkit.rs`, `crates/htui/src/app/update.rs`, `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/box_probe.rs` | serial, after Wave A |
| T5 | `crates/htui/tests/box_probe_pg.rs` | serial, last |

**Intersections, checked.** T1 ∩ T3 = ∅ (different crates). T2 ∩ T3 = ∅: T2's `htui-agent` files are
`src/conformance.rs` and `tests/recorder.rs`, T3's are `src/box_probe/**`, `src/probe.rs`,
`src/launch.rs`, `src/lib.rs`, `tests/box_probe.rs` and its fixtures. T1 ∩ T2 = `{.sqlx/}` and
nothing else: T1 owns `pg/mod.rs`, `identity.rs`, `connect.rs`, `lib.rs` and `cache/*`, T2 owns
`pg/write.rs` and `writer.rs`. **The `.sqlx` rule** (the hidden coupling the MOD-4 fan-out found):
`.sqlx` files are named by query hash, so T1 (removes the old `register_box` query, adds its new
ones) and T2 (adds its own) touch disjoint file names and merge without a conflict — but T2's
worktree still carries the pre-T1 `register_box` file. So **after merging T2, run `cargo sqlx
prepare` once more on the merged tree against a scratch database migrated through `0005`, commit
the result, and gate on `prepare --check`**. **Cargo.lock** moves only in T0. **Build coupling:**
T0 adds types and dependencies nothing uses yet (no lint in the workspace flags an unused
dependency: `Cargo.toml` `[workspace.lints.rust]` names `unsafe_code`,
`missing_debug_implementations` and `unused_qualifications` only). T1 changes `register_box`'s
signature and return type, whose only callers are `bootstrap` (`pg/mod.rs:420`) and
`tests/migrations.rs:1029`, `:1040` — all in T1. T2 adds two trait methods and implements them in
all five implementations inside the same task, so its tree compiles alone. T3 adds a module and
makes one private function `pub(crate)`. **No shared case list** moves outside T2; no snapshot moves
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
  `a_box_probed_at_this_version_needs_none`.
- **Action**: add `BoxProbe { box_id, os_version, cpu, ram_mb: Option<i32>, gpu_present,
  gpu_vendor: Option<String>, tools: Vec<ProbedTool>, probed_tags: Vec<String>, htui_version,
  probed_at }`, `ProbedTool { name, version, path }`, `BoxRecord { row: BoxRow, tools: Vec<BoxTool>
  }` and `BoxRow::needs_probe`; re-export them. Add the two workspace dependencies and the three
  manifest entries with a comment each, in the house style (`Cargo.toml`'s MOD-20 comments).
  Every new `pub` item carries a doc comment and a `Debug` impl in every task: each lib root
  warns on `missing_docs` (`htui-core/src/lib.rs:9`, `htui-store/src/lib.rs:11`,
  `htui-agent/src/lib.rs:40`, `htui/src/lib.rs:10`) and the workspace warns on
  `missing_debug_implementations` (amended at fact-check).
- **Mirror**: `BoxProfile::project`'s doc and test style (`box_.rs:163-200`, `:246-280`).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`;
  `cargo tree -i hmac@0.12.1` (still one `hmac 0.12.1` in the lock; on Linux it was already
  compiled); `cargo tree --target x86_64-pc-windows-msvc -i windows-registry` (still `0.6.1`, now
  also a direct edge). Commit boundary: one red commit (the tests), one green.

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
  0); `a_reconnect_leaves_htui_version_and_the_probe_columns_alone` (set `htui_version = '0.0.0'`
  and `last_probed_at = NULL` by SQL, register again, both unchanged — D5).
  `identity.rs` unit tests: `the_fingerprint_is_hmac_sha256_known_answer`,
  `the_fingerprint_ignores_case_and_surrounding_whitespace`, `an_empty_identity_is_no_fingerprint`,
  `debug_never_prints_the_fingerprint`, `linux_reads_machine_id_then_the_dbus_copy` (over a temp
  root), `the_ioreg_line_is_parsed` (captured sample text), and, `cfg(target_os = "linux")`,
  `this_box_has_a_fingerprint_when_machine_id_is_readable`.
  `tests/migrations.rs`: both applied lists (`:74`, `:591`) and `Pending(5)` (`:623`);
  `register_box_upserts_and_adopts`
  becomes `register_box_keys_on_the_id` (its hostname-adopt half is D3's reversal; its
  `box.toml` write-back half stays); new `the_0005_migration_drops_the_hostname_key_and_adds_two_columns`
  (constraint absent from `pg_constraint`, both columns present, the `CHECK` refuses `'RAW'`).
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
  `probed_tags`, `htui_version` and `last_probed_at` as written, and `hostname`, `declared_tags`,
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
  then the `.sqlx` merge rule above.

### Task 3: `htui-agent` — the box probe (D6–D9, D13's helper)
- **Files**: as tabled.
- **Tests first** (`tests/box_probe.rs` and module unit tests):
  `the_spec_parses_and_every_tag_rule_names_a_listed_tool`;
  `the_derived_vocabulary_is_r_box_3_without_heavy_build`;
  `tags_from_presence` (table-driven: `cargo` alone → `rust`; `podman` → `docker`; `gcc` whose
  `-dumpmachine` prints `x86_64-w64-mingw32` → `mingw`, and `x86_64-linux-gnu` → none; a GPU →
  `gpu`; `vulkaninfo` → `vulkan`; output sorted and deduplicated);
  `tools_over_a_fake_path_report_versions_and_skip_the_absent` (`#[cfg(unix)]`, scripts that print
  the captured first lines listed under Validation); `a_hanging_tool_is_bounded` (a script that
  sleeps, `version_timeout` 200 ms, the tool is found with an empty version);
  `linux_hardware_from_a_fixture_root` (os-release, cpuinfo, meminfo `64927592 kB` → `63405`, one
  AMD display device → `amd`); `a_virtual_display_adapter_is_not_a_gpu` (`0x1234` only);
  `the_vendor_map_prefers_a_discrete_vendor` (`0x8086` and `0x10de` → `nvidia`);
  `missing_files_are_empty_facts_not_errors`; `macos_and_windows_parsers` (captured `sysctl`,
  `system_profiler` and CIM text); `declares_install_reads_discovery_install`; and, on Linux only,
  `this_box_reports_real_hardware` (non-empty `os_version` and `cpu`, `ram_mb > 0`; reads files,
  spawns nothing).
- **Action**: D6–D9 as written; `run_bounded` becomes `pub(crate)`; `declares_install` added to
  `launch.rs` with the body of `declares_a_source`.
- **Mirror**: `probe_tools`' loop and its `debug!`/`warn!` style (`probe.rs:430-485`), without its
  override tier (`HTUI_TOOL_<NAME>` is an agent-row mechanism, `probe.rs:437-460`).
- **Validate**: `cargo test -p htui-agent --all-features -- --test-threads=1`;
  `grep -rn '"rustc"\|"cmake"\|"docker"\|"gcc"' crates/htui-agent/src/box_probe/*.rs` finds nothing
  outside tests (every tool name lives in `spec.json`).

### Task 4: `htui` — the trigger, the request, the report (D11–D13)
- **Files**: as tabled.
- **Tests first**: `agent_worker.rs` unit tests over `Backend::Memory` with an injected `ProbeEnv`
  and host root, and an unresolvable registry (the H-7 rule) —
  `an_online_swap_probes_a_box_that_was_never_probed` (the row is written, `box_tool` holds the
  fake tools, the agent rows read `missing`, one `BoxProbed` at `UNSOLICITED`);
  `an_online_swap_skips_a_box_probed_at_this_version` (no task, no reply);
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
  four).
  `store_worker.rs`: the `name()` of `ProbeBox`; `try_serve` without a runtime refuses it by name;
  the loop answers `BoxInfo` while a probe task is still running (`R-NF-3`).
  `app/update.rs`: `a_box_probed_reply_lands_on_the_status_line_whatever_its_seq`.
  `tests/box_probe.rs` (harness): `probe_box_through_the_shell_reports_on_the_status_line`.
- **Action**: D11–D13. `on_online` is called immediately after the two `go_online` calls, before
  `runs.sweep`, and awaits nothing.
- **Mirror**: `AgentRuntime::probe` and its tests (`agent_worker.rs:900-953`; offline refusal
  `:5038-5086`, single reply at the requester's address `:5088-5130`). The harness test
  `tests/box_probe.rs` builds its runtime with `with_probe_env` over a fake `PATH` and a fixture
  root, never the host (H-7; amended at fact-check).
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
  `register_box`, no second row, the probe columns intact).
- **Validate**: the workspace gate below, then the live check.

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named in the task. **The
first red test of the milestone is T1's `a_hostname_change_keeps_the_box_id_and_registers`.**

**Store conformance** (both stores): three new cases in T2. **Postgres**: T1's nine registration
cases, the `0005` case, the connect write-back and the mirror prune; T5's four end-to-end cases.
**`htui-agent`**: T3's fixture and parser cases; no case spawns a real tool except through scripts
in a temporary `bin`. **`htui`**: T4's runtime, loop and shell cases.

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| Store `CASES` | 53 | 56 | `crates/htui-core/src/store/conformance.rs:37-91`; `crates/htui-core/tests/mem_store.rs:36-37`; `crates/htui-store/tests/pg_conformance.rs:19` |
| `READ_CASES` | 9 | 9 | `conformance.rs:219-229` |
| Applied migrations | `[1, 2, 3, 4]` | `[1, 2, 3, 4, 5]` | `crates/htui-store/tests/migrations.rs:74` (`migrations_apply_on_a_clean_database`) and `:591` (`the_0004_bump_moves_only_an_untouched_six`, whose `MIGRATOR.run` at `:586` applies everything embedded, so `0005` lands there too) |
| Pending count on a bare database | 4 | 5 | `tests/migrations.rs:623`; `tests/connect.rs:95`, `:108-111` (the second amended at fact-check) |
| `TABLES` | 33 | 33 | `tests/migrations.rs:92-96` |
| `StoreRequest` variants | 63 | 64 | `crates/htui/src/store_worker.rs:89` (counted: 63, see disagreements) |
| `StoreReply` variants | 34 | 35 | `store_worker.rs:617` |
| `.sqlx` files | 227 | about 233 (T1: −1 +3, T2: +3 for the writer and +2 for `boxes`; the exact figure is recorded at close) | `crates/htui-store/.sqlx/` |
| `MIRRORED_TABLES` | 17 | 17 | `crates/htui-store/src/cache/mod.rs:40-58` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — A box registered before `0005` has no stored fingerprint; if a *copy* of its `box.toml` connects first after the upgrade, the copy records its fingerprint and the original machine is then the one that mints a new id | Low | Only possible when a copy already exists and connects first; the original's history stays on the old row, visible in milestone 2's list. Logged as `Copied` with both ids |
| **R-2** — Two processes on a machine with a copied `box.toml` connect at the same instant: each mints its own id, the last `box.toml` write wins, one row is orphaned | Low | `FOR UPDATE` serialises the lookup but not the two mints; the orphan is visible and harmless (no runs claim it) |
| **R-3** — The macOS arms (fingerprint and hardware) are verified by nobody: TOOL-3 blocks the Windows lint here, and no HANDOFF item owns a macOS run | Medium | Pure parsers over captured text are tested everywhere; a failed gather is an empty fact, never an error. Recorded for the main thread to route |
| **R-4** — WSL2 and containers report no GPU and often no machine identity | Certain there | `gpu` is declarable in milestone 2; "no fingerprint" is PRD D1's documented "id alone"; MOD-44 owns container boxes |
| **R-5** — `cl` is off `PATH` outside a Developer prompt, so `msvc` is rarely probed | High on Windows | OQ-9; declared tags in milestone 2; `vswhere` is MOD-16's |
| **R-6** — The registration probe spawns ~16 short children plus the agent probe's handshakes on first launch and after every upgrade | Certain | Off the UI task (D11), each child bounded by `VERSION_TIMEOUT`, never on an ordinary reconnect (D5) |
| **R-7** — An agents section open while the registration probe runs keeps showing the old rows | Medium | The unsolicited reply is addressed to the shell, not the tab; the section re-reads on activation (`wants_requests`), and the status line names what changed |
| **R-8** — Deviations the main thread must record: the adopt-DB-id rule removed (D3), `htui_version`'s meaning (D5), an integrated GPU counts (D7), `hmac` added (D1), `edit_version` added ahead of its writer (D14) | Medium | Listed under "Where the PRD, HANDOFF or tree disagree"; each has a test that fails if the old reading returns |

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
```

`--test-threads=1` is not optional (the keyring fake is process-wide). The Windows clippy line
(`cargo clippy --target x86_64-pc-windows-msvc …`) dies in `ring`'s build script on this box
(TOOL-3, `HANDOFF.md:599`); the `cfg(windows)` code is reviewed by eye and its runtime is MOD-16's.

**Live check on this box (after T5).** Point `htui` at a fresh scratch database, accept the
migrations, then read the row back with
`docker exec htui-postgres psql -U postgres -d <scratch> -c "SELECT os_version, cpu, ram_mb,
gpu_present, gpu_vendor, probed_tags, last_probed_at FROM box"` and `… "SELECT name, version FROM
box_tool ORDER BY name"`. Expected from today's read-only probes: `Ubuntu 24.04.5 LTS`,
`AMD Ryzen 5 7600X 6-Core Processor`, `63405`, `true`, `amd`; tools `bash 5.2.21`, `cargo 1.98.1`,
`cmake 4.2.2`, `docker 29.8.1`, `gcc 13.3.0`, `ninja 1.13.2`, `rustc 1.98.1`,
`vcpkg 2025-12-16…`, `zsh 5.9` and no `clang`, `cl`, `podman`, `fish`, `pwsh`, `powershell` or
`vulkaninfo`; tags `{cmake, docker, gpu, rust, vcpkg}` (no `mingw`: `gcc -dumpmachine` answers
`x86_64-linux-gnu`). Quit and relaunch: `last_probed_at` is unchanged. Change the hostname in
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
      ordinary reconnect costs one read and no probe.
- [ ] Agents are probed after the box; a `missing` agent with an install source is named on the
      status line with `Settings > Agents, i`; nothing is fetched or installed.
- [ ] No tool name or tag name appears in `box_probe/*.rs` outside tests.
- [ ] Store `CASES` 56 in all three places; migrations `[1..5]`; `TABLES` 33; `.sqlx` regenerated
      and `prepare --check` clean; no cache migration.
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
