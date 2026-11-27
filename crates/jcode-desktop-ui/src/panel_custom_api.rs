//! Native form and list rows for custom OpenAI-compatible third-party APIs
//! (`[providers.<name>]` in `~/.jcode/config.toml`).
//!
//! API keys stay inside [`LoginInput`] (bullet-masked, never copied or snapshotted)
//! until saved into Jcode's private `provider-<name>.env` file.

use super::*;
use crate::custom_providers::{CustomProviderDraft, CustomProviderSummary, RuntimeReloadStatus};
use crate::locale::{is_zh, tr};
#[path = "panel_custom_api_model_settings.rs"]
mod model_settings;
use model_settings::ModelSettingsEditor;

pub(super) struct CustomApiEditor {
    pub original_name: Option<String>,
    pub has_existing_key: bool,
    pub base_url: Entity<PromptInput>,
    pub api_key: Entity<LoginInput>,
    pub name: Entity<PromptInput>,
    pub default_model: Entity<PromptInput>,
    default_model_filter: Entity<PromptInput>,
    models_expanded: bool,
    pub extra_models: Entity<PromptInput>,
    pub fetched_models: Vec<String>,
    model_settings: ModelSettingsEditor,
    pub set_default: bool,
    pub model_catalog: bool,
    pub fetching: bool,
    pub saving: bool,
    pub notice: Option<(String, bool)>,
    pub task: Option<Task<()>>,
}

