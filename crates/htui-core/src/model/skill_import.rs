//! ANA-22 §7.3's import mapping: a parsed `SKILL.md` becomes the arguments the two writers take,
//! plus the prefill the attachment form will read back out of `skill_version.source`.
//!
//! **One definition, two callers.** [`prefill_from_source`] is the only place §7.3's prefill rules
//! are written down. [`parse`] calls it with the frontmatter it just read, and the attachments
//! matrix calls it with the frontmatter it read back out of a stored version — so a skill imported
//! today and attached next week is prefilled by the same rules that prefilled it on import (plan
//! D96, blueprint A-6).
//!
//! **What import writes.** A `skill` row and a `skill_version` row, and nothing else: ANA-22 §7.3
//! is explicit that "nothing is attached by import". The activation keys are *prefill*, not state.
//!
//! **What the reader could not read is kept.** [`ParsedSkill::source`] holds the whole frontmatter
//! verbatim, so §5.6's lossless promise survives a key this mapping does not understand — and
//! [`ParsedSkill::issues`] says which those were.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value as Json, json};

use crate::model::frontmatter::{self, Issue, Value, split};
use crate::model::skill::{Activation, validate_name};
use crate::store::traits::invalid_skill_name;

/// The file name §7.3's `name` fallback treats as a skill: its parent directory names the skill.
pub const SKILL_FILE: &str = "SKILL.md";

/// What §7.3 says the attachment form should be seeded with, and the one hint that travels with it.
///
/// Every field is optional because §7.3's rows are conditional: a file with no activation key at
/// all prefills nothing, and the form shows its own defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportPrefill {
    /// `always` or `glob`, when the file said so. `None` leaves the form's own default.
    pub activation: Option<Activation>,
    /// The globs §7.3's first prefill row names, from `paths`, `globs`, `applyTo` or
    /// `fileMatchPattern`, split on commas and trimmed.
    pub globs: Vec<String>,
    /// htui's own `languages` key, passed through as authored.
    pub languages: Vec<String>,
    /// Why a model-decided or manual-activation file became `always` here, shown beside the field.
    pub hint: Option<&'static str>,
}

/// One parsed skill file, ready for the two writers.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSkill {
    /// `skill.name`, already checked against the Agent Skills rule.
    pub name: String,
    /// `skill.description`: the file's `description`, with `when_to_use` appended after a blank
    /// line when the file has one.
    pub description: String,
    /// `skill_version.body`: everything after the closing fence.
    pub body: String,
    /// `skill_version.source`: `format`, `path`, `imported_at`, the whole `frontmatter`, and the
    /// `issues` the reader recorded. The prompt builder never reads it (ANA-22 §5.6).
    pub source: Json,
    /// What the attachment form should be seeded with when this skill is attached.
    pub prefill: ImportPrefill,
    /// The keys the reader did not understand. Reported, never silent (§5.7).
    pub issues: Vec<Issue>,
}

/// Parses one skill file.
///
/// `path` is the string the maintainer typed and is what lands in `source.path`; `file_name` is
/// its last segment, which §7.3's name chain and `source.format` both need. `now` is the import
/// instant, taken once by the worker so a batch shares it.
///
/// Fails only for a file that is not a skill file: no opening fence, or a name the writer would
/// refuse. Both refusals are the store's or the reader's own sentence, so the notice the maintainer
/// reads is the one the store would have said.
pub fn parse(
    path: &str,
    file_name: &str,
    text: &str,
    now: DateTime<Utc>,
) -> Result<ParsedSkill, String> {
    let split = split(text).map_err(|error| error.to_string())?;

    let name = name_of(split.scalar("name"), path, file_name);
    if !validate_name(&name) {
        return Err(invalid_skill_name(&name));
    }

    let description = description_of(&split);
    let frontmatter = frontmatter_json(&split);
    let issues = split.issues.clone();

    Ok(ParsedSkill {
        name,
        description,
        body: split.body.clone(),
        source: json!({
            "format": format_of(file_name),
            "path": path,
            "imported_at": now.to_rfc3339(),
            "frontmatter": Json::Object(frontmatter.clone()),
            "issues": issues_json(&issues),
        }),
        prefill: prefill_of(&frontmatter),
        issues,
    })
}

/// §7.3's name chain: the `name` key, else the parent directory for a `SKILL.md`, else the stem.
///
/// The fallbacks are load-bearing rather than decorative: one `SKILL.md` measured on the
/// maintainer's machine carries no `name` key at all, and a Cursor `.mdc` rules file never has one.
///
/// The **stem** fallback reads two file-naming conventions rather than refusing them: GitHub
/// Copilot's `<name>.instructions.md` names the rule `<name>`, and a snake-case stem such as
/// `api_style.mdc` names it `api-style`, the only spelling the Agent Skills rule allows. Nothing
/// else is rewritten — a declared `name` is taken as written, and a stem with a space or a capital
/// is still refused (OQ-22: a name nobody wrote is not invented).
fn name_of(declared: Option<&str>, path: &str, file_name: &str) -> String {
    if let Some(name) = declared.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    let as_path = Path::new(path);
    if file_name == SKILL_FILE
        && let Some(parent) = as_path.parent().and_then(Path::file_name)
    {
        return parent.to_string_lossy().into_owned();
    }
    let stem = Path::new(file_name)
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    let stem = stem.strip_suffix(INSTRUCTIONS_SUFFIX).unwrap_or(&stem);
    stem.replace('_', "-")
}

/// The second extension GitHub Copilot's path-specific instruction files carry
/// (`<name>.instructions.md`), which is not part of the rule's name.
const INSTRUCTIONS_SUFFIX: &str = ".instructions";

