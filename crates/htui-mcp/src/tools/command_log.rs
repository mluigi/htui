//! `command_output_read` (MOD-152 D1-D5): the log a command left, read in slices.
//!
//! Every `command_run` row gets a log of its merged output, kept by [`LogBuffer`] while the
//! command runs and stored by [`LogBuffer::into_stored`] before the row ends (D4): the tail of at
//! most [`COMMAND_LOG_CAP`] bytes, opening with the executor's truncation line when earlier output
//! was dropped and starting on a line boundary, scrubbed as one text and fail closed (D3). The
//! tool answers a slice of it: the last lines, a line range, or the lines matching a pattern, at
//! most `max_bytes`. Line numbers are those of the command's output, counted from its first line
//! (MOD-152 R1 F2): a cut log's truncation line says which lines it stands for
//! ([`truncated_line`]) and takes the number of the last of them, so a line keeps its number
//! while the live log slides and once the log is stored. The read is scoped to the caller's run
//! (D5); a command the session started and that still runs is read from its live log (MOD-152
//! T5).

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use htui_core::model::{COMMAND_LOG_CAP, CommandLog, CommandRunId};
use htui_core::scrub::Scrubber;
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, store_error};

/// The line a cut row output opens with (the executor's own, OQ-7).
pub(crate) const TRUNCATED: &str = "[… earlier output truncated]\n";

/// How a cut log's truncation line opens: [`TRUNCATED`]'s text, then the lines it stands for.
const TRUNCATED_LINES: &str = "[… earlier output truncated] (lines 1-";

/// How a withheld log opens.
const WITHHELD: &str = "[log withheld: it matched credential rule ";

/// MOD-152 R1 F2: the line a cut log opens with, standing for its `dropped` first lines (at
/// least one: the cut dropped a line, or the start of one).
pub(crate) fn truncated_line(dropped: usize) -> String {
    format!("{TRUNCATED_LINES}{})\n", dropped.max(1))
}

/// What a line's place in `text` (counted from 1) adds up to its line number in the command's
/// output: the lines a cut log's truncation line stands for, but the one it takes itself.
fn offset_of(text: &str, truncated: bool) -> usize {
    if !truncated {
        return 0;
    }
    text.strip_prefix(TRUNCATED_LINES)
        .and_then(|rest| rest.split_once(')'))
        .and_then(|(dropped, _)| dropped.parse::<usize>().ok())
        .map_or(0, |dropped| dropped.saturating_sub(1))
}

/// A log as a reader serves it (D3, MOD-152 R1 F1): scrubbed as one text, fail closed, before any
/// slice of it is taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScrubbedLog {
    /// The text; a cut log opens with its [`truncated_line`], an unmaskable one is the withheld
    /// line.
    pub(crate) text: String,
    /// Earlier output was dropped.
    pub(crate) truncated: bool,
    /// The text could not be masked: it is the withheld line, which matches no pattern.
    pub(crate) withheld: bool,
    /// What a line's place in `text` adds up to its line number in the command's output (F2).
    pub(crate) offset: usize,
}

impl ScrubbedLog {
    /// `text` scrubbed by `scrubber`, or the withheld line.
    pub(crate) fn of(text: String, truncated: bool, scrubber: &dyn Scrubber) -> Self {
        match scrub(scrubber, text) {
            Ok(text) => Self {
                offset: offset_of(&text, truncated),
                // A stored log the runner withheld stays withheld.
                withheld: text.starts_with(WITHHELD) && !text.contains('\n'),
                text,
                truncated,
            },
            Err(rule) => Self {
                text: withheld(&rule),
                truncated,
                withheld: true,
                offset: 0,
            },
        }
    }

    /// The number of the log's last line.
    pub(crate) fn total_lines(&self) -> usize {
        self.offset + self.text.lines().count()
    }
}

/// Lines a tail read answers when it names no mode.
const DEFAULT_TAIL: usize = 200;

/// The default and the most a read answers, in bytes.
const DEFAULT_MAX_BYTES: usize = 16 * 1024;
const MAX_MAX_BYTES: usize = 64 * 1024;

/// Lines of context around a match by default, and the most.
const DEFAULT_CONTEXT: usize = 2;
const MAX_CONTEXT: usize = 20;

/// The most a compiled pattern may weigh, in bytes.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// What a read of a missing or unwritten log answers.
const NO_LOG: &str = "not found: command log";

