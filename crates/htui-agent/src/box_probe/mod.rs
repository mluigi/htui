//! The box probe (MOD-7 plan D6–D9): what this box is and what it can build.
//!
//! Red: the signatures are final, the bodies arrive with the green commits.

pub mod hardware;
pub mod spec;

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use htui_core::model::{BoxId, BoxProbe};

use crate::probe::ProbeEnv;

use self::hardware::HardwareSource;
use self::spec::{EffectiveSpec, GpuVendor, TagRule};

/// At most this many tool lookups and version children at once (plan D9).
pub const MAX_CONCURRENT_TOOLS: usize = 8;

/// One box probe (plan D8): hardware through `hardware`, tools and tags through `spec`, on this
/// copy of `env` with `versions` forced on (OQ-11). Never fails; an unreadable fact is empty.
pub async fn probe_box(
    box_id: BoxId,
    env: &ProbeEnv,
    hardware: &dyn HardwareSource,
    spec: &EffectiveSpec,
    htui_version: &str,
    now: DateTime<Utc>,
) -> BoxProbe {
    let _ = (box_id, env, hardware, spec, htui_version, now);
    todo!("MOD-7 T3: probe_box")
}

/// The first vendor of `map`, in its order, whose `pci` is among `found` (plan D7).
#[must_use]
pub fn pick_gpu_vendor<'a>(found: &[String], map: &'a [GpuVendor]) -> Option<&'a str> {
    let _ = (found, map);
    todo!("MOD-7 T3: pick_gpu_vendor")
}

/// The tags `rules` derive (PRD D2), sorted and deduplicated. `asked` holds each `ask` rule's answer
/// by tag; a rule with an `ask` and no answer does not fire.
#[must_use]
pub fn tags_from_presence(
    rules: &[TagRule],
    present: &BTreeSet<String>,
    gpu_present: bool,
    asked: &BTreeMap<String, bool>,
) -> Vec<String> {
    let _ = (rules, present, gpu_present, asked);
    todo!("MOD-7 T3: tags_from_presence")
}
