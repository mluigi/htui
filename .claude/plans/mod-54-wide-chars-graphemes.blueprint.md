# Blueprint: MOD-54 — wide characters and graphemes in the text widgets

Implements the confirmed plan at `.claude/plans/mod-54-wide-chars-graphemes.plan.md` (D1–D19, T0–T3).
Base: `mod-54` at `6b4bcaa`. Every decision id, test name and gate below is the plan's; this document
supplies the code and records the two corrections the implementation pass found.

**Written inline, not by a subagent.** The `code-architect` agent failed twice with
`HTTP 402 Insufficient credits` on `claude-opus-5-5`; the `model` parameter accepts only
`sonnet | opus | haiku | fable`, so the session model (`stealth/space-bunny-alpha`) cannot be selected
for a subagent. The maintainer's instruction was to use the session model and, failing that, go
inline. Recorded here because the blueprint's provenance is part of its audit trail.

---

## A. Two corrections found while writing the code

### A.1 D4's window rule is inverted in the plan (fixed)

The plan (D4) picks the window head as the **largest** `s` with
`cw - cell_width(&glyphs[s..cursor]) <= budget - aw - 1`. That quantity is the width of the
**hidden head**, not of the shown tail, so maximising it is minimising what the user sees. The
correct constraint is on the shown tail:

> **`s` is the smallest index in `0..=cursor` such that `cw - cell_width(&glyphs[0..s]) <= budget - aw - 1`,
> with the ellipsis set.** Equivalently: show as much of the tail as fits.

Only `glyphs[0..s]` is summed, and the scan runs **upward from 0**, breaking at the first fit.
`glyphs[0..cursor]` is already `cw`, so `cw - cell_width(&glyphs[0..s])` is the width of the shown
tail `glyphs[s..cursor]` without concatenating it.

Two independent checks say the corrected rule is right, and the plan's is wrong:

- **A pinned existing test fails under the plan's rule.** `a_masked_field_renders_dots_and_a_count`
  types `0123456789ab` and expects `"…•  (12)"` at width 8 (`budget = 3`, `cw = 12`, `aw = 1`). The
  plan's "largest `s`" takes `s = 12`, empties `before`, and draws `"…  (12)"` — the character under
  the cursor vanishes. The corrected rule takes `s = 11` and draws `"…•  (12)"`.
- **Verified by execution** (scratch crate, `unicode-width 0.2.2` + `unicode-segmentation 1.13.3`,
  `cargo run --offline`). Every ASCII expectation the plan pins reproduces byte-for-byte:

| fixture | width | cursor | drawn | cells |
|---|---|---|---|---|
| `abc` | 10 | end | `abc` | 4 |
| `abcdefghijkl` | 10 | end | `…efghijkl` | 10 |
| `abcdefghijkl` | 10 | 5 | `abcdefghij` | 10 |
| masked `0123456789ab` | 8 | end | `…•  (12)` | 3 of `budget` |
| 12 × `一` | 10 | end | `…` + 4 × `一` + ` ` | 10 |
| 12 × `一` | 10 | 0 | 5 × `一` | 10 |
| 12 × `一` | 3 | end | `…` + ` ` | 2 |
| `👨‍👩‍👧` ×3 | 10 | end | 3 clusters + ` ` | 7 |
| `漢x` | 3 | end | `…x ` | 3 |

**The ASCII reduction still holds**, so D17 is intact: with every width 1 the rule becomes
"smallest `s` with `cursor - s <= budget - 2`", i.e. `s = cursor + 2 - budget` — the old formula
verbatim. It coincides with the old arithmetic *only* when all widths are 1, which is precisely the
case D17 pins.

### A.2 `aw > budget` underflows (fixed, and D10's exception needs a short-circuit)

When the cursor sits on a two-cell cluster in a one- or two-cell field, `aw > budget` and the
`room` computation underflows. D10 already prescribes the right behaviour — draw a single styled
space in place of the cluster so the cursor stays on screen and the line still respects `width` — but
it must be an **early return**, not a `saturating_sub` on a meaningless value. The probe panicked at
`budget - aw - 1` on exactly this fixture. Implement as a branch that draws `Span::styled(" ", …)` and
returns.

---

## B. T0 — the declarations and the shared helper

### B.1 `Cargo.toml`, `[workspace.dependencies]`

