//! MCP servers of the toolbox (MOD-127 M2 D2-D5): the record, its refusals, and the pure
//! resolution of which servers apply to a step.
//!
//! A server is one row (no versions), attached by `toolbox_binding` rows of `kind = 'mcp_server'`
//! at the same five levels a skill uses. Secrets are held **by reference** ([`EnvValue::Secret`]
//! names a project secret): the record, the frozen snapshot, the step record and `Debug` carry
//! names, never values (`R-SEC-2`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{McpServerId, UserId};
use crate::model::skill::{Activation, BindingLevel};
use crate::secret::RESERVED_PREFIX;
use crate::store::traits::has_nul;

/// The server name the agent sees for htui's own server: its tools read `mcp__htui__<tool>`.
///
/// Defined here (MOD-127 M2 D2) so the toolbox's name rule can refuse it; `htui-mcp` re-exports it.
pub const SERVER_NAME: &str = "htui";

/// The longest `mcp_server.name`, in bytes.
pub const MCP_NAME_MAX: usize = 32;

str_enum!(
    /// `mcp_server.transport`: how an agent reaches the server.
    McpTransport {
        /// A child process the agent starts: `command` and `args`, with `env`.
        Stdio => "stdio",
        /// A remote endpoint: `url`, with `headers`.
        Http => "http",
    }
);

/// A value of an environment variable or an HTTP header (MOD-127 M2 D3).
///
/// Serde spells it `{"literal": ".."}` or `{"secret": ".."}`. `Secret` holds the **name** of a
/// project secret that `RunSecrets::env_for` supplies; the value is substituted only when the
/// spec for a launch is built.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvValue {
    /// The text itself: not a secret, so it is stored and shown as written.
    Literal(String),
    /// The name of a project secret.
    Secret(String),
}

/// An `mcp_server` row (MOD-127 M2 D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    /// `mcp_server.id`.
    pub id: McpServerId,
    /// `mcp_server.name`: `^[a-z][a-z0-9_-]{0,31}$`, unique, never [`SERVER_NAME`]. It forms the
    /// agent's `mcp__<name>__<tool>` names.
    pub name: String,
    /// `mcp_server.description`, the picker's one-liner; may be empty.
    pub description: String,
    /// `mcp_server.transport`.
    pub transport: McpTransport,
    /// The program to start (`Stdio`); empty for `Http`.
    pub command: String,
    /// The program's arguments (`Stdio`); empty for `Http`.
    pub args: Vec<String>,
    /// The endpoint (`Http`); `None` for `Stdio`.
    pub url: Option<String>,
    /// HTTP headers, in order.
    pub headers: Vec<(String, EnvValue)>,
    /// Environment variables of the child, in order.
    pub env: Vec<(String, EnvValue)>,
    /// Tool names (without the `mcp__<name>__` prefix) the server's tools are pre-approved for;
    /// empty leaves the existing policy to decide.
    pub allow: Vec<String>,
    /// Tool names removed from the agent.
    pub deny: Vec<String>,
    /// `mcp_server.created_by`.
    pub created_by: UserId,
    /// `mcp_server.created_at`.
    pub created_at: DateTime<Utc>,
    /// `mcp_server.updated_at`, the compare-and-set token.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of `WriteStore::create_mcp_server`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewMcpServer {
    /// `mcp_server.id`, minted client-side as a UUIDv7.
    pub id: McpServerId,
    /// `mcp_server.name`; must pass [`validate_mcp_name`].
    pub name: String,
    /// `mcp_server.description`.
    pub description: String,
    /// `mcp_server.transport`.
    pub transport: McpTransport,
    /// See [`McpServer::command`].
    pub command: String,
    /// See [`McpServer::args`].
    pub args: Vec<String>,
    /// See [`McpServer::url`].
    pub url: Option<String>,
    /// See [`McpServer::headers`].
    pub headers: Vec<(String, EnvValue)>,
    /// See [`McpServer::env`].
    pub env: Vec<(String, EnvValue)>,
    /// See [`McpServer::allow`].
    pub allow: Vec<String>,
    /// See [`McpServer::deny`].
    pub deny: Vec<String>,
    /// `mcp_server.created_by`.
    pub created_by: UserId,
}

