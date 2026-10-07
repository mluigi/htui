//! Reading skill files off the disk and writing them through milestone 3's skill writers (MOD-9
//! milestone 4, import plan D98–D100): `create_skill` for a new name, `update_skill` and
//! `add_skill_version` for one the library already holds.
//!
//! **Why the worker reads the files.** `R-NF-3` puts store-touching work off the UI task, and the
//! PRD enforces it by ownership: reads and writes go through `StoreRequest` on the store worker.
//! A directory walk is unbounded work by nature, so the view types a path and the worker does the
//! rest. `std::fs` in an `async fn` is the [`crate::editor::run`] precedent, and the walk is capped
//! ([`MAX_FILES`]) so it cannot hold the worker for long (import plan R-42).
//!
//! **One read for the tokens, one for the reply.** The library is read **once**, before the batch,
//! and both compare-and-set tokens for every file come from that one read: `skill.updated_at` for
//! the description, the head version for the append. That is the order milestone 2's review
//! finding 5 settled for bindings and versions, and it is what makes the CAS honest rather than a
//! guess. A `Stale` refuses that one file and the batch continues. A name one batch meets twice is
//! refused the second time, naming the first file: two files claiming one name is a collision to
//! report, not a version chain, and the first write has already spent the read's tokens.
//!
//! **What is written.** `skill` and `skill_version` rows, and nothing else — ANA-22 §7.3:
//! "nothing is attached by import". The frontmatter's activation keys ride along in
//! `skill_version.source`, and the attachments pane reads them back when the skill is attached.
//!
//! **The clock.** [`crate::skills`] reads no clock, because the store stamps every instant. This
//! module reads one, once per batch, for `source.imported_at` — the provenance the import itself
//! writes. Every row instant still comes from the store.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use htui_core::model::skill_import::{ParsedSkill, SKILL_FILE, parse};
use htui_core::model::{NewSkill, NewSkillVersion, SkillId, SkillPatch, UserId};
use htui_core::store::{CasOutcome, Result, StoreError, WriteStore, skill_body_refusal};
use htui_store::{Backend, DATABASE_UNREACHABLE};

use crate::skills::{SkillEntry, SkillsSnapshot};

/// How deep below the named directory a `SKILL.md` may sit and still be a candidate. Four is the
/// `*/SKILL.md` shape with room for `.claude/skills/<name>/SKILL.md` beneath it.
pub const MAX_DEPTH: usize = 4;

/// The largest file read as a skill. A file over this is reported as skipped, not truncated.
pub const MAX_BYTES: u64 = 256 * 1024;

/// The most candidate files one path may yield. Past it, the rest are reported as skipped: a cap
/// that stops silently would be a cap the maintainer cannot see.
pub const MAX_FILES: usize = 64;

/// The directories a skill ships with and that are therefore not skills themselves (ANA-22 §6
/// item 12), plus the two every walk prunes. Each prune is a reported row, never a silent one.
pub const SKIP_DIRS: [&str; 5] = ["scripts", "references", "assets", ".git", "node_modules"];

/// The directory names §8 means by "a rules directory": the folders the surveyed tools keep their
/// rule files in (`.cursor/rules`, `.github/instructions`, `.kiro/steering`).
///
/// This is the narrowing import plan R-37 asked for. The alternative — every `*.md` at depth 1 of
/// whatever directory was named — turns a project root into a skill sweep, and `README.md` is a
/// skill with the stem `readme`.
pub const RULES_DIR_NAMES: [&str; 3] = ["rules", "instructions", "steering"];

/// What happened to one file the maintainer named, or one the walk found under it.
///
/// Every outcome is reported, including the ones that wrote nothing: §9's mitigation for a reader
/// that rejects a real file is that it is *reported*, and a batch that says only what it imported
/// is a batch that hides a refusal. No variant carries file content — a path and a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    /// A new skill, at version 1.
    Imported {
        /// `skill.name`.
        name: String,
        /// The file it came from, as the maintainer sees it.
        path: String,
        /// The version written, always 1.
        version: i32,
    },
    /// A skill the library already held: a new version, and the description moved with it.
    Updated {
        /// `skill.name`.
        name: String,
        /// The file it came from.
        path: String,
        /// The version appended.
        version: i32,
    },
    /// A skill the library already held, whose body is byte-identical to its head's. Nothing was
    /// appended (ANA-22 §6 item 12); the description was still refreshed.
    Unchanged {
        /// `skill.name`.
        name: String,
        /// The file it came from.
        path: String,
    },
    /// The file was not imported, and this is why — the reader's sentence, the writer's sentence,
    /// or the CAS's.
    Refused {
        /// The file, or the directory, the maintainer named.
        path: String,
        /// One sentence, already the one the store or the reader would have said.
        message: String,
    },
    /// The file was left alone, and this is why.
    Skipped {
        /// The file, or the directory, that was left alone.
        path: String,
        /// One sentence: a bundled directory, a file over the cap, a file that is not UTF-8.
        reason: String,
    },
}

