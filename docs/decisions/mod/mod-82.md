# MOD-82 - Shared pane chrome: sub-tabs, detail header, overlays, Settings layouts (done, 2026-11-03)

**Requirements:** `R-TUI-1`, `R-TUI-3`, `R-TUI-7`, `R-TUI-8`.
**Origin:** the TUI design review of 2026-10-04 (https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH,
findings 9, 13, 15, 16). **Absorbed by MOD-115** (UI redesign review, 2026-10-17) and closed with it;
the record is `docs/decisions/mod/mod-115.md` (M1-M4).

## What landed (in MOD-115)

- **One sub-tab widget.** Every tab's secondary navigation is the kit pane's chips in the top border:
  Settings' sections (M3), the Backlog detail's sub-tabs (M4 B1), Skills/Templates (M4 L-skills).
- **The Backlog detail header no longer repeats itself.** The pane title is `KEY · title`; Body keeps
  one meta line (status, tags, priority, version); the `KEY  PREFIX` line went, the prefix being the
  key's own (M4 B1, L-detail).
- **Overlays.** Every modal draws through `kit::modal` with a dimmed backdrop, so none leaves a strip
  of the panes at its edges looking live; concepts search keeps the header row at 60 columns (M4 T2,
  L-overlays).
- **Settings layouts.** One pane with its sections as chips, rows through `kit::form` with a shared
  label column and help under the focused row; Connection's hint and sentence name the same key,
  through `Keys` (M3).
- **Carried from MOD-67:** the Prompt pane header's `n / p · N templates` became a pane hint
  (`n/p template`) and a count in words; the Skills/Templates titles lost their literal
  ` J/K scroll ` (M4 L-detail, L-skills).
