//! The "Extensions" overlay: one dialog managing MCP servers, skills and
//! fork plugins. Opened by the sidebar plug button and by `/mcp`, `/skills`
//! (bare) and `/extensions`; modelled on `panel_model_browser.rs`.
//!
//! All file I/O lives in `jcode_desktop_model::extensions`; this module only
//! renders cached lists and applies edits, refreshing after each mutation.
//! Edits apply to new sessions — the running daemon does not hot-reload MCP
//! or skill config, so the dialog says so under the list.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::*;
use crate::extensions::{ExtensionPaths, ExtensionTab, mcp, skills};
use crate::locale::tr;

/// The overlay's open state. Lists are read once on open and after each
/// edit — reads are a handful of small JSON files and directory scans,
/// cheap enough to redo rather than to invalidate precisely.
pub(super) struct ExtensionsState {
    /// Active tab (never `ExtensionTab::Last` once resolved).
    pub tab: ExtensionTab,
    pub search: Entity<PromptInput>,
    pub scroll: ScrollHandle,
    /// Last measured panel bounds, for the narrow-layout fallback.
    pub panel_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// Focus the search field on the first frame after opening.
    pub focus_pending: bool,
    pub paths: ExtensionPaths,
    /// Keeps the screenshot fixture's temp dir alive.
    #[allow(dead_code)]
    pub fixture: Option<tempfile::TempDir>,
    pub mcp: mcp::McpList,
    pub skills: Vec<skills::SkillEntry>,
    pub plugins: Vec<PluginRow>,
    pub plugin_errors: Vec<String>,
    pub plugin_filter: PluginFilter,
    /// Inline test results by server name.
    pub tests: HashMap<String, mcp::McpTestResult>,
    pub testing: HashSet<String>,
    /// One-line confirmation shown under the header (e.g. "Saved `fs`.").
    pub notice: Option<String>,
    /// Inline destructive/sensitive confirmations.
    pub confirm: Option<Confirm>,
    /// MCP add/edit form; replaces the list while set.
    pub mcp_form: Option<McpForm>,
    /// New-skill form.
    pub skill_form: Option<SkillForm>,
    pub plugin_form: Option<PluginForm>,
    pub plugin_studio: Option<super::plugin_studio::PluginStudio>,
    /// Import-skill path field (folder picker fallback).
    pub skill_import: Option<Entity<PromptInput>>,
    /// Import-plugin path field (folder picker fallback).
    pub plugin_import: Option<Entity<PromptInput>>,
    /// Error line from the last failed mutation.
    pub error: Option<String>,
}

/// A pending confirmation shown inline under the affected row's group.
pub(super) enum Confirm {
    /// Delete MCP `name` from `path`.
    McpDelete { name: String, path: PathBuf },
    /// Move the skill dir to `~/.jcode/trash/skills`.
    SkillDelete { dir: PathBuf },
    /// Toggle a skill living in a shared (`~/.agents`) location.
    SkillToggleShared { dir: PathBuf },
    /// Move a local `~/.jcode/plugins/<name>` dir to `~/.jcode/trash/plugins`.
    PluginDelete { name: String },
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum PluginFilter {
    #[default]
    All,
    Enabled,
    Local,
    Claude,
}

/// Which file a new/edited MCP server goes into.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum McpTarget {
    /// `~/.jcode/mcp.json`.
    Global,
    /// `<project>/.jcode/mcp.json`.
    Project,
}

/// One editable env/header row: key + value fields. Existing values render
/// masked (`••••`) until the row's reveal pill is pressed — secrets never
/// render unmasked by default.
pub(super) struct PairRow {
    pub key: Entity<PromptInput>,
    pub value: Entity<PromptInput>,
    /// Real value kept out of the input while masked.
    pub hidden: Option<String>,
}

/// The MCP add/edit form.
pub(super) struct McpForm {
    /// `(file, original name)` when editing; `None` when adding.
    pub editing: Option<(PathBuf, String)>,
    pub target: McpTarget,
    pub name: Entity<PromptInput>,
    pub transport: mcp::McpTransport,
    pub command: Entity<PromptInput>,
    /// Arg rows (one field per arg).
    pub args: Vec<Entity<PromptInput>>,
    pub env: Vec<PairRow>,
    pub url: Entity<PromptInput>,
    pub headers: Vec<PairRow>,
    /// `Paste JSON` mode: a paste field fills the form or imports many.
    pub paste: Option<Entity<PromptInput>>,
    pub error: Option<String>,
}

