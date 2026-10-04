//! The tool-host seam (MOD-11 D4): the engine opens one lease per session; the host behind it
//! (`htui_mcp::McpHost`) speaks MCP. `None` in `EngineParts.tools` keeps every spec as before.
//!
//! The engine never learns how the tools are served: it builds a [`ToolScope`] from what it holds
//! (never from tool arguments, MOD-11 I-1), asks a [`ToolHost`] to [`open`](ToolHost::open) it, and
//! puts the [`ToolLease`]'s [`McpServerSpec`] into the session's `SessionSpec.mcp`. Dropping the
//! lease unregisters the session's token, so a call that arrives later answers `session ended`
//! (I-6).

use std::path::PathBuf;

use htui_agent::driver::{
    McpServerSpec, PermissionDefault, PermissionMatch, PermissionPolicy, PermissionRule,
};
use htui_agent::event::PermissionOptionKind;
use htui_agent::prompt_bridge::PromptPort;
use htui_core::model::{BoxId, ItemId, ProjectId, RunId, StepId, Transport, UserId};
use htui_core::prompt::render::HostnameLine;
use htui_core::store::StepFence;

/// Everything a session's tools are scoped to. Built by the engine or the chat runtime; never
/// from tool arguments (I-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolScope {
    /// The run the session works for.
    pub run_id: RunId,
    /// The step the session is: every write names it.
    pub step_id: StepId,
    /// The run's project: the only project an item key resolves in.
    pub project_id: ProjectId,
    /// `run.item_id`; `None` for a fresh chat (OQ-8).
    pub item_id: Option<ItemId>,
    /// The box the session runs on: `box_profile`'s subject.
    pub box_id: BoxId,
    /// Who the writes are attributed to.
    pub user: UserId,
    /// The lease every write is made under (I-3).
    pub fence: StepFence,
    /// The phase's `output_kind` when non-empty; `None` withholds `document_write` (PRD OQ-5).
    pub output_kind: Option<String>,
    /// `Shown` or `Omitted`, from `settings::resolve_box_hostname` (PRD OQ-3).
    pub hostname: HostnameLine,
    /// D16's resolved exposure; T6 sets `false`, T8 the resolver.
    pub command_queue: bool,
    /// The session's working directory: `command_run`'s cwd root (OQ-7).
    pub cwd: PathBuf,
    /// B-18: the candidate's transport; `Cli` advertises `permission_prompt`.
    pub transport: Transport,
}

/// The unregister a [`ToolLease`] runs when it drops.
type OnDrop = Box<dyn FnOnce() + Send + Sync>;

/// One registration. Dropping it unregisters the token; later calls answer `session ended`.
pub struct ToolLease {
    /// The server the agent is handed: `SessionSpec.mcp`'s one entry.
    pub spec: McpServerSpec,
    /// B-21: `Some` for a `Transport::Cli` scope.
    pub prompt: Option<PromptPort>,
    /// MOD-11 R1 M2: the tools the scope is advertised, by their bare names, in `tools/list`
    /// order — what [`pre_approve`] reads. Empty unless the host says ([`ToolLease::with_tools`]).
    pub tools: Vec<String>,
    on_drop: Option<OnDrop>,
}

impl ToolLease {
    /// A lease that runs `on_drop` exactly once, when it is dropped.
    #[must_use]
    pub fn new(
        spec: McpServerSpec,
        prompt: Option<PromptPort>,
        on_drop: impl FnOnce() + Send + Sync + 'static,
    ) -> Self {
        Self {
            spec,
            prompt,
            tools: Vec::new(),
            on_drop: Some(Box::new(on_drop)),
        }
    }

    /// MOD-11 R1 M2: this lease, naming `tools` as the ones its scope is advertised.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools = tools;
        self
    }
}

/// MOD-11 R1 M2: htui's tools that are never pre-approved. `command_run` runs any command, so under
/// the default `ask` a human sees it; `permission_prompt` is `claude-cli`'s own permission channel.
pub const ASKING_TOOLS: [&str; 2] = ["command_run", "permission_prompt"];

