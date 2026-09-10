//! The agent driver seam of `htui` (`docs/ANA-4.md` §4.1, plan MOD-2 D1/D2).
//!
//! [`driver`] holds the two traits every transport implements — a `Send + Sync` [`AgentDriver`]
//! per `agent` row and a `Send` [`AgentSession`] per live session — plus the inputs a session
//! starts from. [`event`] holds the twelve-variant [`DriverEvent`] and its **total** map onto
//! `htui_core::model::EventKind`. [`error`] holds the one [`DriverError`] this crate returns.
//!
//! Only the *persisted* types stay in `htui-core`: the trait, the event enum and (from milestone
//! 2) the transports live here, so the domain crate never grows a JSON-RPC SDK, a process
//! supervisor or a diff library (ANA-4 §8).
//!
//! [`replay`] is [`record`] read backwards (plan D37): a persisted row back into the envelope the
//! live path renders, so a reopened step and a running one reach the chat tab through one
//! transcript. Milestone 4's offline sink is a `WriteStore` arm in `htui-store`, so this crate
//! still records through the trait and knows nothing about where the rows land (plan D34).
//!
//! [`probe`] is milestone 5's answer to "what can *this box* run" (plan D45–D51): the tiered
//! resolver `tools` delegates to, the glob walker and the version capture that fill
//! `agent_box.probe`, and — over [`acp::handshake()`] — the tier-2 `initialize` that is the only
//! proof the resolved binary actually runs.
//!
//! [`install`] is MOD-20's answer to "and what if this box does not have it": the registry a row
//! *declares* a source in, read into a plan the user consents to before a single archive byte is
//! requested. It knows no agent's name (`R-AGT-5`) — the row supplies an entry id, the registry
//! supplies everything else, and [`probe`] supplies the root and, afterwards, the verdict.
//!
//! [`auth`] is MOD-21's transport-neutral half of "and what if this box is not logged in": the
//! flow a caller hands [`AgentDriver::authenticate`], the events it gets back, and how one ends.
//! It names no agent, no method id and no host (`R-AGT-5`) — the method list is whatever the agent
//! answered `initialize` with, and the verdict afterwards is [`probe`]'s.
//!
//! Deliberately absent in milestones 1–2 (plan D16): any wire protocol (`acp/`, `cli/`), the chat
//! tab, and quota. Each is a named seam, not a plan.
#![warn(missing_docs)]

