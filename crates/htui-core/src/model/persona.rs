//! Agent personas, MOD-26 milestone 1 (plan D2, D3, D8, D9; PRD Q-model, Q-seeds): the registry
//! row, its save-time rules, the frontmatter reader of a persona file and the two seeds.
//!
//! A persona is a named posture — a role text and a narrowing of the tools and permissions the
//! agent row already grants (I-1). It never names a model (I-5): the phase candidate's model is
//! used. The rules here are pure and worded once; the stores call them on every write and the
//! parser calls them on every file (I-2), so a seed is validated the moment it is read.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::frontmatter::{self, FrontmatterError, Value};
use crate::model::ids::PersonaId;
use crate::model::skill::validate_name;
use crate::prompt::digest::sha256_hex;
use crate::store::has_nul;

/// The ten ACP tool kinds, spelled and ordered as `htui_agent::event::ToolKind::ALL` spells them.
/// Core cannot name the agent crate (`htui-agent → htui-core`); `htui_agent::persona`'s
/// `core_tool_kinds_spell_tool_kind_all` pins the two together (plan D2, D10).
pub const TOOL_KINDS: [&str; 10] = [
    "read",
    "edit",
    "delete",
    "move",
    "search",
    "execute",
    "think",
    "fetch",
    "switch_mode",
    "other",
];

/// The kinds a persona may deny (plan D3). `think`, `switch_mode` and `other` are not narrowable.
pub const NARROWABLE_KINDS: [&str; 7] = [
    "read", "edit", "delete", "move", "search", "execute", "fetch",
];

/// The frontmatter keys a persona file may carry (plan D8), in the order a refusal lists them.
pub const FRONTMATTER_KEYS: [&str; 7] = [
    "name",
    "description",
    "tools",
    "disallowed-tools",
    "deny-kinds",
    "command-run",
    "permission-default",
];

/// The prefix of an MCP tool's name, which `allow` refuses (plan D3: `--tools` filters built-ins).
pub const MCP_PREFIX: &str = "mcp__";

/// A row of `persona` (MOD-26 plan D1, D2): one named posture of the global registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Persona {
    /// `persona.id`.
    pub id: PersonaId,
    /// `persona.name`: unique, [`validate_name`]'s alphabet.
    pub name: String,
    /// `persona.description`: the picker's one-liner; never rendered into a prompt.
    pub description: String,
    /// `persona.body`: the role text stage 3 renders ahead of the template (plan D13).
    pub body: String,
    /// `persona.tools`.
    pub tools: PersonaTools,
    /// `persona.permission`.
    pub permission: PersonaPermission,
    /// `persona.created_at`.
    pub created_at: DateTime<Utc>,
    /// `persona.updated_at`: the compare-and-set token of `update_persona`.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of `WriteStore::create_persona` (plan D4): the row minus the store's two stamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPersona {
    /// `persona.id`, minted client-side as a UUIDv7.
    pub id: PersonaId,
    /// `persona.name`.
    pub name: String,
    /// `persona.description`; may be empty.
    pub description: String,
    /// `persona.body`; refused when blank.
    pub body: String,
    /// `persona.tools`.
    pub tools: PersonaTools,
    /// `persona.permission`.
    pub permission: PersonaPermission,
}

/// Edit passed to `WriteStore::update_persona` (plan D4); `None` leaves the column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaPatch {
    /// `persona.name`; a rename is allowed.
    pub name: Option<String>,
    /// `persona.description`.
    pub description: Option<String>,
    /// `persona.body`.
    pub body: Option<String>,
    /// `persona.tools`, replaced whole.
    pub tools: Option<PersonaTools>,
    /// `persona.permission`, replaced whole.
    pub permission: Option<PersonaPermission>,
}

/// `persona.tools` (plan D2, D10): what a persona takes away from the agent row's exposure.
/// `{}` decodes to [`PersonaTools::default`], which narrows nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonaTools {
    /// Built-in tool names to keep; empty keeps every tool the base keeps (`--tools`).
    pub allow: Vec<String>,
    /// Tool names to remove (`--disallowedTools`); MCP tools are named here.
    pub deny: Vec<String>,
    /// ACP tool kinds to deny, each one of [`NARROWABLE_KINDS`].
    pub deny_kinds: Vec<String>,
    /// `false` withdraws `htui`'s `command_run` exposure and the `command_queue` section; `true`
    /// keeps the base (plan D13).
    pub command_run: bool,
}

impl Default for PersonaTools {
    fn default() -> Self {
        Self {
            allow: Vec::new(),
            deny: Vec::new(),
            deny_kinds: Vec::new(),
            command_run: true,
        }
    }
}

/// `persona.permission` (plan D2, D10): deny-only additions to the agent row's policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonaPermission {
    /// The persona's floor for an unmatched request; `None` keeps the base's.
    pub default: Option<PersonaDefault>,
    /// Reject rules, evaluated before the agent row's rules and remembered choices.
    pub rules: Vec<PersonaRule>,
}

/// A persona's `permission.default` (plan D2): never `allow`, by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaDefault {
    /// Park and ask.
    Ask,
    /// Answer with the first reject option.
    Deny,
}

