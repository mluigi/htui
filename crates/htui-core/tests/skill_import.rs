//! The frontmatter reader and the import mapping, against the shapes real `SKILL.md` files use.
//!
//! Every fixture in `the_reader_accepts_the_shapes_real_files_use` is copied from a file on the
//! maintainer's machine, and the test names where each came from. That is the point: the reader's
//! specification is the corpus, not the reader's author.

use htui_core::model::frontmatter::{FrontmatterError, Value, split};

// --- The five real shapes ----------------------------------------------------------------------

/// `/home/mluigi/projects/htui/.claude/skills/handoff-run/SKILL.md` — two keys, `name` plain and
/// `description` double-quoted, the shape every skill in this repository has.
const HANDOFF_RUN: &str = r#"---
name: handoff-run
description: "Run a HANDOFF.md item through its full lifecycle from one command: select (explicit ID, or `next` to resolve the logically next item and ask the maintainer on a tie), route (PRD/plan/ANA), confirm with maintainer."
---

# handoff-run

Body.
"#;

/// `…/agent-toolkit-for-aws/skills/core-skills/aws-containers/SKILL.md` — an unquoted
/// multi-sentence description on one line, a bare scalar, and a nested map with a quoted value.
const AWS_CONTAINERS: &str = r#"---
name: aws-containers
description: Builds and deploys containerized workloads on Elastic Kubernetes Service (EKS), Elastic Container Service (ECS), Fargate, and ECR. Covers general EKS knowledge, Karpenter, AWS Load Balancer Controller.
allowed-tools: Read
metadata:
  version: "2"
---

# aws-containers
"#;

/// `…/agent-toolkit-for-aws/plugins/aws-agents/skills/agents-get-started/SKILL.md` — a folded
/// description, a space-separated tool list with no commas, a four-key `metadata:`, and a
/// comparison string that begins with `>` and would read as a block-scalar indicator to a naive
/// reader.
const AGENTS_GET_STARTED: &str = r#"---
name: agents-get-started
description: >
  Use when a developer wants to create a new agent project or get started
  with AgentCore. Handles framework selection, project scaffolding, first
  deploy, and first invocation. Triggers on: "build an agent", "create an
  agent", "get started", "new project", "agentcore create".
  Not for adding capabilities to existing projects — use agents-build
  or agents-connect.
allowed-tools: Read Grep Glob Bash
metadata:
  type: skill
  version: "1.0.0"
  author: aws-agentcore
  requires-cli: ">=0.9.0"
---

# agents-get-started
"#;

/// `…/cloudflare/1.0.0/rules/workers.mdc` — a Cursor rules file: **no `name` key at all**, a bare
/// `false`, and a block list of quoted globs.
const CURSOR_WORKERS: &str = r#"---
description: When building applications on Cloudflare Workers or with frameworks that deploy to Cloudflare Workers, strongly prefer retrieval and fetching documentation over training data.
alwaysApply: false
globs:
  - "**/*.ts"
  - "**/*.tsx"
  - "wrangler.toml"
---

# Workers
"#;

/// `…/vikingbot/workspace/skills/tmux/SKILL.md` — the single-line flow map holding a nested JSON
/// document, at 63 measured `SKILL.md` paths. A reader that handles only block nesting truncates
/// this at the first `}`.
const FLOW_MAP_METADATA: &str = r#"---
name: tmux
description: Work with tmux.
metadata: {"vikingbot":{"emoji":"x","os":["darwin","linux"],"requires":{"bins":["tmux"]}}}
---

# tmux
"#;

