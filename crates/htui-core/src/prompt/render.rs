//! One renderer per section of ANA-5 §4.2's closed vocabulary (`:435-590`).
//!
//! Every function here returns a section's **content**, without the wrapper. [`wrap`] is applied
//! once, after the trim, because the trimmer mutates content and a wrapper baked in at render time
//! would have to be re-parsed to get at it. Every content string is LF-normalised on the way in
//! (blueprint P-2), so every byte count, every elision marker and the hash are over LF text no
//! matter which line ending the row carried.
//!
//! Three rules of §4.2 are load-bearing and are worth stating where the code is:
//!
//! * **Rule 3, the fence.** A fence is one backtick longer than the longest backtick run anywhere
//!   in the content, minimum three — Aider's collision-avoidance rule
//!   (`aider/coders/base_coder.py:609-633`). Scanning the whole content and not just its first
//!   line is what stops an excerpt containing a fence from ending its own section early.
//! * **Rule 4, attributes.** `"` and `&` are escaped and nothing else; a newline in an attribute
//!   value collapses to a space. `<` and `>` are deliberately left alone: nothing parses a prompt
//!   back, so a body containing a literal `<section>` line is inert (`:460`).
//! * **Rule 5, paths.** No absolute filesystem path may reach a rendered byte. It holds by type
//!   here: [`BoxProfile`] has already dropped `box_tool.path`, and an
//!   [`Excerpt`] carries a repo slug and a repo-relative path with
//!   nowhere to put a root. That is what makes ANA-5 invariant 3 true for two fan-out siblings
//!   whose trees differ only by `<config_dir>/trees/<run_id>/<step_id>/`.
//!
//! The excerpt `<file>` blocks are emitted **unindented**, at column 0. That is not cosmetic: the
//! estimator toggles its code rate on a raw line starting with `<file ` (plan F-21), and an
//! indented block would be billed at the prose rate, silently under-counting every excerpt in the
//! budget.

use crate::model::box_::BoxProfile;
use crate::model::link::UpstreamEntry;
use crate::model::skill::BoundSkill;
use crate::model::{EventKind, SessionEvent};
use crate::prompt::defaults::COMMAND_QUEUE_TEXT;
use crate::prompt::excerpt::{Excerpt, RepoRoot};
use crate::prompt::template::ParsedTemplate;
use crate::prompt::{
    DiffBlock, InputDocument, JudgeCandidate, PromptSpec, SectionName, StepSummary, VerifyFailure,
};

/// §4.2 rule 4's cap on a rendered `title` attribute, in bytes of the title itself.
const TITLE_ATTR_BYTES: usize = 120;
/// §4.3's cap on a title inside an upstream line or heading, in bytes.
const ONE_LINE_TITLE_BYTES: usize = 100;
/// §4.2 rule 3's minimum fence.
const MIN_FENCE: usize = 3;
/// The minimum width of an excerpt's line-number gutter (§4.5's render shows `   1 | `).
const MIN_GUTTER: usize = 4;
/// How many lines of the last assistant message a handoff's `step_summary` keeps (§4.6c).
const ASSISTANT_TAIL_LINES: usize = 40;

/// §4.5's framing paragraph, byte-exact (`:1091-1093`).
///
/// Modelled on the only battle-tested wording in the field, Aider's "Do not propose changes to
/// these files, treat them as *read-only*" (`aider/coders/base_prompts.py:45-48`), and it exists
/// because Aider also documents the failure it prevents: weaker models "sometimes mistakenly try to
/// edit the code in the repo map" (<https://aider.chat/docs/faq.html>).
const EXCERPT_PREAMBLE: &str = "Read-only context, selected by htui. These files may be truncated and may be out of date; the\nworking tree is authoritative. Do not treat an excerpt as the whole file, and open the file before\nediting it.";

/// One rendered section before wrapping: what the trimmer mutates and what the estimator measures.
///
/// `attrs` is in source order and its values are **already** attribute-escaped, so [`wrap`] is a
/// concatenation and never a second escaping pass — escaping twice would turn `&amp;` into
/// `&amp;amp;` and change the digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// Which section this is; also the `name="…"` attribute.
    pub name: SectionName,
    /// The remaining attributes, in source order, values already escaped.
    pub attrs: Vec<(&'static str, String)>,
    /// The content: LF-normalised, no wrapper, no trailing LF.
    pub content: String,
}

/// Which of §4.3 step 5's three states an upstream entry renders in, plus the ladder's fourth.
///
/// The state is an input rather than a property of the entry because §4.4's depth ladder degrades
/// it under budget pressure: an entry whose `summary` is `Some` renders the `Pending` one-liner
/// when the ladder says [`UpstreamState::Pending`], and disappears at
/// [`UpstreamState::Dropped`]. The ladder itself is T65's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamState {
    /// In scope with a summary: a `### …` block.
    Summary,
    /// In scope with no summary: the `- … - no summary yet` line.
    Pending,
    /// Outside the scope: `R-PRM-2`'s bare one-liner.
    Stub,
    /// Removed by the trim ladder; renders nothing.
    Dropped,
}

impl UpstreamState {
    /// The state an entry is in before the trim ladder touches it (§4.3 step 5).
    #[must_use]
    pub const fn of(entry: &UpstreamEntry) -> Self {
        if entry.is_summary() {
            Self::Summary
        } else if entry.is_pending() {
            Self::Pending
        } else {
            Self::Stub
        }
    }
}

/// `\r\n` and lone `\r` become `\n`; a leading U+FEFF is dropped.
///
/// Applied per section at render time rather than once over the assembled whole, because every
/// later byte count, elision marker and floor is measured against the normalised text (P-2).
#[must_use]
pub fn normalise_newlines(s: &str) -> String {
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// A section's content: LF-normalised with its trailing LFs removed.
///
/// The wrapper supplies exactly one LF before `</section>` (§4.2 rule 1), so a body that ends in a
/// newline must not supply a second. Only *trailing* newlines are removed: rule 2 forbids per-line
/// whitespace stripping, because it would change meaning in whitespace-sensitive content and make
/// the digest a function of a cosmetic rule.
fn content_of(s: &str) -> String {
    let normalised = normalise_newlines(s);
    normalised.trim_end_matches('\n').to_owned()
}

/// §4.2 rule 4: escape `"` and `&`, collapse any newline to a space. One pass, so `&` that this
/// function introduced is never escaped again.
#[must_use]
pub fn attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\n' | '\r' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

/// The `title="…"` attribute: newlines to spaces, trimmed, cut to 120 bytes on a character
/// boundary with a trailing `...`, then escaped (§4.2 rule 4).
///
/// The cut happens **before** the escape so it can never land inside an `&quot;`, and the 120 is
/// the budget for the title's own bytes — the `...` is appended past it, as `:504-505` reads.
#[must_use]
pub fn title_attr(title: &str) -> String {
    let collapsed: String = title
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    attr(&truncate_bytes(collapsed.trim(), TITLE_ATTR_BYTES))
}

/// §4.3's title treatment for an upstream heading or one-liner: `\r` and `\n` to spaces, runs of
/// spaces to one, trimmed, cut to 100 bytes on a character boundary with a trailing `...`.
///
/// Trimmed because the result is concatenated into a line of prose rather than into an attribute:
/// a title ending in a newline would otherwise put trailing whitespace inside the section, which
/// §4.7's canonicalisation trims only at the end of the whole string.
#[must_use]
pub fn one_line_title(title: &str) -> String {
    let mut collapsed = String::with_capacity(title.len());
    let mut in_space = false;
    for c in title.chars() {
        if c == ' ' || c == '\n' || c == '\r' {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            collapsed.push(c);
            in_space = false;
        }
    }
    truncate_bytes(collapsed.trim(), ONE_LINE_TITLE_BYTES)
}

/// `s` if it fits, else its longest character-boundary prefix of at most `max` bytes plus `...`.
fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}...", &s[..cut])
}

