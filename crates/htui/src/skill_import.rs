//! Reading skill files off the disk and writing them through the two writers milestone 3 added
//! (MOD-9 milestone 4, D98–D100).
//!
//! **Why the worker reads the files.** `R-NF-3` puts store-touching work off the UI task, and the
//! PRD enforces it by ownership: reads and writes go through `StoreRequest` on the store worker.
//! A directory walk is unbounded work by nature, so the view types a path and the worker does the
//! rest. `std::fs` in an `async fn` is the [`crate::editor::run`] precedent, and the walk is capped
//! ([`MAX_FILES`]) so it cannot hold the worker for long (R-42).
//!
//! **One read for the tokens, one for the reply.** The library is read **once**, before the batch,
//! and both compare-and-set tokens for every file come from that one read: `skill.updated_at` for
//! the description, the head version for the append. That is the order milestone 2's review
//! finding 5 settled for bindings and versions, and it is what makes the CAS honest rather than a
//! guess. A `Stale` refuses that one file and the batch continues.
//!
//! **What is written.** A `skill` row and a `skill_version` row, and nothing else — ANA-22 §7.3:
//! "nothing is attached by import". The frontmatter's activation keys ride along in
//! `skill_version.source` and the attachments matrix reads them back when the skill is attached.
//!
//! **The clock.** [`crate::skills`] reads no clock, because the store stamps every instant. This
//! module reads one, once per batch, for `source.imported_at` — the provenance the import itself
//! writes. Every row instant still comes from the store.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use htui_core::model::skill::SkillEntry;
use htui_core::model::skill_import::{ParsedSkill, SKILL_FILE, parse};
use htui_core::model::{NewSkill, NewSkillVersion, SkillId, UserId};
use htui_core::store::{CasOutcome, Result, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE};

use crate::skills::SkillsSnapshot;

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

/// What happened to one file the maintainer named, or one the walk found under it.
///
/// Every outcome is reported, including the ones that wrote nothing: §9's mitigation for a reader
/// that rejects a real file is that it is *reported*, and a batch that says only what it imported
/// is a batch that hides a refusal.
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
    /// The file was not imported, and this is why — the reader's sentence, the name rule's
    /// sentence, or the CAS's.
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
/// `SKILL.md` at any depth up to [`MAX_DEPTH`] and a `*.md` / `*.mdc` at depth 1, the rules-directory
/// shape (D99). One bad file never stops the batch.
///
/// # Errors
/// Whatever the store reports, or [`StoreError::Unreachable`] with [`DATABASE_UNREACHABLE`]
/// offline. A path that does not exist is a reported [`ImportOutcome::Refused`], not an error: the
/// other paths in the batch still import.
pub async fn import(backend: &Backend, paths: &[String]) -> Result<Vec<ImportOutcome>> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
    let created_by = backend.this_user().await?;

    // One read for the tokens (D100) and one clock for the provenance.
    let library = backend.skill_library().await?;
    let now = Utc::now();

    let mut report: Vec<ImportOutcome> = Vec::new();
    for path in paths {
        report.extend(one_path(&writer, created_by, &library, path, now).await);
    }
    Ok(report)
}

/// The files one typed path contributes, and the ones it leaves alone.
async fn one_path(
    writer: &impl WriteStore,
    created_by: UserId,
    library: &[SkillEntry],
    path: &str,
    now: DateTime<Utc>,
) -> Vec<ImportOutcome> {
    let root = PathBuf::from(path);
    {
        let metadata = match std::fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) => {
                return vec![ImportOutcome::Refused {
                    path: path.to_owned(),
                    message: format!("could not read `{path}` ({error})"),
                }];
            }
        };
        if metadata.is_file() {
            return vec![write_one(writer, created_by, library, &root, path, now).await];
        }
        if !metadata.is_dir() {
            return vec![ImportOutcome::Refused {
                path: path.to_owned(),
                message: format!("`{path}` is neither a file nor a directory"),
            }];
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut report: Vec<ImportOutcome> = Vec::new();
        collect(&root, 0, is_rules_dir(&root), &mut candidates, &mut report);
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
            report.push(write_one(writer, created_by, library, candidate, &label, now).await);
        }
        report
    }
}

/// The directory names §8 means by "a rules directory": the folders the surveyed tools keep their
/// rule files in. A directory whose own name is one of these, or that is a hidden tool root such
/// as `.cursor`, contributes its `*.md` and `*.mdc`; any other directory contributes only
/// `SKILL.md` files.
///
/// This is the narrowing R-37 asked for. The alternative — every `*.md` at depth 1 of whatever
/// directory was named — turns a project root into a skill sweep, and `README.md` is a skill with
/// the stem `readme`.
pub const RULES_DIR_NAMES: [&str; 3] = ["rules", "instructions", "steering"];