#[test]
fn the_reader_accepts_the_shapes_real_files_use() {
    // 1. two keys, one of them quoted.
    let parsed = split(HANDOFF_RUN).expect("a fenced file parses");
    assert_eq!(parsed.scalar("name"), Some("handoff-run"));
    assert_eq!(
        parsed.scalar("description"),
        Some(
            "Run a HANDOFF.md item through its full lifecycle from one command: select (explicit \
             ID, or `next` to resolve the logically next item and ask the maintainer on a tie), \
             route (PRD/plan/ANA), confirm with maintainer."
        ),
        "a quoted scalar keeps its colons and commas"
    );
    assert_eq!(
        parsed.frontmatter.len(),
        2,
        "no phantom entries: {parsed:?}"
    );
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);

    // 2. an unquoted multi-sentence description, a bare scalar, a nested map.
    let parsed = split(AWS_CONTAINERS).expect("a fenced file parses");
    assert!(
        parsed
            .scalar("description")
            .is_some_and(|text| text.contains("Elastic Kubernetes Service (EKS)")),
        "an unquoted scalar runs to the end of its line"
    );
    assert_eq!(parsed.scalar("allowed-tools"), Some("Read"));
    assert_eq!(
        parsed.get("metadata").map(|entry| &entry.value),
        Some(&Value::Raw("version: \"2\"".to_owned())),
        "a nested map is kept verbatim, not parsed into entries"
    );
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);

    // 3. a folded description, a space-separated list, a four-key map.
    let parsed = split(AGENTS_GET_STARTED).expect("a fenced file parses");
    let description = parsed.scalar("description").expect("a folded description");
    assert!(
        description.starts_with("Use when a developer wants to create a new agent project"),
        "a folded block is re-joined with spaces: {description:?}"
    );
    assert!(
        description.contains("Triggers on: \"build an agent\""),
        "a folded line's own colon-space survives: {description:?}"
    );
    assert!(
        description
            .trim_end()
            .ends_with("agents-build or agents-connect."),
        "the final line is folded in too: {description:?}"
    );
    assert!(
        description.ends_with('\n'),
        "and `>` clips to exactly one trailing newline, as YAML says: {description:?}"
    );
    assert_eq!(parsed.scalar("allowed-tools"), Some("Read Grep Glob Bash"));
    assert_eq!(
        parsed.get("metadata").map(|entry| &entry.value),
        Some(&Value::Raw(
            "type: skill\nversion: \"1.0.0\"\nauthor: aws-agentcore\nrequires-cli: \">=0.9.0\""
                .to_owned()
        )),
        "a nested map keeps all four keys, and a value starting with `>` is not a block scalar"
    );
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);

    // 4. no `name` at all, a bare `false`, quoted block-list globs.
    let parsed = split(CURSOR_WORKERS).expect("a fenced file parses");
    assert!(!parsed.has("name"), "this file genuinely has no name key");
    assert_eq!(parsed.flag("alwaysApply"), Some(false));
    assert_eq!(
        parsed.list("globs"),
        Some(vec![
            "**/*.ts".to_owned(),
            "**/*.tsx".to_owned(),
            "wrangler.toml".to_owned()
        ])
    );
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);

    // 5. the flow map.
    let parsed = split(FLOW_MAP_METADATA).expect("a fenced file parses");
    assert_eq!(
        parsed.get("metadata").map(|entry| &entry.value),
        Some(&Value::Raw(
            r#"{"vikingbot":{"emoji":"x","os":["darwin","linux"],"requires":{"bins":["tmux"]}}}"#
                .to_owned()
        )),
        "a single-line flow map round-trips byte for byte"
    );
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
}

#[test]
fn a_nested_map_is_kept_verbatim() {
    let text = "---\nmetadata:\n  a: 1\n  b:\n    c: 2\n---\nbody\n";
    let parsed = split(text).expect("a fenced file parses");
    assert_eq!(
        parsed.get("metadata").map(|entry| &entry.value),
        Some(&Value::Raw("a: 1\nb:\n  c: 2".to_owned())),
        "indentation is stripped relative to the nested block, not to the key"
    );
    assert!(
        parsed.frontmatter.len() == 1,
        "the nested keys are not entries of their own: {parsed:?}"
    );
}

#[test]
fn a_block_list_is_read_at_either_indent_width() {
    // 2-space, as the firecrawl files use it.
    let two = split("---\nallowed-tools:\n  - Read\n  - Bash\n---\n").expect("parses");
    // 4-space, as `…/discover-resources/SKILL.md` uses it.
    let four = split("---\nallowed-tools:\n    - Read\n    - Bash\n---\n").expect("parses");
    assert_eq!(
        two.get("allowed-tools").map(|entry| &entry.value),
        Some(&Value::List(vec!["Read".to_owned(), "Bash".to_owned()]))
    );
    assert_eq!(
        four.get("allowed-tools").map(|entry| &entry.value),
        two.get("allowed-tools").map(|entry| &entry.value),
        "indent width is never assumed"
    );
}

