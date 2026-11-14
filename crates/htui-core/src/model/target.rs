//! Execution targets (MOD-130 M1): where a run executes, named once and chosen per level.
//!
//! A row of `target` names a kind and a locator (names and addresses, never secrets). Its
//! status is derived at read time and never stored (D4).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::box_::BoxRow;
use crate::model::ids::{BoxId, TargetId};
use crate::store::traits::{has_nul, json_has_nul, target_locator_refused, target_name_refused};

str_enum!(
    /// `target.kind` (MOD-130 M1, D1): all five are stored; only [`RUNNABLE_KINDS`] run in this
    /// build.
    TargetKind {
        /// The box that starts the run (D2): the one built-in target.
        Local => "local",
        /// A container on this box (MOD-44).
        Docker => "docker",
        /// A remote machine over ssh (MOD-131).
        Ssh => "ssh",
        /// A container on a remote machine over ssh (MOD-131).
        SshDocker => "ssh_docker",
        /// A machine that enrolled itself (MOD-47).
        Enrolled => "enrolled",
    }
);

str_enum!(
    /// A target's status (MOD-130 M1, D4): derived at read time, never stored.
    TargetStatus {
        /// It can take a run now.
        Ready => "ready",
        /// It has no bound box yet, or was never probed.
        Unprepared => "unprepared",
        /// Its last probe failed, or its host lacks what the kind needs.
        Unreachable => "unreachable",
        /// Its kind does not run in this build.
        Planned => "planned",
    }
);

/// The kinds this build can run (MOD-130 M1, D1). MOD-44, MOD-131 and MOD-47 extend it.
pub const RUNNABLE_KINDS: &[TargetKind] = &[TargetKind::Local];

/// The `app_setting` key of the Settings default target (MOD-130 M1, D3): its value is a target
/// id; an absent row means the local target.
pub const DEFAULT_TARGET_KEY: &str = "default_target";

/// The longest target name, in chars (MOD-130 M1, P9).
const TARGET_NAME_MAX: usize = 64;

/// The fragments of a normalised locator key that read as a secret (MOD-130 M1, blueprint B6).
const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "privatekey",
    "credential",
];

impl TargetKind {
    /// Whether this build runs the kind (MOD-130 M1, D1): it is in [`RUNNABLE_KINDS`].
    #[must_use]
    pub fn runnable(self) -> bool {
        RUNNABLE_KINDS.contains(&self)
    }

    /// The handoff item that makes the kind runnable (MOD-130 M1, D1); `None` for `local`.
    #[must_use]
    pub const fn owner(self) -> Option<&'static str> {
        match self {
            Self::Local => None,
            Self::Docker => Some("MOD-44"),
            Self::Ssh | Self::SshDocker => Some("MOD-131"),
            Self::Enrolled => Some("MOD-47"),
        }
    }
}

/// A row of `target` (MOD-130 M1, ANA-42 §7.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    /// `target.id`.
    pub id: TargetId,
    /// `target.name`: unique, 1-64 characters, stored as given.
    pub name: String,
    /// `target.kind`.
    pub kind: TargetKind,
    /// `target.locator` (`JSONB` object): names and addresses only, never values.
    pub locator: Value,
    /// `target.box_id`: the box the target runs on once bound; always `None` on `local` (D2).
    pub box_id: Option<BoxId>,
    /// `target.probed_at`: the last probe of the bound box through this target.
    pub probed_at: Option<DateTime<Utc>>,
    /// `target.probe_error`: why the last probe failed; `None` after a good one.
    pub probe_error: Option<String>,
    /// `target.retired_at`: a retired target is hidden from pickers and keeps its history (D14).
    pub retired_at: Option<DateTime<Utc>>,
    /// `target.created_at`.
    pub created_at: DateTime<Utc>,
    /// `target.updated_at`: the compare-and-set token of the target's editors (P6).
    pub updated_at: DateTime<Utc>,
}

impl Target {
    /// The status D4 derives with this build's [`RUNNABLE_KINDS`] (MOD-130 M1). `bound` is the
    /// row of `box_id`; `host` is the machine `bound` runs on (its parent, for a container).
    #[must_use]
    pub fn status(&self, bound: Option<&BoxRow>, host: Option<&BoxRow>) -> TargetStatus {
        self.status_with(RUNNABLE_KINDS, bound, host)
    }

