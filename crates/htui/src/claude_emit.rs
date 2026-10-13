//! `htui claude emit` (MOD-93, `R-ID-4`, `R-ID-6`, `R-STO-1`). Writes htui's Claude Code plugin
//! as a directory marketplace: `.claude-plugin/marketplace.json` and the plugin `htui/` (the skill,
//! and the mod when `claude --version` is at least [`CLAUDE_FLOOR`] or does not answer), every
//! file embedded in this binary from `crates/htui/claude-plugin/`. The plugin's version is the
//! binary's ([`VERSION`]). Then it prints the two `claude plugin` commands that install it; it runs
//! neither, and never touches `~/.claude`.
//!
//! The target is written only when it is absent or empty, or, with `--force`, when it holds an
//! earlier emit: nothing but `.claude-plugin` and `htui`, and a marketplace named `htui` (D4).
//! Anything else is refused, `--force` or not. Nothing outside the target and its two siblings
//! `<dir>.emit-<pid>` (staging) and `<dir>.old-<pid>` (the earlier emit, moved aside) is written,
//! but the target's missing parents. Exits are 2 (refused) and 1 (failed), and none reaches Sentry:
//! the sentences carry home paths.

use std::ffi::OsStr;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::cli;

/// The version every emitted plugin carries: this binary's (D5).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The Claude Code version mods need (ANA-29 §2.1); below it the skill alone is written (D6).
pub const CLAUDE_FLOOR: &str = "2.1.287";
/// `claude --version` prints `2.1.296 (Claude Code)`.
pub const CLAUDE_VERSION_PATTERN: &str = r"^(\d+\.\d+\.\d+)";

/// `hooks/version.ts`'s placeholder, quotes included; the source holds it exactly once. The source
/// tree's `plugin.json` carries the same version, which the stamp overwrites without reading it.
const VERSION_TS_LITERAL: &str = "'0.0.0-source'";

/// The plugin manifest, rewritten with the version (and, skill-only, without the mod's keys).
const PLUGIN_JSON: &str = "htui/.claude-plugin/plugin.json";
/// The mod's version module, stamped by text.
const VERSION_TS: &str = "htui/hooks/version.ts";

/// How `htui claude emit` ends when it does not write; `main` maps it to the exit code and prints
/// it on stderr. Neither arm reaches Sentry: its sentences carry home paths (MOD-93 D8).
#[derive(Debug)]
pub enum EmitExit {
    /// Exit 2: the target is not one `emit` may write (foreign, a file, ours without `--force`),
    /// or this box has no default directory.
    Refused(String),
    /// Exit 1: a write, a rename or stdout failed.
    Failed(String),
}

impl EmitExit {
    /// 2 refused, 1 failed.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Refused(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl core::fmt::Display for EmitExit {
    /// The sentence, and nothing else.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(sentence) | Self::Failed(sentence) => f.write_str(sentence),
        }
    }
}

impl std::error::Error for EmitExit {}

/// Which plugin is written (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// The skill and the mod: `hooks/`, `types/`, and `plugin.json` with `types` and `userConfig`.
    WithMod,
    /// The skill alone, for a `claude` below [`CLAUDE_FLOOR`]: no `hooks/`, no `types/`, and
    /// `plugin.json` without `types` and `userConfig`.
    SkillOnly,
}

/// What `claude --version` said, against [`CLAUDE_FLOOR`] (D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeCheck {
    /// At or above the floor: the mod is written.
    Met(String),
    /// Below the floor: the skill alone is written.
    Below(String),
    /// `claude` is not on `PATH`, printed no version, or timed out: the mod is written.
    Unchecked,
}

impl ClaudeCheck {
    /// The check for a found version: [`Below`](Self::Below) when it is below [`CLAUDE_FLOOR`]
    /// by semver, [`Met`](Self::Met) otherwise (an unparsable one included), and
    /// [`Unchecked`](Self::Unchecked) for none.
    #[must_use]
    pub fn from_version(found: Option<String>) -> Self {
        match found {
            Some(version) if htui_agent::probe::below_min(&version, CLAUDE_FLOOR) => {
                Self::Below(version)
            }
            Some(version) => Self::Met(version),
            None => Self::Unchecked,
        }
    }

    /// The skill alone below the floor; the mod too otherwise.
    #[must_use]
    pub const fn variant(&self) -> Variant {
        match self {
            Self::Below(_) => Variant::SkillOnly,
            Self::Met(_) | Self::Unchecked => Variant::WithMod,
        }
    }
}

/// What [`write_to`] is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Replace a directory an earlier `emit` wrote (D4); never a foreign one.
    pub force: bool,
    /// Which plugin is written.
    pub variant: Variant,
}

