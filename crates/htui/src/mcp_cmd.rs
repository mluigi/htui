//! `htui mcp` (MOD-11 D6): the stdio relay an agent starts as its htui MCP server.
//!
//! The agent htui launched runs `<htui binary> mcp` with [`ENV_ADDR`] and [`ENV_TOKEN`] in its
//! environment. This process is a dumb relay (I-2): it connects to the listener of the htui process
//! that opened the session, proves itself with the token, and splices its stdin and stdout onto
//! the connection until the agent closes stdin or the host ends the session. It never parses the
//! MCP it carries.
//!
//! Stdout is the protocol (blueprint H-24): nothing here initialises tracing, prints a banner or
//! logs. A failure is an [`McpExit`], which `main` prints on stderr and maps to the exit code; it is
//! never a crash report (B-13).

use std::ffi::OsString;

use htui_mcp::channel::RelayError;
use htui_mcp::{Address, ENV_ADDR, ENV_TOKEN, Token, relay};

/// How `htui mcp` ends (D6, blueprint B-13): `main` maps it to the exit code. A clean end (the
/// agent closed stdin, or the host ended the session) is `Ok`, exit 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpExit {
    /// Exit 2: a variable is unset or empty, or the token is not one htui mints.
    MissingEnv(String),
    /// Exit 3: the host refused the handshake; the sentence carries its reason.
    Refused(String),
    /// Exit 1: the host could not be reached, or the stream broke.
    Failed(String),
}

impl McpExit {
    /// 2 for a missing environment, 3 for a refusal, 1 for a failure.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::MissingEnv(_) => 2,
            Self::Refused(_) => 3,
            Self::Failed(_) => 1,
        }
    }
}

impl core::fmt::Display for McpExit {
    /// The sentence, and nothing else.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingEnv(sentence) | Self::Refused(sentence) | Self::Failed(sentence) => {
                f.write_str(sentence)
            }
        }
    }
}

impl std::error::Error for McpExit {}

/// Reads the two variables, then [`htui_mcp::channel::relay`]s this process's stdin and stdout.
///
/// # Errors
///
/// [`McpExit::MissingEnv`] (a variable unset, empty, or a token that is not 64 lowercase hex);
/// [`McpExit::Refused`] (the host's reason); [`McpExit::Failed`] (connect or i/o).
pub async fn run() -> Result<(), McpExit> {
    let (addr, token) = read_env(|name| std::env::var_os(name))?;
    relay(&addr, &token, tokio::io::stdin(), tokio::io::stdout())
        .await
        .map_err(|err| match err {
            RelayError::Refused(refusal) => {
                McpExit::Refused(format!("the htui host refused this relay: {refusal}"))
            }
            err @ (RelayError::Connect(..) | RelayError::Io(_)) => McpExit::Failed(err.to_string()),
        })
}

/// The address and the token from `var` (the process environment, or a test's map).
fn read_env(var: impl Fn(&str) -> Option<OsString>) -> Result<(Address, Token), McpExit> {
    let present = |name: &str| var(name).filter(|value| !value.is_empty());
    let missing = |name: &str| {
        McpExit::MissingEnv(format!(
            "{name} is not set: `htui mcp` is started by the agent htui launched, never by hand"
        ))
    };
    let addr = present(ENV_ADDR).ok_or_else(|| missing(ENV_ADDR))?;
    let token = present(ENV_TOKEN).ok_or_else(|| missing(ENV_TOKEN))?;
    let addr = addr
        .into_string()
        .map_err(|_| McpExit::MissingEnv(format!("{ENV_ADDR} is not valid UTF-8")))?;
    // The token's value is a secret (I-6): the sentence names the variable, never what it holds.
    let token = token.to_str().and_then(Token::parse).ok_or_else(|| {
        McpExit::MissingEnv(format!(
            "{ENV_TOKEN} is not an htui session token (64 lowercase hex characters)"
        ))
    })?;
    Ok((Address::new(addr), token))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;

    use super::{McpExit, read_env};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, OsString> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect()
    }

    fn read(pairs: &[(&str, &str)]) -> Result<(String, String), McpExit> {
        let env = env(pairs);
        read_env(|name| env.get(name).cloned())
            .map(|(addr, token)| (addr.as_str().to_owned(), token.as_str().to_owned()))
    }

    #[test]
    fn both_variables_are_read() {
        assert_eq!(
            read(&[("HTUI_MCP_ADDR", "/run/x/s"), ("HTUI_MCP_TOKEN", TOKEN)]),
            Ok(("/run/x/s".to_owned(), TOKEN.to_owned()))
        );
    }

    #[test]
    fn an_unset_or_empty_variable_is_missing_env() {
        for pairs in [
            &[("HTUI_MCP_TOKEN", TOKEN)][..],
            &[("HTUI_MCP_ADDR", ""), ("HTUI_MCP_TOKEN", TOKEN)][..],
            &[("HTUI_MCP_ADDR", "/run/x/s")][..],
            &[("HTUI_MCP_ADDR", "/run/x/s"), ("HTUI_MCP_TOKEN", "")][..],
        ] {
            let exit = read(pairs).expect_err("refused");
            assert!(
                matches!(exit, McpExit::MissingEnv(_)),
                "{pairs:?}: {exit:?}"
            );
            assert_eq!(exit.code(), 2);
            let sentence = exit.to_string();
            assert!(sentence.contains("is not set"), "{sentence}");
            assert!(sentence.contains("never by hand"), "{sentence}");
        }
    }

    /// A token that is not 64 lowercase hex is a missing token, and the sentence never echoes it.
    #[test]
    fn a_malformed_token_is_missing_env_and_never_printed() {
        let upper = TOKEN.to_uppercase();
        for bad in ["short-secret-value", upper.as_str(), &TOKEN[1..]] {
            let exit = read(&[("HTUI_MCP_ADDR", "/run/x/s"), ("HTUI_MCP_TOKEN", bad)])
                .expect_err("refused");
            assert!(matches!(exit, McpExit::MissingEnv(_)), "{exit:?}");
            assert!(!exit.to_string().contains(bad), "{exit}");
        }
    }

    #[test]
    fn display_is_the_sentence() {
        assert_eq!(McpExit::MissingEnv("m".into()).to_string(), "m");
        assert_eq!(McpExit::Refused("r".into()).to_string(), "r");
        assert_eq!(McpExit::Failed("f".into()).to_string(), "f");
    }
}
