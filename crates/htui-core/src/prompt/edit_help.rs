//! The editors' agent help (MOD-55 P6): the fixed prompt a help turn sends and the parser its reply
//! goes through. Pure: no store, no I/O, no clock. The text is htui's own (`R-ID-5`), not a
//! versioned template (PRD open question 2).

use crate::prompt::template::{Placeholder, TemplateRole};
use crate::prompt::{SectionRefused, scrub_section};
use crate::scrub::{REDACTED, Scrubber};

/// What a help turn edits (A-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpTarget {
    /// A prompt template; its role is `TemplateRole::of_name(name)`.
    Template {
        /// `prompt_template.name`.
        name: String,
    },
    /// A skill body; skills have no placeholders.
    Skill {
        /// `skill.name`.
        name: String,
    },
}

/// One help request: what is edited, the body as the editor holds it, and what the user asked.
/// `Debug` is hand-written (lengths only): it rides in `StoreRequest`, which derives `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct HelpPrompt {
    /// What the body is.
    pub target: HelpTarget,
    /// The editor's buffer, unscrubbed.
    pub body: String,
    /// What the user typed.
    pub request: String,
}

impl core::fmt::Debug for HelpPrompt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The target's name is a slug; the body and the request may hold a secret (H-11), so
        // only their lengths are spelled.
        f.debug_struct("HelpPrompt")
            .field("target", &self.target)
            .field("body_len", &self.body.len())
            .field("request_len", &self.request.len())
            .finish()
    }
}

impl HelpPrompt {
    /// MOD-55 P2: the name, the body and the request, each through [`scrub_section`] under
    /// `"name"`, `"body"` and `"request"` in that order; the first refusal wins.
    ///
    /// # Errors
    ///
    /// [`SectionRefused`].
    pub fn scrubbed(&self, scrubber: &dyn Scrubber) -> Result<Self, SectionRefused> {
        let target = match &self.target {
            HelpTarget::Template { name } => HelpTarget::Template {
                name: scrub_section(scrubber, name, "name")?,
            },
            HelpTarget::Skill { name } => HelpTarget::Skill {
                name: scrub_section(scrubber, name, "name")?,
            },
        };
        Ok(Self {
            target,
            body: scrub_section(scrubber, &self.body, "body")?,
            request: scrub_section(scrubber, &self.request, "request")?,
        })
    }
}

/// The rewrite paragraph every help prompt carries.
pub const INSTRUCTION: &str = "Rewrite the body below as the request asks. Reply with the whole new \
body in one fenced code block, fenced with more backticks than any run of backticks inside it. Put \
nothing in the block but the body, and keep anything else you say short and outside it. Do not use \
tools, run commands or edit files: only your reply is read.";

/// The prompt a help turn sends (§1.4). Call on [`HelpPrompt::scrubbed`]'s output.
///
/// The intro line naming the target, the role's [`placeholder_table`] for a template, the
/// [`INSTRUCTION`], the body in a fence no run inside it can close, then the trimmed request.
#[must_use]
pub fn assemble(prompt: &HelpPrompt) -> String {
    let mut out = match &prompt.target {
        HelpTarget::Template { name } => {
            let role = TemplateRole::of_name(name);
            let mut intro = format!(
                "You are helping edit `{name}`, a {} prompt template in htui, a terminal tool that \
                 runs coding agents. It may use only these placeholders, written in double braces, \
                 and must keep every one marked required:\n",
                role.as_str()
            );
            for line in placeholder_table(role) {
                intro.push_str(&line);
                intro.push('\n');
            }
            intro
        }
        HelpTarget::Skill { name } => format!(
            "You are helping edit `{name}`, a skill in htui, a terminal tool that runs coding \
             agents: Markdown instructions added to an agent's prompt. It has no placeholders.\n"
        ),
    };
    out.push('\n');
    out.push_str(INSTRUCTION);
    out.push_str("\n\nThe body:\n");
    let fence = fence(&prompt.body);
    out.push_str(&fence);
    out.push('\n');
    out.push_str(&prompt.body);
    if !prompt.body.is_empty() && !prompt.body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&fence);
    out.push_str("\n\nThe request:\n");
    out.push_str(prompt.request.trim());
    out.push('\n');
    out
}

/// The `sections[]` names of the recorded `prompt` row, in the order [`assemble`] writes them.
#[must_use]
pub const fn sections(target: &HelpTarget) -> &'static [&'static str] {
    match target {
        HelpTarget::Template { .. } => &["instruction", "placeholders", "body", "request"],
        HelpTarget::Skill { .. } => &["instruction", "body", "request"],
    }
}