Appended after the `gix` block, matching the `zeroize` / `regex` / `semver` / `bytes` precedent:

```toml
# MOD-54 D1: the width of a display column and the boundary of a grapheme cluster, so the text
# widgets count cells instead of code points. Both are already in the graph as transitive
# dependencies of `ratatui-core` and `ratatui-widgets`, so declaring them adds a name and not a
# crate. `0.2` is the same floor `ratatui` asks for (`>=0.2.0`), so the workspace keeps one
# instance -- do not relax it to `0.1`, which would compile a second copy.
unicode-width         = "0.2"
unicode-segmentation   = "1.13"
```

### B.2 `crates/htui/Cargo.toml`, `[dependencies]`

```toml
# MOD-54 D1: display-cell width and grapheme boundaries for the text widgets.
unicode-width         = { workspace = true }
unicode-segmentation   = { workspace = true }
```

### B.3 `crates/htui/src/ui/mod.rs`

```rust
mod cells;
```

private, beside `mod layout;` / `mod theme;`. No `pub use`. `pub(crate)` items inside are reachable
from `text_field` and `text_area` because both are descendants of `ui`, and are needed nowhere else —
so the module never enters the crate's public surface.

### B.4 `crates/htui/src/ui/cells.rs` (new)

```rust
//! The two measurements both text widgets draw by: how many terminal **cells** a string occupies,
//! and where its **grapheme cluster** boundaries are (MOD-54 D2).
//!
//! One module, private, so `TextField` and `TextArea` cannot disagree with each other — and so that
//! neither can disagree with `ratatui`, which is the thing actually drawing the result. Every
//! cell count in `ui/` comes from [`cell_width`]; a widget that counts columns any other way drifts
//! from the renderer by exactly the difference, and a cursor highlight lands on the wrong cell.
//!
//! **Two rules a reader must not break.** First, a cluster's width is the width of the cluster's
//! **own string**: `UnicodeWidthStr::width`, never a sum of `UnicodeWidthChar::width` over the code
//! points. A family emoji is one cluster of five code points; summed per `char` it is 6, and
//! measured as a string it is 2. The crate documents the same asymmetry for `"\r\n"` and for
//! emoji modifier and presentation sequences. Second, a per-`char` sum is *also* simply wrong for
//! control characters: `UnicodeWidthChar::width('\u{1}')` is `None` while
//! `UnicodeWidthStr::width("\u{1}")` is `1`, so the natural `.unwrap_or(0)` undercounts every C0
//! control and `DEL` by one — and a `$EDITOR` body can hold those.
//!
//! The non-CJK `width()` is used, never `width_cjk()`. East Asian **Ambiguous** characters — `…`,
//! `•`, `U+FFFD`, every box-drawing char — are 1 cell under `width()` and 2 under `width_cjk()`,
//! and `…` and `•` are two of the glyphs `TextField` draws itself. `ratatui` uses `width()`.

use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// The halfwidth katakana voiced sound mark, which `unicode-width` reports as zero-width
/// `Grapheme_Extend` but terminals draw as an independent halfwidth cell.
const VOICED_SOUND_MARK: char = '\u{FF9E}';
/// Its semi-voiced counterpart, `U+FF9F`.
const SEMI_VOICED_SOUND_MARK: char = '\u{FF9F}';

/// How many terminal cells `s` occupies.
///
/// [`ratatui`] measures a cell the same way and then adds one per halfwidth sound mark, because
/// `unicode-width` calls those two marks zero-width while terminals draw them as cells
/// (`ratatui-core-0.1.2/src/buffer/cell_width.rs:34-46`, citing Ruby reline #832 and Microsoft
/// Terminal #18087). Replicating that is the point of OQ-1: if this function disagreed with the
/// renderer, every halfwidth-katakana line would be measured a cell short of how it is drawn.
///
/// The mark count is taken **inside the string being measured**. `ratatui` counts over the whole
/// string unconditionally, which gives `"あﾞ"` 3; because both marks are `Grapheme_Extend` they
/// always attach to the preceding cluster, so counting per cluster gives the same answer. The test
/// below pins `"あﾞ"` at 3 so that equivalence is tested rather than assumed.
#[must_use]
pub(crate) fn cell_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
        + s.chars()
            .filter(|c| matches!(*c, VOICED_SOUND_MARK | SEMI_VOICED_SOUND_MARK))
            .count()
}

/// `s` split into UAX #29 **extended** grapheme clusters — the boundaries a user perceives as one
/// character, and the boundaries `Left`, `Right`, `Backspace` and `Delete` must step by.
///
/// Extended, not legacy: legacy boundaries separate `"👨‍👩‍👧"` into three code points and split the
/// combining sequence in `"áb"`, which is the defect this item exists to remove. `UnicodeSegmentation`
/// recommends the extended form for exactly this reason.
pub(crate) fn graphemes(s: &str) -> impl Iterator<Item = &str> + '_ {
    s.graphemes(true)
}
```

