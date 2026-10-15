# ANA-28 - `heavy_build`: queue switch vs required box capability (concluded, 2026-10-15)

## Summary

R-MCP-3 says that an item carrying the `heavy_build` tag forces `command_run` on. MOD-11
(`docs/decisions/mod/mod-11.md`) read that tag from `item.required_tags`, which is the item's only tag
field, and that field is also the item's required box capabilities (R-ORCH-10). As a result, tagging an
item `heavy_build` both exposed `command_run` and restricted the item to boxes that declare
`heavy_build`. The options were a non-routing item label, exempting `heavy_build` from the box-tag match,
or a phase/queue setting in the TUI.

## Verdict

**A non-routing item label.** The maintainer decided this on the requirements review of 2026-10-15
(`docs/requirements-review-2026-10-15.md`, R-BOX-3), with no separate analysis document:

- items gain free **labels**, kept apart from the required capability tags (R-ENT-5, amended);
- `heavy_build` is a label: it exposes `command_run` and routes nothing (R-MCP-3, amended);
- `heavy_build` leaves R-BOX-3's seeded box vocabulary.

The same review also made the per-phase `command_run` exposure and the per-box limits editable in
Settings (R-MCP-3, R-TUI-8). That covers the third option as well, and MOD-109 owns it.

## Spawned

- **MOD-107** - Item priority and labels; `heavy_build` as a label. It moves the `heavy_build` read in
  MOD-11's exposure check from `required_tags` to labels, and decides what happens to existing items
  tagged `heavy_build` (moving the tag to a label in a migration).
