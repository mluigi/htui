//! The corpus (MOD-162 T3): `HANDOFF.md` open items, `docs/REQUIREMENTS.md` requirements and
//! `docs/decisions/**` decisions read from a pinned commit (or the work tree), parsed into
//! [`CorpusDoc`]s and into the rows `htui_store::vector_sync` builds points from.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, bail};
use chrono::{DateTime, Utc};
use htui_core::model::{
    Document, DocumentId, Item, ItemId, ItemKindId, Priority, ProjectId, Requirement,
    RequirementAreaId, RequirementId, RequirementState, Resolution, Status, UserId,
};
use htui_store::vector::{ConceptPoint, PointType};
use htui_store::vector_sync::{document_points, item_point, requirement_point};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::types::{CorpusDoc, CorpusId, DECISION_PREFIX, DocKind};

/// ANA-37's corpus commit.
pub const DEFAULT_PIN: &str = "d15e9fea";
/// `--corpus-ref` value meaning "the files on disk", not a commit.
pub const WORKTREE: &str = "WORKTREE";

/// The three sources of one corpus, before parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    /// The resolved full commit sha; `None` for the work tree.
    pub commit: Option<String>,
    /// `HANDOFF.md`.
    pub handoff: String,
    /// `docs/REQUIREMENTS.md`.
    pub requirements: String,
    /// `(repo-relative path, text)` of every `docs/decisions/**/*.md`, sorted by path.
    pub decisions: Vec<(String, String)>,
}

/// An open item of `HANDOFF.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItem {
    /// `MOD-99`.
    pub key: String,
    /// The bold title after `KEY - `, verbatim.
    pub title: String,
    /// The rest of the header line and its continuation lines.
    pub body: String,
    /// 1-based line of the header.
    pub line: usize,
}

/// A requirement bullet of `docs/REQUIREMENTS.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementBullet {
    /// `R-STO-8`.
    pub key: String,
    /// `STO`.
    pub area_code: String,
    /// `8`.
    pub number: i32,
    /// From the header's parenthesis (blueprint §3.2 rule 2).
    pub priority: Priority,
    /// `Withdrawn` for `(withdrawn)`, else `Active`.
    pub state: RequirementState,
    /// The rest of the header line and its continuation lines.
    pub body: String,
    /// 1-based line of the header.
    pub line: usize,
}

/// A decision file's first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionHeader {
    /// `ANA-4` (no `DEC:`).
    pub key: String,
    /// The title, without the status group.
    pub title: String,
    /// From the status group's first word; `Done` when there is none.
    pub resolution: Resolution,
}

/// The corpus: documents for L, G and the query targets; rows for H and the point builders.
#[derive(Debug, Clone, PartialEq)]
pub struct Corpus {
    /// As requested: a rev, or [`WORKTREE`].
    pub corpus_ref: String,
    /// Resolved commit; `None` for the work tree.
    pub commit: Option<String>,
    /// The one synthetic project (`bench_uuid("project")`).
    pub project: ProjectId,
    /// Open items in file order, then requirements in file order, then decisions in path order.
    pub docs: Vec<CorpusDoc>,
    /// Open items, then decision items, in `docs` order.
    pub items: Vec<Item>,
    /// In `docs` order.
    pub requirements: Vec<Requirement>,
    /// One `summary` per decision item, in the same order.
    pub documents: Vec<Document>,
}

/// Points by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct PointCounts {
    /// Item points.
    pub item: usize,
    /// Requirement points.
    pub requirement: usize,
    /// Document (section) points.
    pub document: usize,
}

impl Corpus {
    /// Every document's ID.
    pub fn ids(&self) -> BTreeSet<CorpusId> {
        self.docs.iter().map(|doc| doc.id.clone()).collect()
    }

    /// The document with this ID.
    pub fn doc(&self, id: &CorpusId) -> Option<&CorpusDoc> {
        self.docs.iter().find(|doc| &doc.id == id)
    }

    /// `item_point` for every item, `document_points(item, doc)` for every summary,
    /// `requirement_point` for every requirement, in that order; `htui_store::vector_sync`
    /// unchanged (P3).
    pub fn points(&self) -> Vec<ConceptPoint> {
        let items: HashMap<ItemId, &Item> = self.items.iter().map(|item| (item.id, item)).collect();
        let mut points: Vec<ConceptPoint> = self.items.iter().map(item_point).collect();
        for document in &self.documents {
            if let Some(item) = items.get(&document.item_id) {
                points.extend(document_points(item, document));
            }
        }
        points.extend(self.requirements.iter().map(requirement_point));
        points
    }

    /// Counts by `ConceptPoint::point_type()`.
    pub fn point_counts(&self) -> PointCounts {
        let mut counts = PointCounts::default();
        for point in self.points() {
            match point.point_type() {
                PointType::Item => counts.item += 1,
                PointType::Requirement => counts.requirement += 1,
                PointType::Document => counts.document += 1,
            }
        }
        counts
    }
}

/// `created_at`, `updated_at` and `closed_at` of every row (blueprint §3.1).
fn bench_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_760_000_000, 0).unwrap_or_default()
}

/// Runs `git -C repo <args>`: stdout as UTF-8, an error on a failed spawn, a non-zero exit or
/// output that is not UTF-8.
fn git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let shown = args.join(" ");
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("running `git {shown}`"))?;
    if !out.status.success() {
        bail!(
            "`git {shown}` failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    String::from_utf8(out.stdout).with_context(|| format!("`git {shown}`: output is not UTF-8"))
}

/// `git rev-parse --verify <rev>^{commit}`: the full sha.
fn resolve_commit(repo: &Path, rev: &str) -> anyhow::Result<String> {
    let unreachable =
        || format!("corpus ref `{rev}` is unreachable in this clone (shallow?); pass --corpus-ref");
    // A rev git would read as an option is no rev at all.
    if rev.is_empty() || rev.starts_with('-') {
        bail!(unreachable());
    }
    let spec = format!("{rev}^{{commit}}");
    let sha = git(repo, &["rev-parse", "--verify", "--quiet", &spec]).with_context(unreachable)?;
    let sha = sha.trim();
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("{}: `git rev-parse` answered {sha:?}", unreachable());
    }
    Ok(sha.to_owned())
}

