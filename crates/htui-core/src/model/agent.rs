//! Agents and their per-box enablement (`docs/ANA-9.md` §5.7).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::{AgentId, BoxId};

str_enum!(
    /// `agent.transport` (§5.7): how `htui` talks to the agent.
    Transport {
        /// Agent Client Protocol.
        Acp => "acp",
        /// Command-line fallback adapter.
        Cli => "cli",
    }
);

str_enum!(
    /// `agent.billing` (§5.7).
    Billing {
        /// Flat subscription.
        Subscription => "subscription",
        /// Billed per token.
        PerToken => "per_token",
    }
);

/// A row of `agent` (§5.7): a coding agent `htui` can drive (`R-AGT-4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    /// `agent.id`.
    pub id: AgentId,
    /// `agent.name`, unique, e.g. `claude`.
    pub name: String,
    /// `agent.transport`.
    pub transport: Transport,
    /// `agent.launch` (`JSONB`): `{argv, env}`, shape owned by ANA-4.
    pub launch: Value,
    /// `agent.models`.
    pub models: Vec<String>,
    /// `agent.default_model`.
    pub default_model: Option<String>,
    /// `agent.billing`.
    pub billing: Billing,
    /// `agent.enabled`.
    pub enabled: bool,
    /// `agent.settings` (`JSONB`): adapter-specific, owned by ANA-4.
    pub settings: Value,
    /// `agent.created_at`.
    pub created_at: DateTime<Utc>,
    /// `agent.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `agent_box` (§5.7): per-box enablement, discovery and quota snapshot (`R-AGT-6`,
/// `R-AGT-7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentBox {
    /// `agent_box.agent_id`.
    pub agent_id: AgentId,
    /// `agent_box.box_id`.
    pub box_id: BoxId,
    /// `agent_box.enabled`.
    pub enabled: bool,
    /// `agent_box.version` as reported by the installed agent.
    pub version: Option<String>,
    /// `agent_box.path` of the executable on this box.
    pub path: Option<String>,
    /// `agent_box.probed_at`.
    pub probed_at: Option<DateTime<Utc>>,
    /// `agent_box.quota` (`JSONB`) as reported by the agent.
    pub quota: Option<Value>,
    /// `agent_box.quota_at`.
    pub quota_at: Option<DateTime<Utc>>,
    /// `agent_box.updated_at`.
    pub updated_at: DateTime<Utc>,
}