/// §4.2 rule 3: one backtick more than the longest run **anywhere** in `content`, minimum three.
#[must_use]
pub fn fence_for(content: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in content.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat(longest.saturating_add(1).max(MIN_FENCE))
}

/// Wraps a rendered section in its `<section>` tags (§4.2 rule 1).
///
/// Exactly one LF between the opening tag and the first content byte and exactly one between the
/// last content byte and the closing tag — so empty content produces `<section …>\n</section>` with
/// no blank line, which is the only reading of rule 1 that does not invent a byte.
#[must_use]
pub fn wrap(section: &Rendered) -> String {
    let mut out = String::with_capacity(section.content.len() + 64);
    out.push_str("<section name=\"");
    out.push_str(&attr(&section.name.render()));
    out.push('"');
    for (key, value) in &section.attrs {
        out.push(' ');
        out.push_str(key);
        out.push_str("=\"");
        out.push_str(value);
        out.push('"');
    }
    out.push_str(">\n");
    if !section.content.is_empty() {
        out.push_str(&section.content);
        out.push('\n');
    }
    out.push_str("</section>");
    out
}

/// The `template` section's content: the frame's literal spans, LF-normalised and joined.
///
/// Not a [`Rendered`]: the template is the frame the other sections are substituted into, not a
/// wrapped section (§4.2's first table row). It still gets a `sections[]` entry — always the first
/// one, by P-9 — because its literal spans surround every other section and a reader needs to know
/// what they cost.
///
/// `assemble()` does **not** call this: it normalises and scrubs each span on its own and estimates
/// over the concatenation, so the frame's estimate and the frame's substituted bytes are the same
/// strings (review finding H-1). This remains the one-call answer to "what is the frame" for a
/// caller — MOD-9's editor — that has a [`ParsedTemplate`] and no scrubber.
#[must_use]
pub fn template_text(parsed: &ParsedTemplate) -> String {
    normalise_newlines(&parsed.literal_text())
}

/// `<section name="item" key="…" title="…">` then the item body verbatim.
#[must_use]
pub fn item(spec: &PromptSpec) -> Rendered {
    Rendered {
        name: SectionName::Item,
        attrs: vec![
            ("key", attr(&spec.item_key)),
            ("title", title_attr(&spec.item_title)),
        ],
        content: content_of(&spec.item_body),
    }
}

/// `<section name="documents:<kind>" kind="…" version="N">` then the document body verbatim.
#[must_use]
pub fn document(doc: &InputDocument) -> Rendered {
    Rendered {
        name: SectionName::Documents(doc.kind.clone()),
        attrs: vec![
            ("kind", attr(&doc.kind)),
            ("version", doc.version.to_string()),
        ],
        content: content_of(&doc.body),
    }
}

/// §4.3's upstream section: summary blocks first, then the gathered one-liners.
///
/// `states` is the trim ladder's verdict per entry, positionally; a short slice falls back to
/// [`UpstreamState::of`], so a caller that has not run the ladder gets the untrimmed render.
/// `None` when every entry is [`UpstreamState::Dropped`] or there are none — §4.3 step 7 says an
/// empty section renders nothing and emits no `sections[]` entry.
#[must_use]
pub fn upstream(entries: &[UpstreamEntry], states: &[UpstreamState]) -> Option<Rendered> {
    let mut blocks: Vec<String> = Vec::new();
    let mut liners: Vec<String> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let state = states
            .get(index)
            .copied()
            .unwrap_or_else(|| UpstreamState::of(entry));
        match state {
            UpstreamState::Summary => {
                let body = content_of(entry.summary.as_deref().unwrap_or_default());
                blocks.push(entry.summary_block(&body));
            }
            UpstreamState::Pending => liners.push(entry.one_liner(true)),
            UpstreamState::Stub => liners.push(entry.one_liner(false)),
            UpstreamState::Dropped => {}
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if !blocks.is_empty() {
        parts.push(blocks.join("\n\n"));
    }
    if !liners.is_empty() {
        parts.push(format!("also upstream:\n{}", liners.join("\n")));
    }
    if parts.is_empty() {
        return None;
    }
    Some(Rendered {
        name: SectionName::Upstream,
        attrs: Vec::new(),
        content: parts.join("\n\n"),
    })
}

impl UpstreamEntry {
    /// §4.3's summary block: the heading, a blank line, then the body verbatim (`:741-742`).
    ///
    /// `hop` pluralises to `hops` at anything but one. A body that is empty renders the heading
    /// alone rather than a heading followed by a blank line, because the blank line exists to
    /// separate the heading from the body.
    pub(crate) fn summary_block(&self, body: &str) -> String {
        let plural = if self.depth == 1 { "hop" } else { "hops" };
        let heading = format!(
            "### {} - {} ({}, {} {})",
            self.qualified_key,
            one_line_title(&self.title),
            self.status.as_str(),
            self.depth,
            plural,
        );
        if body.is_empty() {
            heading
        } else {
            format!("{heading}\n\n{body}")
        }
    }

    /// §4.3's two one-line forms, byte-exact (`:726-729`). The ` - no summary yet` suffix appears
    /// on the Pending form and on no other.
    pub(crate) fn one_liner(&self, pending: bool) -> String {
        let suffix = if pending { " - no summary yet" } else { "" };
        format!(
            "- {} - {} ({}){}",
            self.qualified_key,
            one_line_title(&self.title),
            self.status.as_str(),
            suffix,
        )
    }
}

/// §4.2's box projection, a fixed field list (`:516-536`).
///
/// `cpu`, `ram`, `gpu`, `tools` and `quirks` are each omitted when the row has nothing to say, so
/// the section never carries a line that means "unknown". `path` is not in the list, by type.
#[must_use]
pub fn box_profile(profile: &BoxProfile) -> Rendered {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("hostname: {}", profile.hostname));
    lines.push(format!(
        "os: {} {} ({})",
        profile.os_family.as_str(),
        profile.os_version,
        profile.arch,
    ));
    if !profile.cpu.is_empty() {
        lines.push(format!("cpu: {}", profile.cpu));
    }
    if let Some(ram_mb) = profile.ram_mb {
        lines.push(format!("ram: {ram_mb} MB"));
    }
    if let Some(vendor) = &profile.gpu_vendor {
        lines.push(format!("gpu: {vendor}"));
    }
    lines.push(format!("htui: {}", profile.htui_version));
    if !profile.tools.is_empty() {
        let mut tools: Vec<String> = profile
            .tools
            .iter()
            .map(|(name, version)| {
                if version.is_empty() {
                    name.clone()
                } else {
                    format!("{name} {version}")
                }
            })
            .collect();
        if profile.more_tools > 0 {
            tools.push(format!("+{} more", profile.more_tools));
        }
        lines.push(format!("tools: {}", tools.join(", ")));
    }
    // §4.2 `:535-536`: emitted verbatim with newlines collapsed to `; `. Empty lines are dropped
    // rather than rendered as `; ; `, which is what a blank line between two quirks would give.
    let normalised_quirks = normalise_newlines(&profile.quirks);
    let quirks: Vec<&str> = normalised_quirks
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if !quirks.is_empty() {
        lines.push(format!("quirks: {}", quirks.join("; ")));
    }
    Rendered {
        name: SectionName::Box,
        attrs: Vec::new(),
        content: lines.join("\n"),
    }
}

