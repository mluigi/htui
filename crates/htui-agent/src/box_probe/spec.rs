//! The box probe spec (MOD-7 plan D9, D17, D18): which tools a box probe looks for, which tags
//! they derive, and which PCI vendors name a GPU.
//!
//! The compiled `spec.json` is the **seed**. The live spec is the seed overlaid by the JSON value
//! of the `app_setting` row keyed [`SETTING_KEY`], read at probe time and merged by name
//! ([`effective`]): a tool entry replaces the seed's tool of that name or adds one, a tag entry
//! replaces the seed's rule for that tag in place or appends, and a vendor entry replaces by `pci`
//! id in place (keeping the priority order) or appends at the lowest priority. `{"disabled": true}`
//! drops a tool, `{"tag": "<t>", "disabled": true}` a rule and `{"pci": "0x….", "disabled": true}`
//! a vendor. The overlay is all or nothing: one fault and the seed is probed alone, with a sentence
//! saying why (blueprint D23).
//!
//! Milestone 1's edit path is SQL. Add a tool:
//!
//! ```sql
//! INSERT INTO app_setting (key, value) VALUES ('box_probe_spec', '{"tools": {"terraform": {"kind": "path", "names": ["terraform"], "version": {"args": ["version"], "pattern": "^Terraform v(\\S+)$"}}}}'::jsonb) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value;
//! ```
//!
//! then restart `htui` (the next `Online` swap sees a new digest, D18) or send
//! `StoreRequest::ProbeBox`. Return to the seed, which changes the digest and re-probes too:
//!
//! ```sql
//! DELETE FROM app_setting WHERE key = 'box_probe_spec';
//! ```
//!
//! An overlay may add only `kind: "path"` tools whose `names` are bare file names (no separator,
//! no `..`): `node_package` and `glob` stay agent-row mechanisms (plan D9, R-11).
//!
//! On Windows with WSL installed, the seed's `bash` resolves to `%SystemRoot%\System32\bash.exe`,
//! the WSL launcher: probing it boots the WSL VM and records the distribution's `bash`, not a
//! Windows one. `{"tools": {"bash": {"disabled": true}}}` in `box_probe_spec` turns it off; the
//! real fix belongs to MOD-16's verification.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use htui_core::prompt::digest::sha256_hex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::warn;

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

/// The seed, parsed on first use. `the_spec_parses_and_every_tag_rule_names_a_listed_tool` pins
/// that it parses and passes [`validate`], so the `expect` below is a build-time fact, not a
/// runtime hazard.
static SEED: LazyLock<Spec> = LazyLock::new(|| {
    serde_json::from_str(include_str!("spec.json")).expect("the compiled box probe spec parses")
});

/// The compiled seed, parsed once (`LazyLock`; `the_spec_parses…` pins that it parses and
/// validates).
#[must_use]
pub fn seed() -> &'static Spec {
    &SEED
}

/// Merges `stored` into `seed`, all or nothing (D17, D23). Never fails: a rejected overlay is the
/// seed plus `error`.
#[must_use]
pub fn effective(seed: &Spec, stored: Option<&Value>) -> EffectiveSpec {
    let Some(stored) = stored else {
        return EffectiveSpec {
            spec: seed.clone(),
            digest: digest(seed),
            error: None,
        };
    };
    match merge(seed, stored) {
        Ok(spec) => {
            let digest = digest(&spec);
            EffectiveSpec {
                spec,
                digest,
                error: None,
            }
        }
        Err(fault) => {
            let error = format!("{SPEC_IGNORED}: {fault}");
            warn!(%error, "the box probe runs under its seed spec");
            EffectiveSpec {
                spec: seed.clone(),
                digest: digest(seed),
                error: Some(error),
            }
        }
    }
}

/// `htui_core::prompt::digest::sha256_hex(&serde_json::to_string(spec))` (D24): over the merged
/// `Spec` only, so an ignored overlay records the seed's digest. `tools` is a `BTreeMap`, and
/// `tags` and `gpu_vendors` keep their merged order, so equal specs hash equal.
#[must_use]
pub fn digest(spec: &Spec) -> String {
    let text = serde_json::to_string(spec).expect("a Spec has string keys and always serialises");
    sha256_hex(&text)
}

