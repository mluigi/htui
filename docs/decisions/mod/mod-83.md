# MOD-83 - Display labels and actionable errors (done, 2026-11-03)

**Requirements:** `R-TUI-1`, `R-TUI-4`, `R-TUI-8`.
**Origin:** the TUI design review of 2026-10-04 (https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH,
findings 10, 12). **Absorbed by MOD-115** (UI redesign review, 2026-10-17) and closed with it; the
record is `docs/decisions/mod/mod-115.md` (M3, M4).

## What landed (in MOD-115)

- **Every shown enum has a display label** (`kit::label`, `kit::status`), with a glyph where it is a
  status, through exhaustive matches so a new variant fails to compile rather than print its name:
  `awaiting_approval` reads `⚠ needs you`, `(app_setting_default)` a word, step ids such as `0.1/0`
  the step's phase and place (`try 2`, `candidate 1`, `judge`), link kinds `blocked by` (M3, M4).
- **One pluralising helper** (`kit::label::plural`): `1 option`, `6 hits`, `8 templates`, `1 char`,
  `applied 2 migrations`; no `(s)` is left on screen (M3, M4).
- **Units and scores in words.** Basis points read `10% (0.1)` (M3); the queue shows dollars, not
  micros (M4 L-overlays); concepts-search scores read `best`/`strong`/`fair`/`weak` relative to the
  top hit (maintainer Q4).
- **Errors name where to fix them.** Concepts search: Qdrant errors `Settings › Qdrant`, store errors
  `Settings › Connection`, a model download `check the network`; the queue's unavailable line
  `Settings › Connection`; every empty or failed state through `kit::empty` with a next step that acts
  where it is shown (M3, M4 review fixes).
- **Exception (maintainer Q3, 2026-10-28):** the Prompt audit's record words (`head_tail`, `no_path`,
  …) stay as written, because htui-core's docs and ANA-5 §12 name them.
