//! The "Choose model" browser opened by the composer chip and the
//! `OpenModelPicker` action: a large dialog with provider/channel filter
//! pills, search, capability metadata, and favorite stars. Typing `/model`
//! still opens the compact popup next to the composer.

use super::*;
use super::swarm::AgentRole;
use crate::input::model_favorites;
use crate::input::model_menu::{self, BrowserRow, ModelFilter};
use crate::locale::tr;

fn filter_display_label(filter: &model_menu::BrowserFilter) -> &str {
    match &filter.filter {
        ModelFilter::All => tr("All models", "全部模型"),
        ModelFilter::Favorites => tr("Favorites", "收藏"),
        ModelFilter::Recent => tr("Recently used", "最近使用"),
        ModelFilter::Group(_) => filter.label.as_str(),
    }
}

fn header_display_label(header: &str) -> &str {
    match header {
        model_menu::FAVORITES_HEADER => tr("Favorites", "收藏"),
        model_menu::RECENT_HEADER => tr("Recently used", "最近使用"),
        _ => header,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BrowserPurpose {
    MainModel,
    SwitchRoute,
    AgentRole(AgentRole),
}

/// Open state for the model browser dialog.
pub(super) struct ModelBrowserState {
    purpose: BrowserPurpose,
    pub search: Entity<PromptInput>,
    pub filter: ModelFilter,
    pub selection: usize,
    pub scroll: ScrollHandle,
    /// Focus the search field on the first frame after opening.
    pub focus_pending: bool,
    /// Last measured panel bounds (canvas prepaint); decides the narrow
    /// layout so a narrow panel in a wide window collapses its filter
    /// column into a horizontal row.
    pub panel_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// Which management mode the list shows instead of models.
    pub manage: Manage,
}

/// The browser's management modes: plain model list, per-provider/channel
/// group Shown/Hidden toggles, or per-model Shown/Hidden rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Manage {
    #[default]
    Off,
    Providers,
    Models,
}

/// One provider/channel group in Providers manage mode.
struct ManageGroup {
    key: String,
    label: String,
    logo: &'static str,
    count: usize,
    hidden: bool,
}

/// One model route in Models manage mode: every catalog spec (hidden ones
/// included) with its visibility toggle.
struct ManageModel {
    spec: String,
    title: String,
    detail: Option<String>,
    logo: &'static str,
    hidden: bool,
    current: bool,
}


/// A Magpie-style OpenAI-compatible profile for screenshots: channel
/// namespaced ids split one provider into per-channel groups, and
/// `offline_model_metadata` supplies their capability rows.
fn screenshot_magpie_routes(now: u64) -> Vec<jcode_sdk::ModelRouteInfo> {
    [
        ("codex/gpt-6.2-sol", None),
        ("cpr-fast/gpt-6.2-sol", None),
        ("truesota/deepseek-flash", Some(now - 3600)),
        ("workbuddy/deepseek-v4.1-flash", Some(now - 1800)),
        ("group/fast-6.1terra", None),
    ]
    .into_iter()
    .map(|(model, last_used)| jcode_sdk::ModelRouteInfo {
        api_method: "openai-compatible:magpie".into(),
        model: model.to_string(),
        provider: "magpie".into(),
        available: true,
        detail: String::new(),
        usage: last_used.map(|last_used| jcode_sdk::ModelUsage {
            count: 3,
            last_used_unix_secs: Some(last_used),
            tracking_started_unix_secs: Some(now - 86400 * 14),
            selection_count: 1,
            last_selected_unix_secs: Some(last_used),
        }),
    })
    .collect()
}

fn is_subagent_schedulable(meta: Option<&model_menu::ModelMeta>) -> bool {
    meta.and_then(|meta| meta.subagent_schedulable) != Some(false)
}

impl Panel {
    pub(super) fn toggle_model_routes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_browser.is_some() {
            self.close_model_browser(cx);
            self.focus_input(window, cx);
            return;
        }
        self.effort_picker = None;
        self.open_model_browser_with_mode(BrowserPurpose::SwitchRoute, cx);
        self.select_current_browser_group(cx);
    }

    pub(super) fn open_agent_role_browser(&mut self, role: AgentRole, cx: &mut Context<Self>) {
        self.open_model_browser_with_mode(BrowserPurpose::AgentRole(role), cx);
        self.select_current_browser_group(cx);
    }

    fn select_current_browser_group(&mut self, cx: &Context<Self>) {
        let Some(state) = self.model_browser.as_ref() else {
            return;
        };
        let input = self.input.read(cx);
        let details = input.model_details();
        let mut models = self.model_browser_catalog_models(input);
        model_menu::rank(&mut models, details);
        let (_, hidden_groups) = model_favorites::hidden_groups();
        let (_, hidden_models) = model_favorites::hidden_models();
        let hidden = model_menu::Hidden {
            groups: &hidden_groups,
            models: &hidden_models,
        };
        let current = self.browser_current_model();
        let current_spec = current
            .filter(|_| state.purpose == BrowserPurpose::SwitchRoute)
            .and_then(|current| {
                let exact = models.iter().find(|spec| spec.as_str() == current);
                exact.or_else(|| {
                    models.iter().find(|spec| {
                        details.get(*spec).is_some_and(|detail| {
                            detail.model == current
                                && self.provider.as_deref().is_none_or(|provider| {
                                    detail.provider.eq_ignore_ascii_case(provider)
                                })
                                && self.auth_method.as_ref().is_none_or(|method| {
                                    human_auth_method(&detail.api_method) == *method
                                })
                        })
                    })
                })
            });
        let filter = current_spec
            .map(String::as_str)
            .or_else(|| model_menu::current_spec(current, &models, details))
            .map(|spec| model_menu::group_key(spec, details))
            .filter(|key| {
                model_menu::groups_of(&models, details, current, &hidden)
                    .iter()
                    .any(|(visible, _, _)| visible == key)
            })
            .map_or(ModelFilter::All, ModelFilter::Group);
        if let Some(state) = self.model_browser.as_mut() {
            state.filter = filter;
        }
    }

    /// Open the browser, closing the other pickers first.
    pub(super) fn open_model_browser(&mut self, cx: &mut Context<Self>) {
        self.open_model_browser_with_mode(BrowserPurpose::MainModel, cx);
    }

    pub(super) fn has_model_browser_catalog(&self, cx: &App) -> bool {
        let input = self.input.read(cx);
        !self.available_models.is_empty()
            || !input.command_models().is_empty()
            || !input.model_details().is_empty()
    }

    fn open_model_browser_with_mode(&mut self, purpose: BrowserPurpose, cx: &mut Context<Self>) {
        let has_catalog = match purpose {
            BrowserPurpose::MainModel => !self.available_models.is_empty(),
            BrowserPurpose::SwitchRoute | BrowserPurpose::AgentRole(_) => self.has_model_browser_catalog(cx),
        };
        if self.is_side_document() || !has_catalog {
            return;
        }
        self.login = None;
        self.extensions = None;
        self.effort_picker = None;
        self.swarm.focus = None;
        self.recovery_picker_open = false;
        if self.model_picker_open {
            self.close_model_picker(cx);
        } else {
            let input = self.input.clone();
            cx.defer(move |cx| input.update(cx, |input, cx| input.close_effort_menu(cx)));
        }
        let change = cx.weak_entity();
        let cancel = change.clone();
        let search = cx.new(|cx| {
            let mut search = PromptInput::new(cx, tr("Search models…", "搜索模型…"), |_, _, _, _| {})
                .without_command_completion()
                .with_on_overlay_cancel(move |app| {
                    let restored = cancel.update(app, |panel, cx| {
                        panel.dismiss_model_browser(cx)
                    });
                    if let Ok(Some(focus)) = &restored
                        && let Some(window) = app.active_window()
                    {
                        let _ = window.update(app, |_, window, cx| focus.focus(window, cx));
                    }
                    restored.is_ok()
                })
                .with_on_change(move |_, app| {
                    let _ = change.update(app, |panel, cx| {
                        if let Some(state) = panel.model_browser.as_mut() {
                            state.selection = 0;
                            state.scroll.set_offset(point(px(0.), px(0.)));
                        }
                        cx.notify();
                    });
                });
            search.set_submission_enabled(false, cx);
            search
        });
        self.model_browser = Some(ModelBrowserState {
            purpose,
            search,
            filter: ModelFilter::All,
            selection: 0,
            scroll: ScrollHandle::new(),
            focus_pending: true,
            panel_bounds: Default::default(),
            manage: Manage::Off,
        });
        // Offline/reconnected inputs may not have received route metadata
        // yet. Never replace canonical grouped routes once they exist.
        let models = self.available_models.clone();
        let input = self.input.clone();
        input.update(cx, move |input, cx| {
            if input.command_models().is_empty() {
                // Prefer the names that survived preset filtering; an
                // all-dropped catalog must not resurface via this fallback.
                let models = input
                    .filtered_route_models()
                    .map(|models| models.to_vec())
                    .unwrap_or(models);
                input.set_command_models(models, cx);
            }
            input.load_configured_model_meta();
        });
        cx.notify();
    }

    /// Offline screenshot hook (`scripts/screenshot.py --model-menu |
    /// --model-browser`): pretend RuntimeInfo delivered a catalog with a
    /// Magpie-style OpenAI-compatible profile, then open whichever picker
    /// `JCODE_DESKTOP_SCREENSHOT_MODEL_{MENU,BROWSER}` selects.
    pub(super) fn apply_screenshot_model_entry(&mut self, cx: &mut Context<Self>) {
        if std::env::var("JCODE_DESKTOP_SCREENSHOT").as_deref() != Ok("1")
            || std::env::var("JCODE_DESKTOP_SCREENSHOT_MODELS").as_deref() != Ok("1")
        {
            return;
        }
        let names = [
            "anthropic:sonnet-review".to_string(),
            "google:gemini-review".to_string(),
        ]
        .into_iter()
        .chain((1..=12).map(|index| format!("openai:atlas-{index:02}")));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut routes: Vec<jcode_sdk::ModelRouteInfo> = names
            .map(|model| {
                let provider = model.split(':').next().unwrap().to_string();
                let usage = match provider.as_str() {
                    "anthropic" => Some(jcode_sdk::ModelUsage {
                        count: 42,
                        last_used_unix_secs: Some(now - 7200),
                        tracking_started_unix_secs: Some(now - 86400 * 14),
                        selection_count: 5,
                        last_selected_unix_secs: Some(now - 7200),
                    }),
                    "google" => Some(jcode_sdk::ModelUsage {
                        count: 7,
                        last_used_unix_secs: Some(now - 86400 * 3),
                        tracking_started_unix_secs: Some(now - 86400 * 14),
                        selection_count: 2,
                        last_selected_unix_secs: Some(now - 86400 * 3),
                    }),
                    _ => None,
                };
                jcode_sdk::ModelRouteInfo {
                    api_method: format!("{provider}-api-key"),
                    model,
                    provider,
                    available: true,
                    detail: String::new(),
                    usage,
                }
            })
            .collect();
        routes.extend(screenshot_magpie_routes(now));
        self.apply(
            &ApiEvent::RuntimeInfo {
                session_id: self.session_id.clone(),
                provider: Some("openai".into()),
                model: Some("openai:atlas-01".into()),
                reasoning_effort: None,
                auth_method: None,
                routes,
            },
            cx,
        );
        // Screenshot entry points: the compact /model popup or the full
        // model browser.
        let input = self.input.clone();
        if std::env::var("JCODE_DESKTOP_SCREENSHOT_MODEL_BROWSER").as_deref() == Ok("1") {
            cx.defer(move |cx| {
                input.update(cx, |input, _| input.load_configured_model_meta());
            });
            self.open_model_browser(cx);
            if let Ok(query) = std::env::var("JCODE_DESKTOP_SCREENSHOT_MODEL_QUERY")
                && let Some(state) = self.model_browser.as_ref()
            {
                let search = state.search.clone();
                cx.defer(move |cx| {
                    search.update(cx, |search, cx| search.set_content(query, cx));
                });
            }
            if std::env::var("JCODE_DESKTOP_SCREENSHOT_MODEL_MANAGE").as_deref() == Ok("1")
                && let Some(state) = self.model_browser.as_mut()
            {
                state.manage = if std::env::var(
                    "JCODE_DESKTOP_SCREENSHOT_MODEL_MANAGE_MODELS",
                )
                .as_deref()
                    == Ok("1")
                {
                    Manage::Models
                } else {
                    Manage::Providers
                };
            }
        } else if std::env::var("JCODE_DESKTOP_SCREENSHOT_MODEL_MENU").as_deref() == Ok("1") {
            let query = std::env::var("JCODE_DESKTOP_SCREENSHOT_MODEL_QUERY")
                .map(|query| format!("/model {query}"))
                .unwrap_or_else(|_| "/model ".to_string());
            cx.defer(move |cx| {
                input.update(cx, |input, cx| {
                    input.load_configured_model_meta();
                    input.set_content(query, cx);
                });
            });
        }
    }

    pub(super) fn close_model_browser(&mut self, cx: &mut Context<Self>) {
        if self.model_browser.take().is_some() {
            cx.notify();
        }
    }

    fn dismiss_model_browser(&mut self, cx: &mut Context<Self>) -> Option<gpui::FocusHandle> {
        let state = self.model_browser.take()?;
        let focus = if matches!(state.purpose, BrowserPurpose::AgentRole(_)) {
            let focus = cx.focus_handle();
            self.swarm.focus = Some(focus.clone());
            focus
        } else {
            self.input_focus_handle(cx)
        };
        cx.notify();
        Some(focus)
    }

    fn browser_current_model(&self) -> Option<&str> {
        match self.model_browser.as_ref().map(|state| state.purpose) {
            Some(BrowserPurpose::AgentRole(role)) => self.swarm.active_role_model(role),
            _ => self.model.as_deref(),
        }
    }

    fn model_browser_catalog_models(&self, input: &PromptInput) -> Vec<String> {
        let details = input.model_details();
        let mut models: Vec<String> = if details.is_empty() {
            input.command_models().to_vec()
        } else {
            details.keys().cloned().collect()
        };
        if self.model_browser.as_ref()
            .is_some_and(|state| matches!(state.purpose, BrowserPurpose::AgentRole(_)))
        {
            let current = self.browser_current_model();
            models.retain(|spec| {
                current == Some(spec.as_str()) || is_subagent_schedulable(details.metas.get(spec))
            });
        }
        models
    }

    /// Rows for the active filter and search query.
    fn model_browser_rows(&self, cx: &App) -> Vec<BrowserRow> {
        let Some(state) = &self.model_browser else {
            return Vec::new();
        };
        let input = self.input.read(cx);
        let details = input.model_details();
        let models = self.model_browser_catalog_models(input);
        let (_, favorites) = model_favorites::favorites();
        let (_, hidden_groups) = model_favorites::hidden_groups();
        let (_, hidden_models) = model_favorites::hidden_models();
        let query = state.search.read(cx).content.to_string();
        model_menu::browser_rows(
            &state.filter,
            &models,
            details,
            &favorites,
            self.browser_current_model(),
            &query,
            model_menu::now_unix_secs(),
            &model_menu::Hidden {
                groups: &hidden_groups,
                models: &hidden_models,
            },
        )
    }

    /// Provider/channel groups for Providers manage mode: every group the
    /// catalog supplied (hidden groups included — unconfigured presets were
    /// already dropped by `set_model_routes`), filtered by the search query
    /// on the group label.
    fn model_browser_manage_rows(&self, cx: &App) -> Vec<ManageGroup> {
        let input = self.input.read(cx);
        let details = input.model_details();
        let models = self.model_browser_catalog_models(input);
        let (_, hidden) = model_favorites::hidden_groups();
        let query = self
            .model_browser
            .as_ref()
            .map(|state| state.search.read(cx).content.to_lowercase())
            .unwrap_or_default();
        model_menu::groups_of(&models, details, self.browser_current_model(), &model_menu::Hidden::NONE)
            .into_iter()
            .filter(|(_, label, _)| {
                query.is_empty() || label.to_lowercase().contains(&query)
            })
            .map(|(key, label, members)| ManageGroup {
                logo: members
                    .first()
                    .map(|model| model_menu::header_parts(model, details).logo)
                    .unwrap_or(""),
                count: members.len(),
                hidden: model_favorites::is_hidden(&hidden, &key),
                key,
                label,
            })
            .collect()
    }

    /// Toggle a group hidden; if the browser's active filter pointed at it,
    /// fall back to All models.
    fn toggle_browser_group_hidden(&mut self, key: &str, cx: &mut Context<Self>) {
        let now_hidden = model_favorites::toggle_hidden(key);
        if let Some(state) = self.model_browser.as_mut() {
            if now_hidden && matches!(&state.filter, ModelFilter::Group(k) if k == key) {
                state.filter = ModelFilter::All;
            }
            state.selection = 0;
            state.scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }

    /// Toggle an exact model spec hidden. Hiding is display-only: the active
    /// session model keeps running until the user explicitly picks another
    /// one; no automatic fallback ever fires.
    fn toggle_browser_model_hidden(&mut self, spec: &str, cx: &mut Context<Self>) {
        model_favorites::toggle_hidden_model(spec);
        if let Some(state) = self.model_browser.as_mut() {
            state.selection = 0;
            state.scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }

    /// Models manage mode rows: every catalog spec — hidden ones included,
    /// so hiding is reversible — with the current model first, filtered by
    /// the search query on title or spec.
    fn model_browser_manage_model_rows(&self, cx: &App) -> Vec<ManageModel> {
        let input = self.input.read(cx);
        let details = input.model_details();
        let models = self.model_browser_catalog_models(input);
        let (_, hidden_models) = model_favorites::hidden_models();
        let query = self
            .model_browser
            .as_ref()
            .map(|state| state.search.read(cx).content.to_lowercase())
            .unwrap_or_default();
        let metas = &details.metas;
        let now = model_menu::now_unix_secs();
        let current = self.browser_current_model();
        let mut ranked = model_menu::ranked_specs(&models, details);
        ranked.sort_by_key(|model| current != Some(model.as_str()));
        ranked
            .into_iter()
            .filter(|spec| {
                query.is_empty()
                    || spec.to_lowercase().contains(&query)
                    || model_menu::pretty_title(spec, details)
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|spec| ManageModel {
                title: model_menu::pretty_title(&spec, details),
                detail: model_menu::detail_label(metas.get(&spec), None, now),
                logo: model_menu::model_logo(&spec, details),
                hidden: model_favorites::is_model_hidden(&hidden_models, &spec),
                current: current == Some(spec.as_str()),
                spec,
            })
            .collect()
    }

    /// Apply a subagent override or select the main model via `/model <spec>`.
    fn choose_browser_model(&mut self, spec: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(BrowserPurpose::AgentRole(role)) = self.model_browser.as_ref().map(|state| state.purpose) {
            let meta = self.input.read(cx).model_details().metas.get(&spec);
            if is_subagent_schedulable(meta) {
                self.select_agent_role_model(role, Some(spec), window, cx);
            }
            return;
        }
        self.model_browser = None;
        let content = format!("/model {spec}");
        if self.preview_state.is_some() {
            self.handle_preview_command(&content, cx);
        } else if !self.handle_slash_command(&content, cx) {
            self.submit_or_queue(content, Vec::new(), false, cx);
        }
        self.focus_input(window, cx);
        cx.notify();
    }

    pub(super) fn render_model_browser(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if let Some(state) = self.model_browser.as_mut()
            && state.focus_pending
        {
            state.focus_pending = false;
            let focus = state.search.read(cx).focus_handle.clone();
            focus.focus(window, cx);
        }
        let state = self.model_browser.as_ref()?;
        let theme = Theme::global();
        let rows = self.model_browser_rows(cx);
        let filters = {
            let input = self.input.read(cx);
            let details = input.model_details();
            let models = self.model_browser_catalog_models(input);
            let (_, favorites) = model_favorites::favorites();
            let (_, hidden_groups) = model_favorites::hidden_groups();
            let (_, hidden_models) = model_favorites::hidden_models();
            model_menu::browser_filters(
                &models,
                details,
                &favorites,
                self.browser_current_model(),
                &model_menu::Hidden {
                    groups: &hidden_groups,
                    models: &hidden_models,
                },
            )
        };
        let manage = state.manage;
        let manage_groups = if manage == Manage::Providers {
            self.model_browser_manage_rows(cx)
        } else {
            Vec::new()
        };
        let manage_models = if manage == Manage::Models {
            self.model_browser_manage_model_rows(cx)
        } else {
            Vec::new()
        };
        let row_count = match manage {
            Manage::Off => rows.len(),
            Manage::Providers => manage_groups.len(),
            Manage::Models => manage_models.len(),
        };
        let selection = state.selection.min(row_count.saturating_sub(1));
        let search = state.search.clone();
        let scroll = state.scroll.clone();
        let active_filter = state.filter.clone();
        let favorites_empty = rows.is_empty() && active_filter == ModelFilter::Favorites;
        let narrow = state
            .panel_bounds
            .get()
            .map_or_else(|| window.viewport_size().width, |bounds| bounds.size.width)
            < px(592.);

        let logo_svg = |provider: &'static str, fallback: &str, size: f32, color: gpui::Rgba| {
            match crate::accounts::logo(provider) {
                Some(bytes) => gpui::svg()
                    .data(bytes)
                    .size(px(size))
                    .flex_none()
                    .text_color(color)
                    .into_any_element(),
                None => div()
                    .size(px(size))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(size * 0.6))
                    .text_color(color)
                    .child(crate::accounts::lettermark(fallback))
                    .into_any_element(),
            }
        };

        let pill = |index: usize, filter: model_menu::BrowserFilter, active: bool| {
            let logo = filter.logo;
            let label = filter.label.clone();
            let kind = filter.filter.clone();
            let display_label = filter_display_label(&filter);
            let text = format!("{display_label} {}", filter.count);
            div()
                .id(("model-browser-filter", index))
                .debug_selector(move || format!("model-browser-filter-{label}").into())
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .h(px(30.))
                .rounded_full()
                .text_size(px(12.))
                .cursor_pointer()
                .bg(if active {
                    theme.ACCENT
                } else {
                    theme.HEADER_BG
                })
                .text_color(if active { theme.BG } else { theme.TEXT_DIM })
                .when(!active, |el| {
                    el.hover(|style| style.bg(theme.INPUT_BG))
                })
                .when(!logo.is_empty(), |el| {
                    el.child(logo_svg(
                        logo,
                        &filter.label,
                        14.,
                        if active { theme.BG } else { theme.TEXT_DIM },
                    ))
                })
                .child(div().min_w_0().truncate().child(text))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(state) = this.model_browser.as_mut() {
                            state.filter = kind.clone();
                            state.selection = 0;
                            state.scroll.set_offset(point(px(0.), px(0.)));
                        }
                        cx.notify();
                    }),
                )
                .into_any_element()
        };

        let filter_pills: Vec<_> = filters
            .into_iter()
            .enumerate()
            .map(|(index, filter)| pill(index, filter.clone(), filter.filter == active_filter))
            .collect();

        let list_rows: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let spec = row.spec.clone();
                let star_spec = row.spec.clone();
                let selected = index == selection;
                let ink = if selected { theme.BG } else { theme.TEXT };
                let mut item = div()
                    .w_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_1();
                if let Some(header) = &row.header {
                    item = item.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .pt_3()
                            .pb_1()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.TEXT_FAINT)
                            .when(!row.header_logo.is_empty(), |el| {
                                el.child(logo_svg(row.header_logo, header, 13., theme.TEXT_FAINT))
                            })
                            .child(header_display_label(header.as_str()).to_owned()),
                    );
                }
                item.child(
                    div()
                        .id(("model-browser-row", index))
                        .debug_selector(move || format!("model-browser-row-{index}").into())
                        .group("model-browser-row")
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded_full()
                        .bg(if selected { theme.TEXT } else { theme.HEADER_BG })
                        .when(!selected, |el| {
                            el.hover(|style| style.bg(theme.INPUT_BG))
                        })
                        .cursor_pointer()
                        .child(logo_svg(row.logo, &row.title, 18., ink))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(13.))
                                        .text_color(ink)
                                        .when(selected, |el| {
                                            el.font_weight(FontWeight::SEMIBOLD)
                                        })
                                        .child(row.title.clone()),
                                )
                                .children(row.detail.clone().map(|detail| {
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(if selected {
                                            ink
                                        } else {
                                            theme.TEXT_FAINT
                                        })
                                        .child(detail)
                                })),
                        )
                        .when(row.current, |el| {
                            el.child(
                                div()
                                    .flex_none()
                                    .text_size(px(11.))
                                    .text_color(if selected {
                                        ink
                                    } else {
                                        theme.TEXT_FAINT
                                    })
                                    .child(tr("Current", "当前")),
                            )
                        })
                        .child({
                            let favorited = row.favorite;
                            let star = div()
                                .debug_selector(move || {
                                    format!("model-browser-star-{index}").into()
                                })
                                .w(px(22.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .text_color(if favorited {
                                    theme.ACCENT
                                } else if selected {
                                    ink
                                } else {
                                    gpui::transparent_black().into()
                                });
                            let star = if favorited || selected {
                                star
                            } else {
                                star.group_hover("model-browser-row", |style| {
                                    style.text_color(theme.TEXT_FAINT)
                                })
                            };
                            star.child(
                                gpui::svg()
                                    .data(if favorited {
                                        model_menu::STAR_FILLED_ICON
                                    } else {
                                        model_menu::STAR_ICON
                                    })
                                    .size(px(14.))
                                    .flex_none(),
                            )
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |_, _, _, cx| {
                                    cx.stop_propagation();
                                    model_favorites::toggle(&star_spec);
                                    cx.notify();
                                }),
                            )
                        })
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.choose_browser_model(spec.clone(), window, cx);
                            }),
                        ),
                )
            })
            .collect();

        let manage_items: Vec<_> = manage_groups
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let row_key = group.key.clone();
                let hide_key = group.key.clone();
                let toggle_key = group.key.clone();
                let selected = index == selection;
                let ink = if selected { theme.BG } else { theme.TEXT };
                div()
                    .id(("model-browser-manage", index))
                    .debug_selector(move || {
                        format!("model-browser-manage-{row_key}").into()
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_full()
                    .bg(if selected { theme.TEXT } else { theme.HEADER_BG })
                    .when(!selected, |el| {
                        el.hover(|style| style.bg(theme.INPUT_BG))
                    })
                    .child(logo_svg(group.logo, &group.label, 18., ink))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(ink)
                            .child(group.label.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(if selected {
                                ink
                            } else {
                                theme.TEXT_FAINT
                            })
                            .child(group.count.to_string()),
                    )
                    .child({
                        let group_hidden = group.hidden;
                        div()
                            .debug_selector(move || {
                                format!("model-browser-hide-{hide_key}").into()
                            })
                            .flex_none()
                            .px_3()
                            .h(px(24.))
                            .rounded_full()
                            .text_size(px(11.))
                            .cursor_pointer()
                            .bg(if group_hidden {
                                theme.HEADER_BG
                            } else {
                                theme.ACCENT
                            })
                            .text_color(if group_hidden {
                                theme.TEXT_DIM
                            } else {
                                theme.BG
                            })
                            .when(group_hidden, |el| {
                                el.hover(|style| style.bg(theme.INPUT_BG))
                            })
                            .child(if group_hidden { tr("Hidden", "已隐藏") } else { tr("Shown", "已显示") })
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_browser_group_hidden(&toggle_key, cx);
                                }),
                            )
                    })
                    .into_any_element()
            })
            .collect();

        let manage_model_items: Vec<_> = manage_models
            .iter()
            .enumerate()
            .map(|(index, model)| {
                let row_key = model.spec.clone();
                let hide_key = model.spec.clone();
                let toggle_spec = model.spec.clone();
                let selected = index == selection;
                let ink = if selected { theme.BG } else { theme.TEXT };
                div()
                    .id(("model-browser-manage-model", index))
                    .debug_selector(move || {
                        format!("model-browser-manage-model-{row_key}").into()
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_full()
                    .bg(if selected { theme.TEXT } else { theme.HEADER_BG })
                    .when(!selected, |el| {
                        el.hover(|style| style.bg(theme.INPUT_BG))
                    })
                    .child(logo_svg(model.logo, &model.title, 18., ink))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(13.))
                                            .text_color(if model.hidden && !selected {
                                                theme.TEXT_FAINT
                                            } else {
                                                ink
                                            })
                                            .child(model.title.clone()),
                                    )
                                    .when(model.current, |el| {
                                        el.child(
                                            div()
                                                .flex_none()
                                                .text_size(px(11.))
                                                .text_color(theme.TEXT_FAINT)
                                                .child(tr("Current", "当前")),
                                        )
                                    }),
                            )
                            .children(model.detail.clone().map(|detail| {
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(if selected {
                                        ink
                                    } else {
                                        theme.TEXT_FAINT
                                    })
                                    .child(detail)
                            })),
                    )
                    .child({
                        let model_hidden = model.hidden;
                        div()
                            .debug_selector(move || {
                                format!("model-browser-model-hide-{hide_key}").into()
                            })
                            .flex_none()
                            .px_3()
                            .h(px(24.))
                            .rounded_full()
                            .text_size(px(11.))
                            .cursor_pointer()
                            .bg(if model_hidden {
                                theme.HEADER_BG
                            } else {
                                theme.ACCENT
                            })
                            .text_color(if model_hidden {
                                theme.TEXT_DIM
                            } else {
                                theme.BG
                            })
                            .when(model_hidden, |el| {
                                el.hover(|style| style.bg(theme.INPUT_BG))
                            })
                            .child(if model_hidden { tr("Hidden", "已隐藏") } else { tr("Shown", "已显示") })
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_browser_model_hidden(&toggle_spec, cx);
                                }),
                            )
                    })
                    .into_any_element()
            })
            .collect();

        let base_list = div()
            .id("model-browser-list")
            .track_scroll(&scroll)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        let body_list = if manage == Manage::Providers {
            let any_hidden = manage_groups.iter().any(|group| group.hidden)
                || !model_favorites::hidden_groups().1.is_empty();
            base_list
                .debug_selector(|| "model-browser-manage-list".into())
                .when(any_hidden, |el| {
                    el.child(
                        div()
                            .id("model-browser-show-all")
                            .debug_selector(|| "model-browser-show-all".into())
                            .flex_none()
                            .px_3()
                            .h(px(28.))
                            .rounded_full()
                            .bg(theme.HEADER_BG)
                            .text_size(px(12.))
                            .text_color(theme.TEXT_DIM)
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.INPUT_BG))
                            .child(tr("Show all", "全部显示"))
                            .on_click(cx.listener(|_, _, _, cx| {
                                model_favorites::show_all();
                                cx.notify();
                            })),
                    )
                })
                .children(manage_items)
                .when(manage_groups.is_empty(), |el| {
                    el.child(
                        div()
                            .debug_selector(|| "model-browser-empty".into())
                            .px_3()
                            .py_6()
                            .text_size(px(12.))
                            .text_color(theme.TEXT_DIM)
                            .child(tr("No models match", "没有匹配的模型")),
                    )
                })
        } else if manage == Manage::Models {
            let any_hidden = !model_favorites::hidden_models().1.is_empty();
            base_list
                .debug_selector(|| "model-browser-manage-list".into())
                .when(any_hidden, |el| {
                    el.child(
                        div()
                            .id("model-browser-show-all")
                            .debug_selector(|| "model-browser-show-all".into())
                            .flex_none()
                            .px_3()
                            .h(px(28.))
                            .rounded_full()
                            .bg(theme.HEADER_BG)
                            .text_size(px(12.))
                            .text_color(theme.TEXT_DIM)
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.INPUT_BG))
                            .child(tr("Show all", "全部显示"))
                            .on_click(cx.listener(|_, _, _, cx| {
                                model_favorites::show_all_models();
                                cx.notify();
                            })),
                    )
                })
                .children(manage_model_items)
                .when(manage_models.is_empty(), |el| {
                    el.child(
                        div()
                            .debug_selector(|| "model-browser-empty".into())
                            .px_3()
                            .py_6()
                            .text_size(px(12.))
                            .text_color(theme.TEXT_DIM)
                            .child(tr("No models match", "没有匹配的模型")),
                    )
                })
        } else {
            base_list
                .children(list_rows)
                .when(rows.is_empty(), |el| {
                    el.child(
                        div()
                            .debug_selector(|| "model-browser-empty".into())
                            .px_3()
                            .py_6()
                            .text_size(px(12.))
                            .text_color(theme.TEXT_DIM)
                            .child(if favorites_empty {
                                tr("No favorites yet — star a model to pin it here", "暂无收藏。点击模型旁的星标即可收藏。")
                            } else {
                                tr("No models match", "没有匹配的模型")
                            }),
                    )
                })
        };

        let body = if narrow {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .id("model-browser-filters")
                        .debug_selector(|| "model-browser-filters".into())
                        .flex_none()
                        .flex()
                        .gap_2()
                        .overflow_x_scroll()
                        .children(filter_pills),
                )
                .child(body_list)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .gap_3()
                .child(
                    div()
                        .id("model-browser-filter-col")
                        .debug_selector(|| "model-browser-filter-col".into())
                        .w(px(200.))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .overflow_y_scroll()
                        .children(filter_pills),
                )
                .child(body_list)
                .into_any_element()
        };

        Some(
            div()
                .id("model-browser-backdrop")
                .debug_selector(|| "model-browser-backdrop".into())
                .absolute()
                .inset_0()
                .size_full()
                .bg(gpui::black().opacity(0.35))
                .occlude()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        if let Some(focus) = this.dismiss_model_browser(cx) {
                            focus.focus(window, cx);
                        }
                    }),
                )
                .child({
                    let panel_bounds = state.panel_bounds.clone();
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
                    .size_full()
                })
                .child(
                    div()
                        .size_full()
                        .p_4()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .id("model-browser")
                                .debug_selector(|| "model-browser".into())
                                .w_full()
                                .h_full()
                                .max_w(px(760.))
                                .max_h(px(560.))
                                .rounded_xl()
                                .bg(theme.PANEL_BG)
                                .border_1()
                                .border_color(theme.PANEL_BORDER)
                                .flex()
                                .flex_col()
                                .gap_3()
                                .p_4()
                                .occlude()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .capture_action(cx.listener(
                                    |this, _: &crate::input::Clear, window, cx| {
                                        if let Some(focus) = this.dismiss_model_browser(cx) {
                                            focus.focus(window, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                                .capture_action(cx.listener(
                                    |this, _: &crate::input::Submit, window, cx| {
                                        if this.model_browser.is_none() {
                                            return;
                                        }
                                        let index = this
                                            .model_browser
                                            .as_ref()
                                            .map(|state| state.selection)
                                            .unwrap_or(0);
                                        match this
                                            .model_browser
                                            .as_ref()
                                            .map(|state| state.manage)
                                            .unwrap_or(Manage::Off)
                                        {
                                            Manage::Providers => {
                                                let groups =
                                                    this.model_browser_manage_rows(cx);
                                                if let Some(group) = groups.get(index) {
                                                    let key = group.key.clone();
                                                    this.toggle_browser_group_hidden(
                                                        &key, cx,
                                                    );
                                                }
                                            }
                                            Manage::Models => {
                                                let models =
                                                    this.model_browser_manage_model_rows(cx);
                                                if let Some(model) = models.get(index) {
                                                    let spec = model.spec.clone();
                                                    this.toggle_browser_model_hidden(
                                                        &spec, cx,
                                                    );
                                                }
                                            }
                                            Manage::Off => {
                                                let rows = this.model_browser_rows(cx);
                                                if let Some(row) = rows.get(index) {
                                                    let spec = row.spec.clone();
                                                    this.choose_browser_model(
                                                        spec, window, cx,
                                                    );
                                                }
                                            }
                                        }
                                        cx.stop_propagation();
                                    },
                                ))
                                .capture_action(cx.listener(
                                    |this, _: &crate::input::HistoryPrev, _, cx| {
                                        if this.model_browser.is_none() {
                                            return;
                                        }
                                        if let Some(state) = this.model_browser.as_mut() {
                                            state.selection =
                                                state.selection.saturating_sub(1);
                                            let index = state.selection;
                                            state.scroll.scroll_to_item(index);
                                        }
                                        cx.stop_propagation();
                                        cx.notify();
                                    },
                                ))
                                .capture_action(cx.listener(
                                    |this, _: &crate::input::HistoryNext, _, cx| {
                                        if this.model_browser.is_none() {
                                            return;
                                        }
                                        let max = match this
                                            .model_browser
                                            .as_ref()
                                            .map(|state| state.manage)
                                        {
                                            Some(Manage::Providers) => {
                                                this.model_browser_manage_rows(cx).len()
                                            }
                                            Some(Manage::Models) => this
                                                .model_browser_manage_model_rows(cx)
                                                .len(),
                                            _ => this.model_browser_rows(cx).len(),
                                        }
                                        .saturating_sub(1);
                                        if let Some(state) = this.model_browser.as_mut() {
                                            state.selection =
                                                (state.selection + 1).min(max);
                                            let index = state.selection;
                                            state.scroll.scroll_to_item(index);
                                        }
                                        cx.stop_propagation();
                                        cx.notify();
                                    },
                                ))
                                .child(
                                    div()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .when(narrow, |bar| bar.flex_wrap().gap_2())
                                        .child(
                                            div()
                                                .flex_none()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(px(14.))
                                                .text_color(theme.TEXT)
                                                .child(match state.purpose {
                                                    BrowserPurpose::MainModel => tr("Choose model", "选择模型"),
                                                    BrowserPurpose::SwitchRoute => tr("Switch route", "切换线路"),
                                                    BrowserPurpose::AgentRole(role) => role.browser_title(),
                                                }),
                                        )
                                        .child(
                                            div()
                                                .debug_selector(|| "model-browser-search".into())
                                                .flex_1()
                                                .min_w_0()
                                                .when(narrow, |search| {
                                                    search.flex_basis(px(180.)).min_w(px(180.))
                                                })
                                                .rounded_full()
                                                .bg(theme.INPUT_BG)
                                                .border_1()
                                                .border_color(theme.INPUT_BORDER)
                                                .px_2()
                                                .child(search),
                                        )
                                        .when_some(match state.purpose {
                                            BrowserPurpose::AgentRole(role) => Some(role),
                                            _ => None,
                                        }, |el, role| {
                                            el.child(
                                                div()
                                                    .id("model-browser-subagent-inherit")
                                                    .debug_selector(|| "model-browser-subagent-inherit".into())
                                                    .flex_none()
                                                    .px_3()
                                                    .h(px(28.))
                                                    .rounded_full()
                                                    .bg(theme.HEADER_BG)
                                                    .text_size(px(12.))
                                                    .text_color(theme.TEXT_DIM)
                                                    .cursor_pointer()
                                                    .hover(|el| el.bg(theme.INPUT_BG))
                                                    .child(tr("Inherit main model", "继承主模型"))
                                                    .on_click(cx.listener(move |this, _, window, cx| {
                                                        this.select_agent_role_model(role, None, window, cx);
                                                    })),
                                            )
                                        })
                                        .child(
                                            div()
                                                .id("model-browser-providers")
                                                .debug_selector(|| {
                                                    "model-browser-providers".into()
                                                })
                                                .flex_none()
                                                .px_3()
                                                .h(px(28.))
                                                .rounded_full()
                                                .text_size(px(12.))
                                                .cursor_pointer()
                                                .bg(if manage == Manage::Providers {
                                                    theme.ACCENT
                                                } else {
                                                    theme.HEADER_BG
                                                })
                                                .text_color(if manage == Manage::Providers {
                                                    theme.BG
                                                } else {
                                                    theme.TEXT_DIM
                                                })
                                                .when(manage != Manage::Providers, |el| {
                                                    el.hover(|el| el.bg(theme.INPUT_BG))
                                                })
                                                .child(if manage == Manage::Providers {
                                                    tr("Done", "完成")
                                                } else {
                                                    tr("Providers", "供应商")
                                                })
                                                .on_click(cx.listener(
                                                    |this, _, _, cx| {
                                                        if let Some(state) =
                                                            this.model_browser.as_mut()
                                                        {
                                                            state.manage = if state.manage
                                                                == Manage::Providers
                                                            {
                                                                Manage::Off
                                                            } else {
                                                                Manage::Providers
                                                            };
                                                            state.selection = 0;
                                                            state.scroll.set_offset(
                                                                point(px(0.), px(0.)),
                                                            );
                                                        }
                                                        cx.notify();
                                                    },
                                                )),
                                        )
                                        .child(
                                            div()
                                                .id("model-browser-models")
                                                .debug_selector(|| {
                                                    "model-browser-models".into()
                                                })
                                                .flex_none()
                                                .px_3()
                                                .h(px(28.))
                                                .rounded_full()
                                                .text_size(px(12.))
                                                .cursor_pointer()
                                                .bg(if manage == Manage::Models {
                                                    theme.ACCENT
                                                } else {
                                                    theme.HEADER_BG
                                                })
                                                .text_color(if manage == Manage::Models {
                                                    theme.BG
                                                } else {
                                                    theme.TEXT_DIM
                                                })
                                                .when(manage != Manage::Models, |el| {
                                                    el.hover(|el| el.bg(theme.INPUT_BG))
                                                })
                                                .child(if manage == Manage::Models {
                                                    tr("Done", "完成")
                                                } else {
                                                    tr("Models", "模型")
                                                })
                                                .on_click(cx.listener(
                                                    |this, _, _, cx| {
                                                        if let Some(state) =
                                                            this.model_browser.as_mut()
                                                        {
                                                            state.manage = if state.manage
                                                                == Manage::Models
                                                            {
                                                                Manage::Off
                                                            } else {
                                                                Manage::Models
                                                            };
                                                            state.selection = 0;
                                                            state.scroll.set_offset(
                                                                point(px(0.), px(0.)),
                                                            );
                                                        }
                                                        cx.notify();
                                                    },
                                                )),
                                        )
                                        .child(
                                            div()
                                                .id("model-browser-close")
                                                .debug_selector(|| {
                                                    "model-browser-close".into()
                                                })
                                                .flex_none()
                                                .px_3()
                                                .h(px(28.))
                                                .rounded_full()
                                                .bg(theme.HEADER_BG)
                                                .text_size(px(12.))
                                                .text_color(theme.TEXT_DIM)
                                                .cursor_pointer()
                                                .hover(|el| el.bg(theme.INPUT_BG))
                                                .child(tr("Close", "关闭"))
                                                .on_click(cx.listener(
                                                    |this, _, window, cx| {
                                                        if let Some(focus) = this.dismiss_model_browser(cx) {
                                                            focus.focus(window, cx);
                                                        }
                                                    },
                                                )),
                                        ),
                                )
                                .child(body),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_browser_display_labels_are_localized() {
        for (kind, english, chinese) in [
            (ModelFilter::All, "All models", "全部模型"),
            (ModelFilter::Favorites, model_menu::FAVORITES_HEADER, "收藏"),
            (ModelFilter::Recent, model_menu::RECENT_HEADER, "最近使用"),
            (ModelFilter::Group("magpie".into()), "Magpie", "Magpie"),
        ] {
            let filter = model_menu::BrowserFilter {
                filter: kind,
                label: english.into(),
                count: 1,
                logo: "",
            };
            assert_eq!(filter_display_label(&filter), english);
            assert_eq!(header_display_label(english), english);
            crate::locale::with_zh(|| {
                assert_eq!(filter_display_label(&filter), chinese);
                if filter.filter != ModelFilter::All {
                    assert_eq!(header_display_label(english), chinese);
                }
                assert_eq!(filter.label, english);
            });
        }
    }

    fn route(model: &str, provider: &str, api_method: &str) -> jcode_sdk::ModelRouteInfo {
        jcode_sdk::ModelRouteInfo {
            model: model.into(),
            provider: provider.into(),
            api_method: api_method.into(),
            available: true,
            detail: String::new(),
            usage: None,
        }
    }

    fn fixture_routes() -> Vec<jcode_sdk::ModelRouteInfo> {
        vec![
            route("codex/gpt-6.2-sol", "magpie", "openai-compatible:magpie"),
            route("cpr-fast/gpt-6.2-sol", "magpie", "openai-compatible:magpie"),
            route("opus-4-8", "openai", "oauth"),
        ]
    }

    fn setup<'a>(
        cx: &'a mut gpui::TestAppContext,
    ) -> (
        gpui::Entity<Panel>,
        &'a mut gpui::VisualTestContext,
        std::sync::mpsc::Receiver<Command>,
    ) {
        cx.update(crate::input::bind_keys);
        let (bridge, commands) = crate::harness::spawn_recording();
        let (panel, vcx) =
            cx.add_window_view(|_, cx| Panel::new("browser-test".into(), None, None, bridge, cx));
        vcx.update(|_, cx| Panel::connect_input(&panel, cx));
        panel.update(vcx, |panel, cx| {
            panel.available_models =
                fixture_routes().iter().map(|route| route.model.clone()).collect();
            panel.input.update(cx, |input, cx| {
                input.set_model_routes(Vec::new(), &fixture_routes(), None, cx);
            });
        });
        (panel, vcx, commands)
    }

    fn rows(panel: &gpui::Entity<Panel>, vcx: &mut gpui::VisualTestContext) -> Vec<BrowserRow> {
        panel.read_with(vcx, |panel, cx| panel.model_browser_rows(cx))
    }

    #[gpui::test]
    fn subagent_browser_filters_unschedulable_models_but_keeps_current_role_model(
        cx: &mut gpui::TestAppContext,
    ) {
        crate::input::model_favorites::reset();
        let (panel, vcx, _) = setup(cx);
        let spec = panel.update(vcx, |panel, cx| {
            panel.input.update(cx, |input, _| {
                input.set_configured_model_meta(std::collections::HashMap::from([(
                    ("magpie".into(), "codex/gpt-6.2-sol".into()),
                    crate::custom_providers::ConfiguredModelMeta {
                        subagent_schedulable: Some(false),
                        ..Default::default()
                    },
                )]));
                input.model_details().metas.iter()
                    .find(|(_, meta)| meta.subagent_schedulable == Some(false))
                    .expect("configured model metadata is present")
                    .0.clone()
            })
        });
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        assert!(rows(&panel, vcx).iter().any(|row| row.spec == spec));

        panel.update(vcx, |panel, cx| panel.open_agent_role_browser(AgentRole::Memory, cx));
        let role_rows = rows(&panel, vcx);
        assert!(!role_rows.is_empty());
        assert!(role_rows.iter().all(|row| row.spec != spec));

        panel.update(vcx, |panel, _| {
            panel.sync_swarm_result(
                jcode_sdk::api::RuntimeCommand::Agents,
                &serde_json::json!({"overrides": {"memory": &spec}}),
            );
        });
        assert!(rows(&panel, vcx).iter().any(|row| row.spec == spec));
        crate::input::model_favorites::reset();
    }

    #[gpui::test]
    fn chip_opens_browser_while_slash_model_keeps_the_popup(cx: &mut gpui::TestAppContext) {
        crate::input::model_favorites::reset();
        let (panel, vcx, _commands) = setup(cx);
        vcx.run_until_parked();
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser").is_some());
        assert!(vcx.debug_bounds("slash-command-overlay").is_none());
        // The channel split puts codex and cpr-fast in separate group pills.
        panel.read_with(vcx, |panel, cx| {
            assert!(!panel.model_picker_open);
            let input = panel.input.read(cx);
            let filters = model_menu::browser_filters(
                &input.model_details().keys().cloned().collect::<Vec<_>>(),
                input.model_details(),
                &[],
                panel.model.as_deref(), &model_menu::Hidden::NONE,
            );
            let labels: Vec<_> = filters.iter().map(|filter| filter.label.as_str()).collect();
            assert!(labels.contains(&"Magpie · codex"), "{labels:?}");
            assert!(labels.contains(&"Magpie · cpr-fast"), "{labels:?}");
        });
        // Typing `/model` instead still opens the compact popup, not the browser.
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser").is_none());
        vcx.simulate_input("/model");
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| {
            assert!(panel.model_picker_open);
            assert!(panel.model_browser.is_none());
        });
        assert!(vcx.debug_bounds("slash-command-overlay").is_some());
        crate::input::model_favorites::reset();
    }

    #[gpui::test]
    fn browser_filter_search_and_row_click_switch_models(cx: &mut gpui::TestAppContext) {
        crate::input::model_favorites::reset();
        let (panel, vcx, commands) = setup(cx);
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        vcx.run_until_parked();
        assert_eq!(rows(&panel, vcx).len(), 3);

        // A channel pill narrows the list to that group only.
        let pill = vcx
            .debug_bounds("model-browser-filter-Magpie · codex")
            .expect("channel filter pill");
        vcx.simulate_click(pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        let group_rows = rows(&panel, vcx);
        assert_eq!(group_rows.len(), 1);
        assert!(group_rows[0].spec.ends_with("codex/gpt-6.2-sol"));

        // Back to all models; the search query narrows within the filter.
        let all = vcx
            .debug_bounds("model-browser-filter-All models")
            .expect("all filter pill");
        vcx.simulate_click(all.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        panel.update(vcx, |panel, cx| {
            panel
                .model_browser
                .as_ref()
                .unwrap()
                .search
                .update(cx, |search, cx| search.set_content("opus".into(), cx));
        });
        vcx.run_until_parked();
        let found = rows(&panel, vcx);
        assert_eq!(found.len(), 1);
        assert!(found[0].spec.contains("opus-4-8"));

        // Clicking the row runs the same /model <spec> switch and closes.
        let row = vcx.debug_bounds("model-browser-row-0").expect("search hit row");
        vcx.simulate_click(row.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser").is_none());
        assert!(
            matches!(commands.try_recv(), Ok(Command::SetModel { model, .. }) if model.ends_with("opus-4-8")),
            "row click sends SetModel for the exact spec"
        );
        panel.read_with(vcx, |panel, _| {
            assert!(
                panel
                    .items
                    .iter()
                    .any(|item| matches!(item, Item::Assistant(text) if text.contains("Switching model to")))
            );
        });
        crate::input::model_favorites::reset();
    }

    #[gpui::test]
    fn narrow_panel_switches_to_the_horizontal_filter_row(cx: &mut gpui::TestAppContext) {
        crate::input::model_favorites::reset();
        let (panel, vcx, _commands) = setup(cx);
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        vcx.run_until_parked();
        // Wide layout: the left filter column renders.
        assert!(vcx.debug_bounds("model-browser-filter-col").is_some());
        assert!(vcx.debug_bounds("model-browser-filters").is_none());
        // Narrow the window (single panel tracks it): the filters become a
        // horizontal scroll row above the list.
        let handle = vcx.update(|window, _| window.window_handle());
        for width in [480., 1000.] {
            vcx.simulate_window_resize(handle, gpui::size(gpui::px(width), gpui::px(700.)));
            vcx.run_until_parked();
            // Force a fresh paint so the backdrop canvas prepaint records the
            // resized panel bounds and the filters re-layout from it.
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
            let want_col = width > 592.;
            assert_eq!(
                vcx.debug_bounds("model-browser-filter-col").is_some(),
                want_col,
                "width {width}: filter column should be {}present",
                if want_col { "" } else { "not " }
            );
            assert_eq!(
                vcx.debug_bounds("model-browser-filters").is_some(),
                !want_col,
            );
        }
        assert!(
            vcx.debug_bounds("model-browser-filter-Magpie · codex")
                .is_some()
        );
        crate::input::model_favorites::reset();
    }

    #[gpui::test]
    fn narrow_agent_role_browser_keeps_search_and_actions_inside(cx: &mut gpui::TestAppContext) {
        for (chinese, width) in [(false, 550.), (false, 420.), (true, 550.), (true, 420.)] {
            let mut check = || {
                let (panel, vcx, _commands) = setup(cx);
                let handle = vcx.update(|window, _| window.window_handle());
                panel.update(vcx, |panel, cx| panel.open_agent_role_browser(AgentRole::Memory, cx));
                vcx.simulate_window_resize(handle, gpui::size(px(width), px(410.)));
                vcx.run_until_parked();
                vcx.update(|window, _| window.refresh());
                vcx.run_until_parked();

                let browser = vcx.debug_bounds("model-browser").expect("model browser");
                let search = vcx.debug_bounds("model-browser-search").expect("search capsule");
                eprintln!("viewport={width}x410 chinese={chinese}: browser={browser:?}, search={search:?}");
                assert!(search.size.width >= px(150.), "search is squeezed: {search:?}");
                assert!(search.size.height <= px(44.), "search wraps: {search:?}");
                for selector in [
                    "model-browser-search",
                    "model-browser-subagent-inherit",
                    "model-browser-providers",
                    "model-browser-models",
                    "model-browser-close",
                ] {
                    let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
                    eprintln!("{selector}: {bounds:?}");
                    assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
                    assert!(
                        bounds.left() >= browser.left() && bounds.right() <= browser.right()
                            && bounds.top() >= browser.top() && bounds.bottom() <= browser.bottom(),
                        "{selector} exceeds the model browser: {bounds:?} vs {browser:?}",
                    );
                }
            };
            if chinese {
                crate::locale::with_zh(check);
            } else {
                check();
            }
        }
    }

    #[gpui::test]
    fn browser_keyboard_selects_and_escape_restores_composer(cx: &mut gpui::TestAppContext) {
        crate::input::model_favorites::reset();
        let (panel, vcx, commands) = setup(cx);
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        vcx.run_until_parked();
        // Down + Enter picks the second row through the keyboard.
        vcx.simulate_keystrokes("down");
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| {
            assert_eq!(panel.model_browser.as_ref().unwrap().selection, 1);
        });
        let second = rows(&panel, vcx)[1].spec.clone();
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser").is_none());
        assert!(
            matches!(commands.try_recv(), Ok(Command::SetModel { model, .. }) if model == second),
            "Enter sends SetModel for the highlighted row"
        );
        // Reopen; Escape closes and returns focus to the composer.
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        vcx.run_until_parked();
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser").is_none());
        vcx.update(|window, cx| {
            let input = panel.read(cx).input.clone();
            let focused = input.read(cx).focus_handle.is_focused(window);
            assert!(focused, "Escape returns focus to the composer");
        });
        crate::input::model_favorites::reset();
    }

    #[gpui::test]
    fn providers_manage_mode_hides_and_restores_groups(cx: &mut gpui::TestAppContext) {
        crate::input::model_favorites::reset();
        let (panel, vcx, _commands) = setup(cx);
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        vcx.run_until_parked();

        // Open Providers manage mode: group rows carry Shown/Hidden toggles.
        let providers = vcx
            .debug_bounds("model-browser-providers")
            .expect("providers pill");
        vcx.simulate_click(providers.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser-manage-list").is_some());
        assert!(vcx.debug_bounds("model-browser-show-all").is_none());

        let codex_key = "magpie · openai-compatible:magpie · codex";
        let hide_selector: &'static str =
            Box::leak(format!("model-browser-hide-{codex_key}").into_boxed_str());
        let hide = vcx
            .debug_bounds(hide_selector)
            .expect("codex group toggle");
        vcx.simulate_click(hide.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("model-browser-show-all").is_some(),
            "Show all appears once a group is hidden"
        );

        // Done: the codex channel's pill and rows are gone.
        let done = vcx.debug_bounds("model-browser-providers").unwrap();
        vcx.simulate_click(done.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("model-browser-filter-Magpie · codex")
                .is_none(),
            "hidden group's filter pill disappears"
        );
        let visible = rows(&panel, vcx);
        assert_eq!(visible.len(), 2);
        assert!(visible.iter().all(|row| !row.spec.contains("codex/")));

        // Show all restores the group.
        let providers = vcx.debug_bounds("model-browser-providers").unwrap();
        vcx.simulate_click(providers.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        let show_all = vcx
            .debug_bounds("model-browser-show-all")
            .expect("show all pill");
        vcx.simulate_click(show_all.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser-show-all").is_none());
        let done = vcx.debug_bounds("model-browser-providers").unwrap();
        vcx.simulate_click(done.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("model-browser-filter-Magpie · codex")
                .is_some()
        );
        assert!(rows(&panel, vcx).iter().any(|row| row.spec.contains("codex/")));
        crate::input::model_favorites::reset();
    }
    #[gpui::test]
    fn models_manage_mode_hides_and_restores_individual_models(cx: &mut gpui::TestAppContext) {
        crate::input::model_favorites::reset();
        let (panel, vcx, commands) = setup(cx);
        panel.update_in(vcx, |panel, window, cx| panel.toggle_model_picker(window, cx));
        vcx.run_until_parked();

        // Open Models manage mode beside Providers: every route with a
        // Shown/Hidden toggle, none hidden yet.
        let models_pill = vcx.debug_bounds("model-browser-models").expect("models pill");
        vcx.simulate_click(models_pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser-manage-list").is_some());
        assert!(vcx.debug_bounds("model-browser-show-all").is_none());
        let spec = "magpie:cpr-fast/gpt-6.2-sol";
        let row_selector: &'static str =
            Box::leak(format!("model-browser-manage-model-{spec}").into_boxed_str());
        assert!(
            vcx.debug_bounds(row_selector).is_some(),
            "exact-spec row exists"
        );

        // Hide that exact spec through its row toggle.
        let hide_selector: &'static str =
            Box::leak(format!("model-browser-model-hide-{spec}").into_boxed_str());
        let hide = vcx.debug_bounds(hide_selector).expect("model toggle");
        vcx.simulate_click(hide.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser-show-all").is_some());

        // No silent fallback: hiding never sends SetModel — the active model
        // stays selected until the user explicitly switches.
        assert!(
            commands.try_recv().is_err(),
            "hiding a model must not switch the session model"
        );

        // Done: the exact spec drops from the list while same-name routes on
        // other channels stay.
        let done = vcx.debug_bounds("model-browser-models").unwrap();
        vcx.simulate_click(done.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        let visible = rows(&panel, vcx);
        assert_eq!(visible.len(), 2);
        assert!(visible.iter().all(|row| !row.spec.contains("cpr-fast/")));

        // Even a favorited hidden model drops from the Favorites filter.
        panel.update(vcx, |_panel, _| {
            crate::input::model_favorites::toggle(spec);
        });
        let favorites_pill = vcx
            .debug_bounds("model-browser-filter-Favorites")
            .expect("favorites pill");
        vcx.simulate_click(favorites_pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(rows(&panel, vcx).is_empty());

        // Show all restores the model, reversing the hide in the store.
        let models_pill = vcx.debug_bounds("model-browser-models").unwrap();
        vcx.simulate_click(models_pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        let show_all = vcx
            .debug_bounds("model-browser-show-all")
            .expect("show all pill");
        vcx.simulate_click(show_all.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser-show-all").is_none());
        panel.read_with(vcx, |_, _| {
            let (_, hidden_models) = crate::input::model_favorites::hidden_models();
            assert!(!crate::input::model_favorites::is_model_hidden(
                &hidden_models, spec
            ));
        });
        crate::input::model_favorites::reset();
    }
}