/// Namespaced UUID (P3): the first 16 bytes of sha256("htui-bench/v1\0" + name), stamped as a
/// version-8 (custom) UUID by `uuid::Builder::from_custom_bytes(..).into_uuid()`.
pub fn bench_uuid(name: &str) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(b"htui-bench/v1\0");
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_custom_bytes(bytes).into_uuid()
}

/// `git -C repo rev-parse --verify <rev>^{commit}`, then `git show <sha>:HANDOFF.md`,
/// `git show <sha>:docs/REQUIREMENTS.md`, `git ls-tree -r --name-only <sha> -- docs/decisions`
/// (only `*.md`) and `git show` of each. An unreachable rev is an error that names the rev and says
/// "unreachable in this clone (shallow?); pass --corpus-ref".
pub fn read_git(repo: &Path, rev: &str) -> anyhow::Result<Sources> {
    let sha = resolve_commit(repo, rev)?;
    let show = |path: &str| {
        git(repo, &["show", &format!("{sha}:{path}")])
            .with_context(|| format!("reading {path} at {sha}"))
    };
    let handoff = show("HANDOFF.md")?;
    let requirements = show("docs/REQUIREMENTS.md")?;
    // `-z`: paths verbatim, never quoted.
    let listing = git(
        repo,
        &[
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            &sha,
            "--",
            "docs/decisions",
        ],
    )
    .with_context(|| format!("listing docs/decisions at {sha}"))?;
    let mut paths: Vec<&str> = listing
        .split('\0')
        .filter(|path| path.ends_with(".md"))
        .collect();
    paths.sort_unstable();
    let decisions = paths
        .into_iter()
        .map(|path| Ok((path.to_owned(), show(path)?)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Sources {
        commit: Some(sha),
        handoff,
        requirements,
        decisions,
    })
}

/// The same three sources from disk under `root`; decisions found by a recursive `std::fs` walk
/// (no new crate), sorted by repo-relative path with `/` separators.
pub fn read_worktree(root: &Path) -> anyhow::Result<Sources> {
    let read = |rel: &str| {
        let path = root.join(rel);
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    };
    let handoff = read("HANDOFF.md")?;
    let requirements = read("docs/REQUIREMENTS.md")?;
    let mut decisions = Vec::new();
    walk_markdown(root, "docs/decisions", &mut decisions)?;
    decisions.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Sources {
        commit: None,
        handoff,
        requirements,
        decisions,
    })
}

/// Every `*.md` under `root/rel`, recursively, as `(slash path, text)`.
fn walk_markdown(root: &Path, rel: &str, out: &mut Vec<(String, String)>) -> anyhow::Result<()> {
    let dir = root.join(rel);
    let entries = std::fs::read_dir(&dir).with_context(|| format!("listing {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("listing {}", dir.display()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            bail!("{}: a file name that is not UTF-8", dir.display());
        };
        let child = format!("{rel}/{name}");
        let path = entry.path();
        if path.is_dir() {
            walk_markdown(root, &child, out)?;
        } else if path.is_file() && name.ends_with(".md") {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            out.push((child, text));
        }
    }
    Ok(())
}

/// Pure: parse, build rows and documents. An error names a decision file it cannot key.
pub fn build_from_sources(corpus_ref: &str, sources: Sources) -> anyhow::Result<Corpus> {
    let rows = Rows {
        project: ProjectId::from_uuid(bench_uuid("project")),
        user: UserId::from_uuid(bench_uuid("user")),
        time: bench_time(),
    };
    let open = parse_open_items(&sources.handoff);
    let bullets = parse_requirements(&sources.requirements);

    let mut docs = Vec::with_capacity(open.len() + bullets.len() + sources.decisions.len());
    let mut items = Vec::with_capacity(open.len() + sources.decisions.len());
    let mut requirements = Vec::with_capacity(bullets.len());
    let mut documents = Vec::with_capacity(sources.decisions.len());

    for open_item in open {
        let item = rows
            .item(
                &open_item.key,
                open_item.title.clone(),
                open_item.body,
                None,
            )
            .with_context(|| format!("HANDOFF.md:{}", open_item.line))?;
        docs.push(CorpusDoc {
            id: CorpusId::new(open_item.key.as_str()),
            kind: DocKind::OpenItem,
            key: open_item.key,
            title: open_item.title,
            text: item_point(&item).text,
            source: format!("HANDOFF.md:{}", open_item.line),
        });
        items.push(item);
    }

    for bullet in bullets {
        let row = rows.requirement(&bullet);
        docs.push(CorpusDoc {
            id: CorpusId::new(bullet.key.as_str()),
            kind: DocKind::Requirement,
            key: bullet.key,
            title: String::new(),
            text: requirement_point(&row).text,
            source: format!("docs/REQUIREMENTS.md:{}", bullet.line),
        });
        requirements.push(row);
    }

    for (path, text) in sources.decisions {
        let header = decision_header_of(&path, &text)?;
        let item = rows
            .item(
                &header.key,
                header.title.clone(),
                String::new(),
                Some(header.resolution),
            )
            .with_context(|| path.clone())?;
        documents.push(Document {
            id: DocumentId::from_uuid(bench_uuid(&format!(
                "document:{DECISION_PREFIX}{}",
                header.key
            ))),
            item_id: item.id,
            kind: "summary".to_owned(),
            version: 1,
            title: header.title.clone(),
            body: text.clone(),
            produced_by_step_id: None,
            created_by: rows.user,
            created_at: rows.time,
        });
        docs.push(CorpusDoc {
            id: CorpusId::decision(&header.key),
            kind: DocKind::Decision,
            key: header.key,
            title: header.title,
            text,
            source: path,
        });
        items.push(item);
    }

    let mut seen: HashMap<&CorpusId, &str> = HashMap::with_capacity(docs.len());
    for doc in &docs {
        if let Some(first) = seen.insert(&doc.id, &doc.source) {
            bail!(
                "duplicate corpus ID {} ({first} and {})",
                doc.id,
                doc.source
            );
        }
    }

    Ok(Corpus {
        corpus_ref: corpus_ref.to_owned(),
        commit: sources.commit,
        project: rows.project,
        docs,
        items,
        requirements,
        documents,
    })
}