#[test]
fn the_reader_never_coerces_a_yaml_one_one_word() {
    let parsed = split("---\nname: no\ndescription: on\n---\n").expect("parses");
    assert_eq!(
        parsed.scalar("name"),
        Some("no"),
        "a name reading `no` is a name"
    );
    assert_eq!(parsed.scalar("description"), Some("on"));
    assert_eq!(parsed.flag("name"), None, "`no` is not a boolean here");
    assert_eq!(
        parsed.flag("description"),
        None,
        "`on` is not a boolean here"
    );

    let parsed = split("---\nuser-invocable: false\n---\n").expect("parses");
    assert_eq!(parsed.flag("user-invocable"), Some(false), "but `false` is");
}

#[test]
fn a_description_holding_colon_space_is_one_value() {
    // The scalar case: the key ends at the first `: `, and the value keeps its own.
    let parsed = split("---\ndescription: Triggers on: \"build an agent\"\n---\n").expect("parses");
    assert_eq!(
        parsed.scalar("description"),
        Some(r#"Triggers on: "build an agent""#)
    );

    // And the awkward one: no space after the colon at all. The key still ends there.
    let parsed = split("---\ndescription:When building, prefer docs\n---\n").expect("parses");
    assert_eq!(
        parsed.scalar("description"),
        Some("When building, prefer docs")
    );
}

#[test]
fn a_rejected_key_is_reported_with_its_byte_and_its_line_and_the_body_still_imports() {
    let text = "---\nname: fine\ndescription: \"an unterminated quote\nlicense: MIT\n---\n# The body\n\nText.\n";
    let parsed = split(text).expect("a fenced file parses");

    assert_eq!(
        parsed.scalar("name"),
        Some("fine"),
        "the other keys still parse"
    );
    assert_eq!(
        parsed.scalar("license"),
        Some("MIT"),
        "the keys after it too"
    );
    assert_eq!(parsed.issues.len(), 1, "{:?}", parsed.issues);
    let issue = &parsed.issues[0];
    assert_eq!(issue.key, "description");
    assert_eq!(issue.line, 3, "1-based, the line the key is on");
    assert_eq!(
        issue.at,
        text.find("description").expect("the key is in the text")
    );
    assert!(
        issue.message.contains("does not close"),
        "the message says what happened: {issue}"
    );
    assert!(
        parsed.body.starts_with("# The body"),
        "the body is untouched by a frontmatter problem: {:?}",
        parsed.body
    );
    assert!(
        parsed.body.ends_with("Text.\n"),
        "and keeps its trailing newline"
    );
}

#[test]
fn a_line_that_is_neither_key_nor_item_is_reported_and_kept() {
    let parsed = split("---\nname: fine\njust some prose\n---\nbody\n").expect("parses");
    assert_eq!(parsed.scalar("name"), Some("fine"));
    assert_eq!(parsed.issues.len(), 1, "{:?}", parsed.issues);
    assert!(
        parsed.issues[0]
            .message
            .contains("neither a key nor a list item")
    );
}

#[test]
fn a_list_item_with_no_key_above_it_is_reported() {
    let parsed = split("---\nname: fine\n- orphan\n---\nbody\n").expect("parses");
    assert_eq!(parsed.issues.len(), 1, "{:?}", parsed.issues);
    assert!(parsed.issues[0].message.contains("no key above it"));
}

#[test]
fn an_unterminated_fence_is_refused() {
    // Synthetic: no frontmatter file measured on this machine needs it, so the variant exists to
    // keep `split` total rather than because a real file prompted it.
    let text = "---\nname: fine\ndescription: one\n".to_owned();
    assert_eq!(split(&text), Err(FrontmatterError::Unterminated { at: 0 }));

    let long = format!("---\n{}\n---\n", "k: v\n".repeat(MAX_LINES + 10));
    assert!(matches!(
        split(&long),
        Err(FrontmatterError::Unterminated { .. })
    ));
}

const MAX_LINES: usize = 250;

#[test]
fn the_opening_fence_must_be_the_first_line() {
    assert_eq!(
        split("# A heading\n\n---\nname: x\n---\n"),
        Err(FrontmatterError::NoFence)
    );
    assert_eq!(
        split("\n---\nname: x\n---\n"),
        Err(FrontmatterError::NoFence)
    );

    // A leading BOM is skipped, because it is an encoding artefact and not the author.
    let with_bom = "\u{feff}---\nname: x\n---\nbody\n";
    let parsed = split(with_bom).expect("a BOM does not stop a file being a skill");
    assert_eq!(parsed.scalar("name"), Some("x"));

    // A `---` horizontal rule in the body is not a fence, and does not split the body.
    let text = "---\nname: x\n---\nintro\n\n---\n\nafter\n";
    let parsed = split(text).expect("parses");
    assert_eq!(parsed.body, "intro\n\n---\n\nafter\n");
}

#[test]
fn a_body_keeps_its_leading_blank_lines_and_ends_in_exactly_one_newline() {
    let parsed = split("---\nname: x\n---\n\n\n# Title\n\n\n\n").expect("parses");
    assert_eq!(
        parsed.body, "\n\n# Title\n",
        "one trailing newline, leading blanks kept"
    );

    let parsed = split("---\nname: x\n---\n").expect("an empty body is a body");
    assert_eq!(parsed.body, "");

    // CRLF is normalised, and the fence's own line ending is behind us either way.
    let parsed = split("---\r\nname: x\r\n---\r\n# Title\r\n").expect("parses");
    assert_eq!(parsed.scalar("name"), Some("x"));
    assert_eq!(parsed.body, "# Title\n");
}

#[test]
fn the_body_offset_points_at_the_body() {
    let text = "---\nname: x\n---\n# Title\n";
    let parsed = split(text).expect("parses");
    assert_eq!(
        &text[parsed.body_at..],
        parsed.body,
        "body_at indexes the file it read"
    );
}

#[test]
fn an_inline_list_and_a_comma_string_are_both_lists() {
    let parsed = split("---\nglobs: [\"**/*.ts\", \"**/*.rs\"]\ntools: Read, Bash, Grep\n---\n")
        .expect("parses");
    assert_eq!(
        parsed.list("globs"),
        Some(vec!["**/*.ts".to_owned(), "**/*.rs".to_owned()])
    );
    assert_eq!(
        parsed.list("tools"),
        Some(vec![
            "Read".to_owned(),
            "Bash".to_owned(),
            "Grep".to_owned()
        ])
    );
    assert_eq!(parsed.scalar("globs"), None, "a list is not a scalar");
    assert_eq!(parsed.list("absent"), None);
}

#[test]
fn a_space_separated_scalar_is_never_split() {
    // The hazard: splitting on whitespace would make this one bogus tool named
    // "Read Grep Glob Bash".
    let parsed = split("---\nallowed-tools: Read Grep Glob Bash\n---\n").expect("parses");
    assert_eq!(parsed.scalar("allowed-tools"), Some("Read Grep Glob Bash"));
    assert_eq!(
        parsed.list("allowed-tools"),
        Some(vec!["Read Grep Glob Bash".to_owned()]),
        "and `list` splits on commas only, so it stays one entry"
    );
}

#[test]
fn a_literal_block_scalar_keeps_its_newlines() {
    let parsed = split("---\ndescription: |\n  one\n  two\n---\n").expect("parses");
    assert_eq!(
        parsed.scalar("description"),
        Some("one\ntwo\n"),
        "`|` clips to one trailing newline"
    );

    let parsed = split("---\ndescription: |-\n  one\n  two\n---\n").expect("parses");
    assert_eq!(
        parsed.scalar("description"),
        Some("one\ntwo"),
        "`|-` strips it"
    );
}

#[test]
fn a_duplicate_key_keeps_both_and_the_first_one_wins() {
    let parsed = split("---\nname: first\nname: second\n---\n").expect("parses");
    assert_eq!(parsed.scalar("name"), Some("first"));
    assert_eq!(
        parsed.frontmatter.len(),
        2,
        "nothing is dropped: {parsed:?}"
    );
}

// --- ANA-22 §7.3's mapping ---------------------------------------------------------------------

use chrono::{TimeZone, Utc};
use htui_core::model::skill::Activation;
use htui_core::model::skill_import::{ParsedSkill, parse, prefill_from_source};

/// The import instant, so a batch shares one and the tests can pin `imported_at`.
fn at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0)
        .single()
        .expect("a valid timestamp")
}