/// The role's placeholder table as the editor's column draws it: `{{token}} section|scalar`, plus
/// ` required`, in `Placeholder::ALL` order, filtered by `allowed_in(role)`. One table for the
/// prompt and for the Templates editor's placeholder column.
#[must_use]
pub fn placeholder_table(role: TemplateRole) -> Vec<String> {
    let required = Placeholder::required_by(role);
    Placeholder::ALL
        .iter()
        .filter(|placeholder| placeholder.allowed_in(role))
        .map(|placeholder| {
            let kind = if placeholder.is_section() {
                "section"
            } else {
                "scalar"
            };
            let need = if required.contains(placeholder) {
                " required"
            } else {
                ""
            };
            format!("{{{{{}}}}} {kind}{need}", placeholder.token())
        })
        .collect()
}

/// The proposed body in `reply`: the content of its last closed fenced block, or `None`.
///
/// A CommonMark-shaped fence scan over `reply.lines()` (blueprint §1.5). An opener is up to three
/// spaces, then three or more backticks or tildes; a backtick opener's info string may hold no
/// backtick. Only a run of the same character, at least as long and followed by nothing but
/// whitespace, closes it, so a shorter or different fence inside the block is content (the
/// nested-fence rule). Content lines lose up to the opener's indent. `None` without a closed
/// block, when a block is still open at the end (a cut-off reply is never a proposal), and when
/// the last closed block is empty.
#[must_use]
pub fn proposal(reply: &str) -> Option<String> {
    /// The block being read: its fence character, run length, indent and content so far.
    struct Open {
        /// `` ` `` or `~`.
        mark: char,
        /// The opener's run length.
        run: usize,
        /// The opener's leading spaces.
        indent: usize,
        /// The content lines, each followed by `\n`.
        content: String,
    }

    let mut open: Option<Open> = None;
    let mut last: Option<String> = None;
    for line in reply.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[indent..];
        match open.take() {
            Some(mut block) => {
                let closes = indent <= 3 && {
                    let after = rest.trim_start_matches(block.mark);
                    rest.len() - after.len() >= block.run && after.trim().is_empty()
                };
                if closes {
                    last = Some(block.content);
                } else {
                    let strip = indent.min(block.indent);
                    block.content.push_str(&line[strip..]);
                    block.content.push('\n');
                    open = Some(block);
                }
            }
            None => {
                if indent > 3 {
                    continue;
                }
                let Some(mark) = rest.chars().next().filter(|c| matches!(c, '`' | '~')) else {
                    continue;
                };
                let info = rest.trim_start_matches(mark);
                let run = rest.len() - info.len();
                if run >= 3 && !(mark == '`' && info.contains('`')) {
                    open = Some(Open {
                        mark,
                        run,
                        indent,
                        content: String::new(),
                    });
                }
            }
        }
    }
    if open.is_some() {
        return None;
    }
    last.filter(|content| !content.is_empty())
}

/// MOD-55 P9: whether `proposed` holds the mask marker more often than `sent` did: a masked
/// value came back as `[REDACTED]`.
#[must_use]
pub fn holds_mask(sent: &str, proposed: &str) -> bool {
    proposed.matches(REDACTED).count() > sent.matches(REDACTED).count()
}