/// `read_git` then `build_from_sources`.
pub fn build_from_git(repo: &Path, rev: &str) -> anyhow::Result<Corpus> {
    build_from_sources(rev, read_git(repo, rev)?)
}

/// [`WORKTREE`] gives `read_worktree`, anything else `build_from_git`.
pub fn build(repo: &Path, corpus_ref: &str) -> anyhow::Result<Corpus> {
    if corpus_ref == WORKTREE {
        build_from_sources(corpus_ref, read_worktree(repo)?)
    } else {
        build_from_git(repo, corpus_ref)
    }
}

/// The header of a decision file: its first line, or (no key there) the upper-cased file stem
/// with resolution `Done` and the first line, `#` stripped, as title (blueprint §3.2 rule 3).
fn decision_header_of(path: &str, text: &str) -> anyhow::Result<DecisionHeader> {
    let first = text.lines().next().unwrap_or_default();
    if let Some(header) = parse_decision_header(first) {
        return Ok(header);
    }
    let stem = Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_ascii_uppercase();
    if key_len(&stem) != Some(stem.len()) {
        bail!(
            "decision file {path}: neither its first line {first:?} nor its file stem carries a key"
        );
    }
    Ok(DecisionHeader {
        key: stem,
        title: first.trim_start_matches('#').trim().to_owned(),
        resolution: Resolution::Done,
    })
}

/// The constant fields every row shares (blueprint §3.1).
struct Rows {
    project: ProjectId,
    user: UserId,
    time: DateTime<Utc>,
}

impl Rows {
    /// An open item (`resolution` `None`) or a decision item (`Some`).
    fn item(
        &self,
        key: &str,
        title: String,
        body: String,
        resolution: Option<Resolution>,
    ) -> anyhow::Result<Item> {
        let (prefix, number) =
            split_key(key).with_context(|| format!("{key:?} is not a `PREFIX-NUMBER` key"))?;
        let id_name = match resolution {
            Some(_) => format!("item:{DECISION_PREFIX}{key}"),
            None => format!("item:{key}"),
        };
        Ok(Item {
            id: ItemId::from_uuid(bench_uuid(&id_name)),
            project_id: self.project,
            kind_id: ItemKindId::from_uuid(bench_uuid(&format!("kind:{prefix}"))),
            key_prefix: prefix.to_owned(),
            key_number: number,
            key: key.to_owned(),
            title,
            body,
            status: if resolution.is_some() {
                Status::Closed
            } else {
                Status::Open
            },
            priority: 0,
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            step_graph_id: None,
            version: 1,
            created_by: self.user,
            created_at: self.time,
            updated_at: self.time,
            closed_at: resolution.map(|_| self.time),
            resolution,
        })
    }

    /// A requirement row from its bullet.
    fn requirement(&self, bullet: &RequirementBullet) -> Requirement {
        Requirement {
            id: RequirementId::from_uuid(bench_uuid(&format!("requirement:{}", bullet.key))),
            project_id: self.project,
            area_id: RequirementAreaId::from_uuid(bench_uuid(&format!(
                "area:{}",
                bullet.area_code
            ))),
            area_code: bullet.area_code.clone(),
            number: bullet.number,
            key: bullet.key.clone(),
            body: bullet.body.clone(),
            rationale: String::new(),
            priority: bullet.priority,
            state: bullet.state,
            version: 1,
            created_by: self.user,
            created_at: self.time,
            updated_at: self.time,
        }
    }
}

/// `MOD-99` as `("MOD", 99)`: split at the last `-`.
fn split_key(key: &str) -> Option<(&str, i32)> {
    let (prefix, number) = key.rsplit_once('-')?;
    Some((prefix, number.parse().ok()?))
}

/// The byte length of the `[A-Z][A-Z0-9]*-[0-9]+` key that starts `s`, if one does and its
/// number fits an `i32`.
fn key_len(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if !bytes.first()?.is_ascii_uppercase() {
        return None;
    }
    let dash = 1 + bytes[1..]
        .iter()
        .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        .count();
    if bytes.get(dash) != Some(&b'-') {
        return None;
    }
    let digits = bytes[dash + 1..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let end = dash + 1 + digits;
    s[dash + 1..end].parse::<i32>().ok()?;
    Some(end)
}

/// A header line's block: the trimmed `rest` of the header, then every following line that starts
/// with two spaces (those stripped) up to a blank line or any other line, joined with `\n` and
/// trimmed. Returns the body and the index of the first line after the block.
fn block_body(rest: &str, lines: &[&str], mut next: usize) -> (String, usize) {
    let mut parts = vec![rest.trim()];
    while let Some(line) = lines.get(next) {
        match line.strip_prefix("  ") {
            Some(continuation) if !line.trim().is_empty() => parts.push(continuation),
            _ => break,
        }
        next += 1;
    }
    (parts.join("\n").trim().to_owned(), next)
}

/// The open items of `HANDOFF.md` (blueprint §3.2 rule 1).
pub fn parse_open_items(handoff: &str) -> Vec<OpenItem> {
    let lines: Vec<&str> = handoff.lines().collect();
    let mut items = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some((key, title, rest)) = open_item_header(lines[index]) else {
            index += 1;
            continue;
        };
        let (body, next) = block_body(rest, &lines, index + 1);
        items.push(OpenItem {
            key: key.to_owned(),
            title: title.to_owned(),
            body,
            line: index + 1,
        });
        index = next;
    }
    items
}

