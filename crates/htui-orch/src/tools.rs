//! The tool-host seam (MOD-11 D4): the engine opens one lease per session; the host behind it
//! (`htui_mcp::McpHost`) speaks MCP. `None` in `EngineParts.tools` keeps every spec as before.
//!
//! The engine never learns how the tools are served: it builds a [`ToolScope`] from what it holds
//! (never from tool arguments, MOD-11 I-1), asks a [`ToolHost`] to [`open`](ToolHost::open) it, and
//! puts the [`ToolLease`]'s [`McpServerSpec`] into the session's `SessionSpec.mcp`. Dropping the
//! lease unregisters the session's token, so a call that arrives later answers `session ended`
//! (I-6).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use htui_agent::driver::{
    DriverCaps, McpServerSpec, PermissionDefault, PermissionMatch, PermissionPolicy, PermissionRule,
};
use htui_agent::event::PermissionOptionKind;
use htui_agent::prompt_bridge::PromptPort;
use htui_core::model::{
    BoxId, EnvValue, ItemId, McpPick, McpTransport, ProjectId, RunId, SERVER_NAME,
    SnapshotMcpServer, SnapshotToolbox, StepId, Transport, UserId,
};
use htui_core::prompt::render::HostnameLine;
use htui_core::prompt::{McpChoice, McpOutcome, SkipReason};
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
    /// MOD-152 D11: the session's other trees (`SessionSpec.extra_dirs`): `command_run` and
    /// `command_start` resolve their `repo` argument against it, by final path component.
    pub extra_dirs: Vec<PathBuf>,
    /// B-18: the candidate's transport; `Cli` advertises `permission_prompt`.
    pub transport: Transport,
}

/// Why [`ToolScope::extra_dir`] named no directory (MOD-152 D11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraDirRefusal {
    /// No extra directory has this final path component; `available` lists the names there are.
    Unknown {
        /// The name asked for.
        name: String,
        /// The final path components of the session's `extra_dirs`, in order.
        available: Vec<String>,
    },
    /// Two or more extra directories share this final path component.
    Ambiguous {
        /// The name asked for.
        name: String,
    },
}

impl ToolScope {
    /// MOD-152 D11: the extra directory whose final path component is `name`.
    ///
    /// # Errors
    ///
    /// [`ExtraDirRefusal::Unknown`] when none has that name, [`ExtraDirRefusal::Ambiguous`] when
    /// more than one does.
    pub fn extra_dir(&self, name: &str) -> Result<&Path, ExtraDirRefusal> {
        let named = |dir: &&PathBuf| dir.file_name().is_some_and(|last| last == name);
        let mut found = self.extra_dirs.iter().filter(named);
        match (found.next(), found.next()) {
            (Some(dir), None) => Ok(dir.as_path()),
            (Some(_), Some(_)) => Err(ExtraDirRefusal::Ambiguous {
                name: name.to_owned(),
            }),
            (None, _) => Err(ExtraDirRefusal::Unknown {
                name: name.to_owned(),
                available: self
                    .extra_dirs
                    .iter()
                    .filter_map(|dir| dir.file_name())
                    .map(|last| last.to_string_lossy().into_owned())
                    .collect(),
            }),
        }
    }
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
/// MOD-152 D9: `command_start` runs one too and `command_cancel` ends one, so both ask;
/// `command_wait` and `command_output_read` run nothing and are pre-approved.
pub const ASKING_TOOLS: [&str; 4] = [
    "command_run",
    "command_start",
    "command_cancel",
    "permission_prompt",
];

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

/// MOD-127 M2 D9: the tool lists of one server a session launches: `allow` pre-approves, `deny`
/// removes. Bare tool names, as the server advertises them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTools {
    /// The server's name: the `<name>` of `mcp__<name>__<tool>`.
    pub server: String,
    /// Tools pre-approved for the server.
    pub allow: Vec<String>,
    /// Tools removed from the agent.
    pub deny: Vec<String>,
}

/// MOD-127 M2 D7, D10: what one step launches of the run's frozen toolbox, and the record of why.
///
/// `specs` and `choices` come from one pass ([`plan_mcp`]), so the prompt's `mcp_choices` and the
/// session's `mcp` cannot disagree. `specs` hold resolved secret **values** and are for the
/// launch only: their `Debug` redacts them, and nothing here is persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpPlan {
    /// The servers to start after htui's own, by name bytes.
    pub specs: Vec<McpServerSpec>,
    /// Every resolved server, rendered or skipped, by name bytes.
    pub choices: Vec<McpChoice>,
    /// The lists of the rendered servers, in the order of `specs`.
    pub tools: Vec<McpTools>,
}

