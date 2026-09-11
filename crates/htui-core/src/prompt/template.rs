//! The `{{name}}` placeholder scanner and its three closed sets (ANA-5 §4.1 `:302-405`).
//!
//! `htui` owns every prompt (`R-ID-5`), so a template body is substitution and nothing else: no
//! conditionals, no loops, no filters. ANA-5 §4.1 rejected every runtime engine surveyed on
//! placement rather than on quality — the validator has to be callable from MOD-9's editor in
//! `crates/htui`, which reaches `htui-core` and no further — and rejected control flow because a
//! body that can decide its own section layout makes the trim accounting and the digest a function
//! of data the trimmer must reason about. Conditionality is not lost; it moved into Rust, where a
//! section whose data is absent renders the empty string.
//!
//! What the scanner refuses, and why:
//!
//! * `{{ item }}` — no whitespace tolerance inside the braces, so one body has exactly one
//!   spelling and the digest cannot drift on whitespace inside a marker.
//! * `{{Item}}`, `{{itme}}` — a token outside the closed set. Passing it through would ship
//!   `{{itme}}` to a model; rendering it empty is the silent omission ANA-2 refused
//!   (`docs/ANA-2.md:415-416`). Both are [`TemplateError::UnknownPlaceholder`], with the byte
//!   offset so the editor can put the cursor on it.
//! * `{{candidates}}` in a phase body — a real placeholder outside this role's set
//!   ([`TemplateError::WrongRole`]).
//! * `{{item` with no `}}` before the next newline ([`TemplateError::Unterminated`]). Bounding the
//!   search at the newline keeps a single stray `{{` from swallowing the rest of the body.
//! * A `judge` body with no `{{candidates}}`, a `handoff` body missing either of
//!   `{{step_summary}}` and `{{failure_reason}}` ([`TemplateError::MissingRequired`]). A phase body
//!   has no required placeholder: a prompt that is pure instruction is legitimate.
//!
//! `{{{{` is the only escape and yields a literal `{{`. A `}}` outside a placeholder is literal
//! text. The four checks run in ANA-5's order (`:398-399`): syntax, closed set, role, required.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Which closed set a body's placeholders are checked against.
///
/// Derived from the row's `name`: `judge` and `handoff` are reserved (ANA-5 §4.6), everything else
/// is a phase template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateRole {
    /// A step's prompt, built from a phase template.
    Phase,
    /// The reserved `judge` template, one prompt over a fan-out's candidates.
    Judge,
    /// The reserved `handoff` template, promoted after a step gave up.
    Handoff,
}

impl TemplateRole {
    /// `judge` and `handoff` are the reserved names (ANA-5 §4.6); everything else is a phase
    /// template. The match is exact: a row named `Judge` is a phase template.
    #[must_use]
    pub fn of_name(name: &str) -> Self {
        match name {
            "judge" => Self::Judge,
            "handoff" => Self::Handoff,
            _ => Self::Phase,
        }
    }

    /// The stable string written to `trim_record.template.role`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Phase => "phase",
            Self::Judge => "judge",
            Self::Handoff => "handoff",
        }
    }
}

