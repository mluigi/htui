//! The hand-written frontmatter reader (ANA-22 §5.7): the YAML subset real `SKILL.md` files use,
//! read from text with no dependency and no loss.
//!
//! **Why hand-written.** `htui-core` takes no regex and no YAML dependency, and ANA-5 §4.8 made
//! "zero new dependencies" the reason [`crate::prompt::template`] may live here. ANA-22 §5.7 chose
//! this reader over a YAML crate for the same reason, and accepted its cost: exotic YAML is
//! **reported per key, never silently**.
//!
//! **What it reads.** `key: scalar`, `"…"` and `'…'` quoted scalars, inline flow lists `[a, b]`,
//! block lists (`- a`, at any indent width), block scalars (`|`, `|-`, `>`, `>-`), and a nested map
//! under a key — block form or a single-line flow map — kept as [`Value::Raw`] text. Everything
//! else is refused with an [`Issue`] and lands in [`Value::Raw`] too, so nothing is ever lost.
//!
//! **What it refuses to guess.** A scalar is never split on anything but, in [`Split::list`], a
//! comma. YAML 1.1's booleans are not honoured: `no`, `on`, `off` and `y` stay strings
//! ([`Split::flag`]), because a skill name is the library key and a name that reads `no` is a name.
//!
//! **Totality.** Every input either parses or produces an [`Issue`] or a [`FrontmatterError`];
//! there is no path that panics, and no path that drops text.

use serde::{Deserialize, Serialize};

/// How far the closing fence is searched for, in lines. A real frontmatter is under twenty lines;
/// the bound is what makes an unterminated file a refusal instead of a scan of a whole document.
pub const MAX_FRONTMATTER_LINES: usize = 200;

/// One frontmatter value. `Raw` is the lossless landing spot: it holds the **verbatim source
/// text** of a recognised construct (a nested map, a flow map) *and* of a refused one, and the
/// two differ only in whether an [`Issue`] names them (see [`Split::issues`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Value {
    /// A plain or quoted scalar, with quotes and escapes resolved and surrounding space trimmed.
    Scalar(String),
    /// A list, from an inline `[a, b]` or from `- a` lines. Each item is trimmed and unquoted.
    List(Vec<String>),
    /// The value's source text, verbatim, with the key's own indentation stripped per line and
    /// nothing else normalised.
    Raw(String),
}

/// A per-key problem. Never fatal: the rest of the file still parses and the body still imports
/// (ANA-22 §9, "the body still imports if the maintainer accepts `always`").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// The key the problem belongs to; the empty string for a line that named no key.
    pub key: String,
    /// Byte offset of the key's first byte, or of the offending line's first byte.
    pub at: usize,
    /// 1-based line number, for the report a human reads.
    pub line: usize,
    /// One sentence, saying what was not understood and what was kept.
    pub message: String,
}

impl core::fmt::Display for Issue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {} (byte {}): {}", self.line, self.at, self.message)
    }
}

/// One frontmatter entry, with where it starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The key, trimmed. Case-sensitive: `allowed-tools` and `allowedTools` are different keys.
    pub key: String,
    /// Its value.
    pub value: Value,
    /// Byte offset of the key's first byte — the token a cursor should sit on, the way
    /// [`crate::prompt::template::TemplateError`]'s `at` is the opening `{{`.
    pub at: usize,
    /// 1-based line number of the key.
    pub line: usize,
}

/// A file split into its frontmatter and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// The entries, in file order.
    pub frontmatter: Vec<Entry>,
    /// Everything after the closing fence, with the newline that ends the fence removed, line
    /// endings normalised, and exactly one trailing newline.
    pub body: String,
    /// Byte offset of the body's first byte in the file it was read from.
    pub body_at: usize,
    /// The keys this reader did not understand, in the order it met them.
    pub issues: Vec<Issue>,
}

