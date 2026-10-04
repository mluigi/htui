# Blueprint: MOD-84 reflow item bodies before wrapping (T1 `cells`, T2 `ui::markdown`, T3 Body pane)

**Plan**: `.claude/plans/mod-84-body-reflow.plan.md` (confirmed 2026-10-04; D1-D8 approved). Produced by
`code-architect`. One serial lane, three code commits (T1, T2, T3), then T4 close-out per the plan. The expected rows
below come from an emulation of exactly these rules over the fixture bodies (`htui-core/src/fixtures.rs`
`FEAT_1_BODY`, `ANA_1_BODY`); a mismatch in the implementation is a bug to explain, not a number to re-pin silently.

## Plan amendments needed (session to accept or reject before T2)

- **A-1 (D2 order, defect): check the rule predicate before the list-item predicate.** CommonMark 0.31 §4.1: "when
  both a thematic break and a list item are possible interpretations of a line, the thematic break takes
  precedence". Under the plan's order `* * *` and `- - -` open a list item with text `* *` / `- -`, and the next text
  line joins it lazily (`a\n* * *\nb` draws `a` / `* * * b`). With A-1 they stand alone (`a` / `* * *` / `b`). The
  classifier below has A-1 as step R3; rejected, move R3 to just after R4 and drop M-6's A-1 cases.
- **A-2 (Task 3 test text, defect): at width 43 `a top bar and a backlog tab` is not on one row.** The reflowed rows
  are `workspace-scoped shell with a tab strip, a` / `top bar and a backlog tab, all reading` (listing §4.3, rows 1-2).
  The test asserts the row `top bar and a backlog tab, all reading`, which is what the ragged `top bar and a` row
  becomes.
- **A-3 (fact): 13 snapshots carry the head meta line, not 12, and only 6 show a body.** §5.2 lists them; the 7 that
  show `No body for this item.` must not change.
- **A-4 (D2.1 refinement): a backtick fence's opening run must not be followed by a backtick on the line**
  (CommonMark §4.5, "the info string may not contain backtick characters"). Without it a paragraph line starting with
  ```` ```x``` ```` (inline code) opens a fence that swallows the rest of the body. Tilde fences are unaffected.
- **B-2 (blueprint decision for the open question "what do verbatim lines get"):** headings, rules and quotes get
  inline code (D4); **table rows do not**, and are tab-expanded like code. Removing backticks in some cells of a table
  shifts its columns by two per span and breaks the source's alignment, which is the one thing a table row drawn
  verbatim is for. Rejected: move `table` from R6's `Literal` arm to its `Plain` arm and flip test M-6's table row.
- **D5 input type:** `&[Span<'_>]`, not `&[(Cow<str>, Style)]`: it matches `clip_spans`, and `Span` is a
  `(Cow<str>, Style)` pair already; callers build spans directly.

## 1. T1 `crates/htui/src/ui/cells.rs` (D5, D6)

### 1.1 `expand_tabs` / `TAB_STOP` move (D6)

- Move `TAB_STOP` and `expand_tabs` from `detail/notes.rs` (lines ~80-102) to `cells.rs`, placed after `clip_spans`
  (before `#[cfg(test)]`). Bodies verbatim, except `cells::graphemes`/`cells::cell_width` become `graphemes`/
  `cell_width`. Add `use std::iter;` to cells.rs. Visibility `pub(crate)` for both; add `#[must_use]` to
  `expand_tabs`.
- Docs: `TAB_STOP`: "Columns between tab stops: `TextArea`'s, so a `\t` reads in the Notes thread and in an item
  body's code as it did in the compose area (MOD-13 review L2, MOD-84 D6)." `expand_tabs`: keep notes' doc, add
  "Shared by the Notes thread and the Body pane's code lines (MOD-84 D6)."
- `notes.rs`: delete both; `rows()` calls `cells::expand_tabs(row)`. `std::borrow::Cow` and `std::iter` stay used
  there (`rows`). No other change; `a_tab_in_a_note_draws_as_spaces_to_the_next_stop` must stay green.
- `text_area.rs` keeps its own private `TAB_STOP` (line 42): out of scope, do not touch.
- Module doc (line 25-27): the op list becomes "[`clip`], [`pad`], [`pad_left`], [`fit`], [`wrap`], [`wrap_spans`]
  and [`clip_spans`]", and add one sentence: "[`expand_tabs`] draws a `\t` as `TextArea` does (MOD-84 D6)."

### 1.2 `wrap_spans` (D5), placed directly after `wrap`

```rust
/// `spans`, one line, in rows of at most `width` cells, every piece keeping its span's style
/// (MOD-84 D5): [`wrap`]'s rules over the spans' concatenation. Broken at U+0020 where one fits,
/// inside a word by grapheme only when the word alone is wider; leading spaces kept; a space that
/// does not fit is the break and is dropped; width 0 reads as 1; controls flattened per piece (D3),
/// so only U+0020 splits words. A word may cross a style boundary and moves to the next row whole.
/// A space keeps the style of the span it is in. Adjacent pieces of one style on a row are one
/// span. With one style, the rows' text is exactly [`wrap`]'s. No spans is one empty row.
#[must_use]
pub(crate) fn wrap_spans(spans: &[Span<'_>], width: usize) -> Vec<Vec<Span<'static>>>
```