/// What [`write_to`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The directory written, absolute: what the printed command names.
    pub dir: PathBuf,
    /// Which plugin was written.
    pub variant: Variant,
    /// Whether an earlier emit was replaced (`--force`).
    pub replaced: bool,
    /// The earlier emit, moved aside, when removing it failed after the new one was in place.
    pub leftover: Option<PathBuf>,
}

/// Whether a source file is written with every variant or only with the mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// Every variant.
    Always,
    /// [`Variant::WithMod`] only.
    Mod,
}

/// One file of the plugin source tree, embedded in the binary.
#[derive(Debug, Clone, Copy)]
struct Embedded {
    /// Relative to `crates/htui/claude-plugin/`, `/`-separated.
    path: &'static str,
    /// The source text.
    body: &'static str,
    /// Which variants carry it.
    part: Part,
}

/// One table row: the path is written once, and `include_str!` reads that same path.
macro_rules! embedded {
    ($path:literal, $part:expr) => {
        Embedded {
            path: $path,
            body: include_str!(concat!("../claude-plugin/", $path)),
            part: $part,
        }
    };
}

/// Every file of `crates/htui/claude-plugin/` but `htui/tests/` (D9) and the engine's
/// `htui/.claude-plugin/types/` (A-10): D1's explicit table. A unit test walks the source tree
/// against it.
const FILES: [Embedded; 7] = [
    embedded!(".claude-plugin/marketplace.json", Part::Always),
    embedded!("htui/.claude-plugin/plugin.json", Part::Always),
    embedded!("htui/skills/htui/SKILL.md", Part::Always),
    embedded!("htui/hooks/hooks.json", Part::Mod),
    embedded!("htui/hooks/register.tsx", Part::Mod),
    embedded!("htui/hooks/version.ts", Part::Mod),
    embedded!("htui/types/index.d.ts", Part::Mod),
];

/// The files `variant` writes, stamped: relative `/`-paths and their text.
///
/// # Errors
///
/// [`EmitExit::Failed`] when the embedded `plugin.json` is not a JSON object.
fn files_for(variant: Variant) -> Result<Vec<(&'static str, String)>, EmitExit> {
    FILES
        .iter()
        .filter(|file| file.part == Part::Always || variant == Variant::WithMod)
        .map(|file| {
            let body = match file.path {
                PLUGIN_JSON => stamp_manifest(file.body, variant)?,
                // Code, not data: the one literal is replaced by text (the source test pins it).
                VERSION_TS => file
                    .body
                    .replacen(VERSION_TS_LITERAL, &format!("'{VERSION}'"), 1),
                _ => file.body.to_owned(),
            };
            Ok((file.path, body))
        })
        .collect()
}

/// What the target holds before `emit` writes (D4). Read with `symlink_metadata`: a link is never
/// followed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// Nothing there.
    Absent,
    /// A directory with no entries.
    Empty,
    /// An earlier emit: only `.claude-plugin` and `htui`, and a marketplace named `htui`.
    Ours,
    /// Anything else that is a directory (or a link); why, for the refusal.
    Foreign(String),
    /// A file or some other non-directory.
    NotADirectory,
}

/// The entries an earlier emit leaves in its directory.
const OUR_ENTRIES: [&str; 2] = [".claude-plugin", "htui"];

/// What `dir` holds.
fn classify(dir: &Path) -> Result<Target, EmitExit> {
    let unreadable =
        |error: std::io::Error| EmitExit::Failed(format!("cannot read {}: {error}", dir.display()));
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Target::Absent),
        Err(error) => return Err(unreadable(error)),
    };
    if metadata.file_type().is_symlink() {
        return Ok(Target::Foreign("a symbolic link".to_owned()));
    }
    if !metadata.is_dir() {
        return Ok(Target::NotADirectory);
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        entries.push((entry.file_name(), entry.file_type().map_err(unreadable)?));
    }
    if entries.is_empty() {
        return Ok(Target::Empty);
    }
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    if let Some((name, _)) = entries
        .iter()
        .find(|(name, _)| !OUR_ENTRIES.iter().any(|ours| name == OsStr::new(ours)))
    {
        return Ok(Target::Foreign(name.to_string_lossy().into_owned()));
    }
    if entries.iter().any(|(_, kind)| kind.is_symlink()) {
        return Ok(Target::Foreign("a symbolic link".to_owned()));
    }
    let Ok(text) = fs::read_to_string(dir.join(".claude-plugin").join("marketplace.json")) else {
        return Ok(Target::Foreign("no marketplace.json".to_owned()));
    };
    let named_htui = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|marketplace| marketplace.get("name")?.as_str().map(|name| name == "htui"))
        .unwrap_or(false);
    if named_htui {
        Ok(Target::Ours)
    } else {
        Ok(Target::Foreign("marketplace.json is not htui's".to_owned()))
    }
}