/// `- [ ] **KEY - TITLE**REST` as `(KEY, TITLE trimmed, REST)`.
fn open_item_header(line: &str) -> Option<(&str, &str, &str)> {
    let after = line.strip_prefix("- [ ] **")?;
    let key_end = key_len(after)?;
    let (key, after) = after.split_at(key_end);
    let after = after.strip_prefix(" - ")?;
    let (title, rest) = after.split_once("**")?;
    Some((key, title.trim(), rest))
}

/// The requirement bullets of `docs/REQUIREMENTS.md` (blueprint §3.2 rule 2).
pub fn parse_requirements(text: &str) -> Vec<RequirementBullet> {
    let lines: Vec<&str> = text.lines().collect();
    let mut bullets = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some(header) = requirement_header(lines[index]) else {
            index += 1;
            continue;
        };
        let (body, next) = block_body(header.rest, &lines, index + 1);
        let word = header.group.map(|group| {
            group
                .split([',', ';'])
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        });
        let state = if word.as_deref() == Some("withdrawn") {
            RequirementState::Withdrawn
        } else {
            RequirementState::Active
        };
        let later = match word.as_deref() {
            Some(word) => word == "later",
            None => header.area_code == "LATER",
        };
        bullets.push(RequirementBullet {
            key: header.key.to_owned(),
            area_code: header.area_code.to_owned(),
            number: header.number,
            priority: if later {
                Priority::Later
            } else {
                Priority::Must
            },
            state,
            body,
            line: index + 1,
        });
        index = next;
    }
    bullets
}

/// A matched `- **R-AREA-N (group).**REST` header.
struct RequirementHeader<'a> {
    key: &'a str,
    area_code: &'a str,
    number: i32,
    group: Option<&'a str>,
    rest: &'a str,
}

/// `^- \*\*(R-([A-Z][A-Z0-9]*)-([0-9]+))( \(([^)]*)\))?\.\*\*(REST)$`.
fn requirement_header(line: &str) -> Option<RequirementHeader<'_>> {
    let after_bold = line.strip_prefix("- **")?;
    let area_and_number = after_bold.strip_prefix("R-")?;
    let end = key_len(area_and_number)?;
    let key = &after_bold[..2 + end];
    let (area_code, number) = split_key(&area_and_number[..end])?;
    let after = &area_and_number[end..];
    let (group, after) = match after.strip_prefix(" (") {
        Some(inner) => {
            let close = inner.find(')')?;
            (Some(&inner[..close]), &inner[close + 1..])
        }
        None => (None, after),
    };
    let rest = after.strip_prefix(".**")?;
    Some(RequirementHeader {
        key,
        area_code,
        number,
        group,
        rest,
    })
}

/// A decision file's first line (blueprint §3.2 rule 3); `None` when it carries no key.
pub fn parse_decision_header(first_line: &str) -> Option<DecisionHeader> {
    let after = first_line.strip_prefix("# ")?;
    let key_end = key_len(after)?;
    let (key, after) = after.split_at(key_end);
    let rest = after
        .strip_prefix(" - ")
        .or_else(|| after.strip_prefix(": "))?;
    let (title, resolution) = match status_group(rest) {
        Some((open, word)) => (&rest[..open], resolution_of(word)),
        None => (rest, Resolution::Done),
    };
    Some(DecisionHeader {
        key: key.to_owned(),
        title: title.trim().to_owned(),
        resolution,
    })
}

/// The trailing `(word …)` group of `rest`, matched by depth counting from its last `)`, whose
/// first word (ending at `,`, `;`, a space or `)`) is non-empty lowercase ASCII: the byte index of
/// its `(` and the word.
fn status_group(rest: &str) -> Option<(usize, &str)> {
    let rest = rest.trim_end();
    if !rest.ends_with(')') {
        return None;
    }
    let mut depth = 0usize;
    let mut open = None;
    for (index, ch) in rest.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    open = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let open = open?;
    let inner = &rest[open + 1..];
    let word_end = inner.find([',', ';', ' ', ')']).unwrap_or(inner.len());
    let word = &inner[..word_end];
    let lowercase = !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase());
    lowercase.then_some((open, word))
}

