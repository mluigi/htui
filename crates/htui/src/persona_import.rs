//! Reading persona files off the disk and writing them through `create_persona` (MOD-26
//! milestone 2, D20, OQ-8, OQ-9): the `I` path of `Settings > Personas`.
//!
//! **Why the worker reads the files.** `R-NF-3` puts store-touching work off the UI task, and a
//! directory sweep is file I/O of unbounded size by nature, so the section types a path and the
//! store worker does the rest (I-11), exactly as [`crate::skill_import`] does. `std::fs` in an
//! `async fn` is that module's precedent, and the sweep is capped ([`MAX_FILES`], [`MAX_BYTES`]).
//!
//! **The row is the truth.** An import writes one `persona` row per file and stores no path: the
//! file is never re-read, and editing it afterwards changes nothing (I-10, `R-ID-3`). Nothing here
//! asks a model anything (`R-ID-6`): a file is read by
//! [`parse_import`](htui_core::model::persona::parse_import), the persona file grammar with the
//! one import exception (an `mcp__` entry in `tools` is dropped and named, OQ-7).
//!
//! **The sweep is new logic, not the skill walk.** A directory contributes its depth-0 regular
//! `*.md` files only, sorted by file-name bytes, and a file that does not open with a `---` fence
//! is skipped and reported (OQ-9): for personas a directory **is** `.claude/agents/`, and the
//! `README.md` beside the agents is not one. The skill import's R-37 refuses exactly that sweep
//! for skills, which is why the walk is not shared.
//!
//! **Known names are refused** (OQ-8): an import never overwrites a row the maintainer may have
//! edited in the section. A name met twice in one import is refused the second time, naming the
//! first file. The sentence helpers are copied from `skill_import.rs` rather than widened, so
//! that module stays untouched.

use std::path::{Path, PathBuf};

use htui_core::model::frontmatter::FrontmatterError;
use htui_core::model::persona::parse_import;
use htui_core::model::{Persona, PersonaFileError, PersonaId};
use htui_core::store::{Result, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE};

/// The largest file read as a persona. A file over this is reported as skipped, never truncated.
pub const MAX_BYTES: u64 = 256 * 1024;

/// The most `*.md` files one directory contributes; past it the rest are one reported row.
pub const MAX_FILES: usize = 64;

/// Why a fence-less file is left alone (OQ-9): a `README.md` beside the agents.
pub const NO_FRONTMATTER: &str = "no frontmatter: the file does not open with a `---` fence";

/// What happened to one file. No variant carries file content: a name, a path, a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonaOutcome {
    /// A new persona row.
    Imported {
        /// `persona.name`, as stored.
        name: String,
        /// The file it came from.
        path: String,
        /// The `tools` entries dropped as MCP tools (OQ-7), in file order; usually empty.
        dropped: Vec<String>,
    },
    /// Not imported, and the one sentence why: the reader's, the store's, OQ-8's or the batch's.
    Refused {
        /// The file, or the path the maintainer typed.
        path: String,
        /// One sentence.
        message: String,
    },
    /// Left alone: no frontmatter, over the byte cap, not UTF-8, past the file cap.
    Skipped {
        /// The file, or the directory.
        path: String,
        /// One sentence.
        reason: String,
    },
}

/// The import's answer: the registry after the batch, and one row per file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaImports {
    /// A fresh `personas()` read, by name.
    pub personas: Vec<Persona>,
    /// One row per file, in the order the walk met them.
    pub report: Vec<PersonaOutcome>,
}

/// Imports the file or directory at `path` (D20) and reports on every file.
///
/// # Errors
/// [`StoreError::Unreachable`] with [`DATABASE_UNREACHABLE`] offline (before any file is read),
/// or the one `personas()` read's error. A path that does not exist is a reported `Refused`.
pub async fn import(backend: &Backend, path: &str) -> Result<Vec<PersonaOutcome>> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
    // OQ-8's known names: one read, before any write, so a file never meets a row this batch
    // wrote (that is `written`'s sentence, not this one).
    let known = writer
        .personas()
        .await?
        .into_iter()
        .map(|row| row.name)
        .collect();
    let mut batch = Batch {
        writer: &writer,
        known,
        written: Vec::new(),
    };
    Ok(batch.one_path(path).await)
}