/// Every placeholder htui understands. Adding a variant is a contract change.
///
/// Declaration order is ANA-5 §4.1 `:361-367`, which is the order §4.2 renders the phase sections
/// in. The first six are scalars; the rest stand for sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Placeholder {
    /// `{{item_key}}` — `project.slug` + `:` + `item.key`.
    ItemKey,
    /// `{{item_title}}` — `item.title`, newlines collapsed to spaces.
    ItemTitle,
    /// `{{item_kind}}` — the `item_kind.name` of the item's kind.
    ItemKind,
    /// `{{phase}}` — `ResolvedPhase.name`.
    Phase,
    /// `{{output_kind}}` — `ResolvedPhase.output_kind`; empty when the phase declares none.
    OutputKind,
    /// `{{attempt}}` — `run_step.attempt`, 1-based.
    Attempt,
    /// `{{item}}` — the item body. Never empty: "this item has no body" is information.
    Item,
    /// `{{documents}}` — the `input_kinds` resolution, one block per kind in `input_kinds` order.
    Documents,
    /// `{{upstream}}` — ANA-5 §4.3's `blocked_by` walk.
    Upstream,
    /// `{{box}}` — the box profile projection. Never the box's paths (invariant 3).
    Box,
    /// `{{skills}}` — the `R-SKL-2` resolution, ordered by `skill_binding.position`.
    Skills,
    /// `{{excerpts}}` — ANA-5 §4.5's repository excerpts.
    Excerpts,
    /// `{{verify_failure}}` — the previous attempt's exit code and verification output.
    VerifyFailure,
    /// `{{previous_diff}}` — the previous attempt's `before_hash..after_hash`.
    PreviousDiff,
    /// `{{command_queue}}` — the `R-MCP-4` note about routing heavy commands through `command_run`.
    CommandQueue,
    /// `{{task}}` — judge only: what the candidates were asked to do.
    Task,
    /// `{{candidates}}` — judge only: one block per fan-out candidate. Required in a judge body.
    Candidates,
    /// `{{step_summary}}` — handoff only. Required in a handoff body.
    StepSummary,
    /// `{{diff_so_far}}` — handoff only: what the abandoned step had committed.
    DiffSoFar,
    /// `{{failure_reason}}` — handoff only. Required in a handoff body.
    FailureReason,
}

/// A judge body must place `{{candidates}}`.
const JUDGE_REQUIRED: &[Placeholder] = &[Placeholder::Candidates];
/// A handoff body must place both of these, in this order of complaint.
const HANDOFF_REQUIRED: &[Placeholder] = &[Placeholder::StepSummary, Placeholder::FailureReason];
/// A phase body has no required placeholder.
const NONE_REQUIRED: &[Placeholder] = &[];

impl Placeholder {
    /// Every variant, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::ItemKey,
        Self::ItemTitle,
        Self::ItemKind,
        Self::Phase,
        Self::OutputKind,
        Self::Attempt,
        Self::Item,
        Self::Documents,
        Self::Upstream,
        Self::Box,
        Self::Skills,
        Self::Excerpts,
        Self::VerifyFailure,
        Self::PreviousDiff,
        Self::CommandQueue,
        Self::Task,
        Self::Candidates,
        Self::StepSummary,
        Self::DiffSoFar,
        Self::FailureReason,
    ];

    /// The spelling between the braces. This is the contract `R-PRM-4` documents.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ItemKey => "item_key",
            Self::ItemTitle => "item_title",
            Self::ItemKind => "item_kind",
            Self::Phase => "phase",
            Self::OutputKind => "output_kind",
            Self::Attempt => "attempt",
            Self::Item => "item",
            Self::Documents => "documents",
            Self::Upstream => "upstream",
            Self::Box => "box",
            Self::Skills => "skills",
            Self::Excerpts => "excerpts",
            Self::VerifyFailure => "verify_failure",
            Self::PreviousDiff => "previous_diff",
            Self::CommandQueue => "command_queue",
            Self::Task => "task",
            Self::Candidates => "candidates",
            Self::StepSummary => "step_summary",
            Self::DiffSoFar => "diff_so_far",
            Self::FailureReason => "failure_reason",
        }
    }

    /// The inverse of [`Placeholder::token`]. `None` is "outside the closed set", which is an
    /// error at both gates and never a pass-through.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.token() == token)
    }

    /// Whether the role's closed set contains this placeholder (ANA-5 §4.1 `:319-341`).
    #[must_use]
    pub const fn allowed_in(self, role: TemplateRole) -> bool {
        match role {
            TemplateRole::Phase => !matches!(
                self,
                Self::Task
                    | Self::Candidates
                    | Self::StepSummary
                    | Self::DiffSoFar
                    | Self::FailureReason
            ),
            TemplateRole::Judge => {
                matches!(
                    self,
                    Self::ItemKey | Self::Phase | Self::Task | Self::Candidates
                )
            }
            TemplateRole::Handoff => matches!(
                self,
                Self::ItemKey
                    | Self::Phase
                    | Self::Attempt
                    | Self::Documents
                    | Self::StepSummary
                    | Self::DiffSoFar
                    | Self::FailureReason
            ),
        }
    }

    /// Whether this placeholder stands for a wrapped section rather than a scalar substitution.
    /// False for the six scalars of ANA-5 `:362`, true for the other fourteen.
    #[must_use]
    pub const fn is_section(self) -> bool {
        !matches!(
            self,
            Self::ItemKey
                | Self::ItemTitle
                | Self::ItemKind
                | Self::Phase
                | Self::OutputKind
                | Self::Attempt
        )
    }

    /// The placeholders a body in this role must use, in the order a missing one is reported.
    #[must_use]
    pub const fn required_by(role: TemplateRole) -> &'static [Self] {
        match role {
            TemplateRole::Phase => NONE_REQUIRED,
            TemplateRole::Judge => JUDGE_REQUIRED,
            TemplateRole::Handoff => HANDOFF_REQUIRED,
        }
    }
}