// Exactly one of `pattern`, `from_line` with `to_line`, or `tail_lines`; with none, the last 200
// lines. A plain comment, not a doc: schemars would hand a doc to the agent as the schema's
// description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct OutputReadArgs {
    /// The command to read; default the latest one this run queued.
    command_run_id: Option<String>,
    /// Answer the last this-many lines (default 200).
    tail_lines: Option<u32>,
    /// With `to_line`: answer these lines, counted from 1, both included.
    from_line: Option<u32>,
    /// With `from_line`.
    to_line: Option<u32>,
    /// Answer the lines this regular expression matches, each numbered `N:`, with their context
    /// numbered `N-`.
    pattern: Option<String>,
    /// Lines of context around each match, 0 to 20; default 2.
    context: Option<u32>,
    /// The most bytes to answer, 1 to 65536; default 16384.
    max_bytes: Option<u32>,
}

/// Advertised with `command_run`.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "command_output_read",
    description: "Reads the output a command of this run left: the last lines, a line range, or the lines matching a pattern.",
    schema: schema_of::<OutputReadArgs>,
    advertised: |scope, _| scope.command_queue,
};

/// `{"command_run_id", "status", "exit_code", "total_lines", "log_truncated", "text",
/// "first_line", "last_line", "capped"}`.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let read: OutputReadArgs = args(arguments)?;
    let session = ctx.session;
    let max_bytes = max_bytes(read.max_bytes)?;
    let mode = Mode::of(&read)?;
    let id = match &read.command_run_id {
        None => None,
        Some(text) => Some(
            text.parse::<CommandRunId>()
                .map_err(|_| refuse(&format!("command_run_id `{text}` is not an id")))?,
        ),
    };
    let scrubber = session.scrubber.as_ref();
    let read_store = |id| async move {
        htui_core::store::WorkerStore::command_log_of_run(&session.store, session.scope.run_id, id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| ToolError(NO_LOG.to_owned()))
    };
    let mut stored = read_store(id).await?;
    let row_id = stored.row.id;
    // MOD-152 T5: a command this session started and that still runs is read from its live log,
    // complete lines only, scrubbed as one text before a pattern sees it (D3). R1 F3: a row that
    // ended since the store was read has its log and status in the store now: read it again.
    let live = session
        .commands
        .get(row_id)
        .map(|live| live.snapshot(scrubber));
    let (log, status, exit_code) = match live {
        Some((phase, Some(log))) => (log, phase.status(), None),
        live => {
            if live.is_some() {
                stored = read_store(Some(row_id)).await?;
            }
            let log = stored.log.ok_or_else(|| ToolError(NO_LOG.to_owned()))?;
            (
                std::sync::Arc::new(ScrubbedLog::of(log.text, log.truncated, scrubber)),
                stored.row.status.as_str(),
                stored.row.exit_code,
            )
        }
    };
    let log_truncated = log.truncated;

    let total_lines = log.total_lines();
    let mut slice = slice(&log.text, &mode, max_bytes, log.offset);
    // D3: the stored text was scrubbed; the slice is scrubbed again on its way out, fail closed.
    match scrub(scrubber, slice.text) {
        Ok(text) => slice.text = text,
        Err(rule) => {
            slice.text = withheld(&rule);
        }
    }
    Ok(json!({
        "command_run_id": row_id,
        "status": status,
        "exit_code": exit_code,
        "total_lines": total_lines,
        "log_truncated": log_truncated,
        "text": slice.text,
        "first_line": slice.first_line,
        "last_line": slice.last_line,
        "capped": slice.capped,
    }))
}

/// `max_bytes` or its default, 1 to [`MAX_MAX_BYTES`].
fn max_bytes(asked: Option<u32>) -> Result<usize, ToolError> {
    let Some(asked) = asked else {
        return Ok(DEFAULT_MAX_BYTES);
    };
    match usize::try_from(asked) {
        Ok(bytes) if (1..=MAX_MAX_BYTES).contains(&bytes) => Ok(bytes),
        _ => Err(ToolError(format!(
            "refused: max_bytes {asked} is not 1..={MAX_MAX_BYTES}"
        ))),
    }
}