/// One persona rule (plan D2): a matcher and a reject-only answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaRule {
    /// The predicate; named `match` on the wire, as `PermissionRule`'s is.
    #[serde(rename = "match")]
    pub matcher: PersonaMatch,
    /// The reject kind to answer with.
    pub answer: PersonaAnswer,
    /// Why the rule exists; empty records `persona <name>` (B-21).
    #[serde(default)]
    pub reason: String,
}

/// `PermissionMatch`'s four fields (plan D2), core-side.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonaMatch {
    /// Matches `tool_call.tool_kind`, spelled as [`TOOL_KINDS`] spells it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    /// Matches the call's title (ACP carries no tool name, `permission.rs:110-113`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Matches the first path argument by prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    /// Matches the first command argument by prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_prefix: Option<String>,
}

/// A persona rule's answer (plan D2): reject-only, by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaAnswer {
    /// `reject_once`.
    RejectOnce,
    /// `reject_always`.
    RejectAlways,
}

/// One persona as a run froze it at `StartRun` (plan D9, I-3): content only, no id, no stamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPersona {
    /// `persona.name`, what `SnapshotPhase.persona` names.
    pub name: String,
    /// `"sha256:"` + hex over the canonical JSON of `{name, body, tools, permission}`.
    pub digest: String,
    /// `persona.body`.
    pub body: String,
    /// `persona.tools`.
    pub tools: PersonaTools,
    /// `persona.permission`.
    pub permission: PersonaPermission,
}

impl SnapshotPersona {
    /// Freezes `persona` for a run's snapshot (plan D9).
    ///
    /// # Errors
    /// The serialiser's own, which these types cannot produce (no map keys, no floats).
    pub fn freeze(persona: &Persona) -> Result<Self, serde_json::Error> {
        let canonical = Canonical {
            name: &persona.name,
            body: &persona.body,
            tools: &persona.tools,
            permission: &persona.permission,
        };
        let digest = format!("sha256:{}", sha256_hex(&serde_json::to_string(&canonical)?));
        Ok(Self {
            name: persona.name.clone(),
            digest,
            body: persona.body.clone(),
            tools: persona.tools.clone(),
            permission: persona.permission.clone(),
        })
    }
}

/// The digest's input, serialised **typed** in this field order — never through `Value`, whose
/// key order is a feature flag away from changing (`graph.rs`'s `preserve_order` trap). The id and
/// the stamps are left out: two rows with the same content freeze to the same digest.
#[derive(Serialize)]
struct Canonical<'a> {
    name: &'a str,
    body: &'a str,
    tools: &'a PersonaTools,
    permission: &'a PersonaPermission,
}

/// A persona file read by [`parse_file`]: a [`NewPersona`] without its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaFile {
    /// `name`.
    pub name: String,
    /// `description`, `""` when absent.
    pub description: String,
    /// The body (B-16).
    pub body: String,
    /// `tools`, `disallowed-tools`, `deny-kinds`, `command-run`.
    pub tools: PersonaTools,
    /// `permission-default`; rules are not expressible in a file (plan D8).
    pub permission: PersonaPermission,
}

impl PersonaFile {
    /// The insert this file describes, under `id`.
    #[must_use]
    pub fn into_new(self, id: PersonaId) -> NewPersona {
        NewPersona {
            id,
            name: self.name,
            description: self.description,
            body: self.body,
            tools: self.tools,
            permission: self.permission,
        }
    }
}

/// PRD Q-model, plan D8 (I-5).
pub const MODEL_REFUSED: &str =
    "a persona does not set the model; the phase candidate's model is used (MOD-26)";

/// Plan D3: a blank body would render an empty frame that costs tokens and says nothing.
pub const BLANK_PERSONA_BODY: &str = "a persona needs a prompt body";

/// Plan D3: the all-`None` matcher, which would reject every request.
pub const RULE_MATCHES_EVERYTHING: &str =
    "a persona rule with an empty match would deny every request; use `default: deny` instead";

/// Plan D3: a name [`validate_name`] refuses.
#[must_use]
pub fn invalid_persona_name(name: &str) -> String {
    format!(
        "persona.name `{}` must be 1-64 of a-z, 0-9 and single inner hyphens",
        name.escape_debug()
    )
}

/// Plan D3: `list` is `allow` or `deny`; each entry becomes part of one argv value (D11).
#[must_use]
pub fn not_a_tool_name(list: &str, name: &str) -> String {
    format!(
        "persona.tools.{list} entry `{}` is not a tool name: one or more characters, no \
         whitespace, comma or NUL",
        name.escape_debug()
    )
}

/// Plan D3: `--tools` filters built-in tools only.
#[must_use]
pub fn allow_names_an_mcp_tool(name: &str) -> String {
    format!(
        "persona.tools.allow entry `{}` is an MCP tool; `allow` keeps built-in tools only, so \
         deny an MCP tool by name instead",
        name.escape_debug()
    )
}

/// Plan D3: a `deny_kinds` entry outside [`NARROWABLE_KINDS`].
#[must_use]
pub fn kind_not_narrowable(kind: &str) -> String {
    format!(
        "persona.tools.deny_kinds entry `{}` is not one of read, edit, delete, move, search, \
         execute, fetch",
        kind.escape_debug()
    )
}