/// Why a file is not a skill file at all. Two variants because a file either opens with a fence
/// and closes it, or it does not — every other problem is an [`Issue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrontmatterError {
    /// Line 1 is not `---`. A leading BOM is skipped; a leading blank line is not, because a `---`
    /// further down is a markdown horizontal rule, not a fence.
    #[error("the file does not open with a `---` fence on its first line")]
    NoFence,
    /// A fence opened and no closing fence followed within [`MAX_FRONTMATTER_LINES`]. Defensive:
    /// none of the 12,987 frontmatter files measured on the maintainer's machine needs it.
    #[error("the frontmatter opened with `---` and never closed, at byte {at}")]
    Unterminated {
        /// Byte offset of the opening fence.
        at: usize,
    },
}

impl Split {
    /// The first entry with this key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.frontmatter.iter().find(|entry| entry.key == key)
    }

    /// The key's value as a scalar, or `None` when it is a list, a `Raw`, or absent. Never coerces:
    /// a `Raw` is not a scalar even when it reads like one.
    #[must_use]
    pub fn scalar(&self, key: &str) -> Option<&str> {
        match &self.get(key)?.value {
            Value::Scalar(text) => Some(text.as_str()),
            _ => None,
        }
    }

    /// The key's value as a list. A [`Value::List`] is taken as it is; a [`Value::Scalar`] is
    /// split on **commas only** and each part trimmed, because a space-separated list of tools
    /// would otherwise become one bogus token. `Raw` is `None`.
    #[must_use]
    pub fn list(&self, key: &str) -> Option<Vec<String>> {
        match &self.get(key)?.value {
            Value::List(items) => Some(items.clone()),
            Value::Scalar(text) => Some(
                text.split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned)
                    .collect(),
            ),
            Value::Raw(_) => None,
        }
    }

    /// The key's value as a boolean, accepting **only** the literals `true` and `false`. YAML 1.1's
    /// `yes`/`no`/`on`/`off`/`y`/`n` are not booleans here (see the module doc).
    #[must_use]
    pub fn flag(&self, key: &str) -> Option<bool> {
        match self.scalar(key)? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    /// Whether the key appears at all, whatever its value — including a `Raw` and an empty one.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

/// One line of the input, with the byte span it occupies.
#[derive(Clone, Copy)]
struct Line<'a> {
    /// The line without its line ending, with any `\r` removed.
    text: &'a str,
    /// Byte offset of the line's first byte.
    start: usize,
    /// Byte offset just past the line's line ending, which is where the next line starts.
    end: usize,
    /// 1-based line number.
    number: usize,
}

impl Line<'_> {
    /// Leading spaces, counted. A tab counts as one: frontmatter in the wild is space-indented,
    /// and a tab is a reason to refuse, not to guess a width from.
    fn indent(&self) -> usize {
        self.text.len() - self.text.trim_start_matches([' ', '\t']).len()
    }

    /// The line without its leading or trailing space.
    fn trimmed(&self) -> &str {
        self.text.trim()
    }

    /// Whether this line is a `---` fence. Trailing space is allowed: a `---` with a stray space
    /// is still a fence, and refusing it would be the guess this reader does not make.
    fn is_fence(&self) -> bool {
        self.text.trim_end() == "---"
    }

    /// Whether this line names a list item.
    fn is_item(&self) -> bool {
        self.trimmed() == "-" || self.trimmed().starts_with("- ")
    }

    /// The item's text, with the dash and one space removed.
    fn item_text(&self) -> &str {
        let trimmed = self.trimmed();
        if trimmed == "-" { "" } else { &trimmed[2..] }
    }
}

/// Every line of `text`, with the span it occupies, `\r` stripped.
fn lines_of(text: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        let end = start + raw.len();
        let body = raw.strip_suffix('\n').unwrap_or(raw);
        lines.push(Line {
            text: body.strip_suffix('\r').unwrap_or(body),
            start,
            end,
            number: index + 1,
        });
        start = end;
    }
    lines
}

