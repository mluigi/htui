//! MOD-26 D10: a persona narrows a step's tool exposure and permission policy, never widens them
//! (I-1). Pure: no store, no transport, no clock.

use htui_core::model::persona::SnapshotPersona;

use crate::driver::{PermissionPolicy, ToolExposure};
use crate::event::ToolKind;

/// The [`ToolKind`] spelled `text`, `None` outside the ten (`wire_enum!` has no `FromStr`).
#[must_use]
pub fn tool_kind_of(text: &str) -> Option<ToolKind> {
    ToolKind::ALL
        .iter()
        .copied()
        .find(|kind| kind.as_str() == text)
}

/// The step's exposure and policy under `persona`, from the agent row's (I-1):
///
/// - `allow`: base empty → persona's; persona empty → base's; both non-empty → `base ∩ persona`
///   in base order, every base name outside the persona's appended to `deny`, and the base list
///   kept when the intersection is empty (B-8);
/// - `deny`: base, then persona's, then B-8's additions, first occurrence kept;
/// - `deny_kinds`: base, then persona's (`tool_kind_of`, an unknown string → `Other`, B-19);
/// - `command_run`: `base && persona`;
/// - rules: persona rules (reject-only; empty reason → `persona <name>`, B-21), then one
///   `{match: {tool_kind}, answer: reject_once, reason: "persona <name> denies <kind>"}` per
///   persona `deny_kinds` entry, then the base rules;
/// - `remembered`: the base's, evaluated after every rule (`permission.rs:81-92`);
/// - `default`: the stricter of the two, `Allow < Ask < Deny`.
#[must_use]
pub fn narrow(
    base: &ToolExposure,
    policy: &PermissionPolicy,
    persona: &SnapshotPersona,
) -> (ToolExposure, PermissionPolicy) {
    // MOD-26 T2: red - the identity; the I-1 clauses land in the green commit.
    let _ = persona;
    (base.clone(), policy.clone())
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use htui_core::model::persona::{
        PersonaAnswer, PersonaDefault, PersonaMatch, PersonaPermission, PersonaRule, PersonaTools,
        SnapshotPersona, TOOL_KINDS,
    };
    use serde_json::json;

    use super::{narrow, tool_kind_of};
    use crate::driver::{
        PermissionDefault, PermissionMatch, PermissionPolicy, PermissionRule, RememberedPermission,
        ToolExposure,
    };
    use crate::event::{PermissionOption, PermissionOptionKind, ToolCallEvent, ToolKind};
    use crate::permission::{PolicyStage, evaluate};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    /// A persona called `reviewer` with `tools` and `permission`; the digest is never read here.
    fn persona(tools: PersonaTools, permission: PersonaPermission) -> SnapshotPersona {
        SnapshotPersona {
            name: "reviewer".to_owned(),
            digest: "sha256:unread".to_owned(),
            body: "You review.\n".to_owned(),
            tools,
            permission,
        }
    }

    /// A persona that changes only `tools`.
    fn tooled(tools: PersonaTools) -> SnapshotPersona {
        persona(tools, PersonaPermission::default())
    }

    /// An exposure with `command_run` on, so a case sees what a persona takes away.
    fn exposure(allow: &[&str], deny: &[&str]) -> ToolExposure {
        ToolExposure {
            allow: names(allow),
            deny: names(deny),
            command_run: true,
            deny_kinds: Vec::new(),
        }
    }

    fn allowing(allow: &[&str]) -> PersonaTools {
        PersonaTools {
            allow: names(allow),
            ..PersonaTools::default()
        }
    }

    fn denying_kinds(kinds: &[&str]) -> PersonaTools {
        PersonaTools {
            deny_kinds: names(kinds),
            ..PersonaTools::default()
        }
    }

    fn options() -> Vec<PermissionOption> {
        [
            ("allow", PermissionOptionKind::AllowOnce),
            ("allow-always", PermissionOptionKind::AllowAlways),
            ("reject", PermissionOptionKind::RejectOnce),
        ]
        .into_iter()
        .map(|(id, kind)| PermissionOption {
            id: id.to_owned(),
            label: id.to_owned(),
            kind,
        })
        .collect()
    }

    fn edit_call() -> ToolCallEvent {
        ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: "Edit".to_owned(),
            tool_kind: ToolKind::Edit,
            input: json!({ "path": "/src/lib.rs" }),
            locations: Vec::new(),
        }
    }

    /// `Allow < Ask < Deny`, the order I-1's `default` clause reads.
    fn rank(default: PermissionDefault) -> u8 {
        match default {
            PermissionDefault::Allow => 0,
            PermissionDefault::Ask => 1,
            PermissionDefault::Deny => 2,
        }
    }

    /// D2, D10: core spells the ten kinds as this crate does, in the same order — core cannot name
    /// `ToolKind`, so this is the one place the two lists meet.
    #[test]
    fn core_tool_kinds_spell_tool_kind_all() {
        let spelled: Vec<&str> = ToolKind::ALL.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(spelled, TOOL_KINDS.to_vec());
        for kind in ToolKind::ALL {
            assert_eq!(tool_kind_of(kind.as_str()), Some(*kind), "{kind}");
        }
    }

    /// I-1 `allow ⊆ base` (an empty base means "everything").
    #[test]
    fn allow_intersects_and_an_empty_base_means_everything() {
        let (open, _) = narrow(
            &exposure(&[], &[]),
            &PermissionPolicy::default(),
            &tooled(allowing(&["Read", "Grep"])),
        );
        assert_eq!(
            open.allow,
            names(&["Read", "Grep"]),
            "an empty base keeps no list"
        );
        assert!(open.deny.is_empty(), "{open:?}");

        let (kept, _) = narrow(
            &exposure(&["Read", "Bash"], &[]),
            &PermissionPolicy::default(),
            &tooled(PersonaTools::default()),
        );
        assert_eq!(
            kept.allow,
            names(&["Read", "Bash"]),
            "an empty persona list keeps the base"
        );

        let (both, _) = narrow(
            &exposure(&["Read", "Bash", "Grep"], &[]),
            &PermissionPolicy::default(),
            &tooled(allowing(&["Grep", "Read", "Edit"])),
        );
        assert_eq!(
            both.allow,
            names(&["Read", "Grep"]),
            "the intersection, in base order"
        );
        assert_eq!(
            both.deny,
            names(&["Bash"]),
            "a base name the persona omits is denied"
        );
    }

    /// I-1 `allow ⊆ base`, B-8: two disjoint lists intersect to nothing, which must not read as
    /// "no allow-list".
    #[test]
    fn disjoint_allow_lists_leave_no_tool() {
        let (narrowed, _) = narrow(
            &exposure(&["Bash"], &[]),
            &PermissionPolicy::default(),
            &tooled(allowing(&["Read"])),
        );
        assert_eq!(narrowed.allow, names(&["Bash"]), "never the empty list");
        assert!(
            narrowed.deny.contains(&"Bash".to_owned()),
            "every kept name is denied: {narrowed:?}"
        );
    }

    /// I-1 `deny ⊇ base`, `deny_kinds ⊇ base`.
    #[test]
    fn deny_and_deny_kinds_only_grow() {
        let base = ToolExposure {
            deny_kinds: vec![ToolKind::Execute],
            ..exposure(&[], &["a", "b"])
        };
        let (narrowed, _) = narrow(
            &base,
            &PermissionPolicy::default(),
            &tooled(PersonaTools {
                deny: names(&["b", "c", "c"]),
                deny_kinds: names(&["execute", "read"]),
                ..PersonaTools::default()
            }),
        );
        assert_eq!(narrowed.deny, names(&["a", "b", "c"]));
        assert_eq!(narrowed.deny_kinds, vec![ToolKind::Execute, ToolKind::Read]);
    }

    /// I-1 `command_run = base ∧ persona`.
    #[test]
    fn command_run_is_the_conjunction() {
        for (base_run, persona_run, expected) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
            (false, false, false),
        ] {
            let base = ToolExposure {
                command_run: base_run,
                ..ToolExposure::default()
            };
            let tools = PersonaTools {
                command_run: persona_run,
                ..PersonaTools::default()
            };
            let (narrowed, _) = narrow(&base, &PermissionPolicy::default(), &tooled(tools));
            assert_eq!(
                narrowed.command_run, expected,
                "base {base_run} persona {persona_run}"
            );
        }
    }

    /// I-1 "persona rules only reject and run before the base rules" (D10, B-21).
    #[test]
    fn persona_rules_run_first_then_kind_rules_then_base_rules() {
        let base_rule = PermissionRule {
            matcher: PermissionMatch {
                tool_kind: Some("read".to_owned()),
                ..PermissionMatch::default()
            },
            answer: PermissionOptionKind::AllowOnce,
            reason: "reads are safe".to_owned(),
        };
        let policy = PermissionPolicy {
            rules: vec![base_rule.clone()],
            ..PermissionPolicy::default()
        };
        let reviewer = persona(
            denying_kinds(&["edit"]),
            PersonaPermission {
                default: None,
                rules: vec![
                    PersonaRule {
                        matcher: PersonaMatch {
                            command_prefix: Some("rm".to_owned()),
                            ..PersonaMatch::default()
                        },
                        answer: PersonaAnswer::RejectAlways,
                        reason: "no deletes".to_owned(),
                    },
                    PersonaRule {
                        matcher: PersonaMatch {
                            tool_name: Some("Fetch".to_owned()),
                            path_prefix: Some("/etc".to_owned()),
                            ..PersonaMatch::default()
                        },
                        answer: PersonaAnswer::RejectOnce,
                        reason: String::new(),
                    },
                ],
            },
        );

        let (_, narrowed) = narrow(&ToolExposure::default(), &policy, &reviewer);

        assert_eq!(
            narrowed.rules,
            vec![
                PermissionRule {
                    matcher: PermissionMatch {
                        command_prefix: Some("rm".to_owned()),
                        ..PermissionMatch::default()
                    },
                    answer: PermissionOptionKind::RejectAlways,
                    reason: "no deletes".to_owned(),
                },
                PermissionRule {
                    matcher: PermissionMatch {
                        tool_name: Some("Fetch".to_owned()),
                        path_prefix: Some("/etc".to_owned()),
                        ..PermissionMatch::default()
                    },
                    answer: PermissionOptionKind::RejectOnce,
                    reason: "persona reviewer".to_owned(),
                },
                PermissionRule {
                    matcher: PermissionMatch {
                        tool_kind: Some("edit".to_owned()),
                        ..PermissionMatch::default()
                    },
                    answer: PermissionOptionKind::RejectOnce,
                    reason: "persona reviewer denies edit".to_owned(),
                },
                base_rule,
            ]
        );
    }

    /// I-1 "persona rules run before … the remembered choices": an `allow_always` the user once
    /// gave for `edit` does not survive a persona that denies the kind.
    #[test]
    fn a_remembered_allow_loses_to_a_persona_kind_reject() {
        let policy = PermissionPolicy {
            remembered: vec![RememberedPermission {
                matcher: PermissionMatch {
                    tool_kind: Some("edit".to_owned()),
                    ..PermissionMatch::default()
                },
                option_kind: PermissionOptionKind::AllowAlways,
                added_at: DateTime::from_timestamp(1_788_393_600, 0).expect("in range"),
                added_by: "user".to_owned(),
            }],
            ..PermissionPolicy::default()
        };
        let call = edit_call();
        let before = evaluate(&policy, Some(&call), &options()).expect("the base remembers");
        assert_eq!(before.stage, PolicyStage::Remembered, "the premise");

        let (_, narrowed) = narrow(
            &ToolExposure::default(),
            &policy,
            &tooled(denying_kinds(&["edit"])),
        );
        let after = evaluate(&narrowed, Some(&call), &options()).expect("a rule answers");
        assert_eq!(after.option_id, "reject");
        assert_eq!(after.kind, PermissionOptionKind::RejectOnce);
        assert_eq!(after.stage, PolicyStage::Rule);
        assert_eq!(after.reason, "persona reviewer denies edit");
        assert_eq!(
            narrowed.remembered, policy.remembered,
            "kept, and evaluated after"
        );
    }

    /// I-1 `default` = the stricter of the two (`Allow < Ask < Deny`).
    #[test]
    fn default_is_the_stricter_of_the_two() {
        use PermissionDefault::{Allow, Ask, Deny};
        for (base, own, expected) in [
            (Allow, None, Allow),
            (Allow, Some(PersonaDefault::Ask), Ask),
            (Allow, Some(PersonaDefault::Deny), Deny),
            (Ask, None, Ask),
            (Ask, Some(PersonaDefault::Ask), Ask),
            (Ask, Some(PersonaDefault::Deny), Deny),
            (Deny, None, Deny),
            (Deny, Some(PersonaDefault::Ask), Deny),
            (Deny, Some(PersonaDefault::Deny), Deny),
        ] {
            let policy = PermissionPolicy {
                default: base,
                ..PermissionPolicy::default()
            };
            let reviewer = persona(
                PersonaTools::default(),
                PersonaPermission {
                    default: own,
                    rules: Vec::new(),
                },
            );
            let (_, narrowed) = narrow(&ToolExposure::default(), &policy, &reviewer);
            assert_eq!(narrowed.default, expected, "base {base} persona {own:?}");
        }
    }

    /// I-1 `deny_kinds ⊇ base`, B-19: a kind string this build cannot read (impossible after D3)
    /// is denied as `other`, never dropped.
    #[test]
    fn an_unknown_kind_string_is_denied_as_other() {
        assert_eq!(tool_kind_of("bogus"), None);
        let (exposure, policy) = narrow(
            &ToolExposure::default(),
            &PermissionPolicy::default(),
            &tooled(denying_kinds(&["bogus"])),
        );
        assert_eq!(exposure.deny_kinds, vec![ToolKind::Other]);
        assert_eq!(
            policy.rules,
            vec![PermissionRule {
                matcher: PermissionMatch {
                    tool_kind: Some("other".to_owned()),
                    ..PermissionMatch::default()
                },
                answer: PermissionOptionKind::RejectOnce,
                reason: "persona reviewer denies other".to_owned(),
            }]
        );
    }

    /// I-1 as a whole, over a grid of bases × personas: nothing the result allows was refused by
    /// the base, and nothing the base refused is allowed.
    #[test]
    fn narrow_never_widens() {
        let base_rule = PermissionRule {
            matcher: PermissionMatch::default(),
            answer: PermissionOptionKind::AllowOnce,
            reason: "base".to_owned(),
        };
        let mut bases = Vec::new();
        for default in [
            PermissionDefault::Allow,
            PermissionDefault::Ask,
            PermissionDefault::Deny,
        ] {
            let policy = PermissionPolicy {
                default,
                rules: vec![base_rule.clone()],
                ..PermissionPolicy::default()
            };
            bases.push((ToolExposure::default(), policy.clone()));
            bases.push((
                ToolExposure {
                    deny_kinds: vec![ToolKind::Fetch],
                    ..exposure(&["Read", "Bash"], &["x"])
                },
                policy.clone(),
            ));
            bases.push((exposure(&[], &["Bash"]), policy));
        }
        let personas = [
            tooled(PersonaTools::default()),
            tooled(PersonaTools {
                command_run: false,
                ..allowing(&["Grep", "Read"])
            }),
            persona(
                PersonaTools {
                    deny: names(&["Bash", "y"]),
                    deny_kinds: names(&["edit", "bogus"]),
                    ..PersonaTools::default()
                },
                PersonaPermission {
                    default: Some(PersonaDefault::Deny),
                    rules: vec![PersonaRule {
                        matcher: PersonaMatch::default(),
                        answer: PersonaAnswer::RejectOnce,
                        reason: String::new(),
                    }],
                },
            ),
            persona(
                allowing(&["Edit"]),
                PersonaPermission {
                    default: Some(PersonaDefault::Ask),
                    rules: Vec::new(),
                },
            ),
        ];

        for (base, policy) in &bases {
            for persona in &personas {
                let (tools, narrowed) = narrow(base, policy, persona);
                let case = format!("base {base:?} {policy:?} persona {persona:?}");
                if !base.allow.is_empty() {
                    assert!(
                        tools.allow.iter().all(|name| base.allow.contains(name)),
                        "allow ⊆ base: {case}"
                    );
                }
                if !persona.tools.allow.is_empty() {
                    assert!(
                        tools.allow.iter().all(|name| {
                            persona.tools.allow.contains(name) || tools.deny.contains(name)
                        }),
                        "a name the persona omits is never usable: {case}"
                    );
                }
                assert!(
                    base.deny.iter().all(|name| tools.deny.contains(name)),
                    "deny ⊇ base: {case}"
                );
                assert!(
                    base.deny_kinds
                        .iter()
                        .all(|kind| tools.deny_kinds.contains(kind)),
                    "deny_kinds ⊇ base: {case}"
                );
                assert!(
                    !tools.command_run || base.command_run,
                    "command_run never turns on: {case}"
                );
                assert!(
                    rank(narrowed.default) >= rank(policy.default),
                    "default never loosens: {case}"
                );
                assert!(
                    narrowed.rules.ends_with(&policy.rules),
                    "the base rules run last: {case}"
                );
                assert!(
                    narrowed.rules[..narrowed.rules.len() - policy.rules.len()]
                        .iter()
                        .all(|rule| matches!(
                            rule.answer,
                            PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
                        )),
                    "every rule a persona adds rejects: {case}"
                );
                assert_eq!(narrowed.remembered, policy.remembered, "{case}");
            }
        }
    }
}