/// I-4 (plan D12): a phase names a persona its run's snapshot does not carry.
#[must_use]
pub fn persona_not_in_snapshot(persona: &str) -> String {
    format!(
        "persona `{}` is not in the run's snapshot, so the step does not run un-narrowed \
         (MOD-26 I-4)",
        persona.escape_debug()
    )
}

/// Plan D3 over a whole row: name; description NUL; body blank, body NUL; tools; permission.
#[must_use]
pub fn persona_refusal(
    name: &str,
    description: &str,
    body: &str,
    tools: &PersonaTools,
    permission: &PersonaPermission,
) -> Option<String> {
    if !validate_name(name) {
        return Some(invalid_persona_name(name));
    }
    if description.contains('\0') {
        return Some(has_nul("persona.description"));
    }
    body_refusal(body)
        .or_else(|| tools_refusal(tools))
        .or_else(|| permission_refusal(permission))
}

/// [`persona_refusal`] over a [`NewPersona`] (`create_persona`).
#[must_use]
pub fn new_persona_refusal(new: &NewPersona) -> Option<String> {
    persona_refusal(
        &new.name,
        &new.description,
        &new.body,
        &new.tools,
        &new.permission,
    )
}

/// Plan D3 over a [`PersonaPatch`]: each `Some` field through the same rule, in the same order.
#[must_use]
pub fn persona_patch_refusal(patch: &PersonaPatch) -> Option<String> {
    if let Some(name) = &patch.name
        && !validate_name(name)
    {
        return Some(invalid_persona_name(name));
    }
    if patch
        .description
        .as_deref()
        .is_some_and(|description| description.contains('\0'))
    {
        return Some(has_nul("persona.description"));
    }
    patch
        .body
        .as_deref()
        .and_then(body_refusal)
        .or_else(|| patch.tools.as_ref().and_then(tools_refusal))
        .or_else(|| patch.permission.as_ref().and_then(permission_refusal))
}

/// A blank body first, then a NUL (`persona.body`).
fn body_refusal(body: &str) -> Option<String> {
    if body.trim().is_empty() {
        return Some(BLANK_PERSONA_BODY.to_owned());
    }
    body.contains('\0').then(|| has_nul("persona.body"))
}

/// allow entries (tool name, then MCP prefix), deny entries (tool name), deny_kinds (closed list).
fn tools_refusal(tools: &PersonaTools) -> Option<String> {
    for name in &tools.allow {
        if !is_tool_name(name) {
            return Some(not_a_tool_name("allow", name));
        }
        if name.starts_with(MCP_PREFIX) {
            return Some(allow_names_an_mcp_tool(name));
        }
    }
    if let Some(name) = tools.deny.iter().find(|name| !is_tool_name(name)) {
        return Some(not_a_tool_name("deny", name));
    }
    tools
        .deny_kinds
        .iter()
        .find(|kind| !NARROWABLE_KINDS.contains(&kind.as_str()))
        .map(|kind| kind_not_narrowable(kind))
}

/// One rule at a time: the all-`None` matcher, then a NUL in any matcher string or the reason
/// (B-9: Postgres `JSONB` refuses `\u0000` inside a string, so MemStore must refuse it too).
fn permission_refusal(permission: &PersonaPermission) -> Option<String> {
    for rule in &permission.rules {
        let matcher = &rule.matcher;
        let strings = [
            &matcher.tool_kind,
            &matcher.tool_name,
            &matcher.path_prefix,
            &matcher.command_prefix,
        ];
        if strings.iter().all(|string| string.is_none()) {
            return Some(RULE_MATCHES_EVERYTHING.to_owned());
        }
        if strings
            .iter()
            .filter_map(|string| string.as_deref())
            .chain([rule.reason.as_str()])
            .any(|string| string.contains('\0'))
        {
            return Some(has_nul("persona.permission.rules"));
        }
    }
    None
}

/// Non-empty, no `char::is_whitespace`, no `,`, no `\0`: one entry of one argv value (D11).
fn is_tool_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c == ',' || c == '\0')
}

/// Why a persona file was refused (B-24). `Display` is the sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PersonaFileError {
    /// No opening fence, or no closing fence within `MAX_FRONTMATTER_LINES`.
    #[error(transparent)]
    Fence(#[from] FrontmatterError),
    /// The reader could not read a line (any `frontmatter::Issue`; fail closed).
    #[error("persona frontmatter line {line}: {message}")]
    Unreadable {
        /// The 1-based line the reader stopped at.
        line: usize,
        /// The reader's own sentence.
        message: String,
    },
    /// `model:` (PRD Q-model).
    #[error("a persona does not set the model; the phase candidate's model is used (MOD-26)")]
    Model,
    /// Any other key D8 does not name.
    #[error(
        "`{key}` is not a persona key; a persona file takes name, description, tools, \
         disallowed-tools, deny-kinds, command-run and permission-default"
    )]
    UnknownKey {
        /// The key as written.
        key: String,
    },
    /// A key written twice.
    #[error("persona key `{key}` appears more than once")]
    Duplicate {
        /// The repeated key.
        key: String,
    },
    /// A list, block or map value.
    #[error("persona key `{key}` takes one `key: value` line; write a list as `a, b, c`")]
    NotOneLine {
        /// The key whose value is not one line.
        key: String,
    },
    /// No `name:` line.
    #[error("a persona file needs a `name`")]
    MissingName,
    /// `command-run` other than `true`/`false`.
    #[error("`command-run` is `true` or `false`, not `{value}`")]
    CommandRun {
        /// The value as written.
        value: String,
    },
    /// `permission-default` other than `ask`/`deny`.
    #[error("`permission-default` is `ask` or `deny`, not `{value}`")]
    PermissionDefault {
        /// The value as written.
        value: String,
    },
    /// A D3 refusal of the parsed persona.
    #[error("{0}")]
    Refused(String),
}