/// How the staged tree goes into place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Install {
    /// `rename` into a path that does not exist.
    Create,
    /// Remove the empty directory, then `rename`.
    FillEmpty,
    /// Move the earlier emit aside, `rename`, then remove the earlier emit.
    Replace,
}

/// `<parent>/<name>.<kind>-<pid>`: the staging (`emit`) and moved-aside (`old`) siblings.
fn sibling(parent: &Path, name: &OsStr, kind: &str) -> PathBuf {
    let mut sibling = name.to_os_string();
    sibling.push(format!(".{kind}-{}", std::process::id()));
    parent.join(sibling)
}

/// Writes `files` under `root`, each `/`-path split and joined for this platform.
fn fill(root: &Path, files: &[(&str, String)]) -> Result<(), EmitExit> {
    for (path, body) in files {
        let file = path
            .split('/')
            .fold(root.to_path_buf(), |dir, part| dir.join(part));
        let written = match file.parent() {
            Some(parent) => fs::create_dir_all(parent),
            None => Ok(()),
        }
        .and_then(|()| fs::write(&file, body));
        written.map_err(|error| {
            EmitExit::Failed(format!("cannot write {}: {error}", file.display()))
        })?;
    }
    Ok(())
}

/// Writes the plugin into `dir`: stages it in a sibling `<dir>.emit-<pid>`, then renames it into
/// place (D4). With `force`, an earlier emit is moved aside to `<dir>.old-<pid>` first, and
/// removed once the new tree is in place. Nothing outside `dir`, those two siblings and `dir`'s
/// missing parents is written.
///
/// # Errors
///
/// [`EmitExit::Refused`] for a target `emit` did not write (even with `force`), a file, a path
/// naming no directory, or an earlier emit without `force`; nothing is touched then.
/// [`EmitExit::Failed`] when a directory cannot be read, made or renamed, or a file cannot be
/// written; the staging directory is removed, and a moved-aside emit is put back.
pub fn write_to(dir: &Path, options: &Options) -> Result<Report, EmitExit> {
    let dir = std::path::absolute(dir)
        .map_err(|error| EmitExit::Failed(format!("cannot resolve {}: {error}", dir.display())))?;
    let (Some(name), Some(parent)) = (dir.file_name(), dir.parent()) else {
        return Err(EmitExit::Refused(format!(
            "{} names no directory to write",
            dir.display()
        )));
    };
    let install = match classify(&dir)? {
        Target::Absent => Install::Create,
        Target::Empty => Install::FillEmpty,
        Target::Ours if options.force => Install::Replace,
        Target::Ours => {
            return Err(EmitExit::Refused(format!(
                "{} already holds the htui plugin; pass --force to replace it",
                dir.display()
            )));
        }
        Target::Foreign(why) => {
            return Err(EmitExit::Refused(format!(
                "{} holds files htui claude emit did not write ({why}); it is left as it is. \
                 Choose another --dir.",
                dir.display()
            )));
        }
        Target::NotADirectory => {
            return Err(EmitExit::Refused(format!(
                "{} is a file, not a directory",
                dir.display()
            )));
        }
    };
    let files = files_for(options.variant)?;
    fs::create_dir_all(parent).map_err(|error| {
        EmitExit::Failed(format!("cannot create {}: {error}", parent.display()))
    })?;
    // `create_dir`, not `_all`: a path already there fails, and nothing is touched.
    let staging = sibling(parent, name, "emit");
    fs::create_dir(&staging).map_err(|error| {
        EmitExit::Failed(format!("cannot create {}: {error}", staging.display()))
    })?;
    let discard = |error: EmitExit| {
        let _ = fs::remove_dir_all(&staging);
        error
    };
    fill(&staging, &files).map_err(discard)?;
    let into_place = |error: std::io::Error| {
        EmitExit::Failed(format!(
            "cannot move {} to {}: {error}",
            staging.display(),
            dir.display()
        ))
    };

    let mut leftover = None;
    match install {
        Install::Create => fs::rename(&staging, &dir)
            .map_err(into_place)
            .map_err(discard)?,
        Install::FillEmpty => fs::remove_dir(&dir)
            .and_then(|()| fs::rename(&staging, &dir))
            .map_err(into_place)
            .map_err(discard)?,
        Install::Replace => {
            let old = sibling(parent, name, "old");
            fs::rename(&dir, &old)
                .map_err(|error| {
                    EmitExit::Failed(format!(
                        "cannot move {} aside to {}: {error}",
                        dir.display(),
                        old.display()
                    ))
                })
                .map_err(discard)?;
            if let Err(error) = fs::rename(&staging, &dir) {
                let mut sentence = into_place(error).to_string();
                if let Err(back) = fs::rename(&old, &dir) {
                    sentence.push_str(&format!(
                        "; putting it back failed too ({back}): the previous plugin is at {}",
                        old.display()
                    ));
                }
                return Err(discard(EmitExit::Failed(sentence)));
            }
            if fs::remove_dir_all(&old).is_err() {
                leftover = Some(old);
            }
        }
    }
    Ok(Report {
        dir,
        variant: options.variant,
        replaced: install == Install::Replace,
        leftover,
    })
}

