//! `htui claude emit` (MOD-93, `R-ID-4`, `R-ID-6`, `R-STO-1`). Writes htui's Claude Code plugin
//! as a directory marketplace: `.claude-plugin/marketplace.json` and the plugin `htui/` (the skill,
//! and the mod when `claude --version` is at least [`CLAUDE_FLOOR`] or does not answer), every
//! file embedded in this binary from `crates/htui/claude-plugin/`. The plugin's version is the
//! binary's ([`VERSION`]). Then it prints the two `claude plugin` commands that install it; it runs
//! neither, and never touches `~/.claude`.
//!
//! The target is written only when it is absent or empty, or, with `--force`, when it holds an
//! earlier emit: every file one `emit` writes (or the engine's `htui/.claude-plugin/types/`, A-10),
//! no link, a marketplace named `htui`, and not the source tree's placeholder version (D4).
//! Anything else is refused, `--force` or not, and so is a path with a `..` (the default
//! directory's included) or one that is not UTF-8. An earlier emit is checked again right before
//! it is moved aside. Nothing outside the target and its two siblings
//! `<dir>.emit-<pid>` (staging) and `<dir>.old-<pid>` (the earlier emit, moved aside) is written,
//! but the target's missing parents. Exits are 2 (refused) and 1 (failed), and none reaches Sentry:
//! the sentences carry home paths.

use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value};

use crate::cli;

/// The version every emitted plugin carries: this binary's (D5).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The Claude Code version mods need (ANA-29 §2.1); below it the skill alone is written (D6).
pub const CLAUDE_FLOOR: &str = "2.1.287";
/// `claude --version` prints `2.1.296 (Claude Code)`.
pub const CLAUDE_VERSION_PATTERN: &str = r"^(\d+\.\d+\.\d+)";

/// What the source tree's `plugin.json` and `hooks/version.ts` carry in place of a version (D5).
/// A target whose `plugin.json` says it is the source tree, not an emit, and is never replaced.
const SOURCE_VERSION: &str = "0.0.0-source";
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
    /// Exit 2, nothing left written: the target holds an earlier emit and `--force` is
    /// missing; holds anything `emit` did not write (checked again right before an earlier emit
    /// is moved aside); is a file; is a path with a `..`, not valid UTF-8, or naming no
    /// directory; or there is no `--dir` and the default directory is missing (no local data
    /// directory) or has a `..`.
    Refused(String),
    /// Exit 1: a read, a write, a removal, a rename or stdout failed.
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
    /// An earlier emit: every file one of [`FILES`]' paths (the engine's types folder aside), a
    /// marketplace named `htui`, and not the source tree's placeholder version.
    Ours,
    /// Anything else that is a directory (or a link); why, for the refusal.
    Foreign(String),
    /// A file or some other non-directory.
    NotADirectory,
}

/// The folder the engine writes the mod's types into when it hot-reloads a plugin folder (A-10):
/// the one thing an earlier emit may hold that `emit` did not write.
const ENGINE_TYPES: &str = "htui/.claude-plugin/types";