/// Reads one persona file (plan D8): the seeds now, M2's import later.
///
/// # Errors
/// Every [`PersonaFileError`] variant; nothing is half-read.
pub fn parse_file(text: &str) -> Result<PersonaFile, PersonaFileError> {
    let split = frontmatter::split(text)?;
    if let Some(issue) = split.issues.first() {
        return Err(PersonaFileError::Unreadable {
            line: issue.line,
            message: issue.message.clone(),
        });
    }

    let mut seen: Vec<&str> = Vec::new();
    let mut name: Option<String> = None;
    let mut description = String::new();
    let mut tools = PersonaTools::default();
    let mut permission = PersonaPermission::default();
    for entry in &split.frontmatter {
        let key = entry.key.as_str();
        if key == "model" {
            return Err(PersonaFileError::Model);
        }
        if !FRONTMATTER_KEYS.contains(&key) {
            return Err(PersonaFileError::UnknownKey {
                key: key.to_owned(),
            });
        }
        if seen.contains(&key) {
            return Err(PersonaFileError::Duplicate {
                key: key.to_owned(),
            });
        }
        seen.push(key);
        let not_one_line = || PersonaFileError::NotOneLine {
            key: key.to_owned(),
        };
        let Value::Scalar(value) = &entry.value else {
            return Err(not_one_line());
        };
        if value.contains('\n') || is_block_scalar(text, entry.line) {
            return Err(not_one_line());
        }
        match key {
            "name" => name = Some(value.clone()),
            "description" => description.clone_from(value),
            "tools" => tools.allow = list_of(value),
            "disallowed-tools" => tools.deny = list_of(value),
            "deny-kinds" => tools.deny_kinds = list_of(value),
            "command-run" => {
                tools.command_run = match value.as_str() {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(PersonaFileError::CommandRun {
                            value: value.clone(),
                        });
                    }
                };
            }
            "permission-default" => {
                permission.default = Some(match value.as_str() {
                    "ask" => PersonaDefault::Ask,
                    "deny" => PersonaDefault::Deny,
                    _ => {
                        return Err(PersonaFileError::PermissionDefault {
                            value: value.clone(),
                        });
                    }
                });
            }
            // Unreachable after the `FRONTMATTER_KEYS` check; refused rather than ignored, so a
            // key added to the list without an arm here fails closed.
            _ => {
                return Err(PersonaFileError::UnknownKey {
                    key: key.to_owned(),
                });
            }
        }
    }

    let name = name.ok_or(PersonaFileError::MissingName)?;
    // B-16: the reader already dropped the fence's own newline, normalised CRLF and ended the body
    // with exactly one LF; the blank line conventionally written after the fence goes too.
    let body = split
        .body
        .strip_prefix('\n')
        .map_or_else(|| split.body.clone(), str::to_owned);
    let file = PersonaFile {
        name,
        description,
        body,
        tools,
        permission,
    };
    persona_refusal(
        &file.name,
        &file.description,
        &file.body,
        &file.tools,
        &file.permission,
    )
    .map_or(Ok(file), |refusal| Err(PersonaFileError::Refused(refusal)))
}

/// Whether the key on 1-based `line` of `text` opens a `|` or `>` block scalar (B-1). The reader
/// resolves a chomped one (`|-`, `>-`) to a newline-free scalar, so the newline check alone does
/// not see it; the source line does. A plain scalar cannot begin with either indicator, so a
/// value written that way is refused rather than read as text.
fn is_block_scalar(text: &str, line: usize) -> bool {
    text.split('\n')
        .nth(line.saturating_sub(1))
        .and_then(|source| source.split_once(':'))
        .is_some_and(|(_, rest)| rest.trim_start_matches([' ', '\t']).starts_with(['|', '>']))
}