`graphemes(true).collect::<String>() == s` holds for every input, so the widgets can build a drawn
`String` per cluster and `ratatui` will re-split it to the same thing. Verified for
`👨‍👩‍👧`, `a\tb`, `a\u{1}b`, `漢字\t字`, `""`, `\r\n`, `e\u{301}x`.

### B.5 `cells.rs` tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr as _;

    #[test]
    fn ascii_and_the_widgets_own_glyphs_are_one_cell() {
        assert_eq!(cell_width(""), 0);
        for one in ["a", "…", "•", "\u{2500}", "\u{FFFD}", "\u{2400}", "\u{2421}"] {
            assert_eq!(cell_width(one), 1, "{one:?}");
        }
    }

    #[test]
    fn a_wide_grapheme_is_two_cells() {
        assert_eq!(cell_width("一"), 2);
        assert_eq!(cell_width("⌨️"), 2, "VS16 flips the base to emoji presentation");
        assert_eq!(cell_width("⌨"), 1, "and the bare base is not");
    }

    #[test]
    fn a_zero_width_cluster_is_zero_cells() {
        assert_eq!(cell_width("\u{301}"), 0, "a lone combining mark");
        assert_eq!(cell_width("\u{200D}"), 0, "a lone ZWJ");
    }

    /// OQ-1, answered: replicate `ratatui`'s halfwidth sound mark adjustment, so the widget's
    /// arithmetic and the renderer's agree by construction.
    #[test]
    fn a_halfwidth_sound_mark_adds_a_cell() {
        assert_eq!(UnicodeWidthStr::width("ｶﾞ"), 1, "unicode-width alone says one");
        assert_eq!(cell_width("ｶﾞ"), 2, "and a terminal draws two, as ratatui counts it");
        assert_eq!(cell_width("あ"), 2, "a fullwidth kana, untouched by the rule");
    }

    /// The adjustment is unconditional in `ratatui` — it counts the mark wherever it appears, so
    /// `"あﾞ"` is 3 and not 2. Counting inside the cluster gives the same answer because the mark is
    /// `Grapheme_Extend`; this pins the equivalence.
    #[test]
    fn the_sound_mark_count_is_the_same_whole_or_per_cluster() {
        assert_eq!(cell_width("あﾞ"), 3);
        assert_eq!(
            "あ".graphemes(true).chain("ﾞ".graphemes(true)).map(cell_width).sum::<usize>(),
            cell_width("あﾞ"),
        );
    }

    /// D2b, and the trap that R-5 names. `width_cjk` is available without declaring anything --
    /// `unicode-width`'s `cjk` feature is default-on and `ratatui` does not disable it -- and it
    /// doubles every East Asian Ambiguous character, three of which this widget draws itself.
    #[test]
    fn the_non_cjk_width_is_the_one_we_use() {
        for ambiguous in ["…", "•", "\u{FFFD}", "\u{2500}", "\u{2502}"] {
            assert_eq!(cell_width(ambiguous), 1, "{ambiguous:?}");
            assert_eq!(UnicodeWidthStr::width_cjk(ambiguous), 2, "{ambiguous:?} under width_cjk");
        }
    }

    #[test]
    fn a_cluster_is_split_on_user_perceived_boundaries() {
        assert_eq!(graphemes("a\u{301}").count(), 1, "base + combining mark is one");
        assert_eq!(graphemes("👨‍👩‍👧").count(), 1, "a family emoji is one, not three");
        assert_eq!(graphemes("a\r\nb").collect::<Vec<_>>(), ["a", "\r\n", "b"]);
        assert_eq!(graphemes("\u{1}").count(), 1, "a C0 control is its own cluster");
        assert_eq!(graphemes("").count(), 0);
    }

    /// D15's rule, as a test rather than a comment: measure the string, never sum per `char`.
    #[test]
    fn a_cluster_is_measured_as_its_own_string() {
        let family = "👨‍👩‍👧";
        let summed: usize = family
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        assert_eq!(cell_width(family), 2);
        assert_eq!(summed, 6, "which is why the per-char sum is not an option");
    }
}
```

### B.6 T0 gates

```
cargo test -p htui --all-features --lib -- --test-threads=1
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
git diff --stat -- Cargo.lock     # EXACTLY 2 insertions, 0 deletions
git diff -- Cargo.lock            # only + "unicode-segmentation", + "unicode-width",
                                  # both inside the `htui` block; no new [[package]], no version bump
