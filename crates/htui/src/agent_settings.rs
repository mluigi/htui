//! The agent registry editor of `Settings > Agents` (MOD-23): the draft a form produces, the rules
//! that parse it, and (T2) the three writes served in the store loop.
//!
//! Everything here but `serve` is pure: no clock, no id, no store, so the section can run it on the
//! UI task (`R-NF-3`) and the worker runs it again before it writes (plan D247). Nothing here reads
//! or branches on an agent's name (`R-AGT-5`): a name is only ever checked against the character
//! rule of [`valid_name`]. Nothing here prints `launch.env` (`R-SEC-2`): the form never holds it,
//! [`AgentDraft`] has no field for it, and no [`Refusal`] quotes a stored `launch` document.

use chrono::{DateTime, Utc};
use htui_agent::launch::AgentLaunch;
use htui_core::model::{Agent, AgentId, Billing, Transport};
use serde_json::Value;

// F-15: the `y`/`n` convention is the Settings tab's, kept in one place, so this store-side module
// borrows it from the UI module rather than owning a second copy.
use crate::ui::tabs::settings::yes_or_no;

/// The form's labels, in tab order (plan D231). `name` is the create form's only; the edit form
/// starts at index 1. The section's fields take their labels from here, so a refusal's field name
/// and the label on screen cannot drift.
pub const FIELD_LABELS: [&str; 8] = [
    "name",
    "transport",
    "command",
    "args",
    "models",
    "default model",
    "billing",
    "enabled (y/n)",
];

/// The field name of a refusal about the stored `launch` document rather than about a typed field.
pub const LAUNCH_FIELD: &str = "launch";

const NAME: &str = FIELD_LABELS[0];
const TRANSPORT: &str = FIELD_LABELS[1];
const COMMAND: &str = FIELD_LABELS[2];
const ARGS: &str = FIELD_LABELS[3];
const MODELS: &str = FIELD_LABELS[4];
const DEFAULT_MODEL: &str = FIELD_LABELS[5];
const BILLING: &str = FIELD_LABELS[6];
const ENABLED: &str = FIELD_LABELS[7];

/// The longest registry name (plan D234).
const NAME_MAX: usize = 64;

/// What the form edits of an `agent` row (plan D231): everything but `name`, `settings` and the
/// keys of `launch` other than `command` and `args`.
///
/// `env` is not here and is never shown (plan D235, `R-SEC-2`). `args` is not a secret channel:
/// secrets travel through `env` or a `${tool}` placeholder, and `AgentLaunch`'s own `Debug` prints
/// `args` too, so the derived `Debug` is safe to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDraft {
    /// `agent.transport`.
    pub transport: Transport,
    /// `agent.launch.command`, trimmed, non-empty.
    pub command: String,
    /// `agent.launch.args`, as shell words split them.
    pub args: Vec<String>,
    /// `agent.models`, in order, no repeats.
    pub models: Vec<String>,
    /// `agent.default_model`.
    pub default_model: Option<String>,
    /// `agent.billing`.
    pub billing: Billing,
    /// `agent.enabled`: the row everywhere, not the per-box switch.
    pub enabled: bool,
}

/// One refused field (plan D247, blueprint D255): which field and why, printed as
/// `` `<field>`: <reason> ``.
///
/// The section shows exactly this sentence and focuses [`field`](Self::field); the worker answers
/// a refusal with the same sentence, so the two cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// One of [`FIELD_LABELS`], or [`LAUNCH_FIELD`] for a stored document the merge cannot use.
    pub field: &'static str,
    /// The sentence, lower case, no trailing period. Never quotes a stored `launch` document.
    pub reason: String,
}