/// A one-line list value (plan D8): comma-separated, each item trimmed, empty items dropped, so
/// `tools:` alone is the empty list.
fn list_of(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The two seed personas (plan D7, OQ-4), parsed from the compiled-in files with fresh ids.
///
/// # Panics
/// Never in a shipped build: the files are compile-time constants and
/// `seed_rows_are_the_two_seed_files` parses both.
#[must_use]
pub fn seed_rows(now: DateTime<Utc>) -> Vec<Persona> {
    [
        include_str!("../../seeds/persona_reviewer.md"),
        include_str!("../../seeds/persona_architect.md"),
    ]
    .into_iter()
    .map(|text| {
        let file = parse_file(text).expect("a compiled-in persona seed parses");
        Persona {
            id: PersonaId::new(),
            name: file.name,
            description: file.description,
            body: file.body,
            tools: file.tools,
            permission: file.permission,
            created_at: now,
            updated_at: now,
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    use super::*;

    fn parse(front: &str, body: &str) -> Result<PersonaFile, PersonaFileError> {
        parse_file(&format!("---\n{front}---\n{body}"))
    }

    fn at(minute: u32) -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600 + i64::from(minute) * 60, 0).expect("in range")
    }

    fn row(id: PersonaId, stamp: DateTime<Utc>) -> Persona {
        Persona {
            id,
            name: "reviewer".to_owned(),
            description: "Reviews".to_owned(),
            body: "You review.\n".to_owned(),
            tools: PersonaTools {
                deny_kinds: vec!["edit".to_owned()],
                ..PersonaTools::default()
            },
            permission: PersonaPermission {
                default: Some(PersonaDefault::Ask),
                rules: vec![rule(PersonaMatch {
                    command_prefix: Some("rm".to_owned()),
                    ..PersonaMatch::default()
                })],
            },
            created_at: stamp,
            updated_at: stamp,
        }
    }

    fn rule(matcher: PersonaMatch) -> PersonaRule {
        PersonaRule {
            matcher,
            answer: PersonaAnswer::RejectOnce,
            reason: String::new(),
        }
    }

    /// A valid row's fields, varied one at a time by [`every_widening_shape_is_refused`].
    struct Base {
        name: String,
        description: String,
        body: String,
        tools: PersonaTools,
        permission: PersonaPermission,
    }

    impl Base {
        fn new() -> Self {
            Self {
                name: "reviewer".to_owned(),
                description: "Reviews code".to_owned(),
                body: "You review.\n".to_owned(),
                tools: PersonaTools {
                    allow: vec!["Read".to_owned(), "Grep".to_owned()],
                    deny: vec!["mcp__gortex__edit".to_owned()],
                    deny_kinds: vec!["edit".to_owned(), "execute".to_owned()],
                    command_run: false,
                },
                permission: PersonaPermission {
                    default: Some(PersonaDefault::Deny),
                    rules: vec![rule(PersonaMatch {
                        path_prefix: Some("/etc".to_owned()),
                        ..PersonaMatch::default()
                    })],
                },
            }
        }

        fn refusal(&self) -> Option<String> {
            persona_refusal(
                &self.name,
                &self.description,
                &self.body,
                &self.tools,
                &self.permission,
            )
        }
    }

    #[test]
    fn a_claude_agents_shaped_file_parses() {
        let file = parse_file(
            "---\nname: code-reviewer\ndescription: Reviews code\ntools: Read, Grep, Glob\n---\n\n\
             You review.\n",
        )
        .expect("a Claude-agents-shaped file parses");

        assert_eq!(file.name, "code-reviewer");
        assert_eq!(file.description, "Reviews code");
        assert_eq!(file.tools.allow, ["Read", "Grep", "Glob"]);
        assert!(file.tools.deny.is_empty());
        assert!(file.tools.deny_kinds.is_empty());
        assert!(file.tools.command_run);
        assert_eq!(file.permission.default, None);
        assert!(file.permission.rules.is_empty());
        assert_eq!(file.body, "You review.\n");

        let id = PersonaId::new();
        let new = file.clone().into_new(id);
        assert_eq!(new.id, id);
        assert_eq!(new.name, file.name);
        assert_eq!(new.body, file.body);
    }

    #[test]
    fn the_model_key_is_refused_with_its_sentence() {
        assert_eq!(
            parse("name: reviewer\nmodel: opus\n", "body\n"),
            Err(PersonaFileError::Model)
        );
        assert_eq!(PersonaFileError::Model.to_string(), MODEL_REFUSED);
        assert_eq!(
            parse("name: reviewer\nmodel: opus\ncolor: blue\n", "body\n"),
            Err(PersonaFileError::Model),
            "`model` is refused with its own sentence before a later unknown key"
        );
    }

    /// MOD-26 review N3: the persona name is escaped like every sibling sentence's interpolation,
    /// so a stored refusal is one line whatever the snapshot's name holds.
    #[test]
    fn the_snapshot_refusal_escapes_the_persona_name() {
        let sentence = persona_not_in_snapshot("rev\niewer");
        assert!(
            sentence.starts_with("persona `rev\\niewer` is not in the run's snapshot"),
            "the name is escape_debug'd: {sentence}"
        );
        assert!(!sentence.contains('\n'), "one line: {sentence:?}");
        assert_eq!(
            persona_not_in_snapshot("reviewer"),
            "persona `reviewer` is not in the run's snapshot, so the step does not run \
             un-narrowed (MOD-26 I-4)",
            "a plain name reads as before"
        );
    }

    #[test]
    fn an_unknown_key_is_refused_by_name() {
        let refused = parse("name: reviewer\ncolor: blue\n", "body\n");
        assert_eq!(
            refused,
            Err(PersonaFileError::UnknownKey {
                key: "color".to_owned()
            })
        );
        let sentence = refused.expect_err("refused").to_string();
        assert!(sentence.contains("`color`"), "{sentence}");
        assert!(sentence.contains("permission-default"), "{sentence}");
    }

    #[test]
    fn a_missing_closing_fence_is_refused() {
        assert_eq!(
            parse_file("---\nname: reviewer\nbody\n"),
            Err(PersonaFileError::Fence(FrontmatterError::Unterminated {
                at: 0
            }))
        );
        assert_eq!(
            parse_file("name: reviewer\n---\nbody\n"),
            Err(PersonaFileError::Fence(FrontmatterError::NoFence))
        );
    }

    #[test]
    fn a_repeated_key_is_refused() {
        assert_eq!(
            parse("name: reviewer\nname: architect\n", "body\n"),
            Err(PersonaFileError::Duplicate {
                key: "name".to_owned()
            })
        );
    }

    #[test]
    fn a_list_block_or_map_value_is_refused() {
        let not_one_line = |key: &str| {
            Err(PersonaFileError::NotOneLine {
                key: key.to_owned(),
            })
        };
        assert_eq!(
            parse("name: reviewer\ntools: [Read]\n", "body\n"),
            not_one_line("tools")
        );
        assert_eq!(
            parse("name: reviewer\ntools:\n  - Read\n", "body\n"),
            not_one_line("tools")
        );
        assert_eq!(
            parse("name: reviewer\ndescription: |\n  two\n  lines\n", "body\n"),
            not_one_line("description")
        );
        // A chomped block scalar resolves to a newline-free string; it is still refused by key.
        assert_eq!(
            parse(
                "name: reviewer\ndescription: >-\n  two\n  lines\n",
                "body\n"
            ),
            not_one_line("description")
        );
        assert_eq!(
            parse("name: reviewer\ndescription: >-\n  one\n", "body\n"),
            not_one_line("description")
        );
        assert_eq!(
            parse("name: reviewer\ntools: |-\n  Read, Grep\n", "body\n"),
            not_one_line("tools")
        );
        assert_eq!(
            parse("name: reviewer\ntools: |-\n", "body\n"),
            not_one_line("tools")
        );
        assert_eq!(
            parse("name: reviewer\r\ndescription: >+\r\n  one\r\n", "body\n"),
            not_one_line("description")
        );
        assert_eq!(
            parse("name: reviewer\ndescription:\n  nested: map\n", "body\n"),
            not_one_line("description")
        );

        let quoted = parse("name: reviewer\ndescription: \"a, b: c\"\n", "body\n")
            .expect("a quoted scalar is one line");
        assert_eq!(quoted.description, "a, b: c");
    }

    #[test]
    fn command_run_and_permission_default_take_only_their_literals() {
        let off = parse("name: reviewer\ncommand-run: false\n", "body\n").expect("parses");
        assert!(!off.tools.command_run);
        let on = parse("name: reviewer\ncommand-run: true\n", "body\n").expect("parses");
        assert!(on.tools.command_run);
        assert_eq!(
            parse("name: reviewer\ncommand-run: no\n", "body\n"),
            Err(PersonaFileError::CommandRun {
                value: "no".to_owned()
            })
        );

        let deny = parse("name: reviewer\npermission-default: deny\n", "body\n").expect("parses");
        assert_eq!(deny.permission.default, Some(PersonaDefault::Deny));
        let ask = parse("name: reviewer\npermission-default: ask\n", "body\n").expect("parses");
        assert_eq!(ask.permission.default, Some(PersonaDefault::Ask));
        assert_eq!(
            parse("name: reviewer\npermission-default: allow\n", "body\n"),
            Err(PersonaFileError::PermissionDefault {
                value: "allow".to_owned()
            })
        );
    }

    #[test]
    fn the_body_drops_one_leading_blank_line() {
        let two = parse("name: reviewer\n", "\n\nYou review.\n").expect("parses");
        assert_eq!(two.body, "\nYou review.\n");

        let crlf = parse_file("---\r\nname: reviewer\r\n---\r\n\r\nYou review.\r\nTwice.\r\n")
            .expect("a CRLF file parses");
        assert_eq!(crlf.name, "reviewer");
        assert_eq!(crlf.body, "You review.\nTwice.\n");

        let empty = parse("name: reviewer\ntools:\n", "You review.\n").expect("parses");
        assert!(
            empty.tools.allow.is_empty(),
            "`tools:` alone is the empty list"
        );

        assert_eq!(
            parse("description: d\n", "body\n"),
            Err(PersonaFileError::MissingName)
        );
    }

    #[test]
    fn a_parsed_file_meets_the_save_rules() {
        assert_eq!(
            parse("name: reviewer\ndeny-kinds: think\n", "body\n"),
            Err(PersonaFileError::Refused(kind_not_narrowable("think")))
        );
        assert_eq!(
            parse("name: reviewer\ntools: mcp__gortex__search\n", "body\n"),
            Err(PersonaFileError::Refused(allow_names_an_mcp_tool(
                "mcp__gortex__search"
            )))
        );
        assert_eq!(
            parse("name: Reviewer\n", "body\n"),
            Err(PersonaFileError::Refused(invalid_persona_name("Reviewer")))
        );
        assert_eq!(
            parse("name: reviewer\n", ""),
            Err(PersonaFileError::Refused(BLANK_PERSONA_BODY.to_owned()))
        );
    }

    #[test]
    fn every_widening_shape_is_refused() {
        assert_eq!(Base::new().refusal(), None, "the valid base is accepted");

        let mut base = Base::new();
        base.name = "Bad Name".to_owned();
        assert_eq!(base.refusal(), Some(invalid_persona_name("Bad Name")));

        let mut base = Base::new();
        base.description = "a\0b".to_owned();
        assert_eq!(base.refusal(), Some(has_nul("persona.description")));

        let mut base = Base::new();
        base.body = " \n\t\n".to_owned();
        assert_eq!(base.refusal(), Some(BLANK_PERSONA_BODY.to_owned()));

        let mut base = Base::new();
        base.body = "You\0 review".to_owned();
        assert_eq!(base.refusal(), Some(has_nul("persona.body")));

        for bad in ["", "Read Grep", "Read,Grep", "Re\0ad"] {
            let mut base = Base::new();
            base.tools.allow.push(bad.to_owned());
            assert_eq!(
                base.refusal(),
                Some(not_a_tool_name("allow", bad)),
                "{bad:?}"
            );
        }

        let mut base = Base::new();
        base.tools.allow.push("mcp__x__y".to_owned());
        assert_eq!(base.refusal(), Some(allow_names_an_mcp_tool("mcp__x__y")));

        let mut base = Base::new();
        base.tools.deny.push("Bash Output".to_owned());
        assert_eq!(base.refusal(), Some(not_a_tool_name("deny", "Bash Output")));

        for kind in ["think", "switch_mode", "other", "Edit"] {
            let mut base = Base::new();
            base.tools.deny_kinds.push(kind.to_owned());
            assert_eq!(base.refusal(), Some(kind_not_narrowable(kind)), "{kind}");
        }

        let mut base = Base::new();
        base.permission.rules.push(rule(PersonaMatch::default()));
        assert_eq!(base.refusal(), Some(RULE_MATCHES_EVERYTHING.to_owned()));

        let mut base = Base::new();
        base.permission.rules.push(rule(PersonaMatch {
            path_prefix: Some("/e\0tc".to_owned()),
            ..PersonaMatch::default()
        }));
        assert_eq!(base.refusal(), Some(has_nul("persona.permission.rules")));

        let mut base = Base::new();
        base.permission.rules.push(PersonaRule {
            reason: "no\0".to_owned(),
            ..rule(PersonaMatch {
                tool_kind: Some("execute".to_owned()),
                ..PersonaMatch::default()
            })
        });
        assert_eq!(base.refusal(), Some(has_nul("persona.permission.rules")));
    }

    #[test]
    fn a_patch_is_checked_field_by_field() {
        assert_eq!(persona_patch_refusal(&PersonaPatch::default()), None);

        let cases = [
            (
                PersonaPatch {
                    name: Some("Bad".to_owned()),
                    ..PersonaPatch::default()
                },
                invalid_persona_name("Bad"),
            ),
            (
                PersonaPatch {
                    description: Some("\0".to_owned()),
                    ..PersonaPatch::default()
                },
                has_nul("persona.description"),
            ),
            (
                PersonaPatch {
                    body: Some("   ".to_owned()),
                    ..PersonaPatch::default()
                },
                BLANK_PERSONA_BODY.to_owned(),
            ),
            (
                PersonaPatch {
                    body: Some("a\0".to_owned()),
                    ..PersonaPatch::default()
                },
                has_nul("persona.body"),
            ),
            (
                PersonaPatch {
                    tools: Some(PersonaTools {
                        deny_kinds: vec!["think".to_owned()],
                        ..PersonaTools::default()
                    }),
                    ..PersonaPatch::default()
                },
                kind_not_narrowable("think"),
            ),
            (
                PersonaPatch {
                    permission: Some(PersonaPermission {
                        default: None,
                        rules: vec![rule(PersonaMatch::default())],
                    }),
                    ..PersonaPatch::default()
                },
                RULE_MATCHES_EVERYTHING.to_owned(),
            ),
        ];
        for (patch, sentence) in cases {
            assert_eq!(persona_patch_refusal(&patch), Some(sentence), "{patch:?}");
        }

        let valid = PersonaPatch {
            name: Some("architect".to_owned()),
            description: Some(String::new()),
            body: Some("Design.\n".to_owned()),
            tools: Some(PersonaTools::default()),
            permission: Some(PersonaPermission::default()),
        };
        assert_eq!(persona_patch_refusal(&valid), None);
    }

    fn refuses_an_unknown_key<T: Serialize + DeserializeOwned + std::fmt::Debug>(value: &T) {
        let mut json = serde_json::to_value(value).expect("serialises");
        serde_json::from_value::<T>(json.clone()).expect("round-trips without the key");
        json.as_object_mut()
            .expect("an object")
            .insert("color".to_owned(), 1.into());
        let refused = serde_json::from_value::<T>(json).expect_err("an unknown key is refused");
        assert!(refused.to_string().contains("color"), "{refused}");
    }

    #[test]
    fn every_persona_shape_refuses_an_unknown_key() {
        let persona = row(PersonaId::new(), at(0));
        refuses_an_unknown_key(&persona);
        refuses_an_unknown_key(&NewPersona {
            id: persona.id,
            name: persona.name.clone(),
            description: persona.description.clone(),
            body: persona.body.clone(),
            tools: persona.tools.clone(),
            permission: persona.permission.clone(),
        });
        refuses_an_unknown_key(&PersonaPatch::default());
        refuses_an_unknown_key(&persona.tools);
        refuses_an_unknown_key(&persona.permission);
        refuses_an_unknown_key(&persona.permission.rules[0]);
        refuses_an_unknown_key(&persona.permission.rules[0].matcher);
        refuses_an_unknown_key(&SnapshotPersona {
            name: persona.name.clone(),
            digest: "sha256:00".to_owned(),
            body: persona.body.clone(),
            tools: persona.tools.clone(),
            permission: persona.permission.clone(),
        });
    }

    #[test]
    fn an_empty_blob_narrows_nothing() {
        let tools: PersonaTools = serde_json::from_str("{}").expect("`{}` decodes");
        assert_eq!(
            tools,
            PersonaTools {
                allow: Vec::new(),
                deny: Vec::new(),
                deny_kinds: Vec::new(),
                command_run: true,
            }
        );
        assert_eq!(tools, PersonaTools::default());
        let permission: PersonaPermission = serde_json::from_str("{}").expect("`{}` decodes");
        assert_eq!(permission, PersonaPermission::default());
    }

    #[test]
    fn seed_rows_are_the_two_seed_files() {
        let now = at(7);
        let seeds = seed_rows(now);
        let names: Vec<&str> = seeds.iter().map(|seed| seed.name.as_str()).collect();
        assert_eq!(names, ["reviewer", "architect"]);
        for seed in &seeds {
            assert_eq!(
                seed.tools.deny_kinds,
                ["edit", "delete", "move"],
                "{}",
                seed.name
            );
            assert!(seed.tools.allow.is_empty(), "{}", seed.name);
            assert!(seed.tools.deny.is_empty(), "{}", seed.name);
            assert!(seed.tools.command_run, "{}", seed.name);
            assert_eq!(seed.permission.default, None, "{}", seed.name);
            assert!(seed.permission.rules.is_empty(), "{}", seed.name);
            assert!(seed.body.starts_with("You are the "), "{}", seed.name);
            assert!(!seed.description.is_empty(), "{}", seed.name);
            assert_eq!(
                (seed.created_at, seed.updated_at),
                (now, now),
                "{}",
                seed.name
            );
            let new = NewPersona {
                id: seed.id,
                name: seed.name.clone(),
                description: seed.description.clone(),
                body: seed.body.clone(),
                tools: seed.tools.clone(),
                permission: seed.permission.clone(),
            };
            assert_eq!(new_persona_refusal(&new), None, "{}", seed.name);
        }
    }

    #[test]
    fn seed_rows_mint_fresh_ids_per_call() {
        let first = seed_rows(at(0));
        let second = seed_rows(at(0));
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), first.len());
        for (one, two) in first.iter().zip(&second) {
            assert_eq!(one.name, two.name);
            assert_ne!(one.id, two.id);
        }
        assert_ne!(first[0].id, first[1].id);
    }

    #[test]
    fn freeze_digests_the_content_and_not_the_row() {
        let one = SnapshotPersona::freeze(&row(PersonaId::new(), at(0))).expect("freezes");
        let hex = one
            .digest
            .strip_prefix("sha256:")
            .expect("the digest names its algorithm");
        assert_eq!(hex.len(), 64, "{}", one.digest);
        assert!(
            hex.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "{}",
            one.digest
        );

        let two = SnapshotPersona::freeze(&row(PersonaId::new(), at(9))).expect("freezes");
        assert_eq!(one.digest, two.digest, "id and stamps are not content");

        let mut edited = row(PersonaId::new(), at(0));
        edited.body.push_str("And more.\n");
        let three = SnapshotPersona::freeze(&edited).expect("freezes");
        assert_ne!(one.digest, three.digest, "a body edit is a new digest");

        let persona = row(PersonaId::new(), at(0));
        assert_eq!(one.name, persona.name);
        assert_eq!(one.body, persona.body);
        assert_eq!(one.tools, persona.tools);
        assert_eq!(one.permission, persona.permission);
    }

    #[test]
    fn the_tool_kind_lists_are_closed() {
        assert_eq!(TOOL_KINDS.len(), 10);
        assert_eq!(NARROWABLE_KINDS.len(), 7);
        for kind in NARROWABLE_KINDS {
            assert!(TOOL_KINDS.contains(&kind), "{kind}");
        }
        for list in [
            &TOOL_KINDS[..],
            &NARROWABLE_KINDS[..],
            &FRONTMATTER_KEYS[..],
        ] {
            for (index, entry) in list.iter().enumerate() {
                assert!(!list[index + 1..].contains(entry), "{entry} twice");
            }
        }
    }
}