fn plain_field(placeholder: &'static str, initial: &str, cx: &mut Context<Panel>) -> Entity<PromptInput> {
    let editor = cx.weak_entity();
    let entity = cx.new(|cx| {
        let mut field =
            PromptInput::new(cx, placeholder, |_, _, _, _| {}).without_command_completion();
        field.set_submission_enabled(false, cx);
        if !initial.is_empty() {
            field.set_content(initial.to_string(), cx);
        }
        field.with_on_change(move |_, app| {
            let _ = editor.update(app, |_, cx| cx.notify());
        })
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
            tr("Stored securely — leave blank to keep, or paste a new key", "已安全保存。留空保留原密钥，或粘贴新密钥")
        } else {
            tr("sk-... (optional for localhost / LAN servers)", "sk-...（本机或局域网服务可留空）")
        };

        let base_url = plain_field(
            tr("https://api.example.com/v1  or  http://127.0.0.1:3425/v1", "https://api.example.com/v1 或 http://127.0.0.1:3425/v1"),
            base_initial,
            cx,
        );
        let name = plain_field(
            tr("Optional name (auto-filled from URL, e.g. siliconflow)", "选填，根据 URL 自动填写，例如 siliconflow"),
            name_initial,
            cx,
        );
        let default_model = plain_field(
            tr("Default model ID (e.g. deepseek-ai/DeepSeek-V3 or gpt-4o)", "默认模型 ID，例如 deepseek-ai/DeepSeek-V3 或 gpt-4o"),
            default_initial,
            cx,
        );
        let default_model_filter = plain_field(tr("Filter /models…", "筛选已拉取模型…"), "", cx);
        let extra_models = plain_field(
            tr("Optional extra model IDs, comma-separated (or click Fetch /models)", "选填，用逗号分隔额外模型 ID，也可点击拉取 /models"),
            &extra_initial,
            cx,
        );
        let api_key = cx.new(|cx| LoginInput::new(cx, key_placeholder));
        let offline = self.preview_state.is_some() || cfg!(test) || crate::harness::screenshot_mode();
        let model_settings = ModelSettingsEditor::new(name_initial, offline, cx);

        let Some(state) = self.login.as_mut() else {
            return;
        };
        state.provider = None;
        state.complete = false;
        state.error = None;
        state.custom_api_reload = None;
        state.focus_pending = true;
        state.scroll.set_offset(point(px(0.), px(0.)));
        state.custom_api = Some(CustomApiEditor {
            original_name: existing.as_ref().map(|p| p.name.clone()),
            has_existing_key: existing.as_ref().is_some_and(|p| p.has_api_key),
            base_url,
            api_key,
            name,
            default_model,
            default_model_filter,
            models_expanded: false,
            extra_models,
            fetched_models: existing.as_ref().map(|p| p.models.clone()).unwrap_or_default(),
            model_settings,
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
        if offline || cfg!(test) {
            editor.notice = Some((
                tr("Network fetch is disabled in offline preview.", "离线预览无法拉取模型目录。").into(),
                true,
            ));
            cx.notify();
            return;
        }
        let secret = editor.api_key.update(cx, |input, cx| input.take(cx));
        let secret_for_fetch = (!secret.trim().is_empty()).then(|| secret.clone());
        let restore = secret;
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
                            if is_zh() {
                                format!("已从 {working_base} 拉取 {count} 个模型。点击下方模型可设为默认。")
                            } else {
                                format!("Fetched {count} models from {working_base}. Click a model pill below to pick the default.")
                            },
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
        let models = editor.model_ids(cx);
        let model_settings = match editor.model_settings.pending_edits(&models, cx) {
            Ok(edits) => edits,
            Err((id, error)) => {
                editor.model_settings.target = Some(id);
                editor.notice = Some((error, true));
                cx.notify();
                return;
            }
        };
        if self.preview_state.is_some() || crate::harness::screenshot_mode() {
            editor.notice = Some((
                tr("Saving endpoints is disabled in offline preview.", "离线预览无法保存端点。").into(),
                true,
            ));
            cx.notify();
            return;
        }
        let secret = editor.api_key.update(cx, |input, cx| input.take(cx));
        let draft = CustomProviderDraft {
            original_name: editor.original_name.clone(),
            name,
            base_url,
            api_key: (!secret.trim().is_empty()).then_some(secret.clone()),
            default_model,
            models,
            model_settings,
            set_default: editor.set_default,
            model_catalog: editor.model_catalog,
        };
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
                .map(|summary| (summary, RuntimeReloadStatus::NotRunning))
            } else {
                crate::custom_providers::save_with_reload_status(draft)
            }
        });
        editor.task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |panel, cx| {
                match result {
                    Ok((summary, reload)) => {
                        let route_spec = summary.route_spec();
                        panel.input.update(cx, |input, _| input.load_configured_model_meta());
                        if let Some(state) = panel.login.as_mut() {
                            state.custom_api = None;
                            state.provider = None;
                            state.complete = true;
                            state.busy = false;
                            state.error = None;
                            state.custom_api_reload = Some(reload);
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
                .spawn(async move { crate::custom_providers::make_default_with_reload_status(&name) })
                .await;
            let _ = this.update(cx, |panel, cx| match result {
                Ok((spec, reload)) => {
                    if let Some(session_id) = target {
                        bridge.send(Command::RefreshRuntime {
                            session_id: session_id.clone(),
                        });
                        if let Some(model) = spec.or(fallback_spec) {
                            bridge.send(Command::SetModel { session_id, model });
                        }
                    }
                    if let Some(state) = panel.login.as_mut() {
                        state.custom_api_reload = Some(reload);
                        state.error = None;
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
                .spawn(async move { crate::custom_providers::remove_with_reload_status(&name) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                match result {
                    Ok(reload) => {
                        if let Some(state) = panel.login.as_mut() {
                            state.custom_api_reload = Some(reload);
                            state.error = None;
                        }
                    }
                    Err(error) => {
                        if let Some(state) = panel.login.as_mut() {
                            state.error = Some(error);
                            cx.notify();
                        }
                        return;
                    }
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
        if q.is_empty() || custom_api_keyword_matches(&q) {
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
                            || custom_api_keyword_matches(&q)
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
                                    .child(tr("Custom API endpoints", "自定义 API 端点")),
                            )
                            .when(!items.is_empty(), |el| {
                                el.child(pill_badge(items.len().to_string(), theme.TEXT_DIM))
                            }),
                    )
                    .child(
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
                            .child(tr("+ Add custom API", "+ 添加自定义 API"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_custom_api_editor(None, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .px_2()
                    .text_size(px(11.))
                    .text_color(theme.TEXT_DIM)
                    .child(tr("Connect any OpenAI-compatible third-party API (Base URL + API key + /models).", "连接兼容 OpenAI 的第三方 API，支持基础 URL、API 密钥和 /models。")),
            );

        for item in items {
            section = section.child(self.render_custom_api_row(item, cx));
        }
        Some(section.into_any_element())
    }

    fn render_custom_api_row(
        &self,
        item: CustomProviderSummary,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let name = item.name.clone();
        let row_id = format!("custom-api-row-{name}");
        let row_selector = row_id.clone();
        let model_count = item.models.len();
        let model_label = if is_zh() {
            format!("{model_count} 个模型")
        } else if model_count == 1 {
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
                    tr("API key", "API 密钥")
                } else {
                    tr("Local / no key", "本地 / 免密钥")
                },
                theme.TEXT_DIM,
            ));
        if item.is_default {
            header = header.child(pill_badge(tr("Default", "默认"), theme.ACCENT));
        }

        let use_item = item.clone();
        let use_id = format!("custom-api-use-{name}");
        let use_label = if item.is_default { tr("Use now", "立即使用") } else { tr("Set default", "设为默认") };
        let edit_item = item.clone();
        let edit_id = format!("custom-api-edit-{name}");
        let remove_name = item.name.clone();
        let rm_id = format!("custom-api-remove-{name}");
        let actions = div().flex().flex_none().items_center().gap_1p5()
            .child(
                action_pill(use_id.clone(), use_id, use_label, false)
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.activate_custom_provider(use_item.clone(), window, cx);
                    })),
            )
            .child(
                action_pill(edit_id.clone(), edit_id, tr("Edit", "编辑"), false)
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_custom_api_editor(Some(edit_item.clone()), cx);
                    })),
            )
            .child(
                action_pill(rm_id.clone(), rm_id, tr("Remove", "移除"), true)
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.delete_custom_provider(remove_name.clone(), cx);
                    })),
            );

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
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_custom_api_editor(Some(click_item.clone()), cx);
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
            Some(name) => format!("{} · {name}", tr("Edit custom API", "编辑自定义 API")),
            None => tr("Add custom OpenAI-compatible API", "添加兼容 OpenAI 的自定义 API").into(),
        };
        let api_key_entity = editor.api_key.clone();

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
                        login_button("custom-api-back", tr("Back", "返回")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_custom_api_editor(cx);
                            },
                        )),
                    ),
            )
            .child(field_block(
                tr("Base URL", "基础 URL"),
                tr("OpenAI-compatible root URL (https://.../v1, or http://127.0.0.1:PORT/v1 for local gateways).", "兼容 OpenAI 的基础地址。例如 https://…/v1，本地网关可用 http://127.0.0.1:PORT/v1。"),
                editor.base_url.clone().into_any_element(),
            ))
            .child(field_block(
                tr("API key", "API 密钥"),
                if editor.has_existing_key {
                    tr("Stored in Jcode's private env file. Leave blank to keep your existing key.", "密钥保存在 Jcode 的私有环境变量文件中。留空可保留原密钥。")
                } else {
                    tr("Stored in Jcode's private env file, never in plaintext config.toml or chat.", "密钥保存在 Jcode 的私有环境变量文件中，不会明文写入 config.toml 或聊天记录。")
                },
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(editor.api_key.clone()))
                    .child(
                        login_button("custom-api-paste-key", tr("Paste", "粘贴")).on_click(cx.listener(
                            move |_, _, window, cx| {
                                api_key_entity.update(cx, |input, cx| input.paste(window, cx));
                            },
                        )),
                    )
                    .into_any_element(),
            ))
            .child(field_block(
                tr("Endpoint name", "端点名称"),
                tr("Used as the route prefix in /model (for example siliconflow:deepseek-v3).", "用作 /model 中的线路前缀，例如 siliconflow:deepseek-v3。"),
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
                                    .child(tr("Default model", "默认模型")),
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
                                        tr("Fetching /models…", "正在拉取 /models…")
                                    } else {
                                        tr("Fetch /models", "拉取 /models")
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.fetch_custom_api_models(cx);
                                    })),
                            ),
                    )
                    .child(editor.default_model.clone())
                    .when(!editor.fetched_models.is_empty(), |el| {
                        el.child(self.render_custom_api_default_models(editor, cx))
                    }),
            )
            .child(field_block(
                tr("Extra models (optional)", "额外模型（可选）"),
                tr("Comma-separated model IDs exposed in /model alongside the default model.", "用逗号分隔模型 ID。这些模型与默认模型一起显示在 /model 中。"),
                editor.extra_models.clone().into_any_element(),
            ))
            .children(self.render_custom_model_settings(editor, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(toggle_pill(
                        "custom-api-toggle-default",
                        if editor.set_default {
                            tr("Default provider: On", "默认提供商：开启")
                        } else {
                            tr("Default provider: Off", "默认提供商：关闭")
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
                            tr("Sync /models catalog: On", "同步 /models 目录：开启")
                        } else {
                            tr("Sync /models catalog: Off", "同步 /models 目录：关闭")
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
                                tr("Saving…", "正在保存…")
                            } else {
                                tr("Save & connect", "保存并连接")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.submit_custom_api(cx);
                            })),
                    )
                    .child(
                        login_button("custom-api-cancel", tr("Cancel", "取消")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_custom_api_editor(cx);
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    fn render_custom_api_default_models(
        &self,
        editor: &CustomApiEditor,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        const COLLAPSED_LIMIT: usize = 8;
        let theme = Theme::global();
        let filter = &editor.default_model_filter.read(cx).content;
        let query = filter.trim().to_lowercase();
        let selected_default = editor.default_model.read(cx).content.trim();
        let mut visible: Vec<_> = editor
            .fetched_models
            .iter()
            .filter(|model| query.is_empty() || model.to_lowercase().contains(&query))
            .collect();
        let match_count = visible.len();
        let can_toggle = query.is_empty() && match_count > COLLAPSED_LIMIT;
        if can_toggle && !editor.models_expanded {
            let selected_outside = visible[COLLAPSED_LIMIT..]
                .iter()
                .copied()
                .find(|model| model.as_str() == selected_default);
            visible.truncate(COLLAPSED_LIMIT);
            if let Some(selected) = selected_outside
                && !visible.contains(&selected)
            {
                visible[COLLAPSED_LIMIT - 1] = selected;
            }
        }

        let mut model_chips = div().flex().flex_wrap().gap_1p5();
        for model in visible {
            let active = model == selected_default;
            let model_id = model.clone();
            let chip_id = format!("custom-api-model-{model_id}");
            let default_field = editor.default_model.clone();
            model_chips = model_chips.child(
                div()
                    .id(SharedString::from(chip_id.clone()))
                    .debug_selector(move || chip_id.clone())
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
        if can_toggle {
            let label = if editor.models_expanded {
                tr("Show fewer", "收起").to_owned()
            } else if is_zh() {
                format!("展开全部（{match_count}）")
            } else {
                format!("Show all ({match_count})")
            };
            let id = "custom-api-models-toggle";
            model_chips = model_chips.child(
                action_pill(id.into(), id.into(), label, false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(editor) = this.login.as_mut().and_then(|s| s.custom_api.as_mut()) {
                            editor.models_expanded = !editor.models_expanded;
                            cx.notify();
                        }
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .when(editor.fetched_models.len() > COLLAPSED_LIMIT || !filter.is_empty(), |el| {
                el.child(
                    div()
                        .id("custom-api-default-filter")
                        .debug_selector(|| "custom-api-default-filter".into())
                        .child(editor.default_model_filter.clone()),
                )
            })
            .child(if match_count == 0 {
                div()
                    .debug_selector(|| "custom-api-models-empty".into())
                    .text_size(px(11.))
                    .text_color(theme.TEXT_DIM)
                    .child(tr("No fetched models match the filter.", "没有匹配的已拉取模型。"))
                    .into_any_element()
            } else if !query.is_empty() || (can_toggle && editor.models_expanded) {
                div()
                    .id("custom-api-default-models-scroll")
                    .max_h(px(140.))
                    .overflow_y_scroll()
                    .child(model_chips)
                    .into_any_element()
            } else {
                model_chips.into_any_element()
            })
    }
}

fn custom_api_keyword_matches(query: &str) -> bool {
    "custom api openai compatible third party gateway".contains(query)
        || "自定义 api 端点 网关 兼容 第三方".contains(query)
}

#[cfg(test)]
#[test]
fn custom_api_search_matches_chinese_and_english_keywords() {
    for keyword in ["自定义", "端点", "网关", "兼容", "custom api", "gateway"] {
        assert!(custom_api_keyword_matches(keyword), "missing keyword {keyword}");
    }
    assert!(!custom_api_keyword_matches("unrelated"));
    assert!(!custom_api_keyword_matches("无关"));
}

#[cfg(test)]
#[gpui::test]
fn custom_api_default_models_expand_collapse_and_filter_beyond_twenty_four(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::input::bind_keys);
    let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
    let window = vcx.update(|window, _| window.window_handle());
    vcx.simulate_window_resize(window, gpui::size(px(1100.), px(820.)));
    panel.update(vcx, |panel, cx| {
        panel.open_custom_api_editor(None, cx);
        let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
        editor.fetched_models = (1..=30).map(|n| format!("model-{n:02}")).collect();
        editor.default_model.update(cx, |input, cx| input.set_content("model-30".into(), cx));
    });
    vcx.run_until_parked();

    let click = |vcx: &mut gpui::VisualTestContext, selector| {
        let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
        vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
    };
    let assert_selected = |vcx: &mut gpui::VisualTestContext, model| {
        panel.read_with(vcx, |panel, cx| {
            let editor = panel.login.as_ref().unwrap().custom_api.as_ref().unwrap();
            assert_eq!(editor.default_model.read(cx).content.as_ref(), model);
        });
    };

    assert!(vcx.debug_bounds("custom-api-model-model-07").is_some());
    assert!(vcx.debug_bounds("custom-api-model-model-08").is_none());
    assert!(vcx.debug_bounds("custom-api-model-model-25").is_none());
    assert!(vcx.debug_bounds("custom-api-model-model-30").is_some());
    click(vcx, "custom-api-models-toggle");
    click(vcx, "custom-api-model-model-25");
    assert_selected(vcx, "model-25");
    assert!(vcx.debug_bounds("custom-api-model-model-30").is_some());
    click(vcx, "custom-api-models-toggle");
    assert!(vcx.debug_bounds("custom-api-model-model-25").is_some());
    assert!(vcx.debug_bounds("custom-api-model-model-30").is_none());

    click(vcx, "custom-api-default-filter");
    vcx.simulate_input("MoDeL-29");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("custom-api-models-toggle").is_none());
    assert!(vcx.debug_bounds("custom-api-model-model-25").is_none());
    click(vcx, "custom-api-model-model-29");
    assert_selected(vcx, "model-29");

    click(vcx, "custom-api-default-filter");
    vcx.simulate_keystrokes("ctrl-a");
    vcx.simulate_input("missing");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("custom-api-models-empty").is_some());
    assert!(vcx.debug_bounds("custom-api-model-model-29").is_none());
    assert_selected(vcx, "model-29");
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
    label: impl Into<SharedString>,
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
        .child(label.into())
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
