//! The ten default template bodies (ANA-5 §5.4 `:1601-1810`; plan D104, D111).
//!
//! One body per seeded phase name, plus the two reserved names `judge` and `handoff`. They ship as
//! a compiled-in table rather than as a migration because the two acts are different: *confirming*
//! the bodies — parsing them in their role, rendering them into a golden prompt — is MOD-2's, while
//! *seeding* them into a new project's `prompt_template` rows needs a project-creation path that
//! ANA-5 §4.6 assigns to MOD-15 by name. A constant satisfies the first without pre-empting the
//! second, and gives MOD-15 something to seed from rather than a document to retype.
//!
//! Two fragments here are machine-read and are therefore contract rather than prose (ANA-5
//! `:1605-1606`): the `review` body's three-line front matter, which MOD-4 parses to learn the
//! verdict, and the `judge` body's fenced `json` block, which MOD-4 parses to learn the winner.
//! Reword the surrounding paragraphs freely; changing either fragment changes a wire format.
//!
//! Every body follows §5.4's one frame: a line of role, the bulk placeholders in `R-PRM-1`'s order,
//! then the instruction last. Blank lines around a placeholder that renders empty collapse at
//! assembly (§4.7 step 5), so a body may place every section unconditionally.

use crate::prompt::template::TemplateRole;

/// The `command_queue` section's two sentences (ANA-5 §4.2 `:484`, §4.4 `:827`; plan D111).
///
/// ANA-5 fixes that the section exists and never writes its words. They are fixed here, once,
/// because every byte of a rendered section is a `prompt_digest` input: a reworded sentence
/// invalidates every golden prompt and every stored digest taken before it.
pub const COMMAND_QUEUE_TEXT: &str = "Route builds, test suites and verification through the `command_run` tool rather than a shell: htui queues them per box under `R-MCP-4` and records their output on this step. Run everything else directly.";

/// ANA-5 §5.4's `prd` body (`:1611-1628`).
const PRD: &str = r#"You are running the `prd` phase for {{item_key}} ({{item_kind}}).

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Write a `prd` document: the problem in the user's terms, who has it, what "done" looks like, and
the open questions a reader must answer before design can start. State what is explicitly out of
scope. Do not design a solution and do not name files. If a requirement id in docs/REQUIREMENTS.md
governs this work, cite it.
"#;

/// ANA-5 §5.4's `plan` body (`:1630-1646`).
const PLAN: &str = r#"You are running the `plan` phase for {{item_key}} ({{item_kind}}).

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Write a `plan` document: ordered steps, each naming the files it touches and the observable result
that proves it landed. Call out risks and the validation you will run. Cite the requirement ids the
work satisfies. Change no code in this phase.
"#;

/// ANA-5 §5.4's `implement` body (`:1648-1667`).
const IMPLEMENT: &str = r#"You are running the `implement` phase for {{item_key}}, attempt {{attempt}}.

{{item}}
{{documents}}
{{verify_failure}}
{{previous_diff}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Make the change the plan describes, in this working tree. Keep the diff to what the plan asks for.
When a prior attempt is shown above, read its diff and its verification output first and fix the
cause rather than the symptom. Write a `{{output_kind}}` document describing what changed and how
you verified it, one section per file group.
"#;

/// ANA-5 §5.4's `review` body (`:1669-1695`).
///
/// The three-line front matter is byte-identical to `:1686-1688` and MOD-4 parses it.
const REVIEW: &str = r#"You are running the `review` phase for {{item_key}}.

{{item}}
{{documents}}
{{previous_diff}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}

Review the change against the plan, the item body and the requirement ids it cites.

Your `review` document must begin with exactly these three lines and nothing before them:

---
verdict: request-changes
---

Set `verdict` to `approve` or `request-changes`, lowercase, nothing else on the line. Everything
after the closing `---` is your review: one section per finding, each naming the file and the
requirement, invariant or plan step it breaks, and what to do instead. `request-changes` sends the
item back to `implement` with this document attached, so every finding must be actionable. Do not
edit any file in this phase.
"#;

/// ANA-5 §5.4's `research` body (`:1697-1714`).
const RESEARCH: &str = r#"You are running the `research` phase for {{item_key}} ({{item_kind}}).

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Write a `research` document: what is already true in this repository, what prior art exists, what
options are open and what each costs. Cite file paths with line numbers for repository claims and
URLs for external ones. Where two sources disagree, say so and say which you believe. Reach no
verdict; the `verdict` phase does that.
"#;

/// ANA-5 §5.4's `verdict` body (`:1716-1730`).
const VERDICT: &str = r#"You are running the `verdict` phase for {{item_key}}.

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}

