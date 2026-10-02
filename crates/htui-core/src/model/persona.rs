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

/// MOD-26 milestone 2 D18: a rule's `tool_kind` outside [`TOOL_KINDS`]. A misspelt kind matches
/// nothing (`permission.rs` compares the text), so the rule would silently do nothing.
#[must_use]
pub fn rule_kind_unknown(kind: &str) -> String {
    format!(
        "persona.permission.rules entry kind `{}` is not one of read, edit, delete, move, search, \
         execute, think, fetch, switch_mode, other",
        kind.escape_debug()
    )
}

/// I-4 (plan D12; MOD-26 milestone 2 D16, review N4): a phase names a persona its run's snapshot
/// does not carry. `Display` is the sentence `RunFailure::PromptRefused.reason` stores, byte for
/// byte as milestone 1 wrote it; the name is `escape_debug`'d so a stored reason is one line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "persona `{}` is not in the run's snapshot, so the step does not run un-narrowed \
     (MOD-26 I-4)",
    .persona.escape_debug()
)]
pub struct PersonaNotInSnapshot {
    /// The name `SnapshotPhase.persona` carries, unescaped.
    pub persona: String,
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
/// (B-9: Postgres `JSONB` refuses `\u0000` inside a string, so MemStore must refuse it too); then
/// (MOD-26 M2 D18, B-1) a `tool_kind` outside [`TOOL_KINDS`], checked once every rule has passed
/// the two checks above, so a rule set milestone 1 refused keeps its sentence.
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
    permission
        .rules
        .iter()
        .filter_map(|rule| rule.matcher.tool_kind.as_deref())
        .find(|kind| !TOOL_KINDS.contains(kind))
        .map(rule_kind_unknown)
}

/// Non-empty, no `char::is_whitespace`, no `,`, no `\0`: one entry of one argv value (D11).
fn is_tool_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c == ',' || c == '\0')
}

// ---- MOD-26 milestone 2 D17: the rule lines of the Settings › Personas rules editor ----------

/// The four matcher keys of a rule line (MOD-26 M2 D17), in the order [`format_rules`] writes
/// them: `kind` → `tool_kind`, `name` → `tool_name`, `path` → `path_prefix`, `command` →
/// `command_prefix`.
pub const RULE_KEYS: [&str; 4] = ["kind", "name", "path", "command"];

/// Why [`parse_rules`] refused a rules text (MOD-26 M2 D17): the 1-based line and one sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("rules line {line}: {message}")]
pub struct RuleLineError {
    /// The 1-based line of the text.
    pub line: usize,
    /// One sentence; for a kind outside [`TOOL_KINDS`], [`rule_kind_unknown`]'s — the store's.
    pub message: String,
}

/// Reads the Settings › Personas rules editor (MOD-26 M2 D17, OQ-10): one rule per line, blank
/// lines and `#` lines ignored. Pure. The result still goes through the store's rules (I-8):
/// an empty match or a NUL is refused there, not here.
///
/// # Errors
/// The first [`RuleLineError`], in line order.
pub fn parse_rules(text: &str) -> Result<Vec<PersonaRule>, RuleLineError> {
    let mut rules = Vec::new();
    for (index, raw) in text.split('\n').enumerate() {
        // One trailing `\r`: a pasted CRLF.
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let chars: Vec<char> = line.chars().collect();
        match rule_line(&chars) {
            Ok(Some(rule)) => rules.push(rule),
            Ok(None) => {}
            Err(message) => {
                return Err(RuleLineError {
                    line: index + 1,
                    message,
                });
            }
        }
    }
    Ok(rules)
}

/// Writes `rules` one per line, `\n`-separated, no trailing newline, each in the shortest form
/// [`parse_rules`] reads back to the same rule. Pure.
#[must_use]
pub fn format_rules(rules: &[PersonaRule]) -> String {
    rules.iter().map(rule_text).collect::<Vec<_>>().join("\n")
}

/// D17 E-7.
const UNCLOSED_QUOTE: &str = "a quoted string is not closed";

/// One rule line's cursor, over its chars.
struct LineCursor<'a> {
    chars: &'a [char],
    at: usize,
}