/// `Resolution::from_str(word)`, falling back to `Done` (shipped, decided, status, anything).
pub fn resolution_of(word: &str) -> Resolution {
    word.parse().unwrap_or(Resolution::Done)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use htui_core::model::{ItemId, Status};
    use htui_store::vector::PointType;
    use htui_store::vector_sync::{item_point, requirement_point};

    use super::*;
    use crate::types::DocKind;

    const HANDOFF: &str = "\
# Handoff

## Open

- [ ] **MOD-1 - First item** (from ANA-13).
  Body line one.
    - nested bullet

- [x] **MOD-2 - Done item**
  ignored.
- [ ] **ANA-3 - Second `item` (from ANA-13).** trailing text
  runs to a heading
### Heading
- [ ] **TOOL-4 - Third item**
  runs to the next item
- [ ] **CLEAN-5 - Fourth**
- plain bullet
  not a continuation of anything
- [ ] not bold MOD-6
- [ ] **no key here**
- [ ] **MOD-7 without separator**
";

    #[test]
    fn open_items_are_the_unchecked_bold_keyed_bullets() {
        let items = parse_open_items(HANDOFF);
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["MOD-1", "ANA-3", "TOOL-4", "CLEAN-5"]);

        // Body runs to a blank line; a continuation keeps all but its first two spaces.
        assert_eq!(
            items[0],
            OpenItem {
                key: "MOD-1".into(),
                title: "First item".into(),
                body: "(from ANA-13).\nBody line one.\n  - nested bullet".into(),
                line: 5,
            }
        );
        // Title verbatim up to the first closing `**`; trailing text joins the body; a heading
        // ends the block.
        assert_eq!(
            items[1],
            OpenItem {
                key: "ANA-3".into(),
                title: "Second `item` (from ANA-13).".into(),
                body: "trailing text\nruns to a heading".into(),
                line: 11,
            }
        );
        // The next item ends the block.
        assert_eq!(items[2].body, "runs to the next item");
        assert_eq!(items[2].line, 14);
        // Any other line ends it too.
        assert_eq!(items[3].title, "Fourth");
        assert_eq!(items[3].body, "");
        assert_eq!(items[3].line, 16);
    }

    #[test]
    fn open_items_split_crlf_lines() {
        let items = parse_open_items("- [ ] **MOD-9 - Title**\r\n  body\r\n");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Title");
        assert_eq!(items[0].body, "body");
    }

    const REQUIREMENTS: &str = "\
## R-ID: Identity

- **R-ID-1 (must).** First body
  continued here.

- **R-LATER-1.** No parenthesis, later area.
- **R-LATER-3 (must, promoted 2026-10-15).** Promoted.
### Heading
- **R-LATER-4 (later).** Explicit later.
- **R-STO-7 (withdrawn).** Gone.
  still part
