//! The action catalogue: every named action, its context, defaults and help (ANA-26 §7.2).
//!
//! MOD-67 M1 (plan D4, D12) fills the global, overlay and shared contexts. Global and overlay
//! rows reproduce today's `Keymap::default_global` bindings, plus `f1` for help (ANA-26 §6.5).
//! Each shared-context row (`list`, `pane`, `confirm`, `form`, `common`) takes its defaults from
//! today's match arms and cites the arm it mirrors. No view consumes a shared context until M3,
//! so those rows change no behaviour yet. M3-M5 append each view's context in its own block.
//!
//! Defaults are strict spec strings (blueprint B4): lower-case key names (`tab`, `esc`, `pgdn`,
//! `f1`), the character itself for a character (`G`, `]`), and `ctrl-x` for a ctrl chord. This
//! module does not parse them; `keys::Keys::defaults` does, and its tests pin that every one
//! parses. `ctrl-c` is never a default: it is fixed (`keys::chord::CTRL_C`).
//!
//! `in_capture` is a property of an action (blueprint B3). It means "offered while a text field
//! captures", so every default and every user chord of the action must be non-printable
//! (`KeyChord::is_printable`; M2's validator). The global layer is **not** marked. Under a
//! capturing mode (M3) the global layer is filtered chord by chord, and printable chords drop
//! out. So `global.help` (`?`, `f1`) reaches help through `f1` while a field types `?`, and
//! `global.quit` (`q`) is unreachable there. `ctrl-c` quits regardless.
//!
//! MOD-67 M3 appends the Settings tab's context, one context per Settings section and one per
//! overlay, in strip order (plan D1). A view context holds the view's own verbs; the shared verbs
//! it inherits through `Layer::view` may be overridden per view (D10), and [`VIEW_DEFAULTS`] adds
//! the extra chords a view takes today on a shared act (D12 as amended by PA-1).
//!
//! MOD-67 M4 appends the Skills and Requirements contexts (plan D1): `skills`, a shared context
//! holding the verbs Library and Templates share (the view switch, the version cursor, base,
//! diff, `E`, `ctrl-g`), then one view context per Skills view or pane and `requirements`.

use Context::{
    Common, Concepts, Confirm, Editor, Form, Global, List, Overlay, Pane, Requirements, Settings,
    SettingsAgents, SettingsBoxes, SettingsConnection, SettingsHierarchy, SettingsKinds,
    SettingsPersonas, SettingsSecrets, Skills, SkillsAttach, SkillsHelp, SkillsLibrary,
    SkillsTemplates, Switcher, Waiting,
};

/// A key context: a TOML table of `keys.toml` and a layer of a context stack (ANA-26 §7.2-§7.3).
/// M1 has the global, overlay and shared contexts; M3 appends the Settings and overlay view
/// contexts; M4 the Skills and Requirements ones; M5 appends its own (`BacklogRuns`, ...), each in
/// its own block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// `[global]`: reachable from every screen, checked last.
    Global,
    /// `[overlay]`: every overlay's own keys, ahead of the modal swallow.
    Overlay,
    /// `[list]`: moving a cursor through rows.
    List,
    /// `[pane]`: scrolling a read-only pane and cycling its sub-tabs.
    Pane,
    /// `[confirm]`: answering a yes/no question.
    Confirm,
    /// `[form]`: moving between a form's fields and saving it; reachable while a field captures.
    Form,
    /// `[common]`: verbs every view that offers them shares (edit, new, delete, ...).
    Common,
    /// `[editor]`: the in-pane editor's own keys (MOD-57). Focused, only `editor.focus`
    /// resolves; unfocused, the M1 lock (`Stack`).
    Editor,
    /// `[settings]`: the Settings tab's own keys (section cycling).
    Settings,
    /// `[settings.agents]`: the Agents section.
    SettingsAgents,
    /// `[settings.hierarchy]`: the Hierarchy section.
    SettingsHierarchy,
    /// `[settings.kinds]`: the Kinds section.
    SettingsKinds,
    /// `[settings.prompt]`: the Prompt section (no own action; overrides only).
    SettingsPrompt,
    /// `[settings.connection]`: the Connection section.
    SettingsConnection,
    /// `[settings.qdrant]`: the Qdrant section (no own action; overrides only).
    SettingsQdrant,
    /// `[settings.boxes]`: the Boxes section.
    SettingsBoxes,
    /// `[settings.personas]`: the Personas section.
    SettingsPersonas,
    /// `[settings.secrets]`: the Secrets section.
    SettingsSecrets,
    /// `[settings.queue]`: the Queue section (no own action; overrides and view defaults).
    SettingsQueue,
    /// `[concepts]`: the concepts search overlay.
    Concepts,
    /// `[switcher]`: the workspace switcher overlay.
    Switcher,
    /// `[migration]`: the schema migration prompt (no own action; view defaults only).
    Migration,
    /// `[waiting]`: the waiting list overlay.
    Waiting,
    /// `[skills]`: the verbs the Skills tab's Library and Templates views share (MOD-67 M4 D1);
    /// a shared context, like `settings`, so each view's table may override them.
    Skills,
    /// `[skills.library]`: the Library view.
    SkillsLibrary,
    /// `[skills.templates]`: the Templates view.
    SkillsTemplates,
    /// `[skills.attach]`: the Library's attachments pane.
    SkillsAttach,
    /// `[skills.help]`: the editors' agent help (prompt, wait and proposal).
    SkillsHelp,
    /// `[requirements]`: the Requirements tab.
    Requirements,
}