/// Edit passed to `WriteStore::update_mcp_server`; `None` leaves the column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerPatch {
    /// A rename; must pass [`validate_mcp_name`] and be free.
    pub name: Option<String>,
    /// `mcp_server.description`.
    pub description: Option<String>,
    /// `mcp_server.transport`.
    pub transport: Option<McpTransport>,
    /// `mcp_server.command`.
    pub command: Option<String>,
    /// `mcp_server.args`.
    pub args: Option<Vec<String>>,
    /// `mcp_server.url`: `Some(None)` clears it.
    pub url: Option<Option<String>>,
    /// `mcp_server.headers`.
    pub headers: Option<Vec<(String, EnvValue)>>,
    /// `mcp_server.env`.
    pub env: Option<Vec<(String, EnvValue)>>,
    /// `mcp_server.allow`.
    pub allow: Option<Vec<String>>,
    /// `mcp_server.deny`.
    pub deny: Option<Vec<String>>,
}

impl McpServerPatch {
    /// `server` as it stands once this patch is applied (every `Some` replaces its column). The
    /// stamps and the id are the store's.
    pub fn apply(self, server: &mut McpServer) {
        let Self {
            name,
            description,
            transport,
            command,
            args,
            url,
            headers,
            env,
            allow,
            deny,
        } = self;
        if let Some(name) = name {
            server.name = name;
        }
        if let Some(description) = description {
            server.description = description;
        }
        if let Some(transport) = transport {
            server.transport = transport;
        }
        if let Some(command) = command {
            server.command = command;
        }
        if let Some(args) = args {
            server.args = args;
        }
        if let Some(url) = url {
            server.url = url;
        }
        if let Some(headers) = headers {
            server.headers = headers;
        }
        if let Some(env) = env {
            server.env = env;
        }
        if let Some(allow) = allow {
            server.allow = allow;
        }
        if let Some(deny) = deny {
            server.deny = deny;
        }
    }
}

/// Whether `name` may be stored as an `mcp_server.name`: `^[a-z][a-z0-9_-]{0,31}$`, and not
/// htui's own [`SERVER_NAME`]. [`invalid_mcp_name`] phrases the refusal.
#[must_use]
pub fn validate_mcp_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && name.len() <= MCP_NAME_MAX
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && name != SERVER_NAME
}

/// The refusal for a name [`validate_mcp_name`] refuses.
#[must_use]
pub fn invalid_mcp_name(name: &str) -> String {
    if name == SERVER_NAME {
        return format!("mcp_server.name `{name}` is the name of htui's own server");
    }
    format!(
        "mcp_server.name `{}` must start with a-z and be 1-{MCP_NAME_MAX} of a-z, 0-9, `_` and `-`",
        name.escape_debug()
    )
}

/// Words in a key that say its value is a secret (MOD-127 M2 D3), compared in lower case.
const SECRET_WORDS: [&str; 7] = [
    "token",
    "secret",
    "password",
    "passwd",
    "key",
    "authorization",
    "cookie",
];

/// Whether `key` looks like it carries a secret (case-insensitive contains one of
/// [`SECRET_WORDS`]).
fn looks_secret(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|word| lower.contains(word))
}

