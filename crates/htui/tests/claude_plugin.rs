//! MOD-93 D10: the plugin `htui claude emit` writes, checked by the Claude Code on this box. When
//! `claude` at or above [`CLAUDE_FLOOR`] answers `--version`, both variants are emitted into a
//! temp dir and `claude plugin validate --strict` runs on each marketplace and plugin (and on the
//! source tree), the mod's reach is read off validate's `calls:` line (ANA-29 §6.3), and `claude
//! plugin test` runs the kit tests in the source tree. Otherwise one `skip:` line is printed and
//! the test passes. Every `claude` runs with `CLAUDE_CONFIG_DIR` set to a temp dir, never the
//! shared `~/.claude`, and nothing is written outside temp dirs.
//!
//! The skill's frontmatter is checked with no `claude` at all: `validate` never reads it.

use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use htui::claude_emit::{CLAUDE_FLOOR, ClaudeCheck, Options, Variant, probe_claude, write_to};
use serde_json::Value;

/// The plugin source tree the binary embeds.
const SOURCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/claude-plugin");

/// How long one `claude` command may take before the gate fails.
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(120);

/// The only `$` calls the mod may make: `$.process`, `$.clock`, `$.env.get`, `$.command`,
/// `$.state` and `$.ui` (plan T2), as validate names them.
const ALLOWED_CALLS: [&str; 10] = [
    "$.clock.every",
    "$.clock.now",
    "$.command.register",
    "$.env.get",
    "$.process.run",
    "$.state.get",
    "$.state.set",
    "$.ui.open",
    "$.ui.panes",
    "$.ui.resolve",
];

/// Emits `variant` into `dir` (absent), as `htui claude emit --dir` would.
fn emit(dir: &Path, variant: Variant) {
    write_to(
        dir,
        &Options {
            force: false,
            variant,
        },
    )
    .unwrap_or_else(|exit| panic!("emitting into {}: {exit}", dir.display()));
}

/// What one `claude` command ended with.
#[derive(Debug)]
struct Ran {
    /// How it exited.
    status: ExitStatus,
    /// Its stdout.
    stdout: String,
    /// Its stderr, for the failure message.
    stderr: String,
}

/// Runs `claude <args>` in `cwd` with a throwaway config dir. A timeout fails the gate.
async fn claude(args: &[&str], config: &Path, cwd: &Path) -> Ran {
    let child = tokio::process::Command::new("claude")
        .args(args)
        .env("CLAUDE_CONFIG_DIR", config)
        .env_remove("CLAUDE_CODE_PLUGIN_DIRS")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(CLAUDE_TIMEOUT, child)
        .await
        .unwrap_or_else(|_| panic!("`claude {}` timed out", args.join(" ")))
        .unwrap_or_else(|error| panic!("`claude {}` did not start: {error}", args.join(" ")));
    Ran {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// `claude plugin validate --strict --json <path>`, which must pass; its document.
async fn validate(path: &Path, config: &Path, cwd: &Path) -> Value {
    let path_text = path.to_str().expect("a UTF-8 temp path");
    let ran = claude(
        &["plugin", "validate", "--strict", "--json", path_text],
        config,
        cwd,
    )
    .await;
    assert!(
        ran.status.success(),
        "validate {path_text} exited {:?}\nstdout: {}\nstderr: {}",
        ran.status,
        ran.stdout,
        ran.stderr
    );
    let document: Value = serde_json::from_str(ran.stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "validate {path_text} printed no JSON ({error})\nstdout: {}",
            ran.stdout
        )
    });
    assert_eq!(
        document["success"],
        Value::Bool(true),
        "validate {path_text}: {document:#}"
    );
    document
}

/// The note of `document`'s `contents[]` that starts with `prefix`, without it.
fn note<'a>(document: &'a Value, prefix: &str) -> &'a str {
    document["contents"]
        .as_array()
        .expect("contents is an array")
        .iter()
        .flat_map(|content| content["notes"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .find_map(|note| note.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no note starts with {prefix:?}: {document:#}"))
}

/// The `$` names of a `calls:` note, each `(via …)` annotation dropped.
fn call_names(calls: &str) -> Vec<String> {
    let mut bare = String::new();
    let mut depth = 0_u32;
    for character in calls.chars() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => bare.push(character),
            _ => {}
        }
    }
    bare.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// The `key: value` lines of `text`'s leading `---` block, each value unquoted.
fn frontmatter(text: &str) -> Vec<(String, String)> {
    let body = text
        .strip_prefix("---\n")
        .expect("SKILL.md opens with a --- line");
    let (block, _) = body
        .split_once("\n---\n")
        .expect("SKILL.md's frontmatter closes with a --- line");
    block
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(value);
            (key.trim().to_owned(), value.to_owned())
        })
        .collect()
}