impl Context {
    /// Every context in table order: the key file's tables and `--print-keys`' order (MOD-67
    /// M3, L-B Q5). The catalogue's blocks follow it.
    pub const ALL: &'static [Self] = &[
        Self::Global,
        Self::Overlay,
        Self::List,
        Self::Pane,
        Self::Confirm,
        Self::Form,
        Self::Common,
        Self::Editor,
        Self::Settings,
        Self::SettingsAgents,
        Self::SettingsHierarchy,
        Self::SettingsKinds,
        Self::SettingsPrompt,
        Self::SettingsConnection,
        Self::SettingsQdrant,
        Self::SettingsBoxes,
        Self::SettingsPersonas,
        Self::SettingsSecrets,
        Self::SettingsQueue,
        Self::Concepts,
        Self::Switcher,
        Self::Migration,
        Self::Waiting,
        Self::Skills,
        Self::SkillsLibrary,
        Self::SkillsTemplates,
        Self::SkillsAttach,
        Self::SkillsHelp,
        Self::Requirements,
    ];

    /// The TOML table name: `global`, `overlay`, `list`, ..., `settings.agents`, `concepts`.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Overlay => "overlay",
            Self::List => "list",
            Self::Pane => "pane",
            Self::Confirm => "confirm",
            Self::Form => "form",
            Self::Common => "common",
            Self::Editor => "editor",
            Self::Settings => "settings",
            Self::SettingsAgents => "settings.agents",
            Self::SettingsHierarchy => "settings.hierarchy",
            Self::SettingsKinds => "settings.kinds",
            Self::SettingsPrompt => "settings.prompt",
            Self::SettingsConnection => "settings.connection",
            Self::SettingsQdrant => "settings.qdrant",
            Self::SettingsBoxes => "settings.boxes",
            Self::SettingsPersonas => "settings.personas",
            Self::SettingsSecrets => "settings.secrets",
            Self::SettingsQueue => "settings.queue",
            Self::Concepts => "concepts",
            Self::Switcher => "switcher",
            Self::Migration => "migration",
            Self::Waiting => "waiting",
            Self::Skills => "skills",
            Self::SkillsLibrary => "skills.library",
            Self::SkillsTemplates => "skills.templates",
            Self::SkillsAttach => "skills.attach",
            Self::SkillsHelp => "skills.help",
            Self::Requirements => "requirements",
        }
    }

    /// The `?` box heading: `Global`, `Overlay`, ..., a section's or an overlay's title
    /// (`Connection`, `Workspaces`, `Waiting on you`).
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Overlay => "Overlay",
            Self::List => "List",
            Self::Pane => "Pane",
            Self::Confirm => "Confirm",
            Self::Form => "Form",
            Self::Common => "Common",
            Self::Editor => "Editor",
            Self::Settings => "Settings",
            Self::SettingsAgents => "Agents",
            Self::SettingsHierarchy => "Hierarchy",
            Self::SettingsKinds => "Kinds",
            Self::SettingsPrompt => "Prompt",
            Self::SettingsConnection => "Connection",
            Self::SettingsQdrant => "Qdrant",
            Self::SettingsBoxes => "Boxes",
            Self::SettingsPersonas => "Personas",
            Self::SettingsSecrets => "Secrets",
            Self::SettingsQueue => "Queue",
            Self::Concepts => "Search concepts",
            Self::Switcher => "Workspaces",
            Self::Migration => "Schema",
            Self::Waiting => "Waiting on you",
            Self::Skills => "Skills",
            Self::SkillsLibrary => "Library",
            Self::SkillsTemplates => "Templates",
            Self::SkillsAttach => "Attachments",
            Self::SkillsHelp => "Agent help",
            Self::Requirements => "Requirements",
        }
    }

    /// A view's own context (a Settings section, an overlay, a Skills view or pane, the
    /// Requirements tab): hosts view verbs, D10 overrides and [`VIEW_DEFAULTS`]; skipped by the
    /// validator's per-context pass (MOD-67 M3 PA-4).
    #[must_use]
    pub const fn is_view(self) -> bool {
        matches!(
            self,
            Self::SettingsAgents
                | Self::SettingsHierarchy
                | Self::SettingsKinds
                | Self::SettingsPrompt
                | Self::SettingsConnection
                | Self::SettingsQdrant
                | Self::SettingsBoxes
                | Self::SettingsPersonas
                | Self::SettingsSecrets
                | Self::SettingsQueue
                | Self::Concepts
                | Self::Switcher
                | Self::Migration
                | Self::Waiting
                | Self::SkillsLibrary
                | Self::SkillsTemplates
                | Self::SkillsAttach
                | Self::SkillsHelp
                | Self::Requirements
        )
    }

    /// A shared context a view layer may inherit and override (MOD-67 D10, PA-3): `list`,
    /// `pane`, `confirm`, `form`, `common`, `settings`, `skills` (M4).
    #[must_use]
    pub const fn is_shared(self) -> bool {
        matches!(
            self,
            Self::List
                | Self::Pane
                | Self::Confirm
                | Self::Form
                | Self::Common
                | Self::Settings
                | Self::Skills
        )
    }
}

/// A named action: one per meaning, not per letter (ANA-26 §6.2). Views (M3-M5) match on it
/// instead of `KeyCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Act {
    /// `global.quit`: leave htui.
    Quit,
    /// `global.next_tab`: the tab to the right.
    NextTab,
    /// `global.prev_tab`: the tab to the left.
    PrevTab,
    /// `global.select_tab_1`: the first tab.
    SelectTab1,
    /// `global.select_tab_2`: the second tab.
    SelectTab2,
    /// `global.select_tab_3`: the third tab.
    SelectTab3,
    /// `global.select_tab_4`: the fourth tab.
    SelectTab4,
    /// `global.select_tab_5`: the fifth tab.
    SelectTab5,
    /// `global.select_tab_6`: the sixth tab.
    SelectTab6,
    /// `global.select_tab_7`: the seventh tab.
    SelectTab7,
    /// `global.select_tab_8`: the eighth tab.
    SelectTab8,
    /// `global.select_tab_9`: the ninth tab.
    SelectTab9,
    /// `global.help`: toggle the `?` box.
    Help,
    /// `global.workspaces`: open the workspace switcher (offered by `register_all`).
    Workspaces,
    /// `global.find`: open the concepts search (offered by `register_all`).
    Find,
    /// `global.waiting`: open the waiting list (offered by `register_all`).
    Waiting,
    /// `global.queue`: open the queue overlay (offered by `register_all`).
    Queue,
    /// `overlay.close`: close the top overlay.
    OverlayClose,
    /// `list.down`: the next row.
    ListDown,
    /// `list.up`: the previous row.
    ListUp,
    /// `list.top`: the first row.
    ListTop,
    /// `list.bottom`: the last row.
    ListBottom,
    /// `list.fold`: fold or unfold the row under the cursor.
    ListFold,
    /// `pane.scroll_down`: scroll the pane one line down.
    PaneScrollDown,
    /// `pane.scroll_up`: scroll the pane one line up.
    PaneScrollUp,
    /// `pane.page_down`: scroll the pane one page down.
    PanePageDown,
    /// `pane.page_up`: scroll the pane one page up.
    PanePageUp,
    /// `pane.next_subtab`: the next sub-tab of the pane.
    PaneNextSubtab,
    /// `pane.prev_subtab`: the previous sub-tab of the pane.
    PanePrevSubtab,
    /// `confirm.yes`: answer yes.
    ConfirmYes,
    /// `confirm.no`: answer no.
    ConfirmNo,
    /// `form.next_field`: focus the next field.
    FormNextField,
    /// `form.prev_field`: focus the previous field.
    FormPrevField,
    /// `form.save`: save the form.
    FormSave,
    /// `form.external_editor`: edit the focused field in `$EDITOR`.
    FormExternalEditor,
    /// `common.edit`: edit the selected item.
    Edit,
    /// `common.new`: create an item.
    New,
    /// `common.delete`: delete the selected item.
    Delete,
    /// `common.clear`: clear the selected value.
    Clear,
    /// `common.reload`: reload from the source.
    Reload,
    /// `common.back`: leave the view's inner level.
    Back,
    /// `common.dismiss`: clear the notice.
    Dismiss,
    /// `editor.focus`: give the keys to the in-pane editor, or take them back.
    EditorFocus,
    /// `editor.abort`: kill the in-pane editor; nothing is read back.
    EditorAbort,
    /// `settings.next_section`: the next Settings section.
    NextSection,
    /// `settings.prev_section`: the previous Settings section.
    PrevSection,
    /// `settings.agents.probe`: probe the selected agent's CLI.
    AgentsProbe,
    /// `settings.agents.install`: install the selected agent's CLI.
    AgentsInstall,
    /// `settings.agents.authenticate`: log the selected agent in.
    AgentsAuthenticate,
    /// `settings.agents.switch_box`: switch the agent's install target to this box.
    AgentsSwitchBox,
    /// `settings.agents.edit_paths`: edit the agent's paths.
    AgentsEditPaths,
    /// `settings.agents.open_link`: open the login link.
    AgentsOpenLink,
    /// `settings.agents.paste_redirect`: paste the login redirect.
    AgentsPasteRedirect,
    /// `settings.agents.cancel`: cancel the running install or login.
    AgentsCancel,
    /// `settings.agents.choose`: select the highlighted login method.
    AgentsChoose,
    /// `settings.hierarchy.new_workspace`: create a workspace.
    HierarchyNewWorkspace,
    /// `settings.hierarchy.primary`: make the selected checkout primary.
    HierarchyPrimary,
    /// `settings.hierarchy.choose_path`: choose a path with the picker.
    HierarchyChoosePath,
    /// `settings.hierarchy.infer`: infer the workspace's paths.
    HierarchyInfer,
    /// `settings.kinds.new_graph`: create a graph.
    KindsNewGraph,
    /// `settings.kinds.graph`: edit the selected kind's graph.
    KindsGraph,
    /// `settings.connection.rebuild`: rebuild the cache.
    ConnectionRebuild,
    /// `settings.connection.activate`: run the selected row's action (the Rebuild row).
    ConnectionActivate,
    /// `settings.boxes.edit_tags`: edit the box's tags.
    BoxesEditTags,
    /// `settings.boxes.edit_quirks`: edit the box's quirks.
    BoxesEditQuirks,
    /// `settings.boxes.executor`: set the box as the workspace executor.
    BoxesExecutor,
    /// `settings.boxes.probe`: probe the box.
    BoxesProbe,
    /// `settings.boxes.edit_spec`: edit the box's probe spec.
    BoxesEditSpec,
    /// `settings.personas.body`: edit the persona's body.
    PersonasBody,
    /// `settings.personas.rules`: edit the persona's rules.
    PersonasRules,
    /// `settings.personas.import`: import personas from a path.
    PersonasImport,
    /// `settings.secrets.check`: check the secrets backend.
    SecretsCheck,
    /// `concepts.decisions`: search decisions only.
    ConceptsDecisions,
    /// `concepts.project`: cycle the project scope.
    ConceptsProject,
    /// `concepts.reindex`: re-index the scope.
    ConceptsReindex,
    /// `concepts.up`: the previous hit.
    ConceptsUp,
    /// `concepts.down`: the next hit.
    ConceptsDown,
    /// `switcher.switch`: switch to the selected workspace.
    SwitcherSwitch,
    /// `waiting.open`: open the selected waiting step.
    WaitingOpen,
    /// `skills.switch_view`: toggle between the Library and Templates views.
    SkillsSwitchView,
    /// `skills.prev_version`: move the version cursor to the older version.
    SkillsPrevVersion,
    /// `skills.next_version`: move the version cursor to the newer version.
    SkillsNextVersion,
    /// `skills.base`: make the selected version the diff base.
    SkillsBase,
    /// `skills.diff`: diff the selected version against the base.
    SkillsDiff,
    /// `skills.edit_externally`: edit the selected item in `$EDITOR` from browse.
    SkillsEditExternally,
    /// `skills.ask_agent`: open the agent help over an editor (MOD-55).
    SkillsAskAgent,
    /// `skills.library.import`: import skills from a path.
    LibraryImport,
    /// `skills.library.info`: rename and describe the selected skill.
    LibraryInfo,
    /// `skills.library.attach`: open or close the attachments pane.
    LibraryAttach,
    /// `skills.templates.diff_default`: diff the template against its default.
    TemplatesDiffDefault,
    /// `skills.attach.choose`: edit the selected attachment, or insert the picked repo.
    AttachChoose,
    /// `skills.attach.detach`: detach the selected attachment.
    AttachDetach,
    /// `skills.attach.repo`: open the repo picker from the attachment form.
    AttachRepo,
    /// `skills.help.prev_agent`: the previous agent in the help prompt.
    SkillsHelpPrevAgent,
    /// `skills.help.next_agent`: the next agent in the help prompt.
    SkillsHelpNextAgent,
    /// `skills.help.accept`: accept the agent's proposal.
    SkillsHelpAccept,
    /// `skills.help.cancel`: cancel the agent's turn.
    SkillsHelpCancel,
    /// `requirements.new_area`: create an area.
    RequirementsNewArea,
    /// `requirements.amend`: amend the selected requirement.
    RequirementsAmend,
    /// `requirements.withdraw`: withdraw the selected requirement.
    RequirementsWithdraw,
    /// `requirements.filter`: filter the tree.
    RequirementsFilter,
}

