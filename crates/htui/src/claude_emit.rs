//! `htui claude emit` (MOD-93, `R-ID-4`, `R-ID-6`, `R-STO-1`).

use serde_json::{Map, Value};

/// The version every emitted plugin carries: this binary's (D5).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the source tree's `plugin.json` and `hooks/version.ts` carry in place of a version (D5).
const SOURCE_VERSION: &str = "0.0.0-source";
/// `hooks/version.ts`'s placeholder, quotes included; the source holds it exactly once.
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
        EmitExit, FILES, PLUGIN_JSON, SOURCE_VERSION, VERSION, VERSION_TS, VERSION_TS_LITERAL,
        Variant, files_for,
    };

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
}