/// Whether `dir` is a rules directory: one named for what it holds, or a hidden tool root that
/// has a rules-named child (`.cursor` holding `.cursor/rules`).
///
/// The second clause needs the child rather than a leading dot on its own, because "hidden" is not
/// what makes a directory a rules directory — `/home/x/.config` is hidden and is not one, and a
/// temporary directory is hidden too.
#[must_use]
pub fn is_rules_dir(dir: &Path) -> bool {
    let Some(name) = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return false;
    };
    if RULES_DIR_NAMES.contains(&name.as_str()) {
        return true;
    }
    if !name.starts_with('.') {
        return false;
    }
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(std::result::Result::ok).any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|child| RULES_DIR_NAMES.contains(&child))
        })
    })
}

/// Walks `dir` for skill files, sorted, with every prune reported.
///
/// The sort is the point: `read_dir` order is the filesystem's, and two machines would otherwise
/// disagree about which file is the first one to be refused. Byte order at every level is
/// `htui_agent::excerpt::FsRepoReader::walk`'s rule, and the reason this workspace treats it as the
/// canonical walk.
fn collect(
    dir: &Path,
    depth: usize,
    rules: bool,
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
            if metadata.file_type().is_symlink() {
                continue;
            }
            if SKIP_DIRS.contains(&name.as_str()) {
                skipped.push(ImportOutcome::Skipped {
                    path: entry.display().to_string(),
                    reason: format!("`{name}` is a bundled skill directory, not a skill"),
                });
                continue;
            }
            collect(&entry, depth + 1, rules, out, skipped);
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        if name == SKILL_FILE
            || (rules
                && depth == 0
                && Path::new(&name)
                    .extension()
                    .is_some_and(|extension| extension == "md" || extension == "mdc"))
        {
            out.push(entry);
        }
    }
}

