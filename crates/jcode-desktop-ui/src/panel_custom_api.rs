//! Native form and list rows for custom OpenAI-compatible third-party APIs
//! (`[providers.<name>]` in `~/.jcode/config.toml`).
//!
//! API keys stay inside [`LoginInput`] (bullet-masked, never copied or snapshotted)
//! until saved into Jcode's private `provider-<name>.env` file.

use super::*;
use crate::custom_providers::{CustomProviderDraft, CustomProviderSummary};

pub(super) struct CustomApiEditor {
    pub original_name: Option<String>,
    pub has_existing_key: bool,
    pub base_url: Entity<PromptInput>,
    pub api_key: Entity<LoginInput>,
    pub name: Entity<PromptInput>,
    pub default_model: Entity<PromptInput>,
    pub extra_models: Entity<PromptInput>,
    pub fetched_models: Vec<String>,
    pub set_default: bool,
    pub model_catalog: bool,
    pub fetching: bool,
    pub saving: bool,
    pub notice: Option<(String, bool)>,
    pub task: Option<Task<()>>,
}

fn plain_field(placeholder: &'static str, initial: &str, cx: &mut Context<Panel>) -> Entity<PromptInput> {
    let entity = cx.new(|cx| {
        let mut field =
            PromptInput::new(cx, placeholder, |_, _, _, _| {}).without_command_completion();
        field.set_submission_enabled(false, cx);
        if !initial.is_empty() {
            field.set_content(initial.to_string(), cx);
        }
        field
    });
    entity
}

impl Panel {
    pub(super) fn open_custom_api_editor(
        &mut self,
        existing: Option<CustomProviderSummary>,
        cx: &mut Context<Self>,
    ) {
        if self.login.is_none() {
            self.open_login_picker(cx);
        }
        if self.login_is_remote() {
            return;
        }
        let base_initial = existing
            .as_ref()
            .map(|p| p.base_url.as_str())
            .unwrap_or("");
        let name_initial = existing.as_ref().map(|p| p.name.as_str()).unwrap_or("");
        let default_initial = existing
            .as_ref()
            .and_then(CustomProviderSummary::active_model)
            .unwrap_or("");
        let others: Vec<&str> = existing
            .as_ref()
            .map(|p| {
                p.models
                    .iter()
                    .map(String::as_str)
                    .filter(|id| *id != default_initial)
                    .collect()
            })
            .unwrap_or_default();
        let extra_initial = others.join(", ");
        let key_placeholder = if existing.as_ref().is_some_and(|p| p.has_api_key) {
            "Stored securely — leave blank to keep, or paste a new key"
        } else {
            "sk-... (optional for localhost / LAN servers)"
        };

        let base_url = plain_field(
            "https://api.example.com/v1  or  http://127.0.0.1:3425/v1",
            base_initial,
            cx,
        );
        let name = plain_field(
            "Optional name (auto-filled from URL, e.g. siliconflow)",
            name_initial,
            cx,
        );
        let default_model = plain_field(
            "Default model ID (e.g. deepseek-ai/DeepSeek-V3 or gpt-4o)",
            default_initial,
            cx,
        );
        let extra_models = plain_field(
            "Optional extra model IDs, comma-separated (or click Fetch /models)",
            &extra_initial,
            cx,
        );
        let api_key = cx.new(|cx| LoginInput::new(cx, key_placeholder));

        let Some(state) = self.login.as_mut() else {
            return;
        };
        state.provider = None;
        state.complete = false;
        state.error = None;
        state.focus_pending = true;
        state.scroll.set_offset(point(px(0.), px(0.)));
        state.custom_api = Some(CustomApiEditor {
            original_name: existing.as_ref().map(|p| p.name.clone()),
            has_existing_key: existing.as_ref().is_some_and(|p| p.has_api_key),
            base_url,
            api_key,
            name,
            default_model,
            extra_models,
            fetched_models: existing.as_ref().map(|p| p.models.clone()).unwrap_or_default(),
            set_default: existing.as_ref().map_or(true, |p| p.is_default),
            model_catalog: existing.as_ref().map_or(true, |p| p.model_catalog),
            fetching: false,
            saving: false,
            notice: None,
            task: None,
        });
        cx.notify();
    }