fn import(text: &str) -> Result<ParsedSkill, String> {
    parse("/home/x/skills/demo/SKILL.md", "SKILL.md", text, at())
}

#[test]
fn the_mapping_writes_exactly_what_the_table_says() {
    // Row 1: `name` -> skill.name.
    let parsed =
        import("---\nname: rust-style\ndescription: House style.\n---\n# Body\n").expect("parses");
    assert_eq!(parsed.name, "rust-style");
    assert_eq!(parsed.description, "House style.");
    assert_eq!(parsed.body, "# Body\n");

    // Row 2: `when_to_use` is appended after a blank line.
    let parsed = import(
        "---\nname: a\ndescription: House style.\nwhen_to_use: On any Rust change.\n---\nB\n",
    )
    .expect("parses");
    assert_eq!(parsed.description, "House style.\n\nOn any Rust change.");

    // Row 3: the body after the closing fence, kept exactly — including the blank line the file
    // puts between its fence and its first heading.
    assert_eq!(
        import(HANDOFF_RUN).expect("parses").body,
        "\n# handoff-run\n\nBody.\n",
        "a body's leading blank lines are part of it"
    );
}

/// §7.3's first prefill row, one case per ecosystem key.
#[test]
fn the_glob_keys_prefill_glob_activation() {
    for line in [
        "paths:\n  - \"src/**/*.rs\"",
        "globs:\n  - \"src/**/*.ts\"",
        "fileMatchPattern:\n  - \"**/*.py\"",
        "applyTo: \"src/**\"",
    ] {
        let text = format!("---\nname: a\ndescription: d\n{line}\n---\nB\n");
        let parsed = parse("/x/SKILL.md", "SKILL.md", &text, at()).expect("parses");
        assert_eq!(
            parsed.prefill.activation,
            Some(Activation::Glob),
            "{line} is a glob source"
        );
        assert!(!parsed.prefill.globs.is_empty(), "{line} carried no globs");
    }

    // A comma string is split; a list is taken as it is.
    let text = "---\nname: a\ndescription: d\npaths: \"src/**, lib/**\"\n---\nB\n";
    let parsed = parse("/x/SKILL.md", "SKILL.md", text, at()).expect("parses");
    assert_eq!(
        parsed.prefill.globs,
        vec!["src/**".to_owned(), "lib/**".to_owned()]
    );
}

