//! The CLI permission bridge (MOD-11 D18): the `permission_prompt` MCP tool asks, the CLI session
//! answers through the ordinary `DriverEvent::PermissionRequest` / `answer_permission` pair.
//!
//! [`bridge`] mints the two ends with one id. The session end, [`PromptPort`], rides in
//! `SessionSpec.prompt`; the tool end, [`PromptAsk`], stays with `htui`'s MCP session. This module
//! holds the types only: which session takes the port, and how a request becomes a
//! `PermissionRequest`, belong to the CLI transport.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// Depth of the request channel: one session asks at most one question per tool call.
pub const PROMPT_CAPACITY: usize = 16;

/// Mints a connected pair: the session's receiving end and the tool's asking end, one id.
#[must_use]
pub fn bridge() -> (PromptPort, PromptAsk) {
    let id = Uuid::now_v7();
    let (tx, rx) = mpsc::channel(PROMPT_CAPACITY);
    (
        PromptPort {
            id,
            rx: Arc::new(Mutex::new(Some(rx))),
        },
        PromptAsk { id, tx },
    )
}

/// The session side, carried in `SessionSpec.prompt`. `Clone` because `SessionSpec` is; the
/// receiver is taken once ([`take`](Self::take)). Equality is by `id` (tokio's channels have no
/// `PartialEq`).
#[derive(Clone)]
pub struct PromptPort {
    id: Uuid,
    rx: Arc<Mutex<Option<mpsc::Receiver<PromptRequest>>>>,
}

impl PromptPort {
    /// The id this port shares with its [`PromptAsk`]: an identity, not a secret.
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// The receiver, the first time; `None` after (a cloned spec does not get a second stream).
    #[must_use]
    pub fn take(&self) -> Option<mpsc::Receiver<PromptRequest>> {
        // A poisoned lock only means another taker panicked; the `Option` inside is still whole.
        self.rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl PartialEq for PromptPort {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for PromptPort {}

impl core::fmt::Debug for PromptPort {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PromptPort({})", self.id)
    }
}

/// The tool side, kept by `McpHost`'s session for a `Transport::Cli` scope.
#[derive(Clone)]
pub struct PromptAsk {
    id: Uuid,
    tx: mpsc::Sender<PromptRequest>,
}

impl PromptAsk {
    /// The id this end shares with its [`PromptPort`].
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Sends one request and waits for its verdict.
    ///
    /// # Errors
    ///
    /// [`PromptClosed`] when the session dropped the port or the answer.
    pub async fn ask(&self, call: PromptCall) -> Result<PromptVerdict, PromptClosed> {
        let (answer, verdict) = oneshot::channel();
        self.tx
            .send(PromptRequest { call, answer })
            .await
            .map_err(|_| PromptClosed)?;
        verdict.await.map_err(|_| PromptClosed)
    }
}

impl core::fmt::Debug for PromptAsk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PromptAsk({})", self.id)
    }
}

/// What the CLI's `--permission-prompt-tool` call carries.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PromptCall {
    /// The tool the agent wants to run (`Bash`, `Write`, `mcp__server__tool`, ...).
    pub tool_name: String,
    /// The tool's input, verbatim.
    pub input: serde_json::Value,
    /// The CLI's tool-use id, when it sends one.
    #[serde(default)]
    pub tool_use_id: Option<String>,
}

/// One request on the wire between the tool and the session.
#[derive(Debug)]
pub struct PromptRequest {
    /// The question.
    pub call: PromptCall,
    /// Where the session sends its verdict; dropping it unanswered closes the ask.
    pub answer: oneshot::Sender<PromptVerdict>,
}

/// The answer the tool turns into the CLI's JSON (`{"behavior":"allow","updatedInput":…}` /
/// `{"behavior":"deny","message":…}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptVerdict {
    /// Run the tool with its input unchanged.
    Allow,
    /// Refuse the tool; the agent reads `message`.
    Deny {
        /// Why, in a sentence the agent can act on.
        message: String,
    },
}

/// The session is gone: the tool answers `deny` with this sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the agent session ended before the permission was answered")]
pub struct PromptClosed;

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &str) -> PromptCall {
        PromptCall {
            tool_name: tool.to_owned(),
            input: serde_json::json!({ "command": "ls" }),
            tool_use_id: Some("toolu_1".to_owned()),
        }
    }

    #[tokio::test]
    async fn a_port_equals_its_clone_and_no_other_port() {
        let (port, ask) = bridge();
        let (other, _other_ask) = bridge();
        assert_eq!(port, port.clone());
        assert_eq!(port.id(), ask.id());
        assert_ne!(port, other);
        assert_ne!(port.id(), other.id());
    }

    #[tokio::test]
    async fn take_yields_the_receiver_once() {
        let (port, _ask) = bridge();
        let clone = port.clone();
        assert!(port.take().is_some());
        assert!(port.take().is_none());
        assert!(clone.take().is_none());

        // Taking through the clone first leaves nothing for the original either.
        let (port, _ask) = bridge();
        let clone = port.clone();
        assert!(clone.take().is_some());
        assert!(port.take().is_none());
    }

    #[tokio::test]
    async fn an_ask_reaches_the_taken_receiver_and_returns_its_verdict() {
        let (port, ask) = bridge();
        let mut rx = port.take().expect("the first take yields the receiver");

        let answer = async {
            let request = rx.recv().await.expect("one request");
            assert_eq!(request.call, call("Bash"));
            request
                .answer
                .send(PromptVerdict::Allow)
                .expect("the asker waits");
        };
        let (verdict, ()) = tokio::join!(ask.ask(call("Bash")), answer);
        assert_eq!(verdict, Ok(PromptVerdict::Allow));

        let deny = PromptVerdict::Deny {
            message: "not on this box".to_owned(),
        };
        let answer = async {
            let request = rx.recv().await.expect("one request");
            assert_eq!(request.call, call("Write"));
            request.answer.send(deny.clone()).expect("the asker waits");
        };
        let (verdict, ()) = tokio::join!(ask.ask(call("Write")), answer);
        assert_eq!(verdict, Ok(deny));
    }

    #[tokio::test]
    async fn an_ask_after_the_port_is_dropped_is_closed() {
        // The port (and so the receiver) is gone before anyone asks.
        let (port, ask) = bridge();
        drop(port);
        assert_eq!(ask.ask(call("Bash")).await, Err(PromptClosed));

        // The receiver was taken, then dropped.
        let (port, ask) = bridge();
        drop(port.take());
        assert_eq!(ask.ask(call("Bash")).await, Err(PromptClosed));

        // The session received the request and dropped its answer unanswered.
        let (port, ask) = bridge();
        let mut rx = port.take().expect("the first take yields the receiver");
        let answer = async {
            let request = rx.recv().await.expect("one request");
            drop(request.answer);
        };
        let (verdict, ()) = tokio::join!(ask.ask(call("Bash")), answer);
        assert_eq!(verdict, Err(PromptClosed));
        assert_eq!(
            PromptClosed.to_string(),
            "the agent session ended before the permission was answered"
        );
    }

    #[tokio::test]
    async fn debug_prints_the_id_and_nothing_else() {
        let (port, ask) = bridge();
        let id = port.id();
        assert_eq!(format!("{port:?}"), format!("PromptPort({id})"));
        assert_eq!(format!("{ask:?}"), format!("PromptAsk({id})"));
    }
}