/// Whether `rel` is a directory an emit makes: a parent of one of [`FILES`]' paths.
fn emitted_dir(rel: &str) -> bool {
    FILES.iter().any(|file| {
        file.path
            .strip_prefix(rel)
            .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// The first entry under `dir` (at relative `/`-path `rel`) that no emit writes, in name order, as
/// the refusal names it: a [`FILES`] path is a regular file, every directory is a parent of one,
/// and nothing is a link. [`ENGINE_TYPES`] is not looked into.
fn first_foreign(
    dir: &Path,
    rel: &str,
    unreadable: &dyn Fn(std::io::Error) -> EmitExit,
) -> Result<Option<String>, EmitExit> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        entries.push((entry.file_name(), entry.file_type().map_err(unreadable)?));
    }
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (name, kind) in entries {
        let name_text = name.to_string_lossy();
        let path = if rel.is_empty() {
            name_text.into_owned()
        } else {
            format!("{rel}/{name_text}")
        };
        if kind.is_symlink() {
            return Ok(Some(format!("{path}, a symbolic link")));
        }
        if name.to_str().is_none() {
            return Ok(Some(path));
        }
        if kind.is_dir() {
            if path == ENGINE_TYPES {
                continue;
            }
            if !emitted_dir(&path) {
                return Ok(Some(path));
            }
            if let Some(found) = first_foreign(&dir.join(&name), &path, unreadable)? {
                return Ok(Some(found));
            }
        } else if !(kind.is_file() && FILES.iter().any(|file| file.path == path)) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

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
    if fs::read_dir(dir).map_err(unreadable)?.next().is_none() {
        return Ok(Target::Empty);
    }
    if let Some(found) = first_foreign(dir, "", &unreadable)? {
        return Ok(Target::Foreign(found));
    }
    let json = |path: &str| -> Option<Value> {
        let file = path
            .split('/')
            .fold(dir.to_path_buf(), |dir, part| dir.join(part));
        serde_json::from_str(&fs::read_to_string(file).ok()?).ok()
    };
    let Ok(text) = fs::read_to_string(dir.join(".claude-plugin").join("marketplace.json")) else {
        return Ok(Target::Foreign("no marketplace.json".to_owned()));
    };
    let named_htui = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|marketplace| marketplace.get("name")?.as_str().map(|name| name == "htui"))
        .unwrap_or(false);
    if !named_htui {
        return Ok(Target::Foreign("marketplace.json is not htui's".to_owned()));
    }
    // The source tree holds the same paths; its placeholder version tells it from an emit.
    let source = json(PLUGIN_JSON).is_some_and(|manifest| {
        manifest.get("version").and_then(Value::as_str) == Some(SOURCE_VERSION)
    });
    if source {
        return Ok(Target::Foreign(format!(
            "the plugin source tree, version {SOURCE_VERSION}"
        )));
    }
    Ok(Target::Ours)
}

/// `dir` made absolute, with its `.` components and a trailing separator dropped, so that a
/// symbolic link is the link and the printed path is plain.
///
/// # Errors
///
/// [`EmitExit::Refused`] for a path with a `..` (resolving it could create a directory outside
/// the target) or one that is not UTF-8 (Claude Code takes the path as text);
/// [`EmitExit::Failed`] when the current directory cannot be read.
fn target_path(dir: &Path) -> Result<PathBuf, EmitExit> {
    if dir.components().any(|part| part == Component::ParentDir) {
        return Err(EmitExit::Refused(format!(
            "{} contains ..; give the path without ..",
            dir.display()
        )));
    }
    let absolute = std::path::absolute(dir)
        .map_err(|error| EmitExit::Failed(format!("cannot resolve {}: {error}", dir.display())))?;
    let plain: PathBuf = absolute
        .components()
        .filter(|part| *part != Component::CurDir)
        .collect();
    if plain.to_str().is_none() {
        return Err(EmitExit::Refused(format!(
            "{} is not valid UTF-8; Claude Code cannot add it",
            plain.display()
        )));
    }
    Ok(plain)
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

/// A rename, a parameter so that a test can fail one (F5): [`write_to`] passes `fs::rename`.
type Rename<'a> = &'a dyn Fn(&Path, &Path) -> std::io::Result<()>;

/// A target [`prepare`] found `emit` may write: what [`install`] needs, nothing written yet.
#[derive(Debug)]
struct Prepared {
    /// The target, absolute and plain ([`target_path`]).
    dir: PathBuf,
    /// Its last component.
    name: std::ffi::OsString,
    /// Its parent.
    parent: PathBuf,
    /// How the staged tree goes into place.
    install: Install,
}

/// How `target` at `dir` is written (D4), or the refusal.
///
/// # Errors
///
/// [`EmitExit::Refused`] for a target `emit` did not write (even with `force`), a file, or an
/// earlier emit without `force`.
fn decide(dir: &Path, target: Target, force: bool) -> Result<Install, EmitExit> {
    match target {
        Target::Absent => Ok(Install::Create),
        Target::Empty => Ok(Install::FillEmpty),
        Target::Ours if force => Ok(Install::Replace),
        Target::Ours => Err(EmitExit::Refused(format!(
            "{} already holds the htui plugin; pass --force to replace it",
            dir.display()
        ))),
        Target::Foreign(why) => Err(EmitExit::Refused(format!(
            "{} holds files htui claude emit did not write ({why}); it is left as it is. Choose \
             another --dir.",
            dir.display()
        ))),
        Target::NotADirectory => Err(EmitExit::Refused(format!(
            "{} is a file, not a directory",
            dir.display()
        ))),
    }
}

/// Normalises `dir` and classifies it (D4), touching nothing: every refusal but [`install`]'s late
/// `--force` recheck comes from here, so [`run`] gives it before it runs `claude --version`.
///
/// # Errors
///
/// [`EmitExit::Refused`] for a target `emit` did not write (even with `force`), a file, a path
/// naming no directory, with a `..` or not UTF-8, or an earlier emit without `force`;
/// [`EmitExit::Failed`] when the target cannot be read.
fn prepare(dir: &Path, force: bool) -> Result<Prepared, EmitExit> {
    let dir = target_path(dir)?;
    let (Some(name), Some(parent)) = (dir.file_name(), dir.parent()) else {
        return Err(EmitExit::Refused(format!(
            "{} names no directory to write",
            dir.display()
        )));
    };
    let (name, parent) = (name.to_os_string(), parent.to_path_buf());
    let install = decide(&dir, classify(&dir)?, force)?;
    Ok(Prepared {
        dir,
        name,
        parent,
        install,
    })
}

/// Writes `variant` into the prepared target: stages it in a sibling `<dir>.emit-<pid>`, then
/// moves it into place with `rename` (D4). An earlier emit is moved aside to `<dir>.old-<pid>`
/// first, and removed once the new tree is in place.
///
/// # Errors
///
/// [`EmitExit::Refused`] when an earlier emit to replace is no longer one right before it is
/// moved aside (TOCTOU), with [`prepare`]'s sentence; it is left as it is.
/// [`EmitExit::Failed`] when a directory cannot be read, made, removed or renamed, or a file
/// cannot be written. The staging directory is removed, a moved-aside emit is put back (or the
/// sentence names where it is), and an emptied target is made again.
fn install(prepared: Prepared, variant: Variant, rename: Rename<'_>) -> Result<Report, EmitExit> {
    let Prepared {
        dir,
        name,
        parent,
        install,
    } = prepared;
    let files = files_for(variant)?;
    fs::create_dir_all(&parent).map_err(|error| {
        EmitExit::Failed(format!("cannot create {}: {error}", parent.display()))
    })?;
    // `create_dir`, not `_all`: a path already there fails, and nothing is touched.
    let staging = sibling(&parent, &name, "emit");
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

    // `prepare`'s check was made before the staging was written: an earlier emit is classified
    // again right before it is moved, and refused as `prepare` refuses it once it is no longer
    // ours. One that is gone or emptied meanwhile is written as such.
    let install = match install {
        Install::Replace => classify(&dir)
            .and_then(|target| decide(&dir, target, true))
            .map_err(discard)?,
        other => other,
    };
    let mut leftover = None;
    match install {
        Install::Create => rename(&staging, &dir)
            .map_err(into_place)
            .map_err(discard)?,
        Install::FillEmpty => {
            fs::remove_dir(&dir)
                .map_err(|error| {
                    EmitExit::Failed(format!(
                        "cannot remove the empty directory {}: {error}",
                        dir.display()
                    ))
                })
                .map_err(discard)?;
            if let Err(error) = rename(&staging, &dir) {
                // Best effort: the empty directory the person made is there again.
                let _ = fs::create_dir(&dir);
                return Err(discard(into_place(error)));
            }
        }
        Install::Replace => {
            let old = sibling(&parent, &name, "old");
            rename(&dir, &old)
                .map_err(|error| {
                    EmitExit::Failed(format!(
                        "cannot move {} aside to {}: {error}",
                        dir.display(),
                        old.display()
                    ))
                })
                .map_err(discard)?;
            if let Err(error) = rename(&staging, &dir) {
                let mut sentence = into_place(error).to_string();
                if let Err(back) = rename(&old, &dir) {
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
        variant,
        replaced: install == Install::Replace,
        leftover,
    })
}

/// Writes the plugin into `dir`: `prepare`, then `install` with `fs::rename`. Nothing outside
/// `dir`, its siblings `<dir>.emit-<pid>` and `<dir>.old-<pid>`, and `dir`'s missing parents is
/// written.
///
/// # Errors
///
/// [`EmitExit::Refused`] for a target `emit` may not write (even with `force`), a file, a path
/// naming no directory, with a `..` or not UTF-8, or an earlier emit without `force`; nothing is
/// touched then. The same for an earlier emit that is no longer one when it would be moved aside;
/// the target is left as it is, and the staging directory removed. [`EmitExit::Failed`] when a
/// directory cannot be read, made, removed or renamed, or a file cannot be written; the staging
/// directory is removed, and a moved-aside emit is put back.
pub fn write_to(dir: &Path, options: &Options) -> Result<Report, EmitExit> {
    install(
        prepare(dir, options.force)?,
        options.variant,
        &|from, to| fs::rename(from, to),
    )
}

/// `<local data dir>/htui/claude-plugin` (`~/.local/share/htui/claude-plugin` on Linux), the
/// sibling of the agents' install root (D3). `None` on a box with no local data directory.
#[must_use]
pub fn default_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("htui").join("claude-plugin"))
}

/// The directory `emit` writes: `given` (`--dir`), else `default`.
///
/// # Errors
///
/// [`EmitExit::Refused`] when there is neither, or `default` has a `..` (from `HOME` or
/// `XDG_DATA_HOME`): its own sentence, since no `--dir` was given to drop it from. A given
/// `--dir` with a `..` is refused by [`target_path`].
fn chosen_dir(given: Option<PathBuf>, default: Option<PathBuf>) -> Result<PathBuf, EmitExit> {
    if let Some(given) = given {
        return Ok(given);
    }
    let Some(default) = default else {
        return Err(EmitExit::Refused(
            "this machine has no local data directory; pass --dir PATH".to_owned(),
        ));
    };
    if default
        .components()
        .any(|part| part == Component::ParentDir)
    {
        return Err(EmitExit::Refused(format!(
            "{} (the default directory) contains ..; pass --dir PATH without ..",
            default.display()
        )));
    }
    Ok(default)
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
    // The bare verb writes the default dir; any other needs its `--dir` again.
    let rerun = if default_dir().as_deref() == Some(report.dir.as_path()) {
        String::new()
    } else {
        format!(
            " --dir {}",
            shell_words::quote(&report.dir.to_string_lossy())
        )
    };
    out.push_str(&format!(
        "After upgrading htui: htui claude emit --force{rerun}, then /reload-plugins in Claude \
         Code.\n"
    ));
    if let Some(leftover) = &report.leftover {
        out.push_str(&format!(
            "The old copy could not be removed: {}.\n",
            leftover.display()
        ));
    }
    out
}

/// `htui claude emit`: picks the directory, checks it, then checks `claude`'s version, writes,
/// and prints the report.
///
/// # Errors
///
/// [`EmitExit::Refused`] when there is no `--dir` and no local data directory (or one with a
/// `..`), or the target is refused (before `claude` runs, but for the late `--force` recheck);
/// [`EmitExit::Failed`] when the write fails or stdout fails (a closed pipe excepted).
pub async fn run(args: cli::EmitArgs) -> Result<(), EmitExit> {
    run_with(args, probe_claude, &mut std::io::stdout()).await
}

/// [`run`] with its version probe and its stdout given (F4, V-stdout).
async fn run_with<Probe, Checked>(
    args: cli::EmitArgs,
    probe: Probe,
    out: &mut impl std::io::Write,
) -> Result<(), EmitExit>
where
    Probe: FnOnce() -> Checked,
    Checked: Future<Output = ClaudeCheck>,
{
    // Every refusal but install's late `--force` recheck, before `claude --version` runs.
    let dir = chosen_dir(args.dir, default_dir())?;
    let prepared = prepare(&dir, args.force)?;
    let check = probe().await;
    let report = install(prepared, check.variant(), &|from, to| fs::rename(from, to))?;
    match out
        .write_all(render(&report, &check).as_bytes())
        .and_then(|()| out.flush())
    {
        Err(error) if error.kind() != ErrorKind::BrokenPipe => Err(EmitExit::Failed(format!(
            "the plugin was written to {}; cannot write to stdout: {error}",
            report.dir.display()
        ))),
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
        Report, SOURCE_VERSION, VERSION, VERSION_TS, VERSION_TS_LITERAL, Variant, chosen_dir,
        default_dir, files_for, install, prepare, render, run_with, write_to,
    };
    use crate::cli::EmitArgs;

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

    /// D4: `--force` swaps an earlier emit for a fresh one; nothing of the old tree survives: not
    /// the mod a skill-only emit leaves out, and not the types the engine wrote beside it (A-10).
    #[test]
    fn force_replaces_an_earlier_emit_and_leaves_no_stale_file() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        emit(&dir, false, Variant::WithMod).expect("the first emit");
        let types = dir.join("htui/.claude-plugin/types/claude-code");
        std::fs::create_dir_all(&types).expect("the engine's types folder");
        std::fs::write(types.join("index.d.ts"), "export {}\n").expect("the engine's types");
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
        assert!(
            !dir.join("htui/.claude-plugin/types").exists(),
            "types/ is gone"
        );
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
                    htui@htui\nAfter upgrading htui: htui claude emit --force --dir /p/x, then \
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
        assert!(
            spaced.contains(
                "\nAfter upgrading htui: htui claude emit --force --dir '/my plugins/x', then \
                 /reload-plugins in Claude Code.\n"
            ),
            "{spaced}"
        );
    }

    /// The rerun hint names no `--dir` for the default directory: the bare verb writes there.
    #[test]
    fn the_rerun_hint_names_the_dir_only_off_the_default() {
        let Some(dir) = default_dir() else {
            return;
        };
        let rendered = render(
            &Report {
                dir,
                variant: Variant::WithMod,
                replaced: false,
                leftover: None,
            },
            &ClaudeCheck::Unchecked,
        );
        assert!(
            rendered.contains(
                "\nAfter upgrading htui: htui claude emit --force, then /reload-plugins in Claude \
                 Code.\n"
            ),
            "{rendered}"
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

    /// `<root>/plugin.<kind>-<pid>`, a sibling `emit` makes beside `<root>/plugin`.
    fn sibling_of(root: &Path, kind: &str) -> std::path::PathBuf {
        root.join(format!("plugin.{kind}-{}", std::process::id()))
    }

    /// The failure's sentence; panics on anything else.
    fn failure<T: core::fmt::Debug>(outcome: Result<T, EmitExit>) -> String {
        match outcome {
            Err(EmitExit::Failed(sentence)) => sentence,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    /// `htui claude emit --dir <dir> [--force]`, run with a probe answering `check` that counts
    /// its calls, printing into `out`. The outcome and how often `claude` was probed.
    fn run_counted(
        dir: &Path,
        force: bool,
        out: &mut impl std::io::Write,
    ) -> (Result<(), EmitExit>, usize) {
        let probed = std::cell::Cell::new(0);
        let outcome = futures::executor::block_on(run_with(
            EmitArgs {
                dir: Some(dir.to_path_buf()),
                force,
            },
            || {
                probed.set(probed.get() + 1);
                async { ClaudeCheck::Met("2.1.296".to_owned()) }
            },
            out,
        ));
        (outcome, probed.get())
    }

    /// F4: every refusal from `prepare` returns before `claude --version` runs; a write runs it
    /// once and prints the report.
    #[test]
    fn a_refusal_returns_before_claude_is_probed() {
        let root = tempfile::tempdir().expect("a temp dir");
        let file = root.path().join("file");
        std::fs::write(&file, "mine\n").expect("the file");
        let ours = root.path().join("ours");
        emit(&ours, false, Variant::WithMod).expect("an earlier emit");
        let foreign_dir = root.path().join("foreign");
        std::fs::create_dir(&foreign_dir).expect("the foreign dir");
        std::fs::write(foreign_dir.join("notes.txt"), "mine\n").expect("the foreign file");
        let up = root.path().join("a").join("..").join("b");
        for (dir, force) in [
            (&file, true),
            (&ours, false),
            (&foreign_dir, true),
            (&up, false),
        ] {
            let mut out = Vec::new();
            let (outcome, probed) = run_counted(dir, force, &mut out);
            assert!(
                matches!(outcome, Err(EmitExit::Refused(_))),
                "{}: {outcome:?}",
                dir.display()
            );
            assert_eq!(probed, 0, "{}: claude was probed", dir.display());
            assert!(out.is_empty(), "{}", dir.display());
        }

        let dir = root.path().join("fresh");
        let mut out = Vec::new();
        let (outcome, probed) = run_counted(&dir, false, &mut out);
        outcome.expect("the emit succeeds");
        assert_eq!(probed, 1);
        let expected = render(
            &Report {
                dir: dir.clone(),
                variant: Variant::WithMod,
                replaced: false,
                leftover: None,
            },
            &ClaudeCheck::Met("2.1.296".to_owned()),
        );
        assert_eq!(String::from_utf8(out).expect("UTF-8"), expected);
    }

    /// Fails every write with `kind`.
    struct Broken(std::io::ErrorKind);

    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(self.0, "stdout is gone"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(self.0, "stdout is gone"))
        }
    }

    /// V-stdout: stdout failing after the write still says where the plugin went; a closed pipe
    /// is no failure.
    #[test]
    fn a_stdout_failure_after_the_write_names_the_written_dir() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        let (outcome, _) = run_counted(&dir, false, &mut Broken(std::io::ErrorKind::Other));
        assert_eq!(
            failure(outcome),
            format!(
                "the plugin was written to {}; cannot write to stdout: stdout is gone",
                dir.display()
            )
        );
        assert_eq!(files_of(&dir), written_files(Variant::WithMod));

        let (outcome, _) = run_counted(&dir, true, &mut Broken(std::io::ErrorKind::BrokenPipe));
        outcome.expect("a closed pipe is no failure");
    }

    /// F5: when the new tree cannot be renamed into place, the earlier emit is put back byte for
    /// byte and neither sibling is left.
    #[test]
    fn a_failed_swap_puts_the_old_copy_back_byte_for_byte() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        emit(&dir, false, Variant::WithMod).expect("the first emit");
        let before = tree(&dir);
        let staging = sibling_of(root.path(), "emit");
        let prepared = prepare(&dir, true).expect("an earlier emit with --force");
        let sentence = failure(install(prepared, Variant::SkillOnly, &|from, to| {
            if from == staging {
                return Err(std::io::Error::other("rename refused"));
            }
            std::fs::rename(from, to)
        }));
        assert_eq!(
            sentence,
            format!(
                "cannot move {} to {}: rename refused",
                staging.display(),
                dir.display()
            )
        );
        assert_eq!(tree(&dir), before);
        assert_eq!(names_in(root.path()), ["plugin"]);
    }

    /// F5: when putting the earlier emit back fails too, the sentence names where it sits, and it
    /// sits there whole.
    #[test]
    fn a_failed_put_back_names_where_the_old_copy_is() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        emit(&dir, false, Variant::WithMod).expect("the first emit");
        let before = tree(&dir);
        let staging = sibling_of(root.path(), "emit");
        let old = sibling_of(root.path(), "old");
        let prepared = prepare(&dir, true).expect("an earlier emit with --force");
        let sentence = failure(install(prepared, Variant::WithMod, &|from, to| {
            if to == dir {
                return Err(std::io::Error::other("rename refused"));
            }
            std::fs::rename(from, to)
        }));
        assert_eq!(
            sentence,
            format!(
                "cannot move {} to {}: rename refused; putting it back failed too (rename \
                 refused): the previous plugin is at {}",
                staging.display(),
                dir.display(),
                old.display()
            )
        );
        assert_eq!(tree(&old), before);
        assert_eq!(
            names_in(root.path()),
            [format!("plugin.old-{}", std::process::id())]
        );
    }

    /// V-FillEmpty: an empty target whose fill fails is an empty directory again; an absent one
    /// stays absent.
    #[test]
    fn a_failed_fill_leaves_the_target_as_it_was() {
        let refuse = |_: &Path, _: &Path| Err(std::io::Error::other("rename refused"));
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        std::fs::create_dir(&dir).expect("the empty target");
        let prepared = prepare(&dir, false).expect("an empty target");
        let sentence = failure(install(prepared, Variant::WithMod, &refuse));
        assert!(sentence.ends_with(": rename refused"), "{sentence}");
        assert!(dir.is_dir(), "the empty directory is back");
        assert!(names_in(&dir).is_empty());
        assert_eq!(names_in(root.path()), ["plugin"]);

        std::fs::remove_dir(&dir).expect("the target removed");
        let prepared = prepare(&dir, false).expect("an absent target");
        failure(install(prepared, Variant::WithMod, &refuse));
        assert!(names_in(root.path()).is_empty());
    }

    /// The foreign refusal for `dir`, naming `why`.
    fn foreign(dir: &Path, why: &str) -> String {
        format!(
            "{} holds files htui claude emit did not write ({why}); it is left as it is. Choose \
             another --dir.",
            dir.display()
        )
    }

    /// Copies the tree at `from` to `to` (absent), files and directories.
    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("the copy's directory");
        for entry in std::fs::read_dir(from).expect("the directory reads") {
            let entry = entry.expect("the entry reads");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("the file type reads").is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).expect("the file copies");
            }
        }
    }

    /// F1: `--force` deletes nothing `emit` did not write. A copy of the source tree (its kit
    /// tests, or without them its placeholder version) and an emit holding one more file or
    /// folder are refused, and left byte for byte as they were.
    #[test]
    fn force_refuses_a_tree_holding_anything_emit_did_not_write() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        copy_tree(Path::new(SOURCE), &dir);
        let before = tree(&dir);
        assert_eq!(
            refusal(emit(&dir, true, Variant::WithMod)),
            foreign(&dir, "htui/tests")
        );
        assert_eq!(tree(&dir), before);

        std::fs::remove_dir_all(dir.join("htui/tests")).expect("the kit tests removed");
        let before = tree(&dir);
        assert_eq!(
            refusal(emit(&dir, true, Variant::WithMod)),
            foreign(&dir, "the plugin source tree, version 0.0.0-source")
        );
        assert_eq!(tree(&dir), before);
        assert_eq!(names_in(root.path()), ["plugin"]);

        type Plant<'a> = &'a dyn Fn(&Path);
        let cases: [(&str, Plant); 3] = [
            ("htui/hooks/extra.ts", &|dir| {
                std::fs::write(dir.join("htui/hooks/extra.ts"), "export {}\n")
                    .expect("the extra hooks file");
            }),
            ("htui/tests", &|dir| {
                std::fs::create_dir(dir.join("htui/tests")).expect("the extra folder");
            }),
            ("htui/skills/htui/notes.md", &|dir| {
                std::fs::write(dir.join("htui/skills/htui/notes.md"), "mine\n")
                    .expect("the extra skill file");
            }),
        ];
        for (why, plant) in cases {
            let root = tempfile::tempdir().expect("a temp dir");
            let dir = root.path().join("plugin");
            emit(&dir, false, Variant::WithMod).expect("the first emit");
            plant(&dir);
            let before = tree(&dir);
            assert_eq!(
                refusal(emit(&dir, true, Variant::SkillOnly)),
                foreign(&dir, why)
            );
            assert_eq!(tree(&dir), before, "{why}");
            assert_eq!(names_in(root.path()), ["plugin"], "{why}");
        }
    }

    /// F1: a link inside an earlier emit is foreign too, and named.
    #[cfg(unix)]
    #[test]
    fn a_link_inside_an_earlier_emit_is_refused() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        emit(&dir, false, Variant::WithMod).expect("the first emit");
        let outside = root.path().join("outside.ts");
        std::fs::write(&outside, "export {}\n").expect("the file outside");
        std::os::unix::fs::symlink(&outside, dir.join("htui/hooks/up")).expect("the link");
        let before = tree(&dir);
        assert_eq!(
            refusal(emit(&dir, true, Variant::WithMod)),
            foreign(&dir, "htui/hooks/up, a symbolic link")
        );
        assert_eq!(tree(&dir), before);
        assert_eq!(
            std::fs::read(&outside).expect("the file outside reads"),
            b"export {}\n"
        );
    }

    /// F2: a trailing separator never makes the link's target the target: `link/` is the link.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_target_with_a_trailing_slash_is_refused() {
        let root = tempfile::tempdir().expect("a temp dir");
        let real = root.path().join("real");
        emit(&real, false, Variant::WithMod).expect("the first emit");
        let link = root.path().join("plugin");
        std::os::unix::fs::symlink(&real, &link).expect("the link");
        let before = tree(&real);
        for force in [false, true] {
            assert_eq!(
                refusal(emit(&link.join(""), force, Variant::SkillOnly)),
                foreign(&link, "a symbolic link"),
                "force: {force}"
            );
        }
        assert_eq!(tree(&real), before);
        assert_eq!(names_in(root.path()), ["plugin", "real"]);
    }

    /// F2: `.` components and a trailing separator are dropped from the directory written and
    /// printed.
    #[test]
    fn dot_components_and_a_trailing_slash_are_dropped() {
        let root = tempfile::tempdir().expect("a temp dir");
        let given = root.path().join(".").join("plugin").join(".").join("");
        let report = emit(&given, false, Variant::SkillOnly).expect("the emit succeeds");
        let dir = std::path::absolute(root.path().join("plugin")).expect("an absolute path");
        assert_eq!(report.dir.as_os_str(), dir.as_os_str());
        assert_eq!(names_in(root.path()), ["plugin"]);
    }

    /// F2: a `..` is refused, not resolved, and nothing is created on either side of it.
    #[test]
    fn a_dot_dot_component_is_refused_and_creates_nothing() {
        let root = tempfile::tempdir().expect("a temp dir");
        let given = root.path().join("a").join("..").join("b");
        let dir = std::path::absolute(&given).expect("an absolute path");
        for force in [false, true] {
            assert_eq!(
                refusal(emit(&given, force, Variant::WithMod)),
                format!("{} contains ..; give the path without ..", dir.display())
            );
        }
        assert!(names_in(root.path()).is_empty());
    }

    /// F3: Claude Code takes the marketplace path as text, so a path that is not UTF-8 is refused
    /// before anything is created.
    #[cfg(unix)]
    #[test]
    fn a_target_that_is_not_utf8_is_refused() {
        use std::os::unix::ffi::OsStrExt as _;

        let root = tempfile::tempdir().expect("a temp dir");
        let given = root
            .path()
            .join(std::ffi::OsStr::from_bytes(b"plug\xffins"))
            .join("plugin");
        assert_eq!(
            refusal(emit(&given, false, Variant::WithMod)),
            format!(
                "{} is not valid UTF-8; Claude Code cannot add it",
                given.display()
            )
        );
        assert_eq!(
            std::fs::read_dir(root.path())
                .expect("the directory reads")
                .count(),
            0
        );
    }

    /// R1c (TOCTOU): an earlier emit that turns foreign between the check and the swap is
    /// refused with the foreign sentence, and the file dropped into it is left as it is.
    #[test]
    fn a_target_turned_foreign_after_the_check_is_refused_and_left_untouched() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        emit(&dir, false, Variant::WithMod).expect("the first emit");
        let prepared = prepare(&dir, true).expect("an earlier emit with --force");
        std::fs::write(dir.join("notes.txt"), "mine\n").expect("the foreign file");
        let before = tree(&dir);
        assert_eq!(
            refusal(install(prepared, Variant::SkillOnly, &|from, to| {
                std::fs::rename(from, to)
            })),
            foreign(&dir, "notes.txt")
        );
        assert_eq!(tree(&dir), before);
        assert_eq!(names_in(root.path()), ["plugin"]);
    }

    /// R1c: an empty target that is no longer empty when `emit` removes it gets its own
    /// sentence, and what was dropped into it stays.
    #[test]
    fn a_failed_removal_of_the_empty_target_names_it() {
        let root = tempfile::tempdir().expect("a temp dir");
        let dir = root.path().join("plugin");
        std::fs::create_dir(&dir).expect("the empty target");
        let prepared = prepare(&dir, false).expect("an empty target");
        std::fs::write(dir.join("notes.txt"), "mine\n").expect("the late file");
        let sentence = failure(install(prepared, Variant::WithMod, &|from, to| {
            std::fs::rename(from, to)
        }));
        assert!(
            sentence.starts_with(&format!(
                "cannot remove the empty directory {}: ",
                dir.display()
            )),
            "{sentence}"
        );
        assert_eq!(names_in(&dir), ["notes.txt"]);
        assert_eq!(names_in(root.path()), ["plugin"]);
    }

    /// R1c: a default directory with a `..` (from `HOME` or `XDG_DATA_HOME`) is refused with its
    /// own sentence; a given `--dir` is passed on as it is, and no directory at all is refused.
    #[test]
    fn the_default_directory_with_dot_dot_gets_its_own_refusal() {
        let default = std::path::PathBuf::from("/home/u/../x/htui/claude-plugin");
        match chosen_dir(None, Some(default)) {
            Err(EmitExit::Refused(sentence)) => assert_eq!(
                sentence,
                "/home/u/../x/htui/claude-plugin (the default directory) contains ..; pass --dir \
                 PATH without .."
            ),
            other => panic!("expected a refusal, got {other:?}"),
        }
        let given = std::path::PathBuf::from("/p/../q");
        assert_eq!(
            chosen_dir(Some(given.clone()), Some("/d".into())).expect("the given dir"),
            given
        );
        assert_eq!(
            chosen_dir(None, Some("/d/x".into())).expect("the default dir"),
            Path::new("/d/x")
        );
        match chosen_dir(None, None) {
            Err(EmitExit::Refused(sentence)) => assert_eq!(
                sentence,
                "this machine has no local data directory; pass --dir PATH"
            ),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}