cargo tree -p unicode-width -i   # one v0.2.2, shared with ratatui-core
```

`git diff --exit-code Cargo.lock` is **not** the gate. It will be red, and correctly so; a
maintainer who "fixes" it by reverting the lock would break the build. **No Windows gate**:
`cargo check -p htui --target x86_64-pc-windows-msvc` exits 101 in `ring v0.17.14`'s build script
(`cc-rs: failed to find tool "lib.exe"`) and never reaches an `htui` source line. MOD-16 carries
Windows verification; do not run it and do not read its failure as a result.

---

## C. T1 — `TextField`

### C.1 Cursor as a grapheme index (D3)

`cursor: usize` is documented `/// Grapheme index, 0..=len().` `len()` becomes
`self.text.graphemes(true).count()`. `byte_of` keeps its signature and its `map_or(self.text.len(), …)`
tail, iterating `grapheme_indices(true)`. `with_text` seeds `cursor` with `len()`. `Left`,
`Right`, `Home`, `End`, `Delete` and `Backspace` keep their `± 1` shape unchanged.

**`insert` is the trap** and is the only site that is not a one-line change:

```rust
    /// Inserts one char at the cursor and steps over the whole cluster it joined.
    ///
    /// A combining mark, a ZWJ or a variation selector merges into the cluster **before** it, so
    /// the cluster count does not grow and `cursor += 1` would land past the end of the buffer.
    /// The count is recomputed instead: the number of clusters that start before the byte just past
    /// the insertion.
    fn insert(&mut self, c: char) {
        let at = self.byte_of(self.cursor);
        self.text.insert(at, c);
        self.cursor = self
            .text
            .grapheme_indices(true)
            .take_while(|(byte, _)| *byte < at + c.len_utf8())
            .count();
    }
```

Worked: `"a"` + `U+0301` gives one cluster starting at 0, so `cursor = 1` = the end. `"ab"` + a mark
at 1 gives clusters at 0 and 3, so `cursor = 1` = on the `b`. `"👨"` + `U+200D` gives one cluster, so
`cursor = 1`.

### C.2 `line()` windows in cells (D4, corrected by A.1)