/// How a read picks its lines.
#[derive(Debug)]
pub(crate) enum Mode {
    /// The last this many lines.
    Tail(usize),
    /// Lines `from..=to`, counted from 1.
    Range(usize, usize),
    /// The lines `pattern` matches, with `context` lines around each.
    Pattern {
        /// The compiled pattern.
        pattern: Regex,
        /// Lines around a match.
        context: usize,
    },
}

impl Mode {
    /// The one mode the arguments name, or a one-line refusal.
    fn of(read: &OutputReadArgs) -> Result<Self, ToolError> {
        let ranged = read.from_line.is_some() || read.to_line.is_some();
        let modes = [read.pattern.is_some(), ranged, read.tail_lines.is_some()];
        if modes.into_iter().filter(|named| *named).count() > 1 {
            return Err(refuse(
                "give one of pattern, from_line with to_line, or tail_lines",
            ));
        }
        if read.context.is_some() && read.pattern.is_none() {
            return Err(refuse("context goes with pattern"));
        }
        if let Some(pattern) = &read.pattern {
            let context = match read.context {
                None => DEFAULT_CONTEXT,
                Some(asked) => usize::try_from(asked)
                    .ok()
                    .filter(|context| *context <= MAX_CONTEXT)
                    .ok_or_else(|| refuse(&format!("context {asked} is not 0..={MAX_CONTEXT}")))?,
            };
            let pattern = compile(pattern)?;
            return Ok(Self::Pattern { pattern, context });
        }
        if ranged {
            let (Some(from), Some(to)) = (read.from_line, read.to_line) else {
                return Err(refuse("from_line and to_line go together"));
            };
            if from == 0 {
                return Err(refuse("from_line starts at 1"));
            }
            if from > to {
                return Err(refuse(&format!("from_line {from} is after to_line {to}")));
            }
            return Ok(Self::Range(widen(from), widen(to)));
        }
        match read.tail_lines {
            None => Ok(Self::Tail(DEFAULT_TAIL)),
            Some(0) => Err(refuse("tail_lines must be 1 or more")),
            Some(lines) => Ok(Self::Tail(widen(lines))),
        }
    }
}

/// `pattern` compiled within [`REGEX_SIZE_LIMIT`], or the one-line refusal.
pub(crate) fn compile(pattern: &str) -> Result<Regex, ToolError> {
    RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|err| refuse(&format!("pattern: {}", one_line(&err.to_string()))))
}

/// `refused: <reason>`.
fn refuse(reason: &str) -> ToolError {
    ToolError(format!("refused: {reason}"))
}

/// A `u32` as a `usize` (lossless on every platform htui builds for).
fn widen(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// The last line of a multi-line regex error: its `error: ...` sentence.
fn one_line(message: &str) -> &str {
    let last = message.lines().last().unwrap_or(message);
    last.strip_prefix("error: ").unwrap_or(last).trim()
}

/// A slice of a log.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Slice {
    /// The lines, each ending in a newline; matches and their context are numbered in pattern
    /// mode.
    pub(crate) text: String,
    /// The first line the slice holds, counted from 1; `None` when it holds none.
    pub(crate) first_line: Option<usize>,
    /// The last line the slice holds.
    pub(crate) last_line: Option<usize>,
    /// Whether `max_bytes` cut it: more lines were asked for than it holds.
    pub(crate) capped: bool,
}

/// What a slice accumulates, within `max` bytes.
struct Emit {
    out: String,
    max: usize,
    capped: bool,
    first: Option<usize>,
    last: Option<usize>,
}

impl Emit {
    fn new(max: usize) -> Self {
        Self {
            out: String::new(),
            max,
            capped: false,
            first: None,
            last: None,
        }
    }

    /// Adds line `number` (counted from 1) after `prefix`, and `separator` before it; `false`
    /// when it did not fit, and the slice is capped. A first line that alone is over the budget
    /// is cut to it.
    fn line(&mut self, number: usize, separator: &str, prefix: &str, text: &str) -> bool {
        let need = separator.len() + prefix.len() + text.len() + 1;
        let room = self.max.saturating_sub(self.out.len());
        if need > room {
            self.capped = true;
            let fixed = separator.len() + prefix.len() + 1;
            if self.out.is_empty() && room > fixed {
                let kept = cut_head(text, room - fixed);
                self.push(number, separator, prefix, kept);
            }
            return false;
        }
        self.push(number, separator, prefix, text);
        true
    }