/// The fence for `body`: backticks, one more than its longest backtick run, at least three.
///
/// A run anywhere counts, not only at a line start: deliberately conservative, since fencing
/// longer is never wrong. Tildes cannot close a backtick fence, so they do not count.
fn fence(body: &str) -> String {
    let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat((longest + 1).max(3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::DEFAULT_TEMPLATES;
    use crate::scrub::MinimalScrubber;

    fn skill(body: &str, request: &str) -> HelpPrompt {
        HelpPrompt {
            target: HelpTarget::Skill {
                name: "rust-style".to_owned(),
            },
            body: body.to_owned(),
            request: request.to_owned(),
        }
    }

    fn template(name: &str, body: &str) -> HelpPrompt {
        HelpPrompt {
            target: HelpTarget::Template {
                name: name.to_owned(),
            },
            body: body.to_owned(),
            request: "tighten it".to_owned(),
        }
    }

    fn key() -> String {
        format!("ghp_{}", "A1b2".repeat(9))
    }

    /// E-1: blueprint §1.4's listing, byte for byte.
    #[test]
    fn a_skill_prompt_reads_exactly() {
        let expected = concat!(
            "You are helping edit `rust-style`, a skill in htui, a terminal tool that runs coding ",
            "agents: Markdown instructions added to an agent's prompt. It has no placeholders.\n",
            "\n",
            "Rewrite the body below as the request asks. Reply with the whole new body in one ",
            "fenced code block, fenced with more backticks than any run of backticks inside it. ",
            "Put nothing in the block but the body, and keep anything else you say short and ",
            "outside it. Do not use tools, run commands or edit files: only your reply is read.\n",
            "\n",
            "The body:\n",
            "```\n",
            "Use `cargo fmt`.\n",
            "```\n",
            "\n",
            "The request:\n",
            "add clippy\n",
        );
        assert_eq!(
            assemble(&skill("Use `cargo fmt`.\n", "  add clippy \n")),
            expected
        );
    }

    /// E-2: a template's intro names it and its role, and the role's table follows it.
    #[test]
    fn a_template_prompt_carries_its_role_s_table() {
        let assembled = assemble(&template("implement", "Do {{item_key}}.\n"));
        let mut lines = assembled.lines();
        let intro = lines.next().expect("an intro line");
        assert!(
            intro.contains("`implement`, a phase prompt template"),
            "{intro}"
        );
        let table: Vec<String> = lines
            .take_while(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        assert!(!table.is_empty(), "a phase template has placeholders");
        assert_eq!(table, placeholder_table(TemplateRole::Phase));
        assert!(
            table.contains(&"{{item_key}} scalar".to_owned()),
            "{table:?}"
        );

        let judge = assemble(&template("judge", "{{task}} {{candidates}}\n"));
        assert!(
            judge.contains("`judge`, a judge prompt template"),
            "{judge}"
        );
        assert!(
            judge
                .lines()
                .any(|line| line == "{{candidates}} section required"),
            "{judge}"
        );
    }

    /// E-3: a skill has no placeholders, so no table line.
    #[test]
    fn a_skill_prompt_has_no_placeholder_table() {
        let assembled = assemble(&skill("Be brief.\n", "shorter"));
        for line in assembled.lines() {
            let tabled = line.starts_with("{{")
                && (line.contains("}} section") || line.contains("}} scalar"));
            assert!(!tabled, "{line}");
        }
    }

    /// E-4: the body's fence is longer than any backtick run inside it, and the body comes back
    /// out of [`proposal`] unchanged.
    #[test]
    fn the_fence_outruns_every_backtick_run() {
        assert_eq!(fence("a\n```rust\nx\n```\n"), "````");
        assert_eq!(fence("x `````` y"), "```````");
        assert_eq!(fence("plain"), "```");
        assert_eq!(fence("``"), "```");
        assert_eq!(
            fence("~~~~~\n"),
            "```",
            "tildes cannot close a backtick fence"
        );
        for body in [
            "a\n```rust\nx\n```\nB\n",
            "x `````` y\n",
            "plain\n",
            "~~~\nnot a fence here\n~~~\n",
            "    indented\n  ```\n",
        ] {
            let assembled = assemble(&skill(body, "keep it"));
            let cut = assembled
                .find("\n\nThe request:")
                .expect("the request follows the body");
            assert_eq!(
                proposal(&assembled[..cut]).as_deref(),
                Some(body),
                "{assembled}"
            );
        }
    }

    /// E-5: the request is trimmed and is the last thing in the prompt.
    #[test]
    fn the_request_comes_last_trimmed() {
        let assembled = assemble(&skill("x\n", "\n  add clippy \n"));
        assert!(
            assembled.ends_with("The request:\nadd clippy\n"),
            "{assembled}"
        );
    }

    /// E-6: an empty body still gets its fence pair, adjacent.
    #[test]
    fn an_empty_body_is_two_adjacent_fences() {
        let assembled = assemble(&skill("", "write one"));
        assert!(assembled.contains("The body:\n```\n```\n"), "{assembled}");
        let unterminated = assemble(&skill("no newline", "x"));
        assert!(
            unterminated.contains("The body:\n```\nno newline\n```\n"),
            "{unterminated}"
        );
    }

    /// E-7: blueprint §1.5's table.
    #[test]
    fn proposal_cases() {
        let cases: [(&str, Option<&str>); 14] = [
            ("Here:\n```\nnew body\n```\nDone.", Some("new body\n")),
            ("no block at all", None),
            ("```\nold\n```\ntext\n```md\nnew\n```", Some("new\n")),
            (
                "````markdown\nA\n```rust\nx\n```\nB\n````",
                Some("A\n```rust\nx\n```\nB\n"),
            ),
            ("```\na\n```\n```\nb", None),
            ("~~~\nx\n~~~", Some("x\n")),
            ("```\nx\n`````", Some("x\n")),
            ("```a`b\nx\n```", None),
            ("```\n```", None),
            ("```\r\nx\r\n```\r\n", Some("x\n")),
            ("  ```\n  a\n b\n  ```", Some("a\nb\n")),
            ("```\nx\n``` y\n```", Some("x\n``` y\n")),
            ("```\n \n```", Some(" \n")),
            ("~~~\n```\n~~~", Some("```\n")),
        ];
        for (reply, expected) in cases {
            assert_eq!(proposal(reply).as_deref(), expected, "reply: {reply:?}");
        }
    }

    /// E-8: a known value is masked; a credential pattern refuses, naming the section.
    #[test]
    fn scrubbed_masks_a_known_value_and_refuses_a_pattern() {
        let scrubber = MinimalScrubber::new(["hunter2-secret".to_owned()]);
        let masked = skill("pw hunter2-secret", "use hunter2-secret")
            .scrubbed(&scrubber)
            .expect("a known value masks");
        assert_eq!(masked.body, "pw [REDACTED]");
        assert_eq!(masked.request, "use [REDACTED]");
        assert_eq!(masked.target, skill("", "").target);

        let key = key();
        let in_body = skill(&format!("token {key}"), "tidy").scrubbed(&scrubber);
        let refused = in_body.expect_err("a GitHub token pattern is refused");
        assert_eq!(
            refused,
            SectionRefused {
                section: "body",
                rule: "github_token",
            }
        );
        assert_eq!(
            refused.to_string(),
            "the body matches the github_token rule"
        );
        assert!(!refused.to_string().contains("ghp_"));
        assert!(!format!("{refused:?}").contains("ghp_"));

        assert_eq!(
            skill("tidy", &format!("token {key}")).scrubbed(&scrubber),
            Err(SectionRefused {
                section: "request",
                rule: "github_token",
            })
        );
        assert_eq!(
            HelpPrompt {
                target: HelpTarget::Template { name: key.clone() },
                body: key.clone(),
                request: String::new(),
            }
            .scrubbed(&scrubber),
            Err(SectionRefused {
                section: "name",
                rule: "github_token",
            }),
            "the name is scrubbed first"
        );
        assert_eq!(
            skill(&key, &key).scrubbed(&scrubber),
            Err(SectionRefused {
                section: "body",
                rule: "github_token",
            }),
            "the body before the request"
        );
    }

    /// E-9: htui's own default bodies never trip the scrubber.
    #[test]
    fn every_compiled_default_scrubs_clean() {
        let scrubber = MinimalScrubber::new([]);
        for (name, _, body) in DEFAULT_TEMPLATES {
            assert_eq!(
                scrub_section(&scrubber, body, "body").as_deref(),
                Ok(body),
                "{name}"
            );
        }
    }

    /// E-10: only a marker the proposal added counts.
    #[test]
    fn holds_mask_counts_new_markers_only() {
        assert!(holds_mask("a", "a [REDACTED]"));
        assert!(!holds_mask("[REDACTED]", "[REDACTED] b"));
        assert!(!holds_mask("", ""));
        assert!(holds_mask("[REDACTED]", "[REDACTED] [REDACTED]"));
    }

    /// E-11: `Debug` shows lengths, never the body or the request.
    #[test]
    fn a_help_prompt_debugs_lengths_only() {
        let debugged = format!("{:?}", skill("SECRET-BODY", "SECRET-ASK"));
        assert!(!debugged.contains("SECRET-BODY"), "{debugged}");
        assert!(!debugged.contains("SECRET-ASK"), "{debugged}");
        assert!(debugged.contains("body_len: 11"), "{debugged}");
        assert!(debugged.contains("request_len: 10"), "{debugged}");
    }

    /// E-12: the recorded section names follow the assembly order.
    #[test]
    fn sections_follow_the_assembly_order() {
        assert_eq!(
            sections(&template("implement", "").target),
            ["instruction", "placeholders", "body", "request"]
        );
        assert_eq!(
            sections(&skill("", "").target),
            ["instruction", "body", "request"]
        );
    }
}