/// The import's answer: the library as it is after the batch, and what happened to every file.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillImports {
    /// A fresh read, so the view renders from the store rather than patching a row in.
    pub snapshot: SkillsSnapshot,
    /// One row per file, in the order the maintainer named them and the walk found them.
    pub report: Vec<ImportOutcome>,
}

/// Imports every path the maintainer typed, and reports on every file.
///
/// A path may name a file — imported whatever it is called — or a directory, which contributes a
/// `SKILL.md` at any depth up to [`MAX_DEPTH`], and `*.md` / `*.mdc` from the rules directory its
/// [`RootShape`] names (import plan D99). One bad file never stops the batch.
///
/// # Errors
/// Whatever the store reports for the library read or the user lookup, or
/// [`StoreError::Unreachable`] with [`DATABASE_UNREACHABLE`] offline. A path that does not exist
/// is a reported [`ImportOutcome::Refused`], not an error: the other paths still import.
pub async fn import(backend: &Backend, paths: &[String]) -> Result<Vec<ImportOutcome>> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
    let created_by = backend.this_user().await?;

    // One read for the tokens (import plan D100) and one clock for the provenance.
    let mut library = Vec::new();
    for skill in writer.skills().await? {
        let versions = writer.skill_versions(skill.id).await?;
        library.push(SkillEntry { skill, versions });
    }
    let mut batch = Batch {
        writer: &writer,
        created_by,
        library: &library,
        now: Utc::now(),
        written: Vec::new(),
    };

    let mut report: Vec<ImportOutcome> = Vec::new();
    for path in paths {
        report.extend(batch.one_path(path).await);
    }
    Ok(report)
}

/// One import's shared state: the writer, the one library read, the clock, and the names this
/// batch has already written.
struct Batch<'a, W> {
    /// The seam every write goes through.
    writer: &'a W,
    /// `created_by` for every row, looked up once.
    created_by: UserId,
    /// The library as read before the first write: the source of both CAS tokens.
    library: &'a [SkillEntry],
    /// `source.imported_at`, shared by the whole batch.
    now: DateTime<Utc>,
    /// `(name, path)` of every name this batch has already imported, updated or left unchanged.
    written: Vec<(String, String)>,
}