/// §7.3's second prefill row: the four always-sources, including `applyTo: "**"`, which the table
/// gives a second, different meaning from the same key in the row above.
#[test]
fn the_always_keys_prefill_always_activation() {
    for line in [
        "alwaysApply: true",
        "trigger: always_on",
        "inclusion: always",
        "applyTo: \"**\"",
    ] {
        let text = format!("---\nname: a\ndescription: d\n{line}\n---\nB\n");
        let parsed = parse("/x/SKILL.md", "SKILL.md", &text, at()).expect("parses");
        assert_eq!(
            parsed.prefill.activation,
            Some(Activation::Always),
            "{line} is an always-source"
        );
        assert!(
            parsed.prefill.globs.is_empty(),
            "{line} is not a glob source"
        );
    }
}

/// The precedence §7.3 leaves open: an always-source beats a glob-source, so a Cursor file that
/// carries both is prefilled `always` and not silently narrowed to `glob`.
#[test]
fn an_always_source_beats_a_glob_source() {
    let text = "---\nname: a\ndescription: d\nalwaysApply: true\nglobs:\n  - \"**/*.ts\"\n---\nB\n";
    let parsed = parse("/x/SKILL.md", "SKILL.md", text, at()).expect("parses");
    assert_eq!(parsed.prefill.activation, Some(Activation::Always));
    assert_eq!(
        parsed.prefill.globs,
        vec!["**/*.ts".to_owned()],
        "the globs are still kept"
    );
}

