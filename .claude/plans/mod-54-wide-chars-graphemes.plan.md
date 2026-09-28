# Plan: MOD-54 — wide characters and graphemes in the text widgets

> **Status: CONFIRMED 2026-09-28** by the maintainer, with OQ-1/OQ-2/OQ-3 answered as this plan's
> defaults. Cleared for the implementation phase.
>
> **Fact-check, complete:**
> The fact-check ran three passes — a compile probe against `unicode-width 0.2.2` /
> `unicode-segmentation 1.13.3`, a line-by-line tree audit of all 45 navigation claims, and a
> run of the five rows the drafting pass had left `unverified`. **No design decision was
> falsified.** Four things were: the `Cargo.lock` gate, the Windows gate, R-3's DSN claim, and
> two grep expectations. All four are amended in place, with the correction and its evidence
> recorded in the table below. See "Fact-check results".

**Source**: `HANDOFF.md:262-268` (MOD section), verbatim item. "Wide characters and graphemes in the
text widgets (from MOD-7 milestone 2 review). `R-TUI-1`, `R-NF-1`. `TextField` and `TextArea`
(`crates/htui/src/ui/`) count width in `char`s and move the cursor by code point (plan D44 of MOD-7
milestone 2, `TextField`'s module doc), so a CJK or emoji line overruns its column and the cursor
cell can fall off-screen, and `Left`/`Backspace` can split a combining sequence. Measure by display
width (`unicode-width`) and step by grapheme (`unicode-segmentation`) in both widgets together;
declaring those crates is a dependency decision. Found 2026-09-26."

**Requirements**: `R-TUI-1` (`docs/REQUIREMENTS.md:300`, keyboard-driven TUI — a cursor that can fall
off-screen or split a combining sequence is a keyboard-driven defect) and `R-NF-1`
(`docs/REQUIREMENTS.md:345`, "Windows 10+, Linux, macOS" — the two new declarations must build on all
three).

**Complexity**: Small–Medium. One new private module, one `[workspace.dependencies]` entry pair, two
widget files, three comment-only files. **No migration**; nothing under
`crates/htui-store/migrations/` is touched, and none is needed (the item changes rendering, not
stored state). No new `pub` item outside `crates/htui/src/ui/`, no store surface, no request/reply
variant, no `CASES` pin, no `.sqlx` statement, no snapshot.

**Routing**: plan → fact-check → architect → implementer fan-out (T0, then T1 ∥ T2, then T3) →
`rust-reviewer`.

**Numbering**: this item's decisions are **D1…D19**, risks **R-1…R-8**, open questions **OQ-1…OQ-3**,
tasks **T0…T3**. D-numbers restart at D1 because this is a new HANDOFF item, not a milestone of
MOD-7; cite this plan's decisions as "MOD-54 D*n*". MOD-7's D19/D20/D44 and MOD-9's D7/D19/D59 are
cited by their own milestone.

**Base**: `main` at `68c058f` (merge of MOD-7 milestone 4). Every line number below is the file's at
that commit, pre-edit. Toolchain: `rust-toolchain.toml` pins `channel = "1.98.1"` with `rustfmt` and
`clippy`; `edition = "2024"`, `rust-version = "1.98"`; `unsafe_code = "forbid"`.

**Graphify note**: `graphify-out/` does not exist in this checkout. Orientation came from the Gortex
index (which serves the primary checkout); every claim about a file this plan changes was re-read
from `/media/projects/htui-mod-54` and every crate claim from the cargo registry source on disk.

---

## Open questions for the maintainer — **all three answered at CONFIRM, 2026-09-28**

Each was put to the maintainer over the fact-checked plan and each is **answered as the default this
plan adopts**, so implementation is unblocked. The alternatives are kept below as the record of what
was rejected, not as live options.

- [x] **OQ-1 — Does the widget replicate `ratatui`'s halfwidth-katakana correction?** **Answered:
      yes, replicate it** (D2). `ratatui`'s
      buffer measures a grapheme as `UnicodeWidthStr::width(s)` **plus one cell per `U+FF9E`/`U+FF9F`**
      (`ratatui-core-0.1.2/src/buffer/cell_width.rs:34-46`, citing Ruby reline #832 and Microsoft
      Terminal #18087), because `unicode-width` reports those two as zero-width `Grapheme_Extend`
      while terminals draw them as independent halfwidth cells. **Default (D2): replicate it** — four
      lines in the one shared helper, so the widget's arithmetic and the renderer's agree by
      construction. **Alternative:** plain `UnicodeWidthStr::width`, one cell of drift at the end of
      any line containing `ｶﾞ`-style halfwidth katakana. Replicated is recommended only because the
      whole point of the item is that the arithmetic and the drawing cannot disagree.
- [x] **OQ-2 — Does `TextField::len()` change meaning?** **Answered: yes, it becomes a grapheme
      count** (D3). It is `pub` and is documented as a `char`
      count. It is also the mask's dot count and the ` (n)` suffix, so if `len()` stayed a char count
      the mask would draw one dot per grapheme and print a different number. **Default (D3): `len()`
      becomes a grapheme count** and its doc, the `cursor` field's doc, and
      `connection.rs:12`'s "one `\u{2022}` per character and a count" are reworded. **Alternative:**
      keep `len()` as chars and add `grapheme_len()`, so `Debug`'s `len:` and the on-screen count
      report different units — a worse lie than a changed unit.
- [x] **OQ-3 — What does the cursor highlight look like on a two-cell grapheme?** **Answered: the
      first cell only** (D5). `ratatui` writes the
      style onto the first cell and then `reset()`s the continuation cell to `Cell::EMPTY`
      (`ratatui-core-0.1.2/src/buffer/buffer.rs:361-366`, `cell.rs` `reset()` = `*self = Self::EMPTY`),
      so **one styled `Span` cannot reverse both cells of a wide grapheme**. **Default (D5): emit the
      cluster as one `Span` in `theme.selected`** — the glyph is drawn correctly across two cells and
      the first carries the reverse video; the second is left in the buffer's base style. **Alternatives:**
      draw no highlight on a wide cluster (the cursor becomes invisible, worse), or substitute a
      styled one-cell stand-in for the whole cluster (the character stops being readable, worse).

---

## Summary

`TextField::line` (`crates/htui/src/ui/text_field.rs:225-279`) and `TextArea::lines`
(`crates/htui/src/ui/text_area.rs:225-270`) both lay text out in `char`s, both draw a cursor cell
that is one `char` wide, and both move that cursor by code point. `TextArea`'s module doc says so out
loud at `text_area.rs:13-17`, and `a_wide_char_line_is_windowed_by_chars_not_cells`
(`text_area.rs:1015-1028`) pins the broken behaviour deliberately: a 10-cell window over twelve CJK
chars draws ten chars, about nineteen cells. `TextField`'s module doc claims at `text_field.rs:4-7`
that "no `unicode-width` is declared anywhere in the workspace" — true today, and the sentence is one
of the corrections this item makes.

**What changes (D1, D2).** Two crates are declared at `[workspace.dependencies]`, both already
compiled in the graph via `ratatui-core` and `ratatui-widgets`, so `Cargo.lock` does not move. A new
private `crates/htui/src/ui/cells.rs` holds the one place a cell count is computed
(`cell_width(&str) -> usize`) and the one place text is split into clusters
(`graphemes(&str)`), so the two widgets cannot disagree with each other or with `ratatui`.

**`TextField` (D3–D6).** `cursor` becomes a **grapheme index**, which keeps every `± 1` in
`on_key`, keeps `byte_of`'s signature, and turns `len()` into the mask's dot count for free.
`line()` windows in cells: the head is chosen by a backward trim (the largest cluster offset whose
tail still fits) and the tail by a forward accumulation. Both rules reduce **algebraically to
today's** when every grapheme is one cell, which is the byte-identity argument for D17.

**`TextArea` (D7–D15).** The public cursor stays a **byte offset** — `set_cursor` is fed a byte from
a `parse` error at `templates.rs:395` and `:698` and that contract does not move. Only the *stepping*
changes: `Left`/`Right`/`Backspace`/`Delete` move by grapheme, `set_cursor` floors to a grapheme
boundary, `insert` advances past the cluster it joined. `drawn()` becomes cluster-aware; the `follow`
viewport becomes a grapheme-aligned cell column re-derived on every draw; `goal_col` becomes a drawn
cell column; `cursor_line_col` reports a grapheme column.

**The caller (D13).** `templates.rs:439` formats `L{line+1}:C{col+1}` and does not change at all —
only the unit of the number it prints, and only for text that is not one code point per grapheme.

---

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **Declare `unicode-width` and `unicode-segmentation` at `[workspace.dependencies]`; declare neither a new table nor a second copy.** `unicode-width = "0.2"` and `unicode-segmentation = "1.13"`, each with the house comment ("already in the graph as a transitive dependency of `ratatui-core` and `ratatui-widgets`; declaring it adds no compiled crate"), and `unicode-width = { workspace = true }` + `unicode-segmentation = { workspace = true }` in `crates/htui/Cargo.toml`. Only `crates/htui` declares them: the audited consumer list (D18) puts every user of either widget inside `crates/htui/src/ui/`, so no other crate's `Cargo.toml` moves. | **Rejected: a hand-rolled width table.** It would have to carry East Asian Wide/Fullwidth (thousands of ranges), zero-width `Grapheme_Extend` (thousands more), and the emoji ZWJ / VS16 / regional-indicator / script-ligature rules — that is a fork of `unicode-width`'s generated `tables.rs` (22 869 lines in 0.2.2), and every codepoint it omits is silently wrong in a way no test in this repo would catch. **Rejected harder: hand-rolled segmentation.** UAX #29 extended grapheme clusters are not a table: CR/LF pairing, Hangul L/V/T/LV/LVT, Extend, ZWJ, SpacingMark, Prepend and the Regional_Indicator *pairing* rule are a state machine over the whole line, and a wrong answer splits a family emoji. **Precedent**: `zeroize` (MOD-15 M6 D3, "promotes a transitive crate to a declared one and compiles nothing new"), `regex`/`semver` (MOD-2 M5 D47, "Both were already in the graph as transitive dependencies … so this declaration adds no compiled crate"), `bytes` (MOD-20 T5, "this declaration adds a name and not a crate"). **The version is load-bearing**: `"0.2"`, not `"0.1"`, so the workspace resolves the *same* `unicode-width` instance `ratatui-core` uses; declaring `0.1` would compile a second copy. **Gate, corrected by fact-check**: not `git diff --exit-code Cargo.lock`, which is a false alarm. Adding either crate to `htui` adds a line to the `htui` block's alphabetical `dependencies` list, so `Cargo.lock` **does** move — by exactly two lines. The gate is: `git diff Cargo.lock` shows **only** `+ "unicode-segmentation",` and `+ "unicode-width",` inside the `htui` block, **no new `[[package]]` block, and no version bump**. (Proved on a scratch workspace: a new edge into an already-locked package rewrites the dependent's list and adds no package.) |
| D2 | **One private module, `crates/htui/src/ui/cells.rs`, is the only place a cell count is computed.** `mod cells;` (private, no `pub use`) in `crates/htui/src/ui/mod.rs`. It exports to the crate: `pub(crate) fn cell_width(s: &str) -> usize` = `UnicodeWidthStr::width(s)` **plus one per `U+FF9E`/`U+FF9F`** (OQ-1's default, mirroring `ratatui-core-0.1.2/src/buffer/cell_width.rs:34-46`), and `pub(crate) fn graphemes(s: &str) -> impl Iterator<Item = &'a str>` = `s.graphemes(true)`. The sound-mark adjustment is **unconditional and not grapheme-aware** in `ratatui` — it counts every `U+FF9E`/`U+FF9F` in the string, so `"あﾞ"` is 3, not 2. Counting inside the cluster is equivalent here, because both marks are `Grapheme_Extend` and therefore always attach to the preceding cluster; `cells.rs` pins `"あﾞ" == 3` so the equivalence is tested rather than assumed. (UAX #29 **extended** clusters, which is what the crate doc recommends, `unicode-segmentation-1.13.3/src/lib.rs:74-92`). **The width of a cluster is always the width of the cluster's *string*. A per-`char` sum is forbidden** (D15). Its doc names the two rules a reader must not break: measure the string, and use the non-CJK `width()` (D2b). | The whole item is "the arithmetic and the drawing must agree". `ratatui` computes a cell as `CellWidth for str` (`buffer.rs:352`); if the widget computes one cell any other way, the two drift by exactly the difference and the cursor highlight lands on the wrong column. One module is also the only way the two widgets stay in step, which the item text demands ("in both widgets together"). **D2b — use `width()`, never `width_cjk()`**: verified from `tables.rs`, `U+2026 …` is 1 under `width()` and **2** under `width_cjk()`; so is every box-drawing char (`U+2500`, `U+2502`), `U+2588`, `U+00B7`, `U+2014`, `U+2190`, `U+00B0`, and — the two the widget draws itself — `U+2026` (`…`) and `U+2022` (`•`), and `U+FFFD`. `ratatui` uses `width()`. Choosing `width_cjk()` would double the width of the `…` that `TextField::line` itself draws, the `•` mask dots, and the `U+FFFD` stand-in, and move every snapshot in the repo. **The ASCII fast path in `ratatui`'s `cell_width` (`if s.len() == 1 { 1 }`) is not a divergence**: a one-byte string is ASCII or a control char, and no cluster this widget produces is either. |
| D3 | **`TextField::cursor` becomes a grapheme index, `0..=text.grapheme_indices(true).count()`.** `byte_of(index)` keeps its signature and its `map_or(self.text.len(), …)` tail and iterates `grapheme_indices(true)` instead of `char_indices()`. `with_text` seeds `cursor` with the grapheme count. **Every `± 1` in `on_key` is unchanged in shape**: `Left` is still `saturating_sub(1)`, `Right` still `len().min(cursor + 1)`, `Home`/`End` still `0`/`len()`, `Delete` still "remove at `byte_of(cursor)`", `Backspace` still "remove at `byte_of(cursor - 1)` then `cursor -= 1`". **The one thing that must change is `insert`**: `text.insert(byte_of(cursor), c); cursor += 1` is *wrong* for a grapheme index, because a combining mark, a ZWJ or a variation selector merges into the preceding cluster and the count does not grow. `insert` becomes `let at = self.byte_of(self.cursor); self.text.insert(at, c); self.cursor = self.text.grapheme_indices(true).take_while(|(b, _)| *b < at + c.len_utf8()).count();` — the number of clusters that start before the byte just past the insertion, which is 1 for `"a"` + `U+0301` (one cluster, cursor 1 = past the end), 1 for `"ab"` + mark at 1 (`"áb"`, cursor on `b`), and 1 for `"👨"` + `U+200D`. | **A grapheme index keeps the existing arithmetic shape** and is a one-line change to each of six sites; a byte offset breaks every one of them, makes `len()` a byte length (wrong for the mask), and rewrites `line()`'s slicing. `insert` is the one genuine trap and it is called out because a naive port produces a cursor **past the end of the buffer**, which is not a compile error and not caught by any ASCII test. **What `len()` becomes**: a grapheme count, so `Debug`'s `len:` field and the masked ` (n)` suffix both change unit. That is correct, not a regression: the mask draws one `•` per grapheme (D6), so the printed count and the drawn dots must be the same number or the reservation arithmetic lies. ASCII is unaffected: `debug_never_prints_the_text` (`text_field.rs:606`) asserts `len: 6` for `"secret"`, and `a_masked_field_reserves_its_buffer` (`:545`) asserts `field.len() == 68` for an ASCII DSN. `clear_wipes_the_buffer_in_place` (`:579`) asserts `cursor == 0`; a zero is a zero in any unit. |
| D4 | **`TextField::line` windows in cells, by backward trim then forward accumulation.** Let `glyphs: Vec<&str>` be the grapheme slices (`repeat_n("•", len())` when masked), `budget = width.saturating_sub(cell_width(&suffix))` (the suffix is ASCII, so this is today's `suffix.chars().count()`), `cw = cell_width(&glyphs[..cursor].concat())` the cells before the cursor, and `aw = cell_width(glyphs[cursor]).max(1)` (D14 forces a zero-width cluster's cursor cell to one). Then `start` and the ellipsis flag are: `budget <= aw` → `(cursor, false)`; else `cw + aw <= budget` → `(0, false)`; else the **largest** `s <= cursor` with `cw - cell_width(&glyphs[s..cursor]) <= budget - aw - 1`, with `ellipsis = true`. `before = glyphs[start.min(len)..cursor.min(len)].concat()`; `room = budget - ellipsis - cell_width(&before) - aw`; `after` is the longest prefix of `glyphs[cursor+1..]` whose `cell_width` is `<= room`. The invariant is `ellipsis + cell_width(before) + aw + cell_width(after) <= budget <= width`. | The old `start = cursor + 2 - budget` is a *char* arithmetic that has no cell meaning; the backward trim is its cell generalisation and is the only form that keeps the cursor fully visible for any mix of widths. **It reduces to today on ASCII, exactly**: with every width 1, `aw == 1`, so the first branch is `budget <= 1` ⇔ today's `budget < 2`; the second is `cursor < budget` ⇔ today's; the third is "largest `s` with `cursor - s <= budget - 2`" = `s = cursor + 2 - budget` ⇔ today's `else`. And `room` becomes `budget - ell - before.len() - 1` ⇔ today's. This is the algebraic half of D17's byte-identity claim; the mechanical half is the snapshot gate. `glyphs` is `Vec<&str>` rather than `Vec<char>` so a cluster survives to the span intact — a `String` built per-`char` would re-split the combining sequence `ratatui` is about to re-join. |
| D5 | **The cursor highlight is one `Span` carrying the cluster's own text.** `at` is the drawn item under the cursor (`glyphs[cursor]`, or `" "` past the end), styled `theme.selected` while focused and `theme.base` otherwise — exactly the `at.to_string()` / `cursor_style` shape at `text_field.rs:252,263,271`. A two-cell grapheme is therefore **highlighted on its first cell only**; its continuation cell is left in the buffer's base style. The trailing-space cursor cell at end of buffer is unaffected (one cell). Same rule in `TextArea`. Pinned by `a_wide_cursor_cell_highlights_its_first_cell` in both files. | This is forced, not chosen. `Buffer::set_stringn` does `self[(x,y)].set_symbol(symbol).set_style(style)` and then `while x < next_symbol { self[(x,y)].reset(); x += 1 }` (`buffer.rs:360-366`), and `Cell::reset` is `*self = Self::EMPTY` — so the style on the continuation cell is *erased by ratatui itself*. There is no `Span` arrangement that reverses both cells: the glyph is one symbol in one cell. Highlighting the second cell instead (the cursor appearing to the right of the character) reads as "after it", which is a lie about where the insertion point is. The glyph is drawn correctly across two cells regardless, so the only cost of this decision is that half the highlight is missing, and it is a cost ratatui imposes on every wide character in every TUI built on it. |
| D6 | **The mask is one `•` per grapheme, and the ` (n)` suffix is a grapheme count.** `U+2022` is width 1 (verified below) and ` (n)` is ASCII, so the *cell* arithmetic of the masked path is unchanged; only `len()`'s unit moves (D3). A new test `a_masked_field_counts_graphemes_not_code_points` types `U+00E9` and then `U+0065 U+0301` into a masked field and pins `len() == 2`, `"••  (2)"` at width 12. | Keeping the count and the dots in step is the whole reason D3 accepts the `len()` unit change (OQ-2). **Corrected by fact-check: a DSN is not necessarily ASCII.** `Dsn::parse` (`crates/htui-store/src/dsn.rs:119-123`) stores the caller's text **verbatim** — it percent-encodes nothing, decodes nothing and validates no ASCII; `scan` (`:176-215`) rejects only control chars, a bad scheme, an empty host, a bad port and unknown query keys. `sqlx-postgres-0.9.0/src/options/parse.rs:34-39` *decodes* the password back to UTF-8, so a non-ASCII password is a supported configuration, and `url 2.5.8` accepts and normalizes one (it just discards the normalization). So a masked `TextField` can hold non-ASCII and the on-screen ` (n)` count **can** legitimately change. That is correct behaviour, not a regression: the dots and the number stay the same number. The `len() == 68` test is ASCII and does not move. |
| D7 | **The two widgets keep their own units; only `TextArea`'s *stepping* changes.** `TextArea::cursor` stays a byte offset and `set_cursor(byte)` stays a byte setter. `TextField::cursor` stays an index (into graphemes). MOD-54 does **not** make them agree. | `set_cursor`'s contract is load-bearing for two callers that are not the widget: `templates.rs:395` and `templates.rs:698` both pass `error_at(&err).unwrap_or(text.len())` — a **byte** out of `parse` — and the parse error's position must land on the offending token. Converting `TextArea` to an index means rewriting both call sites and `TextArea::cursor()`, `into_text()` round-trip tests and the byte-collision test `set_cursor_floors_inside_a_multibyte_char`. Converting `TextField` to bytes means D3's whole argument in reverse. The smallest change that is *correct* is to leave both units alone and fix only the stepping and the drawing, which is exactly what the item text asks for ("step by grapheme"). |
| D8 | **`TextArea::set_cursor` floors to a grapheme boundary, and `insert` advances past the cluster it joined.** `set_cursor` keeps its `min(text.len())` then walks **down** while `!is_char_boundary(byte)`, then walks **down** again while the byte is inside a grapheme cluster (found with `text.grapheme_indices(true)`'s byte ranges). `insert` keeps `self.cursor += c.len_utf8()` and then, if the cursor is not on a grapheme boundary, advances it to the end of the cluster that now contains it. `previous_boundary` = `self.text[..self.cursor].grapheme_indices(true).next_back().map(|(b, _)| b)`; `next_boundary` = `self.text[self.cursor..].graphemes(true).next().map(|g| self.cursor + g.len())`. `Home`/`End`/`line_start`/`line_end` are unchanged (a `\n` is always a cluster boundary). | `Backspace` and `Delete` stop splitting a combining sequence, which is a named defect in the item. `set_cursor` must floor to a *grapheme* rather than a *char* boundary because a `parse` byte can land inside an emoji ZWJ sequence, and a cursor in the middle of a cluster has no drawn cell to sit on and no defined `Left`/`Right`. `insert`'s advance is the same trap as D3's, with the opposite fix because `TextArea`'s cursor is a byte: a typed combining mark would otherwise leave the cursor between the base and the mark, which is a state nothing else handles. Pinned by `a_typed_combining_mark_joins_the_cluster_before_it` (type `a`, `U+0301`, `b` → text `"áb"`, `cursor_line_col() == (0, 1)`, drawn `"áb"`) and `set_cursor_floors_inside_a_grapheme`. |
| D9 | **`drawn()` becomes cluster-aware and yields `(item, cells)`.** It walks `graphemes(true)`, carrying `col` as an **accumulated cell count**, and per cluster yields: `"\t"` → `n` separate `(' ', 1)` items where `n = TAB_STOP - col % TAB_STOP`; a cluster containing a control char → `(stand_in, 1)` where the stand-in is `0x2400 + c` for C0, `0x2421` for `DEL`, `0xFFFD` for C1. This substitution is also what keeps `ratatui` quiet: `cell_width.rs:34-38` has a `debug_assert!(!self.as_bytes()[0].is_ascii_control())` on its `len() == 1` fast path that **panics in debug and test builds** if a bare control char reaches it unfiltered. `set_stringn` and `set_line` filter control chars first; `Span::styled_graphemes` and `set_symbol`/`set_char` do not. The widgets never hand a raw control char to `ratatui`, and this arm is why; otherwise `(cluster, cell_width(cluster))`. `TAB_STOP` stays 4. The tab arm keeps yielding **n one-cell items**, not one n-cell item, so the span boundaries `text_area.rs:256-266` produce are byte-identical to today's. | Every control char has Grapheme_Break = Control, so each is its own cluster and the old per-`char` arms still line up; the only arm that changes is `c => (c, 1)`, which becomes the measured cluster. Keeping the tab as n one-cell items is required, not cosmetic: `a_tab_draws_as_spaces_to_the_next_stop` asserts `plain(&drawn[0]) == "a   b"` (`:1059`, again at `:1071`) *and*, with the cursor on the tab, that `drawn[0].spans[1].content == " "` (`:1064`) — a one-cell span at index 1, which only holds if the tab's three cells are three separate items. **`col` is now cells, so a wide grapheme before a `\t` moves the stop** — see D14. |
| D10 | **The edge rule: a grapheme that does not fit in the remaining cells is dropped, and the window ends there. Nothing pads, nothing widens, nothing is left to the caller.** A window is built by accumulating whole graphemes while `used + w <= budget`; the first one that would overrun ends the window. This applies to `TextField`'s `after` and `ellipsis`, to `TextArea`'s **non-cursor** rows, and to `TextArea`'s `after` on the cursor row. **The one exception:** if `aw > budget` (a two-cell cluster under the cursor in a one- or two-cell field), the widget draws a single `" "` in `theme.selected` in place of the cluster, so the cursor is still on screen and the line still respects `width`. | "Drop it and show nothing" is what every other clip in these two widgets already does and is what the item's invariant asks for. "Show a space" in the general case would be a lie (the character is there). "Widen the window" and "let the caller clip" are both refusals to answer. The exception matters because the existing contract at `text_field.rs:239-240` is "the cursor is always on screen", and `ratatui` would render a 2-cell `Span` into a 1-cell budget as **nothing at all** (`set_stringn`'s `map_while` fails its `checked_sub` and stops), leaving an invisible cursor. Pinned by `a_wide_grapheme_that_does_not_fit_is_dropped_not_split` (a 10-cell `TextArea` row of twelve CJK chars shows five, not nine-and-a-half) and `a_wide_cursor_in_a_one_cell_field_draws_a_space`. |
| D11 | **`left` is a cell column, re-derived on every draw, and always a grapheme boundary on the cursor's line.** `lines()` computes `col` (the cursor's drawn cell column), then: snap the remembered `from = self.left.get()` **down** to the nearest cluster boundary on the cursor's line (the largest `s <= cursor` with `col - cell_width(clusters[s..cursor]) <= from`); then `left = col` if `col < from`; `left = from` if `col - from + aw <= width`; else the largest `s` with `col - cell_width(clusters[s..cursor]) <= width - aw` (the same backward trim as D4). The result is stored back through the `Cell`. The row iterator then drops any cluster that straddles `left`. `top` is untouched. | `follow` (`text_area.rs:356-364`) already assumes `span` and `at` count the same unit; the moment `at` becomes a cell column the assumption becomes load-bearing, and a `left` that lands mid-grapheme puts half a CJK char on screen — the exact artifact the item exists to remove. Deriving it rather than choosing it is what keeps the "move only as far as you must" behaviour of D19 (`the_viewport_scrolls_to_keep_the_cursor_visible`) while guaranteeing the boundary. **On ASCII this is `follow` verbatim**: every `cell_width` is 1, so the snap is a no-op, the second branch is `col - from + 1 <= width` ⇔ `col < from + width` ⇔ `follow`'s `else`, and the third is `from = col + 1 - width` ⇔ `follow`'s `at + 1 - span`. Pinned by `the_viewport_follows_the_drawn_column` (inverted: it now says *cell* column) and by a new `a_viewport_never_starts_mid_grapheme`. |
| D12 | **`goal_col` becomes a drawn cell column.** `move_lines` no longer reads `cursor_line_col().1`; it computes the cursor's own **cell** column on its line (the same `drawn`-based count `lines()` uses for `col`, D9), takes `goal_col.unwrap_or(that)` as today, and resolves the target by walking the target line's `(item, cells)` pairs accumulating cells and landing on the byte of the **last cluster that starts at or before `goal`**, or at `end` if the line is shorter. `goal_col = Some(goal)` as today. | A goal column exists to mean "stay in the same visual column"; a code-point index does not. And it must be the **drawn** column, not a grapheme column, because `col` in `lines()` is already the drawn one (D20's rule, `a_tab_draws_as_spaces_to_the_next_stop:1078-1079`), and two different notions of "column" in one widget is how the D20 bug happened. Landing on the last cluster that *starts* at or before `goal` guarantees the byte is a grapheme boundary, which is what D8's stepping requires. Pinned by `up_and_down_keep_the_cell_goal_column` (across a line of CJK: goal 4 lands on the third cluster, whose start cell is 4) and by an extension of the existing `a_tab_draws_as_spaces_to_the_next_stop` showing a tab in the target line. The existing `up_and_down_keep_the_goal_column_through_a_short_line:658-663` ("Columns are chars: `é` is one column") is renamed and re-pinned as a cell claim; `é` is one cell as well, so its numbers do not move. |
| D13 | **`cursor_line_col` reports a grapheme column; the call site does not change.** Its doc ("a 0-based **char** column") becomes "a 0-based **grapheme** column". `templates.rs:439` keeps `format!("{EDIT_HINT}  L{}:C{}", line + 1, col + 1)` verbatim. The only edit near it is the comment on `crates/htui/tests/templates.rs:10` that names the unit. | The hint is a **text position**, not a screen cell: it is a line-and-column address into the buffer, it sits next to `L{line}` which is a line number, and it must be comparable with what the arrow keys do. After D8 `Left`/`Right` step by grapheme, so a char counter would jump by 2 on one `Right` across an emoji. A **cell** column would be worse: it is not a text position, it changes meaning when the same text is scrolled, and it would disagree with the `L{line}` unit beside it. So: grapheme, 1-based as printed. `cursor_line_col_counts_chars_not_bytes` is renamed `cursor_line_col_counts_graphemes_not_bytes`; its `ééé\nàbc` expectations do not move (each is one code point *and* one grapheme) and it gains an emoji and a base+combining case. **The public method's own unit change is not a break for any caller**: `templates.rs` is the only non-test caller in the workspace, and it is unaffected for all single-grapheme text, which is all of the repo's fixtures. |
| D14 | **A zero-width cluster occupies no cell, except under the cursor, where it becomes `U+FFFD`.** In the window arithmetic a cluster of width 0 contributes 0 cells. When the cursor sits on one, the widget draws a single `U+FFFD` in `theme.selected` (one cell) in both widgets. `U+FFFD` is the widget's existing vocabulary for "there is a character here with no glyph" (`other_control_chars_draw_visibly` uses it for C1). **Tabs:** a `\t` occupies `TAB_STOP - col % TAB_STOP` cells counted from the **accumulated cell** column, so `"漢\tx"` is `漢` (2) + 2 spaces + `x` at cell 4, drawn `"漢  x"`; `cursor_line_col()` for that `x` is 1 (one grapheme before it). | `ratatui`'s `set_stringn` filters `width > 0` (`buffer.rs:353`), so a zero-width cluster is drawn as **nothing at all** — not the glyph, not the style. A `Span::styled(cluster, theme.selected)` whose cluster is zero-width renders as an empty span and the cursor highlight vanishes; that is a worse failure than the bug being fixed, and it is reachable (a `$EDITOR` body beginning with `U+0301`, which is `Grapheme_Break = Start` and therefore its own cluster). The tab rule is unchanged in kind and *does* change in effect, because `col` was a char count and is now a cell count; today a wide char before a tab is impossible to express. Pinned by `a_zero_width_cluster_at_the_cursor_draws_a_replacement_glyph` and by `a_wide_char_before_a_tab_moves_the_stop`. |
| D15 | **A cluster's width is `UnicodeWidthStr::width` of the cluster's own string. A per-`char` sum is forbidden anywhere in either widget.** The family emoji `U+1F468 U+200D U+1F469 U+200D U+1F467` is **one** cluster; the per-`char` widths are 2 + 0 + 2 + 0 + 2 = **6** (U+200D is width 0, verified) and the string width is **2**. `U+FE0F` is `VARIATION_SELECTOR_16`, width 0 alone, and inside a cluster it flips the base to emoji presentation: `U+2328` alone is width 1, `U+2328 U+FE0F` is width 2 (`ratatui`'s own test `keyboard_emoji`, `buffer.rs:1450`). Pinned by `an_emoji_zwj_sequence_is_one_cluster_two_cells` in both files and by `a_variation_selector_is_measured_with_its_base`. | The crate's own doc states the rule it implements: "the width of a string differs from the sum of the widths of its constituent characters" for "well-formed, fully-qualified emoji ZWJ sequences … have width 2", "emoji modifier sequences … width 2" and "emoji presentation sequences … width 2" (`unicode-width-0.2.2/src/lib.rs:47-52`), and `width_in_str` implements it with the `ZWJ_EMOJI_PRESENTATION` / `EMOJI_PRESENTATION` `WidthInfo` chain (`tables.rs:245-300`). **A second, independent reason the rule exists, found by the fact-check probe**: `UnicodeWidthChar::width('\u{1}')` returns `None`, while `UnicodeWidthStr::width("\u{1}")` returns `1` (`tables.rs:245-263`, `width_in_str`'s `c <= '\u{A0}'` fallthrough). So a per-`char` sum with the natural `.unwrap_or(0)` **undercounts every C0 control and `DEL` by one** — the probe measured `"a\tb"` 3-vs-2, `"a\r\nb"` 3-vs-2, `"\u{1}"` 1-vs-0, `"\u{7F}"` 1-vs-0. It is not only the emoji case. **A note on the framing this plan started from**: it was written into the drafting brief that a family emoji "summing per-char widths gives 8"; the verified figure is **6**, because `U+200D` is width 0. Either way it is wrong, and the right answer is 2 — the plan uses 2 and the test pins 2, not 8 or 6. The `HANDOFF.md` item text says nothing about a per-`char` sum; this correction is to the brief, not to the item. **One thing the probe does *not* license**: across all 17 probe strings the sum of per-*grapheme* widths happened to equal the whole-string width, including the family ZWJ. That is a coincidence of the sample, not an alternative rule — the divergent cases (`\t`, `\r\n`, C0, `DEL`) each fall in their own grapheme and the emoji collapse into one grapheme that already carries the full width. **Only "measure the cluster's string" is the supported rule.** |
| D16 | **The control stand-ins stay one cell; the existing `count = 1` arm of `drawn` is already correct.** Verified from `unicode-width-0.2.2/src/tables.rs` by decoding the packed `WIDTH_ROOT`/`WIDTH_MIDDLE`/`WIDTH_LEAVES` tables along `lookup_width`'s own index path (`tables.rs:176-207`): `U+2400` and `U+2401` = **1**, `U+2421` = **1**, `U+FFFD` = **1**. Control Pictures is East Asian Width Neutral, not Wide, so the existing stand-in is one cell in `unicode-width` exactly as it is in a terminal. No change to that arm, and `other_control_chars_draw_visibly` does not move. | This was the specific thing to check and it passes, which is worth recording rather than assuming: had `U+2400`+ been Wide, every control char in a `$EDITOR` body would have become a two-cell artifact and the item would have needed a second stand-in. The evidence is a mechanical decode, not a recall, and it is in "Verified claims" so the fact-check does not have to redo it. |
| D17 | **For narrow, single-grapheme text the new rendering is byte-identical to the old, and no snapshot may move.** Three independent proofs, all mechanical: (a) **the algebra** — D4's three branches and `room`, and D11's three branches, reduce to today's char arithmetic when every `cell_width` is 1 (worked through in D4 and D11); (b) **the snapshot gate** — after the change, `git diff --exit-code -- 'crates/*/tests/snapshots' 'crates/htui/src/snapshots'` is empty, over all four directories (8 / 28 / 88 / 1 files); (c) **the unit tests** — every existing exact-output expectation in both files stands unchanged, enumerated in "Test plan". A new `ascii_rendering_is_unchanged` in each file inlines today's expected strings as a table so a future change to the arithmetic is visible in the diff. | The two new crates are declared but the widgets are the only thing that changes, so the blast radius is the two files and everything that renders them. The item was found by review, not by a visible crash, which is exactly the class of change that regresses silently; (b) is the one that cannot be argued with. **Nothing in the repo's fixtures exercises the new paths** — no snapshot holds a wide char or a combining mark (verified across all four directories) — which is simultaneously the reason (b) is a clean gate and the reason the new tests in D5/D6/D8/D10/D11/D12/D14/D15 are the only coverage of the fix. |
| D18 | **Not changed, on purpose, and filed separately.** Every other place in the workspace that lays text out in `char`s and has the identical class of bug: `crates/htui/src/ui/diff.rs` (the Templates line diff), `crates/htui/src/ui/top_bar.rs` (the tab bar and the clock), `crates/htui/src/ui/tabs/chat/transcript.rs` (the chat transcript's wrap and clip), `settings::wrapped` and every row renderer under `crates/htui/src/ui/tabs/settings/` and `crates/htui/src/ui/tabs/backlog/detail/`, and the overlays. Also out: soft wrap, bidi, terminals that report a CJK char as one cell, and `crates/htui/src/editor.rs` (its `$EDITOR` round-trip is already byte-exact and has nothing to measure). The HANDOFF item names exactly two widgets; widening it to every renderer would make it untestable and would put nine files in one change. **File one new MOD item** — "display width in every hand-laid-out row" — with this list attached, and cross-link it from MOD-54. `crates/htui/src/ui/diff.rs` is the sharpest of them (a CJK diff hunk overrunning its pane) and should be the first. | Scope. MOD-9's D122 and MOD-7's D122 are the same rule: record the boundary, do not quietly cross it. The chat transcript is the one to be careful about, because a snapshot there *would* move if it were touched, and the item's own hard criterion is that none does. |
| D19 | **Record the corrections, in this item's own words.** The claims that become false and are corrected in the files this item already touches: `text_field.rs:4-7` ("no `unicode-width` is declared anywhere in the workspace" and "Width is counted in `char`s"), `text_field.rs:38` (the struct doc, "counted in `char`s"), `text_field.rs:47` (the `cursor` field doc), `text_field.rs:200` (`len`'s doc), `text_field.rs:218-223` (`line`'s doc), `text_area.rs:13-17` (the module doc's width paragraph), `text_area.rs:214-223` (`lines`' doc), `text_area.rs:48` (`goal_col`'s doc), `text_area.rs:140` (`cursor_line_col`'s doc), `text_area.rs:333-336` (`drawn`'s doc), `connection.rs:12` ("one `\u{2022}` per character and a count"), `connection.rs:186-193` (`Editor`'s `Debug`; the field call is at `:190`), `templates.rs:33` and `:283` and `:249` and `:1203` (the `TextArea` mentions), `templates.rs:439`'s neighbourhood, `tests/templates.rs:10`, and the doc comment on `a_wide_char_line_is_windowed_by_chars_not_cells`, which is inverted rather than deleted (below). | MOD-9's D121 and MOD-7's D121. The doc comment on `a_wide_char_line_is_windowed_by_chars_not_cells` (`text_area.rs:1016-1018`) states the limitation out loud and is the natural place to pin the fixed behaviour: it is **renamed** to `a_wide_char_line_is_windowed_by_cells` and its body rewritten to the new contract — a 10-cell window over twelve CJK chars draws five CJK chars on a non-cursor row and five plus a cursor cell on the cursor row. Deleting it would have removed the only test that names this bug. |

---

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| Declaring a crate already in the graph, with the comment that says so | `zeroize` (MOD-15 M6 D3), `regex`/`semver` (MOD-2 M5 D47), `bytes` (MOD-20 T5) | root `Cargo.toml` (`[workspace.dependencies]`) |
| A private helper module under `ui/` with a `mod` line and no `pub use` | `ui::layout`, `ui::theme` | `crates/htui/src/ui/mod.rs:5-14` |
| An existing unit test that pins a *known limitation*, inverted rather than deleted | — | `text_area.rs:1015-1028` (this item's own, D19) |
| A viewport that scrolls "only as far as it must" through a `Cell`, because `Tab::render` is `&self` | `TextArea::follow`, MOD-9 D19 | `text_area.rs:356-364`; the `Cell` field docs at `:50-52` and `:54` |
| A `pub` accessor's unit stated in its own doc and pinned by a renamed test | `TextArea::len`'s "so `len() == text().chars().count()`" | `text_area.rs:111`, `text_area.rs:881` |
| A renderer's own test proving the buffer already agrees with the arithmetic | `renders_emoji` | `ratatui-core-0.1.2/src/buffer/buffer.rs:1435-1480` |
| Recording a correction rather than leaving a stale doc | MOD-9 D121, MOD-7 D121 | `text_field.rs:4-7` (this item's, D19) |

---

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `Cargo.toml` | edit | T0 | D1's two `[workspace.dependencies]` entries with the "already in the graph" comment |
| `crates/htui/Cargo.toml` | edit | T0 | `{ workspace = true }` for both |
| `crates/htui/src/ui/cells.rs` | **new** | T0 | D2: `cell_width`, `graphemes`, their unit tests |
| `crates/htui/src/ui/mod.rs` | edit | T0 | `mod cells;` (private) |
| `crates/htui/src/ui/text_field.rs` | edit | T1 | D3, D4, D5, D6, D10, D14, D17, D19; every existing test kept |
| `crates/htui/src/ui/text_area.rs` | edit | T2 | D7–D15, D16, D19; `a_wide_char_line_is_windowed_by_chars_not_cells` inverted (D19) |
| `crates/htui/src/ui/tabs/settings/connection.rs` | edit | T3 | D19: the module doc at `:12` and the `Debug` field's unit at `:186-190` |
| `crates/htui/src/ui/tabs/skills/templates.rs` | edit | T3 | D13, D19: the hint's unit in the comment at `:439`, and the `TextArea` mentions at `:249`, `:1203` |
| `crates/htui/src/templates.rs` | edit | T3 | D19: the `TextArea` mentions at `:33`, `:283` |
| `crates/htui/tests/templates.rs` | edit | T3 | D19: the `:10-11` comment naming the hint's unit (the file's eleven `L{n}:C{m}` asserts — `:182,195,204,313,319,397,411,420,438,447,793` — are ASCII and do not move) |

**Not touched, on purpose:** every migration and `crates/htui-store/migrations/`
(`crates/htui-store/tests/migrations.rs` included); `crates/htui/src/ui/diff.rs`,
`crates/htui/src/ui/top_bar.rs`, `crates/htui/src/ui/tabs/chat/transcript.rs` and every other
renderer (D18); `crates/htui/src/store_worker.rs` (its only mention is a doc reference at `:93`);
every other `TextField`/`TextArea` consumer (D18's audit found none that assumes one-char-one-cell —
each hands the widget's `Line` / `Vec<Line>` to a `Paragraph` somewhere: `boxes.rs:712` calls
`TextArea::lines`, whose rows reach the `Paragraph` built at `:588`; in `skills/templates.rs` the
`TextArea` lines go to `Paragraph` at `:1039`, and the `TextField` line's spans are extended at
`:959` and rendered at `:452`/`:461`/`:922`/`:944`/`:1079`); every snapshot; `htui-core`,
`htui-agent`, `htui-orch`,
`htui-store`; `docs/**`, `HANDOFF.md`.

---

## Tasks

**Order.** **T0** alone (everything depends on it). Then **T1 and T2 in parallel**, each in its own
worktree, both after T0 is merged. Then **T3**, which is file-disjoint from all three and could run
in any wave; it is scheduled last so the reworded sentences match the implemented contract.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `Cargo.toml`, `crates/htui/Cargo.toml`, `crates/htui/src/ui/cells.rs` (new), `crates/htui/src/ui/mod.rs` | Wave 0, serial |
| T1 | `crates/htui/src/ui/text_field.rs` | Wave 1, independent of T2 |
| T2 | `crates/htui/src/ui/text_area.rs` | Wave 1, independent of T1 |
| T3 | `crates/htui/src/ui/tabs/settings/connection.rs`, `crates/htui/src/ui/tabs/skills/templates.rs`, `crates/htui/src/templates.rs`, `crates/htui/tests/templates.rs` | Wave 2, file-independent of T0–T2 |

**Intersections, checked.** T0 ∩ T1 = ∅. T0 ∩ T2 = ∅. T0 ∩ T3 = ∅. T1 ∩ T2 = ∅ (`text_field.rs`
and `text_area.rs` are disjoint; neither imports the other today and neither will after D2, which
routes both through `cells.rs`). T1 ∩ T3 = ∅. T2 ∩ T3 = ∅. **No two tasks share a file.**

**Build coupling (the only thing forcing the order).** T1 and T2 both call `crate::ui::cells`, which
T0 creates, so neither compiles against the base tree. There is no other coupling: T1 and T2 add no
`pub` item, no trait method, no `mod` line and no `query!`; T3 is comment-only, so it compiles
against the base tree and against T0–T2. Both T1 and T2 compile `crates/htui`, which is why each runs
in its own worktree. **The one hidden coupling to watch**: `Cargo.lock` is a shared file, so T0 owns
it alone and T1–T3 must not touch it. `Cargo.lock` *is* expected to move — by exactly two lines
inside the `htui` block (D1) — so "T0 owns it" means "no other task adds or removes a line", not
"it stays byte-identical".

### Task 0: the dependency declaration and the shared width helper (D1, D2, D2b)

- **Files**: as tabled. **Tests first.**
- **Tests**, in `cells.rs`: `cell_width` is 1 for `""`→0, `"a"`, `"…"`, `"•"`, `"\u{2500}"`; 2 for
  `"一"`, `"⌨️"`; 0 for a lone `U+0301` and a lone `U+200D`; **2** for `"ｶﾞ"` while
  `UnicodeWidthStr::width("ｶﾞ") == 1`, with the assert message naming `ratatui`'s halfwidth-dakuten
  rule and its citation. `cell_width("あﾞ") == 3` pins that the sound-mark adjustment is
  unconditional, exactly as `ratatui` computes it. `graphemes` splits `a` + `U+0301` (base +
  combining acute, **two codepoints**) into one, `"👨‍👩‍👧"` into one, `"a\r\nb"` into
  `"a"`, `"\r\n"`, `"b"`, `"\u{1}"` into its own cluster, and `""` into none. One test named for D2b
  asserts `cell_width("…") == 1` **and** that `UnicodeWidthStr::width_cjk("…") == 2`, and does the
  same pair for `"•"` and `"\u{FFFD}"`, so the choice of the non-CJK `width()` is pinned rather than
  remembered.
- **Action**: the two `[workspace.dependencies]` entries with the house comment, the two
  `{ workspace = true }` lines, `cells.rs`, `mod cells;`. Nothing else.
- **Validate**: `cargo test -p htui --all-features --lib -- --test-threads=1`;
  `git diff --stat -- Cargo.lock` must be **exactly 2 insertions, 0 deletions**, both inside the
  `htui` block, no new `[[package]]` and no version bump (D1 — *not* `--exit-code`, which will be
  red and correctly so); `cargo tree -p unicode-width -i` must show **one** instance shared with
  `ratatui-core`. **No Windows gate** — `cargo check -p htui --target x86_64-pc-windows-msvc`
  exits 101 in `ring`'s build script on this box and proves nothing about this change (R-4).

### Task 1: `TextField` (D3, D4, D5, D6, D10, D14, D17, D19)

- **Files**: `crates/htui/src/ui/text_field.rs` only.
- **Tests first** (all in the file's `mod tests`, beside the ones they sit next to):
  - `a_wide_grapheme_is_two_cells` — `TextField::with_text("漢字")` at width 6 draws `漢字` and a
    trailing cursor space; the drawn row's cell width is 6, not 4.
  - `the_window_counts_cells_not_chars` — twelve CJK chars at width 10: the drawn row is five chars
    and ten cells, leading `…`, cursor on the sixth.
  - `a_wide_cursor_cell_highlights_its_first_cell` (D5) — the cell at the cluster's first column is
    `Modifier::REVERSED`; the second is not; the message names `Cell::reset`.
  - `left_and_right_step_by_grapheme` (D8's rule in the field's own shape) — over `"áb👨‍👩‍👧c"`,
    `Right` moves one cluster at a time and `End` is 4; a `Right` from the emoji lands after it, not
    inside it.
  - `backspace_removes_a_whole_grapheme` — `Backspace` on `"áb"` gives `"a"`, not `"a"` plus an
    orphaned mark.
  - `a_typed_combining_mark_joins_the_cluster_before_it` (D3's `insert`) — type `a`, `U+0301`, `b`
    into a fresh field: text `"áb"`, `len() == 2`, `cursor == 2`; the last assertion is the one that
    catches the naive `cursor += 1` port.
  - `a_wide_grapheme_that_does_not_fit_is_dropped_not_split` (D10).
  - `a_wide_cursor_in_a_one_cell_field_draws_a_space` (D10's exception).
  - `a_zero_width_cluster_at_the_cursor_draws_a_replacement_glyph` (D14).
  - `a_masked_field_counts_graphemes_not_code_points` (D6).
  - `ascii_rendering_is_unchanged` (D17c) — a table of today's exact expected strings, inline:
    `"abcdefghijkl"` @10 → `"…efghijkl"`; `"abc"` @10 → `"abc"`; cursor at 5 of `"abcdefghijkl"` @10
    → `"abcdefghij"`; masked `"hunter2"` @12 → `"•••••••  (7)"`; masked `"0123456789ab"` @8 →
    `"…•  (12)"`. Plus a cell-width assertion on every row.
- **Kept, unchanged** (D17c): `inserts_at_the_cursor` (its `é` case still holds — one cluster, one
  index), `backspace_and_delete_at_both_ends`, `home_and_end_move`,
  `a_control_char_is_swallowed`, `enter_esc_tab_outcomes`, `the_window_leads_with_an_ellipsis_at_width`,
  `the_cursor_cell_is_selected_only_when_focused`, `a_masked_field_renders_dots_and_a_count`,
  `text_is_none_when_masked`, `take_empties_the_field`, `a_masked_field_reserves_its_buffer`
  (`len() == 68`), `clear_wipes_the_buffer_in_place`, `debug_never_prints_the_text` (`len: 6`).
- **Action**: D3 (including the `insert` recompute), D4, D5, D10, D14, D19's seven doc sites.
- **Validate**: `cargo test -p htui --all-features --lib -- --test-threads=1`;
  `git diff --exit-code -- 'crates/*/tests/snapshots' 'crates/htui/src/snapshots'`.

### Task 2: `TextArea` (D7–D16, D19)

- **Files**: `crates/htui/src/ui/text_area.rs` only.
- **Tests first**:
  - `a_wide_char_line_is_windowed_by_cells` — **the inverted existing test** (D19): a 10-cell window
    over twelve CJK chars draws five on a non-cursor row and `five + cursor cell` on the cursor row,
    and each row's cell width is `<= 10`. The old body and its doc comment are replaced, not deleted.
  - `a_wide_grapheme_that_does_not_fit_is_dropped_not_split` — the same twelve chars, cursor in the
    middle: nothing is half-drawn at either edge.
  - `a_wide_cursor_cell_highlights_its_first_cell` (D5).
  - `left_and_right_step_by_grapheme` (D8) and `backspace_removes_a_whole_grapheme`.
  - `a_typed_combining_mark_joins_the_cluster_before_it` (D8's `insert`).
  - `set_cursor_floors_inside_a_grapheme` (D8) — a byte inside `"👨‍👩‍👧"` floors to its start.
  - `a_viewport_never_starts_mid_grapheme` (D11) — cursor at the end of a long CJK line, a 5-cell
    window: the first visible column is a cluster start, and the cursor's two cells are whole.
  - `up_and_down_keep_the_cell_goal_column` (D12) — over CJK and over a line with a `\t`.
  - `a_zero_width_cluster_at_the_cursor_draws_a_replacement_glyph` (D14) and
    `a_wide_char_before_a_tab_moves_the_stop` (D14) — `"漢\tx"` draws `"漢  x"`, and
    `cursor_line_col()` is `(0, 1)`.
  - `an_emoji_zwj_sequence_is_one_cluster_two_cells` and `a_variation_selector_is_measured_with_its_base`
    (D15) — five family emoji in a 10-cell window draw five; `"⌨️"` is two cells and `"⌨"` is one.
  - `cursor_line_col_counts_graphemes_not_bytes` (D13) — the renamed existing test, same `é`/`à`
    numbers, plus a family emoji and a base+combining case.
  - `a_wide_grapheme_at_the_row_edge_does_not_overrun` — every drawn row's `cell_width <= width`, over
    a table of mixes (CJK, emoji, tabs, control stand-ins, ASCII).
  - `ascii_rendering_is_unchanged` (D17c) — inline today's exact strings: `["line1","line2","line3"]`,
    `["e0 "]`, `["vwxyz0123 "]`, `"abcdefghij"`, `["a   b","    c","abcd    e"]`, `["  b"]`,
    `"a\u{2401}b\u{2421}\u{fffd}c"`, `["c","d "]`, and `up_and_down_keep_the_goal_column_through_a_short_line`'s
    `(1, 2)` / cursor 7.
- **Kept, unchanged**: every other test in the file, including all the boundary-crossing ones
  (`left_and_right_cross_line_*`, `delete_at_line_end_joins_the_next_line`, `page_down_moves_by_the_page_and_clamps`,
  `set_cursor_past_the_end_clamps_to_len`, `the_viewport_scrolls_to_keep_the_cursor_visible`,
  `the_window_keeps_the_cursor_row_visible`, `a_long_other_row_is_cut_at_the_window_edge`,
  `other_control_chars_draw_visibly`, `lines_takes_shared_self`, `debug_prints_lengths_not_text`'s
  `len: 11`).
- **Action**: D7, D8, D9, D10, D11, D12, D13, D14, D15, D16, D19's nine doc sites. `follow` is
  replaced by the grapheme-aligned derivation; `len()` and `Debug` are **not** touched.
- **Validate**: `cargo test -p htui --all-features --lib -- --test-threads=1`;
  `git diff --exit-code -- 'crates/*/tests/snapshots' 'crates/htui/src/snapshots'`.

### Task 3: the call-site and doc corrections (D13, D19)

- **Files**: as tabled. **Comment-only** — no executable line changes, so its first commit is the
  corrected comments and its `cargo check` gate is trivially green; the value is that the diff is
  small enough to review by eye.
- **Tests**: none new. `crates/htui/tests/templates.rs`'s eleven `L{n}:C{m}` asserts
  (`:182,195,204,313,319,397,411,420,438,447,793`) and the two
  templates snapshots' hint rows (`templates__edit_help.snap:33` `L1:C1`,
  `templates__unknown_placeholder_cursor.snap:33` `L3:C1`) are re-run unchanged as the regression
  check that D13's unit change is invisible for ASCII.
- **Action**: D19's `connection.rs`, `templates.rs` (both files), `tests/templates.rs` and
  `templates.rs:439`-neighbourhood edits; the new MOD item's text is the **coordinator's**, not this
  task's (D18).
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`;
  `git diff --exit-code -- 'crates/*/tests/snapshots' 'crates/htui/src/snapshots'`.

Every implementer prompt carries: **TDD — the first commit is the failing tests named in the task**;
no `unsafe` (`unsafe_code = "forbid"` at the workspace level); stage your own paths only;
`Cargo.lock` is T0's alone — T1–T3 add no line to it; **commit incrementally** (uncommitted subagent
work does not survive the session and there is no stash on a shared tree); verify with
`--test-threads=1` on the real tree after your merge, not in your lane.

---

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named above. **The first
red test of the item is T0's `cell_width_is_two_for_a_halfwidth_katakana_with_dakuten`** (it cannot
compile until the dependency exists, so T0's red is "no such crate", which is the honest red for a
dependency task) and, in Wave 1, **T1's `a_typed_combining_mark_joins_the_cluster_before_it`** and
**T2's `set_cursor_floors_inside_a_grapheme`**, one per lane.

**The regression net is D17 and it is three-layered**, because the fix is invisible on every fixture
in the repo: (a) the snapshot gate over all four snapshot directories, which is the layer that cannot
be argued with; (b) the existing exact-output unit tests, every one of which is enumerated above and
none of which may be edited; (c) the new `ascii_rendering_is_unchanged` tables, which turn "nothing
moved" into a diff someone can read.

**The fix's own coverage** is the eleven new `TextField` tests and the thirteen new `TextArea` tests
above, one per decision that is a behavioural contract: D5 (both), D6, D8 (three), D10 (three), D11,
D12, D13, D14 (three), D15 (three), D17c (two). D16 needs no new test because its answer is "no
change" and `other_control_chars_draw_visibly` already pins it.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — an ASCII rendering regression slips through | Medium; this is the default failure mode of a change that no fixture exercises | Three independent gates (D17). The snapshot gate is the backstop and is a single `git diff --exit-code` over 125 files. Both new `ascii_rendering_is_unchanged` tables inline today's expected strings so a change to them is visible in the review diff rather than hidden in a rewritten test. The `insert` recompute (D3) is called out in the task text as the one place a naive port passes every ASCII test and breaks on combining input. |
| **R-2** — the `left`/`top` `Cell` gets stuck because it is only updated on draw | Medium; `lines()` mutates state through `&self` (D19) and a widget that is not drawn for a while carries a stale `left` | D11 makes the stored value a **cell column**, never an offset into a line that may have changed: it is re-snapped down to a cluster boundary on the *current* cursor line at the top of every draw, and the window is then re-derived. So a stale value can only make the window move too far or too little by whole cells, never split a grapheme or index off the end. `top` is a line index and is untouched. Pinned by `a_viewport_never_starts_mid_grapheme` and by a new case that edits, does **not** draw, edits back, and then draws. |
| **R-3** — the masked `len()` meaning change ripples into a test or a DSN | Low for tests; **real** for a non-ASCII DSN; certain for the docs | **Amended by fact-check: the plan's original mitigation was false.** `Dsn::parse` stores the caller's text verbatim (`dsn.rs:119-123`) and `sqlx` decodes a percent-encoded password to UTF-8, so a masked DSN field **can** hold non-ASCII and the on-screen ` (n)` **can** move. That is the correct outcome — the dots and the number must agree (D6) — and it is why `a_masked_field_counts_graphemes_not_code_points` matters: it makes the change visible instead of silent. `a_masked_field_reserves_its_buffer`'s `68` and `a_masked_field_renders_dots_and_a_count` are ASCII and do not move, but **no existing test would catch a non-ASCII DSN regression**, so the new test is the only coverage. Five doc sites state the old unit and are corrected in T3 (D19): `text_field.rs:38` (struct doc), `text_field.rs:47` (the `cursor` field), `text_field.rs:200`, `connection.rs:12`, `connection.rs:186-193`. The `Zeroizing` 256-byte reservation is untouched and its test's intent holds. |
| **R-4** — the Windows build (`R-NF-1`) | **Falsified as a gate by fact-check**; the underlying risk is low | `cargo check -p htui --target x86_64-pc-windows-msvc` **cannot run on this box**: it exits `101` in `ring v0.17.14`'s build script — `error occurred in cc-rs: failed to find tool "lib.exe": No such file or directory` — plus an `aws-lc-sys` C failure (`unknown type name 'pthread_rwlock_t'`). The log never reaches a single `crates/htui` line, so it proves nothing about this change. This is a **missing MSVC C toolchain**, not a linker gap and not a code error, and the README already says so (`README.md:461-462`: "The rest of the workspace does not cross-check from Linux: `sqlx`'s `ring` dependency builds C code and wants an MSVC-compatible compiler"), which is why its Windows gate at `:451` is scoped to `-p htui-agent` only. **So `R-NF-1` is argued structurally here, not gated**: both crates are `#![no_std]`, carry `build = false` with no `build.rs` on disk, have no `[target.*]` section, and forbid `unsafe`. They are pure-Rust and platform-independent, and the change is two function calls in `crates/htui/src/ui/`. **The real Windows verification is MOD-16's** (`README.md:445-451`); this plan records the claim as *not verifiable from this box* rather than asserting it green. |
| **R-5** — `width_cjk` chosen by accident, doubling the width of `…` and every box-drawing char | Low, but catastrophic and invisible until a frame renders | D2b pins it with a test that asserts **both** numbers, so the choice is in the diff. The `width()` import is the only width import in either file. |
| **R-6** — a `Span` is built per `char` somewhere, re-splitting a cluster before `ratatui` re-joins it | Medium; the current code already builds `Vec<char>`/`String` per char | D4's `glyphs` is `Vec<&str>` and D9's `drawn` yields `&str` items. A test that renders a base+combining pair and a ZWJ sequence through the real `Buffer` (both files' `drawn_text`/`window` helpers already render through `Paragraph`) catches it: `ratatui` would place two cells where one is meant. |
| **R-7** — the item is quietly widened to the other renderers | Low, but the pressure is real: `diff.rs` has the same bug and is one file away | D18 lists them by path and says to file one new MOD item instead. The reviewer's check is that the diff touches only the ten files in "Files to Change". |
| **R-8** — the plan's own claims about `unicode-width` are wrong | Low; every one was decoded from the crate's tables, not recalled | "Verified claims" carries the decode method for each (`lookup_width`'s own index path over `WIDTH_ROOT`/`WIDTH_MIDDLE`/`WIDTH_LEAVES` at `tables.rs:176-207`), so a fact-checker can re-run the same three lines of Python and get the same numbers. The one claim I could **not** verify is that the workspace resolves the *same* `unicode-width` instance `ratatui-core` uses; that row is marked `unverified` with `cargo tree -p unicode-width -i` as the check. |
| **R-9** — deviations the main thread must record | — | D13 changes a `pub` method's documented unit (`cursor_line_col`); D3 changes a `pub` method's documented unit (`TextField::len`); D11 replaces `follow`; D2 adds a private module. `HANDOFF.md`'s pins (snapshot count 88, `CASES` counts, `.sqlx` 268, `Cargo.lock` clean) **do not move** and no doc pin needs updating. |

---

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
# D1: no new crate, and a lockfile that moves by exactly two lines
git diff --stat -- Cargo.lock                # expect exactly 2 insertions, 0 deletions
git diff -- Cargo.lock                       # expect only + "unicode-segmentation", + "unicode-width",
                                              #   both inside the `htui` block; NO new [[package]], NO version bump
cargo tree -p unicode-width -i          # one instance, shared with ratatui-core
cargo tree -p unicode-segmentation -i
# D17: the hard criterion — no snapshot moves, in any of the four directories
git diff --exit-code -- 'crates/*/tests/snapshots' 'crates/htui/src/snapshots'
ls crates/htui/tests/snapshots | wc -l          # 88
ls crates/htui-core/tests/snapshots | wc -l     # 28
ls crates/htui-agent/tests/snapshots | wc -l    # 8
ls crates/htui/src/snapshots | wc -l            # 1
# D19: no stale claim about the old limitation survives.
# Corrected by fact-check: the first two greps as originally written each found ONE
# match, not two, and `widthed_by_chars` matched nothing at all. These are the
# patterns that actually catch every stale site.
# Corrected again at implementation: the gates below name the two *stale sentences*, not the
# string "unicode-width". After this item the crate is declared and its module doc explains how
# it is used, so a grep for the bare name would match the correct new text.
grep -rn 'no `unicode-width` is declared' crates/ --include='*.rs'      # expect nothing
grep -rn '(no `unicode-width`)' crates/ --include='*.rs'               # expect nothing
grep -rn 'windowed_by_chars_not_cells' crates/ --include='*.rs'         # expect nothing
grep -rn 'one char is otherwise one cell\|counted in `char`s' crates/htui/src/ui/  # expect nothing
# No per-`char` width sum in either *widget*. `cells.rs` names `UnicodeWidthChar` five times on
# purpose: in the doc saying why it is not used, and in two tests proving the sum is wrong.
grep -rn 'UnicodeWidthChar' crates/htui/src/ui/text_field.rs crates/htui/src/ui/text_area.rs  # expect nothing
# R-4: R-NF-1 is NOT gateable from this box (see R-4). Do not run a windows check
# that dies in ring's build script and reports nothing about this change.
```

The four `D19` greps are *post-change* checks: each is expected to be empty once the
item lands. On the **base** tree they return `text_field.rs:4`, `text_area.rs:1015`,
`text_area.rs:216`, and nothing respectively — which is what makes them a real gate
rather than a tautology. `text_area.rs:13` is worded ``(no `unicode-width`)`` rather than
the `text_field.rs:4` phrasing, which is why a grep for one sentence caught only one of
the two stale sites; the corrected pattern matches just `unicode-width` and catches both.

`--test-threads=1` is the repo's standing rule (the keyring fake is process-wide). Merge order T0 →
{T1, T2} → T3; after each merge re-run the gates of every crate the merged task touched, on the
**merged** tree. After T1 or T2 that means `cargo test -p htui --all-features -- --test-threads=1`
and the snapshot `git diff --exit-code`. A compile probe found no exhaustive match over
`TextField`, `TextArea`, `cursor_line_col` or `len()` anywhere outside the two widget files and the
three T3 files, so T1 and T2 cannot break a sibling task's build.

**Live check (after T2).** `cargo run -p htui -- --demo`, then a field: type a CJK string by
pasting (`xclip`/`$EDITOR` path, or type with an IME if one is running) and watch the dots and the
cursor track the cells; `Home`/`End`/`Left`/`Right` step one visible character at a time. In the
Templates editor (`e` on a template, or `b` on a Settings → Boxes quirks row): paste a line of
`👨‍👩‍👧👨‍👩‍👧` and one of `é`; `Up`/`Down` should hold the same visual column; a line starting with a
bare combining mark should show `�` under the cursor rather than an invisible cursor; the hint's `C{n}`
should step by one per visible character. The demo fixtures hold no wide text, so this is the only
way to see the fix; MOD-54 has no snapshot that shows it, by design (D17).

---

## Acceptance

- [ ] `unicode-width` and `unicode-segmentation` are declared at `[workspace.dependencies]` and in
      `crates/htui/Cargo.toml`, each with a comment recording that it was already in the graph;
      `cargo tree -p unicode-width -i` shows one instance shared with `ratatui-core`; and
      `git diff --stat -- Cargo.lock` is **exactly 2 insertions, 0 deletions**, both additions
      inside the `htui` block's `dependencies` list, with no new `[[package]]` block and no
      version bump. *(`git diff --exit-code Cargo.lock` is the wrong gate — it will be red, and
      legitimately so; see D1.)*
- [ ] `R-NF-1` is **argued, not gated**: the record states that a Windows cross-check of
      `-p htui` cannot run on this box (`ring`'s build script wants the MSVC C toolchain,
      `lib.exe`) and that MOD-16 carries the actual verification. No green Windows claim is made.
- [ ] **No snapshot moves**: `git diff --exit-code -- 'crates/*/tests/snapshots' 'crates/htui/src/snapshots'`
      is empty across all four directories (8 / 28 / 88 / 1 files).
- [ ] Every existing exact-output test in `text_field.rs` and `text_area.rs` is present and
      unedited, including `the_window_leads_with_an_ellipsis_at_width` (`"…efghijkl"`, `"abc"`),
      `a_masked_field_renders_dots_and_a_count` (`"•••••••  (7)"`, `"…•  (12)"`),
      `a_masked_field_reserves_its_buffer` (`68`), `debug_never_prints_the_text` (`len: 6`),
      `the_viewport_scrolls_to_keep_the_cursor_visible` (`["e0 "]`, `["vwxyz0123 "]`),
      `a_long_other_row_is_cut_at_the_window_edge`, `a_tab_draws_as_spaces_to_the_next_stop`,
      `other_control_chars_draw_visibly`, `lines_takes_shared_self`, and all the boundary-crossing and
      `page_down` tests.
- [ ] A ten-cell window over twelve CJK chars draws five CJK chars and no more, on both the cursor
      row and a non-cursor row, and every drawn row's cell width is `<= width` — the inverted
      `a_wide_char_line_is_windowed_by_cells` passes and the old `…chars_not_cells` test is gone.
- [ ] A family emoji is one cluster and two cells, measured as a string; `⌨️` is two cells and `⌨` is
      one; no per-`char` width sum exists in either file (`grep -rn 'UnicodeWidthChar' crates/htui/src/ui/`
      expects nothing).
- [ ] `Left`, `Right`, `Backspace` and `Delete` move and remove **whole** graphemes in both widgets;
      typing a combining mark, a ZWJ or a variation selector joins the cluster before it and leaves
      the cursor after the whole cluster, never past the end of the buffer and never inside one.
- [ ] `TextArea::set_cursor` still accepts a byte offset and still floors to a boundary; both
      `templates.rs` call sites are unchanged and a `parse` error still lands on its token.
- [ ] The cursor is fully visible at any width ≥ 1 in both widgets: a two-cell cluster under the
      cursor highlights its first cell, a two-cell cluster in a one-cell field draws a one-cell
      styled space, and a zero-width cluster under the cursor draws `U+FFFD`.
- [ ] `left` is a cell column, re-derived and re-snapped on every draw, and never leaves a grapheme
      half visible at the window's first column; `Up`/`Down` hold the same **visual** column across
      lines of different widths, with tabs counted as the cells they draw.
- [ ] The Templates hint's `C{n}` is a grapheme column, `L{n}` is a line number, and the call site at
      `templates.rs:439` is unchanged; `crates/htui/tests/templates.rs`'s eleven `L{n}:C{m}` asserts
      and the two hint rows in the templates snapshots are unchanged.
- [ ] `TextField::len()` is a grapheme count; the mask draws one `•` per grapheme and prints that
      same number; `TextArea::len()` is still a char count and `Debug`'s `len:` fields are unchanged.
- [ ] `grep` for the four stale sentences (above) returns nothing; the new follow-up MOD item for
      `diff.rs`, `top_bar.rs`, the transcript and the hand-laid-out rows is filed, not folded in.
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-features --all-targets
      -- -D warnings` are clean; the workspace test suite is green with `--test-threads=1`.

---

## Where the item, the brief and the tree disagree

1. **The drafting brief said a family emoji "summing per-char widths gives 8".** The verified
   per-`char` sum is **6**: `U+1F468`, `U+1F469`, `U+1F467` are each 2 and each of the two `U+200D` is
   0 (`tables.rs`, decoded). The string width is **2**. D15 and its test use 2. This corrects the
   brief this plan was drafted from, not the `HANDOFF.md` item — that text never states a per-`char`
   sum.
2. **The item's `R-TUI-1` citation is about keyboard-driven input**; the actual user-visible symptom
   is a cursor cell off-screen or a combining sequence split by `Backspace`. Both are keyboard-driven
   defects, so the citation holds, but the acceptance criteria above are the ones that actually test
   it and the requirement is not a substitute for them.
3. **`text_field.rs:4-7` claims "no `unicode-width` is declared anywhere in the workspace"** and
   `text_area.rs:13` repeats it. Both are true today and both are the sentences D19 corrects — this
   is the doc debt the item names when it cites "plan D44 of MOD-7 milestone 2, `TextField`'s module
   doc" as the source.

---

## Verified claims

`claim | verdict | evidence`. Every crate claim was read from the cargo registry source on disk
(`~/.cargo/registry/src/index.crates.io-*/`); every tree claim was read from
`/media/projects/htui-mod-54` at `68c058f`. Width numbers come from decoding
`unicode-width-0.2.2/src/tables.rs` along `lookup_width`'s own index path (`tables.rs:176-207`):
`cp >> 13` into `WIDTH_ROOT`, `(cp >> 7) & 0x3F` into `WIDTH_MIDDLE`, `(cp >> 2) & 0x1F` into
`WIDTH_LEAVES`, then `(byte >> (2 * (cp & 3))) & 3`; a value `< 3` is the width, `>= 3` falls to the
match arm.

| claim | verdict | evidence |
|---|---|---|
| `unicode-width 0.2.2` and `unicode-segmentation 1.13.3` are already in `Cargo.lock` | verified | `Cargo.lock`: `ratatui-core` 0.1.2 deps include `unicode-segmentation`, `unicode-truncate`, `unicode-width`; `ratatui-widgets` 0.3.2 deps include `unicode-segmentation`, `unicode-width` |
| The root `Cargo.toml` has the "already in the graph" precedent | verified | `zeroize` ("promotes a transitive crate to a declared one and compiles nothing new"), `regex`/`semver` ("Both were already in the graph as transitive dependencies … so this declaration adds no compiled crate"), `bytes` ("this declaration adds a name and not a crate") |
| `U+2400` (Control Pictures base) is width **1** in `unicode-width 0.2.2` | verified | decoded `tables.rs` via `lookup_width`'s path: `WIDTH_ROOT[0x2400>>13=1] = 0x01`, `WIDTH_MIDDLE[0x01][8] = 0x02`, `WIDTH_LEAVES[0x02][0] = 0x55`, `0x55 & 3 = 1` |
| `U+2401` is width 1; `U+2421` is width 1; `U+FFFD` is width 1 | verified | same decode: `U+2401` leaf byte `0x55`; `U+2421` → `WIDTH_LEAVES[0x02][8] = 0x55` → 1; `U+FFFD` → `WIDTH_ROOT[7]`, `WIDTH_MIDDLE[0x05][63] = 0x02`, `WIDTH_LEAVES[0x02][31] = 0x55`, `(0x55 >> 2) & 3 = 1` |
| `U+4E00` (CJK 一) is width 2; `U+FF21` (fullwidth A) is 2; `U+300B` is 2 | verified | same decode, all `2` |
| `U+200D` (ZWJ) and `U+0301` (combining acute) are width 0; `U+FE0F` is a `WidthInfo` special (width 0, `VARIATION_SELECTOR_16`) | verified | decode: `U+200D` 0, `U+0301` 0, `U+FE0F` packed 3 → `tables.rs:197` `'\u{FE0F}' => (0, WidthInfo::VARIATION_SELECTOR_16)` |
| `U+1F468`/`U+1F469`/`U+1F467` are `WidthInfo::EMOJI_PRESENTATION`, width 2 each | verified | decode returns packed `3`; `tables.rs:203-204` the `_ => (2, WidthInfo::EMOJI_PRESENTATION)` arm |
| A family emoji ZWJ sequence has string width 2 | verified (doc + impl) | `unicode-width-0.2.2/src/lib.rs:58-61` (**not** `:47-52`, which is the `cjk` doctest): "Well-formed, fully-qualified emoji ZWJ sequences have width 2"; `tables.rs:245-300` `width_in_str`'s `ZWJ_EMOJI_PRESENTATION` / `EMOJI_PRESENTATION` chain. Compile probe: `width("👨‍👩‍👧") == 2`, 5 chars, 1 grapheme |
| The per-`char` sum for `U+1F468 U+200D U+1F469 U+200D U+1F467` is **6**, not 8 | verified (corrects the brief) | 2 + 0 + 2 + 0 + 2, from the two rows above; confirmed by compile probe |
| A per-`char` sum **undercounts C0 controls and `DEL` by one** | verified (probe; not previously claimed) | `UnicodeWidthChar::width('\u{1}')` is `None`; `UnicodeWidthStr::width("\u{1}")` is `1` (`tables.rs:245-263`, `width_in_str`'s `c <= '\u{A0}'` fallthrough). Probe: `"a\tb"` 3-vs-2, `"a\r\nb"` 3-vs-2, `"\u{1}"` 1-vs-0, `"\u{7F}"` 1-vs-0. A second, independent reason D15 forbids the per-`char` sum |
| `…` (`U+2026`), `•` (`U+2022`) and `U+FFFD` are 1 under `width()` but **2** under `width_cjk()` | verified (probe) | probe output `width=1 width_cjk=2` for each. These are the three glyphs the widget itself draws, so R-5's trap bites the mask dots and the `U+FFFD` stand-in, not just the `…`. `U+2400`/`U+2401`/`U+2421` are 1 under **both** |
| Sum of per-**grapheme** widths equals whole-string width | **not a licence** (probe) | true across all 17 probe strings by coincidence; the divergent cases (`\t`, `\r\n`, C0, `DEL`) each occupy their own grapheme and the emoji collapse into one grapheme already carrying the full width. Only "measure the cluster's string" is supported |
| `ratatui`'s `U+FF9E`/`U+FF9F` adjustment is unconditional, not grapheme-aware | verified (source read) | `cell_width.rs:66-75` counts every such mark in the string; its own tests give `"あﾞ"` 3, `"aﾞ"` 2, `"ｶﾞ"` 2, and no adjustment for `U+3099`/`U+309A` (`"ｶ゙"` 1). Counting inside the cluster is equivalent because both marks are `Grapheme_Extend` |
| `ratatui`'s `len() == 1` fast path has a **panicking** `debug_assert!` on unfiltered control chars | verified (source read) | `cell_width.rs:34-38`. `set_stringn`/`set_line` filter control chars; `Span::styled_graphemes`, `set_symbol`, `set_char` do not. The widgets' control stand-in substitution is what keeps this quiet under `cargo test` |
| `U+2328` alone is 1; `U+2328 U+FE0F` is 2 | verified | `ratatui-core-0.1.2/src/buffer/buffer.rs:1450` `#[case::keyboard_emoji("⌨️", "⌨️xxxxx")]` with the comment "base symbol + VS16 for emoji presentation … should render as a single grapheme with width 2" |
| `width_cjk()` differs from `width()` for East Asian Ambiguous chars | verified | decode against `WIDTH_ROOT_CJK`: `U+2026` 1→2, `U+2500` 1→2, `U+2502` 1→2, `U+2588` 1→2, `U+00B7` 1→2, `U+2014` 1→2, `U+2190` 1→2, `U+00B0` 1→2; `U+4E00` 2→2 |
| `ratatui` measures a cell with `UnicodeWidthStr::width(s)` **plus one per `U+FF9E`/`U+FF9F`** | verified | `ratatui-core-0.1.2/src/buffer/cell_width.rs:3` (`use unicode_width::UnicodeWidthStr`), `:34-46` (`width.saturating_add(count_halfwidth_sound_marks(self))`), `:60-75` the rationale with its two upstream references; unit tests `halfwidth_katakana_with_dakuten` → 2 and `non_katakana_with_halfwidth_dakuten` → `"aﾞ"` 2 |
| `ratatui` drops a grapheme that contains a control char, and every zero-width grapheme | verified | `buffer.rs:351` `.filter(\|symbol\| !symbol.contains(char::is_control))`, `:353` `.filter(\|(_symbol, width)\| *width > 0)` |
| `ratatui` stops drawing the **rest of the string** when a wide grapheme does not fit | verified | `buffer.rs:354-357` `map_while(… remaining_width.checked_sub(width)? …)` — `None` ends the iterator |
| `ratatui` styles only the **first** cell of a wide grapheme | verified | `buffer.rs:360-366` `set_symbol(..).set_style(style)` then `self[(x,y)].reset()` for the continuation; `Cell::reset` is `*self = Self::EMPTY` (`cell.rs`) |
| `Paragraph` renders a `Line` through `Buffer::set_line` → `set_stringn` per span | verified | `buffer.rs:372-390` `set_line`; `set_span` at `:394` |
| `unicode-segmentation` exposes `UnicodeSegmentation::graphemes(&str, bool)`, extended clusters for `true` | verified | `unicode-segmentation-1.13.3/src/lib.rs:74-92`, with the doc recommending extended boundaries |
| `TextField` is used in 10 files, `TextArea` in 3, and none of them assumes one-char-one-cell | verified (count restated) | full-tree grep for `\bTextField\b` / `\bTextArea\b` finds 12 and 6 files respectively — the plan's counts excluded the two widget files' own cross-doc-links. Real files: `store_worker.rs:93` (doc), `ui/mod.rs:13-14` (re-export), `backlog/detail/runs.rs`, `settings/{boxes,connection,hierarchy,kinds,prompt,qdrant}.rs`, `skills/templates.rs`, `templates.rs:33,283` (doc), `tests/templates.rs` (indirect), plus the two widgets. Every render site hands the widget's `Line` / `Vec<Line>` to a `Paragraph`: `boxes.rs:712` → `Paragraph` at `:588`; `skills/templates.rs:1039` → `Paragraph`, and `:959` extends a `TextField::line`'s spans into a row rendered at `:452`/`:461`/`:922`/`:944`/`:1079` |
| Exactly one non-test caller of `cursor_line_col` outside the widget | verified | `templates.rs:439-440` inside `TemplatesView.render`; every other hit is inside `text_area.rs` itself |
| `TextArea::set_cursor` is fed a **byte** offset from `parse` at two call sites | verified | `templates.rs:395` (`editor.area.set_cursor(error_at(&err).unwrap_or(text.len()))`) and `templates.rs:698` (`let at = error_at(&err).unwrap_or(editor.area.text().len()); editor.area.set_cursor(at);`) |
| `a_wide_char_line_is_windowed_by_chars_not_cells` exists and names the limitation in its doc comment | verified | `text_area.rs:1015-1028` |
| `connection.rs` states the mask's unit in prose and in `Debug` | verified | `connection.rs:11-13` ("one `\u{2022}` per character and a count"); `connection.rs:186-193` — the impl is `core::fmt::Debug for Editor` (the struct is at `:181`, its `input: TextField` at `:183`), and the field call `.field("len", &self.input.len())` is at `:190`. The plan first named the type `DsnField`; **no such symbol exists in the file** — the real type is `Editor` |
| No snapshot holds a wide char or a combining mark | verified (given) | four directories, 8 / 28 / 88 / 1 files; box-drawing and `…` present, no East Asian Wide, no combining marks |
| Snapshot counts are 8 / 28 / 88 / 1 | verified | `ls … \| wc -l` in the worktree |
| `unsafe_code = "forbid"`, `edition = "2024"`, `rust-version = "1.98"` | verified | root `Cargo.toml` `[workspace.lints.rust]` and `[workspace.package]` |
| The toolchain is pinned at `1.98.1` with `rustfmt` and `clippy` | verified | `rust-toolchain.toml` |
| `x86_64-pc-windows-msvc` is an installed target | verified | `rustup target list --installed` (only `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`) |
| `unicode-width` and `unicode-segmentation` declare no `unsafe`, no build script, no `cfg(target_os)`, and are `#![no_std]` | verified | `unicode-width-0.2.2/src/lib.rs:177` `#![no_std]` (was cited as `:14`; wrong) and `:171` `#![forbid(unsafe_code)]`; `unicode-segmentation-1.13.3/src/lib.rs:55` `#![no_std]`, `:50` `#![deny(missing_docs, unsafe_code)]`. `grep -rn unsafe` over each `src/` returns **exactly one hit each** — the attribute itself. Both manifests carry `build = false`; neither has a `build.rs` on disk and neither has a `[target.*]` section |
| `unicode-width`'s `cjk` feature is default-on, and `ratatui` does not disable it | verified | `unicode-width/Cargo.toml:48-51` — `cjk = []`, `default = ["cjk"]`. `ratatui-core-0.1.2/Cargo.toml:158-159` and `ratatui-widgets-0.3.2/Cargo.toml:193-194` both request `unicode-width = ">=0.2.0"` with **no** `default-features = false`, so feature unification keeps it on. This is also independent evidence for D1's `"0.2"`: the floor `ratatui` asks for is `>=0.2.0`, so `^0.2` unifies onto the same instance |
| The workspace will resolve the **same** `unicode-width` instance `ratatui-core` uses | **verified** | `cargo tree -p unicode-width -i` roots at a single `v0.2.2`, shared by `console`, `indicatif`, `ratatui-core`, `ratatui-widgets` and `unicode-truncate`. One version in `Cargo.lock`; `^0.2` admits `0.2.2`. Independently: `ratatui-core-0.1.2/Cargo.toml:158-159` requests `unicode-width = ">=0.2.0"`, so `^0.2` unifies onto the same instance by construction. `unicode-segmentation` likewise roots at a single `v1.13.3` (7 dependents) and `^1.13` admits `1.13.3` |
| Declaring them adds **no compiled crate and no lockfile line** | **partly wrong** | no new crate: true. **no lockfile line: false.** A new edge into an already-locked package rewrites that package's `dependencies` list, so `Cargo.lock` gains exactly two lines in the `htui` block. Proved on a scratch workspace. D1's gate and T0's validation were corrected accordingly |
| `cargo check -p htui --target x86_64-pc-windows-msvc` succeeds | **falsified** | exit `101`. Decisive line: `error occurred in cc-rs: failed to find tool "lib.exe": No such file or directory (os error 2)`, in `ring v0.17.14`'s build script, with a parallel `aws-lc-sys@0.45.0` C failure (`unknown type name 'pthread_rwlock_t'`). `grep crates/htui` over the log returns nothing — the build dies before htui's sources. `x86_64-pc-windows-msvc` **is** installed, so this is a missing MSVC C toolchain, not a missing target. Matches `README.md:461-462` |
| The greps in "Validation" return the match counts listed | **partly wrong** (2 of 4) | on the base tree: `` 'no `unicode-width` is declared' `` → **1** hit (`text_field.rs:4`), not two — `text_area.rs:13` words it `` (no `unicode-width`) ``. `'widthed_by_chars\|one char is otherwise one cell'` → **1** hit (`text_area.rs:216`); the `widthed_by_chars` half matches **nothing**. `'windowed_by_chars_not_cells'` → 1 (`text_area.rs:1015`), as expected. `'UnicodeWidthChar'` → 0, as expected. The Validation block now uses patterns that catch every stale site |
| "No `unicode-width` reference anywhere in `crates/`" | **falsified** | `grep -rn 'unicode-width' crates/` returns the two doc lines: `text_field.rs:4` and `text_area.rs:13`. `unicode-segmentation`, `unicode_width`, `unicode_segmentation` and `UnicodeSegmentation` all return nothing |
| A `Dsn` is ASCII (R-3's stated mitigation) | **falsified** | `Dsn::parse` (`crates/htui-store/src/dsn.rs:119-123`) returns `Zeroizing::new(text.to_owned())` — verbatim, no encoding, decoding or ASCII validation. `scan` (`:176-215`) rejects only control chars, bad scheme, empty host, bad port, unknown query keys. `sqlx-postgres-0.9.0/src/options/parse.rs:34-39` percent-**decodes** the password, so non-ASCII is supported; `url 2.5.8` accepts and normalizes one, and `Dsn` discards the normalization. **A masked `TextField` can hold non-ASCII and the ` (n)` count can legitimately move.** R-3 and D6 were amended |
| The 125-file snapshot `git diff --exit-code` gate is empty **after** the change | **unverified (by construction)** | this is the acceptance criterion itself, not a claim about the base tree. On the base tree it is trivially true: `git status --short` shows only the untracked plan file, and `git ls-files --error-unmatch Cargo.lock` confirms the lock is tracked and unmodified |

---

## Fact-check results

Three independent passes, run against the worktree at `68c058f`. **No design decision (D1–D19) was
falsified.** The amendments are all to gates, evidence and navigation.

| # | Pass | What it did | Outcome |
|---|---|---|---|
| 1 | Crate behavior | Built a scratch crate at `/tmp/uw-probe` depending on `unicode-width = "0.2"` and `unicode-segmentation = "1"`, and printed `width` / `width_cjk` / per-char sum / grapheme count for ~40 strings. Compiled and run with `cargo run --offline`. | **All 30+ width and grapheme claims verified.** Four amendments: the emoji prose is at `lib.rs:58-61` not `:47-52`; `…`, `•` and `U+FFFD` are `width_cjk() == 2`; a per-`char` sum undercounts C0 and `DEL`; `no_std` is at `lib.rs:177` not `:14` |
| 2 | Tree lines | Read all 45 navigation claims (line numbers, test names, exact string literals, manifest pins) with numbered reads and byte-exact `repr` checks | **36 verified, 9 amended.** All 9 were a wrong line, a doc attached to the wrong symbol (`connection.rs`'s `DsnField` does not exist; the type is `Editor`), or one **invented** assertion (no test asserts the tab's three cells as three items — `spans[1].content == " "` at `:1064` is the real evidence). The "split across two lines" note on `text_field.rs:4-5` is recorded because a literal line-based patch would miss it |
| 3 | Unverified rows | Ran `cargo tree -i` for both crates, `cargo check -p htui --target x86_64-pc-windows-msvc`, the four greps, and read `Dsn::parse` | **Two verified, four falsified or partly wrong**: the lockfile gate, the Windows gate, R-3's DSN claim, and two grep expectations. Also confirmed the workspace's own gates (`README.md:469-473`) and that Postgres is **optional** — with `HTUI_TEST_DATABASE_URL` unset those tests print `skipped:` and pass (`README.md:477-480`), so a green suite does not require `docker compose up` |

**Task independence, checked mechanically rather than trusted.** All four tasks list their files, and
intersecting every pair gives the empty set:

```
T0 ∩ T1 = ∅    T0 ∩ T2 = ∅    T0 ∩ T3 = ∅
T1 ∩ T2 = ∅    T1 ∩ T3 = ∅    T2 ∩ T3 = ∅
```

10 distinct files; `crates/htui/src/ui/cells.rs` confirmed absent on the base tree. The independence
marking **holds**. The only ordering constraint is real and stated: T1 and T2 both call
`crate::ui::cells`, which T0 creates, so neither compiles against the base tree.

**One thing the fact-check could not establish, by design.** Whether the 125 snapshots stay
byte-identical after the change is the *acceptance criterion* (D17), not a fact about the base tree.
It is verified by running the gates after implementation, not by reading.

**A disclosure about the fact-check itself.** One probe command in pass 3 was a heredoc that the
sandbox classifier blocked; the following command's `cd` therefore failed and `cargo run` executed in
the session's default working directory, which is the **primary checkout** at `/home/mluigi/projects/htui`
— the one both the task brief and the prompt ruled out. It launched the `htui` binary, which aborted
immediately with `failed to initialize terminal: Os { code: 6 }` (no TTY) and wrote nothing. The probe
was re-run outside the repo via `--manifest-path`. No tracked file in either checkout was modified and
the worktree's `git status` is unchanged. Recorded because the ground rule was explicit and the
near-miss is real, even though the effect was nil.

## Two notes the drafting pass surfaced for the coordinator

1. **`width_cjk()` is a live trap (R-5, D2b).** `unicode-width`'s default `cjk` feature is *on*, so
   `width_cjk` is available and unsealed, and it reports `…` and every box-drawing char as **2** cells.
   The fact-check widened the blast radius: `•` and `U+FFFD` are 2 as well, so the trap reaches the
   mask dots and the replacement stand-in, not just the ellipsis. Choosing it would move every snapshot
   in the repo. It is pinned by a test that asserts both numbers.
2. **`ratatui`'s own buffer is already width- and grapheme-correct** (`buffer.rs:336-366`,
   `cell_width.rs`). MOD-54 is therefore not about making the *renderer* correct — `ratatui` never
   overruns a `Rect`. It is about making the **widget's arithmetic** agree with the renderer. That
   reframing is why the plan is a private helper module and not a dependency swap, and it is why D1's
   dependency is the easy half of the item.