impl<W: WriteStore> Batch<'_, W> {
    /// The files one typed path contributes, and the ones it leaves alone.
    async fn one_path(&mut self, path: &str) -> Vec<ImportOutcome> {
        let root = PathBuf::from(path);
        let metadata = match std::fs::metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) => {
                return vec![ImportOutcome::Refused {
                    path: path.to_owned(),
                    message: format!("could not read `{path}` ({error})"),
                }];
            }
        };
        if metadata.is_file() {
            return vec![self.write_one(&root, path).await];
        }
        if !metadata.is_dir() {
            return vec![ImportOutcome::Refused {
                path: path.to_owned(),
                message: format!("`{path}` is neither a file nor a directory"),
            }];
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut report: Vec<ImportOutcome> = Vec::new();
        walk(&root, &mut candidates, &mut report);
        if candidates.len() > MAX_FILES {
            let dropped = candidates.len() - MAX_FILES;
            candidates.truncate(MAX_FILES);
            report.push(ImportOutcome::Skipped {
                path: path.to_owned(),
                reason: format!("{dropped} more file(s) past the {MAX_FILES}-file cap"),
            });
        }
        for candidate in &candidates {
            let label = candidate.display().to_string();
            report.push(self.write_one(candidate, &label).await);
        }
        report
    }

    /// Reads one file and writes it, or says why it was not written.
    async fn write_one(&mut self, path: &Path, label: &str) -> ImportOutcome {
        let text = match read_text(path) {
            Ok(text) => text,
            Err(message) => {
                return ImportOutcome::Skipped {
                    path: label.to_owned(),
                    reason: message,
                };
            }
        };
        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let parsed = match parse(label, &file_name, &text, self.now) {
            Ok(parsed) => parsed,
            Err(message) => {
                return ImportOutcome::Refused {
                    path: label.to_owned(),
                    message,
                };
            }
        };
        if let Some((_, first)) = self.written.iter().find(|(name, _)| *name == parsed.name) {
            return ImportOutcome::Refused {
                path: label.to_owned(),
                message: format!(
                    "`{}` was already imported from `{first}` by this import; this file was not \
                     written",
                    parsed.name
                ),
            };
        }
        let name = parsed.name.clone();
        let outcome = self.write_skill(parsed, label).await;
        if matches!(
            outcome,
            ImportOutcome::Imported { .. }
                | ImportOutcome::Updated { .. }
                | ImportOutcome::Unchanged { .. }
        ) {
            self.written.push((name, label.to_owned()));
        }
        outcome
    }

    /// The per-file write (import plan D100), with both tokens from the read the batch took before
    /// it started.
    ///
    /// A new name is `create_skill`: the row and its version 1 together, so no skill exists without
    /// a body. A known name is `update_skill` when the description moved (§6 item 12), then
    /// `add_skill_version` when the body did. No attachment is written.
    async fn write_skill(&self, parsed: ParsedSkill, label: &str) -> ImportOutcome {
        let refused = |message: String| ImportOutcome::Refused {
            path: label.to_owned(),
            message,
        };
        let ParsedSkill {
            name,
            description,
            body,
            source,
            ..
        } = parsed;
        let Some(existing) = self.library.iter().find(|entry| entry.skill.name == name) else {
            let new = NewSkill {
                id: SkillId::new(),
                name: name.clone(),
                description,
                body,
                source,
                created_by: self.created_by,
            };
            return match self.writer.create_skill(new).await {
                Ok((_, version)) => ImportOutcome::Imported {
                    name,
                    path: label.to_owned(),
                    version: version.version,
                },
                Err(error) => refused(sentence(error)),
            };
        };

        let skill = existing.skill.id;
        // Refuse a body the version append would refuse before anything is written, so a refused
        // file never leaves a moved description behind it.
        if let Some(refusal) = skill_body_refusal(&body) {
            return refused(refusal);
        }
        let described = existing.skill.description != description;
        if described {
            let patch = SkillPatch {
                name: None,
                description: Some(description),
            };
            match self
                .writer
                .update_skill(skill, existing.skill.updated_at, patch)
                .await
            {
                Ok(CasOutcome::Applied(_)) => {}
                Ok(CasOutcome::Stale(_)) | Err(StoreError::NotFound { .. }) => {
                    return refused(format!(
                        "`{name}` changed while this import was running; nothing was written for \
                         this file, and re-running the import picks it up"
                    ));
                }
                Err(error) => return refused(sentence(error)),
            }
        }

        // A skill with no version at all (a hand-written row) appends over `0` (D89).
        let head = existing.versions.iter().max_by_key(|row| row.version);
        if head.is_some_and(|head| head.body == body) {
            return ImportOutcome::Unchanged {
                name,
                path: label.to_owned(),
            };
        }
        let expected = head.map_or(0, |head| head.version);
        let new = NewSkillVersion {
            body,
            source,
            created_by: self.created_by,
        };
        match self.writer.add_skill_version(skill, expected, new).await {
            Ok(CasOutcome::Applied(version)) => ImportOutcome::Updated {
                name,
                path: label.to_owned(),
                version: version.version,
            },
            Ok(CasOutcome::Stale(_)) | Err(StoreError::NotFound { .. }) => {
                let description = if described {
                    "its description was updated and "
                } else {
                    ""
                };
                refused(format!(
                    "`{name}` gained a version while this import was running; {description}no \
                     version was appended, so re-running the import finishes it"
                ))
            }
            Err(error) => refused(sentence(error)),
        }
    }
}

/// A writer's refusal as the one sentence the report shows: a `Constraint`'s own sentence (the
/// refusal helpers' words, which the Skills view already shows), any other error as it prints.
fn sentence(error: StoreError) -> String {
    match error {
        StoreError::Constraint(sentence) => sentence,
        other => other.to_string(),
    }
}

/// What a named directory is, which decides where its markdown is collected from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootShape {
    /// Named for what it holds (`rules`, `instructions`, `steering`): its own `*.md` / `*.mdc`.
    Rules,
    /// A hidden tool root such as `.cursor`, `.github` or `.kiro` holding a rules-named child:
    /// the `*.md` / `*.mdc` of **that child**, never the root's own. `.cursor/rules/*.mdc` are
    /// rules files; `.cursor/*.md`, `.github/copilot-instructions.md` and a pull-request template
    /// are not.
    ToolRoot,
    /// Anything else: `SKILL.md` files only.
    Plain,
}

/// The shape of the directory the maintainer named.
///
/// The tool-root clause needs the rules-named child rather than a leading dot on its own, because
/// "hidden" is not what makes a directory a rules directory — `/home/x/.config` is hidden and is
/// not one, and a temporary directory is hidden too.
#[must_use]
pub fn root_shape(dir: &Path) -> RootShape {
    let Some(name) = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return RootShape::Plain;
    };
    if RULES_DIR_NAMES.contains(&name.as_str()) {
        return RootShape::Rules;
    }
    if !name.starts_with('.') {
        return RootShape::Plain;
    }
    let has_rules_child = std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(std::result::Result::ok).any(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|child| RULES_DIR_NAMES.contains(&child))
        })
    });
    if has_rules_child {
        RootShape::ToolRoot
    } else {
        RootShape::Plain
    }
}

