//! AI service configuration. Credentials are edited in the masked login input.

use super::*;
use crate::custom_providers::{
    self, CatalogModelEntry, ConnectionTestReport, CustomApiFormat, CustomProviderDraft,
    CustomProviderSummary, RuntimeReloadStatus,
};
use crate::locale::{is_zh, tr};
use std::collections::HashSet;
#[path = "panel_custom_api_model_settings.rs"]
mod model_settings;
use model_settings::ModelSettingsEditor;

pub(super) struct CustomApiEditor {
    original: Option<CustomProviderSummary>,
    pub base_url: Entity<PromptInput>,
    pub api_key: Entity<LoginInput>,
    name: Entity<PromptInput>,
    format: CustomApiFormat,
    selected_models: Vec<String>,
    default_model: Option<String>,
    catalog: Vec<CatalogModelEntry>,
    catalog_source: (String, CustomApiFormat),
    catalog_filter: Entity<PromptInput>,
    settings_filter: Entity<PromptInput>,
    manual_model: Entity<PromptInput>,
    catalog_scroll: ScrollHandle,
    settings_scroll: ScrollHandle,
    model_settings: ModelSettingsEditor,
    set_default: bool,
    model_catalog: bool,
    supports_reasoning_effort: Option<bool>,
    advanced: bool,
    request: Option<CatalogRequest>,
    saving: bool,
    notice: Option<(String, bool)>,
    task: Option<Task<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CatalogRequest {
    Fetch,
    Test,
}

enum CatalogResponse {
    Fetched(String, Vec<CatalogModelEntry>, Option<String>),
    Tested(ConnectionTestReport),
}

#[derive(Clone, Copy)]
enum ServiceAction {
    Use,
    SetEnabled(bool),
    Remove,
}

pub(super) fn plain_field(
    placeholder: &'static str,
    initial: &str,
    cx: &mut Context<Panel>,
) -> Entity<PromptInput> {
    prompt_field(placeholder, initial, None, cx)
}

fn prompt_field(
    placeholder: &'static str,
    initial: &str,
    scroll: Option<ScrollHandle>,
    cx: &mut Context<Panel>,
) -> Entity<PromptInput> {
    let panel = cx.weak_entity();
    cx.new(|cx| {
        let mut field =
            PromptInput::new(cx, placeholder, |_, _, _, _| {}).without_command_completion();
        field.set_submission_enabled(false, cx);
        if !initial.is_empty() {
            field.set_content(initial.to_owned(), cx);
        }
        field.with_on_change(move |_, app| {
            if let Some(scroll) = &scroll {
                scroll.set_offset(point(px(0.), px(0.)));
            }
            let _ = panel.update(app, |_, cx| cx.notify());
        })
    })
}

impl CustomApiEditor {
    pub(super) fn add_model(&mut self, id: String) {
        if id.is_empty() || self.selected_models.contains(&id) {
            return;
        }
        if self.default_model.is_none() {
            self.default_model = Some(id.clone());
        }
        self.selected_models.push(id);
    }

    fn remove_models(&mut self, matches: impl Fn(&str) -> bool) {
        self.selected_models.retain(|id| !matches(id));
        if self.default_model.as_deref().is_some_and(&matches) {
            self.default_model = self.selected_models.first().cloned();
        }
        if self.model_settings.target.as_deref().is_some_and(matches) {
            self.model_settings.target = None;
        }
    }

    fn source(&self, cx: &App) -> (String, CustomApiFormat) {
        (
            self.base_url.read(cx).content.trim().to_owned(),
            self.format,
        )
    }

    fn visible_catalog(&self, cx: &App) -> impl Iterator<Item = &CatalogModelEntry> + '_ {
        let query = self.catalog_filter.read(cx).content.trim().to_lowercase();
        let current = self.source(cx) == self.catalog_source;
        self.catalog
            .iter()
            .filter(move |entry| current && entry.id.to_lowercase().contains(&query))
    }

    fn build_draft(&self, cx: &App) -> Result<CustomProviderDraft, (Option<String>, String)> {
        let default_model = self.default_model.clone().ok_or_else(|| {
            (
                None,
                tr(
                    "Select or add at least one model.",
                    "请勾选或添加至少一个模型。",
                )
                .to_owned(),
            )
        })?;
        let model_settings = self
            .model_settings
            .pending_edits(&self.selected_models, cx)
            .map_err(|(id, error)| (Some(id), error))?;
        let models = std::iter::once(default_model.clone())
            .chain(
                self.selected_models
                    .iter()
                    .filter(|id| **id != default_model)
                    .cloned(),
            )
            .collect();
        let source = self.source(cx);
        let cached_catalog = if source == self.catalog_source {
            Some(self.catalog.clone())
        } else if self.original.is_some() {
            // Discard metadata from the previous address or protocol on save.
            Some(Vec::new())
        } else {
            None
        };
        Ok(CustomProviderDraft {
            original_name: self.original.as_ref().map(|item| item.name.clone()),
            name: self.name.read(cx).content.to_string(),
            base_url: source.0,
            format: source.1,
            default_model,
            models,
            cached_catalog,
            model_settings,
            set_default: self.set_default,
            model_catalog: self.model_catalog,
            supports_reasoning_effort: self.supports_reasoning_effort,
            ..CustomProviderDraft::default()
        })
    }
}

impl Panel {
    fn custom_api_offline(&self) -> bool {
        self.preview_state.is_some() || cfg!(test) || crate::harness::screenshot_mode()
    }

