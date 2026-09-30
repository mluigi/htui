# MOD-23 - Agent registry editing in the Settings agents section (done, 2026-09-30)

**Requirements:** `R-AGT-4` (registry fields, enabled per box), `R-AGT-6` (manual entries allowed),
`R-TUI-8` (Settings: agent registry), `R-NF-3` (off the UI task). `R-AGT-5` held as a constraint:
nothing added branches on an agent's name.
**Origin:** MOD-2 (raised by the maintainer on 2026-09-10 while MOD-2 milestone 7 was in flight).
**Artifacts:** plan
[`.claude/plans/mod-23-agent-registry-editing.plan.md`](../../../.claude/plans/mod-23-agent-registry-editing.plan.md)
(decisions D230-D248, 53 fact-checked claims) and blueprint
`.claude/plans/mod-23-agent-registry-editing.blueprint.md` (D249-D262, findings F-1-F-24). Routed
as **plan** (C4 borderline, 1 criterion fired). Run in a TOOL-7 sandbox (`hr/MOD-23`).
**Decisions:** maintainer, 2026-09-29: route accepted; plan confirmed with OQ-1-OQ-5 on their
recommended answers. 2026-09-30: review L-3 answered "gate chat start", then "gate promote too";
re-review Low-1..Low-3 "fix all three now".
**Commits:** `d2867f3`..`b0f9779` (plan, fact-check, blueprint), `acf0f6c`..`4afb7e9` (T0-T3),
`601919e`..`6fa2d80` (review round), `731bfe4`..`df36258` (re-review round). **Migration `0008_agent_box_user_off`**: the next one is `0009`.

## What shipped

**The per-box switch (T0, OQ-1 answer B).** Migration `0008` adds `agent_box.user_off BOOLEAN NOT
NULL DEFAULT false` (commented; 35 pinned commented columns). The probe keeps owning
`agent_box.enabled` (MOD-2 D50 unchanged); the human vetoes it. `WriteStore::set_agent_box_enabled`
is the only writer of `user_off`, one statement on Postgres that re-derives `enabled` from the stored
probe with `COALESCE(probe->>'status' = 'ready', false)` (blueprint F-1: without the `COALESCE` a
probe with no `status` made `enabled` NULL and the update failed `NOT NULL`). `upsert_agent_box` now
writes `enabled = EXCLUDED.enabled AND NOT agent_box.user_off`, so no probe, `r`, install or login
turns a switched-off row back on. MemStore mirrors both rules with a side set. `AgentSummary` gains
`user_off` (`#[serde(default)]`), filled by Postgres and MemStore; `CacheStore` answers `false`
because `agent_box` is not mirrored. Store conformance `CASES` 96 -> 97; `.sqlx` 288 -> 289.

