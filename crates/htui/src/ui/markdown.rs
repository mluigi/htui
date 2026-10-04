//! An item body's Markdown, reflowed for a pane (MOD-84 D1-D4). Not a Markdown parser: each
//! source line is classed (fenced code, blank, rule, list item, indented code, heading, table row,
//! quote, text), soft-wrapped lines of a paragraph or list item are joined with one space, and the
//! result is wrapped at the pane's width by [`cells::wrap_spans`](crate::ui::cells::wrap_spans), a
//! list item's rows hanging under its text. Blank lines, markers, code and structural lines are
//! kept as written; a hard break (two trailing spaces, or `\`) starts a new row. Inline code is
//! drawn without its backticks in `theme.accent`, except in code and table rows. Emphasis, links
//! and heading styles are not rendered (MOD-80, MOD-82).

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::Theme;
use crate::ui::cells;

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
    /// A fence line, a line inside a fence, an indented-code line or a table row (B-2), as
    /// written: tabs expanded, wrapped, no inline code.
    Literal(&'a str),
    /// A paragraph: its text per hard break, soft-wrapped lines joined.
    Paragraph(Vec<String>),
    /// A list item: its indent and marker as written (`"  -"`, `"10)"`), and its text per hard
    /// break.
    Item {
        /// The indent and the marker, as written.
        lead: &'a str,
        /// The item's text per hard break, soft-wrapped lines joined.
        segments: Vec<String>,
    },
}

/// The paragraph or list item text lines join (D2.5).
#[derive(Debug, Clone, Copy)]
struct Open {
    /// Its index in the blocks.
    at: usize,
    /// Its last line ended in a hard break: the next text starts a new segment.
    broken: bool,
}

/// `body` as drawn at `width` (0: before the first render, unwrapped), one `Line` per row.
#[must_use]
pub(crate) fn lines(body: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let width = wrap_width(width);
    blocks(body)
        .iter()
        .flat_map(|block| draw(block, width, theme.base, theme.accent))
        .map(Line::from)
        .collect()
}

/// How many rows [`lines`] gives at `width`, without a theme (the scroll clamp, D7). Styles
/// never move a break, so this is `lines(..).len()`.
#[must_use]
pub(crate) fn row_count(body: &str, width: u16) -> usize {
    let width = wrap_width(width);
    blocks(body)
        .iter()
        .map(|block| draw(block, width, Style::new(), Style::new()).len())
        .sum()
}

/// The cells to wrap at: `width`, or no wrap before the first render, as the Notes pane's
/// `wrap_width`.
fn wrap_width(width: u16) -> usize {
    if width == 0 {
        usize::MAX
    } else {
        usize::from(width)
    }
}

/// `body`'s blocks (D2): per source line, the first rule that matches wins. `lines()`, so a CRLF
/// body reads as LF and a trailing newline adds no row.
fn blocks(body: &str) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    // The open fence's character and run length.
    let mut fence: Option<(u8, usize)> = None;
    let mut open: Option<Open> = None;
    for line in body.lines() {
        // R0: inside a fence, every line as written, up to the closing run.
        if let Some((ch, n)) = fence {
            out.push(Block::Literal(line));
            if closes(line, ch, n) {
                fence = None;
            }
            continue;
        }
        // R1: a fence opens.
        if let Some(opened) = fence_open(line) {
            open = None;
            out.push(Block::Literal(line));
            fence = Some(opened);
            continue;
        }
        // R2: a blank line.
        if line.trim().is_empty() {
            open = None;
            out.push(Block::Blank);
            continue;
        }
        // R3 (A-1): a rule, before a list item, so `* * *` is not one.
        if is_rule(line) {
            open = None;
            out.push(Block::Plain(line));
            continue;
        }
        // R4: a list item opens.
        if let Some((lead, rest)) = list_item(line) {
            let mut item = Open {
                at: out.len(),
                broken: false,
            };
            out.push(Block::Item {
                lead,
                segments: Vec::new(),
            });
            add(&mut out, &mut item, rest);
            open = Some(item);
            continue;
        }
        // R5: indented code, only when nothing is open; it opens nothing.
        if open.is_none() && indented(line) {
            out.push(Block::Literal(line));
            continue;
        }
        // R6: a heading or quote, or a table row (B-2), stands alone.
        if is_heading(line) || is_quote(line) {
            open = None;
            out.push(Block::Plain(line));
            continue;
        }
        if is_table(line) {
            open = None;
            out.push(Block::Literal(line));
            continue;
        }
        // R7: text joins the open block, or opens a paragraph.
        let mut joined = open.take().unwrap_or_else(|| {
            out.push(Block::Paragraph(Vec::new()));
            Open {
                at: out.len() - 1,
                broken: false,
            }
        });
        add(&mut out, &mut joined, line);
        open = Some(joined);
    }
    out
}

