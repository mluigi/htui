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
use htui_core::store::{CasOutcome, Result, StoreError, StoredSetting, WorkerLogStore, WriteStore};
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
    let boxes = tree_order(writer.boxes().await?);
    let stored = writer.box_probe_spec().await?;
    Ok(BoxesSnapshot {
        this_box,
        boxes,
        spec: spec_view(stored),
    })
}

/// The boxes as the section lists them (MOD-130 M4 P4): a machine, then its containers by
/// hostname and id, machines in the order given. A container whose parent is not in `boxes` (a
/// read race, or a parent of another user) comes after every machine, by hostname and id. Pure;
/// applied once where the snapshot is built, so the cursor indexes the one order.
#[must_use]
pub fn tree_order(boxes: Vec<BoxRecord>) -> Vec<BoxRecord> {
    let (machines, mut children): (Vec<_>, Vec<_>) = boxes
        .into_iter()
        .partition(|record| record.row.parent_box_id.is_none());
    children.sort_by(|a, b| (&a.row.hostname, a.row.id).cmp(&(&b.row.hostname, b.row.id)));
    let mut ordered = Vec::with_capacity(machines.len() + children.len());
    for machine in machines {
        let id = machine.row.id;
        ordered.push(machine);
        let (own, rest): (Vec<_>, Vec<_>) = children
            .into_iter()
            .partition(|child| child.row.parent_box_id == Some(id));
        ordered.extend(own);
        children = rest;
    }
    // What is left has a parent that is not listed.
    ordered.extend(children);
    ordered
}

/// Serves `Boxes`, `EditBox`, `SetProbeSpec` and `SetBoxLogLevel` (D45, D46; MOD-51 D4; MOD-122
/// M2 F9).
/// `Err(Unreachable(DATABASE_UNREACHABLE))` offline (the writer is `None`), for the read too.
/// `EditBox` answers `Boxes` on `Applied`, `BoxesStale` on `Stale`, and `BoxesStale` on
/// `NotFound { entity: "box" }` too, so a box that vanished under an open editor reaches the
/// section as a snapshot without it. `SetProbeSpec` answers `Boxes` on `Applied` and
/// `BoxesStale` on `Stale` (the row gone included); an overlay `spec::check` refuses is
/// `Err(Constraint("{SPEC_REFUSED}: {fault}"))` before the store is reached (MOD-51 D3).
/// `SetBoxLogLevel` answers `Boxes` when the level was written and `BoxesStale` when the box is
/// gone (no box of mine), `EditBox`'s `NotFound` rule.
///
/// # Errors
/// Whatever the seam reports (a `Constraint` becomes `Failed` with the store's tag, executor or
/// overlay sentence), the checker's refusal above, plus
/// `Unreachable` offline and `Backend` for a request that is not one of the four.
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
            // MOD-51 D3: the probe's own merge, without its warning; nothing is written on a
            // refusal. A clear has nothing to check, and the store refuses one with no token.
            if let Some(overlay) = overlay {
                spec::check(overlay).map_err(|fault| {
                    StoreError::Constraint(format!("{}: {fault}", spec::SPEC_REFUSED))
                })?;
            }
            // A row gone under the editor is `Stale(None)`, a miss like a spent token (MOD-51
            // D2), so it answers `BoxesStale` like one.
            let outcome = writer
                .set_box_probe_spec(overlay.clone(), *expected)
                .await?;
            let applied = matches!(outcome, CasOutcome::Applied(_));
            let fresh = Box::new(snapshot(backend, &writer).await?);
            Ok(if applied {
                StoreReply::Boxes(fresh)
            } else {
                StoreReply::BoxesStale(fresh)
            })
        }
        StoreRequest::SetBoxLogLevel { box_id, level } => {
            // MOD-122 M2 D7: last write wins, so there is no token to spend; `false` is a box
            // that is not (or no longer) one of mine, a miss like `EditBox`'s `NotFound`.
            let applied = writer.set_box_log_level(*box_id, Some(*level)).await?;
            let fresh = Box::new(snapshot(backend, &writer).await?);
            Ok(if applied {
                StoreReply::Boxes(fresh)
            } else {
                StoreReply::BoxesStale(fresh)
            })
        }
        // `try_serve` routes exactly this module's four variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a box settings request: {}",
            other.name()
        ))),
    }
}