/// The new-skill form.
pub(super) struct SkillForm {
    pub name: Entity<PromptInput>,
    pub description: Entity<PromptInput>,
    /// `true` = `~/.jcode/skills`, `false` = `<project>/.jcode/skills`.
    pub global: bool,
    pub error: Option<String>,
}

pub(super) struct PluginForm {
    pub name: Entity<PromptInput>,
    pub code: bool,
    pub error: Option<String>,
}

/// One row of the Plugins tab.
pub(super) struct PluginRow {
    pub name: String,
    pub version: String,
    /// `true` = code plugin (`main` in plugin.json), `false` = declarative.
    pub code: bool,
    pub source: String,
    pub enabled: bool,
    pub root: PathBuf,
    pub entry_file: Option<PathBuf>,
    pub description: Option<String>,
    pub commands: Vec<String>,
    pub skills: usize,
    pub hook_events: usize,
    pub mcp_servers: usize,
    pub applets: usize,
}

thread_local! {
    /// The tab `/extensions` and the sidebar button reopen.
    static LAST_TAB: std::cell::Cell<ExtensionTab> = std::cell::Cell::new(ExtensionTab::Mcp);
}

impl ExtensionsState {
    /// Re-read all three lists from disk (called on open and after edits).
    pub(super) fn reload(&mut self) {
        self.mcp = mcp::list_mcp(&self.paths);
        self.skills = skills::list_skills(&self.paths);
        let (plugins, errors) = crate::plugins::plugins_in(&self.plugin_layout());
        self.plugins = plugins.iter().map(PluginRow::from_plugin).collect();
        self.plugin_errors = errors;
    }

    pub(super) fn plugin_layout(&self) -> jcode_plugins::Layout {
        crate::plugins::layout_for_paths(&self.paths)
    }

    pub(super) fn query(&self, cx: &App) -> String {
        self.search.read(cx).content.trim().to_lowercase()
    }
}

/// Insert zero-width break opportunities inside long unbroken paths or tokens
/// so GPUI text shaping wraps inside cards instead of overflowing horizontally.
pub(super) fn soft_wrap_text(text: &str) -> String {
    if !text
        .split_whitespace()
        .any(|word| word.chars().count() > 32)
    {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        out.push(ch);
        if matches!(ch, '/' | '\\' | ':' | '_' | '-') {
            out.push('\u{200B}');
        }
    }
    out
}

/// A small pill button used for row actions and header pills.
pub(super) fn pill(
    id: impl Into<gpui::ElementId>,
    selector: String,
    label: impl Into<gpui::SharedString>,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    div()
        .id(id)
        .debug_selector(move || selector.clone().into())
        .flex_none()
        .flex()
        .items_center()
        .px_2p5()
        .h(px(24.))
        .rounded_full()
        .border_1()
        .border_color(if active {
            theme.ACCENT
        } else {
            theme.PANEL_BORDER
        })
        .text_size(px(11.))
        .whitespace_nowrap()
        .cursor_pointer()
        .bg(if active { theme.ACCENT } else { theme.PANEL_BG })
        .text_color(if active { theme.BG } else { theme.TEXT })
        .when(!active, |el| el.hover(|style| style.bg(theme.ACCENT_DIM)))
        .child(label.into())
}

/// A single-line `PromptInput` field for forms and the search pill.
pub(super) fn field_input(
    cx: &mut Context<Panel>,
    placeholder: &str,
    initial: &str,
    cancel: impl Fn(&mut App) -> bool + 'static,
) -> Entity<PromptInput> {
    let field = cx.new(|cx| {
        PromptInput::new(cx, placeholder.to_string(), |_, _, _, _| {})
            .without_command_completion()
            .with_on_overlay_cancel(cancel)
    });
    if !initial.is_empty() {
        field.update(cx, |field, cx| field.set_content(initial.to_string(), cx));
    }
    field.update(cx, |field, cx| field.set_submission_enabled(false, cx));
    field
}

impl Panel {
    /// Open the Extensions overlay on `tab` (`Last` resolves to the tab it
    /// was closed on). Closes every other picker — one overlay at a time.
    pub(crate) fn open_extensions(&mut self, tab: ExtensionTab, cx: &mut Context<Self>) {
        if self.is_side_document() {
            return;
        }
        if crate::harness::screenshot_mode() || cfg!(test) {
            let (paths, fixture) = screenshot_fixture_paths();
            self.open_extensions_with_paths(paths, Some(fixture), tab, cx);
            return;
        }
        let paths = ExtensionPaths::detect(
            self.working_dir
                .clone()
                .filter(|dir| !dir.trim().is_empty())
                .map(PathBuf::from),
        );
        let Some(paths) = paths else {
            self.items.push(Item::Error(
                tr("Extensions need a home directory (`~/.jcode`).", "扩展需要可用的主目录（`~/.jcode`）。").into(),
            ));
            cx.notify();
            return;
        };
        self.open_extensions_with_paths(paths, None, tab, cx);
    }

