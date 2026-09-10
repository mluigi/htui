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
    /// `agent.launch` (`JSONB`): `{command, args, env, discovery?}`, shape owned by
    /// `docs/ANA-4.md` §5.1 and typed by `htui_agent::launch::AgentLaunch`.
    ///
    /// `command` and every `args` element and `env` **value** may hold a `${tool}` placeholder,
    /// resolved per box from `agent_box.probe.tools`; a row with no placeholder needs no probe.
    /// Held here as a [`Value`] because `htui-core` does not depend on the driver crate — the
    /// column's shape is ANA-4's, and this crate only stores it.
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
    /// `agent_box.quota` (`JSONB`): the `docs/ANA-4.md` §7 allowance document, as last observed.
    ///
    /// **Single-writer** since MOD-2 milestone 7 (plan D74):
    /// `WriteStore::set_agent_box_quota` is the only path that writes this column.
    /// `upsert_agent_box` can neither set nor clear it, so carrying a value here into an upsert is
    /// accepted and ignored rather than written - the probe reads a row and rewrites it seconds
    /// later, and a latch that landed in between must survive that.
    pub quota: Option<Value>,
    /// `agent_box.quota_at`: when [`AgentBox::quota`] was observed. Written by the same single
    /// writer, and by nothing else (plan D74).
    pub quota_at: Option<DateTime<Utc>>,
    /// `agent_box.updated_at`.
    pub updated_at: DateTime<Utc>,
    /// `agent_box.probe` (`JSONB`, migration `0002`): the `docs/ANA-4.md` §4.6 snapshot of what
    /// this box can run - `{transport, resolved, tools, handshake, status, stderr_tail, source}`
    /// - typed by `htui_agent::probe::ProbeSnapshot` (MOD-2 D44).
    ///
    /// Held here as a [`Value`] for the reason [`Agent::launch`] gives: `htui-core` does not
    /// depend on the driver crate, and neither does `htui-store`. MOD-4's skip predicate reads
    /// `probe->>'status'` in SQL rather than matching a Rust enum (`docs/ANA-2.md` §7).
    ///
    /// `#[serde(default)]` so a document written before this column existed still deserialises.
    #[serde(default)]
    pub probe: Option<Value>,
}

/// The two `agent` rows MOD-2 seeds (`docs/ANA-4.md` §5.3), stamped with `now`.
///
/// One source, two JSON documents under `crates/htui-core/seeds/`, compiled in with
/// `include_str!`. `PgStore::seed_if_empty_as` inserts them when the `agent` table is empty, and
/// `fixtures::agents()` is these same rows re-stamped with the fixture's deterministic ids and
/// epoch — so the demo data and the real seed cannot drift, and a fixture that cannot launch stops
/// being a trap for MOD-2's own tests.
///
/// Both rows are `transport: acp`, `billing: subscription`. `claude`'s `models` stays **empty**
/// with no `default_model`: ACP delivers the model list as session config options at
/// `session/new`, so a guessed list would go stale on every vendor release and the first
/// successful handshake fills it (§5.3's three notes).
///
/// `agy`'s list is **not** a guess and is therefore seeded: MOD-2 `T34` read it off a live
/// `session/new` on 2026-09-10 (plan D64), together with the `configOptions` entry id — `"model"`
/// — that `settings.acp.model_config_id` needs before §4.4's selection-by-id can name anything,
/// and the `currentValue` that installation was defaulting to. It stays **data**: no source file
/// reads a model string, and `tests/extensibility.rs`'s vendor sweep is what keeps that true. An
/// adapter release that renames a model degrades visibly rather than silently — `open_session`
/// step 4 emits a `model_unavailable` `other` row and the turn proceeds on the adapter's own
/// default — so a stale list costs a row in the log, never a failed session.
///
/// # Panics
/// Never in a shipped build: the two documents are compile-time constants and a unit test parses
/// both, so a malformed seed fails the suite rather than a session.
#[must_use]
pub fn seed_rows(now: DateTime<Utc>) -> Vec<Agent> {
    [
        include_str!("../../seeds/agent_claude.json"),
        include_str!("../../seeds/agent_agy.json"),
    ]
    .into_iter()
    .map(|document| {
        let seed: AgentSeed =
            serde_json::from_str(document).expect("a compiled-in seed document parses");
        Agent {
            id: AgentId::new(),
            name: seed.name,
            transport: seed.transport,
            launch: seed.launch,
            models: seed.models,
            default_model: seed.default_model,
            billing: seed.billing,
            enabled: true,
            settings: seed.settings,
            created_at: now,
            updated_at: now,
        }
    })
    .collect()
}

