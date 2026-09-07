//! Stages 1 and 2 of `docs/ANA-4.md` §4.3's permission pipeline, as a pure function.
//!
//! §4.3 runs every `session/request_permission` through three stages: the row's `rules[]`, in
//! order; the `remembered[]` `_always` choices; then the user (`R-TUI-6`). The first two are
//! decidable from data alone, so they live here — no channel, no transport, no clock — and the
//! caller (`crates/htui/src/agent_worker.rs`) is what turns a [`PolicyAnswer`] into the wire
//! answer and the `permission_answer { by: "policy" }` row. Blueprint H-5: the recorder lives with
//! the caller, not in the session task, so the decision has to be reachable from both.
//!
//! Stage 3 is `None`: park the request and ask.

use crate::driver::{PermissionDefault, PermissionMatch, PermissionPolicy};
use crate::event::{PermissionOption, PermissionOptionKind, ToolCallEvent};

/// Which stage answered.
///
/// All three record `by: "policy"` — ANA-9 §4.3's `by` vocabulary is `user | policy` and the
/// distinction it draws is "not typed by a human now" (§4.3 stage 2) — so this exists for the
/// recorded `reason` and for the log, not for the row's `by`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyStage {
    /// `agent.settings.permission.rules[]` matched.
    Rule,
    /// `agent.settings.permission.remembered[]` matched.
    Remembered,
    /// Neither matched and `agent.settings.permission.default` is not `ask`.
    Default,
}

/// A stage-1/2 answer: the option to send back, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyAnswer {
    /// The `PermissionOption::id` to return to the agent.
    pub option_id: String,
    /// The kind that option carries; the log and the UI say which door was taken.
    pub kind: PermissionOptionKind,
    /// Which stage decided.
    pub stage: PolicyStage,
    /// `rule.reason`, `"remembered"` or `"default"`.
    pub reason: String,
}

/// Evaluates stages 1 and 2 over one request.
///
/// `call` is the `tool_call` event the caller recorded for this request's `tool_call_id`, when it
/// saw one: a rule that matches on `tool_kind` or a path has nothing to match without it, and
/// `None` therefore matches only an all-`None` matcher (which §5.2 defines as matching every
/// request).
///
/// Returns `None` for stage 3 — park the request and ask the user.
#[must_use]
pub fn evaluate(
    policy: &PermissionPolicy,
    call: Option<&ToolCallEvent>,
    options: &[PermissionOption],
) -> Option<PolicyAnswer> {
    for rule in &policy.rules {
        if matches(&rule.matcher, call) {
            if let Some(option) = option_for(rule.answer, options) {
                return Some(PolicyAnswer {
                    option_id: option.id.clone(),
                    kind: option.kind,
                    stage: PolicyStage::Rule,
                    reason: rule.reason.clone(),
                });
            }
            // A rule that matched but whose answer the agent does not offer is not a silent pass
            // to the next rule: it is a row that cannot be honoured, and asking the user is the
            // fail-safe direction.
            tracing::warn!(
                answer = rule.answer.as_str(),
                "a permission rule matched but the agent offers no such option; asking the user"
            );
            return None;
        }
    }

    for remembered in &policy.remembered {
        if matches(&remembered.matcher, call)
            && let Some(option) = option_for(remembered.option_kind, options)
        {
            return Some(PolicyAnswer {
                option_id: option.id.clone(),
                kind: option.kind,
                stage: PolicyStage::Remembered,
                reason: "remembered".to_owned(),
            });
        }
    }

    let kind = match policy.default {
        PermissionDefault::Ask => return None,
        PermissionDefault::Allow => PermissionOptionKind::AllowOnce,
        PermissionDefault::Deny => PermissionOptionKind::RejectOnce,
    };
    option_for(kind, options).map(|option| PolicyAnswer {
        option_id: option.id.clone(),
        kind: option.kind,
        stage: PolicyStage::Default,
        reason: "default".to_owned(),
    })
}

