//! The client half of the ACP connection: what `htui` advertises, and what the agent may ask it.
//!
//! Three inbound requests reach a client in v1 that this milestone serves —
//! `session/request_permission`, `fs/read_text_file` and `fs/write_text_file` — and every handler
//! here does exactly one thing: forward the request and its responder to the session task and
//! return. A handler that awaited a store write, a file read or a `SentRequest::block_task` would
//! be the documented deadlock of `docs/ANA-4.md` §4.2 (risk 11), so none of them awaits anything.
//!
//! The terminal capability is **not** advertised (§4.3 "Terminal-backed tool content"): MOD-2 does
//! not implement `terminal/*`, and an omitted capability MUST be treated as unsupported (§3).

use agent_client_protocol::Responder;
use agent_client_protocol::schema::v1::{
    ClientCapabilities as AcpClientCapabilities, FileSystemCapabilities, Implementation,
    ReadTextFileRequest, ReadTextFileResponse, RequestId, RequestPermissionRequest,
    RequestPermissionResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use tokio::sync::mpsc;

use crate::driver::PermissionRequestId;
use crate::event::{PermissionOption, PermissionOptionKind, PermissionRequestEvent};
use crate::launch::ClientCapabilities;

/// What `htui` advertises in `initialize.clientCapabilities`.
///
/// `fs.readTextFile` and `fs.writeTextFile` come from `agent.settings.acp.client_capabilities`
/// (§5.2) — advertising the write is what buys an `edit_proposal` with a real diff instead of a
/// post-hoc tool call (§4.3). `terminal` is `false` whatever the row says, because the row's flag
/// describes an intent MOD-11 will implement and this build has no `terminal/*` handler; claiming
/// it would earn requests that could only be answered with an error.
#[must_use]
pub fn client_capabilities(settings: &ClientCapabilities) -> AcpClientCapabilities {
    AcpClientCapabilities::new()
        .fs(FileSystemCapabilities::new()
            .read_text_file(settings.fs_read)
            .write_text_file(settings.fs_write))
        .terminal(false)
}

/// `initialize.clientInfo`: this binary, by name and version.
#[must_use]
pub fn client_info() -> Implementation {
    Implementation::new("htui", env!("CARGO_PKG_VERSION"))
}

/// The JSON-RPC id of an inbound request, as the correlation key ANA-9 §4.3 stores.
///
/// Matched on the variant rather than rendered through `Display`: the id is what pairs a
/// `permission_request` row with its `permission_answer`, and it must not change shape because a
/// derive did.
#[must_use]
pub fn request_id(id: &RequestId) -> PermissionRequestId {
    PermissionRequestId::new(match id {
        RequestId::Number(number) => number.to_string(),
        RequestId::Str(text) => text.clone(),
        RequestId::Null => "null".to_owned(),
    })
}

/// Decodes a `session/request_permission` into the row ANA-9 §4.3 stores.
///
/// All four `PermissionOptionKind` values are accepted even though the `claude` adapter currently
/// offers three (§4.3); the kind is a UI hint and the `optionId` is what is sent back. An
/// unrecognised kind decodes as `reject_once` with a `warn!` — a hint whose meaning is unknown is
/// rendered as the conservative door, and the option's own id still round-trips unchanged.
#[must_use]
pub fn permission_event(
    request: &RequestPermissionRequest,
    id: PermissionRequestId,
) -> PermissionRequestEvent {
    PermissionRequestEvent {
        request_id: id,
        tool_call_id: Some(request.tool_call.tool_call_id.0.to_string()),
        options: request
            .options
            .iter()
            .map(|option| PermissionOption {
                id: option.option_id.0.to_string(),
                label: option.name.clone(),
                kind: option_kind(&option.kind),
            })
            .collect(),
    }
}

/// The wire text of a schema `PermissionOptionKind`, mapped onto `htui`'s own four values.
///
/// Serialised rather than matched: the schema enum is the protocol's, its variant names are not
/// `htui`'s to depend on, and its serde renames *are* the wire vocabulary §3 defines.
fn option_kind(
    kind: &agent_client_protocol::schema::v1::PermissionOptionKind,
) -> PermissionOptionKind {
    let text = serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned));
    let decoded = text.as_deref().and_then(|text| {
        PermissionOptionKind::ALL
            .iter()
            .copied()
            .find(|known| known.as_str() == text)
    });
    decoded.unwrap_or_else(|| {
        tracing::warn!(
            kind = text.as_deref().unwrap_or("<unserialisable>"),
            "unknown permission option kind; rendered as reject_once"
        );
        PermissionOptionKind::RejectOnce
    })
}

/// What an inbound handler forwards to the session task.
///
/// The responder travels with its request: the task parks it across the UI round trip and answers
/// it later, which is the whole reason a handler must not answer inline (§4.3 stage 3).
#[derive(Debug)]
pub enum Inbound {
    /// `session/request_permission`.
    Permission(
        Box<RequestPermissionRequest>,
        Responder<RequestPermissionResponse>,
    ),
    /// `fs/read_text_file`.
    ReadFile(Box<ReadTextFileRequest>, Responder<ReadTextFileResponse>),
    /// `fs/write_text_file`, which the session task intercepts into an `edit_proposal` (§4.3).
    WriteFile(Box<WriteTextFileRequest>, Responder<WriteTextFileResponse>),
}