/// Splits a file into its frontmatter and its body.
///
/// Fails only on a missing or unterminated fence; every other problem is an [`Issue`] on the
/// returned [`Split`], and the body is returned either way.
pub fn split(text: &str) -> Result<Split, FrontmatterError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = lines_of(text);
    let first = lines.first().ok_or(FrontmatterError::NoFence)?;
    if !first.is_fence() {
        return Err(FrontmatterError::NoFence);
    }

    let limit = lines.len().min(1 + MAX_FRONTMATTER_LINES);
    let closing = (1..limit)
        .find(|&at| lines[at].is_fence())
        .ok_or(FrontmatterError::Unterminated { at: first.start })?;

    let (entries, issues) = read_entries(&lines[1..closing]);

    // The fence line's span includes its line ending, so its `end` is the body's first byte.
    let body_at = lines[closing].end;
    let body = normalise_body(text.get(body_at..).unwrap_or_default());

    Ok(Split {
        frontmatter: entries,
        body,
        body_at,
        issues,
    })
}

/// The body after the closing fence: the newline that ends the fence is already behind us, line
/// endings are normalised, one trailing newline is added — and nothing else is touched, because a
/// body's leading blank lines are part of it.
fn normalise_body(raw: &str) -> String {
    let mut body = raw.replace("\r\n", "\n");
    while body.ends_with('\n') {
        body.pop();
    }
    if !body.is_empty() {
        body.push('\n');
    }
    body
}

/// Reads the entries between the two fences.
fn read_entries(lines: &[Line<'_>]) -> (Vec<Entry>, Vec<Issue>) {
    let mut entries: Vec<Entry> = Vec::new();
    let mut issues: Vec<Issue> = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        if line.trimmed().is_empty() {
            index += 1;
            continue;
        }
        if line.is_item() {
            issues.push(issue(
                &line,
                "",
                "a list item with no key above it; the line is kept",
            ));
            index += 1;
            continue;
        }
        let Some(colon) = colon_of(&line) else {
            issues.push(issue(
                &line,
                "",
                "a frontmatter line is neither a key nor a list item; the line is kept",
            ));
            index += 1;
            continue;
        };

        let key = line.text[..colon].trim_end().to_owned();
        let rest = line.text[colon + 1..]
            .trim_start_matches([' ', '\t'])
            .trim_end();
        let (value, consumed) = read_value(&line, rest, &lines[index + 1..], &key, &mut issues);
        entries.push(Entry {
            key,
            value,
            at: line.start,
            line: line.number,
        });
        index += 1 + consumed;
    }

    (entries, issues)
}

fn issue(line: &Line<'_>, key: &str, message: &str) -> Issue {
    Issue {
        key: key.to_owned(),
        at: line.start,
        line: line.number,
        message: message.to_owned(),
    }
}

/// The byte index of the `:` that ends the key: the line's first colon.
///
/// Strict YAML wants a space after it, and this reader does not insist on one: a file written by a
/// generator that omits the space is a real file, and refusing it would lose a skill over a
/// space. Nothing is given up by being lenient here, because a top-level frontmatter line is
/// already required to be a `key: value` pair, a list item, or a nested block — there is no
/// "plain scalar at the top level" for a bare colon to be part of.
fn colon_of(line: &Line<'_>) -> Option<usize> {
    line.text.find(':')
}