    /// [`Target::status`] with an explicit runnable set (MOD-130 M1, D4), first match wins:
    /// not runnable is `Planned`; `local` is `Ready`; no bound box is `Unprepared`; a probe
    /// error is `Unreachable`; never probed is `Unprepared`; a `docker` or `ssh_docker` target
    /// whose `host` lacks the `docker` tag (probed or declared) is `Unreachable`; else `Ready`.
    #[must_use]
    pub fn status_with(
        &self,
        runnable: &[TargetKind],
        bound: Option<&BoxRow>,
        host: Option<&BoxRow>,
    ) -> TargetStatus {
        if !runnable.contains(&self.kind) {
            return TargetStatus::Planned;
        }
        if self.kind == TargetKind::Local {
            return TargetStatus::Ready;
        }
        if bound.is_none() {
            return TargetStatus::Unprepared;
        }
        if self.probe_error.is_some() {
            return TargetStatus::Unreachable;
        }
        if self.probed_at.is_none() {
            return TargetStatus::Unprepared;
        }
        if matches!(self.kind, TargetKind::Docker | TargetKind::SshDocker) {
            let has_docker = host.is_some_and(|host| {
                host.probed_tags
                    .iter()
                    .chain(&host.declared_tags)
                    .any(|tag| tag == "docker")
            });
            if !has_docker {
                return TargetStatus::Unreachable;
            }
        }
        TargetStatus::Ready
    }
}

/// Why `name` cannot be a target name (MOD-130 M1, P9, blueprint B4), or `None`: 1-64 chars, no
/// control character, and no leading or trailing whitespace. The name is stored as given.
#[must_use]
pub fn target_name_refusal(name: &str) -> Option<String> {
    let chars = name.chars().count();
    let refused = chars == 0
        || chars > TARGET_NAME_MAX
        || name.chars().any(char::is_control)
        || name != name.trim();
    refused.then(|| target_name_refused(name))
}

/// Why `locator` cannot be stored (MOD-130 M1, blueprint B4, B6), or `None`. First match wins:
/// a NUL anywhere; not an object; a key at any depth whose normalisation (ASCII-lowercased, with
/// `_`, `-` and `.` removed) contains `password`, `passwd`, `secret`, `token`, `apikey`,
/// `privatekey` or `credential`; a string value holding a URL with a password in its userinfo.
#[must_use]
pub fn locator_refusal(locator: &Value) -> Option<String> {
    if json_has_nul(locator) {
        return Some(has_nul("target.locator"));
    }
    if !locator.is_object() {
        return Some(target_locator_refused("it is not a JSON object"));
    }
    secret_in(locator, "").map(|why| target_locator_refused(&why))
}