Write a `verdict` document: for each question the research raised, the option adopted, the options
rejected and the reason each lost. Name what only the maintainer can decide and pick a default for
each. End with the work items the verdict implies. Cite the requirement ids involved.
"#;

/// ANA-5 §5.4's `reproduce` body (`:1732-1749`).
const REPRODUCE: &str = r#"You are running the `reproduce` phase for {{item_key}}.

{{item}}
{{documents}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Reproduce the reported failure in this working tree and write a `reproduce` document: the exact
command, the exact output, the smallest input that still fails, and the first place in the code
where behaviour diverges from expectation. Add a failing test if the project has a test suite.
Do not fix the bug in this phase.
"#;

/// ANA-5 §5.4's `fix` body (`:1751-1769`).
const FIX: &str = r#"You are running the `fix` phase for {{item_key}}, attempt {{attempt}}.

{{item}}
{{documents}}
{{verify_failure}}
{{previous_diff}}
{{upstream}}
{{box}}
{{skills}}
{{excerpts}}
{{command_queue}}

Fix the cause the reproduction identified, not the symptom. Keep the diff minimal and leave the
failing test passing. Write a `{{output_kind}}` document describing the cause, the fix and the
evidence it is fixed.
"#;

/// ANA-5 §5.4's `judge` body (`:1771-1794`), the first reserved name.
///
/// The fenced `json` block is byte-identical to `:1788-1790` and MOD-4 parses it.
const JUDGE: &str = r#"You are judging {{phase}} candidates for {{item_key}}. You did not write any of them.

{{task}}

{{candidates}}

Pick the candidate that best satisfies the task above. Weigh, in order: correctness against the
task, evidence from verification, the smallest change that does the job, and fit with the
surrounding code. Ignore candidate order, prose confidence and length; a longer answer is not a
better one.

Write one paragraph per candidate saying why it wins or loses, then end the document with exactly
one fenced json block, and nothing after it:

```json
{ "winner": 0, "reasons": { "0": "one line", "1": "one line" } }
```

`winner` must be one of the `fanout_index` values shown above, and `reasons` must have one entry
per candidate shown, keyed by its index as a string.
"#;

/// ANA-5 §5.4's `handoff` body (`:1796-1810`), the second reserved name.
const HANDOFF: &str = r#"You are continuing work already under way on {{item_key}}, phase {{phase}}, attempt {{attempt}}.
A previous session did the work below and could not be resumed, so this is a fresh context. The
working tree is exactly as that session left it.

{{step_summary}}
{{documents}}
{{diff_so_far}}
{{failure_reason}}

Read the diff before changing anything: the work already done is real and must not be redone or
reverted. Continue from where it stopped, address the failure reason, and finish the phase.
"#;

/// ANA-5 §5.4's ten bodies verbatim, in §5.4's order: the eight seeded phase names followed by the
/// two reserved ones.
///
/// Each row is `(name, role, body)`, and the role is always [`TemplateRole::of_name`] of the name —
/// the tuple carries it so a caller seeding a row does not have to re-derive it. `fixtures.rs`
/// seeds the demo corpus from this table; MOD-15 seeds the database from it.
pub const DEFAULT_TEMPLATES: [(&str, TemplateRole, &str); 10] = [
    ("prd", TemplateRole::Phase, PRD),
    ("plan", TemplateRole::Phase, PLAN),
    ("implement", TemplateRole::Phase, IMPLEMENT),
    ("review", TemplateRole::Phase, REVIEW),
    ("research", TemplateRole::Phase, RESEARCH),
    ("verdict", TemplateRole::Phase, VERDICT),
    ("reproduce", TemplateRole::Phase, REPRODUCE),
    ("fix", TemplateRole::Phase, FIX),
    ("judge", TemplateRole::Judge, JUDGE),
    ("handoff", TemplateRole::Handoff, HANDOFF),
];

/// The default body for a template name, or `None` when the name has no default.
///
/// A project may hold templates htui never shipped — `template_name` on a phase is free text
/// (`0001_init.sql:242`) — so a miss is ordinary, not an error.
#[must_use]
pub fn body_of(name: &str) -> Option<&'static str> {
    DEFAULT_TEMPLATES
        .iter()
        .find(|(template_name, _, _)| *template_name == name)
        .map(|(_, _, body)| *body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::template::{Placeholder, TemplateError, parse};

    /// The three roles, so a body can be offered every role it is *not* written for.
    const ROLES: [TemplateRole; 3] = [
        TemplateRole::Phase,
        TemplateRole::Judge,
        TemplateRole::Handoff,
    ];

    /// ANA-5 §12 criterion 1, first half: every shipped body is a body the validator accepts.
    #[test]
    fn every_default_body_parses_in_its_role() {
        assert_eq!(
            DEFAULT_TEMPLATES.len(),
            10,
            "ANA-5 §5.4 names eight phase bodies and two reserved ones"
        );
        let names: Vec<&str> = DEFAULT_TEMPLATES.iter().map(|(name, ..)| *name).collect();
        assert_eq!(
            names,
            [
                "prd",
                "plan",
                "implement",
                "review",
                "research",
                "verdict",
                "reproduce",
                "fix",
                "judge",
                "handoff",
            ],
            "§5.4's order, which is the order `fixtures.rs` mints template ids in"
        );

        for (name, role, body) in DEFAULT_TEMPLATES {
            assert_eq!(
                role,
                TemplateRole::of_name(name),
                "`{name}`'s declared role disagrees with the name it is derived from"
            );
            let parsed = parse(role, body)
                .unwrap_or_else(|error| panic!("§5.4's `{name}` body does not parse: {error}"));
            assert!(
                !parsed.used.is_empty(),
                "`{name}` places no placeholder at all"
            );
            assert!(
                body.ends_with('\n') && !body.ends_with("\n\n"),
                "`{name}` ends with exactly one newline, like every §5.4 block"
            );
            assert!(
                !body.contains('\r'),
                "`{name}` is LF-only; CRLF is normalised at render, not stored here"
            );
            assert_eq!(body_of(name), Some(body), "`{name}` is not reachable");
        }
        assert_eq!(body_of("no-such-template"), None);
        assert_eq!(body_of("Judge"), None, "the lookup is exact, like the role");

        // A phase body is allowed to omit `{{item}}`, but none of these does (ANA-5 §4.1
        // `:402-403` calls that omission almost always a mistake).
        for (name, role, body) in DEFAULT_TEMPLATES {
            if role == TemplateRole::Phase {
                assert!(
                    !parse(role, body).expect("parsed above").omits_item(),
                    "`{name}` never places `{{{{item}}}}`"
                );
            }
        }
    }

    /// ANA-5 §12 criterion 1, second half: the closed sets are closed *against* each other. A body
    /// offered a role it was not written for is refused, and the refusal names the token.
    ///
    /// The expected token is the body's first placeholder outside the offered role's set, which is
    /// not the same token for every body: a `prd` offered `Judge` trips on `{{item_kind}}` long
    /// before it reaches `{{item}}`, and an `implement` trips on `{{attempt}}`.
    #[test]
    fn every_phase_body_is_wrong_role_for_judge_and_handoff_and_vice_versa() {
        /// A body's name, and the token a parse in each of [`ROLES`] trips on. `None` marks the
        /// body's own role, which is the one that parses.
        type RoleTrip = (&'static str, [Option<&'static str>; 3]);

        const EXPECTED: [RoleTrip; 10] = [
            ("prd", [None, Some("item_kind"), Some("item_kind")]),
            ("plan", [None, Some("item_kind"), Some("item_kind")]),
            ("implement", [None, Some("attempt"), Some("item")]),
            ("review", [None, Some("item"), Some("item")]),
            ("research", [None, Some("item_kind"), Some("item_kind")]),
            ("verdict", [None, Some("item"), Some("item")]),
            ("reproduce", [None, Some("item"), Some("item")]),
            ("fix", [None, Some("attempt"), Some("item")]),
            ("judge", [Some("task"), None, Some("task")]),
            ("handoff", [Some("step_summary"), Some("attempt"), None]),
        ];

        for (index, (name, role, body)) in DEFAULT_TEMPLATES.iter().enumerate() {
            let (expected_name, expected) = EXPECTED[index];
            assert_eq!(expected_name, *name, "the two tables are in the same order");

            for (offered, wanted) in ROLES.into_iter().zip(expected) {
                let Some(token) = wanted else {
                    assert_eq!(offered, *role, "`None` marks the body's own role");
                    parse(offered, body).expect("its own role was checked above");
                    continue;
                };
                assert_ne!(offered, *role);
                let error = parse(offered, body)
                    .expect_err("a body must be refused by a role it was not written for");
                assert_eq!(
                    error,
                    TemplateError::WrongRole {
                        token: token.to_owned(),
                        role: offered,
                        at: body
                            .find(&format!("{{{{{token}}}}}"))
                            .expect("the token is in the body"),
                    },
                    "`{name}` offered {offered:?}"
                );
                // The refusal is `WrongRole` and not `UnknownPlaceholder` precisely because the
                // token is real: it belongs to the body's own role and not to the offered one.
                let placeholder =
                    Placeholder::from_token(token).expect("the named token is a real placeholder");
                assert!(placeholder.allowed_in(*role), "`{token}` is in {role:?}");
                assert!(
                    !placeholder.allowed_in(offered),
                    "`{token}` is outside {offered:?}"
                );
            }
        }
    }

    /// MOD-4 reads the verdict out of the first three lines of a `review` document, so the body's
    /// instruction is a wire format. `docs/ANA-5.md:1684-1694`.
    #[test]
    fn the_review_body_states_the_front_matter_verbatim() {
        let body = body_of("review").expect("a seeded name");
        assert!(
            body.contains(
                "Your `review` document must begin with exactly these three lines and nothing \
                 before them:\n\n---\nverdict: request-changes\n---\n"
            ),
            "the three-line front matter moved; MOD-4 parses it"
        );
        assert!(
            body.contains(
                "Set `verdict` to `approve` or `request-changes`, lowercase, nothing else on the \
                 line."
            ),
            "the two legal verdict values are part of the contract"
        );
    }

    /// MOD-4 reads the winner out of the last fenced `json` block of a `judge` document.
    /// `docs/ANA-5.md:1785-1793`.
    #[test]
    fn the_judge_body_states_the_verdict_block_verbatim() {
        let body = body_of("judge").expect("a reserved name");
        assert!(
            body.contains(
                "one fenced json block, and nothing after it:\n\n```json\n{ \"winner\": 0, \
                 \"reasons\": { \"0\": \"one line\", \"1\": \"one line\" } }\n```\n"
            ),
            "the fenced json example moved; MOD-4 parses the block it describes"
        );
        assert!(
            body.contains("`winner` must be one of the `fanout_index` values shown above"),
            "the winner is an index into the fan-out, not an ordinal of the listing"
        );
        // The example is data, not a placeholder: the scanner must have read every `{` here as
        // literal text.
        assert!(
            !body.contains("{{winner}}"),
            "the json example is literal and takes no substitution"
        );
    }

    /// The section is two sentences on one line: `{{command_queue}}` sits on a line of its own in
    /// every body, and a stray newline inside the text would change both the render and the digest.
    #[test]
    fn command_queue_text_is_two_sentences() {
        assert!(
            !COMMAND_QUEUE_TEXT.contains('\n'),
            "one line: the section wrapper supplies the newlines"
        );
        let sentences: Vec<&str> = COMMAND_QUEUE_TEXT.split_terminator(". ").collect();
        assert_eq!(sentences.len(), 2, "ANA-5 §4.2 `:484` asks for two");
        assert!(COMMAND_QUEUE_TEXT.ends_with('.'));
        assert!(
            COMMAND_QUEUE_TEXT.contains("`command_run`")
                && COMMAND_QUEUE_TEXT.contains("`R-MCP-4`"),
            "the tool the section is about and the requirement it cites"
        );
        assert_eq!(
            COMMAND_QUEUE_TEXT,
            "Route builds, test suites and verification through the `command_run` tool rather \
             than a shell: htui queues them per box under `R-MCP-4` and records their output on \
             this step. Run everything else directly.",
            "plan D111 fixed these bytes; they are a `prompt_digest` input"
        );
    }
}
