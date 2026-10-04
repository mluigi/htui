//! `InfisicalProvider` against a loopback stand-in for Infisical (MOD-10 M2, blueprint §D.3).
//! Never a real server: every base URL is the stub's or a closed loopback port.
//!
//! Sentinel rule (H-8): no assert message interpolates `CLIENT_SECRET`, `TOKEN`, `TOKEN_2`,
//! `VALUE` or an error that might carry one. Messages name the case only.

use htui_core::secret::{MachineIdentity, SecretError};
use htui_secrets::{InfisicalConfig, InfisicalProvider};

const CLIENT_ID: &str = "cid-sentinel-1";
const CLIENT_SECRET: &str = "csecret-sentinel-1";

fn identity() -> MachineIdentity {
    MachineIdentity::new(CLIENT_ID, CLIENT_SECRET)
}

// ---------------------------------------------------------------------------------------------
// Build and config
// ---------------------------------------------------------------------------------------------

/// Meaningful only under `cargo test -p htui-secrets`, whose dependency graph has no
/// `sentry`/aws-lc (H-1): there, `Client::build` panics unless this crate installed `ring`
/// first. A workspace-wide build gets a provider from elsewhere and proves nothing.
#[test]
fn a_provider_builds_with_only_this_crate_installing_ring() {
    for _ in 0..2 {
        InfisicalProvider::new(
            InfisicalConfig::new("https://app.infisical.com"),
            identity(),
        )
        .expect("the provider builds with ring installed by htui-secrets");
    }
}

#[test]
fn an_http_base_url_on_a_lan_host_is_refused_before_any_request() {
    match InfisicalProvider::new(InfisicalConfig::new("http://192.168.1.10"), identity()) {
        Err(SecretError::Config(why)) => assert!(why.contains("https"), "{why}"),
        Err(other) => panic!("expected Config, got {other}"),
        Ok(_) => panic!("a plain-text LAN URL was accepted"),
    }
}

#[test]
fn a_blank_identity_half_is_refused_at_build() {
    let cases = [
        (
            MachineIdentity::new("  ", CLIENT_SECRET),
            "the machine identity's client ID is empty",
        ),
        (
            MachineIdentity::new(CLIENT_ID, "  "),
            "the machine identity's client secret is empty",
        ),
    ];
    for (id, sentence) in cases {
        match InfisicalProvider::new(InfisicalConfig::new("https://app.infisical.com"), id) {
            Err(SecretError::Config(why)) => assert_eq!(why, sentence),
            Err(_) => panic!("expected Config for {sentence:?}"),
            Ok(_) => panic!("a blank half was accepted ({sentence})"),
        }
    }
}
