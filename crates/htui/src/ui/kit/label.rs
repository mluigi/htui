//! Display labels (MOD-115 M3 D3, MOD-83): a human word, plus a glyph where it is a status, for
//! every enum a ported tab shows, so no internal snake_case value reaches the screen; and one
//! pluralising helper (`1 option`, `3 options`, never `(s)`). [`kit::status`](super::status)
//! keeps the item statuses.
//!
//! Every function here is an exhaustive `match` with no wildcard arm, so a variant added to one
//! of these enums stops the build instead of printing its raw name.

use htui_agent::event::{PlanEntryStatus, StopReason, ToolKind};
use htui_agent::probe::ProbeStatus;
use htui_core::model::{
    Billing, CitationKind, CommandQueue, Gate, Isolation, OsFamily, Priority, RequirementState,
    Resolution, RunStatus, StepStatus, Transport,
};
use htui_core::prompt::BudgetSource;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::ui::Theme;

/// A status as shown: one glyph, one human word, and how it reads (its style through
/// [`tone_style`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    /// The one-cell glyph.
    pub glyph: &'static str,
    /// The plain word.
    pub word: &'static str,
    /// How the mark reads.
    pub tone: Tone,
}

/// How a [`Mark`] reads, mapped onto a [`Theme`] role by [`tone_style`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Finished well: dim, it needs nothing more.
    Done,
    /// Under way.
    Running,
    /// Waiting on a person.
    NeedsYou,
    /// Went wrong.
    Failed,
    /// Not started, or at rest: ordinary text.
    Idle,
    /// Out of play (cancelled, superseded, withdrawn).
    Dim,
}

/// The [`Theme`] role a [`Tone`] renders in: `Done`/`Dim` dim, `Running` running, `NeedsYou`
/// warning, `Failed` error, `Idle` base.
#[must_use]
pub fn tone_style(tone: Tone, theme: &Theme) -> Style {
    match tone {
        Tone::Done | Tone::Dim => theme.dim,
        Tone::Running => theme.running,
        Tone::NeedsYou => theme.warning,
        Tone::Failed => theme.error,
        Tone::Idle => theme.base,
    }
}

/// A mark as styled spans, all three in its [`tone_style`]: the glyph, a space, the word.
#[must_use]
pub fn spans(mark: Mark, theme: &Theme) -> Vec<Span<'static>> {
    let style = tone_style(mark.tone, theme);
    vec![
        Span::styled(mark.glyph, style),
        Span::styled(" ", style),
        Span::styled(mark.word, style),
    ]
}

const fn mark(glyph: &'static str, word: &'static str, tone: Tone) -> Mark {
    Mark { glyph, word, tone }
}

const PENDING: Mark = mark("○", "pending", Tone::Idle);
const RUNNING: Mark = mark("◐", "running", Tone::Running);
const NEEDS_YOU: Mark = mark("⚠", "needs you", Tone::NeedsYou);
const STOPPED: Mark = mark("⊘", "stopped", Tone::Idle);
const DONE: Mark = mark("●", "done", Tone::Done);
const FAILED: Mark = mark("✗", "failed", Tone::Failed);
const CANCELLED: Mark = mark("⊘", "cancelled", Tone::Dim);

/// How a chat turn ended (`done.stop_reason`).
#[must_use]
pub const fn stop_reason(reason: StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "turn ended",
        StopReason::MaxTokens => "token limit reached",
        StopReason::MaxTurnRequests => "turn limit reached",
        StopReason::Refusal => "refused",
        StopReason::Cancelled => "cancelled",
    }
}

/// What a tool call does, as the transcript's call row names it.
#[must_use]
pub const fn tool_kind(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "run",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch mode",
        ToolKind::Other => "tool",
    }
}

/// A chat plan entry's state.
#[must_use]
pub const fn plan_entry(status: PlanEntryStatus) -> Mark {
    match status {
        PlanEntryStatus::Pending => PENDING,
        PlanEntryStatus::InProgress => mark("◐", "in progress", Tone::Running),
        PlanEntryStatus::Completed => DONE,
    }
}