/// The handler → task edge. Unbounded because a handler must never block (§4.2).
pub type InboundTx = mpsc::UnboundedSender<Inbound>;

/// Forwards a permission request. Never answers it: stage 3 is the user's.
pub fn forward_permission(
    tx: &InboundTx,
    request: RequestPermissionRequest,
    responder: Responder<RequestPermissionResponse>,
) {
    send(tx, Inbound::Permission(Box::new(request), responder));
}

/// Forwards a file read.
pub fn forward_read(
    tx: &InboundTx,
    request: ReadTextFileRequest,
    responder: Responder<ReadTextFileResponse>,
) {
    send(tx, Inbound::ReadFile(Box::new(request), responder));
}

/// Forwards a file write.
pub fn forward_write(
    tx: &InboundTx,
    request: WriteTextFileRequest,
    responder: Responder<WriteTextFileResponse>,
) {
    send(tx, Inbound::WriteFile(Box::new(request), responder));
}

/// Sends, and says so when the task is already gone.
///
/// A failed send drops the responder, which is what the SDK's drop guard turns into an error
/// response: the agent hears a failure rather than waiting forever on a task that has ended.
fn send(tx: &InboundTx, inbound: Inbound) {
    if tx.send(inbound).is_err() {
        tracing::warn!("session task is gone; an inbound request was dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_advertised_capabilities_follow_the_row_but_never_claim_a_terminal() {
        let caps = client_capabilities(&ClientCapabilities {
            fs_read: true,
            fs_write: true,
            terminal: true,
            elicitation: true,
        });
        assert!(caps.fs.read_text_file);
        assert!(caps.fs.write_text_file);
        assert!(
            !caps.terminal,
            "MOD-2 implements no `terminal/*` handler, so it must not advertise one (§4.3)"
        );

        let none = client_capabilities(&ClientCapabilities {
            fs_read: false,
            fs_write: false,
            terminal: false,
            elicitation: false,
        });
        assert!(!none.fs.read_text_file);
        assert!(!none.fs.write_text_file);
    }

    /// Plan MOD-21 D21: the two capabilities a login must **not** claim, pinned rather than
    /// changed.
    ///
    /// `auth.terminal` is what makes an agent allowed to advertise a `terminal`-typed auth method
    /// at all, and `htui` runs no interactive child, so a flow that saw one could only hide it
    /// (`acp::auth`'s `hidden`). `elicitation` is what makes an agent allowed to ask a structured
    /// question mid-`authenticate`; unadvertised, the SDK answers such a request "method not
    /// found" by itself, so the agent hears a refusal instead of the flow hanging on a request
    /// this build has no handler for.
    ///
    /// Asserted on the **serialised** document rather than on the fields, so the case pins what
    /// goes on the wire and not the schema type's shape: a field that is absent and a field that
    /// is `false` are the same claim to an agent (§3, an omitted capability is unsupported), and
    /// this test must not have to be rewritten when the schema chooses between them.
    #[test]
    fn the_advertised_capabilities_claim_neither_terminal_auth_nor_elicitation() {
        let value = serde_json::to_value(client_capabilities(&ClientCapabilities {
            fs_read: true,
            fs_write: true,
            terminal: true,
            elicitation: true,
        }))
        .expect("the capability document serialises");

        let terminal_auth = value.pointer("/auth/terminal");
        assert!(
            matches!(
                terminal_auth,
                None | Some(serde_json::Value::Null) | Some(serde_json::Value::Bool(false))
            ),
            "a terminal auth method would need an interactive child this build never runs: {value}"
        );
        let elicitation = value.get("elicitation");
        assert!(
            matches!(elicitation, None | Some(serde_json::Value::Null)),
            "an elicitation `htui` cannot answer turns a login into a hung one: {value}"
        );
    }

    #[test]
    fn the_client_info_names_this_binary() {
        let info = client_info();
        assert_eq!(info.name, "htui");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_request_id_keeps_its_wire_shape() {
        assert_eq!(request_id(&RequestId::Number(7)).as_str(), "7");
        assert_eq!(
            request_id(&RequestId::Str("req-1".to_owned())).as_str(),
            "req-1"
        );
        assert_eq!(request_id(&RequestId::Null).as_str(), "null");
    }

    /// Every kind the protocol defines decodes; the `htui` and schema vocabularies agree by text.
    #[test]
    fn all_four_permission_option_kinds_decode() {
        use agent_client_protocol::schema::v1::PermissionOptionKind as Wire;
        let pairs = [
            (Wire::AllowOnce, PermissionOptionKind::AllowOnce),
            (Wire::AllowAlways, PermissionOptionKind::AllowAlways),
            (Wire::RejectOnce, PermissionOptionKind::RejectOnce),
            (Wire::RejectAlways, PermissionOptionKind::RejectAlways),
        ];
        for (wire, expected) in pairs {
            assert_eq!(option_kind(&wire), expected);
        }
    }
}