    /// Open with explicit paths (screenshot fixture and tests). `fixture`
    /// keeps a temp dir alive for the overlay's lifetime.
    pub(crate) fn open_extensions_with_paths(
        &mut self,
        paths: ExtensionPaths,
        fixture: Option<tempfile::TempDir>,
        tab: ExtensionTab,
        cx: &mut Context<Self>,
    ) {
        // One overlay at a time.
        self.login = None;
        self.recovery_picker_open = false;
        self.model_browser = None;
        self.swarm.focus = None;
        self.effort_picker = None;
        if self.model_picker_open {
            self.close_model_picker(cx);
        }
        let tab = LAST_TAB.with(|last| {
            if tab == ExtensionTab::Last {
                last.get()
            } else {
                tab
            }
        });
        let weak = cx.weak_entity();
        let cancel = weak.clone();
        let search = cx.new(|cx| {
            let mut search = PromptInput::new(cx, crate::locale::tr("Filter…", "筛选…"), |_, _, _, _| {})
                .without_command_completion()
                .with_on_overlay_cancel(move |app| {
                    let restored = cancel.update(app, |panel, cx| {
                        panel.extensions_cancel(cx);
                        panel.extensions.is_none().then(|| panel.input_focus_handle(cx))
                    });
                    if let Ok(Some(focus)) = &restored
                        && let Some(window) = app.active_window()
                    {
                        let _ = window.update(app, |_, window, cx| focus.focus(window, cx));
                    }
                    restored.is_ok()
                })
                .with_on_change(move |_, app| {
                    let _ = weak.update(app, |panel, cx| {
                        if let Some(state) = panel.extensions.as_mut() {
                            state.scroll.set_offset(point(px(0.), px(0.)));
                        }
                        cx.notify();
                    });
                });
            search.set_submission_enabled(false, cx);
            search
        });
        let mut state = ExtensionsState {
            tab,
            search,
            scroll: ScrollHandle::new(),
            panel_bounds: Default::default(),
            focus_pending: true,
            paths,
            fixture,
            mcp: mcp::McpList::default(),
            skills: Vec::new(),
            plugins: Vec::new(),
            plugin_errors: Vec::new(),
            plugin_filter: PluginFilter::All,
            tests: HashMap::new(),
            testing: HashSet::new(),
            notice: None,
            confirm: None,
            mcp_form: None,
            skill_form: None,
            plugin_form: None,
            plugin_studio: None,
            skill_import: None,
            plugin_import: None,
            error: None,
        };
        state.reload();
        seed_screenshot_test_result(&mut state);
        if std::env::var("JCODE_DESKTOP_SCREENSHOT_EXTENSIONS").as_deref()
            == Ok("skills-bottom")
        {
            state
                .scroll
                .set_offset(gpui::point(gpui::px(0.), gpui::px(-100_000.)));
        }
        self.extensions = Some(state);
        if std::env::var("JCODE_DESKTOP_SCREENSHOT_EXTENSIONS").as_deref() == Ok("form") {
            self.mcp_form_open(Some("filesystem"), cx);
        }
        cx.notify();
    }

    pub(crate) fn has_open_extensions(&self) -> bool {
        self.extensions.is_some()
    }

    pub(crate) fn close_extensions(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.extensions.take() {
            LAST_TAB.with(|last| last.set(state.tab));
            cx.notify();
        }
    }

    /// `/mcp` → MCP tab, `/skills` (bare) → Skills tab, `/plugins` (bare) →
    /// Plugins tab, `/extensions` → last tab. Commands with args fall through.
    pub(super) fn handle_extensions_command(
        &mut self,
        content: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(content, "/plugin-create" | "/plugin-studio") {
            self.open_ai_plugin_builder(None, cx);
            return true;
        }
        let tab = match content {
            "/mcp" | "/mcp-servers" => ExtensionTab::Mcp,
            "/extensions" => ExtensionTab::Last,
            "/skills" => ExtensionTab::Skills,
            "/plugins" | "/plugin" => ExtensionTab::Plugins,
            _ => return false,
        };
        self.open_extensions(tab, cx);
        true
    }

