# CLEAN-6 - Runs pane doc says approve takes a typed note (done, 2026-09-29)

**Requirements:** `R-TUI-4`.
**Origin:** the README rewrite, 2026-09-29.

**What was done.** Folded into MOD-39 (T2, commit `634abfd`; `docs/decisions/mod/mod-39.md`),
because the resolution picker changed the same module doc. The key table in
`crates/htui/src/ui/tabs/backlog/detail/runs.rs` now reads "approve (`AnswerGate`) / reject with a
typed note": `a` sends `AnswerGate(Approved)` at once and only `x` opens a note field. The code
already matched `R-TUI-4`, so only the comment changed.