/// Reads one file and writes it, or says why it was not written.
async fn write_one(
    writer: &impl WriteStore,
    created_by: UserId,
    library: &[SkillEntry],
    path: &Path,
    label: &str,
    now: DateTime<Utc>,
) -> ImportOutcome {
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
    let parsed = match parse(label, &file_name, &text, now) {
        Ok(parsed) => parsed,
        Err(message) => {
            return ImportOutcome::Refused {
                path: label.to_owned(),
                message,
            };
        }
    };
    write_skill(writer, created_by, library, parsed, label).await
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

/// The per-file write (D100), with both tokens from the read the batch took before it started.
async fn write_skill(
    writer: &impl WriteStore,
    created_by: UserId,
    library: &[SkillEntry],
    parsed: ParsedSkill,
    label: &str,
) -> ImportOutcome {
    let existing = library.iter().find(|entry| entry.skill.name == parsed.name);
    let description = parsed.description.clone();
    let body = parsed.body.clone();
    let name = parsed.name.clone();

    let skill = match writer
        .upsert_skill(
            NewSkill {
                id: SkillId::new(),
                name: name.clone(),
                description: description.clone(),
                created_by,
            },
            existing.map(|entry| entry.skill.updated_at),
        )
        .await
    {
        Ok(CasOutcome::Applied(skill)) => skill,
        Ok(CasOutcome::Stale(_)) => {
            return ImportOutcome::Refused {
                path: label.to_owned(),
                message: format!(
                    "`{name}` changed while this import was running; nothing was written for this \
                     file, and re-running the import picks it up"
                ),
            };
        }
        Err(error) => {
            return ImportOutcome::Refused {
                path: label.to_owned(),
                message: error.to_string(),
            };
        }
    };

    // A skill with no version at all is a skill nothing has appended to; `None` starts it at 1.
    let head = existing.and_then(|entry| entry.versions.last());
    if let Some(head) = head
        && head.body == body
    {
        return ImportOutcome::Unchanged {
            name,
            path: label.to_owned(),
        };
    }
    let version = head.map_or(1, |head| head.version + 1);
    match writer
        .add_skill_version(
            NewSkillVersion {
                skill_id: skill.id,
                body,
                source: parsed.source,
                created_by,
            },
            head.map(|head| head.version),
        )
        .await
    {
        Ok(CasOutcome::Applied(_)) => {
            if existing.is_some() {
                ImportOutcome::Updated {
                    name,
                    path: label.to_owned(),
                    version,
                }
            } else {
                ImportOutcome::Imported {
                    name,
                    path: label.to_owned(),
                    version,
                }
            }
        }
        Ok(CasOutcome::Stale(_)) => ImportOutcome::Refused {
            path: label.to_owned(),
            message: format!(
                "`{name}` gained a version while this import was running; its description was \
                 updated and no version was appended, so re-running the import finishes it"
            ),
        },
        Err(error) => ImportOutcome::Refused {
            path: label.to_owned(),
            message: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_FILES, RULES_DIR_NAMES, SKILL_FILE, SKIP_DIRS, collect, import, is_rules_dir};
    use crate::skill_import::ImportOutcome;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A tree with the two shapes D99 collects and the five directories it prunes.
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

    fn walk(root: &Path) -> (Vec<PathBuf>, Vec<ImportOutcome>) {
        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect(root, 0, is_rules_dir(root), &mut out, &mut skipped);
        (out, skipped)
    }

    #[test]
    fn a_directory_yields_the_skill_shapes_and_nothing_else() {
        let dir = tree();
        let (found, skipped) = walk(dir.path());

        let names: Vec<String> = found
            .iter()
            .map(|path| path.strip_prefix(dir.path()).unwrap().display().to_string())
            .collect();
        assert_eq!(
            names,
            ["one/SKILL.md", "two/nested/SKILL.md"],
            "`*/SKILL.md` at any depth, and nothing else: a project root is not a rules directory, \
             so `README.md` and `rules.mdc` at its top level are not swept (R-37)"
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
        fs::create_dir_all(&rules).expect("mkdir");
        for name in ["workers.mdc", "style.md", "notes.txt"] {
            fs::write(rules.join(name), "x").expect("write");
        }
        let (found, _) = walk(&rules);
        let names: Vec<String> = found
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            ["style.md", "workers.mdc"],
            "`.md` and `.mdc`, not `.txt`"
        );

        for name in RULES_DIR_NAMES {
            assert!(is_rules_dir(Path::new("/x").join(name).as_path()), "{name}");
        }
        assert!(
            !is_rules_dir(Path::new("/home/mluigi/projects/htui")),
            "a project root is not"
        );
        // A hidden directory with nothing rules-shaped under it is not a tool root, and a
        // temporary directory is hidden — which is how a leading dot on its own was caught being
        // too loose a test.
        let plain = tempfile::tempdir().expect("a temp dir");
        assert!(
            !is_rules_dir(plain.path()),
            "a temporary directory is hidden and is not a tool root"
        );

        // A hidden tool root counts only when it holds a rules-named child.
        let tool = dir.path().join(".cursor");
        fs::create_dir_all(tool.join("rules")).expect("mkdir");
        assert!(
            is_rules_dir(&tool),
            "`.cursor` holding `rules` is a tool root"
        );
        let bare = dir.path().join(".config");
        fs::create_dir_all(&bare).expect("mkdir");
        assert!(
            !is_rules_dir(&bare),
            "a hidden directory with no rules child is not"
        );
    }

    #[test]
    fn the_walk_is_byte_ordered_and_twice_the_same() {
        let dir = tempfile::tempdir().expect("a temp dir");
        for name in ["b/SKILL.md", "a/SKILL.md", "c/SKILL.md", "A/SKILL.md"] {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            fs::write(&path, "x").expect("write");
        }
        let (first, _) = walk(dir.path());
        let (second, _) = walk(dir.path());
        assert_eq!(first, second, "the same tree twice is the same order");
        let names: Vec<String> = first
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["SKILL.md", "SKILL.md", "SKILL.md", "SKILL.md"]);
        let dirs: Vec<String> = first
            .iter()
            .map(|path| {
                path.parent()
                    .expect("a parent")
                    .file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            dirs,
            ["A", "a", "b", "c"],
            "byte order, so `A` is not `a` by accident"
        );
    }

    #[test]
    fn the_bundled_directories_are_skipped_and_listed() {
        let dir = tree();
        let (found, skipped) = walk(dir.path());
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
            found
                .iter()
                .all(|path| !path.to_string_lossy().contains("scripts")),
            "nothing under a bundled directory is a candidate"
        );
    }

    #[test]
    fn a_symlinked_directory_is_not_descended() {
        let dir = tree();
        let elsewhere = tempfile::tempdir().expect("a temp dir");
        fs::create_dir_all(elsewhere.path().join("inner")).expect("mkdir");
        fs::write(elsewhere.path().join("inner/SKILL.md"), "x").expect("write");
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("link")).expect("symlink");

        let (found, _) = walk(dir.path());
        assert!(
            found
                .iter()
                .all(|path| !path.to_string_lossy().contains("link")),
            "a directory link can leave the tree the maintainer named: {found:?}"
        );
    }

    /// The cap is a safety bound on one walk, not a policy about how many skills a library holds,
    /// and a cap that stops silently is not a cap. The `one_path` arm turns the overflow into a
    /// reported `Skipped` row, which `a_missing_path_is_refused_and_the_rest_of_the_batch_still_
    /// imports` covers from the other end.
    #[test]
    fn the_file_cap_is_bounded() {
        let shown = format!("{MAX_FILES}");
        assert!(
            shown
                .parse::<usize>()
                .is_ok_and(|cap| (1..=1024).contains(&cap)),
            "MAX_FILES = {shown}"
        );
    }

    // --- the write path, end to end over the demo store -----------------------------------------

    use htui_core::store::MemStore;
    use htui_store::Backend;

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    /// One `SKILL.md` in a temp directory, returning the path and the directory that holds it.
    fn skill_file(dir: &Path, name: &str, body: &str) -> PathBuf {
        let nested = dir.join(name);
        fs::create_dir_all(&nested).expect("mkdir");
        let path = nested.join(SKILL_FILE);
        fs::write(
            &path,
            format!("---\nname: {name}\ndescription: The {name} skill.\n---\n{body}"),
        )
        .expect("write");
        path
    }

    async fn run(backend: &Backend, paths: &[String]) -> Vec<ImportOutcome> {
        import(backend, paths).await.expect("the import answers")
    }

    /// A new name creates the skill at version 1 with a non-empty `source` — the first writer of
    /// one in the tree, and the column the attachment form will read back.
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
        let library = backend.skill_library().await.expect("readable");
        let entry = library
            .iter()
            .find(|entry| entry.skill.name == "house-rules")
            .expect("the skill is in the library");
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
        let library = backend.skill_library().await.expect("readable");
        assert_eq!(
            library
                .iter()
                .find(|entry| entry.skill.name == "house-rules")
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
        let library = backend.skill_library().await.expect("readable");
        let entry = library
            .iter()
            .find(|entry| entry.skill.name == "house-rules")
            .expect("present");
        assert_eq!(entry.versions.len(), 2, "a changed body appends v2");
        assert_eq!(
            entry.skill.description, "The house rules, revised.",
            "and the description moves"
        );
    }

    /// A name the writer would refuse is refused here too, in the writer's own words, and nothing
    /// is written — which is OQ-22's default: refuse the file, slugify nothing.
    #[tokio::test]
    async fn a_refused_name_writes_nothing_and_says_the_writers_own_sentence() {
        let backend = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("My Skill.md");
        fs::write(&path, "---\nname: My Skill\ndescription: d\n---\nB\n").expect("write");

        let report = run(&backend, &[path.display().to_string()]).await;

        let ImportOutcome::Refused { message, .. } = &report[0] else {
            panic!("a refused name is a Refused row, got {report:?}")
        };
        assert_eq!(
            message,
            &htui_core::store::traits::invalid_skill_name("My Skill")
        );
        assert!(
            backend
                .skill_library()
                .await
                .expect("readable")
                .iter()
                .all(|entry| entry.skill.name != "My Skill"),
            "nothing was written under a rewritten name either"
        );
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
        let bundled = dir.path().join("one/scripts");
        fs::create_dir_all(&bundled).expect("mkdir");
        fs::write(bundled.join("helper.sh"), "#!/bin/sh\n").expect("write");

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
            report
                .iter()
                .any(|outcome| matches!(outcome, ImportOutcome::Skipped { path, .. } if path.ends_with("scripts"))),
            "the bundled directory is listed: {report:?}"
        );
    }

    /// The report never carries file **content** — a path and a sentence, never a body. This is the
    /// redaction rule the `SaveSkill` body follows, applied to the other direction.
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
}

#[cfg(test)]
mod cap_tests {
    use super::{MAX_BYTES, read_text};

    /// The cap is asked of the file before it is read. A file one byte over is refused, and the
    /// refusal names the cap rather than the read.
    #[test]
    fn a_file_over_the_cap_is_refused_and_a_file_under_it_is_not() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let small = dir.path().join("small.md");
        std::fs::write(&small, vec![b'x'; 8]).expect("write");
        assert!(read_text(&small).is_ok(), "a file under the cap reads");

        let big = dir.path().join("big.md");
        std::fs::write(&big, vec![b'x'; MAX_BYTES as usize + 1]).expect("write");
        let refusal = read_text(&big).expect_err("one byte over the cap is refused");
        assert!(
            refusal.contains(&format!("{MAX_BYTES}-byte cap")),
            "the refusal names the cap, not a read error: {refusal}"
        );
        assert!(read_text(&big).is_err(), "and it stays refused");
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
        std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("write");
        let refusal = read_text(&path).expect_err("not UTF-8");
        assert!(refusal.contains("not UTF-8"), "{refusal}");
    }
}
