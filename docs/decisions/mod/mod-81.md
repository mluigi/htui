# MOD-81 - Narrow and wide terminal widths (done, 2026-11-03)

**Requirements:** `R-TUI-1`, `R-TUI-2`, `R-TUI-8`.
**Origin:** the TUI design review of 2026-10-04 (https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH,
findings 4, 14). **Absorbed by MOD-115** (UI redesign review, 2026-10-17) and closed with it; the
record of how each part was built is `docs/decisions/mod/mod-115.md` (M3, M4).

## What landed (in MOD-115)

- **Identifying columns keep their width.** Settings › Agents drops `models`, then `billing`, then
  `transport` below 98 cells so the name keeps 10 (M3, maintainer copy 2026-10-21).
- **A clipped strip says so.** Sub-tab and section chips that do not fit window around the active one
  with `‹`/`›`, and clicks hit what is drawn (M3 kit chip overflow; the Backlog detail at 100 columns,
  M4 Q1).
- **Backlog titles regain room at 60-80 columns.** The status column is sized to the widest status
  mark (`⚠ needs you`, 11 cells) instead of `awaiting_approval` (17), M4 B1.
- **The Prompt pane no longer clips its right edge unmarked.** Rows that do not fit end in `…` (M4
  L-detail, D-24); Settings › Prompt wraps (M3).
- **Every ported surface has an 80-column snapshot** (M3 and M4 lanes), and every cut hint drops whole
  keys (M4 T3 `fit_hints`).

## Carried

- **The 160-column list cap and the wrapping Prompt pane go to MOD-119** (Backlog redesign), which owns
  Backlog's layout default: the shell's `ListSize` has no maximum yet. The Runs grid's 43 cells at 80
  columns go there too.