impl Act {
    /// The tab index of `select_tab_1`..`_9` (0..=8); `None` for every other action.
    #[must_use]
    pub const fn tab_index(self) -> Option<usize> {
        Some(match self {
            Self::SelectTab1 => 0,
            Self::SelectTab2 => 1,
            Self::SelectTab3 => 2,
            Self::SelectTab4 => 3,
            Self::SelectTab5 => 4,
            Self::SelectTab6 => 5,
            Self::SelectTab7 => 6,
            Self::SelectTab8 => 7,
            Self::SelectTab9 => 8,
            _ => return None,
        })
    }

    /// This action's catalogue row. `None` only if a variant was added without a row, which
    /// the test `every_act_is_listed_and_has_a_row` forbids.
    #[must_use]
    pub fn spec(self) -> Option<&'static ActionSpec> {
        CATALOGUE.iter().find(|row| row.act == self)
    }
}

/// One catalogue row: the single source of truth for an action's name, defaults and help
/// (ANA-26 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSpec {
    /// The action.
    pub act: Act,
    /// Its context (TOML table).
    pub context: Context,
    /// Its key in that table, e.g. `"next_tab"`.
    pub name: &'static str,
    /// Default chords as strict spec strings, parsed by `keys::Keys::defaults`; `[]` is unbound.
    pub defaults: &'static [&'static str],
    /// The `?` box and status-line label, e.g. `"next tab"`.
    pub help: &'static str,
    /// Offered while a text field captures, so every chord must be non-printable (ANA-26 §6.4).
    pub in_capture: bool,
}

/// A row that is not offered while a text field captures.
const fn row(
    act: Act,
    context: Context,
    name: &'static str,
    defaults: &'static [&'static str],
    help: &'static str,
) -> ActionSpec {
    ActionSpec {
        act,
        context,
        name,
        defaults,
        help,
        in_capture: false,
    }
}

/// A row that stays offered while a text field captures (`in_capture`, blueprint B3).
const fn capture_row(
    act: Act,
    context: Context,
    name: &'static str,
    defaults: &'static [&'static str],
    help: &'static str,
) -> ActionSpec {
    ActionSpec {
        in_capture: true,
        ..row(act, context, name, defaults, help)
    }
}