    pub(super) fn load_custom_api_services(&mut self, cx: &mut Context<Self>) {
        let offline = self.custom_api_offline();
        let state = self.login.as_mut().unwrap();
        if offline {
            state.accounts.custom = custom_providers::offline_summaries();
            return;
        }
        state.services_loading = true;
        let work = cx
            .background_executor()
            .spawn(async { custom_providers::list() });
        state.services_task = Some(cx.spawn(async move |panel, cx| {
            let services = work.await;
            let _ = panel.update(cx, |panel, cx| {
                if let Some(state) = panel.login.as_mut() {
                    state.accounts.custom = services;
                    state.services_loading = false;
                    state.focus_pending = state.custom_api.is_none();
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn open_custom_api_services(&mut self, from_accounts: bool, cx: &mut Context<Self>) {
        if self.login.is_none() {
            self.open_login_picker_with_mode(true, cx);
        }
        if self.login_is_remote() {
            return;
        }
        let state = self.login.as_mut().unwrap();
        if !state.services_only {
            state.services_only = true;
            state.services_return_to_accounts = from_accounts;
            state.provider = None;
            state.complete = false;
            state.error = None;
            state.focus_pending = true;
            state.scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }

    pub(super) fn close_custom_api_services(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.login.as_mut() else {
            return;
        };
        if state.services_busy {
            return;
        }
        if state.services_return_to_accounts {
            state.services_only = false;
            state.services_return_to_accounts = false;
            state.error = None;
            state.custom_api_reload = None;
            state.focus_pending = true;
            state.scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        } else {
            self.close_login_picker(cx);
            if self.is_accounts_panel() {
                cx.emit(AccountsPanelClosed);
            } else {
                self.focus_input(window, cx);
            }
        }
    }

    pub(super) fn open_custom_api_editor(
        &mut self,
        existing: Option<CustomProviderSummary>,
        cx: &mut Context<Self>,
    ) {
        self.open_custom_api_services(true, cx);
        if self.login_is_remote()
            || self
                .login
                .as_ref()
                .is_some_and(|state| state.services_busy || state.custom_api.is_some())
        {
            return;
        }
        let offline = self.custom_api_offline();
        let profile = existing.as_ref().map_or("", |item| item.name.as_str());
        let address = existing.as_ref().map_or("", |item| item.base_url.as_str());
        let format = existing
            .as_ref()
            .map_or_else(CustomApiFormat::default, |item| item.format);
        let catalog = if profile.is_empty() {
            Vec::new()
        } else if offline {
            custom_providers::offline_catalog_models(profile)
        } else {
            custom_providers::cached_catalog(profile)
        };
        let default_model = existing
            .as_ref()
            .and_then(CustomProviderSummary::active_model)
            .map(str::to_owned);
        let mut selected_models = existing
            .as_ref()
            .map_or_else(Vec::new, |item| item.models.clone());
        if let Some(id) = default_model
            .as_ref()
            .filter(|id| !selected_models.contains(id))
        {
            selected_models.insert(0, id.clone());
        }
        let key_placeholder = if existing.as_ref().is_some_and(|item| item.has_api_key) {
            tr("Leave blank to keep the saved key", "留空保留已保存的密钥")
        } else {
            tr("Optional for local services", "本地服务可留空")
        };
        let supports_reasoning_effort = if offline || profile.is_empty() {
            None
        } else {
            jcode_base::config::Config::load()
                .providers
                .get(profile)
                .and_then(|provider| provider.supports_reasoning_effort)
        };
        let catalog_scroll = ScrollHandle::new();
        let settings_scroll = ScrollHandle::new();
        let panel = cx.weak_entity();
        let manual_model = cx.new(|cx| {
            PromptInput::new(
                cx,
                tr("Model ID, e.g. my-model-v2", "输入模型 ID，如 my-model-v2"),
                move |id, _, _, app| {
                    let panel = panel.clone();
                    app.defer(move |app| {
                        let _ =
                            panel.update(app, |this, cx| this.add_custom_api_manual_model(id, cx));
                    });
                },
            )
            .without_command_completion()
        });
        let editor = CustomApiEditor {
            base_url: plain_field("https://api.example.com/v1", address, cx),
            api_key: cx.new(|cx| LoginInput::new(cx, key_placeholder)),
            name: plain_field(tr("Auto-filled from URL", "根据地址自动填写"), profile, cx),
            format,
            selected_models,
            default_model,
            catalog,
            catalog_source: (address.to_owned(), format),
            catalog_filter: prompt_field(
                tr("Search model IDs…", "搜索模型 ID…"),
                "",
                Some(catalog_scroll.clone()),
                cx,
            ),
            settings_filter: prompt_field(
                tr("Search added models…", "搜索已添加模型…"),
                "",
                Some(settings_scroll.clone()),
                cx,
            ),
            manual_model,
            catalog_scroll,
            settings_scroll,
            model_settings: ModelSettingsEditor::new(profile, offline, cx),
            set_default: existing.as_ref().is_none_or(|item| item.is_default),
            model_catalog: existing.as_ref().is_some_and(|item| item.model_catalog),
            supports_reasoning_effort,
            original: existing,
            advanced: false,
            request: None,
            saving: false,
            notice: None,
            task: None,
        };
        let state = self.login.as_mut().unwrap();
        state.error = None;
        state.custom_api_reload = None;
        state.focus_pending = true;
        state.custom_api = Some(editor);
        cx.notify();
    }

    pub(super) fn close_custom_api_editor(&mut self, cx: &mut Context<Self>) {
        if self.custom_api_editor_mut().is_none() {
            return;
        }
        let state = self.login.as_mut().unwrap();
        if let Some(editor) = state.custom_api.take() {
            editor.api_key.update(cx, |input, cx| input.clear(cx));
        }
        state.focus_pending = true;
        cx.notify();
    }

    fn custom_api_editor_mut(&mut self) -> Option<&mut CustomApiEditor> {
        let state = self.login.as_mut()?;
        if state.services_busy {
            return None;
        }
        state.custom_api.as_mut().filter(|editor| !editor.saving)
    }

    fn add_custom_api_manual_model(&mut self, content: String, cx: &mut Context<Self>) {
        let Some(editor) = self.custom_api_editor_mut() else {
            return;
        };
        let id = content.trim();
        if id.is_empty() {
            return;
        }
        if id.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
            if editor.manual_model.read(cx).content.is_empty() {
                editor
                    .manual_model
                    .update(cx, |input, cx| input.set_content(content, cx));
            }
            editor.notice = Some((
                tr("Enter a single model ID.", "请输入单个模型 ID。").into(),
                true,
            ));
        } else {
            editor.add_model(id.to_owned());
            editor
                .manual_model
                .update(cx, |input, cx| input.set_content(String::new(), cx));
            editor.notice = None;
        }
        cx.notify();
    }

    fn request_custom_api_catalog(&mut self, request: CatalogRequest, cx: &mut Context<Self>) {
        let offline = self.custom_api_offline();
        let Some(editor) = self.custom_api_editor_mut() else {
            return;
        };
        if editor.request.is_some() {
            return;
        }
        if offline {
            editor.notice = Some((
                tr(
                    "Network requests are disabled in offline preview.",
                    "离线预览无法发起网络请求。",
                )
                .into(),
                true,
            ));
            cx.notify();
            return;
        }
        let source = editor.source(cx);
        let (address, format) = source.clone();
        let profile = editor
            .original
            .as_ref()
            .filter(|item| item.base_url == address)
            .map(|item| item.name.clone());
        let cache_profile = editor
            .original
            .as_ref()
            .filter(|item| item.base_url == address && item.format == format)
            .map(|item| item.name.clone());
        let secret = editor.api_key.update(cx, |input, cx| {
            let secret = input.take(cx);
            if !secret.is_empty() {
                input.set_secret(secret.clone(), cx);
            }
            secret
        });
        editor.request = Some(request);
        editor.notice = None;
        let work = cx.background_executor().spawn(async move {
            let key = (!secret.trim().is_empty()).then_some(secret.as_str());
            match request {
                CatalogRequest::Fetch => custom_providers::fetch_model_catalog_rich(
                    &address,
                    key,
                    profile.as_deref(),
                    format,
                )
                .map(|(base, entries)| {
                    let cache_error = cache_profile.as_ref().and_then(|name| {
                        custom_providers::save_cached_catalog(name, entries.clone()).err()
                    });
                    CatalogResponse::Fetched(base, entries, cache_error)
                }),
                CatalogRequest::Test => {
                    custom_providers::test_connection(&address, key, profile.as_deref(), format)
                        .map(CatalogResponse::Tested)
                }
            }
        });
        editor.task = Some(cx.spawn(async move |panel, cx| {
            let result = work.await;
            let _ = panel.update(cx, |panel, cx| {
                // Removing the service can overlap a read-only catalog request.
                let Some(editor) = panel
                    .login
                    .as_mut()
                    .and_then(|state| state.custom_api.as_mut())
                else {
                    return;
                };
                editor.request = None;
                if editor.source(cx) == source {
                    match result {
                        Ok(CatalogResponse::Fetched(base, entries, cache_error)) => {
                            let count = entries.len();
                            if editor.selected_models.is_empty()
                                && let Some(first) = entries.first()
                            {
                                editor.add_model(first.id.clone());
                            }
                            editor.catalog = entries;
                            editor.catalog_source = (base.clone(), format);
                            editor.catalog_scroll.set_offset(point(px(0.), px(0.)));
                            editor
                                .base_url
                                .update(cx, |input, cx| input.set_content(base, cx));
                            editor.notice = Some(if let Some(error) = cache_error {
                                (
                                    format!(
                                        "{} · {error}",
                                        tr("Could not cache the model list", "无法缓存模型列表")
                                    ),
                                    true,
                                )
                            } else {
                                (
                                    if is_zh() {
                                        format!("已获取 {count} 个模型。")
                                    } else {
                                        format!("Fetched {count} models.")
                                    },
                                    false,
                                )
                            });
                        }
                        Ok(CatalogResponse::Tested(report)) => {
                            let ConnectionTestReport {
                                working_base_url,
                                model_count,
                                latency_ms,
                            } = report;
                            if editor.catalog_source == source {
                                editor.catalog_source.0 = working_base_url.clone();
                            }
                            editor
                                .base_url
                                .update(cx, |input, cx| input.set_content(working_base_url, cx));
                            editor.notice = Some((
                                if is_zh() {
                                    format!(
                                        "连接成功 · 发现 {model_count} 个模型（{latency_ms} ms）"
                                    )
                                } else {
                                    format!(
                                        "Connected · {model_count} models found ({latency_ms} ms)"
                                    )
                                },
                                false,
                            ));
                        }
                        Err(error) => editor.notice = Some((error, true)),
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn submit_custom_api(&mut self, cx: &mut Context<Self>) {
        let preview = self.preview_state.is_some() || crate::harness::screenshot_mode();
        if self
            .login
            .as_ref()
            .is_some_and(|state| state.services_loading)
        {
            return;
        }
        let Some(editor) = self.custom_api_editor_mut() else {
            return;
        };
        if editor.request.is_some() {
            return;
        }
        let mut draft = match editor.build_draft(cx) {
            Ok(draft) => draft,
            Err((model, error)) => {
                if let Some(id) = model {
                    editor.model_settings.target = Some(id.clone());
                    editor
                        .settings_filter
                        .update(cx, |input, cx| input.set_content(id, cx));
                    editor.settings_scroll.set_offset(point(px(0.), px(0.)));
                }
                editor.notice = Some((error, true));
                cx.notify();
                return;
            }
        };
        if preview {
            editor.notice = Some((
                tr(
                    "Offline preview cannot save services.",
                    "离线预览无法保存服务。",
                )
                .into(),
                true,
            ));
            cx.notify();
            return;
        }
        let secret = editor.api_key.update(cx, |input, cx| input.take(cx));
        draft.api_key = (!secret.trim().is_empty()).then(|| secret.clone());
        let original = draft.original_name.clone();
        let field = editor.api_key.clone();
        editor.saving = true;
        editor.notice = None;
        let work = cx.background_executor().spawn(async move {
            #[cfg(test)]
            {
                let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
                custom_providers::save_at(
                    &temp.path().join("config.toml"),
                    draft,
                    &|_, _, _| Ok(()),
                    &|_, _| true,
                )
                .map(|summary| (summary, RuntimeReloadStatus::NotRunning))
            }
            #[cfg(not(test))]
            custom_providers::save_with_reload_status(draft)
        });
        editor.task = Some(cx.spawn(async move |panel, cx| {
            let result = work.await;
            let _ = panel.update(cx, |panel, cx| {
                let Some(state) = panel.login.as_mut() else {
                    return;
                };
                match result {
                    Ok((summary, reload)) => {
                        let route = if summary.is_default && summary.enabled {
                            summary.route_spec()
                        } else {
                            None
                        };
                        if summary.is_default {
                            for item in &mut state.accounts.custom {
                                item.is_default = false;
                            }
                        }
                        if let Some(item) = state.accounts.custom.iter_mut().find(|item| {
                            original.as_deref() == Some(item.name.as_str())
                                || item.name == summary.name
                        }) {
                            *item = summary;
                        } else {
                            state.accounts.custom.push(summary);
                        }
                        state.custom_api = None;
                        state.focus_pending = true;
                        state.custom_api_reload = Some(reload);
                        panel.reload_custom_api_session(route, cx);
                    }
                    Err(error) => {
                        if !secret.is_empty() {
                            field.update(cx, |input, cx| input.set_secret(secret, cx));
                        }
                        if let Some(editor) = state.custom_api.as_mut() {
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

    fn reload_custom_api_session(&mut self, route: Option<String>, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, _| input.load_configured_model_meta());
        if let Some(session_id) = self.login_target_session() {
            self.bridge.send(Command::RefreshRuntime {
                session_id: session_id.clone(),
            });
            if let Some(model) = route {
                self.bridge.send(Command::SetModel { session_id, model });
            }
        }
    }

    fn update_custom_api_service(
        &mut self,
        name: String,
        action: ServiceAction,
        cx: &mut Context<Self>,
    ) {
        let offline = self.custom_api_offline();
        let Some(state) = self.login.as_mut() else {
            return;
        };
        if state.services_busy
            || state.services_loading
            || state
                .custom_api
                .as_ref()
                .is_some_and(|editor| editor.saving || editor.request.is_some())
        {
            return;
        }
        state.error = None;
        state.custom_api_reload = None;
        if offline {
            match action {
                ServiceAction::Use => {
                    let mut route = None;
                    for item in &mut state.accounts.custom {
                        item.is_default = item.name == name;
                        if item.is_default {
                            route = item.route_spec();
                        }
                    }
                    if let (Some(session_id), Some(model)) = (self.login_target_session(), route) {
                        self.bridge.send(Command::SetModel { session_id, model });
                    }
                }
                ServiceAction::SetEnabled(enabled) => {
                    if let Some(item) = state
                        .accounts
                        .custom
                        .iter_mut()
                        .find(|item| item.name == name)
                    {
                        item.enabled = enabled;
                        item.is_default &= enabled;
                    }
                }
                ServiceAction::Remove => {
                    state.accounts.custom.retain(|item| item.name != name);
                    state.custom_api = None;
                    state.focus_pending = true;
                }
            }
            cx.notify();
            return;
        }
        state.services_busy = true;
        let work = cx.background_executor().spawn(async move {
            let (route, reload) = match action {
                ServiceAction::Use => custom_providers::make_default_with_reload_status(&name)?,
                ServiceAction::SetEnabled(enabled) => (
                    None,
                    custom_providers::set_enabled_with_reload_status(&name, enabled)?,
                ),
                ServiceAction::Remove => {
                    (None, custom_providers::remove_with_reload_status(&name)?)
                }
            };
            Ok::<_, String>((route, reload, custom_providers::list()))
        });
        state.services_task = Some(cx.spawn(async move |panel, cx| {
            let result = work.await;
            let _ = panel.update(cx, |panel, cx| {
                let Some(state) = panel.login.as_mut() else {
                    return;
                };
                state.services_busy = false;
                match result {
                    Ok((route, reload, items)) => {
                        state.accounts.custom = items;
                        state.custom_api_reload = Some(reload);
                        if matches!(action, ServiceAction::Remove) {
                            state.custom_api = None;
                            state.focus_pending = true;
                        }
                        panel.reload_custom_api_session(route, cx);
                    }
                    Err(error) => {
                        if let Some(editor) = state.custom_api.as_mut() {
                            editor.notice = Some((error, true));
                        } else {
                            state.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn custom_api_matches_query(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || service_keyword_matches(&query)
            || self.login.as_ref().is_some_and(|state| {
                state
                    .accounts
                    .custom
                    .iter()
                    .any(|item| service_matches(item, &query))
            })
    }

    pub(super) fn render_custom_api_section(
        &self,
        state: &LoginState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let count = state.accounts.custom.len();
        let enabled = state
            .accounts
            .custom
            .iter()
            .filter(|item| item.enabled)
            .count();
        let title = if is_zh() {
            format!("AI 服务 · 已配置 {count} 个（启用 {enabled} 个）")
        } else {
            format!("AI Services · {count} configured ({enabled} enabled)")
        };
        selector("login-group-custom-api")
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded_full()
            .bg(Theme::global().HEADER_BG)
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                action_pill(
                    "custom-api-manage",
                    tr("Manage AI services", "管理 AI 服务"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.open_custom_api_services(true, cx))),
            )
            .child(add_service_button(cx))
            .into_any_element()
    }

    pub(super) fn render_custom_api_services(
        &self,
        state: &LoginState,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let items = &state.accounts.custom;
        let query = state.services_search.read(cx).content.trim().to_lowercase();
        let mut rows = div().flex().flex_col().gap_2();
        let mut found = false;
        for item in items.iter().filter(|item| service_matches(item, &query)) {
            found = true;
            rows = rows.child(self.render_custom_api_row(item, state.services_busy, cx));
        }
        if !found {
            rows = rows.child(
                div()
                    .debug_selector(|| "custom-api-services-empty".into())
                    .p_4()
                    .rounded_xl()
                    .bg(Theme::global().HEADER_BG)
                    .child(if state.services_loading {
                        tr("Loading services…", "正在加载服务…")
                    } else if items.is_empty() {
                        tr(
                            "Add a service to configure its models.",
                            "添加服务后即可配置模型。",
                        )
                    } else {
                        tr("No services match your search.", "没有匹配的服务。")
                    }),
            );
        }
        div()
            .debug_selector(|| "custom-api-services".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .flex_none()
                    .child(heading(tr("AI Services", "AI 服务")))
                    .child(badge(items.len().to_string(), Theme::global().TEXT_DIM))
                    .child(
                        selector("custom-api-services-search")
                            .min_w(px(160.))
                            .max_w(px(240.))
                            .flex_1()
                            .child(state.services_search.clone()),
                    )
                    .child(div().flex_1())
                    .child(
                        add_service_button(cx)
                            .when(state.services_busy || state.services_loading, |el| {
                                el.opacity(0.6)
                            }),
                    )
                    .child(
                        action_pill(
                            "login-close",
                            if state.services_return_to_accounts {
                                tr("Back to accounts", "返回账户")
                            } else {
                                tr("Close", "关闭")
                            },
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_custom_api_services(window, cx);
                        })),
                    ),
            )
            .when_some(state.error.as_ref(), |el, error| {
                el.child(notice(error, true))
            })
            .when_some(state.custom_api_reload.as_ref(), |el, reload| {
                if matches!(reload, RuntimeReloadStatus::Deferred) {
                    el.child(div().text_size(px(12.)).text_color(Theme::global().WARN).child(tr(
                        "Configuration saved. The background service reload did not complete.",
                        "配置已保存。后台服务暂未完成重载。",
                    )))
                } else {
                    el.child(notice(tr("Configuration saved.", "配置已保存。"), false))
                }
            })
            .child(
                div()
                    .id("custom-api-services-scroll")
                    .track_scroll(&state.scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .into_any_element()
    }

    fn render_custom_api_row(
        &self,
        item: &CustomProviderSummary,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global();
        let name = &item.name;
        let count = item.models.len();
        let subtitle = if is_zh() {
            format!("{} · {count} 个模型", item.host_summary())
        } else {
            format!(
                "{} · {count} {}",
                item.host_summary(),
                if count == 1 { "model" } else { "models" }
            )
        };
        let edit = item.clone();
        let row = item.clone();
        let use_name = name.clone();
        let toggle_name = name.clone();
        let enabled = item.enabled;
        let switch_color = if enabled {
            theme.ACCENT
        } else {
            theme.TEXT_DIM
        };
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .when(busy, |el| el.opacity(0.6))
            .when(enabled, |el| {
                el.child(
                    action_pill(
                        format!("custom-api-use-{name}"),
                        if item.is_default {
                            tr("Use now", "立即使用")
                        } else {
                            tr("Set default", "设为默认")
                        },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.update_custom_api_service(use_name.clone(), ServiceAction::Use, cx);
                    })),
                )
            })
            .child(
                action_pill(format!("custom-api-edit-{name}"), tr("Edit", "编辑")).on_click(
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_custom_api_editor(Some(edit.clone()), cx);
                    }),
                ),
            )
            .child(
                selector(format!("custom-api-enable-toggle-{name}"))
                    .w(px(40.))
                    .h(px(22.))
                    .px(px(3.))
                    .flex()
                    .items_center()
                    .when(enabled, |el| el.justify_end())
                    .rounded_full()
                    .cursor_pointer()
                    .border_1()
                    .border_color(switch_color.opacity(0.65))
                    .bg(switch_color.opacity(0.24))
                    .child(div().size(px(14.)).rounded_full().bg(switch_color))
                    .tooltip(move |_, cx| {
                        cx.new(|_| {
                            super::super::usage::MeterTooltip(if enabled {
                                tr("Disable service", "禁用服务").into()
                            } else {
                                tr("Enable service", "启用服务").into()
                            })
                        })
                        .into()
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.update_custom_api_service(
                            toggle_name.clone(),
                            ServiceAction::SetEnabled(!enabled),
                            cx,
                        );
                    })),
            );
        selector(format!("custom-api-row-{name}"))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .min_h(px(52.))
            .rounded_full()
            .bg(theme.HEADER_BG)
            .cursor_pointer()
            .hover(|el| el.bg(theme.ACCENT_DIM))
            .child(
                div()
                    .size(px(32.))
                    .flex_none()
                    .rounded_full()
                    .bg(theme.PANEL_BG)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::accounts::lettermark(name)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(100.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(heading(name.clone()).min_w_0().truncate())
                            .when(!enabled, |el| {
                                el.child(badge(tr("Disabled", "已禁用"), theme.TEXT_DIM))
                            })
                            .when(enabled && item.is_default, |el| {
                                el.child(badge(tr("Default", "默认"), theme.ACCENT))
                            })
                            .when(item.format != CustomApiFormat::OpenAiCompatible, |el| {
                                el.child(badge(short_format(item.format), theme.TEXT_DIM))
                            }),
                    )
                    .child(secondary_text(subtitle).truncate()),
            )
            .child(actions)
            .on_click(
                cx.listener(move |this, _, _, cx| {
                    this.open_custom_api_editor(Some(row.clone()), cx)
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_custom_api_editor(
        &self,
        editor: &CustomApiEditor,
        width: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let busy = editor.saving
            || editor.request.is_some()
            || self.login.as_ref().is_some_and(|state| state.services_busy);
        let title = editor.original.as_ref().map_or_else(
            || tr("Add AI service", "添加 AI 服务").to_owned(),
            |item| format!("{} · {}", tr("Edit AI service", "编辑 AI 服务"), item.name),
        );
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .flex_none()
            .child(heading(title).flex_1().min_w(px(140.)).text_size(px(15.)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .child(toggle_pill(
                        "custom-api-advanced-toggle",
                        tr("Advanced settings", "高级设置"),
                        editor.advanced,
                        edit_action(cx, |editor| editor.advanced = !editor.advanced),
                    ))
                    .child(
                        action_pill(
                            "custom-api-test-conn",
                            if editor.request == Some(CatalogRequest::Test) {
                                tr("Testing…", "正在测试…")
                            } else {
                                tr("Test connection", "测试连接")
                            },
                        )
                        .when(busy, |el| el.opacity(0.6))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.request_custom_api_catalog(CatalogRequest::Test, cx)
                        })),
                    )
                    .child(
                        action_pill("custom-api-back", tr("Cancel", "取消")).on_click(
                            cx.listener(|this, _, _, cx| this.close_custom_api_editor(cx)),
                        ),
                    )
                    .child(
                        primary_pill(
                            "custom-api-save",
                            if editor.saving {
                                tr("Saving…", "正在保存…")
                            } else {
                                tr("Save service", "保存服务")
                            },
                        )
                        .when(busy, |el| el.opacity(0.6))
                        .on_click(cx.listener(|this, _, _, cx| this.submit_custom_api(cx))),
                    ),
            );
        let show_columns = width >= px(640.);
        let wide = width >= px(680.);
        let key = editor.api_key.clone();
        let key_field = div()
            .flex()
            .items_center()
            .gap_1p5()
            .child(div().flex_1().min_w_0().child(editor.api_key.clone()))
            .child(
                action_pill("custom-api-paste-key", tr("Paste", "粘贴")).on_click(cx.listener(
                    move |this, _, window, cx| {
                        if this.custom_api_editor_mut().is_some() {
                            key.update(cx, |input, cx| input.paste(window, cx));
                        }
                    },
                )),
            );
        let formats = div().flex().flex_wrap().gap_1p5().children(
            [
                (
                    "custom-api-format-openai",
                    CustomApiFormat::OpenAiCompatible,
                ),
                (
                    "custom-api-format-anthropic",
                    CustomApiFormat::AnthropicCompatible,
                ),
                ("custom-api-format-openrouter", CustomApiFormat::OpenRouter),
            ]
            .into_iter()
            .map(|(id, format)| {
                toggle_pill(
                    id,
                    short_format(format),
                    editor.format == format,
                    edit_action(cx, move |editor| {
                        editor.format = format;
                        editor.notice = None;
                    }),
                )
            }),
        );
        let basic = div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_none()
            .child(field_row(
                show_columns,
                field_block(tr("Name", "名称"), editor.name.clone()),
                field_block(tr("Base URL", "接口地址"), editor.base_url.clone()),
            ))
            .child(field_row(
                show_columns,
                field_block(tr("API key", "API 密钥"), key_field),
                field_block(tr("API format", "接口格式"), formats),
            ))
            .when(editor.advanced, |el| {
                el.child(self.render_custom_api_advanced(editor, cx))
            });
        let workbench = div()
            .flex()
            .gap_3()
            .min_h(px(280.))
            .when(wide, |el| el.flex_1())
            .when(!wide, |el| el.flex_col().flex_none())
            .child(self.render_custom_api_catalog(editor, wide, cx))
            .child(self.render_custom_api_selected(editor, wide, cx));
        div()
            .debug_selector(|| "custom-api-editor".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(toolbar)
            .when_some(editor.notice.as_ref(), |el, (text, error)| {
                el.child(notice(text, *error))
            })
            .child(
                div()
                    .id("custom-api-editor-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_h_full()
                            .gap_3()
                            .child(basic)
                            .child(workbench),
                    ),
            )
            .into_any_element()
    }

    fn render_custom_api_advanced(
        &self,
        editor: &CustomApiEditor,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .p_2()
            .rounded_xl()
            .bg(Theme::global().HEADER_BG)
            .child(toggle_pill(
                "custom-api-toggle-default",
                tr("Default provider", "默认提供商"),
                editor.set_default,
                edit_action(cx, |editor| editor.set_default = !editor.set_default),
            ))
            .child(toggle_pill(
                "custom-api-toggle-catalog",
                tr("Sync full /models catalog", "同步全部 /models 目录"),
                editor.model_catalog,
                edit_action(cx, |editor| editor.model_catalog = !editor.model_catalog),
            ))
            .child(toggle_pill(
                "custom-api-toggle-reasoning-effort",
                tr("Forward reasoning effort", "透传推理参数"),
                editor.supports_reasoning_effort == Some(true),
                edit_action(cx, |editor| {
                    editor.supports_reasoning_effort =
                        Some(editor.supports_reasoning_effort != Some(true));
                }),
            ))
            .when_some(editor.original.as_ref(), |el, item| {
                let name = item.name.clone();
                el.child(
                    action_pill(
                        format!("custom-api-remove-{name}"),
                        tr("Remove service", "移除服务"),
                    )
                    .text_color(Theme::global().ERROR)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_custom_api_service(name.clone(), ServiceAction::Remove, cx);
                    })),
                )
            })
    }

    fn render_custom_api_catalog(
        &self,
        editor: &CustomApiEditor,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = Theme::global();
        let visible: Vec<_> = editor.visible_catalog(cx).collect();
        let selected: HashSet<_> = editor.selected_models.iter().map(String::as_str).collect();
        let all_selected = !visible.is_empty()
            && visible
                .iter()
                .all(|entry| selected.contains(entry.id.as_str()));
        let mut rows = div().flex().flex_col().gap_1();
        if visible.is_empty() {
            rows = rows.child(
                secondary_text(
                    if editor.catalog.is_empty() || editor.source(cx) != editor.catalog_source {
                        tr(
                            "Fetch the model list to see available models.",
                            "获取列表后即可查看候选模型。",
                        )
                    } else {
                        tr("No models match your search.", "没有匹配的模型。")
                    },
                )
                .debug_selector(|| "custom-api-models-empty".into()),
            );
        }
        for entry in visible {
            let id = entry.id.clone();
            let checked = selected.contains(id.as_str());
            let summary = [entry.context_window, entry.max_output_tokens]
                .into_iter()
                .flatten()
                .map(compact_tokens)
                .collect::<Vec<_>>()
                .join(" · ");
            rows = rows.child(
                selector(format!("custom-api-catalog-row-{id}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_h(px(30.))
                    .px_2()
                    .py_1()
                    .rounded_full()
                    .cursor_pointer()
                    .bg(if checked {
                        theme.ACCENT_DIM
                    } else {
                        theme.PANEL_BG
                    })
                    .hover(|el| el.bg(theme.ACCENT.opacity(0.17)))
                    .child(
                        div()
                            .flex_none()
                            .size(px(18.))
                            .rounded_full()
                            .border_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_color(if checked {
                                theme.ACCENT
                            } else {
                                theme.PANEL_BORDER
                            })
                            .when(checked, |el| {
                                el.bg(theme.ACCENT).text_color(theme.PANEL_BG).child("✓")
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().child(id.clone()))
                            .when_some(
                                entry
                                    .display_name
                                    .as_ref()
                                    .filter(|name| *name != &entry.id),
                                |el, name| el.child(secondary_text(name.clone()).truncate()),
                            ),
                    )
                    .when(!summary.is_empty(), |el| {
                        el.child(secondary_text(summary).flex_none())
                    })
                    .on_click(edit_action(cx, move |editor| {
                        if editor.selected_models.contains(&id) {
                            editor.remove_models(|value| value == id);
                        } else {
                            editor.add_model(id.clone());
                        }
                    })),
            );
        }
        let select_all = toggle_pill(
            "custom-api-catalog-select-all",
            if all_selected {
                tr("Clear results", "清空筛选结果")
            } else {
                tr("Select results", "全选筛选结果")
            },
            all_selected,
            cx.listener(move |this, _, _, cx| {
                let Some(editor) = this.custom_api_editor_mut() else {
                    return;
                };
                let ids: Vec<_> = editor
                    .visible_catalog(cx)
                    .map(|entry| entry.id.clone())
                    .collect();
                if all_selected {
                    let ids: HashSet<_> = ids.into_iter().collect();
                    editor.remove_models(|id| ids.contains(id));
                } else {
                    for id in ids {
                        editor.add_model(id);
                    }
                }
                cx.notify();
            }),
        );
        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1p5()
            .flex_none()
            .child(select_all)
            .child(heading(tr("Service models", "该服务的模型")).flex_1())
            .child(
                action_pill(
                    "custom-api-fetch",
                    if editor.request == Some(CatalogRequest::Fetch) {
                        tr("Fetching…", "正在获取…")
                    } else {
                        tr("Fetch models", "获取列表")
                    },
                )
                .when(editor.request.is_some() || editor.saving, |el| {
                    el.opacity(0.6)
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.request_custom_api_catalog(CatalogRequest::Fetch, cx)
                })),
            )
            .child(input_selector(
                "custom-api-default-filter",
                editor.catalog_filter.clone(),
            ));
        model_pane("custom-api-catalog-pane", wide)
            .child(header)
            .child(
                div()
                    .id("custom-api-catalog-scroll")
                    .track_scroll(&editor.catalog_scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
    }

    fn render_custom_api_selected(
        &self,
        editor: &CustomApiEditor,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = Theme::global();
        let query = editor
            .settings_filter
            .read(cx)
            .content
            .trim()
            .to_lowercase();
        let catalog_matches = editor.source(cx) == editor.catalog_source;
        let mut rows = div().flex().flex_col().gap_1();
        let mut found = false;
        for id in &editor.selected_models {
            let alias = editor.model_settings.alias(id, cx);
            if !id.to_lowercase().contains(&query)
                && !alias.is_some_and(|alias| alias.to_lowercase().contains(&query))
            {
                continue;
            }
            found = true;
            let catalog = if catalog_matches {
                editor.catalog.iter().find(|entry| entry.id == *id)
            } else {
                None
            };
            let summary = editor.model_settings.summary(id, catalog, cx);
            let label = alias.map_or_else(|| id.clone(), |alias| format!("{id} · {alias}"));
            let expanded = editor.model_settings.target.as_ref() == Some(id);
            let default_id = id.clone();
            let advanced_id = id.clone();
            let remove_id = id.clone();
            let default = if editor.default_model.as_ref() == Some(id) {
                badge(tr("Default", "默认"), theme.ACCENT).into_any_element()
            } else {
                action_pill(
                    format!("custom-api-make-default-model-{id}"),
                    tr("Set default", "设为默认"),
                )
                .on_click(edit_action(cx, move |editor| {
                    editor.default_model = Some(default_id.clone())
                }))
                .into_any_element()
            };
            let advanced = toggle_pill(
                format!("custom-api-settings-target-{id}"),
                tr("Advanced", "高级"),
                expanded,
                cx.listener(move |this, _, _, cx| {
                    if let Some(editor) = this.custom_api_editor_mut() {
                        editor.model_settings.toggle(&advanced_id, cx);
                        cx.notify();
                    }
                }),
            );
            rows = rows.child(
                selector(format!("custom-api-selected-row-{id}"))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(expanded, |el| el.rounded_xl().bg(theme.PANEL_BG))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_3()
                            .py_1()
                            .rounded_xl()
                            .bg(theme.PANEL_BG)
                            .child(div().min_w_0().truncate().child(label))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_1p5()
                                    .child(secondary_text(summary).flex_1().min_w_0().truncate())
                                    .child(default)
                                    .child(advanced)
                                    .child(
                                        action_pill(
                                            format!("custom-api-selected-remove-{id}"),
                                            "×",
                                        )
                                        .text_color(theme.ERROR)
                                        .on_click(
                                            edit_action(cx, move |editor| {
                                                editor.remove_models(|id| id == remove_id)
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .children(self.render_custom_model_settings(editor, id, catalog, cx)),
            );
        }
        if !found {
            rows = rows.child(secondary_text(if editor.selected_models.is_empty() {
                tr(
                    "Select or add models to configure them.",
                    "勾选或添加模型后即可配置。",
                )
            } else {
                tr(
                    "No added models match your search.",
                    "已添加模型中没有匹配项。",
                )
            }));
        }
        let manual_model = editor.manual_model.clone();
        model_pane("custom-api-selected-pane", wide)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .flex_none()
                    .child(heading(tr("Model settings", "模型设置")))
                    .child(badge(
                        editor.selected_models.len().to_string(),
                        theme.TEXT_DIM,
                    ))
                    .child(input_selector(
                        "custom-api-settings-filter",
                        editor.settings_filter.clone(),
                    )),
            )
            .child(
                div()
                    .id("custom-api-selected-scroll")
                    .track_scroll(&editor.settings_scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .flex_none()
                    .child(secondary_text(tr("Custom", "自定义")))
                    .child(input_selector(
                        "custom-api-manual-model-input",
                        editor.manual_model.clone(),
                    ))
                    .child(
                        action_pill(
                            "custom-api-manual-model-add",
                            tr("+ Add custom model", "+ 添加自定义模型"),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let content = manual_model.read(cx).content.to_string();
                            this.add_custom_api_manual_model(content, cx);
                        })),
                    ),
            )
    }
}

fn service_matches(item: &CustomProviderSummary, query: &str) -> bool {
    query.is_empty()
        || item.name.to_lowercase().contains(query)
        || item.host_summary().to_lowercase().contains(query)
        || item
            .models
            .iter()
            .any(|id| id.to_lowercase().contains(query))
        || item
            .active_model()
            .is_some_and(|id| id.to_lowercase().contains(query))
}

fn service_keyword_matches(query: &str) -> bool {
    "ai services custom api openai compatible third party gateway".contains(query)
        || "ai 服务 自定义 api 端点 网关 兼容 第三方".contains(query)
}

fn short_format(format: CustomApiFormat) -> &'static str {
    match format {
        CustomApiFormat::OpenAiCompatible => "OpenAI",
        CustomApiFormat::AnthropicCompatible => "Anthropic",
        CustomApiFormat::OpenRouter => "OpenRouter",
    }
}

fn compact_tokens(tokens: usize) -> String {
    match tokens {
        1_000_000.. => format!("{}M", tokens as f64 / 1_000_000.),
        1_000.. => format!("{}K", tokens / 1_000),
        _ => tokens.to_string(),
    }
}

fn edit_action(
    cx: &Context<Panel>,
    update: impl Fn(&mut CustomApiEditor) + 'static,
) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, _, _, cx| {
        if let Some(editor) = this.custom_api_editor_mut() {
            update(editor);
            cx.notify();
        }
    })
}

fn field_row(wide: bool, first: gpui::Div, second: gpui::Div) -> gpui::Div {
    div()
        .flex()
        .gap_2()
        .when(!wide, |el| el.flex_col())
        .child(first.flex_1().min_w_0())
        .child(second.flex_1().min_w_0())
}

fn field_block(label: &'static str, input: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(heading(label).text_size(px(12.)))
        .child(input)
}

fn model_pane(id: &'static str, wide: bool) -> gpui::Stateful<gpui::Div> {
    selector(id)
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .rounded_xl()
        .bg(Theme::global().HEADER_BG)
        .text_size(px(12.))
        .min_w_0()
        .min_h(px(280.))
        .when(wide, |el| el.flex_1())
        .when(!wide, |el| el.flex_none().h(px(310.)))
}

fn input_selector(id: &'static str, input: Entity<PromptInput>) -> gpui::Stateful<gpui::Div> {
    selector(id).flex_1().min_w(px(130.)).child(input)
}

fn selector(id: impl Into<String>) -> gpui::Stateful<gpui::Div> {
    let id = id.into();
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
}

fn heading(label: impl Into<SharedString>) -> gpui::Div {
    div().font_weight(FontWeight::SEMIBOLD).child(label.into())
}

fn secondary_text(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(11.))
        .text_color(Theme::global().TEXT_DIM)
        .child(label.into())
}

fn notice(label: &str, error: bool) -> gpui::Div {
    let theme = Theme::global();
    div()
        .debug_selector(|| "custom-api-notice".into())
        .text_size(px(12.))
        .flex_none()
        .text_color(if error { theme.ERROR } else { theme.OK })
        .child(label.to_owned())
}

fn badge(label: impl Into<SharedString>, color: gpui::Rgba) -> gpui::Div {
    div()
        .flex_none()
        .px_2()
        .py(px(2.))
        .rounded_full()
        .bg(color.opacity(0.12))
        .text_size(px(11.))
        .text_color(color)
        .whitespace_nowrap()
        .child(label.into())
}

fn action_pill(id: impl Into<String>, label: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    selector(id)
        .flex_none()
        .px_2p5()
        .py_1()
        .rounded_full()
        .border_1()
        .border_color(theme.PANEL_BORDER)
        .text_size(px(11.))
        .text_color(theme.TEXT)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|el| el.bg(theme.ACCENT_DIM))
        .child(label.into())
}

fn primary_pill(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    selector(id)
        .flex_none()
        .px_3()
        .py_1()
        .rounded_full()
        .bg(theme.ACCENT)
        .text_color(theme.PANEL_BG)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|el| el.opacity(0.9))
        .child(label)
}

fn add_service_button(cx: &Context<Panel>) -> gpui::Stateful<gpui::Div> {
    primary_pill("custom-api-add", tr("+ Add service", "+ 添加服务"))
        .on_click(cx.listener(|this, _, _, cx| this.open_custom_api_editor(None, cx)))
}

fn toggle_pill(
    id: impl Into<String>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    action_pill(id, label)
        .when(active, |el| {
            el.bg(theme.ACCENT_DIM)
                .border_color(theme.ACCENT)
                .text_color(theme.ACCENT)
        })
        .on_click(on_click)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    #[test]
    fn custom_api_search_matches_chinese_and_english_keywords() {
        for keyword in ["AI 服务", "自定义", "网关", "custom api", "gateway"] {
            assert!(service_keyword_matches(&keyword.to_lowercase()));
        }
        assert!(!service_keyword_matches("unrelated"));
    }

    #[gpui::test]
    fn custom_api_services_toggle_and_return_to_accounts(cx: &mut gpui::TestAppContext) {
        cx.update(crate::input::bind_keys);
        let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
        panel.update(vcx, |panel, cx| assert!(panel.login_command("/api", cx)));
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("custom-api-services").is_some());
        assert!(vcx.debug_bounds("login-group-auto").is_none());
        assert!(vcx.debug_bounds("login-group-available").is_none());
        let name = "local-gateway";
        let toggle = "custom-api-enable-toggle-local-gateway";
        let use_button = "custom-api-use-local-gateway";
        click(vcx, toggle);
        assert!(vcx.debug_bounds("custom-api-editor").is_none());
        assert!(vcx.debug_bounds(use_button).is_none());
        panel.read_with(vcx, |panel, _| {
            let state = panel.login.as_ref().unwrap();
            assert!(
                !state
                    .accounts
                    .custom
                    .iter()
                    .find(|item| item.name == name)
                    .unwrap()
                    .enabled
            );
        });
        click(vcx, toggle);
        assert!(vcx.debug_bounds(use_button).is_some());
        click(vcx, "login-close");
        assert!(vcx.debug_bounds("login-dialog").is_none());
        panel.update(vcx, |panel, cx| assert!(panel.login_command("/login", cx)));
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("login-group-custom-api").is_some());
        assert!(vcx.debug_bounds("custom-api-row-local-gateway").is_none());
        click(vcx, "custom-api-manage");
        assert!(vcx.debug_bounds("custom-api-services").is_some());
        click(vcx, "login-close");
        assert!(vcx.debug_bounds("login-group-custom-api").is_some());
    }

    #[gpui::test]
    fn custom_api_workbench_filters_selects_adds_and_builds_draft(cx: &mut gpui::TestAppContext) {
        cx.update(crate::input::bind_keys);
        let (panel, vcx) = cx.add_window_view(|_, cx| Panel::new_preview(PreviewState::Empty, cx));
        let window = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(window, gpui::size(px(1200.), px(960.)));
        panel.update(vcx, |panel, cx| {
            assert!(panel.login_command("/login custom", cx));
            let editor = panel.login.as_mut().unwrap().custom_api.as_mut().unwrap();
            editor.catalog = (1..=30)
                .map(|n| CatalogModelEntry {
                    id: format!("model-{n:02}"),
                    context_window: Some(512_000),
                    max_output_tokens: Some(64_000),
                    ..CatalogModelEntry::default()
                })
                .collect();
        });
        vcx.run_until_parked();
        let catalog = vcx.debug_bounds("custom-api-catalog-pane").unwrap();
        let selected = vcx.debug_bounds("custom-api-selected-pane").unwrap();
        assert!(catalog.origin.x < selected.origin.x);
        assert_eq!(catalog.origin.y, selected.origin.y);
        click(vcx, "custom-api-default-filter");
        vcx.simulate_input("MoDeL-09");
        vcx.run_until_parked();
        click(vcx, "custom-api-catalog-row-model-09");
        assert!(
            vcx.debug_bounds("custom-api-selected-row-model-09")
                .is_some()
        );
        click(vcx, "custom-api-default-filter");
        vcx.simulate_keystrokes("ctrl-a");
        vcx.simulate_input("model-25");
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("custom-api-catalog-row-model-09")
                .is_none()
        );
        click(vcx, "custom-api-catalog-select-all");
        assert!(
            vcx.debug_bounds("custom-api-selected-row-model-25")
                .is_some()
        );
        click(vcx, "custom-api-catalog-select-all");
        assert!(
            vcx.debug_bounds("custom-api-selected-row-model-25")
                .is_none()
        );
        click(vcx, "custom-api-catalog-row-model-25");
        click(vcx, "custom-api-default-filter");
        vcx.simulate_keystrokes("ctrl-a");
        vcx.simulate_input("missing-model");
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("custom-api-catalog-row-model-25")
                .is_none()
        );
        assert!(
            vcx.debug_bounds("custom-api-selected-row-model-25")
                .is_some()
        );

        click(vcx, "custom-api-manual-model-input");
        vcx.simulate_input("fixture/my-model-v2");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("custom-api-selected-row-fixture/my-model-v2")
                .is_some()
        );
        click(vcx, "custom-api-make-default-model-model-25");
        click(vcx, "custom-api-settings-target-model-25");
        click(vcx, "custom-api-model-alias");
        vcx.simulate_input("fast");
        vcx.run_until_parked();
        click(vcx, "custom-api-context-preset-256k");
        click(vcx, "custom-api-output-preset-32k");
        click(vcx, "custom-api-reasoning-level-low");
        click(vcx, "custom-api-reasoning-level-high");
        click(vcx, "custom-api-effort-high");
        click(vcx, "custom-api-pdf-on");
        panel.read_with(vcx, |panel, cx| {
            let editor = panel.login.as_ref().unwrap().custom_api.as_ref().unwrap();
            assert!(editor.manual_model.read(cx).content.is_empty());
            let draft = editor.build_draft(cx).unwrap();
            assert!(!draft.model_catalog);
            assert_eq!(draft.default_model, "model-25");
            assert_eq!(
                draft.models,
                ["model-25", "model-09", "fixture/my-model-v2"]
            );
            assert_eq!(draft.cached_catalog.unwrap().len(), 30);
            let edit = &draft.model_settings["model-25"];
            assert_eq!(edit.alias.as_ref().unwrap().as_deref(), Some("fast"));
            assert_eq!(edit.context_window, Some(Some(256_000)));
            assert_eq!(edit.max_output_tokens, Some(Some(32_000)));
            assert_eq!(edit.reasoning, Some(Some(true)));
            assert_eq!(
                edit.reasoning_levels.as_ref().unwrap().as_deref().unwrap(),
                ["low", "high"]
            );
            assert_eq!(
                edit.reasoning_effort.as_ref().unwrap().as_deref(),
                Some("high")
            );
            assert_eq!(edit.pdf_input, Some(Some(true)));
        });
        click(vcx, "custom-api-settings-target-model-25");
        assert!(vcx.debug_bounds("custom-api-model-alias").is_none());
        click(vcx, "custom-api-selected-remove-model-25");
        panel.read_with(vcx, |panel, _| {
            let editor = panel.login.as_ref().unwrap().custom_api.as_ref().unwrap();
            assert_eq!(editor.default_model.as_deref(), Some("model-09"));
        });
        vcx.simulate_window_resize(window, gpui::size(px(600.), px(960.)));
        vcx.run_until_parked();
        let catalog = vcx.debug_bounds("custom-api-catalog-pane").unwrap();
        let selected = vcx.debug_bounds("custom-api-selected-pane").unwrap();
        assert_eq!(catalog.origin.x, selected.origin.x);
        assert!(catalog.origin.y < selected.origin.y);
    }
}