impl LineCursor<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    /// Skips `char::is_whitespace`; whether anything was skipped.
    fn skip_ws(&mut self) -> bool {
        let start = self.at;
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
        self.at > start
    }

    /// The maximal run of chars `stop` does not end.
    fn run(&mut self, stop: impl Fn(char) -> bool) -> String {
        let start = self.at;
        while self.peek().is_some_and(|c| !stop(c)) {
            self.at += 1;
        }
        self.chars[start..self.at].iter().collect()
    }

    /// A quoted string, the cursor on its opening `"`: every char literal but `"` and `\`, and
    /// the five escapes (D17, B-2).
    fn quoted(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(UNCLOSED_QUOTE.to_owned()),
                Some('"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some('\\') => {
                    self.at += 1;
                    out.push(match self.peek() {
                        None => return Err(UNCLOSED_QUOTE.to_owned()),
                        Some('"') => '"',
                        Some('\\') => '\\',
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some(other) => {
                            return Err(format!(
                                "`\\{}` is not an escape; use \\\", \\\\, \\n, \\t or \\r",
                                other.escape_debug()
                            ));
                        }
                    });
                    self.at += 1;
                }
                Some(c) => {
                    out.push(c);
                    self.at += 1;
                }
            }
        }
    }
}

/// Ends the answer word and a bare value (D17 rules 2 and 5).
fn ends_word(c: char) -> bool {
    c.is_whitespace() || c == '#'
}

/// Ends a key (D17 rule 4).
fn ends_key(c: char) -> bool {
    c.is_whitespace() || matches!(c, '#' | '=' | '"')
}

/// One line of [`parse_rules`]: `Ok(None)` for a blank or comment line, `Err` the E-n sentence.
fn rule_line(chars: &[char]) -> Result<Option<PersonaRule>, String> {
    let mut cursor = LineCursor { chars, at: 0 };
    cursor.skip_ws();
    if matches!(cursor.peek(), None | Some('#')) {
        return Ok(None);
    }
    let word = cursor.run(ends_word);
    let answer = match word.as_str() {
        "reject_once" => PersonaAnswer::RejectOnce,
        "reject_always" => PersonaAnswer::RejectAlways,
        _ => {
            return Err(format!(
                "a rule starts with reject_once or reject_always, not `{}`",
                word.escape_debug()
            ));
        }
    };
    let mut matcher = PersonaMatch::default();
    let mut reason = String::new();
    let mut after = format!("`{}`", word.escape_debug());
    loop {
        let spaced = cursor.skip_ws();
        match cursor.peek() {
            None => break,
            Some('#') => {
                reason = rule_reason(&mut cursor)?;
                break;
            }
            Some(_) if !spaced => {
                return Err(format!(
                    "expected a space, `#` or the end of the line after {after}"
                ));
            }
            Some(_) => {}
        }
        let start = cursor.at;
        let key = cursor.run(ends_key);
        if key.is_empty() || cursor.peek() != Some('=') {
            cursor.at = start;
            let token = cursor.run(char::is_whitespace);
            return Err(format!("`{}` is not `key=value`", token.escape_debug()));
        }
        cursor.at += 1;
        let slot = match key.as_str() {
            "kind" => &mut matcher.tool_kind,
            "name" => &mut matcher.tool_name,
            "path" => &mut matcher.path_prefix,
            "command" => &mut matcher.command_prefix,
            _ => {
                return Err(format!(
                    "`{}` is not a rule key; a rule takes kind, name, path and command",
                    key.escape_debug()
                ));
            }
        };
        if slot.is_some() {
            return Err(format!("rule key `{key}` appears more than once"));
        }
        let value = if cursor.peek() == Some('"') {
            cursor.quoted()?
        } else {
            let bare = cursor.run(ends_word);
            if bare.is_empty() {
                return Err(format!(
                    "`{key}=` needs a value; write `{key}=\"\"` for an empty one"
                ));
            }
            if bare
                .chars()
                .any(|c| matches!(c, '"' | '=' | '\\') || c.is_control())
            {
                return Err(format!(
                    "the value of `{key}` must be quoted: it holds `\"`, `=`, `\\` or a control \
                     character"
                ));
            }
            bare
        };
        *slot = Some(value);
        after = format!("`{key}`'s value");
    }
    if let Some(kind) = matcher.tool_kind.as_deref()
        && !TOOL_KINDS.contains(&kind)
    {
        return Err(rule_kind_unknown(kind));
    }
    Ok(Some(PersonaRule {
        matcher,
        answer,
        reason,
    }))
}

/// The reason, the cursor on its `#`: quoted iff its first non-`ws` char is `"` (then nothing but
/// `ws` may follow, E-10), otherwise the rest of the line, trimmed.
fn rule_reason(cursor: &mut LineCursor<'_>) -> Result<String, String> {
    cursor.at += 1;
    cursor.skip_ws();
    if cursor.peek() == Some('"') {
        let reason = cursor.quoted()?;
        cursor.skip_ws();
        if cursor.peek().is_some() {
            return Err("nothing may follow a quoted reason".to_owned());
        }
        return Ok(reason);
    }
    Ok(cursor.run(|_| false).trim().to_owned())
}