/// Reads one value and says how many of the lines after the key it ate.
fn read_value(
    line: &Line<'_>,
    rest: &str,
    following: &[Line<'_>],
    key: &str,
    issues: &mut Vec<Issue>,
) -> (Value, usize) {
    if rest.is_empty() {
        return read_empty_value(line.indent(), following);
    }
    if let Some(indicator) = block_scalar_indicator(rest) {
        return read_block_scalar(indicator, line.indent(), following);
    }
    if rest.starts_with('[') {
        return read_flow_list(rest, line, key, issues);
    }
    if rest.starts_with('{') {
        return read_flow_map(rest, line.indent(), following, line, key, issues);
    }
    if rest.starts_with('"') || rest.starts_with('\'') {
        return read_quoted(rest, line, key, issues);
    }
    (Value::Scalar(rest.trim_end().to_owned()), 0)
}

/// A `key:` with nothing after it: a block list, a nested map, or an empty value.
fn read_empty_value(key_indent: usize, following: &[Line<'_>]) -> (Value, usize) {
    let Some(offset) = first_significant(following) else {
        return (Value::Scalar(String::new()), 0);
    };
    if following[offset].indent() <= key_indent {
        return (Value::Scalar(String::new()), 0);
    }
    if following[offset].is_item() {
        return read_block_list(key_indent, &following[offset..]);
    }
    // A more-indented line that is not an item is a nested map: kept whole and verbatim, because
    // §5.7 names `metadata:` as the one level this reader keeps rather than understands.
    let mut lines: Vec<Line<'_>> = Vec::new();
    for candidate in &following[offset..] {
        if !candidate.trimmed().is_empty() && candidate.indent() <= key_indent {
            break;
        }
        lines.push(*candidate);
    }
    trim_trailing_blanks(&mut lines);
    let base = lines
        .iter()
        .map(|entry| entry.indent())
        .filter(|indent| *indent > key_indent)
        .min()
        .unwrap_or(key_indent + 1);
    (Value::Raw(join_block(&lines, base)), lines.len())
}

/// The offset of the first line that is not blank.
fn first_significant(lines: &[Line<'_>]) -> Option<usize> {
    lines.iter().position(|line| !line.trimmed().is_empty())
}

/// Drops trailing blank lines, which belong to whatever follows rather than to this value.
fn trim_trailing_blanks(lines: &mut Vec<Line<'_>>) {
    while lines.last().is_some_and(|line| line.trimmed().is_empty()) {
        lines.pop();
    }
}

/// A block list: every more-indented `- ` line under the key. Indent width is never assumed — the
/// corpus has 2-space and 4-space lists.
fn read_block_list(key_indent: usize, following: &[Line<'_>]) -> (Value, usize) {
    let mut items: Vec<String> = Vec::new();
    let mut consumed = 0;
    for candidate in following {
        if candidate.trimmed().is_empty() {
            consumed += 1;
            continue;
        }
        if candidate.indent() <= key_indent || !candidate.is_item() {
            break;
        }
        items.push(unquote(candidate.item_text().trim()));
        consumed += 1;
    }
    let mut taken: Vec<Line<'_>> = following[..consumed].to_vec();
    trim_trailing_blanks(&mut taken);
    (Value::List(items), taken.len())
}

/// A `|` or `>` block scalar and the lines under it. `keep` is false for the `-` chomping
/// indicator, which drops the trailing newline.
fn read_block_scalar(
    (style, keep): (char, bool),
    key_indent: usize,
    following: &[Line<'_>],
) -> (Value, usize) {
    let mut lines: Vec<Line<'_>> = Vec::new();
    for candidate in following {
        if !candidate.trimmed().is_empty() && candidate.indent() <= key_indent {
            break;
        }
        lines.push(*candidate);
    }
    trim_trailing_blanks(&mut lines);

    let base = lines
        .iter()
        .map(|entry| entry.indent())
        .filter(|indent| *indent > key_indent)
        .min()
        .unwrap_or(key_indent + 1);
    let mut text = if style == '|' {
        join_block(&lines, base)
    } else {
        fold_block(&lines, base)
    };
    if keep {
        text.push('\n');
    }
    (Value::Scalar(text), lines.len())
}

/// The six block-scalar indicators YAML defines, reduced to the four the corpus uses.
fn block_scalar_indicator(rest: &str) -> Option<(char, bool)> {
    let mut chars = rest.chars();
    let style = chars.next()?;
    if style != '|' && style != '>' {
        return None;
    }
    match chars.as_str().trim() {
        "" => Some((style, true)),
        "-" => Some((style, false)),
        "+" => Some((style, true)),
        _ => None,
    }
}

/// Block lines with up to `base` leading spaces removed, joined with newlines.
fn join_block(lines: &[Line<'_>], base: usize) -> String {
    let mut out = String::new();
    for (at, line) in lines.iter().enumerate() {
        if at > 0 {
            out.push('\n');
        }
        out.push_str(strip_indent(line.text, base));
    }
    out
}

/// A folded block: blank lines become one newline, consecutive plain lines join with a space, and
/// a line indented further than `base` keeps its own newlines.
fn fold_block(lines: &[Line<'_>], base: usize) -> String {
    let mut out = String::new();
    let mut previous_was_text = false;
    for line in lines {
        let text = strip_indent(line.text, base);
        if text.is_empty() {
            out.push('\n');
            previous_was_text = false;
            continue;
        }
        let literal = line.indent() > base;
        if previous_was_text {
            out.push(if literal { '\n' } else { ' ' });
        }
        out.push_str(text.trim_end());
        previous_was_text = !literal;
    }
    out
}

/// Up to `base` leading spaces removed, where the line has them.
fn strip_indent(text: &str, base: usize) -> &str {
    let mut at = 0;
    while at < base && matches!(text.as_bytes().get(at), Some(b' ') | Some(b'\t')) {
        at += 1;
    }
    &text[at..]
}

/// `[a, b]` on one line.
fn read_flow_list(
    rest: &str,
    line: &Line<'_>,
    key: &str,
    issues: &mut Vec<Issue>,
) -> (Value, usize) {
    let Some(inner) = rest
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
    else {
        issues.push(issue(
            line,
            key,
            "an inline list does not close on this line; the value is kept as raw",
        ));
        return (Value::Raw(rest.to_owned()), 0);
    };
    let items = inner
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(unquote)
        .collect();
    (Value::List(items), 0)
}

/// A flow map: `{"a": {"b": [1]}}`, the shape 63 measured `SKILL.md` files use under `metadata:`.
/// Recognised when it closes, and kept verbatim; unclosed, it grows over the lines under it and is
/// then reported, because a nested document that runs past its line cannot be delimited.
fn read_flow_map(
    rest: &str,
    key_indent: usize,
    following: &[Line<'_>],
    line: &Line<'_>,
    key: &str,
    issues: &mut Vec<Issue>,
) -> (Value, usize) {
    if balanced(rest) {
        return (Value::Raw(rest.trim_end().to_owned()), 0);
    }
    let mut lines = vec![*line];
    for candidate in following {
        if candidate.trimmed().is_empty() || candidate.indent() <= key_indent {
            break;
        }
        lines.push(*candidate);
    }
    let joined = join_block(&lines, key_indent + 1);
    if balanced(&joined) {
        return (Value::Raw(joined), lines.len() - 1);
    }
    issues.push(issue(
        line,
        key,
        "a flow map does not close; the value is kept as raw text",
    ));
    (Value::Raw(joined), lines.len() - 1)
}

/// Whether every `{`/`[` in `text` is closed, ignoring braces inside quotes. Deliberately simple:
/// an unbalanced read is reported, never guessed at.
fn balanced(text: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in text.chars() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0 && quote.is_none()
}

/// A quoted scalar on one line. An unclosed quote is reported and the line kept as raw.
fn read_quoted(rest: &str, line: &Line<'_>, key: &str, issues: &mut Vec<Issue>) -> (Value, usize) {
    let quote = rest.chars().next().unwrap_or('"');
    let mut out = String::new();
    let mut chars = rest[quote.len_utf8()..].chars();
    while let Some(ch) = chars.next() {
        if ch == quote {
            return (Value::Scalar(out), 0);
        }
        if quote == '"' && ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => {
                    issues.push(issue(
                        line,
                        key,
                        "the value ends inside an escape; it is kept as raw",
                    ));
                    return (Value::Raw(rest.to_owned()), 0);
                }
            }
            continue;
        }
        out.push(ch);
    }
    issues.push(issue(
        line,
        key,
        "a quoted value does not close on this line; it is kept as raw",
    ));
    (Value::Raw(rest.to_owned()), 0)
}

/// Strips one layer of matching quotes, turning `''` into `'` inside a single-quoted scalar.
fn unquote(text: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return if quote == '\'' {
                inner.replace("''", "'")
            } else {
                inner.to_owned()
            };
        }
    }
    text.to_owned()
}