Private helpers, after it:

```rust
/// One word of [`wrap_spans`]: its flattened pieces with their spans' styles, and its cells.
#[derive(Debug, Default)]
struct Word<'a> {
    /// The word's text per span it crosses; never an empty piece.
    pieces: Vec<(Cow<'a, str>, Style)>,
    /// The sum of the pieces' cells.
    cells: usize,
}

/// `text` onto the last row: into its last span when the style is the same, else a new span.
fn push_styled(rows: &mut [Vec<Span<'static>>], text: &str, style: Style)
//   let Some(row) = rows.last_mut() else { return };
//   match row.last_mut() {
//       Some(last) if last.style == style => last.content.to_mut().push_str(text),
//       _ => row.push(Span::styled(text.to_owned(), style)),
//   }
```

Algorithm (mirrors `wrap` line by line; the only differences are pieces instead of one string):

```text
// 1. Words, as `line.split(' ')` over the concatenation. Split BEFORE flatten: flatten turns a control
//    into a space, and that space must not split (wrap flattens per word for the same reason).
words = vec![Word::default()]; gaps: Vec<Style> = []        // gaps[k] = style of the space before words[k+1]
for span in spans:
    for (at, raw) in span.content.split(' ').enumerate():
        if at > 0 { gaps.push(span.style); words.push(Word::default()) }
        if raw.is_empty() { continue }
        piece = flatten(raw); word = words.last_mut(); word.cells += cell_width(&piece); word.pieces.push((piece, span.style))
// 2. Wrap. Pair each word with the space before it: once(None).chain(gaps.into_iter().map(Some)).zip(&words)
width = width.max(1); rows = vec![Vec::new()]; used = 0
for (gap, word) in pairs:
    if let Some(gap) = gap:                                   // wrap's `at > 0`
        if used + 1 + word.cells <= width:
            push_styled(rows, " ", gap); for (piece, style) in word.pieces: push_styled(rows, piece, style)
            used += 1 + word.cells; continue
        if word.pieces.is_empty() { continue }                // wrap's `word.is_empty()`: the space is the break
        if used > 0 { rows.push(Vec::new()); used = 0 }
    for (piece, style) in word.pieces:
        for cluster in graphemes(piece):                       // per piece = per span, as ratatui draws a Line
            cells = cell_width(cluster)
            if rows.last().is_some_and(|row| !row.is_empty()) && used + cells > width { rows.push(Vec::new()); used = 0 }
            push_styled(rows, cluster, style); used += cells
rows
```

Parity: for one span, pieces == wrap's flattened word, cells == `cell_width(&word)`, and "row non-empty" holds exactly
when wrap's row string is non-empty (no empty piece is ever pushed), so the rows' text is identical. Graphemes are
taken per piece: a cluster split across two spans is two clusters, which is how `ratatui` draws two spans.

### 1.3 T1 tests (cells `mod tests`; reuse `style_a`, `style_b`, `spans`)