/// `<local data dir>/htui/claude-plugin` (`~/.local/share/htui/claude-plugin` on Linux), the
/// sibling of the agents' install root (D3). `None` on a box with no local data directory.
#[must_use]
pub fn default_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("htui").join("claude-plugin"))
}

/// Runs `claude --version` through the bounded, killable probe child (cwd: the temp dir), and
/// checks it against the floor.
pub async fn probe_claude() -> ClaudeCheck {
    ClaudeCheck::from_version(
        htui_agent::probe::capture_version(
            Path::new("claude"),
            &htui_agent::VersionProbe {
                args: vec!["--version".to_owned()],
                pattern: CLAUDE_VERSION_PATTERN.to_owned(),
                min: None,
            },
            &htui_agent::ProbeEnv::host(std::env::temp_dir()),
        )
        .await,
    )
}

/// What `htui claude emit` prints on stdout: what was written, whether the mod is in it, the two
/// install commands, the re-run hint, and a leftover copy when there is one. One line each.
#[must_use]
pub fn render(report: &Report, check: &ClaudeCheck) -> String {
    let dir = report.dir.display();
    let mut out = format!("Wrote htui's Claude Code plugin {VERSION} to {dir}.\n");
    out.push_str(&match check {
        ClaudeCheck::Met(version) => format!("The mod is included (claude {version}).\n"),
        ClaudeCheck::Below(version) => format!(
            "The mod is left out: claude {version} is below {CLAUDE_FLOOR}, so only the skill \
             was written.\n"
        ),
        ClaudeCheck::Unchecked => "The mod is included; claude's version could not be checked \
                                   (no answer from claude --version).\n"
            .to_owned(),
    });
    out.push_str("Install it once:\n");
    out.push_str(&format!(
        "  claude plugin marketplace add {}\n",
        shell_words::quote(&report.dir.to_string_lossy())
    ));
    out.push_str("  claude plugin install htui@htui\n");
    out.push_str(
        "After upgrading htui: htui claude emit --force, then /reload-plugins in Claude Code.\n",
    );
    if let Some(leftover) = &report.leftover {
        out.push_str(&format!(
            "The old copy could not be removed: {}.\n",
            leftover.display()
        ));
    }
    out
}

/// `htui claude emit`: picks the directory, checks `claude`'s version, writes, and prints the
/// report.
///
/// # Errors
///
/// [`EmitExit::Refused`] when there is no `--dir` and no local data directory, or [`write_to`]
/// refuses; [`EmitExit::Failed`] when it fails or stdout fails (a closed pipe excepted).
pub async fn run(args: cli::EmitArgs) -> Result<(), EmitExit> {
    let Some(dir) = args.dir.or_else(default_dir) else {
        return Err(EmitExit::Refused(
            "this machine has no local data directory; pass --dir PATH".to_owned(),
        ));
    };
    let check = probe_claude().await;
    let report = write_to(
        &dir,
        &Options {
            force: args.force,
            variant: check.variant(),
        },
    )?;
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(render(&report, &check).as_bytes())
        .and_then(|()| stdout.flush())
    {
        Err(error) if error.kind() != ErrorKind::BrokenPipe => {
            Err(EmitExit::Failed(format!("cannot write to stdout: {error}")))
        }
        _ => Ok(()),
    }
}