/// MOD-11 R1 M2: `tool` of the MCP server `server` as an agent calls it — and as both transports
/// title the call a permission request names: `mcp__<server>__<tool>`.
#[must_use]
pub fn qualified_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// MOD-11 R1 M2: appends to `policy.rules` one `allow_once` per tool `lease` is advertised, except
/// [`ASKING_TOOLS`], matched on the tool's qualified name under the lease's server (whose name the
/// host sets from `htui_mcp::SERVER_NAME`).
///
/// Appended, so they run after every rule already there — the R-MCP-4 denials, a persona's rules
/// and the agent's own — and just ahead of the remembered choices and the default: they only
/// replace the default `ask`, never widen a persona (MOD-26 I-1: a `deny_kinds: [other]` reject
/// still wins on ACP, where htui's tools are kind `other`) nor override an operator's rule. A
/// policy whose default is not `ask` gets none: under `deny` (a persona's or the agent's) they
/// would widen it, and under `allow` the default already answers. Both the engine's `drive_once`
/// and the chat path call this once the lease is open — `drive_once` after the persona's narrowing
/// has set the default.
pub fn pre_approve(policy: &mut PermissionPolicy, lease: &ToolLease) {
    if policy.default != PermissionDefault::Ask {
        return;
    }
    let server = lease.spec.name.as_str();
    policy.rules.extend(
        lease
            .tools
            .iter()
            .filter(|tool| !ASKING_TOOLS.contains(&tool.as_str()))
            .map(|tool| PermissionRule {
                matcher: PermissionMatch {
                    tool_name: Some(qualified_tool_name(server, tool)),
                    ..PermissionMatch::default()
                },
                answer: PermissionOptionKind::AllowOnce,
                reason: format!("htui's own `{tool}` tool (MOD-11)"),
            }),
    );
}

impl Drop for ToolLease {
    fn drop(&mut self) {
        if let Some(unregister) = self.on_drop.take() {
            unregister();
        }
    }
}

impl core::fmt::Debug for ToolLease {
    /// The spec's own `Debug` redacts every env value, so the token never prints (I-6).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolLease")
            .field("spec", &self.spec)
            .field("prompt", &self.prompt.as_ref().map(PromptPort::id))
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}

/// Why a lease could not be opened. The engine fails the step as a driver error (D10).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolHostError {
    /// The host has no store to write to (the TUI's offline backend).
    #[error("htui's tools cannot be hosted offline: no store to write to")]
    Offline,
    /// The listener could not bind, or its directory is not private.
    #[error("htui's MCP listener could not start: {0}")]
    Listener(String),
}

/// Hosts htui's tools for one session at a time. Object-safe, `Send + Sync`.
pub trait ToolHost: Send + Sync + core::fmt::Debug {
    /// Registers `scope` and answers the lease that serves it.
    ///
    /// # Errors
    ///
    /// [`ToolHostError`].
    fn open(&self, scope: ToolScope) -> Result<ToolLease, ToolHostError>;

    /// B-19: stops the listener, removes the socket and its directory, ends every session.
    fn close(&self);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use htui_agent::driver::{McpServerSpec, PermissionMatch, PermissionPolicy, PermissionRule};
    use htui_agent::event::{PermissionOption, PermissionOptionKind, ToolCallEvent, ToolKind};
    use htui_agent::permission::evaluate;

    use super::{ToolLease, pre_approve};

    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn spec() -> McpServerSpec {
        McpServerSpec {
            name: "htui".to_owned(),
            command: "/proc/1/exe".to_owned(),
            args: vec!["mcp".to_owned()],
            env: BTreeMap::from([
                (
                    "HTUI_MCP_ADDR".to_owned(),
                    "/run/user/1/htui-mcp-1-abcd/s".to_owned(),
                ),
                ("HTUI_MCP_TOKEN".to_owned(), SECRET.to_owned()),
            ]),
        }
    }