```rust
    #[must_use]
    pub fn line(&self, width: u16, focused: bool, theme: &Theme) -> Line<'static> {
        let width = usize::from(width);
        let suffix = if self.masked {
            format!(" ({})", self.len())
        } else {
            String::new()
        };
        let budget = width.saturating_sub(cell_width(&suffix));
        let glyphs: Vec<&str> = if self.masked {
            vec!["•"; self.len()]
        } else {
            self.text.graphemes(true).collect()
        };

        // The cursor's own cell, and the cells before it. `glyphs[cursor]` is the cell a space
        // stands in for past the end of the buffer.
        let at: String = glyphs.get(self.cursor).map_or(" ".to_owned(), |g| (*g).to_owned());
        let aw = cell_width(&at);
        let cw: usize = glyphs[..self.cursor.min(glyphs.len())]
            .iter()
            .map(|g| cell_width(g))
            .sum();

        let cursor_style: Style = if focused { theme.selected } else { theme.base };
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(5);

        // A cluster wider than the whole window cannot be drawn and cannot be split (A.2).
        if aw > budget {
            spans.push(Span::styled(" ".to_owned(), cursor_style));
            if !suffix.is_empty() {
                spans.push(Span::styled(suffix, theme.dim));
            }
            return Line::from(spans);
        }

        // `budget` cells hold `['…'] + before + the cursor cell + after`.
        let (start, ellipsis) = if cw + aw <= budget {
            (0, false)
        } else {
            // Show as much of the tail as fits: the smallest `s` whose shown tail
            // `cw - cell_width(glyphs[0..s])` leaves room for the ellipsis and the cursor cell.
            let room = budget - aw - 1;
            let mut head = 0;
            let mut s = 0;
            for g in glyphs.iter().take(self.cursor.min(glyphs.len())) {
                if cw - head <= room {
                    break;
                }
                head += cell_width(g);
                s += 1;
            }
            (s, true)
        };

        if ellipsis {
            spans.push(Span::styled("…", theme.dim));
        }
        let before: String = glyphs[start.min(glyphs.len())..self.cursor.min(glyphs.len())]
            .iter()
            .map(|g| (*g).to_owned())
            .collect();
        if !before.is_empty() {
            spans.push(Span::styled(before, theme.base));
        }
        spans.push(Span::styled(at, cursor_style));

        // The tail, whole clusters only: the first that would overrun ends the window.
        let room = budget
            .saturating_sub(usize::from(ellipsis))
            .saturating_sub(cell_width(&glyphs[start.min(glyphs.len())..self.cursor.min(glyphs.len())].concat()))
            .saturating_sub(aw);
        let mut used = 0;
        let mut after = String::new();
        for g in glyphs.iter().skip(self.cursor + 1) {
            let w = cell_width(g);
            if used + w > room {
                break;
            }
            used += w;
            after.push_str(g);
        }
        if !after.is_empty() {
            spans.push(Span::styled(after, theme.base));
        }
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, theme.dim));
        }
        Line::from(spans)
    }
```

`glyphs` is `Vec<&str>`, not `Vec<char>`: a cluster has to survive to the span intact, because a
`String` built per `char` would re-split the very sequence `ratatui` is about to re-join.

### C.3 D5, D6, D10, D14, D19

- **D5** — `at` is one `Span` in `theme.selected`; a two-cell cluster is therefore highlighted on its
  first cell only, because `Buffer::set_stringn` calls `Cell::reset` on the continuation cell
  (`buffer.rs:360-366`). Pinned by `a_wide_cursor_cell_highlights_its_first_cell`.
- **D6** — the mask is `vec!["•"; self.len()]`, one dot per grapheme, and the ` (n)` suffix is the
  same number. Pinned by `a_masked_field_counts_graphemes_not_code_points`.
- **D10** — the `aw > budget` branch above; plus whole clusters only in `after`.
- **D14** — a zero-width cluster occupies 0 cells, but the cursor must have something to sit on, so
  `at` becomes `U+FFFD` when `aw == 0`.
- **D19** — rewrite seven doc sites: `:4-7` (module doc, both stale sentences), `:38` (struct doc),
  `:47` (`cursor`), `:200` (`len`), `:218-223` (`line`). The `"no `unicode-width` is declared"`
  phrase is **split across lines 4-5** — patch it as prose, not as a line match.

### C.4 T1 new tests

`a_wide_grapheme_is_two_cells` (`漢字`@6 → `漢字 `, 6 cells) · `the_window_counts_cells_not_chars`
(12 CJK@10 → `…`+4+` `, exactly 10 cells) · `a_wide_cursor_cell_highlights_its_first_cell` ·
`left_and_right_step_by_grapheme` (`a\u{301}👨‍👩‍👧c`, `End` = 4) ·
`backspace_removes_a_whole_grapheme` · `a_typed_combining_mark_joins_the_cluster_before_it`
(type `a`,`U+0301`,`b` → `áb`, `len() == 2`, `cursor == 2` — the assertion that catches the naive
port) · `a_wide_grapheme_that_does_not_fit_is_dropped_not_split` ·
`a_wide_cursor_in_a_one_cell_field_draws_a_space` ·
`a_zero_width_cluster_at_the_cursor_draws_a_replacement_glyph` ·
`a_masked_field_counts_graphemes_not_code_points` · `ascii_rendering_is_unchanged` (today's
`"…efghijkl"`, `"abc"`, `"abcdefghij"`, `"•••••••  (7)"`, `"…•  (12)"`).