#[test]
fn the_language_key_is_ours_and_passes_through() {
    let text = "---\nname: a\ndescription: d\nlanguages: [rust, sql]\n---\nB\n";
    let parsed = parse("/x/SKILL.md", "SKILL.md", text, at()).expect("parses");
    assert_eq!(
        parsed.prefill.languages,
        vec!["rust".to_owned(), "sql".to_owned()]
    );
    assert_eq!(
        parsed.prefill.activation,
        Some(Activation::Always),
        "a language is not an activation rule, so §7.3's description-only row applies — and \
         `always` is the column default, so the form is unchanged"
    );
    assert!(parsed.prefill.hint.is_some(), "and the hint says why");
}

/// §7.3's last prefill row: description-only, model-decided, manual, or not-invocable all become
/// `always` here, and the hint names which.
#[test]
fn a_file_with_no_activation_rule_prefills_always_with_a_hint() {
    let cases = [
        (
            "---\nname: a\ndescription: d\n---\nB\n",
            "names no activation rule",
        ),
        (
            "---\nname: a\ndescription: d\ntrigger: model_decision\n---\nB\n",
            "lets the model decide",
        ),
        (
            "---\nname: a\ndescription: d\ninclusion: manual\n---\nB\n",
            "included manually",
        ),
        (
            "---\nname: a\ndescription: d\ndisable-model-invocation: true\n---\nB\n",
            "opts out of model invocation",
        ),
    ];
    for (text, expected) in cases {
        let parsed = parse("/x/SKILL.md", "SKILL.md", text, at()).expect("parses");
        assert_eq!(
            parsed.prefill.activation,
            Some(Activation::Always),
            "{text}"
        );
        assert!(
            parsed
                .prefill
                .hint
                .is_some_and(|hint| hint.contains(expected)),
            "{text} hinted {:?}, expected one naming {expected:?}",
            parsed.prefill.hint
        );
    }
}

/// §7.3's last row: everything, verbatim, into `skill_version.source.frontmatter`.
#[test]
fn the_source_keeps_the_whole_frontmatter_verbatim() {
    let parsed = import(CURSOR_WORKERS).expect("parses");
    let source = &parsed.source;
    let mut keys: Vec<&str> = source
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["format", "frontmatter", "imported_at", "issues", "path"],
        "the key set is a contract with the attachments matrix, which re-reads `frontmatter`"
    );
    assert_eq!(
        source["format"],
        serde_json::json!("skill-md"),
        "the file's shape"
    );
    assert_eq!(
        source["path"],
        serde_json::json!("/home/x/skills/demo/SKILL.md")
    );
    assert_eq!(
        source["imported_at"],
        serde_json::json!("2026-09-28T12:00:00+00:00")
    );

    // The Cursor file has no `name`, and the whole of what it does have survives.
    let frontmatter = &source["frontmatter"];
    assert_eq!(frontmatter["alwaysApply"], serde_json::json!("false"));
    assert_eq!(
        frontmatter["globs"],
        serde_json::json!(["**/*.ts", "**/*.tsx", "wrangler.toml"])
    );

    // A format of `mdc` for the same file read as a rules file.
    let parsed =
        parse("/x/rules/workers.mdc", "workers.mdc", CURSOR_WORKERS, at()).expect("parses");
    assert_eq!(parsed.source["format"], serde_json::json!("mdc"));
    let parsed = parse("/x/rules/notes.md", "notes.md", CURSOR_WORKERS, at()).expect("parses");
    assert_eq!(parsed.source["format"], serde_json::json!("markdown"));
}

/// The T1 ↔ T3 contract: the prefill the attachments matrix re-derives from a stored `source` is
/// the same one the import used. If this drifts, an imported skill attaches with the wrong
/// activation and nobody notices until a step does not fire.
#[test]
fn prefill_from_source_is_the_same_derivation_the_import_used() {
    for text in [
        CURSOR_WORKERS,
        HANDOFF_RUN,
        AGENTS_GET_STARTED,
        FLOW_MAP_METADATA,
        "---\nname: a\ndescription: d\ntrigger: model_decision\n---\nB\n",
        "---\nname: a\ndescription: d\nlanguages: [rust]\n---\nB\n",
        "---\nname: a\ndescription: d\n---\nB\n",
    ] {
        let parsed = import(text).expect("parses");
        assert_eq!(
            prefill_from_source(&parsed.source),
            parsed.prefill,
            "the stored source re-derives the prefill it was imported with: {text}"
        );
    }
}

