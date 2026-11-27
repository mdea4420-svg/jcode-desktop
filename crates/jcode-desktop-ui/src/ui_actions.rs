//! Actions that views dispatch to their containers.
//!
//! Panels and the composer ask the workspace to open or close things by
//! dispatching these. They live here, not in `workspace`, so `panel` and
//! `input` do not depend on `workspace` (which depends on both). Keeping the
//! view layers acyclic is what lets them move into separate crates later.
//! `workspace` and `panel::shortcuts` re-export them under their old paths.
//!
//! Action names are unchanged: unit actions keep their `workspace` and
//! `panel_shortcuts` namespaces, and the data actions are un-namespaced.

/// Route Desktop-local slash commands to the workspace containing the source.
/// Raw input is retained so validation never silently drops command arguments.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(no_json)]
pub(crate) struct WorkspaceSlashCommand {
    pub source: gpui::EntityId,
    pub content: String,
}

/// Toggle the project folder choices from a fresh conversation.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(no_json)]
pub(crate) struct ToggleProjectFolderMenu {
    pub source: Option<gpui::EntityId>,
    pub position: gpui::Point<gpui::Pixels>,
}

pub(crate) fn project_folder_plus_icon() -> gpui::Svg {
    const ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M2 5V4a1 1 0 0 1 1-1h3l2 2h5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V5ZM8 7v4M6 9h4" fill="none" stroke="black" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;
    use gpui::Styled;
    gpui::svg()
        .data(ICON)
        .size(gpui::px(14.0))
        .text_color(crate::theme::Theme::global().ACCENT)
}

/// Open the Accounts panel, optionally starting a login command.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(no_json)]
pub(crate) struct OpenAccounts {
    pub source: gpui::EntityId,
    pub login_command: Option<String>,
}

/// Open the Extensions overlay (MCP / Skills / Plugins tabs) on a panel.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(no_json)]
pub(crate) struct OpenExtensions {
    pub source: gpui::EntityId,
    pub tab: jcode_desktop_model::extensions::ExtensionTab,
}

/// Open the read-only usage reset review for one provider, from `/reset`.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(no_json)]
pub(crate) struct OpenUsageReset {
    pub provider: crate::accounts::ResetProvider,
}

/// Commit, push, release and publish Desktop from the source panel's checkout.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(no_json)]
pub(crate) struct PublishDesktop {
    pub source: gpui::EntityId,
}

/// Open the change review for one tool call's proposed edits.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(no_json)]
pub(crate) struct OpenChangeReview {
    pub source: gpui::EntityId,
    pub name: String,
    pub input: String,
    pub output: String,
    pub selected: usize,
    pub done: bool,
    pub failed: bool,
}

#[derive(Clone, PartialEq, gpui::Action)]
#[action(no_json)]
pub(crate) struct CloseChangeReview {
    pub panel: gpui::EntityId,
}

gpui::actions!(
    workspace,
    [
        OpenAppletShowcase,
        ClosePanel,
        ToggleOnboardingSimulator,
        OpenChangelog,
        OpenResume,
    ]
);

gpui::actions!(
    panel_shortcuts,
    [JumpToLatest, JumpToLatestIfEmpty, BackgroundRunningTool]
);

/// Alt+B/Ctrl+B move the running tool to the background, matching the TUI.
/// Both chords are also composer word-motion keys, so the handler propagates
/// when no tool is running. Registered from `input::bind_keys` right after the
/// composer's own bindings so it outranks them at the same context depth.
pub(crate) fn background_tool_bindings() -> [gpui::KeyBinding; 4] {
    [
        gpui::KeyBinding::new("alt-b", BackgroundRunningTool, Some("ChatPanel")),
        gpui::KeyBinding::new("ctrl-b", BackgroundRunningTool, Some("ChatPanel")),
        gpui::KeyBinding::new(
            "alt-b",
            BackgroundRunningTool,
            Some("ChatPanel > PromptInput"),
        ),
        gpui::KeyBinding::new(
            "ctrl-b",
            BackgroundRunningTool,
            Some("ChatPanel > PromptInput"),
        ),
    ]
}