    #[test]
    fn dropping_a_lease_runs_its_unregister_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let lease = ToolLease::new(spec(), None, move || {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "nothing runs before the drop"
        );
        drop(lease);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the drop runs the unregister once"
        );
    }

    fn advertising(tools: &[&str]) -> ToolLease {
        ToolLease::new(spec(), None, || {})
            .with_tools(tools.iter().map(|tool| (*tool).to_owned()).collect())
    }

    fn call(title: &str) -> ToolCallEvent {
        ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: title.to_owned(),
            tool_kind: ToolKind::Other,
            input: serde_json::json!({}),
            locations: Vec::new(),
        }
    }

    fn options() -> Vec<PermissionOption> {
        [
            ("allow-once", PermissionOptionKind::AllowOnce),
            ("reject-once", PermissionOptionKind::RejectOnce),
        ]
        .into_iter()
        .map(|(id, kind)| PermissionOption {
            id: id.to_owned(),
            label: id.to_owned(),
            kind,
        })
        .collect()
    }

    /// MOD-11 R1 M2: one `allow_once` per advertised tool, named as both transports title it,
    /// appended after every rule the policy already holds; `command_run` and `permission_prompt`
    /// get none, and neither does a tool the scope does not advertise.
    #[test]
    fn pre_approve_appends_one_allow_per_advertised_tool_but_the_asking_two() {
        let operator = PermissionRule {
            matcher: PermissionMatch {
                tool_name: Some("Write".to_owned()),
                ..PermissionMatch::default()
            },
            answer: PermissionOptionKind::RejectOnce,
            reason: "the operator's".to_owned(),
        };
        let mut policy = PermissionPolicy {
            rules: vec![operator.clone()],
            ..PermissionPolicy::default()
        };
        let lease = advertising(&[
            "box_profile",
            "document_write",
            "command_run",
            "permission_prompt",
        ]);
        pre_approve(&mut policy, &lease);

        assert_eq!(
            policy.rules.first(),
            Some(&operator),
            "the operator's rule stays first"
        );
        let named: Vec<(Option<&str>, PermissionOptionKind)> = policy.rules[1..]
            .iter()
            .map(|rule| (rule.matcher.tool_name.as_deref(), rule.answer))
            .collect();
        assert_eq!(
            named,
            [
                (
                    Some("mcp__htui__box_profile"),
                    PermissionOptionKind::AllowOnce
                ),
                (
                    Some("mcp__htui__document_write"),
                    PermissionOptionKind::AllowOnce
                ),
            ]
        );
        assert!(
            policy.rules[1..].iter().all(|rule| rule.matcher
                == PermissionMatch {
                    tool_name: rule.matcher.tool_name.clone(),
                    ..PermissionMatch::default()
                }),
            "a name rule only"
        );

        let answer = |title: &str| evaluate(&policy, Some(&call(title)), &options());
        assert_eq!(
            answer("mcp__htui__document_write").map(|answer| answer.kind),
            Some(PermissionOptionKind::AllowOnce)
        );
        assert_eq!(answer("mcp__htui__command_run"), None, "command_run asks");
        assert_eq!(
            answer("mcp__htui__note_add"),
            None,
            "an unadvertised tool asks"
        );
        assert_eq!(
            answer("mcp__other__document_write"),
            None,
            "another server's asks"
        );
    }

    /// MOD-11 R1 M2, MOD-26 I-1: a rule already in the policy — a persona's `other` reject, an
    /// operator's — wins over the pre-approval, which only replaces the default `ask`.
    #[test]
    fn a_persona_other_reject_still_wins() {
        let mut policy = PermissionPolicy {
            rules: vec![PermissionRule {
                matcher: PermissionMatch {
                    tool_kind: Some("other".to_owned()),
                    ..PermissionMatch::default()
                },
                answer: PermissionOptionKind::RejectOnce,
                reason: "persona reviewer denies other".to_owned(),
            }],
            ..PermissionPolicy::default()
        };
        pre_approve(&mut policy, &advertising(&["document_write"]));
        let answer = evaluate(
            &policy,
            Some(&call("mcp__htui__document_write")),
            &options(),
        )
        .expect("a rule answers");
        assert_eq!(answer.kind, PermissionOptionKind::RejectOnce);
        assert_eq!(answer.reason, "persona reviewer denies other");
    }

    /// MOD-11 R1 M2: a policy whose default is not `ask` gets no pre-approval. Under `deny` — a
    /// persona's `permission-default: deny`, an agent row's — htui's tools stay rejected by the
    /// default (MOD-26 I-1: never widened); under `allow` the default already allows them.
    #[test]
    fn a_default_that_does_not_ask_gets_no_pre_approval() {
        use htui_agent::driver::PermissionDefault;
        use htui_agent::permission::PolicyStage;

        for (default, kind) in [
            (PermissionDefault::Deny, PermissionOptionKind::RejectOnce),
            (PermissionDefault::Allow, PermissionOptionKind::AllowOnce),
        ] {
            let mut policy = PermissionPolicy {
                default,
                ..PermissionPolicy::default()
            };
            pre_approve(&mut policy, &advertising(&["document_write", "note_add"]));
            assert!(policy.rules.is_empty(), "{default:?}: {:?}", policy.rules);
            let answer = evaluate(
                &policy,
                Some(&call("mcp__htui__document_write")),
                &options(),
            )
            .expect("the default answers");
            assert_eq!(answer.kind, kind, "{default:?}");
            assert_eq!(answer.stage, PolicyStage::Default, "{default:?}");
        }
    }

    #[test]
    fn lease_debug_prints_no_env_value() {
        let (port, _ask) = htui_agent::prompt_bridge::bridge();
        let id = port.id();
        let lease = ToolLease::new(spec(), Some(port), || {});
        let debug = format!("{lease:?}");
        assert!(!debug.contains(SECRET), "the token leaked: {debug}");
        assert!(
            debug.contains("HTUI_MCP_TOKEN"),
            "the key stays visible: {debug}"
        );
        assert!(
            debug.contains(&id.to_string()),
            "the prompt id is shown: {debug}"
        );
    }
}