/// Every action, grouped by context, in display order (the status line and the `?` box follow
/// it). M3-M5 append their contexts' blocks at the end.
pub static CATALOGUE: &[ActionSpec] = &[
    // [global]: today's `keymap.rs` `default_global` rows and `app/mod.rs` `register_all`
    // labels, in status-line order. `ctrl-c` is fixed (`CTRL_C`) and never listed.
    row(Act::Quit, Global, "quit", &["q"], "quit"),
    row(Act::NextTab, Global, "next_tab", &["tab"], "next tab"),
    row(
        Act::PrevTab,
        Global,
        "prev_tab",
        &["backtab"],
        "previous tab",
    ),
    row(
        Act::SelectTab1,
        Global,
        "select_tab_1",
        &["1"],
        "select tab",
    ),
    row(
        Act::SelectTab2,
        Global,
        "select_tab_2",
        &["2"],
        "select tab",
    ),
    row(
        Act::SelectTab3,
        Global,
        "select_tab_3",
        &["3"],
        "select tab",
    ),
    row(
        Act::SelectTab4,
        Global,
        "select_tab_4",
        &["4"],
        "select tab",
    ),
    row(
        Act::SelectTab5,
        Global,
        "select_tab_5",
        &["5"],
        "select tab",
    ),
    row(
        Act::SelectTab6,
        Global,
        "select_tab_6",
        &["6"],
        "select tab",
    ),
    row(
        Act::SelectTab7,
        Global,
        "select_tab_7",
        &["7"],
        "select tab",
    ),
    row(
        Act::SelectTab8,
        Global,
        "select_tab_8",
        &["8"],
        "select tab",
    ),
    row(
        Act::SelectTab9,
        Global,
        "select_tab_9",
        &["9"],
        "select tab",
    ),
    // `f1` is new (ANA-26 §6.5, D12); not `in_capture`, `?` is printable (B3).
    row(Act::Help, Global, "help", &["?", "f1"], "help"),
    // Offered by `register_all` (D5): `app/mod.rs` workspace switcher, concepts search,
    // waiting list (MOD-69 D7) and queue overlay (MOD-12 M3 D8). `queue` follows `waiting`, so
    // the status line a 100-column frame cuts before `Ctrl+w waiting` is unchanged.
    row(Act::Workspaces, Global, "workspaces", &["w"], "workspaces"),
    row(Act::Find, Global, "find", &["ctrl-f"], "find"),
    row(Act::Waiting, Global, "waiting", &["ctrl-w"], "waiting"),
    row(Act::Queue, Global, "queue", &["ctrl-q"], "queue"),
    // [overlay]: `keymap.rs`'s wildcard overlay row. In capture because the concepts field
    // returns `Pass` on `Esc` and this layer closes it (`concepts_search.rs:334-335`).
    capture_row(Act::OverlayClose, Overlay, "close", &["esc"], "close"),
    // [list]
    // backlog/mod.rs:655, requirements/mod.rs:430, connection.rs:811, kinds.rs:1492,
    // qdrant.rs:404, workspace_switcher.rs:163, waiting_list.rs:274. agents.rs:2420 and
    // hierarchy.rs:1212 take `j` only (ANA §6.6 adds `down`, M3).
    row(Act::ListDown, List, "down", &["j", "down"], "down"),
    // backlog/mod.rs:656, requirements/mod.rs:431, connection.rs:815, kinds.rs:1496,
    // boxes.rs:699, library.rs:689.
    row(Act::ListUp, List, "up", &["k", "up"], "up"),
    // backlog/mod.rs:657, requirements/mod.rs:432. kinds.rs:1476 binds `g` to open graph (M3).
    row(Act::ListTop, List, "top", &["g", "home"], "top"),
    // backlog/mod.rs:658, requirements/mod.rs:433.
    row(Act::ListBottom, List, "bottom", &["G", "end"], "bottom"),
    // backlog/mod.rs:684-689 (fold, else the detail pane), requirements/mod.rs:434. Elsewhere
    // `Enter` is a view verb (connection.rs:807, workspace_switcher.rs:171).
    row(Act::ListFold, List, "fold", &["enter"], "fold"),
    // [pane]
    // backlog/detail/mod.rs:447 (`Scroll::on_key`), requirements/mod.rs:435, library.rs:708,
    // templates.rs:528. runs.rs:1434 `J` is next step (a view verb, ANA §6.2).
    row(
        Act::PaneScrollDown,
        Pane,
        "scroll_down",
        &["J"],
        "scroll down",
    ),
    // backlog/detail/mod.rs:448.
    row(Act::PaneScrollUp, Pane, "scroll_up", &["K"], "scroll up"),
    // backlog/detail/mod.rs:449, graph.rs:697, divergence.rs:261.
    row(Act::PanePageDown, Pane, "page_down", &["pgdn"], "page down"),
    // backlog/detail/mod.rs:450.
    row(Act::PanePageUp, Pane, "page_up", &["pgup"], "page up"),
    // backlog/mod.rs:659. Settings cycles sections through `settings.next_section` (M3).
    row(
        Act::PaneNextSubtab,
        Pane,
        "next_subtab",
        &["l", "]", "right"],
        "next sub-tab",
    ),
    // backlog/mod.rs:660. Settings: `settings.prev_section` (M3).
    row(
        Act::PanePrevSubtab,
        Pane,
        "prev_subtab",
        &["h", "[", "left"],
        "previous sub-tab",
    ),
    // [confirm]
    // connection.rs:483, qdrant.rs:214, kinds.rs:715, hierarchy.rs:769, personas.rs:785,
    // boxes.rs:545, runs.rs:583, detail/requirements.rs:306. migration_prompt.rs:92 also
    // takes `Y`: a `VIEW_DEFAULTS` row.
    row(Act::ConfirmYes, Confirm, "yes", &["y"], "yes"),
    // connection.rs:487, qdrant.rs:218, kinds.rs:726, agents.rs:1161, boxes.rs:546,
    // personas.rs:791, runs.rs:587, detail/requirements.rs:317, hierarchy.rs:761.
    // migration_prompt.rs:97 also takes `N`: a `VIEW_DEFAULTS` row.
    row(Act::ConfirmNo, Confirm, "no", &["n", "esc"], "no"),
    // [form]: all in capture; every chord is named or ctrl.
    // item_form.rs:613, requirements/forms.rs:306, compose.rs:340. The Settings forms also
    // take `Down` (agents.rs:2166): `VIEW_DEFAULTS` rows (M3); attach.rs:488 in M4.
    capture_row(
        Act::FormNextField,
        Form,
        "next_field",
        &["tab"],
        "next field",
    ),
    // item_form.rs:617, requirements/forms.rs:310, compose.rs:344. The Settings forms also
    // take `Up`: `VIEW_DEFAULTS` rows.
    capture_row(
        Act::FormPrevField,
        Form,
        "prev_field",
        &["backtab"],
        "previous field",
    ),
    // text_area.rs:194, item_form.rs:275, requirements/mod.rs:233, attach.rs:477,
    // library.rs:990. Every site also accepts `ctrl-S` (blueprint F-7, M4). The Library,
    // Templates and Requirements editors resolve it after their widget passes the chord (MOD-67
    // M4 PA-3).
    capture_row(Act::FormSave, Form, "save", &["ctrl-s"], "save"),
    // item_form.rs:281 (label at :63), library.rs:1021, templates.rs:689. Same `E` alias.
    capture_row(
        Act::FormExternalEditor,
        Form,
        "external_editor",
        &["ctrl-e"],
        "$EDITOR",
    ),
    // [common]
    // connection.rs:779, qdrant.rs:382, kinds.rs:1468, hierarchy.rs:1241, prompt.rs:849,
    // agents.rs:2401, backlog/mod.rs:680. boxes.rs:707 `e` edits quirks (a view verb).
    row(Act::Edit, Common, "edit", &["e"], "edit"),
    // kinds.rs:1452, hierarchy.rs:1233, agents.rs:2395, library.rs:694, templates.rs:527.
    // backlog/mod.rs:679 uses `N`.
    row(Act::New, Common, "new", &["n"], "new"),
    // hierarchy.rs:1279, kinds.rs:1484.
    row(Act::Delete, Common, "delete", &["d"], "delete"),
    // connection.rs:785, qdrant.rs:388. detail/requirements.rs:481 `c` is the cite picker.
    row(Act::Clear, Common, "clear", &["c"], "clear"),
    // connection.rs:822, qdrant.rs:414, kinds.rs:1505, hierarchy.rs:1289, prompt.rs:869,
    // boxes.rs:733, requirements/mod.rs:449, library.rs:690, templates.rs:523, attach.rs:398.
    // agents.rs:2495 `r` probes (a view verb).
    row(Act::Reload, Common, "reload", &["r"], "reload"),
    // divergence.rs:245, library.rs:895, chat/mod.rs:233. personas.rs:841 also takes `Enter`:
    // a `VIEW_DEFAULTS` row.
    row(Act::Back, Common, "back", &["esc"], "back"),
    // connection.rs:828, boxes.rs:739, hierarchy.rs:1295, kinds.rs:1511, prompt.rs:875.
    // Shares `Esc` with `back`: `STATE_GUARDED`.
    row(Act::Dismiss, Common, "dismiss", &["esc"], "dismiss"),
    // [editor]: MOD-57 M1 (plan P4, P5). `ctrl-4` is what `ctrl-\` arrives as on unix (`chord.rs`
    // `legacy_arrival`; `ctrl-\` itself is refused as indistinguishable). On Windows crossterm
    // delivers `ctrl-\` as `Char('\\')` + CONTROL, so there only a physical `ctrl-4` matches
    // (MOD-16 H-10/H-11, T7's docs). In capture: while the editor is focused every other key,
    // `ctrl-c` included, is the editor's. The loader refuses a file that unbinds it (MOD-67 M4
    // D8.1), as it does `overlay.close`.
    capture_row(
        Act::EditorFocus,
        Editor,
        "focus",
        &["ctrl-4"],
        "editor focus",
    ),
    // Offered only while the editor is unfocused (`Stack::EDITOR_UNFOCUSED`): focused, `ctrl-x`
    // is nano's exit and goes to the editor.
    row(Act::EditorAbort, Editor, "abort", &["ctrl-x"], "abort edit"),
    // [settings]: the Settings tab's own section cycling (MOD-67 M3 D1). Every non-capturing
    // section stack carries this layer, so a section's view layer may override both.
    // settings/mod.rs:347.
    row(
        Act::NextSection,
        Settings,
        "next_section",
        &["l", "]", "right"],
        "next section",
    ),
    // settings/mod.rs:351.
    row(
        Act::PrevSection,
        Settings,
        "prev_section",
        &["h", "[", "left"],
        "previous section",
    ),
    // [settings.agents]
    // agents.rs:2479-2498 (`r` probes; not `common.reload`).
    row(Act::AgentsProbe, SettingsAgents, "probe", &["r"], "probe"),
    // agents.rs:2428.
    row(
        Act::AgentsInstall,
        SettingsAgents,
        "install",
        &["i"],
        "install",
    ),
    // agents.rs:2432.
    row(
        Act::AgentsAuthenticate,
        SettingsAgents,
        "authenticate",
        &["a"],
        "authenticate",
    ),
    // agents.rs:2407.
    row(
        Act::AgentsSwitchBox,
        SettingsAgents,
        "switch_box",
        &["t"],
        "this box",
    ),
    // agents.rs:2414.
    row(
        Act::AgentsEditPaths,
        SettingsAgents,
        "edit_paths",
        &["m"],
        "paths",
    ),
    // agents.rs:2438.
    row(
        Act::AgentsOpenLink,
        SettingsAgents,
        "open_link",
        &["o"],
        "open link",
    ),
    // agents.rs:2444.
    row(
        Act::AgentsPasteRedirect,
        SettingsAgents,
        "paste_redirect",
        &["p"],
        "paste redirect",
    ),
    // agents.rs:2454 (install), :2468 (login): one verb for both (L-A Q9).
    row(
        Act::AgentsCancel,
        SettingsAgents,
        "cancel",
        &["x"],
        "cancel",
    ),
    // agents.rs:640: the login chooser's `Enter` (L-A Q10).
    row(
        Act::AgentsChoose,
        SettingsAgents,
        "choose",
        &["enter"],
        "select",
    ),
    // [settings.hierarchy]
    // hierarchy.rs:1220.
    row(
        Act::HierarchyNewWorkspace,
        SettingsHierarchy,
        "new_workspace",
        &["N"],
        "new workspace",
    ),
    // hierarchy.rs:1250.
    row(
        Act::HierarchyPrimary,
        SettingsHierarchy,
        "primary",
        &["p"],
        "make primary",
    ),
    // hierarchy.rs:1258.
    row(
        Act::HierarchyChoosePath,
        SettingsHierarchy,
        "choose_path",
        &["b"],
        "choose path",
    ),
    // hierarchy.rs:1268.
    row(
        Act::HierarchyInfer,
        SettingsHierarchy,
        "infer",
        &["i"],
        "infer paths",
    ),
    // [settings.kinds]
    // kinds.rs:1460.
    row(
        Act::KindsNewGraph,
        SettingsKinds,
        "new_graph",
        &["N"],
        "new graph",
    ),
    // kinds.rs:1476: a view verb, not `list.top` (D2).
    row(
        Act::KindsGraph,
        SettingsKinds,
        "graph",
        &["g"],
        "edit graph",
    ),
    // [settings.connection]
    // connection.rs:802.
    row(
        Act::ConnectionRebuild,
        SettingsConnection,
        "rebuild",
        &["R"],
        "rebuild cache",
    ),
    // connection.rs:807: `Enter` runs the Rebuild row; declined on every other row (L-B Q8).
    row(
        Act::ConnectionActivate,
        SettingsConnection,
        "activate",
        &["enter"],
        "run row",
    ),
    // [settings.boxes]
    // boxes.rs:703.
    row(
        Act::BoxesEditTags,
        SettingsBoxes,
        "edit_tags",
        &["t"],
        "edit tags",
    ),
    // boxes.rs:707.
    row(
        Act::BoxesEditQuirks,
        SettingsBoxes,
        "edit_quirks",
        &["e"],
        "edit quirks",
    ),
    // boxes.rs:712-721: shares `w` with `global.workspaces` (`STATE_GUARDED`).
    row(
        Act::BoxesExecutor,
        SettingsBoxes,
        "executor",
        &["w"],
        "executor",
    ),
    // boxes.rs:727.
    row(Act::BoxesProbe, SettingsBoxes, "probe", &["p"], "probe"),
    // boxes.rs:723.
    row(
        Act::BoxesEditSpec,
        SettingsBoxes,
        "edit_spec",
        &["s"],
        "edit probe spec",
    ),
    // [settings.personas]
    // personas.rs:444 `'b'`.
    row(
        Act::PersonasBody,
        SettingsPersonas,
        "body",
        &["b"],
        "edit body",
    ),
    // personas.rs:444 `'r'`.
    row(
        Act::PersonasRules,
        SettingsPersonas,
        "rules",
        &["r"],
        "edit rules",
    ),
    // personas.rs:444 `'I'`.
    row(
        Act::PersonasImport,
        SettingsPersonas,
        "import",
        &["I"],
        "import",
    ),
    // [settings.secrets]
    // secrets.rs:1416.
    row(Act::SecretsCheck, SettingsSecrets, "check", &["t"], "check"),
    // [concepts]: all in capture, the query field types every printable chord.
    // concepts_search.rs:295.
    capture_row(
        Act::ConceptsDecisions,
        Concepts,
        "decisions",
        &["ctrl-d"],
        "decisions only",
    ),
    // concepts_search.rs:299.
    capture_row(
        Act::ConceptsProject,
        Concepts,
        "project",
        &["ctrl-p"],
        "cycle project scope",
    ),
    // concepts_search.rs:303.
    capture_row(
        Act::ConceptsReindex,
        Concepts,
        "reindex",
        &["ctrl-r"],
        "re-index scope",
    ),
    // concepts_search.rs:315.
    capture_row(Act::ConceptsUp, Concepts, "up", &["up"], "previous hit"),
    // concepts_search.rs:319.
    capture_row(Act::ConceptsDown, Concepts, "down", &["down"], "next hit"),
    // [switcher]
    // workspace_switcher.rs:171.
    row(
        Act::SwitcherSwitch,
        Switcher,
        "switch",
        &["enter"],
        "switch workspace",
    ),
    // [waiting]
    // waiting_list.rs:284.
    row(Act::WaitingOpen, Waiting, "open", &["enter"], "open step"),
    // [skills]: the verbs Library and Templates share (MOD-67 M4 D2), one row each, so
    // `[skills.library]` and `[skills.templates]` may override them per view.
    // skills/mod.rs:111-118: one toggle between two views (D2), so no next/prev pair.
    row(
        Act::SkillsSwitchView,
        Skills,
        "switch_view",
        &["h", "l", "[", "]", "left", "right"],
        "switch view",
    ),
    // library.rs:762, :799, templates.rs:576, :616 (`','`).
    row(
        Act::SkillsPrevVersion,
        Skills,
        "prev_version",
        &[","],
        "older version",
    ),
    // The same arms (`'.'`).
    row(
        Act::SkillsNextVersion,
        Skills,
        "next_version",
        &["."],
        "newer version",
    ),
    // library.rs:816, templates.rs:626.
    row(Act::SkillsBase, Skills, "base", &["b"], "diff base"),
    // library.rs:823, templates.rs:630.
    row(Act::SkillsDiff, Skills, "diff", &["d"], "diff"),
    // library.rs:843, templates.rs:657: browse `E`; the editor's `ctrl-e` is
    // `form.external_editor`.
    row(
        Act::SkillsEditExternally,
        Skills,
        "edit_externally",
        &["E"],
        "edit in $EDITOR",
    ),
    // library.rs:1080, templates.rs:741 (MOD-55): offered inside the editors.
    capture_row(
        Act::SkillsAskAgent,
        Skills,
        "ask_agent",
        &["ctrl-g"],
        "ask agent",
    ),
    // [skills.library]
    // library.rs:753.
    row(
        Act::LibraryImport,
        SkillsLibrary,
        "import",
        &["I"],
        "import",
    ),
    // library.rs:854 (`'i'`).
    row(Act::LibraryInfo, SkillsLibrary, "info", &["i"], "rename"),
    // library.rs:863 opens the pane, attach.rs:399 (`a`) closes it: the same toggle.
    row(
        Act::LibraryAttach,
        SkillsLibrary,
        "attach",
        &["a"],
        "attachments",
    ),
    // [skills.templates]
    // templates.rs:643.
    row(
        Act::TemplatesDiffDefault,
        SkillsTemplates,
        "diff_default",
        &["D"],
        "diff default",
    ),
    // [skills.attach]
    // attach.rs:368 (browse: edit the row), :562 (picker: insert the repo).
    row(
        Act::AttachChoose,
        SkillsAttach,
        "choose",
        &["enter"],
        "choose row",
    ),
    // attach.rs:379.
    row(Act::AttachDetach, SkillsAttach, "detach", &["x"], "detach"),
    // attach.rs:476: in the form, beside its text fields.
    capture_row(
        Act::AttachRepo,
        SkillsAttach,
        "repo",
        &["ctrl-r"],
        "repo picker",
    ),
    // [skills.help]: the editors' agent help (MOD-55). Its discard is `confirm.no`.
    // agent_help.rs:328 (`Up`): in the prompt, beside its text field.
    capture_row(
        Act::SkillsHelpPrevAgent,
        SkillsHelp,
        "prev_agent",
        &["up"],
        "previous agent",
    ),
    // agent_help.rs:328 (`Down`).
    capture_row(
        Act::SkillsHelpNextAgent,
        SkillsHelp,
        "next_agent",
        &["down"],
        "next agent",
    ),
    // agent_help.rs:294, and :308's `Enter`.
    row(
        Act::SkillsHelpAccept,
        SkillsHelp,
        "accept",
        &["enter", "y"],
        "accept",
    ),
    // agent_help.rs:273, :280.
    row(
        Act::SkillsHelpCancel,
        SkillsHelp,
        "cancel",
        &["esc"],
        "cancel the turn",
    ),
    // [requirements]
    // requirements/mod.rs:448 (`'a'`).
    row(
        Act::RequirementsNewArea,
        Requirements,
        "new_area",
        &["a"],
        "new area",
    ),
    // requirements/mod.rs:448 (`'e'`).
    row(
        Act::RequirementsAmend,
        Requirements,
        "amend",
        &["e"],
        "amend",
    ),
    // requirements/mod.rs:448 (`'W'`).
    row(
        Act::RequirementsWithdraw,
        Requirements,
        "withdraw",
        &["W"],
        "withdraw",
    ),
    // requirements/mod.rs:438.
    row(
        Act::RequirementsFilter,
        Requirements,
        "filter",
        &["/"],
        "filter",
    ),
];

