//! The boxes behind `Settings > Boxes` (MOD-7 milestone 2, D45): every box of this user with its
//! tools and recorded spec digest, which of them is this box, and the effective probe spec.
//!
//! One read per event, one reply out; the section renders only the last snapshot and never
//! patches a row into it (the rule [`crate::prompt_settings`] states in its module doc). Writes
//! are `WriteStore::edit_box`, a compare-and-set on `box.edit_version` (D39, D41).
//!
//! Known residue, the one [`crate::prompt_settings`] records: a re-read that fails after an
//! applied write answers `Failed`, though the row has changed.
//!
//! Nothing here reads the clock or mints an id.

use htui_agent::box_probe::spec;
use htui_core::model::{BoxId, BoxRecord};
use htui_core::store::Result;
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};
use serde_json::Value;

use crate::store_worker::{StoreReply, StoreRequest};

/// One read: this box, every box of this user, and the effective probe spec.
///
/// `PartialEq` only: [`BoxRecord`] derives no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxesSnapshot {
    /// This process's box, from `Backend::box_info`; `None` before registration.
    pub this_box: Option<BoxId>,
    /// `WriteStore::boxes`: this user's boxes by id, each with tools and recorded digest.
    pub boxes: Vec<BoxRecord>,
    /// The spec the next probe would run under.
    pub spec: SpecView,
}

/// The effective probe spec as the section shows it (D45, D51); computed here, never on the
/// render side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecView {
    /// `EffectiveSpec.digest`: compare with `BoxRecord.probe_spec_digest`.
    pub digest: String,
    /// Whether a stored `box_probe_spec` overlay is in force (stored **and** accepted).
    pub overlay: bool,
    /// `EffectiveSpec.error`: why a stored overlay was ignored (it starts with
    /// `spec::SPEC_IGNORED`).
    pub error: Option<String>,
}

/// The view of `spec::effective(spec::seed(), stored)`. Pure.
#[must_use]
pub fn spec_view(stored: Option<&Value>) -> SpecView {
    let _ = (stored, spec::seed());
    todo!("MOD-7 milestone 2 T3: spec_view")
}

/// One read (D45): `backend.box_info()` for this box, `writer.boxes()`, and
/// `backend.app_settings()` for `spec::SETTING_KEY`.
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot(backend: &Backend, writer: &Writer) -> Result<BoxesSnapshot> {
    let _ = (backend, writer);
    todo!("MOD-7 milestone 2 T3: snapshot")
}

/// Serves `Boxes` and `EditBox` (D45, D46). `Err(Unreachable(DATABASE_UNREACHABLE))` offline
/// (the writer is `None`), for the read too. `EditBox` answers `Boxes` on `Applied`,
/// `BoxesStale` on `Stale`, and `BoxesStale` on `NotFound { entity: "box" }` too, so a box that
/// vanished under an open editor reaches the section as a snapshot without it.
///
/// # Errors
/// Whatever the seam reports (a `Constraint` becomes `Failed` with the tag sentence), plus
/// `Unreachable` offline and `Backend` for a request that is not one of the two.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let _ = (backend, request, DATABASE_UNREACHABLE);
    todo!("MOD-7 milestone 2 T3: serve")
}

/// The two request names, in [`StoreRequest`] order.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here, so a third
/// request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 2] = ["boxes", "edit_box"];

/// The **read**'s name: a refused read leaves the section with no list; a refused write leaves the
/// editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];