/// `plugin.json` with `"version"` set to [`VERSION`], and for [`Variant::SkillOnly`] without
/// `"types"` and `"userConfig"`. Pretty-printed, with a final newline.
fn stamp_manifest(body: &str, variant: Variant) -> Result<String, EmitExit> {
    let mut manifest: Map<String, Value> = serde_json::from_str(body).map_err(|error| {
        EmitExit::Failed(format!(
            "the plugin.json built into htui is not a JSON object: {error}"
        ))
    })?;
    manifest.insert("version".to_owned(), Value::String(VERSION.to_owned()));
    if variant == Variant::SkillOnly {
        manifest.remove("types");
        manifest.remove("userConfig");
    }
    let mut text = serde_json::to_string_pretty(&manifest).map_err(|error| {
        EmitExit::Failed(format!("cannot write the plugin's plugin.json: {error}"))
    })?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use serde_json::Value;

    use super::{
        CLAUDE_FLOOR, CLAUDE_VERSION_PATTERN, ClaudeCheck, EmitExit, FILES, Options, PLUGIN_JSON,
        Report, VERSION, VERSION_TS, VERSION_TS_LITERAL, Variant, default_dir, files_for, render,
        write_to,
    };

    /// What the source tree's `plugin.json` and `hooks/version.ts` carry in place of a version
    /// (D5).
    const SOURCE_VERSION: &str = "0.0.0-source";

    /// The source tree the table embeds.
    const SOURCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/claude-plugin");

    /// Every file under `root`, by relative `/`-path, dot-directories included, but the relative
    /// directories in `skip`.
    fn walk(root: &Path, skip: &[&str]) -> BTreeMap<String, Vec<u8>> {
        fn visit(dir: &Path, rel: &str, skip: &[&str], out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).expect("the directory reads") {
                let entry = entry.expect("the entry reads");
                let name = entry.file_name().into_string().expect("a UTF-8 name");
                let path = if rel.is_empty() {
                    name
                } else {
                    format!("{rel}/{name}")
                };
                let kind = entry.file_type().expect("the file type reads");
                if kind.is_dir() {
                    if !skip.contains(&path.as_str()) {
                        visit(&entry.path(), &path, skip, out);
                    }
                } else {
                    out.insert(path, std::fs::read(entry.path()).expect("the file reads"));
                }
            }
        }
        let mut out = BTreeMap::new();
        visit(root, "", skip, &mut out);
        out
    }

    /// The stamped files of `variant`, by path.
    fn stamped(variant: Variant) -> BTreeMap<&'static str, String> {
        files_for(variant)
            .expect("the embedded files stamp")
            .into_iter()
            .collect()
    }

    /// Every entry under `root` by relative `/`-path: a directory as `None`, a file as its bytes.
    /// What "untouched" compares.
    fn tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
        fn visit(dir: &Path, rel: &str, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
            for entry in std::fs::read_dir(dir).expect("the directory reads") {
                let entry = entry.expect("the entry reads");
                let name = entry.file_name().into_string().expect("a UTF-8 name");
                let path = if rel.is_empty() {
                    name
                } else {
                    format!("{rel}/{name}")
                };
                if entry.file_type().expect("the file type reads").is_dir() {
                    visit(&entry.path(), &path, out);
                    out.insert(path, None);
                } else {
                    out.insert(
                        path,
                        Some(std::fs::read(entry.path()).expect("the file reads")),
                    );
                }
            }
        }
        let mut out = BTreeMap::new();
        visit(root, "", &mut out);
        out
    }

    /// The files `variant` writes, as [`tree`] reads them back (their directories left out).
    fn written_files(variant: Variant) -> BTreeMap<String, Option<Vec<u8>>> {
        stamped(variant)
            .into_iter()
            .map(|(path, body)| (path.to_owned(), Some(body.into_bytes())))
            .collect()
    }

    /// [`tree`] without its directories.
    fn files_of(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
        tree(root)
            .into_iter()
            .filter(|(_, bytes)| bytes.is_some())
            .collect()
    }

    /// The names in `dir`, sorted.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("the directory reads")
            .map(|entry| {
                entry
                    .expect("the entry reads")
                    .file_name()
                    .into_string()
                    .expect("a UTF-8 name")
            })
            .collect();
        names.sort();
        names
    }

    /// [`write_to`] with these options.
    fn emit(dir: &Path, force: bool, variant: Variant) -> Result<Report, EmitExit> {
        write_to(dir, &Options { force, variant })
    }

    /// The sentence of a refusal; panics on anything else.
    fn refusal(outcome: Result<Report, EmitExit>) -> String {
        match outcome {
            Err(EmitExit::Refused(sentence)) => sentence,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// `plugin.json` parsed.
    fn manifest(text: &str) -> serde_json::Map<String, Value> {
        serde_json::from_str(text).expect("plugin.json is a JSON object")
    }

    /// D8: refused is 2, failed is 1, and each prints its sentence alone.
    #[test]
    fn emit_exits_are_2_and_1() {
        assert_eq!(EmitExit::Refused("r".into()).code(), 2);
        assert_eq!(EmitExit::Failed("f".into()).code(), 1);
        assert_eq!(EmitExit::Refused("no".into()).to_string(), "no");
        assert_eq!(EmitExit::Failed("broke".into()).to_string(), "broke");
    }

    /// D1, A-9: the table names every file of the source tree but the kit tests (D9) and the
    /// engine's types folder (A-10), and each body is its own path's text.
    #[test]
    fn every_source_file_is_embedded_and_every_embedded_file_is_the_source() {
        let source = walk(
            Path::new(SOURCE),
            &["htui/tests", "htui/.claude-plugin/types"],
        );
        let on_disk: BTreeSet<&str> = source.keys().map(String::as_str).collect();
        let table: BTreeSet<&str> = FILES.iter().map(|file| file.path).collect();
        assert_eq!(table.len(), FILES.len(), "no path appears twice");
        assert_eq!(
            on_disk, table,
            "the source tree and the table name the same files"
        );
        for file in FILES {
            assert_eq!(
                file.body.as_bytes(),
                source[file.path].as_slice(),
                "{} embeds its own path",
                file.path
            );
        }
    }

    /// D5: the source tree carries the placeholder, in `version.ts` exactly once.
    #[test]
    fn the_source_carries_the_placeholder_version_once() {
        let body = |path: &str| {
            FILES
                .iter()
                .find(|file| file.path == path)
                .unwrap_or_else(|| panic!("{path} is embedded"))
                .body
        };
        assert_eq!(
            manifest(body(PLUGIN_JSON))["version"],
            Value::from(SOURCE_VERSION)
        );
        assert_eq!(body(VERSION_TS).matches(VERSION_TS_LITERAL).count(), 1);
    }

    /// D5: the binary's version, in both places.
    #[test]
    fn the_stamped_version_is_the_binarys_in_both_places() {
        let files = stamped(Variant::WithMod);
        assert_eq!(
            manifest(&files[PLUGIN_JSON])["version"],
            Value::from(VERSION)
        );
        assert_eq!(
            files[VERSION_TS],
            format!("export const VERSION = '{VERSION}'\n")
        );
    }

    /// D6: the full plugin keeps the mod's files and manifest keys as the source has them.
    #[test]
    fn the_full_variant_keeps_hooks_types_and_user_config() {
        let files = stamped(Variant::WithMod);
        let paths: BTreeSet<&str> = files.keys().copied().collect();
        let all: BTreeSet<&str> = FILES.iter().map(|file| file.path).collect();
        assert_eq!(paths, all);
        let source = manifest(FILES[1].body);
        let written = manifest(&files[PLUGIN_JSON]);
        for key in ["types", "userConfig", "name", "description", "author"] {
            assert_eq!(written.get(key), source.get(key), "{key}");
            assert!(written.contains_key(key), "{key}");
        }
        for file in FILES {
            if file.path != PLUGIN_JSON && file.path != VERSION_TS {
                assert_eq!(files[file.path], file.body, "{} is copied as is", file.path);
            }
        }
    }

    /// D6: below the floor, no `hooks/`, no `types/`, and no manifest key pointing at them.
    #[test]
    fn the_skill_only_variant_has_no_hooks_types_or_their_manifest_keys() {
        let files = stamped(Variant::SkillOnly);
        let paths: BTreeSet<&str> = files.keys().copied().collect();
        assert_eq!(
            paths,
            BTreeSet::from([
                ".claude-plugin/marketplace.json",
                PLUGIN_JSON,
                "htui/skills/htui/SKILL.md",
            ])
        );
        let mut source = manifest(FILES[1].body);
        source.remove("types");
        source.remove("userConfig");
        source.insert("version".to_owned(), Value::from(VERSION));
        assert_eq!(manifest(&files[PLUGIN_JSON]), source);
    }

    /// D3: a target that does not exist is created, parents included, holding exactly the
    /// variant's files.
    #[test]
    fn an_absent_directory_is_created_with_its_parents() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("a").join("b").join("plugin");
        let report = emit(&dir, false, Variant::WithMod).expect("the emit succeeds");
        assert_eq!(
            report,
            Report {
                dir: dir.clone(),
                variant: Variant::WithMod,
                replaced: false,
                leftover: None,
            }
        );
        assert_eq!(files_of(&dir), written_files(Variant::WithMod));
    }

    /// D4: an empty directory is not "non-empty", so it needs no `--force`.
    #[test]
    fn an_empty_directory_is_filled_without_force() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        std::fs::create_dir(&dir).expect("the empty target");
        let report = emit(&dir, false, Variant::SkillOnly).expect("the emit succeeds");
        assert!(!report.replaced);
        assert_eq!(files_of(&dir), written_files(Variant::SkillOnly));
    }

    /// D4, PRD: an earlier emit is refused without `--force`, the refusal says so, and nothing
    /// changes.
    #[test]
    fn a_non_empty_directory_is_refused_without_force_and_the_refusal_names_the_flag() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        emit(&dir, false, Variant::WithMod).expect("the first emit");
        let before = tree(&dir);
        let sentence = refusal(emit(&dir, false, Variant::SkillOnly));
        assert_eq!(
            sentence,
            format!(
                "{} already holds the htui plugin; pass --force to replace it",
                dir.display()
            )
        );
        assert_eq!(tree(&dir), before);
        assert_eq!(names_in(root.path()), ["plugin"]);
    }

    /// D4: `--force` swaps an earlier emit for a fresh one; nothing of the old tree survives.
    #[test]
    fn force_replaces_an_earlier_emit_and_leaves_no_stale_file() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        emit(&dir, false, Variant::WithMod).expect("the first emit");
        std::fs::write(dir.join("htui/hooks/stale.ts"), "export {}\n").expect("the stale file");
        let report = emit(&dir, true, Variant::SkillOnly).expect("the forced emit");
        assert_eq!(
            report,
            Report {
                dir: dir.clone(),
                variant: Variant::SkillOnly,
                replaced: true,
                leftover: None,
            }
        );
        assert!(!dir.join("htui/hooks").exists(), "hooks/ is gone");
        assert_eq!(files_of(&dir), written_files(Variant::SkillOnly));
    }

    /// D4: a directory `emit` did not write is refused, `--force` or not, and left byte for byte
    /// as it was.
    #[test]
    fn a_foreign_directory_is_refused_even_with_force_and_left_untouched() {
        /// Turns an earlier emit into a foreign directory.
        type Plant<'a> = &'a dyn Fn(&Path);
        let cases: [(&str, Plant); 4] = [
            ("notes.txt", &|dir| {
                std::fs::write(dir.join("notes.txt"), "mine\n").expect("the extra file");
            }),
            ("marketplace.json is not htui's", &|dir| {
                std::fs::write(
                    dir.join(".claude-plugin/marketplace.json"),
                    r#"{ "name": "other", "plugins": [] }"#,
                )
                .expect("the foreign marketplace");
            }),
            ("no marketplace.json", &|dir| {
                std::fs::remove_file(dir.join(".claude-plugin/marketplace.json"))
                    .expect("the marketplace removed");
            }),
            ("marketplace.json is not htui's", &|dir| {
                std::fs::write(dir.join(".claude-plugin/marketplace.json"), "not json")
                    .expect("the broken marketplace");
            }),
        ];
        for (why, plant) in cases {
            for force in [false, true] {
                let root = tempfile::tempdir().expect("a temp dir");
                let dir = root.path().join("plugin");
                emit(&dir, false, Variant::WithMod).expect("the first emit");
                plant(&dir);
                let before = tree(&dir);
                let sentence = refusal(emit(&dir, force, Variant::WithMod));
                assert_eq!(
                    sentence,
                    format!(
                        "{} holds files htui claude emit did not write ({why}); it is left as it \
                         is. Choose another --dir.",
                        dir.display()
                    ),
                    "force: {force}"
                );
                assert_eq!(tree(&dir), before, "{why}, force: {force}");
                assert_eq!(names_in(root.path()), ["plugin"], "{why}, force: {force}");
            }
        }
    }

    /// D4: a file is never replaced, and a path naming no directory is refused.
    #[test]
    fn a_file_target_is_refused() {
        let root = tempfile::tempdir().expect("a temp dir");
        let file = root.path().join("plugin");
        std::fs::write(&file, "mine\n").expect("the file");
        for force in [false, true] {
            let sentence = refusal(emit(&file, force, Variant::WithMod));
            assert_eq!(
                sentence,
                format!("{} is a file, not a directory", file.display())
            );
            assert_eq!(std::fs::read(&file).expect("the file reads"), b"mine\n");
        }
        assert_eq!(names_in(root.path()), ["plugin"]);
        let sentence = refusal(emit(Path::new("/"), true, Variant::WithMod));
        assert_eq!(sentence, "/ names no directory to write");
    }

    /// D4: a symbolic link is never followed, even to an earlier emit.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_target_is_refused() {
        let root = tempfile::tempdir().expect("a temp dir");
        let real = root.path().join("real");
        emit(&real, false, Variant::WithMod).expect("the first emit");
        let link = root.path().join("plugin");
        std::os::unix::fs::symlink(&real, &link).expect("the link");
        let before = tree(&real);
        for force in [false, true] {
            let sentence = refusal(emit(&link, force, Variant::SkillOnly));
            assert!(sentence.contains("(a symbolic link)"), "{sentence}");
        }
        assert_eq!(tree(&real), before);
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("the link is there")
                .file_type()
                .is_symlink()
        );
        assert_eq!(names_in(root.path()), ["plugin", "real"]);
    }

    /// D7: neither the staging directory nor the moved-aside copy outlives a successful emit.
    #[test]
    fn nothing_is_left_beside_the_target() {
        let root = tempfile::tempdir().expect("a temp dir");
        let parent = root.path().join("a").join("b");
        emit(&parent.join("plugin"), false, Variant::WithMod).expect("an absent target");
        assert_eq!(names_in(&parent), ["plugin"]);

        let root = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir(root.path().join("plugin")).expect("the empty target");
        emit(&root.path().join("plugin"), false, Variant::WithMod).expect("an empty target");
        assert_eq!(names_in(root.path()), ["plugin"]);

        emit(&root.path().join("plugin"), true, Variant::WithMod).expect("a forced emit");
        assert_eq!(names_in(root.path()), ["plugin"]);
    }

    /// D4: when the staging directory cannot be made, the earlier emit is untouched.
    #[test]
    fn a_staging_failure_leaves_the_target_as_it_was() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        emit(&dir, false, Variant::WithMod).expect("the first emit");
        let staging = root
            .path()
            .join(format!("plugin.emit-{}", std::process::id()));
        std::fs::write(&staging, "in the way\n").expect("the blocking file");
        let before = tree(&dir);
        match emit(&dir, true, Variant::SkillOnly) {
            Err(EmitExit::Failed(sentence)) => assert!(
                sentence.starts_with(&format!("cannot create {}: ", staging.display())),
                "{sentence}"
            ),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(tree(&dir), before);
        assert_eq!(
            std::fs::read(&staging).expect("the blocking file reads"),
            b"in the way\n"
        );
    }

    /// D6: below 2.1.287 the skill alone; at or above it, or unchecked, the mod too. The pattern
    /// reads `claude --version`'s line.
    #[test]
    fn the_floor_decides_the_variant() {
        let below = ClaudeCheck::from_version(Some("2.1.286".to_owned()));
        assert_eq!(below, ClaudeCheck::Below("2.1.286".to_owned()));
        assert_eq!(below.variant(), Variant::SkillOnly);
        for met in ["2.1.287", "2.1.296"] {
            let check = ClaudeCheck::from_version(Some(met.to_owned()));
            assert_eq!(check, ClaudeCheck::Met(met.to_owned()));
            assert_eq!(check.variant(), Variant::WithMod);
        }
        assert_eq!(ClaudeCheck::from_version(None), ClaudeCheck::Unchecked);
        assert_eq!(ClaudeCheck::Unchecked.variant(), Variant::WithMod);
        assert_eq!(
            htui_agent::probe::extract_version("2.1.296 (Claude Code)", CLAUDE_VERSION_PATTERN)
                .as_deref(),
            Some("2.1.296")
        );
        assert_eq!(CLAUDE_FLOOR, "2.1.287");
    }

    /// The report: what was written, whether the mod is in it, both commands, the re-run hint,
    /// and a leftover when there is one.
    #[test]
    fn the_report_prints_both_commands_and_the_rerun_hint() {
        let report = |dir: &str, leftover: Option<&str>| Report {
            dir: dir.into(),
            variant: Variant::WithMod,
            replaced: leftover.is_some(),
            leftover: leftover.map(Into::into),
        };
        let tail = "Install it once:\n  claude plugin marketplace add /p/x\n  claude plugin install \
                    htui@htui\nAfter upgrading htui: htui claude emit --force, then \
                    /reload-plugins in Claude Code.\n";
        assert_eq!(
            render(
                &report("/p/x", None),
                &ClaudeCheck::Met("2.1.296".to_owned())
            ),
            format!(
                "Wrote htui's Claude Code plugin {VERSION} to /p/x.\nThe mod is included (claude \
                 2.1.296).\n{tail}"
            )
        );
        assert_eq!(
            render(
                &report("/p/x", None),
                &ClaudeCheck::Below("2.1.200".to_owned())
            ),
            format!(
                "Wrote htui's Claude Code plugin {VERSION} to /p/x.\nThe mod is left out: claude \
                 2.1.200 is below 2.1.287, so only the skill was written.\n{tail}"
            )
        );
        assert_eq!(
            render(&report("/p/x", None), &ClaudeCheck::Unchecked),
            format!(
                "Wrote htui's Claude Code plugin {VERSION} to /p/x.\nThe mod is included; \
                 claude's version could not be checked (no answer from claude --version).\n{tail}"
            )
        );
        let spaced = render(
            &report("/my plugins/x", Some("/my plugins/x.old-7")),
            &ClaudeCheck::Unchecked,
        );
        assert!(
            spaced.contains("\n  claude plugin marketplace add '/my plugins/x'\n"),
            "{spaced}"
        );
        assert!(
            spaced.ends_with("\nThe old copy could not be removed: /my plugins/x.old-7.\n"),
            "{spaced}"
        );
    }

    /// D3: `<local data dir>/htui/claude-plugin`, the sibling of `htui/agents`.
    #[test]
    fn the_default_dir_is_under_the_local_data_dir() {
        let expected = dirs::data_local_dir().map(|dir| dir.join("htui").join("claude-plugin"));
        assert_eq!(default_dir(), expected);
        if let Some(dir) = default_dir() {
            assert!(dir.ends_with("htui/claude-plugin"), "{}", dir.display());
            assert!(dir.is_absolute(), "{}", dir.display());
        }
    }
}