#[test]
fn a_skill_with_no_source_prefills_nothing() {
    assert_eq!(
        prefill_from_source(&serde_json::json!({})),
        htui_core::model::skill_import::ImportPrefill::default()
    );
    assert_eq!(
        prefill_from_source(&serde_json::json!({"format": "skill-md"})),
        htui_core::model::skill_import::ImportPrefill::default(),
        "a source with no frontmatter prefills nothing rather than guessing"
    );
}

#[test]
fn the_name_falls_back_to_the_directory_then_the_stem() {
    // A `SKILL.md` with no `name` takes its parent directory's name.
    let text = "---\ndescription: d\n---\nB\n";
    let parsed = parse("/x/skills/rust-style/SKILL.md", "SKILL.md", text, at()).expect("parses");
    assert_eq!(parsed.name, "rust-style");

    // A rules file with no `name` takes its stem. The real fixture is the Cloudflare `workers.mdc`.
    let parsed =
        parse("/x/rules/workers.mdc", "workers.mdc", CURSOR_WORKERS, at()).expect("parses");
    assert_eq!(parsed.name, "workers");

    // Copilot's `<name>.instructions.md` names the rule `<name>`, and a snake-case stem takes the
    // one spelling the Agent Skills rule allows. Both are file-naming conventions, not typos.
    let parsed = parse(
        "/x/.github/instructions/rust.instructions.md",
        "rust.instructions.md",
        "---\napplyTo: \"**/*.rs\"\n---\nB\n",
        at(),
    )
    .expect("parses");
    assert_eq!(parsed.name, "rust");
    let parsed = parse(
        "/x/.cursor/rules/api_style.mdc",
        "api_style.mdc",
        CURSOR_WORKERS,
        at(),
    )
    .expect("parses");
    assert_eq!(parsed.name, "api-style");

    // A declared name always wins.
    let parsed = parse(
        "/x/rules/workers.mdc",
        "workers.mdc",
        "---\nname: real\n---\nB\n",
        at(),
    )
    .expect("parses");
    assert_eq!(parsed.name, "real");
}

#[test]
fn a_refused_name_carries_the_writers_own_sentence() {
    let err = import("---\nname: My Skill\ndescription: d\n---\nB\n").expect_err("refused");
    assert_eq!(err, invalid_skill_name_for("My Skill"));
    assert!(
        err.contains("1-64 of a-z, 0-9 and single inner hyphens"),
        "{err}"
    );

    // Nothing is slugified: a name the maintainer did not write is not imported under.
    let err = import("---\ndescription: d\n---\nB\n").and_then(|_| {
        parse(
            "/x/My Skill.md",
            "My Skill.md",
            "---\ndescription: d\n---\nB\n",
            at(),
        )
    });
    assert!(
        err.is_err(),
        "a stem that breaks the rule is refused, not rewritten"
    );
}

fn invalid_skill_name_for(name: &str) -> String {
    htui_core::store::traits::invalid_skill_name(name)
}

#[test]
fn a_file_that_is_not_a_skill_file_is_refused_in_the_readers_own_words() {
    let err = import("# A heading\n\nSome prose.\n").expect_err("refused");
    assert!(err.contains("`---` fence"), "{err}");
}

#[test]
fn an_unreadable_key_is_carried_into_the_source() {
    let text = "---\nname: a\ndescription: \"unterminated\n---\nB\n";
    let parsed = import(text).expect("the body still imports");
    assert_eq!(parsed.issues.len(), 1, "{:?}", parsed.issues);
    assert_eq!(
        parsed.source["issues"][0]["key"],
        serde_json::json!("description")
    );
    assert!(parsed.source["issues"][0]["message"].is_string());
    assert_eq!(parsed.body, "B\n", "and the body is whole");
}
