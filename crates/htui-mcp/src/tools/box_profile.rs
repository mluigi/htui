//! `box_profile` (MOD-11 M1, plan "Tools"): the scope's box, as JSON and as the prompt renders it.
//!
//! The text is [`render::box_profile`] with the scope's [`HostnameLine`], so the tool and the
//! prompt's `box` section cannot disagree (PRD OQ-3); with the project's switch off the JSON loses
//! its `hostname` key and the text starts `os:`.

use htui_core::prompt::render::{self, HostnameLine};
use serde_json::{Value, json};

use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, store_error};

// No arguments: the box is the scope's (I-1), so a `run_id` or a `box_id` is refused. A plain
// comment, not a doc: schemars would hand a doc to the agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BoxProfileArgs {}

/// Always advertised.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "box_profile",
    description: "Describes the machine this session runs on: its OS, CPU, RAM, GPU and the tools \
                  found on it.",
    schema: schema_of::<BoxProfileArgs>,
    advertised: |_, _| true,
};

/// `{"profile": <BoxProfile>, "text": <the prompt's box section>}`; the `hostname` key only when
/// the scope shows it.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let BoxProfileArgs {} = args(arguments)?;
    let scope = &ctx.session.scope;
    let profile = htui_core::store::WorkerHost::box_profile(&ctx.host, scope.box_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ToolError(format!("not found: box {}", scope.box_id)))?;
    let text = render::box_profile(&profile, scope.hostname).content;
    let mut profile = serde_json::to_value(&profile)
        .map_err(|err| ToolError(format!("store unavailable: {err}")))?;
    if scope.hostname != HostnameLine::Shown
        && let Some(fields) = profile.as_object_mut()
    {
        fields.remove("hostname");
    }
    Ok(json!({"profile": profile, "text": text}))
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::model::{BoxId, ItemId, Transport};
    use htui_core::prompt::render::{self, HostnameLine};
    use htui_core::store::MemStore;
    use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
    use serde_json::{Value, json};

    use crate::ENV_TOKEN;
    use crate::host::McpClient;
    use crate::host::tests::{demo_host, scope};
    use crate::protocol::CallResult;

    /// The demo box's profile, as the store projects it.
    async fn demo_profile() -> htui_core::model::BoxProfile {
        MemStore::demo()
            .box_profile(ids::BOX)
            .await
            .expect("a read")
            .expect("the demo box has a profile")
    }

    /// A client of a fresh session on `scope`; the lease must outlive the client's calls.
    async fn session(scope: ToolScope) -> (ToolLease, McpClient) {
        let host = demo_host();
        let lease = host.open(scope).expect("a lease");
        let mut client = host
            .client(&lease.spec.env[ENV_TOKEN])
            .expect("a live session");
        client.initialize().await.expect("initialize");
        (lease, client)
    }

    async fn profile_call(hostname: HostnameLine) -> CallResult {
        let mut on = scope(Transport::Acp);
        on.hostname = hostname;
        let (_lease, mut client) = session(on).await;
        client.call("box_profile", json!({})).await.expect("a call")
    }

    fn parsed(result: &CallResult) -> Value {
        assert!(!result.is_error, "{}", result.text);
        serde_json::from_str(&result.text).expect("the result is JSON")
    }

    #[tokio::test]
    async fn box_profile_omits_the_hostname_when_the_switch_is_off() {
        let answer = parsed(&profile_call(HostnameLine::Omitted).await);
        let profile = answer["profile"].as_object().expect("a profile object");
        assert!(!profile.contains_key("hostname"), "{answer:#}");
        assert!(profile.contains_key("os_family"));
        let text = answer["text"].as_str().expect("text");
        assert!(text.starts_with("os:"), "{text}");
        assert!(!text.contains("hostname"), "{text}");
    }

    #[tokio::test]
    async fn box_profile_shows_the_hostname_when_on() {
        let expected = demo_profile().await;
        let answer = parsed(&profile_call(HostnameLine::Shown).await);
        assert_eq!(answer["profile"]["hostname"], expected.hostname.as_str());
        let text = answer["text"].as_str().expect("text");
        assert!(
            text.starts_with(&format!("hostname: {}", expected.hostname)),
            "{text}"
        );
    }

    #[tokio::test]
    async fn box_profile_text_is_the_prompts_render() {
        let expected = demo_profile().await;
        for hostname in [HostnameLine::Omitted, HostnameLine::Shown] {
            let answer = parsed(&profile_call(hostname).await);
            assert_eq!(
                answer["text"],
                render::box_profile(&expected, hostname).content.as_str()
            );
        }
    }

    #[tokio::test]
    async fn an_argument_naming_a_run_is_refused() {
        let (_lease, mut client) = session(scope(Transport::Acp)).await;
        let refused = client
            .call(
                "box_profile",
                json!({"run_id": "0190a5b6-0000-7000-8000-000000000000"}),
            )
            .await
            .expect("a call");
        assert!(refused.is_error);
        assert!(
            refused
                .text
                .starts_with("invalid arguments: unknown field `run_id`"),
            "{}",
            refused.text
        );
    }

    #[tokio::test]
    async fn a_box_with_no_profile_is_not_found() {
        let mut on = scope(Transport::Acp);
        let missing = BoxId::new();
        on.box_id = missing;
        let (_lease, mut client) = session(on).await;
        let answer = client.call("box_profile", json!({})).await.expect("a call");
        assert!(answer.is_error);
        assert_eq!(answer.text, format!("not found: box {missing}"));
    }

    #[tokio::test]
    async fn tools_list_per_scope_shape() {
        let item = Some(ItemId::new());
        let fresh_chat = scope(Transport::Acp);
        let promoted_chat = ToolScope {
            item_id: item,
            output_kind: Some("plan".to_owned()),
            ..scope(Transport::Acp)
        };
        let phase_step = ToolScope {
            item_id: item,
            output_kind: Some("implementation".to_owned()),
            hostname: HostnameLine::Shown,
            ..scope(Transport::Acp)
        };
        let cli_phase_step = ToolScope {
            transport: Transport::Cli,
            ..phase_step.clone()
        };
        let mut lists = serde_json::Map::new();
        for (shape, on) in [
            ("fresh chat", fresh_chat),
            ("promoted chat", promoted_chat),
            ("phase step", phase_step),
            ("CLI phase step", cli_phase_step),
        ] {
            let (_lease, mut client) = session(on).await;
            let answer = client
                .request("tools/list", json!({}))
                .await
                .expect("tools/list");
            lists.insert(shape.to_owned(), answer["result"]["tools"].clone());
        }
        insta::assert_snapshot!(
            serde_json::to_string_pretty(&Value::Object(lists)).expect("serialises")
        );
    }
}