/// `raw` onto the open block (D2.5): trimmed, joined to its last segment with one space, or a
/// new segment after a hard break. A trailing `\` (dropped) or two spaces is a hard break.
fn add(blocks: &mut [Block<'_>], open: &mut Open, raw: &str) {
    let (text, hard) = match raw.strip_suffix('\\') {
        Some(head) => (head.trim(), true),
        None => (raw.trim(), raw.ends_with("  ")),
    };
    let Some(Block::Paragraph(segments) | Block::Item { segments, .. }) = blocks.get_mut(open.at)
    else {
        return;
    };
    match segments.last_mut() {
        Some(last) if !open.broken => {
            if !last.is_empty() && !text.is_empty() {
                last.push(' ');
            }
            last.push_str(text);
        }
        _ => segments.push(text.to_owned()),
    }
    open.broken = hard;
}

/// The count of `line`'s leading spaces (a tab is not one).
fn indent(line: &str) -> usize {
    run(line, b' ')
}

/// The count of `ch` bytes at the start of `text`.
fn run(text: &str, ch: u8) -> usize {
    text.bytes().take_while(|byte| *byte == ch).count()
}

/// The byte length of `line`'s leading run of spaces and tabs.
fn whitespace(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// `line` opens a fence: up to 3 spaces, then 3 or more `` ` `` or `~`; a backtick fence's line
/// holds no further backtick (A-4). Its character and run length.
fn fence_open(line: &str) -> Option<(u8, usize)> {
    let lead = indent(line);
    if lead > 3 {
        return None;
    }
    let rest = &line[lead..];
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let n = run(rest, ch);
    if n < 3 || (ch == b'`' && rest[n..].contains('`')) {
        return None;
    }
    Some((ch, n))
}

/// `line` closes a fence of `n` `ch`: up to 3 spaces, a run of `ch` at least as long, then only
/// whitespace.
fn closes(line: &str, ch: u8, n: usize) -> bool {
    let lead = indent(line);
    if lead > 3 {
        return false;
    }
    let rest = &line[lead..];
    let m = run(rest, ch);
    m >= n && rest[m..].trim().is_empty()
}

/// `line` is a rule or setext underline: up to 3 spaces, then 3 or more of one of `-*_=`, with
/// spaces and tabs between.
fn is_rule(line: &str) -> bool {
    if indent(line) > 3 {
        return false;
    }
    let mut marks = line.chars().filter(|c| *c != ' ' && *c != '\t');
    let Some(mark) = marks.next() else {
        return false;
    };
    if !matches!(mark, '-' | '*' | '_' | '=') {
        return false;
    }
    let mut count = 1;
    for other in marks {
        if other != mark {
            return false;
        }
        count += 1;
    }
    count >= 3
}

/// `line` as a list item: its indent and marker (`-`, `*`, `+`, or 1 to 9 digits and `.` or
/// `)`), and its text after the one space that follows (empty when nothing follows).
fn list_item(line: &str) -> Option<(&str, &str)> {
    let ws = whitespace(line);
    let after = &line[ws..];
    let marker = match after.as_bytes().first()? {
        b'-' | b'*' | b'+' => 1,
        _ => {
            let digits = after.bytes().take_while(u8::is_ascii_digit).count();
            if !(1..=9).contains(&digits) {
                return None;
            }
            match after.as_bytes().get(digits) {
                Some(b'.' | b')') => digits + 1,
                _ => return None,
            }
        }
    };
    let end = ws + marker;
    let tail = &line[end..];
    if tail.is_empty() {
        return Some((line, ""));
    }
    tail.strip_prefix(' ').map(|rest| (&line[..end], rest))
}

/// `line` is indented 4 or more cells by spaces and tabs (indented code when nothing is open).
fn indented(line: &str) -> bool {
    cells::cell_width(&cells::expand_tabs(&line[..whitespace(line)])) >= 4
}

/// `line` is an ATX heading: up to 3 spaces, 1 to 6 `#`, then a space or the end.
fn is_heading(line: &str) -> bool {
    let lead = indent(line);
    if lead > 3 {
        return false;
    }
    let rest = &line[lead..];
    let hashes = run(rest, b'#');
    (1..=6).contains(&hashes) && matches!(rest.as_bytes().get(hashes), None | Some(b' '))
}

/// `line` is a block quote.
fn is_quote(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

/// `line` is a table row.
fn is_table(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// `text` as spans, inline code (D4) in `code` without its backticks, the rest in `base`. A run
/// of n backticks closes at the next run of exactly n; one with no match stays literal. When the
/// code both starts and ends with a space (and is not all spaces), one is stripped from each end.
/// Backslash escapes are not interpreted.
fn inline(text: &str, base: Style, code: Style) -> Vec<Span<'_>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut plain = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        // A backtick is ASCII, so every slice below is on a char boundary.
        let n = run(&text[at..], b'`');
        let mut probe = at + n;
        let mut close = None;
        while probe < bytes.len() {
            if bytes[probe] == b'`' {
                let m = run(&text[probe..], b'`');
                if m == n {
                    close = Some(probe);
                    break;
                }
                probe += m;
            } else {
                probe += 1;
            }
        }
        let Some(end) = close else {
            // The whole run stays literal.
            at += n;
            continue;
        };
        if plain < at {
            out.push(Span::styled(&text[plain..at], base));
        }
        let mut content = &text[at + n..end];
        if content.len() >= 2
            && content.starts_with(' ')
            && content.ends_with(' ')
            && content.bytes().any(|byte| byte != b' ')
        {
            content = &content[1..content.len() - 1];
        }
        out.push(Span::styled(content, code));
        at = end + n;
        plain = at;
    }
    if plain < bytes.len() {
        out.push(Span::styled(&text[plain..], base));
    }
    out
}

/// `block`'s rows at `width` (D3), `usize::MAX` for no wrap.
fn draw(block: &Block<'_>, width: usize, base: Style, code: Style) -> Vec<Vec<Span<'static>>> {
    match block {
        Block::Blank => vec![Vec::new()],
        Block::Plain(line) => cells::wrap_spans(&inline(line, base, code), width),
        Block::Literal(line) => {
            cells::wrap_spans(&[Span::styled(cells::expand_tabs(line), base)], width)
        }
        Block::Paragraph(segments) => segments
            .iter()
            .flat_map(|segment| cells::wrap_spans(&inline(segment, base, code), width))
            .collect(),
        Block::Item { lead, segments } => item(lead, segments, width, base, code),
    }
}

/// A list item's rows at `width` (D3): its lead and a space, then its text, the rows after the
/// first hanging under the text's first cell; with fewer than [`MIN_TEXT`] cells there, every
/// row after the first starts at column 0.
fn item(
    lead: &str,
    segments: &[String],
    width: usize,
    base: Style,
    code: Style,
) -> Vec<Vec<Span<'static>>> {
    let prefix = format!("{} ", cells::expand_tabs(lead));
    let hang = cells::cell_width(&prefix);
    if let Some(room) = width.checked_sub(hang).filter(|room| *room >= MIN_TEXT) {
        let mut rows: Vec<Vec<Span<'static>>> = segments
            .iter()
            .flat_map(|segment| cells::wrap_spans(&inline(segment, base, code), room))
            .collect();
        for (at, row) in rows.iter_mut().enumerate() {
            let head = if at == 0 {
                prefix.clone()
            } else {
                " ".repeat(hang)
            };
            row.insert(0, Span::styled(head, base));
        }
        return rows;
    }
    let Some((first, rest)) = segments.split_first() else {
        return vec![vec![Span::styled(prefix, base)]];
    };
    let mut spans = vec![Span::styled(prefix, base)];
    spans.extend(inline(first, base, code));
    let mut rows = cells::wrap_spans(&spans, width);
    for segment in rest {
        rows.extend(cells::wrap_spans(&inline(segment, base, code), width));
    }
    rows
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::store::{MemStore, ReadStore as _};

    use super::*;

    /// `body` as drawn at `width` with the default theme, one string per row.
    fn rows(body: &str, width: u16) -> Vec<String> {
        lines(body, width, &Theme::default())
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Each case's rows, and [`row_count`] agreeing with them.
    fn check(cases: &[(&str, u16, &[&str])]) {
        for (body, width, want) in cases {
            let got = rows(body, *width);
            assert_eq!(got, *want, "{body:?} at {width}");
            assert_eq!(row_count(body, *width), got.len(), "{body:?} at {width}");
        }
    }

    /// FEAT-1's body from the demo fixture.
    async fn feat_1() -> String {
        MemStore::demo()
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("the read")
            .expect("FEAT-1")
            .body
    }

    #[test]
    fn a_hard_wrapped_paragraph_joins() {
        check(&[
            ("one two\nthree four", 80, &["one two three four"]),
            ("one two\nthree four", 9, &["one two", "three", "four"]),
            ("a\r\nb", 80, &["a b"]),
        ]);
    }

    #[test]
    fn a_blank_line_is_kept() {
        check(&[
            ("a\n\nb", 80, &["a", "", "b"]),
            ("a\n   \nb", 80, &["a", "", "b"]),
        ]);
    }

    #[test]
    fn a_label_line_above_a_list_stays_its_own_block() {
        check(&[("Shape\n- item", 80, &["Shape", "- item"])]);
        assert_eq!(
            blocks("Shape\n- item"),
            [
                Block::Paragraph(vec!["Shape".to_owned()]),
                Block::Item {
                    lead: "-",
                    segments: vec!["item".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn a_list_item_continuation_joins_and_hangs() {
        check(&[
            (
                "- alpha beta\n  gamma delta epsilon",
                16,
                &["- alpha beta", "  gamma delta", "  epsilon"],
            ),
            // Lazy continuation.
            ("- alpha beta\ngamma", 80, &["- alpha beta gamma"]),
            // Room 10: the rows hang.
            ("- alpha beta gamma", 12, &["- alpha beta", "  gamma"]),
            // Room 9: column 0.
            ("- alpha beta gamma", 11, &["- alpha", "beta gamma"]),
            ("12. alpha beta gamma", 16, &["12. alpha beta", "    gamma"]),
            ("  - alpha beta gamma", 16, &["  - alpha beta", "    gamma"]),
            ("- item\n    deeper", 80, &["- item deeper"]),
        ]);
    }

    #[test]
    fn nested_and_ordered_markers_are_kept() {
        check(&[
            (
                "1. one\n  - two\n10) three\n-\n* star\n+ plus",
                80,
                &["1. one", "  - two", "10) three", "- ", "* star", "+ plus"],
            ),
            // Nine digits: an item.
            ("text\n123456789. nine", 80, &["text", "123456789. nine"]),
            // Ten: text.
            ("text\n1234567890. ten", 80, &["text 1234567890. ten"]),
            ("a\n-x", 80, &["a -x"]),
        ]);
    }

    #[test]
    fn structural_lines_stand_alone() {
        check(&[
            (
                "intro\n# Head `x`\nnext\n| a | `b` |\n|---|---|\n> quoted `q`\n> lines\n---\nend",
                80,
                // B-2: the table row keeps its backticks.
                &[
                    "intro",
                    "# Head x",
                    "next",
                    "| a | `b` |",
                    "|---|---|",
                    "> quoted q",
                    "> lines",
                    "---",
                    "end",
                ],
            ),
            ("Title\n===\nbody", 80, &["Title", "===", "body"]),
            (
                "#nope\nstill\n####### seven\nmore",
                80,
                &["#nope still ####### seven more"],
            ),
            // A-1: a rule is not a list item.
            ("a\n* * *\nb", 80, &["a", "* * *", "b"]),
            ("a\n- - -\nb", 80, &["a", "- - -", "b"]),
        ]);
    }

    #[test]
    fn indented_code_only_when_nothing_is_open() {
        check(&[
            (
                "    let x = 1;\n\tfoo\ntext",
                80,
                &["    let x = 1;", "    foo", "text"],
            ),
            ("para\n    continued", 80, &["para continued"]),
        ]);
    }

    #[test]
    fn a_fenced_block_is_verbatim() {
        check(&[
            (
                "intro\n```rust\nlet a\t= `b`;\n  - not a list\n```\nafter",
                80,
                &[
                    "intro",
                    "```rust",
                    "let a   = `b`;",
                    "  - not a list",
                    "```",
                    "after",
                ],
            ),
            // A code line wraps on its own.
            ("```\naaaa bbbb\n```", 5, &["```", "aaaa", "bbbb", "```"]),
            // Unclosed: it runs to the end.
            (
                "~~~\n- x\n```\ntext `y`",
                80,
                &["~~~", "- x", "```", "text `y`"],
            ),
            // It closes at a run as long or longer.
            (
                "````\n```\nx\n````\ny",
                80,
                &["````", "```", "x", "````", "y"],
            ),
            // A-4: a backtick after the opening run means it is not a fence.
            ("```x``` y\nz", 80, &["x y z"]),
        ]);
    }

    #[test]
    fn hard_breaks_start_a_new_row() {
        check(&[
            (
                "one  \ntwo\\\nthree four",
                80,
                &["one", "two", "three four"],
            ),
            ("- one  \n  two", 80, &["- one", "  two"]),
            ("- one\\\ntwo", 80, &["- one", "  two"]),
            // A break at a block's end adds nothing.
            ("one  \n\ntwo", 80, &["one", "", "two"]),
        ]);
    }

    #[test]
    fn inline_code_is_accent_without_backticks() {
        let t = Theme::default();
        let s = Span::styled;
        let cases: [(&str, Vec<Span<'static>>); 6] = [
            (
                "a `b c` d",
                vec![s("a ", t.base), s("b c", t.accent), s(" d", t.base)],
            ),
            // Joined, then paired.
            (
                "x `a\nb` y",
                vec![s("x ", t.base), s("a b", t.accent), s(" y", t.base)],
            ),
            // One space stripped from each end.
            (
                "a `` `x` `` b",
                vec![s("a ", t.base), s("`x`", t.accent), s(" b", t.base)],
            ),
            // It closes only at a run of its own length.
            ("`a``b`", vec![s("a``b", t.accent)]),
            // Unmatched runs stay literal.
            ("a `b", vec![s("a `b", t.base)]),
            ("``a`", vec![s("``a`", t.base)]),
        ];
        for (body, want) in cases {
            assert_eq!(lines(body, 80, &t), [Line::from(want)], "{body:?}");
        }
    }

    #[tokio::test]
    async fn row_count_is_the_number_of_lines() {
        let body = feat_1().await;
        let theme = Theme::default();
        for width in [0, 1, 5, 11, 12, 20, 43, 80, 200] {
            assert_eq!(
                row_count(&body, width),
                lines(&body, width, &theme).len(),
                "at {width}"
            );
        }
        assert_eq!(row_count(&body, 0), 21);
        assert_eq!(row_count(&body, 43), 44);
        assert_eq!(row_count(&body, 20), 91);
    }

    #[tokio::test]
    async fn width_0_does_not_wrap() {
        let rows = rows(&feat_1().await, 0);
        assert_eq!(rows.len(), 21, "{rows:#?}");
        assert_eq!(
            rows[0],
            "Stand up the terminal application: a workspace-scoped shell with a tab strip, a top bar and a backlog tab, all reading through the store seam."
        );
        assert_eq!(
            rows[3],
            "- Two crates: htui-core holds the domain model and the store traits, htui holds the terminal application. Nothing in the view layer holds a store handle."
        );
    }

    #[tokio::test]
    async fn the_feat_1_body_reflows_at_the_detail_width() {
        assert_eq!(
            rows(&feat_1().await, 43),
            [
                "Stand up the terminal application: a",
                "workspace-scoped shell with a tab strip, a",
                "top bar and a backlog tab, all reading",
                "through the store seam.",
                "",
                "Shape",
                "- Two crates: htui-core holds the domain",
                "  model and the store traits, htui holds",
                "  the terminal application. Nothing in the",
                "  view layer holds a store handle.",
                "- A store worker task owns the backend. The",
                "  UI sends a request, the worker replies,",
                "  the event loop folds the reply into",
                "  state. Three select arms: terminal",
                "  events, store replies, a tick.",
                "- Tabs, detail sub-tabs and overlays are",
                "  trait objects in registries, so a later",
                "  module adds a screen by registering it",
                "  rather than by editing the loop.",
                "",
                "Scope",
                "- Backlog list grouped by project and key",
                "  prefix, with the five detail sub-tabs:",
                "  body, runs, graph, documents, notes.",
                "- A workspace switcher overlay, opened with",
                "  w.",
                "- The top bar reads workspace, box, store",
                "  label and the active run count.",
                "",
                "Out of scope",
                "- Editing anything. Filters, actions and",
                "  the item editor are MOD-13.",
                "- Driving an agent; the chat tab is MOD-2.",
                "- Any real database. MOD-1 runs against the",
                "  in-memory store and its demo fixture.",
                "",
                "Done when",
                "- cargo run -- --demo opens on the backlog",
                "  of the Platform workspace.",
                "- The snapshot tests render at 100x30 and",
                "  every sub-tab has an empty state.",
                "- The terminal is restored on quit, on",
                "  panic and on an error out of the run",
                "  loop.",
            ]
        );
    }
}