**Survive unedited:** all thirteen existing tests, including `len: 6`, `len() == 68` and
`cursor == 0`.

---

## D. T2 — `TextArea`

### D.1 D7, D8 — the public cursor stays a byte offset

`cursor`, `set_cursor(byte)` and `cursor()` are unchanged in unit. `templates.rs:395` and `:698` pass
a byte from `parse`; that contract does not move. Only the stepping changes:

```rust
    fn previous_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(byte, _)| byte)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map(|g| self.cursor + g.len())
    }
```

`set_cursor` keeps `min(text.len())`, keeps the `is_char_boundary` walk, and then adds a
**grapheme** walk: while the byte lies strictly inside a cluster, step down to that cluster's start.
`insert` keeps `self.cursor += c.len_utf8()` and then, if the cursor is no longer on a grapheme
boundary, advances it to the end of the cluster that now contains it — the mirror of T1's recompute,
because this cursor is a byte rather than an index.

### D.2 D9 — `drawn()` yields `(item, cells)`

`drawn` changes from an iterator of `char` to a `Vec<(String, usize)>`: one entry per drawn **cell**,
paired with the string that occupies it. A `Vec` rather than a lazy iterator because the `\t` arm
must emit *n separate one-cell items*; and `String` rather than `&str` because the control stand-in
is computed per char and cannot borrow the line. `lines()` already builds three `String`s per row
(`before`, `at`, `after`), so the extra small allocation per cell buys every lifetime question going
away. What the existing tab test constrains is the **item count**, not the item's storage.

```rust
/// `line` as drawn, one `(item, cells)` pair per drawn cell: the string that goes in the cell and
/// how many cells it takes.
///
/// A `\t` is spaces to the next [`TAB_STOP`] counted from the **accumulated cell column**, so a wide
/// grapheme before a tab moves the stop (D14). A control char is its one-cell stand-in. Every other
/// cluster is itself, with its measured width — and `.max(1)`, so a zero-width cluster still
/// occupies a cell and the cursor has somewhere to sit.
fn drawn(line: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    let mut col = 0;
    for c in line.graphemes(true) {
        if c == "\t" {
            // One one-cell item per cell, never one n-cell item:
            // `a_tab_draws_as_spaces_to_the_next_stop` asserts `drawn[0].spans[1].content == " "`,
            // which only holds if the tab's three cells are three separate items.
            let cells = TAB_STOP - col % TAB_STOP;
            col += cells;
            out.extend(core::iter::repeat_n((" ".to_owned(), 1), cells));
        } else if let Some(ch) = c.chars().find(char::is_control) {
            col += 1;
            out.push((stand_in(ch), 1));
        } else {
            let w = cell_width(c).max(1);
            col += w;
            out.push((c.to_owned(), w));
        }
    }
    out
}

/// A control char as something visible: a Control Pictures glyph for C0 and `DEL`, `U+FFFD` for C1.
/// Every shape here is width 1 under `width()` (pinned in `cells.rs`), which is what keeps
/// `ratatui`'s panicking `debug_assert!` on unfiltered control chars (`cell_width.rs:34-38`) quiet:
/// the widgets never hand a raw control char to the renderer, and this arm is why.
fn stand_in(c: char) -> String {
    match c {
        '\0'..='\u{1f}' => char::from_u32(0x2400 + u32::from(c))
            .map_or_else(|| "\u{fffd}".to_owned(), |g| g.to_string()),
        '\u{7f}' => "\u{2421}".to_owned(),
        _ => "\u{fffd}".to_owned(),
    }
}
```

`lines()` then slices this `Vec` by `left` and accumulates cells rather than counting `char`s, and the
"first cluster that would overrun ends the window" rule of D10 is a plain `break` in the accumulation
loop. The invariant to hold is `cell_width(row) <= width` for **every** row, cursor row included.

### D.3 D10, D11 — the window and the viewport

`lines()` computes `col` as the **cell** column of the cursor on its line, from `drawn()`. `left`
becomes a cell column, re-derived and re-snapped on every draw (D11), never a `left` that lands
mid-cluster; the row iterator then drops any cluster that straddles `left`. `top` is untouched — it
is a line index. `follow` is replaced. On ASCII, D11 reduces to `follow` verbatim, which is what
`the_viewport_scrolls_to_keep_the_cursor_visible` (`["e0 "]` at width 3, `["line1","line2","line3"]`)
pins.