/// §4.2's skills section: one `<skill name="…" version="N">` block per binding (P-10).
///
/// The blocks follow the `<section>` rules one level down — a closing tag of its own, one LF each
/// side of the content — and are joined by a single LF, because a skill body is instruction text
/// and a blank line between two of them would read as a paragraph break inside one.
///
/// `None` when there are no bindings: §4.2's vocabulary has no empty `skills` section.
#[must_use]
pub fn skills(skills: &[BoundSkill]) -> Option<Rendered> {
    if skills.is_empty() {
        return None;
    }
    let blocks: Vec<String> = skills
        .iter()
        .map(|skill| {
            let body = content_of(&skill.body);
            let open = format!(
                "<skill name=\"{}\" version=\"{}\">",
                attr(&skill.name),
                skill.version,
            );
            if body.is_empty() {
                format!("{open}\n</skill>")
            } else {
                format!("{open}\n{body}\n</skill>")
            }
        })
        .collect();
    Some(Rendered {
        name: SectionName::Skills,
        attrs: Vec::new(),
        content: blocks.join("\n"),
    })
}

/// §4.5's excerpt section: the framing paragraph, a blank line, then one `<file>` block per
/// excerpt in `(repo, path)` byte order (§4.7 rule 5).
///
/// Byte order and not rank order: the rank decides which files survive, the path decides where they
/// sit, and mixing the two would make the rendered bytes a function of a float comparison.
///
/// `None` when there are no excerpts.
#[must_use]
pub fn excerpts(files: &[Excerpt]) -> Option<Rendered> {
    if files.is_empty() {
        return None;
    }
    let mut ordered: Vec<&Excerpt> = files.iter().collect();
    ordered.sort_by(|a, b| {
        a.repo
            .as_bytes()
            .cmp(b.repo.as_bytes())
            .then_with(|| a.path.as_bytes().cmp(b.path.as_bytes()))
    });
    let blocks: Vec<String> = ordered.into_iter().map(file_block).collect();
    Some(Rendered {
        name: SectionName::Excerpts,
        attrs: Vec::new(),
        content: format!("{EXCERPT_PREAMBLE}\n\n{}", blocks.join("\n")),
    })
}

/// One `<file>` block, unindented (plan F-21) and unfenced (§4.5 `:1115-1117`).
///
/// The gutter is as wide as the file's **last** line number, minimum four, so a file never
/// re-aligns when the trimmer takes lines off its tail.
///
/// `pub(crate)` rather than private (plan F-39, closed by T66): `FileRecord.sha256` is defined over
/// **these** bytes (§4.5 `:1136-1137`), so the audit has to hash the same function the prompt
/// renders — and [`excerpt::select`](crate::prompt::excerpt::select) has to price a candidate
/// against the residual budget before it takes it. Two renderings of one block is exactly the drift
/// the record exists to make impossible.
pub(crate) fn file_block(excerpt: &Excerpt) -> String {
    let truncated = if excerpt.truncated {
        " truncated=\"true\""
    } else {
        ""
    };
    let mut out = format!(
        "<file path=\"{}:{}\" lines=\"{}-{}\" reason=\"{}\"{}>\n",
        attr(&excerpt.repo),
        attr(&excerpt.path),
        excerpt.first_line,
        excerpt.last_line,
        attr(&excerpt.reason.render(excerpt.provider.as_deref())),
        truncated,
    );
    let gutter = excerpt.last_line.to_string().len().max(MIN_GUTTER);
    let body = normalise_newlines(&excerpt.content);
    for (offset, line) in body.trim_end_matches('\n').split('\n').enumerate() {
        let number = excerpt.first_line as u64 + offset as u64;
        if line.is_empty() {
            out.push_str(&format!("{number:>gutter$} |\n"));
        } else {
            out.push_str(&format!("{number:>gutter$} | {line}\n"));
        }
    }
    if excerpt.truncated {
        out.push_str(&elision_marker(excerpt.elided_lines, excerpt.elided_bytes));
        out.push('\n');
    }
    out.push_str("</file>");
    out
}

/// §4.2's `verify_failure`: the exit code as an attribute, the output as a fenced block.
///
/// Fenced because it is command output (rule 3), and command output that happened to contain a
/// fence would otherwise end the section early.
#[must_use]
pub fn verify_failure(v: &VerifyFailure) -> Rendered {
    let output = content_of(&v.output);
    let fence = fence_for(&output);
    Rendered {
        name: SectionName::VerifyFailure,
        attrs: vec![("exit_code", v.exit_code.to_string())],
        content: format!("{fence}\n{output}\n{fence}"),
    }
}

/// §4.2's `previous_diff` and `diff_so_far`: the range as an attribute, then the stat, then the
/// fenced unified diff.
///
/// `stat_only` is the trim ladder's middle rung (§4.4): the stat carries most of the signal at a
/// fraction of the cost, which is why a diff is the most compressible content in the prompt. The
/// stat and the fence are separated by a blank line so the stat reads as its own paragraph, and
/// the fence carries no info string — §4.2 asks for "a fenced unified diff" and an info string
/// would be a byte no requirement asked for.
#[must_use]
pub fn diff(name: SectionName, d: &DiffBlock, stat_only: bool) -> Rendered {
    Rendered {
        name,
        attrs: vec![("range", attr(&d.range))],
        content: diff_content(d, stat_only),
    }
}

/// The shared body of [`diff`] and of a judge candidate's `<diff>` block.
fn diff_content(d: &DiffBlock, stat_only: bool) -> String {
    let stat = content_of(&d.stat);
    let body = content_of(&d.diff);
    let mut parts: Vec<String> = Vec::new();
    if !stat.is_empty() {
        parts.push(stat);
    }
    if !stat_only && !body.is_empty() {
        let fence = fence_for(&body);
        parts.push(format!("{fence}\n{body}\n{fence}"));
    }
    parts.join("\n\n")
}

/// §4.2's `command_queue`: `R-MCP-4`'s two sentences, fixed once in
/// [`COMMAND_QUEUE_TEXT`] because every byte of a section is a
/// digest input (plan D111).
#[must_use]
pub fn command_queue() -> Rendered {
    Rendered {
        name: SectionName::CommandQueue,
        attrs: Vec::new(),
        content: content_of(COMMAND_QUEUE_TEXT),
    }
}

/// §4.6a's `judge_task`: the judged step's own stored prompt text, replayed verbatim.
#[must_use]
pub fn judge_task(text: &str) -> Rendered {
    Rendered {
        name: SectionName::JudgeTask,
        attrs: Vec::new(),
        content: content_of(text),
    }
}