    fn push(&mut self, number: usize, separator: &str, prefix: &str, text: &str) {
        self.out.push_str(separator);
        self.out.push_str(prefix);
        self.out.push_str(text);
        self.out.push('\n');
        self.first.get_or_insert(number);
        self.last = Some(number);
    }

    fn finish(self) -> Slice {
        Slice {
            text: self.out,
            first_line: self.first,
            last_line: self.last,
            capped: self.capped,
        }
    }
}

/// The first at most `max` bytes of `text`, ending on a character boundary.
pub(crate) fn cut_head(text: &str, max: usize) -> &str {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The last at most `max` bytes of `text`, starting on a character boundary.
fn cut_tail(text: &str, max: usize) -> &str {
    let mut start = text.len().saturating_sub(max);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// The slice of `text` that `mode` names, within `max_bytes`. A line's number is its place in
/// `text` plus `offset` (MOD-152 R1 F2): a range names those numbers, and the slice answers
/// them.
pub(crate) fn slice(text: &str, mode: &Mode, max_bytes: usize, offset: usize) -> Slice {
    let lines: Vec<&str> = text.lines().collect();
    let mut emit = Emit::new(max_bytes);
    match mode {
        Mode::Tail(count) => {
            let wanted = lines.len().saturating_sub(*count);
            let mut start = lines.len();
            let mut used = 0_usize;
            while start > wanted && used + lines[start - 1].len() < max_bytes {
                start -= 1;
                used += lines[start].len() + 1;
            }
            if start == lines.len() {
                // Not even the last line fits whole: its tail is the answer.
                if let Some(last) = lines.last() {
                    let kept = cut_tail(last, max_bytes.saturating_sub(1));
                    emit.push(offset + lines.len(), "", "", kept);
                    emit.capped = true;
                }
            } else {
                for (index, line) in lines.iter().enumerate().skip(start) {
                    emit.line(offset + index + 1, "", "", line);
                }
                emit.capped = start > wanted;
            }
        }
        Mode::Range(from, to) => {
            let first = from.saturating_sub(offset).max(1);
            let end = to.saturating_sub(offset).min(lines.len());
            for place in first..=end {
                if !emit.line(offset + place, "", "", lines[place - 1]) {
                    break;
                }
            }
        }
        Mode::Pattern { pattern, context } => {
            let mut groups: Vec<(usize, usize)> = Vec::new();
            for (index, line) in lines.iter().enumerate() {
                if !pattern.is_match(line) {
                    continue;
                }
                let start = index.saturating_sub(*context);
                let end = (index + context).min(lines.len() - 1);
                match groups.last_mut() {
                    Some(group) if start <= group.1 + 1 => group.1 = group.1.max(end),
                    _ => groups.push((start, end)),
                }
            }
            'groups: for (group, (start, end)) in groups.into_iter().enumerate() {
                for (index, line) in lines.iter().enumerate().skip(start).take(end - start + 1) {
                    let separator = if group > 0 && index == start {
                        "--\n"
                    } else {
                        ""
                    };
                    let mark = if pattern.is_match(line) { ':' } else { '-' };
                    let number = offset + index + 1;
                    let prefix = format!("{number}{mark}");
                    if !emit.line(number, separator, &prefix, line) {
                        break 'groups;
                    }
                }
            }
        }
    }
    emit.finish()
}

/// `text` masked by `scrubber`; the rule that could not mask it as the error (R-ID-7, R-SEC-3).
pub(crate) fn scrub(scrubber: &dyn Scrubber, text: String) -> Result<String, String> {
    let mut value = Value::String(text);
    scrubber
        .scrub(&mut value)
        .map_err(|unmasked| unmasked.rule.to_owned())?;
    match value {
        Value::String(text) => Ok(text),
        other => Ok(other.to_string()),
    }
}

/// What stands for a log or a slice the scrubber could not mask (D3).
pub(crate) fn withheld(rule: &str) -> String {
    format!("[log withheld: it matched credential rule {rule}]")
}

/// The merged output of one command while it runs (D2): the tail of at most [`COMMAND_LOG_CAP`]
/// bytes, plus the size of everything written to it and the lines dropped from its front. A
/// running command's reader asks [`complete`](Self::complete); the runner ends it with
/// [`into_stored`](Self::into_stored).
#[derive(Debug, Default, Clone)]
pub(crate) struct LogBuffer {
    /// The last bytes pushed: at most twice the cap, so a trim is paid once per cap of output.
    bytes: Vec<u8>,
    /// Every byte pushed.
    total: u64,
    /// The newlines dropped from the front of `bytes` (MOD-152 R1 F2).
    dropped: usize,
    /// Earlier output was dropped from the front of `bytes`.
    cut: bool,
    /// What `bytes` starts with is the middle of a line.
    partial: bool,
}

/// A shared, lockable [`LogBuffer`]: the sink writes it, the runner and a reader read it.
pub(crate) type SharedLog = Mutex<LogBuffer>;

/// The newlines in `bytes`.
fn newlines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| **byte == b'\n').count()
}

