//! The boxes behind `Settings > Boxes` (MOD-7 milestone 2, D45): every box of this user with its
//! tools and recorded spec digest, which of them is this box, and the effective probe spec.
//!
//! One read per event, one reply out; the section renders only the last snapshot and never
//! patches a row into it (the rule [`crate::prompt_settings`] states in its module doc). Writes
//! are `WriteStore::edit_box`, a compare-and-set on `box.edit_version` (D39, D41), and
//! `WriteStore::set_box_probe_spec`, a compare-and-set on the overlay row's `updated_at` (MOD-51
//! D2), refused first by `spec::check` when the probe would ignore the overlay (MOD-51 D3).
//!
//! Known residue, the one [`crate::prompt_settings`] records: a re-read that fails after an
//! applied write answers `Failed`, though the row has changed.
//!
//! Nothing here reads the clock or mints an id.

use htui_agent::box_probe::spec;
use htui_core::model::{BoxId, BoxRecord};
use htui_core::store::{CasOutcome, Result, StoreError, StoredSetting, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

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
    /// The stored `box_probe_spec` row and its token (MOD-51 D5), `None` when there is no row;
    /// the spec editor opens on it. Kept even when the probe ignores the value (R-6): the editor
    /// shows what is stored, and clearing it is always accepted.
    pub stored: Option<StoredSetting>,
}

/// The view of `spec::effective(spec::seed(), stored value)`, carrying the row. Pure.
#[must_use]
pub fn spec_view(stored: Option<StoredSetting>) -> SpecView {
    let value = stored.as_ref().and_then(|row| row.value.as_ref());
    let effective = spec::effective(spec::seed(), value);
    SpecView {
        overlay: value.is_some() && effective.error.is_none(),
        digest: effective.digest,
        error: effective.error,
        stored,
    }
}

/// One read (D45): `backend.box_info()` for this box, `writer.boxes()`, and
/// `writer.box_probe_spec()`: the view and its token from one read (MOD-51 D5).
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot(backend: &Backend, writer: &Writer) -> Result<BoxesSnapshot> {
    let this_box = backend.box_info().await?.map(|info| info.box_id);
    let boxes = writer.boxes().await?;
    let stored = writer.box_probe_spec().await?;
    Ok(BoxesSnapshot {
        this_box,
        boxes,
        spec: spec_view(stored),
    })
}

/// Serves `Boxes`, `EditBox` and `SetProbeSpec` (D45, D46; MOD-51 D4).
/// `Err(Unreachable(DATABASE_UNREACHABLE))` offline (the writer is `None`), for the read too.
/// `EditBox` answers `Boxes` on `Applied`, `BoxesStale` on `Stale`, and `BoxesStale` on
/// `NotFound { entity: "box" }` too, so a box that vanished under an open editor reaches the
/// section as a snapshot without it. `SetProbeSpec` answers `Boxes` on `Applied` and
/// `BoxesStale` on `Stale` (the row gone included); an overlay `spec::check` refuses is
/// `Err(Constraint("{SPEC_REFUSED}: {fault}"))` before the store is reached (MOD-51 D3).
///
/// # Errors
/// Whatever the seam reports (a `Constraint` becomes `Failed` with the store's tag, executor or
/// overlay sentence), the checker's refusal above, plus
/// `Unreachable` offline and `Backend` for a request that is not one of the three.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;

    match request {
        StoreRequest::Boxes => Ok(StoreReply::Boxes(Box::new(
            snapshot(backend, &writer).await?,
        ))),
        StoreRequest::EditBox {
            box_id,
            expected,
            edit,
        } => {
            // The worker re-reads rather than handing the section the single row `Stale` carries:
            // the section renders the whole list, and a row patched in locally would be a second
            // source of truth. A box gone under the editor is a miss like a spent token (D48's
            // `DELETED_ELSEWHERE`); a refused tag list or executor (MOD-41 plan D10) stays an
            // error, so `Failed` carries the store's sentence.
            let applied = match writer.edit_box(*box_id, *expected, edit.clone()).await {
                Ok(CasOutcome::Applied(_)) => true,
                Ok(CasOutcome::Stale(_)) | Err(StoreError::NotFound { entity: "box", .. }) => false,
                Err(other) => return Err(other),
            };
            let fresh = Box::new(snapshot(backend, &writer).await?);
            Ok(if applied {
                StoreReply::Boxes(fresh)
            } else {
                StoreReply::BoxesStale(fresh)
            })
        }
        StoreRequest::SetProbeSpec { overlay, expected } => {
            todo!("MOD-51 T3: {overlay:?} {expected:?}")
        }
        // `try_serve` routes exactly this module's three variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a box settings request: {}",
            other.name()
        ))),
    }
}

/// The three request names, in [`StoreRequest`] order.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here, so a fourth
/// request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 3] = ["boxes", "edit_box", "set_probe_spec"];

/// The **read**'s name: a refused read leaves the section with no list; a refused write leaves the
/// editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];