impl Refusal {
    fn new(field: &'static str, reason: impl Into<String>) -> Self {
        Self {
            field,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`: {}", self.field, self.reason)
    }
}

impl std::error::Error for Refusal {}

/// The form's text, one `&str` per field of [`FIELD_LABELS`] after `name`, as typed.
#[derive(Debug, Clone, Copy)]
pub struct DraftFields<'a> {
    /// `transport`: `acp` or `cli`, any case, any surrounding space.
    pub transport: &'a str,
    /// `command`: the executable or a `${tool}` placeholder.
    pub command: &'a str,
    /// `args`: POSIX shell words (plan D236).
    pub args: &'a str,
    /// `models`: comma-separated (plan D237).
    pub models: &'a str,
    /// `default model`: empty for none.
    pub default_model: &'a str,
    /// `billing`: `subscription` or `per_token`, any case, any surrounding space.
    pub billing: &'a str,
    /// `enabled (y/n)`.
    pub enabled: &'a str,
}

/// Whether `name` is a registry name (plan D234): 1 to 64 characters from `[a-z0-9._-]`, the first
/// from `[a-z0-9]`. No trimming: [`parse_name`] trims.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let lower_or_digit = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    // Every accepted character is ASCII, so the byte length is the character count.
    name.len() <= NAME_MAX
        && lower_or_digit(first)
        && chars.all(|c| lower_or_digit(c) || matches!(c, '.' | '_' | '-'))
}

/// The create form's `name` (plan D234), trimmed and checked with [`valid_name`].
///
/// # Errors
///
/// A [`Refusal`] on `name` when the rule does not hold.
pub fn parse_name(text: &str) -> Result<String, Refusal> {
    let name = text.trim();
    if valid_name(name) {
        Ok(name.to_owned())
    } else {
        // The rule, never the text: a refused name may hold a control character.
        Err(Refusal::new(
            NAME,
            "1-64 of a-z 0-9 . _ -, starting with a letter or digit",
        ))
    }
}

/// `transport` (plan D238): trimmed, ASCII-lowercased, then [`Transport`]'s `FromStr`.
///
/// # Errors
///
/// A [`Refusal`] on `transport` naming the accepted values.
pub fn parse_transport(text: &str) -> Result<Transport, Refusal> {
    text.trim()
        .to_ascii_lowercase()
        .parse()
        .map_err(|_| Refusal::new(TRANSPORT, format!("is {}", one_of(Transport::ALL))))
}

/// `command`: trimmed and non-empty.
///
/// # Errors
///
/// A [`Refusal`] on `command` when nothing is left after trimming.
pub fn parse_command(text: &str) -> Result<String, Refusal> {
    let command = text.trim();
    if command.is_empty() {
        Err(Refusal::new(COMMAND, "is required"))
    } else {
        Ok(command.to_owned())
    }
}

/// `args` (plan D236): POSIX shell words. The inverse of [`format_args()`].
///
/// # Errors
///
/// A [`Refusal`] on `args` carrying the parser's sentence (an unclosed quote).
pub fn parse_args(text: &str) -> Result<Vec<String>, Refusal> {
    shell_words::split(text).map_err(|err| Refusal::new(ARGS, err.to_string()))
}

/// `args` as the form prefills it (plan D236): each argument quoted only where it has to be, so
/// [`parse_args`] gives back exactly `args`.
#[must_use]
pub fn format_args(args: &[String]) -> String {
    shell_words::join(args)
}

/// `models` (plan D237): comma-separated, each trimmed, empties dropped, order kept.
///
/// # Errors
///
/// A [`Refusal`] on `models` naming the first model listed twice.
pub fn parse_models(text: &str) -> Result<Vec<String>, Refusal> {
    let mut models: Vec<String> = Vec::new();
    for model in text.split(',').map(str::trim).filter(|m| !m.is_empty()) {
        if models.iter().any(|kept| kept == model) {
            return Err(Refusal::new(MODELS, format!("`{model}` is listed twice")));
        }
        models.push(model.to_owned());
    }
    Ok(models)
}

/// `models` as the form prefills it: joined with `", "`.
#[must_use]
pub fn format_models(models: &[String]) -> String {
    models.join(", ")
}

/// `default model` (plan D237): trimmed, empty for none. A non-empty `models` must list it; an
/// empty `models` accepts any value (the model travels as an argv flag and nothing reports a list).
///
/// # Errors
///
/// A [`Refusal`] on `default model` when a non-empty `models` does not list it.
pub fn parse_default(text: &str, models: &[String]) -> Result<Option<String>, Refusal> {
    let default = text.trim();
    if default.is_empty() {
        return Ok(None);
    }
    if !models.is_empty() && !models.iter().any(|model| model == default) {
        return Err(Refusal::new(
            DEFAULT_MODEL,
            format!("`{default}` is not one of the models"),
        ));
    }
    Ok(Some(default.to_owned()))
}

/// `billing` (plan D238): trimmed, ASCII-lowercased, then [`Billing`]'s `FromStr`.
///
/// # Errors
///
/// A [`Refusal`] on `billing` naming the accepted values.
pub fn parse_billing(text: &str) -> Result<Billing, Refusal> {
    text.trim()
        .to_ascii_lowercase()
        .parse()
        .map_err(|_| Refusal::new(BILLING, format!("is {}", one_of(Billing::ALL))))
}

/// `enabled (y/n)` (plan D238): the Settings tab's `yes_or_no` convention (blueprint F-15).
///
/// # Errors
///
/// A [`Refusal`] on `enabled (y/n)` for anything but `y`, `yes`, `n` or `no`.
pub fn parse_enabled(text: &str) -> Result<bool, Refusal> {
    yes_or_no(text).ok_or_else(|| Refusal::new(ENABLED, "is y or n"))
}

/// The whole form (plan D247): each parser in [`FIELD_LABELS`] order.
///
/// # Errors
///
/// The first field's [`Refusal`], in tab order.
pub fn draft_from_fields(fields: &DraftFields<'_>) -> Result<AgentDraft, Refusal> {
    let transport = parse_transport(fields.transport)?;
    let command = parse_command(fields.command)?;
    let args = parse_args(fields.args)?;
    let models = parse_models(fields.models)?;
    let default_model = parse_default(fields.default_model, &models)?;
    let billing = parse_billing(fields.billing)?;
    let enabled = parse_enabled(fields.enabled)?;
    Ok(AgentDraft {
        transport,
        command,
        args,
        models,
        default_model,
        billing,
        enabled,
    })
}

/// The worker's second pass over a draft it received (plan D247): the draft formatted back into
/// the form's text and run through [`draft_from_fields`] again, so the store boundary trusts
/// nothing the render side built and uses the same rules. Answers the draft as the parsers
/// normalise it.
///
/// # Errors
///
/// The [`Refusal`] the section would have shown for the same text.
pub fn check_draft(draft: &AgentDraft) -> Result<AgentDraft, Refusal> {
    let args = format_args(&draft.args);
    let models = format_models(&draft.models);
    draft_from_fields(&DraftFields {
        transport: draft.transport.as_str(),
        command: &draft.command,
        args: &args,
        models: &models,
        default_model: draft.default_model.as_deref().unwrap_or(""),
        billing: draft.billing.as_str(),
        enabled: if draft.enabled { "y" } else { "n" },
    })
}

/// Merges the form's `command` and `args` into a stored `agent.launch` (plan D235).
///
/// `stored` must be a JSON object. `command` and `args` replace theirs, and every other key is
/// carried unchanged: `env`, `discovery` (with MOD-20's `install` inside it) and any key this
/// build does not know. The result must deserialise as [`AgentLaunch`].
///
/// # Errors
///
/// A [`Refusal`] on [`LAUNCH_FIELD`] when `stored` is not an object or the merge is not a valid
/// `AgentLaunch`. The sentence never quotes the document: serde's own message can quote a
/// value, and `env` values are not for the screen (`R-SEC-2`).
pub fn merge_launch(stored: &Value, command: &str, args: &[String]) -> Result<Value, Refusal> {
    let Some(object) = stored.as_object() else {
        return Err(Refusal::new(
            LAUNCH_FIELD,
            "the stored launch document is not a JSON object",
        ));
    };
    let mut merged = object.clone();
    merged.insert("command".to_owned(), Value::from(command));
    merged.insert("args".to_owned(), Value::from(args.to_vec()));
    let merged = Value::Object(merged);
    checked_launch(merged)
}

/// An edit (plan D235, D239): `stored` with the draft applied.
///
/// The draft is checked again ([`check_draft`]), `launch` is [`merge_launch`]d, and `transport`,
/// `models`, `default_model`, `billing` and `enabled` are replaced. `id`, `name`, `settings`,
/// `created_at` and `updated_at` are kept: the store stamps `updated_at`, and the compare-and-set
/// token is the caller's.
///
/// # Errors
///
/// The [`Refusal`] of [`check_draft`] or [`merge_launch`].
pub fn apply_draft(stored: &Agent, draft: &AgentDraft) -> Result<Agent, Refusal> {
    let draft = check_draft(draft)?;
    let launch = merge_launch(&stored.launch, &draft.command, &draft.args)?;
    Ok(Agent {
        transport: draft.transport,
        launch,
        models: draft.models,
        default_model: draft.default_model,
        billing: draft.billing,
        enabled: draft.enabled,
        ..stored.clone()
    })
}

/// A new row (plan D239): `launch` is `{command, args, env: {}}`, with no `discovery`, checked as
/// [`AgentLaunch`]; `settings` is the caller's (OQ-5); `created_at = updated_at = now`. The name
/// and the draft are checked again ([`parse_name`], [`check_draft`]).
///
/// # Errors
///
/// The [`Refusal`] of [`parse_name`], [`check_draft`] or the `AgentLaunch` check.
pub fn new_agent(
    id: AgentId,
    name: String,
    draft: &AgentDraft,
    settings: Value,
    now: DateTime<Utc>,
) -> Result<Agent, Refusal> {
    let name = parse_name(&name)?;
    let draft = check_draft(draft)?;
    let launch = checked_launch(serde_json::json!({
        "command": draft.command,
        "args": draft.args,
        "env": {},
    }))?;
    Ok(Agent {
        id,
        name,
        transport: draft.transport,
        launch,
        models: draft.models,
        default_model: draft.default_model,
        billing: draft.billing,
        enabled: draft.enabled,
        settings,
        created_at: now,
        updated_at: now,
    })
}

/// The draft a stored row prefills the edit form with (blueprint F-16): `launch.command` if it is a
/// string, else empty; the string elements of `launch.args`, in order. Reads nothing else of
/// `launch`.
#[must_use]
pub fn draft_of(agent: &Agent) -> AgentDraft {
    let command = agent
        .launch
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let args = agent
        .launch
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    AgentDraft {
        transport: agent.transport,
        command,
        args,
        models: agent.models.clone(),
        default_model: agent.default_model.clone(),
        billing: agent.billing,
        enabled: agent.enabled,
    }
}

/// Whether the draft changes what a probe checked (plan D246): `transport`, `command` or `args`.
#[must_use]
pub fn launch_changed(stored: &Agent, draft: &AgentDraft) -> bool {
    let before = draft_of(stored);
    before.transport != draft.transport
        || before.command != draft.command
        || before.args != draft.args
}

/// `"a or b"` / `"a, b or c"`: the accepted values of a closed vocabulary, for a refusal.
fn one_of<T: Copy + std::fmt::Display>(all: &[T]) -> String {
    let words: Vec<String> = all.iter().map(ToString::to_string).collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => words.concat(),
    }
}

/// `launch` checked as [`AgentLaunch`]. The refusal drops serde's sentence, which can quote a
/// value (`invalid type: string "…"`), and an `env` value must not reach the screen (`R-SEC-2`).
fn checked_launch(launch: Value) -> Result<Value, Refusal> {
    match serde_json::from_value::<AgentLaunch>(launch.clone()) {
        Ok(_) => Ok(launch),
        Err(_) => Err(Refusal::new(
            LAUNCH_FIELD,
            "the merged launch document is not a valid AgentLaunch",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::agent::seed_rows;
    use serde_json::json;

    fn epoch() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("a valid instant")
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    fn fields() -> DraftFields<'static> {
        DraftFields {
            transport: "acp",
            command: "/usr/bin/true",
            args: "--flag",
            models: "a, b",
            default_model: "b",
            billing: "subscription",
            enabled: "y",
        }
    }

    fn draft() -> AgentDraft {
        draft_from_fields(&fields()).expect("the base fields parse")
    }

    /// A `launch` shaped like the section suite's `registry_row`, plus an `env` placeholder and a
    /// key this build does not know.
    fn stored_launch() -> Value {
        json!({
            "command": "${demo_server}",
            "args": ["--old"],
            "env": { "TOKEN": "${tok}" },
            "discovery": {
                "tools": {
                    "demo_server": {
                        "kind": "glob",
                        "patterns": ["%HTUI_AGENTS_ROOT%/demo-acp/*/demo_server"],
                    },
                },
                "handshake": true,
                "install": { "source": "acp_registry", "id": "demo-acp", "tool": "demo_server" },
            },
            "x_future": [1],
        })
    }

    fn stored_agent() -> Agent {
        Agent {
            id: AgentId::new(),
            name: "agent-a".to_owned(),
            transport: Transport::Acp,
            launch: stored_launch(),
            models: strings(&["m1", "m2"]),
            default_model: Some("m1".to_owned()),
            billing: Billing::Subscription,
            enabled: true,
            settings: json!({ "acp": { "protocol_version": 1 } }),
            created_at: epoch(),
            updated_at: epoch() + chrono::Duration::seconds(5),
        }
    }

    #[test]
    fn args_round_trip_through_shell_words() {
        let claude = seed_rows(epoch())
            .into_iter()
            .find_map(|row| {
                let args = row.launch.get("args")?.as_array()?.clone();
                (!args.is_empty()).then_some(args)
            })
            .expect("a seed row with args");
        let claude: Vec<String> = claude
            .iter()
            .map(|arg| arg.as_str().expect("a string arg").to_owned())
            .collect();
        let table: Vec<Vec<String>> = vec![
            vec![],
            strings(&[""]),
            strings(&["a b"]),
            strings(&["it's"]),
            strings(&["say \"hi\""]),
            strings(&["--uid="]),
            strings(&["${claude_agent_acp}"]),
            strings(&["C:\\tools\\x.exe"]),
            strings(&[
                "-y",
                "",
                "two words",
                "#not-a-comment",
                "tab\there",
                "new\nline",
            ]),
            claude,
        ];
        for args in table {
            let text = format_args(&args);
            assert_eq!(
                parse_args(&text),
                Ok(args.clone()),
                "{args:?} must survive join then split (plan D236), via {text:?}"
            );
        }
    }

    #[test]
    fn an_unclosed_quote_is_refused_by_field() {
        for text in ["'abc", "\"abc", "a 'b c"] {
            let refusal = parse_args(text).expect_err("an unclosed quote is refused");
            assert_eq!(refusal.field, "args");
            assert_eq!(refusal.to_string(), "`args`: missing closing quote");
        }
        assert_eq!(parse_args("  "), Ok(vec![]), "blank is no arguments");
    }

    #[test]
    fn names_follow_the_d234_rule() {
        for seed in seed_rows(epoch()) {
            assert!(
                valid_name(&seed.name),
                "the seed name {:?} passes",
                seed.name
            );
        }
        for name in ["a", "0", "a.b_c-d", &"x".repeat(64)] {
            assert!(valid_name(name), "{name:?} passes");
        }
        for name in [
            "",
            &"x".repeat(65),
            "Claude",
            "a b",
            "-x",
            ".x",
            "_x",
            "a\u{7}",
            "é",
            " a",
        ] {
            assert!(!valid_name(name), "{name:?} is refused");
        }
        assert_eq!(parse_name("  agent-x "), Ok("agent-x".to_owned()));
        assert_eq!(
            parse_name("Bad Name").map_err(|refusal| refusal.to_string()),
            Err("`name`: 1-64 of a-z 0-9 . _ -, starting with a letter or digit".to_owned())
        );
    }

    #[test]
    fn a_name_refusal_never_echoes_the_name() {
        let refusal = parse_name("a\u{1b}[31mb").expect_err("a control character is refused");
        assert!(!refusal.to_string().contains('\u{1b}'));
    }

    #[test]
    fn models_keep_order_drop_empties_and_refuse_a_repeat() {
        assert_eq!(
            parse_models(" b, a,, b ").map_err(|refusal| refusal.to_string()),
            Err("`models`: `b` is listed twice".to_owned())
        );
        assert_eq!(parse_models(" b, a ,"), Ok(strings(&["b", "a"])));
        assert_eq!(parse_models(""), Ok(vec![]));
        assert_eq!(parse_models(" , ,"), Ok(vec![]));
        assert_eq!(format_models(&strings(&["b", "a"])), "b, a");
        assert_eq!(format_models(&[]), "");
    }

    #[test]
    fn a_default_must_be_listed_unless_the_list_is_empty() {
        let listed = strings(&["a"]);
        assert_eq!(
            parse_default("x", &listed).map_err(|refusal| refusal.to_string()),
            Err("`default model`: `x` is not one of the models".to_owned())
        );
        assert_eq!(parse_default(" a ", &listed), Ok(Some("a".to_owned())));
        assert_eq!(parse_default("x", &[]), Ok(Some("x".to_owned())));
        assert_eq!(parse_default("  ", &listed), Ok(None));
        assert_eq!(parse_default("", &[]), Ok(None));
    }

    #[test]
    fn transport_and_billing_accept_case_and_space_and_name_the_values() {
        assert_eq!(parse_transport(" ACP "), Ok(Transport::Acp));
        assert_eq!(parse_transport("cli"), Ok(Transport::Cli));
        assert_eq!(
            parse_transport("ssh").map_err(|refusal| refusal.to_string()),
            Err("`transport`: is acp or cli".to_owned())
        );
        assert_eq!(parse_billing("Per_Token"), Ok(Billing::PerToken));
        assert_eq!(parse_billing(" subscription\t"), Ok(Billing::Subscription));
        assert_eq!(
            parse_billing("free").map_err(|refusal| refusal.to_string()),
            Err("`billing`: is subscription or per_token".to_owned())
        );
    }

    #[test]
    fn command_is_trimmed_and_required() {
        assert_eq!(parse_command("  ${node} "), Ok("${node}".to_owned()));
        assert_eq!(
            parse_command(" \t").map_err(|refusal| refusal.to_string()),
            Err("`command`: is required".to_owned())
        );
    }

    #[test]
    fn enabled_is_y_or_n() {
        assert_eq!(parse_enabled(" Y "), Ok(true));
        assert_eq!(parse_enabled("no"), Ok(false));
        assert_eq!(
            parse_enabled("maybe").map_err(|refusal| refusal.to_string()),
            Err("`enabled (y/n)`: is y or n".to_owned())
        );
    }

    #[test]
    fn every_refusal_names_a_form_label() {
        let refusals = [
            parse_name("").expect_err("empty"),
            parse_transport("").expect_err("empty"),
            parse_command("").expect_err("empty"),
            parse_args("'").expect_err("unclosed"),
            parse_models("a, a").expect_err("repeat"),
            parse_default("x", &strings(&["a"])).expect_err("unlisted"),
            parse_billing("").expect_err("empty"),
            parse_enabled("").expect_err("empty"),
        ];
        let named: Vec<&str> = refusals.iter().map(|refusal| refusal.field).collect();
        assert_eq!(named, FIELD_LABELS, "one refusal per label, in tab order");
    }

    #[test]
    fn draft_from_fields_parses_every_field() {
        assert_eq!(
            draft(),
            AgentDraft {
                transport: Transport::Acp,
                command: "/usr/bin/true".to_owned(),
                args: strings(&["--flag"]),
                models: strings(&["a", "b"]),
                default_model: Some("b".to_owned()),
                billing: Billing::Subscription,
                enabled: true,
            }
        );
    }

    #[test]
    fn draft_from_fields_returns_the_first_refusal_in_field_order() {
        let bad = DraftFields {
            transport: "ssh",
            models: "a, a",
            enabled: "?",
            ..fields()
        };
        assert_eq!(
            draft_from_fields(&bad).map_err(|refusal| refusal.field),
            Err("transport")
        );
        let bad = DraftFields {
            args: "'",
            default_model: "zzz",
            ..fields()
        };
        assert_eq!(
            draft_from_fields(&bad).map_err(|refusal| refusal.field),
            Err("args")
        );
        let bad = DraftFields {
            default_model: "zzz",
            billing: "free",
            ..fields()
        };
        assert_eq!(
            draft_from_fields(&bad).map_err(|refusal| refusal.field),
            Err("default model")
        );
    }

    #[test]
    fn check_draft_runs_the_form_rules_again() {
        assert_eq!(check_draft(&draft()), Ok(draft()));
        let unlisted = AgentDraft {
            default_model: Some("zzz".to_owned()),
            ..draft()
        };
        assert_eq!(
            check_draft(&unlisted).map_err(|refusal| refusal.to_string()),
            Err("`default model`: `zzz` is not one of the models".to_owned())
        );
        let blank = AgentDraft {
            command: "  ".to_owned(),
            ..draft()
        };
        assert_eq!(
            check_draft(&blank).map_err(|refusal| refusal.field),
            Err("command")
        );
        let repeated = AgentDraft {
            models: strings(&["a", "a"]),
            default_model: None,
            ..draft()
        };
        assert_eq!(
            check_draft(&repeated).map_err(|refusal| refusal.field),
            Err("models")
        );
        let padded = AgentDraft {
            command: " /bin/x ".to_owned(),
            models: strings(&[" a", "", "b "]),
            default_model: Some(" a ".to_owned()),
            ..draft()
        };
        assert_eq!(
            check_draft(&padded),
            Ok(AgentDraft {
                command: "/bin/x".to_owned(),
                models: strings(&["a", "b"]),
                default_model: Some("a".to_owned()),
                ..draft()
            }),
            "normalised the way the form's parsers normalise"
        );
        let empty_default = AgentDraft {
            default_model: Some(String::new()),
            ..draft()
        };
        assert_eq!(
            check_draft(&empty_default).map(|checked| checked.default_model),
            Ok(None)
        );
    }

    #[test]
    fn merge_launch_keeps_env_discovery_install_and_unknown_keys() {
        let stored = stored_launch();
        let merged =
            merge_launch(&stored, "/opt/x", &strings(&["--a", "b c"])).expect("a valid merge");
        assert_eq!(merged["command"], json!("/opt/x"));
        assert_eq!(merged["args"], json!(["--a", "b c"]));
        for key in ["env", "discovery", "x_future"] {
            assert_eq!(merged[key], stored[key], "`{key}` is carried unchanged");
        }
        assert_eq!(
            merged["discovery"]["install"], stored["discovery"]["install"],
            "MOD-20's install block survives"
        );
        let keys: Vec<&String> = merged.as_object().expect("an object").keys().collect();
        let stored_keys: Vec<&String> = stored.as_object().expect("an object").keys().collect();
        assert_eq!(keys, stored_keys, "no key added, none dropped");
    }

    #[test]
    fn merge_launch_adds_command_and_args_to_an_object_without_them() {
        let merged = merge_launch(&json!({ "env": {} }), "x", &[]).expect("a valid merge");
        assert_eq!(merged, json!({ "command": "x", "args": [], "env": {} }));
    }

    #[test]
    fn merge_launch_refuses_a_non_object_and_an_invalid_result() {
        for stored in [json!([]), json!(null), json!("x")] {
            assert_eq!(
                merge_launch(&stored, "x", &[]).map_err(|refusal| refusal.to_string()),
                Err("`launch`: the stored launch document is not a JSON object".to_owned())
            );
        }
        let bad_discovery = json!({ "command": "x", "discovery": 7 });
        assert_eq!(
            merge_launch(&bad_discovery, "x", &[]).map_err(|refusal| refusal.to_string()),
            Err("`launch`: the merged launch document is not a valid AgentLaunch".to_owned())
        );
    }

    #[test]
    fn a_launch_refusal_never_quotes_an_env_value() {
        // serde's own sentence would be `invalid type: string "TOKEN=hunter2", expected a map`.
        let stored = json!({ "command": "x", "env": "TOKEN=hunter2" });
        let refusal = merge_launch(&stored, "x", &[]).expect_err("a string env is refused");
        assert_eq!(refusal.field, LAUNCH_FIELD);
        assert!(!refusal.to_string().contains("hunter2"), "{refusal}");
        assert!(!format!("{refusal:?}").contains("hunter2"), "{refusal:?}");
    }

    #[test]
    fn apply_draft_keeps_name_settings_and_both_stamps() {
        let stored = stored_agent();
        let edit = AgentDraft {
            transport: Transport::Cli,
            billing: Billing::PerToken,
            enabled: false,
            ..draft()
        };
        let merged = apply_draft(&stored, &edit).expect("a valid edit");
        assert_eq!(merged.id, stored.id);
        assert_eq!(merged.name, stored.name);
        assert_eq!(merged.settings, stored.settings);
        assert_eq!(merged.created_at, stored.created_at);
        assert_eq!(merged.updated_at, stored.updated_at);
        assert_eq!(merged.transport, Transport::Cli);
        assert_eq!(merged.models, strings(&["a", "b"]));
        assert_eq!(merged.default_model, Some("b".to_owned()));
        assert_eq!(merged.billing, Billing::PerToken);
        assert!(!merged.enabled);
        assert_eq!(merged.launch["command"], json!("/usr/bin/true"));
        assert_eq!(merged.launch["args"], json!(["--flag"]));
        assert_eq!(merged.launch["env"], stored.launch["env"]);
        assert_eq!(merged.launch["discovery"], stored.launch["discovery"]);
    }

    #[test]
    fn apply_draft_refuses_a_draft_the_form_would_refuse() {
        let unlisted = AgentDraft {
            default_model: Some("zzz".to_owned()),
            ..draft()
        };
        assert_eq!(
            apply_draft(&stored_agent(), &unlisted).map_err(|refusal| refusal.field),
            Err("default model")
        );
        let broken = Agent {
            launch: json!([]),
            ..stored_agent()
        };
        assert_eq!(
            apply_draft(&broken, &draft()).map_err(|refusal| refusal.field),
            Err(LAUNCH_FIELD)
        );
    }

    #[test]
    fn new_agent_has_a_blank_launch_and_the_given_settings() {
        let id = AgentId::new();
        let settings = json!({ "cli": { "stream": "claude_stream_json" } });
        let row = new_agent(
            id,
            "agent-x".to_owned(),
            &draft(),
            settings.clone(),
            epoch(),
        )
        .expect("a valid row");
        assert_eq!(row.id, id);
        assert_eq!(row.name, "agent-x");
        assert_eq!(
            row.launch,
            json!({ "command": "/usr/bin/true", "args": ["--flag"], "env": {} }),
            "no discovery: the probe must not look for another row's tools"
        );
        assert_eq!(row.settings, settings);
        assert_eq!(row.created_at, epoch());
        assert_eq!(row.updated_at, epoch());
        assert_eq!(row.transport, Transport::Acp);
        assert_eq!(row.models, strings(&["a", "b"]));
        assert_eq!(row.default_model, Some("b".to_owned()));
        assert_eq!(row.billing, Billing::Subscription);
        assert!(row.enabled);
        serde_json::from_value::<AgentLaunch>(row.launch).expect("an AgentLaunch");
    }

    #[test]
    fn new_agent_checks_the_name_and_the_draft_again() {
        assert_eq!(
            new_agent(
                AgentId::new(),
                "Bad Name".to_owned(),
                &draft(),
                json!({}),
                epoch()
            )
            .map_err(|refusal| refusal.field),
            Err("name")
        );
        let blank = AgentDraft {
            command: String::new(),
            ..draft()
        };
        assert_eq!(
            new_agent(AgentId::new(), "ok".to_owned(), &blank, json!({}), epoch())
                .map_err(|refusal| refusal.field),
            Err("command")
        );
    }

    #[test]
    fn draft_of_prefills_from_the_row_and_round_trips_through_the_form() {
        let stored = stored_agent();
        let prefill = draft_of(&stored);
        assert_eq!(
            prefill,
            AgentDraft {
                transport: Transport::Acp,
                command: "${demo_server}".to_owned(),
                args: strings(&["--old"]),
                models: strings(&["m1", "m2"]),
                default_model: Some("m1".to_owned()),
                billing: Billing::Subscription,
                enabled: true,
            }
        );
        for seed in seed_rows(epoch()) {
            let prefill = draft_of(&seed);
            let args = format_args(&prefill.args);
            let models = format_models(&prefill.models);
            let retyped = draft_from_fields(&DraftFields {
                transport: prefill.transport.as_str(),
                command: &prefill.command,
                args: &args,
                models: &models,
                default_model: prefill.default_model.as_deref().unwrap_or(""),
                billing: prefill.billing.as_str(),
                enabled: if prefill.enabled { "y" } else { "n" },
            });
            assert_eq!(
                retyped,
                Ok(prefill),
                "an untouched form over a seed row parses back to its prefill"
            );
        }
    }

    #[test]
    fn draft_of_tolerates_a_launch_it_cannot_read() {
        let odd = Agent {
            launch: json!({ "command": 7, "args": ["a", 1, "b"] }),
            ..stored_agent()
        };
        let prefill = draft_of(&odd);
        assert_eq!(prefill.command, "");
        assert_eq!(prefill.args, strings(&["a", "b"]));
        let none = Agent {
            launch: json!(null),
            ..stored_agent()
        };
        assert_eq!(draft_of(&none).command, "");
        assert!(draft_of(&none).args.is_empty());
    }

    #[test]
    fn launch_changed_sees_only_transport_command_and_args() {
        let stored = stored_agent();
        let same = draft_of(&stored);
        assert!(!launch_changed(&stored, &same));
        let models_only = AgentDraft {
            models: strings(&["z"]),
            default_model: None,
            billing: Billing::PerToken,
            enabled: false,
            ..same.clone()
        };
        assert!(!launch_changed(&stored, &models_only));
        let args = AgentDraft {
            args: strings(&["--new"]),
            ..same.clone()
        };
        assert!(launch_changed(&stored, &args));
        let command = AgentDraft {
            command: "/bin/other".to_owned(),
            ..same.clone()
        };
        assert!(launch_changed(&stored, &command));
        let transport = AgentDraft {
            transport: Transport::Cli,
            ..same
        };
        assert!(launch_changed(&stored, &transport));
    }

    #[test]
    fn a_draft_debug_carries_no_env() {
        let prefill = draft_of(&stored_agent());
        let printed = format!("{prefill:?}");
        assert!(!printed.contains("TOKEN"), "{printed}");
        assert!(!printed.contains("${tok}"), "{printed}");
    }
}