/// A run's status, in the item statuses' words ([`kit::status`](super::status)).
#[must_use]
pub const fn run_status(status: RunStatus) -> Mark {
    match status {
        RunStatus::Queued => mark("◌", "queued", Tone::Idle),
        RunStatus::Running => RUNNING,
        RunStatus::AwaitingApproval => NEEDS_YOU,
        RunStatus::Stopped => STOPPED,
        RunStatus::Done => DONE,
        RunStatus::Failed => FAILED,
        RunStatus::Cancelled => CANCELLED,
    }
}

/// A run step's status: a run's marks, plus `pending` and `superseded`.
#[must_use]
pub const fn step_status(status: StepStatus) -> Mark {
    match status {
        StepStatus::Pending => PENDING,
        StepStatus::Running => RUNNING,
        StepStatus::AwaitingApproval => NEEDS_YOU,
        StepStatus::Stopped => STOPPED,
        StepStatus::Done => DONE,
        StepStatus::Failed => FAILED,
        StepStatus::Cancelled => CANCELLED,
        StepStatus::Superseded => mark("↷", "superseded", Tone::Dim),
    }
}

/// How `htui` talks to an agent.
#[must_use]
pub const fn transport(transport: Transport) -> &'static str {
    match transport {
        Transport::Acp => "ACP",
        Transport::Cli => "CLI",
    }
}

/// How an agent is billed.
#[must_use]
pub const fn billing(billing: Billing) -> &'static str {
    match billing {
        Billing::Subscription => "subscription",
        Billing::PerToken => "per token",
    }
}

/// An agent's probe verdict on a box. Every mark fits the Agents table's 13-cell `on this box`
/// column whole; `⚠ needs login` fills it. The column already says it is the probe, so a failed
/// probe reads `✗ failed`, the word that cell shows today.
#[must_use]
pub const fn probe(status: ProbeStatus) -> Mark {
    match status {
        ProbeStatus::Ready => mark("●", "ready", Tone::Done),
        ProbeStatus::Unauthenticated => mark("⚠", "needs login", Tone::NeedsYou),
        ProbeStatus::Missing => mark("✗", "missing", Tone::Failed),
        ProbeStatus::Failed => FAILED,
    }
}

/// Which rung of the chain a budget setting came from.
#[must_use]
pub const fn setting_source(source: BudgetSource) -> &'static str {
    match source {
        BudgetSource::Phase => "phase",
        BudgetSource::Project => "project",
        BudgetSource::AppSetting => "app setting",
        BudgetSource::AppSettingDefault => "default",
    }
}

/// A requirement's priority.
#[must_use]
pub const fn priority(priority: Priority) -> &'static str {
    match priority {
        Priority::Must => "must",
        Priority::Later => "later",
    }
}

/// A requirement's state. `withdrawn` carries the Requirements tree's `✕`.
#[must_use]
pub const fn requirement_state(state: RequirementState) -> Mark {
    match state {
        RequirementState::Active => mark("●", "active", Tone::Idle),
        RequirementState::Withdrawn => mark("✕", "withdrawn", Tone::Dim),
    }
}

/// How an item cites a requirement.
#[must_use]
pub const fn citation(kind: CitationKind) -> &'static str {
    match kind {
        CitationKind::Addresses => "addresses",
        CitationKind::Amends => "amends",
        CitationKind::Withdraws => "withdraws",
        CitationKind::Reserves => "reserves",
    }
}

/// Why a closed item closed.
#[must_use]
pub const fn resolution(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::Done => "done",
        Resolution::Concluded => "concluded",
        Resolution::Rejected => "rejected",
        Resolution::Withdrawn => "withdrawn",
        Resolution::Superseded => "superseded",
        Resolution::Duplicate => "duplicate",
    }
}

/// When a phase stops for a person.
#[must_use]
pub const fn gate(gate: Gate) -> &'static str {
    match gate {
        Gate::Always => "always",
        Gate::OnFailure => "on failure",
        Gate::Never => "never",
    }
}