Add a helper `fn texts(rows: &[Vec<Span<'_>>]) -> Vec<String>` (each row's contents concatenated). Extract the line
generator out of `wrap_with_a_running_sum_matches_the_remeasuring_wrap` into `fn wrap_lines() -> Vec<String>` (that
test then iterates `wrap_lines()`; behaviour unchanged).

| Test | Input | Expect |
|---|---|---|
| C-1 `wrap_spans_with_one_style_is_wrap` | every `wrap_lines()` line plus `cross_cluster_inputs()`, widths `0..=12`; (a) `[Span::styled(line, a)]`; (b) the line split at every `' '` into `[w0, " ", w1, " ", ...]`, all style `a` | `texts(rows) == wrap(line, w)` for both; every row has at most 1 span and every span's style is `a` (the merge) |
| C-2 `a_word_crossing_a_style_boundary_moves_whole` | `spans(&[("ab ", a), ("c", b), ("d", a)])`, width 3 | `[spans(&[("ab", a)]), spans(&[("c", b), ("d", a)])]` |
| C-3 `a_space_keeps_its_span_s_style` | `spans(&[("a ", a), ("b", b)])`, width 80 | `[spans(&[("a ", a), ("b", b)])]` |
| C-4 `an_overlong_word_splits_by_grapheme_and_keeps_styles` | `spans(&[("(", a), ("code", b), ("),", a), (" next", a)])`, width 6 | `[spans(&[("(", a), ("code", b), (")", a)]), spans(&[(", next", a)])]` |
| C-5 `wrap_spans_width_0_reads_as_1_and_nothing_is_one_row` | `spans(&[("ab", a)])` w 0; `&[]` w 5; `spans(&[("", a)])` w 5 | `[[("a",a)],[("b",a)]]`; `vec![Vec::new()]`; `vec![Vec::new()]` |
| C-6 `wrap_spans_flattens_controls_without_splitting_at_them` | `spans(&[("a\tb c", a)])` w 3; `spans(&[("x\u{1}", a), ("y", b)])` w 80 | `[[("a b",a)],[("c",a)]]`; `[[("x ",a),("y",b)]]` |
| C-7 `expand_tabs_pads_to_the_next_stop_in_cells` | `"a\tb"`, `"\tc"`, `"abcd\te"`, `"\u{6f22}\tx"`, `"plain"` | `"a   b"`, `"    c"`, `"abcd    e"`, `"\u{6f22}  x"`, and `matches!(expand_tabs("plain"), Cow::Borrowed(_))` |

Validate: `cargo test -p htui --lib ui::cells` and `cargo test -p htui --lib notes`.
Commit: `feat(mod-84): cells::wrap_spans, expand_tabs moved from notes (T1)`.

## 2. T2 `crates/htui/src/ui/markdown.rs` (D1-D4) + `ui/mod.rs`

`ui/mod.rs`: `mod markdown;` on the line after `mod cells;`.

### 2.1 Module doc (`//!`)

"An item body's Markdown, reflowed for a pane (MOD-84 D1-D4). Not a Markdown parser: each source line is classed
(fenced code, blank, rule, list item, indented code, heading, table row, quote, text), soft-wrapped lines of a
paragraph or list item are joined with one space, and the result is wrapped at the pane's width by
[`cells::wrap_spans`](crate::ui::cells::wrap_spans), a list item's rows hanging under its text. Blank lines, markers,
code and structural lines are kept as written; a hard break (two trailing spaces, or `\`) starts a new row. Inline code
is drawn without its backticks in `theme.accent`, except in code and table rows. Emphasis, links and heading styles are
not rendered (MOD-80, MOD-82)." (Intra-doc links are fine here: the module is private, as `cells` is.)

### 2.2 Types and constants

```rust
/// A list item's text needs this many cells beside its marker to hang under it (D3); fewer, and
/// its rows start at column 0.
const MIN_TEXT: usize = 10;

/// One block of a body (D2).
#[derive(Debug, PartialEq, Eq)]
enum Block<'a> {
    /// A blank line: one empty row.
    Blank,
    /// A heading, rule or quote, as written: wrapped, inline code parsed.
    Plain(&'a str),
    /// A fence line, a line inside a fence, an indented-code line or a table row (B-2), as written:
    /// tabs expanded, wrapped, no inline code.
    Literal(&'a str),
    /// A paragraph: its text per hard break, soft-wrapped lines joined.
    Paragraph(Vec<String>),
    /// A list item: its indent and marker as written (`"  -"`, `"10)"`), and its text per hard break.
    Item { lead: &'a str, segments: Vec<String> },
}

/// The paragraph or list item text lines join (D2.5).
#[derive(Debug, Clone, Copy)]
struct Open {
    /// Its index in the blocks.
    at: usize,
    /// Its last line ended in a hard break: the next text starts a new segment.
    broken: bool,
}
```

### 2.3 Public surface

```rust
/// `body` as drawn at `width` (0: before the first render, unwrapped), one `Line` per row.
#[must_use]
pub(crate) fn lines(body: &str, width: u16, theme: &Theme) -> Vec<Line<'static>>
//   let width = wrap_width(width);
//   blocks(body).iter().flat_map(|b| draw(b, width, theme.base, theme.accent)).map(Line::from).collect()

/// How many rows [`lines`] gives at `width`, without a theme (the scroll clamp, D7).
#[must_use]
pub(crate) fn row_count(body: &str, width: u16) -> usize
//   let width = wrap_width(width);
//   blocks(body).iter().map(|b| draw(b, width, Style::new(), Style::new()).len()).sum()
```

Styles never move a break (the merge only regroups spans), so `row_count == lines().len()` by construction; test M-11
pins it. `wrap_width` is a private copy of `notes.rs`' (`0 => usize::MAX`, else `usize::from(width)`), doc "as the
Notes pane's `wrap_width`".

### 2.4 Classifier `fn blocks(body: &str) -> Vec<Block<'_>>`

Source lines are `body.lines()` (CRLF safe; a trailing `\n` adds no row; `""` is no blocks). State:
`fence: Option<(u8, usize)>` (char, run length), `open: Option<Open>`. "Close" means `open = None`. Per line, first
rule that matches wins:

| Step | Predicate | Action |
|---|---|---|
| R0 | `fence == Some((ch, n))` | push `Literal(line)`; if `closes(line, ch, n)` then `fence = None` |
| R1 | `fence_open(line) == Some((ch, n))` | close; push `Literal(line)`; `fence = Some((ch, n))` |
| R2 | `line.trim().is_empty()` | close; push `Blank` |
| R3 (A-1) | `is_rule(line)` | close; push `Plain(line)` |
| R4 | `list_item(line) == Some((lead, rest))` | close; push `Item { lead, segments: vec![] }`; `open = Some(Open { at, broken: false })`; `add(rest)` |
| R5 | `open.is_none() && indented(line)` | push `Literal(line)` (opens nothing) |
| R6 | `is_heading(line) \|\| is_quote(line)` → close, push `Plain(line)`; `is_table(line)` → close, push `Literal(line)` (B-2) | |
| R7 | anything else | if `open.is_none()`: push `Paragraph(vec![])`, open it; then `add(line)` |

Predicates (`fn indent(line) -> usize` = count of leading `' '` only):

- `fence_open(line) -> Option<(u8, usize)>`: `indent(line) <= 3`; `rest = &line[indent..]`; first byte `ch` is
  `` b'`' `` or `b'~'`; `n` = run of `ch` from the start, `n >= 3`; if `` ch == b'`' `` then `!rest[n..].contains('`')`
  (A-4). Returns `(ch, n)`.
- `closes(line, ch, n) -> bool`: `indent(line) <= 3`; `rest = &line[indent..]`; run `m` of `ch` from its start;
  `m >= n && rest[m..].trim().is_empty()`.
- `is_rule(line)`: `indent(line) <= 3`; the chars of `line` other than `' '`/`'\t'` are all one char `c` in
  `- * _ =` and there are at least 3 of them.
- `list_item(line) -> Option<(&str, &str)>`: `ws` = leading run of `' '`/`'\t'`; `after = &line[ws..]`; marker length:
  `-`/`*`/`+` → 1; else `d` = leading ASCII digits, `1..=9` of them, followed by `.` or `)` → `d + 1`; else `None`.
  `end = ws + marker`. Then `line[end..]` is empty → `Some((line, ""))`; starts with `' '` → `Some((&line[..end],
  &line[end + 1..]))`; else `None`. (`-x`, `1.5`, a 10-digit number: not items.)
- `indented(line)`: the leading run of `' '`/`'\t'`, through `cells::expand_tabs`, is at least 4 cells (`"    x"`,
  `"\tx"`, `" \tx"`).
- `is_heading(line)`: `indent(line) <= 3`; `rest = &line[indent..]`; `h` = leading `#` count, `1..=6`; next byte is
  none or `b' '`.
- `is_quote(line)`: `line.trim_start().starts_with('>')`. `is_table(line)`: `line.trim_start().starts_with('|')`.

`add(blocks, open: &mut Open, raw: &str)` (D2.5 + hard breaks):

```text
(text, hard) = if let Some(head) = raw.strip_suffix('\\') { (head.trim(), true) }
               else { (raw.trim(), raw.ends_with("  ")) }
segments = match &mut blocks[open.at] { Paragraph(s) | Item { segments: s, .. } => s, _ => unreachable }   // use if-let, no panic
if segments.is_empty() || open.broken { segments.push(text.to_owned()) }
else { last = segments.last_mut(); if !last.is_empty() && !text.is_empty() { last.push(' ') }; last.push_str(text) }
open.broken = hard
```

A hard break on a block's last line adds nothing (no empty row). Any text line with a block open joins it, indented
or not (lazy continuation); 4+ spaces only mean code when nothing is open (R5 after R4, so `    - x` is an item).

### 2.5 Inline code `fn inline(text: &str, base: Style, code: Style) -> Vec<Span<'_>>` (D4)

Byte scan (`` b'`' `` is ASCII, so every slice is on a char boundary). `fn run(bytes, at) -> usize` counts backticks
from `at`.

```text
out = []; plain = 0; i = 0
while i < len:
    if bytes[i] != b'`' { i += 1; continue }
    n = run(bytes, i); j = i + n; close = None
    while j < len:                                    // the next run of EXACTLY n
        if bytes[j] == b'`' { m = run(bytes, j); if m == n { close = Some(j); break }; j += m } else { j += 1 }
    match close:
        None => i += n                                // the whole run stays literal; keep scanning after it
        Some(j) =>
            push text[plain..i] in base if non-empty
            content = &text[i + n..j]
            if content.len() >= 2 && content.starts_with(' ') && content.ends_with(' ') && content.bytes().any(|b| b != b' ')
                { content = &content[1..content.len() - 1] }
            push content in code                      // never empty: runs are maximal
            i = j + n; plain = i
push text[plain..] in base if non-empty
```

Parsed per joined segment (after joining), so a span broken across source lines pairs up. Never called on `Literal`.
Backslash escapes are not interpreted (out of scope; a `` \` `` counts as a backtick).

### 2.6 Drawing `fn draw(block: &Block<'_>, width: usize, base: Style, code: Style) -> Vec<Vec<Span<'static>>>` (D3)

- `Blank` → `vec![Vec::new()]`.
- `Plain(line)` → `cells::wrap_spans(&inline(line, base, code), width)`.
- `Literal(line)` → `cells::wrap_spans(&[Span::styled(cells::expand_tabs(line), base)], width)`.
- `Paragraph(segments)` → each segment `cells::wrap_spans(&inline(seg, base, code), width)`, rows concatenated.
- `Item { lead, segments }`: `prefix = format!("{} ", cells::expand_tabs(lead))`, `hang = cells::cell_width(&prefix)`.
  - `width.checked_sub(hang).filter(|room| *room >= MIN_TEXT)` is `Some(room)`: every segment wrapped at `room`, rows
    concatenated; row 0 gets `Span::styled(prefix, base)` inserted at index 0, every other row `" ".repeat(hang)` in
    `base` (hard-break rows hang too).
  - `None` (fewer than 10 cells): the first segment is wrapped at `width` with the prefix span in front of its spans
    (`[Span::styled(prefix, base)]` then `inline(..)`), each later segment wrapped at `width` alone; every row after
    the first starts at column 0. (`segments.split_first()`; `None` → `vec![vec![Span::styled(prefix, base)]]`.)
  - An item with no text draws its prefix alone: `"- "` (trailing space kept; test M-5 pins it).
- Width 0 reaches `draw` as `usize::MAX` (`wrap_width`): nothing wraps; `checked_sub` keeps the hang path.

### 2.7 T2 tests (`#[cfg(test)] mod tests` in markdown.rs)

Helper `fn rows(body: &str, width: u16) -> Vec<String>` = `lines(body, width, &Theme::default())` mapped with
`ToString::to_string` (as notes' tests do). Every row case also asserts `row_count(body, width) == rows.len()`. Cases
as `(body, width, expected rows)`, one `assert_eq!` with a `"{body:?} at {width}"` message each:

```text
M-1  a_hard_wrapped_paragraph_joins
     ("one two\nthree four", 80, ["one two three four"])
     ("one two\nthree four", 9,  ["one two", "three", "four"])
     ("a\r\nb", 80, ["a b"])
M-2  a_blank_line_is_kept
     ("a\n\nb", 80, ["a", "", "b"]);  ("a\n   \nb", 80, ["a", "", "b"])
M-3  a_label_line_above_a_list_stays_its_own_block
     ("Shape\n- item", 80, ["Shape", "- item"])
     and blocks("Shape\n- item") == [Paragraph(vec!["Shape"]), Item { lead: "-", segments: vec!["item"] }]
M-4  a_list_item_continuation_joins_and_hangs
     ("- alpha beta\n  gamma delta epsilon", 16, ["- alpha beta", "  gamma delta", "  epsilon"])
     ("- alpha beta\ngamma", 80, ["- alpha beta gamma"])                  // lazy continuation
     ("- alpha beta gamma", 12, ["- alpha beta", "  gamma"])              // room 10: hangs
     ("- alpha beta gamma", 11, ["- alpha", "beta gamma"])                // room 9: column 0
     ("12. alpha beta gamma", 16, ["12. alpha beta", "    gamma"])
     ("  - alpha beta gamma", 16, ["  - alpha beta", "    gamma"])
     ("- item\n    deeper", 80, ["- item deeper"])
M-5  nested_and_ordered_markers_are_kept
     ("1. one\n  - two\n10) three\n-\n* star\n+ plus", 80,
      ["1. one", "  - two", "10) three", "- ", "* star", "+ plus"])
     ("text\n123456789. nine", 80, ["text", "123456789. nine"])         // 9 digits: an item
     ("text\n1234567890. ten", 80, ["text 1234567890. ten"])            // 10: text
     ("a\n-x", 80, ["a -x"])
M-6  structural_lines_stand_alone
     ("intro\n# Head `x`\nnext\n| a | `b` |\n|---|---|\n> quoted `q`\n> lines\n---\nend", 80,
      ["intro", "# Head x", "next", "| a | `b` |", "|---|---|", "> quoted q", "> lines", "---", "end"])
                                                                          // B-2: the table row keeps its backticks
     ("Title\n===\nbody", 80, ["Title", "===", "body"])
     ("#nope\nstill\n####### seven\nmore", 80, ["#nope still ####### seven more"])
     ("a\n* * *\nb", 80, ["a", "* * *", "b"]);  ("a\n- - -\nb", 80, ["a", "- - -", "b"])   // A-1
M-7  indented_code_only_when_nothing_is_open
     ("    let x = 1;\n\tfoo\ntext", 80, ["    let x = 1;", "    foo", "text"])
     ("para\n    continued", 80, ["para continued"])
M-8  a_fenced_block_is_verbatim
     ("intro\n```rust\nlet a\t= `b`;\n  - not a list\n```\nafter", 80,
      ["intro", "```rust", "let a   = `b`;", "  - not a list", "```", "after"])
     ("```\naaaa bbbb\n```", 5, ["```", "aaaa", "bbbb", "```"])        // a code line wraps on its own
     ("~~~\n- x\n```\ntext `y`", 80, ["~~~", "- x", "```", "text `y`"]) // unclosed: runs to the end
     ("````\n```\nx\n````\ny", 80, ["````", "```", "x", "````", "y"])   // closes at a run as long or longer
     ("```x``` y\nz", 80, ["x y z"])                                      // A-4: not a fence
M-9  hard_breaks_start_a_new_row
     ("one  \ntwo\\\nthree four", 80, ["one", "two", "three four"])
     ("- one  \n  two", 80, ["- one", "  two"]);  ("- one\\\ntwo", 80, ["- one", "  two"])
     ("one  \n\ntwo", 80, ["one", "", "two"])                             // a break at a block's end adds nothing
```

- **M-10 `inline_code_is_accent_without_backticks`** (Line equality; `t = Theme::default()`, `s = Span::styled`;
  width 80, one Line each):
  - `"a `b c` d"` → `[s("a ", t.base), s("b c", t.accent), s(" d", t.base)]`
  - `"x `a\nb` y"` → `[s("x ", t.base), s("a b", t.accent), s(" y", t.base)]` (joined, then paired)
  - ``"a `` `x` `` b"`` → `[s("a ", t.base), s("`x`", t.accent), s(" b", t.base)]` (one space stripped each end)
  - ``"`a``b`"`` → `[s("a``b", t.accent)]` (closes only at a run of its own length)
  - `"a `b"` → `[s("a `b", t.base)]`; ``"``a`"`` → `[s("``a`", t.base)]` (unmatched runs stay literal)
- **M-11 `row_count_is_the_number_of_lines`** (`#[tokio::test]`; body = `MemStore::demo().item(ids::HTUI_FEAT_1)
  .await.expect("the read").expect("FEAT-1").body`): at widths `0, 1, 5, 11, 12, 20, 43, 80, 200`,
  `row_count == lines().len()`; and `row_count(body, 0) == 21`, `row_count(body, 43) == 44`,
  `row_count(body, 20) == 91`.
- **M-12 `width_0_does_not_wrap`** (FEAT-1 body, width 0): 21 rows; row 0 is `"Stand up the terminal application: a
  workspace-scoped shell with a tab strip, a top bar and a backlog tab, all reading through the store seam."`; row 3
  is `"- Two crates: htui-core holds the domain model and the store traits, htui holds the terminal application.
  Nothing in the view layer holds a store handle."` (each one line in the test literal).
- **M-13 `the_feat_1_body_reflows_at_the_detail_width`** (FEAT-1 body, width 43): exactly the 44 rows of §4.3.

Validate: `cargo test -p htui --lib ui::markdown`.
Commit: `feat(mod-84): ui::markdown, the body classifier, joiner and drawer (T2)`.

## 3. T3 `crates/htui/src/ui/tabs/backlog/detail/body.rs` (D7)

`body` is `pub mod body` and `BodyTab` is public: **no intra-doc links** to `ui::markdown` or `ui::cells` in this file
(private items; `rustdoc::private_intra_doc_links = "deny"`). Plain backticks, as `notes.rs` writes `cells::wrap`.

- Imports: add `use std::cell::Cell;` and `use crate::ui::markdown;`. `Wrap` stays (the head block).
- Module doc, whole: "The Body sub-tab: the item's head fields and its Markdown body. // The body is hard-wrapped
  Markdown, so it is reflowed before it is wrapped (MOD-84): `ui::markdown` classes its lines (fenced and indented
  code, blank lines, list items, headings, rules, table rows, quotes and text), joins the soft-wrapped lines of a
  paragraph or list item, and wraps the result at the pane's width with a list item's rows hanging under its text.
  Blank lines, markers and structural lines are kept as written, a hard break keeps its row, and inline code is drawn
  in the accent style without its backticks. Nothing else is rendered: no emphasis, links or heading styles. The
  scroll clamps against the rows on screen at the last render's width, as the Notes pane does (MOD-13 review L1)."
- `BodyTab` doc: "Title, key, kind, status, tags, priority, version, and the body reflowed and wrapped at the pane's
  width (see the module doc)." (The "rendered raw, not parsed" paragraph goes.)
- Field, after `scroll`:
  ```rust
  /// The body area's width at the last render, for the scroll clamp (MOD-84 D7); 0 before the
  /// first, which counts the body unwrapped. `Cell` because `render` is `&self`.
  drawn: Cell<u16>,
  ```
  `#[derive(Debug, Default)]` covers it (`Cell<u16>: Debug + Default`). `on_item_change` does not reset it (a pane
  width, not an item's), as notes.
- `len`: doc "The body's rows as drawn at the last render's width: what the scroll clamps against (MOD-84 D7)." Body
  `self.item.as_ref().map_or(0, |item| markdown::row_count(&item.body, self.drawn.get()))`.
- `render`, after the layout split and head draw:
  ```rust
  self.drawn.set(body_area.width);
  if item.body.is_empty() { message(..); return; }            // unchanged
  let lines = markdown::lines(&item.body, body_area.width, ctx.theme);
  // A pane that widened since the last key has fewer rows than the offset assumed.
  let last = u16::try_from(lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
  frame.render_widget(
      Paragraph::new(Text::from(lines)).scroll((self.scroll.offset().min(last), 0)),
      body_area,
  );
  ```
  No `Wrap` on the body paragraph. The head block is unchanged.

### 3.1 T3 unit tests (new `#[cfg(test)] mod tests` in body.rs)

Imports: `crossterm::event::KeyCode`, `htui_core::fixtures::ids`, `htui_core::store::{MemStore, ReadStore as _}`,
`super::*`, `crate::ui::tabs::backlog::detail::compose::bench::{Shell, drawn, key}`. Helper `async fn pane(shell:
&Shell) -> BodyTab`: `new()`, `on_item_change(Some(ids::HTUI_FEAT_1))`, `on_reply(&StoreReply::Item(Box::new(
MemStore::demo().item(ids::HTUI_FEAT_1).await.expect("the item"))), &mut shell.ctx())`. Rows of a drawing:
`text.lines().map(str::trim_end)`.

| Test | Steps | Expect |
|---|---|---|
| B-1 `the_body_reads_as_paragraphs_at_the_detail_width` | `drawn(43, 30, render)` | rows `0..4` = `TUI scaffold`, `FEAT-1  FEAT  in_progress`, `rust · priority 2 · version 1`, `""`; rows `4..30` = §4.3 rows `0..26`; a row equals `top bar and a backlog tab, all reading` (A-2); no `` ` `` anywhere |
| B-2 `page_down_reaches_the_last_row_in_a_narrow_pane` (mirrors notes' L1 test) | `drawn(20, 12, ..)` (body 20x8); assert no `run loop.`; 20 x `on_key(key(KeyCode::PageDown))`; draw again | contains `run loop.` (row 90 of 91 at width 20; the old clamp stopped at line 25) |
| B-3 `a_pane_that_widened_clamps_to_its_last_row` | B-2's state (offset 90), then `drawn(43, 30, ..)` | row 4 is `  loop.` (§4.3 row 43) |
| B-4 `before_the_first_render_the_scroll_counts_unwrapped_rows` | fresh `pane`, 5 x `PageDown` (clamped at 20 = `row_count(.., 0) - 1`), then `drawn(43, 30, ..)` | row 4 is `Scope` (§4.3 row 20) |

### 3.2 Snapshots

Expected to change (body rows only; head, list, chrome, overlays byte-identical): see §5.2. Then the full gate.
Commit: `feat(mod-84): the Body pane reflows its body; snapshots (T3)`.

## 4. Reference renderings at width 43 (the Body pane's width in the 100x30 snapshots)

### 4.1 ANA-1 (`backlog__filter_form`, `filtered_list`, `list_grouped`, `shell__after_switch`, `waiting__platform_mixed`), 15 rows

```text
 0 The store seam is the one interface every
 1 other module reaches through, so it is
 2 settled first.
 3
 4 Postgres holds the truth; every box keeps a
 5 read-only cache it refreshes on connect.
 6 Writes are compare-and-set on a version
 7 column, so a losing edit is shown against
 8 its common ancestor rather than silently
 9 dropped.
10
11 Item keys are minted from a per-project
12 counter table, which keeps numbers gapless
13 and lets the legacy importer align the
14 counter above the numbers it brings in.
```

### 4.2 What disappears

Old ANA-1 rows `Writes are`, `ancestor rather`, `and lets the`; old FEAT-1 rows `top bar and a`, `event`, `the
terminal` and every backtick.

### 4.3 FEAT-1 (`backlog__detail_body` shows rows 0-19), 44 rows

```text
 0 Stand up the terminal application: a
 1 workspace-scoped shell with a tab strip, a
 2 top bar and a backlog tab, all reading
 3 through the store seam.
 4
 5 Shape
 6 - Two crates: htui-core holds the domain
 7   model and the store traits, htui holds
 8   the terminal application. Nothing in the
 9   view layer holds a store handle.
10 - A store worker task owns the backend. The
11   UI sends a request, the worker replies,
12   the event loop folds the reply into
13   state. Three select arms: terminal
14   events, store replies, a tick.
15 - Tabs, detail sub-tabs and overlays are
16   trait objects in registries, so a later
17   module adds a screen by registering it
18   rather than by editing the loop.
19
20 Scope
21 - Backlog list grouped by project and key
22   prefix, with the five detail sub-tabs:
23   body, runs, graph, documents, notes.
24 - A workspace switcher overlay, opened with
25   w.
26 - The top bar reads workspace, box, store
27   label and the active run count.
28
29 Out of scope
30 - Editing anything. Filters, actions and
31   the item editor are MOD-13.
32 - Driving an agent; the chat tab is MOD-2.
33 - Any real database. MOD-1 runs against the
34   in-memory store and its demo fixture.
35
36 Done when
37 - cargo run -- --demo opens on the backlog
38   of the Platform workspace.
39 - The snapshot tests render at 100x30 and
40   every sub-tab has an empty state.
41 - The terminal is restored on quit, on
42   panic and on an error out of the run
43   loop.
```

(`htui-core`, `htui`, `w`, `cargo run -- --demo` are accent spans; snapshots are text-only, so styles are not
visible there and M-10 pins them.) Width 20 ends `  error out of the` / `  run loop.` (91 rows).

## 5. Build order, gates, snapshots

### 5.1 Order and commits (serial, single lane; tests first in each task)

1. T1 cells (§1) → `cargo test -p htui --lib ui::cells`, `cargo test -p htui --lib notes` → commit.
2. T2 markdown (§2) → `cargo test -p htui --lib ui::markdown` → commit.
3. T3 body (§3) → `cargo test -p htui --lib detail::body` → snapshots (§5.2) → full gate (§5.3) → commit.
4. T4 close-out per the plan.

Stage explicit paths only (`git add <path>`); every commit ends with
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. `wrap_spans` (T1) and `lines`/`row_count` (T2) have no
non-test caller until T3, so `cargo clippy --workspace -- -D warnings` reports `dead_code` at the T1 and T2 commits.
Expected: do **not** add `#[allow(dead_code)]`/`#[expect]`; the clippy gates run after T3 only.

### 5.2 Snapshot update

`cargo-insta 1.48.0` is installed. **`--features testkit` is mandatory**: without it `tests/*.rs` run 0 tests and
report ok, and no snapshot is checked.

```bash
cargo insta test -p htui --features testkit --test backlog --test shell --test waiting --test integration --accept
git diff --stat -- crates/htui/tests/snapshots     # exactly the 6 files below
git diff -- crates/htui/tests/snapshots            # review: only rows inside the detail pane's body area move
```

| Snapshot | Item shown | Changes |
|---|---|---|
| `backlog__detail_body.snap` | htui FEAT-1 | yes: §4.3 rows 0-19 |
| `backlog__filter_form.snap` | htui ANA-1 (form overlay over part of it) | yes: visible rows of §4.1 |
| `backlog__filtered_list.snap` | htui ANA-1 | yes: §4.1 |
| `backlog__list_grouped.snap` | htui ANA-1 | yes: §4.1 |
| `shell__after_switch.snap` | htui ANA-1 | yes: §4.1 |
| `waiting__platform_mixed.snap` | htui ANA-1 (rows 4-10 covered) | yes: visible rows of §4.1 |
| `backlog__empty_body.snap` | htui ANA-2 | no (`No body for this item.`) |
| `backlog__list_last_row.snap` | agy FIX-1 | no |
| `integration__demo_shell.snap` | vulkan FEAT-1 | no |
| `shell__migration_prompt.snap`, `shell__offline_label.snap`, `shell__switcher_open.snap` | vulkan FEAT-1 | no |
| `waiting__graphics_empty.snap` | vulkan FEAT-1 | no |

Any other file in the diff, or a changed row outside the body area, is a regression: revert it and find the cause.
`tests/backlog.rs` asserts `contains("Stand up the terminal application")` twice; §4.3 row 0 keeps that text.

### 5.3 Full gate (after T3)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --all-features -- --test-threads=1
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Gate serially (`--test-threads=1`; the suite's green is scheduling-dependent). `*_pg` tests need the dev Postgres
DSN; a failure there with no body/snapshot in it is environmental, re-run before calling it a regression.

## 6. Hazards

- **Lints**: `[workspace.lints.clippy] all = warn` only (no pedantic, no `missing_docs`); `-D warnings` in the gate.
  Watch `manual_strip` (use `strip_suffix`/`strip_prefix`), `needless_range_loop` (byte scan uses `while`),
  `collapsible_if`, `redundant_clone`. `unused_qualifications` is on: no `std::iter::repeat_n` with `iter` imported.
  `missing_debug_implementations` is on: derive `Debug` on `Block`, `Open`, `Word`.
- **Docs convention**: every item, field and variant has a `///` doc (crate convention, not a lint). rustdoc
  `private_intra_doc_links = deny`: no links from `body.rs` (public module) to `ui::markdown`/`ui::cells`.
- **Dependencies**: `unicode-segmentation` 1.13 and `unicode-width` 0.2 are already `htui` deps; `Cargo.lock` must not
  move. MSRV 1.98, edition 2024 (let-chains and `iter::repeat_n` are fine).
- **Split before flatten** in `wrap_spans` (§1.2): flattening first turns `\t` into a splitting space and breaks C-1.
- **`lines()` not `split('\n')`** in `blocks`: notes uses `split` for V17 (a one-line note), the body wants CRLF safety
  and no row for a trailing newline.
- **Plan-accepted deviations from CommonMark** (no change, recorded so the reviewer does not re-raise them): any
  ordered marker interrupts a paragraph (CommonMark allows only `1.`); after a blank line, a 4-space-indented list
  continuation paragraph is indented code; backslash escapes are not interpreted; an item's continuation after a
  fence or a blank line inside a list item is a paragraph of its own and starts at column 0 (review LOW-2:
  `- a\n  ```\n  x\n  ```\n  more` draws `more` at column 0, `- a\n\n  b` draws `b` there).
- **Flakiness**: all new tests are pure or use `MemStore::demo()` with no timers; none depends on scheduling. Snapshot
  churn collides with MOD-81/82/83 (same files): re-accept after review on merge.
