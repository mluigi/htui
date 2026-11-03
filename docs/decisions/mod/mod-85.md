# MOD-85 - Remaining accent (cyan) uses that do not mean focus or selection (done, 2026-11-03)

**Requirements:** `R-TUI-1`.
**Origin:** MOD-80 (`docs/decisions/mod/mod-80.md` "Carried"; blueprint B-6, review M1/L5).
**Absorbed by MOD-115** (UI redesign review, 2026-10-17) and closed with it; the record is
`docs/decisions/mod/mod-115.md` (M3 acceptance 5, M4 E9).

## What landed (in MOD-115)

Accent now means focus or selection only; each other use was decided and pinned by a buffer test in
colour and under `NO_COLOR`:
- **Became `base`:** the Documents kind column, flow edge labels, divergence `theirs`/`mine`
  (`conflict` stays `error`, `same` `dim`) (M4 L-docs, L-runs, L-forms).
- **Chat** (M3): headers, transcript `you` and notes, the permission strip's digits decided per use;
  the composer marks dim unless focused (M3 review).
- **Requirements form `[value]`** (M3): through `kit::form`.
- **Cursor rows marked by accent alone** (concepts search, workspace switcher, path picker, queue,
  Skills lists and picker): the kit gutter and selection band, legible in monochrome (M3, M4).
- **Skills attach activation field:** the form marker, not `selected` (M4 L-skills).
- **Kept on purpose:** `kit::empty`'s next-step line and the queue's unavailable next step keep the
  accent role as the kit's next-step convention, pinned (M4 review RV1-L3); a pane's border takes its
  tab's accent when focused (M2 D3).