/// T1: `claude plugin validate` never reads the skill's frontmatter, so it is checked here, in
/// both emitted variants: `name: htui` and a description.
#[test]
fn the_emitted_skill_names_itself_htui_and_describes_itself() {
    let out = tempfile::tempdir().expect("a temp dir");
    for (folder, variant) in [("full", Variant::WithMod), ("skill", Variant::SkillOnly)] {
        let dir = out.path().join(folder);
        emit(&dir, variant);
        let skill = std::fs::read_to_string(dir.join("htui/skills/htui/SKILL.md"))
            .expect("the skill is written");
        let fields = frontmatter(&skill);
        let field = |key: &str| {
            fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(field("name"), Some("htui"), "{folder}: {fields:?}");
        assert!(
            field("description").is_some_and(|text| !text.trim().is_empty()),
            "{folder}: {fields:?}"
        );
    }
}

/// D9, D10, ANA-29 §6.3: what users get validates, the mod reaches only what the plan allows, and
/// the kit tests pass.
#[tokio::test]
async fn the_emitted_plugin_validates_and_the_mod_tests_pass_under_claude() {
    match probe_claude().await {
        ClaudeCheck::Unchecked => {
            eprintln!(
                "skip: no claude on PATH answered --version; MOD-93's plugin gate did not run"
            );
            return;
        }
        ClaudeCheck::Below(version) => {
            eprintln!(
                "skip: claude {version} is below {CLAUDE_FLOOR}; MOD-93's plugin gate did not run"
            );
            return;
        }
        ClaudeCheck::Met(version) => eprintln!("claude {version}: running MOD-93's plugin gate"),
    }

    let config = tempfile::tempdir().expect("a temp config dir");
    let out = tempfile::tempdir().expect("a temp out dir");
    let full = out.path().join("full");
    let skill = out.path().join("skill");
    emit(&full, Variant::WithMod);
    emit(&skill, Variant::SkillOnly);
    assert!(
        !full.join("htui/tests").exists(),
        "D9: no kit tests emitted"
    );

    let source = PathBuf::from(SOURCE);
    let mut checked = Vec::new();
    for path in [
        source.clone(),
        source.join("htui"),
        full.clone(),
        full.join("htui"),
        skill.clone(),
        skill.join("htui"),
    ] {
        checked.push((
            path.clone(),
            validate(&path, config.path(), out.path()).await,
        ));
    }

    let (_, plugin) = checked
        .iter()
        .find(|(path, _)| *path == full.join("htui"))
        .expect("the full plugin was validated");
    for content in plugin["contents"].as_array().expect("contents is an array") {
        assert_eq!(
            content["gatingHooks"],
            Value::Array(Vec::new()),
            "no gating hook: {content:#}"
        );
    }
    let calls = call_names(note(plugin, "./register.tsx calls: "));
    assert!(!calls.is_empty(), "the mod calls something: {plugin:#}");
    for call in &calls {
        assert!(
            ALLOWED_CALLS.contains(&call.as_str()),
            "the mod calls {call}, outside its reach: {calls:?}"
        );
    }
    assert_eq!(note(plugin, "./register.tsx env reads: "), "HTUI_SESSION");
    assert_eq!(note(plugin, "./register.tsx env writes: "), "nothing");

    let tested = claude(
        &[
            "plugin",
            "test",
            source.join("htui").to_str().expect("a UTF-8 source path"),
        ],
        config.path(),
        out.path(),
    )
    .await;
    assert!(
        tested.status.success(),
        "claude plugin test exited {:?}\nstdout: {}\nstderr: {}",
        tested.status,
        tested.stdout,
        tested.stderr
    );
    eprintln!("{}", tested.stdout.trim_end());
}