/// How a phase's step gets its own view of the repository.
#[must_use]
pub const fn isolation(isolation: Isolation) -> &'static str {
    match isolation {
        Isolation::Worktree => "worktree",
        Isolation::Copy => "copy",
        Isolation::SharedSerialized => "shared, one at a time",
        Isolation::Local => "none (local tree)",
    }
}

/// When a phase's commands are queued.
#[must_use]
pub const fn command_queue(queue: CommandQueue) -> &'static str {
    match queue {
        CommandQueue::Off => "not queued",
        CommandQueue::FanOutOnly => "queued when fanned out",
        CommandQueue::Always => "always queued",
    }
}

/// A box's operating system, spelled as its vendor does.
#[must_use]
pub const fn os_family(os: OsFamily) -> &'static str {
    match os {
        OsFamily::Windows => "Windows",
        OsFamily::Linux => "Linux",
        OsFamily::Macos => "macOS",
    }
}

/// A count with its noun: `1 option`, `0 options`, `3 options`. Never `option(s)`.
#[must_use]
pub fn plural(count: usize, one: &str, many: &str) -> String {
    let noun = if count == 1 { one } else { many };
    format!("{count} {noun}")
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use super::*;
    use crate::ui::cells::cell_width;

    /// `BudgetSource` has no generated `ALL`, so this list is by hand. It is checked against
    /// [`budget_source_after`], whose `match` stops compiling when a variant is added.
    const BUDGET_SOURCES: [BudgetSource; 4] = [
        BudgetSource::Phase,
        BudgetSource::Project,
        BudgetSource::AppSetting,
        BudgetSource::AppSettingDefault,
    ];

    /// The rung after `source`, in chain order. Walking it from `Phase` visits every variant, so
    /// the walk is what [`BUDGET_SOURCES`] must equal. A new variant needs an arm here, and the
    /// rung before it must name it.
    const fn budget_source_after(source: BudgetSource) -> Option<BudgetSource> {
        match source {
            BudgetSource::Phase => Some(BudgetSource::Project),
            BudgetSource::Project => Some(BudgetSource::AppSetting),
            BudgetSource::AppSetting => Some(BudgetSource::AppSettingDefault),
            BudgetSource::AppSettingDefault => None,
        }
    }

    /// Every word a word-only label table shows, as `(raw, label)`, in the enum's order.
    fn words<T: Copy + Debug>(
        all: &[T],
        raw: fn(T) -> &'static str,
        label: fn(T) -> &'static str,
    ) -> Vec<(&'static str, &'static str)> {
        all.iter()
            .map(|value| (raw(*value), label(*value)))
            .collect()
    }

    /// Every mark a status table shows, as `(raw, mark)`, in the enum's order.
    fn marks<T: Copy + Debug>(
        all: &[T],
        raw: fn(T) -> &'static str,
        mark: fn(T) -> Mark,
    ) -> Vec<(&'static str, Mark)> {
        all.iter()
            .map(|value| (raw(*value), mark(*value)))
            .collect()
    }

    fn word_tables() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
        vec![
            (
                "StopReason",
                words(StopReason::ALL, StopReason::as_str, stop_reason),
            ),
            (
                "ToolKind",
                words(ToolKind::ALL, ToolKind::as_str, tool_kind),
            ),
            (
                "Transport",
                words(Transport::ALL, Transport::as_str, transport),
            ),
            ("Billing", words(Billing::ALL, Billing::as_str, billing)),
            (
                "BudgetSource",
                words(&BUDGET_SOURCES, BudgetSource::as_str, setting_source),
            ),
            ("Priority", words(Priority::ALL, Priority::as_str, priority)),
            (
                "CitationKind",
                words(CitationKind::ALL, CitationKind::as_str, citation),
            ),
            (
                "Resolution",
                words(Resolution::ALL, Resolution::as_str, resolution),
            ),
            ("Gate", words(Gate::ALL, Gate::as_str, gate)),
            (
                "Isolation",
                words(Isolation::ALL, Isolation::as_str, isolation),
            ),
            (
                "CommandQueue",
                words(CommandQueue::ALL, CommandQueue::as_str, command_queue),
            ),
            (
                "OsFamily",
                words(OsFamily::ALL, OsFamily::as_str, os_family),
            ),
        ]
    }

    fn mark_tables() -> Vec<(&'static str, Vec<(&'static str, Mark)>)> {
        vec![
            (
                "PlanEntryStatus",
                marks(PlanEntryStatus::ALL, PlanEntryStatus::as_str, plan_entry),
            ),
            (
                "RunStatus",
                marks(RunStatus::ALL, RunStatus::as_str, run_status),
            ),
            (
                "StepStatus",
                marks(StepStatus::ALL, StepStatus::as_str, step_status),
            ),
            (
                "ProbeStatus",
                marks(ProbeStatus::ALL, ProbeStatus::as_str, probe),
            ),
            (
                "RequirementState",
                marks(
                    RequirementState::ALL,
                    RequirementState::as_str,
                    requirement_state,
                ),
            ),
        ]
    }

    /// A mark table as `(glyph, word, tone)` rows, for pinning.
    fn shown(table: &[(&'static str, Mark)]) -> Vec<(&'static str, &'static str, Tone)> {
        table
            .iter()
            .map(|(_, mark)| (mark.glyph, mark.word, mark.tone))
            .collect()
    }

    /// A word table's labels alone, for pinning.
    fn labels(table: &[(&'static str, &'static str)]) -> Vec<&'static str> {
        table.iter().map(|(_, label)| *label).collect()
    }

    #[test]
    fn the_budget_source_hand_list_is_complete() {
        let mut chain = vec![BudgetSource::Phase];
        while let Some(next) = chain.last().copied().and_then(budget_source_after) {
            assert!(!chain.contains(&next), "the chain loops at {next:?}");
            chain.push(next);
        }
        assert_eq!(BUDGET_SOURCES.as_slice(), chain.as_slice());
    }

    /// D-4's "no label contains `_`" and "every multi-word value is relabelled" are one check: a
    /// multi-word value is snake_case, so a label without `_` cannot be that value verbatim.
    #[test]
    fn no_label_carries_an_underscore_so_no_multi_word_value_shows_raw() {
        let mut every: Vec<(&str, &str, &str)> = Vec::new();
        for (name, table) in word_tables() {
            every.extend(table.into_iter().map(|(raw, label)| (name, raw, label)));
        }
        for (name, table) in mark_tables() {
            every.extend(table.into_iter().map(|(raw, mark)| (name, raw, mark.word)));
        }
        for (name, raw, label) in every {
            assert!(!label.is_empty(), "{name}::{raw} has no label");
            assert!(!label.contains('_'), "{name}::{raw} shows {label:?}");
        }
    }

    #[test]
    fn every_glyph_is_one_cell_and_no_two_values_of_one_enum_read_alike() {
        for (name, table) in mark_tables() {
            for (raw, mark) in &table {
                assert_eq!(cell_width(mark.glyph), 1, "{name}::{raw}: {}", mark.glyph);
            }
            // Without colour the word alone tells two values apart.
            for (i, (raw, a)) in table.iter().enumerate() {
                for (other, b) in &table[i + 1..] {
                    assert_ne!(a.word, b.word, "{name}: {raw} and {other}");
                }
            }
        }
        for (name, table) in word_tables() {
            for (i, (raw, a)) in table.iter().enumerate() {
                for (other, b) in &table[i + 1..] {
                    assert_ne!(a, b, "{name}: {raw} and {other}");
                }
            }
        }
    }

    #[test]
    fn a_stop_reason_reads_as_how_the_turn_ended() {
        let table = words(StopReason::ALL, StopReason::as_str, stop_reason);
        assert_eq!(
            table,
            [
                ("end_turn", "turn ended"),
                ("max_tokens", "token limit reached"),
                ("max_turn_requests", "turn limit reached"),
                ("refusal", "refused"),
                ("cancelled", "cancelled"),
            ]
        );
    }

    #[test]
    fn a_tool_kind_reads_as_what_the_call_does() {
        let table = words(ToolKind::ALL, ToolKind::as_str, tool_kind);
        assert_eq!(
            labels(&table),
            [
                "read",
                "edit",
                "delete",
                "move",
                "search",
                "run",
                "think",
                "fetch",
                "switch mode",
                "tool",
            ]
        );
        assert_eq!(tool_kind(ToolKind::Execute), "run");
        assert_eq!(tool_kind(ToolKind::Other), "tool");
    }

    #[test]
    fn a_plan_entry_shows_a_glyph_and_its_state() {
        let table = marks(PlanEntryStatus::ALL, PlanEntryStatus::as_str, plan_entry);
        assert_eq!(
            shown(&table),
            [
                ("○", "pending", Tone::Idle),
                ("◐", "in progress", Tone::Running),
                ("●", "done", Tone::Done),
            ]
        );
    }

    #[test]
    fn a_run_status_matches_the_item_status_words() {
        let table = marks(RunStatus::ALL, RunStatus::as_str, run_status);
        assert_eq!(
            shown(&table),
            [
                ("◌", "queued", Tone::Idle),
                ("◐", "running", Tone::Running),
                ("⚠", "needs you", Tone::NeedsYou),
                ("⊘", "stopped", Tone::Idle),
                ("●", "done", Tone::Done),
                ("✗", "failed", Tone::Failed),
                ("⊘", "cancelled", Tone::Dim),
            ]
        );
    }

    #[test]
    fn a_step_status_reads_as_a_run_status_plus_pending_and_superseded() {
        let table = marks(StepStatus::ALL, StepStatus::as_str, step_status);
        assert_eq!(
            shown(&table),
            [
                ("○", "pending", Tone::Idle),
                ("◐", "running", Tone::Running),
                ("⚠", "needs you", Tone::NeedsYou),
                ("⊘", "stopped", Tone::Idle),
                ("●", "done", Tone::Done),
                ("✗", "failed", Tone::Failed),
                ("⊘", "cancelled", Tone::Dim),
                ("↷", "superseded", Tone::Dim),
            ]
        );
        // The statuses both enums have read the same in both.
        for (step, run) in [
            (StepStatus::Running, RunStatus::Running),
            (StepStatus::AwaitingApproval, RunStatus::AwaitingApproval),
            (StepStatus::Stopped, RunStatus::Stopped),
            (StepStatus::Done, RunStatus::Done),
            (StepStatus::Failed, RunStatus::Failed),
            (StepStatus::Cancelled, RunStatus::Cancelled),
        ] {
            assert_eq!(step_status(step), run_status(run), "{step:?}");
        }
    }

    #[test]
    fn a_transport_is_an_acronym_and_billing_reads_as_words() {
        assert_eq!(
            labels(&words(Transport::ALL, Transport::as_str, transport)),
            ["ACP", "CLI"]
        );
        assert_eq!(
            labels(&words(Billing::ALL, Billing::as_str, billing)),
            ["subscription", "per token"]
        );
    }

    #[test]
    fn a_probe_verdict_says_what_to_do_and_fits_the_agents_column() {
        let table = marks(ProbeStatus::ALL, ProbeStatus::as_str, probe);
        assert_eq!(
            shown(&table),
            [
                ("●", "ready", Tone::Done),
                ("⚠", "needs login", Tone::NeedsYou),
                ("✗", "missing", Tone::Failed),
                ("✗", "failed", Tone::Failed),
            ]
        );
        // The Agents table's `on this box` cell is 13 wide: every verdict fits it whole, and the
        // login prompt fills it.
        for (raw, mark) in &table {
            let width = cell_width(&format!("{} {}", mark.glyph, mark.word));
            assert!(width <= 13, "ProbeStatus::{raw} is {width} cells");
        }
        let login = probe(ProbeStatus::Unauthenticated);
        assert_eq!(cell_width(&format!("{} {}", login.glyph, login.word)), 13);
    }

    #[test]
    fn a_setting_source_names_the_rung_in_words() {
        assert_eq!(
            labels(&words(
                &BUDGET_SOURCES,
                BudgetSource::as_str,
                setting_source
            )),
            ["phase", "project", "app setting", "default"]
        );
    }

    #[test]
    fn the_requirement_labels_read_as_the_detail_shows_them() {
        assert_eq!(
            labels(&words(Priority::ALL, Priority::as_str, priority)),
            ["must", "later"]
        );
        assert_eq!(
            shown(&marks(
                RequirementState::ALL,
                RequirementState::as_str,
                requirement_state
            )),
            [("●", "active", Tone::Idle), ("✕", "withdrawn", Tone::Dim)]
        );
        assert_eq!(
            labels(&words(CitationKind::ALL, CitationKind::as_str, citation)),
            ["addresses", "amends", "withdraws", "reserves"]
        );
        assert_eq!(
            labels(&words(Resolution::ALL, Resolution::as_str, resolution)),
            [
                "done",
                "concluded",
                "rejected",
                "withdrawn",
                "superseded",
                "duplicate"
            ]
        );
    }

    #[test]
    fn the_phase_settings_read_as_plain_words() {
        assert_eq!(
            labels(&words(Gate::ALL, Gate::as_str, gate)),
            ["always", "on failure", "never"]
        );
        assert_eq!(
            labels(&words(Isolation::ALL, Isolation::as_str, isolation)),
            [
                "worktree",
                "copy",
                "shared, one at a time",
                "none (local tree)"
            ]
        );
        assert_eq!(
            labels(&words(
                CommandQueue::ALL,
                CommandQueue::as_str,
                command_queue
            )),
            ["not queued", "queued when fanned out", "always queued"]
        );
    }

    #[test]
    fn an_os_family_is_spelled_as_its_vendor_does() {
        assert_eq!(
            labels(&words(OsFamily::ALL, OsFamily::as_str, os_family)),
            ["Windows", "Linux", "macOS"]
        );
    }

    #[test]
    fn each_tone_takes_its_theme_role() {
        let theme = Theme::default();
        assert_eq!(tone_style(Tone::Done, &theme), theme.dim);
        assert_eq!(tone_style(Tone::Dim, &theme), theme.dim);
        assert_eq!(tone_style(Tone::Running, &theme), theme.running);
        assert_eq!(tone_style(Tone::NeedsYou, &theme), theme.warning);
        assert_eq!(tone_style(Tone::Failed, &theme), theme.error);
        assert_eq!(tone_style(Tone::Idle, &theme), theme.base);
        assert_ne!(
            tone_style(Tone::Failed, &theme),
            tone_style(Tone::NeedsYou, &theme)
        );
    }

    #[test]
    fn the_spans_are_glyph_space_word_in_the_tone_style() {
        let theme = Theme::default();
        for (name, table) in mark_tables() {
            for (raw, mark) in table {
                let spans = spans(mark, &theme);
                let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
                assert_eq!(
                    text,
                    format!("{} {}", mark.glyph, mark.word),
                    "{name}::{raw}"
                );
                for span in &spans {
                    assert_eq!(span.style, tone_style(mark.tone, &theme), "{name}::{raw}");
                }
            }
        }
    }

    #[test]
    fn under_monochrome_no_tone_names_a_colour() {
        let theme = Theme::monochrome();
        for tone in [
            Tone::Done,
            Tone::Running,
            Tone::NeedsYou,
            Tone::Failed,
            Tone::Idle,
            Tone::Dim,
        ] {
            let style = tone_style(tone, &theme);
            assert_eq!((style.fg, style.bg), (None, None), "{tone:?}");
        }
    }

    #[test]
    fn one_takes_the_singular_and_every_other_count_the_plural() {
        assert_eq!(plural(0, "option", "options"), "0 options");
        assert_eq!(plural(1, "option", "options"), "1 option");
        assert_eq!(plural(2, "option", "options"), "2 options");
        assert_eq!(plural(1, "entry", "entries"), "1 entry");
        assert_eq!(plural(12, "entry", "entries"), "12 entries");
    }

    #[test]
    fn a_plural_never_hedges_with_a_parenthesis() {
        for count in 0..4 {
            let text = plural(count, "agent", "agents");
            assert!(!text.contains("(s)"), "{text}");
            assert!(text.starts_with(&count.to_string()), "{text}");
        }
    }
}