/// One import's shared state: the writer, the names one read found, and the names this batch
/// wrote.
struct Batch<'a, W> {
    /// The seam every write goes through.
    writer: &'a W,
    /// The registry's names as read before the first write (OQ-8).
    known: Vec<String>,
    /// `(name, path)` of every row this batch created.
    written: Vec<(String, String)>,
}

impl<W: WriteStore> Batch<'_, W> {
    /// The files one typed path contributes (OQ-9, B-14).
    async fn one_path(&mut self, path: &str) -> Vec<PersonaOutcome> {
        let root = PathBuf::from(path);
        let metadata = match std::fs::metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) => {
                return vec![PersonaOutcome::Refused {
                    path: path.to_owned(),
                    message: format!("could not read `{path}` ({error})"),
                }];
            }
        };
        // A directly named file is read whatever its extension (the skill import's rule).
        if metadata.is_file() {
            return vec![self.write_one(&root, path).await];
        }
        if !metadata.is_dir() {
            return vec![PersonaOutcome::Refused {
                path: path.to_owned(),
                message: format!("`{path}` is neither a file nor a directory"),
            }];
        }

        let mut candidates = markdown_files(&root);
        let past = candidates.len().saturating_sub(MAX_FILES);
        candidates.truncate(MAX_FILES);
        let mut report = Vec::with_capacity(candidates.len() + 1);
        for candidate in &candidates {
            let label = candidate.display().to_string();
            report.push(self.write_one(candidate, &label).await);
        }
        if past > 0 {
            report.push(PersonaOutcome::Skipped {
                path: path.to_owned(),
                reason: format!("{past} more file(s) past the {MAX_FILES}-file cap"),
            });
        }
        report
    }

    /// Reads one file and writes it, or says why it was not written; first match wins.
    async fn write_one(&mut self, path: &Path, label: &str) -> PersonaOutcome {
        let refused = |message: String| PersonaOutcome::Refused {
            path: label.to_owned(),
            message,
        };
        let text = match read_text(path) {
            Ok(text) => text,
            Err(reason) => {
                return PersonaOutcome::Skipped {
                    path: label.to_owned(),
                    reason,
                };
            }
        };
        let imported = match parse_import(&text) {
            Ok(imported) => imported,
            Err(PersonaFileError::Fence(FrontmatterError::NoFence)) => {
                return PersonaOutcome::Skipped {
                    path: label.to_owned(),
                    reason: NO_FRONTMATTER.to_owned(),
                };
            }
            Err(error) => return refused(error.to_string()),
        };
        let name = imported.file.name.clone();
        if let Some((_, first)) = self.written.iter().find(|(written, _)| *written == name) {
            return refused(format!(
                "`{}` was already imported from `{first}` by this import; this file was not \
                 written",
                name.escape_debug()
            ));
        }
        if self.known.contains(&name) {
            return refused(format!(
                "persona `{}` exists; edit it in Settings \u{203a} Personas, or delete it and \
                 import again",
                name.escape_debug()
            ));
        }
        match self
            .writer
            .create_persona(imported.file.into_new(PersonaId::new()))
            .await
        {
            Ok(row) => {
                self.written.push((row.name.clone(), label.to_owned()));
                PersonaOutcome::Imported {
                    name: row.name,
                    path: label.to_owned(),
                    dropped: imported.dropped,
                }
            }
            Err(StoreError::Constraint(sentence)) => refused(sentence),
            Err(other) => refused(other.to_string()),
        }
    }
}

/// A directory's depth-0 regular `*.md` files (extension exactly `md`), links followed to files,
/// sorted by file-name bytes (B-14). Subdirectories, other files and broken links are left alone
/// and not reported: a sweep that reported them would open the report for every directory.
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .filter(|path| std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()))
        .collect();
    files.sort_by(|left, right| {
        left.file_name()
            .unwrap_or_default()
            .as_encoded_bytes()
            .cmp(right.file_name().unwrap_or_default().as_encoded_bytes())
    });
    files
}