/// A running command's complete lines, copied out of its [`LogBuffer`] so that they are decoded
/// and scrubbed outside its lock (MOD-152 R1 F5).
#[derive(Debug)]
pub(crate) struct Complete {
    /// The whole lines of the tail.
    bytes: Vec<u8>,
    /// Earlier output was dropped.
    cut: bool,
    /// The lines of output before `bytes`.
    dropped: usize,
}

impl Complete {
    /// The lines as a log opens (with the truncation line when they are a tail), scrubbed as one
    /// text, fail closed (D3, R1 F1). A NUL is U+FFFD.
    pub(crate) fn scrubbed(self, scrubber: &dyn Scrubber) -> ScrubbedLog {
        let body = decode(&self.bytes);
        let text = if self.cut {
            format!("{}{body}", truncated_line(self.dropped))
        } else {
            body
        };
        ScrubbedLog::of(text, self.cut, scrubber)
    }
}

impl LogBuffer {
    /// Appends a chunk of the command's output.
    pub(crate) fn push(&mut self, chunk: &[u8]) {
        self.total += chunk.len() as u64;
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > 2 * COMMAND_LOG_CAP {
            let cut = self.bytes.len() - COMMAND_LOG_CAP;
            self.partial = self.bytes[cut - 1] != b'\n';
            self.cut = true;
            self.dropped += newlines(&self.bytes[..cut]);
            self.bytes.drain(..cut);
        }
    }

    /// Every byte pushed so far.
    pub(crate) const fn total(&self) -> u64 {
        self.total
    }

    /// Where in `bytes` the last [`COMMAND_LOG_CAP`] bytes start, from a line boundary when that
    /// cut anything — with nothing of a line the cut split (MOD-152 R1 F6: a line over the cap
    /// is dropped, never kept from its middle); whether earlier output is missing.
    fn tail_start(&self) -> (usize, bool) {
        let (mut start, mut cut, mut partial) = (0, self.cut, self.partial);
        if self.bytes.len() > COMMAND_LOG_CAP {
            start = self.bytes.len() - COMMAND_LOG_CAP;
            partial = self.bytes[start - 1] != b'\n';
            cut = true;
        }
        if cut && partial {
            start = self.bytes[start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(self.bytes.len(), |newline| start + newline + 1);
        }
        (start, cut)
    }

    /// The output so far, whole lines only, and the lines before them; not decoded nor scrubbed
    /// yet: [`Complete::scrubbed`] does that once the lock is let go.
    pub(crate) fn complete(&self) -> Complete {
        let (start, cut) = self.tail_start();
        let bytes = &self.bytes[start..];
        let end = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |last| last + 1);
        Complete {
            bytes: bytes[..end].to_vec(),
            cut,
            dropped: self.dropped + newlines(&self.bytes[..start]),
        }
    }

    /// The row's log (D2, D3): the tail within the cap, scrubbed as one text, fail closed — a text
    /// that cannot be masked is stored as the rule that stopped it. A cut log opens with the
    /// [`truncated_line`] of the lines it dropped (R1 F2).
    pub(crate) fn into_stored(
        self,
        scrubber: &dyn Scrubber,
        command_run_id: CommandRunId,
        now: DateTime<Utc>,
    ) -> CommandLog {
        let (start, mut truncated) = self.tail_start();
        let mut dropped = self.dropped + newlines(&self.bytes[..start]);
        // Decoding can grow a text (U+FFFD is three bytes), and so can masking: the cap is kept
        // on the text that is stored.
        let (body, cut, lines) = cap_tail(decode(&self.bytes[start..]), COMMAND_LOG_CAP);
        truncated |= cut;
        dropped += lines;
        let text = match scrub(scrubber, body) {
            Ok(body) => {
                let (body, cut, lines) = cap_tail(body, COMMAND_LOG_CAP);
                truncated |= cut;
                dropped += lines;
                if truncated {
                    format!("{}{body}", truncated_line(dropped))
                } else {
                    body
                }
            }
            Err(rule) => withheld(&rule),
        };
        CommandLog {
            command_run_id,
            text,
            truncated,
            total_bytes: self.total,
            written_at: now,
        }
    }
}