/// §7.3's description row, plus the `when_to_use` append that row names.
fn description_of(split: &frontmatter::Split) -> String {
    let mut description = split
        .scalar("description")
        .unwrap_or_default()
        .trim()
        .to_owned();
    if let Some(when) = split
        .scalar("when_to_use")
        .map(str::trim)
        .filter(|when| !when.is_empty())
    {
        if description.is_empty() {
            description = when.to_owned();
        } else {
            description.push_str("\n\n");
            description.push_str(when);
        }
    }
    description
}

/// §6 item 11's `format`: the file's shape, not a guessed ecosystem. `globs` alone cannot tell
/// Cursor's rules from Windsurf's from Continue's, and a wrong guess in a provenance column is
/// worse than an honest one — the keys themselves are in `frontmatter` either way.
fn format_of(file_name: &str) -> &'static str {
    if file_name == SKILL_FILE {
        "skill-md"
    } else if Path::new(file_name)
        .extension()
        .is_some_and(|extension| extension == "mdc")
    {
        "mdc"
    } else {
        "markdown"
    }
}

/// The frontmatter as JSON, keeping every key: a scalar and a `Raw` are both strings here, and a
/// list is an array. §5.6's row is "everything, verbatim", and a `Raw` is exactly that.
fn frontmatter_json(split: &frontmatter::Split) -> Map<String, Json> {
    let mut map = Map::new();
    for entry in &split.frontmatter {
        let value = match &entry.value {
            Value::Scalar(text) => Json::String(text.clone()),
            Value::List(items) => Json::Array(items.iter().cloned().map(Json::String).collect()),
            Value::Raw(text) => Json::String(text.clone()),
        };
        map.insert(entry.key.clone(), value);
    }
    map
}

fn issues_json(issues: &[Issue]) -> Json {
    Json::Array(
        issues
            .iter()
            .map(|issue| {
                json!({
                    "key": issue.key,
                    "at": issue.at,
                    "line": issue.line,
                    "message": issue.message,
                })
            })
            .collect(),
    )
}

/// Re-derives the prefill from a stored `skill_version.source`.
///
/// Returns [`ImportPrefill::default`] for a version that was not imported (`{}`), or whose
/// `frontmatter` is missing or not an object — a skill typed in the TUI prefills nothing, and the
/// form looks exactly as it did before this milestone.
#[must_use]
pub fn prefill_from_source(source: &Json) -> ImportPrefill {
    source
        .get("frontmatter")
        .and_then(Json::as_object)
        .map_or_else(ImportPrefill::default, prefill_of)
}

/// §7.3's four prefill rows, applied to a frontmatter map.
///
/// **Precedence, which §7.3 leaves open and this states once:** an always-source beats a
/// glob-source. A Cursor file carrying `alwaysApply: true` beside a `globs` list means "these
/// globs, and also always", and `always` is the narrower thing to prefill — a form seeded with
/// `glob` would silently narrow a skill the file always wanted.
fn prefill_of(frontmatter: &Map<String, Json>) -> ImportPrefill {
    let text = |key: &str| frontmatter.get(key).and_then(Json::as_str).map(str::trim);
    let list = |key: &str| -> Vec<String> {
        match frontmatter.get(key) {
            Some(Json::Array(items)) => items
                .iter()
                .filter_map(Json::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect(),
            Some(Json::String(text)) => text
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    };

    // §7.3 row 1: the four ecosystems' glob keys, split when they are comma strings, taken as they
    // are when they are lists. `applyTo` appears twice in the table and is read value-aware below.
    let mut globs = Vec::new();
    for key in ["paths", "globs", "fileMatchPattern"] {
        globs.extend(list(key));
    }
    if let Some(apply_to) = frontmatter.get("applyTo").and_then(Json::as_str) {
        let apply_to = apply_to.trim();
        if apply_to != "**" {
            globs.extend(
                apply_to
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    globs.dedup();

    // §7.3 row 2: the four always-sources.
    let always = frontmatter
        .get("alwaysApply")
        .and_then(Json::as_str)
        .map(str::trim)
        == Some("true")
        || matches!(text("trigger"), Some("always_on"))
        || matches!(text("inclusion"), Some("always"))
        || frontmatter
            .get("applyTo")
            .and_then(Json::as_str)
            .map(str::trim)
            == Some("**");

    // §7.3 row 4: a file whose activation is the model's choice, or manual, becomes `always` here
    // — and the maintainer is told why, beside the field.
    let model_decided = matches!(text("trigger"), Some("model_decision"));
    let manual = matches!(text("inclusion"), Some("manual"));
    let not_invocable = frontmatter.contains_key("disable-model-invocation");
    let no_activation_key = !frontmatter.keys().any(|key| {
        matches!(
            key.as_str(),
            "paths"
                | "globs"
                | "applyTo"
                | "fileMatchPattern"
                | "alwaysApply"
                | "trigger"
                | "inclusion"
        )
    });
    // "description only" is one of §7.3's four cases, not a summary of the other three: a file
    // that gives a description and no activation rule at all is prefilled `always`, which is also
    // the column default, so the form looks as it always has.
    let no_rule = no_activation_key && frontmatter.contains_key("description");

    let hint = if model_decided {
        Some("this file lets the model decide when to use the skill; htui prefills `always`")
    } else if manual {
        Some("this file is included manually; htui prefills `always`")
    } else if not_invocable {
        Some("this file opts out of model invocation; htui prefills `always`")
    } else if no_rule {
        Some("this file names no activation rule; htui prefills `always`")
    } else {
        None
    };

    let activation = if always || model_decided || manual || not_invocable || no_rule {
        Some(Activation::Always)
    } else if !globs.is_empty() {
        Some(Activation::Glob)
    } else {
        None
    };

    ImportPrefill {
        activation,
        globs,
        languages: list("languages"),
        hint,
    }
}