    /// Apply an edit result: on success refresh the lists and flash a
    /// notice; on failure show the error inline.
    pub(super) fn extensions_apply(
        &mut self,
        result: Result<String, String>,
        done: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.extensions.as_mut() else {
            return;
        };
        match result {
            Ok(detail) => {
                state.error = None;
                state.notice = Some(format!("{done} {detail}"));
                state.reload();
            }
            Err(error) => {
                state.error = Some(error);
                state.notice = None;
            }
        }
        cx.notify();
    }

    /// The cancel callback every form field shares: backs out of a sub-view
    /// (form, confirm) first, closes the overlay from the list.
    pub(super) fn extensions_cancel(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.extensions.as_mut() else {
            return false;
        };
        if state.mcp_form.is_some() {
            state.mcp_form = None;
        } else if state.skill_form.is_some() {
            state.skill_form = None;
        } else if let Some(super::plugin_studio::PluginStudio::CodeEditor {
            new_file: slot @ Some(_),
            ..
        }) = state.plugin_studio.as_mut()
        {
            *slot = None;
        } else if state.plugin_studio.take().is_some() {
            state.focus_pending = true;
        } else if state.plugin_form.take().is_some() {
            state.focus_pending = true;
        } else if state.skill_import.is_some() {
            state.skill_import = None;
        } else if state.plugin_import.is_some() {
            state.plugin_import = None;
        } else if state.confirm.is_some() {
            state.confirm = None;
        } else {
            let tab = state.tab;
            LAST_TAB.with(|last| last.set(tab));
            self.extensions = None;
        }
        cx.notify();
        true
    }

    pub(super) fn render_extensions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if let Some(state) = self.extensions.as_mut()
            && state.focus_pending
        {
            state.focus_pending = false;
            let focus = state.search.read(cx).focus_handle.clone();
            focus.focus(window, cx);
        }
        let state = self.extensions.as_ref()?;
        let theme = Theme::global();
        let query = state.query(cx);
        let narrow = state
            .panel_bounds
            .get()
            .map_or_else(|| window.viewport_size().width, |bounds| bounds.size.width)
            < px(660.);