/// Whether `key` is a valid environment variable name: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_env_name(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether `key` is a valid HTTP header name (a token of letters, digits and `-`).
fn is_header_name(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The refusal of one `(key, value)` list. `what` is `env` or `header`; `valid` judges the key.
fn pairs_refusal(
    what: &str,
    pairs: &[(String, EnvValue)],
    valid: fn(&str) -> bool,
    case_insensitive: bool,
) -> Option<String> {
    for (index, (key, value)) in pairs.iter().enumerate() {
        if !valid(key) {
            return Some(format!(
                "an mcp_server {what} name `{}` is not a valid name",
                key.escape_debug()
            ));
        }
        if what == "env"
            && key
                .get(..RESERVED_PREFIX.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(RESERVED_PREFIX))
        {
            return Some(format!(
                "an mcp_server env name `{key}` starts with the reserved prefix `{RESERVED_PREFIX}`"
            ));
        }
        let same = |other: &str| {
            if case_insensitive {
                other.eq_ignore_ascii_case(key)
            } else {
                other == key
            }
        };
        if pairs[..index].iter().any(|(earlier, _)| same(earlier)) {
            return Some(format!("an mcp_server {what} name `{key}` is listed twice"));
        }
        match value {
            EnvValue::Literal(text) => {
                if text.contains('\0') {
                    return Some(has_nul(&format!("mcp_server {what} `{key}`")));
                }
                if looks_secret(key) {
                    return Some(format!(
                        "the {what} `{key}` looks like a secret: store it as a secret reference \
                         (`secret`), not as a literal value"
                    ));
                }
            }
            EnvValue::Secret(name) => {
                if name.trim().is_empty() || name.chars().any(char::is_control) {
                    return Some(format!(
                        "the secret reference of {what} `{key}` must name a project secret"
                    ));
                }
            }
        }
    }
    None
}

/// The refusal of a tool-name list (`allow` or `deny`).
fn tools_refusal(column: &str, tools: &[String]) -> Option<String> {
    tools
        .iter()
        .find(|tool| tool.is_empty() || tool.chars().any(|c| c.is_whitespace() || c.is_control()))
        .map(|tool| {
            format!(
                "mcp_server.{column} holds `{}`, which is not a tool name",
                tool.escape_debug()
            )
        })
}

/// A record's columns as the shape rules read them.
struct Fields<'a> {
    name: &'a str,
    description: &'a str,
    transport: McpTransport,
    command: &'a str,
    args: &'a [String],
    url: Option<&'a str>,
    headers: &'a [(String, EnvValue)],
    env: &'a [(String, EnvValue)],
    allow: &'a [String],
    deny: &'a [String],
}

/// The one refusal chain `create` and `update` share, in order: name, description, the
/// transport's shape, env, headers, `allow`, `deny`.
fn refusal(fields: &Fields<'_>) -> Option<String> {
    if !validate_mcp_name(fields.name) {
        return Some(invalid_mcp_name(fields.name));
    }
    if fields.description.contains('\0') {
        return Some(has_nul("mcp_server.description"));
    }
    match fields.transport {
        McpTransport::Stdio => {
            if fields.command.trim().is_empty() {
                return Some("a stdio mcp_server needs a command".to_owned());
            }
            if fields.url.is_some() {
                return Some("a stdio mcp_server has no url".to_owned());
            }
            if fields.command.contains('\0') || fields.args.iter().any(|arg| arg.contains('\0')) {
                return Some(has_nul("mcp_server.command"));
            }
        }
        McpTransport::Http => {
            let Some(url) = fields.url.filter(|url| !url.trim().is_empty()) else {
                return Some("an http mcp_server needs a url".to_owned());
            };
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Some(format!(
                    "the url `{}` of an http mcp_server must start with http:// or https://",
                    url.escape_debug()
                ));
            }
            if url.contains('\0') {
                return Some(has_nul("mcp_server.url"));
            }
            if !fields.command.is_empty() || !fields.args.is_empty() {
                return Some("an http mcp_server has no command or args".to_owned());
            }
        }
    }
    pairs_refusal("env", fields.env, is_env_name, false)
        .or_else(|| pairs_refusal("header", fields.headers, is_header_name, true))
        .or_else(|| tools_refusal("allow", fields.allow))
        .or_else(|| tools_refusal("deny", fields.deny))
}

/// `create_mcp_server`'s input refusal (MOD-127 M2 D2, D3), the sentence both stores give.
#[must_use]
pub fn new_mcp_server_refusal(new: &NewMcpServer) -> Option<String> {
    refusal(&Fields {
        name: &new.name,
        description: &new.description,
        transport: new.transport,
        command: &new.command,
        args: &new.args,
        url: new.url.as_deref(),
        headers: &new.headers,
        env: &new.env,
        allow: &new.allow,
        deny: &new.deny,
    })
}

