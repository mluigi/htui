//! The transport registry: an `agent` row in, a driver out (plan MOD-2 D12, `R-AGT-5`).
//!
//! `R-AGT-5` says a new agent costs "a registry row and, at most, one stream adapter". That is a
//! statement about *keys*: this factory holds one entry per **adapter** and zero per agent, and the
//! only thing it reads to pick one is row data —
//!
//! | `agent.transport` | adapter id |
//! |---|---|
//! | `acp` | `acp` |
//! | `cli` | `cli/<settings.cli.stream>` |
//!
//! There is deliberately no method, match arm or map entry keyed on `agent.name`. An agent the
//! codebase has never heard of reaches a working session because its row names a transport that
//! already exists, and `crates/htui-agent/tests/extensibility.rs` proves it by sweeping the tree
//! for the name it uses.

use std::collections::BTreeMap;

use htui_core::model::{Agent, AgentBox, Transport};

use crate::driver::{AgentDriver, DriverCaps};
use crate::error::{DriverError, Result};
use crate::launch::AgentSettings;

/// Builds one transport's driver from a registry row.
///
/// Milestone 3 registers the ACP builder under `acp`, milestone 8 the stream adapter under
/// `cli/claude_stream_json`, and `test-support` registers the fake under `cli/fake`.
pub trait TransportBuilder: Send + Sync + core::fmt::Debug {
    /// Builds a driver for `agent`, with this box's `agent_box` row when there is one.
    ///
    /// `caps` is computed by the factory from the row, so every transport advertises the same
    /// profile for the same row rather than each deciding for itself.
    ///
    /// # Errors
    /// [`DriverError::Spawn`] or [`DriverError::Transport`] when the transport cannot be prepared;
    /// [`DriverError::Unresolved`] when the row still holds an unresolved `${tool}` placeholder.
    fn build(
        &self,
        agent: &Agent,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>>;
}

/// The adapter map. Registration happens in one place, like the tab and detail registries of
/// `crates/htui/src/ui/tabs/registry.rs`.
#[derive(Debug, Default)]
pub struct DriverFactory {
    adapters: BTreeMap<String, Box<dyn TransportBuilder>>,
}

impl DriverFactory {
    /// An empty factory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `builder` under an adapter id. A second registration under the same id replaces
    /// the first.
    pub fn register(&mut self, id: impl Into<String>, builder: Box<dyn TransportBuilder>) {
        self.adapters.insert(id.into(), builder);
    }

    /// Every registered adapter id, sorted. The count is the honest measure of what adding an
    /// agent costs: `R-AGT-5` is satisfied when this list does not grow with the registry.
    #[must_use]
    pub fn adapter_ids(&self) -> Vec<&str> {
        self.adapters.keys().map(String::as_str).collect()
    }

    /// Builds the driver `agent`'s row asks for.
    ///
    /// # Errors
    /// [`DriverError::UnknownAdapter`] naming the id derived from the row, when no builder is
    /// registered under it — including a `cli` row whose `settings.cli.stream` this build has no
    /// adapter for. Whatever the builder itself returns, otherwise.
    pub fn driver_for(
        &self,
        agent: &Agent,
        on_box: Option<&AgentBox>,
    ) -> Result<Box<dyn AgentDriver>> {
        // Parsed once and shared by both derivations: `adapter_id` and `caps_for` each read the
        // same `settings` document, and calling the public pair here would deserialise (and clone)
        // it twice per session start.
        let settings = settings(agent);
        let id = adapter_id_from(agent, &settings);
        let builder = self
            .adapters
            .get(&id)
            .ok_or(DriverError::UnknownAdapter(id))?;
        builder.build(agent, on_box, caps_from(agent, &settings))
    }
}

/// The adapter id a row asks for: `acp`, or `cli/<stream>`.
///
/// A `cli` row with no `settings.cli` block yields the bare `cli`, which no build registers — the
/// resulting [`DriverError::UnknownAdapter`] says the row is incomplete, which is more use than a
/// serde error would be.
#[must_use]
pub fn adapter_id(agent: &Agent) -> String {
    adapter_id_from(agent, &settings(agent))
}

/// [`adapter_id`] over settings the caller has already parsed.
fn adapter_id_from(agent: &Agent, settings: &AgentSettings) -> String {
    match agent.transport {
        Transport::Acp => "acp".to_owned(),
        Transport::Cli => match settings.cli.as_ref() {
            Some(cli) if !cli.stream.is_empty() => format!("cli/{}", cli.stream),
            _ => "cli".to_owned(),
        },
    }
}

/// What a row's transport can do (`docs/ANA-4.md` §4.3, §6.2).
///
/// Computed here rather than by each transport so the chat tab's capability banner and the
/// orchestrator's gate check read one profile per row, whoever built the driver.
#[must_use]
pub fn caps_for(agent: &Agent) -> DriverCaps {
    caps_from(agent, &settings(agent))
}

/// [`caps_for`] over settings the caller has already parsed.
fn caps_from(agent: &Agent, settings: &AgentSettings) -> DriverCaps {
    match agent.transport {
        Transport::Acp => DriverCaps {
            permission_requests: true,
            edit_proposals: true,
            plans: true,
            thoughts: true,
            follow_up_in_session: true,
            resume: settings.acp.session.resume,
            usage: true,
        },
        // §4.3 verbatim: "the CLI transport reports `DriverCaps { permission_requests: false,
        // edit_proposals: false, plans: false }`". The other four are not stated there and are
        // derived from §6.2's mapping table: `thinking` blocks map to `thought`, a stdin NDJSON
        // user message maps to `follow_up` (so a follow-up stays in the running process), and
        // `result.usage` maps to `usage`. `resume` is the CLI's own `--resume`.
        Transport::Cli => DriverCaps {
            permission_requests: false,
            edit_proposals: false,
            plans: false,
            thoughts: true,
            follow_up_in_session: true,
            resume: true,
            usage: true,
        },
    }
}

/// Parses `agent.settings`, falling back to the documented defaults.
///
/// A row whose settings do not parse is treated as a row with no settings rather than as a fatal
/// error: the column is `JSONB NOT NULL DEFAULT '{}'` and hand-editable, and refusing to compute
/// capabilities for it would take the Settings tab down with it. The adapter id is unaffected —
/// a `cli` row whose block is unreadable ends up as the bare `cli`, which is a named error.
fn settings(agent: &Agent) -> AgentSettings {
    serde_json::from_value(agent.settings.clone()).unwrap_or_default()
}
