//! Identifier newtypes, one per `UUID` primary key of `docs/ANA-9.md` §5.
//!
//! A struct field never holds a bare [`Uuid`] (blueprint B.1): mixing an `ItemId` with a
//! `ProjectId` is then a compile error rather than a runtime lookup miss. New identifiers are
//! minted client-side as UUIDv7 (§3), so a row is named before Postgres has answered and B-tree
//! inserts stay ordered.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Declares one or more `UUID` newtypes with their constructors and conversions.
macro_rules! id_newtype {
    ($( $(#[$meta:meta])* $name:ident ),* $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(
                Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize,
                Deserialize,
            )]
            #[serde(transparent)]
            #[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
            #[cfg_attr(feature = "sqlx", sqlx(transparent))]
            pub struct $name(pub Uuid);

            impl $name {
                #[doc = concat!("Mints a fresh `", stringify!($name), "` as a UUIDv7 (§3).")]
                #[must_use]
                pub fn new() -> Self {
                    Self(Uuid::now_v7())
                }

                #[doc = concat!("Wraps an existing `Uuid` as a `", stringify!($name), "`.")]
                #[must_use]
                pub const fn from_uuid(u: Uuid) -> Self {
                    Self(u)
                }

                /// The wrapped `Uuid`, for the database driver and for logging.
                #[must_use]
                pub const fn as_uuid(self) -> Uuid {
                    self.0
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Display::fmt(&self.0.hyphenated(), f)
                }
            }

            impl FromStr for $name {
                type Err = uuid::Error;

                fn from_str(s: &str) -> Result<Self, Self::Err> {
                    Uuid::from_str(s).map(Self)
                }
            }

            impl From<Uuid> for $name {
                fn from(u: Uuid) -> Self {
                    Self(u)
                }
            }

            impl From<$name> for Uuid {
                fn from(id: $name) -> Self {
                    id.0
                }
            }
        )*
    };
}

id_newtype!(
    /// `app_user.id` (§5.2).
    UserId,
    /// `box.id` (§5.2), minted on first launch and kept in `box.toml`.
    BoxId,
    /// `workspace.id` (§5.3).
    WorkspaceId,
    /// `project.id` (§5.3).
    ProjectId,
    /// `repo.id` (§5.3).
    RepoId,
    /// `item_kind.id` (§5.5).
    ItemKindId,
    /// `item.id` (§5.5).
    ItemId,
    /// `item_note.id` (§5.5).
    NoteId,
    /// `document.id` (§5.5).
    DocumentId,
    /// `step_graph.id` (§5.4).
    StepGraphId,
    /// `step_graph_phase.id` (§5.4).
    PhaseId,
    /// `prompt_template.id` (§5.4).
    PromptTemplateId,
    /// `agent.id` (§5.7).
    AgentId,
    /// `run.id` (§5.8).
    RunId,
    /// `run_step.id` (§5.8); §6.1 names this one `StepId`, so the newtype keeps that name.
    StepId,
    /// `skill.id` (§5.6).
    SkillId,
    /// `skill_binding.id` (§5.6).
    SkillBindingId,
    /// `command_run.id` (§5.8).
    CommandRunId,
    /// `requirement_area.id` (ANA-11 §5).
    RequirementAreaId,
    /// `requirement.id` (ANA-11 §5).
    RequirementId,
    /// `step_permission.id` (MOD-42 plan D1): one parked stage-3 permission request.
    PermissionId,
    /// `run_command.id` (MOD-42 plan D1): one requested command on a run (today: cancel).
    RunCommandId,
    /// `step_permission.session` (MOD-42 plan D1): one driven agent session, minted per
    /// `drive_once` (a candidate's session, each judge call), so request ids that repeat across
    /// sessions never collide (PRD MVP item 3).
    RelaySessionId,
    /// `persona.id` (MOD-26 plan D1): one agent persona of the global registry.
    PersonaId,
);