/// `update_mcp_server`'s input refusal: the rules of [`new_mcp_server_refusal`] over `current`
/// with `patch` applied, so a patch that leaves a stdio server without its command, or an http
/// one with a command, is refused.
#[must_use]
pub fn mcp_server_patch_refusal(current: &McpServer, patch: &McpServerPatch) -> Option<String> {
    let mut next = current.clone();
    patch.clone().apply(&mut next);
    refusal(&Fields {
        name: &next.name,
        description: &next.description,
        transport: next.transport,
        command: &next.command,
        args: &next.args,
        url: next.url.as_deref(),
        headers: &next.headers,
        env: &next.env,
        allow: &next.allow,
        deny: &next.deny,
    })
}

/// One attachment of a server that applies to a step, before the most specific wins: the joined
/// server, the level the row sits at and its activation. Not a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCandidate {
    /// The attached server.
    pub server: McpServer,
    /// The level of the attachment.
    pub level: BindingLevel,
    /// The attachment's activation: `always` or `off` (`glob` is refused for a server).
    pub activation: Activation,
}

/// A server the step resolved (MOD-127 M2 D5): the winning attachment's server, level and
/// activation. An `off` winner is kept, `active: false`, so the step record can show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundMcp {
    /// The server, as stored now.
    pub server: McpServer,
    /// The level of the attachment that won.
    pub level: BindingLevel,
    /// The winning attachment's activation.
    pub activation: Activation,
    /// Whether the server launches: `activation` is not `off`.
    pub active: bool,
}