/// MOD-127 M2 D3, D7, D10: partitions a phase's frozen picks by what the step's agent can carry
/// and by the secrets the project supplies, and builds a spec for each server that launches.
///
/// Pure: no store, no clock, no secret provider. The rules, in order, per pick:
/// - the winning attachment is `off`: skipped `off`;
/// - the server's name is htui's own: `reserved_name`;
/// - the agent carries neither transport: `agent_unsupported`; carries the other one:
///   `transport_unsupported`;
/// - a `Secret` the transport's own field names (stdio: `env`; http: `headers`; a leftover field
///   of the other transport is neither resolved nor sent) is not in `secrets`: `secret_missing`
///   with that **name** (keys are case-sensitive, as the provider's are). The first one in order.
///
/// A pick whose index is outside the toolbox is dropped: a snapshot is written whole, so none is.
/// `toolbox: None` (a run made before M2) yields no pick and an empty plan.
#[must_use]
pub fn plan_mcp(
    toolbox: Option<&SnapshotToolbox>,
    picks: &[McpPick],
    caps: DriverCaps,
    secrets: &BTreeMap<String, String>,
) -> McpPlan {
    let Some(toolbox) = toolbox else {
        return McpPlan::default();
    };
    let mut rows: Vec<(&SnapshotMcpServer, &McpPick)> = picks
        .iter()
        .filter_map(|pick| {
            toolbox
                .servers
                .get(pick.server)
                .map(|server| (server, pick))
        })
        .collect();
    rows.sort_by(|(a, _), (b, _)| a.name.as_bytes().cmp(b.name.as_bytes()));
    let mut plan = McpPlan::default();
    for (server, pick) in rows {
        let (spec, outcome) = match launch_of(server, pick, caps, secrets) {
            Ok(spec) => (Some(spec), McpOutcome::Rendered),
            Err(reason) => (None, McpOutcome::Skipped(reason)),
        };
        plan.choices.push(McpChoice {
            server: server.id,
            name: server.name.clone(),
            level: pick.level,
            activation: if pick.active {
                htui_core::model::Activation::Always
            } else {
                htui_core::model::Activation::Off
            },
            active: pick.active,
            outcome,
        });
        if let Some(spec) = spec {
            plan.tools.push(McpTools {
                server: server.name.clone(),
                allow: server.allow.clone(),
                deny: server.deny.clone(),
            });
            plan.specs.push(spec);
        }
    }
    plan
}