/// §4.6a's `judge_candidate:<i>`: the one place §4.7 rule 8 permits a `fan_out_index` in a prompt,
/// because the verdict block names a winner by that index (`docs/ANA-2.md:828`).
///
/// The three inner parts carry their own tags — `<document>`, `<diff>`, `<verification>` — one
/// level down, the way `<skill>` and `<file>` do. §4.2 lists them ("document body, diff stat, diff,
/// verification tail") and fixes no delimiter; a flat concatenation would leave the judge unable to
/// tell where a document ended and a verification tail began, which is exactly the boundary
/// problem `<section>` was adopted to solve.
#[must_use]
pub fn judge_candidate(c: &JudgeCandidate, stat_only: bool) -> Rendered {
    let mut attrs = vec![("fanout_index", c.fanout_index.to_string())];
    if let Some(verify) = c.verify {
        attrs.push(("verify", if verify { "pass" } else { "fail" }.to_owned()));
    }
    if let Some(exit_code) = c.exit_code {
        attrs.push(("exit_code", exit_code.to_string()));
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(doc) = &c.document {
        let body = content_of(&doc.body);
        let open = format!(
            "<document kind=\"{}\" version=\"{}\">",
            attr(&doc.kind),
            doc.version,
        );
        parts.push(if body.is_empty() {
            format!("{open}\n</document>")
        } else {
            format!("{open}\n{body}\n</document>")
        });
    }
    if let Some(d) = &c.diff {
        let body = diff_content(d, stat_only);
        let open = format!("<diff range=\"{}\">", attr(&d.range));
        parts.push(if body.is_empty() {
            format!("{open}\n</diff>")
        } else {
            format!("{open}\n{body}\n</diff>")
        });
    }
    if let Some(tail) = &c.verification_tail {
        let body = content_of(tail);
        let fence = fence_for(&body);
        parts.push(format!(
            "<verification>\n{fence}\n{body}\n{fence}\n</verification>"
        ));
    }

    Rendered {
        name: SectionName::JudgeCandidate(c.fanout_index),
        attrs,
        content: parts.join("\n"),
    }
}

/// §4.6c's `step_summary` (`:1306-1313`): four lines and a tail, every one of them derived by code.
///
/// `R-ID-6` keeps a model out of a bookkeeping path, and re-running a model to summarise a step in
/// order to restart that step is both circular and non-reproducible. Each of the first three lines
/// is omitted when it has nothing to report, so the section never carries a line that means "none".
#[must_use]
pub fn step_summary(s: &StepSummary) -> Rendered {
    let mut lines: Vec<String> = Vec::new();
    if !s.tool_calls.is_empty() {
        let calls: Vec<String> = s
            .tool_calls
            .iter()
            .map(|(kind, count)| format!("{kind}({count})"))
            .collect();
        lines.push(format!("Tool calls, in order: {}.", calls.join(", ")));
    }
    if !s.files_edited.is_empty() {
        lines.push(format!("Files edited: {}.", s.files_edited.join(", ")));
    }
    if !s.errors.is_empty() {
        // No trailing period: an error line ends with the row's own message, which carries its own
        // punctuation, and appending one would produce `..` on the common case.
        lines.push(format!(
            "Errors: {} - {}",
            s.errors.len(),
            s.errors.join("; "),
        ));
    }
    let tail = content_of(&s.last_assistant_tail);
    if !tail.is_empty() {
        lines.push(format!("Last assistant message (turn {}, tail):", s.turns));
        lines.push(tail);
    }
    Rendered {
        name: SectionName::StepSummary,
        attrs: vec![
            ("turns", s.turns.to_string()),
            ("events", s.events.to_string()),
        ],
        content: lines.join("\n"),
    }
}

/// §4.2's `failure_reason`: one paragraph, and the reason a handoff prompt exists. Protected.
#[must_use]
pub fn failure_reason(text: &str) -> Rendered {
    Rendered {
        name: SectionName::FailureReason,
        attrs: Vec::new(),
        content: content_of(text),
    }
}

/// §4.4's elision marker (`:902-906`), one space-free ASCII form on its own line.
///
/// Plain decimal digits with no thousands separator (blueprint P-4), so the marker's two numbers
/// equal `trim_record`'s `elided_lines` and `elided_bytes` as strings and criterion 9's last clause
/// is a string compare rather than a parse.
#[must_use]
pub fn elision_marker(lines: u32, bytes: u64) -> String {
    format!("[... htui elided {lines} lines / {bytes} bytes ...]")
}

impl StepSummary {
    /// §4.6c's summariser: counts from `EventKind` frequencies, file lists from `edit_proposal`
    /// payload paths, errors from `error` rows, and the tail from the last `assistant_text` row
    /// (`:1316-1318`).
    ///
    /// Takes the step's own events and no others, which is what keeps `R-PRM-1`'s "never raw
    /// transcripts of other items" true: the caller has the step id it is restarting and nothing
    /// else. Ordering is `events`' own, which is `seq` order by `docs/ANA-9.md:247`; nothing here
    /// sorts, so a caller that hands them out of order gets its own order back rather than a
    /// silently different summary.
    ///
    /// `roots` is what makes rule 5 true for the one section built from a transcript. An ACP `diff`
    /// block and an `fs/write_text_file` carry **absolute** paths by protocol
    /// (`crates/htui-agent/src/acp/fs.rs:57` admits them) and nothing between the wire and here
    /// rewrote one, so a handoff prompt's `Files edited:` line shipped a tree root — an absolute
    /// path, a run id and a step id in a digested byte, against §4.2 rule 5 and §4.7 rule 8 (review
    /// finding H-2). Every path is now put through `repo_relative` against these roots, which is
    /// the same `RepoRoot` list §4.5's excerpt pass resolved, and one that strips to nothing is
    /// dropped rather than rendered.
    ///
    /// `errors[]` and `last_assistant_tail` are the same exposure class and are **not** rewritten:
    /// §4.6c sanctions them as windowed transcript text, and a path inside an error message is part
    /// of the message rather than a field.
    #[must_use]
    pub fn from_events(events: &[SessionEvent], roots: &[RepoRoot]) -> Self {
        let mut tool_calls: Vec<(String, u32)> = Vec::new();
        let mut files_edited: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut last_assistant = String::new();
        let mut max_turn = 0i32;

        // Longest root first, so a repo checked out inside another resolves to the deeper one; the
        // slug breaks a tie, so two roots of equal length resolve in one order on every box.
        let mut roots: Vec<(&str, String)> = roots
            .iter()
            .map(|root| {
                (
                    root.repo.as_str(),
                    root.root.to_string_lossy().replace('\\', "/"),
                )
            })
            .collect();
        roots.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));

        for event in events {
            max_turn = max_turn.max(event.turn);
            match event.kind {
                EventKind::ToolCall => {
                    let kind = event
                        .payload
                        .get("tool_kind")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("other");
                    if let Some(entry) = tool_calls.iter_mut().find(|(name, _)| name == kind) {
                        entry.1 += 1;
                    } else {
                        tool_calls.push((kind.to_owned(), 1));
                    }
                }
                EventKind::EditProposal => {
                    if let Some(path) = event
                        .payload
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        && let Some(rendered) = repo_relative(path, &roots)
                        && !files_edited.iter().any(|seen| seen == &rendered)
                    {
                        // Deduped on the **rendered** form: two roots cannot make one file look
                        // like two, and two files cannot collapse into one.
                        files_edited.push(rendered);
                    }
                }
                EventKind::Error => {
                    let code = event
                        .payload
                        .get("code")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let message = event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    errors.push(format!("`{code}` — {message} at turn {}", event.turn));
                }
                EventKind::AssistantText => {
                    if let Some(text) = event
                        .payload
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                    {
                        last_assistant = text.to_owned();
                    }
                }
                // Enumerated rather than `_ => {}`: a new `EventKind` must be considered for the
                // summary deliberately, not be silently absent from it (review finding L-3).
                EventKind::Prompt
                | EventKind::FollowUp
                | EventKind::Thought
                | EventKind::ToolResult
                | EventKind::PermissionRequest
                | EventKind::PermissionAnswer
                | EventKind::Plan
                | EventKind::Usage
                | EventKind::Done
                | EventKind::Other => {}
            }
        }

        let turns = if events.is_empty() {
            0
        } else {
            u32::try_from(max_turn.saturating_add(1)).unwrap_or(0)
        };
        Self {
            turns,
            events: u32::try_from(events.len()).unwrap_or(u32::MAX),
            tool_calls,
            files_edited,
            errors,
            last_assistant_tail: head_tail_lines(
                &normalise_newlines(&last_assistant),
                ASSISTANT_TAIL_LINES,
            ),
        }
    }
}