/// One rule in [`format_rules`]' shortest form.
fn rule_text(rule: &PersonaRule) -> String {
    let mut line = match rule.answer {
        PersonaAnswer::RejectOnce => "reject_once",
        PersonaAnswer::RejectAlways => "reject_always",
    }
    .to_owned();
    let matcher = &rule.matcher;
    let values = [
        &matcher.tool_kind,
        &matcher.tool_name,
        &matcher.path_prefix,
        &matcher.command_prefix,
    ];
    for (key, value) in RULE_KEYS.into_iter().zip(values) {
        if let Some(value) = value {
            line.push(' ');
            line.push_str(key);
            line.push('=');
            let bare = !value.is_empty()
                && !value.chars().any(|c| {
                    c.is_whitespace() || c.is_control() || matches!(c, '"' | '#' | '=' | '\\')
                });
            if bare {
                line.push_str(value);
            } else {
                push_quoted(&mut line, value);
            }
        }
    }
    let reason = rule.reason.as_str();
    if !reason.is_empty() {
        line.push_str(" # ");
        let plain = reason == reason.trim()
            && !reason.starts_with('"')
            && !reason.chars().any(char::is_control);
        if plain {
            line.push_str(reason);
        } else {
            push_quoted(&mut line, reason);
        }
    }
    line
}

/// `value` in quotes, with D17's five escapes; every other char raw (B-2).
fn push_quoted(line: &mut String, value: &str) {
    line.push('"');
    for c in value.chars() {
        match c {
            '"' => line.push_str("\\\""),
            '\\' => line.push_str("\\\\"),
            '\n' => line.push_str("\\n"),
            '\t' => line.push_str("\\t"),
            '\r' => line.push_str("\\r"),
            other => line.push(other),
        }
    }
    line.push('"');
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
    /// MOD-26 M2 OQ-7's exception, import only: a non-empty `tools` whose every entry is an MCP
    /// tool would import as an empty `allow`, which keeps every built-in tool.
    #[error(
        "every `tools` entry is an MCP tool; htui's `allow` keeps built-in tools only, so this \
         file would keep all of them \u{2014} write `disallowed-tools` or `deny-kinds` instead"
    )]
    OnlyMcpTools,
}

/// A persona file as the import reads it (MOD-26 M2 D19, OQ-7): the file, which has passed the
/// save rules, and the `tools` entries dropped because they name MCP tools, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The file, `allow` without its `mcp__` entries.
    pub file: PersonaFile,
    /// The dropped entries, as written.
    pub dropped: Vec<String>,
}

/// Reads one persona file (plan D8): the seeds; the import reads through [`parse_import`].
///
/// # Errors
/// Every [`PersonaFileError`] variant but [`PersonaFileError::OnlyMcpTools`]; nothing is
/// half-read.
pub fn parse_file(text: &str) -> Result<PersonaFile, PersonaFileError> {
    read_file(text).and_then(checked)
}

/// Reads one file for the Settings › Personas import (MOD-26 M2 D19): [`parse_file`]'s reader,
/// then every `tools` entry starting with [`MCP_PREFIX`] is moved to `dropped` (`--tools` never
/// filtered MCP tools, so the file meant nothing htui can enforce through it), then a `tools`
/// that was non-empty and is now empty is [`PersonaFileError::OnlyMcpTools`], then the save
/// rules. The seed path keeps [`parse_file`], which still refuses `mcp__` in `tools`.
///
/// # Errors
/// Every [`PersonaFileError`] variant.
pub fn parse_import(text: &str) -> Result<Imported, PersonaFileError> {
    let mut file = read_file(text)?;
    let written = std::mem::take(&mut file.tools.allow);
    let had_tools = !written.is_empty();
    let (dropped, kept): (Vec<String>, Vec<String>) = written
        .into_iter()
        .partition(|name| name.starts_with(MCP_PREFIX));
    if had_tools && kept.is_empty() {
        return Err(PersonaFileError::OnlyMcpTools);
    }
    file.tools.allow = kept;
    Ok(Imported {
        file: checked(file)?,
        dropped,
    })
}