    pub(super) fn close_custom_api_editor(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.login.as_mut() {
            if let Some(editor) = state.custom_api.take() {
                editor.api_key.update(cx, |input, cx| input.clear(cx));
            }
            state.error = None;
            state.focus_pending = true;
            state.scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }

    pub(super) fn fetch_custom_api_models(&mut self, cx: &mut Context<Self>) {
        let offline =
            self.preview_state.is_some() || crate::harness::screenshot_mode();
        let Some(state) = self.login.as_mut() else {
            return;
        };
        let Some(editor) = state.custom_api.as_mut() else {
            return;
        };
        if editor.fetching || editor.saving {
            return;
        }
        let base_url = editor.base_url.read(cx).content.to_string();
        let existing_name = editor.original_name.clone();
        // Peek secret without clearing it so Save still has the key afterwards.
        let secret = editor.api_key.update(cx, |input, cx| {
            let taken = input.take(cx);
            taken
        });
        // Put the secret back into the masked field immediately if non-empty.
        let secret_for_fetch = (!secret.trim().is_empty()).then(|| secret.clone());
        let restore = secret;
        if offline {
            editor.notice = Some((
                "Network fetch is disabled in offline preview.".into(),
                true,
            ));
            cx.notify();
            return;
        }
        editor.fetching = true;
        editor.notice = None;
        let api_key_entity = editor.api_key.clone();
        if !restore.is_empty() {
            api_key_entity.update(cx, |input, cx| input.set_secret(restore, cx));
        }

        let work = cx.background_executor().spawn(async move {
            crate::custom_providers::fetch_models(
                &base_url,
                secret_for_fetch.as_deref(),
                existing_name.as_deref(),
            )
        });
        editor.task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |panel, cx| {
                let Some(editor) = panel.login.as_mut().and_then(|s| s.custom_api.as_mut()) else {
                    return;
                };
                editor.fetching = false;
                match result {
                    Ok((working_base, models)) => {
                        let count = models.len();
                        editor
                            .base_url
                            .update(cx, |input, cx| input.set_content(working_base.clone(), cx));
                        if editor.name.read(cx).content.trim().is_empty() {
                            let suggested = crate::custom_providers::suggest_name(&working_base);
                            editor
                                .name
                                .update(cx, |input, cx| input.set_content(suggested, cx));
                        }
                        let current_default =
                            editor.default_model.read(cx).content.trim().to_string();
                        let chosen = if current_default.is_empty()
                            || !models.iter().any(|m| m == &current_default)
                        {
                            let first = models[0].clone();
                            editor
                                .default_model
                                .update(cx, |input, cx| input.set_content(first.clone(), cx));
                            first
                        } else {
                            current_default
                        };
                        let rest: Vec<&str> = models
                            .iter()
                            .map(String::as_str)
                            .filter(|m| *m != chosen)
                            .collect();
                        editor
                            .extra_models
                            .update(cx, |input, cx| input.set_content(rest.join(", "), cx));
                        editor.fetched_models = models;
                        editor.notice = Some((
                            format!("Fetched {count} models from {working_base}. Click a model pill below to pick the default."),
                            false,
                        ));
                    }
                    Err(error) => {
                        editor.notice = Some((error, true));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn submit_custom_api(&mut self, cx: &mut Context<Self>) {
        let offline =
            self.preview_state.is_some() || cfg!(test) || crate::harness::screenshot_mode();
        let Some(state) = self.login.as_mut() else {
            return;
        };
        let Some(editor) = state.custom_api.as_mut() else {
            return;
        };
        if editor.saving || editor.fetching {
            return;
        }
        let base_url = editor.base_url.read(cx).content.to_string();
        let name = editor.name.read(cx).content.to_string();
        let default_model = editor.default_model.read(cx).content.to_string();
        let extra = crate::custom_providers::parse_model_list(
            editor.extra_models.read(cx).content.as_ref(),
        );
        let mut models = Vec::new();
        for id in extra.into_iter().chain(editor.fetched_models.iter().cloned()) {
            if !models.contains(&id) {
                models.push(id);
            }
        }
        let secret = editor.api_key.update(cx, |input, cx| input.take(cx));
        let draft = CustomProviderDraft {
            original_name: editor.original_name.clone(),
            name,
            base_url,
            api_key: (!secret.trim().is_empty()).then_some(secret.clone()),
            default_model,
            models,
            set_default: editor.set_default,
            model_catalog: editor.model_catalog,
            model_settings: Default::default(),
        };
        if self.preview_state.is_some() || crate::harness::screenshot_mode() {
            editor.notice = Some((
                "Saving endpoints is disabled in offline preview.".into(),
                true,
            ));
            cx.notify();
            return;
        }
        editor.saving = true;
        editor.notice = None;
        let restore_on_err = secret;
        let api_key_entity = editor.api_key.clone();
        let work = cx.background_executor().spawn(async move {
            if offline {
                // Unit tests verify validation and UI transitions without touching ~/.jcode.
                let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
                let path = temp.path().join("config.toml");
                crate::custom_providers::save_at(
                    &path,
                    draft,
                    &|_, _, _| Ok(()),
                    &|_, _| true,
                )
            } else {
                crate::custom_providers::save(draft)
            }
        });
        editor.task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |panel, cx| {
                match result {
                    Ok(summary) => {
                        let route_spec = summary.route_spec();
                        if let Some(state) = panel.login.as_mut() {
                            state.custom_api = None;
                            state.provider = None;
                            state.complete = true;
                            state.busy = false;
                            state.error = None;
                        }
                        if let Some(session_id) = panel.login_target_session() {
                            panel.bridge.send(Command::RefreshRuntime {
                                session_id: session_id.clone(),
                            });
                            if summary.is_default
                                && let Some(model) = route_spec
                            {
                                panel.bridge.send(Command::SetModel { session_id, model });
                            }
                        }
                    }
                    Err(error) => {
                        if !restore_on_err.is_empty() {
                            api_key_entity
                                .update(cx, |input, cx| input.set_secret(restore_on_err, cx));
                        }
                        if let Some(editor) =
                            panel.login.as_mut().and_then(|s| s.custom_api.as_mut())
                        {
                            editor.saving = false;
                            editor.notice = Some((error, true));
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn activate_custom_provider(
        &mut self,
        summary: CustomProviderSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offline =
            self.preview_state.is_some() || cfg!(test) || crate::harness::screenshot_mode();
        let name = summary.name.clone();
        let fallback_spec = summary.route_spec();
        if offline {
            if let Some(session_id) = self.login_target_session()
                && let Some(model) = fallback_spec
            {
                self.bridge.send(Command::SetModel { session_id, model });
            }
            self.close_login_picker(cx);
            if self.is_accounts_panel() {
                cx.emit(AccountsPanelClosed);
            } else {
                self.focus_input(window, cx);
            }
            return;
        }
        let target = self.login_target_session();
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::custom_providers::make_default(&name) })
                .await;
            let _ = this.update(cx, |panel, cx| match result {
                Ok(spec) => {
                    if let Some(session_id) = target {
                        bridge.send(Command::RefreshRuntime {
                            session_id: session_id.clone(),
                        });
                        if let Some(model) = spec.or(fallback_spec) {
                            bridge.send(Command::SetModel { session_id, model });
                        }
                    }
                    panel.refresh_login_status(cx);
                }
                Err(error) => {
                    if let Some(state) = panel.login.as_mut() {
                        state.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn delete_custom_provider(&mut self, name: String, cx: &mut Context<Self>) {
        let offline =
            self.preview_state.is_some() || cfg!(test) || crate::harness::screenshot_mode();
        if offline {
            if let Some(state) = self.login.as_mut() {
                state.accounts.custom.retain(|item| item.name != name);
                cx.notify();
            }
            return;
        }
        let target = self.login_target_session();
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::custom_providers::remove(&name) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                if let Err(error) = result {
                    if let Some(state) = panel.login.as_mut() {
                        state.error = Some(error);
                        cx.notify();
                    }
                    return;
                }
                if let Some(session_id) = target {
                    bridge.send(Command::RefreshRuntime { session_id });
                }
                panel.refresh_login_status(cx);
            });
        })
        .detach();
    }

    pub(super) fn custom_api_matches_query(&self, query: &str) -> bool {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() || "custom api openai compatible third party gateway".contains(&q) {
            return true;
        }
        self.login.as_ref().is_some_and(|state| {
            state.accounts.custom.iter().any(|item| {
                item.name.to_ascii_lowercase().contains(&q)
                    || item.base_url.to_ascii_lowercase().contains(&q)
                    || item.models.iter().any(|m| m.to_ascii_lowercase().contains(&q))
            })
        })
    }

    pub(super) fn render_custom_api_section(
        &self,
        query: &str,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.custom_api_matches_query(query) {
            return None;
        }
        let theme = Theme::global();
        let selecting = self
            .login
            .as_ref()
            .is_some_and(|state| state.selection_only);
        let items: Vec<CustomProviderSummary> = self
            .login
            .as_ref()
            .map(|state| {
                let q = query.trim().to_ascii_lowercase();
                state
                    .accounts
                    .custom
                    .iter()
                    .filter(|item| {
                        q.is_empty()
                            || item.name.to_ascii_lowercase().contains(&q)
                            || item.base_url.to_ascii_lowercase().contains(&q)
                            || item
                                .models
                                .iter()
                                .any(|m| m.to_ascii_lowercase().contains(&q))
                            || "custom api openai compatible third party gateway".contains(&q)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let mut section = div()
            .id("login-group-custom-api")
            .debug_selector(|| "login-group-custom-api".into())
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_xl()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.TEXT)
                                    .child("Custom API endpoints"),
                            )
                            .when(!items.is_empty(), |el| {
                                el.child(pill_badge(items.len().to_string(), theme.TEXT_DIM))
                            }),
                    )
                    .when(!selecting, |el| {
                        el.child(
                            div()
                                .id("custom-api-add")
                                .debug_selector(|| "custom-api-add".into())
                                .px_3()
                                .py_1()
                                .rounded_full()
                                .bg(theme.ACCENT.opacity(0.16))
                                .text_color(theme.ACCENT)
                                .text_size(px(12.))
                                .cursor_pointer()
                                .hover(|el| el.bg(theme.ACCENT.opacity(0.28)))
                                .child("+ Add custom API")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_custom_api_editor(None, cx);
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .px_2()
                    .text_size(px(11.))
                    .text_color(theme.TEXT_DIM)
                    .child("Connect any OpenAI-compatible third-party API (Base URL + API key + /models)."),
            );

        for item in items {
            section = section.child(self.render_custom_api_row(item, selecting, cx));
        }
        Some(section.into_any_element())
    }

    fn render_custom_api_row(
        &self,
        item: CustomProviderSummary,
        selecting: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let name = item.name.clone();
        let row_id = format!("custom-api-row-{name}");
        let row_selector = row_id.clone();
        let model_count = item.models.len();
        let model_label = if model_count == 1 {
            "1 model".to_string()
        } else {
            format!("{model_count} models")
        };
        let subtitle = match item.active_model() {
            Some(model) => format!("{} · {model}", item.base_url),
            None => item.base_url.clone(),
        };

        let mut header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_2()
            .gap_y_1()
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme.TEXT)
                    .child(item.name.clone()),
            )
            .child(pill_badge(model_label, theme.TEXT_DIM))
            .child(pill_badge(
                if item.has_api_key {
                    "API key"
                } else {
                    "Local / no key"
                },
                theme.TEXT_DIM,
            ));
        if item.is_default {
            header = header.child(pill_badge("Default", theme.ACCENT));
        }

        let mut actions = div().flex().flex_none().items_center().gap_1p5();
        if selecting {
            actions = actions.child(pill_badge("Select", theme.ACCENT));
        } else {
            let use_item = item.clone();
            let use_id = format!("custom-api-use-{name}");
            let use_sel = use_id.clone();
            actions = actions.child(
                action_pill(use_id, use_sel, if item.is_default { "Use now" } else { "Set default" }, false)
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.activate_custom_provider(use_item.clone(), window, cx);
                    })),
            );
            let edit_item = item.clone();
            let edit_id = format!("custom-api-edit-{name}");
            let edit_sel = edit_id.clone();
            actions = actions.child(
                action_pill(edit_id, edit_sel, "Edit", false)
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_custom_api_editor(Some(edit_item.clone()), cx);
                    })),
            );
            let remove_name = item.name.clone();
            let rm_id = format!("custom-api-remove-{name}");
            let rm_sel = rm_id.clone();
            actions = actions.child(
                action_pill(rm_id, rm_sel, "Remove", true)
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.delete_custom_provider(remove_name.clone(), cx);
                    })),
            );
        }

        let click_item = item.clone();
        div()
            .id(SharedString::from(row_id))
            .debug_selector(move || row_selector.clone())
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .min_h(px(52.))
            .rounded_xl()
            .bg(theme.HEADER_BG)
            .cursor_pointer()
            .hover(|el| el.bg(theme.ACCENT_DIM))
            .child(
                div()
                    .flex_none()
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.PANEL_BG)
                    .child(crate::accounts::lettermark(&item.name)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(header)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.TEXT_DIM)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(subtitle),
                    ),
            )
            .child(actions)
            .on_click(cx.listener(move |this, _, window, cx| {
                if selecting {
                    this.activate_custom_provider(click_item.clone(), window, cx);
                } else {
                    this.open_custom_api_editor(Some(click_item.clone()), cx);
                }
            }))
            .into_any_element()
    }

    pub(super) fn render_custom_api_editor(
        &self,
        editor: &CustomApiEditor,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let editing = editor.original_name.as_deref();
        let heading = match editing {
            Some(name) => format!("Edit custom API · {name}"),
            None => "Add custom OpenAI-compatible API".to_string(),
        };
        let selected_default = editor.default_model.read(cx).content.trim().to_string();
        let api_key_entity = editor.api_key.clone();

        let mut model_chips = div().flex().flex_wrap().gap_1p5();
        for model in editor.fetched_models.iter().take(24) {
            let active = model == &selected_default;
            let model_id = model.clone();
            let chip_id = format!("custom-api-model-{model_id}");
            let default_field = editor.default_model.clone();
            model_chips = model_chips.child(
                div()
                    .id(SharedString::from(chip_id))
                    .px_2p5()
                    .py(px(3.))
                    .rounded_full()
                    .bg(if active {
                        theme.ACCENT.opacity(0.22)
                    } else {
                        theme.PANEL_BG
                    })
                    .text_size(px(11.))
                    .text_color(if active { theme.ACCENT } else { theme.TEXT_DIM })
                    .cursor_pointer()
                    .hover(|el| el.bg(theme.ACCENT_DIM).text_color(theme.TEXT))
                    .child(model.clone())
                    .on_click(cx.listener(move |_, _, _, cx| {
                        default_field
                            .update(cx, |input, cx| input.set_content(model_id.clone(), cx));
                        cx.notify();
                    })),
            );
        }

        div()
            .debug_selector(|| "custom-api-editor".into())
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded_xl()
            .bg(theme.HEADER_BG)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.))
                            .child(heading),
                    )
                    .child(
                        login_button("custom-api-back", "Back").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_custom_api_editor(cx);
                            },
                        )),
                    ),
            )
            .child(field_block(
                "Base URL",
                "OpenAI-compatible root URL (https://.../v1, or http://127.0.0.1:PORT/v1 for local gateways).",
                editor.base_url.clone().into_any_element(),
            ))
            .child(field_block(
                "API key",
                if editor.has_existing_key {
                    "Stored in Jcode's private env file. Leave blank to keep your existing key."
                } else {
                    "Stored in Jcode's private env file, never in plaintext config.toml or chat."
                },
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(editor.api_key.clone()))
                    .child(
                        login_button("custom-api-paste-key", "Paste").on_click(cx.listener(
                            move |_, _, window, cx| {
                                api_key_entity.update(cx, |input, cx| input.paste(window, cx));
                            },
                        )),
                    )
                    .into_any_element(),
            ))
            .child(field_block(
                "Endpoint name",
                "Used as the route prefix in /model (for example siliconflow:deepseek-v3).",
                editor.name.clone().into_any_element(),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Default model"),
                            )
                            .child(
                                div()
                                    .id("custom-api-fetch")
                                    .debug_selector(|| "custom-api-fetch".into())
                                    .px_3()
                                    .py_1()
                                    .rounded_full()
                                    .bg(theme.ACCENT.opacity(0.16))
                                    .text_color(theme.ACCENT)
                                    .text_size(px(12.))
                                    .when(!editor.fetching, |el| {
                                        el.cursor_pointer()
                                            .hover(|el| el.bg(theme.ACCENT.opacity(0.28)))
                                    })
                                    .when(editor.fetching, |el| el.opacity(0.6))
                                    .child(if editor.fetching {
                                        "Fetching /models…"
                                    } else {
                                        "Fetch /models"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.fetch_custom_api_models(cx);
                                    })),
                            ),
                    )
                    .child(editor.default_model.clone())
                    .when(!editor.fetched_models.is_empty(), |el| el.child(model_chips)),
            )
            .child(field_block(
                "Extra models (optional)",
                "Comma-separated model IDs exposed in /model alongside the default model.",
                editor.extra_models.clone().into_any_element(),
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(toggle_pill(
                        "custom-api-toggle-default",
                        if editor.set_default {
                            "Default provider: On"
                        } else {
                            "Default provider: Off"
                        },
                        editor.set_default,
                        cx.listener(|this, _, _, cx| {
                            if let Some(ed) =
                                this.login.as_mut().and_then(|s| s.custom_api.as_mut())
                            {
                                ed.set_default = !ed.set_default;
                                cx.notify();
                            }
                        }),
                    ))
                    .child(toggle_pill(
                        "custom-api-toggle-catalog",
                        if editor.model_catalog {
                            "Sync /models catalog: On"
                        } else {
                            "Sync /models catalog: Off"
                        },
                        editor.model_catalog,
                        cx.listener(|this, _, _, cx| {
                            if let Some(ed) =
                                this.login.as_mut().and_then(|s| s.custom_api.as_mut())
                            {
                                ed.model_catalog = !ed.model_catalog;
                                cx.notify();
                            }
                        }),
                    )),
            )
            .when_some(editor.notice.as_ref(), |el, (text, is_err)| {
                el.child(
                    div()
                        .debug_selector(|| "custom-api-notice".into())
                        .text_size(px(12.))
                        .text_color(if *is_err { theme.ERROR } else { theme.OK })
                        .child(text.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .child(
                        div()
                            .id("custom-api-save")
                            .debug_selector(|| "custom-api-save".into())
                            .px_4()
                            .py_1p5()
                            .rounded_full()
                            .bg(theme.ACCENT)
                            .text_color(theme.PANEL_BG)
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(!editor.saving, |el| {
                                el.cursor_pointer().hover(|el| el.opacity(0.9))
                            })
                            .when(editor.saving, |el| el.opacity(0.6))
                            .child(if editor.saving {
                                "Saving…"
                            } else {
                                "Save & connect"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.submit_custom_api(cx);
                            })),
                    )
                    .child(
                        login_button("custom-api-cancel", "Cancel").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_custom_api_editor(cx);
                            },
                        )),
                    ),
            )
            .into_any_element()
    }
}