/// The candidates under `root`, by its [`root_shape`], with every prune reported.
fn walk(root: &Path, out: &mut Vec<PathBuf>, skipped: &mut Vec<ImportOutcome>) {
    let shape = root_shape(root);
    collect(root, 0, shape == RootShape::Rules, shape, out, skipped);
}

/// Walks `dir` for skill files, sorted, with every prune reported. `markdown` is whether `dir`'s
/// own `*.md` / `*.mdc` are candidates; a `SKILL.md` always is.
///
/// The sort is the point: `read_dir` order is the filesystem's, and two machines would otherwise
/// disagree about which file is the first one to be refused. Byte order at every level is
/// `htui_agent::excerpt::FsRepoReader::walk`'s rule, and the reason this workspace treats it as the
/// canonical walk.
fn collect(
    dir: &Path,
    depth: usize,
    markdown: bool,
    shape: RootShape,
    out: &mut Vec<PathBuf>,
    skipped: &mut Vec<ImportOutcome>,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<PathBuf> = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .collect();
    names.sort_by(|left, right| {
        left.file_name()
            .unwrap_or_default()
            .as_encoded_bytes()
            .cmp(right.file_name().unwrap_or_default().as_encoded_bytes())
    });

    for entry in names {
        let name = entry
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        // A symlink is not descended: a directory link can leave the tree the maintainer named.
        let Ok(metadata) = std::fs::symlink_metadata(&entry) else {
            continue;
        };
        if metadata.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                skipped.push(ImportOutcome::Skipped {
                    path: entry.display().to_string(),
                    reason: format!("`{name}` is a bundled skill directory, not a skill"),
                });
                continue;
            }
            // A tool root's rules files live one level down, in its rules-named child, and only
            // there: the root's own markdown is not a rules file.
            let child_markdown = shape == RootShape::ToolRoot
                && depth == 0
                && RULES_DIR_NAMES.contains(&name.as_str());
            collect(&entry, depth + 1, child_markdown, shape, out, skipped);
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        if name == SKILL_FILE
            || (markdown
                && Path::new(&name)
                    .extension()
                    .is_some_and(|extension| extension == "md" || extension == "mdc"))
        {
            out.push(entry);
        }
    }
}