/// The four request names, in [`StoreRequest`] order.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here, so a fifth
/// request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 4] = ["boxes", "edit_box", "set_probe_spec", "set_box_log_level"];

/// The **read**'s name: a refused read leaves the section with no list; a refused write leaves the
/// editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use htui_core::model::{BoxKind, BoxRow, OsFamily, UserId};

    use super::*;

    /// A box called `hostname`, a child of `parent` when there is one.
    fn record(hostname: &str, parent: Option<BoxId>) -> BoxRecord {
        let at = DateTime::<Utc>::UNIX_EPOCH;
        BoxRecord {
            row: BoxRow {
                id: BoxId::new(),
                user_id: UserId::from_uuid(uuid::Uuid::nil()),
                hostname: hostname.to_owned(),
                os_family: Some(OsFamily::Linux),
                os_version: "6.8".to_owned(),
                arch: "x86_64".to_owned(),
                cpu: String::new(),
                ram_mb: None,
                gpu_present: false,
                gpu_vendor: None,
                htui_version: "0.1.0".to_owned(),
                probed_tags: Vec::new(),
                declared_tags: Vec::new(),
                quirks: String::new(),
                settings: serde_json::json!({}),
                registered_at: at,
                last_seen_at: at,
                last_probed_at: None,
                updated_at: at,
                edit_version: 0,
                kind: if parent.is_some() {
                    BoxKind::Container
                } else {
                    BoxKind::Machine
                },
                parent_box_id: parent,
            },
            tools: Vec::new(),
            probe_spec_digest: None,
        }
    }

    fn names(boxes: &[BoxRecord]) -> Vec<&str> {
        boxes.iter().map(|b| b.row.hostname.as_str()).collect()
    }

    #[test]
    fn machines_only_keep_the_order_the_store_gave() {
        let boxes = vec![
            record("zed", None),
            record("alpha", None),
            record("mid", None),
        ];
        let ordered = tree_order(boxes.clone());
        assert_eq!(ordered, boxes);
    }

    #[test]
    fn a_parent_is_followed_by_its_children_by_hostname() {
        let parent = record("host", None);
        let other = record("other", None);
        let b = record("b-box", Some(parent.row.id));
        let a = record("a-box", Some(parent.row.id));
        // The store gives the children ahead of, and between, the machines.
        let ordered = tree_order(vec![b, other, a, parent]);
        assert_eq!(names(&ordered), ["other", "host", "a-box", "b-box"]);
    }

    #[test]
    fn two_machines_each_keep_their_own_child() {
        let one = record("one", None);
        let two = record("two", None);
        let under_two = record("under-two", Some(two.row.id));
        let under_one = record("under-one", Some(one.row.id));
        let ordered = tree_order(vec![one, under_two, two, under_one]);
        assert_eq!(names(&ordered), ["one", "under-one", "two", "under-two"]);
    }

    #[test]
    fn a_child_of_an_unlisted_parent_comes_last() {
        let machine = record("machine", None);
        let orphan = record("orphan", Some(BoxId::new()));
        let child = record("child", Some(machine.row.id));
        let ordered = tree_order(vec![orphan, machine, child]);
        assert_eq!(names(&ordered), ["machine", "child", "orphan"]);
    }

    #[test]
    fn a_same_host_pair_of_children_falls_back_to_id() {
        let parent = record("host", None);
        let mut first = record("same", Some(parent.row.id));
        let mut second = record("same", Some(parent.row.id));
        first.row.id = BoxId::from_uuid(uuid::Uuid::from_u128(2));
        second.row.id = BoxId::from_uuid(uuid::Uuid::from_u128(1));
        let ordered = tree_order(vec![parent, first, second]);
        assert_eq!(
            ordered[1].row.id,
            BoxId::from_uuid(uuid::Uuid::from_u128(1))
        );
        assert_eq!(
            ordered[2].row.id,
            BoxId::from_uuid(uuid::Uuid::from_u128(2))
        );
    }
}