/// Extra default chords a view adds to a shared act, on top of whatever the shared row resolves
/// to (MOD-67 M3 PA-1). `Keys` derives the view's row after the key file is merged; an explicit
/// `[<view>] <name>` line replaces it. Order: `Context::ALL`, then `Act` order. Each row cites
/// the arm it mirrors.
pub static VIEW_DEFAULTS: &[(Context, Act, &[&str])] = &[
    // agents.rs:2166, :2170: the create/edit and paths forms also move on `Down`/`Up`.
    (SettingsAgents, Act::FormNextField, &["down"]),
    (SettingsAgents, Act::FormPrevField, &["up"]),
    // hierarchy.rs:866, :870.
    (SettingsHierarchy, Act::FormNextField, &["down"]),
    (SettingsHierarchy, Act::FormPrevField, &["up"]),
    // kinds.rs:773, :777.
    (SettingsKinds, Act::FormNextField, &["down"]),
    (SettingsKinds, Act::FormPrevField, &["up"]),
    // personas.rs:1576, :1580.
    (SettingsPersonas, Act::FormNextField, &["down"]),
    (SettingsPersonas, Act::FormPrevField, &["up"]),
    // personas.rs:841: the import report also closes on `Enter`.
    (SettingsPersonas, Act::Back, &["enter"]),
    // secrets.rs:674, :675.
    (SettingsSecrets, Act::FormNextField, &["down"]),
    (SettingsSecrets, Act::FormPrevField, &["up"]),
    // queue.rs:617: `Enter` edits too.
    (Context::SettingsQueue, Act::Edit, &["enter"]),
    // migration_prompt.rs:92, :97: the capitals answer too.
    (Context::Migration, Act::ConfirmYes, &["Y"]),
    (Context::Migration, Act::ConfirmNo, &["N"]),
];

