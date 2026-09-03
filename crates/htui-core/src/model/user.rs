//! Users and the capability vocabulary (`docs/ANA-9.md` §5.2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::UserId;

/// A row of `app_user` (§5.2). Version one seeds exactly one row on first connect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUser {
    /// `app_user.id`.
    pub id: UserId,
    /// `app_user.name`, unique across the database.
    pub name: String,
    /// `app_user.email`.
    pub email: Option<String>,
    /// `app_user.created_at`.
    pub created_at: DateTime<Utc>,
    /// `app_user.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `capability_tag` (§5.2): the open vocabulary a box is probed and declared against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityTag {
    /// `capability_tag.tag`, the primary key, e.g. `gpu` or `msvc`.
    pub tag: String,
    /// `capability_tag.description`.
    pub description: String,
    /// `capability_tag.seeded`: true for the vocabulary shipped with `htui`.
    pub seeded: bool,
}
