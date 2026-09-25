//! The box probe spec (MOD-7 plan D9, D17, D18): which tools a box probe looks for, which tags
//! they derive, and which PCI vendors name a GPU.
//!
//! Red: the types are final, the bodies arrive with the green commit.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::launch::ToolProbe;

/// The `app_setting` key of the stored overlay (plan D17).
pub const SETTING_KEY: &str = "box_probe_spec";

/// The first words of every overlay refusal (blueprint D23).
pub const SPEC_IGNORED: &str = "box_probe_spec ignored";

/// The probe spec: tools, tag rules and the GPU vendor map (plan D9). The compiled `spec.json` is
/// the seed; a stored overlay is merged into it by name ([`effective`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Tool name → probe; `launch::ToolProbe` verbatim, always `kind: path`.
    pub tools: BTreeMap<String, ToolProbe>,
    /// In order; the first rule per tag wins nothing: each fires on its own.
    pub tags: Vec<TagRule>,
    /// Priority order: the first present vendor names the GPU.
    pub gpu_vendors: Vec<GpuVendor>,
}

/// One derived tag (PRD D2): exactly one of `any_tool` (non-empty) or `fact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagRule {
    /// The tag.
    pub tag: String,
    /// Fires when any of these tools is present (and `ask`, if set, matches).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_tool: Vec<String>,
    /// Fires on a box fact instead of a tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact: Option<Fact>,
    /// Runs the first present tool of `any_tool` with `args`; fires when a line matches `matches`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
}

/// A box fact a rule can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fact {
    /// A display device from a mapped vendor.
    Gpu,
}

/// A question asked of a present tool (the MinGW `gcc` case).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    /// Arguments.
    pub args: Vec<String>,
    /// A regex, matched against each trimmed line of stdout then the stderr tail.
    pub matches: String,
}

/// One PCI vendor id and the name `box.gpu_vendor` records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuVendor {
    /// `0x` + four lowercase hex digits.
    pub pci: String,
    /// E.g. `nvidia`.
    pub name: String,
}

/// The spec a probe runs under, its digest, and why a stored overlay was ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSpec {
    /// The merged spec (or the seed).
    pub spec: Spec,
    /// [`digest`] of `spec`.
    pub digest: String,
    /// `Some(sentence)` when a stored overlay was rejected and the seed used.
    pub error: Option<String>,
}

/// The compiled seed, parsed once.
#[must_use]
pub fn seed() -> &'static Spec {
    todo!("MOD-7 T3: seed")
}

/// Merges `stored` into `seed`, all or nothing (D17, D23). Never fails: a rejected overlay is the
/// seed plus `error`.
#[must_use]
pub fn effective(seed: &Spec, stored: Option<&serde_json::Value>) -> EffectiveSpec {
    let _ = (seed, stored);
    todo!("MOD-7 T3: effective")
}

/// `sha256_hex` of the spec's JSON text (D24).
#[must_use]
pub fn digest(spec: &Spec) -> String {
    let _ = spec;
    todo!("MOD-7 T3: digest")
}

/// The checks the seed and every merged result must pass; `Err` is the fault without the prefix.
///
/// # Errors
/// The first fault, as one sentence naming its key.
pub fn validate(spec: &Spec) -> Result<(), String> {
    let _ = spec;
    todo!("MOD-7 T3: validate")
}