/// One piece of a parsed body, in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    /// Text copied through verbatim. Adjacent literal runs are merged into one span, so a body has
    /// exactly one span sequence.
    Literal(String),
    /// A placeholder the renderer substitutes. Duplicates are legal and each occurrence
    /// substitutes.
    Slot(Placeholder),
}

/// A parsed body: literal spans and placeholder spans, in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTemplate {
    /// Literal and placeholder spans, in source order. This order is the render order and the
    /// `sections[]` order (ANA-5 §4.7).
    pub spans: Vec<Span>,
    /// The placeholders used: first-occurrence order, deduped.
    pub used: Vec<Placeholder>,
}

impl ParsedTemplate {
    /// MOD-9's warning, not an error: a phase body that never places `{{item}}` (ANA-5 §4.1
    /// `:402-403`). Almost always a mistake, never illegal.
    #[must_use]
    pub fn omits_item(&self) -> bool {
        !self.used.contains(&Placeholder::Item)
    }

    /// The literal spans joined — what the `template` section is estimated over.
    #[expect(
        dead_code,
        reason = "T64's render.rs is its only caller; T59 ships the scanner alone"
    )]
    pub(crate) fn literal_text(&self) -> String {
        self.spans
            .iter()
            .filter_map(|span| match span {
                Span::Literal(text) => Some(text.as_str()),
                Span::Slot(_) => None,
            })
            .collect()
    }
}