        let mcp_count = state.mcp.entries.len();
        let skill_count = state.skills.len();
        let plugin_count = state.plugins.len();
        let active = state.tab;
        let tab_pill = |tab: ExtensionTab, slug: &'static str, label: String, cx: &mut Context<Self>| {
            let id = format!("extensions-tab-{slug}");
            pill(id.clone(), id, label, active == tab)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(state) = this.extensions.as_mut() {
                        state.tab = tab;
                        state.confirm = None;
                        state.error = None;
                        state.mcp_form = None;
                        state.skill_form = None;
                        if state.plugin_form.take().is_some() {
                            state.focus_pending = true;
                        }
                        state.skill_import = None;
                        state.plugin_import = None;
                        state.scroll.set_offset(point(px(0.), px(0.)));
                        LAST_TAB.with(|last| last.set(tab));
                    }
                    cx.notify();
                }),
            )
            .into_any_element()
        };

        let search = state.search.clone();
        let scroll = state.scroll.clone();
        let scroll_wheel = scroll.clone();
        let panel_bounds = state.panel_bounds.clone();
        let notice = state.notice.clone();
        let error = state.error.clone();
        let body = match active {
            ExtensionTab::Mcp => self.render_extensions_mcp(&query, cx),
            ExtensionTab::Skills => self.render_extensions_skills(&query, cx),
            ExtensionTab::Plugins => self.render_extensions_plugins(&query, cx),
            ExtensionTab::Last => unreachable!("Last resolves at open"),
        };

        Some(
            div()
                .id("extensions-backdrop")
                .debug_selector(|| "extensions-backdrop".into())
                .absolute()
                .inset_0()
                .size_full()
                .bg(gpui::black().opacity(0.35))
                .occlude()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.close_extensions(cx);
                        this.focus_input(window, cx);
                    }),
                )
                .child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            let resized = panel_bounds
                                .get()
                                .is_none_or(|prev| prev.size.width != bounds.size.width);
                            panel_bounds.set(Some(bounds));
                            if resized {
                                window.request_animation_frame();
                            }
                        },
                    )
                    .absolute()
                    .inset_0()
                    .size_full(),
                )
                .child(
                    div()
                        .size_full()
                        .p_4()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .id("extensions")
                                .debug_selector(|| "extensions".into())
                                .w_full()
                                .h_full()
                                .max_w(px(820.))
                                .max_h(px(600.))
                                .min_w_0()
                                .min_h_0()
                                .overflow_hidden()
                                .rounded_xl()
                                .bg(theme.PANEL_BG)
                                .border_1()
                                .border_color(theme.PANEL_BORDER)
                                .flex()
                                .flex_col()
                                .gap_2()
                                .p_4()
                                .occlude()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                                .capture_action(cx.listener(
                                    |this, _: &crate::input::Clear, window, cx| {
                                        if !this.extensions_cancel(cx) {
                                            return;
                                        }
                                        if this.extensions.is_none() {
                                            this.focus_input(window, cx);
                                        }
                                        cx.stop_propagation();
                                    },
                                ))
                                .child(
                                    div()
                                        .id("extensions-header")
                                        .debug_selector(|| "extensions-header".into())
                                        .flex_none()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .when(narrow, |el| el.flex_wrap())
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_size(px(14.))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(crate::locale::tr("Extensions", "扩展管理")),
                                        )
                                        .child(div().flex_1().min_w_0())
                                        .child(tab_pill(
                                            ExtensionTab::Mcp,
                                            "mcp",
                                            format!("MCP {mcp_count}"),
                                            cx,
                                        ))
                                        .child(tab_pill(
                                            ExtensionTab::Skills,
                                            "skills",
                                            format!("{} {skill_count}", crate::locale::tr("Skills", "技能")),
                                            cx,
                                        ))
                                        .child(tab_pill(
                                            ExtensionTab::Plugins,
                                            "plugins",
                                            format!("{} {plugin_count}", crate::locale::tr("Plugins", "插件")),
                                            cx,
                                        ))
                                        .child(
                                            div()
                                                .when(!narrow, |el| el.flex_none().w(px(180.)))
                                                .when(narrow, |el| el.flex_1().min_w(px(140.)))
                                                .child(search),
                                        )
                                        .child(
                                            pill(
                                                "extensions-close",
                                                "extensions-close".to_string(),
                                                crate::locale::tr("Close", "关闭"),
                                                false,
                                            )
                                            .on_mouse_down(
                                                gpui::MouseButton::Left,
                                                cx.listener(|this, _, window, cx| {
                                                    this.close_extensions(cx);
                                                    this.focus_input(window, cx);
                                                }),
                                            ),
                                        ),
                                )
                                .when_some(notice, |el, notice| {
                                    el.child(
                                        div()
                                            .flex_none()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_size(px(11.))
                                            .text_color(theme.TEXT_DIM)
                                            .child(soft_wrap_text(&notice)),
                                    )
                                })
                                .when_some(error, |el, error| {
                                    el.child(
                                        div()
                                            .flex_none()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_size(px(11.))
                                            .text_color(theme.ERROR)
                                            .child(soft_wrap_text(&error)),
                                    )
                                })
                                .child(
                                    div()
                                        .id("extensions-body")
                                        .debug_selector(|| "extensions-body".into())
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .flex()
                                        .flex_col()
                                        .on_scroll_wheel(cx.listener(
                                            move |_, event: &gpui::ScrollWheelEvent, window, cx| {
                                                if crate::scrollbar::advance_scroll_wheel(
                                                    &scroll_wheel,
                                                    event,
                                                    window,
                                                ) {
                                                    cx.notify();
                                                }
                                                cx.stop_propagation();
                                            },
                                        ))
                                        .child(body)
                                        .child(crate::scrollbar::interactive_vertical(
                                            &scroll,
                                            "extensions-scrollbar",
                                        )),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(10.))
                                        .text_color(theme.TEXT_FAINT)
                                        .child(tr("Changes apply to new sessions.", "更改会在新会话中生效。")),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

// ---------------------------------------------------------- screenshot ----

/// `JCODE_DESKTOP_SCREENSHOT_EXTENSIONS=<mcp|skills|skills-bottom|plugins|form>`:
/// which tab the overlay opens on in screenshot mode; `form` opens the MCP
/// add form, `skills-bottom` seeds a long list and pins it to the bottom.
pub(super) fn screenshot_tab() -> Option<ExtensionTab> {
    if !crate::harness::screenshot_mode() {
        return None;
    }
    match std::env::var("JCODE_DESKTOP_SCREENSHOT_EXTENSIONS").as_deref() {
        Ok("mcp") => Some(ExtensionTab::Mcp),
        Ok("skills") | Ok("skills-bottom") => Some(ExtensionTab::Skills),
        Ok("plugins") => Some(ExtensionTab::Plugins),
        Ok("form") => Some(ExtensionTab::Mcp),
        _ => None,
    }
}