/// Reads a file as text, refusing in one sentence each — the two sentences
/// [`crate::editor::run`] already uses, because they are the ones a maintainer has read before.
fn read_text(path: &Path) -> std::result::Result<String, String> {
    // The cap is asked of the file **before** it is read, not after: a refusal that has already
    // pulled two gigabytes into memory is not a refusal.
    let size = std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| format!("could not read this file ({error}); nothing was imported"))?;
    if size > MAX_BYTES {
        return Err(format!(
            "the file is over the {MAX_BYTES}-byte cap; nothing was imported"
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read this file ({error}); nothing was imported"))?;
    String::from_utf8(bytes).map_err(|_| "the file is not UTF-8; nothing was imported".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        ImportOutcome, MAX_BYTES, MAX_FILES, RULES_DIR_NAMES, RootShape, SKILL_FILE, SKIP_DIRS,
        import, read_text, root_shape, walk,
    };
    use crate::skills::SkillEntry;
    use htui_core::store::{MemStore, WriteStore as _};
    use htui_store::Backend;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A tree with the two shapes import plan D99 collects and the five directories it prunes.
    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        for file in [
            "one/SKILL.md",
            "two/nested/SKILL.md",
            "rules.mdc",
            "README.md",
            "docs/deep.md",
        ] {
            let path = root.join(file);
            fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            fs::write(&path, "x").expect("write");
        }
        for skip in SKIP_DIRS {
            fs::create_dir_all(root.join(skip).join("inner")).expect("mkdir");
            fs::write(root.join(skip).join("inner/SKILL.md"), "x").expect("write");
        }
        dir
    }

    fn found(root: &Path) -> (Vec<PathBuf>, Vec<ImportOutcome>) {
        let mut out = Vec::new();
        let mut skipped = Vec::new();
        walk(root, &mut out, &mut skipped);
        (out, skipped)
    }

    /// The candidates under `root`, relative to it, `/`-joined.
    fn relative(root: &Path) -> Vec<String> {
        found(root)
            .0
            .iter()
            .map(|path| {
                path.strip_prefix(root)
                    .expect("under the root")
                    .display()
                    .to_string()
            })
            .collect()
    }

    fn plant(root: &Path, files: &[&str]) {
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            fs::write(&path, "x").expect("write");
        }
    }

    #[test]
    fn a_directory_yields_the_skill_shapes_and_nothing_else() {
        let dir = tree();
        let (_, skipped) = found(dir.path());
        assert_eq!(
            relative(dir.path()),
            ["one/SKILL.md", "two/nested/SKILL.md"],
            "`*/SKILL.md` at any depth, and nothing else: a project root is not a rules directory, \
             so `README.md` and `rules.mdc` at its top level are not swept (import plan R-37)"
        );
        // The five bundled directories this fixture plants are pruned and listed, which
        // `the_bundled_directories_are_skipped_and_listed` pins exactly.
        assert_eq!(skipped.len(), SKIP_DIRS.len(), "{skipped:?}");
    }

    /// §8's "`*.md`/`*.mdc` under a rules directory" is a condition on the directory, and this is
    /// what makes it load-bearing rather than an assumption that any directory is one.
    #[test]
    fn a_rules_directory_also_yields_its_markdown() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let rules = dir.path().join("rules");
        plant(&rules, &["workers.mdc", "style.md", "notes.txt"]);
        assert_eq!(
            relative(&rules),
            ["style.md", "workers.mdc"],
            "`.md` and `.mdc`, not `.txt`"
        );

        for name in RULES_DIR_NAMES {
            assert_eq!(
                root_shape(Path::new("/x").join(name).as_path()),
                RootShape::Rules,
                "{name}"
            );
        }
        assert_eq!(
            root_shape(Path::new("/home/mluigi/projects/htui")),
            RootShape::Plain,
            "a project root is not"
        );
        // A hidden directory with nothing rules-shaped under it is not a tool root, and a
        // temporary directory is hidden — which is how a leading dot on its own was caught being
        // too loose a test.
        let plain = tempfile::tempdir().expect("a temp dir");
        assert_eq!(
            root_shape(plain.path()),
            RootShape::Plain,
            "a temporary directory is hidden and is not a tool root"
        );
        let bare = dir.path().join(".config");
        fs::create_dir_all(&bare).expect("mkdir");
        assert_eq!(
            root_shape(&bare),
            RootShape::Plain,
            "a hidden directory with no rules child is not"
        );
        // A *file* named `rules` does not make a tool root either.
        let fake = dir.path().join(".fake");
        plant(&fake, &["rules"]);
        assert_eq!(root_shape(&fake), RootShape::Plain);
    }

    /// The review finding against PR #10: a hidden tool root counted as a rules directory, so its
    /// own depth-0 markdown was collected (`.cursor/*.md`, `.github/copilot-instructions.md`, a
    /// pull-request template) while the rules files one level down (`.cursor/rules/*.mdc`) were
    /// not. A tool root's markdown is its rules-named child's, and only that.
    #[test]
    fn a_hidden_tool_root_collects_its_rules_child_and_not_its_own_markdown() {
        let dir = tempfile::tempdir().expect("a temp dir");

        let cursor = dir.path().join(".cursor");
        plant(
            &cursor,
            &[
                "rules/api_style.mdc",
                "rules/workers.mdc",
                "rules/nested/deeper.mdc",
                "stray.md",
                "notes/other.md",
            ],
        );
        assert_eq!(root_shape(&cursor), RootShape::ToolRoot);
        assert_eq!(
            relative(&cursor),
            ["rules/api_style.mdc", "rules/workers.mdc"],
            "the rules child's own files; not the root's `stray.md`, not a grandchild's"
        );

        let github = dir.path().join(".github");
        plant(
            &github,
            &[
                "copilot-instructions.md",
                "PULL_REQUEST_TEMPLATE.md",
                "ISSUE_TEMPLATE/bug.md",
                "instructions/rust.instructions.md",
                "workflows/ci.yml",
            ],
        );
        assert_eq!(
            relative(&github),
            ["instructions/rust.instructions.md"],
            "Copilot's path-specific instructions, and none of the repository's own markdown"
        );

        let kiro = dir.path().join(".kiro");
        plant(
            &kiro,
            &["steering/product.md", "specs/feature/requirements.md"],
        );
        assert_eq!(relative(&kiro), ["steering/product.md"]);

        // A `SKILL.md` anywhere under a tool root is still a skill.
        plant(&cursor, &["skills/house/SKILL.md"]);
        assert!(
            relative(&cursor).contains(&"skills/house/SKILL.md".to_owned()),
            "{:?}",
            relative(&cursor)
        );
    }

    #[test]
    fn the_walk_is_byte_ordered_and_twice_the_same() {
        let dir = tempfile::tempdir().expect("a temp dir");
        plant(
            dir.path(),
            &["b/SKILL.md", "a/SKILL.md", "c/SKILL.md", "A/SKILL.md"],
        );
        let (first, _) = found(dir.path());
        let (second, _) = found(dir.path());
        assert_eq!(first, second, "the same tree twice is the same order");
        assert_eq!(
            relative(dir.path()),
            ["A/SKILL.md", "a/SKILL.md", "b/SKILL.md", "c/SKILL.md"],
            "byte order, so `A` is not `a` by accident"
        );
    }

    #[test]
    fn the_bundled_directories_are_skipped_and_listed() {
        let dir = tree();
        let (candidates, skipped) = found(dir.path());
        let skipped_names: Vec<String> = skipped
            .iter()
            .map(|outcome| match outcome {
                ImportOutcome::Skipped { path, .. } => path.clone(),
                other => panic!("a directory prune is a Skipped row, not {other:?}"),
            })
            .collect();
        assert_eq!(skipped_names.len(), SKIP_DIRS.len());
        for skip in SKIP_DIRS {
            assert!(
                skipped_names.iter().any(|path| path.ends_with(skip)),
                "{skip} is listed as skipped: {skipped_names:?}"
            );
        }
        assert!(
            candidates
                .iter()
                .all(|path| !path.to_string_lossy().contains("scripts")),
            "nothing under a bundled directory is a candidate"
        );
    }

    // `std::os::unix::fs::symlink`: a Windows symlink needs a privilege the test box may lack.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_is_not_descended() {
        let dir = tree();
        let elsewhere = tempfile::tempdir().expect("a temp dir");
        plant(elsewhere.path(), &["inner/SKILL.md"]);
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("link")).expect("symlink");

        let (candidates, _) = found(dir.path());
        assert!(
            candidates
                .iter()
                .all(|path| !path.to_string_lossy().contains("link")),
            "a directory link can leave the tree the maintainer named: {candidates:?}"
        );
    }

    // --- the write path, end to end over the demo store -----------------------------------------

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    /// The library as the writer reads it.
    async fn library(backend: &Backend) -> Vec<SkillEntry> {
        let writer = backend.writer().expect("the demo store writes");
        let mut out = Vec::new();
        for skill in writer.skills().await.expect("readable") {
            let versions = writer.skill_versions(skill.id).await.expect("readable");
            out.push(SkillEntry { skill, versions });
        }
        out
    }

    async fn entry(backend: &Backend, name: &str) -> Option<SkillEntry> {
        library(backend)
            .await
            .into_iter()
            .find(|entry| entry.skill.name == name)
    }

    /// One `SKILL.md` under `dir/<dir_name>`, declaring `name`, returning its path.
    fn skill_at(dir: &Path, dir_name: &str, name: &str, body: &str) -> PathBuf {
        let nested = dir.join(dir_name);
        fs::create_dir_all(&nested).expect("mkdir");
        let path = nested.join(SKILL_FILE);
        fs::write(
            &path,
            format!("---\nname: {name}\ndescription: The {name} skill.\n---\n{body}"),
        )
        .expect("write");
        path
    }

    fn skill_file(dir: &Path, name: &str, body: &str) -> PathBuf {
        skill_at(dir, name, name, body)
    }

    async fn run(backend: &Backend, paths: &[String]) -> Vec<ImportOutcome> {
        import(backend, paths).await.expect("the import answers")
    }

    /// A new name creates the skill at version 1 with a non-empty `source` — the first writer of
    /// one in the tree, and the column the attachments pane reads back.
    #[tokio::test]
    async fn a_first_import_creates_the_skill_at_version_one_with_its_source() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = skill_file(dir.path(), "house-rules", "# Rules\n");

        let report = run(&backend, &[path.display().to_string()]).await;

        assert_eq!(report.len(), 1, "{report:?}");
        assert!(
            matches!(&report[0], ImportOutcome::Imported { name, version: 1, .. } if name == "house-rules"),
            "{report:?}"
        );
        let entry = entry(&backend, "house-rules")
            .await
            .expect("the skill is in the library");
        assert_eq!(entry.skill.description, "The house-rules skill.");
        let head = entry.versions.last().expect("a version");
        assert_eq!(head.version, 1);
        assert_eq!(head.body, "# Rules\n");
        assert_eq!(head.source["format"], serde_json::json!("skill-md"));
        assert_eq!(
            head.source["path"],
            serde_json::json!(path.display().to_string())
        );
        assert!(
            head.source["frontmatter"]["description"].is_string(),
            "the whole frontmatter is kept: {}",
            head.source
        );
        assert!(
            head.source["imported_at"].is_string(),
            "and the instant it was imported"
        );
    }

    /// ANA-22 §7.3: "nothing is attached by import". The attachments are exactly what they were.
    #[tokio::test]
    async fn an_import_writes_no_attachment() {
        let backend = demo();
        let writer = backend.writer().expect("writes");
        let before = writer.skill_bindings(None).await.expect("readable");
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("rules").join("always.mdc");
        fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        fs::write(&path, "---\nalwaysApply: true\nglobs: src/**\n---\nBody\n").expect("write");

        let report = run(&backend, &[dir.path().join("rules").display().to_string()]).await;

        assert!(
            matches!(&report[..], [ImportOutcome::Imported { name, .. }] if name == "always"),
            "{report:?}"
        );
        assert_eq!(
            writer.skill_bindings(None).await.expect("readable"),
            before,
            "the activation keys are prefill, not state"
        );
    }

    /// §6 item 12's same-name rule, both halves: a changed body appends and moves the description;
    /// an identical body writes no version at all.
    #[tokio::test]
    async fn a_same_name_import_appends_only_when_the_body_differs() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = skill_file(dir.path(), "house-rules", "# Rules\n");
        let label = path.display().to_string();
        run(&backend, std::slice::from_ref(&label)).await;

        // The same file again: nothing to append.
        let report = run(&backend, std::slice::from_ref(&label)).await;
        assert!(
            matches!(&report[0], ImportOutcome::Unchanged { name, .. } if name == "house-rules"),
            "{report:?}"
        );
        assert_eq!(
            entry(&backend, "house-rules")
                .await
                .expect("present")
                .versions
                .len(),
            1,
            "an identical body appends nothing"
        );

        // A changed body, and a changed description, at the same name.
        fs::write(
            &path,
            "---\nname: house-rules\ndescription: The house rules, revised.\n---\n# Rules v2\n",
        )
        .expect("write");
        let report = run(&backend, &[label]).await;
        assert!(
            matches!(&report[0], ImportOutcome::Updated { version: 2, .. }),
            "{report:?}"
        );
        let entry = entry(&backend, "house-rules").await.expect("present");
        assert_eq!(entry.versions.len(), 2, "a changed body appends v2");
        assert_eq!(
            entry.skill.description, "The house rules, revised.",
            "and the description moves"
        );
    }

    /// A name the batch has already written is refused the second time, naming the first file —
    /// not a spurious "changed while this import was running", which is what the one read's spent
    /// token answered before this was caught in review.
    #[tokio::test]
    async fn a_name_repeated_in_one_import_is_refused_naming_the_first_file() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let first = skill_at(dir.path(), "a", "twice", "# First\n");
        skill_at(dir.path(), "b", "twice", "# Second\n");

        let report = run(&backend, &[dir.path().display().to_string()]).await;

        assert!(
            matches!(&report[0], ImportOutcome::Imported { name, .. } if name == "twice"),
            "{report:?}"
        );
        let ImportOutcome::Refused { message, .. } = &report[1] else {
            panic!("the repeat is refused: {report:?}")
        };
        assert!(
            message.contains("already imported from")
                && message.contains(&first.display().to_string()),
            "{message}"
        );
        assert!(
            !message.contains("while this import was running"),
            "{message}"
        );
        let entry = entry(&backend, "twice").await.expect("present");
        assert_eq!(entry.versions.len(), 1, "the second file wrote nothing");
        assert_eq!(entry.versions[0].body, "# First\n");
    }

    /// A name the writer would refuse is refused here too, in the writer's own words, and nothing
    /// is written — which is import plan OQ-22's default: refuse the file, slugify nothing.
    #[tokio::test]
    async fn a_refused_name_writes_nothing_and_says_the_writers_own_sentence() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("My Skill.md");
        fs::write(&path, "---\nname: My Skill\ndescription: d\n---\nB\n").expect("write");
        let before = library(&backend).await.len();

        let report = run(&backend, &[path.display().to_string()]).await;

        let ImportOutcome::Refused { message, .. } = &report[0] else {
            panic!("a refused name is a Refused row, got {report:?}")
        };
        assert_eq!(message, &htui_core::store::invalid_skill_name("My Skill"));
        assert_eq!(library(&backend).await.len(), before, "nothing was written");
    }

    /// A blank body is the writer's refusal, in the writer's own sentence (D77).
    #[tokio::test]
    async fn a_blank_body_is_refused_in_the_writers_sentence() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = skill_file(dir.path(), "empty-skill", "\n");

        let report = run(&backend, &[path.display().to_string()]).await;

        assert_eq!(
            report,
            [ImportOutcome::Refused {
                path: path.display().to_string(),
                message: htui_core::store::BLANK_SKILL_BODY.to_owned(),
            }]
        );
        assert!(entry(&backend, "empty-skill").await.is_none());
    }

    /// A re-import whose body the append would refuse writes nothing at all, not even the new
    /// description it carries: the refusal is asked before `update_skill`, so a `Refused` row
    /// really means the file was not imported.
    #[tokio::test]
    async fn a_refused_body_on_re_import_leaves_the_description_alone() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = skill_file(dir.path(), "house-rules", "# Rules\n");
        let label = path.display().to_string();
        run(&backend, std::slice::from_ref(&label)).await;
        let before = entry(&backend, "house-rules").await.expect("present");

        fs::write(
            &path,
            "---\nname: house-rules\ndescription: A new description.\n---\n\n",
        )
        .expect("write");
        let report = run(&backend, std::slice::from_ref(&label)).await;

        assert_eq!(
            report,
            [ImportOutcome::Refused {
                path: label,
                message: htui_core::store::BLANK_SKILL_BODY.to_owned(),
            }]
        );
        let after = entry(&backend, "house-rules").await.expect("present");
        assert_eq!(after.skill.description, before.skill.description);
        assert_eq!(after.skill.updated_at, before.skill.updated_at);
        assert_eq!(after.versions.len(), 1);
    }

    /// A path that is not there is a reported refusal, not an error: the other paths still import.
    #[tokio::test]
    async fn a_missing_path_is_refused_and_the_rest_of_the_batch_still_imports() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = skill_file(dir.path(), "second-skill", "# S\n");

        let report = run(
            &backend,
            &[
                dir.path().join("nope").display().to_string(),
                path.display().to_string(),
            ],
        )
        .await;

        assert_eq!(report.len(), 2, "{report:?}");
        assert!(
            matches!(&report[0], ImportOutcome::Refused { .. }),
            "{report:?}"
        );
        assert!(
            matches!(&report[1], ImportOutcome::Imported { .. }),
            "{report:?}"
        );
    }

    /// A directory import takes the `*/SKILL.md` shape, and the bundled directories are listed
    /// rather than silently pruned (ANA-22 §6 item 12).
    #[tokio::test]
    async fn a_directory_import_skips_the_bundled_directories_and_lists_them() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        skill_file(dir.path(), "one", "# One\n");
        skill_file(dir.path(), "two", "# Two\n");
        plant(dir.path(), &["one/scripts/helper.sh"]);

        let report = run(&backend, &[dir.path().display().to_string()]).await;

        let imported: Vec<&str> = report
            .iter()
            .filter_map(|outcome| match outcome {
                ImportOutcome::Imported { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(imported, ["one", "two"], "{report:?}");
        assert!(
            report.iter().any(|outcome| matches!(
                outcome,
                ImportOutcome::Skipped { path, .. } if path.ends_with("scripts")
            )),
            "the bundled directory is listed: {report:?}"
        );
    }

    /// A tree past the cap imports [`MAX_FILES`] and lists the rest in one `Skipped` row: a cap
    /// that stops silently is not a cap.
    #[tokio::test]
    async fn a_tree_past_the_file_cap_imports_the_cap_and_reports_the_rest() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        for index in 0..=MAX_FILES {
            skill_file(dir.path(), &format!("capped-{index:03}"), "# C\n");
        }

        let report = run(&backend, &[dir.path().display().to_string()]).await;

        let imported = report
            .iter()
            .filter(|outcome| matches!(outcome, ImportOutcome::Imported { .. }))
            .count();
        assert_eq!(imported, MAX_FILES, "{report:?}");
        assert!(
            report.iter().any(|outcome| matches!(
                outcome,
                ImportOutcome::Skipped { reason, .. } if reason.starts_with("1 more file")
            )),
            "{report:?}"
        );
    }

    /// The report never carries file **content** — a path and a sentence, never a body. This is
    /// the redaction rule the `CreateSkill` body follows, applied to the other direction.
    #[tokio::test]
    async fn the_report_prints_no_file_content() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = skill_file(dir.path(), "secretive", "the launch codes are 12345\n");

        let report = run(&backend, &[path.display().to_string()]).await;
        let shown = format!("{report:?}");

        assert!(
            !shown.contains("12345"),
            "no file content reaches a Debug sink: {shown}"
        );
        assert!(
            shown.contains("secretive"),
            "but the name and the path do: {shown}"
        );
    }

    /// The cap is asked of the file before it is read. A file one byte over is refused, and the
    /// refusal names the cap rather than the read.
    #[test]
    fn a_file_over_the_cap_is_refused_and_a_file_under_it_is_not() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let small = dir.path().join("small.md");
        fs::write(&small, vec![b'x'; 8]).expect("write");
        assert!(read_text(&small).is_ok(), "a file under the cap reads");

        let big = dir.path().join("big.md");
        fs::write(
            &big,
            vec![b'x'; usize::try_from(MAX_BYTES).expect("fits") + 1],
        )
        .expect("write");
        let refusal = read_text(&big).expect_err("one byte over the cap is refused");
        assert!(
            refusal.contains(&format!("{MAX_BYTES}-byte cap")),
            "the refusal names the cap, not a read error: {refusal}"
        );
    }

    #[test]
    fn a_missing_file_is_refused_in_the_readers_own_sentence() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let refusal = read_text(&dir.path().join("gone.md")).expect_err("no such file");
        assert!(refusal.contains("could not read this file"), "{refusal}");
    }

    #[test]
    fn a_file_that_is_not_utf8_is_refused() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("binary.md");
        fs::write(&path, [0xff, 0xfe, 0x00]).expect("write");
        let refusal = read_text(&path).expect_err("not UTF-8");
        assert!(refusal.contains("not UTF-8"), "{refusal}");
    }
}