**The draft rules (T1).** New `crates/htui/src/agent_settings.rs`: `AgentDraft`, one parser per
field, `draft_from_fields`, `check_draft`, `merge_launch`, `apply_draft`, `new_agent`, `draft_of`,
`launch_changed`. `name` is create-only (a renamed seed would return on the next connect through the
seed top-up's `ON CONFLICT (name) DO NOTHING`). `args` are POSIX shell words through `shell-words`
1.1 (OQ-2), which was already locked as a dependency of `agent-client-protocol`: the only
`Cargo.lock` change is `htui`'s edge. `models` is comma-separated and ordered; the default must be one
of them when the list is non-empty. A launch edit is a merge: only `command` and `args` change, every
other key (`env`, `discovery` with MOD-20's `install`, unknown keys) is carried byte for byte.
`env` is never shown, and an invalid merged `launch` is refused with a fixed sentence because serde's
own message would quote an `env` value.

**The writes (T2).** `StoreRequest::{CreateAgent, EditAgent, SetAgentOnBox}` (85 -> 88) are served
in the store loop by `agent_settings::serve`, which mints the id and clock, re-validates the draft
(the section's checks are advisory), and calls `upsert_agent` (MOD-40's compare-and-set: MOD-23 is its
first production caller) or `set_agent_box_enabled`. Every outcome answers one self-naming
`StoreReply::AgentWritten { agents, outcome }` (47 -> 48; MOD-59's recommended shape) carrying the
re-read registry. Refusals are `Failed` with the field's sentence; offline all three are
`REGISTRY_ON_SERVER_ONLY`. A new row copies only `settings` from the highlighted row (OQ-5), so a new
`cli` row keeps its `settings.cli.stream`; `launch` starts blank. `probe_agents_on`'s reply applies
the switch too (D243).

**The pane (T3).** In the Agents section `n` opens a create form and `e` an edit form under the
table (hierarchy's `Field`/`Editor` shape over `TextField`), and `t` flips this box's switch. The
eight-column table is unchanged; the `on this box` cell reads `switched off` ahead of the probe
words. The hint is now two lines (keys, then the note), which moved six snapshots; three are new (110
total). `n`/`e`/`t` are refused while a probe, install or login is in flight, and `n` stays the
decline key of the consent and chooser modals. A plain `Agents` reply never closes the form.

## Review round (`rust-reviewer`: approve with fixes, then re-checked)

- **M-1:** a retry after a Stale reply reverted another writer's change to fields the user never
  touched. Now the untouched fields are reloaded from the re-read row and fields changed on both
  sides are named by label (`601919e`).
- **L-1:** the probe reply's start-of-walk `user_off` window is documented (`08e75dd`).
- **L-2:** switching on an unprobed row says `not probed yet · r probes` (`7a9bab0`).
- **L-3 (maintainer):** a row switched off on this box is refused at **chat start** and at **step
  promotion**, through one helper `refuse_switched_off` (`08e75dd`, `6fa2d80`). The switch therefore
  means "nothing spawns this agent here": orchestrator selection already honoured it.
- **L-4:** a row whose stored model name holds a comma is refused for editing, in the section and in
  `serve` (`ea7cbbb`).
- **L-5:** negative tests for the closed-form answers, a foreign `Failed`, a non-object `launch`
  (`935bbc1`); no bug found.

## Re-review round (`rust-reviewer`: approve; three LOWs fixed by maintainer decision)

- **Low-1:** a step already admitted before a switch-off (a pending fan-out member, or a step a
  crashed walk left pending) still ran on the switched-off agent, because `Kit::driver` built its
  driver without the switch. It now refuses before building anything, with the one shared sentence
  (`agent_worker::switched_off`). **Consequence, by the existing refused-driver path:** the step goes
  `failed` and the **run fails terminally** (`agent spawn failed: agent `<name>` is switched off on
  this box; ...`); switching the agent back on does not revive it, a fresh `StartRun` on the item
  does. Steps not yet created are unaffected: selection skips the agent and the run fails with
  `no_candidate_agent` as before (`731bfe4`).
- **Low-2:** the both-sides clash notice lists labels only while they fit 98 columns, then `+N more`
  (`dd4a0bf`).
- **Low-3:** `refuse_switched_off`'s doc now says a refused promotion leaves the step promoted
  (`awaiting_approval`, `promoted_at` set) with no chat, as the `agent is disabled` refusal already
  did; the test pins that state and that a second promote after switching on opens the chat
  (`df36258`).

## Where the item text was stale

The Settings tab already had a text-input widget (`TextField`, MOD-15) used by Boxes and Hierarchy.
The `name` column is `Fill(1)` since MOD-2 D89, not sized to `amp-acp`. The conclusion (an edit pane,
not a ninth column) held.

## Not done, on purpose

- **ANA-4 §4.6's per-box manual tool path** (OQ-3): a hand-written `probe.resolved` never reaches a
  spawn today (`recorded_launch` answers `None` for `source: manual`). Opened as **MOD-66**. A
  registry row with a literal `launch.command` already covers "manual entries allowed".
- **`settings.weights`** (OQ-4, ANA-21 item 8): left to **MOD-36**, which adds a weights field to
  this form; every edit carries `settings` untouched.
- Deleting a row (retirement is `enabled = false`), the caps editor (MOD-12), box capabilities (MOD-7).

## Gates

`cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -D warnings`, and
`cargo test --workspace --all-features -- --test-threads=1` green on the final tree (91 binaries,
2677 passed, 0 failed), Postgres suites run, not skipped; `cargo sqlx prepare --check` clean against a
scratch database migrated through `0008`. The Postgres gate on the merged host tree runs on the host
after `scripts/hr collect MOD-23`.