/// Whether `token` matches `^[a-z][a-z0-9_]*$`. Hand-written because `htui-core` takes no regex
/// dependency and ANA-5 §4.8 made "zero new dependencies" the reason this module may live here.
fn is_token_shaped(token: &str) -> bool {
    let mut chars = token.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The save-time gate MOD-9 calls, and the stage-3 gate the assembler calls on a row that bypassed
/// it. Byte offsets, so the editor can put the cursor on the error.
///
/// # Errors
///
/// The four checks of ANA-5 §4.1 `:398-399`, in order: syntax
/// ([`TemplateError::Unterminated`]), closed-set membership and token shape
/// ([`TemplateError::UnknownPlaceholder`]), role membership ([`TemplateError::WrongRole`]), and the
/// role's required placeholders ([`TemplateError::MissingRequired`]).
pub fn parse(role: TemplateRole, body: &str) -> Result<ParsedTemplate, TemplateError> {
    let mut spans: Vec<Span> = Vec::new();
    let mut used: Vec<Placeholder> = Vec::new();
    let mut literal = String::new();
    let mut i = 0usize;

    while i < body.len() {
        let rest = &body[i..];
        // The only escape, and it is checked first so `{{{{` never reads as an opener.
        if rest.starts_with("{{{{") {
            literal.push_str("{{");
            i += 4;
            continue;
        }
        if let Some(after) = rest.strip_prefix("{{") {
            // The closing `}}` must arrive before the next newline, or a single stray `{{`
            // swallows the rest of the body and reports an offset far from the mistake.
            let line_end = after.find('\n').unwrap_or(after.len());
            let Some(close) = after[..line_end].find("}}") else {
                return Err(TemplateError::Unterminated { at: i });
            };
            let token = &after[..close];
            if !is_token_shaped(token) {
                return Err(TemplateError::UnknownPlaceholder {
                    token: token.to_string(),
                    at: i,
                });
            }
            let Some(placeholder) = Placeholder::from_token(token) else {
                return Err(TemplateError::UnknownPlaceholder {
                    token: token.to_string(),
                    at: i,
                });
            };
            if !placeholder.allowed_in(role) {
                return Err(TemplateError::WrongRole {
                    token: token.to_string(),
                    role,
                    at: i,
                });
            }
            if !literal.is_empty() {
                spans.push(Span::Literal(std::mem::take(&mut literal)));
            }
            spans.push(Span::Slot(placeholder));
            if !used.contains(&placeholder) {
                used.push(placeholder);
            }
            i += 2 + close + 2;
            continue;
        }
        // Everything else, `}}` included, is literal text.
        let c = rest.chars().next().expect("`i` is on a char boundary");
        literal.push(c);
        i += c.len_utf8();
    }
    if !literal.is_empty() {
        spans.push(Span::Literal(literal));
    }

    for &required in Placeholder::required_by(role) {
        if !used.contains(&required) {
            return Err(TemplateError::MissingRequired {
                role,
                token: required.token(),
            });
        }
    }

    Ok(ParsedTemplate { spans, used })
}

/// Why a body was refused. Every variant carries what the editor needs to point at the mistake.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateError {
    /// A token outside the closed set, or not `[a-z][a-z0-9_]*` at all — `{{ item }}`, `{{Item}}`,
    /// `{{itme}}`, `{{}}`.
    #[error("unknown prompt placeholder `{{{{{token}}}}}` at byte {at}")]
    UnknownPlaceholder {
        /// The bytes between the braces, verbatim — whitespace and case included.
        token: String,
        /// The byte offset of the opening `{{`.
        at: usize,
    },
    /// A placeholder htui understands, outside this role's closed set.
    #[error("placeholder `{{{{{token}}}}}` is not available to a {role:?} template, at byte {at}")]
    WrongRole {
        /// The bytes between the braces, verbatim.
        token: String,
        /// The role the body was parsed in.
        role: TemplateRole,
        /// The byte offset of the opening `{{`.
        at: usize,
    },
    /// An opening `{{` with no `}}` before the next newline or the end of input.
    #[error("unterminated `{{{{` at byte {at}")]
    Unterminated {
        /// The byte offset of the opening `{{`.
        at: usize,
    },
    /// A required placeholder never appeared. No offset: the mistake is the body as a whole.
    #[error("a {role:?} template must use {token}")]
    MissingRequired {
        /// The role the body was parsed in.
        role: TemplateRole,
        /// The missing placeholder's token.
        token: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_for(role: TemplateRole) -> Vec<&'static str> {
        Placeholder::ALL
            .iter()
            .filter(|p| p.allowed_in(role))
            .map(|p| p.token())
            .collect()
    }

    #[test]
    fn every_placeholder_round_trips_its_token() {
        assert_eq!(
            Placeholder::ALL.len(),
            20,
            "ANA-5 §4.1 `:361-367` names exactly twenty placeholders"
        );
        let mut seen: Vec<&str> = Vec::new();
        for &p in Placeholder::ALL {
            let token = p.token();
            assert_eq!(
                Placeholder::from_token(token),
                Some(p),
                "`{token}` does not round-trip"
            );
            assert!(!seen.contains(&token), "`{token}` is spelled twice");
            let mut chars = token.chars();
            assert!(
                chars.next().is_some_and(|c| c.is_ascii_lowercase())
                    && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "`{token}` is not `[a-z][a-z0-9_]*`"
            );
            seen.push(token);
        }
        assert_eq!(Placeholder::from_token("itme"), None);
        assert_eq!(Placeholder::from_token(""), None);
    }

    #[test]
    fn the_three_role_sets_are_closed() {
        assert_eq!(
            tokens_for(TemplateRole::Phase),
            vec![
                "item_key",
                "item_title",
                "item_kind",
                "phase",
                "output_kind",
                "attempt",
                "item",
                "documents",
                "upstream",
                "box",
                "skills",
                "excerpts",
                "verify_failure",
                "previous_diff",
                "command_queue",
            ],
            "ANA-5 §4.1 `:321-337`"
        );
        assert_eq!(
            tokens_for(TemplateRole::Judge),
            vec!["item_key", "phase", "task", "candidates"],
            "ANA-5 §4.1 `:339`"
        );
        assert_eq!(
            tokens_for(TemplateRole::Handoff),
            vec![
                "item_key",
                "phase",
                "attempt",
                "documents",
                "step_summary",
                "diff_so_far",
                "failure_reason",
            ],
            "ANA-5 §4.1 `:340-341`"
        );

        // The six scalars are not sections; the other fourteen are.
        let scalars = [
            Placeholder::ItemKey,
            Placeholder::ItemTitle,
            Placeholder::ItemKind,
            Placeholder::Phase,
            Placeholder::OutputKind,
            Placeholder::Attempt,
        ];
        for &p in Placeholder::ALL {
            assert_eq!(
                p.is_section(),
                !scalars.contains(&p),
                "`{}` is on the wrong side of the scalar/section line",
                p.token()
            );
        }

        // The role a name resolves to; `judge` and `handoff` are the only reserved names.
        assert_eq!(TemplateRole::of_name("judge"), TemplateRole::Judge);
        assert_eq!(TemplateRole::of_name("handoff"), TemplateRole::Handoff);
        assert_eq!(TemplateRole::of_name("implement"), TemplateRole::Phase);
        assert_eq!(TemplateRole::of_name("Judge"), TemplateRole::Phase);
        assert_eq!(TemplateRole::Phase.as_str(), "phase");
        assert_eq!(TemplateRole::Judge.as_str(), "judge");
        assert_eq!(TemplateRole::Handoff.as_str(), "handoff");
    }

    #[test]
    fn quadruple_brace_is_a_literal_double_brace() {
        let parsed = parse(TemplateRole::Phase, "a {{{{item}} b").expect("the only escape");
        assert_eq!(
            parsed.spans,
            vec![Span::Literal("a {{item}} b".to_string())]
        );
        assert!(parsed.used.is_empty());
        // And it escapes the opener only: the trailing `}}` is literal either way.
        let parsed = parse(TemplateRole::Phase, "{{{{").expect("a bare escape");
        assert_eq!(parsed.spans, vec![Span::Literal("{{".to_string())]);
    }

    #[test]
    fn spaces_inside_braces_are_unknown() {
        assert_eq!(
            parse(TemplateRole::Phase, "{{ item }}"),
            Err(TemplateError::UnknownPlaceholder {
                token: " item ".to_string(),
                at: 0,
            }),
            "no whitespace tolerance: one body has exactly one spelling"
        );
    }

    #[test]
    fn a_typo_is_unknown_with_its_offset() {
        assert_eq!(
            parse(TemplateRole::Phase, "lead\n{{itme}}"),
            Err(TemplateError::UnknownPlaceholder {
                token: "itme".to_string(),
                at: 5,
            })
        );
    }

    #[test]
    fn uppercase_is_unknown() {
        assert_eq!(
            parse(TemplateRole::Phase, "{{Item}}"),
            Err(TemplateError::UnknownPlaceholder {
                token: "Item".to_string(),
                at: 0,
            })
        );
        assert_eq!(
            parse(TemplateRole::Phase, "{{}}"),
            Err(TemplateError::UnknownPlaceholder {
                token: String::new(),
                at: 0,
            })
        );
    }

    #[test]
    fn a_bare_open_is_unterminated() {
        assert_eq!(
            parse(TemplateRole::Phase, "start {{item\nnext line"),
            Err(TemplateError::Unterminated { at: 6 }),
            "the closing `}}}}` must arrive before the next newline"
        );
        assert_eq!(
            parse(TemplateRole::Phase, "tail {{item"),
            Err(TemplateError::Unterminated { at: 5 })
        );
    }

    #[test]
    fn candidates_in_a_phase_body_is_wrong_role() {
        assert_eq!(
            parse(TemplateRole::Phase, "x {{candidates}}"),
            Err(TemplateError::WrongRole {
                token: "candidates".to_string(),
                role: TemplateRole::Phase,
                at: 2,
            })
        );
        assert_eq!(
            parse(TemplateRole::Judge, "{{candidates}} {{item}}"),
            Err(TemplateError::WrongRole {
                token: "item".to_string(),
                role: TemplateRole::Judge,
                at: 15,
            }),
            "the closed set is checked before the role, and the role before the required set"
        );
    }

    #[test]
    fn a_judge_body_without_candidates_is_missing_required() {
        assert_eq!(
            parse(TemplateRole::Judge, "{{item_key}} {{phase}} {{task}}"),
            Err(TemplateError::MissingRequired {
                role: TemplateRole::Judge,
                token: "candidates",
            })
        );
        parse(TemplateRole::Judge, "{{task}}\n{{candidates}}").expect("candidates is enough");
    }

    #[test]
    fn a_handoff_body_needs_two() {
        assert_eq!(
            parse(TemplateRole::Handoff, "{{step_summary}}"),
            Err(TemplateError::MissingRequired {
                role: TemplateRole::Handoff,
                token: "failure_reason",
            })
        );
        assert_eq!(
            parse(TemplateRole::Handoff, "{{failure_reason}}"),
            Err(TemplateError::MissingRequired {
                role: TemplateRole::Handoff,
                token: "step_summary",
            })
        );
        parse(
            TemplateRole::Handoff,
            "{{step_summary}}\n{{failure_reason}}",
        )
        .expect("both required placeholders present");
        // A phase body has no required placeholder: pure instruction is legitimate.
        parse(TemplateRole::Phase, "Do the thing.").expect("no phase requirement");
    }

    #[test]
    fn literal_close_braces_pass_through() {
        let parsed = parse(TemplateRole::Phase, "a }} b }}}} c").expect("`}}` outside a slot");
        assert_eq!(
            parsed.spans,
            vec![Span::Literal("a }} b }}}} c".to_string())],
            "a `}}}}` outside a placeholder is literal text, and literal runs merge into one span"
        );
    }

    #[test]
    fn duplicates_substitute_twice_and_used_lists_once() {
        let parsed =
            parse(TemplateRole::Phase, "{{item}} x {{item}} y {{phase}}").expect("legal body");
        assert_eq!(
            parsed.spans,
            vec![
                Span::Slot(Placeholder::Item),
                Span::Literal(" x ".to_string()),
                Span::Slot(Placeholder::Item),
                Span::Literal(" y ".to_string()),
                Span::Slot(Placeholder::Phase),
            ]
        );
        assert_eq!(parsed.used, vec![Placeholder::Item, Placeholder::Phase]);
    }

    #[test]
    fn omits_item_warns() {
        assert!(
            parse(TemplateRole::Phase, "{{item_key}} only")
                .expect("legal")
                .omits_item()
        );
        assert!(
            !parse(TemplateRole::Phase, "{{item}}")
                .expect("legal")
                .omits_item()
        );
    }
}