/// Default pairs in one declared stack where the narrower act always wins (ANA-26 §7.4 step 7,
/// "shadowing"; MOD-67 D11 as amended by PA-2): allowed only for a chord that is a default of
/// both, with the first act's layer narrower. Each entry is demanded by the compiled defaults
/// (`every_allow_list_entry_is_demanded`).
pub static SHADOWING: &[(Act, Act)] = &[
    // migration_prompt.rs:97 + VIEW_DEFAULTS: `[migration] no` derives ["n", "esc", "N"], and
    // `Esc` is also overlay.close below it. Both close the prompt, nothing applied.
    (Act::ConfirmNo, Act::OverlayClose),
];

/// Default pairs that share a chord in one context or stack because a view accepts at most one
/// of them in any state and declines the other (ANA-26 §7.4 step 7, "state-guarded"). M2's
/// reviewed allow-list; each entry is demanded by the compiled defaults.
pub static STATE_GUARDED: &[(Act, Act)] = &[
    (Act::Back, Act::Dismiss),
    // boxes.rs:712-721: the executor question opens only over a listed box with a readable
    // list; otherwise `w` declines and falls through to the switcher (box_settings.rs:1356).
    (Act::BoxesExecutor, Act::Workspaces),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{Act, CATALOGUE, Context, STATE_GUARDED};

    /// `context`'s index in [`Context::ALL`]. No wildcard arm: a new context fails to compile
    /// here until it is listed, and then in `ALL` too, or `every_context_is_listed_once_in_all`
    /// fails.
    const fn context_position(context: Context) -> usize {
        match context {
            Context::Global => 0,
            Context::Overlay => 1,
            Context::List => 2,
            Context::Pane => 3,
            Context::Confirm => 4,
            Context::Form => 5,
            Context::Common => 6,
            Context::Editor => 7,
            Context::Settings => 8,
            Context::SettingsAgents => 9,
            Context::SettingsHierarchy => 10,
            Context::SettingsKinds => 11,
            Context::SettingsPrompt => 12,
            Context::SettingsConnection => 13,
            Context::SettingsQdrant => 14,
            Context::SettingsBoxes => 15,
            Context::SettingsPersonas => 16,
            Context::SettingsSecrets => 17,
            Context::SettingsQueue => 18,
            Context::Concepts => 19,
            Context::Switcher => 20,
            Context::Migration => 21,
            Context::Waiting => 22,
            Context::Skills => 23,
            Context::SkillsLibrary => 24,
            Context::SkillsTemplates => 25,
            Context::SkillsAttach => 26,
            Context::SkillsHelp => 27,
            Context::Requirements => 28,
        }
    }

    #[test]
    fn every_context_is_listed_once_in_all() {
        assert_eq!(Context::ALL.len(), 29);
        for (index, context) in Context::ALL.iter().enumerate() {
            assert_eq!(
                context_position(*context),
                index,
                "{context:?} is out of place in Context::ALL"
            );
        }
        let unique: HashSet<Context> = Context::ALL.iter().copied().collect();
        assert_eq!(
            unique.len(),
            Context::ALL.len(),
            "a context is listed twice"
        );
    }

    #[test]
    fn catalogue_blocks_follow_context_all() {
        let mut last = 0;
        for row in CATALOGUE {
            let here = context_position(row.context);
            assert!(here >= last, "{:?} is out of its context's block", row.act);
            last = here;
        }
    }

    #[test]
    fn shared_names_are_unique_across_shared_contexts() {
        let mut seen = HashSet::new();
        for row in CATALOGUE.iter().filter(|row| row.context.is_shared()) {
            assert!(
                seen.insert(row.name),
                "{} is a name in two shared contexts",
                row.name
            );
        }
    }

    /// Every [`Act`], in declaration order. [`position`] keeps it complete.
    const ALL: &[Act] = &[
        Act::Quit,
        Act::NextTab,
        Act::PrevTab,
        Act::SelectTab1,
        Act::SelectTab2,
        Act::SelectTab3,
        Act::SelectTab4,
        Act::SelectTab5,
        Act::SelectTab6,
        Act::SelectTab7,
        Act::SelectTab8,
        Act::SelectTab9,
        Act::Help,
        Act::Workspaces,
        Act::Find,
        Act::Waiting,
        Act::Queue,
        Act::OverlayClose,
        Act::ListDown,
        Act::ListUp,
        Act::ListTop,
        Act::ListBottom,
        Act::ListFold,
        Act::PaneScrollDown,
        Act::PaneScrollUp,
        Act::PanePageDown,
        Act::PanePageUp,
        Act::PaneNextSubtab,
        Act::PanePrevSubtab,
        Act::ConfirmYes,
        Act::ConfirmNo,
        Act::FormNextField,
        Act::FormPrevField,
        Act::FormSave,
        Act::FormExternalEditor,
        Act::Edit,
        Act::New,
        Act::Delete,
        Act::Clear,
        Act::Reload,
        Act::Back,
        Act::Dismiss,
        Act::EditorFocus,
        Act::EditorAbort,
        Act::NextSection,
        Act::PrevSection,
        Act::AgentsProbe,
        Act::AgentsInstall,
        Act::AgentsAuthenticate,
        Act::AgentsSwitchBox,
        Act::AgentsEditPaths,
        Act::AgentsOpenLink,
        Act::AgentsPasteRedirect,
        Act::AgentsCancel,
        Act::AgentsChoose,
        Act::HierarchyNewWorkspace,
        Act::HierarchyPrimary,
        Act::HierarchyChoosePath,
        Act::HierarchyInfer,
        Act::KindsNewGraph,
        Act::KindsGraph,
        Act::ConnectionRebuild,
        Act::ConnectionActivate,
        Act::BoxesEditTags,
        Act::BoxesEditQuirks,
        Act::BoxesExecutor,
        Act::BoxesProbe,
        Act::BoxesEditSpec,
        Act::PersonasBody,
        Act::PersonasRules,
        Act::PersonasImport,
        Act::SecretsCheck,
        Act::ConceptsDecisions,
        Act::ConceptsProject,
        Act::ConceptsReindex,
        Act::ConceptsUp,
        Act::ConceptsDown,
        Act::SwitcherSwitch,
        Act::WaitingOpen,
        Act::SkillsSwitchView,
        Act::SkillsPrevVersion,
        Act::SkillsNextVersion,
        Act::SkillsBase,
        Act::SkillsDiff,
        Act::SkillsEditExternally,
        Act::SkillsAskAgent,
        Act::LibraryImport,
        Act::LibraryInfo,
        Act::LibraryAttach,
        Act::TemplatesDiffDefault,
        Act::AttachChoose,
        Act::AttachDetach,
        Act::AttachRepo,
        Act::SkillsHelpPrevAgent,
        Act::SkillsHelpNextAgent,
        Act::SkillsHelpAccept,
        Act::SkillsHelpCancel,
        Act::RequirementsNewArea,
        Act::RequirementsAmend,
        Act::RequirementsWithdraw,
        Act::RequirementsFilter,
    ];

    /// `act`'s index in [`ALL`]. No wildcard arm: a new variant fails to compile here until it is
    /// listed, and then in [`ALL`] too, or `every_act_is_listed_and_has_a_row` fails.
    const fn position(act: Act) -> usize {
        match act {
            Act::Quit => 0,
            Act::NextTab => 1,
            Act::PrevTab => 2,
            Act::SelectTab1 => 3,
            Act::SelectTab2 => 4,
            Act::SelectTab3 => 5,
            Act::SelectTab4 => 6,
            Act::SelectTab5 => 7,
            Act::SelectTab6 => 8,
            Act::SelectTab7 => 9,
            Act::SelectTab8 => 10,
            Act::SelectTab9 => 11,
            Act::Help => 12,
            Act::Workspaces => 13,
            Act::Find => 14,
            Act::Waiting => 15,
            Act::Queue => 16,
            Act::OverlayClose => 17,
            Act::ListDown => 18,
            Act::ListUp => 19,
            Act::ListTop => 20,
            Act::ListBottom => 21,
            Act::ListFold => 22,
            Act::PaneScrollDown => 23,
            Act::PaneScrollUp => 24,
            Act::PanePageDown => 25,
            Act::PanePageUp => 26,
            Act::PaneNextSubtab => 27,
            Act::PanePrevSubtab => 28,
            Act::ConfirmYes => 29,
            Act::ConfirmNo => 30,
            Act::FormNextField => 31,
            Act::FormPrevField => 32,
            Act::FormSave => 33,
            Act::FormExternalEditor => 34,
            Act::Edit => 35,
            Act::New => 36,
            Act::Delete => 37,
            Act::Clear => 38,
            Act::Reload => 39,
            Act::Back => 40,
            Act::Dismiss => 41,
            Act::EditorFocus => 42,
            Act::EditorAbort => 43,
            Act::NextSection => 44,
            Act::PrevSection => 45,
            Act::AgentsProbe => 46,
            Act::AgentsInstall => 47,
            Act::AgentsAuthenticate => 48,
            Act::AgentsSwitchBox => 49,
            Act::AgentsEditPaths => 50,
            Act::AgentsOpenLink => 51,
            Act::AgentsPasteRedirect => 52,
            Act::AgentsCancel => 53,
            Act::AgentsChoose => 54,
            Act::HierarchyNewWorkspace => 55,
            Act::HierarchyPrimary => 56,
            Act::HierarchyChoosePath => 57,
            Act::HierarchyInfer => 58,
            Act::KindsNewGraph => 59,
            Act::KindsGraph => 60,
            Act::ConnectionRebuild => 61,
            Act::ConnectionActivate => 62,
            Act::BoxesEditTags => 63,
            Act::BoxesEditQuirks => 64,
            Act::BoxesExecutor => 65,
            Act::BoxesProbe => 66,
            Act::BoxesEditSpec => 67,
            Act::PersonasBody => 68,
            Act::PersonasRules => 69,
            Act::PersonasImport => 70,
            Act::SecretsCheck => 71,
            Act::ConceptsDecisions => 72,
            Act::ConceptsProject => 73,
            Act::ConceptsReindex => 74,
            Act::ConceptsUp => 75,
            Act::ConceptsDown => 76,
            Act::SwitcherSwitch => 77,
            Act::WaitingOpen => 78,
            Act::SkillsSwitchView => 79,
            Act::SkillsPrevVersion => 80,
            Act::SkillsNextVersion => 81,
            Act::SkillsBase => 82,
            Act::SkillsDiff => 83,
            Act::SkillsEditExternally => 84,
            Act::SkillsAskAgent => 85,
            Act::LibraryImport => 86,
            Act::LibraryInfo => 87,
            Act::LibraryAttach => 88,
            Act::TemplatesDiffDefault => 89,
            Act::AttachChoose => 90,
            Act::AttachDetach => 91,
            Act::AttachRepo => 92,
            Act::SkillsHelpPrevAgent => 93,
            Act::SkillsHelpNextAgent => 94,
            Act::SkillsHelpAccept => 95,
            Act::SkillsHelpCancel => 96,
            Act::RequirementsNewArea => 97,
            Act::RequirementsAmend => 98,
            Act::RequirementsWithdraw => 99,
            Act::RequirementsFilter => 100,
        }
    }

    #[test]
    fn every_act_is_listed_and_has_a_row() {
        for (index, act) in ALL.iter().enumerate() {
            assert_eq!(position(*act), index, "{act:?} is out of place in ALL");
            assert!(act.spec().is_some(), "{act:?} has no catalogue row");
        }
        assert_eq!(
            ALL.len(),
            CATALOGUE.len(),
            "a row's act is missing from ALL"
        );
    }

    #[test]
    fn global_help_is_question_mark_and_f1() {
        let help = Act::Help.spec().expect("help has a row");
        assert_eq!(help.defaults, ["?", "f1"]);
    }

    #[test]
    fn every_act_has_exactly_one_row() {
        assert_eq!(CATALOGUE.len(), 101);
        let acts: HashSet<Act> = CATALOGUE.iter().map(|row| row.act).collect();
        assert_eq!(acts.len(), CATALOGUE.len(), "an act has two rows");
        for row in CATALOGUE {
            let found = row.act.spec().expect("every act has a row");
            assert!(std::ptr::eq(found, row), "{:?}", row.act);
        }
    }

    #[test]
    fn names_are_unique_per_context() {
        let mut seen = HashSet::new();
        for row in CATALOGUE {
            assert!(
                seen.insert((row.context, row.name)),
                "[{}] {} is named twice",
                row.context.table(),
                row.name
            );
        }
    }

    #[test]
    fn help_is_never_empty() {
        for row in CATALOGUE {
            assert!(!row.help.is_empty(), "{:?}", row.act);
        }
    }

    #[test]
    fn overlay_close_keeps_a_chord() {
        let close = Act::OverlayClose.spec().expect("overlay.close has a row");
        assert!(!close.defaults.is_empty());
    }

    #[test]
    fn editor_focus_keeps_a_chord() {
        let focus = Act::EditorFocus.spec().expect("editor.focus has a row");
        assert!(!focus.defaults.is_empty());
    }

    #[test]
    fn no_default_spells_ctrl_c() {
        for row in CATALOGUE {
            for spec in row.defaults {
                let folded: String = spec
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .to_lowercase();
                assert!(
                    !["ctrl-c", "ctrl+c", "control-c"].contains(&folded.as_str()),
                    "{:?} binds {spec:?}",
                    row.act
                );
            }
        }
    }

    #[test]
    fn no_spec_is_shared_in_a_context_unless_state_guarded() {
        let guarded = |a: Act, b: Act| {
            STATE_GUARDED
                .iter()
                .any(|&pair| pair == (a, b) || pair == (b, a))
        };
        for (i, first) in CATALOGUE.iter().enumerate() {
            for second in &CATALOGUE[i + 1..] {
                if first.context != second.context || guarded(first.act, second.act) {
                    continue;
                }
                for spec in first.defaults {
                    assert!(
                        !second.defaults.contains(spec),
                        "{:?} and {:?} share {spec:?}",
                        first.act,
                        second.act
                    );
                }
            }
        }
    }

    #[test]
    fn the_global_block_is_in_status_line_order() {
        let names: Vec<&str> = CATALOGUE
            .iter()
            .filter(|row| row.context == Context::Global)
            .map(|row| row.name)
            .collect();
        assert_eq!(
            names,
            [
                "quit",
                "next_tab",
                "prev_tab",
                "select_tab_1",
                "select_tab_2",
                "select_tab_3",
                "select_tab_4",
                "select_tab_5",
                "select_tab_6",
                "select_tab_7",
                "select_tab_8",
                "select_tab_9",
                "help",
                "workspaces",
                "find",
                "waiting",
                "queue",
            ]
        );
    }

    #[test]
    fn context_tables_and_headings() {
        let table = [
            (Context::Global, "global", "Global"),
            (Context::Overlay, "overlay", "Overlay"),
            (Context::List, "list", "List"),
            (Context::Pane, "pane", "Pane"),
            (Context::Confirm, "confirm", "Confirm"),
            (Context::Form, "form", "Form"),
            (Context::Common, "common", "Common"),
            (Context::Editor, "editor", "Editor"),
            (Context::Settings, "settings", "Settings"),
            (Context::SettingsAgents, "settings.agents", "Agents"),
            (
                Context::SettingsHierarchy,
                "settings.hierarchy",
                "Hierarchy",
            ),
            (Context::SettingsKinds, "settings.kinds", "Kinds"),
            (Context::SettingsPrompt, "settings.prompt", "Prompt"),
            (
                Context::SettingsConnection,
                "settings.connection",
                "Connection",
            ),
            (Context::SettingsQdrant, "settings.qdrant", "Qdrant"),
            (Context::SettingsBoxes, "settings.boxes", "Boxes"),
            (Context::SettingsPersonas, "settings.personas", "Personas"),
            (Context::SettingsSecrets, "settings.secrets", "Secrets"),
            (Context::SettingsQueue, "settings.queue", "Queue"),
            (Context::Concepts, "concepts", "Search concepts"),
            (Context::Switcher, "switcher", "Workspaces"),
            (Context::Migration, "migration", "Schema"),
            (Context::Waiting, "waiting", "Waiting on you"),
            (Context::Skills, "skills", "Skills"),
            (Context::SkillsLibrary, "skills.library", "Library"),
            (Context::SkillsTemplates, "skills.templates", "Templates"),
            (Context::SkillsAttach, "skills.attach", "Attachments"),
            (Context::SkillsHelp, "skills.help", "Agent help"),
            (Context::Requirements, "requirements", "Requirements"),
        ];
        assert_eq!(table.len(), Context::ALL.len());
        for (context, name, heading) in table {
            assert_eq!(context.table(), name);
            assert_eq!(context.heading(), heading);
        }
        for context in Context::ALL {
            assert!(
                !(context.is_view() && context.is_shared()),
                "{context:?} is both a view and a shared context"
            );
        }
        // MOD-67 M4 §3.1: `skills` is shared (Library and Templates inherit it), the rest views.
        assert!(Context::Skills.is_shared() && !Context::Skills.is_view());
        for context in [
            Context::SkillsLibrary,
            Context::SkillsTemplates,
            Context::SkillsAttach,
            Context::SkillsHelp,
            Context::Requirements,
        ] {
            assert!(context.is_view() && !context.is_shared(), "{context:?}");
        }
    }

    #[test]
    fn tab_index_maps_the_nine_digits() {
        let digits = [
            Act::SelectTab1,
            Act::SelectTab2,
            Act::SelectTab3,
            Act::SelectTab4,
            Act::SelectTab5,
            Act::SelectTab6,
            Act::SelectTab7,
            Act::SelectTab8,
            Act::SelectTab9,
        ];
        for (index, act) in digits.into_iter().enumerate() {
            assert_eq!(act.tab_index(), Some(index), "{act:?}");
            assert_eq!(
                act.spec().expect("a row").defaults,
                [format!("{}", index + 1)]
            );
        }
        assert_eq!(Act::Quit.tab_index(), None);
        assert_eq!(Act::NextTab.tab_index(), None);
    }

    #[test]
    fn in_capture_acts_are_the_form_overlay_close_editor_focus_concepts_and_the_skills_chords() {
        let captured: HashSet<Act> = CATALOGUE
            .iter()
            .filter(|row| row.in_capture)
            .map(|row| row.act)
            .collect();
        assert_eq!(
            captured,
            HashSet::from([
                Act::FormNextField,
                Act::FormPrevField,
                Act::FormSave,
                Act::FormExternalEditor,
                Act::OverlayClose,
                Act::EditorFocus,
                Act::ConceptsDecisions,
                Act::ConceptsProject,
                Act::ConceptsReindex,
                Act::ConceptsUp,
                Act::ConceptsDown,
                Act::SkillsAskAgent,
                Act::AttachRepo,
                Act::SkillsHelpPrevAgent,
                Act::SkillsHelpNextAgent,
            ])
        );
    }
}