/// The first secret-looking key or value under `value`, as a sentence naming its path.
fn secret_in(value: &Value, path: &str) -> Option<String> {
    let join = |key: &str| {
        if path.is_empty() {
            key.to_owned()
        } else {
            format!("{path}.{key}")
        }
    };
    match value {
        Value::Object(map) => map.iter().find_map(|(key, item)| {
            let at = join(key);
            if key_looks_secret(key) {
                return Some(format!("key `{at}` looks like a secret"));
            }
            secret_in(item, &at)
        }),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| secret_in(item, &join(&index.to_string()))),
        Value::String(text) if url_has_password(text) => {
            Some(format!("the value at `{path}` holds a password in a URL"))
        }
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

/// Blueprint B6's key rule.
fn key_looks_secret(key: &str) -> bool {
    let normal: String = key
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    SECRET_KEY_FRAGMENTS
        .iter()
        .any(|fragment| normal.contains(fragment))
}

/// Whether any `scheme://user:pass@host` in `text` carries a password in its userinfo.
fn url_has_password(text: &str) -> bool {
    text.match_indices("://").any(|(at, sep)| {
        let rest = &text[at + sep.len()..];
        let authority = rest
            .split(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .next()
            .unwrap_or_default();
        authority
            .rsplit_once('@')
            .is_some_and(|(userinfo, _)| userinfo.contains(':'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::box_::{BoxKind, OsFamily};
    use crate::model::ids::UserId;
    use serde_json::json;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    fn target(kind: TargetKind) -> Target {
        Target {
            id: TargetId::new(),
            name: format!("a {kind}"),
            kind,
            locator: json!({}),
            box_id: None,
            probed_at: None,
            probe_error: None,
            retired_at: None,
            created_at: at(),
            updated_at: at(),
        }
    }

    fn a_box(tags: &[&str], declared: &[&str]) -> BoxRow {
        BoxRow {
            id: BoxId::new(),
            user_id: UserId::new(),
            hostname: "dev".to_owned(),
            os_family: Some(OsFamily::Linux),
            os_version: String::new(),
            arch: String::new(),
            cpu: String::new(),
            ram_mb: None,
            gpu_present: false,
            gpu_vendor: None,
            htui_version: String::new(),
            probed_tags: tags.iter().map(|&t| t.to_owned()).collect(),
            declared_tags: declared.iter().map(|&t| t.to_owned()).collect(),
            quirks: String::new(),
            settings: json!({}),
            registered_at: at(),
            last_seen_at: at(),
            last_probed_at: None,
            updated_at: at(),
            edit_version: 0,
            kind: BoxKind::Machine,
            parent_box_id: None,
        }
    }

    /// D1: in this build only `local` runs, so every other kind reads `planned` whatever its box.
    #[test]
    fn every_kind_but_local_is_planned_in_this_build() {
        assert_eq!(RUNNABLE_KINDS, &[TargetKind::Local]);
        let host = a_box(&["docker"], &[]);
        for &kind in TargetKind::ALL {
            let mut row = target(kind);
            row.probed_at = Some(at());
            let want = if kind == TargetKind::Local {
                TargetStatus::Ready
            } else {
                TargetStatus::Planned
            };
            assert_eq!(row.status(None, None), want, "{kind}");
            assert_eq!(row.status(Some(&host), Some(&host)), want, "{kind}");
            assert_eq!(kind.runnable(), kind == TargetKind::Local, "{kind}");
        }
        assert_eq!(TargetKind::Local.owner(), None);
        assert_eq!(TargetKind::Docker.owner(), Some("MOD-44"));
        assert_eq!(TargetKind::Ssh.owner(), Some("MOD-131"));
        assert_eq!(TargetKind::SshDocker.owner(), Some("MOD-131"));
        assert_eq!(TargetKind::Enrolled.owner(), Some("MOD-47"));
    }

    /// D4's rules 1-7 in order, each one shadowing the rules below it.
    #[test]
    fn status_follows_the_d4_order() {
        let all = TargetKind::ALL;
        let bound = a_box(&[], &[]);
        let docker_host = a_box(&["docker"], &[]);
        let declared_host = a_box(&[], &["docker"]);
        let bare_host = a_box(&["linux"], &["gaming"]);

        // 1. Not runnable → planned, before anything else is read.
        let mut docker = target(TargetKind::Docker);
        docker.probed_at = Some(at());
        assert_eq!(
            docker.status_with(&[TargetKind::Local], Some(&bound), Some(&docker_host)),
            TargetStatus::Planned
        );
        // 2. Local → ready, whatever it carries.
        let mut local = target(TargetKind::Local);
        local.probe_error = Some("ignored".to_owned());
        assert_eq!(local.status_with(all, None, None), TargetStatus::Ready);
        // 3. No bound box → unprepared, even with a probe error.
        let mut unbound = target(TargetKind::Ssh);
        unbound.probe_error = Some("refused".to_owned());
        assert_eq!(
            unbound.status_with(all, None, None),
            TargetStatus::Unprepared
        );
        // 4. A probe error → unreachable, even when never probed.
        let mut failed = target(TargetKind::Ssh);
        failed.probe_error = Some("connection refused".to_owned());
        assert_eq!(
            failed.status_with(all, Some(&bound), Some(&bound)),
            TargetStatus::Unreachable
        );
        // 5. Never probed → unprepared.
        let fresh = target(TargetKind::Docker);
        assert_eq!(
            fresh.status_with(all, Some(&bound), Some(&docker_host)),
            TargetStatus::Unprepared
        );
        // 6. A docker or ssh_docker target whose host lacks the docker tag → unreachable.
        for kind in [TargetKind::Docker, TargetKind::SshDocker] {
            let mut row = target(kind);
            row.probed_at = Some(at());
            assert_eq!(
                row.status_with(all, Some(&bound), Some(&bare_host)),
                TargetStatus::Unreachable,
                "{kind}"
            );
            assert_eq!(
                row.status_with(all, Some(&bound), None),
                TargetStatus::Unreachable,
                "{kind} without a host"
            );
            // 7. With the tag, probed or declared → ready.
            assert_eq!(
                row.status_with(all, Some(&bound), Some(&docker_host)),
                TargetStatus::Ready,
                "{kind}"
            );
            assert_eq!(
                row.status_with(all, Some(&bound), Some(&declared_host)),
                TargetStatus::Ready,
                "{kind}"
            );
        }
        // 7. An ssh or enrolled target needs no docker tag.
        for kind in [TargetKind::Ssh, TargetKind::Enrolled] {
            let mut row = target(kind);
            row.probed_at = Some(at());
            assert_eq!(
                row.status_with(all, Some(&bound), Some(&bare_host)),
                TargetStatus::Ready,
                "{kind}"
            );
        }
    }

    /// Blueprint B6: any depth, normalised keys that contain a secret word, and a URL password.
    #[test]
    fn locator_refusal_names_a_secret_key_at_any_depth() {
        for (locator, path) in [
            (json!({"password": "x"}), "password"),
            (json!({"Token": "x"}), "Token"),
            (json!({"docker": {"API_KEY": "x"}}), "docker.API_KEY"),
            (
                json!({"docker": {"client_secret": "x"}}),
                "docker.client_secret",
            ),
            (json!({"hosts": [{"passwd": "x"}]}), "hosts.0.passwd"),
            (json!({"access_token": "x"}), "access_token"),
            (json!({"clientSecret": "x"}), "clientSecret"),
            (
                json!({"AWS_SECRET_ACCESS_KEY": "x"}),
                "AWS_SECRET_ACCESS_KEY",
            ),
            (json!({"private-key": "x"}), "private-key"),
            (json!({"privateKey": "x"}), "privateKey"),
            (json!({"ssh_password": "x"}), "ssh_password"),
            (json!({"credentials": {}}), "credentials"),
        ] {
            assert_eq!(
                locator_refusal(&locator),
                Some(target_locator_refused(&format!(
                    "key `{path}` looks like a secret"
                ))),
                "{locator}"
            );
        }
        assert_eq!(
            locator_refusal(&json!({"docker": {"host": "tcp://u:p@h:2375"}})),
            Some(target_locator_refused(
                "the value at `docker.host` holds a password in a URL"
            ))
        );
        assert_eq!(
            locator_refusal(&json!([])),
            Some(target_locator_refused("it is not a JSON object"))
        );
        assert_eq!(
            locator_refusal(&json!("dev")),
            Some(target_locator_refused("it is not a JSON object"))
        );
        assert_eq!(
            locator_refusal(&json!({"host": "de\u{0}v"})),
            Some(has_nul("target.locator"))
        );
        assert_eq!(
            locator_refusal(&json!({"ho\u{0}st": "dev"})),
            Some(has_nul("target.locator"))
        );
        for allowed in [
            json!({}),
            json!({"host": "dev", "user": "me", "context": "default"}),
            json!({"host": "ssh://me@dev:22", "docker": {"host": "tcp://[::1]:2375"}}),
            json!({"hosts": ["dev", "build"], "port": 22, "tls": true}),
        ] {
            assert_eq!(locator_refusal(&allowed), None, "{allowed}");
        }
    }

    /// Blueprint B4: a name is stored as given, so space around it is refused, not trimmed.
    #[test]
    fn a_name_is_trimmed_and_bounded() {
        for good in [
            "this box",
            "a",
            "docker: rust 1.95",
            &"x".repeat(64),
            &"é".repeat(64),
        ] {
            assert_eq!(target_name_refusal(good), None, "{good:?}");
        }
        for bad in [
            "",
            " ",
            " dev",
            "dev ",
            "\tdev",
            "de\nv",
            "de\u{0}v",
            "de\u{7f}v",
            &"x".repeat(65),
        ] {
            assert_eq!(
                target_name_refusal(bad),
                Some(target_name_refused(bad)),
                "{bad:?}"
            );
        }
    }

    /// `chk_target_kind`'s list in `0037_execution_targets.sql`, in order; the status texts are
    /// pinned too, as the UI and the cache read them.
    #[test]
    fn kind_and_status_match_the_check_list() {
        let kinds: Vec<_> = TargetKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, ["local", "docker", "ssh", "ssh_docker", "enrolled"]);
        let statuses: Vec<_> = TargetStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(statuses, ["ready", "unprepared", "unreachable", "planned"]);
        for &kind in TargetKind::ALL {
            assert_eq!(kind.as_str().parse::<TargetKind>(), Ok(kind));
            assert_eq!(
                serde_json::to_value(kind).expect("a kind serialises"),
                json!(kind.as_str())
            );
        }
        for &status in TargetStatus::ALL {
            assert_eq!(status.as_str().parse::<TargetStatus>(), Ok(status));
        }
    }
}
