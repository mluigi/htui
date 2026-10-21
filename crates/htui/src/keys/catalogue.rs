//! The action catalogue: every named action, its context, defaults and help (ANA-26 §7.2).
//!
//! MOD-67 M1 (plan D4, D12) fills the global, overlay and shared contexts. Global and overlay
//! rows reproduce the M1 global bindings, plus `f1` for help (ANA-26 §6.5).
//! Each shared-context row (`list`, `pane`, `confirm`, `form`, `common`) takes its defaults from
//! today's match arms and cites the arm it mirrors. No view consumes a shared context until M3,
//! so those rows change no behaviour yet. M3-M5 append each view's context in its own block.
//!
//! Defaults are strict spec strings (blueprint B4): lower-case key names (`tab`, `esc`, `pgdn`,
//! `f1`), the character itself for a character (`G`, `]`), and `ctrl-x` for a ctrl chord. This
//! module does not parse them; `keys::Keys::defaults` does, and its tests pin that every one
//! parses. MOD-115 M2 (MD2): `ctrl-c` is a default of `global.quit` and `leave.confirm`, like any
//! chord; the shell still answers it ahead of every stack until M2's Task 6 removes that branch.
//!
//! `in_capture` is a property of an action (blueprint B3). It means "offered while a text field
//! captures", so every default and every user chord of the action must be non-printable
//! (`KeyChord::is_printable`; M2's validator). The global layer is **not** marked. Under a
//! capturing mode (M3) the global layer is filtered chord by chord, and printable chords drop
//! out. So `global.help` (`?`, `f1`) reaches help through `f1` while a field types `?`, and
//! `global.quit` (`q`) is unreachable there, while its `ctrl-c` passes the filter.
//!
//! MOD-67 M3 appends the Settings tab's context, one context per Settings section and one per
//! overlay, in strip order (plan D1). A view context holds the view's own verbs; the shared verbs
//! it inherits through `Layer::view` may be overridden per view (D10), and [`VIEW_DEFAULTS`] adds
//! the extra chords a view takes today on a shared act (D12 as amended by PA-1).
//!
//! MOD-67 M4 appends the Skills and Requirements contexts (plan D1): `skills`, a shared context
//! holding the verbs Library and Templates share (the view switch, the version cursor, base,
//! diff, `E`, `ctrl-g`), then one view context per Skills view or pane and `requirements`.
//!
//! MOD-67 M5 appends the Backlog and Chat contexts (plan D1, blueprint PA-1): `backlog`, a shared
//! context holding the Backlog tab's own verbs (filter, new item, queue, the replay miss, ...),
//! then one view context per Backlog sub-tab in strip order, the filter and item forms, and
//! `chat`. Every chord is today's: no default changes.
//!
//! MOD-67 M6 appends `queue` (the queue overlay) and `picker` (the directory picker), the last
//! two views that matched raw keys.

//!
//! MOD-115 M2 appends the shell's and the text fields' contexts (blueprint §1.2): `layout`,
//! `history`, `leave` (the leave dialog), `field` (a text field's editing keys), `choice` (a
//! choice field's value keys) and `answer` (a parked request's numbered options). Every row M2
//! adds or changes carries `since: 2` (§1.1), so a key file from before M2 can give way to it.

use Context::{
    Answer, Backlog, BacklogDocs, BacklogFilter, BacklogGraph, BacklogItem, BacklogNotes,
    BacklogPrompt, BacklogReqs, BacklogRuns, Chat, Choice, Common, Concepts, Confirm, Editor,
    Field, Form, Global, History, Layout, Leave, List, Overlay, Pane, Picker, Queue, Requirements,
    Settings, SettingsAgents, SettingsBoxes, SettingsConnection, SettingsHierarchy, SettingsKinds,
    SettingsPersonas, SettingsSecrets, Skills, SkillsAttach, SkillsHelp, SkillsLibrary,
    SkillsTemplates, Switcher, Waiting,
};