/// Whether `matcher` matches the call being gated.
///
/// Every field is `AND`ed, and an absent field matches anything, so `PermissionMatch::default()`
/// matches every request (§5.2). The four fields read:
///
/// - `tool_kind` against `call.tool_kind`;
/// - `tool_name` against `call.title` — ACP v1 carries **no** tool name (a name field exists only
///   behind the SDK's `unstable_tool_call_name` feature), and the title is the only human-readable
///   identifier on the wire. Milestone 8's CLI transport has real names; until then a `tool_name`
///   rule is a title rule, and this doc comment is the contract;
/// - `path_prefix` against `locations[0].path`, falling back to `input.path` / `input.file_path`;
/// - `command_prefix` against `input.command` (`R-MCP-4`'s shell rules).
#[must_use]
pub fn matches(matcher: &PermissionMatch, call: Option<&ToolCallEvent>) -> bool {
    if matcher == &PermissionMatch::default() {
        return true;
    }
    let Some(call) = call else {
        return false;
    };
    if let Some(kind) = &matcher.tool_kind
        && kind != call.tool_kind.as_str()
    {
        return false;
    }
    if let Some(name) = &matcher.tool_name
        && name != &call.title
    {
        return false;
    }
    if let Some(prefix) = &matcher.path_prefix
        && !path_of(call).is_some_and(|path| path.starts_with(prefix.as_str()))
    {
        return false;
    }
    if let Some(prefix) = &matcher.command_prefix
        && !string_at(call, "command").is_some_and(|command| command.starts_with(prefix.as_str()))
    {
        return false;
    }
    true
}

/// The option that answers `kind`: the exact kind when the agent offers it, else its `_once` /
/// `_always` sibling, else `None`.
///
/// The sibling fallback is what makes a `reject_always` rule work against the `claude` adapter,
/// which currently offers three of the four kinds (§4.3).
#[must_use]
pub fn option_for(
    kind: PermissionOptionKind,
    options: &[PermissionOption],
) -> Option<&PermissionOption> {
    options
        .iter()
        .find(|option| option.kind == kind)
        .or_else(|| options.iter().find(|option| option.kind == sibling(kind)))
}

/// The other half of a kind's pair: `allow_once` ↔ `allow_always`, `reject_once` ↔ `reject_always`.
const fn sibling(kind: PermissionOptionKind) -> PermissionOptionKind {
    match kind {
        PermissionOptionKind::AllowOnce => PermissionOptionKind::AllowAlways,
        PermissionOptionKind::AllowAlways => PermissionOptionKind::AllowOnce,
        PermissionOptionKind::RejectOnce => PermissionOptionKind::RejectAlways,
        PermissionOptionKind::RejectAlways => PermissionOptionKind::RejectOnce,
    }
}

/// The path a call names: its first location, else `input.path`, else `input.file_path`.
fn path_of(call: &ToolCallEvent) -> Option<&str> {
    call.locations
        .first()
        .map(|location| location.path.as_str())
        .or_else(|| string_at(call, "path"))
        .or_else(|| string_at(call, "file_path"))
}

