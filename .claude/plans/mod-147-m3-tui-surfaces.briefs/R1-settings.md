# Brief R1-settings — MOD-147 M3 review fix round, Settings area

Source: rust-reviewer over `0dc19385..4cb58dbd` (plan `.claude/plans/mod-147-m3-tui-surfaces.plan.md`).
**Confirm each finding against the code before acting**; if one is wrong, say so in `rejected_findings` and skip it.
Tests first for each behaviour change.

## M1 — folder line home source (`crates/htui/src/ui/tabs/settings/hierarchy.rs`)
- `HierarchySection::new()` fills its `home` field from `std::env::var("HOME")` (~`:448`); the worker resolves the
  default with `dirs::home_dir()` (`crates/htui/src/hierarchy.rs:259`). On Windows `HOME` is unset, so the row says
  `unset` while the worker uses `C:\Users\<u>\workspaces\<slug>`. Plan D12 says `dirs::home_dir()`.
- Fix: fill the field from `dirs::home_dir()` in `new()` (this also feeds the picker's `start_dir`); in
  `folder_on_this_box` (~`:161-176`) join the `~` remainder with `/` (components joined by `/`), not `display()`'s
  native separator.
- Make the moved `it__hierarchy__*` snapshots independent of the runner's home: the snapshot harnesses in
  `crates/htui/tests/it/hierarchy.rs` (e.g. ~`:1592`) build the section with `.with_home(Some(<fixed home>))` such
  that the frames keep reading `~/workspaces/graphics (default)`. The snapshots' content must not change; if one does,
  explain why.

## M2 — owner-document write failure keeps the draft (`crates/htui/src/ui/tabs/settings/owner_documents.rs`)
- Today (~`:611-640`, `:678-681`) a failed write (offline `Unreachable`, a refusal, or `document_body` refusing the
  returned body ~`:630`) shows `not saved: …` and drops the kind, title and `$EDITOR` text; an ambiguous failure
  (connection lost after commit) may have landed, and a retry makes a duplicate version.
- Mirror the Backlog documents compose: `crates/htui/src/ui/tabs/backlog/detail/documents.rs:263-280`
  (`write_refused`, `compose.settle`, `may_have_landed`).
  - On a refusal, keep the draft (kind, title, text). The pane says so, and `v`, `n` or Enter on the pane reopens
    `$EDITOR` on the kept text, not an empty or old body.
  - On an ambiguous failure, re-read (`OwnerDocuments`) and compare versions. If the kind's newest version is
    newer than before the write, report it saved. Otherwise keep the draft as above, with the hedge sentence the
    Backlog uses.
- Optional, do it if small: `StoreReply::OwnerDocuments` gains `writable: bool` (`backend.writer().is_some()` in
  `crates/htui/src/owner_documents.rs`), and the pane refuses `n`/`v` offline before `$EDITOR` opens, with the
  existing offline sentence. That touches `crates/htui/src/store_worker.rs` only if the reply variant lives there.
- Tests (`crates/htui/tests/it/settings_documents.rs`):
  - a refused write keeps the draft, and reopening hands the kept text to `$EDITOR`;
  - an ambiguous failure whose re-read shows the new version reports saved;
  - the `STILL_SAVING` and `EDITOR_OPEN` notices;
  - the refused-read state (Low 3).

## Low 2 — "active section only" is pinned
`tests/it/settings_documents.rs:~427` builds `SettingsTab` with one section. Use two sections with Hierarchy **not**
active and assert the outcome is not delivered to it (nothing sent), then active and delivered.

## Low 6 — misleading NotFound
`crates/htui/src/owner_documents.rs:~116`: a workspace with no scope row answers `NotFound { entity: "workspace" }`
("workspace … not found") though the workspace exists. Use the scope row's entity (e.g. `"scope_row"` / the
existing project NotFound shape) so the sentence names what is missing; adjust its test.

## Low 7a — sort allocation
`owner_documents.rs:~400` allocates two `Vec<u8>` per comparison; `str` ordering is byte order already: compare
`row.kind.as_str()`.

## Files owned
`crates/htui/src/ui/tabs/settings/{hierarchy.rs,owner_documents.rs}`, `crates/htui/src/owner_documents.rs`,
`crates/htui/src/store_worker.rs` (only for `writable`), `crates/htui/tests/it/{hierarchy.rs,settings_documents.rs}`,
their snapshots.