/// [`plan_mcp`]'s verdict for one pick: the spec to launch, or why not.
fn launch_of(
    server: &SnapshotMcpServer,
    pick: &McpPick,
    caps: DriverCaps,
    secrets: &BTreeMap<String, String>,
) -> Result<McpServerSpec, SkipReason> {
    if !pick.active {
        return Err(SkipReason::Off);
    }
    if server.name == SERVER_NAME {
        return Err(SkipReason::ReservedName);
    }
    let (carries, other) = match server.transport {
        McpTransport::Stdio => (caps.mcp_stdio, caps.mcp_http),
        McpTransport::Http => (caps.mcp_http, caps.mcp_stdio),
    };
    if !carries {
        return Err(if other {
            SkipReason::TransportUnsupported
        } else {
            SkipReason::AgentUnsupported
        });
    }
    // Only the transport's own fields are resolved and sent: a leftover `headers` on a stdio row
    // (or `env` on an http one) cannot cause `secret_missing` and reaches no spec.
    Ok(match server.transport {
        McpTransport::Stdio => McpServerSpec {
            name: server.name.clone(),
            command: server.command.clone(),
            args: server.args.clone(),
            env: substitute(&server.env, secrets)?,
            transport: McpTransport::Stdio,
            url: None,
            headers: BTreeMap::new(),
        },
        McpTransport::Http => McpServerSpec {
            name: server.name.clone(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            transport: McpTransport::Http,
            url: server.url.clone(),
            headers: substitute(&server.headers, secrets)?,
        },
    })
}

/// A key's literal, or the project secret it names; the first name `secrets` lacks is the refusal.
fn substitute(
    values: &[(String, EnvValue)],
    secrets: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, SkipReason> {
    let mut resolved = BTreeMap::new();
    for (key, value) in values {
        let text = match value {
            EnvValue::Literal(text) => text.clone(),
            EnvValue::Secret(name) => secrets
                .get(name)
                .cloned()
                .ok_or_else(|| SkipReason::SecretMissing(name.clone()))?,
        };
        resolved.insert(key.clone(), text);
    }
    Ok(resolved)
}

/// MOD-127 M2 D9: `server`'s `allow` tools pre-approved, as [`pre_approve`] does for htui's own:
/// one `allow_once` per tool, matched on `mcp__<server>__<tool>` and appended after every rule
/// the policy holds, and only under the default `ask`. A tool the server also lists in `deny` is
/// left unapproved. Empty `allow` appends nothing: the existing policy decides.
pub fn pre_approve_server(policy: &mut PermissionPolicy, server: &str, tools: &McpTools) {
    if policy.default != PermissionDefault::Ask {
        return;
    }
    policy.rules.extend(
        tools
            .allow
            .iter()
            .filter(|tool| !tools.deny.contains(tool))
            .map(|tool| PermissionRule {
                matcher: PermissionMatch {
                    tool_name: Some(qualified_tool_name(server, tool)),
                    ..PermissionMatch::default()
                },
                answer: PermissionOptionKind::AllowOnce,
                reason: format!("`{server}`'s `{tool}` tool, allowed on the server (MOD-127)"),
            }),
    );
}

/// MOD-127 M2 D9: applies `plan`'s tool lists to a session. Each rendered server's `deny` becomes
/// a `RejectOnce` rule at the **front** of `policy.rules`, ahead of every allow rule (rules run in
/// order), and its qualified name joins `deny` (`ToolExposure.deny`, which the CLI turns into
/// `--disallowedTools`); then its `allow` list is pre-approved ([`pre_approve_server`]).
/// `ToolExposure.allow` is never touched: it is a real allow-list, and an MCP `allow` must not
/// restrict.
pub fn apply_mcp_tools(policy: &mut PermissionPolicy, deny: &mut Vec<String>, plan: &McpPlan) {
    let mut rejects = Vec::new();
    for tools in &plan.tools {
        for tool in &tools.deny {
            let name = qualified_tool_name(&tools.server, tool);
            rejects.push(PermissionRule {
                matcher: PermissionMatch {
                    tool_name: Some(name.clone()),
                    ..PermissionMatch::default()
                },
                answer: PermissionOptionKind::RejectOnce,
                reason: format!(
                    "`{}`'s `{tool}` tool, denied on the server (MOD-127)",
                    tools.server
                ),
            });
            if !deny.contains(&name) {
                deny.push(name);
            }
        }
    }
    policy.rules.splice(0..0, rejects);
    for tools in &plan.tools {
        pre_approve_server(policy, &tools.server, tools);
    }
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

    use htui_agent::driver::{
        DriverCaps, McpServerSpec, PermissionMatch, PermissionPolicy, PermissionRule,
    };
    use htui_agent::event::{PermissionOption, PermissionOptionKind, ToolCallEvent, ToolKind};
    use htui_agent::permission::evaluate;
    use htui_core::model::{
        EnvValue, McpPick, McpTransport, SERVER_NAME, SnapshotMcpServer, SnapshotToolbox,
    };
    use htui_core::prompt::{McpOutcome, SkipReason};

    use std::path::{Path, PathBuf};

    use super::{
        ASKING_TOOLS, ExtraDirRefusal, McpPlan, McpTools, ToolLease, ToolScope, apply_mcp_tools,
        plan_mcp, pre_approve, pre_approve_server,
    };

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
            ..McpServerSpec::default()
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

    /// MOD-11 R1 M2, MOD-152 D9: one `allow_once` per advertised tool, named as both transports
    /// title it, appended after every rule the policy already holds; the [`ASKING_TOOLS`] get
    /// none, and neither does a tool the scope does not advertise.
    #[test]
    fn pre_approve_appends_one_allow_per_advertised_tool_but_the_asking_ones() {
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
            "command_start",
            "command_wait",
            "command_cancel",
            "command_output_read",
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
                (
                    Some("mcp__htui__command_wait"),
                    PermissionOptionKind::AllowOnce
                ),
                (
                    Some("mcp__htui__command_output_read"),
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
        for asking in ASKING_TOOLS {
            assert_eq!(
                answer(&format!("mcp__htui__{asking}")),
                None,
                "{asking} asks"
            );
        }
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

    /// MOD-152 D9: the asking set is exactly the tools that run or stop a command and the CLI's
    /// own permission channel; the two read-only command tools are pre-approved.
    #[test]
    fn the_asking_tools_are_the_command_runners_and_the_permission_channel() {
        assert_eq!(
            ASKING_TOOLS,
            [
                "command_run",
                "command_start",
                "command_cancel",
                "permission_prompt"
            ]
        );
    }

    fn scope_with(extra_dirs: &[&str]) -> ToolScope {
        use htui_core::fixtures::ids;
        use htui_core::model::Transport;
        use htui_core::prompt::render::HostnameLine;
        use htui_core::store::StepFence;
        ToolScope {
            run_id: ids::RUN_2,
            step_id: ids::STEP_R2_PRD,
            project_id: ids::PROJECT_HTUI,
            item_id: None,
            box_id: ids::BOX,
            user: ids::USER,
            fence: StepFence::Unleased,
            output_kind: None,
            hostname: HostnameLine::Omitted,
            command_queue: true,
            cwd: PathBuf::from("/work/main"),
            extra_dirs: extra_dirs.iter().map(PathBuf::from).collect(),
            transport: Transport::Acp,
        }
    }

    /// MOD-152 D11: a repo name is the directory's final path component.
    #[test]
    fn extra_dir_resolves_by_the_final_path_component() {
        let scope = scope_with(&["/work/docs", "/srv/other/api"]);
        assert_eq!(scope.extra_dir("api"), Ok(Path::new("/srv/other/api")));
        assert_eq!(scope.extra_dir("docs"), Ok(Path::new("/work/docs")));
    }

    /// MOD-152 D11: an unknown name lists the names there are; two directories with one name are
    /// refused rather than guessed.
    #[test]
    fn extra_dir_refuses_unknown_and_ambiguous_names() {
        let scope = scope_with(&["/work/docs", "/srv/a/api", "/srv/b/api"]);
        assert_eq!(
            scope.extra_dir("nope"),
            Err(ExtraDirRefusal::Unknown {
                name: "nope".to_owned(),
                available: vec!["docs".to_owned(), "api".to_owned(), "api".to_owned()],
            })
        );
        assert_eq!(
            scope.extra_dir("api"),
            Err(ExtraDirRefusal::Ambiguous {
                name: "api".to_owned()
            })
        );
        assert_eq!(scope.extra_dir("docs"), Ok(Path::new("/work/docs")));
        assert_eq!(
            scope_with(&[]).extra_dir("docs"),
            Err(ExtraDirRefusal::Unknown {
                name: "docs".to_owned(),
                available: Vec::new(),
            })
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

    // -- MOD-127 M2 T6: `plan_mcp`, `pre_approve_server`, `apply_mcp_tools` ---------------------

    const DOCS_TOKEN: &str = "sentinel-docs-token-value-0001";

    fn frozen(name: &str, transport: McpTransport) -> SnapshotMcpServer {
        SnapshotMcpServer {
            id: htui_core::model::McpServerId::new(),
            name: name.to_owned(),
            description: String::new(),
            transport,
            command: if transport == McpTransport::Stdio {
                format!("{name}-server")
            } else {
                String::new()
            },
            args: if transport == McpTransport::Stdio {
                vec!["--stdio".to_owned()]
            } else {
                Vec::new()
            },
            url: (transport == McpTransport::Http).then(|| format!("https://{name}.test/rpc")),
            headers: Vec::new(),
            env: Vec::new(),
            allow: Vec::new(),
            deny: Vec::new(),
        }
    }

    fn toolbox(servers: Vec<SnapshotMcpServer>) -> (SnapshotToolbox, Vec<McpPick>) {
        let picks = (0..servers.len())
            .map(|server| McpPick {
                server,
                level: htui_core::model::BindingLevel::Project,
                active: true,
            })
            .collect();
        (SnapshotToolbox { servers }, picks)
    }

    fn caps(stdio: bool, http: bool) -> DriverCaps {
        DriverCaps {
            mcp_stdio: stdio,
            mcp_http: http,
            ..DriverCaps::default()
        }
    }

    fn names(plan: &McpPlan) -> Vec<&str> {
        plan.specs.iter().map(|spec| spec.name.as_str()).collect()
    }

    fn outcomes(plan: &McpPlan) -> Vec<(&str, McpOutcome)> {
        plan.choices
            .iter()
            .map(|choice| (choice.name.as_str(), choice.outcome.clone()))
            .collect()
    }

    #[test]
    fn renders_servers_by_name_bytes() {
        let (toolbox, picks) = toolbox(vec![
            frozen("zulu", McpTransport::Stdio),
            frozen("alpha", McpTransport::Stdio),
            frozen("Mid", McpTransport::Stdio),
        ]);
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), &BTreeMap::new());
        assert_eq!(names(&plan), ["Mid", "alpha", "zulu"], "byte order");
        assert!(
            plan.choices
                .iter()
                .all(|choice| choice.outcome == McpOutcome::Rendered)
        );
        let alpha = &plan.specs[1];
        assert_eq!(
            (alpha.command.as_str(), alpha.args.as_slice()),
            ("alpha-server", ["--stdio".to_owned()].as_slice())
        );
        assert_eq!(alpha.transport, McpTransport::Stdio);
    }

    #[test]
    fn an_off_server_is_listed_not_rendered() {
        let (toolbox, mut picks) = toolbox(vec![
            frozen("docs", McpTransport::Stdio),
            frozen("wiki", McpTransport::Stdio),
        ]);
        picks[0].active = false;
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), &BTreeMap::new());
        assert_eq!(names(&plan), ["wiki"]);
        assert_eq!(
            outcomes(&plan),
            [
                ("docs", McpOutcome::Skipped(SkipReason::Off)),
                ("wiki", McpOutcome::Rendered)
            ]
        );
        assert!(!plan.choices[0].active);
        assert_eq!(
            plan.choices[0].activation,
            htui_core::model::Activation::Off
        );
    }

    #[test]
    fn a_cli_agent_gets_http() {
        let (toolbox, picks) = toolbox(vec![frozen("web", McpTransport::Http)]);
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), &BTreeMap::new());
        let spec = &plan.specs[0];
        assert_eq!(spec.transport, McpTransport::Http);
        assert_eq!(spec.url.as_deref(), Some("https://web.test/rpc"));
        assert_eq!(spec.command, "");
    }

    #[test]
    fn an_acp_agent_skips_http_with_a_reason() {
        let (toolbox, picks) = toolbox(vec![
            frozen("local", McpTransport::Stdio),
            frozen("web", McpTransport::Http),
        ]);
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, false), &BTreeMap::new());
        assert_eq!(names(&plan), ["local"]);
        assert_eq!(
            outcomes(&plan),
            [
                ("local", McpOutcome::Rendered),
                ("web", McpOutcome::Skipped(SkipReason::TransportUnsupported))
            ]
        );
    }

    #[test]
    fn an_unsupported_agent_skips_every_server_and_the_run_starts() {
        let (toolbox, picks) = toolbox(vec![
            frozen("local", McpTransport::Stdio),
            frozen("web", McpTransport::Http),
        ]);
        let plan = plan_mcp(Some(&toolbox), &picks, caps(false, false), &BTreeMap::new());
        assert!(plan.specs.is_empty() && plan.tools.is_empty());
        assert_eq!(
            outcomes(&plan),
            [
                ("local", McpOutcome::Skipped(SkipReason::AgentUnsupported)),
                ("web", McpOutcome::Skipped(SkipReason::AgentUnsupported))
            ]
        );
    }

    #[test]
    fn a_missing_secret_skips_that_server_only() {
        let mut docs = frozen("docs", McpTransport::Stdio);
        docs.env = vec![("API".to_owned(), EnvValue::Secret("DOCS_TOKEN".to_owned()))];
        let mut web = frozen("web", McpTransport::Http);
        web.headers = vec![(
            "X-Api".to_owned(),
            EnvValue::Secret("docs_token".to_owned()),
        )];
        let (toolbox, picks) = toolbox(vec![docs, frozen("plain", McpTransport::Stdio), web]);
        // The provider supplies `DOCS_TOKEN` only: names are case-sensitive.
        let secrets = BTreeMap::from([("DOCS_TOKEN".to_owned(), DOCS_TOKEN.to_owned())]);
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), &secrets);
        assert_eq!(names(&plan), ["docs", "plain"]);
        assert_eq!(
            outcomes(&plan)[2],
            (
                "web",
                McpOutcome::Skipped(SkipReason::SecretMissing("docs_token".to_owned()))
            )
        );
        // An empty map (a provider-less project) skips every secret-bearing server.
        let bare = plan_mcp(Some(&toolbox), &picks, caps(true, true), &BTreeMap::new());
        assert_eq!(names(&bare), ["plain"]);
    }

    /// R2: a server resolves its own transport's fields only. A leftover `headers` on a stdio
    /// row, or a leftover `env` on an http row, is neither resolved nor sent, so it cannot cause
    /// `secret_missing` and cannot put a value in a spec that has no use for it.
    #[test]
    fn a_leftover_field_of_the_other_transport_is_neither_resolved_nor_sent() {
        let mut stdio = frozen("local", McpTransport::Stdio);
        stdio.headers = vec![(
            "Authorization".to_owned(),
            EnvValue::Secret("LEFTOVER_HEADER".to_owned()),
        )];
        stdio.env = vec![("MODE".to_owned(), EnvValue::Literal("fast".to_owned()))];
        let mut http = frozen("web", McpTransport::Http);
        http.env = vec![(
            "API".to_owned(),
            EnvValue::Secret("LEFTOVER_ENV".to_owned()),
        )];
        http.headers = vec![("X-Mode".to_owned(), EnvValue::Literal("fast".to_owned()))];
        let (toolbox, picks) = toolbox(vec![stdio, http]);
        // Neither leftover secret is supplied; a value is supplied under both names to show it is
        // never read either.
        let secrets = BTreeMap::from([
            ("LEFTOVER_HEADER".to_owned(), DOCS_TOKEN.to_owned()),
            ("LEFTOVER_ENV".to_owned(), DOCS_TOKEN.to_owned()),
        ]);
        for secrets in [&BTreeMap::new(), &secrets] {
            let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), secrets);
            assert_eq!(
                outcomes(&plan),
                [
                    ("local", McpOutcome::Rendered),
                    ("web", McpOutcome::Rendered)
                ]
            );
            let (local, web) = (&plan.specs[0], &plan.specs[1]);
            assert!(local.headers.is_empty(), "stdio sends no headers");
            assert_eq!(local.env.get("MODE").map(String::as_str), Some("fast"));
            assert!(web.env.is_empty(), "http sends no env");
            assert_eq!(web.headers.get("X-Mode").map(String::as_str), Some("fast"));
            assert!(!format!("{plan:?}").contains(DOCS_TOKEN));
        }
    }

    #[test]
    fn a_missing_secret_in_the_transports_own_field_still_skips() {
        let mut stdio = frozen("local", McpTransport::Stdio);
        stdio.env = vec![("API".to_owned(), EnvValue::Secret("GONE_ENV".to_owned()))];
        let mut http = frozen("web", McpTransport::Http);
        http.headers = vec![(
            "Authorization".to_owned(),
            EnvValue::Secret("GONE_HEADER".to_owned()),
        )];
        let (toolbox, picks) = toolbox(vec![stdio, http]);
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), &BTreeMap::new());
        assert_eq!(
            outcomes(&plan),
            [
                (
                    "local",
                    McpOutcome::Skipped(SkipReason::SecretMissing("GONE_ENV".to_owned()))
                ),
                (
                    "web",
                    McpOutcome::Skipped(SkipReason::SecretMissing("GONE_HEADER".to_owned()))
                )
            ]
        );
    }

    #[test]
    fn a_secret_value_reaches_the_spec_and_no_record() {
        let mut docs = frozen("docs", McpTransport::Stdio);
        docs.env = vec![
            ("API".to_owned(), EnvValue::Secret("DOCS_TOKEN".to_owned())),
            ("MODE".to_owned(), EnvValue::Literal("fast".to_owned())),
        ];
        let mut web = frozen("web", McpTransport::Http);
        web.headers = vec![(
            "Authorization".to_owned(),
            EnvValue::Secret("DOCS_TOKEN".to_owned()),
        )];
        let (toolbox, picks) = toolbox(vec![docs, web]);
        let secrets = BTreeMap::from([("DOCS_TOKEN".to_owned(), DOCS_TOKEN.to_owned())]);
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), &secrets);

        assert_eq!(
            plan.specs[0].env,
            BTreeMap::from([
                ("API".to_owned(), DOCS_TOKEN.to_owned()),
                ("MODE".to_owned(), "fast".to_owned())
            ])
        );
        assert_eq!(
            plan.specs[1]
                .headers
                .get("Authorization")
                .map(String::as_str),
            Some(DOCS_TOKEN)
        );
        // The record, the tool lists and every Debug leave the value out.
        let record = serde_json::to_string(&plan.choices).expect("encodes");
        assert!(!record.contains(DOCS_TOKEN), "{record}");
        assert!(!format!("{plan:?}").contains(DOCS_TOKEN), "{plan:?}");
        assert!(
            !serde_json::to_string(&toolbox.servers)
                .expect("encodes")
                .contains(DOCS_TOKEN),
            "the frozen set holds names only"
        );
    }

    #[test]
    fn a_reserved_name_is_dropped() {
        let (toolbox, picks) = toolbox(vec![
            frozen(SERVER_NAME, McpTransport::Stdio),
            frozen("docs", McpTransport::Stdio),
        ]);
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), &BTreeMap::new());
        assert_eq!(names(&plan), ["docs"]);
        assert_eq!(
            outcomes(&plan)[1],
            (SERVER_NAME, McpOutcome::Skipped(SkipReason::ReservedName))
        );
    }

    #[test]
    fn a_snapshot_without_a_toolbox_plans_nothing() {
        let (_, picks) = toolbox(vec![frozen("docs", McpTransport::Stdio)]);
        assert_eq!(
            plan_mcp(None, &picks, caps(true, true), &BTreeMap::new()),
            McpPlan::default()
        );
    }

    #[test]
    fn allow_pre_approves_only_under_ask_and_skips_denied_tools() {
        let tools = McpTools {
            server: "docs".to_owned(),
            allow: vec!["search".to_owned(), "drop".to_owned()],
            deny: vec!["drop".to_owned()],
        };
        let mut ask = PermissionPolicy::default();
        pre_approve_server(&mut ask, "docs", &tools);
        let answer = |policy: &PermissionPolicy, title: &str| {
            evaluate(policy, Some(&call(title)), &options()).map(|answer| answer.kind)
        };
        assert_eq!(
            answer(&ask, "mcp__docs__search"),
            Some(PermissionOptionKind::AllowOnce)
        );
        assert_eq!(answer(&ask, "mcp__docs__drop"), None);
        assert_eq!(
            answer(&ask, "mcp__docs__other"),
            None,
            "empty means unlisted asks"
        );

        let mut allow_all = PermissionPolicy {
            default: htui_agent::driver::PermissionDefault::Deny,
            ..PermissionPolicy::default()
        };
        pre_approve_server(&mut allow_all, "docs", &tools);
        assert!(allow_all.rules.is_empty(), "never widens a non-ask default");
    }

    #[test]
    fn deny_rules_precede_allow_rules() {
        let (mut toolbox, picks) = toolbox(vec![frozen("docs", McpTransport::Stdio)]);
        toolbox.servers[0].allow = vec!["search".to_owned(), "wipe".to_owned()];
        toolbox.servers[0].deny = vec!["wipe".to_owned()];
        let plan = plan_mcp(Some(&toolbox), &picks, caps(true, true), &BTreeMap::new());
        let operator = PermissionRule {
            matcher: PermissionMatch {
                tool_name: Some("mcp__docs__wipe".to_owned()),
                ..PermissionMatch::default()
            },
            answer: PermissionOptionKind::AllowOnce,
            reason: "a persona's allow".to_owned(),
        };
        let mut policy = PermissionPolicy {
            rules: vec![operator],
            ..PermissionPolicy::default()
        };
        let mut deny = vec!["Write".to_owned()];
        apply_mcp_tools(&mut policy, &mut deny, &plan);

        assert_eq!(deny, ["Write", "mcp__docs__wipe"]);
        assert_eq!(
            policy.rules[0].answer,
            PermissionOptionKind::RejectOnce,
            "the reject is the first rule"
        );
        let answer = |title: &str| {
            evaluate(&policy, Some(&call(title)), &options()).map(|answer| answer.kind)
        };
        assert_eq!(
            answer("mcp__docs__wipe"),
            Some(PermissionOptionKind::RejectOnce),
            "ahead of an earlier allow too"
        );
        assert_eq!(
            answer("mcp__docs__search"),
            Some(PermissionOptionKind::AllowOnce)
        );
        // `deny` is never an allow-list.
        let again = {
            let mut twice = deny.clone();
            apply_mcp_tools(&mut PermissionPolicy::default(), &mut twice, &plan);
            twice
        };
        assert_eq!(again, deny, "a name is listed once");
    }
}
