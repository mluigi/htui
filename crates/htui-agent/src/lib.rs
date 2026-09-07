//! The agent driver seam of `htui` (`docs/ANA-4.md` §4.1, plan MOD-2 D1/D2).
//!
//! [`driver`] holds the two traits every transport implements — a `Send + Sync` [`AgentDriver`]
//! per `agent` row and a `Send` [`AgentSession`] per live session — plus the inputs a session
//! starts from. [`event`] holds the eleven-variant [`DriverEvent`] and its **total** map onto
//! `htui_core::model::EventKind`. [`error`] holds the one [`DriverError`] this crate returns.
//!
//! Only the *persisted* types stay in `htui-core`: the trait, the event enum and (from milestone
//! 2) the transports live here, so the domain crate never grows a JSON-RPC SDK, a process
//! supervisor or a diff library (ANA-4 §8).
//!
//! Deliberately absent in milestones 1–2 (plan D16): any wire protocol (`acp/`, `cli/`), probes,
//! the chat tab, quota, and the offline session path. Each is a named seam, not a plan.
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

#[cfg(feature = "test-support")]
pub mod conformance;
pub mod driver;
pub mod error;
pub mod event;
#[cfg(feature = "test-support")]
pub mod fake;
pub mod launch;
pub mod record;

pub use driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, McpServerSpec,
    PermissionAnswer, PermissionDefault, PermissionMatch, PermissionPolicy, PermissionRequestId,
    PermissionRule, RememberedPermission, SessionSpec, ToolExposure,
};
pub use error::{DriverError, Result};
pub use event::{
    DoneEvent, DriverEnvelope, DriverEvent, EditProposalEvent, ErrorEvent, OtherEvent,
    PermissionOption, PermissionOptionKind, PermissionRequestEvent, PlanEntry, PlanEntryPriority,
    PlanEntryStatus, PlanEvent, StopReason, TerminalReason, TextChunk, ToolCallEvent, ToolKind,
    ToolLocation, ToolResultEvent, ToolResultStatus, UsageEvent,
};
pub use launch::{
    AcpSettings, AgentLaunch, AgentSettings, CliSettings, ClientCapabilities, Discovery,
    FallbackCommand, PlatformGlob, QuotaSettings, QuotaSource, ResolvedLaunch, SessionSettings,
    Spawned, ToolMap, ToolProbe, UsageScope, UsageSettings, VersionProbe, resolve, spawn,
};
pub use record::{AnsweredBy, CHUNK_FLUSH_BYTES, RecordError, Recorder, RecorderSummary, pump};

#[cfg(feature = "test-support")]
pub use conformance::{CASES, CaseHarness, Script, ScriptEvent, Turn, run_all, run_case};
#[cfg(feature = "test-support")]
pub use fake::{FAKE_AGENT_NAME, FakeDriver, FakeSession, SESSION_STARTED};