/// One `crates/htui-core/seeds/*.json` document: an [`Agent`] minus the four fields the *inserter*
/// owns (`id`, `enabled`, `created_at`, `updated_at`).
///
/// A separate type rather than `#[serde(default)]` on `Agent` so a seed document cannot quietly
/// carry an id or a timestamp: those come from the box doing the seeding, never from the file.
#[derive(Debug, Deserialize)]
struct AgentSeed {
    name: String,
    transport: Transport,
    launch: Value,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    default_model: Option<String>,
    billing: Billing,
    settings: Value,
}

/// One registry row as the Settings tab lists it: the `agent` row plus **this box's**
/// [`AgentBox`], when the box has one (MOD-2 plan D3 / D14).
///
/// The result row of the inherent `agents()` read, which is inherent rather than a
/// [`ReadStore`](crate::store::ReadStore) method because neither `agent` nor `agent_box` is
/// mirrored (`docs/ANA-9.md` §4.4): offline there is nothing to answer from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSummary {
    /// The `agent` row.
    pub agent: Agent,
    /// This box's `agent_box` row; `Some` when a probe has run on this box, `None` otherwise.
    pub on_box: Option<AgentBox>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seed is a *contract* with `docs/ANA-4.md` §5.3, not a convenience: milestone 5's probe
    /// resolves `${node}` and `${agy_acp_server}` from these rows, so a typo here is a box that
    /// cannot launch an agent rather than a test that fails.
    #[test]
    fn seed_rows_match_ana4_5_3() {
        let now = DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp");
        let rows = seed_rows(now);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "claude");
        assert_eq!(rows[1].name, "agy");

        for row in &rows {
            // Both rows speak ACP and both are subscription-billed: `agy`'s credit overage is
            // undocumented as to whether it meters per token, so `per_token` would be a guess
            // (§5.3). A box on API-key auth flips the row in Settings.
            assert_eq!(row.transport, Transport::Acp, "{}", row.name);
            assert_eq!(row.billing, Billing::Subscription, "{}", row.name);
            assert!(row.enabled, "{}", row.name);
            assert_eq!(row.created_at, now);
            assert_eq!(row.updated_at, now);
        }

        // Empty, not guessed: ACP delivers the model list at `session/new`, and no live capture
        // has been folded into this row.
        assert!(rows[0].models.is_empty(), "claude");
        assert_eq!(rows[0].default_model, None, "claude");
        assert_eq!(rows[0].settings["acp"]["model_config_id"], Value::Null);

        // `agy`'s list *is* a live capture (plan D64, MOD-2 T34): `antigravity-acp` 1.1.1 answered
        // `session/new` with a `configOptions` entry `id: "model"` carrying these eleven values
        // and `currentValue: "gemini-3.7-flash-high"`. The count and the two ends are pinned
        // rather than the whole vector: what must not rot is the *shape* — a non-empty list, a
        // default drawn from it, and the option id §4.4 selects by.
        assert_eq!(rows[1].models.len(), 11, "agy");
        assert_eq!(rows[1].models[0], "gemini-3.8-flash-high");
        assert_eq!(rows[1].models[10], "gemini-3.1-pro-low");
        assert_eq!(
            rows[1].default_model.as_deref(),
            Some("gemini-3.7-flash-high"),
            "agy"
        );
        assert!(
            rows[1]
                .models
                .iter()
                .any(|model| Some(model.as_str()) == rows[1].default_model.as_deref()),
            "a seeded default has to be one of the seeded models, or §4.4 selects nothing"
        );
        assert_eq!(rows[1].settings["acp"]["model_config_id"], "model");

        assert_eq!(rows[0].launch["command"], "${node}");
        assert_eq!(rows[0].launch["args"][0], "${claude_agent_acp}");
        assert_eq!(rows[0].launch["env"]["CLAUDE_CODE_EXECUTABLE"], "${claude}");
        assert_eq!(rows[1].launch["command"], "${agy_acp_server}");

        // The CLI block is `claude`'s alone: `agy` has no CLI path built, so the row must not
        // advertise one (§5.3).
        assert_eq!(rows[0].settings["cli"]["stream"], "claude_stream_json");
        assert!(rows[1].settings.get("cli").is_none());
        assert_eq!(rows[0].settings["quota"]["source"], "acp_meta_rate_limit");
        assert_eq!(rows[1].settings["quota"]["source"], "none");
    }

    /// Ids are minted per call, not baked into the document: two seeds of the same file are two
    /// distinct rows, and `agent.name` is what makes a re-seed a no-op (the unique constraint).
    #[test]
    fn seed_rows_mint_fresh_ids_per_call() {
        let now = DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp");
        let first = seed_rows(now);
        let second = seed_rows(now);

        assert_ne!(first[0].id, second[0].id);
        assert_eq!(first[0].name, second[0].name);
    }
}
