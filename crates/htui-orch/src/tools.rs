//! The tool-host seam (MOD-11 D4): the engine opens one lease per session; the host behind it
//! (`htui_mcp::McpHost`) speaks MCP. `None` in `EngineParts.tools` keeps every spec as before.
//!
//! The engine never learns how the tools are served: it builds a [`ToolScope`] from what it holds
//! (never from tool arguments, MOD-11 I-1), asks a [`ToolHost`] to [`open`](ToolHost::open) it, and
//! puts the [`ToolLease`]'s [`McpServerSpec`] into the session's `SessionSpec.mcp`. Dropping the
//! lease unregisters the session's token, so a call that arrives later answers `session ended`
//! (I-6).

use std::path::PathBuf;

use htui_agent::driver::McpServerSpec;
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
            on_drop: Some(Box::new(on_drop)),
        }
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

    use htui_agent::driver::McpServerSpec;

    use super::ToolLease;

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