/// A key context: a TOML table of `keys.toml` and a layer of a context stack (ANA-26 §7.2-§7.3).
/// M1 has the global, overlay and shared contexts; M3 appends the Settings and overlay view
/// contexts; M4 the Skills and Requirements ones. M5 appends the Backlog and Chat contexts:
/// `backlog` (shared), one view context per Backlog sub-tab, the filter and item forms, and
/// `chat`. MOD-67 M6 appends `queue` (the queue overlay) and `picker` (the directory picker),
/// the last two views that matched raw keys.
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
    /// `[settings.project_types]`: the Project types section (no own action; list moves only).
    SettingsProjectTypes,
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
    /// `[backlog]`: the Backlog tab's own verbs (MOD-67 M5 D1); a shared context, like `settings`
    /// and `skills`, so each sub-tab's table may override them.
    Backlog,
    /// `[backlog.body]`: the Backlog's Body sub-tab (no own action; overrides only, M5 PA-1).
    BacklogBody,
    /// `[backlog.runs]`: the Backlog's Runs sub-tab.
    BacklogRuns,
    /// `[backlog.graph]`: the Backlog's Graph sub-tab.
    BacklogGraph,
    /// `[backlog.docs]`: the Backlog's Docs sub-tab and its compose area.
    BacklogDocs,
    /// `[backlog.notes]`: the Backlog's Notes sub-tab and its compose area.
    BacklogNotes,
    /// `[backlog.prompt]`: the Backlog's Prompt sub-tab (M5 PA-1).
    BacklogPrompt,
    /// `[backlog.reqs]`: the Backlog's Reqs sub-tab, its picker and questions.
    BacklogReqs,
    /// `[backlog.filter_form]`: the Backlog's filter form. Not `backlog.filter`: that is the
    /// `[backlog]` verb `filter`, and one TOML key cannot be both a value and a table
    /// (`no_act_name_is_a_nested_table`).
    BacklogFilter,
    /// `[backlog.item]`: the Backlog's item form and its divergence view.
    BacklogItem,
    /// `[chat]`: the Chat tab (live, parked, ended, replay).
    Chat,
    /// `[queue]`: the queue overlay (MOD-67 M6 D2).
    Queue,
    /// `[picker]`: the directory picker a Settings section opens (MOD-67 M6 D3; today
    /// Hierarchy's).
    Picker,
    /// `[layout]`: the shell's pane layout of a list + detail tab (MOD-115 M2 B1): presets,
    /// split sizes, hiding and zoom. Only the list + detail browse stacks carry it.
    Layout,
    /// `[history]`: back and forward through the places visited (MOD-115 M2 B1). Every tab
    /// browse stack carries it; no capturing one does.
    History,
    /// `[leave]`: the leave dialog (MOD-115 M2 D6), an overlay's view like `[waiting]`.
    Leave,
    /// `[field]`: a text field's editing keys (MOD-115 M2 D7). The shell translates them into the
    /// widgets' canonical events while a stack captures text.
    Field,
    /// `[choice]`: a choice field's value keys (MOD-115 M2 D7): move the option, tick it.
    Choice,
    /// `[answer]`: a parked request's numbered options (MOD-115 M2 D7), in Runs and in Chat.
    Answer,
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
        Self::SettingsProjectTypes,
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
        Self::Backlog,
        Self::BacklogBody,
        Self::BacklogRuns,
        Self::BacklogGraph,
        Self::BacklogDocs,
        Self::BacklogNotes,
        Self::BacklogPrompt,
        Self::BacklogReqs,
        Self::BacklogFilter,
        Self::BacklogItem,
        Self::Chat,
        Self::Queue,
        Self::Picker,
        Self::Layout,
        Self::History,
        Self::Leave,
        Self::Field,
        Self::Choice,
        Self::Answer,
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
            Self::SettingsProjectTypes => "settings.project_types",
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
            Self::Backlog => "backlog",
            Self::BacklogBody => "backlog.body",
            Self::BacklogRuns => "backlog.runs",
            Self::BacklogGraph => "backlog.graph",
            Self::BacklogDocs => "backlog.docs",
            Self::BacklogNotes => "backlog.notes",
            Self::BacklogPrompt => "backlog.prompt",
            Self::BacklogReqs => "backlog.reqs",
            Self::BacklogFilter => "backlog.filter_form",
            Self::BacklogItem => "backlog.item",
            Self::Chat => "chat",
            Self::Queue => "queue",
            Self::Picker => "picker",
            Self::Layout => "layout",
            Self::History => "history",
            Self::Leave => "leave",
            Self::Field => "field",
            Self::Choice => "choice",
            Self::Answer => "answer",
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
            Self::SettingsProjectTypes => "Project types",
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
            Self::Backlog => "Backlog",
            Self::BacklogBody => "Body",
            Self::BacklogRuns => "Runs",
            Self::BacklogGraph => "Graph",
            Self::BacklogDocs => "Docs",
            Self::BacklogNotes => "Notes",
            Self::BacklogPrompt => "Prompt",
            Self::BacklogReqs => "Reqs",
            Self::BacklogFilter => "Filter",
            Self::BacklogItem => "Item",
            Self::Chat => "Chat",
            Self::Queue => "Queue",
            Self::Picker => "Picker",
            Self::Layout => "Layout",
            Self::History => "History",
            Self::Leave => "Leave htui?",
            Self::Field => "Text field",
            Self::Choice => "Choice",
            Self::Answer => "Answer",
        }
    }

    /// A view's own context (a Settings section, an overlay, a Skills view or pane, the
    /// Requirements tab, a Backlog sub-tab or form, the Chat tab, the queue overlay, the
    /// directory picker): hosts view verbs, D10
    /// overrides and [`VIEW_DEFAULTS`]; skipped by the validator's per-context pass (MOD-67 M3
    /// PA-4).
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
                | Self::SettingsProjectTypes
                | Self::Concepts
                | Self::Switcher
                | Self::Migration
                | Self::Waiting
                | Self::SkillsLibrary
                | Self::SkillsTemplates
                | Self::SkillsAttach
                | Self::SkillsHelp
                | Self::Requirements
                | Self::BacklogBody
                | Self::BacklogRuns
                | Self::BacklogGraph
                | Self::BacklogDocs
                | Self::BacklogNotes
                | Self::BacklogPrompt
                | Self::BacklogReqs
                | Self::BacklogFilter
                | Self::BacklogItem
                | Self::Chat
                | Self::Queue
                | Self::Picker
                | Self::Leave
        )
    }

    /// A shared context a view layer may inherit and override (MOD-67 D10, PA-3): `list`,
    /// `pane`, `confirm`, `form`, `common`, `settings`, `skills` (M4), `backlog` (M5).
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
                | Self::Backlog
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
    /// `global.about`: toggle the about panel under the header (MOD-115).
    About,
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
    /// `backlog.replay`: replay the step under the Runs cursor, or say how to select one (the
    /// miss).
    BacklogReplay,
    /// `backlog.open_graph`: open the Graph sub-tab.
    BacklogOpenGraph,
    /// `backlog.filter`: open the filter form.
    BacklogOpenFilter,
    /// `backlog.clear_filter`: clear the filter.
    BacklogClearFilter,
    /// `backlog.new_item`: create an item.
    BacklogNewItem,
    /// `backlog.queue`: queue or dequeue the selected item.
    BacklogQueue,
    /// `backlog.pause_queue`: pause or resume the queue.
    BacklogPauseQueue,
    /// `backlog.runs.next_step`: the next step.
    RunsNextStep,
    /// `backlog.runs.prev_step`: the previous step.
    RunsPrevStep,
    /// `backlog.runs.approve`: approve the step.
    RunsApprove,
    /// `backlog.runs.reject`: reject the step with a note.
    RunsReject,
    /// `backlog.runs.retry`: retry the step.
    RunsRetry,
    /// `backlog.runs.promote`: promote the step to a chat.
    RunsPromote,
    /// `backlog.runs.select`: select the winning candidate.
    RunsSelect,
    /// `backlog.runs.accept`: accept the step's output.
    RunsAccept,
    /// `backlog.runs.open`: open the step's output.
    RunsOpen,
    /// `backlog.runs.cancel_run`: cancel the run.
    RunsCancelRun,
    /// `backlog.runs.cleanup`: retry the run's cleanup.
    RunsCleanup,
    /// `backlog.runs.unblock`: unblock the item.
    RunsUnblock,
    /// `backlog.runs.start_run`: start a run.
    RunsStartRun,
    /// `backlog.runs.close_out`: close the item out.
    RunsCloseOut,
    /// `backlog.runs.follow_up`: send a follow-up.
    RunsFollowUp,
    /// `backlog.runs.toggle_view`: toggle between the run list and the flow.
    RunsToggleView,
    /// `backlog.runs.zoom_in`: zoom the flow in.
    RunsZoomIn,
    /// `backlog.runs.zoom_out`: zoom the flow out.
    RunsZoomOut,
    /// `backlog.runs.zoom_fit`: fit the run in the flow.
    RunsZoomFit,
    /// `backlog.graph.next_row`: the next graph row.
    GraphNextRow,
    /// `backlog.graph.prev_row`: the previous graph row.
    GraphPrevRow,
    /// `backlog.graph.deeper`: one level deeper.
    GraphDeeper,
    /// `backlog.graph.shallower`: one level shallower.
    GraphShallower,
    /// `backlog.graph.reroot`: re-root the graph at the row under the cursor.
    GraphReroot,
    /// `backlog.docs.next_row`: the next document.
    DocsNextRow,
    /// `backlog.docs.prev_row`: the previous document.
    DocsPrevRow,
    /// `backlog.docs.add`: add a document.
    DocsAdd,
    /// `backlog.docs.new_version`: add a version of the selected document.
    DocsNewVersion,
    /// `backlog.notes.add`: add a note.
    NotesAdd,
    /// `backlog.prompt.next_template`: the next prompt template.
    PromptNextTemplate,
    /// `backlog.prompt.prev_template`: the previous prompt template.
    PromptPrevTemplate,
    /// `backlog.reqs.next_row`: the next requirement row.
    ReqsNextRow,
    /// `backlog.reqs.prev_row`: the previous requirement row.
    ReqsPrevRow,
    /// `backlog.reqs.reconfirm`: reconfirm the selected citation.
    ReqsReconfirm,
    /// `backlog.reqs.cite`: open the requirement picker to cite one.
    ReqsCite,
    /// `backlog.reqs.uncite`: uncite the selected requirement.
    ReqsUncite,
    /// `backlog.reqs.pick`: pick the requirement under the picker's cursor.
    ReqsPick,
    /// `backlog.reqs.addresses`: cite as addressing the requirement.
    ReqsAddresses,
    /// `backlog.reqs.reserves`: cite as reserving the requirement.
    ReqsReserves,
    /// `backlog.filter_form.next_row`: the filter form's next row.
    FilterNextRow,
    /// `backlog.filter_form.prev_row`: the filter form's previous row.
    FilterPrevRow,
    /// `backlog.filter_form.clear`: clear the filter draft.
    FilterClear,
    /// `backlog.item.theirs`: resolve the divergence with their version.
    ItemTheirs,
    /// `backlog.item.mine`: resolve the divergence with my version.
    ItemMine,
    /// `backlog.item.switch_diff`: switch the divergence diff between body and paths.
    ItemSwitchDiff,
    /// `chat.compose`: open the composer.
    ChatCompose,
    /// `chat.next_agent`: the next agent.
    ChatNextAgent,
    /// `chat.end`: end the chat (pressed twice).
    ChatEnd,
    /// `chat.thoughts`: show or hide the agent's thoughts.
    ChatThoughts,
    /// `chat.top`: the transcript's top.
    ChatTop,
    /// `chat.bottom`: the transcript's bottom.
    ChatBottom,
    /// `queue.move_down`: move the selected entry down the queue.
    QueueMoveDown,
    /// `queue.move_up`: move the selected entry up the queue.
    QueueMoveUp,
    /// `queue.pause`: pause or resume the selected entry.
    QueuePause,
    /// `queue.dequeue`: take the selected entry off the queue.
    QueueDequeue,
    /// `queue.open`: open the selected entry's item.
    QueueOpen,
    /// `picker.open`: open the selected directory.
    PickerOpen,
    /// `picker.parent`: go to the parent directory.
    PickerParent,
    /// `picker.choose`: choose the selected entry.
    PickerChoose,
    /// `picker.choose_dir`: choose the directory being listed.
    PickerChooseDir,
    /// `picker.go_to`: type a path to go to.
    PickerGoTo,
    /// `picker.hidden`: show or hide hidden entries.
    PickerHidden,
    /// `global.next_pane`: focus the next visible pane (MOD-115 M2 D3).
    NextPane,
    /// `global.prev_pane`: focus the previous visible pane.
    PrevPane,
    /// `layout.next_preset`: the next layout preset.
    LayoutNextPreset,
    /// `layout.prev_preset`: the previous layout preset.
    LayoutPrevPreset,
    /// `layout.grow_list`: widen the list.
    LayoutGrowList,
    /// `layout.shrink_list`: narrow the list.
    LayoutShrinkList,
    /// `layout.grow_activity`: grow the activity pane.
    LayoutGrowActivity,
    /// `layout.shrink_activity`: shrink the activity pane.
    LayoutShrinkActivity,
    /// `layout.toggle_list`: show or hide the list.
    LayoutToggleList,
    /// `layout.toggle_activity`: show or hide the activity pane.
    LayoutToggleActivity,
    /// `layout.zoom`: zoom the focused pane, or unzoom.
    LayoutZoom,
    /// `history.back`: back to the place visited before (MOD-115 M2 D5).
    HistoryBack,
    /// `history.forward`: forward again after going back.
    HistoryForward,
    /// `leave.confirm`: leave htui from the leave dialog (MOD-115 M2 D6).
    LeaveConfirm,
    /// `leave.stay`: close the leave dialog and stay.
    LeaveStay,
    /// `field.submit`: a text field's submit (its canonical `Enter`, MOD-115 M2 D7).
    FieldSubmit,
    /// `field.cancel`: a text field's cancel (its canonical `Esc`).
    FieldCancel,
    /// `field.left`: the cursor one character left.
    FieldLeft,
    /// `field.right`: the cursor one character right.
    FieldRight,
    /// `field.home`: the cursor to the line's start.
    FieldHome,
    /// `field.end`: the cursor to the line's end.
    FieldEnd,
    /// `field.delete_back`: delete the character before the cursor.
    FieldDeleteBack,
    /// `field.delete_forward`: delete the character under the cursor.
    FieldDeleteForward,
    /// `field.line_up`: a text area's cursor one line up (MOD-115 M2 D-8).
    FieldLineUp,
    /// `field.line_down`: a text area's cursor one line down.
    FieldLineDown,
    /// `field.page_up`: a text area's cursor one page up.
    FieldPageUp,
    /// `field.page_down`: a text area's cursor one page down.
    FieldPageDown,
    /// `choice.prev`: a choice field's previous option (MOD-115 M2 D7).
    ChoicePrev,
    /// `choice.next`: a choice field's next option.
    ChoiceNext,
    /// `choice.toggle`: tick or untick a choice field's option.
    ChoiceToggle,
    /// `answer.option_1`: answer a parked request with option 1 (MOD-115 M2 D7).
    AnswerOption1,
    /// `answer.option_2`: answer a parked request with option 2.
    AnswerOption2,
    /// `answer.option_3`: answer a parked request with option 3.
    AnswerOption3,
    /// `answer.option_4`: answer a parked request with option 4.
    AnswerOption4,
    /// `answer.option_5`: answer a parked request with option 5.
    AnswerOption5,
    /// `answer.option_6`: answer a parked request with option 6.
    AnswerOption6,
    /// `answer.option_7`: answer a parked request with option 7.
    AnswerOption7,
    /// `answer.option_8`: answer a parked request with option 8.
    AnswerOption8,
    /// `answer.option_9`: answer a parked request with option 9.
    AnswerOption9,
    /// `requirements.priority_must`: set a requirement form's priority to must.
    RequirementsPriorityMust,
    /// `requirements.priority_later`: set a requirement form's priority to later.
    RequirementsPriorityLater,
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

    /// The option index of `answer.option_1`..`_9` (0..=8), [`tab_index`](Self::tab_index)'s
    /// twin; `None` for every other action.
    #[must_use]
    pub const fn answer_index(self) -> Option<usize> {
        Some(match self {
            Self::AnswerOption1 => 0,
            Self::AnswerOption2 => 1,
            Self::AnswerOption3 => 2,
            Self::AnswerOption4 => 3,
            Self::AnswerOption5 => 4,
            Self::AnswerOption6 => 5,
            Self::AnswerOption7 => 6,
            Self::AnswerOption8 => 7,
            Self::AnswerOption9 => 8,
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
    /// The key-file version the row's defaults are from (MOD-115 M2 §1.1): `1` for a row as it
    /// was before M2, `2` for a row M2 added or whose defaults it changed. A `version = 1` file
    /// may take a `since: 2` default's chord (D8).
    pub since: u8,
}

impl ActionSpec {
    /// This row, tagged as from key-file `version` (MOD-115 M2 §1.1).
    const fn since(self, version: u8) -> Self {
        Self {
            since: version,
            ..self
        }
    }
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
        since: 1,
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
    // [global]: the pre-MOD-67 keymap's `default_global` rows and `app/mod.rs` `register_all`
    // labels, in status-line order. MOD-115 M2 (MD2): `ctrl-c` is a default of quit (the leave
    // dialog), no longer a fixed key. Plan D4: `next_tab` and `prev_tab` moved to
    // `ctrl-pgdn`/`ctrl-pgup` (T6, B6), `tab`/`backtab` to the pane rows, so they are `since: 2`;
    // digits and clicks remain the main way between tabs.
    row(Act::Quit, Global, "quit", &["q", "ctrl-c"], "quit").since(2),
    row(Act::NextTab, Global, "next_tab", &["ctrl-pgdn"], "next tab").since(2),
    row(
        Act::PrevTab,
        Global,
        "prev_tab",
        &["ctrl-pgup"],
        "previous tab",
    )
    .since(2),
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
    // MOD-115: `f9` toggles the about panel; `?` and `f1` stay help's.
    row(Act::About, Global, "about", &["f9"], "about"),
    // Offered by `register_all` (D5): `app/mod.rs` workspace switcher, concepts search,
    // waiting list (MOD-69 D7) and queue overlay (MOD-12 M3 D8). `queue` follows `waiting`, so
    // the status line a 100-column frame cuts before `Ctrl+w waiting` is unchanged.
    row(Act::Workspaces, Global, "workspaces", &["w"], "workspaces"),
    row(Act::Find, Global, "find", &["ctrl-f"], "find"),
    row(Act::Waiting, Global, "waiting", &["ctrl-w"], "waiting"),
    row(Act::Queue, Global, "queue", &["ctrl-q"], "queue"),
    // MOD-115 M2 D3: pane focus. After `queue`, so every keys line keeps its prefix. `tab` and
    // `backtab` since `next_tab` gave them up (T6, B6); the modal stack never offers them (B2).
    row(
        Act::NextPane,
        Global,
        "next_pane",
        &["tab", "ctrl-right"],
        "next pane",
    )
    .since(2),
    row(
        Act::PrevPane,
        Global,
        "prev_pane",
        &["backtab", "ctrl-left"],
        "previous pane",
    )
    .since(2),
    // [overlay]: the pre-MOD-67 keymap's wildcard overlay row. In capture because the concepts
    // field returns `Pass` on `Esc` and this layer closes it (`concepts_search.rs:334-335`).
    capture_row(Act::OverlayClose, Overlay, "close", &["esc"], "close"),
    // [list]
    // backlog/mod.rs:789, requirements/mod.rs:443, connection.rs:811, kinds.rs:1492,
    // qdrant.rs:404, workspace_switcher.rs:163, waiting_list.rs:274, queue.rs:440,
    // path_picker.rs:142. agents.rs:2420 and hierarchy.rs:1212 take `j` only (ANA §6.6 adds
    // `down`, M3).
    row(Act::ListDown, List, "down", &["j", "down"], "down"),
    // backlog/mod.rs:790, requirements/mod.rs:444, connection.rs:815, kinds.rs:1496,
    // boxes.rs:699, library.rs:689, queue.rs:445, path_picker.rs:147.
    row(Act::ListUp, List, "up", &["k", "up"], "up"),
    // backlog/mod.rs:791, requirements/mod.rs:445. kinds.rs:1476 binds `g` to open graph (M3).
    row(Act::ListTop, List, "top", &["g", "home"], "top"),
    // backlog/mod.rs:792, requirements/mod.rs:446.
    row(Act::ListBottom, List, "bottom", &["G", "end"], "bottom"),
    // backlog/mod.rs:779 (fold, else the detail pane at :784; the Backlog tries it before the
    // pane, MOD-67 M5 PA-2), requirements/mod.rs:448. Elsewhere `Enter` is a view verb
    // (connection.rs:807, workspace_switcher.rs:171).
    row(Act::ListFold, List, "fold", &["enter"], "fold"),
    // [pane]
    // backlog/detail/mod.rs:466 (`Scroll::apply`), requirements/mod.rs:453, library.rs:812,
    // templates.rs:616. runs.rs:1701 `J` is next step (a view verb, ANA §6.2:
    // `backlog.runs.next_step`).
    row(
        Act::PaneScrollDown,
        Pane,
        "scroll_down",
        &["J"],
        "scroll down",
    ),
    // backlog/detail/mod.rs:467.
    row(Act::PaneScrollUp, Pane, "scroll_up", &["K"], "scroll up"),
    // backlog/detail/mod.rs:468, graph.rs:706, divergence.rs:266.
    row(Act::PanePageDown, Pane, "page_down", &["pgdn"], "page down"),
    // backlog/detail/mod.rs:469.
    row(Act::PanePageUp, Pane, "page_up", &["pgup"], "page up"),
    // backlog/mod.rs:793. Settings cycles sections through `settings.next_section` (M3).
    // MOD-115 M2 T6 (B6, plan D4): the shift arrows replace the plain ones (arrows stay in the
    // focused pane). Appended, so the hint rows' first chords (`h/l sub-tab`) stay.
    row(
        Act::PaneNextSubtab,
        Pane,
        "next_subtab",
        &["l", "]", "shift-right"],
        "next sub-tab",
    )
    .since(2),
    // backlog/mod.rs:794. Settings: `settings.prev_section` (M3).
    row(
        Act::PanePrevSubtab,
        Pane,
        "prev_subtab",
        &["h", "[", "shift-left"],
        "previous sub-tab",
    )
    .since(2),
    // [confirm]
    // connection.rs:483, qdrant.rs:214, kinds.rs:715, hierarchy.rs:769, personas.rs:785,
    // boxes.rs:545, runs.rs:721, :1144, detail/requirements.rs:352. migration_prompt.rs:92 also
    // takes `Y`: a `VIEW_DEFAULTS` row.
    row(Act::ConfirmYes, Confirm, "yes", &["y"], "yes"),
    // connection.rs:487, qdrant.rs:218, kinds.rs:726, agents.rs:1161, boxes.rs:546,
    // personas.rs:791, runs.rs:725, :1151, detail/requirements.rs:363, hierarchy.rs:761.
    // migration_prompt.rs:97 also takes `N`: a `VIEW_DEFAULTS` row.
    row(Act::ConfirmNo, Confirm, "no", &["n", "esc"], "no"),
    // [form]: all in capture; every chord is named or ctrl.
    // item_form.rs:664, requirements/mod.rs:555, compose.rs:376. The Settings forms also
    // take `Down` (agents.rs:2166): `VIEW_DEFAULTS` rows (M3); so do the Library rename form,
    // the attachment form and the Requirements forms (M4 `VIEW_DEFAULTS` rows), and the item
    // form, the Docs compose area and the filter form (M5 `VIEW_DEFAULTS` rows).
    capture_row(
        Act::FormNextField,
        Form,
        "next_field",
        &["tab"],
        "next field",
    ),
    // item_form.rs:665, requirements/mod.rs:556, compose.rs:377. The Settings forms also
    // take `Up`: `VIEW_DEFAULTS` rows (M3); so do the M4 and M5 forms above.
    capture_row(
        Act::FormPrevField,
        Form,
        "prev_field",
        &["backtab"],
        "previous field",
    ),
    // text_area.rs:194, item_form.rs:661, requirements/mod.rs:554, attach.rs:477,
    // library.rs:990. Every site also accepts `ctrl-S` (blueprint F-7, M4). The Library and
    // Templates editors resolve it after their widget passes the chord; Requirements sends a
    // modified chord (`ctrl-s` by default) straight to the resolver and a plain one (a rebound
    // `f2`) to its widget first (MOD-67 M4 PA-3). The Backlog's item form and Notes/Docs compose
    // areas resolve it after their widgets (MOD-67 M5 D8; compose.rs:372).
    capture_row(Act::FormSave, Form, "save", &["ctrl-s"], "save"),
    // item_form.rs:663 (label at :75), compose.rs:374, library.rs:1021, templates.rs:689. Same
    // `E` alias.
    capture_row(
        Act::FormExternalEditor,
        Form,
        "external_editor",
        &["ctrl-e"],
        "$EDITOR",
    ),
    // [common]
    // connection.rs:779, qdrant.rs:382, kinds.rs:1468, hierarchy.rs:1241, prompt.rs:849,
    // agents.rs:2401, backlog/mod.rs:811. boxes.rs:707 `e` edits quirks (a view verb).
    row(Act::Edit, Common, "edit", &["e"], "edit"),
    // kinds.rs:1452, hierarchy.rs:1233, agents.rs:2395, library.rs:694, templates.rs:527.
    // The Backlog's `N` is `backlog.new_item`.
    row(Act::New, Common, "new", &["n"], "new"),
    // hierarchy.rs:1279, kinds.rs:1484.
    row(Act::Delete, Common, "delete", &["d"], "delete"),
    // connection.rs:785, qdrant.rs:388. detail/requirements.rs:546 `c` is the cite picker.
    row(Act::Clear, Common, "clear", &["c"], "clear"),
    // connection.rs:822, qdrant.rs:414, kinds.rs:1505, hierarchy.rs:1289, prompt.rs:869,
    // boxes.rs:733, requirements/mod.rs:471, library.rs:690, templates.rs:523, attach.rs:398.
    // agents.rs:2495 `r` probes (a view verb).
    row(Act::Reload, Common, "reload", &["r"], "reload"),
    // divergence.rs:248, library.rs:895, chat/mod.rs:280, path_picker.rs:182 (the directory
    // picker, MOD-67 M6). personas.rs:841 also takes `Enter`: a `VIEW_DEFAULTS` row.
    row(Act::Back, Common, "back", &["esc"], "back"),
    // connection.rs:828, boxes.rs:739, hierarchy.rs:1295, kinds.rs:1511, prompt.rs:875.
    // Shares `Esc` with `back`: `STATE_GUARDED`.
    row(Act::Dismiss, Common, "dismiss", &["esc"], "dismiss"),
    // [editor]: MOD-57 M1 (plan P4, P5). `ctrl-4` is what `ctrl-\` arrives as on unix (`chord.rs`
    // `legacy_arrival`; `ctrl-\` itself is refused as indistinguishable). On Windows crossterm
    // delivers `ctrl-\` as `Char('\\')` + CONTROL, so there only a physical `ctrl-4` matches
    // (MOD-16 H-10/H-11, T7's docs). In capture: while the editor is focused every other key,
    // `ctrl-c` included, is the editor's. The loader refuses a file that unbinds it, as it does
    // `overlay.close`, and it never gives way (MOD-57 M2 E9 (a), (b)).
    capture_row(
        Act::EditorFocus,
        Editor,
        "focus",
        &["ctrl-4"],
        "editor focus",
    ),
    // Offered only on the asking tab while the editor is unfocused (`Stack::EDITOR_OPEN`):
    // focused, `ctrl-x` is nano's exit and goes to the editor.
    row(Act::EditorAbort, Editor, "abort", &["ctrl-x"], "abort edit"),
    // [settings]: the Settings tab's own section cycling (MOD-67 M3 D1). Every non-capturing
    // section stack carries this layer, so a section's view layer may override both.
    // settings/mod.rs:347. MOD-115 M2 T6: the shift arrows, as on `pane.next_subtab`.
    row(
        Act::NextSection,
        Settings,
        "next_section",
        &["l", "]", "shift-right"],
        "next section",
    )
    .since(2),
    // settings/mod.rs:351.
    row(
        Act::PrevSection,
        Settings,
        "prev_section",
        &["h", "[", "shift-left"],
        "previous section",
    )
    .since(2),
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
    // MOD-115 M2 T6: the shift arrows, as on `pane.next_subtab`.
    row(
        Act::SkillsSwitchView,
        Skills,
        "switch_view",
        &["h", "l", "[", "]", "shift-left", "shift-right"],
        "switch view",
    )
    .since(2),
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
    // requirements/mod.rs:467 (`'a'`).
    row(
        Act::RequirementsNewArea,
        Requirements,
        "new_area",
        &["a"],
        "new area",
    ),
    // requirements/mod.rs:469 (`'e'`).
    row(
        Act::RequirementsAmend,
        Requirements,
        "amend",
        &["e"],
        "amend",
    ),
    // requirements/mod.rs:470 (`'W'`).
    row(
        Act::RequirementsWithdraw,
        Requirements,
        "withdraw",
        &["W"],
        "withdraw",
    ),
    // requirements/mod.rs:456.
    row(
        Act::RequirementsFilter,
        Requirements,
        "filter",
        &["/"],
        "filter",
    ),
    // MOD-115 M2 D7: requirements/forms.rs:325-333, the priority's `m`/`l` (once `FIXED`).
    row(
        Act::RequirementsPriorityMust,
        Requirements,
        "priority_must",
        &["m"],
        "priority must",
    )
    .since(2),
    row(
        Act::RequirementsPriorityLater,
        Requirements,
        "priority_later",
        &["l"],
        "priority later",
    )
    .since(2),
    // [backlog]: the Backlog tab's own verbs (MOD-67 M5 D1), one row each, so each sub-tab's
    // table may override them; every Backlog browse stack carries this layer.
    // backlog/mod.rs:784 (the pane's `Enter`: runs.rs:1711 replays) and backlog/mod.rs:820 (the
    // miss, MOD-67 M5 D6): offered on every sub-tab, so the miss fires from each.
    row(
        Act::BacklogReplay,
        Backlog,
        "replay",
        &["enter"],
        "replay step",
    ),
    // backlog/mod.rs:797.
    row(
        Act::BacklogOpenGraph,
        Backlog,
        "open_graph",
        &["m"],
        "open graph",
    ),
    // backlog/mod.rs:800.
    row(Act::BacklogOpenFilter, Backlog, "filter", &["f"], "filter"),
    // backlog/mod.rs:803.
    row(
        Act::BacklogClearFilter,
        Backlog,
        "clear_filter",
        &["F"],
        "clear filter",
    ),
    // backlog/mod.rs:810: `N`, not `common.new`'s `n` (MOD-67 M5 D2).
    row(Act::BacklogNewItem, Backlog, "new_item", &["N"], "new item"),
    // backlog/mod.rs:814.
    row(
        Act::BacklogQueue,
        Backlog,
        "queue",
        &["Q"],
        "queue / dequeue",
    ),
    // backlog/mod.rs:815.
    row(
        Act::BacklogPauseQueue,
        Backlog,
        "pause_queue",
        &["P"],
        "pause / resume queue",
    ),
    // [backlog.runs]
    // runs.rs:1701 (ANA-26 §6.2: `J` is the step cursor here, not `pane.scroll_down`).
    row(
        Act::RunsNextStep,
        BacklogRuns,
        "next_step",
        &["J"],
        "next step",
    ),
    // runs.rs:1705.
    row(
        Act::RunsPrevStep,
        BacklogRuns,
        "prev_step",
        &["K"],
        "previous step",
    ),
    // runs.rs:1725 -> :888 (`RunsTab::on_key` hands every verb to `action`).
    row(Act::RunsApprove, BacklogRuns, "approve", &["a"], "approve"),
    // runs.rs:895.
    row(Act::RunsReject, BacklogRuns, "reject", &["x"], "reject"),
    // runs.rs:902.
    row(Act::RunsRetry, BacklogRuns, "retry", &["r"], "retry step"),
    // runs.rs:905.
    row(
        Act::RunsPromote,
        BacklogRuns,
        "promote",
        &["p"],
        "promote to chat",
    ),
    // runs.rs:908.
    row(
        Act::RunsSelect,
        BacklogRuns,
        "select",
        &["s"],
        "select winner",
    ),
    // runs.rs:916.
    row(
        Act::RunsAccept,
        BacklogRuns,
        "accept",
        &["A"],
        "accept output",
    ),
    // runs.rs:924.
    row(Act::RunsOpen, BacklogRuns, "open", &["o"], "open output"),
    // runs.rs:848, :864.
    row(
        Act::RunsCancelRun,
        BacklogRuns,
        "cancel_run",
        &["c"],
        "cancel run",
    ),
    // runs.rs:848, :867.
    row(
        Act::RunsCleanup,
        BacklogRuns,
        "cleanup",
        &["T"],
        "retry cleanup",
    ),
    // runs.rs:828.
    row(Act::RunsUnblock, BacklogRuns, "unblock", &["u"], "unblock"),
    // runs.rs:833.
    row(
        Act::RunsStartRun,
        BacklogRuns,
        "start_run",
        &["R"],
        "start run",
    ),
    // runs.rs:842.
    row(
        Act::RunsCloseOut,
        BacklogRuns,
        "close_out",
        &["C"],
        "close out",
    ),
    // runs.rs:819.
    row(
        Act::RunsFollowUp,
        BacklogRuns,
        "follow_up",
        &["i"],
        "follow up",
    ),
    // runs.rs:1715.
    row(
        Act::RunsToggleView,
        BacklogRuns,
        "toggle_view",
        &["v"],
        "list / flow",
    ),
    // runs.rs:1716.
    row(Act::RunsZoomIn, BacklogRuns, "zoom_in", &["+"], "zoom in"),
    // runs.rs:1717.
    row(
        Act::RunsZoomOut,
        BacklogRuns,
        "zoom_out",
        &["-"],
        "zoom out",
    ),
    // runs.rs:1718.
    row(
        Act::RunsZoomFit,
        BacklogRuns,
        "zoom_fit",
        &["="],
        "fit the run",
    ),
    // [backlog.graph]
    // graph.rs:704.
    row(
        Act::GraphNextRow,
        BacklogGraph,
        "next_row",
        &["J"],
        "next row",
    ),
    // graph.rs:705.
    row(
        Act::GraphPrevRow,
        BacklogGraph,
        "prev_row",
        &["K"],
        "previous row",
    ),
    // graph.rs:708.
    row(Act::GraphDeeper, BacklogGraph, "deeper", &["+"], "deeper"),
    // graph.rs:711.
    row(
        Act::GraphShallower,
        BacklogGraph,
        "shallower",
        &["-"],
        "shallower",
    ),
    // graph.rs:712 (MOD-67 M5 PA-2): always consumes `Enter`, so it shadows
    // `backlog.replay` (`SHADOWING`).
    row(
        Act::GraphReroot,
        BacklogGraph,
        "reroot",
        &["enter"],
        "re-root",
    ),
    // [backlog.docs]
    // documents.rs:461.
    row(
        Act::DocsNextRow,
        BacklogDocs,
        "next_row",
        &["J"],
        "next row",
    ),
    // documents.rs:462.
    row(
        Act::DocsPrevRow,
        BacklogDocs,
        "prev_row",
        &["K"],
        "previous row",
    ),
    // documents.rs:463.
    row(Act::DocsAdd, BacklogDocs, "add", &["a"], "new document"),
    // documents.rs:465.
    row(
        Act::DocsNewVersion,
        BacklogDocs,
        "new_version",
        &["v"],
        "new version",
    ),
    // [backlog.notes]
    // notes.rs:306.
    row(Act::NotesAdd, BacklogNotes, "add", &["a"], "add note"),
    // [backlog.prompt]
    // prompt.rs:358 (MOD-67 M5 PA-1).
    row(
        Act::PromptNextTemplate,
        BacklogPrompt,
        "next_template",
        &["n"],
        "next template",
    ),
    // prompt.rs:359.
    row(
        Act::PromptPrevTemplate,
        BacklogPrompt,
        "prev_template",
        &["p"],
        "previous template",
    ),
    // [backlog.reqs]
    // detail/requirements.rs:543, :381 (the picker's `J`).
    row(
        Act::ReqsNextRow,
        BacklogReqs,
        "next_row",
        &["J"],
        "next row",
    ),
    // detail/requirements.rs:544, :384.
    row(
        Act::ReqsPrevRow,
        BacklogReqs,
        "prev_row",
        &["K"],
        "previous row",
    ),
    // detail/requirements.rs:545.
    row(
        Act::ReqsReconfirm,
        BacklogReqs,
        "reconfirm",
        &["r"],
        "reconfirm",
    ),
    // detail/requirements.rs:546.
    row(Act::ReqsCite, BacklogReqs, "cite", &["c"], "cite"),
    // detail/requirements.rs:547.
    row(Act::ReqsUncite, BacklogReqs, "uncite", &["u"], "uncite"),
    // detail/requirements.rs:387.
    row(Act::ReqsPick, BacklogReqs, "pick", &["enter"], "pick"),
    // detail/requirements.rs:325.
    row(
        Act::ReqsAddresses,
        BacklogReqs,
        "addresses",
        &["a"],
        "addresses",
    ),
    // detail/requirements.rs:325 (`v`).
    row(
        Act::ReqsReserves,
        BacklogReqs,
        "reserves",
        &["v"],
        "reserves",
    ),
    // [backlog.filter_form]
    // filter.rs:266 (`'j'`; `Down` and `Tab` are `form.next_field`, `VIEW_DEFAULTS`).
    row(
        Act::FilterNextRow,
        BacklogFilter,
        "next_row",
        &["j"],
        "next row",
    ),
    // filter.rs:267.
    row(
        Act::FilterPrevRow,
        BacklogFilter,
        "prev_row",
        &["k"],
        "previous row",
    ),
    // filter.rs:268: clears the whole draft (a form verb, not an option's value).
    row(Act::FilterClear, BacklogFilter, "clear", &["x"], "clear"),
    // [backlog.item]
    // divergence.rs:250.
    row(
        Act::ItemTheirs,
        BacklogItem,
        "theirs",
        &["t", "T"],
        "theirs wins",
    ),
    // divergence.rs:251.
    row(Act::ItemMine, BacklogItem, "mine", &["m", "M"], "mine wins"),
    // divergence.rs:252.
    row(
        Act::ItemSwitchDiff,
        BacklogItem,
        "switch_diff",
        &["tab", "backtab"],
        "body/paths",
    ),
    // [chat]
    // chat/mod.rs:598.
    row(
        Act::ChatCompose,
        Chat,
        "compose",
        &["i", "enter"],
        "compose",
    ),
    // chat/mod.rs:604.
    row(Act::ChatNextAgent, Chat, "next_agent", &["a"], "next agent"),
    // chat/mod.rs:608.
    row(Act::ChatEnd, Chat, "end", &["esc"], "end chat (twice)"),
    // transcript.rs:492.
    row(Act::ChatThoughts, Chat, "thoughts", &["t"], "thoughts"),
    // transcript.rs:484: no Chat stack offers `list.top`, so the name is the view's.
    row(Act::ChatTop, Chat, "top", &["g"], "top"),
    // transcript.rs:488.
    row(Act::ChatBottom, Chat, "bottom", &["G"], "bottom"),
    // [queue]: the queue overlay (MOD-67 M6 D2). `j`/`k` are `list.down`/`up`, `Esc` is
    // `overlay.close`, `?` is `global.help`.
    // queue.rs:450.
    row(Act::QueueMoveDown, Queue, "move_down", &["J"], "move down"),
    // queue.rs:451.
    row(Act::QueueMoveUp, Queue, "move_up", &["K"], "move up"),
    // queue.rs:455-466.
    row(Act::QueuePause, Queue, "pause", &["P"], "pause / resume"),
    // queue.rs:467-473.
    row(Act::QueueDequeue, Queue, "dequeue", &["Q"], "dequeue"),
    // queue.rs:474.
    row(Act::QueueOpen, Queue, "open", &["enter"], "open"),
    // [picker]: the directory picker (MOD-67 M6 D3). `j`/`k` are `list.down`/`up`, `Esc` is
    // `common.back` (PA-10).
    // path_picker.rs:151-154.
    row(
        Act::PickerOpen,
        Picker,
        "open",
        &["enter", "l"],
        "open directory",
    ),
    // path_picker.rs:155-167. Not `up`: that would hide the inherited `list.up` (PA-10).
    row(
        Act::PickerParent,
        Picker,
        "parent",
        &["h", "backspace"],
        "parent directory",
    ),
    // path_picker.rs:168-170.
    row(Act::PickerChoose, Picker, "choose", &["s"], "choose"),
    // path_picker.rs:171-173.
    row(
        Act::PickerChooseDir,
        Picker,
        "choose_dir",
        &["S"],
        "choose this directory",
    ),
    // path_picker.rs:174-177.
    row(Act::PickerGoTo, Picker, "go_to", &["/"], "go to path"),
    // path_picker.rs:178-181.
    row(Act::PickerHidden, Picker, "hidden", &["."], "show hidden"),
    // [layout]: MOD-115 M2 D2/D4, the list + detail tabs' layout (B1: its own context, only in
    // the list + detail browse stacks). `p`/`P` give way to a view's own letter (D4a,
    // `SHADOWING`); `<`/`>` need no AltGr on IT/DE/FR keyboards; `{`/`}` do on IT, so the ctrl
    // arrows are the AltGr-free default.
    row(
        Act::LayoutNextPreset,
        Layout,
        "next_preset",
        &["p"],
        "next layout",
    )
    .since(2),
    row(
        Act::LayoutPrevPreset,
        Layout,
        "prev_preset",
        &["P"],
        "previous layout",
    )
    .since(2),
    row(
        Act::LayoutGrowList,
        Layout,
        "grow_list",
        &[">"],
        "wider list",
    )
    .since(2),
    row(
        Act::LayoutShrinkList,
        Layout,
        "shrink_list",
        &["<"],
        "narrower list",
    )
    .since(2),
    row(
        Act::LayoutGrowActivity,
        Layout,
        "grow_activity",
        &["}", "ctrl-down"],
        "larger activity",
    )
    .since(2),
    row(
        Act::LayoutShrinkActivity,
        Layout,
        "shrink_activity",
        &["{", "ctrl-up"],
        "smaller activity",
    )
    .since(2),
    row(
        Act::LayoutToggleList,
        Layout,
        "toggle_list",
        &["f2", "alt-1"],
        "show/hide list",
    )
    .since(2),
    row(
        Act::LayoutToggleActivity,
        Layout,
        "toggle_activity",
        &["f3", "alt-2"],
        "show/hide activity",
    )
    .since(2),
    row(Act::LayoutZoom, Layout, "zoom", &["z"], "zoom").since(2),
    // [history]: MOD-115 M2 D5. A capturing field keeps `backspace` (no capturing stack has
    // the layer, B1); the directory picker keeps it as `picker.parent`.
    row(
        Act::HistoryBack,
        History,
        "back",
        &["backspace", "alt-left"],
        "back",
    )
    .since(2),
    row(
        Act::HistoryForward,
        History,
        "forward",
        &["alt-right"],
        "forward",
    )
    .since(2),
    // [leave]: MOD-115 M2 D6, the leave dialog. The second `ctrl-c` leaves (R-TUI-18).
    row(
        Act::LeaveConfirm,
        Leave,
        "confirm",
        &["enter", "ctrl-c"],
        "leave",
    )
    .since(2),
    row(Act::LeaveStay, Leave, "stay", &["esc", "n"], "stay").since(2),
    // [field]: MOD-115 M2 D7, a text field's editing keys: the `TextField`, `TextArea` and
    // `Composer` literals (text_field.rs, text_area.rs:236-239). All named keys, so in capture.
    capture_row(Act::FieldSubmit, Field, "submit", &["enter"], "submit").since(2),
    capture_row(Act::FieldCancel, Field, "cancel", &["esc"], "cancel").since(2),
    capture_row(Act::FieldLeft, Field, "left", &["left"], "cursor left").since(2),
    capture_row(Act::FieldRight, Field, "right", &["right"], "cursor right").since(2),
    capture_row(Act::FieldHome, Field, "home", &["home"], "line start").since(2),
    capture_row(Act::FieldEnd, Field, "end", &["end"], "line end").since(2),
    capture_row(
        Act::FieldDeleteBack,
        Field,
        "delete_back",
        &["backspace"],
        "delete back",
    )
    .since(2),
    capture_row(
        Act::FieldDeleteForward,
        Field,
        "delete_forward",
        &["delete"],
        "delete forward",
    )
    .since(2),
    capture_row(Act::FieldLineUp, Field, "line_up", &["up"], "line up").since(2),
    capture_row(
        Act::FieldLineDown,
        Field,
        "line_down",
        &["down"],
        "line down",
    )
    .since(2),
    capture_row(Act::FieldPageUp, Field, "page_up", &["pgup"], "page up").since(2),
    capture_row(
        Act::FieldPageDown,
        Field,
        "page_down",
        &["pgdn"],
        "page down",
    )
    .since(2),
    // [choice]: MOD-115 M2 D7, a choice field's value keys (filter.rs:254-258,
    // item_form.rs:753-759, runs.rs:1126-1127, attach.rs:271). Printable, so view-resolved.
    row(
        Act::ChoicePrev,
        Choice,
        "prev",
        &["left", "h"],
        "previous option",
    )
    .since(2),
    row(
        Act::ChoiceNext,
        Choice,
        "next",
        &["right", "l"],
        "next option",
    )
    .since(2),
    row(Act::ChoiceToggle, Choice, "toggle", &["space"], "toggle").since(2),
    // [answer]: MOD-115 M2 D7, a parked request's options (runs.rs:1669-1675,
    // chat/mod.rs:603-611). One help label, so the `?` box merges them into `1-9 answer`.
    row(Act::AnswerOption1, Answer, "option_1", &["1"], "answer").since(2),
    row(Act::AnswerOption2, Answer, "option_2", &["2"], "answer").since(2),
    row(Act::AnswerOption3, Answer, "option_3", &["3"], "answer").since(2),
    row(Act::AnswerOption4, Answer, "option_4", &["4"], "answer").since(2),
    row(Act::AnswerOption5, Answer, "option_5", &["5"], "answer").since(2),
    row(Act::AnswerOption6, Answer, "option_6", &["6"], "answer").since(2),
    row(Act::AnswerOption7, Answer, "option_7", &["7"], "answer").since(2),
    row(Act::AnswerOption8, Answer, "option_8", &["8"], "answer").since(2),
    row(Act::AnswerOption9, Answer, "option_9", &["9"], "answer").since(2),
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
    // library.rs:1018: the rename form also moves on `Down`/`Up` (MOD-67 M4 §3.3).
    (SkillsLibrary, Act::FormNextField, &["down"]),
    (SkillsLibrary, Act::FormPrevField, &["up"]),
    // attach.rs:487, :488.
    (SkillsAttach, Act::FormNextField, &["down"]),
    (SkillsAttach, Act::FormPrevField, &["up"]),
    // requirements/forms.rs:96: the area form; every Requirements form takes the rows (§1).
    (Requirements, Act::FormNextField, &["down"]),
    (Requirements, Act::FormPrevField, &["up"]),
    // runs.rs:746, :747: the document view scrolls on `j`/`k` as on `J`/`K` (only
    // `RUNS_ARTIFACT` offers the scroll acts, so `J`/`K` stay the step cursor in browse; MOD-67
    // M5 §3.3).
    (BacklogRuns, Act::PaneScrollDown, &["j"]),
    (BacklogRuns, Act::PaneScrollUp, &["k"]),
    // compose.rs:376, :377: the kind and title rows pass `Down`/`Up`.
    (BacklogDocs, Act::FormNextField, &["down"]),
    (BacklogDocs, Act::FormPrevField, &["up"]),
    // filter.rs:262, :263.
    (BacklogFilter, Act::FormNextField, &["down"]),
    (BacklogFilter, Act::FormPrevField, &["up"]),
    // item_form.rs:664, :665 (a picker passes `Down`/`Up`, :735).
    (BacklogItem, Act::FormNextField, &["down"]),
    (BacklogItem, Act::FormPrevField, &["up"]),
];

/// Default pairs in one declared stack where the narrower act always wins (ANA-26 §7.4 step 7,
/// "shadowing"; MOD-67 D11 as amended by PA-2): allowed only for a chord that is a default of
/// both, with the first act's layer narrower. Each entry is demanded by the compiled defaults
/// (`every_allow_list_entry_is_demanded`).
pub static SHADOWING: &[(Act, Act)] = &[
    // migration_prompt.rs:97 + VIEW_DEFAULTS: `[migration] no` derives ["n", "esc", "N"], and
    // `Esc` is also overlay.close below it. Both close the prompt, nothing applied.
    (Act::ConfirmNo, Act::OverlayClose),
    // graph.rs:712: the Graph pane answers `Enter` itself (re-root), always; the Backlog's
    // replay miss never fires there (MOD-67 M5 PA-2).
    (Act::GraphReroot, Act::BacklogReplay),
    // MOD-115 M2 D4a (blueprint §1.6): a view's own letter wins over the layout's. `p` promotes
    // in Runs and is the previous template in Prompt; `P` pauses the queue on every Backlog
    // sub-tab, so there no default cycles the presets backwards.
    (Act::RunsPromote, Act::LayoutNextPreset),
    (Act::PromptPrevTemplate, Act::LayoutNextPreset),
    (Act::BacklogPauseQueue, Act::LayoutPrevPreset),
    // concepts_search.rs:334-335: the query field passes `Esc` (cancel) and `overlay.close`
    // closes the box: both end the search.
    (Act::FieldCancel, Act::OverlayClose),
    // requirements/forms.rs:326: on the priority `l` sets later, as today; `h` toggles.
    (Act::RequirementsPriorityLater, Act::ChoiceNext),
    // concepts_search.rs:315-319: the query is one line, so the hit cursor always wins.
    (Act::ConceptsUp, Act::FieldLineUp),
    (Act::ConceptsDown, Act::FieldLineDown),
    // agent_help.rs:328: the help prompt's arrows pick the agent.
    (Act::SkillsHelpPrevAgent, Act::FieldLineUp),
    (Act::SkillsHelpNextAgent, Act::FieldLineDown),
];

/// The version-1 defaults of every row M2 changed (MOD-115 M2 D8): each `since: 2` row that
/// existed before M2, with the defaults it had then. A version-1 key file's entry takes only an
/// M2 chord from a default: a chord listed here already met the file's entries before M2, and the
/// file loaded (or was refused) with it. A row M2 appended had no chord before.
pub static V1_DEFAULTS: &[(Act, &[&str])] = &[
    (Act::Quit, &["q"]),
    (Act::NextTab, &["tab"]),
    (Act::PrevTab, &["backtab"]),
    (Act::PaneNextSubtab, &["l", "]", "right"]),
    (Act::PanePrevSubtab, &["h", "[", "left"]),
    (Act::NextSection, &["l", "]", "right"]),
    (Act::PrevSection, &["h", "[", "left"]),
    (
        Act::SkillsSwitchView,
        &["h", "l", "[", "]", "left", "right"],
    ),
];

/// Default pairs that share a chord in one context or stack because a view accepts at most one
/// of them in any state and declines the other (ANA-26 §7.4 step 7, "state-guarded"). M2's
/// reviewed allow-list; each entry is demanded by the compiled defaults.
pub static STATE_GUARDED: &[(Act, Act)] = &[
    (Act::Back, Act::Dismiss),
    // boxes.rs:712-721: the executor question opens only over a listed box with a readable
    // list; otherwise `w` declines and falls through to the switcher (box_settings.rs:1356).
    (Act::BoxesExecutor, Act::Workspaces),
    // backlog/mod.rs:779-786, ANA-26 §7.4: a project header folds; on an item row the fold
    // declines and the pane (Runs replays) or the miss answers. The tab tries the fold first
    // (MOD-67 M5 PA-2).
    (Act::ListFold, Act::BacklogReplay),
    // On a header the Graph has no item; on an item row the fold declines.
    (Act::ListFold, Act::GraphReroot),
    // MOD-115 M2 §1.6: a form focuses a text field or a choice, never both (the filter form,
    // the item form, the Requirements forms).
    (Act::FieldLeft, Act::ChoicePrev),
    (Act::FieldRight, Act::ChoiceNext),
    // A `TextArea` takes `Down`/`Up`, a `TextField` passes them to the form's field moves
    // (text_field.rs:197, the `VIEW_DEFAULTS` rows).
    (Act::FieldLineDown, Act::FormNextField),
    (Act::FieldLineUp, Act::FormPrevField),
    // runs.rs:1669-1675, chat/mod.rs:603-611: the digits answer only while a request is parked,
    // else they select a tab.
    (Act::AnswerOption1, Act::SelectTab1),
    (Act::AnswerOption2, Act::SelectTab2),
    (Act::AnswerOption3, Act::SelectTab3),
    (Act::AnswerOption4, Act::SelectTab4),
    (Act::AnswerOption5, Act::SelectTab5),
    (Act::AnswerOption6, Act::SelectTab6),
    (Act::AnswerOption7, Act::SelectTab7),
    (Act::AnswerOption8, Act::SelectTab8),
    (Act::AnswerOption9, Act::SelectTab9),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{Act, CATALOGUE, Context, STATE_GUARDED, V1_DEFAULTS};

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
            Context::SettingsProjectTypes => 19,
            Context::Concepts => 20,
            Context::Switcher => 21,
            Context::Migration => 22,
            Context::Waiting => 23,
            Context::Skills => 24,
            Context::SkillsLibrary => 25,
            Context::SkillsTemplates => 26,
            Context::SkillsAttach => 27,
            Context::SkillsHelp => 28,
            Context::Requirements => 29,
            Context::Backlog => 30,
            Context::BacklogBody => 31,
            Context::BacklogRuns => 32,
            Context::BacklogGraph => 33,
            Context::BacklogDocs => 34,
            Context::BacklogNotes => 35,
            Context::BacklogPrompt => 36,
            Context::BacklogReqs => 37,
            Context::BacklogFilter => 38,
            Context::BacklogItem => 39,
            Context::Chat => 40,
            Context::Queue => 41,
            Context::Picker => 42,
            Context::Layout => 43,
            Context::History => 44,
            Context::Leave => 45,
            Context::Field => 46,
            Context::Choice => 47,
            Context::Answer => 48,
        }
    }

    #[test]
    fn every_context_is_listed_once_in_all() {
        assert_eq!(Context::ALL.len(), 49);
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
        Act::About,
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
        Act::BacklogReplay,
        Act::BacklogOpenGraph,
        Act::BacklogOpenFilter,
        Act::BacklogClearFilter,
        Act::BacklogNewItem,
        Act::BacklogQueue,
        Act::BacklogPauseQueue,
        Act::RunsNextStep,
        Act::RunsPrevStep,
        Act::RunsApprove,
        Act::RunsReject,
        Act::RunsRetry,
        Act::RunsPromote,
        Act::RunsSelect,
        Act::RunsAccept,
        Act::RunsOpen,
        Act::RunsCancelRun,
        Act::RunsCleanup,
        Act::RunsUnblock,
        Act::RunsStartRun,
        Act::RunsCloseOut,
        Act::RunsFollowUp,
        Act::RunsToggleView,
        Act::RunsZoomIn,
        Act::RunsZoomOut,
        Act::RunsZoomFit,
        Act::GraphNextRow,
        Act::GraphPrevRow,
        Act::GraphDeeper,
        Act::GraphShallower,
        Act::GraphReroot,
        Act::DocsNextRow,
        Act::DocsPrevRow,
        Act::DocsAdd,
        Act::DocsNewVersion,
        Act::NotesAdd,
        Act::PromptNextTemplate,
        Act::PromptPrevTemplate,
        Act::ReqsNextRow,
        Act::ReqsPrevRow,
        Act::ReqsReconfirm,
        Act::ReqsCite,
        Act::ReqsUncite,
        Act::ReqsPick,
        Act::ReqsAddresses,
        Act::ReqsReserves,
        Act::FilterNextRow,
        Act::FilterPrevRow,
        Act::FilterClear,
        Act::ItemTheirs,
        Act::ItemMine,
        Act::ItemSwitchDiff,
        Act::ChatCompose,
        Act::ChatNextAgent,
        Act::ChatEnd,
        Act::ChatThoughts,
        Act::ChatTop,
        Act::ChatBottom,
        Act::QueueMoveDown,
        Act::QueueMoveUp,
        Act::QueuePause,
        Act::QueueDequeue,
        Act::QueueOpen,
        Act::PickerOpen,
        Act::PickerParent,
        Act::PickerChoose,
        Act::PickerChooseDir,
        Act::PickerGoTo,
        Act::PickerHidden,
        Act::NextPane,
        Act::PrevPane,
        Act::LayoutNextPreset,
        Act::LayoutPrevPreset,
        Act::LayoutGrowList,
        Act::LayoutShrinkList,
        Act::LayoutGrowActivity,
        Act::LayoutShrinkActivity,
        Act::LayoutToggleList,
        Act::LayoutToggleActivity,
        Act::LayoutZoom,
        Act::HistoryBack,
        Act::HistoryForward,
        Act::LeaveConfirm,
        Act::LeaveStay,
        Act::FieldSubmit,
        Act::FieldCancel,
        Act::FieldLeft,
        Act::FieldRight,
        Act::FieldHome,
        Act::FieldEnd,
        Act::FieldDeleteBack,
        Act::FieldDeleteForward,
        Act::FieldLineUp,
        Act::FieldLineDown,
        Act::FieldPageUp,
        Act::FieldPageDown,
        Act::ChoicePrev,
        Act::ChoiceNext,
        Act::ChoiceToggle,
        Act::AnswerOption1,
        Act::AnswerOption2,
        Act::AnswerOption3,
        Act::AnswerOption4,
        Act::AnswerOption5,
        Act::AnswerOption6,
        Act::AnswerOption7,
        Act::AnswerOption8,
        Act::AnswerOption9,
        Act::RequirementsPriorityMust,
        Act::RequirementsPriorityLater,
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
            Act::About => 13,
            Act::Workspaces => 14,
            Act::Find => 15,
            Act::Waiting => 16,
            Act::Queue => 17,
            Act::OverlayClose => 18,
            Act::ListDown => 19,
            Act::ListUp => 20,
            Act::ListTop => 21,
            Act::ListBottom => 22,
            Act::ListFold => 23,
            Act::PaneScrollDown => 24,
            Act::PaneScrollUp => 25,
            Act::PanePageDown => 26,
            Act::PanePageUp => 27,
            Act::PaneNextSubtab => 28,
            Act::PanePrevSubtab => 29,
            Act::ConfirmYes => 30,
            Act::ConfirmNo => 31,
            Act::FormNextField => 32,
            Act::FormPrevField => 33,
            Act::FormSave => 34,
            Act::FormExternalEditor => 35,
            Act::Edit => 36,
            Act::New => 37,
            Act::Delete => 38,
            Act::Clear => 39,
            Act::Reload => 40,
            Act::Back => 41,
            Act::Dismiss => 42,
            Act::EditorFocus => 43,
            Act::EditorAbort => 44,
            Act::NextSection => 45,
            Act::PrevSection => 46,
            Act::AgentsProbe => 47,
            Act::AgentsInstall => 48,
            Act::AgentsAuthenticate => 49,
            Act::AgentsSwitchBox => 50,
            Act::AgentsEditPaths => 51,
            Act::AgentsOpenLink => 52,
            Act::AgentsPasteRedirect => 53,
            Act::AgentsCancel => 54,
            Act::AgentsChoose => 55,
            Act::HierarchyNewWorkspace => 56,
            Act::HierarchyPrimary => 57,
            Act::HierarchyChoosePath => 58,
            Act::HierarchyInfer => 59,
            Act::KindsNewGraph => 60,
            Act::KindsGraph => 61,
            Act::ConnectionRebuild => 62,
            Act::ConnectionActivate => 63,
            Act::BoxesEditTags => 64,
            Act::BoxesEditQuirks => 65,
            Act::BoxesExecutor => 66,
            Act::BoxesProbe => 67,
            Act::BoxesEditSpec => 68,
            Act::PersonasBody => 69,
            Act::PersonasRules => 70,
            Act::PersonasImport => 71,
            Act::SecretsCheck => 72,
            Act::ConceptsDecisions => 73,
            Act::ConceptsProject => 74,
            Act::ConceptsReindex => 75,
            Act::ConceptsUp => 76,
            Act::ConceptsDown => 77,
            Act::SwitcherSwitch => 78,
            Act::WaitingOpen => 79,
            Act::SkillsSwitchView => 80,
            Act::SkillsPrevVersion => 81,
            Act::SkillsNextVersion => 82,
            Act::SkillsBase => 83,
            Act::SkillsDiff => 84,
            Act::SkillsEditExternally => 85,
            Act::SkillsAskAgent => 86,
            Act::LibraryImport => 87,
            Act::LibraryInfo => 88,
            Act::LibraryAttach => 89,
            Act::TemplatesDiffDefault => 90,
            Act::AttachChoose => 91,
            Act::AttachDetach => 92,
            Act::AttachRepo => 93,
            Act::SkillsHelpPrevAgent => 94,
            Act::SkillsHelpNextAgent => 95,
            Act::SkillsHelpAccept => 96,
            Act::SkillsHelpCancel => 97,
            Act::RequirementsNewArea => 98,
            Act::RequirementsAmend => 99,
            Act::RequirementsWithdraw => 100,
            Act::RequirementsFilter => 101,
            Act::BacklogReplay => 102,
            Act::BacklogOpenGraph => 103,
            Act::BacklogOpenFilter => 104,
            Act::BacklogClearFilter => 105,
            Act::BacklogNewItem => 106,
            Act::BacklogQueue => 107,
            Act::BacklogPauseQueue => 108,
            Act::RunsNextStep => 109,
            Act::RunsPrevStep => 110,
            Act::RunsApprove => 111,
            Act::RunsReject => 112,
            Act::RunsRetry => 113,
            Act::RunsPromote => 114,
            Act::RunsSelect => 115,
            Act::RunsAccept => 116,
            Act::RunsOpen => 117,
            Act::RunsCancelRun => 118,
            Act::RunsCleanup => 119,
            Act::RunsUnblock => 120,
            Act::RunsStartRun => 121,
            Act::RunsCloseOut => 122,
            Act::RunsFollowUp => 123,
            Act::RunsToggleView => 124,
            Act::RunsZoomIn => 125,
            Act::RunsZoomOut => 126,
            Act::RunsZoomFit => 127,
            Act::GraphNextRow => 128,
            Act::GraphPrevRow => 129,
            Act::GraphDeeper => 130,
            Act::GraphShallower => 131,
            Act::GraphReroot => 132,
            Act::DocsNextRow => 133,
            Act::DocsPrevRow => 134,
            Act::DocsAdd => 135,
            Act::DocsNewVersion => 136,
            Act::NotesAdd => 137,
            Act::PromptNextTemplate => 138,
            Act::PromptPrevTemplate => 139,
            Act::ReqsNextRow => 140,
            Act::ReqsPrevRow => 141,
            Act::ReqsReconfirm => 142,
            Act::ReqsCite => 143,
            Act::ReqsUncite => 144,
            Act::ReqsPick => 145,
            Act::ReqsAddresses => 146,
            Act::ReqsReserves => 147,
            Act::FilterNextRow => 148,
            Act::FilterPrevRow => 149,
            Act::FilterClear => 150,
            Act::ItemTheirs => 151,
            Act::ItemMine => 152,
            Act::ItemSwitchDiff => 153,
            Act::ChatCompose => 154,
            Act::ChatNextAgent => 155,
            Act::ChatEnd => 156,
            Act::ChatThoughts => 157,
            Act::ChatTop => 158,
            Act::ChatBottom => 159,
            Act::QueueMoveDown => 160,
            Act::QueueMoveUp => 161,
            Act::QueuePause => 162,
            Act::QueueDequeue => 163,
            Act::QueueOpen => 164,
            Act::PickerOpen => 165,
            Act::PickerParent => 166,
            Act::PickerChoose => 167,
            Act::PickerChooseDir => 168,
            Act::PickerGoTo => 169,
            Act::PickerHidden => 170,
            Act::NextPane => 171,
            Act::PrevPane => 172,
            Act::LayoutNextPreset => 173,
            Act::LayoutPrevPreset => 174,
            Act::LayoutGrowList => 175,
            Act::LayoutShrinkList => 176,
            Act::LayoutGrowActivity => 177,
            Act::LayoutShrinkActivity => 178,
            Act::LayoutToggleList => 179,
            Act::LayoutToggleActivity => 180,
            Act::LayoutZoom => 181,
            Act::HistoryBack => 182,
            Act::HistoryForward => 183,
            Act::LeaveConfirm => 184,
            Act::LeaveStay => 185,
            Act::FieldSubmit => 186,
            Act::FieldCancel => 187,
            Act::FieldLeft => 188,
            Act::FieldRight => 189,
            Act::FieldHome => 190,
            Act::FieldEnd => 191,
            Act::FieldDeleteBack => 192,
            Act::FieldDeleteForward => 193,
            Act::FieldLineUp => 194,
            Act::FieldLineDown => 195,
            Act::FieldPageUp => 196,
            Act::FieldPageDown => 197,
            Act::ChoicePrev => 198,
            Act::ChoiceNext => 199,
            Act::ChoiceToggle => 200,
            Act::AnswerOption1 => 201,
            Act::AnswerOption2 => 202,
            Act::AnswerOption3 => 203,
            Act::AnswerOption4 => 204,
            Act::AnswerOption5 => 205,
            Act::AnswerOption6 => 206,
            Act::AnswerOption7 => 207,
            Act::AnswerOption8 => 208,
            Act::AnswerOption9 => 209,
            Act::RequirementsPriorityMust => 210,
            Act::RequirementsPriorityLater => 211,
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
    fn global_about_is_f9_and_leaves_help_alone() {
        let about = Act::About.spec().expect("about has a row");
        assert_eq!(about.defaults, ["f9"]);
        assert_eq!(about.context, Context::Global);
        let help = Act::Help.spec().expect("help has a row");
        assert_eq!(help.defaults, ["?", "f1"]);
    }

    #[test]
    fn every_act_has_exactly_one_row() {
        assert_eq!(CATALOGUE.len(), 212);
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

    /// MOD-115 M2 §1.4 (MD2): `ctrl-c` is an ordinary default chord now, of `global.quit` and
    /// `leave.confirm` only, spelled `ctrl-c` (no other spelling of it is a default).
    #[test]
    fn only_quit_and_leave_confirm_default_to_ctrl_c() {
        let mut holders = Vec::new();
        for row in CATALOGUE {
            for spec in row.defaults {
                let folded: String = spec
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .to_lowercase();
                if ["ctrl-c", "ctrl+c", "control-c"].contains(&folded.as_str()) {
                    assert_eq!(*spec, "ctrl-c", "{:?} spells ctrl-c {spec:?}", row.act);
                    holders.push(row.act);
                }
            }
        }
        assert_eq!(holders, [Act::Quit, Act::LeaveConfirm]);
    }

    /// MOD-115 M2 §1.1: every row M2 adds or changes, and nothing else, is `since: 2`.
    #[test]
    fn every_m2_row_is_since_2() {
        let changed = [
            Act::Quit,
            Act::NextTab,
            Act::PrevTab,
            Act::PaneNextSubtab,
            Act::PanePrevSubtab,
            Act::NextSection,
            Act::PrevSection,
            Act::SkillsSwitchView,
        ];
        let added = ALL
            .iter()
            .copied()
            .skip_while(|act| *act != Act::NextPane)
            .collect::<Vec<_>>();
        assert_eq!(added.len(), 41, "M2 appends 41 acts");
        for row in CATALOGUE {
            let m2 = changed.contains(&row.act) || added.contains(&row.act);
            assert_eq!(row.since == 2, m2, "{:?} is since {}", row.act, row.since);
        }
    }

    /// MOD-115 M2 D8: `V1_DEFAULTS` lists exactly the `since: 2` rows that existed before M2,
    /// each with defaults that parse.
    #[test]
    fn the_v1_defaults_are_the_changed_rows_before_m2() {
        let appended: Vec<Act> = ALL
            .iter()
            .copied()
            .skip_while(|act| *act != Act::NextPane)
            .collect();
        let changed: Vec<Act> = CATALOGUE
            .iter()
            .filter(|row| row.since == 2 && !appended.contains(&row.act))
            .map(|row| row.act)
            .collect();
        let listed: Vec<Act> = V1_DEFAULTS.iter().map(|&(act, _)| act).collect();
        assert_eq!(listed, changed);
        for &(act, defaults) in V1_DEFAULTS {
            for written in defaults {
                assert!(
                    crate::keys::KeyChord::parse_strict(written).is_ok(),
                    "{act:?} {written:?}"
                );
            }
        }
    }

    #[test]
    fn since_is_1_or_2() {
        for row in CATALOGUE {
            assert!(matches!(row.since, 1 | 2), "{:?}: {}", row.act, row.since);
        }
    }

    /// MOD-115 M2 §1.4 (B6, D-4, plan D4): Task 6 flips the meanings with their wiring: `tab`
    /// and `backtab` move the pane focus, the tabs move to `ctrl-pgdn`/`ctrl-pgup`, and the
    /// sub-tab, section and view switches take the shift arrows instead of the plain ones
    /// (arrows stay in the pane), appended so every hint's first chord stays.
    #[test]
    fn the_m2_rows_have_their_task_6_defaults() {
        let defaults = |act: Act| act.spec().expect("a row").defaults;
        assert_eq!(defaults(Act::Quit), ["q", "ctrl-c"]);
        assert_eq!(defaults(Act::NextTab), ["ctrl-pgdn"]);
        assert_eq!(defaults(Act::PrevTab), ["ctrl-pgup"]);
        assert_eq!(defaults(Act::NextPane), ["tab", "ctrl-right"]);
        assert_eq!(defaults(Act::PrevPane), ["backtab", "ctrl-left"]);
        assert_eq!(defaults(Act::PaneNextSubtab), ["l", "]", "shift-right"]);
        assert_eq!(defaults(Act::PanePrevSubtab), ["h", "[", "shift-left"]);
        assert_eq!(defaults(Act::NextSection), ["l", "]", "shift-right"]);
        assert_eq!(defaults(Act::PrevSection), ["h", "[", "shift-left"]);
        assert_eq!(
            defaults(Act::SkillsSwitchView),
            ["h", "l", "[", "]", "shift-left", "shift-right"]
        );
        assert_eq!(defaults(Act::LayoutNextPreset), ["p"]);
        assert_eq!(defaults(Act::LayoutPrevPreset), ["P"]);
        assert_eq!(defaults(Act::LayoutGrowList), [">"]);
        assert_eq!(defaults(Act::LayoutShrinkList), ["<"]);
        assert_eq!(defaults(Act::LayoutGrowActivity), ["}", "ctrl-down"]);
        assert_eq!(defaults(Act::LayoutShrinkActivity), ["{", "ctrl-up"]);
        assert_eq!(defaults(Act::LayoutToggleList), ["f2", "alt-1"]);
        assert_eq!(defaults(Act::LayoutToggleActivity), ["f3", "alt-2"]);
        assert_eq!(defaults(Act::LayoutZoom), ["z"]);
        assert_eq!(defaults(Act::HistoryBack), ["backspace", "alt-left"]);
        assert_eq!(defaults(Act::HistoryForward), ["alt-right"]);
        assert_eq!(defaults(Act::LeaveConfirm), ["enter", "ctrl-c"]);
        assert_eq!(defaults(Act::LeaveStay), ["esc", "n"]);
        assert_eq!(defaults(Act::ChoicePrev), ["left", "h"]);
        assert_eq!(defaults(Act::ChoiceNext), ["right", "l"]);
        assert_eq!(defaults(Act::ChoiceToggle), ["space"]);
        assert_eq!(defaults(Act::RequirementsPriorityMust), ["m"]);
        assert_eq!(defaults(Act::RequirementsPriorityLater), ["l"]);
        let field: Vec<&[&str]> = [
            Act::FieldSubmit,
            Act::FieldCancel,
            Act::FieldLeft,
            Act::FieldRight,
            Act::FieldHome,
            Act::FieldEnd,
            Act::FieldDeleteBack,
            Act::FieldDeleteForward,
            Act::FieldLineUp,
            Act::FieldLineDown,
            Act::FieldPageUp,
            Act::FieldPageDown,
        ]
        .into_iter()
        .map(defaults)
        .collect();
        assert_eq!(
            field,
            [
                &["enter"][..],
                &["esc"],
                &["left"],
                &["right"],
                &["home"],
                &["end"],
                &["backspace"],
                &["delete"],
                &["up"],
                &["down"],
                &["pgup"],
                &["pgdn"],
            ]
        );
    }

    #[test]
    fn answer_index_maps_the_nine_options() {
        let options = [
            Act::AnswerOption1,
            Act::AnswerOption2,
            Act::AnswerOption3,
            Act::AnswerOption4,
            Act::AnswerOption5,
            Act::AnswerOption6,
            Act::AnswerOption7,
            Act::AnswerOption8,
            Act::AnswerOption9,
        ];
        for (index, act) in options.into_iter().enumerate() {
            assert_eq!(act.answer_index(), Some(index), "{act:?}");
            assert_eq!(act.tab_index(), None, "{act:?}");
            let spec = act.spec().expect("a row");
            assert_eq!(spec.context, Context::Answer);
            assert_eq!(spec.defaults, [format!("{}", index + 1)]);
        }
        assert_eq!(Act::SelectTab1.answer_index(), None);
        assert_eq!(Act::Quit.answer_index(), None);
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
                "about",
                "workspaces",
                "find",
                "waiting",
                "queue",
                // MOD-115 M2 §1.4: after `queue`, so every pinned keys line keeps its prefix.
                "next_pane",
                "prev_pane",
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
            (
                Context::SettingsProjectTypes,
                "settings.project_types",
                "Project types",
            ),
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
            (Context::Backlog, "backlog", "Backlog"),
            (Context::BacklogBody, "backlog.body", "Body"),
            (Context::BacklogRuns, "backlog.runs", "Runs"),
            (Context::BacklogGraph, "backlog.graph", "Graph"),
            (Context::BacklogDocs, "backlog.docs", "Docs"),
            (Context::BacklogNotes, "backlog.notes", "Notes"),
            (Context::BacklogPrompt, "backlog.prompt", "Prompt"),
            (Context::BacklogReqs, "backlog.reqs", "Reqs"),
            (Context::BacklogFilter, "backlog.filter_form", "Filter"),
            (Context::BacklogItem, "backlog.item", "Item"),
            (Context::Chat, "chat", "Chat"),
            (Context::Queue, "queue", "Queue"),
            (Context::Picker, "picker", "Picker"),
            (Context::Layout, "layout", "Layout"),
            (Context::History, "history", "History"),
            (Context::Leave, "leave", "Leave htui?"),
            (Context::Field, "field", "Text field"),
            (Context::Choice, "choice", "Choice"),
            (Context::Answer, "answer", "Answer"),
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
        // MOD-146 D8: Project types is a view; `is_view` is a `matches!`, so only this catches
        // a context left out of it.
        assert!(
            Context::SettingsProjectTypes.is_view() && !Context::SettingsProjectTypes.is_shared()
        );
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
        // MOD-67 M5 §3.1: `backlog` is shared (every Backlog sub-tab inherits it), the rest views.
        assert!(Context::Backlog.is_shared() && !Context::Backlog.is_view());
        for context in [
            Context::BacklogBody,
            Context::BacklogRuns,
            Context::BacklogGraph,
            Context::BacklogDocs,
            Context::BacklogNotes,
            Context::BacklogPrompt,
            Context::BacklogReqs,
            Context::BacklogFilter,
            Context::BacklogItem,
            Context::Chat,
        ] {
            assert!(context.is_view() && !context.is_shared(), "{context:?}");
        }
        // MOD-67 M6 §4.1: the queue overlay and the directory picker are views.
        for context in [Context::Queue, Context::Picker] {
            assert!(context.is_view() && !context.is_shared(), "{context:?}");
        }
        // MOD-115 M2 §1.2: the leave dialog is an overlay's view; the shell's and the fields'
        // contexts are neither views nor shared (B1: `history.back` would clash with the shared
        // `common.back` name).
        assert!(Context::Leave.is_view() && !Context::Leave.is_shared());
        for context in [
            Context::Layout,
            Context::History,
            Context::Field,
            Context::Choice,
            Context::Answer,
        ] {
            assert!(!context.is_view() && !context.is_shared(), "{context:?}");
        }
    }

    /// A key file nests a context's table under its parent's (`[backlog.runs]` is `runs` in
    /// `[backlog]`), so an act named like a nested context would be one TOML key twice: the
    /// default print would not read back (MOD-67 M5).
    #[test]
    fn no_act_name_is_a_nested_table() {
        for row in CATALOGUE {
            let nested = format!("{}.{}", row.context.table(), row.name);
            assert!(
                Context::ALL.iter().all(|context| context.table() != nested),
                "[{}] {} is also the table [{nested}]",
                row.context.table(),
                row.name
            );
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
    fn in_capture_acts_are_the_form_overlay_close_editor_focus_concepts_skills_and_field_chords() {
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
                // MOD-115 M2 §1.4: a text field's editing keys, all named keys.
                Act::FieldSubmit,
                Act::FieldCancel,
                Act::FieldLeft,
                Act::FieldRight,
                Act::FieldHome,
                Act::FieldEnd,
                Act::FieldDeleteBack,
                Act::FieldDeleteForward,
                Act::FieldLineUp,
                Act::FieldLineDown,
                Act::FieldPageUp,
                Act::FieldPageDown,
            ])
        );
    }
}