fn field_block(
    label: &'static str,
    hint: &'static str,
    input: gpui::AnyElement,
) -> gpui::AnyElement {
    let theme = Theme::global();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.TEXT)
                .child(label),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme.TEXT_DIM)
                .child(hint),
        )
        .child(input)
        .into_any_element()
}

fn pill_badge(label: impl Into<SharedString>, color: gpui::Rgba) -> gpui::Div {
    div()
        .flex_none()
        .px_2p5()
        .py(px(2.))
        .rounded_full()
        .bg(color.opacity(0.12))
        .text_size(px(11.))
        .text_color(color)
        .whitespace_nowrap()
        .child(label.into())
}

fn action_pill(
    id: String,
    selector: String,
    label: &'static str,
    danger: bool,
) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    let fg = if danger { theme.ERROR } else { theme.TEXT_DIM };
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px_2p5()
        .py(px(3.))
        .rounded_full()
        .border_1()
        .border_color(theme.PANEL_BORDER)
        .text_size(px(11.))
        .text_color(fg)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |el| {
            el.bg(if danger {
                theme.ERROR.opacity(0.14)
            } else {
                theme.ACCENT_DIM
            })
            .text_color(if danger { theme.ERROR } else { theme.TEXT })
        })
        .child(label)
}

fn toggle_pill(
    id: &'static str,
    label: &'static str,
    active: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let theme = Theme::global();
    let color = if active { theme.ACCENT } else { theme.TEXT_DIM };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px_3()
        .py_1()
        .rounded_full()
        .bg(color.opacity(if active { 0.18 } else { 0.1 }))
        .text_size(px(12.))
        .text_color(color)
        .cursor_pointer()
        .hover(|el| el.bg(theme.ACCENT_DIM))
        .child(label)
        .on_click(on_click)
        .into_any_element()
}