/// MOD-127 M2 D5: per server, the candidate at the **most specific** level wins whole, with its
/// own activation; an `off` winner hides the server and is returned inactive; the result is
/// ordered by server name bytes.
///
/// The mirror of `BoundSkill::collapse` for a kind with no pins or positions: one definition both
/// stores call. Two candidates of one server at one level cannot come from a store (the binding
/// key is unique), but a caller's list is re-collapsed, so the tie is defined: the first in input
/// order wins.
#[must_use]
pub fn resolve_mcp(candidates: &[McpCandidate]) -> Vec<BoundMcp> {
    let mut ordered: Vec<&McpCandidate> = candidates.iter().collect();
    // Most specific first; `sort_by_key` is stable, so equal levels keep their input order.
    ordered.sort_by_key(|candidate| core::cmp::Reverse(candidate.level));
    let mut resolved: Vec<BoundMcp> = Vec::with_capacity(ordered.len());
    for candidate in ordered {
        if resolved
            .iter()
            .any(|kept| kept.server.id == candidate.server.id)
        {
            continue;
        }
        resolved.push(BoundMcp {
            server: candidate.server.clone(),
            level: candidate.level,
            activation: candidate.activation,
            active: candidate.activation != Activation::Off,
        });
    }
    // `str`'s own `Ord` is byte order; Postgres would order these by collation.
    resolved.sort_by(|a, b| a.server.name.as_bytes().cmp(b.server.name.as_bytes()));
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ids::{ItemKindId, PhaseId, ProjectId, WorkspaceId};
    use crate::model::skill::{Attachment, BindingKey, BindingRef};
    use crate::store::traits::{BindingFacts, check_attachment};

    fn new_server(name: &str) -> NewMcpServer {
        NewMcpServer {
            id: McpServerId::new(),
            name: name.to_owned(),
            description: String::new(),
            transport: McpTransport::Stdio,
            command: "mcp-docs".to_owned(),
            args: vec!["--stdio".to_owned()],
            url: None,
            headers: vec![],
            env: vec![],
            allow: vec![],
            deny: vec![],
            created_by: UserId::new(),
        }
    }

    fn http_server(name: &str) -> NewMcpServer {
        NewMcpServer {
            transport: McpTransport::Http,
            command: String::new(),
            args: vec![],
            url: Some("https://mcp.example.test/rpc".to_owned()),
            ..new_server(name)
        }
    }

    fn stored(name: &str) -> McpServer {
        let new = new_server(name);
        McpServer {
            id: new.id,
            name: new.name,
            description: new.description,
            transport: new.transport,
            command: new.command,
            args: new.args,
            url: new.url,
            headers: new.headers,
            env: new.env,
            allow: new.allow,
            deny: new.deny,
            created_by: new.created_by,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        }
    }

    fn candidate(server: &McpServer, level: BindingLevel, activation: Activation) -> McpCandidate {
        McpCandidate {
            server: server.clone(),
            level,
            activation,
        }
    }

    #[test]
    fn a_name_is_lowercase_letters_digits_underscore_and_hyphen_from_a_letter() {
        for good in ["docs", "a", "my-server_2", &"a".repeat(32)] {
            assert!(validate_mcp_name(good), "{good} must be accepted");
            assert_eq!(new_mcp_server_refusal(&new_server(good)), None, "{good}");
        }
        for bad in [
            "",
            "Docs",
            "1docs",
            "-docs",
            "_docs",
            "do cs",
            "do.cs",
            "d\u{e9}",
            &"a".repeat(33),
        ] {
            assert!(!validate_mcp_name(bad), "{bad:?} must be refused");
            let sentence = new_mcp_server_refusal(&new_server(bad)).expect("refused");
            assert_eq!(sentence, invalid_mcp_name(bad));
        }
    }

    #[test]
    fn htuis_own_server_name_is_reserved() {
        assert_eq!(SERVER_NAME, "htui");
        assert!(!validate_mcp_name(SERVER_NAME));
        let sentence = new_mcp_server_refusal(&new_server(SERVER_NAME)).expect("refused");
        assert!(sentence.contains("htui's own server"), "{sentence}");
        // A name that merely starts with it is fine.
        assert!(validate_mcp_name("htui-extra"));
    }

    #[test]
    fn stdio_needs_a_command_and_no_url() {
        assert_eq!(new_mcp_server_refusal(&new_server("a")), None);
        let mut blank = new_server("a");
        blank.command = "  ".to_owned();
        assert_eq!(
            new_mcp_server_refusal(&blank).as_deref(),
            Some("a stdio mcp_server needs a command")
        );
        let mut with_url = new_server("a");
        with_url.url = Some("https://x.test".to_owned());
        assert_eq!(
            new_mcp_server_refusal(&with_url).as_deref(),
            Some("a stdio mcp_server has no url")
        );
    }

    #[test]
    fn http_needs_a_url_and_no_command_or_args() {
        assert_eq!(new_mcp_server_refusal(&http_server("a")), None);
        let mut no_url = http_server("a");
        no_url.url = None;
        assert_eq!(
            new_mcp_server_refusal(&no_url).as_deref(),
            Some("an http mcp_server needs a url")
        );
        let mut bad_scheme = http_server("a");
        bad_scheme.url = Some("ftp://x.test".to_owned());
        assert!(
            new_mcp_server_refusal(&bad_scheme)
                .expect("refused")
                .contains("http:// or https://")
        );
        let mut command = http_server("a");
        command.command = "mcp-docs".to_owned();
        assert_eq!(
            new_mcp_server_refusal(&command).as_deref(),
            Some("an http mcp_server has no command or args")
        );
        let mut args = http_server("a");
        args.args = vec!["--x".to_owned()];
        assert_eq!(
            new_mcp_server_refusal(&args).as_deref(),
            Some("an http mcp_server has no command or args")
        );
    }

    #[test]
    fn a_patch_is_judged_against_the_record_it_lands_on() {
        let current = stored("docs");
        // Switching to http without a url leaves a record that cannot launch.
        let patch = McpServerPatch {
            transport: Some(McpTransport::Http),
            ..McpServerPatch::default()
        };
        assert!(mcp_server_patch_refusal(&current, &patch).is_some());
        // Switching with the whole http shape is fine.
        let patch = McpServerPatch {
            transport: Some(McpTransport::Http),
            command: Some(String::new()),
            args: Some(vec![]),
            url: Some(Some("https://mcp.example.test".to_owned())),
            ..McpServerPatch::default()
        };
        assert_eq!(mcp_server_patch_refusal(&current, &patch), None);
        // A rename is held to the name rule.
        let patch = McpServerPatch {
            name: Some(SERVER_NAME.to_owned()),
            ..McpServerPatch::default()
        };
        assert_eq!(
            mcp_server_patch_refusal(&current, &patch),
            Some(invalid_mcp_name(SERVER_NAME))
        );
        // An empty patch of a sound record is accepted.
        assert_eq!(
            mcp_server_patch_refusal(&current, &McpServerPatch::default()),
            None
        );
    }

    #[test]
    fn env_and_header_keys_must_be_valid_names_without_the_reserved_prefix() {
        let literal = |key: &str| vec![(key.to_owned(), EnvValue::Literal("x".to_owned()))];
        let mut server = new_server("a");
        for good in ["REGION", "_x", "a1"] {
            server.env = literal(good);
            assert_eq!(new_mcp_server_refusal(&server), None, "{good}");
        }
        for bad in ["", "1A", "A-B", "A B", "HTUI_MCP_TOKEN", "htui_x"] {
            server.env = literal(bad);
            assert!(new_mcp_server_refusal(&server).is_some(), "{bad:?}");
        }
        server.env = vec![
            ("A".to_owned(), EnvValue::Literal("1".to_owned())),
            ("A".to_owned(), EnvValue::Literal("2".to_owned())),
        ];
        assert!(
            new_mcp_server_refusal(&server)
                .expect("refused")
                .contains("listed twice")
        );
        let mut http = http_server("a");
        http.headers = literal("X-Tenant");
        assert_eq!(new_mcp_server_refusal(&http), None);
        http.headers = literal("X Tenant");
        assert!(new_mcp_server_refusal(&http).is_some());
        // The reserved prefix is an environment rule; a header may begin with it.
        http.headers = literal("HTUI-Tenant");
        assert_eq!(new_mcp_server_refusal(&http), None);
    }

    #[test]
    fn a_literal_under_a_secret_looking_key_is_refused() {
        for word in SECRET_WORDS {
            for key in [
                format!("MY_{}_X", word.to_ascii_uppercase()),
                format!("my_{word}_x"),
            ] {
                let mut server = new_server("a");
                server.env = vec![(key.clone(), EnvValue::Literal("sentinel-value".to_owned()))];
                let sentence = new_mcp_server_refusal(&server)
                    .unwrap_or_else(|| panic!("`{key}` must be refused as an env literal"));
                assert!(sentence.contains("secret reference"), "{sentence}");
                assert!(!sentence.contains("sentinel-value"), "{sentence}");
            }
            for key in [
                format!("X-{word}"),
                format!("X-{}", word.to_ascii_uppercase()),
            ] {
                let mut http = http_server("a");
                http.headers = vec![(key.clone(), EnvValue::Literal("sentinel-value".to_owned()))];
                let sentence = new_mcp_server_refusal(&http)
                    .unwrap_or_else(|| panic!("`{key}` must be refused as a header literal"));
                assert!(sentence.contains("secret reference"), "{sentence}");
                assert!(!sentence.contains("sentinel-value"), "{sentence}");
            }
        }
        // The headers an http server carries a credential in.
        let mut http = http_server("a");
        for key in ["Authorization", "Cookie"] {
            http.headers = vec![(
                key.to_owned(),
                EnvValue::Literal("sentinel-value".to_owned()),
            )];
            assert!(new_mcp_server_refusal(&http).is_some(), "{key}");
        }
    }

    #[test]
    fn a_secret_reference_is_accepted() {
        let mut server = new_server("a");
        server.env = vec![
            (
                "API_TOKEN".to_owned(),
                EnvValue::Secret("DOCS_TOKEN".to_owned()),
            ),
            ("REGION".to_owned(), EnvValue::Literal("eu".to_owned())),
        ];
        assert_eq!(new_mcp_server_refusal(&server), None);
        let mut http = http_server("a");
        http.headers = vec![(
            "Authorization".to_owned(),
            EnvValue::Secret("DOCS_BEARER".to_owned()),
        )];
        assert_eq!(new_mcp_server_refusal(&http), None);
        // A reference must name something.
        http.headers = vec![("Authorization".to_owned(), EnvValue::Secret(" ".to_owned()))];
        assert!(new_mcp_server_refusal(&http).is_some());
    }

    #[test]
    fn tool_lists_hold_tool_names() {
        let mut server = new_server("a");
        server.allow = vec!["search".to_owned()];
        server.deny = vec!["delete_all".to_owned()];
        assert_eq!(new_mcp_server_refusal(&server), None);
        server.allow = vec![String::new()];
        assert!(new_mcp_server_refusal(&server).is_some());
        server.allow = vec![];
        server.deny = vec!["two words".to_owned()];
        assert!(new_mcp_server_refusal(&server).is_some());
    }

    #[test]
    fn env_value_serde_spells_literal_and_secret() {
        let literal = serde_json::to_value(EnvValue::Literal("eu".to_owned())).expect("ser");
        assert_eq!(literal, serde_json::json!({"literal": "eu"}));
        let secret = serde_json::to_value(EnvValue::Secret("DOCS".to_owned())).expect("ser");
        assert_eq!(secret, serde_json::json!({"secret": "DOCS"}));
        for value in [
            EnvValue::Literal("eu".to_owned()),
            EnvValue::Secret("DOCS".to_owned()),
        ] {
            let back: EnvValue =
                serde_json::from_value(serde_json::to_value(&value).expect("ser")).expect("de");
            assert_eq!(back, value);
        }
        assert!(serde_json::from_value::<EnvValue>(serde_json::json!({"plain": "x"})).is_err());
    }

    #[test]
    fn transport_spells_as_the_column_does() {
        assert_eq!(McpTransport::Stdio.as_str(), "stdio");
        assert_eq!(McpTransport::Http.as_str(), "http");
        assert_eq!(
            serde_json::to_value(McpTransport::Http).expect("ser"),
            serde_json::json!("http")
        );
    }

    #[test]
    fn most_specific_level_wins_whole() {
        let docs = stored("docs");
        let rows = [
            candidate(&docs, BindingLevel::Global, Activation::Always),
            candidate(&docs, BindingLevel::Phase, Activation::Always),
            candidate(&docs, BindingLevel::Project, Activation::Always),
        ];
        let bound = resolve_mcp(&rows);
        assert_eq!(bound.len(), 1, "one entry per server");
        assert_eq!(bound[0].level, BindingLevel::Phase);
        assert!(bound[0].active);
        // The order of the input does not decide.
        let mut reversed = rows.to_vec();
        reversed.reverse();
        assert_eq!(resolve_mcp(&reversed), bound);
        // Each level beats the one below it, in specificity order.
        for (wins, loses) in [
            (BindingLevel::Workspace, BindingLevel::Global),
            (BindingLevel::Project, BindingLevel::Workspace),
            (BindingLevel::ItemKind, BindingLevel::Project),
            (BindingLevel::Phase, BindingLevel::ItemKind),
        ] {
            let bound = resolve_mcp(&[
                candidate(&docs, loses, Activation::Always),
                candidate(&docs, wins, Activation::Always),
            ]);
            assert_eq!(bound[0].level, wins);
        }
        // A tie at one level keeps the first in input order.
        let mut first = docs.clone();
        first.description = "first".to_owned();
        let mut second = docs.clone();
        second.description = "second".to_owned();
        let bound = resolve_mcp(&[
            candidate(&first, BindingLevel::Project, Activation::Always),
            candidate(&second, BindingLevel::Project, Activation::Always),
        ]);
        assert_eq!(bound[0].server.description, "first");
    }

    #[test]
    fn off_at_a_more_specific_level_hides_the_server() {
        let docs = stored("docs");
        let bound = resolve_mcp(&[
            candidate(&docs, BindingLevel::Global, Activation::Always),
            candidate(&docs, BindingLevel::Phase, Activation::Off),
        ]);
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].level, BindingLevel::Phase);
        assert_eq!(bound[0].activation, Activation::Off);
        assert!(!bound[0].active, "the broader always does not show through");
        // The converse: an always at the more specific level re-enables it.
        let bound = resolve_mcp(&[
            candidate(&docs, BindingLevel::Global, Activation::Off),
            candidate(&docs, BindingLevel::Project, Activation::Always),
        ]);
        assert!(bound[0].active);
    }

    #[test]
    fn off_row_is_returned_inactive() {
        let docs = stored("docs");
        let bound = resolve_mcp(&[candidate(&docs, BindingLevel::Project, Activation::Off)]);
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].server, docs);
        assert!(!bound[0].active);
    }

    #[test]
    fn output_orders_by_name_bytes() {
        let names = ["zed", "Beta", "alpha", "a-b", "a_b", "a1"];
        let rows: Vec<McpCandidate> = names
            .iter()
            .map(|name| {
                let mut server = stored("x");
                server.name = (*name).to_owned();
                candidate(&server, BindingLevel::Global, Activation::Always)
            })
            .collect();
        let order: Vec<String> = resolve_mcp(&rows)
            .into_iter()
            .map(|bound| bound.server.name)
            .collect();
        let mut expected: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
        expected.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(order, expected);
        assert_eq!(
            order[0], "Beta",
            "uppercase sorts before lowercase in bytes"
        );
    }

    fn facts(key: BindingKey) -> BindingFacts<'static> {
        BindingFacts {
            key,
            phase_project: None,
            workspace_exists: true,
            kind_project: None,
            skill_name: "",
            versions: &[],
            repos: &[],
            project_slug: "",
            target_exists: true,
        }
    }

    fn attachment(activation: Activation) -> Attachment {
        Attachment {
            pinned_version: None,
            position: 0,
            activation,
            globs: vec![],
            languages: vec![],
        }
    }

    #[test]
    fn glob_and_pin_are_refused_for_a_server_binding() {
        let key = BindingKey::global(BindingRef::McpServer(McpServerId::new()));
        // A plain always and a plain off are accepted, with nothing stored beside them.
        for activation in [Activation::Always, Activation::Off] {
            let stored = check_attachment(&facts(key), &attachment(activation)).expect("accepted");
            assert!(stored.globs.is_empty() && stored.languages.is_empty());
        }
        let mut pinned = attachment(Activation::Always);
        pinned.pinned_version = Some(1);
        let sentence = check_attachment(&facts(key), &pinned).expect_err("a pin is refused");
        assert!(sentence.contains("no versions"), "{sentence}");
        let sentence = check_attachment(&facts(key), &attachment(Activation::Glob))
            .expect_err("glob activation is refused");
        assert!(sentence.contains("glob"), "{sentence}");
        let mut globbed = attachment(Activation::Always);
        globbed.globs = vec!["**/*.rs".to_owned()];
        assert!(check_attachment(&facts(key), &globbed).is_err());
        let mut languages = attachment(Activation::Always);
        languages.languages = vec!["rust".to_owned()];
        assert!(check_attachment(&facts(key), &languages).is_err());
        // Position is irrelevant but still not negative.
        let mut positioned = attachment(Activation::Always);
        positioned.position = 7;
        assert!(check_attachment(&facts(key), &positioned).is_ok());
        positioned.position = -1;
        assert!(check_attachment(&facts(key), &positioned).is_err());
    }

    #[test]
    fn a_server_binding_keeps_the_key_shape_rules_of_a_skill() {
        let server = McpServerId::new();
        let target = BindingRef::McpServer(server);
        let workspace = WorkspaceId::new();
        let project = ProjectId::new();
        let mut key = BindingKey::workspace(target, workspace);
        key.project = Some(project);
        assert!(check_attachment(&facts(key), &attachment(Activation::Always)).is_err());
        let item_kind = BindingKey::item_kind(target, project, ItemKindId::new());
        assert!(check_attachment(&facts(item_kind), &attachment(Activation::Always)).is_err());
        let phase = BindingKey::phase(target, project, PhaseId::new());
        assert!(check_attachment(&facts(phase), &attachment(Activation::Always)).is_err());
        let mut known = facts(phase);
        known.phase_project = Some(project);
        assert!(check_attachment(&known, &attachment(Activation::Always)).is_ok());
    }

    #[test]
    fn a_server_binding_refuses_a_server_that_is_not_there() {
        let server = McpServerId::new();
        let mut absent = facts(BindingKey::global(BindingRef::McpServer(server)));
        absent.target_exists = false;
        let sentence =
            check_attachment(&absent, &attachment(Activation::Always)).expect_err("refused");
        assert!(sentence.contains(&server.to_string()), "{sentence}");
        assert!(sentence.contains("mcp_server"), "{sentence}");
    }
}