### D.4 D12, D13

- **D12** — `goal_col` is a drawn **cell** column, resolved by walking the target line's `(item, cells)`
  pairs and landing on the byte of the last cluster that *starts* at or before the goal, so the byte
  is always a grapheme boundary.
- **D13** — `cursor_line_col` returns a **grapheme** column; its doc changes from "a 0-based **char**
  column" to "a 0-based **grapheme** column". The call site at `templates.rs:439-440` is unchanged.

### D.5 D19 — nine doc sites

`:13-17` (module doc width paragraph) · `:48` (`goal_col`) · `:111` (`len`'s doc, which says
`len() == text().chars().count()` and stays true — `TextArea::len` is **not** touched) · `:140`
(`cursor_line_col`) · `:214-223` (`lines`) · `:333-336` (`drawn`) · plus the test rename below.

`a_wide_char_line_is_windowed_by_chars_not_cells` (`:1014-1028`) is **renamed** to
`a_wide_char_line_is_windowed_by_cells` and its body rewritten, not deleted — its doc comment at
`:1016-1018` is the only place in the repo that names this bug out loud.

### D.6 T2 new tests

`a_wide_char_line_is_windowed_by_cells` (the inversion) · `a_wide_grapheme_that_does_not_fit_is_dropped_not_split` ·
`a_wide_cursor_cell_highlights_its_first_cell` · `left_and_right_step_by_grapheme` ·
`backspace_removes_a_whole_grapheme` · `a_typed_combining_mark_joins_the_cluster_before_it` ·
`set_cursor_floors_inside_a_grapheme` · `a_viewport_never_starts_mid_grapheme` ·
`up_and_down_keep_the_cell_goal_column` · `a_zero_width_cluster_at_the_cursor_draws_a_replacement_glyph` ·
`a_wide_char_before_a_tab_moves_the_stop` (`漢\tx` → `漢  x`) ·
`an_emoji_zwj_sequence_is_one_cluster_two_cells` · `a_variation_selector_is_measured_with_its_base` ·
`cursor_line_col_counts_graphemes_not_bytes` (renamed) · `a_wide_grapheme_at_the_row_edge_does_not_overrun` ·
`ascii_rendering_is_unchanged`.

**Survive unedited:** all thirty-one existing tests, including `len: 11`, `["e0 "]`,
`["vwxyz0123 "]`, `["a   b","    c","abcd    e"]`, `["  b"]`, `"a\u{2401}b\u{2421}\u{fffd}c"`,
`["c","d "]`, and `cursor_line_col() == (1, 2)` / `cursor() == 7`.

---

## E. T3 — comments only

`connection.rs:11-13` and `:186-193` (the impl is `Editor`, **not** `DsnField` — no such symbol
exists) · `skills/templates.rs:439` and `:249`, `:1203` · `templates.rs:33`, `:283` ·
`tests/templates.rs:10-11`. The file's **eleven** `L{n}:C{m}` asserts (`:182,195,204,313,319,397,411,
420,438,447,793`) are ASCII and do not move.

## F. Order and merge

T0 → {T1, T2} → T3. T1 and T2 both call `crate::ui::cells`, so neither compiles against the base
tree; the parallel wave gives each its own worktree. After each merge, re-run the gates **on the
merged tree**, not in the lane.

## G. Reviewer checklist

1. `git diff --exit-code -- 'crates/*/tests/snapshots' 'crates/htui/src/snapshots'` is **empty** —
   the item's hard criterion.
2. `grep -rn 'UnicodeWidthChar' crates/htui/src/ui/` returns nothing; no per-`char` width sum exists.
3. The diff touches only the ten files in the plan's "Files to Change".
4. `grep -rn 'unicode-width' crates/ --include='*.rs'` returns nothing (D19's stale sentences).
5. `git diff --stat -- Cargo.lock` is 2 insertions, 0 deletions, both in the `htui` block.
6. **The corrected D4 rule from A.1 is what was implemented** — a reviewer who reads the plan's
   inverted "largest `s`" and the code will see a mismatch; that mismatch is the correction, not a bug.
7. `aw > budget` short-circuits (A.2) rather than underflowing.
8. Every existing exact-output test is present and unedited.