/// `bytes` as text, with any NUL replaced (`TEXT` holds none).
fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\0', "\u{fffd}")
}

/// `text` cut to its last `max` bytes, from a line boundary, with nothing of a line the cut split
/// (MOD-152 R1 F6); whether it was cut, and the lines it dropped.
fn cap_tail(text: String, max: usize) -> (String, bool, usize) {
    if text.len() <= max {
        return (text, false, 0);
    }
    let kept = cut_tail(&text, max);
    let mut start = text.len() - kept.len();
    if text.as_bytes()[start - 1] != b'\n' {
        start = kept
            .find('\n')
            .map_or(text.len(), |newline| start + newline + 1);
    }
    let lines = newlines(&text.as_bytes()[..start]);
    (text[start..].to_owned(), true, lines)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use htui_core::model::{COMMAND_LOG_CAP, COMMAND_LOG_STORE_MAX, CommandRunId};
    use htui_core::scrub::{MinimalScrubber, Scrubber, Unmasked};
    use serde_json::Value;

    use super::{LogBuffer, Mode, ScrubbedLog, TRUNCATED_LINES, slice, truncated_line};

    /// Masks `secret-9d8e7f`; what the real scrubber does with a known value.
    fn scrubber() -> MinimalScrubber {
        MinimalScrubber::new(["secret-9d8e7f".to_owned()])
    }

    fn stored(buffer: LogBuffer) -> htui_core::model::CommandLog {
        buffer.into_stored(&scrubber(), CommandRunId::new(), Utc::now())
    }

    /// A cut log's dropped line count and the lines under its truncation line.
    fn cut_body(text: &str) -> (usize, &str) {
        let rest = text
            .strip_prefix(TRUNCATED_LINES)
            .expect("the truncation line opens it");
        let (dropped, body) = rest.split_once(")\n").expect("a whole truncation line");
        (dropped.parse().expect("a count"), body)
    }

    /// What a reader of the running command sees.
    fn live(buffer: &LogBuffer) -> ScrubbedLog {
        buffer.complete().scrubbed(&scrubber())
    }

    /// `1\n2\n...` up to `last`.
    fn numbers(last: u32) -> Vec<u8> {
        (1..=last)
            .map(|n| format!("{n}\n"))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn a_short_log_is_kept_whole() {
        let mut buffer = LogBuffer::default();
        buffer.push(b"one\ntw");
        buffer.push(b"o\nthree");
        let log = stored(buffer);
        assert_eq!(log.text, "one\ntwo\nthree");
        assert!(!log.truncated);
        assert_eq!(log.total_bytes, 13);
    }

    #[test]
    fn a_long_log_keeps_the_tail_from_a_line_boundary_under_the_marker() {
        let mut buffer = LogBuffer::default();
        for chunk in numbers(600_000).chunks(65_536) {
            buffer.push(chunk);
        }
        let total = numbers(600_000).len() as u64;
        let log = stored(buffer);
        assert!(log.truncated);
        assert_eq!(log.total_bytes, total);
        assert!(log.text.len() <= COMMAND_LOG_CAP + truncated_line(600_000).len());
        assert!(log.text.len() <= COMMAND_LOG_STORE_MAX);
        let (dropped, body) = cut_body(&log.text);
        let first = body.lines().next().expect("a line");
        // MOD-152 R1 F2: the truncation line counts the lines before the first one kept.
        assert_eq!(
            first.parse::<usize>().ok(),
            Some(dropped + 1),
            "a whole line opens the tail"
        );
        assert!(body.ends_with("600000\n"));
        // The tail is contiguous: every number follows the one before.
        let mut previous: Option<u32> = None;
        for line in body.lines() {
            let n: u32 = line.parse().expect("a number");
            assert!(previous.is_none_or(|p| p + 1 == n), "{previous:?} then {n}");
            previous = Some(n);
        }
    }

    #[test]
    fn a_cut_exactly_on_a_line_boundary_drops_no_line() {
        let mut buffer = LogBuffer::default();
        let line = "x".repeat(1023) + "\n";
        for _ in 0..(COMMAND_LOG_CAP / 1024 + 5) {
            buffer.push(line.as_bytes());
        }
        let log = stored(buffer);
        let (dropped, body) = cut_body(&log.text);
        assert_eq!(body.len(), COMMAND_LOG_CAP, "the cap falls on a line");
        assert_eq!(dropped, 5);
        assert!(body.lines().all(|line| line.len() == 1023));
    }

    #[test]
    fn nuls_become_replacement_characters_and_bad_utf8_decodes_lossily() {
        let mut buffer = LogBuffer::default();
        buffer.push(b"a\0b\n\xff\n");
        let log = stored(buffer);
        assert_eq!(log.text, "a\u{fffd}b\n\u{fffd}\n");
        assert!(!log.text.contains('\0'));
    }

    #[test]
    fn a_text_grown_by_decoding_still_fits_the_store() {
        let mut buffer = LogBuffer::default();
        // 1 byte becomes 3 when decoded.
        buffer.push(&b"\0".repeat(COMMAND_LOG_CAP));
        let log = stored(buffer);
        assert!(
            log.text.len() <= COMMAND_LOG_STORE_MAX,
            "{}",
            log.text.len()
        );
        assert!(log.truncated);
    }

    #[test]
    fn a_known_secret_is_masked_across_the_whole_text() {
        let mut buffer = LogBuffer::default();
        buffer.push(b"token secret-9d8e7f end\n");
        assert_eq!(stored(buffer).text, "token [REDACTED] end\n");
    }

    #[test]
    fn a_text_that_cannot_be_masked_is_withheld_by_rule() {
        #[derive(Debug)]
        struct Refuses;
        impl Scrubber for Refuses {
            fn scrub(&self, _: &mut Value) -> Result<(), Unmasked> {
                Err(Unmasked {
                    path: String::new(),
                    rule: "test_rule",
                })
            }
        }
        let mut buffer = LogBuffer::default();
        buffer.push(b"AKIA-looking\n");
        let log = buffer.into_stored(&Refuses, CommandRunId::new(), Utc::now());
        assert_eq!(
            log.text,
            "[log withheld: it matched credential rule test_rule]"
        );
        assert_eq!(log.total_bytes, 13);
    }

    #[test]
    fn complete_lines_hold_back_the_line_still_being_written() {
        let mut buffer = LogBuffer::default();
        assert_eq!(live(&buffer).text, "");
        buffer.push(b"one\ntw");
        assert_eq!(live(&buffer).text, "one\n");
        buffer.push(b"o\n");
        let log = live(&buffer);
        assert_eq!((log.text.as_str(), log.truncated), ("one\ntwo\n", false));
    }

    /// MOD-152 R1 F2: a cut live log numbers its lines as the command printed them.
    #[test]
    fn complete_lines_of_a_cut_log_open_with_the_marker_and_keep_their_numbers() {
        let mut buffer = LogBuffer::default();
        for chunk in numbers(600_000).chunks(65_536) {
            buffer.push(chunk);
        }
        buffer.push(b"par");
        let log = live(&buffer);
        assert!(log.truncated);
        assert!(log.text.starts_with(TRUNCATED_LINES));
        assert!(
            log.text.ends_with("600000\n"),
            "the partial line is held back"
        );
        assert_eq!(log.total_lines(), 600_000);
        for (place, line) in log.text.lines().enumerate().skip(1) {
            assert_eq!(line.parse::<usize>().ok(), Some(log.offset + place + 1));
        }
        // The truncation line takes the number of the last line it stands for.
        let (dropped, _) = cut_body(&log.text);
        assert_eq!(log.offset + 1, dropped);
    }

    /// MOD-152 R1 F1: a live log is scrubbed as one text, fail closed.
    #[test]
    fn a_live_log_is_scrubbed_whole() {
        let mut buffer = LogBuffer::default();
        buffer.push(b"token secret-9d8e7f\n-----BEGIN RSA PRIVATE KEY-----\nMIIa\n");
        let log = buffer.complete().scrubbed(&MinimalScrubber::new([]));
        assert!(log.withheld);
        assert_eq!(
            log.text,
            "[log withheld: it matched credential rule private_key_pem]"
        );
        let mut buffer = LogBuffer::default();
        buffer.push(b"token secret-9d8e7f\n");
        assert_eq!(live(&buffer).text, "token [REDACTED]\n");
    }

    /// MOD-152 R1 F6: a line over the cap is dropped whole, not kept from its middle, so a secret
    /// the cut split leaves nothing behind.
    #[test]
    fn a_line_over_the_cap_is_dropped_not_split() {
        const SECRET: &str = "secret-9d8e7f";
        // The cap falls five bytes before the secret ends.
        let tail = "b".repeat(COMMAND_LOG_CAP - 5);
        let head = "a".repeat(3 * 1024 * 1024 - SECRET.len() - tail.len());
        let line = format!("{head}{SECRET}{tail}");
        for chunked in [false, true] {
            let mut buffer = LogBuffer::default();
            if chunked {
                // Past twice the cap: the buffer drops the front itself.
                buffer.push(head.as_bytes());
                buffer.push(&b"a".repeat(2 * COMMAND_LOG_CAP));
                buffer.push(SECRET.as_bytes());
                buffer.push(tail.as_bytes());
            } else {
                buffer.push(line.as_bytes());
            }
            assert_eq!(live(&buffer).text, truncated_line(1));
            let log = stored(buffer.clone());
            assert_eq!(log.text, truncated_line(1), "chunked: {chunked}");
            assert!(log.truncated);
            buffer.push(b"\nnext\n");
            assert_eq!(live(&buffer).text, format!("{}next\n", truncated_line(1)));
            assert_eq!(live(&buffer).total_lines(), 2);
        }
    }

    /// MOD-152 R1 F2: a stored cut log is numbered from its truncation line; an uncut one, or a
    /// text that only looks cut, from 1.
    #[test]
    fn a_stored_log_numbers_its_lines_from_its_truncation_line() {
        let text = format!("{}x\ny\n", truncated_line(41));
        let log = ScrubbedLog::of(text.clone(), true, &scrubber());
        assert_eq!((log.offset, log.total_lines()), (40, 43));
        let forged = ScrubbedLog::of(text, false, &scrubber());
        assert_eq!((forged.offset, forged.total_lines()), (0, 3));
    }

    /// MOD-152 R1 F2: a slice names and answers lines by their number in the output.
    #[test]
    fn a_slice_counts_lines_from_the_offset() {
        let text = "m\nx\ny\n";
        let range = slice(text, &Mode::Range(11, 12), 1024, 9);
        assert_eq!(
            (range.text.as_str(), range.first_line, range.last_line),
            ("x\ny\n", Some(11), Some(12))
        );
        let before = slice(text, &Mode::Range(1, 10), 1024, 9);
        assert_eq!((before.text.as_str(), before.first_line), ("m\n", Some(10)));
        assert_eq!(slice(text, &Mode::Range(1, 9), 1024, 9).first_line, None);
        let pattern = Mode::Pattern {
            pattern: super::compile("^y$").expect("a pattern"),
            context: 1,
        };
        assert_eq!(slice(text, &pattern, 1024, 9).text, "11-x\n12:y\n");
        assert_eq!(slice(text, &Mode::Tail(1), 1024, 9).first_line, Some(12));
    }

    #[test]
    fn a_tail_slice_answers_the_last_lines_and_counts_them() {
        let text = "a\nb\nc\nd\n";
        let tail = slice(text, &Mode::Tail(2), 1024, 0);
        assert_eq!(
            (
                tail.text.as_str(),
                tail.first_line,
                tail.last_line,
                tail.capped
            ),
            ("c\nd\n", Some(3), Some(4), false)
        );
        let none = slice("", &Mode::Tail(2), 1024, 0);
        assert_eq!(
            (none.first_line, none.last_line, none.capped),
            (None, None, false)
        );
    }

    #[test]
    fn a_line_longer_than_the_budget_is_cut_to_it() {
        let text = format!("{}\n", "é".repeat(100));
        let tail = slice(&text, &Mode::Tail(1), 11, 0);
        assert!(tail.capped);
        assert!(
            tail.text.len() <= 11 && tail.text.ends_with('\n'),
            "{:?}",
            tail.text
        );
        let range = slice(&text, &Mode::Range(1, 1), 11, 0);
        assert!(range.capped);
        assert!(
            range.text.len() <= 11 && range.text.ends_with('\n'),
            "{:?}",
            range.text
        );
    }
}
