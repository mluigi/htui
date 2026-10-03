//! `htui-mcp`: htui's MCP server (MOD-11) — the per-session tool host and the stdio relay's
//! channel.
//!
//! An agent session gets one MCP server named [`SERVER_NAME`]. The agent starts it as
//! `<htui binary> mcp` with two environment variables, [`ENV_ADDR`] and [`ENV_TOKEN`]; that child
//! is a dumb relay (plan D6, I-2) that connects to the listener of the htui process that opened the
//! session, proves itself with the token and the [`RELAY_VERSION`], and splices its stdio onto the
//! connection. The host process answers the MCP itself, with the session's scope, store and fence:
//! the agent never holds a DSN and never names a run (I-1).
//!
//! - [`protocol`]: JSON-RPC 2.0 / MCP over newline-delimited JSON, against a
//!   [`protocol::Handler`] (D2).
//! - [`channel`]: the listener, the handshake and the child's [`relay`] (D3).
//! - [`host`]: [`McpHost`], htui's `ToolHost` (D5), and its in-process [`McpClient`].
//! - [`search`]: the `search_concepts` seam, in this crate's own types (D12, blueprint B-3).
//! - `tools`: the eight tools, one file each, in one fixed table (blueprint §2.10).
#![warn(missing_docs)]

pub mod channel;
pub mod host;
pub mod protocol;
pub mod search;
mod tools;

pub use channel::{Address, Refusal, RelayError, Token, relay};
pub use host::{McpClient, McpHost};

/// The server name the agent sees: tools read `mcp__htui__<tool>`.
pub const SERVER_NAME: &str = "htui";

/// The environment variable naming the listener's address (a socket path or a pipe name).
pub const ENV_ADDR: &str = "HTUI_MCP_ADDR";

/// The environment variable carrying the session's token.
pub const ENV_TOKEN: &str = "HTUI_MCP_TOKEN";

/// The version the relay's handshake carries and the host compares whole (blueprint B-12): the
/// package version plus the relay's own revision, so a release bump or a relay change is refused
/// with a sentence rather than misread.
pub const RELAY_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+relay.1");
