//! The "Choose model" browser opened by the composer chip and the
//! `OpenModelPicker` action: a large dialog with provider/channel filter
//! pills, search, capability metadata, and favorite stars. Typing `/model`
//! still opens the compact popup next to the composer.

use super::*;
use crate::input::model_favorites;
use crate::input::model_menu::{self, BrowserRow, ModelFilter};

/// Open state for the model browser dialog.
pub(super) struct ModelBrowserState {
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
    /// Providers manage mode: the list shows per-group Shown/Hidden
    /// toggles instead of models.
    pub manage: bool,
}

/// One provider/channel group in Providers manage mode.
struct ManageGroup {
    key: String,
    label: String,
    logo: &'static str,
    count: usize,
    hidden: bool,
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

impl Panel {
    /// Open the browser, closing the other pickers first.
    pub(super) fn open_model_browser(&mut self, cx: &mut Context<Self>) {
        if self.is_side_document() || self.available_models.is_empty() {
            return;
        }
        self.login = None;
        self.extensions = None;
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
            let mut search = PromptInput::new(cx, "Search models…", |_, _, _, _| {})
                .without_command_completion()
                .with_on_overlay_cancel(move |app| {
                    let restored = cancel.update(app, |panel, cx| {
                        panel.model_browser = None;
                        cx.notify();
                        Some(panel.input_focus_handle(cx))
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
            search,
            filter: ModelFilter::All,
            selection: 0,
            scroll: ScrollHandle::new(),
            focus_pending: true,
            panel_bounds: Default::default(),
            manage: false,
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
        });
        let input = self.input.clone();
        cx.defer(move |cx| input.update(cx, |input, _cx| input.load_configured_model_meta()));
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
                state.manage = true;
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

    /// Rows for the active filter and search query.
    fn model_browser_rows(&self, cx: &App) -> Vec<BrowserRow> {
        let Some(state) = &self.model_browser else {
            return Vec::new();
        };
        let input = self.input.read(cx);
        let details = input.model_details();
        let models: Vec<String> = if details.is_empty() {
            input.command_models().to_vec()
        } else {
            details.keys().cloned().collect()
        };
        let (_, favorites) = model_favorites::favorites();
        let (_, hidden) = model_favorites::hidden_groups();
        let query = state.search.read(cx).content.to_string();
        model_menu::browser_rows(
            &state.filter,
            &models,
            details,
            input.model_metas(),
            &favorites,
            self.model.as_deref(),
            &query,
            model_menu::now_unix_secs(),
            &hidden,
        )
    }

    /// Provider/channel groups for Providers manage mode: every group the
    /// catalog supplied (hidden groups included — unconfigured presets were
    /// already dropped by `set_model_routes`), filtered by the search query
    /// on the group label.
    fn model_browser_manage_rows(&self, cx: &App) -> Vec<ManageGroup> {
        let input = self.input.read(cx);
        let details = input.model_details();
        let models: Vec<String> = if details.is_empty() {
            input.command_models().to_vec()
        } else {
            details.keys().cloned().collect()
        };
        let (_, hidden) = model_favorites::hidden_groups();
        let query = self
            .model_browser
            .as_ref()
            .map(|state| state.search.read(cx).content.to_lowercase())
            .unwrap_or_default();
        model_menu::groups_of(&models, details, self.model.as_deref(), &[])
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

    /// Select `spec`: the same path as submitting `/model <spec>` (preview
    /// interception, transcript note, `Command::SetModel`).
    fn choose_browser_model(&mut self, spec: String, window: &mut Window, cx: &mut Context<Self>) {
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
            let models: Vec<String> = if details.is_empty() {
                input.command_models().to_vec()
            } else {
                details.keys().cloned().collect()
            };
            let (_, favorites) = model_favorites::favorites();
            let (_, hidden) = model_favorites::hidden_groups();
            model_menu::browser_filters(
                &models,
                details,
                &favorites,
                self.model.as_deref(),
                &hidden,
            )
        };
        let manage = state.manage;
        let manage_rows = if manage {
            self.model_browser_manage_rows(cx)
        } else {
            Vec::new()
        };
        let row_count = if manage { manage_rows.len() } else { rows.len() };
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
            let text = format!("{} {}", filter.label, filter.count);
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
                            .child(header.clone()),
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
                                    .child("Current"),
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

        let manage_items: Vec<_> = manage_rows
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
                            .child(if group_hidden { "Hidden" } else { "Shown" })
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
        let body_list = if manage {
            let any_hidden = manage_rows.iter().any(|group| group.hidden)
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
                            .child("Show all")
                            .on_click(cx.listener(|_, _, _, cx| {
                                model_favorites::show_all();
                                cx.notify();
                            })),
                    )
                })
                .children(manage_items)
                .when(manage_rows.is_empty(), |el| {
                    el.child(
                        div()
                            .debug_selector(|| "model-browser-empty".into())
                            .px_3()
                            .py_6()
                            .text_size(px(12.))
                            .text_color(theme.TEXT_DIM)
                            .child("No models match"),
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
                                "No favorites yet — star a model to pin it here"
                            } else {
                                "No models match"
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
                        this.model_browser = None;
                        this.focus_input(window, cx);
                        cx.notify();
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
                                        if this.model_browser.is_none() {
                                            return;
                                        }
                                        this.model_browser = None;
                                        this.focus_input(window, cx);
                                        cx.stop_propagation();
                                        cx.notify();
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
                                        if this
                                            .model_browser
                                            .as_ref()
                                            .is_some_and(|state| state.manage)
                                        {
                                            let groups =
                                                this.model_browser_manage_rows(cx);
                                            if let Some(group) = groups.get(index) {
                                                let key = group.key.clone();
                                                this.toggle_browser_group_hidden(
                                                    &key, cx,
                                                );
                                            }
                                        } else {
                                            let rows = this.model_browser_rows(cx);
                                            if let Some(row) = rows.get(index) {
                                                let spec = row.spec.clone();
                                                this.choose_browser_model(
                                                    spec, window, cx,
                                                );
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
                                        let max = if this
                                            .model_browser
                                            .as_ref()
                                            .is_some_and(|state| state.manage)
                                        {
                                            this.model_browser_manage_rows(cx).len()
                                        } else {
                                            this.model_browser_rows(cx).len()
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
                                        .child(
                                            div()
                                                .flex_none()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(px(14.))
                                                .text_color(theme.TEXT)
                                                .child("Choose model"),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .rounded_full()
                                                .bg(theme.INPUT_BG)
                                                .border_1()
                                                .border_color(theme.INPUT_BORDER)
                                                .px_2()
                                                .child(search),
                                        )
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
                                                .bg(if manage {
                                                    theme.ACCENT
                                                } else {
                                                    theme.HEADER_BG
                                                })
                                                .text_color(if manage {
                                                    theme.BG
                                                } else {
                                                    theme.TEXT_DIM
                                                })
                                                .when(!manage, |el| {
                                                    el.hover(|el| el.bg(theme.INPUT_BG))
                                                })
                                                .child(if manage { "Done" } else { "Providers" })
                                                .on_click(cx.listener(
                                                    |this, _, _, cx| {
                                                        if let Some(state) =
                                                            this.model_browser.as_mut()
                                                        {
                                                            state.manage = !state.manage;
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
                                                .child("Close")
                                                .on_click(cx.listener(
                                                    |this, _, window, cx| {
                                                        this.model_browser = None;
                                                        this.focus_input(window, cx);
                                                        cx.notify();
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
                panel.model.as_deref(), &[],
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
}