- **R-ENT-13 (reopened, pending ANA-38).** Reopened.
not a continuation
- **R-ID-2 (must)** missing dot: not a header
- **R-X-1.** No parenthesis, another area.
";

    fn bullet(
        key: &str,
        priority: Priority,
        state: RequirementState,
        body: &str,
        line: usize,
    ) -> RequirementBullet {
        let (area, number) = key["R-".len()..].rsplit_once('-').expect("a key");
        RequirementBullet {
            key: key.into(),
            area_code: area.into(),
            number: number.parse().expect("a number"),
            priority,
            state,
            body: body.into(),
            line,
        }
    }

    #[test]
    fn requirements_cover_every_header_shape() {
        use Priority::{Later, Must};
        use RequirementState::{Active, Withdrawn};
        assert_eq!(
            parse_requirements(REQUIREMENTS),
            [
                bullet("R-ID-1", Must, Active, "First body\ncontinued here.", 3),
                bullet("R-LATER-1", Later, Active, "No parenthesis, later area.", 6),
                bullet("R-LATER-3", Must, Active, "Promoted.", 7),
                bullet("R-LATER-4", Later, Active, "Explicit later.", 9),
                bullet("R-STO-7", Must, Withdrawn, "Gone.\nstill part", 10),
                bullet("R-ENT-13", Must, Active, "Reopened.", 12),
                bullet("R-X-1", Must, Active, "No parenthesis, another area.", 15),
            ]
        );
    }

    fn header(key: &str, title: &str, resolution: Resolution) -> Option<DecisionHeader> {
        Some(DecisionHeader {
            key: key.into(),
            title: title.into(),
            resolution,
        })
    }

    #[test]
    fn decision_headers_cover_every_shape_at_the_pin() {
        use Resolution::{Concluded, Done, Rejected};
        let cases = [
            (
                "# MOD-1 - Built it (done, 2026-09-03)",
                header("MOD-1", "Built it", Done),
            ),
            (
                "# ANA-4 - Looked into it (concluded, 2026-09-04)",
                header("ANA-4", "Looked into it", Concluded),
            ),
            (
                "# TOOL-7 - Containerized sandboxes (shipped, 2026-09-29)",
                header("TOOL-7", "Containerized sandboxes", Done),
            ),
            (
                "# MOD-61 - Folded into MOD-10 (decided, 2026-09-29)",
                header("MOD-61", "Folded into MOD-10", Done),
            ),
            (
                "# ANA-14 - Research Redis (rejected, 2026-09-19)",
                header("ANA-14", "Research Redis", Rejected),
            ),
            (
                "# ANA-10 - Local-only mode (done; verdict withdrawn by MOD-25, 2026-09-08)",
                header("ANA-10", "Local-only mode", Done),
            ),
            (
                "# ANA-3 - External context tools (status, 2026-09-14)",
                header("ANA-3", "External context tools", Done),
            ),
            (
                "# MOD-35: Add Qdrant connection settings",
                header("MOD-35", "Add Qdrant connection settings", Done),
            ),
            (
                "# TOOL-5 - Dev Postgres Crash Fix",
                header("TOOL-5", "Dev Postgres Crash Fix", Done),
            ),
            (
                "# MOD-8 - Split the store (from ANA-9) (done, 2026-09-10)",
                header("MOD-8", "Split the store (from ANA-9)", Done),
            ),
            (
                "# MOD-9 - Nested (a (b) c) group (concluded (late), 2026)",
                header("MOD-9", "Nested (a (b) c) group", Concluded),
            ),
            (
                "# MOD-11 - Ends in a key (MOD-12)",
                header("MOD-11", "Ends in a key (MOD-12)", Done),
            ),
            ("# Not a key - title", None),
            ("# MOD-13 title without separator", None),
            ("# MOD-14x - suffix after the number", None),
            ("MOD-15 - no heading marker", None),
            ("", None),
        ];
        for (line, want) in cases {
            assert_eq!(parse_decision_header(line), want, "{line:?}");
        }
    }

    #[test]
    fn resolution_of_falls_back_to_done() {
        assert_eq!(resolution_of("shipped"), Resolution::Done);
        assert_eq!(resolution_of("decided"), Resolution::Done);
        assert_eq!(resolution_of("status"), Resolution::Done);
        assert_eq!(resolution_of("rejected"), Resolution::Rejected);
        assert_eq!(resolution_of("concluded"), Resolution::Concluded);
        assert_eq!(resolution_of("withdrawn"), Resolution::Withdrawn);
        assert_eq!(resolution_of("done"), Resolution::Done);
    }

    #[test]
    fn bench_uuid_is_deterministic_distinct_and_version_8() {
        assert_eq!(bench_uuid("project"), bench_uuid("project"));
        assert_ne!(bench_uuid("project"), bench_uuid("user"));
        assert_ne!(bench_uuid("item:MOD-1"), bench_uuid("item:DEC:MOD-1"));
        assert_eq!(bench_uuid("project").get_version_num(), 8);
        assert_eq!(bench_uuid("item:MOD-99").get_version_num(), 8);
        // Pinned: a change here moves every point of every corpus.
        assert_eq!(
            bench_uuid("project").to_string(),
            "43187e92-94e1-80c3-84cb-fc9ead8ba12e"
        );
        assert_eq!(
            bench_uuid("item:MOD-99").to_string(),
            "6d1ecf70-643b-836a-8403-aec3419c4eb3"
        );
    }

    fn fixture_sources() -> Sources {
        let long_section = "word ".repeat(500);
        Sources {
            commit: Some("abc123".into()),
            handoff: "- [ ] **MOD-99 - Open one**\n  Its body.\n\n- [ ] **TOOL-13 - Open two**\n"
                .into(),
            requirements: "- **R-STO-8 (must).** Store things.\n- **R-LATER-2.** Later.\n".into(),
            decisions: vec![
                (
                    "docs/decisions/ana/ana-4.md".into(),
                    "# ANA-4 - An analysis (concluded, 2026-09-01)\n\nShort.\n".into(),
                ),
                (
                    "docs/decisions/mod/mod-41.md".into(),
                    format!(
                        "# MOD-41 - A module (rejected, 2026-09-02)\n\n## One\n\n{long_section}\n\n## Two\n\n{long_section}\n\n{long_section}\n"
                    ),
                ),
                (
                    "docs/decisions/tool/tool-9.md".into(),
                    "Untitled notes without a key\n\nbody\n".into(),
                ),
            ],
        }
    }

    #[test]
    fn build_from_sources_makes_docs_rows_and_points() {
        let sources = fixture_sources();
        let corpus = build_from_sources("abc123", sources.clone()).expect("builds");

        assert_eq!(corpus.corpus_ref, "abc123");
        assert_eq!(corpus.commit.as_deref(), Some("abc123"));
        assert_eq!(corpus.project, ProjectId::from_uuid(bench_uuid("project")));

        let ids: Vec<&str> = corpus.docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "MOD-99",
                "TOOL-13",
                "R-STO-8",
                "R-LATER-2",
                "DEC:ANA-4",
                "DEC:MOD-41",
                "DEC:TOOL-9"
            ]
        );
        let kinds: Vec<DocKind> = corpus.docs.iter().map(|d| d.kind).collect();
        assert_eq!(
            kinds,
            [
                DocKind::OpenItem,
                DocKind::OpenItem,
                DocKind::Requirement,
                DocKind::Requirement,
                DocKind::Decision,
                DocKind::Decision,
                DocKind::Decision
            ]
        );
        assert_eq!(corpus.ids().len(), 7);
        assert!(corpus.ids().contains(&CorpusId::decision("ANA-4")));
        assert_eq!(
            corpus
                .doc(&CorpusId::new("R-STO-8"))
                .map(|d| d.key.as_str()),
            Some("R-STO-8")
        );
        assert!(corpus.doc(&CorpusId::new("ANA-4")).is_none());

        // Items: open, then decisions; their doc text is the point text.
        assert_eq!(corpus.items.len(), 5);
        for (i, item) in corpus.items.iter().enumerate() {
            let doc = corpus
                .doc(&CorpusId::of_item(item))
                .expect("a doc per item");
            if item.resolution.is_none() {
                assert_eq!(doc.text, item_point(&corpus.items[i]).text);
            }
            assert_eq!(item.project_id, corpus.project);
            assert_eq!(item.priority, 0);
            assert_eq!(item.version, 1);
            assert_eq!(item.created_at, bench_time());
            assert_eq!(item.updated_at, bench_time());
        }
        let open = &corpus.items[0];
        assert_eq!(open.id, ItemId::from_uuid(bench_uuid("item:MOD-99")));
        assert_eq!((open.key_prefix.as_str(), open.key_number), ("MOD", 99));
        assert_eq!(open.title, "Open one");
        assert_eq!(open.body, "Its body.");
        assert_eq!(open.status, Status::Open);
        assert_eq!(open.resolution, None);
        assert_eq!(open.closed_at, None);
        assert_eq!(open.kind_id, ItemKindId::from_uuid(bench_uuid("kind:MOD")));
        assert_eq!(open.created_by, UserId::from_uuid(bench_uuid("user")));
        assert_eq!(corpus.docs[0].source, "HANDOFF.md:1");
        assert_eq!(corpus.docs[1].source, "HANDOFF.md:4");
        assert_eq!(corpus.docs[0].title, "Open one");

        let decision = &corpus.items[3];
        assert_eq!(decision.key, "MOD-41");
        assert_eq!(
            decision.id,
            ItemId::from_uuid(bench_uuid("item:DEC:MOD-41"))
        );
        assert_eq!(decision.status, Status::Closed);
        assert_eq!(decision.resolution, Some(Resolution::Rejected));
        assert_eq!(decision.closed_at, Some(bench_time()));
        assert_eq!(decision.body, "");
        assert_eq!(decision.title, "A module");
        assert_eq!(corpus.items[2].resolution, Some(Resolution::Concluded));
        // A header without a key: the upper-cased file stem keys it, resolution `Done`.
        assert_eq!(corpus.items[4].key, "TOOL-9");
        assert_eq!(corpus.items[4].resolution, Some(Resolution::Done));

        // Decision docs: the file verbatim, the path as source.
        let dec = corpus.doc(&CorpusId::decision("MOD-41")).expect("decision");
        assert_eq!(dec.text, sources.decisions[1].1);
        assert_eq!(dec.source, "docs/decisions/mod/mod-41.md");
        assert_eq!(dec.title, "A module");
        assert_eq!(dec.key, "MOD-41");

        // Requirements.
        assert_eq!(corpus.requirements.len(), 2);
        for (row, doc) in corpus.requirements.iter().zip(&corpus.docs[2..4]) {
            assert_eq!(doc.text, requirement_point(row).text);
            assert_eq!(doc.title, "");
            assert_eq!(row.rationale, "");
            assert_eq!(row.version, 1);
        }
        let sto = &corpus.requirements[0];
        assert_eq!(sto.id.0, bench_uuid("requirement:R-STO-8"));
        assert_eq!(sto.area_id.0, bench_uuid("area:STO"));
        assert_eq!((sto.area_code.as_str(), sto.number), ("STO", 8));
        assert_eq!(sto.priority, Priority::Must);
        assert_eq!(corpus.requirements[1].priority, Priority::Later);
        assert_eq!(corpus.docs[3].source, "docs/REQUIREMENTS.md:2");

        // Documents: one summary per decision item, in order.
        assert_eq!(corpus.documents.len(), 3);
        for (document, item) in corpus.documents.iter().zip(&corpus.items[2..]) {
            assert_eq!(document.item_id, item.id);
            assert_eq!(document.kind, "summary");
            assert_eq!(document.version, 1);
            assert_eq!(document.title, item.title);
            assert_eq!(document.produced_by_step_id, None);
            assert_eq!(
                document.id.0,
                bench_uuid(&format!("document:DEC:{}", item.key))
            );
        }
        assert_eq!(corpus.documents[1].body, sources.decisions[1].1);

        // Points: one per item, >= 1 per decision summary, one per requirement; IDs unique.
        let points = corpus.points();
        let counts = corpus.point_counts();
        assert_eq!(counts.item, 5);
        assert_eq!(counts.requirement, 2);
        assert!(counts.document >= 3 + 2, "{counts:?}");
        assert_eq!(
            points.len(),
            counts.item + counts.requirement + counts.document
        );
        let types: Vec<PointType> = points.iter().map(|p| p.point_type()).collect();
        let first_doc = types
            .iter()
            .position(|t| *t == PointType::Document)
            .expect("document points");
        assert_eq!(first_doc, 5, "items first");
        assert!(
            types[first_doc..first_doc + counts.document]
                .iter()
                .all(|t| *t == PointType::Document)
        );
        assert!(
            types[first_doc + counts.document..]
                .iter()
                .all(|t| *t == PointType::Requirement)
        );
        let unique: BTreeSet<Uuid> = points.iter().map(|p| p.id()).collect();
        assert_eq!(unique.len(), points.len());

        // Same sources, same points.
        let again = build_from_sources("abc123", sources).expect("builds");
        assert_eq!(again, corpus);
        assert_eq!(again.points(), points);
    }

    #[test]
    fn a_decision_file_without_a_key_is_an_error_naming_it() {
        let mut sources = fixture_sources();
        sources
            .decisions
            .push(("docs/decisions/notes.md".into(), "no key\n".into()));
        let err = build_from_sources("abc123", sources)
            .expect_err("unkeyable")
            .to_string();
        assert!(err.contains("docs/decisions/notes.md"), "{err}");
    }

    #[test]
    fn a_duplicate_corpus_id_is_an_error() {
        let mut sources = fixture_sources();
        sources.decisions.push((
            "docs/decisions/ana/ana-4-again.md".into(),
            "# ANA-4 - Twice (done, 2026-09-01)\n".into(),
        ));
        let err = build_from_sources("abc123", sources)
            .expect_err("duplicate")
            .to_string();
        assert!(err.contains("DEC:ANA-4"), "{err}");
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "htui-bench-corpus-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn read_worktree_walks_decisions_sorted_by_slash_path() {
        let root = scratch_dir("worktree");
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            std::fs::write(path, text).expect("write");
        };
        write("HANDOFF.md", "handoff");
        write("docs/REQUIREMENTS.md", "requirements");
        write("docs/decisions/mod/mod-2.md", "# MOD-2 - b");
        write("docs/decisions/ana/ana-1.md", "# ANA-1 - a");
        write("docs/decisions/ana/deep/ana-3.md", "# ANA-3 - c");
        write("docs/decisions/README.txt", "not markdown");

        let sources = read_worktree(&root);
        let _ = std::fs::remove_dir_all(&root);
        let sources = sources.expect("reads");

        assert_eq!(sources.commit, None);
        assert_eq!(sources.handoff, "handoff");
        assert_eq!(sources.requirements, "requirements");
        let paths: Vec<&str> = sources.decisions.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            [
                "docs/decisions/ana/ana-1.md",
                "docs/decisions/ana/deep/ana-3.md",
                "docs/decisions/mod/mod-2.md"
            ]
        );
        assert_eq!(sources.decisions[2].1, "# MOD-2 - b");
    }

    #[test]
    fn build_reads_the_work_tree_for_worktree() {
        let root = scratch_dir("build");
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            std::fs::write(path, text).expect("write");
        };
        write("HANDOFF.md", "- [ ] **MOD-99 - Open one**\n");
        write(
            "docs/REQUIREMENTS.md",
            "- **R-STO-8 (must).** Store things.\n",
        );
        write(
            "docs/decisions/ana/ana-4.md",
            "# ANA-4 - An analysis (concluded, 2026-09-01)\n",
        );

        let corpus = build(&root, WORKTREE);
        let _ = std::fs::remove_dir_all(&root);
        let corpus = corpus.expect("builds");

        assert_eq!(corpus.corpus_ref, WORKTREE);
        assert_eq!(corpus.commit, None);
        let ids: Vec<&str> = corpus.docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["MOD-99", "R-STO-8", "DEC:ANA-4"]);
    }

    #[test]
    fn read_worktree_without_a_handoff_is_an_error() {
        let root = scratch_dir("empty");
        let result = read_worktree(&root);
        let _ = std::fs::remove_dir_all(&root);
        let err = result.expect_err("no HANDOFF.md").to_string();
        assert!(err.contains("HANDOFF.md"), "{err}");
    }

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn git_works() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success())
    }

    #[test]
    fn an_unreachable_rev_names_the_rev() {
        if !git_works() {
            eprintln!("skipped: git not available");
            return;
        }
        let rev = "0123456789abcdef0123456789abcdef01234567";
        let err = read_git(&repo_root(), rev)
            .expect_err("unreachable")
            .to_string();
        assert!(err.contains(rev), "{err}");
        assert!(
            err.contains("unreachable in this clone (shallow?); pass --corpus-ref"),
            "{err}"
        );
        let err = read_git(&repo_root(), "--help").expect_err("an option is not a rev");
        assert!(err.to_string().contains("--help"), "{err}");
    }

    #[test]
    fn the_pinned_corpus_is_ana37s() {
        let repo = repo_root();
        if resolve_commit(&repo, DEFAULT_PIN).is_err() {
            eprintln!("skipped: {DEFAULT_PIN} unreachable");
            return;
        }
        let corpus = build_from_git(&repo, DEFAULT_PIN).expect("the pin builds");
        assert_eq!(corpus.corpus_ref, DEFAULT_PIN);
        let commit = corpus.commit.as_deref().expect("a commit");
        assert!(
            commit.starts_with(DEFAULT_PIN) && commit.len() == 40,
            "{commit}"
        );

        let count = |kind: DocKind| corpus.docs.iter().filter(|d| d.kind == kind).count();
        assert_eq!(count(DocKind::OpenItem), 71);
        assert_eq!(count(DocKind::Requirement), 121);
        assert_eq!(count(DocKind::Decision), 130);

        let ids = corpus.ids();
        assert_eq!(ids.len(), 322);
        for target in [
            "MOD-99",
            "MOD-101",
            "MOD-97",
            "TOOL-13",
            "MOD-106",
            "MOD-36",
            "MOD-8",
            "MOD-125",
            "MOD-108",
            "ANA-42",
            "R-ORCH-15",
            "MOD-130",
            "DEC:ANA-4",
            "R-TUI-18",
            "DEC:MOD-41",
            "R-ORCH-7",
            "ANA-33",
            "MOD-94",
            "MOD-98",
            "R-ID-4",
            "R-NF-3",
            "R-STO-8",
        ] {
            assert!(ids.contains(&CorpusId::new(target)), "{target}");
        }

        // F1: the six requirement header shapes.
        let requirement = |key: &str| {
            corpus
                .requirements
                .iter()
                .find(|r| r.key == key)
                .unwrap_or_else(|| panic!("{key}"))
        };
        assert_eq!(requirement("R-STO-7").state, RequirementState::Withdrawn);
        assert_eq!(requirement("R-ENT-13").state, RequirementState::Active);
        assert_eq!(requirement("R-ENT-13").priority, Priority::Must);
        assert_eq!(requirement("R-LATER-3").priority, Priority::Must);
        assert_eq!(requirement("R-LATER-7").priority, Priority::Must);
        let later = corpus
            .requirements
            .iter()
            .filter(|r| r.priority == Priority::Later)
            .count();
        assert_eq!(later, 13);
        assert!(
            corpus
                .requirements
                .iter()
                .filter(|r| r.area_code == "LATER" && r.number != 3 && r.number != 7)
                .all(|r| r.priority == Priority::Later)
        );

        // F2: every decision header keys its file, and agrees with the file stem.
        for doc in corpus.docs.iter().filter(|d| d.kind == DocKind::Decision) {
            let stem = Path::new(&doc.source)
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("a stem")
                .to_ascii_uppercase();
            assert_eq!(doc.key, stem, "{}", doc.source);
        }
        let closed = |resolution: Resolution| {
            corpus
                .items
                .iter()
                .filter(|i| i.resolution == Some(resolution))
                .count()
        };
        assert_eq!(closed(Resolution::Concluded), 20);
        assert_eq!(closed(Resolution::Rejected), 1);
        assert_eq!(closed(Resolution::Done), 98 + 3 + 3 + 5);

        let counts = corpus.point_counts();
        eprintln!(
            "pinned corpus points: item {}, requirement {}, document {} (F3 expects 777), total {}",
            counts.item,
            counts.requirement,
            counts.document,
            counts.item + counts.requirement + counts.document
        );
        assert_eq!(counts.item, 201);
        assert_eq!(counts.requirement, 121);
        assert!(counts.document >= 700, "{counts:?}");
        let points = corpus.points();
        let unique: BTreeSet<Uuid> = points.iter().map(|p| p.id()).collect();
        assert_eq!(unique.len(), points.len());
    }
}