/// Reads a file as text, refusing in one sentence each (copied from `skill_import.rs`, which
/// stays untouched): the cap is asked of the file **before** it is read.
fn read_text(path: &Path) -> std::result::Result<String, String> {
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
    use super::{MAX_BYTES, NO_FRONTMATTER, PersonaOutcome, import};
    use htui_core::model::PersonaFileError;
    use htui_core::model::persona::MODEL_REFUSED;
    use htui_core::store::{MemStore, StoreError, WriteStore as _};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};
    use std::fs;
    use std::path::Path;

    /// `.claude/agents/code-architect.md`'s frontmatter, verbatim (copied; no test reads a real
    /// agent file), then a one-line body.
    const CODE_ARCHITECT: &str = "---\n\
        name: code-architect\n\
        description: Designs feature architectures by analyzing existing codebase patterns and \
        conventions, then providing implementation blueprints with concrete files, interfaces, \
        data flow, and build order.\n\
        tools: Read, Grep, Glob, Bash, mcp__gortex__capabilities, mcp__gortex__explore, \
        mcp__gortex__search, mcp__gortex__read, mcp__gortex__relations, mcp__gortex__trace, \
        mcp__gortex__analyze, mcp__gortex__recall, mcp__gortex__workspace\n\
        ---\n\nYou design.\n";

    /// A two-key persona file.
    const SCOUT: &str = "---\nname: scout\ndeny-kinds: execute\n---\n\nYou scout.\n";

    /// `~/.claude/agents/gortex-search.md`'s frontmatter, verbatim, then a one-line body.
    const GORTEX_SEARCH: &str = "---\n\
        name: gortex-search\n\
        description: \"Locate code, trace call paths, or map architecture in a fresh context.\"\n\
        tools: mcp__gortex__capabilities, mcp__gortex__explore, mcp__gortex__search, \
        mcp__gortex__read, mcp__gortex__relations, mcp__gortex__trace, mcp__gortex__analyze, \
        mcp__gortex__recall, mcp__gortex__workspace\n\
        ---\n\nYou search.\n";

    /// A `README.md` beside the agents: no fence.
    const README: &str = "# Agents\n\nThe files beside this one are personas.\n";

    /// The nine `mcp__gortex__*` entries the architect's `tools` carries, in file order.
    const ARCHITECT_MCP_TOOLS: [&str; 9] = [
        "mcp__gortex__capabilities",
        "mcp__gortex__explore",
        "mcp__gortex__search",
        "mcp__gortex__read",
        "mcp__gortex__relations",
        "mcp__gortex__trace",
        "mcp__gortex__analyze",
        "mcp__gortex__recall",
        "mcp__gortex__workspace",
    ];

    fn demo() -> (MemStore, Backend) {
        let store = MemStore::demo();
        (store.clone(), Backend::memory(store))
    }

    fn write(dir: &Path, name: &str, text: &str) -> String {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        fs::write(&path, text).expect("write");
        path.display().to_string()
    }

    fn label(path: &Path) -> String {
        path.display().to_string()
    }

    async fn names(store: &MemStore) -> Vec<String> {
        store
            .personas()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .map(|row| row.name)
            .collect()
    }

    #[tokio::test]
    async fn a_file_imports_one_persona_and_names_its_dropped_mcp_tools() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = write(dir.path(), "code-architect.md", CODE_ARCHITECT);

        let report = import(&backend, &path).await.expect("the import runs");

        assert_eq!(
            report,
            [PersonaOutcome::Imported {
                name: "code-architect".to_owned(),
                path,
                dropped: ARCHITECT_MCP_TOOLS.map(str::to_owned).to_vec(),
            }]
        );
        let row = store
            .personas()
            .await
            .expect("read")
            .into_iter()
            .find(|row| row.name == "code-architect")
            .expect("the row landed");
        assert_eq!(row.tools.allow, ["Read", "Grep", "Glob", "Bash"]);
        assert_eq!(row.body, "You design.\n");
    }

    #[tokio::test]
    async fn a_directory_imports_its_depth_zero_md_files_in_byte_order() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let b = write(dir.path(), "b.md", SCOUT);
        let a = write(dir.path(), "a.md", CODE_ARCHITECT);

        let report = import(&backend, &label(dir.path()))
            .await
            .expect("the import runs");

        let paths: Vec<(&str, &str)> = report
            .iter()
            .map(|outcome| match outcome {
                PersonaOutcome::Imported { name, path, .. } => (name.as_str(), path.as_str()),
                other => panic!("every file imports: {other:?}"),
            })
            .collect();
        assert_eq!(
            paths,
            [("code-architect", a.as_str()), ("scout", b.as_str())]
        );
        let names = names(&store).await;
        assert!(names.contains(&"code-architect".to_owned()), "{names:?}");
        assert!(names.contains(&"scout".to_owned()), "{names:?}");
    }

    #[tokio::test]
    async fn a_readme_without_frontmatter_is_skipped() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let readme = write(dir.path(), "README.md", README);
        let before = names(&store).await;

        let report = import(&backend, &label(dir.path()))
            .await
            .expect("the import runs");

        assert_eq!(
            report,
            [PersonaOutcome::Skipped {
                path: readme,
                reason: NO_FRONTMATTER.to_owned(),
            }]
        );
        assert_eq!(names(&store).await, before, "no row");
    }

    /// B-14: a sweep collects depth-0 `*.md` only, and what it leaves alone it does not report.
    #[tokio::test]
    async fn a_non_md_file_and_a_subdirectory_are_left_alone() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        write(dir.path(), "notes.txt", SCOUT);
        write(dir.path(), "sub/x.md", SCOUT);
        let before = names(&store).await;

        let report = import(&backend, &label(dir.path()))
            .await
            .expect("the import runs");

        assert_eq!(
            report,
            [],
            "neither is a candidate, and neither is reported"
        );
        assert_eq!(names(&store).await, before, "no row");
    }

    #[tokio::test]
    async fn an_all_mcp_tools_file_is_refused() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = write(dir.path(), "gortex-search.md", GORTEX_SEARCH);
        let before = names(&store).await;

        let report = import(&backend, &path).await.expect("the import runs");

        assert_eq!(
            report,
            [PersonaOutcome::Refused {
                path,
                message: PersonaFileError::OnlyMcpTools.to_string(),
            }]
        );
        assert_eq!(names(&store).await, before, "no row");
    }

    /// OQ-8: a known name is refused, and the row the maintainer may have edited is untouched.
    #[tokio::test]
    async fn a_known_name_is_refused_and_the_row_is_untouched() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = write(
            dir.path(),
            "reviewer.md",
            "---\nname: reviewer\n---\n\nA different body.\n",
        );
        let before = store.personas().await.expect("read");

        let report = import(&backend, &path).await.expect("the import runs");

        assert_eq!(
            report,
            [PersonaOutcome::Refused {
                path,
                message: "persona `reviewer` exists; edit it in Settings \u{203a} Personas, or \
                          delete it and import again"
                    .to_owned(),
            }]
        );
        assert_eq!(store.personas().await.expect("read"), before);
    }

    #[tokio::test]
    async fn a_name_met_twice_in_one_import_is_refused_naming_the_first_file() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let a = write(dir.path(), "a.md", SCOUT);
        let b = write(dir.path(), "b.md", SCOUT);

        let report = import(&backend, &label(dir.path()))
            .await
            .expect("the import runs");

        assert_eq!(report.len(), 2, "{report:?}");
        assert!(
            matches!(&report[0], PersonaOutcome::Imported { name, .. } if name == "scout"),
            "{report:?}"
        );
        assert_eq!(
            report[1],
            PersonaOutcome::Refused {
                path: b,
                message: format!(
                    "`scout` was already imported from `{a}` by this import; this file was not \
                     written"
                ),
            }
        );
        let scouts = names(&store)
            .await
            .into_iter()
            .filter(|name| name == "scout")
            .count();
        assert_eq!(scouts, 1, "one row");
    }

    #[tokio::test]
    async fn a_directory_past_the_file_cap_imports_the_cap_and_reports_the_rest() {
        let (_store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        for index in 0..65 {
            write(
                dir.path(),
                &format!("p{index:02}.md"),
                &format!("---\nname: p-{index:02}\n---\n\nYou are number {index}.\n"),
            );
        }
        let root = label(dir.path());

        let report = import(&backend, &root).await.expect("the import runs");

        assert_eq!(report.len(), 65, "{report:?}");
        assert!(
            report[..64]
                .iter()
                .all(|outcome| matches!(outcome, PersonaOutcome::Imported { .. })),
            "{report:?}"
        );
        assert!(
            matches!(&report[63], PersonaOutcome::Imported { name, .. } if name == "p-63"),
            "{report:?}"
        );
        assert_eq!(
            report[64],
            PersonaOutcome::Skipped {
                path: root,
                reason: "1 more file(s) past the 64-file cap".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn a_file_over_the_byte_cap_is_skipped() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut text = SCOUT.to_owned();
        let pad = usize::try_from(MAX_BYTES).expect("fits") + 1 - text.len();
        text.push_str(&"x".repeat(pad));
        let path = write(dir.path(), "big.md", &text);
        let before = names(&store).await;

        let report = import(&backend, &path).await.expect("the import runs");

        assert_eq!(
            report,
            [PersonaOutcome::Skipped {
                path,
                reason: "the file is over the 262144-byte cap; nothing was imported".to_owned(),
            }]
        );
        assert_eq!(names(&store).await, before, "no row");
    }

    #[tokio::test]
    async fn a_missing_path_is_refused() {
        let (_store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = label(&dir.path().join("nowhere"));

        let report = import(&backend, &path).await.expect("the import runs");

        let [
            PersonaOutcome::Refused {
                path: named,
                message,
            },
        ] = report.as_slice()
        else {
            panic!("a missing path is one refusal: {report:?}")
        };
        assert_eq!(*named, path);
        assert!(
            message.starts_with(&format!("could not read `{path}` (")) && message.ends_with(')'),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_refused_file_writes_nothing() {
        let (store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = write(
            dir.path(),
            "opus.md",
            "---\nname: opus-scout\nmodel: opus\n---\n\nYou scout.\n",
        );
        let before = store.personas().await.expect("read");

        let report = import(&backend, &path).await.expect("the import runs");

        assert_eq!(
            report,
            [PersonaOutcome::Refused {
                path,
                message: MODEL_REFUSED.to_owned(),
            }]
        );
        assert_eq!(store.personas().await.expect("read"), before, "unchanged");
    }

    /// No outcome carries file content: a name, a path, a sentence.
    #[tokio::test]
    async fn the_report_prints_no_file_content() {
        let (_store, backend) = demo();
        let dir = tempfile::tempdir().expect("a temp dir");
        write(dir.path(), "a.md", CODE_ARCHITECT);
        write(dir.path(), "b.md", SCOUT);
        write(dir.path(), "c.md", GORTEX_SEARCH);
        write(dir.path(), "README.md", README);

        let report = import(&backend, &label(dir.path()))
            .await
            .expect("the import runs");

        assert_eq!(report.len(), 4, "{report:?}");
        let shown = format!("{report:?}");
        for body in ["You design.", "You scout.", "You search.", "are personas"] {
            assert!(!shown.contains(body), "{body}: {shown}");
        }
    }

    #[tokio::test]
    async fn offline_the_import_is_unreachable() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "personas-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline { cache, since: None };
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = write(dir.path(), "scout.md", SCOUT);

        assert_eq!(
            import(&backend, &path).await,
            Err(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))
        );
    }
}