/// One `edit_proposal` payload path as `<repo_slug>:<repo-relative>` — or nothing at all.
///
/// §4.2 rule 5 admits no absolute filesystem path into a digested byte, and §4.7 rule 8 admits no
/// run id or step id, so a path under `<config_dir>/trees/<run_id>/<step_id>/` violates both at
/// once. §4.6c sanctions `edit_proposal` as a *source* for the handoff summary and writes its
/// example repo-relative; this is the function that makes the payload match the example.
///
/// `roots` is `(slug, root)` with `/` separators, **longest root first**. The rules, in order:
///
/// * a path under a known root renders `<slug>:<rest>`, and one that strips to nothing — the root
///   itself — is dropped, because "the repository was edited" names no file;
/// * a path that escapes with a `..` segment is dropped: it is not repo-relative whatever it looks
///   like;
/// * a path under no root that is still absolute is dropped, because there is no slug to qualify it
///   with and no rewrite that would make it safe;
/// * anything else is already a repo-relative path — §4.6c's own example form — and is kept, with
///   `\` normalised to `/` so a Windows step and a Linux step summarise identically.
fn repo_relative(path: &str, roots: &[(&str, String)]) -> Option<String> {
    let candidate = path.replace('\\', "/");
    let escapes = |p: &str| p.split('/').any(|segment| segment == "..");
    for (repo, root) in roots {
        let root = root.trim_end_matches('/');
        if root.is_empty() {
            continue;
        }
        let Some(rest) = candidate.strip_prefix(root) else {
            continue;
        };
        // `/a/b` must not match `/a/bc/d`: the remainder is a whole path segment or nothing.
        if !rest.is_empty() && !rest.starts_with('/') {
            continue;
        }
        let relative = rest.trim_start_matches('/');
        if relative.is_empty() || escapes(relative) {
            return None;
        }
        return Some(format!("{repo}:{relative}"));
    }
    let bytes = candidate.as_bytes();
    let absolute = bytes.first() == Some(&b'/')
        // A drive letter, after the separator normalisation above turned `C:\x` into `C:/x`.
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    if absolute || escapes(&candidate) || candidate.is_empty() {
        return None;
    }
    Some(candidate)
}