/// Offline fixture paths for `--extensions` screenshots: a temp dir with a
/// few MCP servers (one disabled, one plugin-owned, one Claude) and skills
/// across jcode/agents/project with one disabled.
pub(super) fn screenshot_fixture_paths() -> (ExtensionPaths, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("screenshot fixture dir");
    let base = root.path();
    let jcode_home = base.join("jcode_home");
    let user_home = base.join("home");
    let project = base.join("project");
    if screenshot_tab() == Some(ExtensionTab::Plugins) {
        jcode_plugins::scaffold::create(&jcode_home.join("plugins"), "project-notes")
            .expect("offline plugins fixture");
    }
    let put = |path: PathBuf, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    };
    put(
        jcode_home.join("mcp.json"),
        r#"{
  "mcpServers": {
    "filesystem": {"command": "npx", "args": ["-y", "@mcp/fs", "/tmp"]},
    "notes-search": {"type": "http", "url": "https://mcp.example.com/notes", "headers": {"Authorization": "Bearer sekret"}},
    "guard-mcp": {"command": "node", "args": ["guard.mjs"], "disabled": true}
  }
}"#,
    );
    put(
        jcode_home.join("plugins/.state.json"),
        r#"{"enabled":{"guard":true},"managed":{"skills":["plugin-guard-triage"],"mcp":["guard-mcp"]}}"#,
    );
    put(
        user_home.join(".claude.json"),
        &format!(
            r#"{{"mcpServers":{{"web-reader":{{"type":"http","url":"https://claude.example/mcp"}}}},"projects":{{"{}":{{"mcpServers":{{"local-cache":{{"command":"cache-ctl"}}}}}}}}}}"#,
            project.to_string_lossy().replace('\\', "\\\\")
        ),
    );
    put(
        project.join(".mcp.json"),
        r#"{"mcpServers":{"docs":{"command":"uvx","args":["docs-mcp"],"env":{"DOCS_TOKEN":"abc123"}}}}"#,
    );
    let skill = |dir: PathBuf, name: &str, desc: &str| {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\n\n# {name}\n"),
        )
        .unwrap();
    };
    skill(
        jcode_home.join("skills/commit-style"),
        "commit-style",
        "Write logical commits",
    );
    skill(
        jcode_home.join("skills/pdf-review"),
        "pdf-review",
        "Review PDFs before sending",
    );
    skill(
        jcode_home.join("skills/plugin-guard-triage"),
        "guard-triage",
        "Triage guard findings",
    );
    std::fs::create_dir_all(jcode_home.join("skills/old-draft")).unwrap();
    std::fs::write(
        jcode_home.join("skills/old-draft/SKILL.md.disabled"),
        "---\nname: old-draft\ndescription: Retired draft helper\n---\n",
    )
    .unwrap();
    skill(
        user_home.join(".agents/skills/data-pipeline"),
        "data-pipeline",
        "Shared data helpers",
    );
    skill(
        project.join(".jcode/skills/api-contracts"),
        "api-contracts",
        "Check API contracts",
    );
    if std::env::var("JCODE_DESKTOP_SCREENSHOT_EXTENSIONS").as_deref()
        == Ok("skills-bottom")
    {
        for index in 0..30 {
            skill(
                jcode_home.join(format!("skills/extra-skill-{index:02}")),
                &format!("extra-skill-{index:02}"),
                "Fixture row for the scroll screenshot",
            );
        }
    }
    let paths = ExtensionPaths {
        jcode_home,
        user_home,
        project_dir: Some(project),
    };
    (paths, root)
}

/// In screenshot mode, seed one finished test result so the MCP tab shows
/// the tool chips without spawning a real `jcode` binary.
fn seed_screenshot_test_result(state: &mut ExtensionsState) {
    if !crate::harness::screenshot_mode() {
        return;
    }
    if let Ok(name) = std::env::var("JCODE_DESKTOP_SCREENSHOT_MCP_TEST") {
        state.tests.insert(
            name,
            mcp::McpTestResult {
                ok: true,
                server: Some("filesystem 2025.9".into()),
                tools: vec![
                    "read_file".into(),
                    "write_file".into(),
                    "list_dir".into(),
                    "move_file".into(),
                    "search".into(),
                ],
                error: None,
            },
        );
    }
}

#[cfg(test)]
#[path = "panel_extensions_tests.rs"]
mod tests;