/// The checks the seed and every merged result must pass; `Err` is the fault without the prefix.
///
/// # Errors
/// The first fault, as one sentence naming its key.
pub fn validate(spec: &Spec) -> Result<(), String> {
    for (name, probe) in &spec.tools {
        check_tool(name, probe)?;
    }
    let mut tags = BTreeSet::new();
    for rule in &spec.tags {
        check_rule(rule, &spec.tools)?;
        if !tags.insert(rule.tag.as_str()) {
            return Err(format!("tags.{}: the tag has two rules", rule.tag));
        }
    }
    let mut vendors = BTreeSet::new();
    for vendor in &spec.gpu_vendors {
        check_vendor(vendor)?;
        if !vendors.insert(vendor.pci.as_str()) {
            return Err(format!(
                "gpu_vendors.{}: the id has two entries",
                vendor.pci
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The merge (blueprint D23)
// ---------------------------------------------------------------------------------------------

/// The overlay's three keys.
const KEYS: [&str; 3] = ["tools", "tags", "gpu_vendors"];

/// Steps 2–7 of D23: the merged spec, or the first fault.
fn merge(seed: &Spec, stored: &Value) -> Result<Spec, String> {
    let Value::Object(overlay) = stored else {
        return Err("the value is not a JSON object".to_owned());
    };
    if let Some(key) = overlay.keys().find(|key| !KEYS.contains(&key.as_str())) {
        return Err(format!("unknown key `{key}`"));
    }

    let mut spec = seed.clone();
    if let Some(tools) = overlay.get("tools") {
        merge_tools(&mut spec.tools, tools)?;
    }
    prune_seeded_rules(&mut spec);
    if let Some(tags) = overlay.get("tags") {
        merge_tags(&mut spec, tags)?;
    }
    if let Some(vendors) = overlay.get("gpu_vendors") {
        merge_vendors(&mut spec.gpu_vendors, vendors)?;
    }
    validate(&spec)?;
    Ok(spec)
}

/// Step 3: every `tools` entry adds, replaces or (`{"disabled": true}`) drops one tool.
fn merge_tools(tools: &mut BTreeMap<String, ToolProbe>, overlay: &Value) -> Result<(), String> {
    let Value::Object(entries) = overlay else {
        return Err("tools: the value is not a JSON object".to_owned());
    };
    for (name, entry) in entries {
        if !is_bare(name) {
            return Err(format!("tools: `{name}` is not a bare tool name"));
        }
        if is_disabled(entry, None) {
            tools.remove(name);
            continue;
        }
        let probe: ToolProbe = serde_json::from_value(entry.clone())
            .map_err(|error| format!("tools.{name}: {error}"))?;
        check_tool(name, &probe)?;
        tools.insert(name.clone(), probe);
    }
    Ok(())
}

/// Step 4 (blueprint F-C): a tool the overlay dropped leaves the seeded rules that named it; a
/// seeded rule left with no tool and no fact goes with it.
fn prune_seeded_rules(spec: &mut Spec) {
    let tools = &spec.tools;
    for rule in &mut spec.tags {
        rule.any_tool.retain(|name| tools.contains_key(name));
    }
    spec.tags
        .retain(|rule| !rule.any_tool.is_empty() || rule.fact.is_some());
}

/// Step 5: every `tags` element replaces a rule in place, appends one, or drops one.
fn merge_tags(spec: &mut Spec, overlay: &Value) -> Result<(), String> {
    let Value::Array(entries) = overlay else {
        return Err("tags: the value is not a JSON array".to_owned());
    };
    for (index, entry) in entries.iter().enumerate() {
        if let Some(tag) = disabled_key(entry, "tag") {
            spec.tags.retain(|rule| rule.tag != tag);
            continue;
        }
        let rule: TagRule = serde_json::from_value(entry.clone())
            .map_err(|error| format!("tags[{index}]: {error}"))?;
        check_rule(&rule, &spec.tools)?;
        match spec.tags.iter_mut().find(|seeded| seeded.tag == rule.tag) {
            Some(seeded) => *seeded = rule,
            None => spec.tags.push(rule),
        }
    }
    Ok(())
}

/// Step 6: every `gpu_vendors` element replaces a vendor by `pci` in place (keeping its
/// priority), appends one at the lowest priority, or drops one.
fn merge_vendors(vendors: &mut Vec<GpuVendor>, overlay: &Value) -> Result<(), String> {
    let Value::Array(entries) = overlay else {
        return Err("gpu_vendors: the value is not a JSON array".to_owned());
    };
    for (index, entry) in entries.iter().enumerate() {
        if let Some(pci) = disabled_key(entry, "pci") {
            vendors.retain(|vendor| vendor.pci != pci);
            continue;
        }
        let vendor: GpuVendor = serde_json::from_value(entry.clone())
            .map_err(|error| format!("gpu_vendors[{index}]: {error}"))?;
        check_vendor(&vendor)?;
        match vendors.iter_mut().find(|seeded| seeded.pci == vendor.pci) {
            Some(seeded) => *seeded = vendor,
            None => vendors.push(vendor),
        }
    }
    Ok(())
}

/// Whether `entry` is exactly `{"disabled": true}`, plus `key` when one is named.
fn is_disabled(entry: &Value, key: Option<&str>) -> bool {
    let Value::Object(fields) = entry else {
        return false;
    };
    let expected = 1 + usize::from(key.is_some());
    fields.len() == expected
        && fields.get("disabled") == Some(&Value::Bool(true))
        && key.is_none_or(|key| fields.get(key).is_some_and(Value::is_string))
}

/// `entry[key]` when `entry` is exactly `{key: "<text>", "disabled": true}`.
fn disabled_key<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    if !is_disabled(entry, Some(key)) {
        return None;
    }
    entry
        .as_object()
        .and_then(|fields: &Map<String, Value>| fields.get(key))
        .and_then(Value::as_str)
}

// ---------------------------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------------------------

/// Non-empty, not `.` or `..`, and no `/` or `\`: a name `which` looks up on `PATH` and nothing
/// else (plan D9's overlay limits).
fn is_bare(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

/// A `kind: path` tool with bare, non-empty `names` and a compiling version pattern.
fn check_tool(name: &str, probe: &ToolProbe) -> Result<(), String> {
    if !is_bare(name) {
        return Err(format!("tools: `{name}` is not a bare tool name"));
    }
    let ToolProbe::Path { names, version } = probe else {
        return Err(format!(
            "tools.{name}: only `kind: path` tools may be added"
        ));
    };
    if names.is_empty() {
        return Err(format!("tools.{name}: `names` is empty"));
    }
    if let Some(file) = names.iter().find(|file| !is_bare(file)) {
        return Err(format!("tools.{name}: `{file}` is not a bare file name"));
    }
    if let Some(version) = version {
        compiles(&version.pattern)
            .map_err(|error| format!("tools.{name}.version.pattern: {error}"))?;
    }
    Ok(())
}

/// A non-empty tag with exactly one of tools or a fact, every tool listed, and a compiling `ask`
/// that has tools to ask.
fn check_rule(rule: &TagRule, tools: &BTreeMap<String, ToolProbe>) -> Result<(), String> {
    let tag = &rule.tag;
    if tag.is_empty() {
        return Err("tags: a rule has an empty `tag`".to_owned());
    }
    if rule.any_tool.is_empty() == rule.fact.is_none() {
        return Err(format!(
            "tags.{tag}: a rule names tools or a fact, not both or neither"
        ));
    }
    if let Some(missing) = rule.any_tool.iter().find(|name| !tools.contains_key(*name)) {
        return Err(format!(
            "tags.{tag}: names `{missing}`, which no tool provides"
        ));
    }
    if let Some(ask) = &rule.ask {
        if rule.any_tool.is_empty() {
            return Err(format!("tags.{tag}: `ask` needs `any_tool`"));
        }
        compiles(&ask.matches).map_err(|error| format!("tags.{tag}.ask.matches: {error}"))?;
    }
    Ok(())
}

/// `pci` is `0x` and four lowercase hex digits; `name` is not empty.
fn check_vendor(vendor: &GpuVendor) -> Result<(), String> {
    let pci = &vendor.pci;
    let digits = pci.strip_prefix("0x").unwrap_or_default();
    if digits.len() != 4
        || !digits
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!(
            "gpu_vendors.{pci}: `pci` is not `0x` and four lowercase hex digits"
        ));
    }
    if vendor.name.is_empty() {
        return Err(format!("gpu_vendors.{pci}: `name` is empty"));
    }
    Ok(())
}

/// `Ok` when `pattern` compiles; otherwise the regex error as one line.
///
/// The **last** non-empty line, not the first: a syntax error's first line is the bare header
/// `regex parse error:`, and the line that says what is wrong (`error: unclosed group`) comes after
/// the pattern and its caret.
fn compiles(pattern: &str) -> Result<(), String> {
    Regex::new(pattern).map(drop).map_err(|error| {
        let text = error.to_string();
        text.lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or_default()
            .to_owned()
    })
}