/// The reader half of [`parse_file`]: every D8 key rule, no D3 save rule.
fn read_file(text: &str) -> Result<PersonaFile, PersonaFileError> {
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
        // I-5, review N7: `Model:` is the same refusal, not an unknown key.
        if key.eq_ignore_ascii_case("model") {
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
    Ok(PersonaFile {
        name,
        description,
        body,
        tools,
        permission,
    })
}

/// The save-rule half of [`parse_file`]: [`persona_refusal`] as [`PersonaFileError::Refused`].
fn checked(file: PersonaFile) -> Result<PersonaFile, PersonaFileError> {
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

/// A one-line list value (plan D8), the grammar of a persona file **and** of the Settings ›
/// Personas form (MOD-26 M2 B-3): comma-separated, each item trimmed, empty items dropped, so
/// `tools:` alone is the empty list.
#[must_use]
pub fn list_of(value: &str) -> Vec<String> {
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

    /// MOD-26 I-5 (no model, `R-AGT-8`): a persona file cannot carry a model — `model` is refused
    /// with [`MODEL_REFUSED`], before any later key, so the phase candidate's model is the step's.
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
        let sentence = PersonaNotInSnapshot {
            persona: "rev\niewer".into(),
        }
        .to_string();
        assert!(
            sentence.starts_with("persona `rev\\niewer` is not in the run's snapshot"),
            "the name is escape_debug'd: {sentence}"
        );
        assert!(!sentence.contains('\n'), "one line: {sentence:?}");
        assert_eq!(
            PersonaNotInSnapshot {
                persona: "reviewer".into()
            }
            .to_string(),
            "persona `reviewer` is not in the run's snapshot, so the step does not run \
             un-narrowed (MOD-26 I-4)",
            "a plain name reads as before"
        );
    }

    /// MOD-26 review N7: `model` is refused with its own sentence whatever its ASCII case, so
    /// `Model:` does not read as an unknown key (I-5).
    #[test]
    fn the_model_key_is_refused_in_any_case() {
        for key in ["Model", "MODEL", "mOdEl"] {
            assert_eq!(
                parse(&format!("name: reviewer\n{key}: opus\n"), "body\n"),
                Err(PersonaFileError::Model),
                "`{key}` gets the model sentence"
            );
        }
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

        // MOD-26 M2 D18: a misspelt rule kind matches nothing, so it is refused.
        let mut base = Base::new();
        base.permission.rules.push(rule(PersonaMatch {
            tool_kind: Some("exec".to_owned()),
            ..PersonaMatch::default()
        }));
        assert_eq!(base.refusal(), Some(rule_kind_unknown("exec")));
    }

    /// MOD-26 M2 D18, B-1: a rule's `tool_kind` must be one of all ten [`TOOL_KINDS`]; the check
    /// is a second pass, so a rule set milestone 1 refused keeps its older sentence.
    #[test]
    fn a_misspelt_rule_kind_is_refused_after_the_older_sentences() {
        let kind = |kind: &str| {
            rule(PersonaMatch {
                tool_kind: Some(kind.to_owned()),
                ..PersonaMatch::default()
            })
        };
        let refusal = |rules: Vec<PersonaRule>| {
            permission_refusal(&PersonaPermission {
                default: None,
                rules,
            })
        };

        assert_eq!(refusal(vec![kind("exec")]), Some(rule_kind_unknown("exec")));
        assert_eq!(
            refusal(vec![kind("exec"), rule(PersonaMatch::default())]),
            Some(RULE_MATCHES_EVERYTHING.to_owned()),
            "rule 2's empty match beats rule 1's kind (B-1)"
        );
        assert_eq!(
            refusal(vec![kind("ex\0ec")]),
            Some(has_nul("persona.permission.rules")),
            "a NUL in a kind keeps the NUL sentence"
        );
        assert_eq!(refusal(vec![kind("")]), Some(rule_kind_unknown("")));
        for accepted in TOOL_KINDS {
            assert_eq!(refusal(vec![kind(accepted)]), None, "{accepted}");
        }
        assert_eq!(
            refusal(TOOL_KINDS.iter().map(|accepted| kind(accepted)).collect()),
            None
        );

        assert_eq!(
            rule_kind_unknown("exec"),
            "persona.permission.rules entry kind `exec` is not one of read, edit, delete, move, \
             search, execute, think, fetch, switch_mode, other"
        );
        for odd in ["e`x", "e\nx"] {
            let sentence = rule_kind_unknown(odd);
            assert!(!sentence.contains('\n'), "one line: {sentence:?}");
            assert!(
                sentence.contains(&format!("`{}`", odd.escape_debug())),
                "the kind is escape_debug'd: {sentence}"
            );
        }
        assert!(rule_kind_unknown("e\nx").contains("`e\\nx`"));
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
            (
                PersonaPatch {
                    permission: Some(PersonaPermission {
                        default: None,
                        rules: vec![rule(PersonaMatch {
                            tool_kind: Some("exec".to_owned()),
                            ..PersonaMatch::default()
                        })],
                    }),
                    ..PersonaPatch::default()
                },
                rule_kind_unknown("exec"),
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

    /// MOD-26 I-6 (row is the truth, `R-ID-3`): the seed rows are the two `include_str!`'d seed
    /// files, parsed once into rows; nothing reads a persona file at run time.
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

    fn rule_with(answer: PersonaAnswer, matcher: [Option<&str>; 4], reason: &str) -> PersonaRule {
        let [tool_kind, tool_name, path_prefix, command_prefix] =
            matcher.map(|value| value.map(str::to_owned));
        PersonaRule {
            matcher: PersonaMatch {
                tool_kind,
                tool_name,
                path_prefix,
                command_prefix,
            },
            answer,
            reason: reason.to_owned(),
        }
    }

    /// MOD-26 M2 D17: §2.3's fixed table, each row both ways.
    fn fixed_rule_lines() -> Vec<(PersonaRule, &'static str)> {
        use PersonaAnswer::{RejectAlways as Always, RejectOnce as Once};
        vec![
            (
                rule_with(
                    Once,
                    [Some("execute"), None, None, Some("rm -rf")],
                    "never wipe",
                ),
                r#"reject_once kind=execute command="rm -rf" # never wipe"#,
            ),
            (
                rule_with(Always, [None, None, Some("/etc"), None], ""),
                "reject_always path=/etc",
            ),
            (
                rule_with(Once, [None, Some(""), None, None], ""),
                r#"reject_once name="""#,
            ),
            (
                rule_with(Once, [None, None, None, Some("say \"hi\"")], ""),
                r#"reject_once command="say \"hi\"""#,
            ),
            (
                rule_with(Once, [None, None, Some("C:\\tmp"), None], ""),
                r#"reject_once path="C:\\tmp""#,
            ),
            (
                rule_with(Once, [None, None, None, Some("a#b")], ""),
                r#"reject_once command="a#b""#,
            ),
            (
                rule_with(Once, [None, None, None, Some("x=1")], ""),
                r#"reject_once command="x=1""#,
            ),
            (
                rule_with(Once, [None, None, None, Some("l1\nl2\tx\r")], ""),
                r#"reject_once command="l1\nl2\tx\r""#,
            ),
            (
                rule_with(Once, [None, Some("café→"), None, None], ""),
                "reject_once name=café→",
            ),
            (
                rule_with(Once, [Some("read"), None, None, None], "  padded  "),
                r#"reject_once kind=read # "  padded  ""#,
            ),
            (
                rule_with(Once, [Some("read"), None, None, None], "\"quoted\" first"),
                r#"reject_once kind=read # "\"quoted\" first""#,
            ),
            (
                rule_with(Once, [Some("read"), None, None, None], "two\nlines"),
                r#"reject_once kind=read # "two\nlines""#,
            ),
            (
                rule_with(Once, [Some("read"), None, None, None], "has # inside"),
                "reject_once kind=read # has # inside",
            ),
            (
                rule_with(
                    Always,
                    [Some("fetch"), Some("WebFetch"), Some("/"), Some("curl")],
                    "no network",
                ),
                "reject_always kind=fetch name=WebFetch path=/ command=curl # no network",
            ),
            (
                rule_with(Once, [None, None, None, Some("\u{1b}[0m")], ""),
                "reject_once command=\"\u{1b}[0m\"",
            ),
            (
                rule_with(Once, [None, Some("a\u{a0}b"), None, None], ""),
                "reject_once name=\"a\u{a0}b\"",
            ),
            (
                rule_with(Once, [Some("switch_mode"), None, None, None], "#tag"),
                "reject_once kind=switch_mode # #tag",
            ),
        ]
    }

    #[test]
    fn rule_lines_round_trip_the_fixed_table() {
        let table = fixed_rule_lines();
        assert_eq!(table.len(), 17);
        for (rule, line) in &table {
            assert_eq!(format_rules(std::slice::from_ref(rule)), *line, "{rule:?}");
            assert_eq!(parse_rules(line), Ok(vec![rule.clone()]), "{line:?}");
        }

        let two = [table[0].0.clone(), table[1].0.clone()];
        let text = format!("{}\n{}", table[0].1, table[1].1);
        assert_eq!(format_rules(&two), text);
        assert_eq!(parse_rules(&text), Ok(two.to_vec()));
    }

    #[test]
    fn rule_lines_round_trip_generated_values() {
        const FRAGMENTS: [&str; 15] = [
            "a", "Z9", " ", "\"", "\\", "#", "=", "\n", "\t", "\r", "é", "→", "\u{1b}", "\u{a0}",
            "/",
        ];
        let mut values = vec![String::new()];
        for one in FRAGMENTS {
            values.push(one.to_owned());
        }
        for one in FRAGMENTS {
            for two in FRAGMENTS {
                values.push(format!("{one}{two}"));
            }
        }
        for one in FRAGMENTS {
            for two in FRAGMENTS {
                for three in FRAGMENTS {
                    values.push(format!("{one}{two}{three}"));
                }
            }
        }
        assert_eq!(values.len(), 3_616);

        let mut all = Vec::new();
        for (index, value) in values.iter().enumerate() {
            let v = Some(value.as_str());
            let rules = [
                rule_with(PersonaAnswer::RejectOnce, [None, v, None, None], ""),
                rule_with(
                    PersonaAnswer::RejectAlways,
                    [Some(TOOL_KINDS[index % 10]), None, v, None],
                    value,
                ),
                rule_with(PersonaAnswer::RejectOnce, [None, None, None, v], "x"),
            ];
            for rule in rules {
                let line = format_rules(std::slice::from_ref(&rule));
                assert_eq!(line.lines().count(), 1, "{line:?}");
                assert_eq!(parse_rules(&line), Ok(vec![rule.clone()]), "{line:?}");
                all.push(rule);
            }
        }
        let text = format_rules(&all);
        assert_eq!(text.lines().count(), all.len());
        assert_eq!(parse_rules(&text), Ok(all));
    }

    #[test]
    fn rule_lines_parse_spacing_comments_and_blank_lines() {
        let read = |reason: &str| {
            rule_with(
                PersonaAnswer::RejectOnce,
                [Some("read"), None, None, None],
                reason,
            )
        };

        assert_eq!(parse_rules("\n  # header\n\n"), Ok(Vec::new()));
        assert_eq!(parse_rules(""), Ok(Vec::new()));
        assert_eq!(
            parse_rules("  reject_once   kind=read   "),
            Ok(vec![read("")])
        );
        assert_eq!(
            parse_rules("reject_once command=\"rm\"#why"),
            Ok(vec![rule_with(
                PersonaAnswer::RejectOnce,
                [None, None, None, Some("rm")],
                "why"
            )])
        );
        assert_eq!(
            parse_rules("reject_once kind=read #   "),
            Ok(vec![read("")])
        );
        assert_eq!(
            parse_rules("reject_once kind=read\r\nreject_always path=/x\r"),
            Ok(vec![
                read(""),
                rule_with(
                    PersonaAnswer::RejectAlways,
                    [None, None, Some("/x"), None],
                    ""
                ),
            ])
        );
        assert_eq!(
            parse_rules("reject_once path=/a#b"),
            Ok(vec![rule_with(
                PersonaAnswer::RejectOnce,
                [None, None, Some("/a"), None],
                "b"
            )])
        );
        assert_eq!(
            parse_rules("reject_once"),
            Ok(vec![rule(PersonaMatch::default())]),
            "an empty match is the store's refusal, not the parser's"
        );
        assert_eq!(
            parse_rules("reject_once command=x kind=read"),
            Ok(vec![rule_with(
                PersonaAnswer::RejectOnce,
                [Some("read"), None, None, Some("x")],
                ""
            )])
        );
    }

    #[test]
    fn rule_line_errors_name_their_line() {
        let cases: [(&str, String); 11] = [
            (
                "reject kind=read",
                "a rule starts with reject_once or reject_always, not `reject`".to_owned(),
            ),
            (
                "reject_once colour=red",
                "`colour` is not a rule key; a rule takes kind, name, path and command".to_owned(),
            ),
            (
                "reject_once kind=read kind=edit",
                "rule key `kind` appears more than once".to_owned(),
            ),
            ("reject_once kind", "`kind` is not `key=value`".to_owned()),
            (
                "reject_once kind= name=x",
                r#"`kind=` needs a value; write `kind=""` for an empty one"#.to_owned(),
            ),
            (
                "reject_once path=a=b",
                r#"the value of `path` must be quoted: it holds `"`, `=`, `\` or a control character"#
                    .to_owned(),
            ),
            (
                r#"reject_once path="open"#,
                "a quoted string is not closed".to_owned(),
            ),
            (
                r#"reject_once path="a\qb""#,
                r#"`\q` is not an escape; use \", \\, \n, \t or \r"#.to_owned(),
            ),
            (
                r#"reject_once path="a"b"#,
                "expected a space, `#` or the end of the line after `path`'s value".to_owned(),
            ),
            (
                r#"reject_once kind=read # "r" x"#,
                "nothing may follow a quoted reason".to_owned(),
            ),
            (
                "reject_oncekind=read",
                "a rule starts with reject_once or reject_always, not `reject_oncekind=read`"
                    .to_owned(),
            ),
        ];
        for (input, message) in cases {
            let refused = parse_rules(&format!("reject_once kind=read\n{input}"));
            let expected = RuleLineError {
                line: 2,
                message: message.clone(),
            };
            assert_eq!(refused, Err(expected.clone()), "{input:?}");
            assert_eq!(expected.to_string(), format!("rules line 2: {message}"));
        }
    }

    #[test]
    fn a_rule_line_kind_outside_the_list_is_the_store_sentence() {
        assert_eq!(
            parse_rules("reject_once kind=exec"),
            Err(RuleLineError {
                line: 1,
                message: rule_kind_unknown("exec"),
            })
        );
        assert_eq!(
            parse_rules(r#"reject_once kind="""#),
            Err(RuleLineError {
                line: 1,
                message: rule_kind_unknown(""),
            })
        );
        assert_eq!(
            parse_rules(r#"reject_once kind="read""#),
            Ok(vec![rule_with(
                PersonaAnswer::RejectOnce,
                [Some("read"), None, None, None],
                ""
            )])
        );
    }

    #[test]
    fn an_empty_value_is_quoted_and_an_absent_key_is_none() {
        let empty = rule_with(PersonaAnswer::RejectOnce, [None, Some(""), None, None], "");
        assert_eq!(
            parse_rules(r#"reject_once name="""#),
            Ok(vec![empty.clone()])
        );
        assert_eq!(
            format_rules(std::slice::from_ref(&empty)),
            r#"reject_once name="""#
        );

        let parsed = parse_rules("reject_once path=/x").expect("parses");
        assert_eq!(parsed[0].matcher.tool_kind, None);
        assert_eq!(parsed[0].matcher.tool_name, None);
        assert_eq!(parsed[0].matcher.command_prefix, None);
        let line = format_rules(&parsed);
        for key in ["kind", "name", "command"] {
            assert!(!line.contains(&format!("{key}=")), "{line}");
        }
    }

    #[test]
    fn format_rules_writes_one_line_per_rule() {
        assert_eq!(format_rules(&[]), "");
        let rules = [
            rule_with(
                PersonaAnswer::RejectOnce,
                [Some("read"), None, None, None],
                "",
            ),
            rule_with(
                PersonaAnswer::RejectAlways,
                [None, None, Some("/etc"), None],
                "why",
            ),
            rule_with(
                PersonaAnswer::RejectOnce,
                [None, None, None, Some("rm")],
                "",
            ),
        ];
        let text = format_rules(&rules);
        assert_eq!(
            text,
            "reject_once kind=read\nreject_always path=/etc # why\nreject_once command=rm"
        );
        assert!(!text.ends_with('\n'));
        assert_eq!(text.split('\n').count(), 3);
    }

    /// `.claude/agents/code-architect.md`'s frontmatter, verbatim (copied; no test reads a real
    /// agent file), then a one-line body.
    const CODE_ARCHITECT: &str = "---\n\
        name: code-architect\n\
        description: Designs feature architectures by analyzing existing codebase patterns and \
        conventions, then providing implementation blueprints with concrete files, interfaces, \
        data flow, and build order.\n\
        tools: Read, Grep, Glob, Bash, mcp__gortex__capabilities, mcp__gortex__explore, \
        mcp__gortex__search, mcp__gortex__read, mcp__gortex__relations, mcp__gortex__trace, \
        mcp__gortex__analyze, mcp__gortex__recall, mcp__gortex__workspace\n\
        ---\n\nYou design.\n";

    /// `.claude/agents/rust-reviewer.md`'s frontmatter, verbatim, then a one-line body.
    const RUST_REVIEWER: &str = "---\n\
        name: rust-reviewer\n\
        description: Expert Rust code reviewer specializing in ownership, lifetimes, error \
        handling, unsafe usage, and idiomatic patterns. Use for all Rust code changes. MUST BE \
        USED for Rust projects.\n\
        tools: Read, Grep, Glob, Bash, mcp__gortex__capabilities, mcp__gortex__explore, \
        mcp__gortex__search, mcp__gortex__read, mcp__gortex__relations, mcp__gortex__trace, \
        mcp__gortex__analyze, mcp__gortex__recall, mcp__gortex__workspace\n\
        ---\n\nYou review.\n";

    /// `~/.claude/agents/gortex-impact.md`'s frontmatter, verbatim, then a one-line body.
    const GORTEX_IMPACT: &str = "---\n\
        name: gortex-impact\n\
        description: \"Assess a change's blast radius, contracts, guards, and tests.\"\n\
        tools: mcp__gortex__capabilities, mcp__gortex__explore, mcp__gortex__search, \
        mcp__gortex__read, mcp__gortex__relations, mcp__gortex__trace, mcp__gortex__analyze, \
        mcp__gortex__change, mcp__gortex__recall, mcp__gortex__workspace\n\
        ---\n\nYou assess.\n";

    /// `~/.claude/agents/gortex-search.md`'s frontmatter, verbatim, then a one-line body.
    const GORTEX_SEARCH: &str = "---\n\
        name: gortex-search\n\
        description: \"Locate code, trace call paths, or map architecture in a fresh context.\"\n\
        tools: mcp__gortex__capabilities, mcp__gortex__explore, mcp__gortex__search, \
        mcp__gortex__read, mcp__gortex__relations, mcp__gortex__trace, mcp__gortex__analyze, \
        mcp__gortex__recall, mcp__gortex__workspace\n\
        ---\n\nYou search.\n";

    /// The nine `mcp__gortex__*` entries both repo agent files carry, in file order.
    const REPO_AGENT_MCP_TOOLS: [&str; 9] = [
        "mcp__gortex__capabilities",
        "mcp__gortex__explore",
        "mcp__gortex__search",
        "mcp__gortex__read",
        "mcp__gortex__relations",
        "mcp__gortex__trace",
        "mcp__gortex__analyze",
        "mcp__gortex__recall",
        "mcp__gortex__workspace",
    ];

    fn file_refusal(file: &PersonaFile) -> Option<String> {
        persona_refusal(
            &file.name,
            &file.description,
            &file.body,
            &file.tools,
            &file.permission,
        )
    }

    /// MOD-26 M2 D19, OQ-7: the import drops `mcp__` entries from `tools` and names them.
    #[test]
    fn parse_import_drops_mcp_tools_and_names_them() {
        let imported = parse_import(CODE_ARCHITECT).expect("the repo's architect imports");
        assert_eq!(imported.file.name, "code-architect");
        assert_eq!(
            imported.file.description,
            "Designs feature architectures by analyzing existing codebase patterns and \
             conventions, then providing implementation blueprints with concrete files, \
             interfaces, data flow, and build order."
        );
        assert_eq!(imported.file.tools.allow, ["Read", "Grep", "Glob", "Bash"]);
        assert_eq!(imported.dropped, REPO_AGENT_MCP_TOOLS);
        assert_eq!(imported.file.body, "You design.\n");
    }

    #[test]
    fn parse_import_accepts_both_repo_agent_files() {
        let architect = parse_import(CODE_ARCHITECT).expect("the repo's architect imports");
        let reviewer = parse_import(RUST_REVIEWER).expect("the repo's reviewer imports");
        assert_eq!(reviewer.file.name, "rust-reviewer");
        assert_eq!(
            reviewer.file.description,
            "Expert Rust code reviewer specializing in ownership, lifetimes, error handling, \
             unsafe usage, and idiomatic patterns. Use for all Rust code changes. MUST BE USED \
             for Rust projects."
        );
        assert_eq!(reviewer.file.tools.allow, ["Read", "Grep", "Glob", "Bash"]);
        assert_eq!(reviewer.dropped, REPO_AGENT_MCP_TOOLS);
        for imported in [&architect, &reviewer] {
            assert_eq!(file_refusal(&imported.file), None, "{}", imported.file.name);
        }
    }

    #[test]
    fn parse_import_refuses_an_all_mcp_tools_file() {
        for text in [GORTEX_IMPACT, GORTEX_SEARCH] {
            assert_eq!(
                parse_import(text),
                Err(PersonaFileError::OnlyMcpTools),
                "{text}"
            );
        }
        assert_eq!(
            PersonaFileError::OnlyMcpTools.to_string(),
            "every `tools` entry is an MCP tool; htui's `allow` keeps built-in tools only, so \
             this file would keep all of them \u{2014} write `disallowed-tools` or `deny-kinds` \
             instead"
        );
    }

    /// The seed path is unchanged: `parse_file` still refuses an `mcp__` entry in `tools`.
    #[test]
    fn parse_file_still_refuses_every_mcp_tools_file() {
        for text in [CODE_ARCHITECT, RUST_REVIEWER, GORTEX_IMPACT, GORTEX_SEARCH] {
            assert_eq!(
                parse_file(text),
                Err(PersonaFileError::Refused(allow_names_an_mcp_tool(
                    "mcp__gortex__capabilities"
                ))),
                "{text}"
            );
        }
    }

    #[test]
    fn parse_import_keeps_parse_files_other_refusals() {
        let import = |front: &str| parse_import(&format!("---\n{front}---\n\nbody\n"));
        assert_eq!(
            import("name: reviewer\nmodel: opus\n"),
            Err(PersonaFileError::Model)
        );
        assert_eq!(
            parse_import("name: reviewer\n---\nbody\n"),
            Err(PersonaFileError::Fence(FrontmatterError::NoFence))
        );
        let empty = import("name: reviewer\ntools:\n").expect("an empty `tools` imports");
        assert!(empty.file.tools.allow.is_empty());
        assert!(empty.dropped.is_empty());
        assert_eq!(
            import("name: reviewer\ntools: Read, Bad Name\n"),
            Err(PersonaFileError::Refused(not_a_tool_name(
                "allow", "Bad Name"
            )))
        );
    }

    /// MOD-26 M2 B-3: one list grammar for the file and the Settings › Personas form.
    #[test]
    fn list_of_is_the_forms_list_grammar() {
        assert_eq!(list_of(" a, ,b ,"), ["a", "b"]);
        assert!(list_of("").is_empty());
    }
}