/// A fixed head+tail window over lines, with §4.4's marker between the halves.
///
/// Fixed rather than budget-driven: this window sizes the last assistant message inside a summary
/// the trimmer has not reached yet. The trimmer's own reclaim-driven `head_tail` is T65's, and the
/// two are deliberately separate — one answers "how much does this section have to give up", the
/// other "how much of a transcript tail is worth quoting at all".
fn head_tail_lines(text: &str, keep: usize) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    if lines.len() <= keep {
        return text.to_owned();
    }
    let head = keep / 2;
    let tail = keep - head;
    let elided = &lines[head..lines.len() - tail];
    let elided_lines = u32::try_from(elided.len()).unwrap_or(u32::MAX);
    let elided_bytes =
        u64::try_from(elided.iter().map(|line| line.len()).sum::<usize>()).unwrap_or(u64::MAX);

    let mut out = String::with_capacity(text.len());
    for line in &lines[..head] {
        out.push_str(line);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&elision_marker(elided_lines, elided_bytes));
    out.push('\n');
    for line in &lines[lines.len() - tail..] {
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ids::{ItemId, StepId};
    use crate::model::item::Status;
    use crate::model::{EventRole, OsFamily};
    use crate::prompt::excerpt::{ExcerptReason, RootSource};
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use uuid::Uuid;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    fn entry(key: &str, depth: u8, summary: Option<&str>, in_scope: bool) -> UpstreamEntry {
        UpstreamEntry {
            item_id: ItemId::from_uuid(Uuid::from_u128(u128::from(depth))),
            qualified_key: key.to_owned(),
            title: format!("{key} title"),
            status: Status::Closed,
            depth,
            in_scope,
            summary: summary.map(str::to_owned),
        }
    }

    fn event(kind: EventKind, turn: i32, payload: serde_json::Value) -> SessionEvent {
        SessionEvent {
            run_step_id: StepId::from_uuid(Uuid::from_u128(1)),
            seq: 0,
            turn,
            kind,
            role: EventRole::Agent,
            tool_call_id: None,
            payload,
            raw: None,
            at: at(),
        }
    }

    #[test]
    fn fence_is_one_longer_than_the_longest_run_min_three() {
        assert_eq!(fence_for("plain text"), "```");
        assert_eq!(
            fence_for("a ` b"),
            "```",
            "a run of one still floors at three"
        );
        assert_eq!(fence_for("```"), "````");
        assert_eq!(fence_for("`````"), "``````");
    }

    #[test]
    fn nested_fences_do_not_close_early() {
        // The longest run is in the *middle* of the content, not on its first line — which is what
        // a first-line scan would miss (hazard H-4).
        let content = "line one\n````\nnested\n````\nline five";
        let fence = fence_for(content);
        assert_eq!(fence, "`````");
        let wrapped = format!("{fence}\n{content}\n{fence}");
        assert!(
            !content.contains(fence.as_str()),
            "the fence must not occur inside the content it delimits"
        );
        assert_eq!(wrapped.matches(fence.as_str()).count(), 2);
    }

    #[test]
    fn attr_escapes_quote_and_amp_and_collapses_newlines() {
        assert_eq!(attr(r#"a & b"#), "a &amp; b");
        assert_eq!(attr(r#"say "hi""#), "say &quot;hi&quot;");
        assert_eq!(attr("one\ntwo\r\nthree"), "one two  three");
        assert_eq!(
            attr("<section>"),
            "<section>",
            "angle brackets are inert: nothing parses a prompt back (§4.2 `:460`)"
        );
        assert_eq!(
            attr("&amp;"),
            "&amp;amp;",
            "one pass: the escape is not re-escaped, and an input that already looked escaped is \
             not silently accepted"
        );
    }

    #[test]
    fn title_truncates_at_120_bytes_on_a_char_boundary() {
        let short = title_attr("  A short title\n ");
        assert_eq!(short, "A short title");
        // 60 two-byte characters is 120 bytes exactly: it fits.
        let exact = "é".repeat(60);
        assert_eq!(title_attr(&exact), exact);
        // 61 does not, and the cut lands on a character boundary rather than inside `é`.
        let over = "é".repeat(61);
        let cut = title_attr(&over);
        assert_eq!(cut, format!("{}...", "é".repeat(60)));
        assert!(cut.is_char_boundary(cut.len() - 3));
        // The cut happens before the escape, so it can never land inside an entity: the 120-byte
        // budget counts the *title's* bytes, and the `&quot;` it expands to arrives whole.
        let quoted = format!("{}\"tail", "a".repeat(119));
        assert_eq!(title_attr(&quoted), format!("{}&quot;...", "a".repeat(119)));
    }

    #[test]
    fn one_line_title_collapses_runs_and_cuts_at_100() {
        assert_eq!(one_line_title("a   b\n\nc\r\nd"), "a b c d");
        assert_eq!(one_line_title("  padded  "), "padded");
        let long = "x".repeat(101);
        assert_eq!(one_line_title(&long), format!("{}...", "x".repeat(100)));
        assert_eq!(one_line_title(&"x".repeat(100)), "x".repeat(100));
    }

    #[test]
    fn wrap_of_empty_content_has_no_blank_line() {
        let empty = Rendered {
            name: SectionName::Item,
            attrs: Vec::new(),
            content: String::new(),
        };
        assert_eq!(wrap(&empty), "<section name=\"item\">\n</section>");
        let full = Rendered {
            name: SectionName::Documents("plan".to_owned()),
            attrs: vec![("kind", "plan".to_owned()), ("version", "3".to_owned())],
            content: "body".to_owned(),
        };
        assert_eq!(
            wrap(&full),
            "<section name=\"documents:plan\" kind=\"plan\" version=\"3\">\nbody\n</section>"
        );
    }

    #[test]
    fn a_section_tag_in_content_is_inert() {
        // §4.2 `:460`: htui never parses the prompt back, so a body carrying a literal closing tag
        // is text. The renderer must not escape it either — escaping would change the bytes the
        // model sees and make the digest a function of a defensive rule nothing needs.
        let doc = InputDocument {
            kind: "plan".to_owned(),
            version: 1,
            body: "before\n</section>\nafter".to_owned(),
        };
        let wrapped = wrap(&document(&doc));
        assert!(wrapped.contains("before\n</section>\nafter"));
        assert!(wrapped.ends_with("after\n</section>"));
    }

    #[test]
    fn line_endings_are_normalised_per_section_before_anything_measures() {
        let doc = InputDocument {
            kind: "plan".to_owned(),
            version: 1,
            body: "\u{feff}a\r\nb\rc\r\n".to_owned(),
        };
        assert_eq!(document(&doc).content, "a\nb\nc");
    }

    #[test]
    fn upstream_summary_blocks_precede_one_liners() {
        let entries = vec![
            entry("htui:ANA-9", 1, Some("the schema"), true),
            entry("htui:MOD-7", 1, None, true),
            entry("auth:MOD-1", 2, None, false),
            entry("ws:MOD-11", 2, Some("the rule"), true),
        ];
        let rendered = upstream(&entries, &[]).expect("four entries render");
        assert_eq!(
            rendered.content,
            concat!(
                "### htui:ANA-9 - htui:ANA-9 title (closed, 1 hop)\n",
                "\n",
                "the schema\n",
                "\n",
                "### ws:MOD-11 - ws:MOD-11 title (closed, 2 hops)\n",
                "\n",
                "the rule\n",
                "\n",
                "also upstream:\n",
                "- htui:MOD-7 - htui:MOD-7 title (closed) - no summary yet\n",
                "- auth:MOD-1 - auth:MOD-1 title (closed)",
            )
        );
        assert!(rendered.attrs.is_empty());
        // §4.3 step 7: an empty section renders nothing at all.
        assert!(upstream(&[], &[]).is_none());
        assert!(
            upstream(&entries, &[UpstreamState::Dropped; 4]).is_none(),
            "a ladder that dropped every row leaves no section behind"
        );
    }

    #[test]
    fn the_pending_suffix_is_only_on_pending_rows() {
        let in_scope = entry("htui:MOD-7", 1, None, true);
        assert!(in_scope.one_liner(true).ends_with(" - no summary yet"));
        let out_of_scope = entry("auth:MOD-1", 1, None, false);
        assert_eq!(
            out_of_scope.one_liner(false),
            "- auth:MOD-1 - auth:MOD-1 title (closed)"
        );
        // The ladder's degrade: an entry that *has* a summary still renders the Pending line when
        // the ladder says so.
        let degraded = entry("htui:ANA-9", 2, Some("the schema"), true);
        let rendered = upstream(std::slice::from_ref(&degraded), &[UpstreamState::Pending])
            .expect("one entry");
        assert_eq!(
            rendered.content,
            "also upstream:\n- htui:ANA-9 - htui:ANA-9 title (closed) - no summary yet",
            "no leading blank line when there is no summary block above"
        );
    }

    #[test]
    fn hop_pluralises_at_two() {
        let one = entry("a:b", 1, Some("x"), true);
        assert!(
            one.summary_block("x")
                .starts_with("### a:b - a:b title (closed, 1 hop)")
        );
        let two = entry("a:b", 2, Some("x"), true);
        assert!(
            two.summary_block("x")
                .starts_with("### a:b - a:b title (closed, 2 hops)")
        );
        assert_eq!(
            two.summary_block(""),
            "### a:b - a:b title (closed, 2 hops)",
            "an empty body renders the heading alone"
        );
    }

    #[test]
    fn the_box_profile_omits_what_the_row_does_not_have_and_never_a_path() {
        let full = BoxProfile {
            hostname: "dev-win-01".to_owned(),
            os_family: OsFamily::Windows,
            os_version: "10.0.26200".to_owned(),
            arch: "x86_64".to_owned(),
            cpu: "AMD Ryzen 9 7950X, 32 threads".to_owned(),
            ram_mb: Some(65_536),
            gpu_vendor: Some("nvidia".to_owned()),
            htui_version: "0.4.1".to_owned(),
            tools: vec![
                ("cargo".to_owned(), "1.98.1".to_owned()),
                ("git".to_owned(), String::new()),
            ],
            more_tools: 3,
            quirks: "MSVC toolchain only\nno WSL\n".to_owned(),
        };
        assert_eq!(
            box_profile(&full).content,
            concat!(
                "hostname: dev-win-01\n",
                "os: windows 10.0.26200 (x86_64)\n",
                "cpu: AMD Ryzen 9 7950X, 32 threads\n",
                "ram: 65536 MB\n",
                "gpu: nvidia\n",
                "htui: 0.4.1\n",
                "tools: cargo 1.98.1, git, +3 more\n",
                "quirks: MSVC toolchain only; no WSL",
            )
        );
        let bare = BoxProfile {
            cpu: String::new(),
            ram_mb: None,
            gpu_vendor: None,
            tools: Vec::new(),
            more_tools: 0,
            quirks: String::new(),
            ..full
        };
        assert_eq!(
            box_profile(&bare).content,
            "hostname: dev-win-01\nos: windows 10.0.26200 (x86_64)\nhtui: 0.4.1"
        );
        assert!(
            !box_profile(&bare).content.contains('/'),
            "§4.2 rule 5: no path reaches the section, and BoxProfile has none to give"
        );
    }

    #[test]
    fn skills_blocks_carry_their_own_closing_tag() {
        let skills_list = vec![
            BoundSkill {
                skill_id: crate::model::ids::SkillId::from_uuid(Uuid::from_u128(1)),
                name: "rust-style".to_owned(),
                version: 3,
                position: 0,
                body: "Use `expect` over `unwrap`.\n".to_owned(),
            },
            BoundSkill {
                skill_id: crate::model::ids::SkillId::from_uuid(Uuid::from_u128(2)),
                name: "tests".to_owned(),
                version: 1,
                position: 1,
                body: String::new(),
            },
        ];
        assert_eq!(
            skills(&skills_list).expect("two bindings").content,
            concat!(
                "<skill name=\"rust-style\" version=\"3\">\n",
                "Use `expect` over `unwrap`.\n",
                "</skill>\n",
                "<skill name=\"tests\" version=\"1\">\n",
                "</skill>",
            )
        );
        assert!(skills(&[]).is_none());
    }

    fn excerpt(repo: &str, path: &str, last: u32, truncated: bool) -> Excerpt {
        Excerpt {
            repo: repo.to_owned(),
            path: path.to_owned(),
            first_line: 1,
            last_line: last,
            truncated,
            elided_lines: if truncated { 412 } else { 0 },
            elided_bytes: if truncated { 18_903 } else { 0 },
            rank: 1,
            weight: 100,
            reason: ExcerptReason::TouchedPath,
            provider: None,
            content: "fn main() {}\n\nlet x = 1;\n".to_owned(),
        }
    }

    #[test]
    fn excerpt_line_numbers_are_right_aligned_to_the_last_line() {
        let wide = excerpt("htui", "a.rs", 1_200, false);
        let rendered = excerpts(std::slice::from_ref(&wide)).expect("one file");
        assert!(rendered.content.contains("\n   1 | fn main() {}\n"));
        assert!(
            rendered.content.contains("\n   2 |\n"),
            "an empty line carries no trailing space"
        );
        assert!(rendered.content.contains("\n   3 | let x = 1;\n"));
        // A four-digit last line still uses a four-wide gutter; a five-digit one widens.
        let wider = excerpt("htui", "a.rs", 12_000, false);
        let rendered = excerpts(std::slice::from_ref(&wider)).expect("one file");
        assert!(rendered.content.contains("\n    1 | fn main() {}\n"));
    }

    #[test]
    fn excerpt_blocks_are_unindented_and_path_ordered() {
        // F-21: the estimator's code toggle matches `<file ` at column 0 on the raw line.
        let files = vec![
            excerpt("htui", "z.rs", 3, true),
            excerpt("agy", "a.rs", 3, false),
            excerpt("htui", "a.rs", 3, false),
        ];
        let rendered = excerpts(&files).expect("three files");
        for line in rendered.content.lines() {
            if line.contains("<file ") {
                assert!(line.starts_with("<file "), "`{line}` must be at column 0");
            }
        }
        let order: Vec<&str> = rendered
            .content
            .lines()
            .filter(|line| line.starts_with("<file "))
            .collect();
        assert_eq!(order.len(), 3);
        assert!(order[0].contains("path=\"agy:a.rs\""));
        assert!(order[1].contains("path=\"htui:a.rs\""));
        assert!(
            order[2].contains("path=\"htui:z.rs\""),
            "byte order, not rank"
        );
        assert!(rendered.content.starts_with(EXCERPT_PREAMBLE));
        assert!(rendered.content.contains(
            "<file path=\"htui:z.rs\" lines=\"1-3\" reason=\"touched_path\" truncated=\"true\">"
        ));
        assert!(
            rendered
                .content
                .contains("[... htui elided 412 lines / 18903 bytes ...]"),
            "P-4: plain decimal digits, so the marker and the record compare as strings"
        );
        assert!(excerpts(&[]).is_none());
    }

    #[test]
    fn a_fenced_section_survives_content_that_contains_a_fence() {
        let v = VerifyFailure {
            exit_code: 101,
            output: "error: see\n```\nnested\n```\n".to_owned(),
        };
        let rendered = verify_failure(&v);
        assert_eq!(rendered.attrs, vec![("exit_code", "101".to_owned())]);
        assert_eq!(rendered.content, "````\nerror: see\n```\nnested\n```\n````");
    }

    #[test]
    fn a_diff_degrades_to_its_stat_and_keeps_its_range() {
        let d = DiffBlock {
            range: "abc1234..def5678".to_owned(),
            stat: " 2 files changed, 4 insertions(+)".to_owned(),
            diff: "--- a\n+++ b\n".to_owned(),
        };
        let full = diff(SectionName::PreviousDiff, &d, false);
        assert_eq!(full.attrs, vec![("range", "abc1234..def5678".to_owned())]);
        assert_eq!(
            full.content,
            " 2 files changed, 4 insertions(+)\n\n```\n--- a\n+++ b\n```"
        );
        let stat_only = diff(SectionName::DiffSoFar, &d, true);
        assert_eq!(stat_only.content, " 2 files changed, 4 insertions(+)");
        assert_eq!(stat_only.name, SectionName::DiffSoFar);
    }

    #[test]
    fn a_judge_candidate_names_its_index_and_bounds_its_three_parts() {
        let candidate = JudgeCandidate {
            fanout_index: 2,
            verify: Some(true),
            exit_code: Some(0),
            document: Some(InputDocument {
                kind: "impl".to_owned(),
                version: 1,
                body: "what changed".to_owned(),
            }),
            diff: Some(DiffBlock {
                range: "aaa..bbb".to_owned(),
                stat: " 1 file changed".to_owned(),
                diff: "--- a".to_owned(),
            }),
            verification_tail: Some("ok\n".to_owned()),
        };
        let rendered = judge_candidate(&candidate, false);
        assert_eq!(rendered.name, SectionName::JudgeCandidate(2));
        assert_eq!(
            rendered.attrs,
            vec![
                ("fanout_index", "2".to_owned()),
                ("verify", "pass".to_owned()),
                ("exit_code", "0".to_owned()),
            ]
        );
        assert_eq!(
            rendered.content,
            concat!(
                "<document kind=\"impl\" version=\"1\">\n",
                "what changed\n",
                "</document>\n",
                "<diff range=\"aaa..bbb\">\n",
                " 1 file changed\n",
                "\n",
                "```\n",
                "--- a\n",
                "```\n",
                "</diff>\n",
                "<verification>\n",
                "```\n",
                "ok\n",
                "```\n",
                "</verification>",
            )
        );
        // The two optional attributes disappear rather than rendering a placeholder.
        let bare = JudgeCandidate {
            verify: None,
            exit_code: None,
            document: None,
            diff: None,
            verification_tail: None,
            ..candidate
        };
        let rendered = judge_candidate(&bare, false);
        assert_eq!(rendered.attrs, vec![("fanout_index", "2".to_owned())]);
        assert_eq!(rendered.content, "");
        assert_eq!(
            wrap(&rendered),
            "<section name=\"judge_candidate:2\" fanout_index=\"2\">\n</section>"
        );
    }

    #[test]
    fn step_summary_from_events_counts_and_tails() {
        let events = vec![
            event(EventKind::Prompt, 0, json!({ "text": "go" })),
            event(EventKind::ToolCall, 0, json!({ "tool_kind": "read" })),
            event(EventKind::ToolCall, 1, json!({ "tool_kind": "edit" })),
            event(EventKind::ToolCall, 1, json!({ "tool_kind": "read" })),
            event(EventKind::ToolCall, 1, json!({})),
            event(
                EventKind::EditProposal,
                1,
                json!({ "path": "crates/htui-core/src/prompt/mod.rs" }),
            ),
            event(
                EventKind::EditProposal,
                2,
                json!({ "path": "crates/htui-core/src/prompt/mod.rs" }),
            ),
            event(
                EventKind::EditProposal,
                2,
                json!({ "path": "crates/htui-core/src/prompt/trim.rs" }),
            ),
            event(
                EventKind::Error,
                2,
                json!({ "code": "command_failed", "message": "`cargo test` exited 101" }),
            ),
            event(EventKind::AssistantText, 1, json!({ "text": "first" })),
            event(EventKind::AssistantText, 2, json!({ "text": "last\n" })),
        ];
        let summary = StepSummary::from_events(&events, &[]);
        assert_eq!(summary.turns, 3, "turns 0, 1 and 2 are three turns");
        assert_eq!(summary.events, 11);
        assert_eq!(
            summary.tool_calls,
            vec![
                ("read".to_owned(), 2),
                ("edit".to_owned(), 1),
                ("other".to_owned(), 1),
            ],
            "first-seen order, counted; a payload with no `tool_kind` is `other`"
        );
        assert_eq!(
            summary.files_edited,
            vec![
                "crates/htui-core/src/prompt/mod.rs".to_owned(),
                "crates/htui-core/src/prompt/trim.rs".to_owned(),
            ],
            "deduped, first-seen order"
        );
        assert_eq!(
            summary.errors,
            vec!["`command_failed` — `cargo test` exited 101 at turn 2".to_owned()]
        );
        assert_eq!(summary.last_assistant_tail, "last\n");

        let rendered = step_summary(&summary);
        assert_eq!(
            rendered.attrs,
            vec![("turns", "3".to_owned()), ("events", "11".to_owned())]
        );
        assert_eq!(
            rendered.content,
            concat!(
                "Tool calls, in order: read(2), edit(1), other(1).\n",
                "Files edited: crates/htui-core/src/prompt/mod.rs, \
                 crates/htui-core/src/prompt/trim.rs.\n",
                "Errors: 1 - `command_failed` — `cargo test` exited 101 at turn 2\n",
                "Last assistant message (turn 3, tail):\n",
                "last",
            )
        );

        // Nothing to say is nothing rendered, not a line that says "none".
        let empty = StepSummary::from_events(&[], &[]);
        assert_eq!(empty.turns, 0);
        assert_eq!(step_summary(&empty).content, "");
    }

    #[test]
    fn an_edit_proposal_path_is_repo_qualified_and_never_absolute() {
        // **H-2.** ACP `diff` blocks and `fs/write_text_file` carry absolute paths by protocol, so
        // `Files edited:` shipped `<config_dir>/trees/<run_id>/<step_id>/…` — an absolute path, a
        // run id and a step id in a digested byte, against §4.2 rule 5 and §4.7 rule 8 at once.
        let roots = vec![
            RepoRoot {
                repo: "htui".to_owned(),
                root: std::path::PathBuf::from(
                    "/home/htui/.local/share/htui/trees/01a0-aa/01a0-bb",
                ),
                source: RootSource::RunStepTree,
            },
            RepoRoot {
                repo: "vendored".to_owned(),
                root: std::path::PathBuf::from(
                    "/home/htui/.local/share/htui/trees/01a0-aa/01a0-bb/vendor/agy",
                ),
                source: RootSource::RepoBoxPath,
            },
        ];
        let edit = |path: &str| {
            event(
                EventKind::EditProposal,
                0,
                json!({ "path": path.to_owned() }),
            )
        };
        let events = vec![
            edit("/home/htui/.local/share/htui/trees/01a0-aa/01a0-bb/crates/htui-core/src/lib.rs"),
            // The deeper root wins, so a repo checked out inside another is named as itself.
            edit("/home/htui/.local/share/htui/trees/01a0-aa/01a0-bb/vendor/agy/src/main.rs"),
            // The same file twice, spelled with `\`: one entry, not two.
            edit(
                "\\home\\htui\\.local\\share\\htui\\trees\\01a0-aa\\01a0-bb\\crates\\htui-core\\src\\lib.rs",
            ),
            // Already repo-relative: §4.6c's own example form, kept.
            edit("crates/htui/src/main.rs"),
            // Under no root and absolute: there is no slug to qualify it with, so it goes.
            edit("/etc/shadow"),
            edit("C:\\Users\\dev\\secret.rs"),
            // The root itself strips to nothing, and `..` is not repo-relative whatever it looks
            // like.
            edit("/home/htui/.local/share/htui/trees/01a0-aa/01a0-bb"),
            edit("../../etc/shadow"),
        ];
        let summary = StepSummary::from_events(&events, &roots);
        assert_eq!(
            summary.files_edited,
            vec![
                "htui:crates/htui-core/src/lib.rs".to_owned(),
                "vendored:src/main.rs".to_owned(),
                "crates/htui/src/main.rs".to_owned(),
            ]
        );
        let rendered = step_summary(&summary);
        for leak in ["/home/", "01a0-aa", "01a0-bb", "/etc/", "C:"] {
            assert!(
                !rendered.content.contains(leak),
                "`{leak}` reached a digested byte:\n{}",
                rendered.content
            );
        }
        // No roots at all — a caller that resolved none — still refuses an absolute path rather
        // than falling back to rendering it.
        let bare = StepSummary::from_events(&events, &[]);
        assert_eq!(
            bare.files_edited,
            vec!["crates/htui/src/main.rs".to_owned()]
        );
    }

    #[test]
    fn a_long_assistant_tail_is_head_tail_windowed_with_the_marker() {
        let long: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let events = vec![event(
            EventKind::AssistantText,
            0,
            json!({ "text": long.clone() }),
        )];
        let summary = StepSummary::from_events(&events, &[]);
        let lines: Vec<&str> = summary.last_assistant_tail.lines().collect();
        assert_eq!(lines.len(), 41, "20 head + the marker + 20 tail");
        assert_eq!(lines[0], "line 1");
        assert_eq!(lines[19], "line 20");
        assert!(lines[20].starts_with("[... htui elided 60 lines / "));
        assert!(lines[20].ends_with(" bytes ...]"));
        assert_eq!(lines[21], "line 81");
        assert_eq!(lines[40], "line 100");
        // The marker's byte count is the elided bytes, newlines included.
        let elided_bytes: usize = (21..=80).map(|n| format!("line {n}\n").len()).sum();
        assert!(
            summary
                .last_assistant_tail
                .contains(&elision_marker(60, elided_bytes as u64))
        );
    }

    #[test]
    fn the_command_queue_section_is_the_two_fixed_sentences() {
        let rendered = command_queue();
        assert_eq!(rendered.name, SectionName::CommandQueue);
        assert_eq!(rendered.content, COMMAND_QUEUE_TEXT);
        assert!(rendered.attrs.is_empty());
    }

    #[test]
    fn the_template_section_is_the_literal_spans_only() {
        let parsed = crate::prompt::parse(
            crate::prompt::TemplateRole::Phase,
            "head\r\n{{item}}\r\ntail",
        )
        .expect("a legal body");
        assert_eq!(
            template_text(&parsed),
            "head\n\ntail",
            "the slot contributes nothing, and the frame is LF before anything measures it"
        );
    }
}
