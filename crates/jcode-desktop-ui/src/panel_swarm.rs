use super::*;
use super::effort_switch::{PillBounds, pill_bounds_marker};
use crate::locale::tr;
use jcode_sdk::api::RuntimeCommand;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum AgentRole {
    Swarm,
    Review,
    Judge,
    Memory,
}

impl AgentRole {
    const ALL: [Self; 4] = [Self::Swarm, Self::Review, Self::Judge, Self::Memory];

    fn key(self) -> &'static str {
        match self {
            Self::Swarm => "swarm",
            Self::Review => "review",
            Self::Judge => "judge",
            Self::Memory => "memory",
        }
    }

    pub(super) fn browser_title(self) -> &'static str {
        match self {
            Self::Swarm => tr("Choose worker model", "选择并发执行模型"),
            Self::Review => tr("Choose review model", "选择代码评审模型"),
            Self::Judge => tr("Choose judge model", "选择验收裁决模型"),
            Self::Memory => tr("Choose memory model", "选择记忆提炼模型"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Scope {
    #[default]
    Session,
    Global,
}

type Request = (RuntimeCommand, Vec<String>);

#[derive(Default)]
pub(super) struct State {
    pub(super) focus: Option<gpui::FocusHandle>,
    pill_bounds: PillBounds,
    enabled: Option<bool>,
    scope: Scope,
    overrides: HashMap<AgentRole, String>,
    defaults: HashMap<AgentRole, String>,
    subagent_model: Option<Option<String>>,
    autoreview: Option<bool>,
    autojudge: Option<bool>,
    feature_pending: bool,
    model_pending: bool,
    agents_pending: bool,
    autoreview_pending: bool,
    autojudge_pending: bool,
    notice: Option<String>,
    swarm_default_fallback: Option<String>,
    agents_defaults_loaded: bool,
}

pub(super) fn is_swarm_effort(level: &str) -> bool {
    matches!(level, "swarm" | "swarm-deep")
}

pub(super) fn effort_label(level: &str) -> &'static str {
    if level == "swarm-deep" {
        "Swarm Deep"
    } else {
        "Swarm"
    }
}

impl Panel {
    fn swarm_live(&self) -> bool {
        !self.is_side_document()
            && !self.is_pending_session()
            && self.preview_state.is_none()
            && !crate::harness::screenshot_mode()
    }

    fn toggle_swarm_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.swarm.focus.take().is_some() {
            self.focus_input(window, cx);
        } else {
            self.effort_picker = None;
            self.close_model_browser(cx);
            self.close_model_picker(cx);
            self.close_login_picker(cx);
            self.recovery_picker_open = false;
            self.swarm.notice = None;
            self.reopen_swarm_picker(window, cx);
            self.request_swarm(RuntimeCommand::Swarm, Vec::new(), cx);
            self.swarm.scope = Scope::Session;
            for command in [RuntimeCommand::SubagentModel, RuntimeCommand::Agents,
                RuntimeCommand::Autoreview, RuntimeCommand::Autojudge] {
                self.request_swarm(command, vec!["status".into()], cx);
            }
        }
        cx.notify();
    }

    pub(super) fn reopen_swarm_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.swarm.focus = Some(focus);
        cx.notify();
    }

    pub(super) fn select_agent_role_model(
        &mut self, role: AgentRole, spec: Option<String>, window: &mut Window, cx: &mut Context<Self>,
    ) {
        self.model_browser = None;
        self.reopen_swarm_picker(window, cx);
        self.set_role_model(role, spec, cx);
    }

    fn set_role_model(&mut self, role: AgentRole, spec: Option<String>, cx: &mut Context<Self>) {
        self.request_swarm_batch(self.swarm.role_requests(role, spec), cx);
    }

    fn reset_role_models(&mut self, cx: &mut Context<Self>) {
        let mut requests: Vec<_> = AgentRole::ALL.into_iter()
            .filter(|&role| self.swarm.can_reset_role(role))
            .flat_map(|role| self.swarm.role_requests(role, None))
            .collect();
        if !requests.iter().any(|(command, _)| *command == RuntimeCommand::SubagentModel) {
            requests.push((RuntimeCommand::SubagentModel, vec!["default".into()]));
        }
        self.request_swarm_batch(requests, cx);
    }

    fn request_swarm(&mut self, command: RuntimeCommand, args: Vec<String>, cx: &mut Context<Self>) {
        self.request_swarm_batch(vec![(command, args)], cx);
    }

    // Role mutations can return full state snapshots. Apply each reply before the next mutation.
    fn request_swarm_batch(&mut self, requests: Vec<Request>, cx: &mut Context<Self>) {
        if requests.is_empty() || !self.swarm_live()
            || requests.iter().any(|(command, _)| *self.swarm.pending(*command))
        {
            return;
        }
        for (command, _) in &requests {
            *self.swarm.pending(*command) = true;
        }
        let pending: Vec<_> = requests.iter().map(|(command, _)| *command).collect();
        self.swarm.notice = None;
        let session_id = self.session_id.clone();
        let bridge = self.bridge.clone();
        let send = move |(command, args): Request| {
            let (reply, result) = async_channel::bounded(1);
            bridge.send(Command::RuntimeCommandWithReply {
                session_id: session_id.clone(), command, args, reply,
            });
            result
        };
        let session_id = self.session_id.clone();
        let mut requests = requests.into_iter();
        let first = requests.next().expect("nonempty runtime requests");
        let mut result = send(first);
        cx.spawn(async move |this, cx| {
            let mut error = None;
            loop {
                match result.recv().await {
                    Ok(Ok(output)) => {
                        let _ = this.update(cx, |panel, _| {
                            if panel.session_id == session_id {
                                panel.sync_swarm_result(output.command, &output.data);
                            }
                        });
                    }
                    Ok(Err(message)) => { error.get_or_insert(message); }
                    Err(_) => {
                        error.get_or_insert_with(|| tr("The runtime did not return a response.", "运行时未返回结果。").into());
                    }
                }
                let Some(request) = requests.next() else { break };
                let still_current = this.update(cx, |panel, _| panel.session_id == session_id)
                    .unwrap_or(false);
                if !still_current {
                    break;
                }
                result = send(request);
            }
            let _ = this.update(cx, |panel, cx| {
                for command in pending {
                    *panel.swarm.pending(command) = false;
                }
                if panel.session_id == session_id && error.is_some() {
                    panel.swarm.notice = error;
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn sync_swarm_result(&mut self, command: RuntimeCommand, data: &Value) {
        match command {
            RuntimeCommand::Swarm => {
                if let Some(enabled) = data.get("enabled").and_then(Value::as_bool) {
                    self.swarm.enabled = Some(enabled);
                }
            }
            RuntimeCommand::SubagentModel => {
                if let Some(model) = data.get("model") {
                    self.swarm.subagent_model = Some(model.as_str().map(str::to_owned));
                }
                if let Some(default) = data.get("configured_default") {
                    self.swarm.swarm_default_fallback = default.as_str().map(str::to_owned);
                }
            }
            RuntimeCommand::Agents => {
                if let Some(overrides) = data.get("overrides").and_then(Value::as_object) {
                    self.swarm.overrides = role_models(overrides);
                }
                if let Some(defaults) = data.get("defaults").and_then(Value::as_object) {
                    self.swarm.defaults = role_models(defaults);
                    self.swarm.agents_defaults_loaded = true;
                }
            }
            RuntimeCommand::Autoreview | RuntimeCommand::Autojudge => {
                if let Some(enabled) = data.get("enabled").and_then(Value::as_bool) {
                    if command == RuntimeCommand::Autoreview {
                        self.swarm.autoreview = Some(enabled);
                    } else {
                        self.swarm.autojudge = Some(enabled);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn render_swarm_pill(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let active = self.reasoning_effort.as_deref().is_some_and(is_swarm_effort);
        if self.is_side_document() || self.is_pending_session()
            || ((self.preview_state.is_some() || crate::config::display().screenshot_mode)
                && !active && self.swarm.focus.is_none())
        {
            self.swarm.pill_bounds.set(None);
            return None;
        }
        let label = match self.reasoning_effort.as_deref() {
            Some("swarm") => tr("Swarm", "协作 · Swarm"),
            Some("swarm-deep") => tr("Swarm Deep", "协作 · 深度"),
            _ => match self.swarm.enabled {
                Some(true) => tr("Swarm · On", "协作 · 开"),
                Some(false) => tr("Swarm · Off", "协作 · 关"),
                None => tr("Swarm", "协作"),
            },
        };
        let label = format!("{label}{}", self.swarm.pill_summary());
        Some(super::composer::composer_pill("panel-swarm")
            .relative()
            .flex_shrink(6.)
            .min_w_0()
            .px(px(6.))
            .overflow_hidden()
            .when(active, |pill| pill.text_color(Theme::global().ACCENT))
            .child(div().min_w_0().truncate().child(label))
            .child(pill_bounds_marker(self.swarm.pill_bounds.clone()))
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_swarm_picker(window, cx);
                cx.stop_propagation();
            }))
            .into_any_element())
    }

    pub(super) fn render_swarm_picker(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let focus = self.swarm.focus.as_ref()?;
        let theme = Theme::global();
        let live = self.swarm_live();
        let current = self.reasoning_effort.as_deref().filter(|level| is_swarm_effort(level));
        let main = self.model.as_deref().map(|spec| {
            crate::input::model_menu::pretty_title(spec, self.input.read(cx).model_details())
        }).unwrap_or_else(|| tr("Unknown", "未查询").into());
        let feature = [
            ("swarm-feature-on", tr("On", "开启"), true),
            ("swarm-feature-off", tr("Off", "关闭"), false),
        ];
        let modes = [
            ("swarm-mode-standard", tr("Standard", "标准单代理"), None),
            ("swarm-mode-light", tr("Swarm", "Swarm 轻量并发"), Some("swarm")),
            ("swarm-mode-deep", tr("Swarm Deep", "Swarm Deep 深度任务图"), Some("swarm-deep")),
        ];
        let scope_buttons = [
            ("swarm-scope-session", tr("This chat", "本会话"), Scope::Session),
            ("swarm-scope-global", tr("Global default", "全局默认"), Scope::Global),
        ];
        let model_idle = !self.swarm.agents_pending && !self.swarm.model_pending;
        let reset_available = live && model_idle && AgentRole::ALL.into_iter().any(|role| self.swarm.can_reset_role(role));
        let header = div().flex().flex_wrap().items_center().justify_between().gap_1()
            .child(div().flex_1().flex_basis(px(250.)).min_w_0().flex().items_center().gap_2()
                .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child(tr("Task & role models", "多任务模型分工")))
                .child(div().min_w_0().truncate().text_color(theme.TEXT_DIM).child(format!("{} · {main}", tr("Main", "主控")))))
            .child(div().flex_none().max_w_full().flex().flex_wrap().gap_1()
                .children(scope_buttons.map(|(id, label, scope)| {
                    swarm_button(id, label, self.swarm.scope == scope, live && model_idle)
                        .when(live && model_idle, |button| button.on_click(cx.listener(move |this, _, _, cx| {
                            this.swarm.scope = scope;
                            cx.notify();
                        })))
                }))
                .child(swarm_button("swarm-reset-all", tr("Inherit all", "全部继承"), false, reset_available)
                    .when(reset_available, |button| button.on_click(cx.listener(|this, _, _, cx| this.reset_role_models(cx)))))
                .child(swarm_button("swarm-open-prompt", tr("Routing prompt", "路由提示词"), false, live)
                    .when(live, |button| button.on_click(cx.listener(|this, _, _, cx| {
                        this.handle_special_command("/swarm-prompt", cx);
                    })))));
        Some(div()
            .id("swarm-picker")
            .debug_selector(|| "swarm-picker".into())
            .track_focus(focus)
            .max_h(px(440.))
            .overflow_y_scroll()
            .flex().flex_col().gap_1().mb_2().p_2()
            .rounded_xl().bg(theme.HEADER_BG).text_color(theme.TEXT).text_size(px(12.))
            .child(div().flex().flex_wrap().items_center().gap_1()
                .child(tr("Swarm tool permission", "协作工具权限"))
                .children(feature.map(|(id, label, enabled)| {
                    let available = live && !self.swarm.feature_pending;
                    swarm_button(id, label, self.swarm.enabled == Some(enabled), available)
                        .when(available, |button| button.on_click(cx.listener(move |this, _, _, cx| {
                            let action = if enabled { "on" } else { "off" };
                            this.request_swarm(RuntimeCommand::Swarm, vec![action.into()], cx);
                        })))
                })))
            .child(div().flex().flex_wrap().items_center().gap_1()
                .child(tr("Orchestration mode", "编排模式"))
                .children(modes.map(|(id, label, mode)| {
                    swarm_button(id, label, current == mode, live)
                        .when(live, |button| button.on_click(cx.listener(move |this, _, _, cx| {
                            let current = this.reasoning_effort.as_deref().filter(|level| is_swarm_effort(level));
                            if current != mode {
                                let effort = mode.or(this.last_standard_effort.as_deref()).unwrap_or("high").to_owned();
                                this.send_effort(&effort, cx);
                            }
                        })))
                })))
            .child(header)
            .children(AgentRole::ALL.map(|role| self.render_agent_role(role, live, cx)))
            .when_some(self.swarm.notice.clone(), |menu, notice| menu.child(div().text_color(theme.WARN).child(notice)))
            // Keep the panel's MouseDown handler from moving focus back to the composer.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                // Let the pill's MouseUp toggle close an already-open picker.
                if this.swarm.pill_bounds.get().is_some_and(|bounds| bounds.contains(&event.position)) {
                    return;
                }
                this.swarm.focus = None;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.toggle_swarm_picker(window, cx);
                    cx.stop_propagation();
                }
            }))
            .into_any_element())
    }

    fn render_agent_role(&self, role: AgentRole, live: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::global();
        let (name, duty, choose, inherit) = match role {
            AgentRole::Swarm => (tr("Concurrent execution", "并发执行"), tr("Swarm & task graph", "协作与任务图"), "swarm-subagent-choose", "swarm-subagent-inherit"),
            AgentRole::Review => (tr("Code review", "代码评审"), "/review", "swarm-role-review-choose", "swarm-role-review-inherit"),
            AgentRole::Judge => (tr("Acceptance", "验收裁决"), "/judge", "swarm-role-judge-choose", "swarm-role-judge-inherit"),
            AgentRole::Memory => (tr("Memory", "记忆提炼"), tr("Background summaries", "后台摘要"), "swarm-role-memory-choose", "swarm-role-memory-inherit"),
        };
        let active = self.swarm.active_role_model(role);
        let details = self.input.read(cx).model_details();
        let model = active
            .map(|spec| crate::input::model_menu::pretty_title(spec, details))
            .unwrap_or_else(|| {
                if role == AgentRole::Swarm {
                    self.swarm.model_label().into()
                } else {
                    tr("Inherit main model", "继承主模型").into()
                }
            });
        let source = active.zip(self.swarm.model_source(role)).map(|(spec, (label, description))| {
            (label, format!("{model}\n{spec}\n{description}"))
        });
        let available = live && !self.swarm.agents_pending && (role != AgentRole::Swarm || !self.swarm.model_pending);
        let choose_available = available && self.has_model_browser_catalog(cx);
        let actions = div().flex_none().max_w_full().flex().flex_wrap().items_center().gap_1()
            .when_some(match role {
                AgentRole::Review => Some(("swarm-autoreview-toggle", tr("Auto review", "自动评审"), RuntimeCommand::Autoreview, self.swarm.autoreview, self.swarm.autoreview_pending)),
                AgentRole::Judge => Some(("swarm-autojudge-toggle", tr("Auto judge", "自动裁决"), RuntimeCommand::Autojudge, self.swarm.autojudge, self.swarm.autojudge_pending)),
                _ => None,
            }, |actions, (id, label, command, enabled, pending)| {
                let available = live && !pending;
                actions.child(swarm_button(id, label, enabled == Some(true), available)
                    .when(available, |button| button.on_click(cx.listener(move |this, _, _, cx| {
                        let action = if enabled == Some(true) { "off" } else { "on" };
                        this.request_swarm(command, vec![action.into()], cx);
                    }))))
            })
            .child(swarm_button(choose, tr("Choose model…", "选择模型…"), active.is_some(), choose_available)
                .when(choose_available, |button| button.on_click(cx.listener(move |this, _, _, cx| this.open_agent_role_browser(role, cx)))))
            .when(role == AgentRole::Swarm, |actions| {
                let available = available && self.model.is_some();
                actions.child(swarm_button("swarm-subagent-pin-current", tr("Use current model", "当前主模"), false, available)
                    .when(available, |button| button.on_click(cx.listener(|this, _, _, cx| {
                        if let Some(model) = this.model.clone() {
                            this.set_role_model(AgentRole::Swarm, Some(model), cx);
                        }
                    }))))
            })
            .child(swarm_button(inherit, tr("Inherit", "继承"), active.is_none(), available)
                .when(available, |button| button.on_click(cx.listener(move |this, _, _, cx| this.set_role_model(role, None, cx)))));
        div().flex().flex_wrap().items_center().justify_between().gap_1()
            .child(div().flex_1().flex_basis(px(150.)).min_w_0().flex().flex_col()
                .child(div().min_w_0().flex().items_center().gap_2()
                    .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(div().min_w_0().truncate().text_color(theme.TEXT_DIM).child(duty)))
                .child(div().id(role.key()).min_w_0().flex().items_center().gap_1()
                    .text_color(theme.TEXT_DIM)
                    .child(div().min_w_0().truncate().child(model))
                    .when_some(source, |display, (label, tooltip)| {
                        display
                            .child(div().flex_none().px_1().rounded_full().bg(theme.TOOL_BG)
                                .text_size(px(10.)).child(label))
                            .tooltip(move |_, cx| {
                                cx.new(|_| super::usage::MeterTooltip(tooltip.clone())).into()
                            })
                    })))
            .child(actions)
            .into_any_element()
    }
}

impl State {
    pub(super) fn active_model_override(&self) -> Option<&str> {
        self.active_role_model(AgentRole::Swarm)
    }

    pub(super) fn active_role_model(&self, role: AgentRole) -> Option<&str> {
        if let Some(model) = self.overrides.get(&role) {
            return (model != "inherit").then_some(model.as_str());
        }
        if role == AgentRole::Swarm && let Some(Some(model)) = &self.subagent_model {
            return Some(model);
        }
        self.default_model(role)
    }

    fn model_label(&self) -> &str {
        self.active_model_override().unwrap_or_else(|| {
            if self.subagent_model.is_some() || self.overrides.contains_key(&AgentRole::Swarm) || self.agents_defaults_loaded {
                tr("Inherit main model", "继承主模型")
            } else {
                tr("Unknown", "未查询")
            }
        })
    }

    fn model_source(&self, role: AgentRole) -> Option<(&'static str, &'static str)> {
        self.active_role_model(role)?;
        let session_override = self.overrides.get(&role).is_some_and(|model| model != "inherit")
            || (role == AgentRole::Swarm && matches!(&self.subagent_model, Some(Some(_))));
        if session_override {
            Some((tr("Session", "本会话"), tr("Session override", "会话级覆盖")))
        } else {
            self.default_model(role)
                .map(|_| (tr("Global", "全局"), tr("Global default", "全局默认")))
        }
    }

    fn pill_summary(&self) -> String {
        let role_count = AgentRole::ALL.into_iter()
            .filter(|&role| self.active_role_model(role).is_some())
            .count();
        match role_count {
            0 if self.autoreview == Some(true) || self.autojudge == Some(true) => {
                tr(" · Auto", " · 自动").into()
            }
            0 => String::new(),
            1 => format!(" · 1{}", tr(" role", "分工")),
            _ => format!(" · {role_count}{}", tr(" roles", "分工")),
        }
    }

    fn default_model(&self, role: AgentRole) -> Option<&str> {
        let fallback = role == AgentRole::Swarm;
        self.defaults.get(&role).map(String::as_str)
            .or_else(|| self.swarm_default_fallback.as_deref().filter(|_| fallback))
            .filter(|model| !model.is_empty() && !model.eq_ignore_ascii_case("inherit"))
    }

    fn can_reset_role(&self, role: AgentRole) -> bool {
        self.active_role_model(role).is_some() || (self.scope == Scope::Global
            && (self.default_model(role).is_some() || self.overrides.contains_key(&role)))
    }

    fn role_requests(&self, role: AgentRole, spec: Option<String>) -> Vec<Request> {
        let key = role.key();
        let reset = spec.is_none();
        let mut requests = Vec::new();
        match self.scope {
            Scope::Session => {
                let arg = spec.clone().unwrap_or_else(|| {
                    (if self.default_model(role).is_some() { "inherit" } else { "default" }).into()
                });
                requests.push((RuntimeCommand::Agents, vec![key.into(), arg]));
            }
            Scope::Global => {
                let arg = spec.clone().unwrap_or_else(|| "default".into());
                requests.push((RuntimeCommand::Agents, vec!["global".into(), key.into(), arg]));
                if reset && self.overrides.contains_key(&role) {
                    requests.push((RuntimeCommand::Agents, vec![key.into(), "default".into()]));
                }
            }
        }
        if role == AgentRole::Swarm {
            let arg = spec.filter(|_| self.scope == Scope::Session).unwrap_or_else(|| "default".into());
            requests.push((RuntimeCommand::SubagentModel, vec![arg]));
        }
        requests
    }

    fn pending(&mut self, command: RuntimeCommand) -> &mut bool {
        match command {
            RuntimeCommand::Swarm => &mut self.feature_pending,
            RuntimeCommand::SubagentModel => &mut self.model_pending,
            RuntimeCommand::Agents => &mut self.agents_pending,
            RuntimeCommand::Autoreview => &mut self.autoreview_pending,
            RuntimeCommand::Autojudge => &mut self.autojudge_pending,
            _ => unreachable!("unsupported collaboration command"),
        }
    }
}

fn role_models(data: &serde_json::Map<String, Value>) -> HashMap<AgentRole, String> {
    AgentRole::ALL.into_iter().filter_map(|role| {
        let model = data.get(role.key())?.as_str()?;
        Some((role, model.to_owned()))
    }).collect()
}

fn swarm_button(id: &'static str, label: &'static str, selected: bool, enabled: bool) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::global();
    div().id(id).debug_selector(move || id.into())
        .flex_none().whitespace_nowrap().px_2().py_1().rounded_full().bg(theme.TOOL_BG)
        .when(selected, |button| button.bg(theme.ACCENT_DIM).text_color(theme.ACCENT))
        .when(enabled, |button| button.cursor_pointer().hover(|button| button.bg(theme.ACCENT_DIM)))
        .when(!enabled, |button| button.text_color(theme.TEXT_FAINT))
        .child(label)
}

pub(super) fn runtime_summary(command: RuntimeCommand, data: &Value) -> Option<String> {
    let title = match command {
        RuntimeCommand::Swarm => tr("Swarm", "协作"),
        RuntimeCommand::SubagentModel => tr("Subagent model", "子代理模型"),
        RuntimeCommand::Agents => tr("Agent models", "代理模型"),
        RuntimeCommand::Memory => tr("Memory", "记忆"),
        RuntimeCommand::Autoreview => tr("Automatic review", "自动评审"),
        RuntimeCommand::Autojudge => tr("Automatic judge", "自动裁决"),
        RuntimeCommand::Fast => tr("Service tier", "服务档位"),
        _ => return None,
    };
    let object = data.as_object()?;
    let mut summary = format!("**{title}**\n\n");
    if object.is_empty() {
        summary.push_str(tr("No details returned.", "运行时未返回详细信息。"));
    }
    for (key, value) in object {
        summary.push_str(&format!("- **{}** · {}\n", summary_key(key), summary_value(value)));
    }
    Some(summary)
}

fn summary_key(key: &str) -> String {
    match key {
        "enabled" => tr("Enabled", "已开启"),
        "default_enabled" | "config_default" => tr("Default", "默认状态"),
        "swarm_id" => tr("Swarm ID", "协作 ID"),
        "role" => tr("Role", "角色"),
        "members" => tr("Members", "成员"),
        "model" => tr("Model", "模型"),
        "configured_default" => tr("Configured default", "默认配置"),
        "inherit_model" => tr("Main model", "主模型"),
        "scope" => tr("Scope", "作用范围"),
        "defaults" => tr("Defaults", "默认模型"),
        "overrides" => tr("Overrides", "会话覆盖"),
        "models" => tr("Available models", "可用模型"),
        "activity" => tr("Activity", "活动"),
        "pipeline_active" => tr("Pipeline active", "流程运行中"),
        "scheduled" => tr("Scheduled", "已安排"),
        "provider" => tr("Provider", "提供商"),
        "service_tier" => tr("Tier", "档位"),
        _ => return key.replace('_', " "),
    }.into()
}

fn summary_value(value: &Value) -> String {
    match value {
        Value::Bool(true) => tr("Yes", "是").into(),
        Value::Bool(false) => tr("No", "否").into(),
        Value::Null => "—".into(),
        Value::String(value) => value.split_whitespace().collect::<Vec<_>>().join(" "),
        Value::Number(value) => value.to_string(),
        Value::Array(values) if values.is_empty() => "—".into(),
        Value::Array(values) => values.iter().map(summary_value).collect::<Vec<_>>().join(", "),
        Value::Object(values) if values.is_empty() => "—".into(),
        Value::Object(values) => values.iter().map(|(key, value)| {
            format!("{} · {}", summary_key(key), summary_value(value))
        }).collect::<Vec<_>>().join(" / "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jcode_sdk::api::RuntimeCommandOutput;
    use serde_json::json;

    fn expect_runtime_reply(
        command: Command,
        expected: RuntimeCommand,
        expected_args: &[&str],
    ) -> crate::harness::RuntimeCommandReply {
        let Command::RuntimeCommandWithReply { session_id, command, args, reply } = command else {
            panic!("expected a runtime command with a direct reply");
        };
        assert_eq!(session_id, "swarm-test");
        assert_eq!(command, expected);
        assert_eq!(args, expected_args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>());
        reply
    }

    fn output(command: RuntimeCommand, data: Value) -> RuntimeCommandOutput {
        RuntimeCommandOutput {
            session_id: "swarm-test".into(),
            command,
            data,
            message: "raw runtime message".into(),
        }
    }

    fn reply_runtime(commands: &std::sync::mpsc::Receiver<Command>, command: RuntimeCommand, args: &[&str], data: Value) {
        expect_runtime_reply(commands.try_recv().unwrap(), command, args)
            .try_send(Ok(output(command, data))).unwrap();
    }

    fn reply_open_queries(commands: &std::sync::mpsc::Receiver<Command>, model_busy: bool) {
        let feature = expect_runtime_reply(commands.try_recv().unwrap(), RuntimeCommand::Swarm, &[]);
        let [model, agents, review, judge] = [RuntimeCommand::SubagentModel, RuntimeCommand::Agents,
            RuntimeCommand::Autoreview, RuntimeCommand::Autojudge].map(|command| {
            expect_runtime_reply(commands.try_recv().unwrap(), command, &["status"])
        });
        assert!(commands.try_recv().is_err());
        judge.try_send(Ok(output(RuntimeCommand::Autojudge, json!({"enabled": true})))).unwrap();
        agents.try_send(Ok(output(RuntimeCommand::Agents, json!({"defaults": {}, "overrides": {}})))).unwrap();
        review.try_send(Ok(output(RuntimeCommand::Autoreview, json!({"enabled": false})))).unwrap();
        feature.try_send(Ok(output(RuntimeCommand::Swarm, json!({"enabled": !model_busy})))).unwrap();
        let model_result = if model_busy { Err("Session busy".into()) } else {
            Ok(output(RuntimeCommand::SubagentModel, json!({"model": null})))
        };
        model.try_send(model_result).unwrap();
    }

    #[test]
    fn role_inheritance_respects_session_and_global_defaults() {
        let mut state = State::default();
        state.defaults.insert(AgentRole::Swarm, "worker-default".into());
        state.defaults.insert(AgentRole::Review, "review-default".into());
        state.overrides.insert(AgentRole::Swarm, "inherit".into());
        state.subagent_model = Some(Some("legacy-worker".into()));
        assert_eq!(state.active_role_model(AgentRole::Swarm), None);
        assert_eq!(state.active_role_model(AgentRole::Review), Some("review-default"));
        assert_eq!(state.role_requests(AgentRole::Swarm, None), vec![
            (RuntimeCommand::Agents, vec!["swarm".to_owned(), "inherit".to_owned()]),
            (RuntimeCommand::SubagentModel, vec!["default".to_owned()]),
        ]);
        state.scope = Scope::Global;
        assert_eq!(state.role_requests(AgentRole::Swarm, None), vec![
            (RuntimeCommand::Agents, vec!["global".to_owned(), "swarm".to_owned(), "default".to_owned()]),
            (RuntimeCommand::Agents, vec!["swarm".to_owned(), "default".to_owned()]),
            (RuntimeCommand::SubagentModel, vec!["default".to_owned()]),
        ]);
    }

    #[test]
    fn swarm_source_and_pill_summary_are_localized() {
        let mut state = State::default();
        assert_eq!(state.pill_summary(), "");
        crate::locale::with_zh(|| assert_eq!(state.pill_summary(), ""));
        state.autoreview = Some(true);
        assert_eq!(state.pill_summary(), " · Auto");
        crate::locale::with_zh(|| assert_eq!(state.pill_summary(), " · 自动"));
        state.autoreview = Some(false);
        state.autojudge = Some(true);
        assert_eq!(state.pill_summary(), " · Auto");

        state.defaults.insert(AgentRole::Review, "global-review".into());
        assert_eq!(state.model_source(AgentRole::Review), Some(("Global", "Global default")));
        assert_eq!(state.pill_summary(), " · 1 role");
        crate::locale::with_zh(|| {
            assert_eq!(state.model_source(AgentRole::Review), Some(("全局", "全局默认")));
            assert_eq!(state.pill_summary(), " · 1分工");
        });

        state.overrides.insert(AgentRole::Review, "session-review".into());
        state.swarm_default_fallback = Some("global-worker".into());
        assert_eq!(state.model_source(AgentRole::Swarm), Some(("Global", "Global default")));
        state.subagent_model = Some(Some("legacy-worker".into()));
        for role in [AgentRole::Review, AgentRole::Swarm] {
            assert_eq!(state.model_source(role), Some(("Session", "Session override")));
            crate::locale::with_zh(|| {
                assert_eq!(state.model_source(role), Some(("本会话", "会话级覆盖")));
            });
        }
        assert_eq!(state.pill_summary(), " · 2 roles");
        crate::locale::with_zh(|| assert_eq!(state.pill_summary(), " · 2分工"));
        state.overrides.insert(AgentRole::Swarm, "inherit".into());
        assert_eq!(state.model_source(AgentRole::Swarm), None);
        assert_eq!(state.pill_summary(), " · 1 role");
    }

    struct NarrowPanelView(Entity<Panel>);

    impl Render for NarrowPanelView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .child(div().w(px(420.)).h_full().flex_none().child(self.0.clone()))
                .child(div().flex_1().h_full().debug_selector(|| "blank-beside-panel".into()))
        }
    }

    fn narrow_panel_fixture(cx: &mut gpui::TestAppContext) -> (
        Entity<Panel>,
        &mut gpui::VisualTestContext,
        std::sync::mpsc::Receiver<Command>,
    ) {
        cx.update(crate::input::bind_keys);
        let (bridge, commands) = crate::harness::spawn_recording();
        let (view, vcx) = cx.add_window_view(|_, cx| {
            NarrowPanelView(cx.new(|cx| Panel::new("swarm-test".into(), None, None, bridge, cx)))
        });
        let panel = view.read_with(vcx, |view, _| view.0.clone());
        let handle = vcx.update(|window, cx| {
            Panel::connect_input(&panel, cx);
            window.window_handle()
        });
        vcx.simulate_window_resize(handle, gpui::size(px(1100.), px(820.)));
        panel.update(vcx, |panel, cx| {
            panel.model = Some("gpt-5.6-sol".into());
            panel.provider = Some("openai".into());
            panel.identity_effort(Some("high".into()));
            cx.notify();
        });
        vcx.run_until_parked();
        (panel, vcx, commands)
    }

    #[gpui::test]
    fn swarm_and_effort_pickers_close_from_their_pill_or_outside(cx: &mut gpui::TestAppContext) {
        let (panel, vcx, _commands) = narrow_panel_fixture(cx);
        for (pill, picker) in [("panel-swarm", "swarm-picker"), ("panel-model-effort", "effort-picker")] {
            let click_pill = |vcx: &mut gpui::VisualTestContext| {
                let point = vcx.debug_bounds(pill).expect("trigger pill").center();
                vcx.simulate_click(point, gpui::Modifiers::default());
                vcx.run_until_parked();
            };

            click_pill(vcx);
            assert!(vcx.debug_bounds(picker).is_some());
            let point = vcx.debug_bounds(pill).expect("trigger pill").center();
            vcx.simulate_mouse_down(point, gpui::MouseButton::Left, Default::default());
            vcx.run_until_parked();
            assert!(vcx.debug_bounds(picker).is_some(), "still open on pill MouseDown");
            vcx.simulate_mouse_up(point, gpui::MouseButton::Left, Default::default());
            vcx.run_until_parked();
            assert!(vcx.debug_bounds(picker).is_none(), "closed on pill MouseUp");
            vcx.update(|window, cx| assert!(panel.read(cx).input_focus_handle(cx).is_focused(window)));

            click_pill(vcx);
            let bounds = vcx.debug_bounds(picker).expect("reopened picker");
            let other_focus = vcx.update(|window, cx| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                focus
            });
            let outside = vcx.debug_bounds("blank-beside-panel").expect("blank surface").center();
            assert!(!bounds.contains(&outside));
            vcx.simulate_mouse_down(outside, gpui::MouseButton::Left, Default::default());
            vcx.run_until_parked();
            assert!(vcx.debug_bounds(picker).is_none(), "closed on outside MouseDown");
            vcx.update(|window, _| assert!(other_focus.is_focused(window)));
            vcx.simulate_mouse_up(outside, gpui::MouseButton::Left, Default::default());
        }
    }

    #[gpui::test]
    fn long_routes_keep_all_swarm_role_actions_inside_the_picker(cx: &mut gpui::TestAppContext) {
        let (panel, vcx, _commands) = narrow_panel_fixture(cx);
        let long_route = format!("magpie:fixture/{}", "very-long-model-route-".repeat(10));
        vcx.update(|window, cx| {
            panel.update(cx, |panel, cx| {
                panel.model = Some(long_route.clone());
                panel.available_models = vec![long_route.clone()];
                for role in AgentRole::ALL {
                    panel.swarm.defaults.insert(role, long_route.clone());
                }
                panel.swarm.autoreview = Some(true);
                panel.reopen_swarm_picker(window, cx);
            });
        });
        vcx.run_until_parked();
        let picker = vcx.debug_bounds("swarm-picker").expect("Swarm picker");
        assert!(picker.size.width <= px(420.));
        for selector in [
            "swarm-subagent-choose", "swarm-subagent-inherit",
            "swarm-role-review-choose", "swarm-role-review-inherit",
            "swarm-role-judge-choose", "swarm-role-judge-inherit",
            "swarm-role-memory-choose", "swarm-role-memory-inherit",
        ] {
            let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
            assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.), "{selector} has layout");
            assert!(
                bounds.left() >= picker.left() && bounds.right() <= picker.right()
                    && bounds.top() >= picker.top() && bounds.bottom() <= picker.bottom(),
                "{selector} must stay within the picker: {bounds:?} vs {picker:?}"
            );
        }
    }

    #[gpui::test]
    fn swarm_picker_chooses_subagent_model_directly_without_changing_main_model(
        cx: &mut gpui::TestAppContext,
    ) {
        const MAIN: &str = "magpie:codex/gpt-6.2-sol";
        const SUBAGENT: &str = "magpie:workbuddy/deepseek-v4.1-flash";
        const JUDGE: &str = "magpie:cpr-fast/gpt-6.2-sol";
        const MEMORY: &str = "magpie:truesota/deepseek-flash";
        cx.update(crate::input::bind_keys);
        let (bridge, commands) = crate::harness::spawn_recording();
        let (panel, vcx) =
            cx.add_window_view(|_, cx| Panel::new("swarm-test".into(), None, None, bridge, cx));
        vcx.update(|_, cx| Panel::connect_input(&panel, cx));
        let handle = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(handle, gpui::size(px(1100.), px(820.)));
        panel.update(vcx, |panel, cx| {
            panel.model = Some(MAIN.into());
            panel.provider = Some("magpie".into());
            panel.available_models = [MAIN, SUBAGENT, JUDGE, MEMORY].map(str::to_owned).to_vec();
            let routes = [MAIN, SUBAGENT, JUDGE, MEMORY].map(|spec| {
                let model = spec.strip_prefix("magpie:").unwrap();
                jcode_sdk::ModelRouteInfo {
                    model: model.into(),
                    provider: "magpie".into(),
                    api_method: "openai-compatible:magpie".into(),
                    available: true,
                    detail: String::new(),
                    usage: None,
                }
            });
            panel.input.update(cx, |input, cx| {
                input.set_remote_model_routes(Vec::new(), &routes, Some(MAIN.into()), cx);
            });
            cx.notify();
        });
        vcx.run_until_parked();
        while commands.try_recv().is_ok() {}
        let before = panel.read_with(vcx, |panel, _| panel.items.len());
        let click = |vcx: &mut gpui::VisualTestContext, selector: &'static str| {
            let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
            vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
            vcx.run_until_parked();
        };

        click(vcx, "panel-swarm");
        reply_open_queries(&commands, false);
        vcx.run_until_parked();

        click(vcx, "swarm-subagent-choose");
        assert!(vcx.debug_bounds("model-browser").is_some());
        vcx.simulate_input("deepseek-v4.1-flash");
        vcx.run_until_parked();
        click(vcx, "model-browser-row-0");
        reply_runtime(&commands, RuntimeCommand::Agents, &["swarm", SUBAGENT],
            json!({"overrides": {"swarm": SUBAGENT}}));
        vcx.run_until_parked();
        let model = expect_runtime_reply(commands.try_recv().unwrap(), RuntimeCommand::SubagentModel, &[SUBAGENT]);
        model.try_send(Ok(output(RuntimeCommand::SubagentModel, json!({"model": SUBAGENT})))).unwrap();
        vcx.run_until_parked();
        assert!(commands.try_recv().is_err());
        assert!(vcx.debug_bounds("model-browser").is_none());
        assert!(vcx.debug_bounds("swarm-picker").is_some());
        panel.read_with(vcx, |panel, _| {
            assert_eq!(panel.model.as_deref(), Some(MAIN));
            assert_eq!(panel.swarm.model_label(), SUBAGENT);
            assert_eq!(panel.items.len(), before);
        });

        click(vcx, "swarm-subagent-choose");
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser").is_none());
        assert!(vcx.debug_bounds("swarm-picker").is_some());
        assert!(commands.try_recv().is_err());

        click(vcx, "swarm-subagent-choose");
        click(vcx, "model-browser-subagent-inherit");
        reply_runtime(&commands, RuntimeCommand::Agents, &["swarm", "default"], json!({"overrides": {}}));
        vcx.run_until_parked();
        let model = expect_runtime_reply(commands.try_recv().unwrap(), RuntimeCommand::SubagentModel, &["default"]);
        model.try_send(Ok(output(RuntimeCommand::SubagentModel, json!({"model": null})))).unwrap();
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("model-browser").is_none());
        assert!(vcx.debug_bounds("swarm-picker").is_some());
        panel.read_with(vcx, |panel, _| {
            assert_eq!(panel.model.as_deref(), Some(MAIN));
            assert_eq!(panel.swarm.active_model_override(), None);
        });
        assert!(commands.try_recv().is_err());

        let mut overrides = serde_json::Map::new();
        for (role, selector, spec, query) in [
            (AgentRole::Review, "swarm-role-review-choose", MAIN, "codex/gpt-6.2-sol"),
            (AgentRole::Judge, "swarm-role-judge-choose", JUDGE, "cpr-fast/gpt-6.2-sol"),
            (AgentRole::Memory, "swarm-role-memory-choose", MEMORY, "truesota/deepseek-flash"),
        ] {
            click(vcx, selector);
            assert!(vcx.debug_bounds("model-browser").is_some());
            vcx.simulate_input(query);
            vcx.run_until_parked();
            click(vcx, "model-browser-row-0");
            overrides.insert(role.key().into(), json!(spec));
            reply_runtime(&commands, RuntimeCommand::Agents, &[role.key(), spec],
                json!({"overrides": &overrides}));
            vcx.run_until_parked();
            assert!(commands.try_recv().is_err());
            assert!(vcx.debug_bounds("swarm-picker").is_some());
            panel.read_with(vcx, |panel, _| {
                assert_eq!(panel.swarm.active_role_model(role), Some(spec));
                assert_eq!(panel.model.as_deref(), Some(MAIN));
            });
        }

        click(vcx, "swarm-autoreview-toggle");
        reply_runtime(&commands, RuntimeCommand::Autoreview, &["on"], json!({"enabled": true}));
        click(vcx, "swarm-autojudge-toggle");
        reply_runtime(&commands, RuntimeCommand::Autojudge, &["off"], json!({"enabled": false}));
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| {
            assert_eq!(panel.swarm.autoreview, Some(true));
            assert_eq!(panel.swarm.autojudge, Some(false));
        });

        click(vcx, "swarm-scope-global");
        click(vcx, "swarm-subagent-choose");
        vcx.simulate_input("deepseek-v4.1-flash");
        vcx.run_until_parked();
        click(vcx, "model-browser-row-0");
        reply_runtime(&commands, RuntimeCommand::Agents, &["global", "swarm", SUBAGENT],
            json!({"defaults": {"swarm": SUBAGENT}, "overrides": &overrides}));
        vcx.run_until_parked();
        reply_runtime(&commands, RuntimeCommand::SubagentModel, &["default"],
            json!({"model": null, "configured_default": SUBAGENT}));
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| assert_eq!(panel.swarm.active_model_override(), Some(SUBAGENT)));

        click(vcx, "swarm-reset-all");
        for role in AgentRole::ALL {
            reply_runtime(&commands, RuntimeCommand::Agents, &["global", role.key(), "default"],
                json!({"defaults": {}, "overrides": &overrides}));
            vcx.run_until_parked();
            if overrides.remove(role.key()).is_some() {
                reply_runtime(&commands, RuntimeCommand::Agents, &[role.key(), "default"],
                    json!({"overrides": &overrides}));
            } else if role == AgentRole::Swarm {
                reply_runtime(&commands, RuntimeCommand::SubagentModel, &["default"],
                    json!({"model": null, "configured_default": null}));
            }
            vcx.run_until_parked();
        }
        assert!(commands.try_recv().is_err());
        click(vcx, "swarm-scope-session");
        panel.read_with(vcx, |panel, _| {
            assert_eq!(panel.swarm.scope, Scope::Session);
            assert!(AgentRole::ALL.into_iter().all(|role| panel.swarm.active_role_model(role).is_none()));
            assert_eq!(panel.model.as_deref(), Some(MAIN));
            assert_eq!(panel.items.len(), before);
        });
    }

    #[gpui::test]
    fn swarm_picker_dispatches_native_commands_and_keeps_queries_out_of_chat(cx: &mut gpui::TestAppContext) {
        cx.update(crate::input::bind_keys);
        let (bridge, commands) = crate::harness::spawn_recording();
        let (workspace, vcx) = cx.add_window_view(|_, cx| {
            let mut workspace = crate::workspace::Workspace::for_test(crate::learning::Coach::new(), cx);
            workspace.set_test_bridge(bridge);
            workspace.push_test_panel("swarm-test", cx);
            workspace
        });
        let mut panel = None;
        workspace.update(vcx, |workspace, _| panel = workspace.test_panel(0));
        let panel = panel.unwrap();
        let handle = vcx.update(|window, _| window.window_handle());
        vcx.simulate_window_resize(handle, gpui::size(px(1100.), px(820.)));
        panel.update(vcx, |panel, cx| {
            panel.model = Some("magpie:fixture/model".into());
            panel.provider = Some("magpie".into());
            panel.magpie_effort = Some(crate::custom_providers::MagpieEffort::Unsupported);
            panel.identity_effort(Some("medium".into()));
            cx.notify();
        });
        vcx.run_until_parked();
        while commands.try_recv().is_ok() {}
        let before = panel.read_with(vcx, |panel, _| panel.items.len());
        let pill = vcx.debug_bounds("panel-swarm").unwrap();
        vcx.simulate_click(pill.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("swarm-picker").is_some());
        reply_open_queries(&commands, true);
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| {
            assert_eq!(panel.items.len(), before);
            assert_eq!(panel.swarm.enabled, Some(false));
            assert!(!panel.swarm.model_pending);
            assert_eq!(panel.swarm.autoreview, Some(false));
            assert_eq!(panel.swarm.autojudge, Some(true));
            assert!(panel.swarm.agents_defaults_loaded);
        });
        let on = vcx.debug_bounds("swarm-feature-on").unwrap();
        vcx.simulate_click(on.center(), gpui::Modifiers::default());
        let feature = expect_runtime_reply(commands.try_recv().unwrap(), RuntimeCommand::Swarm, &["on"]);
        panel.read_with(vcx, |panel, _| assert_eq!(panel.swarm.enabled, Some(false)));
        feature.try_send(Ok(output(RuntimeCommand::Swarm, json!({"enabled": true})))).unwrap();
        vcx.run_until_parked();
        let off = vcx.debug_bounds("swarm-feature-off").unwrap();
        vcx.simulate_click(off.center(), gpui::Modifiers::default());
        let feature = expect_runtime_reply(commands.try_recv().unwrap(), RuntimeCommand::Swarm, &["off"]);
        feature.try_send(Ok(output(RuntimeCommand::Swarm, json!({"enabled": false})))).unwrap();
        vcx.run_until_parked();
        let pin = vcx.debug_bounds("swarm-subagent-pin-current").unwrap();
        vcx.simulate_click(pin.center(), gpui::Modifiers::default());
        reply_runtime(&commands, RuntimeCommand::Agents, &["swarm", "magpie:fixture/model"],
            json!({"overrides": {"swarm": "magpie:fixture/model"}}));
        vcx.run_until_parked();
        let model = expect_runtime_reply(commands.try_recv().unwrap(), RuntimeCommand::SubagentModel, &["magpie:fixture/model"]);
        model.try_send(Ok(output(RuntimeCommand::SubagentModel, json!({"model": "magpie:fixture/model"})))).unwrap();
        vcx.run_until_parked();
        panel.read_with(vcx, |panel, _| assert_eq!(panel.swarm.model_label(), "magpie:fixture/model"));
        let inherit = vcx.debug_bounds("swarm-subagent-inherit").unwrap();
        vcx.simulate_click(inherit.center(), gpui::Modifiers::default());
        reply_runtime(&commands, RuntimeCommand::Agents, &["swarm", "default"], json!({"overrides": {}}));
        vcx.run_until_parked();
        let model = expect_runtime_reply(commands.try_recv().unwrap(), RuntimeCommand::SubagentModel, &["default"]);
        model.try_send(Ok(output(RuntimeCommand::SubagentModel, json!({"model": null})))).unwrap();
        vcx.run_until_parked();

        for (selector, expected) in [("swarm-mode-light", "swarm"), ("swarm-mode-deep", "swarm-deep"), ("swarm-mode-standard", "medium")] {
            let button = vcx.debug_bounds(selector).unwrap();
            vcx.simulate_click(button.center(), gpui::Modifiers::default());
            assert!(matches!(commands.try_recv(), Ok(Command::SessionOperation {
                session_id, operation: SessionOperation::SetEffort(level),
            }) if session_id == "swarm-test" && level == expected));
            if is_swarm_effort(expected) {
                panel.read_with(vcx, |panel, _| assert_eq!(panel.effort_pill_label().as_deref(), Some(effort_label(expected))));
            }
            vcx.run_until_parked();
        }
        assert!(commands.try_recv().is_err());
        panel.update(vcx, |panel, cx| {
            assert_eq!(panel.items.len(), before);
            assert_eq!(panel.swarm.enabled, Some(false));
            assert_eq!(panel.swarm.model_label(), tr("Inherit main model", "继承主模型"));
            panel.apply(&ApiEvent::RuntimeCommandResult {
                session_id: "swarm-test".into(),
                command: RuntimeCommand::Swarm,
                data: json!({"enabled": true}),
                message: "raw runtime message".into(),
            }, cx);
            assert_eq!(panel.swarm.enabled, Some(true));
            assert_eq!(panel.items.len(), before + 1);
            assert!(matches!(panel.items.last(), Some(Item::Assistant(text))
                if text.contains(tr("**Swarm**", "**协作**")) && !text.contains("raw runtime message")));
            panel.apply(&ApiEvent::RuntimeCommandResult {
                session_id: "swarm-test".into(),
                command: RuntimeCommand::Agents,
                data: json!({"overrides": {"swarm": "agent-model"}, "defaults": {"swarm": "default-model"}}),
                message: "raw runtime message".into(),
            }, cx);
            assert_eq!(panel.swarm.model_label(), "agent-model");
            assert_eq!(panel.items.len(), before + 2);
        });
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("swarm-picker").is_none());
    }
}