/// Declares a closed JSON vocabulary of `docs/ANA-4.md` §5 / §6 as an enum.
///
/// The `JSONB`-payload counterpart of `htui-core`'s `str_enum!`
/// (`crates/htui-core/src/model/mod.rs:25`): `ALL`, `as_str`, `Display` and the serde renames all
/// come from one string literal per variant, so the JSON text has exactly one definition and
/// `as_str` can never drift from what is written to `session_event.payload`. Unlike `str_enum!`
/// there is no `FromStr` and no `sqlx::Type`, because these values live inside a `JSONB` payload
/// and never in a `TEXT ... CHECK` column.
// Textually scoped: every `mod` below is declared after this definition, so the macro is in scope
// in each of them without an import.
macro_rules! wire_enum {
    (
        $(#[$enum_meta:meta])*
        $name:ident {
            $( $(#[$variant_meta:meta])* $variant:ident => $text:literal ),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize
        )]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                #[serde(rename = $text)]
                $variant,
            )+
        }

        impl $name {
            #[doc = concat!("Every `", stringify!($name), "` variant, in `docs/ANA-4.md` order.")]
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            #[doc = concat!("The JSON text of this `", stringify!($name), "`.")]
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

pub mod acp;
pub mod auth;
#[cfg(feature = "test-support")]
pub mod conformance;
pub mod driver;
pub mod error;
pub mod event;
#[cfg(feature = "test-support")]
pub mod fake;
pub mod install;
pub mod launch;
pub mod permission;
pub mod probe;
pub mod record;
pub mod registry;
pub mod replay;
pub mod tools;

pub use acp::{
    AcpAdapter, AcpDriver, AcpIo, AcpSession, Handshake, SessionCommand, SessionOptions, Stamp,
    handshake, open_session,
};
// `authenticate_acp`, not `authenticate`: at the crate root the bare name reads as the trait
// method every transport has, and this one is the ACP driver's whole operation. The
// `plan_install`/`resolve_tools` precedent below.
pub use auth::{
    AUTH_IDLE_CAP, AuthCall, AuthChoice, AuthEvent, AuthFlow, AuthMethodInfo, AuthOutcome,
    BrowserPolicy, OpenerCommand, authenticate as authenticate_acp, first_url, open_url,
};
pub use driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, McpServerSpec,
    PermissionAnswer, PermissionDefault, PermissionMatch, PermissionPolicy, PermissionRequestId,
    PermissionRule, RememberedPermission, SessionSpec, ToolExposure,
};
pub use error::{DriverError, Result};
pub use event::{
    DoneEvent, DriverEnvelope, DriverEvent, EditProposalEvent, ErrorEvent, OtherEvent,
    PermissionAnswerEvent, PermissionOption, PermissionOptionKind, PermissionRequestEvent,
    PlanEntry, PlanEntryPriority, PlanEntryStatus, PlanEvent, StopReason, TerminalReason,
    TextChunk, ToolCallEvent, ToolKind, ToolLocation, ToolResultEvent, ToolResultStatus,
    UsageEvent,
};
// `plan_install`, not `plan`: at the crate root the bare name says nothing about what is being
// planned, and `resolve_tools` set the precedent.
pub use install::{
    ArchiveFormat, Consent, DISK_HEADROOM_FACTOR, InstallConfig, InstallError, InstallJob,
    InstallOutcome, InstallPhase, InstallPlan, InstallProgress, InstallRecord, Installer, Layout,
    Manifest, ManualSteps, PlanError, REGISTRY_BASE, RegistryAgent, RegistryDocument,
    RegistrySource, STAGING_MAX_AGE, SweepReport, Throttle, install, plan as plan_install,
};
pub use launch::{
    AcpSettings, AgentLaunch, AgentSettings, CliSettings, ClientCapabilities, CredentialProbe,
    Discovery, FallbackCommand, Install, InstallSource, PlatformGlob, QuotaSettings, QuotaSource,
    ResolvedLaunch, SessionSettings, Spawned, ToolMap, ToolProbe, UsageScope, UsageSettings,
    VersionProbe, resolve, spawn,
};
pub use permission::{PolicyAnswer, PolicyStage, evaluate as evaluate_permission};
pub use probe::{
    CredentialTier, INSTALL_ROOT_VAR, ProbeContext, ProbeEnv, ProbeOutcome, ProbeSnapshot,
    ProbeSource, ProbeStatus, SpawnTier2, Tier2, ToolReport, ToolResolution, default_install_root,
    install_root, platform_key, probe_agent, probe_tools, resolve_credential,
};
pub use record::{
    AnsweredBy, CHUNK_FLUSH_BYTES, PERMISSION_DENIED, RecordError, Recorder, RecorderSummary, pump,
};
pub use registry::{DriverFactory, TransportBuilder, adapter_id, caps_for};
// `replay_envelopes`, not `envelopes`: at the crate root the bare name says nothing about which
// direction it runs, and `record`'s counterpart is spelled out too.
pub use replay::{
    ReplayError, envelope_from_row, envelope_or_other, envelopes as replay_envelopes,
};
// `resolve_tools`, not `resolve`: `launch::resolve` already owns that name at the crate root, and
// the two are the halves of one step — find the tools, then substitute them into the row.
pub use tools::{env_override_key, resolve as resolve_tools};

#[cfg(feature = "test-support")]
pub use conformance::{CASES, CaseHarness, Script, ScriptEvent, Turn, run_all, run_case};
#[cfg(feature = "test-support")]
pub use fake::{FAKE_AGENT_NAME, FakeAdapter, FakeDriver, FakeSession, SESSION_STARTED};