/// A string field of the call's raw input.
fn string_at<'a>(call: &'a ToolCallEvent, key: &str) -> Option<&'a str> {
    call.input.get(key).and_then(serde_json::Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{PermissionRule, RememberedPermission};
    use crate::event::{ToolKind, ToolLocation};
    use chrono::Utc;
    use serde_json::json;

    fn options() -> Vec<PermissionOption> {
        vec![
            PermissionOption {
                id: "allow".to_owned(),
                label: "Allow".to_owned(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "allow-always".to_owned(),
                label: "Allow always".to_owned(),
                kind: PermissionOptionKind::AllowAlways,
            },
            PermissionOption {
                id: "reject".to_owned(),
                label: "Reject".to_owned(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ]
    }

    fn call(kind: ToolKind, title: &str, input: serde_json::Value) -> ToolCallEvent {
        ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: title.to_owned(),
            tool_kind: kind,
            input,
            locations: Vec::new(),
        }
    }

    fn rule(
        matcher: PermissionMatch,
        answer: PermissionOptionKind,
        reason: &str,
    ) -> PermissionRule {
        PermissionRule {
            matcher,
            answer,
            reason: reason.to_owned(),
        }
    }

    #[test]
    fn no_rules_and_the_default_ask_park_the_request() {
        let policy = PermissionPolicy::default();
        let call = call(ToolKind::Edit, "Edit", json!({}));
        assert_eq!(evaluate(&policy, Some(&call), &options()), None);
    }

    #[test]
    fn rules_are_evaluated_in_order_and_the_first_match_answers() {
        let policy = PermissionPolicy {
            rules: vec![
                rule(
                    PermissionMatch {
                        tool_kind: Some("read".to_owned()),
                        ..PermissionMatch::default()
                    },
                    PermissionOptionKind::AllowOnce,
                    "reads are safe",
                ),
                rule(
                    PermissionMatch::default(),
                    PermissionOptionKind::RejectOnce,
                    "everything else",
                ),
            ],
            ..PermissionPolicy::default()
        };

        let read = call(ToolKind::Read, "Read", json!({}));
        let answered = evaluate(&policy, Some(&read), &options()).expect("rule 1");
        assert_eq!(answered.option_id, "allow");
        assert_eq!(answered.stage, PolicyStage::Rule);
        assert_eq!(answered.reason, "reads are safe");

        let execute = call(ToolKind::Execute, "Bash", json!({}));
        let answered = evaluate(&policy, Some(&execute), &options()).expect("rule 2");
        assert_eq!(answered.option_id, "reject", "the catch-all rule answers");
    }

    #[test]
    fn a_remembered_entry_answers_only_after_every_rule_missed() {
        let policy = PermissionPolicy {
            rules: vec![rule(
                PermissionMatch {
                    tool_kind: Some("read".to_owned()),
                    ..PermissionMatch::default()
                },
                PermissionOptionKind::RejectOnce,
                "not reads",
            )],
            remembered: vec![RememberedPermission {
                matcher: PermissionMatch {
                    tool_kind: Some("edit".to_owned()),
                    ..PermissionMatch::default()
                },
                option_kind: PermissionOptionKind::AllowAlways,
                added_at: Utc::now(),
                added_by: "user".to_owned(),
            }],
            ..PermissionPolicy::default()
        };

        let edit = call(ToolKind::Edit, "Edit", json!({}));
        let answered = evaluate(&policy, Some(&edit), &options()).expect("remembered");
        assert_eq!(answered.option_id, "allow-always");
        assert_eq!(answered.stage, PolicyStage::Remembered);
        assert_eq!(answered.reason, "remembered");

        let read = call(ToolKind::Read, "Read", json!({}));
        assert_eq!(
            evaluate(&policy, Some(&read), &options())
                .expect("the rule wins")
                .stage,
            PolicyStage::Rule
        );
    }

    #[test]
    fn the_default_answers_when_it_is_not_ask() {
        let allow = PermissionPolicy {
            default: PermissionDefault::Allow,
            ..PermissionPolicy::default()
        };
        let deny = PermissionPolicy {
            default: PermissionDefault::Deny,
            ..PermissionPolicy::default()
        };
        let call = call(ToolKind::Execute, "Bash", json!({}));

        let answered = evaluate(&allow, Some(&call), &options()).expect("allow");
        assert_eq!(answered.option_id, "allow");
        assert_eq!(answered.stage, PolicyStage::Default);
        assert_eq!(answered.reason, "default");

        assert_eq!(
            evaluate(&deny, Some(&call), &options())
                .expect("deny")
                .option_id,
            "reject"
        );
    }

    #[test]
    fn a_kind_the_agent_does_not_offer_falls_back_to_its_sibling() {
        let only_always = vec![PermissionOption {
            id: "always".to_owned(),
            label: "Allow always".to_owned(),
            kind: PermissionOptionKind::AllowAlways,
        }];
        let picked = option_for(PermissionOptionKind::AllowOnce, &only_always).expect("sibling");
        assert_eq!(picked.id, "always");

        assert!(
            option_for(PermissionOptionKind::RejectOnce, &only_always).is_none(),
            "a reject never falls back onto an allow"
        );
    }

    #[test]
    fn a_matched_rule_the_agent_cannot_honour_asks_the_user() {
        let policy = PermissionPolicy {
            rules: vec![rule(
                PermissionMatch::default(),
                PermissionOptionKind::RejectOnce,
                "no",
            )],
            ..PermissionPolicy::default()
        };
        let only_allow = vec![PermissionOption {
            id: "allow".to_owned(),
            label: "Allow".to_owned(),
            kind: PermissionOptionKind::AllowOnce,
        }];
        let call = call(ToolKind::Execute, "Bash", json!({}));
        assert_eq!(
            evaluate(&policy, Some(&call), &only_allow),
            None,
            "a rule that cannot be honoured must not silently allow"
        );
    }

    #[test]
    fn an_empty_matcher_matches_every_request_including_one_with_no_call() {
        assert!(matches(&PermissionMatch::default(), None));
        let call = call(ToolKind::Other, "?", json!({}));
        assert!(matches(&PermissionMatch::default(), Some(&call)));
    }

    #[test]
    fn a_matcher_with_a_field_never_matches_a_request_whose_call_is_unknown() {
        let matcher = PermissionMatch {
            tool_kind: Some("edit".to_owned()),
            ..PermissionMatch::default()
        };
        assert!(!matches(&matcher, None));
    }

    #[test]
    fn the_path_prefix_reads_locations_then_the_input() {
        let matcher = PermissionMatch {
            path_prefix: Some("/repo/src".to_owned()),
            ..PermissionMatch::default()
        };

        let mut located = call(ToolKind::Edit, "Edit", json!({ "path": "/etc/passwd" }));
        located.locations = vec![ToolLocation {
            path: "/repo/src/main.rs".to_owned(),
            line: Some(1),
        }];
        assert!(
            matches(&matcher, Some(&located)),
            "locations[0] is read before the input"
        );

        let from_input = call(
            ToolKind::Edit,
            "Edit",
            json!({ "file_path": "/repo/src/a.rs" }),
        );
        assert!(matches(&matcher, Some(&from_input)));

        let elsewhere = call(ToolKind::Edit, "Edit", json!({ "path": "/tmp/x" }));
        assert!(!matches(&matcher, Some(&elsewhere)));
    }

    #[test]
    fn the_command_prefix_reads_the_input_command() {
        let matcher = PermissionMatch {
            command_prefix: Some("git push".to_owned()),
            ..PermissionMatch::default()
        };
        let push = call(
            ToolKind::Execute,
            "Bash",
            json!({ "command": "git push origin" }),
        );
        let status = call(
            ToolKind::Execute,
            "Bash",
            json!({ "command": "git status" }),
        );
        assert!(matches(&matcher, Some(&push)));
        assert!(!matches(&matcher, Some(&status)));
    }

    #[test]
    fn every_field_of_a_matcher_must_hold() {
        let matcher = PermissionMatch {
            tool_kind: Some("execute".to_owned()),
            command_prefix: Some("rm ".to_owned()),
            ..PermissionMatch::default()
        };
        let rm = call(ToolKind::Execute, "Bash", json!({ "command": "rm -rf /" }));
        let ls = call(ToolKind::Execute, "Bash", json!({ "command": "ls" }));
        let edit = call(ToolKind::Edit, "Edit", json!({ "command": "rm -rf /" }));
        assert!(matches(&matcher, Some(&rm)));
        assert!(!matches(&matcher, Some(&ls)), "the command must match too");
        assert!(!matches(&matcher, Some(&edit)), "the kind must match too");
    }

    /// ACP v1 carries no tool name, so a `tool_name` rule matches the title (documented on
    /// [`matches`]). The test exists so the contract cannot be changed silently.
    #[test]
    fn tool_name_matches_the_title_on_this_protocol() {
        let matcher = PermissionMatch {
            tool_name: Some("Bash".to_owned()),
            ..PermissionMatch::default()
        };
        let bash = call(ToolKind::Execute, "Bash", json!({}));
        let read = call(ToolKind::Read, "Read", json!({}));
        assert!(matches(&matcher, Some(&bash)));
        assert!(!matches(&matcher, Some(&read)));
    }
}
